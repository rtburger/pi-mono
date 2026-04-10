use pi_ai::{
    built_in_models, get_env_api_key, get_model, get_models, get_providers, models_are_equal,
    supports_xhigh,
};
use std::{
    ffi::OsString,
    sync::{Mutex, OnceLock},
};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn with_env_vars<F>(updates: &[(&str, Option<&str>)], test: F)
where
    F: FnOnce(),
{
    let _guard = env_lock().lock().unwrap();
    let snapshot = updates
        .iter()
        .map(|(key, _)| ((*key).to_string(), std::env::var_os(key)))
        .collect::<Vec<(String, Option<OsString>)>>();

    for (key, value) in updates {
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    test();

    for (key, value) in snapshot {
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }
}

#[test]
fn loads_known_models_from_typescript_generated_catalog() {
    let all_models = built_in_models();
    assert!(!all_models.is_empty());

    let model = get_model("openai", "gpt-5.4").expect("expected openai/gpt-5.4 model");
    assert_eq!(model.id, "gpt-5.4");
    assert_eq!(model.provider, "openai");
    assert_eq!(model.api, "openai-responses");
    assert!(model.reasoning);
    assert!(model.input.iter().any(|input| input == "text"));
}

#[test]
fn exposes_only_migrated_providers() {
    let providers = get_providers();
    assert_eq!(
        providers,
        vec![
            String::from("anthropic"),
            String::from("openai"),
            String::from("openai-codex"),
        ]
    );

    let anthropic_models = get_models("anthropic");
    assert!(
        anthropic_models
            .iter()
            .any(|model| model.id == "claude-opus-4-6")
    );
    assert!(get_models("missing-provider").is_empty());
    assert!(get_model("missing-provider", "missing-model").is_none());
}

#[test]
fn supports_xhigh_matches_typescript_rules() {
    let anthropic_opus = get_model("anthropic", "claude-opus-4-6").unwrap();
    let anthropic_sonnet = get_model("anthropic", "claude-sonnet-4-5").unwrap();
    let openai_gpt = get_model("openai-codex", "gpt-5.4").unwrap();

    assert!(supports_xhigh(&anthropic_opus));
    assert!(!supports_xhigh(&anthropic_sonnet));
    assert!(supports_xhigh(&openai_gpt));
}

#[test]
fn models_are_equal_matches_provider_and_id_only() {
    let left = get_model("openai", "gpt-5.4").unwrap();
    let right = get_model("openai", "gpt-5.4").unwrap();
    let other_provider = get_model("openai-codex", "gpt-5.4").unwrap();

    assert!(models_are_equal(Some(&left), Some(&right)));
    assert!(!models_are_equal(Some(&left), Some(&other_provider)));
    assert!(!models_are_equal(Some(&left), None));
}

#[test]
fn env_api_key_prefers_anthropic_oauth_token() {
    with_env_vars(
        &[
            ("ANTHROPIC_API_KEY", Some("api-key")),
            ("ANTHROPIC_OAUTH_TOKEN", Some("oauth-token")),
        ],
        || {
            assert_eq!(get_env_api_key("anthropic").as_deref(), Some("oauth-token"));
        },
    );
}

#[test]
fn env_api_key_reads_openai_api_key() {
    with_env_vars(&[("OPENAI_API_KEY", Some("openai-token"))], || {
        assert_eq!(get_env_api_key("openai").as_deref(), Some("openai-token"));
    });
}

#[test]
fn env_api_key_returns_none_for_unsupported_provider() {
    with_env_vars(&[("UNUSED_API_KEY", Some("unused"))], || {
        assert_eq!(get_env_api_key("unsupported-provider"), None);
    });
}
