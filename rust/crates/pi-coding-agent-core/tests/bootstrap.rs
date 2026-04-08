use pi_agent::ThinkingLevel;
use pi_coding_agent_core::{
    BootstrapDiagnosticLevel, ExistingSessionSelection, MemoryAuthStorage, ModelRegistry,
    ScopedModel, SessionBootstrapOptions, bootstrap_session,
};
use pi_events::Model;
use std::sync::Arc;

fn mock_model(provider: &str, id: &str, name: &str, reasoning: bool) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        api: "openai-completions".into(),
        provider: provider.into(),
        base_url: format!("https://{provider}.example.com/v1"),
        reasoning,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn built_in_models() -> Vec<Model> {
    vec![
        mock_model("anthropic", "claude-sonnet-4-5", "Claude Sonnet 4.5", true),
        mock_model("anthropic", "claude-opus-4-6", "Claude Opus 4.6", true),
        mock_model("openai", "gpt-4o", "GPT-4o", false),
    ]
}

#[test]
fn bootstrap_uses_cli_model_and_model_shorthand_thinking() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            cli_model: Some("sonnet:high".into()),
            ..SessionBootstrapOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::High);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn bootstrap_prefers_explicit_cli_thinking_over_model_shorthand() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            cli_model: Some("sonnet:high".into()),
            cli_thinking_level: Some(ThinkingLevel::Low),
            ..SessionBootstrapOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::Low);
}

#[test]
fn bootstrap_uses_saved_default_when_it_is_in_scope() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([
        ("anthropic", "anthropic-token"),
        ("openai", "openai-token"),
    ]));
    let models = built_in_models();
    let registry = ModelRegistry::in_memory(auth, models.clone());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            default_provider: Some("openai".into()),
            default_model_id: Some("gpt-4o".into()),
            scoped_models: vec![
                ScopedModel {
                    model: models[0].clone(),
                    thinking_level: Some(ThinkingLevel::High),
                },
                ScopedModel {
                    model: models[2].clone(),
                    thinking_level: Some(ThinkingLevel::Low),
                },
            ],
            ..SessionBootstrapOptions::default()
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
    assert_eq!(result.thinking_level, ThinkingLevel::Off);
}

#[test]
fn bootstrap_restores_existing_session_model_when_auth_is_configured() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("openai", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            existing_session: ExistingSessionSelection {
                has_messages: true,
                saved_model_provider: Some("openai".into()),
                saved_model_id: Some("gpt-4o".into()),
                saved_thinking_level: Some(ThinkingLevel::High),
                has_thinking_entry: true,
            },
            ..SessionBootstrapOptions::default()
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
    assert_eq!(result.thinking_level, ThinkingLevel::Off);
    assert_eq!(result.model_fallback_message, None);
}

#[test]
fn bootstrap_falls_back_when_existing_session_model_cannot_be_restored() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            existing_session: ExistingSessionSelection {
                has_messages: true,
                saved_model_provider: Some("openai".into()),
                saved_model_id: Some("gpt-4o".into()),
                ..ExistingSessionSelection::default()
            },
            ..SessionBootstrapOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.provider.as_str()),
        Some("anthropic")
    );
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-opus-4-6")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::Medium);
    assert_eq!(
        result.model_fallback_message.as_deref(),
        Some("Could not restore model openai/gpt-4o. Using anthropic/claude-opus-4-6")
    );
}

#[test]
fn bootstrap_uses_existing_session_thinking_without_explicit_entry_fallback() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            default_thinking_level: Some(ThinkingLevel::Low),
            existing_session: ExistingSessionSelection {
                has_messages: true,
                saved_model_provider: Some("anthropic".into()),
                saved_model_id: Some("claude-opus-4-6".into()),
                has_thinking_entry: false,
                saved_thinking_level: Some(ThinkingLevel::High),
            },
            ..SessionBootstrapOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-opus-4-6")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::Low);
}

#[test]
fn bootstrap_clamps_cli_model_shorthand_xhigh_for_non_xhigh_models() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            cli_model: Some("sonnet:xhigh".into()),
            ..SessionBootstrapOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::High);
}

#[test]
fn bootstrap_clamps_explicit_cli_xhigh_for_default_non_xhigh_models() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(
        auth,
        vec![mock_model(
            "anthropic",
            "claude-sonnet-4-5",
            "Claude Sonnet 4.5",
            true,
        )],
    );

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            cli_thinking_level: Some(ThinkingLevel::XHigh),
            ..SessionBootstrapOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::High);
}

#[test]
fn bootstrap_preserves_cli_xhigh_for_xhigh_capable_models() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            cli_model: Some("claude-opus-4-6:xhigh".into()),
            ..SessionBootstrapOptions::default()
        },
    );

    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-opus-4-6")
    );
    assert_eq!(result.thinking_level, ThinkingLevel::XHigh);
}

#[test]
fn bootstrap_reports_cli_resolution_errors() {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([("anthropic", "token")]));
    let registry = ModelRegistry::in_memory(auth, built_in_models());

    let result = bootstrap_session(
        &registry,
        SessionBootstrapOptions {
            cli_provider: Some("unknown-provider".into()),
            cli_model: Some("whatever".into()),
            ..SessionBootstrapOptions::default()
        },
    );

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.level == BootstrapDiagnosticLevel::Error
            && diagnostic
                .message
                .contains("Unknown provider \"unknown-provider\"")
    }));
    assert_eq!(
        result.model.as_ref().map(|model| model.id.as_str()),
        Some("claude-opus-4-6")
    );
}
