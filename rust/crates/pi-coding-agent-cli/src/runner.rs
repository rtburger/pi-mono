use crate::{
    AppMode, Args, Diagnostic, DiagnosticKind, ListModels, OverlayAuthSource, PrintModeOptions,
    build_initial_message, list_models::render_list_models, parse_args, process_file_arguments,
    resolve_app_mode, run_print_mode, to_print_output_mode,
};
use pi_ai::StreamOptions;
use pi_coding_agent_core::{
    AuthSource, BootstrapDiagnosticLevel, CodingAgentCoreError, CodingAgentCoreOptions,
    ModelRegistry, ScopedModel, SessionBootstrapOptions, create_coding_agent_core,
    resolve_cli_model, resolve_model_scope,
};
use pi_events::Model;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const NO_MODELS_ENV_HINT: &str = "  ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY, etc.";
const API_KEY_MODEL_REQUIREMENT: &str =
    "--api-key requires a model to be specified via --model, --provider/--model, or --models";

pub struct RunCommandOptions {
    pub args: Vec<String>,
    pub stdin_is_tty: bool,
    pub stdin_content: Option<String>,
    pub auth_source: Arc<dyn AuthSource>,
    pub built_in_models: Vec<Model>,
    pub models_json_path: Option<PathBuf>,
    pub cwd: PathBuf,
    pub default_system_prompt: String,
    pub version: String,
    pub stream_options: StreamOptions,
}

pub struct RunCommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub async fn run_command(options: RunCommandOptions) -> RunCommandResult {
    let RunCommandOptions {
        args,
        stdin_is_tty,
        stdin_content,
        auth_source,
        built_in_models,
        models_json_path,
        cwd,
        default_system_prompt,
        version,
        stream_options,
    } = options;

    let parsed = parse_args(&args);
    let mut stdout = String::new();
    let mut stderr = render_parse_diagnostics(&parsed.diagnostics);

    if parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == DiagnosticKind::Error)
    {
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    }

    if parsed.version {
        push_line(&mut stdout, &version);
        return RunCommandResult {
            exit_code: 0,
            stdout,
            stderr,
        };
    }

    if parsed.help {
        push_line(&mut stdout, &render_help());
        return RunCommandResult {
            exit_code: 0,
            stdout,
            stderr,
        };
    }

    if parsed.export.is_some() {
        push_line(&mut stderr, "--export is not supported in the Rust CLI yet");
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    }

    if let Some(list_models) = parsed.list_models.as_ref() {
        let registry = ModelRegistry::new(auth_source, built_in_models, models_json_path);
        let search_pattern = match list_models {
            ListModels::All => None,
            ListModels::Search(pattern) => Some(pattern.as_str()),
        };
        stdout.push_str(&render_list_models(&registry, search_pattern));
        return RunCommandResult {
            exit_code: 0,
            stdout,
            stderr,
        };
    }

    if let Some(message) = unsupported_flag_message(&parsed) {
        push_line(&mut stderr, &message);
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    }

    let app_mode = resolve_app_mode(&parsed, stdin_is_tty);
    let Some(print_mode) = to_print_output_mode(app_mode) else {
        push_line(&mut stderr, &unsupported_app_mode_message(app_mode));
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    };

    let scoped_models = if let Some(patterns) = parsed.models.as_ref() {
        let registry = ModelRegistry::new(
            auth_source.clone(),
            built_in_models.clone(),
            models_json_path.clone(),
        );
        let resolved = resolve_model_scope(patterns, &registry.get_available());
        stderr.push_str(&render_scope_warnings(&resolved.warnings));
        resolved.scoped_models
    } else {
        Vec::new()
    };

    let stdin_content = normalize_stdin_content(stdin_is_tty, stdin_content);
    let processed_files = match process_file_arguments(&parsed.file_args, &cwd) {
        Ok(files) => files,
        Err(error) => {
            push_line(&mut stderr, &format!("Error: {error}"));
            return RunCommandResult {
                exit_code: 1,
                stdout,
                stderr,
            };
        }
    };

    let mut messages = parsed.messages.clone();
    let initial_message = build_initial_message(
        &mut messages,
        (!processed_files.text.is_empty()).then_some(processed_files.text),
        processed_files.images,
        stdin_content,
    );

    let overlay_auth = OverlayAuthSource::new(auth_source);
    if let Err(error) = apply_runtime_api_key_override(
        &parsed,
        &overlay_auth,
        &built_in_models,
        models_json_path.as_deref(),
        &scoped_models,
    ) {
        push_line(&mut stderr, &format!("Error: {error}"));
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    }

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(overlay_auth),
        built_in_models,
        models_json_path: models_json_path.clone(),
        cwd: Some(cwd),
        tools: None,
        system_prompt: resolve_system_prompt(
            &default_system_prompt,
            parsed.system_prompt.as_deref(),
            parsed.append_system_prompt.as_deref(),
        ),
        bootstrap: SessionBootstrapOptions {
            cli_provider: parsed.provider.clone(),
            cli_model: parsed.model.clone(),
            cli_thinking_level: parsed.thinking,
            scoped_models,
            ..SessionBootstrapOptions::default()
        },
        stream_options,
    });

    let created = match created {
        Ok(created) => created,
        Err(CodingAgentCoreError::NoModelAvailable) => {
            stderr.push_str(&render_no_models_message(models_json_path.as_deref()));
            return RunCommandResult {
                exit_code: 1,
                stdout,
                stderr,
            };
        }
        Err(error) => {
            push_line(&mut stderr, &format!("Error: {error}"));
            return RunCommandResult {
                exit_code: 1,
                stdout,
                stderr,
            };
        }
    };

    stderr.push_str(&render_bootstrap_diagnostics(&created.diagnostics));
    if created
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == BootstrapDiagnosticLevel::Error)
    {
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    }

    let run_result = run_print_mode(
        &created.core,
        PrintModeOptions {
            mode: print_mode,
            messages,
            initial_message: initial_message.initial_message,
            initial_images: initial_message.initial_images,
        },
    )
    .await;

    stdout.push_str(&run_result.stdout);
    stderr.push_str(&run_result.stderr);

    RunCommandResult {
        exit_code: run_result.exit_code,
        stdout,
        stderr,
    }
}

fn apply_runtime_api_key_override(
    parsed: &Args,
    overlay_auth: &OverlayAuthSource,
    built_in_models: &[Model],
    models_json_path: Option<&Path>,
    scoped_models: &[ScopedModel],
) -> Result<(), String> {
    let Some(api_key) = parsed.api_key.as_ref() else {
        return Ok(());
    };

    if parsed.model.is_none() {
        if let Some(scoped_model) = scoped_models.first() {
            overlay_auth.set_runtime_api_key(scoped_model.model.provider.clone(), api_key.clone());
            return Ok(());
        }

        return Err(API_KEY_MODEL_REQUIREMENT.into());
    }

    let registry = ModelRegistry::new(
        Arc::new(overlay_auth.clone()),
        built_in_models.to_vec(),
        models_json_path.map(Path::to_path_buf),
    );
    let resolved = resolve_cli_model(
        &registry.catalog(),
        parsed.provider.as_deref(),
        parsed.model.as_deref(),
    );

    if let Some(model) = resolved.model {
        overlay_auth.set_runtime_api_key(model.provider, api_key.clone());
        return Ok(());
    }

    if let Some(provider) = parsed.provider.as_deref() {
        overlay_auth.set_runtime_api_key(provider.to_string(), api_key.clone());
        return Ok(());
    }

    Err(API_KEY_MODEL_REQUIREMENT.into())
}

fn normalize_stdin_content(stdin_is_tty: bool, stdin_content: Option<String>) -> Option<String> {
    if stdin_is_tty {
        return None;
    }

    stdin_content
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
}

fn resolve_system_prompt(
    default_system_prompt: &str,
    override_system_prompt: Option<&str>,
    append_system_prompt: Option<&str>,
) -> String {
    let mut system_prompt = override_system_prompt
        .unwrap_or(default_system_prompt)
        .to_string();

    if let Some(append_system_prompt) = append_system_prompt {
        if !system_prompt.is_empty() && !append_system_prompt.is_empty() {
            system_prompt.push('\n');
        }
        system_prompt.push_str(append_system_prompt);
    }

    system_prompt
}

fn render_parse_diagnostics(diagnostics: &[Diagnostic]) -> String {
    let mut output = String::new();
    for diagnostic in diagnostics {
        let label = match diagnostic.kind {
            DiagnosticKind::Warning => "Warning",
            DiagnosticKind::Error => "Error",
        };
        push_line(&mut output, &format!("{label}: {}", diagnostic.message));
    }
    output
}

fn render_scope_warnings(warnings: &[String]) -> String {
    let mut output = String::new();
    for warning in warnings {
        push_line(&mut output, &format!("Warning: {warning}"));
    }
    output
}

fn render_bootstrap_diagnostics(
    diagnostics: &[pi_coding_agent_core::BootstrapDiagnostic],
) -> String {
    let mut output = String::new();
    for diagnostic in diagnostics {
        let label = match diagnostic.level {
            BootstrapDiagnosticLevel::Warning => "Warning",
            BootstrapDiagnosticLevel::Error => "Error",
        };
        push_line(&mut output, &format!("{label}: {}", diagnostic.message));
    }
    output
}

fn render_no_models_message(models_json_path: Option<&Path>) -> String {
    let mut output = String::new();
    push_line(&mut output, "No models available.");
    push_line(&mut output, "");
    push_line(&mut output, "Set an API key environment variable:");
    push_line(&mut output, NO_MODELS_ENV_HINT);
    if let Some(models_json_path) = models_json_path {
        push_line(&mut output, "");
        push_line(
            &mut output,
            &format!("Or create {}", models_json_path.display()),
        );
    }
    output
}

fn unsupported_flag_message(parsed: &Args) -> Option<String> {
    if parsed.export.is_some() {
        return Some(String::from(
            "--export is not supported in the Rust CLI yet",
        ));
    }
    if parsed.continue_session
        || parsed.resume
        || parsed.no_session
        || parsed.session.is_some()
        || parsed.fork.is_some()
        || parsed.session_dir.is_some()
    {
        return Some(String::from(
            "Session flags are not supported in the Rust CLI yet",
        ));
    }
    if parsed.no_tools
        || parsed.tools.is_some()
        || parsed.extensions.is_some()
        || parsed.no_extensions
        || parsed.skills.is_some()
        || parsed.no_skills
        || parsed.prompt_templates.is_some()
        || parsed.no_prompt_templates
        || parsed.themes.is_some()
        || parsed.no_themes
    {
        return Some(String::from(
            "Resource and tool selection flags are not supported in the Rust CLI yet",
        ));
    }
    None
}

fn unsupported_app_mode_message(app_mode: AppMode) -> String {
    match app_mode {
        AppMode::Interactive => {
            String::from("Interactive mode is not supported in the Rust CLI yet")
        }
        AppMode::Rpc => String::from("RPC mode is not supported in the Rust CLI yet"),
        AppMode::Print | AppMode::Json => String::new(),
    }
}

fn render_help() -> String {
    [
        "pi Rust CLI migration",
        "",
        "Supported today:",
        "  - non-interactive text mode (-p / piped stdin)",
        "  - non-interactive json mode (--mode json)",
        "  - --provider, --model, --models, --api-key, --system-prompt, --append-system-prompt, --thinking",
        "  - --list-models [search]",
        "  - @file text/image preprocessing",
        "",
        "Not yet supported:",
        "  - interactive mode",
        "  - rpc mode",
        "  - sessions, export",
    ]
    .join("\n")
}

fn push_line(buffer: &mut String, line: &str) {
    buffer.push_str(line);
    buffer.push('\n');
}
