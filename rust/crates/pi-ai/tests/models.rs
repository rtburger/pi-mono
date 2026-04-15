use pi_ai::{
    BUILT_IN_MODEL_PROVIDERS, built_in_models, calculate_cost, get_env_api_key, get_model,
    get_models, get_providers, models_are_equal, supports_xhigh,
};
use pi_events::{Model, ModelCost, Usage, UsageCost};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
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

#[derive(Debug, Deserialize)]
struct CatalogModelEntry {
    id: String,
}

type RawCatalog = BTreeMap<String, BTreeMap<String, CatalogModelEntry>>;

#[test]
fn loads_known_models_from_rust_owned_catalog() {
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
        BUILT_IN_MODEL_PROVIDERS
            .iter()
            .map(|provider| (*provider).to_string())
            .collect::<Vec<_>>()
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
fn built_in_models_match_entries_in_local_catalog_file() {
    let raw_catalog: RawCatalog = serde_json::from_str(include_str!("../src/models.catalog.json"))
        .expect("expected local rust model catalog to parse");
    let providers = get_providers();
    let expected_model_count = providers
        .iter()
        .map(|provider| raw_catalog.get(provider).map_or(0, BTreeMap::len))
        .sum::<usize>();

    let all_models = built_in_models();
    assert_eq!(all_models.len(), expected_model_count);

    for model in all_models {
        let provider_models = raw_catalog
            .get(&model.provider)
            .unwrap_or_else(|| panic!("missing provider {} in local rust catalog", model.provider));
        let raw_entry = provider_models.get(&model.id).unwrap_or_else(|| {
            panic!(
                "missing model {}/{} in local rust catalog",
                model.provider, model.id
            )
        });
        assert_eq!(raw_entry.id, model.id);
    }
}

#[test]
fn supports_xhigh_matches_model_id_rules() {
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
fn calculate_cost_populates_openai_and_anthropic_usage_costs() {
    let mut openai_usage = Usage {
        input: 1_000_000,
        output: 1_000_000,
        cache_read: 1_000_000,
        cache_write: 1_000_000,
        total_tokens: 4_000_000,
        cost: UsageCost::default(),
    };
    let openai_model = get_model("openai", "gpt-5.4").expect("expected openai/gpt-5.4 model");
    let openai_cost = calculate_cost(&openai_model, &mut openai_usage);
    assert_eq!(
        openai_cost,
        UsageCost {
            input: 2.5,
            output: 15.0,
            cache_read: 0.25,
            cache_write: 0.0,
            total: 17.75,
        }
    );
    assert_eq!(openai_usage.cost, openai_cost);

    let mut anthropic_usage = Usage {
        input: 1_000_000,
        output: 1_000_000,
        cache_read: 1_000_000,
        cache_write: 1_000_000,
        total_tokens: 4_000_000,
        cost: UsageCost::default(),
    };
    let anthropic_model = get_model("anthropic", "claude-opus-4-6")
        .expect("expected anthropic/claude-opus-4-6 model");
    let anthropic_cost = calculate_cost(&anthropic_model, &mut anthropic_usage);
    assert_eq!(
        anthropic_cost,
        UsageCost {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
            total: 36.75,
        }
    );
    assert_eq!(anthropic_usage.cost, anthropic_cost);
}

#[test]
fn calculate_cost_uses_model_embedded_costs_for_custom_models() {
    let custom_model = Model {
        id: "custom-openai-model".into(),
        name: "Custom OpenAI Model".into(),
        api: "openai-completions".into(),
        provider: "custom-provider".into(),
        base_url: "https://custom.example.com/v1".into(),
        reasoning: false,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 0.25,
            output: 0.75,
            cache_read: 0.05,
            cache_write: 0.0,
        },
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    };
    let mut usage = Usage {
        input: 1_000_000,
        output: 2_000_000,
        cache_read: 3_000_000,
        cache_write: 4_000_000,
        total_tokens: 10_000_000,
        cost: UsageCost::default(),
    };

    let cost = calculate_cost(&custom_model, &mut usage);

    assert!((cost.input - 0.25).abs() < 1e-12);
    assert!((cost.output - 1.5).abs() < 1e-12);
    assert!((cost.cache_read - 0.15).abs() < 1e-12);
    assert!((cost.cache_write - 0.0).abs() < 1e-12);
    assert!((cost.total - 1.9).abs() < 1e-12);
    assert_eq!(usage.cost, cost);
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
