use pi_ai::anthropic_messages::{
    AnthropicContentBlock, AnthropicMessageContent, convert_anthropic_messages,
};
use pi_ai::openai_codex_responses::build_openai_codex_responses_request_params;
use pi_ai::openai_completions::{
    OpenAiCompletionsCompat, OpenAiCompletionsMessageContent, convert_openai_completions_messages,
};
use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, ResponsesFunctionCallOutput, ResponsesInputItem,
    convert_openai_responses_messages,
};
use pi_events::{AssistantContent, Context, Message, Model, StopReason, Usage, UserContent};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn tool_call_arguments(arguments: &[(&str, Value)]) -> BTreeMap<String, Value> {
    arguments
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
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
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 200_000,
        max_tokens: 8_192,
        compat: None,
    }
}

fn openai_responses_model() -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
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

fn openai_completions_model() -> Model {
    Model {
        id: "gpt-4o-mini".into(),
        name: "gpt-4o-mini".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: false,
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

fn openai_codex_model() -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url: "https://chatgpt.com/backend-api".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 272_000,
        max_tokens: 128_000,
        compat: None,
    }
}

fn usage() -> Usage {
    Usage::default()
}

fn anthropic_context() -> Context {
    Context {
        system_prompt: None,
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Please calculate 25 * 18 using the calculate tool.".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: "tool_123".into(),
                    name: "calculate".into(),
                    arguments: tool_call_arguments(&[("expression", json!("25 * 18"))]),
                    thought_signature: None,
                }],
                api: "anthropic-messages".into(),
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "Never mind, just tell me what is 2 + 2?".into(),
                }],
                timestamp: 3,
            },
        ],
        tools: vec![],
    }
}

fn openai_context(api: &str, provider: &str, model_id: &str) -> Context {
    Context {
        system_prompt: None,
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Please calculate 25 * 18 using the calculate tool.".into(),
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
                api: api.into(),
                provider: provider.into(),
                model: model_id.into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "Never mind, just tell me what is 2 + 2?".into(),
                }],
                timestamp: 3,
            },
        ],
        tools: vec![],
    }
}

#[test]
fn anthropic_inserts_synthetic_tool_result_before_followup_user_message() {
    let converted = convert_anthropic_messages(
        &anthropic_context().messages,
        &anthropic_model(),
        false,
        None,
    );

    let roles = converted
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["user", "assistant", "user", "user"]);

    match &converted[2].content {
        AnthropicMessageContent::Blocks(blocks) => match &blocks[0] {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "tool_123");
                assert!(*is_error);
                assert_eq!(
                    content,
                    &AnthropicMessageContent::Text("No result provided".into())
                );
            }
            other => panic!("expected synthetic tool_result block, got {other:?}"),
        },
        other => panic!("expected synthetic tool_result user message, got {other:?}"),
    }

    match &converted[3].content {
        AnthropicMessageContent::Text(text) => {
            assert_eq!(text, "Never mind, just tell me what is 2 + 2?")
        }
        other => panic!("expected follow-up user text, got {other:?}"),
    }
}

#[test]
fn openai_responses_inserts_synthetic_function_call_output_before_followup_user_message() {
    let items = convert_openai_responses_messages(
        &openai_responses_model(),
        &openai_context("openai-responses", "openai", "gpt-5-mini"),
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
        ResponsesInputItem::Message { role, content, .. } => {
            assert_eq!(role, "user");
            assert_eq!(
                content,
                &vec![pi_ai::openai_responses::ResponsesContentPart::InputText {
                    text: "Never mind, just tell me what is 2 + 2?".into(),
                }]
            );
        }
        other => panic!("expected follow-up user message, got {other:?}"),
    }
}

#[test]
fn openai_completions_inserts_synthetic_tool_message_before_followup_user_message() {
    let messages = convert_openai_completions_messages(
        &openai_completions_model(),
        &openai_context("openai-completions", "openai", "gpt-4o-mini"),
        &OpenAiCompletionsCompat::default(),
    );

    let roles = messages
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["user", "assistant", "tool", "user"]);

    assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_123|fc_123"));
    match &messages[2].content {
        OpenAiCompletionsMessageContent::Text(text) => assert_eq!(text, "No result provided"),
        other => panic!("expected synthetic tool text, got {other:?}"),
    }
    match &messages[3].content {
        OpenAiCompletionsMessageContent::Text(text) => {
            assert_eq!(text, "Never mind, just tell me what is 2 + 2?")
        }
        other => panic!("expected follow-up user text, got {other:?}"),
    }
}

#[test]
fn openai_codex_inserts_synthetic_function_call_output_before_followup_user_message() {
    let params = build_openai_codex_responses_request_params(
        &openai_codex_model(),
        &openai_context("openai-codex-responses", "openai-codex", "gpt-5.2-codex"),
        &Default::default(),
    );

    assert!(matches!(
        params.input[0],
        ResponsesInputItem::Message { .. }
    ));
    assert!(matches!(
        params.input[1],
        ResponsesInputItem::FunctionCall { .. }
    ));
    match &params.input[2] {
        ResponsesInputItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "call_123");
            assert_eq!(
                output,
                &ResponsesFunctionCallOutput::Text("No result provided".into())
            );
        }
        other => panic!("expected synthetic function_call_output, got {other:?}"),
    }
    match &params.input[3] {
        ResponsesInputItem::Message { role, content, .. } => {
            assert_eq!(role, "user");
            assert_eq!(
                content,
                &vec![pi_ai::openai_responses::ResponsesContentPart::InputText {
                    text: "Never mind, just tell me what is 2 + 2?".into(),
                }]
            );
        }
        other => panic!("expected follow-up user message, got {other:?}"),
    }
}
