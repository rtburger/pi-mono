use pi_events::Model;
use regex::Regex;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

const MODELS_GENERATED_TS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../packages/ai/src/models.generated.ts"
));

#[derive(Debug)]
struct BuiltInModelCatalog {
    providers: Vec<String>,
    provider_models: BTreeMap<String, Vec<Model>>,
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
    reasoning: bool,
    input: Vec<String>,
    context_window: u64,
    max_tokens: u64,
}

type RawCatalog = BTreeMap<String, BTreeMap<String, RawModel>>;

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
    let sanitized = sanitize_generated_models_source(MODELS_GENERATED_TS);
    let raw_catalog: RawCatalog =
        json5::from_str(&sanitized).expect("failed to parse packages/ai/src/models.generated.ts");

    let mut providers = Vec::with_capacity(raw_catalog.len());
    let mut provider_models = BTreeMap::new();
    let mut all_models = Vec::new();

    for (provider, models) in raw_catalog {
        providers.push(provider.clone());
        let mut provider_entries = Vec::with_capacity(models.len());

        for raw_model in models.into_values() {
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

        provider_models.insert(provider, provider_entries);
    }

    BuiltInModelCatalog {
        providers,
        provider_models,
        all_models,
    }
}

fn sanitize_generated_models_source(source: &str) -> String {
    let mut sanitized = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import type "))
        .collect::<Vec<_>>()
        .join("\n");

    sanitized = sanitized.replacen("export const MODELS =", "", 1);
    sanitized = satisfies_model_regex()
        .replace_all(&sanitized, "")
        .into_owned();

    let trimmed = sanitized.trim();
    let trimmed = trimmed.strip_suffix("as const;").unwrap_or(trimmed).trim();

    trimmed.to_string()
}

fn satisfies_model_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"\s+satisfies\s+Model<"[^"]+">"#).unwrap())
}
