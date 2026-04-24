use crate::{
    AiProvider, AssistantEventStream, StreamOptions,
    models::{calculate_cost_with, get_model_headers, get_provider_headers},
    register_provider,
    unicode::sanitize_provider_text,
};
use async_stream::stream;
use futures::StreamExt;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, ModelCost,
    OpenAiCompletionsCompatConfig, StopReason, ToolDefinition, Usage, UserContent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }
}

pub use pi_events::{ModelRouting, OpenAiCompletionsMaxTokensField, OpenAiThinkingFormat};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsCompat {
    pub supports_store: bool,
    pub supports_developer_role: bool,
    pub supports_reasoning_effort: bool,
    pub reasoning_effort_map: BTreeMap<String, String>,
    pub supports_usage_in_streaming: bool,
    pub max_tokens_field: OpenAiCompletionsMaxTokensField,
    pub requires_tool_result_name: bool,
    pub requires_assistant_after_tool_result: bool,
    pub requires_thinking_as_text: bool,
    pub thinking_format: OpenAiThinkingFormat,
    pub open_router_routing: ModelRouting,
    pub vercel_gateway_routing: ModelRouting,
    pub zai_tool_stream: bool,
    pub supports_strict_mode: bool,
}

impl Default for OpenAiCompletionsCompat {
    fn default() -> Self {
        Self {
            supports_store: true,
            supports_developer_role: true,
            supports_reasoning_effort: true,
            reasoning_effort_map: BTreeMap::new(),
            supports_usage_in_streaming: true,
            max_tokens_field: OpenAiCompletionsMaxTokensField::MaxCompletionTokens,
            requires_tool_result_name: false,
            requires_assistant_after_tool_result: false,
            requires_thinking_as_text: false,
            thinking_format: OpenAiThinkingFormat::OpenAi,
            open_router_routing: ModelRouting::default(),
            vercel_gateway_routing: ModelRouting::default(),
            zai_tool_stream: false,
            supports_strict_mode: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCompletionsRequestOptions {
    pub tool_choice: Option<OpenAiCompletionsToolChoice>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
}

impl Default for OpenAiCompletionsRequestOptions {
    fn default() -> Self {
        Self {
            tool_choice: None,
            reasoning_effort: None,
            max_tokens: None,
            temperature: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompletionsToolChoiceMode {
    Auto,
    None,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsToolChoiceFunction {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsFunctionChoice {
    #[serde(rename = "type")]
    pub choice_type: String,
    pub function: OpenAiCompletionsToolChoiceFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpenAiCompletionsToolChoice {
    Mode(OpenAiCompletionsToolChoiceMode),
    Function(OpenAiCompletionsFunctionChoice),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCompletionsRequestParams {
    pub model: String,
    pub messages: Vec<OpenAiCompletionsMessageParam>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<OpenAiCompletionsStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAiCompletionsToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<OpenAiCompletionsToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<OpenAiCompletionsReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<OpenAiCompletionsChatTemplateKwargs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ModelRouting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<OpenAiCompletionsProviderOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_stream: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsReasoning {
    pub effort: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsChatTemplateKwargs {
    pub enable_thinking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsProviderOptions {
    pub gateway: ModelRouting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsStreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCompletionsMessageParam {
    pub role: String,
    pub content: OpenAiCompletionsMessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiCompletionsToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpenAiCompletionsMessageContent {
    Text(String),
    Parts(Vec<OpenAiCompletionsContentPart>),
    Null(()),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAiCompletionsContentPart {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<OpenAiCompletionsCacheControl>,
    },
    ImageUrl {
        image_url: OpenAiCompletionsImageUrl,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsCacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAiCompletionsToolCallFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAiCompletionsToolDefinition {
    Function {
        function: OpenAiCompletionsFunctionDefinition,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCompletionsFunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

pub fn detect_openai_completions_compat(model: &Model) -> OpenAiCompletionsCompat {
    let provider = model.provider.as_str();
    let base_url = model.base_url.as_str();

    let is_zai = provider == "zai" || base_url.contains("api.z.ai");
    let is_non_standard = provider == "cerebras"
        || base_url.contains("cerebras.ai")
        || provider == "xai"
        || base_url.contains("api.x.ai")
        || base_url.contains("chutes.ai")
        || base_url.contains("deepseek.com")
        || is_zai
        || provider == "opencode"
        || base_url.contains("opencode.ai");
    let use_max_tokens = base_url.contains("chutes.ai");
    let is_grok = provider == "xai" || base_url.contains("api.x.ai");
    let is_groq = provider == "groq" || base_url.contains("groq.com");

    let mut detected = OpenAiCompletionsCompat {
        supports_store: !is_non_standard,
        supports_developer_role: !is_non_standard,
        supports_reasoning_effort: !is_grok && !is_zai,
        reasoning_effort_map: if is_groq && model.id == "qwen/qwen3-32b" {
            BTreeMap::from([
                ("minimal".into(), "default".into()),
                ("low".into(), "default".into()),
                ("medium".into(), "default".into()),
                ("high".into(), "default".into()),
                ("xhigh".into(), "default".into()),
            ])
        } else {
            BTreeMap::new()
        },
        supports_usage_in_streaming: true,
        max_tokens_field: if use_max_tokens {
            OpenAiCompletionsMaxTokensField::MaxTokens
        } else {
            OpenAiCompletionsMaxTokensField::MaxCompletionTokens
        },
        requires_tool_result_name: false,
        requires_assistant_after_tool_result: false,
        requires_thinking_as_text: false,
        thinking_format: if is_zai {
            OpenAiThinkingFormat::Zai
        } else if provider == "openrouter" || base_url.contains("openrouter.ai") {
            OpenAiThinkingFormat::OpenRouter
        } else {
            OpenAiThinkingFormat::OpenAi
        },
        open_router_routing: ModelRouting::default(),
        vercel_gateway_routing: ModelRouting::default(),
        zai_tool_stream: false,
        supports_strict_mode: true,
    };

    if let Some(compat) = model
        .compat
        .as_ref()
        .and_then(|compat| compat.as_openai_completions())
    {
        merge_openai_completions_compat(&mut detected, compat);
    }

    detected
}

fn merge_openai_completions_compat(
    detected: &mut OpenAiCompletionsCompat,
    override_compat: &OpenAiCompletionsCompatConfig,
) {
    if let Some(value) = override_compat.supports_store {
        detected.supports_store = value;
    }
    if let Some(value) = override_compat.supports_developer_role {
        detected.supports_developer_role = value;
    }
    if let Some(value) = override_compat.supports_reasoning_effort {
        detected.supports_reasoning_effort = value;
    }
    if let Some(reasoning_effort_map) = override_compat.reasoning_effort_map.as_ref() {
        detected.reasoning_effort_map = reasoning_effort_map.clone();
    }
    if let Some(value) = override_compat.supports_usage_in_streaming {
        detected.supports_usage_in_streaming = value;
    }
    if let Some(value) = override_compat.max_tokens_field {
        detected.max_tokens_field = value;
    }
    if let Some(value) = override_compat.requires_tool_result_name {
        detected.requires_tool_result_name = value;
    }
    if let Some(value) = override_compat.requires_assistant_after_tool_result {
        detected.requires_assistant_after_tool_result = value;
    }
    if let Some(value) = override_compat.requires_thinking_as_text {
        detected.requires_thinking_as_text = value;
    }
    if let Some(value) = override_compat.thinking_format {
        detected.thinking_format = value;
    }
    if let Some(routing) = override_compat.open_router_routing.as_ref() {
        detected.open_router_routing = routing.clone();
    }
    if let Some(routing) = override_compat.vercel_gateway_routing.as_ref() {
        detected.vercel_gateway_routing = routing.clone();
    }
    if let Some(value) = override_compat.zai_tool_stream {
        detected.zai_tool_stream = value;
    }
    if let Some(value) = override_compat.supports_strict_mode {
        detected.supports_strict_mode = value;
    }
}

pub fn build_openai_completions_request_params(
    model: &Model,
    context: &Context,
    compat: &OpenAiCompletionsCompat,
    options: &OpenAiCompletionsRequestOptions,
) -> OpenAiCompletionsRequestParams {
    let mut messages = convert_openai_completions_messages(model, context, compat);
    maybe_add_openrouter_anthropic_cache_control(model, &mut messages);
    let tools = if !context.tools.is_empty() {
        Some(convert_tools(&context.tools, compat))
    } else if has_tool_history(&context.messages) {
        Some(Vec::new())
    } else {
        None
    };

    let mapped_reasoning_effort = options
        .reasoning_effort
        .as_ref()
        .map(|effort| map_reasoning_effort(*effort, compat));
    let reasoning_enabled = options.reasoning_effort.is_some();

    let mut params = OpenAiCompletionsRequestParams {
        model: model.id.clone(),
        messages,
        stream: true,
        stream_options: compat.supports_usage_in_streaming.then_some(
            OpenAiCompletionsStreamOptions {
                include_usage: true,
            },
        ),
        store: compat.supports_store.then_some(false),
        max_completion_tokens: match compat.max_tokens_field {
            OpenAiCompletionsMaxTokensField::MaxCompletionTokens => options.max_tokens,
            OpenAiCompletionsMaxTokensField::MaxTokens => None,
        },
        max_tokens: match compat.max_tokens_field {
            OpenAiCompletionsMaxTokensField::MaxCompletionTokens => None,
            OpenAiCompletionsMaxTokensField::MaxTokens => options.max_tokens,
        },
        temperature: options.temperature,
        tools,
        tool_choice: options.tool_choice.clone(),
        reasoning_effort: None,
        reasoning: None,
        enable_thinking: None,
        chat_template_kwargs: None,
        provider: None,
        provider_options: None,
        tool_stream: None,
    };

    if model.reasoning {
        match compat.thinking_format {
            OpenAiThinkingFormat::Zai | OpenAiThinkingFormat::Qwen => {
                params.enable_thinking = Some(reasoning_enabled);
            }
            OpenAiThinkingFormat::QwenChatTemplate => {
                params.chat_template_kwargs = Some(OpenAiCompletionsChatTemplateKwargs {
                    enable_thinking: reasoning_enabled,
                });
            }
            OpenAiThinkingFormat::OpenRouter => {
                params.reasoning = Some(OpenAiCompletionsReasoning {
                    effort: mapped_reasoning_effort.unwrap_or_else(|| "none".into()),
                });
            }
            OpenAiThinkingFormat::OpenAi => {
                if compat.supports_reasoning_effort {
                    params.reasoning_effort = mapped_reasoning_effort;
                }
            }
        }
    }

    if !context.tools.is_empty() && compat.zai_tool_stream {
        params.tool_stream = Some(true);
    }

    if model.base_url.contains("openrouter.ai") && !compat.open_router_routing.is_empty() {
        params.provider = Some(compat.open_router_routing.clone());
    }

    if model.base_url.contains("ai-gateway.vercel.sh") && !compat.vercel_gateway_routing.is_empty()
    {
        params.provider_options = Some(OpenAiCompletionsProviderOptions {
            gateway: compat.vercel_gateway_routing.clone(),
        });
    }

    params
}

pub fn convert_openai_completions_messages(
    model: &Model,
    context: &Context,
    compat: &OpenAiCompletionsCompat,
) -> Vec<OpenAiCompletionsMessageParam> {
    let transformed_messages = transform_messages_for_openai_completions(model, &context.messages);
    let mut params = Vec::new();

    if let Some(system_prompt) = &context.system_prompt {
        let role = if model.reasoning && compat.supports_developer_role {
            "developer"
        } else {
            "system"
        };
        params.push(OpenAiCompletionsMessageParam {
            role: role.to_string(),
            content: OpenAiCompletionsMessageContent::Text(sanitize_provider_text(system_prompt)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            extra: BTreeMap::new(),
        });
    }

    let mut last_role: Option<&str> = None;
    let mut index = 0usize;
    while index < transformed_messages.len() {
        let message = &transformed_messages[index];

        if compat.requires_assistant_after_tool_result
            && last_role == Some("toolResult")
            && matches!(message, Message::User { .. })
        {
            params.push(assistant_bridge_message());
        }

        match message {
            Message::User { content, .. } => {
                if let Some(role) = push_user_message(&mut params, content, model) {
                    last_role = Some(role);
                }
                index += 1;
            }
            Message::Assistant { content, .. } => {
                if let Some(role) = push_assistant_message(&mut params, content, compat) {
                    last_role = Some(role);
                }
                index += 1;
            }
            Message::ToolResult { .. } => {
                let mut image_parts = Vec::new();
                while index < transformed_messages.len() {
                    let Message::ToolResult {
                        tool_call_id,
                        tool_name,
                        content,
                        ..
                    } = &transformed_messages[index]
                    else {
                        break;
                    };

                    let text_result = content
                        .iter()
                        .filter_map(|item| match item {
                            UserContent::Text { text } => Some(text.as_str()),
                            UserContent::Image { .. } => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let text_result = sanitize_provider_text(&text_result);
                    let has_images = content
                        .iter()
                        .any(|item| matches!(item, UserContent::Image { .. }));
                    params.push(OpenAiCompletionsMessageParam {
                        role: "tool".into(),
                        content: OpenAiCompletionsMessageContent::Text(if text_result.is_empty() {
                            sanitize_provider_text("(see attached image)")
                        } else {
                            text_result
                        }),
                        tool_calls: None,
                        tool_call_id: Some(tool_call_id.clone()),
                        name: (compat.requires_tool_result_name && !tool_name.is_empty())
                            .then(|| tool_name.clone()),
                        extra: BTreeMap::new(),
                    });

                    if has_images && model.input.iter().any(|input| input == "image") {
                        for block in content {
                            if let UserContent::Image { data, mime_type } = block {
                                image_parts.push(OpenAiCompletionsContentPart::ImageUrl {
                                    image_url: OpenAiCompletionsImageUrl {
                                        url: format!("data:{mime_type};base64,{data}"),
                                    },
                                });
                            }
                        }
                    }

                    index += 1;
                }

                if !image_parts.is_empty() {
                    if compat.requires_assistant_after_tool_result {
                        params.push(assistant_bridge_message());
                    }
                    let mut parts = Vec::with_capacity(image_parts.len() + 1);
                    parts.push(OpenAiCompletionsContentPart::Text {
                        text: "Attached image(s) from tool result:".into(),
                        cache_control: None,
                    });
                    parts.extend(image_parts);
                    params.push(OpenAiCompletionsMessageParam {
                        role: "user".into(),
                        content: OpenAiCompletionsMessageContent::Parts(parts),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                        extra: BTreeMap::new(),
                    });
                    last_role = Some("user");
                } else {
                    last_role = Some("toolResult");
                }
            }
        }
    }

    params
}

pub fn normalize_openai_completions_tool_call_id(model: &Model, id: &str) -> String {
    if let Some((call_id, _)) = id.split_once('|') {
        return normalize_openai_tool_call_part(call_id);
    }
    if model.provider == "openai" {
        return id.chars().take(40).collect();
    }
    id.to_string()
}

fn has_tool_history(messages: &[Message]) -> bool {
    messages.iter().any(|message| match message {
        Message::ToolResult { .. } => true,
        Message::Assistant { content, .. } => content
            .iter()
            .any(|block| matches!(block, AssistantContent::ToolCall { .. })),
        Message::User { .. } => false,
    })
}

fn map_reasoning_effort(effort: ReasoningEffort, compat: &OpenAiCompletionsCompat) -> String {
    compat
        .reasoning_effort_map
        .get(effort.as_str())
        .cloned()
        .unwrap_or_else(|| effort.as_str().to_string())
}

fn maybe_add_openrouter_anthropic_cache_control(
    model: &Model,
    messages: &mut [OpenAiCompletionsMessageParam],
) {
    if model.provider != "openrouter" || !model.id.starts_with("anthropic/") {
        return;
    }

    let cache_control = Some(OpenAiCompletionsCacheControl {
        cache_type: "ephemeral".into(),
    });

    for message in messages.iter_mut().rev() {
        if message.role != "user" && message.role != "assistant" {
            continue;
        }

        match &mut message.content {
            OpenAiCompletionsMessageContent::Text(text) => {
                let text = std::mem::take(text);
                message.content = OpenAiCompletionsMessageContent::Parts(vec![
                    OpenAiCompletionsContentPart::Text {
                        text,
                        cache_control: cache_control.clone(),
                    },
                ]);
                return;
            }
            OpenAiCompletionsMessageContent::Parts(parts) => {
                for part in parts.iter_mut().rev() {
                    if let OpenAiCompletionsContentPart::Text {
                        cache_control: slot,
                        ..
                    } = part
                    {
                        *slot = cache_control.clone();
                        return;
                    }
                }
            }
            OpenAiCompletionsMessageContent::Null(()) => {}
        }
    }
}

fn convert_tools(
    tools: &[ToolDefinition],
    compat: &OpenAiCompletionsCompat,
) -> Vec<OpenAiCompletionsToolDefinition> {
    tools
        .iter()
        .map(|tool| OpenAiCompletionsToolDefinition::Function {
            function: OpenAiCompletionsFunctionDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.parameters.clone(),
                strict: compat.supports_strict_mode.then_some(false),
            },
        })
        .collect()
}

fn convert_user_content(
    content: &[UserContent],
    model: &Model,
) -> Option<OpenAiCompletionsMessageContent> {
    let supports_images = model.input.iter().any(|input| input == "image");
    let has_images = supports_images
        && content
            .iter()
            .any(|item| matches!(item, UserContent::Image { .. }));

    if !has_images {
        let text = content
            .iter()
            .filter_map(|item| match item {
                UserContent::Text { text } => Some(text.as_str()),
                UserContent::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return (!text.is_empty()).then_some(OpenAiCompletionsMessageContent::Text(
            sanitize_provider_text(&text),
        ));
    }

    let mut parts = Vec::new();
    for item in content {
        match item {
            UserContent::Text { text } if !text.is_empty() => {
                parts.push(OpenAiCompletionsContentPart::Text {
                    text: sanitize_provider_text(text),
                    cache_control: None,
                });
            }
            UserContent::Image { data, mime_type } if supports_images => {
                parts.push(OpenAiCompletionsContentPart::ImageUrl {
                    image_url: OpenAiCompletionsImageUrl {
                        url: format!("data:{mime_type};base64,{data}"),
                    },
                });
            }
            _ => {}
        }
    }

    (!parts.is_empty()).then_some(OpenAiCompletionsMessageContent::Parts(parts))
}

fn push_user_message(
    params: &mut Vec<OpenAiCompletionsMessageParam>,
    content: &[UserContent],
    model: &Model,
) -> Option<&'static str> {
    let content = convert_user_content(content, model)?;
    params.push(OpenAiCompletionsMessageParam {
        role: "user".into(),
        content,
        tool_calls: None,
        tool_call_id: None,
        name: None,
        extra: BTreeMap::new(),
    });
    Some("user")
}

// TODO(wave3+): subdivide into text/thinking/tool_calls helpers
#[allow(clippy::too_many_lines)]
fn push_assistant_message(
    params: &mut Vec<OpenAiCompletionsMessageParam>,
    content: &[AssistantContent],
    compat: &OpenAiCompletionsCompat,
) -> Option<&'static str> {
    let mut assistant = OpenAiCompletionsMessageParam {
        role: "assistant".into(),
        content: if compat.requires_assistant_after_tool_result {
            OpenAiCompletionsMessageContent::Text(String::new())
        } else {
            OpenAiCompletionsMessageContent::Null(())
        },
        tool_calls: None,
        tool_call_id: None,
        name: None,
        extra: BTreeMap::new(),
    };

    let text_blocks = content
        .iter()
        .filter_map(|block| match block {
            AssistantContent::Text { text, .. } if !text.trim().is_empty() => {
                Some(sanitize_provider_text(text))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if !text_blocks.is_empty() {
        assistant.content = OpenAiCompletionsMessageContent::Text(text_blocks.join(""));
    }

    let thinking_blocks = content
        .iter()
        .filter_map(|block| match block {
            AssistantContent::Thinking { thinking, .. } if !thinking.trim().is_empty() => {
                Some(sanitize_provider_text(thinking))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if !thinking_blocks.is_empty() {
        if compat.requires_thinking_as_text {
            let thinking_text = thinking_blocks.join("\n\n");
            assistant.content = match assistant.content {
                OpenAiCompletionsMessageContent::Text(ref existing) if !existing.is_empty() => {
                    OpenAiCompletionsMessageContent::Text(format!("{thinking_text}\n\n{existing}"))
                }
                _ => OpenAiCompletionsMessageContent::Text(thinking_text),
            };
        } else if let Some(signature) = content.iter().find_map(|block| match block {
            AssistantContent::Thinking {
                thinking_signature: Some(signature),
                ..
            } if !signature.is_empty() => Some(signature.as_str()),
            _ => None,
        }) {
            assistant.extra.insert(
                signature.to_string(),
                Value::String(thinking_blocks.join("\n")),
            );
        }
    }

    let tool_calls = content
        .iter()
        .filter_map(|block| match block {
            AssistantContent::ToolCall {
                id,
                name,
                arguments,
                thought_signature,
            } => Some((
                OpenAiCompletionsToolCall {
                    id: id.clone(),
                    call_type: "function".into(),
                    function: OpenAiCompletionsToolCallFunction {
                        name: name.clone(),
                        arguments: serde_json::to_string(arguments).unwrap_or_else(|_| "{}".into()),
                    },
                },
                thought_signature.clone(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !tool_calls.is_empty() {
        assistant.tool_calls = Some(
            tool_calls
                .iter()
                .map(|(tool_call, _)| tool_call.clone())
                .collect(),
        );
        let reasoning_details = tool_calls
            .iter()
            .filter_map(|(_, signature)| signature.as_ref())
            .filter_map(|signature| serde_json::from_str::<Value>(signature).ok())
            .collect::<Vec<_>>();
        if !reasoning_details.is_empty() {
            assistant
                .extra
                .insert("reasoning_details".into(), Value::Array(reasoning_details));
        }
    }

    let has_content = match &assistant.content {
        OpenAiCompletionsMessageContent::Text(text) => !text.is_empty(),
        OpenAiCompletionsMessageContent::Parts(parts) => !parts.is_empty(),
        OpenAiCompletionsMessageContent::Null(()) => false,
    };
    if has_content || assistant.tool_calls.is_some() {
        params.push(assistant);
        Some("assistant")
    } else {
        None
    }
}

fn assistant_bridge_message() -> OpenAiCompletionsMessageParam {
    OpenAiCompletionsMessageParam {
        role: "assistant".into(),
        content: OpenAiCompletionsMessageContent::Text("I have processed the tool results.".into()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        extra: BTreeMap::new(),
    }
}

fn transform_messages_for_openai_completions(model: &Model, messages: &[Message]) -> Vec<Message> {
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
                let normalized = tool_call_id_map
                    .get(&tool_call_id)
                    .cloned()
                    .unwrap_or(tool_call_id);
                transformed.push(Message::ToolResult {
                    tool_call_id: normalized,
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
                let is_same_model =
                    provider == model.provider && api == model.api && source_model == model.id;
                let mut transformed_content = Vec::new();

                for block in content {
                    match block {
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
                            if is_same_model && thinking_signature.is_some() {
                                transformed_content.push(AssistantContent::Thinking {
                                    thinking,
                                    thinking_signature,
                                    redacted,
                                });
                            } else if !thinking.trim().is_empty() {
                                if is_same_model {
                                    transformed_content.push(AssistantContent::Thinking {
                                        thinking,
                                        thinking_signature,
                                        redacted,
                                    });
                                } else {
                                    transformed_content.push(AssistantContent::Text {
                                        text: thinking,
                                        text_signature: None,
                                    });
                                }
                            }
                        }
                        AssistantContent::Text {
                            text,
                            text_signature,
                        } => {
                            transformed_content.push(AssistantContent::Text {
                                text,
                                text_signature: if is_same_model { text_signature } else { None },
                            });
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
                                normalize_openai_completions_tool_call_id(model, &id)
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

fn normalize_openai_tool_call_part(id: &str) -> String {
    id.chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => character,
            _ => '_',
        })
        .take(40)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCompletionsChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<OpenAiCompletionsChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<OpenAiCompletionsRawUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsChoice {
    #[serde(default)]
    delta: OpenAiCompletionsDelta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<OpenAiCompletionsRawUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiCompletionsToolCallDeltaChunk>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_details: Option<Vec<OpenAiCompletionsReasoningDetail>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsToolCallDeltaChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    function: Option<OpenAiCompletionsToolCallFunctionDeltaChunk>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsToolCallFunctionDeltaChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    arguments: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsReasoningDetail {
    #[serde(rename = "type")]
    detail_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsRawUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_tokens_details: Option<OpenAiCompletionsPromptTokensDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_tokens_details: Option<OpenAiCompletionsCompletionTokensDetails>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsPromptTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_write_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OpenAiCompletionsCompletionTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAiCompletionsBlockKind {
    Text,
    Thinking,
    ToolCall,
}

#[derive(Debug, Clone)]
struct OpenAiCompletionsStreamState {
    output: AssistantMessage,
    model_cost: ModelCost,
    current_block_index: Option<usize>,
    current_block_kind: Option<OpenAiCompletionsBlockKind>,
    current_tool_json: String,
    current_tool_stream_key: Option<String>,
}

impl OpenAiCompletionsStreamState {
    fn new(model: &Model) -> Self {
        let mut output =
            AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
        output.timestamp = now_ms();
        Self {
            output,
            model_cost: model.cost,
            current_block_index: None,
            current_block_kind: None,
            current_tool_json: String::new(),
            current_tool_stream_key: None,
        }
    }

    fn start_event(&self) -> AssistantEvent {
        AssistantEvent::Start {
            partial: self.output.clone(),
        }
    }

    fn aborted_event(&self) -> AssistantEvent {
        let mut error = self.output.clone();
        error.stop_reason = StopReason::Aborted;
        error.error_message = Some("Request was aborted".into());
        AssistantEvent::Error {
            reason: StopReason::Aborted,
            error,
        }
    }

    fn error_event(&self, error_message: impl Into<String>) -> AssistantEvent {
        let mut error = self.output.clone();
        error.stop_reason = StopReason::Error;
        error.error_message = Some(error_message.into());
        AssistantEvent::Error {
            reason: StopReason::Error,
            error,
        }
    }

    fn process_chunk(&mut self, chunk: OpenAiCompletionsChunk) -> Vec<AssistantEvent> {
        let mut emitted = Vec::new();
        self.output.response_id = chunk.id.or_else(|| self.output.response_id.take());

        if let Some(usage) = chunk.usage.as_ref() {
            self.output.usage = parse_chunk_usage(usage);
            calculate_cost_with(self.model_cost, &mut self.output.usage);
        }

        let Some(choice) = chunk.choices.first() else {
            return emitted;
        };

        if chunk.usage.is_none()
            && let Some(usage) = choice.usage.as_ref()
        {
            self.output.usage = parse_chunk_usage(usage);
            calculate_cost_with(self.model_cost, &mut self.output.usage);
        }

        if let Some(finish_reason) = choice.finish_reason.as_deref() {
            let (stop_reason, error_message) = map_stop_reason(finish_reason);
            self.output.stop_reason = stop_reason;
            if let Some(error_message) = error_message {
                self.output.error_message = Some(error_message);
            }
        }

        if let Some(content) = choice
            .delta
            .content
            .as_deref()
            .filter(|content| !content.is_empty())
        {
            self.ensure_text_block(&mut emitted);
            if let Some(index) = self.current_block_index {
                if let Some(AssistantContent::Text { text, .. }) =
                    self.output.content.get_mut(index)
                {
                    text.push_str(content);
                }
                emitted.push(AssistantEvent::TextDelta {
                    content_index: index,
                    delta: content.to_string(),
                    partial: self.output.clone(),
                });
            }
        }

        if let Some((reasoning_field, delta)) = reasoning_delta(&choice.delta) {
            self.ensure_thinking_block(reasoning_field, &mut emitted);
            if let Some(index) = self.current_block_index {
                if let Some(AssistantContent::Thinking { thinking, .. }) =
                    self.output.content.get_mut(index)
                {
                    thinking.push_str(delta);
                }
                emitted.push(AssistantEvent::ThinkingDelta {
                    content_index: index,
                    delta: delta.to_string(),
                    partial: self.output.clone(),
                });
            }
        }

        if let Some(tool_calls) = choice.delta.tool_calls.as_ref() {
            for tool_call in tool_calls {
                self.process_tool_call_delta(tool_call, &mut emitted);
            }
        }

        if let Some(reasoning_details) = choice.delta.reasoning_details.as_ref() {
            for detail in reasoning_details {
                if detail.detail_type != "reasoning.encrypted" {
                    continue;
                }
                let Some(id) = detail.id.as_deref() else {
                    continue;
                };
                if detail.data.as_deref().is_none() {
                    continue;
                }
                let Some(signature) = serde_json::to_string(detail).ok() else {
                    continue;
                };
                if let Some(AssistantContent::ToolCall {
                    thought_signature, ..
                }) = self.output.content.iter_mut().find(|block| {
                    matches!(
                        block,
                        AssistantContent::ToolCall {
                            id: tool_call_id,
                            ..
                        } if tool_call_id == id
                    )
                }) {
                    *thought_signature = Some(signature);
                }
            }
        }

        emitted
    }

    fn finish_stream(&mut self) -> Vec<AssistantEvent> {
        let mut emitted = Vec::new();
        self.finish_current_block(&mut emitted);

        match self.output.stop_reason.clone() {
            StopReason::Error => emitted.push(AssistantEvent::Error {
                reason: StopReason::Error,
                error: self.output.clone(),
            }),
            StopReason::Aborted => emitted.push(self.aborted_event()),
            reason => emitted.push(AssistantEvent::Done {
                reason,
                message: self.output.clone(),
            }),
        }

        emitted
    }

    fn ensure_text_block(&mut self, emitted: &mut Vec<AssistantEvent>) {
        if self.current_block_kind == Some(OpenAiCompletionsBlockKind::Text) {
            return;
        }

        self.finish_current_block(emitted);
        self.output.content.push(AssistantContent::Text {
            text: String::new(),
            text_signature: None,
        });
        let index = self.output.content.len() - 1;
        self.current_block_index = Some(index);
        self.current_block_kind = Some(OpenAiCompletionsBlockKind::Text);
        emitted.push(AssistantEvent::TextStart {
            content_index: index,
            partial: self.output.clone(),
        });
    }

    fn ensure_thinking_block(&mut self, reasoning_field: &str, emitted: &mut Vec<AssistantEvent>) {
        if self.current_block_kind == Some(OpenAiCompletionsBlockKind::Thinking) {
            return;
        }

        self.finish_current_block(emitted);
        self.output.content.push(AssistantContent::Thinking {
            thinking: String::new(),
            thinking_signature: Some(reasoning_field.to_string()),
            redacted: false,
        });
        let index = self.output.content.len() - 1;
        self.current_block_index = Some(index);
        self.current_block_kind = Some(OpenAiCompletionsBlockKind::Thinking);
        emitted.push(AssistantEvent::ThinkingStart {
            content_index: index,
            partial: self.output.clone(),
        });
    }

    fn process_tool_call_delta(
        &mut self,
        tool_call: &OpenAiCompletionsToolCallDeltaChunk,
        emitted: &mut Vec<AssistantEvent>,
    ) {
        let tool_stream_key = tool_call
            .id
            .as_ref()
            .filter(|id| !id.is_empty())
            .cloned()
            .or_else(|| tool_call.index.map(|index| format!("index:{index}")));
        let needs_new_block = self.current_block_kind != Some(OpenAiCompletionsBlockKind::ToolCall)
            || tool_stream_key.is_some() && tool_stream_key != self.current_tool_stream_key;

        if needs_new_block {
            self.finish_current_block(emitted);
            self.output.content.push(AssistantContent::ToolCall {
                id: tool_call.id.clone().unwrap_or_default(),
                name: tool_call
                    .function
                    .as_ref()
                    .and_then(|function| function.name.clone())
                    .unwrap_or_default(),
                arguments: BTreeMap::new(),
                thought_signature: None,
            });
            let index = self.output.content.len() - 1;
            self.current_block_index = Some(index);
            self.current_block_kind = Some(OpenAiCompletionsBlockKind::ToolCall);
            self.current_tool_stream_key = tool_stream_key;
            self.current_tool_json.clear();
            emitted.push(AssistantEvent::ToolCallStart {
                content_index: index,
                partial: self.output.clone(),
            });
        }

        let Some(index) = self.current_block_index else {
            return;
        };

        let delta = tool_call
            .function
            .as_ref()
            .and_then(|function| function.arguments.as_deref())
            .unwrap_or_default()
            .to_string();
        self.current_tool_json.push_str(&delta);

        if let Some(AssistantContent::ToolCall {
            id,
            name,
            arguments,
            ..
        }) = self.output.content.get_mut(index)
        {
            if let Some(next_id) = tool_call.id.as_ref().filter(|next_id| !next_id.is_empty()) {
                *id = next_id.clone();
            }
            if let Some(next_name) = tool_call
                .function
                .as_ref()
                .and_then(|function| function.name.as_ref())
                .filter(|next_name| !next_name.is_empty())
            {
                *name = next_name.clone();
            }
            *arguments = parse_streaming_json_map(&self.current_tool_json);
        }

        emitted.push(AssistantEvent::ToolCallDelta {
            content_index: index,
            delta,
            partial: self.output.clone(),
        });
    }

    fn finish_current_block(&mut self, emitted: &mut Vec<AssistantEvent>) {
        let Some(index) = self.current_block_index else {
            self.reset_current_block();
            return;
        };

        match self.current_block_kind {
            Some(OpenAiCompletionsBlockKind::Text) => {
                let content = text_content(&self.output, index).unwrap_or_default();
                emitted.push(AssistantEvent::TextEnd {
                    content_index: index,
                    content,
                    partial: self.output.clone(),
                });
            }
            Some(OpenAiCompletionsBlockKind::Thinking) => {
                let content = thinking_content(&self.output, index).unwrap_or_default();
                emitted.push(AssistantEvent::ThinkingEnd {
                    content_index: index,
                    content,
                    partial: self.output.clone(),
                });
            }
            Some(OpenAiCompletionsBlockKind::ToolCall) => {
                let mut tool_call = AssistantContent::ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: BTreeMap::new(),
                    thought_signature: None,
                };
                if let Some(content) = self.output.content.get_mut(index) {
                    if let AssistantContent::ToolCall { arguments, .. } = content {
                        *arguments = parse_streaming_json_map(&self.current_tool_json);
                    }
                    tool_call = content.clone();
                }
                emitted.push(AssistantEvent::ToolCallEnd {
                    content_index: index,
                    tool_call,
                    partial: self.output.clone(),
                });
            }
            None => {}
        }

        self.reset_current_block();
    }

    fn reset_current_block(&mut self) {
        self.current_block_index = None;
        self.current_block_kind = None;
        self.current_tool_json.clear();
        self.current_tool_stream_key = None;
    }
}

#[derive(Default)]
struct OpenAiCompletionsSseDecoder {
    buffer: Vec<u8>,
    current_data_lines: Vec<String>,
}

impl OpenAiCompletionsSseDecoder {
    fn push_bytes(&mut self, chunk: &[u8]) -> Result<Vec<OpenAiCompletionsChunk>, crate::AiError> {
        self.buffer.extend_from_slice(chunk);

        let mut chunks = Vec::new();
        let mut line_start = 0usize;
        let mut consumed = 0usize;

        while let Some(relative_newline) = self.buffer[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let newline = line_start + relative_newline;
            let line = self.buffer[line_start..newline].to_vec();
            self.process_line(&line, &mut chunks)?;
            line_start = newline + 1;
            consumed = line_start;
        }

        if consumed > 0 {
            self.buffer.drain(..consumed);
        }

        Ok(chunks)
    }

    fn finish(&mut self) -> Result<Vec<OpenAiCompletionsChunk>, crate::AiError> {
        let mut chunks = Vec::new();

        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.process_line(&line, &mut chunks)?;
        }

        if let Some(chunk) = flush_openai_completions_sse_event(&mut self.current_data_lines)? {
            chunks.push(chunk);
        }

        Ok(chunks)
    }

    fn process_line(
        &mut self,
        line: &[u8],
        chunks: &mut Vec<OpenAiCompletionsChunk>,
    ) -> Result<(), crate::AiError> {
        let line = match line.strip_suffix(b"\r") {
            Some(stripped) => stripped,
            None => line,
        };
        let line = std::str::from_utf8(line).map_err(|error| {
            crate::AiError::Message(format!(
                "Invalid UTF-8 in OpenAI Completions SSE stream: {error}"
            ))
        })?;

        if line.is_empty() {
            if let Some(chunk) = flush_openai_completions_sse_event(&mut self.current_data_lines)? {
                chunks.push(chunk);
            }
            return Ok(());
        }

        if let Some(data) = line.strip_prefix("data:") {
            self.current_data_lines.push(data.trim_start().to_string());
        }

        Ok(())
    }
}

pub fn parse_openai_completions_sse_text(
    payload: &str,
) -> Result<Vec<OpenAiCompletionsChunk>, crate::AiError> {
    let mut decoder = OpenAiCompletionsSseDecoder::default();
    let mut chunks = decoder.push_bytes(payload.as_bytes())?;
    chunks.extend(decoder.finish()?);
    Ok(chunks)
}

pub fn stream_openai_completions_sse_text(
    model: Model,
    payload: &str,
) -> Result<AssistantEventStream, crate::AiError> {
    let chunks = parse_openai_completions_sse_text(payload)?;
    Ok(stream_openai_completions_chunks(model, chunks))
}

pub fn stream_openai_completions_chunks(
    model: Model,
    chunks: Vec<OpenAiCompletionsChunk>,
) -> AssistantEventStream {
    Box::pin(stream! {
        let mut state = OpenAiCompletionsStreamState::new(&model);
        yield Ok(state.start_event());

        for chunk in chunks {
            let emitted = state.process_chunk(chunk);
            for assistant_event in emitted {
                let is_terminal = is_terminal_event(&assistant_event);
                yield Ok(assistant_event);
                if is_terminal {
                    return;
                }
            }
        }

        for assistant_event in state.finish_stream() {
            let is_terminal = is_terminal_event(&assistant_event);
            yield Ok(assistant_event);
            if is_terminal {
                return;
            }
        }
    })
}

pub fn stream_openai_completions_http<T>(
    model: Model,
    params: T,
    api_key: String,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    stream_openai_completions_http_with_headers(model, params, api_key, signal, BTreeMap::new())
}

pub fn stream_openai_completions_http_with_headers<T>(
    model: Model,
    params: T,
    api_key: String,
    signal: Option<tokio::sync::watch::Receiver<bool>>,
    request_headers: BTreeMap<String, String>,
) -> AssistantEventStream
where
    T: Serialize + Send + Sync + 'static,
{
    Box::pin(stream! {
        let mut signal = signal;
        let mut state = OpenAiCompletionsStreamState::new(&model);

        if is_signal_aborted(&signal) {
            yield Ok(state.aborted_event());
            return;
        }

        let mut request_builder = reqwest::Client::new()
            .post(format!("{}/chat/completions", model.base_url.trim_end_matches('/')))
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

        let mut decoder = OpenAiCompletionsSseDecoder::default();

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
                    let chunks = match decoder.push_bytes(chunk.as_ref()) {
                        Ok(chunks) => chunks,
                        Err(error) => {
                            yield Ok(state.error_event(error.to_string()));
                            return;
                        }
                    };

                    for chunk in chunks {
                        if is_signal_aborted(&signal) {
                            yield Ok(state.aborted_event());
                            return;
                        }
                        let emitted = state.process_chunk(chunk);
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
                    let chunks = match decoder.finish() {
                        Ok(chunks) => chunks,
                        Err(error) => {
                            yield Ok(state.error_event(error.to_string()));
                            return;
                        }
                    };
                    for chunk in chunks {
                        if is_signal_aborted(&signal) {
                            yield Ok(state.aborted_event());
                            return;
                        }
                        let emitted = state.process_chunk(chunk);
                        for assistant_event in emitted {
                            let is_terminal = is_terminal_event(&assistant_event);
                            yield Ok(assistant_event);
                            if is_terminal {
                                return;
                            }
                        }
                    }
                    for assistant_event in state.finish_stream() {
                        let is_terminal = is_terminal_event(&assistant_event);
                        yield Ok(assistant_event);
                        if is_terminal {
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

#[derive(Default)]
pub struct OpenAiCompletionsProvider;

impl AiProvider for OpenAiCompletionsProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        Box::pin(stream! {
            let compat = detect_openai_completions_compat(&model);
            let params = build_openai_completions_request_params(
                &model,
                &context,
                &compat,
                &OpenAiCompletionsRequestOptions {
                    tool_choice: options.tool_choice.clone(),
                    reasoning_effort: parse_reasoning_effort(options.reasoning_effort.as_deref()),
                    max_tokens: options.max_tokens,
                    temperature: options.temperature,
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
            let request_headers = build_runtime_request_headers(&model, &options.headers);

            let api_key = options
                .api_key
                .clone()
                .or_else(|| crate::get_env_api_key(&model.provider));

            let mut inner = match api_key {
                Some(api_key) => stream_openai_completions_http_with_headers(
                    model,
                    payload,
                    api_key,
                    options.signal.clone(),
                    request_headers,
                ),
                None => Box::pin(stream! {
                    yield Ok(AssistantEvent::Error {
                        reason: StopReason::Error,
                        error: error_message(&model, "OpenAI Completions API key is required".into()),
                    });
                }),
            };

            while let Some(event) = inner.next().await {
                yield event;
            }
        })
    }
}

pub fn register_openai_completions_provider() {
    register_provider("openai-completions", Arc::new(OpenAiCompletionsProvider));
}

fn build_runtime_request_headers(
    model: &Model,
    option_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut headers = get_model_headers(&model.provider, &model.id)
        .or_else(|| get_provider_headers(&model.provider))
        .unwrap_or_default();
    headers.extend(option_headers.clone());
    headers
}

fn parse_reasoning_effort(value: Option<&str>) -> Option<ReasoningEffort> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::Xhigh),
        _ => None,
    }
}

fn reasoning_delta(delta: &OpenAiCompletionsDelta) -> Option<(&'static str, &str)> {
    [
        ("reasoning_content", delta.reasoning_content.as_deref()),
        ("reasoning", delta.reasoning.as_deref()),
        ("reasoning_text", delta.reasoning_text.as_deref()),
    ]
    .into_iter()
    .find_map(|(field, value)| {
        value
            .filter(|value| !value.is_empty())
            .map(|value| (field, value))
    })
}

fn flush_openai_completions_sse_event(
    current_data_lines: &mut Vec<String>,
) -> Result<Option<OpenAiCompletionsChunk>, crate::AiError> {
    if current_data_lines.is_empty() {
        return Ok(None);
    }

    let payload = current_data_lines.join("\n");
    current_data_lines.clear();

    if payload == "[DONE]" {
        return Ok(None);
    }

    serde_json::from_str::<OpenAiCompletionsChunk>(&payload)
        .map(Some)
        .map_err(|error| {
            crate::AiError::Message(format!("Invalid OpenAI Completions SSE event: {error}"))
        })
}

fn parse_streaming_json_map(input: &str) -> BTreeMap<String, Value> {
    crate::partial_json::parse_partial_json_map(input)
}

fn parse_chunk_usage(raw_usage: &OpenAiCompletionsRawUsage) -> Usage {
    let prompt_tokens = raw_usage.prompt_tokens.unwrap_or(0);
    let reported_cached_tokens = raw_usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens)
        .unwrap_or(0);
    let cache_write_tokens = raw_usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cache_write_tokens)
        .unwrap_or(0);
    let reasoning_tokens = raw_usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens)
        .unwrap_or(0);
    let cache_read_tokens = if cache_write_tokens > 0 {
        reported_cached_tokens.saturating_sub(cache_write_tokens)
    } else {
        reported_cached_tokens
    };
    let input = prompt_tokens.saturating_sub(cache_read_tokens + cache_write_tokens);
    let output = raw_usage.completion_tokens.unwrap_or(0) + reasoning_tokens;

    Usage {
        input,
        output,
        cache_read: cache_read_tokens,
        cache_write: cache_write_tokens,
        total_tokens: input + output + cache_read_tokens + cache_write_tokens,
        ..Usage::default()
    }
}

fn map_stop_reason(reason: &str) -> (StopReason, Option<String>) {
    match reason {
        "stop" | "end" => (StopReason::Stop, None),
        "length" => (StopReason::Length, None),
        "function_call" | "tool_calls" => (StopReason::ToolUse, None),
        "content_filter" => (
            StopReason::Error,
            Some("Provider finish_reason: content_filter".into()),
        ),
        "network_error" => (
            StopReason::Error,
            Some("Provider finish_reason: network_error".into()),
        ),
        other => (
            StopReason::Error,
            Some(format!("Provider finish_reason: {other}")),
        ),
    }
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
