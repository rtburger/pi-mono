use httpmock::prelude::*;
use parking_lot::Mutex;
use pi_ai::{PayloadHook, StreamOptions, complete};
use pi_events::{
    AssistantContent, Context, Message, Model, StopReason, ToolDefinition, Usage, UserContent,
};
use std::{collections::BTreeMap, sync::Arc};

const HELLO_SSE: &str = concat!(
    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
    "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
    "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
);

const THINKING_SIGNATURE: &str = r#"{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"I should think first."}]}"#;

fn model(base_url: String, id: &str) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    }
}

fn double_number_tool() -> ToolDefinition {
    ToolDefinition {
        name: "double_number".into(),
        description: "Doubles a number and returns the result".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "number" }
            },
            "required": ["value"]
        }),
    }
}

fn tool_call_arguments(
    arguments: &[(&str, serde_json::Value)],
) -> BTreeMap<String, serde_json::Value> {
    arguments
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn capture_payload_hook(store: Arc<Mutex<Option<serde_json::Value>>>) -> PayloadHook {
    PayloadHook::new(move |payload, _model| {
        let store = store.clone();
        async move {
            *store.lock() = Some(payload);
            Ok(None)
        }
    })
}

fn user_message(text: &str, timestamp: u64) -> Message {
    Message::User {
        content: vec![UserContent::Text { text: text.into() }],
        timestamp,
    }
}

fn aborted_reasoning_assistant(timestamp: u64) -> Message {
    Message::Assistant {
        content: vec![AssistantContent::Thinking {
            thinking: "I should think first.".into(),
            thinking_signature: Some(THINKING_SIGNATURE.into()),
            redacted: false,
        }],
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "gpt-5-mini".into(),
        response_id: Some("resp_aborted".into()),
        usage: Usage::default(),
        stop_reason: StopReason::Aborted,
        error_message: Some("Request was aborted".into()),
        timestamp,
    }
}

fn tool_call_assistant(timestamp: u64) -> Message {
    Message::Assistant {
        content: vec![
            AssistantContent::Thinking {
                thinking: "I should think first.".into(),
                thinking_signature: Some(THINKING_SIGNATURE.into()),
                redacted: false,
            },
            AssistantContent::ToolCall {
                id: "call_123|fc_123".into(),
                name: "double_number".into(),
                arguments: tool_call_arguments(&[("value", serde_json::json!(21))]),
                thought_signature: Some(THINKING_SIGNATURE.into()),
            },
        ],
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "gpt-5-mini".into(),
        response_id: Some("resp_tool_use".into()),
        usage: Usage::default(),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp,
    }
}

#[tokio::test]
async fn skips_aborted_reasoning_history_in_request_payload() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .header("authorization", "Bearer test-key");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(HELLO_SSE);
    });

    let captured_payload: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let response = complete(
        model(server.base_url(), "gpt-5-mini"),
        Context {
            system_prompt: Some("You are a helpful assistant.".into()),
            messages: vec![
                user_message("Use the double_number tool to double 21.", 1),
                aborted_reasoning_assistant(2),
                user_message("Say hello to confirm you can continue.", 3),
            ],
            tools: vec![double_number_tool()],
        },
        StreamOptions {
            api_key: Some("test-key".into()),
            reasoning_effort: Some("high".into()),
            on_payload: Some(capture_payload_hook(captured_payload.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.stop_reason, StopReason::Stop);
    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert_eq!(
        response.content,
        vec![AssistantContent::Text {
            text: "Hello".into(),
            text_signature: Some(r#"{"v":1,"id":"msg_1"}"#.into()),
        }]
    );

    let payload = captured_payload.lock().clone().expect("captured payload");
    assert_eq!(
        payload["input"],
        serde_json::json!([
            {
                "type": "message",
                "role": "developer",
                "content": [
                    { "type": "input_text", "text": "You are a helpful assistant." }
                ]
            },
            {
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "Use the double_number tool to double 21." }
                ]
            },
            {
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "Say hello to confirm you can continue." }
                ]
            }
        ])
    );
    assert_eq!(
        payload["tools"],
        serde_json::json!([
            {
                "type": "function",
                "name": "double_number",
                "description": "Doubles a number and returns the result",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "number" }
                    },
                    "required": ["value"]
                },
                "strict": false
            }
        ])
    );
    assert_eq!(
        payload["reasoning"],
        serde_json::json!({"effort": "high", "summary": "auto"})
    );
    assert_eq!(
        payload["include"],
        serde_json::json!(["reasoning.encrypted_content"])
    );
}

#[tokio::test]
async fn replays_same_provider_different_model_handoff_without_reasoning_items() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .header("authorization", "Bearer test-key");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(HELLO_SSE);
    });

    let captured_payload: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let response = complete(
        model(server.base_url(), "gpt-5.2-codex"),
        Context {
            system_prompt: Some("You are a helpful assistant. Answer concisely.".into()),
            messages: vec![
                user_message("Use the double_number tool to double 21.", 1),
                tool_call_assistant(2),
                Message::ToolResult {
                    tool_call_id: "call_123|fc_123".into(),
                    tool_name: "double_number".into(),
                    content: vec![UserContent::Text { text: "42".into() }],
                    details: None,
                    is_error: false,
                    timestamp: 3,
                },
                user_message("What was the result? Answer with just the number.", 4),
            ],
            tools: vec![double_number_tool()],
        },
        StreamOptions {
            api_key: Some("test-key".into()),
            reasoning_effort: Some("high".into()),
            on_payload: Some(capture_payload_hook(captured_payload.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.stop_reason, StopReason::Stop);
    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert_eq!(
        response.content,
        vec![AssistantContent::Text {
            text: "Hello".into(),
            text_signature: Some(r#"{"v":1,"id":"msg_1"}"#.into()),
        }]
    );

    let payload = captured_payload.lock().clone().expect("captured payload");
    assert_eq!(payload["model"], serde_json::json!("gpt-5.2-codex"));
    assert_eq!(
        payload["input"],
        serde_json::json!([
            {
                "type": "message",
                "role": "developer",
                "content": [
                    {
                        "type": "input_text",
                        "text": "You are a helpful assistant. Answer concisely."
                    }
                ]
            },
            {
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "Use the double_number tool to double 21." }
                ]
            },
            {
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "id": "msg_1",
                "content": [
                    {
                        "type": "output_text",
                        "text": "I should think first.",
                        "annotations": []
                    }
                ]
            },
            {
                "type": "function_call",
                "call_id": "call_123",
                "name": "double_number",
                "arguments": "{\"value\":21}"
            },
            {
                "type": "function_call_output",
                "call_id": "call_123",
                "output": "42"
            },
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "What was the result? Answer with just the number."
                    }
                ]
            }
        ])
    );
    assert_eq!(
        payload["reasoning"],
        serde_json::json!({"effort": "high", "summary": "auto"})
    );
    assert_eq!(
        payload["include"],
        serde_json::json!(["reasoning.encrypted_content"])
    );
    assert_eq!(
        payload["tools"],
        serde_json::json!([
            {
                "type": "function",
                "name": "double_number",
                "description": "Doubles a number and returns the result",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "number" }
                    },
                    "required": ["value"]
                },
                "strict": false
            }
        ])
    );
}
