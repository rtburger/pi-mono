use crate::r#loop::AssistantStreamer;
use async_stream::stream;
use pi_ai::{AiError, AssistantEventStream, StreamOptions};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
    UsageCost, UserContent,
};
use reqwest::Client;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyStreamConfig {
    pub auth_token: String,
    pub proxy_url: String,
}

impl ProxyStreamConfig {
    pub fn new(auth_token: impl Into<String>, proxy_url: impl Into<String>) -> Self {
        Self {
            auth_token: auth_token.into(),
            proxy_url: proxy_url.into(),
        }
    }
}

#[derive(Clone)]
pub struct ProxyStreamer {
    client: Client,
    config: ProxyStreamConfig,
}

impl ProxyStreamer {
    pub fn new(config: ProxyStreamConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }
}

impl AssistantStreamer for ProxyStreamer {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError> {
        Ok(stream_proxy_with_client(
            model,
            context,
            options,
            self.config.clone(),
            self.client.clone(),
        ))
    }
}

pub fn stream_proxy(
    model: Model,
    context: Context,
    options: StreamOptions,
    config: ProxyStreamConfig,
) -> AssistantEventStream {
    stream_proxy_with_client(model, context, options, config, Client::new())
}

fn stream_proxy_with_client(
    model: Model,
    context: Context,
    options: StreamOptions,
    config: ProxyStreamConfig,
    client: Client,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut state = ProxyPartialState::new(&model);

        if is_signal_aborted(&options.signal) {
            yield Ok(state.aborted_event(String::from("Request aborted by user")));
            return;
        }

        let request = client
            .post(format!("{}/api/stream", config.proxy_url))
            .bearer_auth(config.auth_token)
            .json(&build_proxy_request_body(&model, &context, &options));

        let mut signal = options.signal.clone();
        let send_future = request.send();
        tokio::pin!(send_future);

        let mut response = if let Some(signal) = signal.as_mut() {
            tokio::select! {
                response = &mut send_future => match response {
                    Ok(response) => response,
                    Err(error) => {
                        yield Ok(state.error_event(error.to_string()));
                        return;
                    }
                },
                _ = wait_for_abort(signal) => {
                    yield Ok(state.aborted_event(String::from("Request aborted by user")));
                    return;
                }
            }
        } else {
            match send_future.await {
                Ok(response) => response,
                Err(error) => {
                    yield Ok(state.error_event(error.to_string()));
                    return;
                }
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body_future = response.text();
            tokio::pin!(body_future);
            let body = if let Some(signal) = signal.as_mut() {
                tokio::select! {
                    body = &mut body_future => body.unwrap_or_default(),
                    _ = wait_for_abort(signal) => {
                        yield Ok(state.aborted_event(String::from("Request aborted by user")));
                        return;
                    }
                }
            } else {
                body_future.await.unwrap_or_default()
            };

            let error_message = proxy_http_error_message(status.as_u16(), status.canonical_reason().unwrap_or(""), &body);
            yield Ok(state.error_event(error_message));
            return;
        }

        let mut buffer = String::new();
        loop {
            let chunk_future = response.chunk();
            tokio::pin!(chunk_future);
            let next_chunk = if let Some(signal) = signal.as_mut() {
                tokio::select! {
                    chunk = &mut chunk_future => chunk,
                    _ = wait_for_abort(signal) => {
                        yield Ok(state.aborted_event(String::from("Request aborted by user")));
                        return;
                    }
                }
            } else {
                chunk_future.await
            };

            match next_chunk {
                Ok(Some(chunk)) => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    let lines = take_complete_lines(&mut buffer);
                    for line in lines {
                        let Some(data) = line.strip_prefix("data: ") else {
                            continue;
                        };
                        let trimmed = data.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        let proxy_event = match serde_json::from_str::<Value>(trimmed) {
                            Ok(proxy_event) => proxy_event,
                            Err(error) => {
                                yield Ok(state.error_event(error.to_string()));
                                return;
                            }
                        };

                        let assistant_event = match process_proxy_event(&proxy_event, &mut state) {
                            Ok(Some(event)) => event,
                            Ok(None) => continue,
                            Err(error) => {
                                yield Ok(state.error_event(error));
                                return;
                            }
                        };
                        let is_terminal = matches!(assistant_event, AssistantEvent::Done { .. } | AssistantEvent::Error { .. });
                        yield Ok(assistant_event);
                        if is_terminal {
                            return;
                        }
                    }
                }
                Ok(None) => return,
                Err(error) => {
                    yield Ok(state.error_event(format!("Failed to read proxy response body: {error}")));
                    return;
                }
            }
        }
    })
}

struct ProxyPartialState {
    partial: AssistantMessage,
    tool_call_json: BTreeMap<usize, String>,
}

impl ProxyPartialState {
    fn new(model: &Model) -> Self {
        let mut partial =
            AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
        partial.timestamp = now_ms();
        Self {
            partial,
            tool_call_json: BTreeMap::new(),
        }
    }

    fn error_event(&self, error_message: String) -> AssistantEvent {
        let mut error = self.partial.clone();
        error.stop_reason = StopReason::Error;
        error.error_message = Some(error_message);
        AssistantEvent::Error {
            reason: StopReason::Error,
            error,
        }
    }

    fn aborted_event(&self, error_message: String) -> AssistantEvent {
        let mut error = self.partial.clone();
        error.stop_reason = StopReason::Aborted;
        error.error_message = Some(error_message);
        AssistantEvent::Error {
            reason: StopReason::Aborted,
            error,
        }
    }
}

fn build_proxy_request_body(model: &Model, context: &Context, options: &StreamOptions) -> Value {
    let mut stream_options = Map::new();
    if let Some(temperature) = options.temperature {
        stream_options.insert(String::from("temperature"), json!(temperature));
    }
    if let Some(max_tokens) = options.max_tokens {
        stream_options.insert(String::from("maxTokens"), json!(max_tokens));
    }
    if let Some(reasoning) = &options.reasoning_effort {
        stream_options.insert(String::from("reasoning"), Value::String(reasoning.clone()));
    }

    json!({
        "model": model_to_proxy_json(model),
        "context": context_to_proxy_json(context),
        "options": Value::Object(stream_options),
    })
}

fn model_to_proxy_json(model: &Model) -> Value {
    json!({
        "id": model.id,
        "name": model.name,
        "api": model.api,
        "provider": model.provider,
        "baseUrl": model.base_url,
        "reasoning": model.reasoning,
        "input": model.input,
        "contextWindow": model.context_window,
        "maxTokens": model.max_tokens,
    })
}

fn context_to_proxy_json(context: &Context) -> Value {
    let mut object = Map::new();
    if let Some(system_prompt) = &context.system_prompt {
        object.insert(
            String::from("systemPrompt"),
            Value::String(system_prompt.clone()),
        );
    }
    object.insert(
        String::from("messages"),
        Value::Array(context.messages.iter().map(message_to_proxy_json).collect()),
    );
    if !context.tools.is_empty() {
        object.insert(
            String::from("tools"),
            Value::Array(
                context
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(object)
}

fn message_to_proxy_json(message: &Message) -> Value {
    match message {
        Message::User { content, timestamp } => json!({
            "role": "user",
            "content": Value::Array(content.iter().map(user_content_to_proxy_json).collect()),
            "timestamp": timestamp,
        }),
        Message::Assistant {
            content,
            api,
            provider,
            model,
            response_id,
            usage,
            stop_reason,
            error_message,
            timestamp,
        } => {
            let mut object = Map::new();
            object.insert(
                String::from("role"),
                Value::String(String::from("assistant")),
            );
            object.insert(
                String::from("content"),
                Value::Array(
                    content
                        .iter()
                        .map(assistant_content_to_proxy_json)
                        .collect(),
                ),
            );
            object.insert(String::from("api"), Value::String(api.clone()));
            object.insert(String::from("provider"), Value::String(provider.clone()));
            object.insert(String::from("model"), Value::String(model.clone()));
            if let Some(response_id) = response_id {
                object.insert(
                    String::from("responseId"),
                    Value::String(response_id.clone()),
                );
            }
            object.insert(String::from("usage"), usage_to_proxy_json(usage));
            object.insert(
                String::from("stopReason"),
                Value::String(stop_reason_to_proxy_string(stop_reason).into()),
            );
            if let Some(error_message) = error_message {
                object.insert(
                    String::from("errorMessage"),
                    Value::String(error_message.clone()),
                );
            }
            object.insert(String::from("timestamp"), json!(timestamp));
            Value::Object(object)
        }
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            is_error,
            timestamp,
        } => json!({
            "role": "toolResult",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "content": Value::Array(content.iter().map(user_content_to_proxy_json).collect()),
            "isError": is_error,
            "timestamp": timestamp,
        }),
    }
}

fn user_content_to_proxy_json(content: &UserContent) -> Value {
    match content {
        UserContent::Text { text } => json!({
            "type": "text",
            "text": text,
        }),
        UserContent::Image { data, mime_type } => json!({
            "type": "image",
            "data": data,
            "mimeType": mime_type,
        }),
    }
}

fn assistant_content_to_proxy_json(content: &AssistantContent) -> Value {
    match content {
        AssistantContent::Text {
            text,
            text_signature,
        } => {
            let mut object = Map::new();
            object.insert(String::from("type"), Value::String(String::from("text")));
            object.insert(String::from("text"), Value::String(text.clone()));
            if let Some(text_signature) = text_signature {
                object.insert(
                    String::from("textSignature"),
                    Value::String(text_signature.clone()),
                );
            }
            Value::Object(object)
        }
        AssistantContent::Thinking {
            thinking,
            thinking_signature,
            redacted,
        } => {
            let mut object = Map::new();
            object.insert(
                String::from("type"),
                Value::String(String::from("thinking")),
            );
            object.insert(String::from("thinking"), Value::String(thinking.clone()));
            if let Some(thinking_signature) = thinking_signature {
                object.insert(
                    String::from("thinkingSignature"),
                    Value::String(thinking_signature.clone()),
                );
            }
            if *redacted {
                object.insert(String::from("redacted"), Value::Bool(true));
            }
            Value::Object(object)
        }
        AssistantContent::ToolCall {
            id,
            name,
            arguments,
            thought_signature,
        } => {
            let mut object = Map::new();
            object.insert(
                String::from("type"),
                Value::String(String::from("toolCall")),
            );
            object.insert(String::from("id"), Value::String(id.clone()));
            object.insert(String::from("name"), Value::String(name.clone()));
            object.insert(
                String::from("arguments"),
                Value::Object(Map::from_iter(arguments.clone())),
            );
            if let Some(thought_signature) = thought_signature {
                object.insert(
                    String::from("thoughtSignature"),
                    Value::String(thought_signature.clone()),
                );
            }
            Value::Object(object)
        }
    }
}

fn usage_to_proxy_json(usage: &Usage) -> Value {
    json!({
        "input": usage.input,
        "output": usage.output,
        "cacheRead": usage.cache_read,
        "cacheWrite": usage.cache_write,
        "totalTokens": usage.total_tokens,
        "cost": {
            "input": usage.cost.input,
            "output": usage.cost.output,
            "cacheRead": usage.cost.cache_read,
            "cacheWrite": usage.cost.cache_write,
            "total": usage.cost.total,
        }
    })
}

fn process_proxy_event(
    proxy_event: &Value,
    state: &mut ProxyPartialState,
) -> Result<Option<AssistantEvent>, String> {
    let event_type = string_field(proxy_event, "type")?;
    match event_type {
        "start" => Ok(Some(AssistantEvent::Start {
            partial: state.partial.clone(),
        })),
        "text_start" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            replace_content(
                &mut state.partial,
                content_index,
                AssistantContent::Text {
                    text: String::new(),
                    text_signature: None,
                },
            );
            Ok(Some(AssistantEvent::TextStart {
                content_index,
                partial: state.partial.clone(),
            }))
        }
        "text_delta" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            let delta = string_field(proxy_event, "delta")?.to_string();
            match state.partial.content.get_mut(content_index) {
                Some(AssistantContent::Text { text, .. }) => {
                    text.push_str(&delta);
                    Ok(Some(AssistantEvent::TextDelta {
                        content_index,
                        delta,
                        partial: state.partial.clone(),
                    }))
                }
                _ => Err(String::from("Received text_delta for non-text content")),
            }
        }
        "text_end" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            match state.partial.content.get_mut(content_index) {
                Some(AssistantContent::Text {
                    text,
                    text_signature,
                }) => {
                    *text_signature = optional_string_field(proxy_event, "contentSignature")?;
                    Ok(Some(AssistantEvent::TextEnd {
                        content_index,
                        content: text.clone(),
                        partial: state.partial.clone(),
                    }))
                }
                _ => Err(String::from("Received text_end for non-text content")),
            }
        }
        "thinking_start" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            replace_content(
                &mut state.partial,
                content_index,
                AssistantContent::Thinking {
                    thinking: String::new(),
                    thinking_signature: None,
                    redacted: false,
                },
            );
            Ok(Some(AssistantEvent::ThinkingStart {
                content_index,
                partial: state.partial.clone(),
            }))
        }
        "thinking_delta" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            let delta = string_field(proxy_event, "delta")?.to_string();
            match state.partial.content.get_mut(content_index) {
                Some(AssistantContent::Thinking { thinking, .. }) => {
                    thinking.push_str(&delta);
                    Ok(Some(AssistantEvent::ThinkingDelta {
                        content_index,
                        delta,
                        partial: state.partial.clone(),
                    }))
                }
                _ => Err(String::from(
                    "Received thinking_delta for non-thinking content",
                )),
            }
        }
        "thinking_end" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            match state.partial.content.get_mut(content_index) {
                Some(AssistantContent::Thinking {
                    thinking,
                    thinking_signature,
                    ..
                }) => {
                    *thinking_signature = optional_string_field(proxy_event, "contentSignature")?;
                    Ok(Some(AssistantEvent::ThinkingEnd {
                        content_index,
                        content: thinking.clone(),
                        partial: state.partial.clone(),
                    }))
                }
                _ => Err(String::from(
                    "Received thinking_end for non-thinking content",
                )),
            }
        }
        "toolcall_start" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            let tool_call = AssistantContent::ToolCall {
                id: string_field(proxy_event, "id")?.to_string(),
                name: string_field(proxy_event, "toolName")?.to_string(),
                arguments: BTreeMap::new(),
                thought_signature: None,
            };
            replace_content(&mut state.partial, content_index, tool_call);
            state.tool_call_json.insert(content_index, String::new());
            Ok(Some(AssistantEvent::ToolCallStart {
                content_index,
                partial: state.partial.clone(),
            }))
        }
        "toolcall_delta" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            let delta = string_field(proxy_event, "delta")?.to_string();
            let partial_json = state.tool_call_json.entry(content_index).or_default();
            partial_json.push_str(&delta);
            if let Some(arguments) = parse_partial_object(partial_json) {
                match state.partial.content.get_mut(content_index) {
                    Some(AssistantContent::ToolCall {
                        arguments: tool_arguments,
                        ..
                    }) => {
                        *tool_arguments = arguments;
                    }
                    _ => {
                        return Err(String::from(
                            "Received toolcall_delta for non-toolCall content",
                        ));
                    }
                }
            }
            Ok(Some(AssistantEvent::ToolCallDelta {
                content_index,
                delta,
                partial: state.partial.clone(),
            }))
        }
        "toolcall_end" => {
            let content_index = usize_field(proxy_event, "contentIndex")?;
            if let Some(partial_json) = state.tool_call_json.remove(&content_index)
                && let Some(arguments) = parse_complete_object(&partial_json)
            {
                match state.partial.content.get_mut(content_index) {
                    Some(AssistantContent::ToolCall {
                        arguments: tool_arguments,
                        ..
                    }) => {
                        *tool_arguments = arguments;
                    }
                    _ => {
                        return Err(String::from(
                            "Received toolcall_end for non-toolCall content",
                        ));
                    }
                }
            }

            let tool_call = state
                .partial
                .content
                .get(content_index)
                .cloned()
                .ok_or_else(|| String::from("Received toolcall_end for missing content"))?;
            if !matches!(tool_call, AssistantContent::ToolCall { .. }) {
                return Err(String::from(
                    "Received toolcall_end for non-toolCall content",
                ));
            }
            Ok(Some(AssistantEvent::ToolCallEnd {
                content_index,
                tool_call,
                partial: state.partial.clone(),
            }))
        }
        "done" => {
            let reason = stop_reason_field(proxy_event, "reason")?;
            state.partial.stop_reason = reason.clone();
            state.partial.usage = usage_field(proxy_event, "usage")?;
            Ok(Some(AssistantEvent::Done {
                reason,
                message: state.partial.clone(),
            }))
        }
        "error" => {
            let reason = stop_reason_field(proxy_event, "reason")?;
            state.partial.stop_reason = reason.clone();
            state.partial.error_message = optional_string_field(proxy_event, "errorMessage")?;
            state.partial.usage = usage_field(proxy_event, "usage")?;
            Ok(Some(AssistantEvent::Error {
                reason,
                error: state.partial.clone(),
            }))
        }
        _ => Ok(None),
    }
}

fn take_complete_lines(buffer: &mut String) -> Vec<String> {
    let mut lines = buffer
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect::<Vec<_>>();
    let remainder = lines.pop().unwrap_or_default();
    *buffer = remainder;
    lines
}

fn replace_content(message: &mut AssistantMessage, index: usize, content: AssistantContent) {
    if message.content.len() <= index {
        message
            .content
            .resize_with(index + 1, || AssistantContent::Text {
                text: String::new(),
                text_signature: None,
            });
    }
    message.content[index] = content;
}

fn string_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing string field: {key}"))
}

fn optional_string_field(value: &Value, key: &str) -> Result<Option<String>, String> {
    match value.get(key) {
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(format!("Invalid string field: {key}")),
    }
}

fn usize_field(value: &Value, key: &str) -> Result<usize, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| format!("Missing numeric field: {key}"))
}

fn stop_reason_field(value: &Value, key: &str) -> Result<StopReason, String> {
    match string_field(value, key)? {
        "stop" => Ok(StopReason::Stop),
        "length" => Ok(StopReason::Length),
        "toolUse" => Ok(StopReason::ToolUse),
        "error" => Ok(StopReason::Error),
        "aborted" => Ok(StopReason::Aborted),
        other => Err(format!("Unknown stop reason: {other}")),
    }
}

fn usage_field(value: &Value, key: &str) -> Result<Usage, String> {
    let usage = value
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("Missing usage field: {key}"))?;
    let cost = usage
        .get("cost")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("Missing usage.cost field"))?;

    Ok(Usage {
        input: unsigned_field(usage, "input")?,
        output: unsigned_field(usage, "output")?,
        cache_read: unsigned_field(usage, "cacheRead")?,
        cache_write: unsigned_field(usage, "cacheWrite")?,
        total_tokens: unsigned_field(usage, "totalTokens")?,
        cost: UsageCost {
            input: float_field(cost, "input")?,
            output: float_field(cost, "output")?,
            cache_read: float_field(cost, "cacheRead")?,
            cache_write: float_field(cost, "cacheWrite")?,
            total: float_field(cost, "total")?,
        },
    })
}

fn unsigned_field(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Missing numeric field: {key}"))
}

fn float_field(object: &Map<String, Value>, key: &str) -> Result<f64, String> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("Missing float field: {key}"))
}

fn parse_partial_object(partial_json: &str) -> Option<BTreeMap<String, Value>> {
    parse_complete_object(partial_json)
}

fn parse_complete_object(json_text: &str) -> Option<BTreeMap<String, Value>> {
    match serde_json::from_str::<Value>(json_text) {
        Ok(Value::Object(object)) => Some(BTreeMap::from_iter(object)),
        _ => None,
    }
}

fn proxy_http_error_message(status: u16, status_text: &str, body: &str) -> String {
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(body)
        && let Some(error) = object.get("error").and_then(Value::as_str)
    {
        return format!("Proxy error: {error}");
    }

    if status_text.is_empty() {
        format!("Proxy error: {status}")
    } else {
        format!("Proxy error: {status} {status_text}")
    }
}

fn stop_reason_to_proxy_string(stop_reason: &StopReason) -> &'static str {
    match stop_reason {
        StopReason::Stop => "stop",
        StopReason::Length => "length",
        StopReason::ToolUse => "toolUse",
        StopReason::Error => "error",
        StopReason::Aborted => "aborted",
    }
}

fn is_signal_aborted(signal: &Option<watch::Receiver<bool>>) -> bool {
    signal
        .as_ref()
        .map(|signal| *signal.borrow())
        .unwrap_or(false)
}

async fn wait_for_abort(signal: &mut watch::Receiver<bool>) {
    if *signal.borrow() {
        return;
    }

    while signal.changed().await.is_ok() {
        if *signal.borrow() {
            return;
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{ProxyPartialState, build_proxy_request_body, process_proxy_event};
    use pi_ai::StreamOptions;
    use pi_events::{Context, Model, StopReason, Usage, UserContent};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn model() -> Model {
        Model {
            id: String::from("mock"),
            name: String::from("Mock"),
            api: String::from("openai-responses"),
            provider: String::from("openai"),
            base_url: String::from("https://example.com"),
            reasoning: true,
            input: vec![String::from("text"), String::from("image")],
            context_window: 128_000,
            max_tokens: 16_384,
        }
    }

    #[test]
    fn request_body_uses_camel_case_shapes() {
        let request = build_proxy_request_body(
            &model(),
            &Context {
                system_prompt: Some(String::from("sys")),
                messages: vec![],
                tools: vec![],
            },
            &StreamOptions {
                temperature: Some(0.2),
                max_tokens: Some(123),
                reasoning_effort: Some(String::from("high")),
                ..StreamOptions::default()
            },
        );

        assert_eq!(request["model"]["baseUrl"], json!("https://example.com"));
        assert_eq!(request["model"]["contextWindow"], json!(128_000));
        assert!(request["model"].get("base_url").is_none());
        assert_eq!(request["context"]["systemPrompt"], json!("sys"));
        assert!(request["context"].get("system_prompt").is_none());
        assert_eq!(request["options"]["maxTokens"], json!(123));
        assert_eq!(request["options"]["reasoning"], json!("high"));
        assert!(request["options"].get("max_tokens").is_none());
    }

    #[test]
    fn process_proxy_event_reconstructs_tool_call_arguments_and_usage() {
        let mut state = ProxyPartialState::new(&model());

        let start = process_proxy_event(&json!({ "type": "start" }), &mut state)
            .unwrap()
            .unwrap();
        assert!(matches!(start, pi_events::AssistantEvent::Start { .. }));

        process_proxy_event(
            &json!({ "type": "toolcall_start", "contentIndex": 0, "id": "tool-1", "toolName": "echo" }),
            &mut state,
        )
        .unwrap();
        process_proxy_event(
            &json!({ "type": "toolcall_delta", "contentIndex": 0, "delta": "{\"value\":\"hi" }),
            &mut state,
        )
        .unwrap();
        let tool_end = process_proxy_event(
            &json!({ "type": "toolcall_delta", "contentIndex": 0, "delta": "\"}" }),
            &mut state,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            tool_end,
            pi_events::AssistantEvent::ToolCallDelta { .. }
        ));

        let tool_end = process_proxy_event(
            &json!({ "type": "toolcall_end", "contentIndex": 0 }),
            &mut state,
        )
        .unwrap()
        .unwrap();
        match tool_end {
            pi_events::AssistantEvent::ToolCallEnd { tool_call, .. } => match tool_call {
                pi_events::AssistantContent::ToolCall { arguments, .. } => {
                    assert_eq!(arguments.get("value"), Some(&json!("hi")));
                }
                other => panic!("expected tool call content, got {other:?}"),
            },
            other => panic!("expected tool call end event, got {other:?}"),
        }

        let done = process_proxy_event(
            &json!({
                "type": "done",
                "reason": "toolUse",
                "usage": {
                    "input": 1,
                    "output": 2,
                    "cacheRead": 3,
                    "cacheWrite": 4,
                    "totalTokens": 10,
                    "cost": {
                        "input": 0.1,
                        "output": 0.2,
                        "cacheRead": 0.3,
                        "cacheWrite": 0.4,
                        "total": 1.0
                    }
                }
            }),
            &mut state,
        )
        .unwrap()
        .unwrap();
        match done {
            pi_events::AssistantEvent::Done { reason, message } => {
                assert_eq!(reason, StopReason::ToolUse);
                assert_eq!(message.stop_reason, StopReason::ToolUse);
                assert_eq!(
                    message.usage,
                    Usage {
                        input: 1,
                        output: 2,
                        cache_read: 3,
                        cache_write: 4,
                        total_tokens: 10,
                        cost: pi_events::UsageCost {
                            input: 0.1,
                            output: 0.2,
                            cache_read: 0.3,
                            cache_write: 0.4,
                            total: 1.0,
                        },
                    }
                );
            }
            other => panic!("expected done event, got {other:?}"),
        }

        assert_eq!(
            state.partial.content,
            vec![pi_events::AssistantContent::ToolCall {
                id: String::from("tool-1"),
                name: String::from("echo"),
                arguments: BTreeMap::from_iter([(String::from("value"), json!("hi"))]),
                thought_signature: None,
            }]
        );
    }

    #[test]
    fn assistant_and_tool_result_messages_use_camel_case_fields() {
        let request = build_proxy_request_body(
            &model(),
            &Context {
                system_prompt: None,
                messages: vec![
                    pi_events::Message::Assistant {
                        content: vec![pi_events::AssistantContent::Text {
                            text: String::from("hello"),
                            text_signature: Some(String::from("sig")),
                        }],
                        api: String::from("openai-responses"),
                        provider: String::from("openai"),
                        model: String::from("mock"),
                        response_id: Some(String::from("resp_1")),
                        usage: Usage::default(),
                        stop_reason: StopReason::ToolUse,
                        error_message: None,
                        timestamp: 1,
                    },
                    pi_events::Message::ToolResult {
                        tool_call_id: String::from("tool-1"),
                        tool_name: String::from("echo"),
                        content: vec![UserContent::Text {
                            text: String::from("done"),
                        }],
                        is_error: false,
                        timestamp: 2,
                    },
                ],
                tools: vec![],
            },
            &StreamOptions::default(),
        );

        let assistant = &request["context"]["messages"][0];
        assert_eq!(assistant["responseId"], json!("resp_1"));
        assert_eq!(assistant["stopReason"], json!("toolUse"));
        assert_eq!(assistant["content"][0]["textSignature"], json!("sig"));
        let tool_result = &request["context"]["messages"][1];
        assert_eq!(tool_result["toolCallId"], json!("tool-1"));
        assert_eq!(tool_result["toolName"], json!("echo"));
        assert_eq!(tool_result["isError"], json!(false));
    }
}
