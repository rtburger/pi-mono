use crate::{
    AiProvider, AssistantEventStream, StreamOptions, Transport,
    models::{get_model_headers, get_provider_headers},
    openai_responses::{
        OpenAiResponsesConvertOptions, OpenAiResponsesReasoning, OpenAiResponsesSseDecoder,
        OpenAiResponsesStreamEnvelope, OpenAiResponsesStreamState, ResponsesInputItem,
        convert_openai_responses_messages, is_signal_aborted, is_terminal_event, wait_for_abort,
    },
    register_provider,
    unicode::sanitize_provider_text,
};
use async_stream::stream;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::{SinkExt, StreamExt};
use pi_events::{
    AssistantEvent, AssistantMessage, Context, Model, StopReason, ToolDefinition, Usage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message as WebSocketMessage,
        client::IntoClientRequest,
        http::{HeaderName, HeaderValue, Request},
    },
};

const OPENAI_CODEX_AUTH_CLAIM: &str = "https://api.openai.com/auth";
const CODEX_ALLOWED_TOOL_CALL_PROVIDERS: &[&str] = &["openai", "openai-codex", "opencode"];
const DEFAULT_TEXT_VERBOSITY: &str = "medium";
const IN_MEMORY_CACHE_RETENTION: &str = "in-memory";
const OPENAI_BETA_RESPONSES_WEBSOCKETS: &str = "responses_websockets=2026-02-06";
#[cfg(test)]
const SESSION_WEBSOCKET_CACHE_TTL_MS: u64 = 50;
#[cfg(not(test))]
const SESSION_WEBSOCKET_CACHE_TTL_MS: u64 = 5 * 60 * 1000;
const MAX_HTTP_RETRIES: u32 = 3;
const BASE_RETRY_DELAY_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpenAiCodexResponsesRequestOptions {
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub temperature: Option<f64>,
    pub session_id: Option<String>,
    pub text_verbosity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCodexResponsesTextConfig {
    pub verbosity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCodexResponsesRequestParams {
    pub model: String,
    pub store: bool,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAiCodexResponsesToolDefinition>>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
    pub text: OpenAiCodexResponsesTextConfig,
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_retention: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<OpenAiResponsesReasoning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAiCodexResponsesToolDefinition {
    Function {
        name: String,
        description: String,
        parameters: Value,
        strict: Option<bool>,
    },
}

pub fn build_openai_codex_responses_request_params(
    model: &Model,
    context: &Context,
    options: &OpenAiCodexResponsesRequestOptions,
) -> OpenAiCodexResponsesRequestParams {
    let input = convert_openai_responses_messages(
        model,
        context,
        CODEX_ALLOWED_TOOL_CALL_PROVIDERS,
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    OpenAiCodexResponsesRequestParams {
        model: model.id.clone(),
        store: false,
        stream: true,
        instructions: context.system_prompt.as_deref().map(sanitize_provider_text),
        input,
        tools: (!context.tools.is_empty()).then(|| convert_codex_tools(&context.tools)),
        tool_choice: "auto".into(),
        parallel_tool_calls: true,
        text: OpenAiCodexResponsesTextConfig {
            verbosity: options
                .text_verbosity
                .clone()
                .unwrap_or_else(|| DEFAULT_TEXT_VERBOSITY.into()),
        },
        include: vec!["reasoning.encrypted_content".into()],
        prompt_cache_key: options.session_id.clone(),
        prompt_cache_retention: options
            .session_id
            .as_ref()
            .map(|_| IN_MEMORY_CACHE_RETENTION.into()),
        temperature: options.temperature,
        reasoning: options
            .reasoning_effort
            .as_ref()
            .map(|effort| OpenAiResponsesReasoning {
                effort: clamp_reasoning_effort(&model.id, effort),
                summary: options
                    .reasoning_summary
                    .clone()
                    .unwrap_or_else(|| "auto".into()),
            }),
    }
}

pub fn parse_openai_codex_sse_text(
    payload: &str,
) -> Result<Vec<OpenAiResponsesStreamEnvelope>, crate::AiError> {
    let mut decoder = OpenAiResponsesSseDecoder::default();
    let mut events = decoder.push_bytes(payload.as_bytes())?;
    events.extend(decoder.finish()?);
    Ok(events.into_iter().map(map_codex_event).collect())
}

pub fn stream_openai_codex_sse_text(
    model: Model,
    payload: &str,
) -> Result<AssistantEventStream, crate::AiError> {
    let events = parse_openai_codex_sse_text(payload)?;
    Ok(crate::openai_responses::stream_openai_responses_sse_events(
        model, events,
    ))
}

pub fn stream_openai_codex_http<T>(
    model: Model,
    params: T,
    request_headers: BTreeMap<String, String>,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    Box::pin(stream! {
        let mut signal = signal;
        let mut state = OpenAiResponsesStreamState::new(&model);

        if is_signal_aborted(&signal) {
            yield Ok(state.aborted_event());
            return;
        }

        let mut response = match send_codex_http_request_with_retry(
            &model,
            &params,
            &request_headers,
            &mut signal,
        ).await {
            Ok(response) => response,
            Err(error) if error == "Request was aborted" => {
                yield Ok(state.aborted_event());
                return;
            }
            Err(error) => {
                yield Ok(state.error_event(error));
                return;
            }
        };

        yield Ok(state.start_event());

        let mut decoder = OpenAiResponsesSseDecoder::default();

        loop {
            let chunk_future = response.chunk();
            tokio::pin!(chunk_future);
            let next_chunk = if let Some(signal) = signal.as_mut() {
                tokio::select! {
                    chunk = &mut chunk_future => chunk,
                    _ = wait_for_abort(signal) => {
                        yield Ok(state.aborted_event());
                        return;
                    }
                }
            } else {
                chunk_future.await
            };

            match next_chunk {
                Ok(Some(chunk)) => {
                    let events = match decoder.push_bytes(chunk.as_ref()) {
                        Ok(events) => events,
                        Err(error) => {
                            yield Ok(state.error_event(error.to_string()));
                            return;
                        }
                    };

                    for assistant_event in process_codex_events(&mut state, events) {
                        let terminal = is_terminal_event(&assistant_event);
                        yield Ok(assistant_event);
                        if terminal {
                            return;
                        }
                    }
                }
                Ok(None) => {
                    let events = match decoder.finish() {
                        Ok(events) => events,
                        Err(error) => {
                            yield Ok(state.error_event(error.to_string()));
                            return;
                        }
                    };

                    for assistant_event in process_codex_events(&mut state, events) {
                        let terminal = is_terminal_event(&assistant_event);
                        yield Ok(assistant_event);
                        if terminal {
                            return;
                        }
                    }
                    return;
                }
                Err(error) => {
                    yield Ok(state.error_event(format!("Failed to read SSE response body: {error}")));
                    return;
                }
            }
        }
    })
}

pub fn stream_openai_codex_websocket<T>(
    model: Model,
    params: T,
    request_headers: BTreeMap<String, String>,
    session_cache_id: Option<String>,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    Box::pin(stream! {
        let mut signal = signal;
        let state = OpenAiResponsesStreamState::new(&model);

        if is_signal_aborted(&signal) {
            yield Ok(state.aborted_event());
            return;
        }

        let websocket_url = resolve_codex_websocket_url(&model.base_url);
        let socket = match connect_and_start_codex_websocket(
            &websocket_url,
            &request_headers,
            session_cache_id.as_deref(),
            &params,
            &mut signal,
        ).await {
            Ok(socket) => socket,
            Err(error) if error == "Request was aborted" => {
                yield Ok(state.aborted_event());
                return;
            }
            Err(error) => {
                yield Ok(state.error_event(error));
                return;
            }
        };

        let mut websocket_stream = stream_openai_codex_connected_websocket(model.clone(), socket, signal.clone());
        while let Some(event) = websocket_stream.next().await {
            yield event;
        }
    })
}

pub fn stream_openai_codex_auto<T>(
    model: Model,
    params: T,
    http_request_headers: BTreeMap<String, String>,
    websocket_request_headers: BTreeMap<String, String>,
    session_cache_id: Option<String>,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream
where
    T: Serialize + Clone + Send + Sync + 'static,
{
    Box::pin(stream! {
        let mut signal = signal;
        let state = OpenAiResponsesStreamState::new(&model);

        if is_signal_aborted(&signal) {
            yield Ok(state.aborted_event());
            return;
        }

        let websocket_url = resolve_codex_websocket_url(&model.base_url);
        match connect_and_start_codex_websocket(
            &websocket_url,
            &websocket_request_headers,
            session_cache_id.as_deref(),
            &params,
            &mut signal,
        ).await {
            Ok(socket) => {
                let mut websocket_stream = stream_openai_codex_connected_websocket(model.clone(), socket, signal.clone());
                while let Some(event) = websocket_stream.next().await {
                    yield event;
                }
            }
            Err(error) if error == "Request was aborted" => {
                yield Ok(state.aborted_event());
            }
            Err(_) => {
                let mut http_stream = stream_openai_codex_http(
                    model.clone(),
                    params.clone(),
                    http_request_headers.clone(),
                    signal.clone(),
                );
                while let Some(event) = http_stream.next().await {
                    yield event;
                }
            }
        }
    })
}

type CodexWebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct CachedCodexWebSocketEntry {
    socket: tokio::sync::Mutex<Option<CodexWebSocket>>,
    idle_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl CachedCodexWebSocketEntry {
    fn new() -> Self {
        Self {
            socket: tokio::sync::Mutex::new(None),
            idle_task: Mutex::new(None),
        }
    }
}

struct AcquiredCodexWebSocket {
    socket: Option<CodexWebSocket>,
    cache_entry: Option<Arc<CachedCodexWebSocketEntry>>,
    session_id: Option<String>,
    reused_cached_socket: bool,
}

impl AcquiredCodexWebSocket {
    async fn release(self, keep: bool) {
        let AcquiredCodexWebSocket {
            mut socket,
            cache_entry,
            session_id,
            ..
        } = self;

        let Some(mut socket) = socket.take() else {
            return;
        };

        if keep {
            if let (Some(entry), Some(session_id)) = (cache_entry.as_ref(), session_id.as_ref()) {
                {
                    let mut guard = entry.socket.lock().await;
                    *guard = Some(socket);
                }
                schedule_session_websocket_expiry(session_id.clone(), entry.clone());
                return;
            }
        }

        if let (Some(entry), Some(session_id)) = (cache_entry.as_ref(), session_id.as_deref()) {
            abort_idle_task(entry);
            remove_cached_websocket_entry_if_same(session_id, entry);
        }

        let _ = socket.close(None).await;
    }
}

struct CodexSocketGuard {
    websocket: Option<AcquiredCodexWebSocket>,
    socket: Option<CodexWebSocket>,
}

impl CodexSocketGuard {
    fn new(mut websocket: AcquiredCodexWebSocket) -> Self {
        let socket = websocket.socket.take().expect("websocket missing socket");
        Self {
            websocket: Some(websocket),
            socket: Some(socket),
        }
    }

    fn socket_mut(&mut self) -> &mut CodexWebSocket {
        self.socket.as_mut().expect("websocket missing socket")
    }

    async fn release(mut self, keep: bool) {
        let mut websocket = self.websocket.take().expect("websocket missing socket");
        websocket.socket = self.socket.take();
        websocket.release(keep).await;
    }
}

fn codex_websocket_cache() -> &'static Mutex<BTreeMap<String, Arc<CachedCodexWebSocketEntry>>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Arc<CachedCodexWebSocketEntry>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn abort_idle_task(entry: &CachedCodexWebSocketEntry) {
    if let Some(handle) = entry.idle_task.lock().unwrap().take() {
        handle.abort();
    }
}

fn remove_cached_websocket_entry_if_same(session_id: &str, entry: &Arc<CachedCodexWebSocketEntry>) {
    let mut cache = codex_websocket_cache().lock().unwrap();
    if cache
        .get(session_id)
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        cache.remove(session_id);
    }
}

fn schedule_session_websocket_expiry(session_id: String, entry: Arc<CachedCodexWebSocketEntry>) {
    abort_idle_task(&entry);

    let entry_for_task = entry.clone();
    let session_id_for_task = session_id.clone();
    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SESSION_WEBSOCKET_CACHE_TTL_MS)).await;

        let should_remove = {
            let cache = codex_websocket_cache().lock().unwrap();
            cache
                .get(&session_id_for_task)
                .is_some_and(|current| Arc::ptr_eq(current, &entry_for_task))
        };

        if !should_remove {
            return;
        }

        remove_cached_websocket_entry_if_same(&session_id_for_task, &entry_for_task);
        let mut guard = entry_for_task.socket.lock().await;
        if let Some(mut socket) = guard.take() {
            let _ = socket.close(None).await;
        }
    });

    *entry.idle_task.lock().unwrap() = Some(handle);
}

fn stream_openai_codex_connected_websocket(
    model: Model,
    websocket: AcquiredCodexWebSocket,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut signal = signal;
        let mut socket = CodexSocketGuard::new(websocket);
        let mut state = OpenAiResponsesStreamState::new(&model);

        if is_signal_aborted(&signal) {
            socket.release(false).await;
            yield Ok(state.aborted_event());
            return;
        }

        yield Ok(state.start_event());

        loop {
            let next_message = socket.socket_mut().next();
            tokio::pin!(next_message);
            let next_message = if let Some(signal) = signal.as_mut() {
                tokio::select! {
                    message = &mut next_message => message,
                    _ = wait_for_abort(signal) => {
                        socket.release(false).await;
                        yield Ok(state.aborted_event());
                        return;
                    }
                }
            } else {
                next_message.await
            };

            let Some(result) = next_message else {
                socket.release(false).await;
                yield Ok(state.error_event("WebSocket stream closed before response.completed"));
                return;
            };

            let message_text = match result {
                Ok(WebSocketMessage::Text(text)) => Some(text.to_string()),
                Ok(WebSocketMessage::Binary(bytes)) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                Ok(WebSocketMessage::Ping(_)) | Ok(WebSocketMessage::Pong(_)) => None,
                Ok(WebSocketMessage::Close(frame)) => {
                    let detail = frame
                        .map(|frame| format!("WebSocket closed {} {}", u16::from(frame.code), frame.reason))
                        .unwrap_or_else(|| "WebSocket closed before response.completed".into());
                    socket.release(false).await;
                    yield Ok(state.error_event(detail.trim().to_string()));
                    return;
                }
                Ok(_) => None,
                Err(error) => {
                    socket.release(false).await;
                    yield Ok(state.error_event(format!("WebSocket error: {error}")));
                    return;
                }
            };

            let Some(message_text) = message_text else {
                continue;
            };

            let event = match parse_codex_websocket_event(&message_text) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(error) => {
                    socket.release(false).await;
                    yield Ok(state.error_event(error));
                    return;
                }
            };

            for assistant_event in process_codex_events(&mut state, vec![event]) {
                let keep = matches!(assistant_event, AssistantEvent::Done { .. });
                let terminal = is_terminal_event(&assistant_event);
                yield Ok(assistant_event);
                if terminal {
                    socket.release(keep).await;
                    return;
                }
            }
        }
    })
}

async fn connect_and_start_codex_websocket<T>(
    url: &str,
    request_headers: &BTreeMap<String, String>,
    session_cache_id: Option<&str>,
    params: &T,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<AcquiredCodexWebSocket, String>
where
    T: Serialize + Sync,
{
    let mut websocket =
        acquire_codex_websocket(url, request_headers, session_cache_id, signal).await?;
    match send_codex_websocket_request(
        websocket.socket.as_mut().expect("websocket missing socket"),
        params,
        signal,
    )
    .await
    {
        Ok(()) => Ok(websocket),
        Err(error) => {
            let should_retry_cached_socket =
                websocket.reused_cached_socket && session_cache_id.is_some();
            websocket.release(false).await;
            if !should_retry_cached_socket {
                return Err(error);
            }

            let mut websocket =
                acquire_codex_websocket(url, request_headers, session_cache_id, signal).await?;
            send_codex_websocket_request(
                websocket.socket.as_mut().expect("websocket missing socket"),
                params,
                signal,
            )
            .await?;
            Ok(websocket)
        }
    }
}

async fn acquire_codex_websocket(
    url: &str,
    request_headers: &BTreeMap<String, String>,
    session_cache_id: Option<&str>,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<AcquiredCodexWebSocket, String> {
    if let Some(session_cache_id) = session_cache_id {
        let cached_entry = {
            let cache = codex_websocket_cache().lock().unwrap();
            cache.get(session_cache_id).cloned()
        };
        if let Some(entry) = cached_entry {
            abort_idle_task(&entry);
            let mut guard = entry.socket.lock().await;
            if let Some(socket) = guard.take() {
                drop(guard);
                return Ok(AcquiredCodexWebSocket {
                    socket: Some(socket),
                    cache_entry: Some(entry),
                    session_id: Some(session_cache_id.to_string()),
                    reused_cached_socket: true,
                });
            }
            drop(guard);
            return Ok(AcquiredCodexWebSocket {
                socket: Some(connect_codex_websocket(url, request_headers, signal).await?),
                cache_entry: None,
                session_id: None,
                reused_cached_socket: false,
            });
        }

        let socket = connect_codex_websocket(url, request_headers, signal).await?;
        let entry = Arc::new(CachedCodexWebSocketEntry::new());
        codex_websocket_cache()
            .lock()
            .unwrap()
            .insert(session_cache_id.to_string(), entry.clone());
        return Ok(AcquiredCodexWebSocket {
            socket: Some(socket),
            cache_entry: Some(entry),
            session_id: Some(session_cache_id.to_string()),
            reused_cached_socket: false,
        });
    }

    Ok(AcquiredCodexWebSocket {
        socket: Some(connect_codex_websocket(url, request_headers, signal).await?),
        cache_entry: None,
        session_id: None,
        reused_cached_socket: false,
    })
}

async fn connect_codex_websocket(
    url: &str,
    request_headers: &BTreeMap<String, String>,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<CodexWebSocket, String> {
    let request = build_codex_websocket_request(url, request_headers)?;
    let connect_future = connect_async(request);
    tokio::pin!(connect_future);

    if let Some(signal) = signal.as_mut() {
        tokio::select! {
            response = &mut connect_future => {
                response
                    .map(|(socket, _)| socket)
                    .map_err(|error| format!("WebSocket connection failed: {error}"))
            }
            _ = wait_for_abort(signal) => Err("Request was aborted".into()),
        }
    } else {
        connect_future
            .await
            .map(|(socket, _)| socket)
            .map_err(|error| format!("WebSocket connection failed: {error}"))
    }
}

fn build_codex_websocket_request(
    url: &str,
    request_headers: &BTreeMap<String, String>,
) -> Result<Request<()>, String> {
    let mut request = url
        .into_client_request()
        .map_err(|error| format!("WebSocket request failed: {error}"))?;

    for (name, value) in request_headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("Invalid WebSocket header name {name:?}: {error}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|error| format!("Invalid WebSocket header value for {name}: {error}"))?;
        request.headers_mut().insert(name, value);
    }

    Ok(request)
}

async fn send_codex_websocket_request<T>(
    socket: &mut CodexWebSocket,
    params: &T,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<(), String>
where
    T: Serialize + Sync,
{
    let payload = build_codex_websocket_payload(params)?;
    let send_future = socket.send(WebSocketMessage::Text(payload.into()));
    tokio::pin!(send_future);

    if let Some(signal) = signal.as_mut() {
        tokio::select! {
            result = &mut send_future => result.map_err(|error| format!("WebSocket send failed: {error}")),
            _ = wait_for_abort(signal) => Err("Request was aborted".into()),
        }
    } else {
        send_future
            .await
            .map_err(|error| format!("WebSocket send failed: {error}"))
    }
}

fn build_codex_websocket_payload<T>(params: &T) -> Result<String, String>
where
    T: Serialize,
{
    let value = serde_json::to_value(params)
        .map_err(|error| format!("Failed to serialize WebSocket payload: {error}"))?;
    let Value::Object(mut object) = value else {
        return Err("Failed to serialize WebSocket payload".into());
    };
    object.insert("type".into(), Value::String("response.create".into()));
    Ok(Value::Object(object).to_string())
}

fn parse_codex_websocket_event(
    payload: &str,
) -> Result<Option<OpenAiResponsesStreamEnvelope>, String> {
    let value: Value = serde_json::from_str(payload)
        .map_err(|error| format!("Invalid Codex WebSocket event: {error}"))?;
    let Value::Object(mut object) = value else {
        return Ok(None);
    };
    let Some(event_type) = object
        .remove("type")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
    else {
        return Ok(None);
    };

    Ok(Some(OpenAiResponsesStreamEnvelope {
        event_type,
        data: object,
    }))
}

async fn send_codex_http_request_with_retry<T>(
    model: &Model,
    params: &T,
    request_headers: &BTreeMap<String, String>,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<reqwest::Response, String>
where
    T: Serialize + Sync,
{
    let client = reqwest::Client::new();
    let url = resolve_codex_url(&model.base_url);
    let mut last_error = None;

    for attempt in 0..=MAX_HTTP_RETRIES {
        if is_signal_aborted(signal) {
            return Err("Request was aborted".into());
        }

        let mut request_builder = client.post(&url);
        for (name, value) in request_headers {
            request_builder = request_builder.header(name, value);
        }
        let send_future = request_builder.json(params).send();
        tokio::pin!(send_future);

        let response = if let Some(signal) = signal.as_mut() {
            tokio::select! {
                response = &mut send_future => response,
                _ = wait_for_abort(signal) => return Err("Request was aborted".into()),
            }
        } else {
            send_future.await
        };

        match response {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                let body = read_codex_http_error_body(response, signal).await?;
                if attempt < MAX_HTTP_RETRIES
                    && is_retryable_codex_http_error(status.as_u16(), &body)
                {
                    sleep_with_abort(
                        Duration::from_millis(BASE_RETRY_DELAY_MS * 2u64.pow(attempt)),
                        signal,
                    )
                    .await?;
                    continue;
                }
                let detail = if body.is_empty() {
                    format!("HTTP request failed with status {status}")
                } else {
                    format!("HTTP request failed with status {status}: {body}")
                };
                return Err(detail);
            }
            Err(error) => {
                if attempt < MAX_HTTP_RETRIES {
                    sleep_with_abort(
                        Duration::from_millis(BASE_RETRY_DELAY_MS * 2u64.pow(attempt)),
                        signal,
                    )
                    .await?;
                    last_error = Some(format!("HTTP request failed: {error}"));
                    continue;
                }
                return Err(format!("HTTP request failed: {error}"));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "HTTP request failed after retries".into()))
}

async fn read_codex_http_error_body(
    response: reqwest::Response,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<String, String> {
    let body_future = response.text();
    tokio::pin!(body_future);

    if let Some(signal) = signal.as_mut() {
        tokio::select! {
            body = &mut body_future => Ok(body.unwrap_or_default()),
            _ = wait_for_abort(signal) => Err("Request was aborted".into()),
        }
    } else {
        Ok(body_future.await.unwrap_or_default())
    }
}

async fn sleep_with_abort(
    duration: Duration,
    signal: &mut Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<(), String> {
    if let Some(signal) = signal.as_mut() {
        tokio::select! {
            _ = tokio::time::sleep(duration) => Ok(()),
            _ = wait_for_abort(signal) => Err("Request was aborted".into()),
        }
    } else {
        tokio::time::sleep(duration).await;
        Ok(())
    }
}

fn is_retryable_codex_http_error(status: u16, error_text: &str) -> bool {
    if matches!(status, 429 | 500 | 502 | 503 | 504) {
        return true;
    }

    let error_text = error_text.to_ascii_lowercase();
    error_text.contains("rate limit")
        || error_text.contains("ratelimit")
        || error_text.contains("overloaded")
        || error_text.contains("service unavailable")
        || error_text.contains("upstream connect")
        || error_text.contains("connection refused")
}

pub fn register_openai_codex_responses_provider() {
    register_provider(
        "openai-codex-responses",
        Arc::new(OpenAiCodexResponsesProvider),
    );
}

#[derive(Default)]
pub struct OpenAiCodexResponsesProvider;

impl AiProvider for OpenAiCodexResponsesProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        Box::pin(stream! {
            let Some(api_key) = options.api_key.clone() else {
                let mut inner = terminal_error_stream(&model, "OpenAI Codex API key is required");
                while let Some(event) = inner.next().await {
                    yield event;
                }
                return;
            };

            let params = build_openai_codex_responses_request_params(
                &model,
                &context,
                &OpenAiCodexResponsesRequestOptions {
                    reasoning_effort: options.reasoning_effort.clone(),
                    reasoning_summary: options.reasoning_summary.clone(),
                    temperature: options.temperature,
                    session_id: options.session_id.clone(),
                    text_verbosity: options.text_verbosity.clone(),
                },
            );
            let payload = match crate::apply_payload_hook(&model, params, options.on_payload.as_ref()).await {
                Ok(payload) => payload,
                Err(error) => {
                    let message = error.to_string();
                    let mut inner = terminal_error_stream(&model, &message);
                    while let Some(event) = inner.next().await {
                        yield event;
                    }
                    return;
                }
            };

            let http_request_headers = match build_sse_request_headers(
                &model,
                &options.headers,
                &api_key,
                options.session_id.as_deref(),
            ) {
                Ok(headers) => headers,
                Err(error) => {
                    let mut inner = terminal_error_stream(&model, &error);
                    while let Some(event) = inner.next().await {
                        yield event;
                    }
                    return;
                }
            };

            let mut inner = match options.transport.unwrap_or(Transport::Sse) {
                Transport::Sse => stream_openai_codex_http(
                    model,
                    payload,
                    http_request_headers,
                    options.signal.clone(),
                ),
                Transport::WebSocket | Transport::Auto => {
                    let websocket_request_id = options
                        .session_id
                        .clone()
                        .unwrap_or_else(create_codex_request_id);
                    let websocket_request_headers = match build_websocket_request_headers(
                        &model,
                        &options.headers,
                        &api_key,
                        &websocket_request_id,
                    ) {
                        Ok(headers) => headers,
                        Err(error) => {
                            let mut inner = terminal_error_stream(&model, &error);
                            while let Some(event) = inner.next().await {
                                yield event;
                            }
                            return;
                        }
                    };

                    match options.transport.unwrap_or(Transport::Sse) {
                        Transport::WebSocket => stream_openai_codex_websocket(
                            model,
                            payload,
                            websocket_request_headers,
                            options.session_id.clone(),
                            options.signal.clone(),
                        ),
                        Transport::Auto => stream_openai_codex_auto(
                            model,
                            payload,
                            http_request_headers,
                            websocket_request_headers,
                            options.session_id.clone(),
                            options.signal.clone(),
                        ),
                        Transport::Sse => unreachable!(),
                    }
                }
            };

            while let Some(event) = inner.next().await {
                yield event;
            }
        })
    }
}

fn convert_codex_tools(tools: &[ToolDefinition]) -> Vec<OpenAiCodexResponsesToolDefinition> {
    tools
        .iter()
        .map(|tool| OpenAiCodexResponsesToolDefinition::Function {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
            strict: None,
        })
        .collect()
}

fn clamp_reasoning_effort(model_id: &str, effort: &str) -> String {
    let id = model_id.rsplit('/').next().unwrap_or(model_id);
    match id {
        value
            if (value.starts_with("gpt-5.2")
                || value.starts_with("gpt-5.3")
                || value.starts_with("gpt-5.4"))
                && effort == "minimal" =>
        {
            "low".into()
        }
        "gpt-5.1" if effort == "xhigh" => "high".into(),
        "gpt-5.1-codex-mini" => {
            if effort == "high" || effort == "xhigh" {
                "high".into()
            } else {
                "medium".into()
            }
        }
        _ => effort.into(),
    }
}

fn build_codex_base_headers(
    model: &Model,
    option_headers: &BTreeMap<String, String>,
    api_key: &str,
) -> Result<BTreeMap<String, String>, String> {
    let account_id = extract_openai_codex_account_id(api_key)
        .ok_or_else(|| "Failed to extract accountId from token".to_string())?;

    let mut headers = get_model_headers(&model.provider, &model.id)
        .or_else(|| get_provider_headers(&model.provider))
        .unwrap_or_default();
    headers.extend(option_headers.clone());

    headers.insert("Authorization".into(), format!("Bearer {api_key}"));
    headers.insert("chatgpt-account-id".into(), account_id);
    headers.insert("originator".into(), "pi".into());
    headers.insert("User-Agent".into(), default_user_agent());

    Ok(headers)
}

fn build_sse_request_headers(
    model: &Model,
    option_headers: &BTreeMap<String, String>,
    api_key: &str,
    session_id: Option<&str>,
) -> Result<BTreeMap<String, String>, String> {
    let mut headers = build_codex_base_headers(model, option_headers, api_key)?;
    headers.insert("OpenAI-Beta".into(), "responses=experimental".into());
    headers.insert("accept".into(), "text/event-stream".into());
    headers.insert("content-type".into(), "application/json".into());

    if let Some(session_id) = session_id {
        headers.insert("session_id".into(), session_id.into());
        headers.insert("conversation_id".into(), session_id.into());
    }

    Ok(headers)
}

fn build_websocket_request_headers(
    model: &Model,
    option_headers: &BTreeMap<String, String>,
    api_key: &str,
    request_id: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut headers = build_codex_base_headers(model, option_headers, api_key)?;
    headers.remove("accept");
    headers.remove("content-type");
    headers.remove("OpenAI-Beta");
    headers.remove("openai-beta");
    headers.insert(
        "OpenAI-Beta".into(),
        OPENAI_BETA_RESPONSES_WEBSOCKETS.into(),
    );
    headers.insert("x-client-request-id".into(), request_id.into());
    headers.insert("session_id".into(), request_id.into());
    Ok(headers)
}

fn process_codex_events(
    state: &mut OpenAiResponsesStreamState,
    events: Vec<OpenAiResponsesStreamEnvelope>,
) -> Vec<AssistantEvent> {
    let mut emitted = Vec::new();
    for event in events.into_iter().map(map_codex_event) {
        emitted.extend(state.process_event(&event));
        if emitted.last().is_some_and(is_terminal_event) {
            break;
        }
    }
    emitted
}

fn map_codex_event(mut event: OpenAiResponsesStreamEnvelope) -> OpenAiResponsesStreamEnvelope {
    match event.event_type.as_str() {
        "response.done" | "response.completed" | "response.incomplete" => {
            let original_type = event.event_type.clone();
            event.event_type = "response.completed".into();

            let mut response = match event.data.remove("response") {
                Some(Value::Object(response)) => response,
                _ => serde_json::Map::new(),
            };

            let fallback_status = match original_type.as_str() {
                "response.done" | "response.completed" => Some("completed"),
                "response.incomplete" => Some("incomplete"),
                _ => None,
            };
            let normalized_status = response
                .get("status")
                .and_then(Value::as_str)
                .or(fallback_status)
                .filter(|status| {
                    matches!(
                        *status,
                        "completed"
                            | "incomplete"
                            | "failed"
                            | "cancelled"
                            | "queued"
                            | "in_progress"
                    )
                });

            if let Some(status) = normalized_status {
                response.insert("status".into(), Value::String(status.to_string()));
            } else {
                response.remove("status");
            }

            event
                .data
                .insert("response".into(), Value::Object(response));
        }
        _ => {}
    }
    event
}

fn resolve_codex_url(base_url: &str) -> String {
    let normalized = base_url.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

fn resolve_codex_websocket_url(base_url: &str) -> String {
    let url = resolve_codex_url(base_url);
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url
    }
}

fn create_codex_request_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    format!(
        "codex_{}_{}",
        now_ms(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn extract_openai_codex_account_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = decode_base64_token_component(payload)?;
    let json = serde_json::from_slice::<Value>(&decoded).ok()?;
    json.get(OPENAI_CODEX_AUTH_CLAIM)?
        .get("chatgpt_account_id")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn decode_base64_token_component(input: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(input.trim_end_matches('=')).ok()
}

fn default_user_agent() -> String {
    format!(
        "pi (rust; {} {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn terminal_error_stream(model: &Model, error_message: &str) -> AssistantEventStream {
    let error = AssistantMessage {
        role: "assistant".into(),
        content: Vec::new(),
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_id: None,
        usage: Usage::default(),
        stop_reason: StopReason::Error,
        error_message: Some(error_message.into()),
        timestamp: now_ms(),
    };

    Box::pin(stream! {
        yield Ok(AssistantEvent::Error {
            reason: StopReason::Error,
            error,
        });
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StreamOptions, Transport, complete};
    use futures::{SinkExt, StreamExt};
    use pi_events::{Context, Message, UserContent};
    use tokio::{
        net::TcpListener,
        time::{Duration, timeout},
    };
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{
            Message as WebSocketMessage,
            handshake::server::{Request as ServerRequest, Response as ServerResponse},
        },
    };

    fn mock_token() -> String {
        format!(
            "aaa.{}.bbb",
            "eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjX3Rlc3QifX0="
        )
    }

    fn model(base_url: String) -> Model {
        Model {
            id: "gpt-5.2-codex".into(),
            name: "gpt-5.2-codex".into(),
            api: "openai-codex-responses".into(),
            provider: "openai-codex".into(),
            base_url,
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            cost: pi_events::ModelCost {
                input: 1.0,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.1,
            },
            context_window: 272_000,
            max_tokens: 128_000,
            compat: None,
        }
    }

    fn context() -> Context {
        Context {
            system_prompt: Some("sys".into()),
            messages: vec![Message::User {
                content: vec![UserContent::Text { text: "hi".into() }],
                timestamp: 1,
            }],
            tools: vec![],
        }
    }

    async fn send_completed_websocket_response(
        websocket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        response_id: &str,
        message_id: &str,
    ) {
        for message in [
            serde_json::json!({"type":"response.created","response":{"id":response_id}}),
            serde_json::json!({"type":"response.output_item.added","item":{"type":"message","id":message_id,"role":"assistant","status":"in_progress","content":[]}}),
            serde_json::json!({"type":"response.output_text.delta","delta":"Hello"}),
            serde_json::json!({"type":"response.output_item.done","item":{"type":"message","id":message_id,"role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello"}]}}),
            serde_json::json!({"type":"response.completed","response":{"id":response_id,"status":"completed","usage":{"input_tokens":5,"output_tokens":3,"total_tokens":8,"input_tokens_details":{"cached_tokens":0}}}}),
        ] {
            websocket
                .send(WebSocketMessage::Text(message.to_string().into()))
                .await
                .unwrap();
        }
    }

    #[test]
    fn clamps_newer_codex_minimal_reasoning_to_low() {
        assert_eq!(clamp_reasoning_effort("gpt-5.3-codex", "minimal"), "low");
    }

    #[test]
    fn remaps_gpt_5_1_codex_mini_reasoning_levels() {
        assert_eq!(
            clamp_reasoning_effort("gpt-5.1-codex-mini", "low"),
            "medium"
        );
        assert_eq!(
            clamp_reasoning_effort("gpt-5.1-codex-mini", "xhigh"),
            "high"
        );
    }

    #[tokio::test]
    async fn reconnects_after_cached_websocket_idle_expiry() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (first_stream, _) = listener.accept().await.unwrap();
            let mut first_websocket = accept_hdr_async(
                first_stream,
                |request: &ServerRequest, response: ServerResponse| {
                    assert_eq!(
                        request.headers().get("x-client-request-id").unwrap(),
                        "session-expire"
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let first_request = timeout(Duration::from_secs(1), first_websocket.next())
                .await
                .expect("timed out waiting for first websocket request")
                .unwrap()
                .unwrap();
            assert!(matches!(first_request, WebSocketMessage::Text(_)));
            send_completed_websocket_response(
                &mut first_websocket,
                "resp_expire_1",
                "msg_expire_1",
            )
            .await;

            let (second_stream, _) = timeout(Duration::from_secs(1), listener.accept())
                .await
                .expect("timed out waiting for second websocket connection")
                .unwrap();
            let mut second_websocket = accept_hdr_async(
                second_stream,
                |request: &ServerRequest, response: ServerResponse| {
                    assert_eq!(
                        request.headers().get("x-client-request-id").unwrap(),
                        "session-expire"
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let second_request = timeout(Duration::from_secs(1), second_websocket.next())
                .await
                .expect("timed out waiting for second websocket request")
                .unwrap()
                .unwrap();
            assert!(matches!(second_request, WebSocketMessage::Text(_)));
            send_completed_websocket_response(
                &mut second_websocket,
                "resp_expire_2",
                "msg_expire_2",
            )
            .await;
            second_websocket.close(None).await.unwrap();
        });

        let first = complete(
            model(format!("http://{address}")),
            context(),
            StreamOptions {
                api_key: Some(mock_token()),
                session_id: Some("session-expire".into()),
                transport: Some(Transport::WebSocket),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(first.response_id.as_deref(), Some("resp_expire_1"));

        tokio::time::sleep(Duration::from_millis(SESSION_WEBSOCKET_CACHE_TTL_MS + 50)).await;

        let second = complete(
            model(format!("http://{address}")),
            context(),
            StreamOptions {
                api_key: Some(mock_token()),
                session_id: Some("session-expire".into()),
                transport: Some(Transport::WebSocket),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(second.response_id.as_deref(), Some("resp_expire_2"));

        server.await.unwrap();
    }
}
