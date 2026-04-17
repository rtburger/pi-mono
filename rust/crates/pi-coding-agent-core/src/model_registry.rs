use crate::{
    auth::AuthSource,
    config_value::{
        resolve_config_value_or_err, resolve_config_value_uncached, resolve_headers_or_err,
    },
    model_resolver::ModelCatalog,
};
use pi_events::{Model, ModelCompat, ModelCost, ModelRouting, OpenAiCompletionsCompatConfig};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestAuth {
    pub api_key: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
}

#[derive(Clone)]
pub struct ModelRegistry {
    auth_source: Arc<dyn AuthSource>,
    built_in_models: Vec<Model>,
    models_json_path: Option<PathBuf>,
    models: Vec<Model>,
    provider_request_configs: BTreeMap<String, ProviderRequestConfig>,
    model_request_headers: BTreeMap<String, BTreeMap<String, String>>,
    load_error: Option<String>,
}

impl ModelRegistry {
    pub fn new(
        auth_source: Arc<dyn AuthSource>,
        built_in_models: Vec<Model>,
        models_json_path: Option<PathBuf>,
    ) -> Self {
        let mut registry = Self {
            auth_source,
            built_in_models,
            models_json_path,
            models: Vec::new(),
            provider_request_configs: BTreeMap::new(),
            model_request_headers: BTreeMap::new(),
            load_error: None,
        };
        registry.refresh();
        registry
    }

    pub fn in_memory(auth_source: Arc<dyn AuthSource>, built_in_models: Vec<Model>) -> Self {
        Self::new(auth_source, built_in_models, None)
    }

    pub fn refresh(&mut self) {
        self.provider_request_configs.clear();
        self.model_request_headers.clear();
        self.load_error = None;

        let custom = match self.models_json_path.clone() {
            Some(path) => self.load_custom_models(&path),
            None => CustomModelsResult::default(),
        };

        if let Some(error) = custom.error.clone() {
            self.load_error = Some(error);
        }

        let built_in_models = self.load_built_in_models(&custom.overrides, &custom.model_overrides);
        let combined = merge_custom_models(built_in_models, custom.models);
        self.models = self.apply_auth_model_mutations(combined);
    }

    pub fn get_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn get_all(&self) -> &[Model] {
        &self.models
    }

    pub fn get_available(&self) -> Vec<Model> {
        self.models
            .iter()
            .filter(|model| self.has_configured_auth(model))
            .cloned()
            .collect()
    }

    pub fn catalog(&self) -> ModelCatalog {
        ModelCatalog::new(self.models.clone(), self.get_available())
    }

    pub fn find(&self, provider: &str, model_id: &str) -> Option<Model> {
        self.models
            .iter()
            .find(|model| model.provider == provider && model.id == model_id)
            .cloned()
    }

    pub fn has_configured_auth(&self, model: &Model) -> bool {
        self.auth_source.has_auth(&model.provider)
            || self
                .provider_request_configs
                .get(&model.provider)
                .and_then(|config| config.api_key.as_ref())
                .is_some()
    }

    pub fn get_api_key_for_provider(&self, provider: &str) -> Option<String> {
        self.auth_source.get_api_key(provider).or_else(|| {
            self.provider_request_configs
                .get(provider)
                .and_then(|config| config.api_key.as_deref())
                .and_then(resolve_config_value_uncached)
        })
    }

    pub fn get_api_key_and_headers(&self, model: &Model) -> Result<RequestAuth, String> {
        let provider_config = self.provider_request_configs.get(&model.provider);
        let api_key = self.auth_source.get_api_key(&model.provider).or_else(|| {
            provider_config
                .and_then(|config| config.api_key.as_deref())
                .map(|value| {
                    resolve_config_value_or_err(
                        value,
                        &format!("API key for provider \"{}\"", model.provider),
                    )
                })
                .transpose()
                .ok()
                .flatten()
        });

        self.finalize_request_auth(model, provider_config, api_key)
    }

    pub async fn get_api_key_and_headers_async(
        &self,
        model: &Model,
    ) -> Result<RequestAuth, String> {
        let provider_config = self.provider_request_configs.get(&model.provider);
        let api_key = match self
            .auth_source
            .get_api_key_for_request(&model.provider)
            .await
        {
            Some(api_key) => Some(api_key),
            None => provider_config
                .and_then(|config| config.api_key.as_deref())
                .map(|value| {
                    resolve_config_value_or_err(
                        value,
                        &format!("API key for provider \"{}\"", model.provider),
                    )
                })
                .transpose()?,
        };

        self.finalize_request_auth(model, provider_config, api_key)
    }

    fn finalize_request_auth(
        &self,
        model: &Model,
        provider_config: Option<&ProviderRequestConfig>,
        api_key: Option<String>,
    ) -> Result<RequestAuth, String> {
        if provider_config
            .and_then(|config| config.api_key.as_deref())
            .is_some()
            && api_key.is_none()
            && !self.auth_source.has_auth(&model.provider)
        {
            let provider_api_key = provider_config
                .and_then(|config| config.api_key.as_deref())
                .unwrap_or_default();
            resolve_config_value_or_err(
                provider_api_key,
                &format!("API key for provider \"{}\"", model.provider),
            )?;
        }

        let provider_headers = resolve_headers_or_err(
            provider_config.and_then(|config| config.headers.as_ref()),
            &format!("provider \"{}\"", model.provider),
        )?;
        let model_headers = resolve_headers_or_err(
            self.model_request_headers
                .get(&model_request_key(&model.provider, &model.id)),
            &format!("model \"{}/{}\"", model.provider, model.id),
        )?;

        let mut headers = BTreeMap::new();
        if let Some(provider_headers) = provider_headers {
            headers.extend(provider_headers);
        }
        if let Some(model_headers) = model_headers {
            headers.extend(model_headers);
        }

        if provider_config.is_some_and(|config| config.auth_header) {
            let Some(api_key) = api_key.clone() else {
                return Err(format!("No API key found for \"{}\"", model.provider));
            };
            headers.insert("Authorization".into(), format!("Bearer {api_key}"));
        }

        Ok(RequestAuth {
            api_key,
            headers: (!headers.is_empty()).then_some(headers),
        })
    }

    fn load_built_in_models(
        &self,
        overrides: &BTreeMap<String, ProviderOverride>,
        model_overrides: &BTreeMap<String, BTreeMap<String, ModelOverrideFile>>,
    ) -> Vec<Model> {
        self.built_in_models
            .iter()
            .map(|model| {
                let mut next = model.clone();

                if let Some(provider_override) = overrides.get(&model.provider) {
                    if let Some(base_url) = provider_override.base_url.as_ref() {
                        next.base_url = base_url.clone();
                    }
                    next.compat =
                        merge_compat(next.compat.as_ref(), provider_override.compat.as_ref());
                }

                if let Some(provider_model_overrides) = model_overrides.get(&model.provider)
                    && let Some(model_override) = provider_model_overrides.get(&model.id)
                {
                    next = apply_model_override(&next, model_override);
                }

                next
            })
            .collect()
    }

    fn apply_auth_model_mutations(&self, models: Vec<Model>) -> Vec<Model> {
        let mut provider_base_urls = BTreeMap::<String, Option<String>>::new();

        models
            .into_iter()
            .map(|mut model| {
                let base_url = provider_base_urls
                    .entry(model.provider.clone())
                    .or_insert_with(|| self.auth_source.model_base_url(&model.provider));
                if let Some(base_url) = base_url.as_ref() {
                    model.base_url = base_url.clone();
                }
                model
            })
            .collect()
    }

    fn load_custom_models(&mut self, path: &Path) -> CustomModelsResult {
        if !path.exists() {
            return CustomModelsResult::default();
        }

        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) => {
                return CustomModelsResult::with_error(format!(
                    "Failed to load models.json: {error}\n\nFile: {}",
                    path.display()
                ));
            }
        };

        let config: ModelsConfigFile = match serde_json::from_str(&content) {
            Ok(config) => config,
            Err(error) => {
                return CustomModelsResult::with_error(format!(
                    "Failed to parse models.json: {error}\n\nFile: {}",
                    path.display()
                ));
            }
        };

        if let Err(error) = validate_config(&config) {
            return CustomModelsResult::with_error(format!("{error}\n\nFile: {}", path.display()));
        }

        let mut overrides = BTreeMap::new();
        let mut model_overrides = BTreeMap::new();

        for (provider_name, provider_config) in &config.providers {
            if provider_config.base_url.is_some() || provider_config.compat.is_some() {
                overrides.insert(
                    provider_name.clone(),
                    ProviderOverride {
                        base_url: provider_config.base_url.clone(),
                        compat: provider_config.compat.clone(),
                    },
                );
            }

            self.store_provider_request_config(provider_name, provider_config);

            if let Some(overrides_for_provider) = provider_config.model_overrides.as_ref() {
                model_overrides.insert(provider_name.clone(), overrides_for_provider.clone());
                for (model_id, model_override) in overrides_for_provider {
                    self.store_model_headers(
                        provider_name,
                        model_id,
                        model_override.headers.as_ref(),
                    );
                }
            }
        }

        CustomModelsResult {
            models: parse_custom_models(&config, self),
            overrides,
            model_overrides,
            error: None,
        }
    }

    fn store_provider_request_config(&mut self, provider_name: &str, config: &ProviderConfigFile) {
        if config.api_key.is_none()
            && config.headers.is_none()
            && !config.auth_header.unwrap_or(false)
        {
            return;
        }

        self.provider_request_configs.insert(
            provider_name.to_string(),
            ProviderRequestConfig {
                api_key: config.api_key.clone(),
                headers: config.headers.clone(),
                auth_header: config.auth_header.unwrap_or(false),
            },
        );
    }

    fn store_model_headers(
        &mut self,
        provider_name: &str,
        model_id: &str,
        headers: Option<&BTreeMap<String, String>>,
    ) {
        let key = model_request_key(provider_name, model_id);
        if let Some(headers) = headers
            && !headers.is_empty()
        {
            self.model_request_headers.insert(key, headers.clone());
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CustomModelsResult {
    models: Vec<Model>,
    overrides: BTreeMap<String, ProviderOverride>,
    model_overrides: BTreeMap<String, BTreeMap<String, ModelOverrideFile>>,
    error: Option<String>,
}

impl CustomModelsResult {
    fn with_error(error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
struct ProviderOverride {
    base_url: Option<String>,
    compat: Option<ModelCompat>,
}

#[derive(Debug, Clone)]
struct ProviderRequestConfig {
    api_key: Option<String>,
    headers: Option<BTreeMap<String, String>>,
    auth_header: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ModelsConfigFile {
    #[serde(default)]
    providers: BTreeMap<String, ProviderConfigFile>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProviderConfigFile {
    base_url: Option<String>,
    api_key: Option<String>,
    api: Option<String>,
    headers: Option<BTreeMap<String, String>>,
    compat: Option<ModelCompat>,
    auth_header: Option<bool>,
    models: Option<Vec<ModelDefinitionFile>>,
    model_overrides: Option<BTreeMap<String, ModelOverrideFile>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ModelDefinitionFile {
    id: String,
    name: Option<String>,
    api: Option<String>,
    base_url: Option<String>,
    reasoning: Option<bool>,
    input: Option<Vec<ModelInputKind>>,
    cost: Option<ModelCost>,
    context_window: Option<u64>,
    max_tokens: Option<u64>,
    headers: Option<BTreeMap<String, String>>,
    compat: Option<ModelCompat>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ModelOverrideFile {
    name: Option<String>,
    reasoning: Option<bool>,
    input: Option<Vec<ModelInputKind>>,
    cost: Option<ModelCostOverrideFile>,
    context_window: Option<u64>,
    max_tokens: Option<u64>,
    headers: Option<BTreeMap<String, String>>,
    compat: Option<ModelCompat>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ModelCostOverrideFile {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ModelInputKind {
    Text,
    Image,
}

impl ModelInputKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }
}

fn validate_config(config: &ModelsConfigFile) -> Result<(), String> {
    for (provider_name, provider_config) in &config.providers {
        let models = provider_config.models.as_deref().unwrap_or_default();
        let has_model_overrides = provider_config
            .model_overrides
            .as_ref()
            .is_some_and(|overrides| !overrides.is_empty());

        if models.is_empty() {
            if provider_config.base_url.is_none()
                && provider_config.compat.is_none()
                && !has_model_overrides
            {
                return Err(format!(
                    "Provider {provider_name}: must specify \"baseUrl\", \"compat\", \"modelOverrides\", or \"models\"."
                ));
            }
        } else {
            if provider_config.base_url.is_none() {
                return Err(format!(
                    "Provider {provider_name}: \"baseUrl\" is required when defining custom models."
                ));
            }
            if provider_config.api_key.is_none() {
                return Err(format!(
                    "Provider {provider_name}: \"apiKey\" is required when defining custom models."
                ));
            }
        }

        for model_definition in models {
            if model_definition.id.trim().is_empty() {
                return Err(format!("Provider {provider_name}: model missing \"id\""));
            }
            if provider_config.api.is_none() && model_definition.api.is_none() {
                return Err(format!(
                    "Provider {provider_name}, model {}: no \"api\" specified. Set at provider or model level.",
                    model_definition.id
                ));
            }
            if model_definition.context_window == Some(0) {
                return Err(format!(
                    "Provider {provider_name}, model {}: invalid contextWindow",
                    model_definition.id
                ));
            }
            if model_definition.max_tokens == Some(0) {
                return Err(format!(
                    "Provider {provider_name}, model {}: invalid maxTokens",
                    model_definition.id
                ));
            }
        }
    }

    Ok(())
}

fn parse_custom_models(config: &ModelsConfigFile, registry: &mut ModelRegistry) -> Vec<Model> {
    let mut models = Vec::new();

    for (provider_name, provider_config) in &config.providers {
        let Some(model_definitions) = provider_config.models.as_ref() else {
            continue;
        };

        for model_definition in model_definitions {
            let api = model_definition
                .api
                .as_deref()
                .or(provider_config.api.as_deref())
                .unwrap_or_default();
            if api.is_empty() {
                continue;
            }

            registry.store_model_headers(
                provider_name,
                &model_definition.id,
                model_definition.headers.as_ref(),
            );

            models.push(Model {
                id: model_definition.id.clone(),
                name: model_definition
                    .name
                    .clone()
                    .unwrap_or_else(|| model_definition.id.clone()),
                api: api.to_string(),
                provider: provider_name.clone(),
                base_url: model_definition
                    .base_url
                    .clone()
                    .or_else(|| provider_config.base_url.clone())
                    .unwrap_or_default(),
                reasoning: model_definition.reasoning.unwrap_or(false),
                input: model_definition
                    .input
                    .as_ref()
                    .map(|input| model_inputs_to_strings(input))
                    .unwrap_or_else(|| vec!["text".into()]),
                cost: model_definition.cost.unwrap_or_default(),
                context_window: model_definition.context_window.unwrap_or(128_000),
                max_tokens: model_definition.max_tokens.unwrap_or(16_384),
                compat: merge_compat(
                    provider_config.compat.as_ref(),
                    model_definition.compat.as_ref(),
                ),
            });
        }
    }

    models
}

fn model_inputs_to_strings(input: &[ModelInputKind]) -> Vec<String> {
    input.iter().map(|kind| kind.as_str().to_string()).collect()
}

fn apply_model_override(model: &Model, model_override: &ModelOverrideFile) -> Model {
    let mut result = model.clone();

    if let Some(name) = model_override.name.as_ref() {
        result.name = name.clone();
    }
    if let Some(reasoning) = model_override.reasoning {
        result.reasoning = reasoning;
    }
    if let Some(input) = model_override.input.as_ref() {
        result.input = model_inputs_to_strings(input);
    }
    if let Some(cost) = model_override.cost.as_ref() {
        result.cost = ModelCost {
            input: cost.input.unwrap_or(result.cost.input),
            output: cost.output.unwrap_or(result.cost.output),
            cache_read: cost.cache_read.unwrap_or(result.cost.cache_read),
            cache_write: cost.cache_write.unwrap_or(result.cost.cache_write),
        };
    }
    if let Some(context_window) = model_override.context_window {
        result.context_window = context_window;
    }
    if let Some(max_tokens) = model_override.max_tokens {
        result.max_tokens = max_tokens;
    }
    result.compat = merge_compat(result.compat.as_ref(), model_override.compat.as_ref());

    result
}

fn merge_compat(
    base: Option<&ModelCompat>,
    override_compat: Option<&ModelCompat>,
) -> Option<ModelCompat> {
    match (base, override_compat) {
        (None, None) => None,
        (Some(base), None) => Some(base.clone()),
        (None, Some(override_compat)) => Some(override_compat.clone()),
        (
            Some(ModelCompat::OpenAiCompletions(base)),
            Some(ModelCompat::OpenAiCompletions(override_compat)),
        ) => Some(ModelCompat::OpenAiCompletions(
            merge_openai_completions_compat(base, override_compat),
        )),
        (
            Some(ModelCompat::OpenAiResponses(_)),
            Some(ModelCompat::OpenAiResponses(override_compat)),
        ) => Some(ModelCompat::OpenAiResponses(override_compat.clone())),
        (Some(ModelCompat::OpenAiCompletions(base)), Some(ModelCompat::OpenAiResponses(_))) => {
            Some(ModelCompat::OpenAiCompletions(base.clone()))
        }
        (
            Some(ModelCompat::OpenAiResponses(_)),
            Some(ModelCompat::OpenAiCompletions(override_compat)),
        ) => Some(ModelCompat::OpenAiCompletions(override_compat.clone())),
    }
}

fn merge_openai_completions_compat(
    base: &OpenAiCompletionsCompatConfig,
    override_compat: &OpenAiCompletionsCompatConfig,
) -> OpenAiCompletionsCompatConfig {
    OpenAiCompletionsCompatConfig {
        supports_store: override_compat.supports_store.or(base.supports_store),
        supports_developer_role: override_compat
            .supports_developer_role
            .or(base.supports_developer_role),
        supports_reasoning_effort: override_compat
            .supports_reasoning_effort
            .or(base.supports_reasoning_effort),
        reasoning_effort_map: override_compat
            .reasoning_effort_map
            .clone()
            .or(base.reasoning_effort_map.clone()),
        supports_usage_in_streaming: override_compat
            .supports_usage_in_streaming
            .or(base.supports_usage_in_streaming),
        max_tokens_field: override_compat.max_tokens_field.or(base.max_tokens_field),
        requires_tool_result_name: override_compat
            .requires_tool_result_name
            .or(base.requires_tool_result_name),
        requires_assistant_after_tool_result: override_compat
            .requires_assistant_after_tool_result
            .or(base.requires_assistant_after_tool_result),
        requires_thinking_as_text: override_compat
            .requires_thinking_as_text
            .or(base.requires_thinking_as_text),
        thinking_format: override_compat.thinking_format.or(base.thinking_format),
        open_router_routing: merge_routing(
            base.open_router_routing.as_ref(),
            override_compat.open_router_routing.as_ref(),
        ),
        vercel_gateway_routing: merge_routing(
            base.vercel_gateway_routing.as_ref(),
            override_compat.vercel_gateway_routing.as_ref(),
        ),
        zai_tool_stream: override_compat.zai_tool_stream.or(base.zai_tool_stream),
        supports_strict_mode: override_compat
            .supports_strict_mode
            .or(base.supports_strict_mode),
    }
}

fn merge_routing(
    base: Option<&ModelRouting>,
    override_routing: Option<&ModelRouting>,
) -> Option<ModelRouting> {
    match (base, override_routing) {
        (None, None) => None,
        (Some(base), None) => Some(base.clone()),
        (None, Some(override_routing)) => Some(override_routing.clone()),
        (Some(base), Some(override_routing)) => Some(ModelRouting {
            only: override_routing.only.clone().or(base.only.clone()),
            order: override_routing.order.clone().or(base.order.clone()),
        }),
    }
}

fn merge_custom_models(built_in_models: Vec<Model>, custom_models: Vec<Model>) -> Vec<Model> {
    let mut merged = built_in_models;

    for custom_model in custom_models {
        if let Some(index) = merged.iter().position(|model| {
            model.provider == custom_model.provider && model.id == custom_model.id
        }) {
            merged[index] = custom_model;
        } else {
            merged.push(custom_model);
        }
    }

    merged
}

fn model_request_key(provider: &str, model_id: &str) -> String {
    format!("{provider}:{model_id}")
}
