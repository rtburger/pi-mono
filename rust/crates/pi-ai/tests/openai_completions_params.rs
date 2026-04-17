use pi_ai::openai_completions::{
    ModelRouting, OpenAiCompletionsCompat, OpenAiCompletionsContentPart,
    OpenAiCompletionsMaxTokensField, OpenAiCompletionsMessageContent, OpenAiCompletionsReasoning,
    OpenAiCompletionsRequestOptions, OpenAiCompletionsToolChoice, OpenAiCompletionsToolChoiceMode,
    OpenAiThinkingFormat, ReasoningEffort, build_openai_completions_request_params,
    detect_openai_completions_compat,
};
use pi_events::{
    Context, Model, ModelCompat, ModelCost, OpenAiCompletionsCompatConfig, ToolDefinition,
    UserContent,
};
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
        cost: ModelCost::default(),
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
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
        assert_eq!(compat.thinking_format, OpenAiThinkingFormat::OpenAi);
        assert!(compat.reasoning_effort_map.is_empty());
    }
}

#[test]
fn detects_non_standard_provider_compat_and_reasoning_mappings() {
    let grok =
        detect_openai_completions_compat(&model("xai", "grok-4", "https://api.x.ai/v1", true));
    assert!(!grok.supports_store);
    assert!(!grok.supports_developer_role);
    assert!(!grok.supports_reasoning_effort);
    assert_eq!(grok.thinking_format, OpenAiThinkingFormat::OpenAi);

    let groq_qwen = detect_openai_completions_compat(&model(
        "groq",
        "qwen/qwen3-32b",
        "https://api.groq.com/openai/v1",
        true,
    ));
    assert_eq!(
        groq_qwen
            .reasoning_effort_map
            .get("high")
            .map(String::as_str),
        Some("default")
    );
    assert_eq!(
        groq_qwen
            .reasoning_effort_map
            .get("xhigh")
            .map(String::as_str),
        Some("default")
    );
}

#[test]
fn applies_model_compat_overrides_to_openrouter_request_shapes() {
    let mut model = model(
        "openrouter",
        "anthropic/claude-sonnet-4-5",
        "https://openrouter.ai/api/v1",
        true,
    );
    model.compat = Some(ModelCompat::OpenAiCompletions(
        OpenAiCompletionsCompatConfig {
            open_router_routing: Some(ModelRouting {
                only: Some(vec!["anthropic".into()]),
                order: Some(vec!["openai".into()]),
            }),
            ..OpenAiCompletionsCompatConfig::default()
        },
    ));

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

    assert_eq!(compat.thinking_format, OpenAiThinkingFormat::OpenRouter);
    assert_eq!(
        params.reasoning,
        Some(OpenAiCompletionsReasoning {
            effort: "high".into(),
        })
    );
    assert_eq!(
        params.provider,
        Some(ModelRouting {
            only: Some(vec!["anthropic".into()]),
            order: Some(vec!["openai".into()]),
        })
    );
    assert_eq!(params.reasoning_effort, None);
}

#[test]
fn applies_zai_enable_thinking_and_tool_stream_overrides() {
    let mut model = model("zai", "glm-4.5", "https://api.z.ai/v1", true);
    model.compat = Some(ModelCompat::OpenAiCompletions(
        OpenAiCompletionsCompatConfig {
            zai_tool_stream: Some(true),
            ..OpenAiCompletionsCompatConfig::default()
        },
    ));

    let compat = detect_openai_completions_compat(&model);
    let params = build_openai_completions_request_params(
        &model,
        &context_with_tool(),
        &compat,
        &OpenAiCompletionsRequestOptions {
            reasoning_effort: Some(ReasoningEffort::Medium),
            ..OpenAiCompletionsRequestOptions::default()
        },
    );

    assert_eq!(compat.thinking_format, OpenAiThinkingFormat::Zai);
    assert_eq!(params.enable_thinking, Some(true));
    assert_eq!(params.tool_stream, Some(true));
    assert_eq!(params.reasoning_effort, None);
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
fn adds_openrouter_anthropic_cache_control_to_last_text_message_part() {
    let model = model(
        "openrouter",
        "anthropic/claude-sonnet-4-5",
        "https://openrouter.ai/api/v1",
        false,
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
        &OpenAiCompletionsRequestOptions::default(),
    );

    match &params.messages[0].content {
        OpenAiCompletionsMessageContent::Parts(parts) => match &parts[0] {
            OpenAiCompletionsContentPart::Text {
                text,
                cache_control,
            } => {
                assert_eq!(text, "Hi");
                assert_eq!(
                    serde_json::to_value(cache_control).unwrap(),
                    json!({"type": "ephemeral"})
                );
            }
            other => panic!("expected cached text part, got {other:?}"),
        },
        other => panic!("expected multipart content, got {other:?}"),
    }
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
