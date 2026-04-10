use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, ResponsesFunctionCallOutput, ResponsesInputItem,
    convert_openai_responses_messages, normalize_tool_call_id, tool_call_arguments,
};
use pi_events::{AssistantContent, Context, Message, Model, StopReason, Usage, UserContent};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

const RAW_TOOL_CALL_ID: &str = "call_4VnzVawQXPB9MgYib7CiQFEY|I9b95oN1wD/cHXKTw3PpRkL6KkCtzTJhUxMouMWYwHeTo2j3htzfSk7YPx2vifiIM4g3A8XXyOj8q4Bt6SLUG7gqY1E3ELkrkVQNHglRfUmWj84lqxJY+Puieb3VKyX0FB+83TUzn91cDMF/4gzt990IzqVrc+nIb9RRscRD070Du16q1glydVjWR0SBJsE6TbY/esOjFpqplogQqrajm1eI++f3eLi73R6q7hVusY0QbeFySVxABCjhN0lXB04caBe1rzHjYzul6MAXj7uq+0r17VLq+yrtyYhN12wkmFqHeqTyEei6EFPbMy24Nc+IbJlkP0OCg02W+gOnyBFcbi2ctvJFSOhSjt1CqBdqCnnhwUqXjbWiT0wh3DmLScRgTHmGkaI+oAcQQjfic65nxj+TnEkReA==";

fn model(provider: &str, id: &str) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "openai-responses".into(),
        provider: provider.into(),
        base_url: "https://api.example.test".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn usage() -> Usage {
    Usage::default()
}

#[test]
fn normalizes_foreign_tool_call_ids_for_openai_responses() {
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("openai_responses_foreign_tool_call.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let normalized = normalize_tool_call_id(RAW_TOOL_CALL_ID, true, true);
    let (call_id, item_id) = normalized.split_once('|').unwrap();

    assert_eq!(call_id, expected["call_id"].as_str().unwrap());
    assert!(item_id.starts_with(expected["id_prefix"].as_str().unwrap()));
    assert!(item_id.len() <= 64);
    assert!(
        item_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    );
}

#[test]
fn converts_foreign_assistant_tool_call_to_function_call() {
    let context = Context {
        system_prompt: Some("You are concise.".into()),
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Use the tool.".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: RAW_TOOL_CALL_ID.into(),
                    name: "edit".into(),
                    arguments: tool_call_arguments(&[("path", json!("src/styles/app.css"))]),
                    thought_signature: None,
                }],
                api: "openai-responses".into(),
                provider: "openai".into(),
                model: "gpt-5.3-codex".into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
        ],
        tools: vec![],
    };

    let items = convert_openai_responses_messages(
        &model("openai-codex", "gpt-5.3-codex"),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
    );

    let function_call = items
        .iter()
        .find(|item| matches!(item, ResponsesInputItem::FunctionCall { .. }))
        .unwrap();

    match function_call {
        ResponsesInputItem::FunctionCall {
            id,
            call_id,
            name,
            arguments,
        } => {
            assert_eq!(call_id, "call_4VnzVawQXPB9MgYib7CiQFEY");
            assert_eq!(name, "edit");
            assert_eq!(arguments, r#"{"path":"src/styles/app.css"}"#);
            let id = id.as_ref().unwrap();
            assert!(id.starts_with("fc_"));
            assert!(id.len() <= 64);
        }
        _ => unreachable!(),
    }
}

#[test]
fn inserts_synthetic_tool_result_for_orphaned_tool_call() {
    let context = Context {
        system_prompt: None,
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Use the tool first.".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: "call_123|fc_123".into(),
                    name: "calculate".into(),
                    arguments: tool_call_arguments(&[("expression", json!("25 * 18"))]),
                    thought_signature: None,
                }],
                api: "openai-responses".into(),
                provider: "openai".into(),
                model: "gpt-5-mini".into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "Never mind, what is 2 + 2?".into(),
                }],
                timestamp: 3,
            },
        ],
        tools: vec![],
    };

    let items = convert_openai_responses_messages(
        &model("openai", "gpt-5-mini"),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    assert!(matches!(items[0], ResponsesInputItem::Message { .. }));
    assert!(matches!(items[1], ResponsesInputItem::FunctionCall { .. }));
    match &items[2] {
        ResponsesInputItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "call_123");
            assert_eq!(
                output,
                &ResponsesFunctionCallOutput::Text("No result provided".into())
            );
        }
        other => panic!("expected synthetic function_call_output, got {other:?}"),
    }
    match &items[3] {
        ResponsesInputItem::Message { role, .. } => assert_eq!(role, "user"),
        other => panic!("expected follow-up user message, got {other:?}"),
    }
}

#[test]
fn skips_aborted_assistant_messages_during_replay() {
    let context = Context {
        system_prompt: None,
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "first".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![AssistantContent::Thinking {
                    thinking: "internal reasoning".into(),
                    thinking_signature: Some(
                        r#"{"type":"reasoning","id":"rs_abort","summary":[]}"#.into(),
                    ),
                    redacted: false,
                }],
                api: "openai-responses".into(),
                provider: "openai".into(),
                model: "gpt-5-mini".into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::Aborted,
                error_message: Some("Request was aborted".into()),
                timestamp: 2,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "second".into(),
                }],
                timestamp: 3,
            },
        ],
        tools: vec![],
    };

    let items = convert_openai_responses_messages(
        &model("openai", "gpt-5-mini"),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    assert_eq!(items.len(), 2);
    for item in items {
        match item {
            ResponsesInputItem::Message { role, .. } => assert_eq!(role, "user"),
            other => panic!("expected only user messages after filtering, got {other:?}"),
        }
    }
}

#[test]
fn omits_fc_item_id_for_same_provider_different_model_handoff() {
    let context = Context {
        system_prompt: None,
        messages: vec![Message::Assistant {
            content: vec![AssistantContent::ToolCall {
                id: "call_123|fc_123".into(),
                name: "edit".into(),
                arguments: tool_call_arguments(&[("path", json!("src/main.rs"))]),
                thought_signature: None,
            }],
            api: "openai-responses".into(),
            provider: "openai".into(),
            model: "gpt-5-mini".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 1,
        }],
        tools: vec![],
    };

    let items = convert_openai_responses_messages(
        &model("openai", "gpt-5.2-codex"),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    match items.first().unwrap() {
        ResponsesInputItem::FunctionCall { id, call_id, .. } => {
            assert_eq!(call_id, "call_123");
            assert!(id.is_none());
        }
        other => panic!("expected function_call, got {other:?}"),
    }
}

#[test]
fn converts_different_model_thinking_to_assistant_output_text() {
    let context = Context {
        system_prompt: None,
        messages: vec![Message::Assistant {
            content: vec![AssistantContent::Thinking {
                thinking: "Let me think about this...".into(),
                thinking_signature: None,
                redacted: false,
            }],
            api: "openai-responses".into(),
            provider: "openai".into(),
            model: "gpt-5-mini".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 1,
        }],
        tools: vec![],
    };

    let items = convert_openai_responses_messages(
        &model("openai", "gpt-5.2-codex"),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    match items.first().unwrap() {
        ResponsesInputItem::Message {
            role,
            content,
            status,
            ..
        } => {
            assert_eq!(role, "assistant");
            assert_eq!(status.as_deref(), Some("completed"));
            assert_eq!(
                content,
                &vec![pi_ai::openai_responses::ResponsesContentPart::OutputText {
                    text: "Let me think about this...".into(),
                    annotations: Vec::new(),
                }]
            );
        }
        other => panic!("expected assistant message, got {other:?}"),
    }
}

#[test]
fn replays_same_model_reasoning_and_text_signatures() {
    let context = Context {
        system_prompt: None,
        messages: vec![Message::Assistant {
            content: vec![
                AssistantContent::Thinking {
                    thinking: "Short summary".into(),
                    thinking_signature: Some(
                        r#"{"type":"reasoning","id":"rs_123","summary":[{"type":"summary_text","text":"Short summary"}],"encrypted_content":"enc"}"#
                            .into(),
                    ),
                    redacted: false,
                },
                AssistantContent::Text {
                    text: "Final answer".into(),
                    text_signature: Some(
                        r#"{"v":1,"id":"msg_signature_123","phase":"final_answer"}"#.into(),
                    ),
                },
            ],
            api: "openai-responses".into(),
            provider: "openai".into(),
            model: "gpt-5-mini".into(),
            response_id: Some("resp_123".into()),
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 1,
        }],
        tools: vec![],
    };

    let items = convert_openai_responses_messages(
        &model("openai", "gpt-5-mini"),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    match &items[0] {
        ResponsesInputItem::Reasoning { data } => {
            assert_eq!(data.get("id").and_then(Value::as_str), Some("rs_123"));
            assert_eq!(
                data.get("encrypted_content").and_then(Value::as_str),
                Some("enc")
            );
        }
        other => panic!("expected reasoning replay item, got {other:?}"),
    }

    match &items[1] {
        ResponsesInputItem::Message {
            role,
            id,
            phase,
            status,
            content,
        } => {
            assert_eq!(role, "assistant");
            assert_eq!(id.as_deref(), Some("msg_signature_123"));
            assert_eq!(phase.as_deref(), Some("final_answer"));
            assert_eq!(status.as_deref(), Some("completed"));
            assert_eq!(
                content,
                &vec![pi_ai::openai_responses::ResponsesContentPart::OutputText {
                    text: "Final answer".into(),
                    annotations: Vec::new(),
                }]
            );
        }
        other => panic!("expected assistant replay message, got {other:?}"),
    }
}

#[test]
fn keeps_tool_result_images_inside_function_call_output() {
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("openai_responses_tool_result_images.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let context = Context {
        system_prompt: Some("Use the tool when asked.".into()),
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Call get_circle_with_description and summarize it.".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: "call_123|fc_123".into(),
                    name: "get_circle_with_description".into(),
                    arguments: tool_call_arguments(&[]),
                    thought_signature: None,
                }],
                api: "openai-responses".into(),
                provider: "openai".into(),
                model: "gpt-5-mini".into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
            Message::ToolResult {
                tool_call_id: "call_123|fc_123".into(),
                tool_name: "get_circle_with_description".into(),
                content: vec![
                    UserContent::Text {
                        text: "A red circle with a diameter of 100 pixels.".into(),
                    },
                    UserContent::Image {
                        data: "ZmFrZS1iYXNlNjQ=".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                is_error: false,
                timestamp: 3,
            },
        ],
        tools: vec![],
    };

    let items = convert_openai_responses_messages(
        &model("openai", "gpt-5-mini"),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
    );

    let output = items
        .iter()
        .find(|item| matches!(item, ResponsesInputItem::FunctionCallOutput { .. }))
        .unwrap();

    match output {
        ResponsesInputItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, expected["call_id"].as_str().unwrap());
            match output {
                ResponsesFunctionCallOutput::Content(parts) => {
                    let types = parts
                        .iter()
                        .map(|part| match part {
                            pi_ai::openai_responses::ResponsesContentPart::InputText { .. } => {
                                "input_text"
                            }
                            pi_ai::openai_responses::ResponsesContentPart::InputImage {
                                ..
                            } => "input_image",
                            other => panic!("unexpected content part: {other:?}"),
                        })
                        .collect::<Vec<_>>();
                    let expected_types = expected["output_types"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| value.as_str().unwrap())
                        .collect::<Vec<_>>();
                    assert_eq!(types, expected_types);
                }
                ResponsesFunctionCallOutput::Text(_) => {
                    panic!("expected structured content output")
                }
            }
        }
        _ => unreachable!(),
    }
}
