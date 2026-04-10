use globset::GlobBuilder;
use pi_agent::ThinkingLevel;
use pi_events::Model;
use std::collections::HashMap;

pub const DEFAULT_THINKING_LEVEL: ThinkingLevel = ThinkingLevel::Medium;

pub const DEFAULT_MODELS: [(&str, &str); 3] = [
    ("anthropic", "claude-opus-4-6"),
    ("openai", "gpt-5.4"),
    ("openai-codex", "gpt-5.4"),
];

#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    all_models: Vec<Model>,
    available_models: Vec<Model>,
}

impl ModelCatalog {
    pub fn new(all_models: Vec<Model>, available_models: Vec<Model>) -> Self {
        Self {
            all_models,
            available_models,
        }
    }

    pub fn from_all_models(all_models: Vec<Model>) -> Self {
        Self {
            available_models: all_models.clone(),
            all_models,
        }
    }

    pub fn all_models(&self) -> &[Model] {
        &self.all_models
    }

    pub fn available_models(&self) -> &[Model] {
        &self.available_models
    }

    pub fn find(&self, provider: &str, model_id: &str) -> Option<Model> {
        self.all_models
            .iter()
            .find(|model| model.provider == provider && model.id == model_id)
            .cloned()
    }

    pub fn has_configured_auth(&self, model: &Model) -> bool {
        self.available_models
            .iter()
            .any(|candidate| candidate.provider == model.provider && candidate.id == model.id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopedModel {
    pub model: Model,
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, Copy)]
pub struct ParseModelPatternOptions {
    pub allow_invalid_thinking_level_fallback: bool,
}

impl Default for ParseModelPatternOptions {
    fn default() -> Self {
        Self {
            allow_invalid_thinking_level_fallback: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedModelResult {
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResolveCliModelResult {
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResolveModelScopeResult {
    pub scoped_models: Vec<ScopedModel>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InitialModelOptions {
    pub scoped_models: Vec<ScopedModel>,
    pub is_continuing: bool,
    pub default_provider: Option<String>,
    pub default_model_id: Option<String>,
    pub default_thinking_level: Option<ThinkingLevel>,
}

impl Default for InitialModelOptions {
    fn default() -> Self {
        Self {
            scoped_models: Vec::new(),
            is_continuing: false,
            default_provider: None,
            default_model_id: None,
            default_thinking_level: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InitialModelResult {
    pub model: Option<Model>,
    pub thinking_level: ThinkingLevel,
    pub fallback_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestoreModelResult {
    pub model: Option<Model>,
    pub fallback_message: Option<String>,
}

pub fn default_model_id_for_provider(provider: &str) -> Option<&'static str> {
    DEFAULT_MODELS
        .iter()
        .find_map(|(name, model_id)| (*name == provider).then_some(*model_id))
}

pub fn parse_thinking_level(level: &str) -> Option<ThinkingLevel> {
    match level.to_ascii_lowercase().as_str() {
        "off" => Some(ThinkingLevel::Off),
        "minimal" => Some(ThinkingLevel::Minimal),
        "low" => Some(ThinkingLevel::Low),
        "medium" => Some(ThinkingLevel::Medium),
        "high" => Some(ThinkingLevel::High),
        "xhigh" => Some(ThinkingLevel::XHigh),
        _ => None,
    }
}

pub fn find_exact_model_reference_match(
    model_reference: &str,
    available_models: &[Model],
) -> Option<Model> {
    let trimmed_reference = model_reference.trim();
    if trimmed_reference.is_empty() {
        return None;
    }

    let canonical_matches: Vec<Model> = available_models
        .iter()
        .filter(|model| canonical_model_reference(model).eq_ignore_ascii_case(trimmed_reference))
        .cloned()
        .collect();
    if canonical_matches.len() == 1 {
        return canonical_matches.into_iter().next();
    }
    if canonical_matches.len() > 1 {
        return None;
    }

    if let Some((provider, model_id)) = trimmed_reference.split_once('/') {
        let provider = provider.trim();
        let model_id = model_id.trim();
        if !provider.is_empty() && !model_id.is_empty() {
            let provider_matches: Vec<Model> = available_models
                .iter()
                .filter(|model| {
                    model.provider.eq_ignore_ascii_case(provider)
                        && model.id.eq_ignore_ascii_case(model_id)
                })
                .cloned()
                .collect();
            if provider_matches.len() == 1 {
                return provider_matches.into_iter().next();
            }
            if provider_matches.len() > 1 {
                return None;
            }
        }
    }

    let id_matches: Vec<Model> = available_models
        .iter()
        .filter(|model| model.id.eq_ignore_ascii_case(trimmed_reference))
        .cloned()
        .collect();
    (id_matches.len() == 1)
        .then(|| id_matches.into_iter().next())
        .flatten()
}

pub fn parse_model_pattern(
    pattern: &str,
    available_models: &[Model],
    options: ParseModelPatternOptions,
) -> ParsedModelResult {
    if let Some(model) = try_match_model(pattern, available_models) {
        return ParsedModelResult {
            model: Some(model),
            thinking_level: None,
            warning: None,
        };
    }

    let Some(last_colon_index) = pattern.rfind(':') else {
        return ParsedModelResult::default();
    };

    let prefix = &pattern[..last_colon_index];
    let suffix = &pattern[last_colon_index + 1..];

    if let Some(thinking_level) = parse_thinking_level(suffix) {
        let result = parse_model_pattern(prefix, available_models, options);
        if result.model.is_some() {
            return ParsedModelResult {
                model: result.model,
                thinking_level: result.warning.is_none().then_some(thinking_level),
                warning: result.warning,
            };
        }
        return result;
    }

    if !options.allow_invalid_thinking_level_fallback {
        return ParsedModelResult::default();
    }

    let result = parse_model_pattern(prefix, available_models, options);
    if result.model.is_some() {
        return ParsedModelResult {
            model: result.model,
            thinking_level: None,
            warning: Some(format!(
                "Invalid thinking level \"{suffix}\" in pattern \"{pattern}\". Using default instead."
            )),
        };
    }

    result
}

pub fn resolve_cli_model(
    catalog: &ModelCatalog,
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
) -> ResolveCliModelResult {
    let Some(cli_model) = cli_model else {
        return ResolveCliModelResult::default();
    };

    let available_models = catalog.all_models();
    if available_models.is_empty() {
        return ResolveCliModelResult {
            model: None,
            thinking_level: None,
            warning: None,
            error: Some(
                "No models available. Check your installation or add models to models.json.".into(),
            ),
        };
    }

    let mut provider_map = HashMap::<String, String>::new();
    for model in available_models {
        provider_map
            .entry(model.provider.to_ascii_lowercase())
            .or_insert_with(|| model.provider.clone());
    }

    let mut provider =
        cli_provider.and_then(|name| provider_map.get(&name.to_ascii_lowercase()).cloned());
    if cli_provider.is_some() && provider.is_none() {
        return ResolveCliModelResult {
            model: None,
            thinking_level: None,
            warning: None,
            error: Some(format!(
                "Unknown provider \"{}\". Use --list-models to see available providers/models.",
                cli_provider.unwrap_or_default()
            )),
        };
    }

    let mut pattern = cli_model.to_string();
    let mut inferred_provider = false;

    if provider.is_none()
        && let Some((maybe_provider, remainder)) = cli_model.split_once('/')
        && let Some(canonical_provider) = provider_map.get(&maybe_provider.to_ascii_lowercase())
    {
        provider = Some(canonical_provider.clone());
        pattern = remainder.to_string();
        inferred_provider = true;
    }

    if provider.is_none()
        && let Some(exact) = available_models.iter().find(|model| {
            model.id.eq_ignore_ascii_case(cli_model)
                || canonical_model_reference(model).eq_ignore_ascii_case(cli_model)
        })
    {
        return ResolveCliModelResult {
            model: Some(exact.clone()),
            thinking_level: None,
            warning: None,
            error: None,
        };
    }

    if let Some(provider_name) = provider.as_deref()
        && let Some(stripped) = strip_provider_prefix(cli_model, provider_name)
    {
        pattern = stripped.to_string();
    }

    let candidate_models: Vec<Model> = match provider.as_deref() {
        Some(provider_name) => available_models
            .iter()
            .filter(|model| model.provider == provider_name)
            .cloned()
            .collect(),
        None => available_models.to_vec(),
    };

    let parsed = parse_model_pattern(
        &pattern,
        &candidate_models,
        ParseModelPatternOptions {
            allow_invalid_thinking_level_fallback: false,
        },
    );
    if parsed.model.is_some() {
        return ResolveCliModelResult {
            model: parsed.model,
            thinking_level: parsed.thinking_level,
            warning: parsed.warning,
            error: None,
        };
    }

    if inferred_provider {
        if let Some(exact) = available_models.iter().find(|model| {
            model.id.eq_ignore_ascii_case(cli_model)
                || canonical_model_reference(model).eq_ignore_ascii_case(cli_model)
        }) {
            return ResolveCliModelResult {
                model: Some(exact.clone()),
                thinking_level: None,
                warning: None,
                error: None,
            };
        }

        let fallback = parse_model_pattern(
            cli_model,
            available_models,
            ParseModelPatternOptions {
                allow_invalid_thinking_level_fallback: false,
            },
        );
        if fallback.model.is_some() {
            return ResolveCliModelResult {
                model: fallback.model,
                thinking_level: fallback.thinking_level,
                warning: fallback.warning,
                error: None,
            };
        }
    }

    if let Some(provider_name) = provider.as_deref()
        && let Some(model) = build_fallback_model(provider_name, &pattern, available_models)
    {
        let warning = parsed.warning.map_or_else(
            || {
                format!(
                    "Model \"{pattern}\" not found for provider \"{provider_name}\". Using custom model id."
                )
            },
            |warning| {
                format!(
                    "{warning} Model \"{pattern}\" not found for provider \"{provider_name}\". Using custom model id."
                )
            },
        );
        return ResolveCliModelResult {
            model: Some(model),
            thinking_level: None,
            warning: Some(warning),
            error: None,
        };
    }

    let display = provider
        .as_deref()
        .map(|provider_name| format!("{provider_name}/{pattern}"))
        .unwrap_or_else(|| cli_model.to_string());
    ResolveCliModelResult {
        model: None,
        thinking_level: None,
        warning: parsed.warning,
        error: Some(format!(
            "Model \"{display}\" not found. Use --list-models to see available models."
        )),
    }
}

pub fn resolve_model_scope(
    patterns: &[String],
    available_models: &[Model],
) -> ResolveModelScopeResult {
    let mut scoped_models = Vec::new();
    let mut warnings = Vec::new();

    for pattern in patterns {
        if has_glob_chars(pattern) {
            let (glob_pattern, thinking_level) = parse_glob_pattern(pattern);
            let matches = available_models
                .iter()
                .filter(|model| {
                    let full_id = canonical_model_reference(model);
                    glob_matches(glob_pattern, &full_id) || glob_matches(glob_pattern, &model.id)
                })
                .cloned()
                .collect::<Vec<_>>();

            if matches.is_empty() {
                warnings.push(format!("No models match pattern \"{pattern}\""));
                continue;
            }

            for model in matches {
                push_scoped_model(&mut scoped_models, model, thinking_level);
            }
            continue;
        }

        let parsed = parse_model_pattern(
            pattern,
            available_models,
            ParseModelPatternOptions::default(),
        );

        if let Some(warning) = parsed.warning {
            warnings.push(warning);
        }

        if let Some(model) = parsed.model {
            push_scoped_model(&mut scoped_models, model, parsed.thinking_level);
        } else {
            warnings.push(format!("No models match pattern \"{pattern}\""));
        }
    }

    ResolveModelScopeResult {
        scoped_models,
        warnings,
    }
}

pub fn find_initial_model(
    catalog: &ModelCatalog,
    options: InitialModelOptions,
) -> InitialModelResult {
    if !options.is_continuing
        && let Some(scoped_model) = options.scoped_models.first()
    {
        return InitialModelResult {
            model: Some(scoped_model.model.clone()),
            thinking_level: scoped_model
                .thinking_level
                .or(options.default_thinking_level)
                .unwrap_or(DEFAULT_THINKING_LEVEL),
            fallback_message: None,
        };
    }

    if let (Some(default_provider), Some(default_model_id)) = (
        options.default_provider.as_deref(),
        options.default_model_id.as_deref(),
    ) && let Some(model) = catalog.find(default_provider, default_model_id)
    {
        return InitialModelResult {
            model: Some(model),
            thinking_level: options
                .default_thinking_level
                .unwrap_or(DEFAULT_THINKING_LEVEL),
            fallback_message: None,
        };
    }

    if let Some(model) = select_default_available_model(catalog.available_models()) {
        return InitialModelResult {
            model: Some(model),
            thinking_level: DEFAULT_THINKING_LEVEL,
            fallback_message: None,
        };
    }

    InitialModelResult {
        model: None,
        thinking_level: DEFAULT_THINKING_LEVEL,
        fallback_message: None,
    }
}

pub fn restore_model_from_session(
    catalog: &ModelCatalog,
    saved_provider: &str,
    saved_model_id: &str,
    current_model: Option<&Model>,
) -> RestoreModelResult {
    let restored_model = catalog.find(saved_provider, saved_model_id);
    let has_configured_auth = restored_model
        .as_ref()
        .is_some_and(|model| catalog.has_configured_auth(model));

    if restored_model.is_some() && has_configured_auth {
        return RestoreModelResult {
            model: restored_model,
            fallback_message: None,
        };
    }

    let reason = if restored_model.is_none() {
        "model no longer exists"
    } else {
        "no auth configured"
    };

    if let Some(current_model) = current_model {
        return RestoreModelResult {
            model: Some(current_model.clone()),
            fallback_message: Some(format!(
                "Could not restore model {saved_provider}/{saved_model_id} ({reason}). Using {}/{}.",
                current_model.provider, current_model.id
            )),
        };
    }

    if let Some(fallback_model) = select_default_available_model(catalog.available_models()) {
        return RestoreModelResult {
            model: Some(fallback_model.clone()),
            fallback_message: Some(format!(
                "Could not restore model {saved_provider}/{saved_model_id} ({reason}). Using {}/{}.",
                fallback_model.provider, fallback_model.id
            )),
        };
    }

    RestoreModelResult {
        model: None,
        fallback_message: None,
    }
}

fn try_match_model(model_pattern: &str, available_models: &[Model]) -> Option<Model> {
    if let Some(exact_match) = find_exact_model_reference_match(model_pattern, available_models) {
        return Some(exact_match);
    }

    let normalized_pattern = model_pattern.to_ascii_lowercase();
    let mut alias_matches = Vec::<Model>::new();
    let mut dated_matches = Vec::<Model>::new();

    for model in available_models {
        let id_match = model.id.to_ascii_lowercase().contains(&normalized_pattern);
        let name_match = model
            .name
            .to_ascii_lowercase()
            .contains(&normalized_pattern);
        if !id_match && !name_match {
            continue;
        }

        if is_alias(&model.id) {
            alias_matches.push(model.clone());
        } else {
            dated_matches.push(model.clone());
        }
    }

    alias_matches.sort_by(|left, right| right.id.cmp(&left.id));
    dated_matches.sort_by(|left, right| right.id.cmp(&left.id));

    alias_matches
        .into_iter()
        .next()
        .or_else(|| dated_matches.into_iter().next())
}

fn is_alias(id: &str) -> bool {
    if id.ends_with("-latest") {
        return true;
    }

    match id.rsplit_once('-') {
        Some((_, suffix)) => {
            !(suffix.len() == 8 && suffix.chars().all(|character| character.is_ascii_digit()))
        }
        None => true,
    }
}

fn canonical_model_reference(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

fn strip_provider_prefix<'a>(cli_model: &'a str, provider: &str) -> Option<&'a str> {
    let prefix = format!("{provider}/");
    if cli_model.len() < prefix.len() {
        return None;
    }
    cli_model[..prefix.len()]
        .eq_ignore_ascii_case(&prefix)
        .then_some(&cli_model[prefix.len()..])
}

fn has_glob_chars(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn parse_glob_pattern(pattern: &str) -> (&str, Option<ThinkingLevel>) {
    let Some(last_colon_index) = pattern.rfind(':') else {
        return (pattern, None);
    };

    let suffix = &pattern[last_colon_index + 1..];
    let Some(thinking_level) = parse_thinking_level(suffix) else {
        return (pattern, None);
    };

    (&pattern[..last_colon_index], Some(thinking_level))
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .build()
        .ok()
        .is_some_and(|glob| glob.compile_matcher().is_match(value))
}

fn push_scoped_model(
    scoped_models: &mut Vec<ScopedModel>,
    model: Model,
    thinking_level: Option<ThinkingLevel>,
) {
    if scoped_models
        .iter()
        .any(|scoped| same_model_reference(&scoped.model, &model))
    {
        return;
    }

    scoped_models.push(ScopedModel {
        model,
        thinking_level,
    });
}

fn same_model_reference(left: &Model, right: &Model) -> bool {
    left.provider == right.provider && left.id == right.id
}

fn build_fallback_model(
    provider: &str,
    model_id: &str,
    available_models: &[Model],
) -> Option<Model> {
    let provider_models: Vec<&Model> = available_models
        .iter()
        .filter(|model| model.provider == provider)
        .collect();
    if provider_models.is_empty() {
        return None;
    }

    let base_model = default_model_id_for_provider(provider)
        .and_then(|default_id| {
            provider_models
                .iter()
                .find(|model| model.id == default_id)
                .copied()
        })
        .or_else(|| provider_models.first().copied())?;

    let mut fallback = base_model.clone();
    fallback.id = model_id.to_string();
    fallback.name = model_id.to_string();
    Some(fallback)
}

fn select_default_available_model(available_models: &[Model]) -> Option<Model> {
    for (provider, default_id) in DEFAULT_MODELS {
        if let Some(model) = available_models
            .iter()
            .find(|model| model.provider == provider && model.id == default_id)
        {
            return Some(model.clone());
        }
    }

    available_models.first().cloned()
}
