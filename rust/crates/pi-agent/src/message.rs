use pi_events::Message;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Typed custom-message payload contract.
///
/// This is the Rust counterpart to TypeScript declaration-merging custom
/// messages: apps declare a payload type, bind it to a role string, then use
/// `CustomAgentMessage::typed()` or `AgentMessage::typed_custom()` to move
/// between typed payloads and the raw JSON-backed agent transcript.
pub trait CustomAgentPayload: Serialize + DeserializeOwned {
    const ROLE: &'static str;
}

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

    pub fn encode<T>(
        role: impl Into<String>,
        payload: T,
        timestamp: u64,
    ) -> serde_json::Result<Self>
    where
        T: Serialize,
    {
        Ok(Self::new(role, serde_json::to_value(payload)?, timestamp))
    }

    pub fn typed<T>(payload: T, timestamp: u64) -> serde_json::Result<Self>
    where
        T: CustomAgentPayload,
    {
        Self::encode(T::ROLE, payload, timestamp)
    }

    pub fn is_role(&self, role: &str) -> bool {
        self.role == role
    }

    pub fn decode_payload<T>(&self) -> serde_json::Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(self.payload.clone())
    }

    pub fn decode_typed_payload<T>(&self) -> serde_json::Result<Option<T>>
    where
        T: CustomAgentPayload,
    {
        if !self.is_role(T::ROLE) {
            return Ok(None);
        }

        self.decode_payload().map(Some)
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

    pub fn custom_typed<T>(
        role: impl Into<String>,
        payload: T,
        timestamp: u64,
    ) -> serde_json::Result<Self>
    where
        T: Serialize,
    {
        CustomAgentMessage::encode(role, payload, timestamp).map(Self::Custom)
    }

    pub fn typed_custom<T>(payload: T, timestamp: u64) -> serde_json::Result<Self>
    where
        T: CustomAgentPayload,
    {
        CustomAgentMessage::typed(payload, timestamp).map(Self::Custom)
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
        self.role() == "assistant"
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

    pub fn as_custom_message(&self) -> Option<&CustomAgentMessage> {
        match self {
            Self::Standard(_) => None,
            Self::Custom(message) => Some(message),
        }
    }

    pub fn into_custom_message(self) -> Option<CustomAgentMessage> {
        match self {
            Self::Standard(_) => None,
            Self::Custom(message) => Some(message),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct NotificationPayload {
        text: String,
        level: String,
    }

    impl CustomAgentPayload for NotificationPayload {
        const ROLE: &'static str = "notification";
    }

    #[test]
    fn typed_custom_payloads_round_trip_without_manual_json() {
        let payload = NotificationPayload {
            text: "hello".into(),
            level: "info".into(),
        };

        let message = AgentMessage::typed_custom(payload.clone(), 42).unwrap();
        let custom = message.as_custom_message().expect("custom message");

        assert_eq!(custom.role, "notification");
        assert_eq!(custom.timestamp, 42);
        assert_eq!(
            custom
                .decode_typed_payload::<NotificationPayload>()
                .unwrap(),
            Some(payload)
        );
    }

    #[test]
    fn explicit_role_custom_payloads_round_trip_without_manual_json() {
        let payload = NotificationPayload {
            text: "hello".into(),
            level: "warning".into(),
        };

        let message = AgentMessage::custom_typed("notification", payload.clone(), 7).unwrap();
        let custom = message.into_custom_message().expect("custom message");

        assert!(custom.is_role("notification"));
        assert_eq!(
            custom.decode_payload::<NotificationPayload>().unwrap(),
            payload
        );
    }

    #[test]
    fn typed_payload_decode_returns_none_when_role_does_not_match() {
        let payload = NotificationPayload {
            text: "hello".into(),
            level: "info".into(),
        };

        let custom = CustomAgentMessage::encode("other", payload, 5).unwrap();

        assert_eq!(
            custom
                .decode_typed_payload::<NotificationPayload>()
                .unwrap(),
            None
        );
    }
}
