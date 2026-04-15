use pi_ai::anthropic_messages::{
    AnthropicContentBlock, AnthropicMessageContent, convert_anthropic_messages,
};
use pi_ai::openai_codex_responses::build_openai_codex_responses_request_params;
use pi_ai::openai_completions::{
    OpenAiCompletionsCompat, OpenAiCompletionsContentPart, OpenAiCompletionsMessageContent,
    convert_openai_completions_messages,
};
use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, ResponsesContentPart, ResponsesFunctionCallOutput,
    ResponsesInputItem, convert_openai_responses_messages,
};
use pi_events::{AssistantContent, Context, Message, Model, StopReason, Usage, UserContent};
use serde_json::Value;
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

fn tool_result_context(api: &str, provider: &str, model_id: &str) -> Context {
    Context {
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
                api: api.into(),
                provider: provider.into(),
                model: model_id.into(),
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
                details: None,
                is_error: false,
                timestamp: 3,
            },
        ],
        tools: vec![],
    }
}

fn image_only_tool_result_context(api: &str, provider: &str, model_id: &str) -> Context {
    Context {
        system_prompt: Some("Use the tool when asked.".into()),
        messages: vec![
            Message::User {
                content: vec![UserContent::Text {
                    text: "Call get_circle and summarize it.".into(),
                }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: "call_456|fc_456".into(),
                    name: "get_circle".into(),
                    arguments: tool_call_arguments(&[]),
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
            Message::ToolResult {
                tool_call_id: "call_456|fc_456".into(),
                tool_name: "get_circle".into(),
                content: vec![UserContent::Image {
                    data: "ZmFrZS1iYXNlNjQ=".into(),
                    mime_type: "image/png".into(),
                }],
                details: None,
                is_error: false,
                timestamp: 3,
            },
        ],
        tools: vec![],
    }
}

#[test]
fn anthropic_keeps_text_and_image_inside_tool_result_block() {
    let converted = convert_anthropic_messages(
        &tool_result_context("anthropic-messages", "anthropic", "claude-sonnet-4-5").messages,
        &anthropic_model(),
        false,
        None,
    );

    assert_eq!(converted.len(), 3);
    assert_eq!(converted[2].role, "user");

    match &converted[2].content {
        AnthropicMessageContent::Blocks(blocks) => match &blocks[0] {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "call_123|fc_123");
                match content {
                    AnthropicMessageContent::Blocks(tool_result_blocks) => {
                        assert_eq!(tool_result_blocks.len(), 2);
                        match &tool_result_blocks[0] {
                            AnthropicContentBlock::Text { text, .. } => {
                                assert_eq!(text, "A red circle with a diameter of 100 pixels.");
                            }
                            other => panic!("expected text block, got {other:?}"),
                        }
                        match &tool_result_blocks[1] {
                            AnthropicContentBlock::Image { source, .. } => {
                                assert_eq!(source.media_type, "image/png");
                                assert_eq!(source.data, "ZmFrZS1iYXNlNjQ=");
                            }
                            other => panic!("expected image block, got {other:?}"),
                        }
                    }
                    other => panic!("expected nested tool-result blocks, got {other:?}"),
                }
            }
            other => panic!("expected tool_result block, got {other:?}"),
        },
        other => panic!("expected user block content, got {other:?}"),
    }
}

#[test]
fn anthropic_inserts_placeholder_text_for_image_only_tool_result() {
    let converted = convert_anthropic_messages(
        &image_only_tool_result_context("anthropic-messages", "anthropic", "claude-sonnet-4-5")
            .messages,
        &anthropic_model(),
        false,
        None,
    );

    match &converted[2].content {
        AnthropicMessageContent::Blocks(blocks) => match &blocks[0] {
            AnthropicContentBlock::ToolResult { content, .. } => match content {
                AnthropicMessageContent::Blocks(tool_result_blocks) => {
                    assert_eq!(tool_result_blocks.len(), 2);
                    match &tool_result_blocks[0] {
                        AnthropicContentBlock::Text { text, .. } => {
                            assert_eq!(text, "(see attached image)");
                        }
                        other => panic!("expected placeholder text block, got {other:?}"),
                    }
                    assert!(matches!(
                        tool_result_blocks[1],
                        AnthropicContentBlock::Image { .. }
                    ));
                }
                other => panic!("expected nested tool-result blocks, got {other:?}"),
            },
            other => panic!("expected tool_result block, got {other:?}"),
        },
        other => panic!("expected user block content, got {other:?}"),
    }
}

#[test]
fn openai_responses_keeps_tool_result_images_inside_function_call_output() {
    let items = convert_openai_responses_messages(
        &openai_responses_model(),
        &tool_result_context("openai-responses", "openai", "gpt-5-mini"),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
    );

    let function_call_output_index = items
        .iter()
        .position(|item| matches!(item, ResponsesInputItem::FunctionCallOutput { .. }))
        .expect("missing function_call_output");

    match &items[function_call_output_index] {
        ResponsesInputItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "call_123");
            match output {
                ResponsesFunctionCallOutput::Content(parts) => {
                    assert_eq!(parts.len(), 2);
                    match &parts[0] {
                        ResponsesContentPart::InputText { text } => {
                            assert_eq!(text, "A red circle with a diameter of 100 pixels.");
                        }
                        other => panic!("expected input_text part, got {other:?}"),
                    }
                    match &parts[1] {
                        ResponsesContentPart::InputImage { image_url, .. } => {
                            assert_eq!(image_url, "data:image/png;base64,ZmFrZS1iYXNlNjQ=");
                        }
                        other => panic!("expected input_image part, got {other:?}"),
                    }
                }
                other => panic!("expected structured function_call_output, got {other:?}"),
            }
        }
        other => panic!("expected function_call_output, got {other:?}"),
    }

    let later_user_messages = items[function_call_output_index + 1..]
        .iter()
        .filter(|item| matches!(item, ResponsesInputItem::Message { role, .. } if role == "user"))
        .count();
    assert_eq!(later_user_messages, 0);
}

#[test]
fn openai_completions_moves_tool_result_images_to_followup_user_message() {
    let messages = convert_openai_completions_messages(
        &openai_completions_model(),
        &tool_result_context("openai-completions", "openai", "gpt-4o-mini"),
        &OpenAiCompletionsCompat::default(),
    );

    let roles = messages
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["system", "user", "assistant", "tool", "user"]);

    match &messages[3].content {
        OpenAiCompletionsMessageContent::Text(text) => {
            assert_eq!(text, "A red circle with a diameter of 100 pixels.");
        }
        other => panic!("expected tool text content, got {other:?}"),
    }

    match &messages[4].content {
        OpenAiCompletionsMessageContent::Parts(parts) => {
            assert_eq!(parts.len(), 2);
            match &parts[0] {
                OpenAiCompletionsContentPart::Text { text } => {
                    assert_eq!(text, "Attached image(s) from tool result:");
                }
                other => panic!("expected follow-up text part, got {other:?}"),
            }
            match &parts[1] {
                OpenAiCompletionsContentPart::ImageUrl { image_url } => {
                    assert_eq!(image_url.url, "data:image/png;base64,ZmFrZS1iYXNlNjQ=");
                }
                other => panic!("expected follow-up image part, got {other:?}"),
            }
        }
        other => panic!("expected multipart follow-up user message, got {other:?}"),
    }
}

#[test]
fn openai_codex_keeps_tool_result_images_inside_function_call_output() {
    let params = build_openai_codex_responses_request_params(
        &openai_codex_model(),
        &tool_result_context("openai-responses", "openai-codex", "gpt-5.2-codex"),
        &Default::default(),
    );

    let function_call_output_index = params
        .input
        .iter()
        .position(|item| matches!(item, ResponsesInputItem::FunctionCallOutput { .. }))
        .expect("missing function_call_output");

    match &params.input[function_call_output_index] {
        ResponsesInputItem::FunctionCallOutput { call_id, output } => {
            assert_eq!(call_id, "call_123");
            match output {
                ResponsesFunctionCallOutput::Content(parts) => {
                    assert_eq!(parts.len(), 2);
                    match &parts[0] {
                        ResponsesContentPart::InputText { text } => {
                            assert_eq!(text, "A red circle with a diameter of 100 pixels.");
                        }
                        other => panic!("expected input_text part, got {other:?}"),
                    }
                    match &parts[1] {
                        ResponsesContentPart::InputImage { image_url, .. } => {
                            assert_eq!(image_url, "data:image/png;base64,ZmFrZS1iYXNlNjQ=");
                        }
                        other => panic!("expected input_image part, got {other:?}"),
                    }
                }
                other => panic!("expected structured function_call_output, got {other:?}"),
            }
        }
        other => panic!("expected function_call_output, got {other:?}"),
    }

    let later_user_messages = params.input[function_call_output_index + 1..]
        .iter()
        .filter(|item| matches!(item, ResponsesInputItem::Message { role, .. } if role == "user"))
        .count();
    assert_eq!(later_user_messages, 0);

    assert_eq!(
        params.instructions.as_deref(),
        Some("Use the tool when asked.")
    );
    assert_eq!(
        params.input[0],
        ResponsesInputItem::Message {
            role: "user".into(),
            content: vec![ResponsesContentPart::InputText {
                text: "Call get_circle_with_description and summarize it.".into()
            }],
            status: None,
            id: None,
            phase: None,
        }
    );
}
