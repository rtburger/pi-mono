use crate::{AiProvider, AssistantEventStream, StreamOptions, register_provider};
use async_stream::stream;
use futures::StreamExt;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
    UserContent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpenAiResponsesParamsOptions {
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub session_id: Option<String>,
    pub cache_retention: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponsesReasoning {
    pub effort: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponsesRequestParams {
    pub model: String,
    pub input: Vec<ResponsesInputItem>,
    pub stream: bool,
    pub store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_retention: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<OpenAiResponsesReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiResponsesConvertOptions {
    pub include_system_prompt: bool,
}

impl Default for OpenAiResponsesConvertOptions {
    fn default() -> Self {
        Self {
            include_system_prompt: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesInputItem {
    Message {
        role: String,
        content: Vec<ResponsesContentPart>,
    },
    FunctionCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: ResponsesFunctionCallOutput,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesContentPart {
    InputText { text: String },
    InputImage { detail: String, image_url: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesFunctionCallOutput {
    Text(String),
    Content(Vec<ResponsesContentPart>),
}

pub fn build_openai_responses_request_params(
    model: &Model,
    context: &Context,
    allowed_tool_call_providers: &[&str],
    convert_options: OpenAiResponsesConvertOptions,
    options: OpenAiResponsesParamsOptions,
) -> OpenAiResponsesRequestParams {
    let input = convert_openai_responses_messages(
        model,
        context,
        allowed_tool_call_providers,
        convert_options,
    );
    let cache_retention = options.cache_retention.unwrap_or_else(|| "short".into());

    let reasoning = if model.reasoning {
        if options.reasoning_effort.is_some() || options.reasoning_summary.is_some() {
            Some(OpenAiResponsesReasoning {
                effort: options.reasoning_effort.unwrap_or_else(|| "medium".into()),
                summary: options.reasoning_summary.unwrap_or_else(|| "auto".into()),
            })
        } else if model.provider != "github-copilot" {
            Some(OpenAiResponsesReasoning {
                effort: "none".into(),
                summary: "auto".into(),
            })
        } else {
            None
        }
    } else {
        None
    };

    OpenAiResponsesRequestParams {
        model: model.id.clone(),
        input,
        stream: true,
        store: false,
        prompt_cache_key: if cache_retention == "none" {
            None
        } else {
            options.session_id
        },
        prompt_cache_retention: if cache_retention == "long"
            && model.base_url.contains("api.openai.com")
        {
            Some("24h".into())
        } else {
            None
        },
        max_output_tokens: options.max_output_tokens,
        temperature: options.temperature,
        include: reasoning
            .as_ref()
            .map(|_| vec!["reasoning.encrypted_content".into()]),
        reasoning,
    }
}

pub fn convert_openai_responses_messages(
    model: &Model,
    context: &Context,
    allowed_tool_call_providers: &[&str],
    options: OpenAiResponsesConvertOptions,
) -> Vec<ResponsesInputItem> {
    let mut items = Vec::new();
    let allowed_tool_call_providers = allowed_tool_call_providers
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();

    if options.include_system_prompt
        && let Some(system_prompt) = &context.system_prompt
    {
        items.push(ResponsesInputItem::Message {
            role: if model.reasoning {
                "developer".into()
            } else {
                "system".into()
            },
            content: vec![ResponsesContentPart::InputText {
                text: sanitize_surrogates(system_prompt),
            }],
        });
    }

    for message in &context.messages {
        match message {
            Message::User { content, .. } => {
                let content = convert_user_content(content, model);
                if !content.is_empty() {
                    items.push(ResponsesInputItem::Message {
                        role: "user".into(),
                        content,
                    });
                }
            }
            Message::Assistant {
                content,
                provider,
                api,
                ..
            } => {
                for block in content {
                    match block {
                        AssistantContent::Text { text } => {
                            items.push(ResponsesInputItem::Message {
                                role: "assistant".into(),
                                content: vec![ResponsesContentPart::InputText {
                                    text: sanitize_surrogates(text),
                                }],
                            });
                        }
                        AssistantContent::Thinking { .. } => {}
                        AssistantContent::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            let (call_id, item_id) = split_tool_call_id(id);
                            let normalized = normalize_tool_call_id(
                                id,
                                provider != &model.provider || api != &model.api,
                                allowed_tool_call_providers.contains(model.provider.as_str()),
                            );
                            let (normalized_call_id, normalized_item_id) =
                                split_tool_call_id(&normalized);
                            items.push(ResponsesInputItem::FunctionCall {
                                id: if provider == &model.provider && api == &model.api {
                                    Some(
                                        normalized_item_id.map(ToOwned::to_owned).unwrap_or_else(
                                            || item_id.unwrap_or_default().to_string(),
                                        ),
                                    )
                                } else {
                                    normalized_item_id.map(ToOwned::to_owned)
                                },
                                call_id: normalized_call_id.to_string(),
                                name: name.clone(),
                                arguments: serde_json::to_string(arguments)
                                    .unwrap_or_else(|_| "{}".into()),
                            });
                            let _ = call_id;
                        }
                    }
                }
            }
            Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                let (call_id, _) = split_tool_call_id(tool_call_id);
                let text = content
                    .iter()
                    .filter_map(|part| match part {
                        UserContent::Text { text } => Some(text.as_str()),
                        UserContent::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let has_images = content
                    .iter()
                    .any(|part| matches!(part, UserContent::Image { .. }))
                    && model.input.iter().any(|input| input == "image");

                let output = if has_images {
                    let mut parts = Vec::new();
                    if !text.is_empty() {
                        parts.push(ResponsesContentPart::InputText {
                            text: sanitize_surrogates(&text),
                        });
                    }
                    for part in content {
                        if let UserContent::Image { data, mime_type } = part {
                            parts.push(ResponsesContentPart::InputImage {
                                detail: "auto".into(),
                                image_url: format!("data:{mime_type};base64,{data}"),
                            });
                        }
                    }
                    ResponsesFunctionCallOutput::Content(parts)
                } else {
                    ResponsesFunctionCallOutput::Text(sanitize_surrogates(if text.is_empty() {
                        "(see attached image)"
                    } else {
                        &text
                    }))
                };

                items.push(ResponsesInputItem::FunctionCallOutput {
                    call_id: call_id.to_string(),
                    output,
                });
            }
        }
    }

    items
}

fn convert_user_content(content: &[UserContent], model: &Model) -> Vec<ResponsesContentPart> {
    content
        .iter()
        .filter_map(|part| match part {
            UserContent::Text { text } => Some(ResponsesContentPart::InputText {
                text: sanitize_surrogates(text),
            }),
            UserContent::Image { data, mime_type }
                if model.input.iter().any(|input| input == "image") =>
            {
                Some(ResponsesContentPart::InputImage {
                    detail: "auto".into(),
                    image_url: format!("data:{mime_type};base64,{data}"),
                })
            }
            UserContent::Image { .. } => None,
        })
        .collect()
}

pub fn normalize_tool_call_id(
    id: &str,
    is_foreign_tool_call: bool,
    target_provider_supports_openai_tool_ids: bool,
) -> String {
    if !target_provider_supports_openai_tool_ids {
        return normalize_id_part(id);
    }
    if !id.contains('|') {
        return normalize_id_part(id);
    }
    let (call_id, item_id) = split_tool_call_id(id);
    let normalized_call_id = normalize_id_part(call_id);
    let Some(item_id) = item_id else {
        return normalized_call_id;
    };
    let mut normalized_item_id = if is_foreign_tool_call {
        build_foreign_responses_item_id(item_id)
    } else {
        normalize_id_part(item_id)
    };
    if !normalized_item_id.starts_with("fc_") {
        normalized_item_id = normalize_id_part(&format!("fc_{normalized_item_id}"));
    }
    format!("{normalized_call_id}|{normalized_item_id}")
}

fn split_tool_call_id(id: &str) -> (&str, Option<&str>) {
    match id.split_once('|') {
        Some((call_id, item_id)) => (call_id, Some(item_id)),
        None => (id, None),
    }
}

fn normalize_id_part(part: &str) -> String {
    let sanitized: String = part
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => character,
            _ => '_',
        })
        .collect();
    sanitized.trim_end_matches('_').chars().take(64).collect()
}

fn build_foreign_responses_item_id(item_id: &str) -> String {
    let normalized = format!("fc_{}", short_hash(item_id));
    normalized.chars().take(64).collect()
}

fn short_hash(input: &str) -> String {
    let mut h1: u32 = 0xdeadbeef;
    let mut h2: u32 = 0x41c6ce57;
    for character in input.chars() {
        let ch = character as u32;
        h1 = (h1 ^ ch).wrapping_mul(2654435761);
        h2 = (h2 ^ ch).wrapping_mul(1597334677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2246822507) ^ (h2 ^ (h2 >> 13)).wrapping_mul(3266489909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2246822507) ^ (h1 ^ (h1 >> 13)).wrapping_mul(3266489909);
    format!("{:x}{:x}", h2, h1)
}

fn sanitize_surrogates(text: &str) -> String {
    text.to_owned()
}

pub fn tool_call_arguments(arguments: &[(&str, Value)]) -> BTreeMap<String, Value> {
    arguments
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponsesStreamEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub data: serde_json::Map<String, Value>,
}

pub fn parse_openai_responses_sse_text(
    payload: &str,
) -> Result<Vec<OpenAiResponsesStreamEnvelope>, crate::AiError> {
    let mut events = Vec::new();
    let mut current_data_lines = Vec::new();

    for line in payload.lines() {
        if line.is_empty() {
            if let Some(event) = flush_sse_event(&mut current_data_lines)? {
                events.push(event);
            }
            continue;
        }

        if let Some(data) = line.strip_prefix("data:") {
            current_data_lines.push(data.trim_start().to_string());
        }
    }

    if let Some(event) = flush_sse_event(&mut current_data_lines)? {
        events.push(event);
    }

    Ok(events)
}

pub fn stream_openai_responses_sse_text(
    model: Model,
    payload: &str,
) -> Result<AssistantEventStream, crate::AiError> {
    let events = parse_openai_responses_sse_text(payload)?;
    Ok(stream_openai_responses_sse_events(model, events))
}

pub fn stream_openai_responses_http(
    model: Model,
    params: OpenAiResponsesRequestParams,
    api_key: String,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut signal = signal;

        if is_signal_aborted(&signal) {
            yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted_message(&model) });
            return;
        }

        let send_future = reqwest::Client::new()
            .post(format!("{}/responses", model.base_url.trim_end_matches('/')))
            .bearer_auth(api_key)
            .header("accept", "text/event-stream")
            .json(&params)
            .send();
        tokio::pin!(send_future);

        let response = if let Some(signal) = signal.as_mut() {
            tokio::select! {
                response = &mut send_future => {
                    match response {
                        Ok(response) => response,
                        Err(error) => {
                            let message = error_message(&model, format!("HTTP request failed: {error}"));
                            yield Ok(AssistantEvent::Error { reason: StopReason::Error, error: message });
                            return;
                        }
                    }
                }
                _ = wait_for_abort(signal) => {
                    yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted_message(&model) });
                    return;
                }
            }
        } else {
            match send_future.await {
                Ok(response) => response,
                Err(error) => {
                    let message = error_message(&model, format!("HTTP request failed: {error}"));
                    yield Ok(AssistantEvent::Error { reason: StopReason::Error, error: message });
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
                        yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted_message(&model) });
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
            let message = error_message(&model, detail);
            yield Ok(AssistantEvent::Error { reason: StopReason::Error, error: message });
            return;
        }

        let text_future = response.text();
        tokio::pin!(text_future);
        let payload = if let Some(signal) = signal.as_mut() {
            tokio::select! {
                payload = &mut text_future => {
                    match payload {
                        Ok(payload) => payload,
                        Err(error) => {
                            let message = error_message(&model, format!("Failed to read SSE response body: {error}"));
                            yield Ok(AssistantEvent::Error { reason: StopReason::Error, error: message });
                            return;
                        }
                    }
                }
                _ = wait_for_abort(signal) => {
                    yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted_message(&model) });
                    return;
                }
            }
        } else {
            match text_future.await {
                Ok(payload) => payload,
                Err(error) => {
                    let message = error_message(&model, format!("Failed to read SSE response body: {error}"));
                    yield Ok(AssistantEvent::Error { reason: StopReason::Error, error: message });
                    return;
                }
            }
        };

        let inner = match stream_openai_responses_sse_text(model.clone(), &payload) {
            Ok(stream) => stream,
            Err(error) => {
                let message = error_message(&model, error.to_string());
                yield Ok(AssistantEvent::Error { reason: StopReason::Error, error: message });
                return;
            }
        };

        futures::pin_mut!(inner);
        while let Some(event) = inner.next().await {
            match event {
                Ok(event) => yield Ok(event),
                Err(error) => {
                    let message = error_message(&model, error.to_string());
                    yield Ok(AssistantEvent::Error { reason: StopReason::Error, error: message });
                    return;
                }
            }
        }
    })
}

pub fn stream_openai_responses_sse_events(
    model: Model,
    events: Vec<OpenAiResponsesStreamEnvelope>,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut output = AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
        output.timestamp = now_ms();
        yield Ok(AssistantEvent::Start { partial: output.clone() });

        let mut current_block_index: Option<usize> = None;
        let mut current_tool_json = String::new();

        for event in events {
            match event.event_type.as_str() {
                "response.created" => {
                    output.response_id = event
                        .data
                        .get("response")
                        .and_then(Value::as_object)
                        .and_then(|response| response.get("id"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                }
                "response.output_item.added" => {
                    let item = event.data.get("item").and_then(Value::as_object).cloned().unwrap_or_default();
                    match item.get("type").and_then(Value::as_str) {
                        Some("message") => {
                            output.content.push(AssistantContent::Text { text: String::new() });
                            current_block_index = Some(output.content.len() - 1);
                            yield Ok(AssistantEvent::TextStart {
                                content_index: output.content.len() - 1,
                                partial: output.clone(),
                            });
                        }
                        Some("function_call") => {
                            let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                            let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or_default();
                            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                            current_tool_json = item.get("arguments").and_then(Value::as_str).unwrap_or_default().to_string();
                            output.content.push(AssistantContent::ToolCall {
                                id: format!("{call_id}|{id}"),
                                name: name.to_string(),
                                arguments: BTreeMap::new(),
                            });
                            current_block_index = Some(output.content.len() - 1);
                            yield Ok(AssistantEvent::ToolCallStart {
                                content_index: output.content.len() - 1,
                                partial: output.clone(),
                            });
                        }
                        Some("reasoning") => {
                            output.content.push(AssistantContent::Thinking { thinking: String::new() });
                            current_block_index = Some(output.content.len() - 1);
                            yield Ok(AssistantEvent::ThinkingStart {
                                content_index: output.content.len() - 1,
                                partial: output.clone(),
                            });
                        }
                        _ => {}
                    }
                }
                "response.output_text.delta" => {
                    if let Some(index) = current_block_index
                        && let Some(AssistantContent::Text { text }) = output.content.get_mut(index) {
                        let delta = event.data.get("delta").and_then(Value::as_str).unwrap_or_default().to_string();
                        text.push_str(&delta);
                        yield Ok(AssistantEvent::TextDelta {
                            content_index: index,
                            delta,
                            partial: output.clone(),
                        });
                    }
                }
                "response.function_call_arguments.delta" => {
                    if let Some(index) = current_block_index {
                        let delta = event.data.get("delta").and_then(Value::as_str).unwrap_or_default().to_string();
                        current_tool_json.push_str(&delta);
                        if let Some(AssistantContent::ToolCall { arguments, .. }) = output.content.get_mut(index) {
                            *arguments = parse_streaming_json_map(&current_tool_json);
                        }
                        yield Ok(AssistantEvent::ToolCallDelta {
                            content_index: index,
                            delta,
                            partial: output.clone(),
                        });
                    }
                }
                "response.function_call_arguments.done" => {
                    if let Some(index) = current_block_index {
                        let full = event.data.get("arguments").and_then(Value::as_str).unwrap_or_default().to_string();
                        if full.starts_with(&current_tool_json) {
                            let delta = full[current_tool_json.len()..].to_string();
                            if !delta.is_empty() {
                                current_tool_json = full.clone();
                                if let Some(AssistantContent::ToolCall { arguments, .. }) = output.content.get_mut(index) {
                                    *arguments = parse_streaming_json_map(&current_tool_json);
                                }
                                yield Ok(AssistantEvent::ToolCallDelta {
                                    content_index: index,
                                    delta,
                                    partial: output.clone(),
                                });
                            }
                        }
                    }
                }
                "response.output_item.done" => {
                    let item = event.data.get("item").and_then(Value::as_object).cloned().unwrap_or_default();
                    match item.get("type").and_then(Value::as_str) {
                        Some("message") => {
                            if let Some(index) = current_block_index
                                && let Some(AssistantContent::Text { text }) = output.content.get(index) {
                                yield Ok(AssistantEvent::TextEnd {
                                    content_index: index,
                                    content: text.clone(),
                                    partial: output.clone(),
                                });
                                current_block_index = None;
                            }
                        }
                        Some("function_call") => {
                            if let Some(index) = current_block_index {
                                let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or_default();
                                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                                let arguments_text = item.get("arguments").and_then(Value::as_str).unwrap_or(&current_tool_json);
                                let tool_call = AssistantContent::ToolCall {
                                    id: format!("{call_id}|{id}"),
                                    name: name.to_string(),
                                    arguments: parse_streaming_json_map(arguments_text),
                                };
                                output.content[index] = tool_call.clone();
                                yield Ok(AssistantEvent::ToolCallEnd {
                                    content_index: index,
                                    tool_call,
                                    partial: output.clone(),
                                });
                                current_block_index = None;
                            }
                        }
                        Some("reasoning") => {
                            if let Some(index) = current_block_index
                                && let Some(AssistantContent::Thinking { thinking }) = output.content.get(index) {
                                yield Ok(AssistantEvent::ThinkingEnd {
                                    content_index: index,
                                    content: thinking.clone(),
                                    partial: output.clone(),
                                });
                                current_block_index = None;
                            }
                        }
                        _ => {}
                    }
                }
                "response.completed" => {
                    let response = event.data.get("response").and_then(Value::as_object).cloned().unwrap_or_default();
                    output.response_id = response.get("id").and_then(Value::as_str).map(ToOwned::to_owned).or(output.response_id.clone());
                    apply_usage_from_response(&mut output, &response);
                    output.stop_reason = map_response_status(response.get("status").and_then(Value::as_str));
                    if output.content.iter().any(|block| matches!(block, AssistantContent::ToolCall { .. }))
                        && output.stop_reason == StopReason::Stop {
                        output.stop_reason = StopReason::ToolUse;
                    }
                    yield Ok(AssistantEvent::Done {
                        reason: output.stop_reason.clone(),
                        message: output.clone(),
                    });
                    return;
                }
                "response.failed" => {
                    let response = event.data.get("response").and_then(Value::as_object).cloned().unwrap_or_default();
                    output.response_id = response.get("id").and_then(Value::as_str).map(ToOwned::to_owned).or(output.response_id.clone());
                    apply_usage_from_response(&mut output, &response);
                    let error_message = response
                        .get("error")
                        .and_then(Value::as_object)
                        .map(|error| {
                            let code = error.get("code").and_then(Value::as_str).unwrap_or("unknown");
                            let message = error.get("message").and_then(Value::as_str).unwrap_or("no message");
                            format!("{code}: {message}")
                        })
                        .or_else(|| {
                            response
                                .get("incomplete_details")
                                .and_then(Value::as_object)
                                .and_then(|details| details.get("reason"))
                                .and_then(Value::as_str)
                                .map(|reason| format!("incomplete: {reason}"))
                        })
                        .unwrap_or_else(|| "Unknown error (no error details in response)".into());
                    output.stop_reason = StopReason::Error;
                    output.error_message = Some(error_message);
                    yield Ok(AssistantEvent::Error {
                        reason: StopReason::Error,
                        error: output.clone(),
                    });
                    return;
                }
                "error" => {
                    let code = event.data.get("code").and_then(Value::as_str).unwrap_or("unknown");
                    let message = event.data.get("message").and_then(Value::as_str).unwrap_or("Unknown error");
                    output.stop_reason = StopReason::Error;
                    output.error_message = Some(format!("Error Code {code}: {message}"));
                    yield Ok(AssistantEvent::Error {
                        reason: StopReason::Error,
                        error: output.clone(),
                    });
                    return;
                }
                _ => {}
            }
        }
    })
}

fn error_message(model: &Model, error_message: String) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content: Vec::new(),
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_id: None,
        usage: Usage::default(),
        stop_reason: StopReason::Error,
        error_message: Some(error_message),
        timestamp: now_ms(),
    }
}

fn aborted_message(model: &Model) -> AssistantMessage {
    AssistantMessage {
        stop_reason: StopReason::Aborted,
        error_message: Some("Request was aborted".into()),
        ..AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone())
    }
}

fn is_signal_aborted(signal: &Option<tokio::sync::watch::Receiver<bool>>) -> bool {
    signal
        .as_ref()
        .map(|signal| *signal.borrow())
        .unwrap_or(false)
}

async fn wait_for_abort(signal: &mut tokio::sync::watch::Receiver<bool>) {
    while !*signal.borrow() {
        if signal.changed().await.is_err() {
            return;
        }
    }
}

fn flush_sse_event(
    current_data_lines: &mut Vec<String>,
) -> Result<Option<OpenAiResponsesStreamEnvelope>, crate::AiError> {
    if current_data_lines.is_empty() {
        return Ok(None);
    }

    let payload = current_data_lines.join("\n");
    current_data_lines.clear();

    if payload == "[DONE]" {
        return Ok(None);
    }

    serde_json::from_str::<OpenAiResponsesStreamEnvelope>(&payload)
        .map(Some)
        .map_err(|error| {
            crate::AiError::Message(format!("Invalid OpenAI Responses SSE event: {error}"))
        })
}

fn parse_streaming_json_map(input: &str) -> BTreeMap<String, Value> {
    serde_json::from_str::<BTreeMap<String, Value>>(input).unwrap_or_default()
}

fn apply_usage_from_response(
    output: &mut AssistantMessage,
    response: &serde_json::Map<String, Value>,
) {
    let usage = response
        .get("usage")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let cached = usage
        .get("input_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    output.usage = Usage {
        input: input_tokens.saturating_sub(cached),
        output: output_tokens,
        cache_read: cached,
        cache_write: 0,
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(input_tokens + output_tokens),
        ..Usage::default()
    };
}

fn map_response_status(status: Option<&str>) -> StopReason {
    match status.unwrap_or("completed") {
        "completed" => StopReason::Stop,
        "incomplete" => StopReason::Length,
        "failed" | "cancelled" => StopReason::Error,
        "in_progress" | "queued" => StopReason::Stop,
        _ => StopReason::Error,
    }
}

#[derive(Default)]
pub struct OpenAiResponsesProvider;

impl AiProvider for OpenAiResponsesProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        let params = build_openai_responses_request_params(
            &model,
            &context,
            &["openai", "openai-codex", "opencode"],
            OpenAiResponsesConvertOptions::default(),
            OpenAiResponsesParamsOptions {
                max_output_tokens: options.max_tokens,
                temperature: options.temperature,
                reasoning_effort: options.reasoning_effort.clone(),
                reasoning_summary: options.reasoning_summary.clone(),
                session_id: options.session_id.clone(),
                cache_retention: Some(match options.cache_retention {
                    crate::CacheRetention::None => "none".into(),
                    crate::CacheRetention::Short => "short".into(),
                    crate::CacheRetention::Long => "long".into(),
                }),
            },
        );

        let api_key = options
            .api_key
            .clone()
            .or_else(|| crate::get_env_api_key(&model.provider));

        match api_key {
            Some(api_key) => {
                stream_openai_responses_http(model, params, api_key, options.signal.clone())
            }
            None => Box::pin(stream! {
                yield Ok(AssistantEvent::Error {
                    reason: StopReason::Error,
                    error: error_message(&model, "OpenAI Responses API key is required".into()),
                });
            }),
        }
    }
}

pub fn register_openai_responses_provider() {
    register_provider("openai-responses", Arc::new(OpenAiResponsesProvider));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
