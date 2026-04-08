use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Api = String;
pub type Provider = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub cost: UsageCost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistantContent {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        text_signature: Option<String>,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking_signature: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        redacted: bool,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: BTreeMap<String, serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserContent {
    Text { text: String },
    Image { data: String, mime_type: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub role: String,
    pub content: Vec<AssistantContent>,
    pub api: Api,
    pub provider: Provider,
    pub model: String,
    pub response_id: Option<String>,
    pub usage: Usage,
    pub stop_reason: StopReason,
    pub error_message: Option<String>,
    pub timestamp: u64,
}

impl AssistantMessage {
    pub fn empty(
        api: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content: Vec::new(),
            api: api.into(),
            provider: provider.into(),
            model: model.into(),
            response_id: None,
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    pub role: String,
    pub content: Vec<UserContent>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub role: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<UserContent>,
    pub is_error: bool,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum Message {
    #[serde(rename = "user")]
    User {
        content: Vec<UserContent>,
        timestamp: u64,
    },
    #[serde(rename = "assistant")]
    Assistant {
        content: Vec<AssistantContent>,
        api: Api,
        provider: Provider,
        model: String,
        response_id: Option<String>,
        usage: Usage,
        stop_reason: StopReason,
        error_message: Option<String>,
        timestamp: u64,
    },
    #[serde(rename = "toolResult")]
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        content: Vec<UserContent>,
        is_error: bool,
        timestamp: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: Api,
    pub provider: Provider,
    pub base_url: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    pub context_window: u64,
    pub max_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Context {
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantEvent {
    Start {
        partial: AssistantMessage,
    },
    TextStart {
        content_index: usize,
        partial: AssistantMessage,
    },
    TextDelta {
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    TextEnd {
        content_index: usize,
        content: String,
        partial: AssistantMessage,
    },
    ThinkingStart {
        content_index: usize,
        partial: AssistantMessage,
    },
    ThinkingDelta {
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    ThinkingEnd {
        content_index: usize,
        content: String,
        partial: AssistantMessage,
    },
    ToolCallStart {
        content_index: usize,
        partial: AssistantMessage,
    },
    ToolCallDelta {
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    ToolCallEnd {
        content_index: usize,
        tool_call: AssistantContent,
        partial: AssistantMessage,
    },
    Done {
        reason: StopReason,
        message: AssistantMessage,
    },
    Error {
        reason: StopReason,
        error: AssistantMessage,
    },
}
