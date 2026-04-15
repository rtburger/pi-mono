use futures::StreamExt;
use pi_ai::anthropic_messages::{AnthropicStreamEnvelope, stream_anthropic_sse_events};
use pi_events::{AssistantContent, AssistantEvent, Model, StopReason, ToolDefinition};
use serde_json::json;
use std::{fs, path::PathBuf};

fn model(provider: &str) -> Model {
    Model {
        id: "claude-sonnet-4-20250514".into(),
        name: "claude-sonnet-4-20250514".into(),
        api: "anthropic-messages".into(),
        provider: provider.into(),
        base_url: "https://api.anthropic.com/v1".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 200_000,
        max_tokens: 8_192,
    }
}

fn fixture(name: &str) -> Vec<String> {
    serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn streams_text_thinking_and_tool_call_events() {
    let events = vec![
        AnthropicStreamEnvelope {
            event_type: "message_start".into(),
            data: serde_json::from_value(json!({
                "message": {
                    "id": "msg_1",
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "cache_creation_input_tokens": 0
                    }
                }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_start".into(),
            data: serde_json::from_value(json!({
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_delta".into(),
            data: serde_json::from_value(json!({
                "index": 0,
                "delta": { "type": "text_delta", "text": "Hello" }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_stop".into(),
            data: serde_json::from_value(json!({ "index": 0 })).unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_start".into(),
            data: serde_json::from_value(json!({
                "index": 1,
                "content_block": { "type": "thinking", "thinking": "" }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_delta".into(),
            data: serde_json::from_value(json!({
                "index": 1,
                "delta": { "type": "thinking_delta", "thinking": "Plan" }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_delta".into(),
            data: serde_json::from_value(json!({
                "index": 1,
                "delta": { "type": "signature_delta", "signature": "sig_1" }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_stop".into(),
            data: serde_json::from_value(json!({ "index": 1 })).unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_start".into(),
            data: serde_json::from_value(json!({
                "index": 2,
                "content_block": {
                    "type": "tool_use",
                    "id": "tool_1",
                    "name": "TodoWrite",
                    "input": {}
                }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_delta".into(),
            data: serde_json::from_value(json!({
                "index": 2,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": "{\"task\":\"buy milk\"}"
                }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_stop".into(),
            data: serde_json::from_value(json!({ "index": 2 })).unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "message_delta".into(),
            data: serde_json::from_value(json!({
                "delta": { "stop_reason": "tool_use" },
                "usage": { "output_tokens": 5 }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_anthropic_sse_events(
        model("anthropic"),
        events,
        true,
        vec![ToolDefinition {
            name: "todowrite".into(),
            description: "todo".into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }],
    )
    .collect::<Vec<_>>()
    .await;

    let actual = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start".to_string(),
            AssistantEvent::TextStart { .. } => "text_start".to_string(),
            AssistantEvent::TextDelta { .. } => "text_delta".to_string(),
            AssistantEvent::TextEnd { .. } => "text_end".to_string(),
            AssistantEvent::ThinkingStart { .. } => "thinking_start".to_string(),
            AssistantEvent::ThinkingDelta { .. } => "thinking_delta".to_string(),
            AssistantEvent::ThinkingEnd { .. } => "thinking_end".to_string(),
            AssistantEvent::ToolCallStart { .. } => "tool_call_start".to_string(),
            AssistantEvent::ToolCallDelta { .. } => "tool_call_delta".to_string(),
            AssistantEvent::ToolCallEnd { .. } => "tool_call_end".to_string(),
            AssistantEvent::Done { .. } => "done".to_string(),
            AssistantEvent::Error { .. } => "error".to_string(),
        })
        .collect::<Vec<_>>();

    assert_eq!(actual, fixture("anthropic_messages_stream_mixed.json"));

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { reason, message } => {
            assert_eq!(*reason, StopReason::ToolUse);
            assert_eq!(message.response_id.as_deref(), Some("msg_1"));
            assert_eq!(message.usage.total_tokens, 15);
            assert!(message.usage.cost.total > 0.0);
            assert_eq!(
                message.content,
                vec![
                    AssistantContent::Text {
                        text: "Hello".into(),
                        text_signature: None,
                    },
                    AssistantContent::Thinking {
                        thinking: "Plan".into(),
                        thinking_signature: Some("sig_1".into()),
                        redacted: false,
                    },
                    AssistantContent::ToolCall {
                        id: "tool_1".into(),
                        name: "todowrite".into(),
                        arguments: [("task".into(), json!("buy milk"))].into_iter().collect(),
                        thought_signature: None,
                    },
                ]
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn surfaces_explicit_error_events() {
    let collected = stream_anthropic_sse_events(
        model("anthropic"),
        vec![AnthropicStreamEnvelope {
            event_type: "error".into(),
            data: serde_json::from_value(json!({
                "error": {
                    "type": "invalid_request_error",
                    "message": "prompt is too long"
                }
            }))
            .unwrap(),
        }],
        false,
        vec![],
    )
    .collect::<Vec<_>>()
    .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Error { reason, error } => {
            assert_eq!(*reason, StopReason::Error);
            assert_eq!(error.error_message.as_deref(), Some("prompt is too long"));
        }
        other => panic!("expected error event, got {other:?}"),
    }
}

#[tokio::test]
async fn oauth_tool_names_round_trip_and_passthrough_in_stream() {
    for (tool_name, stream_tool_name) in [
        ("todowrite", "TodoWrite"),
        ("read", "Read"),
        ("find", "find"),
        ("my_custom_tool", "my_custom_tool"),
    ] {
        let collected = stream_anthropic_sse_events(
            model("anthropic"),
            vec![
                AnthropicStreamEnvelope {
                    event_type: "message_start".into(),
                    data: serde_json::from_value(json!({
                        "message": {
                            "id": "msg_1",
                            "usage": {
                                "input_tokens": 10,
                                "output_tokens": 0,
                                "cache_read_input_tokens": 0,
                                "cache_creation_input_tokens": 0
                            }
                        }
                    }))
                    .unwrap(),
                },
                AnthropicStreamEnvelope {
                    event_type: "content_block_start".into(),
                    data: serde_json::from_value(json!({
                        "index": 0,
                        "content_block": {
                            "type": "tool_use",
                            "id": "tool_1",
                            "name": stream_tool_name,
                            "input": {}
                        }
                    }))
                    .unwrap(),
                },
                AnthropicStreamEnvelope {
                    event_type: "content_block_stop".into(),
                    data: serde_json::from_value(json!({ "index": 0 })).unwrap(),
                },
                AnthropicStreamEnvelope {
                    event_type: "message_delta".into(),
                    data: serde_json::from_value(json!({
                        "delta": { "stop_reason": "tool_use" },
                        "usage": { "output_tokens": 1 }
                    }))
                    .unwrap(),
                },
            ],
            true,
            vec![ToolDefinition {
                name: tool_name.into(),
                description: format!("{tool_name} tool"),
                parameters: json!({ "type": "object", "properties": {} }),
            }],
        )
        .collect::<Vec<_>>()
        .await;

        let tool_call_name = collected.iter().find_map(|event| match event.as_ref().unwrap() {
            AssistantEvent::ToolCallEnd { tool_call, .. } => match tool_call {
                AssistantContent::ToolCall { name, .. } => Some(name.clone()),
                _ => None,
            },
            _ => None,
        });

        assert_eq!(tool_call_name.as_deref(), Some(tool_name));

        match collected.last().unwrap().as_ref().unwrap() {
            AssistantEvent::Done { reason, message } => {
                assert_eq!(*reason, StopReason::ToolUse);
                assert_eq!(message.response_id.as_deref(), Some("msg_1"));
            }
            other => panic!("expected done event, got {other:?}"),
        }
    }
}
