use pi_events::{Model, Usage, UsageCost};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

const MODELS_CATALOG_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/models.catalog.json"
));

#[derive(Debug)]
struct BuiltInModelCatalog {
    providers: Vec<String>,
    provider_models: BTreeMap<String, Vec<Model>>,
    model_headers: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>,
    model_costs: BTreeMap<String, BTreeMap<String, RawModelCost>>,
    all_models: Vec<Model>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawModel {
    id: String,
    name: String,
    api: String,
    provider: String,
    #[serde(rename = "baseUrl")]
    base_url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    reasoning: bool,
    input: Vec<String>,
    cost: RawModelCost,
    context_window: u64,
    max_tokens: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawModelCost {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

type RawCatalog = BTreeMap<String, BTreeMap<String, RawModel>>;

const SUPPORTED_PROVIDERS: &[&str] = &["anthropic", "openai", "openai-codex"];

pub fn built_in_models() -> &'static [Model] {
    catalog().all_models.as_slice()
}

pub fn get_providers() -> Vec<String> {
    catalog().providers.clone()
}

pub fn get_models(provider: &str) -> Vec<Model> {
    catalog()
        .provider_models
        .get(provider)
        .cloned()
        .unwrap_or_default()
}

pub fn get_model(provider: &str, model_id: &str) -> Option<Model> {
    catalog()
        .provider_models
        .get(provider)
        .and_then(|models| models.iter().find(|model| model.id == model_id))
        .cloned()
}

pub fn get_model_headers(provider: &str, model_id: &str) -> Option<BTreeMap<String, String>> {
    catalog()
        .model_headers
        .get(provider)
        .and_then(|models| models.get(model_id))
        .cloned()
}

pub fn get_provider_headers(provider: &str) -> Option<BTreeMap<String, String>> {
    catalog()
        .model_headers
        .get(provider)
        .and_then(|models| models.values().next())
        .cloned()
}

pub fn calculate_cost(model: &Model, usage: &mut Usage) -> UsageCost {
    calculate_cost_for(model.provider.as_str(), model.id.as_str(), usage)
}

pub(crate) fn calculate_cost_for(
    provider: &str,
    model_id: &str,
    usage: &mut Usage,
) -> UsageCost {
    let Some(cost) = model_cost(provider, model_id) else {
        usage.cost = UsageCost::default();
        return usage.cost.clone();
    };

    usage.cost.input = (cost.input / 1_000_000.0) * usage.input as f64;
    usage.cost.output = (cost.output / 1_000_000.0) * usage.output as f64;
    usage.cost.cache_read = (cost.cache_read / 1_000_000.0) * usage.cache_read as f64;
    usage.cost.cache_write = (cost.cache_write / 1_000_000.0) * usage.cache_write as f64;
    usage.cost.total =
        usage.cost.input + usage.cost.output + usage.cost.cache_read + usage.cost.cache_write;
    usage.cost.clone()
}

pub fn supports_xhigh(model: &Model) -> bool {
    model.id.contains("gpt-5.2")
        || model.id.contains("gpt-5.3")
        || model.id.contains("gpt-5.4")
        || model.id.contains("opus-4-6")
        || model.id.contains("opus-4.6")
}

pub fn models_are_equal(left: Option<&Model>, right: Option<&Model>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.id == right.id && left.provider == right.provider,
        _ => false,
    }
}

fn catalog() -> &'static BuiltInModelCatalog {
    static CATALOG: OnceLock<BuiltInModelCatalog> = OnceLock::new();
    CATALOG.get_or_init(load_catalog)
}

fn load_catalog() -> BuiltInModelCatalog {
    let raw_catalog: RawCatalog = serde_json::from_str(MODELS_CATALOG_JSON)
        .expect("failed to parse rust/crates/pi-ai/src/models.catalog.json");

    let mut providers = Vec::with_capacity(raw_catalog.len());
    let mut provider_models = BTreeMap::new();
    let mut model_headers = BTreeMap::new();
    let mut model_costs = BTreeMap::new();
    let mut all_models = Vec::new();

    for (provider, models) in raw_catalog {
        if !SUPPORTED_PROVIDERS.contains(&provider.as_str()) {
            continue;
        }

        providers.push(provider.clone());
        let mut provider_entries = Vec::with_capacity(models.len());
        let mut provider_header_entries = BTreeMap::new();
        let mut provider_cost_entries = BTreeMap::new();

        for raw_model in models.into_values() {
            let model_id = raw_model.id.clone();
            if !raw_model.headers.is_empty() {
                provider_header_entries.insert(model_id.clone(), raw_model.headers.clone());
            }
            provider_cost_entries.insert(model_id.clone(), raw_model.cost);

            let model = Model {
                id: raw_model.id,
                name: raw_model.name,
                api: raw_model.api,
                provider: raw_model.provider,
                base_url: raw_model.base_url,
                reasoning: raw_model.reasoning,
                input: raw_model.input,
                context_window: raw_model.context_window,
                max_tokens: raw_model.max_tokens,
            };
            provider_entries.push(model.clone());
            all_models.push(model);
        }

        if !provider_header_entries.is_empty() {
            model_headers.insert(provider.clone(), provider_header_entries);
        }
        if !provider_cost_entries.is_empty() {
            model_costs.insert(provider.clone(), provider_cost_entries);
        }
        provider_models.insert(provider, provider_entries);
    }

    BuiltInModelCatalog {
        providers,
        provider_models,
        model_headers,
        model_costs,
        all_models,
    }
}

fn model_cost(provider: &str, model_id: &str) -> Option<RawModelCost> {
    catalog()
        .model_costs
        .get(provider)
        .and_then(|models| models.get(model_id))
        .copied()
}
