use pi_ai::openai_completions::{
    OpenAiCompletionsCompat, OpenAiCompletionsMaxTokensField, OpenAiCompletionsRequestOptions,
    OpenAiCompletionsThinkingFormat, OpenAiCompletionsToolChoice, OpenAiCompletionsToolChoiceMode,
    ReasoningEffort, build_openai_completions_request_params, detect_openai_completions_compat,
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
fn uses_openrouter_reasoning_object_instead_of_reasoning_effort() {
    let model = model(
        "openrouter",
        "deepseek/deepseek-r1",
        "https://openrouter.ai/api/v1",
        true,
    );
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

    assert_eq!(params.reasoning_effort, None);
    assert_eq!(
        params.reasoning.as_ref().map(|value| value.effort.as_str()),
        Some("high")
    );
}

#[test]
fn detects_groq_qwen_reasoning_mapping() {
    let model = model(
        "groq",
        "qwen/qwen3-32b",
        "https://api.groq.com/openai/v1",
        true,
    );
    let compat = detect_openai_completions_compat(&model);

    assert_eq!(
        compat.reasoning_effort_map.get(&ReasoningEffort::Medium),
        Some(&"default".to_string())
    );

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
            reasoning_effort: Some(ReasoningEffort::Medium),
            ..OpenAiCompletionsRequestOptions::default()
        },
    );

    assert_eq!(params.reasoning_effort.as_deref(), Some("default"));
}

#[test]
fn detects_zai_tool_stream_models_and_sets_tool_stream() {
    let model = model("zai", "glm-5", "https://api.z.ai/api/paas/v4", true);
    let compat = detect_openai_completions_compat(&model);

    assert!(compat.zai_tool_stream);
    assert_eq!(compat.thinking_format, OpenAiCompletionsThinkingFormat::Zai);
    assert!(!compat.supports_reasoning_effort);

    let params = build_openai_completions_request_params(
        &model,
        &context_with_tool(),
        &compat,
        &OpenAiCompletionsRequestOptions {
            reasoning_effort: Some(ReasoningEffort::High),
            ..OpenAiCompletionsRequestOptions::default()
        },
    );

    assert_eq!(params.tool_stream, Some(true));
    assert_eq!(params.enable_thinking, Some(true));
    assert_eq!(params.reasoning_effort, None);
}

#[test]
fn detects_chutes_max_tokens_field() {
    let model = model("custom", "model", "https://llm.chutes.ai/v1", false);
    let compat = detect_openai_completions_compat(&model);
    assert_eq!(
        compat.max_tokens_field,
        OpenAiCompletionsMaxTokensField::MaxTokens
    );
}
