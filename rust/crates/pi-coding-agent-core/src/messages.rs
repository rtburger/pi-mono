use pi_agent::{AgentMessage, CustomAgentPayload};
use pi_events::{Message, UserContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const COMPACTION_SUMMARY_PREFIX: &str = "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";
pub const COMPACTION_SUMMARY_SUFFIX: &str = "\n</summary>";

pub const BRANCH_SUMMARY_PREFIX: &str =
    "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n";
pub const BRANCH_SUMMARY_SUFFIX: &str = "</summary>";
pub const BLOCKED_IMAGE_PLACEHOLDER: &str = "Image reading is disabled.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashExecutionMessage {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i64>,
    pub cancelled: bool,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
    #[serde(default)]
    pub exclude_from_context: bool,
}

impl CustomAgentPayload for BashExecutionMessage {
    const ROLE: &'static str = "bashExecution";
}

impl BashExecutionMessage {
    pub fn into_agent_message(self, timestamp: u64) -> AgentMessage {
        custom_agent_message(self, timestamp)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CustomMessageContent {
    Text(String),
    Blocks(Vec<UserContent>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessage {
    pub custom_type: String,
    pub content: CustomMessageContent,
    pub display: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl CustomAgentPayload for CustomMessage {
    const ROLE: &'static str = "custom";
}

impl CustomMessage {
    pub fn into_agent_message(self, timestamp: u64) -> AgentMessage {
        custom_agent_message(self, timestamp)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryMessage {
    pub summary: String,
    pub from_id: String,
}

impl CustomAgentPayload for BranchSummaryMessage {
    const ROLE: &'static str = "branchSummary";
}

impl BranchSummaryMessage {
    pub fn into_agent_message(self, timestamp: u64) -> AgentMessage {
        custom_agent_message(self, timestamp)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummaryMessage {
    pub summary: String,
    pub tokens_before: u64,
}

impl CustomAgentPayload for CompactionSummaryMessage {
    const ROLE: &'static str = "compactionSummary";
}

impl CompactionSummaryMessage {
    pub fn into_agent_message(self, timestamp: u64) -> AgentMessage {
        custom_agent_message(self, timestamp)
    }
}

pub fn create_bash_execution_message(
    command: impl Into<String>,
    output: impl Into<String>,
    exit_code: Option<i64>,
    cancelled: bool,
    truncated: bool,
    full_output_path: Option<String>,
    exclude_from_context: bool,
    timestamp: u64,
) -> AgentMessage {
    BashExecutionMessage {
        command: command.into(),
        output: output.into(),
        exit_code,
        cancelled,
        truncated,
        full_output_path,
        exclude_from_context,
    }
    .into_agent_message(timestamp)
}

pub fn create_custom_message(
    custom_type: impl Into<String>,
    content: CustomMessageContent,
    display: bool,
    details: Option<Value>,
    timestamp: u64,
) -> AgentMessage {
    CustomMessage {
        custom_type: custom_type.into(),
        content,
        display,
        details,
    }
    .into_agent_message(timestamp)
}

pub fn create_branch_summary_message(
    summary: impl Into<String>,
    from_id: impl Into<String>,
    timestamp: u64,
) -> AgentMessage {
    BranchSummaryMessage {
        summary: summary.into(),
        from_id: from_id.into(),
    }
    .into_agent_message(timestamp)
}

pub fn create_compaction_summary_message(
    summary: impl Into<String>,
    tokens_before: u64,
    timestamp: u64,
) -> AgentMessage {
    CompactionSummaryMessage {
        summary: summary.into(),
        tokens_before,
    }
    .into_agent_message(timestamp)
}

pub fn bash_execution_to_text(message: &BashExecutionMessage) -> String {
    let mut text = format!("Ran `{}`\n", message.command);
    if message.output.is_empty() {
        text.push_str("(no output)");
    } else {
        text.push_str(&format!("```\n{}\n```", message.output));
    }

    if message.cancelled {
        text.push_str("\n\n(command cancelled)");
    } else if let Some(exit_code) = message.exit_code.filter(|exit_code| *exit_code != 0) {
        text.push_str(&format!("\n\nCommand exited with code {exit_code}"));
    }

    if message.truncated {
        if let Some(full_output_path) = &message.full_output_path {
            text.push_str(&format!(
                "\n\n[Output truncated. Full output: {full_output_path}]"
            ));
        }
    }

    text
}

pub fn convert_to_llm(messages: Vec<AgentMessage>) -> Vec<Message> {
    messages
        .into_iter()
        .filter_map(convert_message_to_llm)
        .collect()
}

pub fn filter_blocked_images(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .map(filter_blocked_images_in_message)
        .collect()
}

fn convert_message_to_llm(message: AgentMessage) -> Option<Message> {
    match message {
        AgentMessage::Standard(message) => Some(message),
        AgentMessage::Custom(message) => convert_custom_message_to_llm(message),
    }
}

fn convert_custom_message_to_llm(message: pi_agent::CustomAgentMessage) -> Option<Message> {
    let timestamp = message.timestamp;

    if let Some(payload) = message
        .decode_typed_payload::<BashExecutionMessage>()
        .ok()
        .flatten()
    {
        if payload.exclude_from_context {
            return None;
        }
        return Some(Message::User {
            content: vec![UserContent::Text {
                text: bash_execution_to_text(&payload),
            }],
            timestamp,
        });
    }

    if let Some(payload) = message
        .decode_typed_payload::<CustomMessage>()
        .ok()
        .flatten()
    {
        let content = match payload.content {
            CustomMessageContent::Text(text) => vec![UserContent::Text { text }],
            CustomMessageContent::Blocks(content) => content,
        };
        return Some(Message::User { content, timestamp });
    }

    if let Some(payload) = message
        .decode_typed_payload::<BranchSummaryMessage>()
        .ok()
        .flatten()
    {
        return Some(Message::User {
            content: vec![UserContent::Text {
                text: format!(
                    "{BRANCH_SUMMARY_PREFIX}{}{BRANCH_SUMMARY_SUFFIX}",
                    payload.summary
                ),
            }],
            timestamp,
        });
    }

    if let Some(payload) = message
        .decode_typed_payload::<CompactionSummaryMessage>()
        .ok()
        .flatten()
    {
        return Some(Message::User {
            content: vec![UserContent::Text {
                text: format!(
                    "{COMPACTION_SUMMARY_PREFIX}{}{COMPACTION_SUMMARY_SUFFIX}",
                    payload.summary
                ),
            }],
            timestamp,
        });
    }

    None
}

fn filter_blocked_images_in_message(message: Message) -> Message {
    match message {
        Message::User { content, timestamp } => Message::User {
            content: filter_blocked_images_in_content(content),
            timestamp,
        },
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            details,
            is_error,
            timestamp,
        } => Message::ToolResult {
            tool_call_id,
            tool_name,
            content: filter_blocked_images_in_content(content),
            details,
            is_error,
            timestamp,
        },
        Message::Assistant { .. } => message,
    }
}

fn filter_blocked_images_in_content(content: Vec<UserContent>) -> Vec<UserContent> {
    let mut filtered = Vec::with_capacity(content.len());

    for block in content {
        match block {
            UserContent::Image { .. } => push_blocked_image_placeholder(&mut filtered),
            UserContent::Text { .. } => filtered.push(block),
        }
    }

    filtered
}

fn push_blocked_image_placeholder(content: &mut Vec<UserContent>) {
    if content.last().is_some_and(
        |block| matches!(block, UserContent::Text { text } if text == BLOCKED_IMAGE_PLACEHOLDER),
    ) {
        return;
    }

    content.push(UserContent::Text {
        text: BLOCKED_IMAGE_PLACEHOLDER.into(),
    });
}

fn custom_agent_message<T>(payload: T, timestamp: u64) -> AgentMessage
where
    T: CustomAgentPayload,
{
    AgentMessage::typed_custom(payload, timestamp)
        .expect("coding-agent message payload should always serialize")
}
