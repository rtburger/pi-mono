use crate::{
    AiProvider, AssistantEventStream, StreamOptions,
    models::{calculate_cost_with, get_model_headers, get_provider_headers},
    register_provider,
};
use async_stream::stream;
use futures::StreamExt;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, ModelCost,
    StopReason, ToolDefinition, Usage, UserContent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const OPENAI_RESPONSES_ALLOWED_TOOL_CALL_PROVIDERS: &[&str] =
    &["openai", "openai-codex", "opencode"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiResponsesServiceTier {
    Auto,
    Default,
    Flex,
    Scale,
    Priority,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpenAiResponsesParamsOptions {
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub session_id: Option<String>,
    pub cache_retention: Option<String>,
    pub service_tier: Option<OpenAiResponsesServiceTier>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponsesToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<OpenAiResponsesServiceTier>,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    Reasoning {
        #[serde(flatten)]
        data: serde_json::Map<String, Value>,
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
    InputText {
        text: String,
    },
    InputImage {
        detail: String,
        image_url: String,
    },
    OutputText {
        text: String,
        annotations: Vec<Value>,
    },
    Refusal {
        refusal: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesFunctionCallOutput {
    Text(String),
    Content(Vec<ResponsesContentPart>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesToolDefinition {
    Function {
        name: String,
        description: String,
        parameters: Value,
        strict: bool,
    },
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
    let cache_retention = resolve_cache_retention(options.cache_retention.as_deref());

    let reasoning = if model.reasoning {
        if options.reasoning_effort.is_some() || options.reasoning_summary.is_some() {
            Some(OpenAiResponsesReasoning {
                effort: options.reasoning_effort.unwrap_or_else(|| "medium".into()),
                summary: options.reasoning_summary.unwrap_or_else(|| "auto".into()),
            })
        } else {
            Some(OpenAiResponsesReasoning {
                effort: "none".into(),
                summary: "auto".into(),
            })
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
        tools: if context.tools.is_empty() {
            None
        } else {
            Some(convert_openai_responses_tools(&context.tools))
        },
        service_tier: options.service_tier,
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
        .collect::<BTreeSet<_>>();
    let transformed_messages = transform_messages_for_openai_responses(
        model,
        &context.messages,
        allowed_tool_call_providers.contains(model.provider.as_str()),
    );

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
            status: None,
            id: None,
            phase: None,
        });
    }

    for (message_index, message) in transformed_messages.iter().enumerate() {
        match message {
            Message::User { content, .. } => {
                let content = convert_user_content(content, model);
                if !content.is_empty() {
                    items.push(ResponsesInputItem::Message {
                        role: "user".into(),
                        content,
                        status: None,
                        id: None,
                        phase: None,
                    });
                }
            }
            Message::Assistant {
                content,
                provider,
                api,
                model: source_model,
                ..
            } => {
                let is_same_provider_and_api = provider == &model.provider && api == &model.api;
                let is_different_model = is_same_provider_and_api && source_model != &model.id;

                for block in content {
                    match block {
                        AssistantContent::Text {
                            text,
                            text_signature,
                        } => {
                            let parsed_signature = parse_text_signature(text_signature.as_deref());
                            let message_id = parsed_signature
                                .as_ref()
                                .map(|signature| normalize_message_item_id(&signature.id))
                                .unwrap_or_else(|| format!("msg_{message_index}"));
                            items.push(ResponsesInputItem::Message {
                                role: "assistant".into(),
                                content: vec![ResponsesContentPart::OutputText {
                                    text: sanitize_surrogates(text),
                                    annotations: Vec::new(),
                                }],
                                status: Some("completed".into()),
                                id: Some(message_id),
                                phase: parsed_signature.and_then(|signature| signature.phase),
                            });
                        }
                        AssistantContent::Thinking {
                            thinking_signature, ..
                        } => {
                            if let Some(thinking_signature) = thinking_signature {
                                if let Ok(reasoning_item) =
                                    serde_json::from_str::<ResponsesInputItem>(thinking_signature)
                                {
                                    items.push(reasoning_item);
                                }
                            }
                        }
                        AssistantContent::ToolCall {
                            id,
                            name,
                            arguments,
                            ..
                        } => {
                            let (call_id, item_id) = split_tool_call_id(id);
                            let item_id = if is_different_model
                                && item_id.is_some_and(|item_id| item_id.starts_with("fc_"))
                            {
                                None
                            } else {
                                item_id.map(ToOwned::to_owned)
                            };

                            items.push(ResponsesInputItem::FunctionCall {
                                id: item_id,
                                call_id: call_id.to_string(),
                                name: name.clone(),
                                arguments: serde_json::to_string(arguments)
                                    .unwrap_or_else(|_| "{}".into()),
                            });
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

fn transform_messages_for_openai_responses(
    model: &Model,
    messages: &[Message],
    target_provider_supports_openai_tool_ids: bool,
) -> Vec<Message> {
    let mut tool_call_id_map = BTreeMap::<String, String>::new();
    let mut transformed = Vec::new();

    for message in messages.iter().cloned() {
        match message {
            user_message @ Message::User { .. } => transformed.push(user_message),
            Message::ToolResult {
                tool_call_id,
                tool_name,
                content,
                details,
                is_error,
                timestamp,
            } => {
                let normalized_tool_call_id = tool_call_id_map
                    .get(&tool_call_id)
                    .cloned()
                    .unwrap_or(tool_call_id);
                transformed.push(Message::ToolResult {
                    tool_call_id: normalized_tool_call_id,
                    tool_name,
                    content,
                    details,
                    is_error,
                    timestamp,
                });
            }
            Message::Assistant {
                content,
                api,
                provider,
                model: source_model,
                response_id,
                usage,
                stop_reason,
                error_message,
                timestamp,
            } => {
                let is_same_model = provider == model.provider.as_str()
                    && api == model.api.as_str()
                    && source_model == model.id.as_str();
                let mut transformed_content = Vec::new();

                for block in content {
                    match block {
                        AssistantContent::Text {
                            text,
                            text_signature,
                        } => {
                            transformed_content.push(AssistantContent::Text {
                                text,
                                text_signature,
                            });
                        }
                        AssistantContent::Thinking {
                            thinking,
                            thinking_signature,
                            redacted,
                        } => {
                            if is_same_model {
                                transformed_content.push(AssistantContent::Thinking {
                                    thinking,
                                    thinking_signature,
                                    redacted,
                                });
                            } else if !thinking.trim().is_empty() {
                                transformed_content.push(AssistantContent::Text {
                                    text: thinking,
                                    text_signature: None,
                                });
                            }
                        }
                        AssistantContent::ToolCall {
                            id,
                            name,
                            arguments,
                            thought_signature,
                        } => {
                            let normalized_id = if is_same_model {
                                id.clone()
                            } else {
                                normalize_tool_call_id(
                                    &id,
                                    provider != model.provider.as_str()
                                        || api != model.api.as_str(),
                                    target_provider_supports_openai_tool_ids,
                                )
                            };
                            if normalized_id != id {
                                tool_call_id_map.insert(id.clone(), normalized_id.clone());
                            }
                            transformed_content.push(AssistantContent::ToolCall {
                                id: normalized_id,
                                name,
                                arguments,
                                thought_signature: if is_same_model {
                                    thought_signature
                                } else {
                                    None
                                },
                            });
                        }
                    }
                }

                transformed.push(Message::Assistant {
                    content: transformed_content,
                    api,
                    provider,
                    model: source_model,
                    response_id,
                    usage,
                    stop_reason,
                    error_message,
                    timestamp,
                });
            }
        }
    }

    let mut result = Vec::new();
    let mut pending_tool_calls = Vec::<(String, String)>::new();
    let mut existing_tool_result_ids = BTreeSet::<String>::new();

    for message in transformed {
        match &message {
            Message::Assistant {
                content,
                stop_reason,
                ..
            } => {
                flush_orphaned_tool_calls(
                    &mut result,
                    &mut pending_tool_calls,
                    &mut existing_tool_result_ids,
                );

                if matches!(stop_reason, StopReason::Error | StopReason::Aborted) {
                    continue;
                }

                pending_tool_calls = content
                    .iter()
                    .filter_map(|block| match block {
                        AssistantContent::ToolCall { id, name, .. } => {
                            Some((id.clone(), name.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                existing_tool_result_ids.clear();
                result.push(message);
            }
            Message::ToolResult { tool_call_id, .. } => {
                existing_tool_result_ids.insert(tool_call_id.clone());
                result.push(message);
            }
            Message::User { .. } => {
                flush_orphaned_tool_calls(
                    &mut result,
                    &mut pending_tool_calls,
                    &mut existing_tool_result_ids,
                );
                result.push(message);
            }
        }
    }

    result
}

fn flush_orphaned_tool_calls(
    result: &mut Vec<Message>,
    pending_tool_calls: &mut Vec<(String, String)>,
    existing_tool_result_ids: &mut BTreeSet<String>,
) {
    for (tool_call_id, tool_name) in pending_tool_calls.iter() {
        if !existing_tool_result_ids.contains(tool_call_id) {
            result.push(Message::ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                content: vec![UserContent::Text {
                    text: "No result provided".into(),
                }],
                details: None,
                is_error: true,
                timestamp: now_ms(),
            });
        }
    }
    pending_tool_calls.clear();
    existing_tool_result_ids.clear();
}

fn convert_openai_responses_tools(tools: &[ToolDefinition]) -> Vec<ResponsesToolDefinition> {
    tools
        .iter()
        .map(|tool| ResponsesToolDefinition::Function {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
            strict: false,
        })
        .collect()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TextSignatureV1 {
    v: u8,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedTextSignature {
    id: String,
    phase: Option<String>,
}

fn encode_text_signature_v1(id: &str, phase: Option<&str>) -> String {
    serde_json::to_string(&TextSignatureV1 {
        v: 1,
        id: id.to_string(),
        phase: phase.map(ToOwned::to_owned),
    })
    .unwrap_or_else(|_| id.to_string())
}

fn parse_text_signature(signature: Option<&str>) -> Option<ParsedTextSignature> {
    let signature = signature?;
    if signature.starts_with('{')
        && let Ok(parsed) = serde_json::from_str::<TextSignatureV1>(signature)
    {
        if parsed.v == 1 {
            return Some(ParsedTextSignature {
                id: parsed.id,
                phase: parsed.phase,
            });
        }
    }
    Some(ParsedTextSignature {
        id: signature.to_string(),
        phase: None,
    })
}

fn normalize_message_item_id(id: &str) -> String {
    if id.chars().count() > 64 {
        format!("msg_{}", short_hash(id))
    } else {
        id.to_string()
    }
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
    for code_unit in input.encode_utf16() {
        let ch = u32::from(code_unit);
        h1 = (h1 ^ ch).wrapping_mul(2654435761);
        h2 = (h2 ^ ch).wrapping_mul(1597334677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2246822507) ^ (h2 ^ (h2 >> 13)).wrapping_mul(3266489909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2246822507) ^ (h1 ^ (h1 >> 13)).wrapping_mul(3266489909);
    format!("{}{}", to_base36(h2), to_base36(h1))
}

fn to_base36(mut value: u32) -> String {
    if value == 0 {
        return "0".into();
    }

    let mut digits = Vec::new();
    while value > 0 {
        let digit = (value % 36) as u8;
        digits.push(match digit {
            0..=9 => (b'0' + digit) as char,
            _ => (b'a' + (digit - 10)) as char,
        });
        value /= 36;
    }

    digits.into_iter().rev().collect()
}

fn sanitize_surrogates(text: &str) -> String {
    crate::unicode::sanitize_provider_text(text)
}

fn resolve_cache_retention(cache_retention: Option<&str>) -> String {
    match cache_retention {
        Some(cache_retention) => cache_retention.to_string(),
        None => {
            if env::var("PI_CACHE_RETENTION").ok().as_deref() == Some("long") {
                "long".into()
            } else {
                "short".into()
            }
        }
    }
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

#[derive(Default)]
pub(crate) struct OpenAiResponsesSseDecoder {
    buffer: Vec<u8>,
    current_data_lines: Vec<String>,
}

impl OpenAiResponsesSseDecoder {
    pub(crate) fn push_bytes(
        &mut self,
        chunk: &[u8],
    ) -> Result<Vec<OpenAiResponsesStreamEnvelope>, crate::AiError> {
        self.buffer.extend_from_slice(chunk);

        let mut events = Vec::new();
        let mut line_start = 0usize;
        let mut consumed = 0usize;

        while let Some(relative_newline) = self.buffer[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let newline = line_start + relative_newline;
            let line = self.buffer[line_start..newline].to_vec();
            self.process_line(&line, &mut events)?;
            line_start = newline + 1;
            consumed = line_start;
        }

        if consumed > 0 {
            self.buffer.drain(..consumed);
        }

        Ok(events)
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<OpenAiResponsesStreamEnvelope>, crate::AiError> {
        let mut events = Vec::new();

        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.process_line(&line, &mut events)?;
        }

        if let Some(event) = flush_sse_event(&mut self.current_data_lines)? {
            events.push(event);
        }

        Ok(events)
    }

    fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<OpenAiResponsesStreamEnvelope>,
    ) -> Result<(), crate::AiError> {
        let line = match line.strip_suffix(b"\r") {
            Some(stripped) => stripped,
            None => line,
        };
        let line = std::str::from_utf8(line).map_err(|error| {
            crate::AiError::Message(format!(
                "Invalid UTF-8 in OpenAI Responses SSE stream: {error}"
            ))
        })?;

        if line.is_empty() {
            if let Some(event) = flush_sse_event(&mut self.current_data_lines)? {
                events.push(event);
            }
            return Ok(());
        }

        if let Some(data) = line.strip_prefix("data:") {
            self.current_data_lines.push(data.trim_start().to_string());
        }

        Ok(())
    }
}

pub fn parse_openai_responses_sse_text(
    payload: &str,
) -> Result<Vec<OpenAiResponsesStreamEnvelope>, crate::AiError> {
    let mut decoder = OpenAiResponsesSseDecoder::default();
    let mut events = decoder.push_bytes(payload.as_bytes())?;
    events.extend(decoder.finish()?);
    Ok(events)
}

pub fn stream_openai_responses_sse_text(
    model: Model,
    payload: &str,
) -> Result<AssistantEventStream, crate::AiError> {
    let events = parse_openai_responses_sse_text(payload)?;
    Ok(stream_openai_responses_sse_events(model, events))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenAiResponsesBlockKind {
    Text,
    Thinking,
    ToolCall,
}

#[derive(Debug, Clone)]
pub(crate) struct OpenAiResponsesStreamState {
    output: AssistantMessage,
    model_cost: ModelCost,
    requested_service_tier: Option<OpenAiResponsesServiceTier>,
    current_block_index: Option<usize>,
    current_block_kind: Option<OpenAiResponsesBlockKind>,
    current_tool_json: String,
}

impl OpenAiResponsesStreamState {
    pub(crate) fn new(model: &Model) -> Self {
        Self::with_requested_service_tier(model, None)
    }

    pub(crate) fn with_requested_service_tier(
        model: &Model,
        requested_service_tier: Option<OpenAiResponsesServiceTier>,
    ) -> Self {
        let mut output =
            AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
        output.timestamp = now_ms();
        Self {
            output,
            model_cost: model.cost,
            requested_service_tier,
            current_block_index: None,
            current_block_kind: None,
            current_tool_json: String::new(),
        }
    }

    pub(crate) fn start_event(&self) -> AssistantEvent {
        AssistantEvent::Start {
            partial: self.output.clone(),
        }
    }

    pub(crate) fn aborted_event(&self) -> AssistantEvent {
        let mut error = self.output.clone();
        error.stop_reason = StopReason::Aborted;
        error.error_message = Some("Request was aborted".into());
        AssistantEvent::Error {
            reason: StopReason::Aborted,
            error,
        }
    }

    pub(crate) fn error_event(&self, error_message: impl Into<String>) -> AssistantEvent {
        let mut error = self.output.clone();
        error.stop_reason = StopReason::Error;
        error.error_message = Some(error_message.into());
        AssistantEvent::Error {
            reason: StopReason::Error,
            error,
        }
    }

    pub(crate) fn process_event(
        &mut self,
        event: &OpenAiResponsesStreamEnvelope,
    ) -> Vec<AssistantEvent> {
        let mut emitted = Vec::new();

        match event.event_type.as_str() {
            "response.created" => self.handle_response_created(event),
            "response.output_item.added" => {
                emitted = self.handle_response_output_item_added(event);
            }
            "response.reasoning_summary_part.added" => {
                self.handle_response_reasoning_summary_part_added(event);
            }
            "response.reasoning_summary_text.delta" => {
                emitted = self.handle_response_reasoning_summary_text_delta(event);
            }
            "response.reasoning_summary_part.done" => {
                emitted = self.handle_response_reasoning_summary_part_done(event);
            }
            "response.content_part.added" => {
                self.handle_response_content_part_added(event);
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                if self.current_block_kind == Some(OpenAiResponsesBlockKind::Text)
                    && let Some(index) = self.current_block_index
                {
                    let delta = event
                        .data
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(AssistantContent::Text { text, .. }) =
                        self.output.content.get_mut(index)
                    {
                        text.push_str(&delta);
                    }
                    emitted.push(AssistantEvent::TextDelta {
                        content_index: index,
                        delta,
                        partial: self.output.clone(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                if self.current_block_kind == Some(OpenAiResponsesBlockKind::ToolCall)
                    && let Some(index) = self.current_block_index
                {
                    let delta = event
                        .data
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.current_tool_json.push_str(&delta);
                    if let Some(AssistantContent::ToolCall { arguments, .. }) =
                        self.output.content.get_mut(index)
                    {
                        *arguments = parse_streaming_json_map(&self.current_tool_json);
                    }
                    emitted.push(AssistantEvent::ToolCallDelta {
                        content_index: index,
                        delta,
                        partial: self.output.clone(),
                    });
                }
            }
            "response.function_call_arguments.done" => {
                if self.current_block_kind == Some(OpenAiResponsesBlockKind::ToolCall)
                    && let Some(index) = self.current_block_index
                {
                    let full = event
                        .data
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let previous = self.current_tool_json.clone();
                    self.current_tool_json = full.clone();
                    if let Some(AssistantContent::ToolCall { arguments, .. }) =
                        self.output.content.get_mut(index)
                    {
                        *arguments = parse_streaming_json_map(&self.current_tool_json);
                    }
                    if full.starts_with(&previous) {
                        let delta = full[previous.len()..].to_string();
                        if !delta.is_empty() {
                            emitted.push(AssistantEvent::ToolCallDelta {
                                content_index: index,
                                delta,
                                partial: self.output.clone(),
                            });
                        }
                    }
                }
            }
            "response.output_item.done" => {
                let item = event
                    .data
                    .get("item")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                match item.get("type").and_then(Value::as_str) {
                    Some("message")
                        if self.current_block_kind == Some(OpenAiResponsesBlockKind::Text) =>
                    {
                        if let Some(index) = self.current_block_index {
                            let content = message_item_text(&item)
                                .or_else(|| text_content(&self.output, index))
                                .unwrap_or_default();
                            if let Some(AssistantContent::Text {
                                text,
                                text_signature,
                            }) = self.output.content.get_mut(index)
                            {
                                *text = content.clone();
                                *text_signature =
                                    item.get("id").and_then(Value::as_str).map(|id| {
                                        encode_text_signature_v1(
                                            id,
                                            item.get("phase").and_then(Value::as_str),
                                        )
                                    });
                            }
                            emitted.push(AssistantEvent::TextEnd {
                                content_index: index,
                                content,
                                partial: self.output.clone(),
                            });
                            self.reset_current_block();
                        }
                    }
                    Some("function_call")
                        if self.current_block_kind == Some(OpenAiResponsesBlockKind::ToolCall) =>
                    {
                        if let Some(index) = self.current_block_index {
                            let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                            let call_id = item
                                .get("call_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                            let arguments = item
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or(self.current_tool_json.as_str());
                            let tool_call = AssistantContent::ToolCall {
                                id: format!("{call_id}|{id}"),
                                name: name.to_string(),
                                arguments: parse_streaming_json_map(arguments),
                                thought_signature: None,
                            };
                            self.output.content[index] = tool_call.clone();
                            emitted.push(AssistantEvent::ToolCallEnd {
                                content_index: index,
                                tool_call,
                                partial: self.output.clone(),
                            });
                            self.reset_current_block();
                        }
                    }
                    Some("reasoning")
                        if self.current_block_kind == Some(OpenAiResponsesBlockKind::Thinking) =>
                    {
                        if let Some(index) = self.current_block_index {
                            let content = reasoning_summary_text(&item)
                                .or_else(|| thinking_content(&self.output, index))
                                .unwrap_or_default();
                            if let Some(AssistantContent::Thinking {
                                thinking,
                                thinking_signature,
                                ..
                            }) = self.output.content.get_mut(index)
                            {
                                *thinking = content.clone();
                                *thinking_signature = Some(Value::Object(item.clone()).to_string());
                            }
                            emitted.push(AssistantEvent::ThinkingEnd {
                                content_index: index,
                                content,
                                partial: self.output.clone(),
                            });
                            self.reset_current_block();
                        }
                    }
                    _ => {}
                }
            }
            "response.completed" => {
                let response = event
                    .data
                    .get("response")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                self.output.response_id = response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(self.output.response_id.clone());
                apply_usage_from_response(&mut self.output, &response, self.model_cost);
                apply_service_tier_pricing(
                    &mut self.output.usage,
                    response_service_tier(&response).or(self.requested_service_tier),
                );
                self.output.stop_reason =
                    map_response_status(response.get("status").and_then(Value::as_str));
                if self
                    .output
                    .content
                    .iter()
                    .any(|block| matches!(block, AssistantContent::ToolCall { .. }))
                    && self.output.stop_reason == StopReason::Stop
                {
                    self.output.stop_reason = StopReason::ToolUse;
                }
                emitted.push(AssistantEvent::Done {
                    reason: self.output.stop_reason.clone(),
                    message: self.output.clone(),
                });
            }
            "response.failed" => {
                let response = event
                    .data
                    .get("response")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                self.output.response_id = response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or(self.output.response_id.clone());
                apply_usage_from_response(&mut self.output, &response, self.model_cost);
                self.output.stop_reason = StopReason::Error;
                self.output.error_message = Some(
                    response
                        .get("error")
                        .and_then(Value::as_object)
                        .map(|error| {
                            let code = error
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown");
                            let message = error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("no message");
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
                        .unwrap_or_else(|| "Unknown error (no error details in response)".into()),
                );
                emitted.push(AssistantEvent::Error {
                    reason: StopReason::Error,
                    error: self.output.clone(),
                });
            }
            "error" => {
                self.output.stop_reason = StopReason::Error;
                self.output.error_message = Some(format!(
                    "Error Code {}: {}",
                    event
                        .data
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    event
                        .data
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown error")
                ));
                emitted.push(AssistantEvent::Error {
                    reason: StopReason::Error,
                    error: self.output.clone(),
                });
            }
            _ => {}
        }

        emitted
    }

    fn handle_response_created(&mut self, event: &OpenAiResponsesStreamEnvelope) {
        self.output.response_id = event
            .data
            .get("response")
            .and_then(Value::as_object)
            .and_then(|response| response.get("id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }

    fn handle_response_output_item_added(
        &mut self,
        event: &OpenAiResponsesStreamEnvelope,
    ) -> Vec<AssistantEvent> {
        let mut emitted = Vec::new();
        let item = event
            .data
            .get("item")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                self.output.content.push(AssistantContent::Text {
                    text: String::new(),
                    text_signature: None,
                });
                let index = self.output.content.len() - 1;
                self.current_block_index = Some(index);
                self.current_block_kind = Some(OpenAiResponsesBlockKind::Text);
                self.current_tool_json.clear();
                emitted.push(AssistantEvent::TextStart {
                    content_index: index,
                    partial: self.output.clone(),
                });
            }
            Some("function_call") => {
                let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                self.current_tool_json = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.output.content.push(AssistantContent::ToolCall {
                    id: format!("{call_id}|{id}"),
                    name: name.to_string(),
                    arguments: BTreeMap::new(),
                    thought_signature: None,
                });
                let index = self.output.content.len() - 1;
                self.current_block_index = Some(index);
                self.current_block_kind = Some(OpenAiResponsesBlockKind::ToolCall);
                emitted.push(AssistantEvent::ToolCallStart {
                    content_index: index,
                    partial: self.output.clone(),
                });
            }
            Some("reasoning") => {
                self.output.content.push(AssistantContent::Thinking {
                    thinking: String::new(),
                    thinking_signature: None,
                    redacted: false,
                });
                let index = self.output.content.len() - 1;
                self.current_block_index = Some(index);
                self.current_block_kind = Some(OpenAiResponsesBlockKind::Thinking);
                self.current_tool_json.clear();
                emitted.push(AssistantEvent::ThinkingStart {
                    content_index: index,
                    partial: self.output.clone(),
                });
            }
            _ => {}
        }

        emitted
    }

    fn handle_response_reasoning_summary_text_delta(
        &mut self,
        event: &OpenAiResponsesStreamEnvelope,
    ) -> Vec<AssistantEvent> {
        let mut emitted = Vec::new();

        if self.current_block_kind == Some(OpenAiResponsesBlockKind::Thinking)
            && let Some(index) = self.current_block_index
        {
            let delta = event
                .data
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(AssistantContent::Thinking { thinking, .. }) =
                self.output.content.get_mut(index)
            {
                thinking.push_str(&delta);
            }
            emitted.push(AssistantEvent::ThinkingDelta {
                content_index: index,
                delta,
                partial: self.output.clone(),
            });
        }

        emitted
    }

    fn handle_response_reasoning_summary_part_done(
        &mut self,
        _event: &OpenAiResponsesStreamEnvelope,
    ) -> Vec<AssistantEvent> {
        let mut emitted = Vec::new();

        if self.current_block_kind == Some(OpenAiResponsesBlockKind::Thinking)
            && let Some(index) = self.current_block_index
        {
            let delta = "\n\n".to_string();
            if let Some(AssistantContent::Thinking { thinking, .. }) =
                self.output.content.get_mut(index)
            {
                thinking.push_str(&delta);
            }
            emitted.push(AssistantEvent::ThinkingDelta {
                content_index: index,
                delta,
                partial: self.output.clone(),
            });
        }

        emitted
    }

    #[allow(
        clippy::missing_const_for_fn,
        clippy::needless_pass_by_ref_mut,
        clippy::unused_self
    )]
    fn handle_response_reasoning_summary_part_added(
        &mut self,
        _event: &OpenAiResponsesStreamEnvelope,
    ) {
    }

    #[allow(
        clippy::missing_const_for_fn,
        clippy::needless_pass_by_ref_mut,
        clippy::unused_self
    )]
    fn handle_response_content_part_added(
        &mut self,
        _event: &OpenAiResponsesStreamEnvelope,
    ) {
    }

    fn reset_current_block(&mut self) {
        self.current_block_index = None;
        self.current_block_kind = None;
        self.current_tool_json.clear();
    }
}

pub fn stream_openai_responses_http<T>(
    model: Model,
    params: T,
    api_key: String,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    stream_openai_responses_http_with_runtime_options(
        model,
        params,
        api_key,
        signal,
        None,
        BTreeMap::new(),
    )
}

pub fn stream_openai_responses_http_with_headers<T>(
    model: Model,
    params: T,
    api_key: String,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
    request_headers: BTreeMap<String, String>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    stream_openai_responses_http_with_runtime_options(
        model,
        params,
        api_key,
        signal,
        None,
        request_headers,
    )
}

fn stream_openai_responses_http_with_runtime_options<T>(
    model: Model,
    params: T,
    api_key: String,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
    requested_service_tier: Option<OpenAiResponsesServiceTier>,
    request_headers: BTreeMap<String, String>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    Box::pin(stream! {
        let mut signal = signal;
        let mut state = OpenAiResponsesStreamState::with_requested_service_tier(&model, requested_service_tier);

        if is_signal_aborted(&signal) {
            yield Ok(state.aborted_event());
            return;
        }

        let mut request_builder = reqwest::Client::new()
            .post(format!("{}/responses", model.base_url.trim_end_matches('/')))
            .bearer_auth(api_key)
            .header("accept", "text/event-stream");
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

                    for event in events {
                        if is_signal_aborted(&signal) {
                            yield Ok(state.aborted_event());
                            return;
                        }
                        let emitted = state.process_event(&event);
                        for assistant_event in emitted {
                            let is_terminal = is_terminal_event(&assistant_event);
                            yield Ok(assistant_event);
                            if is_terminal {
                                return;
                            }
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
                    for event in events {
                        if is_signal_aborted(&signal) {
                            yield Ok(state.aborted_event());
                            return;
                        }
                        let emitted = state.process_event(&event);
                        for assistant_event in emitted {
                            let is_terminal = is_terminal_event(&assistant_event);
                            yield Ok(assistant_event);
                            if is_terminal {
                                return;
                            }
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

pub fn stream_openai_responses_sse_events(
    model: Model,
    events: Vec<OpenAiResponsesStreamEnvelope>,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut state = OpenAiResponsesStreamState::new(&model);
        yield Ok(state.start_event());

        for event in events {
            let emitted = state.process_event(&event);
            for assistant_event in emitted {
                let is_terminal = is_terminal_event(&assistant_event);
                yield Ok(assistant_event);
                if is_terminal {
                    return;
                }
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

pub(crate) fn is_signal_aborted(signal: &Option<tokio::sync::watch::Receiver<bool>>) -> bool {
    signal
        .as_ref()
        .map(|signal| *signal.borrow())
        .unwrap_or(false)
}

pub(crate) async fn wait_for_abort(signal: &mut tokio::sync::watch::Receiver<bool>) {
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
    crate::partial_json::parse_partial_json_map(input)
}

fn response_service_tier(
    response: &serde_json::Map<String, Value>,
) -> Option<OpenAiResponsesServiceTier> {
    match response.get("service_tier").and_then(Value::as_str) {
        Some("auto") => Some(OpenAiResponsesServiceTier::Auto),
        Some("default") => Some(OpenAiResponsesServiceTier::Default),
        Some("flex") => Some(OpenAiResponsesServiceTier::Flex),
        Some("scale") => Some(OpenAiResponsesServiceTier::Scale),
        Some("priority") => Some(OpenAiResponsesServiceTier::Priority),
        _ => None,
    }
}

fn service_tier_cost_multiplier(service_tier: Option<OpenAiResponsesServiceTier>) -> f64 {
    match service_tier {
        Some(OpenAiResponsesServiceTier::Flex) => 0.5,
        Some(OpenAiResponsesServiceTier::Priority) => 2.0,
        _ => 1.0,
    }
}

fn apply_service_tier_pricing(usage: &mut Usage, service_tier: Option<OpenAiResponsesServiceTier>) {
    let multiplier = service_tier_cost_multiplier(service_tier);
    if (multiplier - 1.0).abs() < f64::EPSILON {
        return;
    }

    usage.cost.input *= multiplier;
    usage.cost.output *= multiplier;
    usage.cost.cache_read *= multiplier;
    usage.cost.cache_write *= multiplier;
    usage.cost.total =
        usage.cost.input + usage.cost.output + usage.cost.cache_read + usage.cost.cache_write;
}

fn apply_usage_from_response(
    output: &mut AssistantMessage,
    response: &serde_json::Map<String, Value>,
    model_cost: ModelCost,
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
    calculate_cost_with(model_cost, &mut output.usage);
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

fn message_item_text(item: &serde_json::Map<String, Value>) -> Option<String> {
    item.get("content")
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|part| {
                    let part = part.as_object()?;
                    match part.get("type").and_then(Value::as_str) {
                        Some("output_text") => part.get("text").and_then(Value::as_str),
                        Some("refusal") => part.get("refusal").and_then(Value::as_str),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
}

fn reasoning_summary_text(item: &serde_json::Map<String, Value>) -> Option<String> {
    item.get("summary")
        .and_then(Value::as_array)
        .map(|summary| {
            summary
                .iter()
                .filter_map(|part| part.as_object())
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
}

fn text_content(output: &AssistantMessage, index: usize) -> Option<String> {
    match output.content.get(index) {
        Some(AssistantContent::Text { text, .. }) => Some(text.clone()),
        _ => None,
    }
}

fn thinking_content(output: &AssistantMessage, index: usize) -> Option<String> {
    match output.content.get(index) {
        Some(AssistantContent::Thinking { thinking, .. }) => Some(thinking.clone()),
        _ => None,
    }
}

pub(crate) fn is_terminal_event(event: &AssistantEvent) -> bool {
    matches!(
        event,
        AssistantEvent::Done { .. } | AssistantEvent::Error { .. }
    )
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
        Box::pin(stream! {
            let params = build_openai_responses_request_params(
                &model,
                &context,
                OPENAI_RESPONSES_ALLOWED_TOOL_CALL_PROVIDERS,
                OpenAiResponsesConvertOptions::default(),
                OpenAiResponsesParamsOptions {
                    max_output_tokens: options.max_tokens,
                    temperature: options.temperature,
                    reasoning_effort: options.reasoning_effort.clone(),
                    reasoning_summary: options.reasoning_summary.clone(),
                    session_id: options.session_id.clone(),
                    cache_retention: options.cache_retention.map(|cache_retention| match cache_retention {
                        crate::CacheRetention::None => "none".into(),
                        crate::CacheRetention::Short => "short".into(),
                        crate::CacheRetention::Long => "long".into(),
                    }),
                    service_tier: options.service_tier,
                },
            );
            let payload = match crate::apply_payload_hook(&model, params, options.on_payload.as_ref()).await {
                Ok(payload) => payload,
                Err(error) => {
                    yield Ok(AssistantEvent::Error {
                        reason: StopReason::Error,
                        error: error_message(&model, error.to_string()),
                    });
                    return;
                }
            };
            let request_headers = build_runtime_request_headers(&model, &context, &options.headers);

            let api_key = options
                .api_key
                .clone()
                .or_else(|| crate::get_env_api_key(&model.provider));

            let mut inner = match api_key {
                Some(api_key) => stream_openai_responses_http_with_runtime_options(
                    model,
                    payload,
                    api_key,
                    options.signal.clone(),
                    options.service_tier,
                    request_headers,
                ),
                None => Box::pin(stream! {
                    yield Ok(AssistantEvent::Error {
                        reason: StopReason::Error,
                        error: error_message(&model, "OpenAI Responses API key is required".into()),
                    });
                }),
            };

            while let Some(event) = inner.next().await {
                yield event;
            }
        })
    }
}

pub fn register_openai_responses_provider() {
    register_provider("openai-responses", Arc::new(OpenAiResponsesProvider));
}

fn build_runtime_request_headers(
    model: &Model,
    _context: &Context,
    option_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut headers = get_model_headers(&model.provider, &model.id)
        .or_else(|| get_provider_headers(&model.provider))
        .unwrap_or_default();

    headers.extend(option_headers.clone());
    headers
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
