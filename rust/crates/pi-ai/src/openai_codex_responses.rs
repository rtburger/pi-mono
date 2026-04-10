use crate::{
    AiProvider, AssistantEventStream, StreamOptions,
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
use pi_events::{
    AssistantEvent, AssistantMessage, Context, Model, StopReason, ToolDefinition, Usage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const OPENAI_CODEX_AUTH_CLAIM: &str = "https://api.openai.com/auth";
const CODEX_ALLOWED_TOOL_CALL_PROVIDERS: &[&str] = &["openai", "openai-codex"];
const DEFAULT_TEXT_VERBOSITY: &str = "medium";
const IN_MEMORY_CACHE_RETENTION: &str = "in-memory";

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

pub fn stream_openai_codex_http(
    model: Model,
    params: OpenAiCodexResponsesRequestParams,
    request_headers: BTreeMap<String, String>,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut signal = signal;
        let mut state = OpenAiResponsesStreamState::new(&model);

        if is_signal_aborted(&signal) {
            yield Ok(state.aborted_event());
            return;
        }

        let mut request_builder = reqwest::Client::new().post(resolve_codex_url(&model.base_url));
        for (name, value) in &request_headers {
            request_builder = request_builder.header(name, value);
        }
        let send_future = request_builder.json(&params).send();
        tokio::pin!(send_future);

        let mut response = if let Some(signal) = signal.as_mut() {
            tokio::select! {
                response = &mut send_future => {
                    match response {
                        Ok(response) => response,
                        Err(error) => {
                            yield Ok(state.error_event(format!("HTTP request failed: {error}")));
                            return;
                        }
                    }
                }
                _ = wait_for_abort(signal) => {
                    yield Ok(state.aborted_event());
                    return;
                }
            }
        } else {
            match send_future.await {
                Ok(response) => response,
                Err(error) => {
                    yield Ok(state.error_event(format!("HTTP request failed: {error}")));
                    return;
                }
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let text_future = response.text();
            tokio::pin!(text_future);
            let body = if let Some(signal) = signal.as_mut() {
                tokio::select! {
                    body = &mut text_future => body.unwrap_or_default(),
                    _ = wait_for_abort(signal) => {
                        yield Ok(state.aborted_event());
                        return;
                    }
                }
            } else {
                text_future.await.unwrap_or_default()
            };
            let detail = if body.is_empty() {
                format!("HTTP request failed with status {status}")
            } else {
                format!("HTTP request failed with status {status}: {body}")
            };
            yield Ok(state.error_event(detail));
            return;
        }

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
        let Some(api_key) = options.api_key.clone() else {
            return terminal_error_stream(&model, "OpenAI Codex API key is required");
        };

        let request_headers = match build_runtime_request_headers(
            &model,
            &options.headers,
            &api_key,
            options.session_id.as_deref(),
        ) {
            Ok(headers) => headers,
            Err(error) => return terminal_error_stream(&model, &error),
        };

        let params = build_openai_codex_responses_request_params(
            &model,
            &context,
            &OpenAiCodexResponsesRequestOptions {
                reasoning_effort: options.reasoning_effort.clone(),
                reasoning_summary: options.reasoning_summary.clone(),
                temperature: options.temperature,
                session_id: options.session_id.clone(),
                text_verbosity: None,
            },
        );

        stream_openai_codex_http(model, params, request_headers, options.signal.clone())
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

fn build_runtime_request_headers(
    model: &Model,
    option_headers: &BTreeMap<String, String>,
    api_key: &str,
    session_id: Option<&str>,
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
    headers.insert("OpenAI-Beta".into(), "responses=experimental".into());
    headers.insert("accept".into(), "text/event-stream".into());
    headers.insert("content-type".into(), "application/json".into());

    if let Some(session_id) = session_id {
        headers.insert("session_id".into(), session_id.into());
        headers.insert("conversation_id".into(), session_id.into());
    }

    Ok(headers)
}

fn process_codex_events(
    state: &mut OpenAiResponsesStreamState,
    events: Vec<OpenAiResponsesStreamEnvelope>,
) -> Vec<AssistantEvent> {
    let mut emitted = Vec::new();
    for event in events.into_iter().map(map_codex_event) {
        emitted.extend(state.process_event(event));
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

            let mut response = event
                .data
                .remove("response")
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();

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
    let mut output = Vec::new();
    let mut accumulator = 0u32;
    let mut bits = 0u8;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        } as u32;

        accumulator = (accumulator << 6) | value;
        bits += 6;

        while bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
        }
    }

    Some(output)
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
    use super::clamp_reasoning_effort;

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
}
