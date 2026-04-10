use pi_agent::ThinkingLevel;
use pi_coding_agent_core::{
    DEFAULT_MODELS, DEFAULT_THINKING_LEVEL, InitialModelOptions, ModelCatalog,
    ParseModelPatternOptions, ScopedModel, default_model_id_for_provider, find_initial_model,
    parse_model_pattern, resolve_cli_model, resolve_model_scope, restore_model_from_session,
};
use pi_events::Model;

fn mock_model(provider: &str, id: &str, name: &str, reasoning: bool) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        api: "test-api".into(),
        provider: provider.into(),
        base_url: format!("https://{provider}.example.com"),
        reasoning,
        input: vec!["text".into(), "image".into()],
        context_window: 128_000,
        max_tokens: 8_192,
    }
}

fn base_models() -> Vec<Model> {
    vec![
        mock_model("anthropic", "claude-sonnet-4-5", "Claude Sonnet 4.5", true),
        mock_model("openai", "gpt-4o", "GPT-4o", false),
        mock_model("openai-codex", "gpt-5.4:exacto", "GPT-5.4 Exacto", true),
        mock_model("openai-codex", "gpt-5.4:extended", "GPT-5.4 Extended", true),
    ]
}

#[test]
fn parse_model_pattern_matches_simple_patterns() {
    let result = parse_model_pattern(
        "claude-sonnet-4-5",
        &base_models(),
        ParseModelPatternOptions::default(),
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(result.thinking_level, None);
    assert_eq!(result.warning, None);
}

#[test]
fn parse_model_pattern_extracts_valid_thinking_level() {
    let result = parse_model_pattern(
        "sonnet:high",
        &base_models(),
        ParseModelPatternOptions::default(),
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(result.thinking_level, Some(ThinkingLevel::High));
    assert_eq!(result.warning, None);
}

#[test]
fn parse_model_pattern_handles_model_ids_with_colons() {
    let result = parse_model_pattern(
        "gpt-5.4:exacto:high",
        &base_models(),
        ParseModelPatternOptions::default(),
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-5.4:exacto")
    );
    assert_eq!(result.thinking_level, Some(ThinkingLevel::High));
    assert_eq!(result.warning, None);
}

#[test]
fn parse_model_pattern_warns_on_invalid_thinking_level_in_scope_mode() {
    let result = parse_model_pattern(
        "gpt-5.4:exacto:random",
        &base_models(),
        ParseModelPatternOptions::default(),
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-5.4:exacto")
    );
    assert_eq!(result.thinking_level, None);
    assert!(
        result
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Invalid thinking level \"random\""))
    );
}

#[test]
fn resolve_model_scope_supports_globs_and_glob_thinking_levels() {
    let models = vec![
        mock_model("anthropic", "claude-sonnet-4-5", "Claude Sonnet 4.5", true),
        mock_model("anthropic", "claude-opus-4-6", "Claude Opus 4.6", true),
        mock_model("openai", "gpt-4o", "GPT-4o", false),
    ];

    let result = resolve_model_scope(
        &[
            String::from("anthropic/*:high"),
            String::from("openai/gpt-4o"),
        ],
        &models,
    );

    assert!(result.warnings.is_empty());
    assert_eq!(result.scoped_models.len(), 3);
    assert_eq!(result.scoped_models[0].model.id, "claude-sonnet-4-5");
    assert_eq!(
        result.scoped_models[0].thinking_level,
        Some(ThinkingLevel::High)
    );
    assert_eq!(result.scoped_models[1].model.id, "claude-opus-4-6");
    assert_eq!(
        result.scoped_models[1].thinking_level,
        Some(ThinkingLevel::High)
    );
    assert_eq!(result.scoped_models[2].model.id, "gpt-4o");
    assert_eq!(result.scoped_models[2].thinking_level, None);
}

#[test]
fn resolve_model_scope_keeps_first_duplicate_match_and_emits_scope_warnings() {
    let result = resolve_model_scope(
        &[
            String::from("claude-sonnet-4-5"),
            String::from("anthropic/*:high"),
            String::from("sonnet:random"),
            String::from("missing"),
        ],
        &base_models(),
    );

    assert_eq!(result.scoped_models.len(), 1);
    assert_eq!(result.scoped_models[0].model.id, "claude-sonnet-4-5");
    assert_eq!(result.scoped_models[0].thinking_level, None);
    assert_eq!(result.warnings.len(), 2);
    assert!(result.warnings[0].contains("Invalid thinking level \"random\""));
    assert_eq!(result.warnings[1], "No models match pattern \"missing\"");
}

#[test]
fn resolve_cli_model_supports_provider_prefixed_ids() {
    let catalog = ModelCatalog::from_all_models(base_models());

    let result = resolve_cli_model(&catalog, None, Some("openai/gpt-4o"));

    assert_eq!(result.error, None);
    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-4o")
    );
}

#[test]
fn resolve_cli_model_supports_fuzzy_matching_with_explicit_provider() {
    let catalog = ModelCatalog::from_all_models(base_models());

    let result = resolve_cli_model(&catalog, Some("openai"), Some("4o"));

    assert_eq!(result.error, None);
    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-4o")
    );
}

#[test]
fn resolve_cli_model_matches_exact_colon_model_ids() {
    let catalog = ModelCatalog::from_all_models(base_models());

    let result = resolve_cli_model(&catalog, None, Some("gpt-5.4:extended"));

    assert_eq!(result.error, None);
    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai-codex")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-5.4:extended")
    );
}

#[test]
fn resolve_cli_model_builds_custom_model_ids_for_explicit_provider() {
    let catalog = ModelCatalog::from_all_models(base_models());

    let result = resolve_cli_model(
        &catalog,
        Some("openai-codex"),
        Some("openai-codex/ghost-model"),
    );

    assert_eq!(result.error, None);
    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai-codex")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("ghost-model")
    );
    assert!(
        result
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Using custom model id"))
    );
}

#[test]
fn resolve_cli_model_errors_when_no_models_exist() {
    let catalog = ModelCatalog::new(Vec::new(), Vec::new());

    let result = resolve_cli_model(&catalog, Some("openai"), Some("gpt-4o"));

    assert_eq!(result.model, None);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|message| message.contains("No models available"))
    );
}

#[test]
fn find_initial_model_uses_scoped_model_before_defaults() {
    let models = base_models();
    let catalog = ModelCatalog::from_all_models(models.clone());

    let result = find_initial_model(
        &catalog,
        InitialModelOptions {
            scoped_models: vec![ScopedModel {
                model: models[0].clone(),
                thinking_level: Some(ThinkingLevel::High),
            }],
            default_thinking_level: Some(ThinkingLevel::Low),
            ..InitialModelOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::High);
}

#[test]
fn find_initial_model_uses_saved_default_even_without_available_auth() {
    let models = base_models();
    let catalog = ModelCatalog::new(models.clone(), vec![models[0].clone()]);

    let result = find_initial_model(
        &catalog,
        InitialModelOptions {
            default_provider: Some("openai".into()),
            default_model_id: Some("gpt-4o".into()),
            default_thinking_level: Some(ThinkingLevel::Low),
            ..InitialModelOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-4o")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::Low);
}

#[test]
fn find_initial_model_prefers_ordered_default_available_model() {
    let codex_model = mock_model("openai-codex", "gpt-5.4", "GPT-5.4", true);
    let catalog = ModelCatalog::new(vec![codex_model.clone()], vec![codex_model.clone()]);

    let result = find_initial_model(&catalog, InitialModelOptions::default());

    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai-codex")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-5.4")
    );
    assert_eq!(result.thinking_level, DEFAULT_THINKING_LEVEL);
}

#[test]
fn restore_model_from_session_keeps_restored_model_when_auth_is_available() {
    let models = base_models();
    let catalog = ModelCatalog::new(models.clone(), vec![models[1].clone()]);

    let result = restore_model_from_session(&catalog, "openai", "gpt-4o", None);

    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("gpt-4o")
    );
    assert_eq!(result.fallback_message, None);
}

#[test]
fn restore_model_from_session_falls_back_to_current_model() {
    let models = base_models();
    let catalog = ModelCatalog::new(models.clone(), vec![models[0].clone()]);

    let result = restore_model_from_session(&catalog, "openai", "gpt-4o", Some(&models[0]));

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert!(result.fallback_message.as_deref().is_some_and(|message| {
        message.contains("Could not restore model openai/gpt-4o")
            && message.contains("Using anthropic/claude-sonnet-4-5")
    }));
}

#[test]
fn exports_current_default_model_ids() {
    assert_eq!(
        default_model_id_for_provider("anthropic"),
        Some("claude-opus-4-6")
    );
    assert_eq!(default_model_id_for_provider("openai"), Some("gpt-5.4"));
    assert_eq!(
        default_model_id_for_provider("openai-codex"),
        Some("gpt-5.4")
    );
    assert_eq!(DEFAULT_MODELS.len(), 3);
}
