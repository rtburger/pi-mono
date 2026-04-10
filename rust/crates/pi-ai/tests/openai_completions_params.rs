use pi_ai::openai_completions::{
    OpenAiCompletionsCompat, OpenAiCompletionsMaxTokensField, OpenAiCompletionsRequestOptions,
    OpenAiCompletionsToolChoice, OpenAiCompletionsToolChoiceMode, ReasoningEffort,
    build_openai_completions_request_params, detect_openai_completions_compat,
};
use pi_events::{Context, Model, ToolDefinition, UserContent};
use serde_json::json;

fn model(provider: &str, id: &str, base_url: &str, reasoning: bool) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "openai-completions".into(),
        provider: provider.into(),
        base_url: base_url.into(),
        reasoning,
        input: vec!["text".into(), "image".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn tool() -> ToolDefinition {
    ToolDefinition {
        name: "ping".into(),
        description: "Ping tool".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "ok": { "type": "boolean" }
            },
            "required": ["ok"]
        }),
    }
}

fn context_with_tool() -> Context {
    Context {
        system_prompt: None,
        messages: vec![pi_events::Message::User {
            content: vec![UserContent::Text {
                text: "Call ping".into(),
            }],
            timestamp: 1,
        }],
        tools: vec![tool()],
    }
}

#[test]
fn forwards_tool_choice_and_includes_strict_false_by_default() {
    let model = model("openai", "gpt-4o-mini", "https://api.openai.com/v1", false);
    let compat = OpenAiCompletionsCompat::default();
    let context = context_with_tool();
    let params = build_openai_completions_request_params(
        &model,
        &context,
        &compat,
        &OpenAiCompletionsRequestOptions {
            tool_choice: Some(OpenAiCompletionsToolChoice::Mode(
                OpenAiCompletionsToolChoiceMode::Required,
            )),
            ..OpenAiCompletionsRequestOptions::default()
        },
    );

    assert_eq!(
        params.tool_choice,
        Some(OpenAiCompletionsToolChoice::Mode(
            OpenAiCompletionsToolChoiceMode::Required,
        ))
    );
    let tools = params.tools.expect("expected tools");
    assert_eq!(tools.len(), 1);
    match &tools[0] {
        pi_ai::openai_completions::OpenAiCompletionsToolDefinition::Function { function } => {
            assert_eq!(function.strict, Some(false));
        }
    }
}

#[test]
fn omits_strict_when_compat_disables_strict_mode() {
    let model = model("openai", "gpt-4o-mini", "https://api.openai.com/v1", false);
    let compat = OpenAiCompletionsCompat {
        supports_strict_mode: false,
        ..OpenAiCompletionsCompat::default()
    };
    let params = build_openai_completions_request_params(
        &model,
        &context_with_tool(),
        &compat,
        &OpenAiCompletionsRequestOptions::default(),
    );

    let tools = params.tools.expect("expected tools");
    match &tools[0] {
        pi_ai::openai_completions::OpenAiCompletionsToolDefinition::Function { function } => {
            assert_eq!(function.strict, None);
        }
    }
}

#[test]
fn detects_default_openai_compat_for_supported_providers() {
    let openai = detect_openai_completions_compat(&model(
        "openai",
        "gpt-4o-mini",
        "https://api.openai.com/v1",
        false,
    ));
    let codex = detect_openai_completions_compat(&model(
        "openai-codex",
        "gpt-5.4",
        "https://chatgpt.com/backend-api",
        true,
    ));

    for compat in [openai, codex] {
        assert!(compat.supports_store);
        assert!(compat.supports_developer_role);
        assert!(compat.supports_reasoning_effort);
        assert_eq!(
            compat.max_tokens_field,
            OpenAiCompletionsMaxTokensField::MaxCompletionTokens
        );
        assert!(compat.reasoning_effort_map.is_empty());
    }
}

#[test]
fn applies_reasoning_effort_for_reasoning_models() {
    let model = model("openai", "gpt-5-mini", "https://api.openai.com/v1", true);
    let compat = detect_openai_completions_compat(&model);
    let params = build_openai_completions_request_params(
        &model,
        &Context {
            system_prompt: None,
            messages: vec![pi_events::Message::User {
                content: vec![UserContent::Text { text: "Hi".into() }],
                timestamp: 1,
            }],
            tools: vec![],
        },
        &compat,
        &OpenAiCompletionsRequestOptions {
            reasoning_effort: Some(ReasoningEffort::High),
            ..OpenAiCompletionsRequestOptions::default()
        },
    );

    assert_eq!(params.reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn supports_max_tokens_field_overrides_for_runtime_compat() {
    let model = model("openai", "gpt-4o-mini", "https://api.openai.com/v1", false);
    let compat = OpenAiCompletionsCompat {
        max_tokens_field: OpenAiCompletionsMaxTokensField::MaxTokens,
        ..OpenAiCompletionsCompat::default()
    };
    let params = build_openai_completions_request_params(
        &model,
        &Context {
            system_prompt: None,
            messages: vec![pi_events::Message::User {
                content: vec![UserContent::Text { text: "Hi".into() }],
                timestamp: 1,
            }],
            tools: vec![],
        },
        &compat,
        &OpenAiCompletionsRequestOptions {
            max_tokens: Some(321),
            ..OpenAiCompletionsRequestOptions::default()
        },
    );

    assert_eq!(params.max_completion_tokens, None);
    assert_eq!(params.max_tokens, Some(321));
}
