use pi_ai::anthropic_messages::{
    AnthropicContentBlock, AnthropicMessageContent, AnthropicOptions,
    build_anthropic_request_params, normalize_anthropic_tool_call_id,
};
use pi_ai::openai_codex_responses::{
    OpenAiCodexResponsesRequestOptions, OpenAiCodexResponsesToolDefinition,
    build_openai_codex_responses_request_params,
};
use pi_ai::openai_responses::{
    ResponsesContentPart, ResponsesFunctionCallOutput, ResponsesInputItem, tool_call_arguments,
};
use pi_events::{AssistantContent, Context, Message, Model, StopReason, ToolDefinition, Usage, UserContent};
use serde_json::json;

fn openai_codex_model() -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url: "https://chatgpt.com/backend-api".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 272_000,
        max_tokens: 128_000,
    }
}

fn anthropic_model() -> Model {
    Model {
        id: "claude-sonnet-4-5".into(),
        name: "claude-sonnet-4-5".into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url: "https://api.anthropic.com/v1".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 200_000,
        max_tokens: 8_192,
    }
}

fn edit_tool() -> ToolDefinition {
    ToolDefinition {
        name: "edit".into(),
        description: "Edit a file".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        }),
    }
}

fn anthropic_to_codex_context() -> Context {
    Context {
        system_prompt: Some("You are concise.".into()),
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Use the tool to double 21.".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![
                    AssistantContent::Thinking {
                        thinking: "I should think first.".into(),
                        thinking_signature: Some("sig_1".into()),
                        redacted: false,
                    },
                    AssistantContent::ToolCall {
                        id: "toolu_123".into(),
                        name: "edit".into(),
                        arguments: tool_call_arguments(&[("path", json!("src/main.rs"))]),
                        thought_signature: None,
                    },
                ],
                api: "anthropic-messages".into(),
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into(),
                response_id: Some("msg_1".into()),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
            Message::ToolResult {
                tool_call_id: "toolu_123".into(),
                tool_name: "edit".into(),
                content: vec![UserContent::Text { text: "42".into() }],
                is_error: false,
                timestamp: 3,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "What was the result? Answer with just the number.".into(),
                }],
                timestamp: 4,
            },
        ],
        tools: vec![edit_tool()],
    }
}

fn openai_responses_to_anthropic_context() -> Context {
    Context {
        system_prompt: Some("You are concise.".into()),
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Use the tool to double 21.".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![
                    AssistantContent::Thinking {
                        thinking: "I should think first.".into(),
                        thinking_signature: Some(
                            r#"{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"I should think first."}]}"#.into(),
                        ),
                        redacted: false,
                    },
                    AssistantContent::ToolCall {
                        id: "call_123|fc_123".into(),
                        name: "edit".into(),
                        arguments: tool_call_arguments(&[("path", json!("src/main.rs"))]),
                        thought_signature: Some(
                            r#"{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"I should think first."}],"encrypted_content":"enc"}"#.into(),
                        ),
                    },
                ],
                api: "openai-responses".into(),
                provider: "openai".into(),
                model: "gpt-5-mini".into(),
                response_id: Some("resp_1".into()),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
            Message::ToolResult {
                tool_call_id: "call_123|fc_123".into(),
                tool_name: "edit".into(),
                content: vec![UserContent::Text { text: "42".into() }],
                is_error: false,
                timestamp: 3,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "What was the result? Answer with just the number.".into(),
                }],
                timestamp: 4,
            },
        ],
        tools: vec![edit_tool()],
    }
}

#[test]
fn replays_anthropic_tool_turn_into_openai_codex_request() {
    let params = build_openai_codex_responses_request_params(
        &openai_codex_model(),
        &anthropic_to_codex_context(),
        &OpenAiCodexResponsesRequestOptions::default(),
    );

    assert_eq!(params.model, "gpt-5.2-codex");
    assert_eq!(params.instructions.as_deref(), Some("You are concise."));
    assert_eq!(params.tool_choice, "auto");
    assert!(params.parallel_tool_calls);
    assert_eq!(params.text.verbosity, "medium");
    assert_eq!(params.include, vec!["reasoning.encrypted_content".to_string()]);
    assert!(params.reasoning.is_none());

    let tools = params.tools.as_ref().expect("expected codex tool definitions");
    assert_eq!(tools.len(), 1);
    match &tools[0] {
        OpenAiCodexResponsesToolDefinition::Function { name, strict, .. } => {
            assert_eq!(name, "edit");
            assert!(strict.is_none());
        }
    }

    assert_eq!(params.input.len(), 5);
    assert!(
        params
            .input
            .iter()
            .all(|item| !matches!(item, ResponsesInputItem::Reasoning { .. }))
    );

    match &params.input[0] {
        ResponsesInputItem::Message {
            role,
            content,
            status,
            ..
        } => {
            assert_eq!(role, "user");
            assert!(status.is_none());
            assert_eq!(
                content,
                &vec![ResponsesContentPart::InputText {
                    text: "Use the tool to double 21.".into(),
                }]
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }

    match &params.input[1] {
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
                &vec![ResponsesContentPart::OutputText {
                    text: "I should think first.".into(),
                    annotations: Vec::new(),
                }]
            );
        }
        other => panic!("expected assistant message, got {other:?}"),
    }

    match &params.input[2] {
        ResponsesInputItem::FunctionCall {
            id,
            call_id,
            name,
            arguments,
        } => {
            assert!(id.is_none());
            assert_eq!(call_id, "toolu_123");
            assert_eq!(name, "edit");
            assert_eq!(arguments, r#"{"path":"src/main.rs"}"#);
        }
        other => panic!("expected function call, got {other:?}"),
    }

    match &params.input[3] {
        ResponsesInputItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "toolu_123");
            assert_eq!(output, &ResponsesFunctionCallOutput::Text("42".into()));
        }
        other => panic!("expected function call output, got {other:?}"),
    }

    match &params.input[4] {
        ResponsesInputItem::Message {
            role,
            content,
            status,
            ..
        } => {
            assert_eq!(role, "user");
            assert!(status.is_none());
            assert_eq!(
                content,
                &vec![ResponsesContentPart::InputText {
                    text: "What was the result? Answer with just the number.".into(),
                }]
            );
        }
        other => panic!("expected follow-up user message, got {other:?}"),
    }
}

#[test]
fn replays_openai_responses_tool_turn_into_anthropic_request() {
    let params = build_anthropic_request_params(
        &anthropic_model(),
        &openai_responses_to_anthropic_context(),
        false,
        &AnthropicOptions::default(),
    );

    assert_eq!(params.system.as_ref().unwrap()[0].text, "You are concise.");
    assert_eq!(params.messages.len(), 4);

    match &params.messages[1].content {
        AnthropicMessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            match &blocks[0] {
                AnthropicContentBlock::Text { text, cache_control } => {
                    assert_eq!(text, "I should think first.");
                    assert!(cache_control.is_none());
                }
                other => panic!("expected replayed thinking as text, got {other:?}"),
            }
            match &blocks[1] {
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    assert_eq!(id, &normalize_anthropic_tool_call_id("call_123|fc_123"));
                    assert_eq!(name, "edit");
                    assert_eq!(input, &tool_call_arguments(&[("path", json!("src/main.rs"))]));
                }
                other => panic!("expected replayed tool use, got {other:?}"),
            }
        }
        other => panic!("expected assistant blocks, got {other:?}"),
    }

    match &params.messages[2].content {
        AnthropicMessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 1);
            match &blocks[0] {
                AnthropicContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    cache_control,
                } => {
                    assert_eq!(tool_use_id, &normalize_anthropic_tool_call_id("call_123|fc_123"));
                    assert!(!is_error);
                    assert!(cache_control.is_none());
                    assert_eq!(content, &AnthropicMessageContent::Text("42".into()));
                }
                other => panic!("expected replayed tool result, got {other:?}"),
            }
        }
        other => panic!("expected tool result blocks, got {other:?}"),
    }

    match &params.messages[3].content {
        AnthropicMessageContent::Text(text) => {
            assert_eq!(text, "What was the result? Answer with just the number.");
        }
        AnthropicMessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 1);
            match &blocks[0] {
                AnthropicContentBlock::Text { text, .. } => {
                    assert_eq!(text, "What was the result? Answer with just the number.");
                }
                other => panic!("expected follow-up text block, got {other:?}"),
            }
        }
    }
}
