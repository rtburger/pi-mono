use pi_events::{
    AssistantContent, Context, Message, Model, StopReason, ToolDefinition, UserContent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompletionsMaxTokensField {
    MaxCompletionTokens,
    MaxTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenAiCompletionsThinkingFormat {
    Openai,
    Openrouter,
    Zai,
    Qwen,
    QwenChatTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsCompat {
    pub supports_store: bool,
    pub supports_developer_role: bool,
    pub supports_reasoning_effort: bool,
    pub reasoning_effort_map: BTreeMap<ReasoningEffort, String>,
    pub supports_usage_in_streaming: bool,
    pub max_tokens_field: OpenAiCompletionsMaxTokensField,
    pub requires_tool_result_name: bool,
    pub requires_assistant_after_tool_result: bool,
    pub requires_thinking_as_text: bool,
    pub thinking_format: OpenAiCompletionsThinkingFormat,
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
            thinking_format: OpenAiCompletionsThinkingFormat::Openai,
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
    pub tool_stream: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsStreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsReasoning {
    pub effort: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiCompletionsChatTemplateKwargs {
    pub enable_thinking: bool,
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
    let is_grok = provider == "xai" || base_url.contains("api.x.ai");
    let is_groq = provider == "groq" || base_url.contains("groq.com");

    let mut compat = OpenAiCompletionsCompat {
        supports_store: !is_non_standard,
        supports_developer_role: !is_non_standard,
        supports_reasoning_effort: !is_grok && !is_zai,
        max_tokens_field: if base_url.contains("chutes.ai") {
            OpenAiCompletionsMaxTokensField::MaxTokens
        } else {
            OpenAiCompletionsMaxTokensField::MaxCompletionTokens
        },
        thinking_format: if is_zai {
            OpenAiCompletionsThinkingFormat::Zai
        } else if provider == "openrouter" || base_url.contains("openrouter.ai") {
            OpenAiCompletionsThinkingFormat::Openrouter
        } else {
            OpenAiCompletionsThinkingFormat::Openai
        },
        zai_tool_stream: is_zai
            && matches!(
                model.id.as_str(),
                "glm-5" | "glm-4.7" | "glm-4.7-flash" | "glm-4.6v"
            ),
        ..OpenAiCompletionsCompat::default()
    };

    if is_groq && model.id == "qwen/qwen3-32b" {
        for effort in [
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ] {
            compat
                .reasoning_effort_map
                .insert(effort, "default".to_string());
        }
    }

    compat
}

pub fn build_openai_completions_request_params(
    model: &Model,
    context: &Context,
    compat: &OpenAiCompletionsCompat,
    options: &OpenAiCompletionsRequestOptions,
) -> OpenAiCompletionsRequestParams {
    let messages = convert_openai_completions_messages(model, context, compat);
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

    OpenAiCompletionsRequestParams {
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
        reasoning_effort: if model.reasoning
            && compat.supports_reasoning_effort
            && matches!(
                compat.thinking_format,
                OpenAiCompletionsThinkingFormat::Openai
            ) {
            mapped_reasoning_effort.clone()
        } else {
            None
        },
        reasoning: if model.reasoning
            && matches!(
                compat.thinking_format,
                OpenAiCompletionsThinkingFormat::Openrouter
            ) {
            Some(OpenAiCompletionsReasoning {
                effort: mapped_reasoning_effort
                    .clone()
                    .unwrap_or_else(|| "none".into()),
            })
        } else {
            None
        },
        enable_thinking: if model.reasoning
            && matches!(
                compat.thinking_format,
                OpenAiCompletionsThinkingFormat::Zai | OpenAiCompletionsThinkingFormat::Qwen
            ) {
            Some(options.reasoning_effort.is_some())
        } else {
            None
        },
        chat_template_kwargs: if model.reasoning
            && matches!(
                compat.thinking_format,
                OpenAiCompletionsThinkingFormat::QwenChatTemplate
            ) {
            Some(OpenAiCompletionsChatTemplateKwargs {
                enable_thinking: options.reasoning_effort.is_some(),
            })
        } else {
            None
        },
        tool_stream: (compat.zai_tool_stream && !context.tools.is_empty()).then_some(true),
    }
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
            content: OpenAiCompletionsMessageContent::Text(system_prompt.clone()),
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
                if let Some(content) = convert_user_content(content, model) {
                    params.push(OpenAiCompletionsMessageParam {
                        role: "user".into(),
                        content,
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                        extra: BTreeMap::new(),
                    });
                    last_role = Some("user");
                }
                index += 1;
            }
            Message::Assistant { content, .. } => {
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
                            Some(text.as_str())
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
                        AssistantContent::Thinking { thinking, .. }
                            if !thinking.trim().is_empty() =>
                        {
                            Some(thinking.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if !thinking_blocks.is_empty() {
                    if compat.requires_thinking_as_text {
                        let thinking_text = thinking_blocks.join("\n\n");
                        assistant.content = match assistant.content {
                            OpenAiCompletionsMessageContent::Text(ref existing)
                                if !existing.is_empty() =>
                            {
                                OpenAiCompletionsMessageContent::Text(format!(
                                    "{thinking_text}\n\n{existing}"
                                ))
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
                                    arguments: serde_json::to_string(arguments)
                                        .unwrap_or_else(|_| "{}".into()),
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
                    last_role = Some("assistant");
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
                    let has_images = content
                        .iter()
                        .any(|item| matches!(item, UserContent::Image { .. }));
                    params.push(OpenAiCompletionsMessageParam {
                        role: "tool".into(),
                        content: OpenAiCompletionsMessageContent::Text(if text_result.is_empty() {
                            "(see attached image)".into()
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
        .get(&effort)
        .cloned()
        .unwrap_or_else(|| effort.as_str().to_string())
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
        return (!text.is_empty()).then_some(OpenAiCompletionsMessageContent::Text(text));
    }

    let mut parts = Vec::new();
    for item in content {
        match item {
            UserContent::Text { text } if !text.is_empty() => {
                parts.push(OpenAiCompletionsContentPart::Text { text: text.clone() });
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
