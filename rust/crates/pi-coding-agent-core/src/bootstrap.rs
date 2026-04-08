use crate::{
    model_registry::ModelRegistry,
    model_resolver::{
        DEFAULT_THINKING_LEVEL, InitialModelOptions, ScopedModel, find_initial_model,
        resolve_cli_model,
    },
};
use pi_agent::ThinkingLevel;
use pi_ai::supports_xhigh;
use pi_events::Model;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapDiagnosticLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapDiagnostic {
    pub level: BootstrapDiagnosticLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExistingSessionSelection {
    pub has_messages: bool,
    pub saved_model_provider: Option<String>,
    pub saved_model_id: Option<String>,
    pub saved_thinking_level: Option<ThinkingLevel>,
    pub has_thinking_entry: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionBootstrapOptions {
    pub cli_provider: Option<String>,
    pub cli_model: Option<String>,
    pub cli_thinking_level: Option<ThinkingLevel>,
    pub scoped_models: Vec<ScopedModel>,
    pub default_provider: Option<String>,
    pub default_model_id: Option<String>,
    pub default_thinking_level: Option<ThinkingLevel>,
    pub existing_session: ExistingSessionSelection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionBootstrapResult {
    pub model: Option<Model>,
    pub thinking_level: ThinkingLevel,
    pub model_fallback_message: Option<String>,
    pub diagnostics: Vec<BootstrapDiagnostic>,
}

pub fn bootstrap_session(
    model_registry: &ModelRegistry,
    options: SessionBootstrapOptions,
) -> SessionBootstrapResult {
    let catalog = model_registry.catalog();
    let mut diagnostics = Vec::new();
    let mut model = None;
    let mut thinking_level = None;
    let mut should_clamp_xhigh = options.cli_thinking_level.is_some();

    if options.cli_model.is_some() {
        let resolved = resolve_cli_model(
            &catalog,
            options.cli_provider.as_deref(),
            options.cli_model.as_deref(),
        );
        if let Some(warning) = resolved.warning {
            diagnostics.push(BootstrapDiagnostic {
                level: BootstrapDiagnosticLevel::Warning,
                message: warning,
            });
        }
        if let Some(error) = resolved.error {
            diagnostics.push(BootstrapDiagnostic {
                level: BootstrapDiagnosticLevel::Error,
                message: error,
            });
        }
        if let Some(resolved_model) = resolved.model {
            model = Some(resolved_model);
            if options.cli_thinking_level.is_none() {
                if resolved.thinking_level.is_some() {
                    should_clamp_xhigh = true;
                }
                thinking_level = resolved.thinking_level;
            }
        }
    }

    if model.is_none()
        && !options.existing_session.has_messages
        && !options.scoped_models.is_empty()
    {
        if let Some(saved_in_scope) = options
            .default_provider
            .as_deref()
            .zip(options.default_model_id.as_deref())
            .and_then(|(provider, model_id)| model_registry.find(provider, model_id))
            .and_then(|saved_model| {
                options
                    .scoped_models
                    .iter()
                    .find(|scoped| same_model(&scoped.model, &saved_model))
                    .cloned()
            })
        {
            if options.cli_thinking_level.is_none() && saved_in_scope.thinking_level.is_some() {
                thinking_level = saved_in_scope.thinking_level;
            }
            model = Some(saved_in_scope.model);
        } else if let Some(first_scoped) = options.scoped_models.first().cloned() {
            if options.cli_thinking_level.is_none() && first_scoped.thinking_level.is_some() {
                thinking_level = first_scoped.thinking_level;
            }
            model = Some(first_scoped.model);
        }
    }

    if let Some(cli_thinking_level) = options.cli_thinking_level {
        thinking_level = Some(cli_thinking_level);
    }

    let mut model_fallback_message = None;

    if model.is_none() && options.existing_session.has_messages {
        if let (Some(saved_provider), Some(saved_model_id)) = (
            options.existing_session.saved_model_provider.as_deref(),
            options.existing_session.saved_model_id.as_deref(),
        ) {
            let restored_model = model_registry.find(saved_provider, saved_model_id);
            if restored_model
                .as_ref()
                .is_some_and(|restored| model_registry.has_configured_auth(restored))
            {
                model = restored_model;
            }
            if model.is_none() {
                model_fallback_message = Some(format!(
                    "Could not restore model {saved_provider}/{saved_model_id}"
                ));
            }
        }
    }

    if model.is_none() {
        let initial = find_initial_model(
            &catalog,
            InitialModelOptions {
                scoped_models: Vec::new(),
                is_continuing: options.existing_session.has_messages,
                default_provider: options.default_provider.clone(),
                default_model_id: options.default_model_id.clone(),
                default_thinking_level: options.default_thinking_level,
            },
        );
        model = initial.model;
        if let Some(message) = model_fallback_message.as_mut()
            && let Some(selected_model) = model.as_ref()
        {
            *message = format!(
                "{message}. Using {}/{}",
                selected_model.provider, selected_model.id
            );
        }
    }

    let mut effective_thinking_level = if let Some(level) = thinking_level {
        level
    } else if options.existing_session.has_messages {
        if options.existing_session.has_thinking_entry {
            options
                .existing_session
                .saved_thinking_level
                .unwrap_or(DEFAULT_THINKING_LEVEL)
        } else {
            options
                .default_thinking_level
                .unwrap_or(DEFAULT_THINKING_LEVEL)
        }
    } else {
        options
            .default_thinking_level
            .unwrap_or(DEFAULT_THINKING_LEVEL)
    };

    if model
        .as_ref()
        .is_none_or(|selected_model| !selected_model.reasoning)
    {
        effective_thinking_level = ThinkingLevel::Off;
    } else if should_clamp_xhigh
        && effective_thinking_level == ThinkingLevel::XHigh
        && model
            .as_ref()
            .is_some_and(|selected_model| !supports_xhigh(selected_model))
    {
        effective_thinking_level = ThinkingLevel::High;
    }

    SessionBootstrapResult {
        model,
        thinking_level: effective_thinking_level,
        model_fallback_message,
        diagnostics,
    }
}

fn same_model(left: &Model, right: &Model) -> bool {
    left.provider == right.provider && left.id == right.id
}
