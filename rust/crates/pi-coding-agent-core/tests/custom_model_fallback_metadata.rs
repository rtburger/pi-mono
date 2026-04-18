use pi_coding_agent_core::{ModelCatalog, resolve_cli_model};
use pi_events::{
    Model, ModelCompat, ModelCost, ModelRouting, OpenAiCompletionsCompatConfig,
    OpenAiThinkingFormat,
};

fn openrouter_model() -> Model {
    Model {
        id: "anthropic/claude-sonnet-4-5".into(),
        name: "Claude Sonnet via OpenRouter".into(),
        api: "openai-completions".into(),
        provider: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        cost: ModelCost {
            input: 0.5,
            output: 1.5,
            cache_read: 0.1,
            cache_write: 0.2,
        },
        context_window: 200_000,
        max_tokens: 16_384,
        compat: Some(ModelCompat::OpenAiCompletions(
            OpenAiCompletionsCompatConfig {
                thinking_format: Some(OpenAiThinkingFormat::OpenRouter),
                open_router_routing: Some(ModelRouting {
                    only: None,
                    order: Some(vec!["anthropic".into()]),
                }),
                requires_tool_result_name: Some(true),
                ..OpenAiCompletionsCompatConfig::default()
            },
        )),
    }
}

#[test]
fn custom_model_id_fallback_preserves_openai_metadata_and_costs() {
    let base_model = openrouter_model();
    let catalog = ModelCatalog::new(vec![base_model.clone()], vec![base_model.clone()]);

    let result = resolve_cli_model(&catalog, Some("openrouter"), Some("my-custom-model"));
    let model = result.model.expect("resolved fallback model");

    assert_eq!(model.provider, "openrouter");
    assert_eq!(model.id, "my-custom-model");
    assert_eq!(model.name, "my-custom-model");
    assert_eq!(model.base_url, base_model.base_url);
    assert_eq!(model.cost, base_model.cost);
    assert_eq!(model.compat, base_model.compat);
    assert!(
        result
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Using custom model id"))
    );
}
