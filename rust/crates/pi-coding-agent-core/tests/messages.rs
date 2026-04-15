use pi_agent::AgentMessage;
use pi_ai::{
    FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions, StreamOptions,
    register_faux_provider,
};
use pi_coding_agent_core::{
    BLOCKED_IMAGE_PLACEHOLDER, BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX, BashExecutionMessage,
    BranchSummaryMessage, COMPACTION_SUMMARY_PREFIX, COMPACTION_SUMMARY_SUFFIX,
    CodingAgentCoreOptions, CompactionSummaryMessage, CustomMessage, CustomMessageContent,
    MemoryAuthStorage, SessionBootstrapOptions, bash_execution_to_text, convert_to_llm,
    create_coding_agent_core, filter_blocked_images,
};
use pi_events::{Message, UserContent};
use serde_json::json;
use std::sync::Arc;

#[test]
fn bash_execution_to_text_formats_output_metadata_and_truncation() {
    let formatted = bash_execution_to_text(&BashExecutionMessage {
        command: "npm run check".into(),
        output: "Checked 42 files".into(),
        exit_code: Some(2),
        cancelled: false,
        truncated: true,
        full_output_path: Some("/tmp/check.log".into()),
        exclude_from_context: false,
    });

    assert_eq!(
        formatted,
        "Ran `npm run check`\n```\nChecked 42 files\n```\n\nCommand exited with code 2\n\n[Output truncated. Full output: /tmp/check.log]"
    );
}

#[test]
fn convert_to_llm_converts_supported_coding_agent_messages() {
    let converted = convert_to_llm(vec![
        Message::User {
            content: vec![UserContent::Text {
                text: "hello".into(),
            }],
            timestamp: 1,
        }
        .into(),
        BashExecutionMessage {
            command: "pwd".into(),
            output: "/repo".into(),
            exit_code: Some(0),
            cancelled: false,
            truncated: false,
            full_output_path: None,
            exclude_from_context: false,
        }
        .into_agent_message(2),
        BashExecutionMessage {
            command: "secret".into(),
            output: "ignored".into(),
            exit_code: Some(0),
            cancelled: false,
            truncated: false,
            full_output_path: None,
            exclude_from_context: true,
        }
        .into_agent_message(3),
        CustomMessage {
            custom_type: "note".into(),
            content: CustomMessageContent::Text("custom text".into()),
            display: true,
            details: Some(json!({ "source": "extension" })),
        }
        .into_agent_message(4),
        BranchSummaryMessage {
            summary: "wrong branch".into(),
            from_id: "msg-3".into(),
        }
        .into_agent_message(5),
        CompactionSummaryMessage {
            summary: "earlier context".into(),
            tokens_before: 2048,
        }
        .into_agent_message(6),
        Message::ToolResult {
            tool_call_id: "call-1".into(),
            tool_name: "read".into(),
            content: vec![UserContent::Text { text: "ok".into() }],
            details: None,
            is_error: false,
            timestamp: 7,
        }
        .into(),
        AgentMessage::custom("unknown", json!({ "ignored": true }), 8),
    ]);

    assert_eq!(converted.len(), 6);
    assert_eq!(
        converted[0],
        Message::User {
            content: vec![UserContent::Text {
                text: "hello".into()
            }],
            timestamp: 1,
        }
    );
    assert_eq!(
        converted[1],
        Message::User {
            content: vec![UserContent::Text {
                text: "Ran `pwd`\n```\n/repo\n```".into(),
            }],
            timestamp: 2,
        }
    );
    assert_eq!(
        converted[2],
        Message::User {
            content: vec![UserContent::Text {
                text: "custom text".into(),
            }],
            timestamp: 4,
        }
    );
    assert_eq!(
        converted[3],
        Message::User {
            content: vec![UserContent::Text {
                text: format!("{BRANCH_SUMMARY_PREFIX}wrong branch{BRANCH_SUMMARY_SUFFIX}"),
            }],
            timestamp: 5,
        }
    );
    assert_eq!(
        converted[4],
        Message::User {
            content: vec![UserContent::Text {
                text: format!(
                    "{COMPACTION_SUMMARY_PREFIX}earlier context{COMPACTION_SUMMARY_SUFFIX}"
                ),
            }],
            timestamp: 6,
        }
    );
    assert_eq!(
        converted[5],
        Message::ToolResult {
            tool_call_id: "call-1".into(),
            tool_name: "read".into(),
            content: vec![UserContent::Text { text: "ok".into() }],
            details: None,
            is_error: false,
            timestamp: 7,
        }
    );
}

#[test]
fn filter_blocked_images_replaces_user_and_tool_result_images_with_placeholder() {
    let filtered = filter_blocked_images(vec![
        Message::User {
            content: vec![
                UserContent::Text {
                    text: "before".into(),
                },
                UserContent::Image {
                    data: "image-1".into(),
                    mime_type: "image/png".into(),
                },
                UserContent::Image {
                    data: "image-2".into(),
                    mime_type: "image/png".into(),
                },
                UserContent::Text {
                    text: "middle".into(),
                },
                UserContent::Image {
                    data: "image-3".into(),
                    mime_type: "image/png".into(),
                },
            ],
            timestamp: 10,
        },
        Message::ToolResult {
            tool_call_id: "call-1".into(),
            tool_name: "read".into(),
            content: vec![
                UserContent::Image {
                    data: "image-4".into(),
                    mime_type: "image/jpeg".into(),
                },
                UserContent::Image {
                    data: "image-5".into(),
                    mime_type: "image/jpeg".into(),
                },
            ],
            details: None,
            is_error: false,
            timestamp: 11,
        },
        Message::Assistant {
            content: vec![],
            api: "api".into(),
            provider: "provider".into(),
            model: "model".into(),
            response_id: None,
            usage: Default::default(),
            stop_reason: pi_events::StopReason::Stop,
            error_message: None,
            timestamp: 12,
        },
    ]);

    assert_eq!(
        filtered,
        vec![
            Message::User {
                content: vec![
                    UserContent::Text {
                        text: "before".into(),
                    },
                    UserContent::Text {
                        text: BLOCKED_IMAGE_PLACEHOLDER.into(),
                    },
                    UserContent::Text {
                        text: "middle".into(),
                    },
                    UserContent::Text {
                        text: BLOCKED_IMAGE_PLACEHOLDER.into(),
                    },
                ],
                timestamp: 10,
            },
            Message::ToolResult {
                tool_call_id: "call-1".into(),
                tool_name: "read".into(),
                content: vec![UserContent::Text {
                    text: BLOCKED_IMAGE_PLACEHOLDER.into(),
                }],
                details: None,
                is_error: false,
                timestamp: 11,
            },
            Message::Assistant {
                content: vec![],
                api: "api".into(),
                provider: "provider".into(),
                model: "model".into(),
                response_id: None,
                usage: Default::default(),
                stop_reason: pi_events::StopReason::Stop,
                error_message: None,
                timestamp: 12,
            },
        ]
    );
}

#[test]
fn convert_to_llm_preserves_custom_message_blocks() {
    let converted = convert_to_llm(vec![
        CustomMessage {
            custom_type: "screenshot".into(),
            content: CustomMessageContent::Blocks(vec![
                UserContent::Text {
                    text: "before".into(),
                },
                UserContent::Image {
                    data: "abc123".into(),
                    mime_type: "image/png".into(),
                },
            ]),
            display: false,
            details: None,
        }
        .into_agent_message(9),
    ]);

    assert_eq!(
        converted,
        vec![Message::User {
            content: vec![
                UserContent::Text {
                    text: "before".into(),
                },
                UserContent::Image {
                    data: "abc123".into(),
                    mime_type: "image/png".into(),
                },
            ],
            timestamp: 9,
        }]
    );
}

#[tokio::test]
async fn runtime_uses_coding_agent_message_conversion_hook() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "messages-faux".into(),
        models: vec![FauxModelDefinition {
            id: "messages-faux-1".into(),
            name: Some("Messages Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![
        FauxResponse::text("branch summary"),
        FauxResponse::text("unknown custom"),
    ]);
    let model = faux.get_model(Some("messages-faux-1")).unwrap();
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        model.provider.clone(),
        "test-token",
    )]));

    let included = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: auth.clone(),
        built_in_models: vec![model.clone()],
        models_json_path: None,
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();
    included
        .core
        .agent()
        .prompt(
            BranchSummaryMessage {
                summary: "Investigated wrong path".into(),
                from_id: "msg-10".into(),
            }
            .into_agent_message(1),
        )
        .await
        .unwrap();
    let included_input = last_assistant_input_tokens(&included.core.state());

    let unknown = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: auth,
        built_in_models: vec![model],
        models_json_path: None,
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();
    unknown
        .core
        .agent()
        .prompt(AgentMessage::custom(
            "unknown",
            json!({ "ignored": true }),
            1,
        ))
        .await
        .unwrap();
    let unknown_input = last_assistant_input_tokens(&unknown.core.state());

    assert!(
        included_input > unknown_input,
        "expected converted branch summary to add prompt tokens: {included_input} <= {unknown_input}"
    );

    faux.unregister();
}

fn last_assistant_input_tokens(state: &pi_agent::AgentState) -> u64 {
    let message = state
        .messages
        .last()
        .expect("assistant response should be present")
        .as_standard_message()
        .expect("last message should be standard");

    match message {
        Message::Assistant { usage, .. } => usage.input,
        other => panic!("expected assistant message, got {other:?}"),
    }
}
