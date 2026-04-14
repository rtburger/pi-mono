use crate::{
    AiProvider, AssistantEventStream, CacheRetention, StreamOptions, get_env_api_key,
    models::{get_model_headers, get_provider_headers},
    register_provider,
};
use async_stream::stream;
use futures::StreamExt;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason,
    ToolDefinition, Usage, UserContent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const CLAUDE_CODE_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

const CLAUDE_CODE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

#[derive(Debug, Clone)]
pub struct AnthropicOptions {
    pub signal: Option<tokio::sync::watch::Receiver<bool>>,
    pub api_key: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub cache_retention: Option<CacheRetention>,
    pub thinking_enabled: Option<bool>,
    pub thinking_budget_tokens: Option<u64>,
    pub effort: Option<String>,
    pub interleaved_thinking: bool,
    pub metadata_user_id: Option<String>,
}

impl Default for AnthropicOptions {
    fn default() -> Self {
        Self {
            signal: None,
            api_key: None,
            headers: BTreeMap::new(),
            max_tokens: None,
            temperature: None,
            cache_retention: None,
            thinking_enabled: None,
            thinking_budget_tokens: None,
            effort: None,
            interleaved_thinking: true,
            metadata_user_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicCacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicRequestParams {
    pub model: String,
    pub messages: Vec<AnthropicMessageParam>,
    pub max_tokens: u64,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<AnthropicSystemBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<AnthropicThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<AnthropicOutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AnthropicMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicMessageParam {
    pub role: String,
    pub content: AnthropicMessageContent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    Image {
        source: AnthropicImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: BTreeMap<String, Value>,
    },
    ToolResult {
        tool_use_id: String,
        content: AnthropicMessageContent,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicThinkingConfig {
    Enabled { budget_tokens: u64 },
    Disabled,
    Adaptive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicOutputConfig {
    pub effort: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicMetadata {
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicStreamEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub data: serde_json::Map<String, Value>,
}

pub fn build_anthropic_request_params(
    model: &Model,
    context: &Context,
    is_oauth_token: bool,
    options: &AnthropicOptions,
) -> AnthropicRequestParams {
    let cache_control = cache_control_for(model, options.cache_retention);
    let messages = convert_anthropic_messages(
        &context.messages,
        model,
        is_oauth_token,
        cache_control.clone(),
    );

    let system = build_system_blocks(context, is_oauth_token, cache_control.clone());

    let thinking = build_thinking_config(model, options);
    let output_config = if matches!(thinking, Some(AnthropicThinkingConfig::Adaptive)) {
        options.effort.as_ref().map(|effort| AnthropicOutputConfig {
            effort: effort.clone(),
        })
    } else {
        None
    };

    let metadata = options
        .metadata_user_id
        .as_ref()
        .map(|user_id| AnthropicMetadata {
            user_id: user_id.clone(),
        });

    AnthropicRequestParams {
        model: model.id.clone(),
        messages,
        max_tokens: options
            .max_tokens
            .unwrap_or_else(|| (model.max_tokens / 3).max(1)),
        stream: true,
        system,
        temperature: if options.temperature.is_some()
            && !matches!(
                thinking,
                Some(AnthropicThinkingConfig::Enabled { .. } | AnthropicThinkingConfig::Adaptive)
            ) {
            options.temperature
        } else {
            None
        },
        tools: (!context.tools.is_empty()).then(|| convert_tools(&context.tools, is_oauth_token)),
        thinking,
        output_config,
        metadata,
    }
}

pub fn convert_anthropic_messages(
    messages: &[Message],
    model: &Model,
    is_oauth_token: bool,
    cache_control: Option<AnthropicCacheControl>,
) -> Vec<AnthropicMessageParam> {
    let transformed_messages = transform_messages_for_anthropic(model, messages);
    let mut params = Vec::new();
    let mut index = 0usize;

    while index < transformed_messages.len() {
        match &transformed_messages[index] {
            Message::User { content, .. } => {
                let content = convert_user_message_content(content, model);
                if content_is_empty(&content) {
                    index += 1;
                    continue;
                }
                params.push(AnthropicMessageParam {
                    role: "user".into(),
                    content,
                });
            }
            Message::Assistant { content, .. } => {
                let mut blocks = Vec::new();
                for block in content {
                    match block {
                        AssistantContent::Text { text, .. } => {
                            if text.trim().is_empty() {
                                continue;
                            }
                            blocks.push(AnthropicContentBlock::Text {
                                text: sanitize_text(text),
                                cache_control: None,
                            });
                        }
                        AssistantContent::Thinking {
                            thinking,
                            thinking_signature,
                            redacted,
                        } => {
                            if *redacted {
                                if let Some(signature) = thinking_signature {
                                    blocks.push(AnthropicContentBlock::RedactedThinking {
                                        data: signature.clone(),
                                    });
                                }
                                continue;
                            }
                            if thinking.trim().is_empty() {
                                continue;
                            }
                            if let Some(signature) = thinking_signature {
                                if !signature.trim().is_empty() {
                                    blocks.push(AnthropicContentBlock::Thinking {
                                        thinking: sanitize_text(thinking),
                                        signature: signature.clone(),
                                    });
                                } else {
                                    blocks.push(AnthropicContentBlock::Text {
                                        text: sanitize_text(thinking),
                                        cache_control: None,
                                    });
                                }
                            } else {
                                blocks.push(AnthropicContentBlock::Text {
                                    text: sanitize_text(thinking),
                                    cache_control: None,
                                });
                            }
                        }
                        AssistantContent::ToolCall {
                            id,
                            name,
                            arguments,
                            ..
                        } => blocks.push(AnthropicContentBlock::ToolUse {
                            id: id.clone(),
                            name: if is_oauth_token {
                                to_claude_code_name(name)
                            } else {
                                name.clone()
                            },
                            input: arguments.clone(),
                        }),
                    }
                }
                if !blocks.is_empty() {
                    params.push(AnthropicMessageParam {
                        role: "assistant".into(),
                        content: AnthropicMessageContent::Blocks(blocks),
                    });
                }
            }
            Message::ToolResult { .. } => {
                let mut tool_results = Vec::new();
                while index < transformed_messages.len() {
                    match &transformed_messages[index] {
                        Message::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                            ..
                        } => {
                            tool_results.push(AnthropicContentBlock::ToolResult {
                                tool_use_id: tool_call_id.clone(),
                                content: convert_tool_result_content(content),
                                is_error: *is_error,
                                cache_control: None,
                            });
                            index += 1;
                        }
                        _ => break,
                    }
                }
                params.push(AnthropicMessageParam {
                    role: "user".into(),
                    content: AnthropicMessageContent::Blocks(tool_results),
                });
                continue;
            }
        }
        index += 1;
    }

    if let Some(cache_control) = cache_control
        && let Some(last_user_message) = params
            .iter_mut()
            .rev()
            .find(|message| message.role == "user")
    {
        attach_cache_control(&mut last_user_message.content, cache_control);
    }

    params
}

pub fn normalize_anthropic_tool_call_id(id: &str) -> String {
    id.chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => character,
            _ => '_',
        })
        .take(64)
        .collect()
}

pub fn stream_anthropic_http<T>(
    model: Model,
    params: T,
    api_key: String,
    is_oauth_token: bool,
    tools: Vec<ToolDefinition>,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
    request_headers: BTreeMap<String, String>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    Box::pin(stream! {
        let mut signal = signal;
        let mut state = AnthropicStreamState::new(&model, is_oauth_token, tools);

        if is_signal_aborted(&signal) {
            yield Ok(state.aborted_event());
            return;
        }

        let use_bearer_auth = is_oauth_token;
        let mut request_builder = reqwest::Client::new()
            .post(format!("{}/messages", model.base_url.trim_end_matches('/')))
            .header("accept", "text/event-stream");
        if use_bearer_auth {
            request_builder = request_builder.bearer_auth(api_key);
        } else {
            request_builder = request_builder.header("x-api-key", api_key);
        }
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

        let mut decoder = AnthropicSseDecoder::default();

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
                            yield Ok(state.error_event(error));
                            return;
                        }
                    };

                    for event in events {
                        let emitted = state.process_event(event);
                        for assistant_event in emitted {
                            let terminal = is_terminal_event(&assistant_event);
                            yield Ok(assistant_event);
                            if terminal {
                                return;
                            }
                        }
                    }
                }
                Ok(None) => {
                    let events = match decoder.finish() {
                        Ok(events) => events,
                        Err(error) => {
                            yield Ok(state.error_event(error));
                            return;
                        }
                    };
                    for event in events {
                        let emitted = state.process_event(event);
                        for assistant_event in emitted {
                            let terminal = is_terminal_event(&assistant_event);
                            yield Ok(assistant_event);
                            if terminal {
                                return;
                            }
                        }
                    }
                    yield Ok(state.finish_event());
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

pub fn stream_anthropic_sse_events(
    model: Model,
    events: Vec<AnthropicStreamEnvelope>,
    is_oauth_token: bool,
    tools: Vec<ToolDefinition>,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut state = AnthropicStreamState::new(&model, is_oauth_token, tools);
        yield Ok(state.start_event());
        for event in events {
            let emitted = state.process_event(event);
            for assistant_event in emitted {
                let terminal = is_terminal_event(&assistant_event);
                yield Ok(assistant_event);
                if terminal {
                    return;
                }
            }
        }
        yield Ok(state.finish_event());
    })
}

pub fn register_anthropic_provider() {
    register_provider("anthropic-messages", Arc::new(AnthropicMessagesProvider));
}

#[derive(Default)]
struct AnthropicSseDecoder {
    buffer: Vec<u8>,
    current_event_type: Option<String>,
    current_data_lines: Vec<String>,
}

impl AnthropicSseDecoder {
    fn push_bytes(&mut self, chunk: &[u8]) -> Result<Vec<AnthropicStreamEnvelope>, String> {
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

    fn finish(&mut self) -> Result<Vec<AnthropicStreamEnvelope>, String> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.process_line(&line, &mut events)?;
        }
        if let Some(event) =
            flush_sse_event(&mut self.current_event_type, &mut self.current_data_lines)?
        {
            events.push(event);
        }
        Ok(events)
    }

    fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<AnthropicStreamEnvelope>,
    ) -> Result<(), String> {
        let mut text = std::str::from_utf8(line)
            .map_err(|error| format!("invalid UTF-8 in SSE line: {error}"))?
            .trim_end_matches('\r')
            .to_string();

        if text.is_empty() {
            if let Some(event) =
                flush_sse_event(&mut self.current_event_type, &mut self.current_data_lines)?
            {
                events.push(event);
            }
            return Ok(());
        }

        if text.starts_with(':') {
            return Ok(());
        }

        if let Some(rest) = text.strip_prefix("event:") {
            self.current_event_type = Some(rest.trim_start().to_string());
            return Ok(());
        }

        if let Some(rest) = text.strip_prefix("data:") {
            self.current_data_lines.push(rest.trim_start().to_string());
            return Ok(());
        }

        text.clear();
        Ok(())
    }
}

fn flush_sse_event(
    current_event_type: &mut Option<String>,
    current_data_lines: &mut Vec<String>,
) -> Result<Option<AnthropicStreamEnvelope>, String> {
    if current_data_lines.is_empty() {
        *current_event_type = None;
        return Ok(None);
    }

    let data = current_data_lines.join("\n");
    current_data_lines.clear();
    let event_type = current_event_type.take().unwrap_or_default();

    if data.trim() == "[DONE]" {
        return Ok(None);
    }

    let value: Value = serde_json::from_str(&data)
        .map_err(|error| format!("invalid Anthropic SSE JSON: {error}"))?;
    let object = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Anthropic SSE payload must be a JSON object".to_string())?;

    Ok(Some(AnthropicStreamEnvelope {
        event_type,
        data: object,
    }))
}

struct AnthropicStreamState {
    output: AssistantMessage,
    block_indices: BTreeMap<usize, usize>,
    partial_tool_json: BTreeMap<usize, String>,
    is_oauth_token: bool,
    tools: Vec<ToolDefinition>,
}

impl AnthropicStreamState {
    fn new(model: &Model, is_oauth_token: bool, tools: Vec<ToolDefinition>) -> Self {
        Self {
            output: AssistantMessage::empty(
                model.api.clone(),
                model.provider.clone(),
                model.id.clone(),
            ),
            block_indices: BTreeMap::new(),
            partial_tool_json: BTreeMap::new(),
            is_oauth_token,
            tools,
        }
    }

    fn start_event(&self) -> AssistantEvent {
        AssistantEvent::Start {
            partial: self.output.clone(),
        }
    }

    fn aborted_event(&mut self) -> AssistantEvent {
        self.output.stop_reason = StopReason::Aborted;
        self.output.error_message = Some("Request was aborted".into());
        self.output.timestamp = now_ms();
        AssistantEvent::Error {
            reason: StopReason::Aborted,
            error: self.output.clone(),
        }
    }

    fn error_event(&mut self, message: String) -> AssistantEvent {
        self.output.stop_reason = StopReason::Error;
        self.output.error_message = Some(message);
        self.output.timestamp = now_ms();
        AssistantEvent::Error {
            reason: StopReason::Error,
            error: self.output.clone(),
        }
    }

    fn finish_event(&mut self) -> AssistantEvent {
        self.output.timestamp = now_ms();
        match self.output.stop_reason {
            StopReason::Error => {
                if self.output.error_message.is_none() {
                    self.output.error_message = Some("An unknown error occurred".into());
                }
                AssistantEvent::Error {
                    reason: StopReason::Error,
                    error: self.output.clone(),
                }
            }
            StopReason::Aborted => AssistantEvent::Error {
                reason: StopReason::Aborted,
                error: self.output.clone(),
            },
            _ => AssistantEvent::Done {
                reason: self.output.stop_reason.clone(),
                message: self.output.clone(),
            },
        }
    }

    fn process_event(&mut self, event: AnthropicStreamEnvelope) -> Vec<AssistantEvent> {
        let mut emitted = Vec::new();
        match event.event_type.as_str() {
            "message_start" => {
                if let Some(message) = object_field(&event.data, "message") {
                    if let Some(id) = string_field(message, "id") {
                        self.output.response_id = Some(id.to_string());
                    }
                    if let Some(usage) = object_field(message, "usage") {
                        self.apply_usage(usage, true);
                    }
                }
            }
            "content_block_start" => {
                let Some(provider_index) = usize_field(&event.data, "index") else {
                    return emitted;
                };
                let Some(content_block) = object_field(&event.data, "content_block") else {
                    return emitted;
                };
                let Some(block_type) = string_field(content_block, "type") else {
                    return emitted;
                };
                let content_index = self.output.content.len();
                self.block_indices.insert(provider_index, content_index);

                match block_type {
                    "text" => {
                        self.output.content.push(AssistantContent::Text {
                            text: String::new(),
                            text_signature: None,
                        });
                        emitted.push(AssistantEvent::TextStart {
                            content_index,
                            partial: self.output.clone(),
                        });
                    }
                    "thinking" => {
                        self.output.content.push(AssistantContent::Thinking {
                            thinking: String::new(),
                            thinking_signature: Some(String::new()),
                            redacted: false,
                        });
                        emitted.push(AssistantEvent::ThinkingStart {
                            content_index,
                            partial: self.output.clone(),
                        });
                    }
                    "redacted_thinking" => {
                        self.output.content.push(AssistantContent::Thinking {
                            thinking: "[Reasoning redacted]".into(),
                            thinking_signature: string_field(content_block, "data")
                                .map(ToOwned::to_owned),
                            redacted: true,
                        });
                        emitted.push(AssistantEvent::ThinkingStart {
                            content_index,
                            partial: self.output.clone(),
                        });
                    }
                    "tool_use" => {
                        let name = string_field(content_block, "name").unwrap_or_default();
                        let normalized_name = if self.is_oauth_token {
                            from_claude_code_name(name, &self.tools)
                        } else {
                            name.to_string()
                        };
                        self.output.content.push(AssistantContent::ToolCall {
                            id: string_field(content_block, "id")
                                .unwrap_or_default()
                                .to_string(),
                            name: normalized_name,
                            arguments: object_field(content_block, "input")
                                .map(value_object_to_btree)
                                .unwrap_or_default(),
                            thought_signature: None,
                        });
                        self.partial_tool_json.insert(provider_index, String::new());
                        emitted.push(AssistantEvent::ToolCallStart {
                            content_index,
                            partial: self.output.clone(),
                        });
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let Some(provider_index) = usize_field(&event.data, "index") else {
                    return emitted;
                };
                let Some(content_index) = self.block_indices.get(&provider_index).copied() else {
                    return emitted;
                };
                let Some(delta) = object_field(&event.data, "delta") else {
                    return emitted;
                };
                let Some(delta_type) = string_field(delta, "type") else {
                    return emitted;
                };
                match delta_type {
                    "text_delta" => {
                        let text = string_field(delta, "text").unwrap_or_default().to_string();
                        if let Some(AssistantContent::Text { text: existing, .. }) =
                            self.output.content.get_mut(content_index)
                        {
                            existing.push_str(&text);
                        }
                        emitted.push(AssistantEvent::TextDelta {
                            content_index,
                            delta: text,
                            partial: self.output.clone(),
                        });
                    }
                    "thinking_delta" => {
                        let thinking = string_field(delta, "thinking")
                            .unwrap_or_default()
                            .to_string();
                        if let Some(AssistantContent::Thinking {
                            thinking: existing, ..
                        }) = self.output.content.get_mut(content_index)
                        {
                            existing.push_str(&thinking);
                        }
                        emitted.push(AssistantEvent::ThinkingDelta {
                            content_index,
                            delta: thinking,
                            partial: self.output.clone(),
                        });
                    }
                    "input_json_delta" => {
                        let partial_json = string_field(delta, "partial_json")
                            .unwrap_or_default()
                            .to_string();
                        let buffer = self.partial_tool_json.entry(provider_index).or_default();
                        buffer.push_str(&partial_json);
                        if let Some(AssistantContent::ToolCall { arguments, .. }) =
                            self.output.content.get_mut(content_index)
                        {
                            let parsed = parse_tool_arguments_best_effort(buffer);
                            if !parsed.is_empty() || buffer.trim() == "{}" {
                                *arguments = parsed;
                            }
                        }
                        emitted.push(AssistantEvent::ToolCallDelta {
                            content_index,
                            delta: partial_json,
                            partial: self.output.clone(),
                        });
                    }
                    "signature_delta" => {
                        let signature = string_field(delta, "signature")
                            .unwrap_or_default()
                            .to_string();
                        if let Some(AssistantContent::Thinking {
                            thinking_signature, ..
                        }) = self.output.content.get_mut(content_index)
                        {
                            let value = thinking_signature.get_or_insert_with(String::new);
                            value.push_str(&signature);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let Some(provider_index) = usize_field(&event.data, "index") else {
                    return emitted;
                };
                let Some(content_index) = self.block_indices.remove(&provider_index) else {
                    return emitted;
                };
                match self.output.content.get(content_index).cloned() {
                    Some(AssistantContent::Text { text, .. }) => {
                        emitted.push(AssistantEvent::TextEnd {
                            content_index,
                            content: text,
                            partial: self.output.clone(),
                        });
                    }
                    Some(AssistantContent::Thinking { thinking, .. }) => {
                        emitted.push(AssistantEvent::ThinkingEnd {
                            content_index,
                            content: thinking,
                            partial: self.output.clone(),
                        });
                    }
                    Some(AssistantContent::ToolCall { mut arguments, .. }) => {
                        if let Some(buffer) = self.partial_tool_json.remove(&provider_index)
                            && (!buffer.is_empty() || arguments.is_empty())
                        {
                            let parsed = parse_tool_arguments_best_effort(&buffer);
                            if let Some(AssistantContent::ToolCall {
                                arguments: existing,
                                ..
                            }) = self.output.content.get_mut(content_index)
                            {
                                *existing = parsed.clone();
                            }
                            arguments = parsed;
                        }
                        let tool_call = self.output.content[content_index].clone();
                        let _ = arguments;
                        emitted.push(AssistantEvent::ToolCallEnd {
                            content_index,
                            tool_call,
                            partial: self.output.clone(),
                        });
                    }
                    None => {}
                }
            }
            "message_delta" => {
                if let Some(delta) = object_field(&event.data, "delta")
                    && let Some(stop_reason) = string_field(delta, "stop_reason")
                {
                    match map_stop_reason(stop_reason) {
                        Ok(stop_reason) => self.output.stop_reason = stop_reason,
                        Err(message) => return vec![self.error_event(message)],
                    }
                }
                if let Some(usage) = object_field(&event.data, "usage") {
                    self.apply_usage(usage, false);
                }
            }
            "error" => {
                let message = object_field(&event.data, "error")
                    .and_then(|error| string_field(error, "message"))
                    .unwrap_or("Unknown error")
                    .to_string();
                emitted.push(self.error_event(message));
            }
            _ => {}
        }
        emitted
    }

    fn apply_usage(&mut self, usage: &serde_json::Map<String, Value>, include_input: bool) {
        if include_input {
            if let Some(value) = u64_from_map(usage, "input_tokens") {
                self.output.usage.input = value;
            }
            if let Some(value) = u64_from_map(usage, "cache_read_input_tokens") {
                self.output.usage.cache_read = value;
            }
            if let Some(value) = u64_from_map(usage, "cache_creation_input_tokens") {
                self.output.usage.cache_write = value;
            }
        }
        if let Some(value) = u64_from_map(usage, "output_tokens") {
            self.output.usage.output = value;
        }
        self.output.usage.total_tokens = self.output.usage.input
            + self.output.usage.output
            + self.output.usage.cache_read
            + self.output.usage.cache_write;
    }
}

struct AnthropicMessagesProvider;

impl AiProvider for AnthropicMessagesProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        Box::pin(stream! {
            let api_key = options
                .api_key
                .clone()
                .or_else(|| get_env_api_key(&model.provider));
            let Some(api_key) = api_key else {
                let mut inner = terminal_error_stream(
                    &model,
                    format!("{} API key is required", provider_label(&model)),
                );
                while let Some(event) = inner.next().await {
                    yield event;
                }
                return;
            };

            let anthropic_options = anthropic_options_from_stream_options(&model, &options);
            let is_oauth_token = is_oauth_token(&api_key);
            let params = build_anthropic_request_params(
                &model,
                &context,
                is_oauth_token,
                &anthropic_options,
            );
            let payload = match crate::apply_payload_hook(&model, params, options.on_payload.as_ref()).await {
                Ok(payload) => payload,
                Err(error) => {
                    let message = error.to_string();
                    let mut inner = terminal_error_stream(&model, message);
                    while let Some(event) = inner.next().await {
                        yield event;
                    }
                    return;
                }
            };
            let request_headers = build_runtime_request_headers(
                &model,
                &context,
                &anthropic_options.headers,
                anthropic_options.interleaved_thinking,
                is_oauth_token,
            );

            let mut inner = stream_anthropic_http(
                model,
                payload,
                api_key,
                is_oauth_token,
                context.tools.clone(),
                anthropic_options.signal,
                request_headers,
            );
            while let Some(event) = inner.next().await {
                yield event;
            }
        })
    }
}

fn anthropic_options_from_stream_options(
    model: &Model,
    options: &StreamOptions,
) -> AnthropicOptions {
    let mut mapped = AnthropicOptions {
        signal: options.signal.clone(),
        api_key: options.api_key.clone(),
        headers: options.headers.clone(),
        max_tokens: options.max_tokens,
        temperature: options.temperature,
        cache_retention: options.cache_retention,
        metadata_user_id: options
            .metadata
            .get("user_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        ..AnthropicOptions::default()
    };

    if model.reasoning {
        match options.reasoning_effort.as_deref() {
            Some("none") | None => mapped.thinking_enabled = Some(false),
            Some(level) => {
                mapped.thinking_enabled = Some(true);
                if supports_adaptive_thinking(&model.id) {
                    mapped.effort = Some(map_reasoning_effort_to_adaptive(level, &model.id));
                } else {
                    mapped.thinking_budget_tokens = Some(reasoning_budget_for_level(level));
                }
            }
        }
    }

    mapped
}

fn build_runtime_request_headers(
    model: &Model,
    _context: &Context,
    option_headers: &BTreeMap<String, String>,
    interleaved_thinking: bool,
    is_oauth_token: bool,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([
        ("accept".to_string(), "text/event-stream".to_string()),
        ("anthropic-version".to_string(), "2023-06-01".to_string()),
    ]);

    if let Some(model_headers) = get_model_headers(&model.provider, &model.id)
        .or_else(|| get_provider_headers(&model.provider))
    {
        headers.extend(model_headers);
    }

    if is_oauth_token {
        let mut betas = vec![
            "claude-code-20250219",
            "oauth-2025-04-20",
            "fine-grained-tool-streaming-2025-05-14",
        ];
        if interleaved_thinking && !supports_adaptive_thinking(&model.id) {
            betas.push("interleaved-thinking-2025-05-14");
        }
        headers.extend(BTreeMap::from([
            ("user-agent".into(), "claude-cli/2.1.75".into()),
            ("x-app".into(), "cli".into()),
            ("anthropic-beta".into(), betas.join(",")),
        ]));
    } else {
        let mut betas = vec!["fine-grained-tool-streaming-2025-05-14"];
        if interleaved_thinking && !supports_adaptive_thinking(&model.id) {
            betas.push("interleaved-thinking-2025-05-14");
        }
        headers.insert("anthropic-beta".into(), betas.join(","));
    }

    headers.extend(option_headers.clone());
    headers
}

fn provider_label(model: &Model) -> &'static str {
    match model.api.as_str() {
        "anthropic-messages" => "Anthropic Messages",
        _ => "Provider",
    }
}

fn terminal_error_stream(model: &Model, error_message: String) -> AssistantEventStream {
    let error = AssistantMessage {
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
    };

    Box::pin(stream! {
        yield Ok(AssistantEvent::Error {
            reason: StopReason::Error,
            error,
        });
    })
}

fn build_thinking_config(
    model: &Model,
    options: &AnthropicOptions,
) -> Option<AnthropicThinkingConfig> {
    if !model.reasoning {
        return None;
    }

    match options.thinking_enabled {
        Some(false) => Some(AnthropicThinkingConfig::Disabled),
        Some(true) => {
            if supports_adaptive_thinking(&model.id) {
                Some(AnthropicThinkingConfig::Adaptive)
            } else {
                Some(AnthropicThinkingConfig::Enabled {
                    budget_tokens: options.thinking_budget_tokens.unwrap_or(1024),
                })
            }
        }
        None => None,
    }
}

fn build_system_blocks(
    context: &Context,
    is_oauth_token: bool,
    cache_control: Option<AnthropicCacheControl>,
) -> Option<Vec<AnthropicSystemBlock>> {
    let mut blocks = Vec::new();

    if is_oauth_token {
        blocks.push(AnthropicSystemBlock {
            block_type: "text".into(),
            text: CLAUDE_CODE_IDENTITY.into(),
            cache_control: cache_control.clone(),
        });
    }

    if let Some(system_prompt) = &context.system_prompt {
        blocks.push(AnthropicSystemBlock {
            block_type: "text".into(),
            text: sanitize_text(system_prompt),
            cache_control,
        });
    }

    (!blocks.is_empty()).then_some(blocks)
}

fn convert_tools(tools: &[ToolDefinition], is_oauth_token: bool) -> Vec<AnthropicToolDefinition> {
    tools
        .iter()
        .map(|tool| AnthropicToolDefinition {
            name: if is_oauth_token {
                to_claude_code_name(&tool.name)
            } else {
                tool.name.clone()
            },
            description: tool.description.clone(),
            input_schema: normalized_input_schema(&tool.parameters),
        })
        .collect()
}

fn normalized_input_schema(parameters: &Value) -> Value {
    let properties = parameters
        .get("properties")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let required = parameters
        .get("required")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn convert_user_message_content(content: &[UserContent], model: &Model) -> AnthropicMessageContent {
    let has_images = content
        .iter()
        .any(|part| matches!(part, UserContent::Image { .. }))
        && model.input.iter().any(|input| input == "image");

    if !has_images {
        let text = content
            .iter()
            .filter_map(|part| match part {
                UserContent::Text { text } => Some(text.as_str()),
                UserContent::Image { .. } => None,
            })
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return AnthropicMessageContent::Text(sanitize_text(&text));
    }

    let mut blocks = Vec::new();
    for part in content {
        match part {
            UserContent::Text { text } if !text.trim().is_empty() => {
                blocks.push(AnthropicContentBlock::Text {
                    text: sanitize_text(text),
                    cache_control: None,
                });
            }
            UserContent::Image { data, mime_type }
                if model.input.iter().any(|input| input == "image") =>
            {
                blocks.push(AnthropicContentBlock::Image {
                    source: AnthropicImageSource {
                        source_type: "base64".into(),
                        media_type: mime_type.clone(),
                        data: data.clone(),
                    },
                    cache_control: None,
                });
            }
            _ => {}
        }
    }

    if !blocks
        .iter()
        .any(|block| matches!(block, AnthropicContentBlock::Text { .. }))
    {
        blocks.insert(
            0,
            AnthropicContentBlock::Text {
                text: "(see attached image)".into(),
                cache_control: None,
            },
        );
    }

    AnthropicMessageContent::Blocks(blocks)
}

fn convert_tool_result_content(content: &[UserContent]) -> AnthropicMessageContent {
    let has_images = content
        .iter()
        .any(|part| matches!(part, UserContent::Image { .. }));

    if !has_images {
        let text = content
            .iter()
            .filter_map(|part| match part {
                UserContent::Text { text } => Some(text.as_str()),
                UserContent::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return AnthropicMessageContent::Text(sanitize_text(&text));
    }

    let mut blocks = Vec::new();
    for part in content {
        match part {
            UserContent::Text { text } if !text.trim().is_empty() => {
                blocks.push(AnthropicContentBlock::Text {
                    text: sanitize_text(text),
                    cache_control: None,
                });
            }
            UserContent::Image { data, mime_type } => {
                blocks.push(AnthropicContentBlock::Image {
                    source: AnthropicImageSource {
                        source_type: "base64".into(),
                        media_type: mime_type.clone(),
                        data: data.clone(),
                    },
                    cache_control: None,
                });
            }
            _ => {}
        }
    }

    if !blocks
        .iter()
        .any(|block| matches!(block, AnthropicContentBlock::Text { .. }))
    {
        blocks.insert(
            0,
            AnthropicContentBlock::Text {
                text: "(see attached image)".into(),
                cache_control: None,
            },
        );
    }

    AnthropicMessageContent::Blocks(blocks)
}

fn transform_messages_for_anthropic(model: &Model, messages: &[Message]) -> Vec<Message> {
    let mut tool_call_id_map = BTreeMap::<String, String>::new();
    let mut transformed = Vec::new();

    for message in messages.iter().cloned() {
        match message {
            user_message @ Message::User { .. } => transformed.push(user_message),
            Message::ToolResult {
                tool_call_id,
                tool_name,
                content,
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
                let is_same_model =
                    provider == model.provider && api == model.api && source_model == model.id;
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
                            if redacted {
                                if is_same_model {
                                    transformed_content.push(AssistantContent::Thinking {
                                        thinking,
                                        thinking_signature,
                                        redacted,
                                    });
                                }
                                continue;
                            }

                            if is_same_model {
                                if thinking_signature
                                    .as_ref()
                                    .is_some_and(|signature| !signature.is_empty())
                                {
                                    transformed_content.push(AssistantContent::Thinking {
                                        thinking,
                                        thinking_signature,
                                        redacted,
                                    });
                                } else if !thinking.trim().is_empty() {
                                    transformed_content.push(AssistantContent::Thinking {
                                        thinking,
                                        thinking_signature,
                                        redacted,
                                    });
                                }
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
                                normalize_anthropic_tool_call_id(&id)
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
                is_error: true,
                timestamp: now_ms(),
            });
        }
    }
    pending_tool_calls.clear();
    existing_tool_result_ids.clear();
}

fn content_is_empty(content: &AnthropicMessageContent) -> bool {
    match content {
        AnthropicMessageContent::Text(text) => text.trim().is_empty(),
        AnthropicMessageContent::Blocks(blocks) => blocks.is_empty(),
    }
}

fn attach_cache_control(
    content: &mut AnthropicMessageContent,
    cache_control: AnthropicCacheControl,
) {
    match content {
        AnthropicMessageContent::Text(text) => {
            let text_value = std::mem::take(text);
            *content = AnthropicMessageContent::Blocks(vec![AnthropicContentBlock::Text {
                text: text_value,
                cache_control: Some(cache_control),
            }]);
        }
        AnthropicMessageContent::Blocks(blocks) => {
            if let Some(last) = blocks.last_mut() {
                match last {
                    AnthropicContentBlock::Text {
                        cache_control: slot,
                        ..
                    }
                    | AnthropicContentBlock::Image {
                        cache_control: slot,
                        ..
                    }
                    | AnthropicContentBlock::ToolResult {
                        cache_control: slot,
                        ..
                    } => {
                        *slot = Some(cache_control);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn cache_control_for(
    model: &Model,
    cache_retention: Option<CacheRetention>,
) -> Option<AnthropicCacheControl> {
    let retention = resolve_cache_retention(cache_retention);
    match retention {
        CacheRetention::None => None,
        CacheRetention::Short => Some(AnthropicCacheControl {
            cache_type: "ephemeral".into(),
            ttl: None,
        }),
        CacheRetention::Long => Some(AnthropicCacheControl {
            cache_type: "ephemeral".into(),
            ttl: model
                .base_url
                .contains("api.anthropic.com")
                .then(|| "1h".into()),
        }),
    }
}

fn resolve_cache_retention(cache_retention: Option<CacheRetention>) -> CacheRetention {
    match cache_retention {
        Some(cache_retention) => cache_retention,
        None => {
            if env::var("PI_CACHE_RETENTION").ok().as_deref() == Some("long") {
                CacheRetention::Long
            } else {
                CacheRetention::Short
            }
        }
    }
}

fn supports_adaptive_thinking(model_id: &str) -> bool {
    model_id.contains("opus-4-6")
        || model_id.contains("opus-4.6")
        || model_id.contains("sonnet-4-6")
        || model_id.contains("sonnet-4.6")
}

fn reasoning_budget_for_level(level: &str) -> u64 {
    match level {
        "minimal" => 1024,
        "low" => 2048,
        "medium" => 8192,
        "high" | "xhigh" => 16_384,
        _ => 8192,
    }
}

fn map_reasoning_effort_to_adaptive(level: &str, model_id: &str) -> String {
    match level {
        "minimal" | "low" => "low".into(),
        "medium" => "medium".into(),
        "xhigh" if model_id.contains("opus-4-6") || model_id.contains("opus-4.6") => "max".into(),
        "high" | "xhigh" => "high".into(),
        other => other.to_string(),
    }
}

fn is_oauth_token(api_key: &str) -> bool {
    api_key.contains("sk-ant-oat")
}

fn to_claude_code_name(name: &str) -> String {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .map(|candidate| (*candidate).to_string())
        .unwrap_or_else(|| name.to_string())
}

fn from_claude_code_name(name: &str, tools: &[ToolDefinition]) -> String {
    let lower_name = name.to_ascii_lowercase();
    tools
        .iter()
        .find(|tool| tool.name.to_ascii_lowercase() == lower_name)
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| name.to_string())
}

fn sanitize_text(text: &str) -> String {
    crate::unicode::sanitize_provider_text(text)
}

fn parse_tool_arguments_best_effort(partial_json: &str) -> BTreeMap<String, Value> {
    crate::partial_json::parse_partial_json_map(partial_json)
}

fn value_object_to_btree(object: &serde_json::Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn map_stop_reason(reason: &str) -> Result<StopReason, String> {
    match reason {
        "end_turn" | "pause_turn" | "stop_sequence" => Ok(StopReason::Stop),
        "max_tokens" => Ok(StopReason::Length),
        "tool_use" => Ok(StopReason::ToolUse),
        "refusal" | "sensitive" => Ok(StopReason::Error),
        other => Err(format!("Unhandled stop reason: {other}")),
    }
}

fn object_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    object.get(key)?.as_object()
}

fn string_field<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key)?.as_str()
}

fn usize_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<usize> {
    object.get(key)?.as_u64().map(|value| value as usize)
}

fn u64_from_map(object: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key)?.as_u64()
}

fn is_terminal_event(event: &AssistantEvent) -> bool {
    matches!(
        event,
        AssistantEvent::Done { .. } | AssistantEvent::Error { .. }
    )
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
            break;
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
