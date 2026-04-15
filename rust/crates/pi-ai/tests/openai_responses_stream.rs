use futures::StreamExt;
use pi_ai::openai_responses::{OpenAiResponsesStreamEnvelope, stream_openai_responses_sse_events};
use pi_events::{AssistantContent, AssistantEvent, Model, StopReason};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};

fn model() -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 16_384,
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
async fn streams_text_response_events() {
    let events = vec![
        OpenAiResponsesStreamEnvelope {
            event_type: "response.created".into(),
            data: serde_json::from_value(json!({ "response": { "id": "resp_1" } })).unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.added".into(),
            data: serde_json::from_value(json!({
                "item": { "type": "message", "id": "msg_1", "role": "assistant", "status": "in_progress", "content": [] }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_text.delta".into(),
            data: serde_json::from_value(json!({ "delta": "Hello" })).unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.done".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{ "type": "output_text", "text": "Hello" }]
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.completed".into(),
            data: serde_json::from_value(json!({
                "response": {
                    "status": "completed",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8,
                        "input_tokens_details": { "cached_tokens": 0 }
                    }
                }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_openai_responses_sse_events(model(), events)
        .collect::<Vec<_>>()
        .await;

    let actual = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start".to_string(),
            AssistantEvent::TextStart { .. } => "text_start".to_string(),
            AssistantEvent::TextDelta { .. } => "text_delta".to_string(),
            AssistantEvent::TextEnd { .. } => "text_end".to_string(),
            AssistantEvent::Done { .. } => "done".to_string(),
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(actual, fixture("openai_responses_stream_text.json"));
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(
                message.content,
                vec![AssistantContent::Text {
                    text: "Hello".into(),
                    text_signature: Some(r#"{"v":1,"id":"msg_1"}"#.into()),
                }]
            );
            assert!(message.usage.cost.total > 0.0);
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn streams_tool_call_response_events() {
    let events = vec![
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.added".into(),
            data: serde_json::from_value(json!({
                "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "edit", "arguments": "" }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.function_call_arguments.delta".into(),
            data: serde_json::from_value(json!({ "delta": "{\"path\":\"src/main.rs\"}" })).unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.done".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "edit",
                    "arguments": "{\"path\":\"src/main.rs\"}"
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.completed".into(),
            data: serde_json::from_value(json!({
                "response": {
                    "status": "completed",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8,
                        "input_tokens_details": { "cached_tokens": 0 }
                    }
                }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_openai_responses_sse_events(model(), events)
        .collect::<Vec<_>>()
        .await;

    let names = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start".to_string(),
            AssistantEvent::ToolCallStart { .. } => "tool_call_start".to_string(),
            AssistantEvent::ToolCallDelta { .. } => "tool_call_delta".to_string(),
            AssistantEvent::ToolCallEnd { .. } => "tool_call_end".to_string(),
            AssistantEvent::Done { .. } => "done".to_string(),
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(names, fixture("openai_responses_stream_tool.json"));

    let done = collected.last().unwrap().as_ref().unwrap();
    match done {
        AssistantEvent::Done { reason, message } => {
            assert_eq!(*reason, StopReason::ToolUse);
            assert!(matches!(
                message.content[0],
                pi_events::AssistantContent::ToolCall { .. }
            ));
            assert!(message.usage.cost.total > 0.0);
        }
        _ => panic!("expected terminal done event"),
    }
}

#[tokio::test]
async fn streams_partial_tool_call_json_events() {
    let events = vec![
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.added".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "edit",
                    "arguments": ""
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.function_call_arguments.delta".into(),
            data: serde_json::from_value(json!({
                "delta": "{\"path\":{\"inner\":"
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.function_call_arguments.delta".into(),
            data: serde_json::from_value(json!({
                "delta": "1}}"
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.done".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "edit",
                    "arguments": "{\"path\":{\"inner\":1}}"
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.completed".into(),
            data: serde_json::from_value(json!({
                "response": {
                    "status": "completed",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8,
                        "input_tokens_details": { "cached_tokens": 0 }
                    }
                }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_openai_responses_sse_events(model(), events)
        .collect::<Vec<_>>()
        .await;

    let names = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start".to_string(),
            AssistantEvent::ToolCallStart { .. } => "tool_call_start".to_string(),
            AssistantEvent::ToolCallDelta { .. } => "tool_call_delta".to_string(),
            AssistantEvent::ToolCallEnd { .. } => "tool_call_end".to_string(),
            AssistantEvent::Done { .. } => "done".to_string(),
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "start",
            "tool_call_start",
            "tool_call_delta",
            "tool_call_delta",
            "tool_call_end",
            "done"
        ]
    );

    let tool_call_deltas = collected
        .iter()
        .filter_map(|event| match event.as_ref().unwrap() {
            AssistantEvent::ToolCallDelta { partial, .. } => Some(partial.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(tool_call_deltas.len(), 2);

    match &tool_call_deltas[0].content[0] {
        AssistantContent::ToolCall { arguments, .. } => {
            assert_eq!(arguments, &BTreeMap::from([("path".into(), json!({}))]));
        }
        other => panic!("expected partial tool call after first delta, got {other:?}"),
    }

    match &tool_call_deltas[1].content[0] {
        AssistantContent::ToolCall { arguments, .. } => {
            assert_eq!(
                arguments,
                &BTreeMap::from([("path".into(), json!({"inner": 1}))])
            );
        }
        other => panic!("expected partial tool call after second delta, got {other:?}"),
    }

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => match &message.content[0] {
            AssistantContent::ToolCall { arguments, .. } => {
                assert_eq!(
                    arguments,
                    &BTreeMap::from([("path".into(), json!({"inner": 1}))])
                );
            }
            other => panic!("expected completed tool call, got {other:?}"),
        },
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn streams_partial_numeric_tool_call_json_events() {
    let events = vec![
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.added".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "function_call",
                    "id": "fc_2",
                    "call_id": "call_2",
                    "name": "edit",
                    "arguments": ""
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.function_call_arguments.delta".into(),
            data: serde_json::from_value(json!({
                "delta": "{\"path\":1e"
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.function_call_arguments.delta".into(),
            data: serde_json::from_value(json!({
                "delta": "2}"
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.done".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "function_call",
                    "id": "fc_2",
                    "call_id": "call_2",
                    "name": "edit",
                    "arguments": "{\"path\":100}"
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.completed".into(),
            data: serde_json::from_value(json!({
                "response": {
                    "status": "completed",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8,
                        "input_tokens_details": { "cached_tokens": 0 }
                    }
                }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_openai_responses_sse_events(model(), events)
        .collect::<Vec<_>>()
        .await;

    let names = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start".to_string(),
            AssistantEvent::ToolCallStart { .. } => "tool_call_start".to_string(),
            AssistantEvent::ToolCallDelta { .. } => "tool_call_delta".to_string(),
            AssistantEvent::ToolCallEnd { .. } => "tool_call_end".to_string(),
            AssistantEvent::Done { .. } => "done".to_string(),
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "start",
            "tool_call_start",
            "tool_call_delta",
            "tool_call_delta",
            "tool_call_end",
            "done"
        ]
    );

    let tool_call_deltas = collected
        .iter()
        .filter_map(|event| match event.as_ref().unwrap() {
            AssistantEvent::ToolCallDelta { partial, .. } => Some(partial.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(tool_call_deltas.len(), 2);

    match &tool_call_deltas[0].content[0] {
        AssistantContent::ToolCall { arguments, .. } => {
            assert_eq!(arguments, &BTreeMap::from([("path".into(), json!(1))]));
        }
        other => panic!("expected partial tool call after first delta, got {other:?}"),
    }

    match &tool_call_deltas[1].content[0] {
        AssistantContent::ToolCall { arguments, .. } => {
            assert_eq!(arguments, &BTreeMap::from([("path".into(), json!(100))]));
        }
        other => panic!("expected partial tool call after second delta, got {other:?}"),
    }

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => match &message.content[0] {
            AssistantContent::ToolCall { arguments, .. } => {
                assert_eq!(arguments, &BTreeMap::from([("path".into(), json!(100))]));
            }
            other => panic!("expected completed tool call, got {other:?}"),
        },
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn streams_reasoning_summary_response_events() {
    let events = vec![
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.added".into(),
            data: serde_json::from_value(json!({
                "item": { "type": "reasoning", "id": "rs_1", "summary": [] }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.reasoning_summary_part.added".into(),
            data: serde_json::from_value(json!({
                "part": { "type": "summary_text", "text": "" }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.reasoning_summary_text.delta".into(),
            data: serde_json::from_value(json!({ "delta": "I reasoned" })).unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.reasoning_summary_part.done".into(),
            data: serde_json::from_value(json!({})).unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.done".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "reasoning",
                    "id": "rs_1",
                    "summary": [{ "type": "summary_text", "text": "I reasoned" }]
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.completed".into(),
            data: serde_json::from_value(json!({
                "response": {
                    "status": "completed",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8,
                        "input_tokens_details": { "cached_tokens": 0 }
                    }
                }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_openai_responses_sse_events(model(), events)
        .collect::<Vec<_>>()
        .await;

    let names = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start",
            AssistantEvent::ThinkingStart { .. } => "thinking_start",
            AssistantEvent::ThinkingDelta { .. } => "thinking_delta",
            AssistantEvent::ThinkingEnd { .. } => "thinking_end",
            AssistantEvent::Done { .. } => "done",
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "start",
            "thinking_start",
            "thinking_delta",
            "thinking_delta",
            "thinking_end",
            "done"
        ]
    );
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(
                message.content,
                vec![AssistantContent::Thinking {
                    thinking: "I reasoned".into(),
                    thinking_signature: Some(
                        r#"{"id":"rs_1","summary":[{"text":"I reasoned","type":"summary_text"}],"type":"reasoning"}"#
                            .into(),
                    ),
                    redacted: false,
                }]
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn maps_failed_response_to_terminal_error_event() {
    let events = vec![OpenAiResponsesStreamEnvelope {
        event_type: "response.failed".into(),
        data: serde_json::from_value(json!({
            "response": {
                "id": "resp_fail_1",
                "usage": {
                    "input_tokens": 9,
                    "output_tokens": 0,
                    "total_tokens": 9,
                    "input_tokens_details": { "cached_tokens": 2 }
                },
                "error": { "code": "bad_request", "message": "boom" }
            }
        }))
        .unwrap(),
    }];

    let collected = stream_openai_responses_sse_events(model(), events)
        .collect::<Vec<_>>()
        .await;

    assert_eq!(collected.len(), 2);
    assert!(matches!(
        collected[0].as_ref().unwrap(),
        AssistantEvent::Start { .. }
    ));
    match collected[1].as_ref().unwrap() {
        AssistantEvent::Error { reason, error } => {
            assert_eq!(*reason, StopReason::Error);
            assert_eq!(error.error_message.as_deref(), Some("bad_request: boom"));
            assert_eq!(error.response_id.as_deref(), Some("resp_fail_1"));
            assert_eq!(error.usage.input, 7);
            assert_eq!(error.usage.cache_read, 2);
        }
        other => panic!("expected error event, got {other:?}"),
    }
}
