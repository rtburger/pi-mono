use pi_ai::anthropic_messages::{
    AnthropicContentBlock, AnthropicMessageContent, AnthropicOptions,
    build_anthropic_request_params,
};
use pi_ai::openai_codex_responses::{
    OpenAiCodexResponsesRequestOptions, build_openai_codex_responses_request_params,
};
use pi_ai::openai_completions::{
    OpenAiCompletionsCompat, OpenAiCompletionsMessageContent, OpenAiCompletionsRequestOptions,
    build_openai_completions_request_params,
};
use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, OpenAiResponsesParamsOptions, ResponsesContentPart,
    ResponsesFunctionCallOutput, ResponsesInputItem, build_openai_responses_request_params,
};
use pi_events::{AssistantContent, Context, Message, Model, StopReason, Usage, UserContent};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn rich_text() -> String {
    "Mario Zechner wann? Wo? Bin grad äußersr eventuninformiert 🙈\nこんにちは\n你好\n∑∫∂√".into()
}

fn lossy_surrogate_text() -> String {
    format!(
        "Lossy UTF-16 surrogate fallback: {}",
        String::from_utf16_lossy(&[0xD83D])
    )
}

fn combined_text() -> String {
    format!("{}\n{}", rich_text(), lossy_surrogate_text())
}

fn tool_arguments() -> BTreeMap<String, Value> {
    json!({ "path": "README.md" })
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn usage() -> Usage {
    Usage::default()
}

fn anthropic_model() -> Model {
    Model {
        id: "claude-sonnet-4-5".into(),
        name: "claude-sonnet-4-5".into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url: "https://api.anthropic.com/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
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
        base_url: "https://api.example.test/v1".into(),
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

fn openai_completions_model() -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: "https://api.example.test/v1".into(),
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

fn openai_codex_model() -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url: "https://api.example.test/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
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

fn anthropic_context() -> Context {
    let text = combined_text();
    Context {
        system_prompt: Some(text.clone()),
        messages: vec![
            Message::User {
                content: vec![UserContent::Text { text: text.clone() }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![
                    AssistantContent::Text {
                        text: text.clone(),
                        text_signature: None,
                    },
                    AssistantContent::Thinking {
                        thinking: text.clone(),
                        thinking_signature: Some("sig_1".into()),
                        redacted: false,
                    },
                    AssistantContent::ToolCall {
                        id: "tool_1".into(),
                        name: "read".into(),
                        arguments: tool_arguments(),
                        thought_signature: None,
                    },
                ],
                api: "anthropic-messages".into(),
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 2,
            },
            Message::ToolResult {
                tool_call_id: "tool_1".into(),
                tool_name: "read".into(),
                content: vec![UserContent::Text { text }],
                details: None,
                is_error: false,
                timestamp: 3,
            },
        ],
        tools: vec![],
    }
}

fn openai_context(api: &str, provider: &str, model_id: &str) -> Context {
    let text = combined_text();
    Context {
        system_prompt: Some(text.clone()),
        messages: vec![
            Message::User {
                content: vec![UserContent::Text { text: text.clone() }],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![
                    AssistantContent::Text {
                        text: text.clone(),
                        text_signature: None,
                    },
                    AssistantContent::Thinking {
                        thinking: text.clone(),
                        thinking_signature: Some("sig_1".into()),
                        redacted: false,
                    },
                    AssistantContent::ToolCall {
                        id: "tool_1|fc_tool_1".into(),
                        name: "read".into(),
                        arguments: tool_arguments(),
                        thought_signature: None,
                    },
                ],
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
                tool_call_id: "tool_1|fc_tool_1".into(),
                tool_name: "read".into(),
                content: vec![UserContent::Text { text }],
                details: None,
                is_error: false,
                timestamp: 3,
            },
        ],
        tools: vec![],
    }
}

#[test]
fn anthropic_request_params_preserve_unicode_and_lossy_text() {
    let text = combined_text();
    let params = build_anthropic_request_params(
        &anthropic_model(),
        &anthropic_context(),
        false,
        &AnthropicOptions::default(),
    );

    let json = serde_json::to_string(&params).expect("anthropic params should serialize");
    assert!(json.contains("🙈"));
    assert!(json.contains("こんにちは"));
    assert!(json.contains("\u{FFFD}"));

    let system = params.system.expect("missing system blocks");
    assert_eq!(system[0].text, text);

    match &params.messages[0].content {
        AnthropicMessageContent::Text(user_text) => assert_eq!(user_text, &text),
        other => panic!("expected anthropic user text message, got {other:?}"),
    }

    match &params.messages[1].content {
        AnthropicMessageContent::Blocks(blocks) => {
            assert!(matches!(
                &blocks[0],
                AnthropicContentBlock::Text { text: value, .. } if value == &text
            ));
            assert!(matches!(
                &blocks[1],
                AnthropicContentBlock::Thinking { thinking, .. } if thinking == &text
            ));
        }
        other => panic!("expected anthropic assistant blocks, got {other:?}"),
    }

    match &params.messages[2].content {
        AnthropicMessageContent::Blocks(blocks) => match &blocks[0] {
            AnthropicContentBlock::ToolResult { content, .. } => match content {
                AnthropicMessageContent::Text(tool_text) => assert_eq!(tool_text, &text),
                other => panic!("expected anthropic tool result text, got {other:?}"),
            },
            other => panic!("expected anthropic tool_result block, got {other:?}"),
        },
        other => panic!("expected anthropic tool result message, got {other:?}"),
    }
}

#[test]
fn openai_responses_request_params_preserve_unicode_and_lossy_text() {
    let text = combined_text();
    let params = build_openai_responses_request_params(
        &openai_responses_model(),
        &openai_context("openai-responses", "openai", "gpt-5-mini"),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    let json = serde_json::to_string(&params).expect("openai responses params should serialize");
    assert!(json.contains("🙈"));
    assert!(json.contains("こんにちは"));
    assert!(json.contains("\u{FFFD}"));

    match &params.input[0] {
        ResponsesInputItem::Message { content, .. } => match &content[0] {
            ResponsesContentPart::InputText { text: value } => assert_eq!(value, &text),
            other => panic!("expected openai responses system text, got {other:?}"),
        },
        other => panic!("expected openai responses system message, got {other:?}"),
    }

    match &params.input[1] {
        ResponsesInputItem::Message { content, .. } => match &content[0] {
            ResponsesContentPart::InputText { text: value } => assert_eq!(value, &text),
            other => panic!("expected openai responses user text, got {other:?}"),
        },
        other => panic!("expected openai responses user message, got {other:?}"),
    }

    match &params.input[2] {
        ResponsesInputItem::Message { content, .. } => match &content[0] {
            ResponsesContentPart::OutputText { text: value, .. } => assert_eq!(value, &text),
            other => panic!("expected openai responses assistant text, got {other:?}"),
        },
        other => panic!("expected openai responses assistant message, got {other:?}"),
    }

    match &params.input[4] {
        ResponsesInputItem::FunctionCallOutput { output, .. } => match output {
            ResponsesFunctionCallOutput::Text(value) => assert_eq!(value, &text),
            other => panic!("expected openai responses tool result text, got {other:?}"),
        },
        other => panic!("expected openai responses tool result item, got {other:?}"),
    }
}

#[test]
fn openai_completions_request_params_preserve_unicode_and_lossy_text() {
    let text = combined_text();
    let params = build_openai_completions_request_params(
        &openai_completions_model(),
        &openai_context("openai-completions", "openai", "gpt-5-mini"),
        &OpenAiCompletionsCompat {
            requires_thinking_as_text: true,
            ..OpenAiCompletionsCompat::default()
        },
        &OpenAiCompletionsRequestOptions::default(),
    );

    let json = serde_json::to_string(&params).expect("openai completions params should serialize");
    assert!(json.contains("🙈"));
    assert!(json.contains("こんにちは"));
    assert!(json.contains("\u{FFFD}"));

    assert_eq!(
        params.messages[0].content,
        OpenAiCompletionsMessageContent::Text(text.clone())
    );
    assert_eq!(
        params.messages[1].content,
        OpenAiCompletionsMessageContent::Text(text.clone())
    );
    assert_eq!(
        params.messages[2].content,
        OpenAiCompletionsMessageContent::Text(format!("{text}\n\n{text}"))
    );
    assert_eq!(
        params.messages[3].content,
        OpenAiCompletionsMessageContent::Text(text)
    );
}

#[test]
fn openai_codex_request_params_preserve_unicode_and_lossy_text() {
    let text = combined_text();
    let params = build_openai_codex_responses_request_params(
        &openai_codex_model(),
        &openai_context("openai-codex-responses", "openai-codex", "gpt-5.2-codex"),
        &OpenAiCodexResponsesRequestOptions::default(),
    );

    let json = serde_json::to_string(&params).expect("openai codex params should serialize");
    assert!(json.contains("🙈"));
    assert!(json.contains("こんにちは"));
    assert!(json.contains("\u{FFFD}"));

    assert_eq!(params.instructions.as_deref(), Some(text.as_str()));

    match &params.input[0] {
        ResponsesInputItem::Message { content, .. } => match &content[0] {
            ResponsesContentPart::InputText { text: value } => assert_eq!(value, &text),
            other => panic!("expected codex user text, got {other:?}"),
        },
        other => panic!("expected codex user message, got {other:?}"),
    }

    match &params.input[1] {
        ResponsesInputItem::Message { content, .. } => match &content[0] {
            ResponsesContentPart::OutputText { text: value, .. } => assert_eq!(value, &text),
            other => panic!("expected codex assistant text, got {other:?}"),
        },
        other => panic!("expected codex assistant message, got {other:?}"),
    }

    match &params.input[3] {
        ResponsesInputItem::FunctionCallOutput { output, .. } => match output {
            ResponsesFunctionCallOutput::Text(value) => assert_eq!(value, &text),
            other => panic!("expected codex tool result text, got {other:?}"),
        },
        other => panic!("expected codex tool result item, got {other:?}"),
    }
}
