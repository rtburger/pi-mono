use pi_ai::openai_completions::{
    OpenAiCompletionsCompat, OpenAiCompletionsContentPart, OpenAiCompletionsMessageContent,
    convert_openai_completions_messages,
};
use pi_events::{AssistantContent, Context, Message, Model, StopReason, Usage, UserContent};

fn model(provider: &str, id: &str, reasoning: bool, input: &[&str]) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "openai-completions".into(),
        provider: provider.into(),
        base_url: "https://api.example.test/v1".into(),
        reasoning,
        input: input.iter().map(|value| (*value).to_string()).collect(),
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn usage() -> Usage {
    Usage::default()
}

#[test]
fn batches_tool_result_images_after_consecutive_tool_results() {
    let model = model("openai", "gpt-4o-mini", false, &["text", "image"]);
    let compat = OpenAiCompletionsCompat::default();
    let now = 1_725_000_000_000u64;

    let context = Context {
        system_prompt: None,
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Read the images".into(),
                }],
                timestamp: now - 2,
            },
            Message::Assistant {
                content: vec![
                    AssistantContent::ToolCall {
                        id: "tool-1".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({"path": "img-1.png"})
                            .as_object()
                            .unwrap()
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect(),
                        thought_signature: None,
                    },
                    AssistantContent::ToolCall {
                        id: "tool-2".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({"path": "img-2.png"})
                            .as_object()
                            .unwrap()
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect(),
                        thought_signature: None,
                    },
                ],
                api: model.api.clone(),
                provider: model.provider.clone(),
                model: model.id.clone(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: now,
            },
            Message::ToolResult {
                tool_call_id: "tool-1".into(),
                tool_name: "read".into(),
                content: vec![
                    UserContent::Text {
                        text: "Read image file [image/png]".into(),
                    },
                    UserContent::Image {
                        data: "ZmFrZQ==".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                details: None,
                is_error: false,
                timestamp: now + 1,
            },
            Message::ToolResult {
                tool_call_id: "tool-2".into(),
                tool_name: "read".into(),
                content: vec![
                    UserContent::Text {
                        text: "Read image file [image/png]".into(),
                    },
                    UserContent::Image {
                        data: "ZmFrZQ==".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                details: None,
                is_error: false,
                timestamp: now + 2,
            },
        ],
        tools: vec![],
    };

    let messages = convert_openai_completions_messages(&model, &context, &compat);
    let roles = messages
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["user", "assistant", "tool", "tool", "user"]);

    let image_message = messages.last().expect("missing image follow-up message");
    match &image_message.content {
        OpenAiCompletionsMessageContent::Parts(parts) => {
            let image_count = parts
                .iter()
                .filter(|part| matches!(part, OpenAiCompletionsContentPart::ImageUrl { .. }))
                .count();
            assert_eq!(image_count, 2);
        }
        other => panic!("expected multipart user message, got {other:?}"),
    }
}

#[test]
fn inserts_synthetic_tool_result_for_orphaned_tool_call() {
    let model = model("openai", "gpt-4o-mini", false, &["text"]);
    let compat = OpenAiCompletionsCompat::default();

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
                    arguments: serde_json::json!({"expression": "25 * 18"})
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                    thought_signature: None,
                }],
                api: model.api.clone(),
                provider: model.provider.clone(),
                model: model.id.clone(),
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

    let messages = convert_openai_completions_messages(&model, &context, &compat);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[2].role, "tool");
    assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_123|fc_123"));
    match &messages[2].content {
        OpenAiCompletionsMessageContent::Text(text) => assert_eq!(text, "No result provided"),
        other => panic!("expected synthetic tool text, got {other:?}"),
    }
    assert_eq!(messages[3].role, "user");
}

#[test]
fn uses_developer_role_for_reasoning_system_prompt_when_supported() {
    let model = model("openai", "gpt-5-mini", true, &["text"]);
    let compat = OpenAiCompletionsCompat::default();
    let context = Context {
        system_prompt: Some("You are concise.".into()),
        messages: vec![],
        tools: vec![],
    };

    let messages = convert_openai_completions_messages(&model, &context, &compat);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "developer");
    assert_eq!(
        messages[0].content,
        OpenAiCompletionsMessageContent::Text("You are concise.".into())
    );
}

#[test]
fn inserts_assistant_bridge_before_user_after_tool_results_when_required() {
    let model = model("anthropic-proxy", "claude-proxy", false, &["text"]);
    let compat = OpenAiCompletionsCompat {
        requires_assistant_after_tool_result: true,
        ..OpenAiCompletionsCompat::default()
    };
    let context = Context {
        system_prompt: None,
        messages: vec![
            Message::ToolResult {
                tool_call_id: "tool-1".into(),
                tool_name: "read".into(),
                content: vec![UserContent::Text {
                    text: "done".into(),
                }],
                details: None,
                is_error: false,
                timestamp: 1,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "What next?".into(),
                }],
                timestamp: 2,
            },
        ],
        tools: vec![],
    };

    let messages = convert_openai_completions_messages(&model, &context, &compat);
    let roles = messages
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["tool", "assistant", "user"]);
    match &messages[1].content {
        OpenAiCompletionsMessageContent::Text(text) => {
            assert_eq!(text, "I have processed the tool results.")
        }
        other => panic!("expected assistant bridge text, got {other:?}"),
    }
}
