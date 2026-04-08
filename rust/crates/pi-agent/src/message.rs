use pi_events::Message;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct CustomAgentMessage {
    pub role: String,
    pub payload: Value,
    pub timestamp: u64,
}

impl CustomAgentMessage {
    pub fn new(role: impl Into<String>, payload: Value, timestamp: u64) -> Self {
        Self {
            role: role.into(),
            payload,
            timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentMessage {
    Standard(Message),
    Custom(CustomAgentMessage),
}

impl AgentMessage {
    pub fn custom(role: impl Into<String>, payload: Value, timestamp: u64) -> Self {
        Self::Custom(CustomAgentMessage::new(role, payload, timestamp))
    }

    pub fn role(&self) -> &str {
        match self {
            Self::Standard(Message::User { .. }) => "user",
            Self::Standard(Message::Assistant { .. }) => "assistant",
            Self::Standard(Message::ToolResult { .. }) => "toolResult",
            Self::Custom(message) => &message.role,
        }
    }

    pub fn timestamp(&self) -> u64 {
        match self {
            Self::Standard(Message::User { timestamp, .. }) => *timestamp,
            Self::Standard(Message::Assistant { timestamp, .. }) => *timestamp,
            Self::Standard(Message::ToolResult { timestamp, .. }) => *timestamp,
            Self::Custom(message) => message.timestamp,
        }
    }

    pub fn is_assistant(&self) -> bool {
        matches!(self, Self::Standard(Message::Assistant { .. }))
    }

    pub fn as_standard_message(&self) -> Option<&Message> {
        match self {
            Self::Standard(message) => Some(message),
            Self::Custom(_) => None,
        }
    }

    pub fn into_standard_message(self) -> Option<Message> {
        match self {
            Self::Standard(message) => Some(message),
            Self::Custom(_) => None,
        }
    }
}

impl From<Message> for AgentMessage {
    fn from(message: Message) -> Self {
        Self::Standard(message)
    }
}

impl From<CustomAgentMessage> for AgentMessage {
    fn from(message: CustomAgentMessage) -> Self {
        Self::Custom(message)
    }
}
