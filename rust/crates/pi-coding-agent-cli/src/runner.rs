use crate::{
    AppMode, Args, Diagnostic, DiagnosticKind, ListModels, OverlayAuthSource, PrintModeOptions,
    ProcessFileOptions, build_initial_message, list_models::render_list_models, parse_args,
    process_file_arguments, resolve_app_mode, run_print_mode, to_print_output_mode,
};
use pi_ai::{StreamOptions, ThinkingBudgets};
use pi_coding_agent_core::{
    AuthSource, BootstrapDiagnosticLevel, CodingAgentCoreError, CodingAgentCoreOptions,
    FooterDataProvider, ModelRegistry, ScopedModel, SessionBootstrapOptions,
    create_coding_agent_core, resolve_cli_model, resolve_model_scope,
};
use pi_coding_agent_tui::{
    InteractiveCoreBinding, KeybindingsManager, PlainKeyHintStyler, StartupShellComponent,
};
use pi_config::{ThinkingBudgetsSettings, load_runtime_settings};
use pi_events::{Message, Model, UserContent};
use pi_tui::{CombinedAutocompleteProvider, ProcessTerminal, Terminal, Tui};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::time::sleep;

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
    pub agent_dir: Option<PathBuf>,
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

type InteractiveTerminalFactory = Arc<dyn Fn() -> Box<dyn Terminal> + Send + Sync>;

pub async fn run_interactive_command(options: RunCommandOptions) -> i32 {
    run_interactive_command_with_terminal(options, Arc::new(|| Box::new(ProcessTerminal::new())))
        .await
}

pub async fn run_interactive_command_with_terminal(
    options: RunCommandOptions,
    interactive_terminal_factory: InteractiveTerminalFactory,
) -> i32 {
    let RunCommandOptions {
        args,
        stdin_is_tty,
        stdin_content,
        auth_source,
        built_in_models,
        models_json_path,
        agent_dir,
        cwd,
        default_system_prompt,
        version,
        stream_options,
    } = options;

    let parsed = parse_args(&args);
    let parse_diagnostics = render_parse_diagnostics(&parsed.diagnostics);
    if !parse_diagnostics.is_empty() {
        eprint!("{parse_diagnostics}");
    }
    if parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == DiagnosticKind::Error)
    {
        return 1;
    }

    if parsed.help || parsed.version || parsed.export.is_some() || parsed.list_models.is_some() {
        let result = run_command(RunCommandOptions {
            args,
            stdin_is_tty,
            stdin_content,
            auth_source,
            built_in_models,
            models_json_path,
            agent_dir,
            cwd,
            default_system_prompt,
            version,
            stream_options,
        })
        .await;
        if !result.stdout.is_empty() {
            print!("{}", result.stdout);
        }
        if !result.stderr.is_empty() {
            eprint!("{}", result.stderr);
        }
        return result.exit_code;
    }

    if let Some(message) = unsupported_flag_message(&parsed) {
        eprintln!("{message}");
        return 1;
    }

    let app_mode = resolve_app_mode(&parsed, stdin_is_tty);
    if app_mode != AppMode::Interactive {
        let result = run_command(RunCommandOptions {
            args,
            stdin_is_tty,
            stdin_content,
            auth_source,
            built_in_models,
            models_json_path,
            agent_dir,
            cwd,
            default_system_prompt,
            version,
            stream_options,
        })
        .await;
        if !result.stdout.is_empty() {
            print!("{}", result.stdout);
        }
        if !result.stderr.is_empty() {
            eprint!("{}", result.stderr);
        }
        return result.exit_code;
    }

    let runtime_settings = agent_dir
        .as_deref()
        .map(|agent_dir| load_runtime_settings(&cwd, agent_dir))
        .unwrap_or_default();
    eprint!("{}", render_settings_warnings(&runtime_settings.warnings));

    let scoped_models = if let Some(patterns) = parsed.models.as_ref() {
        let registry = ModelRegistry::new(
            auth_source.clone(),
            built_in_models.clone(),
            models_json_path.clone(),
        );
        let resolved = resolve_model_scope(patterns, &registry.get_available());
        eprint!("{}", render_scope_warnings(&resolved.warnings));
        resolved.scoped_models
    } else {
        Vec::new()
    };

    let stdin_content = normalize_stdin_content(stdin_is_tty, stdin_content);
    let processed_files = match process_file_arguments(
        &parsed.file_args,
        &cwd,
        ProcessFileOptions {
            auto_resize_images: runtime_settings.settings.images.auto_resize_images,
        },
    ) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("Error: {error}");
            return 1;
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
        eprintln!("Error: {error}");
        return 1;
    }

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(overlay_auth),
        built_in_models,
        models_json_path: models_json_path.clone(),
        cwd: Some(cwd.clone()),
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
            eprint!("{}", render_no_models_message(models_json_path.as_deref()));
            return 1;
        }
        Err(error) => {
            eprintln!("Error: {error}");
            return 1;
        }
    };

    created
        .core
        .set_auto_resize_images(runtime_settings.settings.images.auto_resize_images);
    created
        .core
        .set_block_images(runtime_settings.settings.images.block_images);
    created.core.set_thinking_budgets(map_thinking_budgets(
        &runtime_settings.settings.thinking_budgets,
    ));

    let bootstrap_output = render_bootstrap_diagnostics(&created.diagnostics);
    if !bootstrap_output.is_empty() {
        eprint!("{bootstrap_output}");
    }
    if created
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == BootstrapDiagnosticLevel::Error)
    {
        return 1;
    }

    let mut keybindings = match &agent_dir {
        Some(agent_dir) => KeybindingsManager::create(agent_dir),
        None => KeybindingsManager::new(BTreeMap::new(), None),
    };
    keybindings.reload();

    let mut shell = StartupShellComponent::new(
        "Pi",
        &version,
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_autocomplete_max_visible(runtime_settings.settings.autocomplete_max_visible);
    shell.set_autocomplete_provider(Arc::new(CombinedAutocompleteProvider::new(
        Vec::new(),
        cwd.clone(),
    )));

    let exit_requested = Arc::new(AtomicBool::new(false));
    let exit_requested_for_shell = Arc::clone(&exit_requested);
    shell.set_on_exit(move || {
        exit_requested_for_shell.store(true, Ordering::Relaxed);
    });

    let footer_provider = FooterDataProvider::new(&cwd);
    let mut tui = Tui::new(interactive_terminal_factory());
    shell.bind_footer_data_provider_with_render_handle(&footer_provider, tui.render_handle());
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, tui.render_handle());
    let shell_id = tui.add_child(Box::new(shell));
    let _ = tui.set_focus_child(shell_id);
    let _ = tui.terminal_mut().set_title("pi");

    if let Err(error) = tui.start() {
        eprintln!("Error: {error}");
        drop(binding);
        return 1;
    }

    spawn_initial_interactive_messages(created.core.clone(), initial_message, messages);

    let mut exit_code = 0;
    while !exit_requested.load(Ordering::Relaxed) {
        if let Err(error) = tui.drain_terminal_events() {
            eprintln!("Error: {error}");
            exit_code = 1;
            break;
        }
        sleep(Duration::from_millis(16)).await;
    }

    created.core.abort();
    created.core.wait_for_idle().await;
    let _ = tui
        .terminal_mut()
        .drain_input(Duration::from_millis(1000), Duration::from_millis(50));
    let _ = tui.stop();
    drop(binding);

    exit_code
}

pub async fn run_command(options: RunCommandOptions) -> RunCommandResult {
    let RunCommandOptions {
        args,
        stdin_is_tty,
        stdin_content,
        auth_source,
        built_in_models,
        models_json_path,
        agent_dir,
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

    let runtime_settings = agent_dir
        .as_deref()
        .map(|agent_dir| load_runtime_settings(&cwd, agent_dir))
        .unwrap_or_default();
    stderr.push_str(&render_settings_warnings(&runtime_settings.warnings));

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
    let processed_files = match process_file_arguments(
        &parsed.file_args,
        &cwd,
        ProcessFileOptions {
            auto_resize_images: runtime_settings.settings.images.auto_resize_images,
        },
    ) {
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

    created
        .core
        .set_auto_resize_images(runtime_settings.settings.images.auto_resize_images);
    created
        .core
        .set_block_images(runtime_settings.settings.images.block_images);
    created.core.set_thinking_budgets(map_thinking_budgets(
        &runtime_settings.settings.thinking_budgets,
    ));

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

fn spawn_initial_interactive_messages(
    core: pi_coding_agent_core::CodingAgentCore,
    initial_message: crate::InitialMessageResult,
    messages: Vec<String>,
) {
    tokio::spawn(async move {
        if let Some(message) = build_initial_user_message(initial_message) {
            let _ = core.prompt_message(message).await;
        }

        for message in messages {
            let _ = core.prompt_text(message).await;
        }
    });
}

fn build_initial_user_message(initial_message: crate::InitialMessageResult) -> Option<Message> {
    let mut content = Vec::new();

    if let Some(text) = initial_message.initial_message {
        content.push(UserContent::Text { text });
    }
    if let Some(images) = initial_message.initial_images {
        content.extend(images);
    }

    if content.is_empty() {
        None
    } else {
        Some(Message::User {
            content,
            timestamp: 0,
        })
    }
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

fn map_thinking_budgets(settings: &ThinkingBudgetsSettings) -> ThinkingBudgets {
    ThinkingBudgets {
        minimal: settings.minimal,
        low: settings.low,
        medium: settings.medium,
        high: settings.high,
    }
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

fn render_settings_warnings(warnings: &[pi_config::SettingsWarning]) -> String {
    let mut output = String::new();
    for warning in warnings {
        push_line(
            &mut output,
            &format!(
                "Warning: ({} settings) {}",
                warning.scope.label(),
                warning.message
            ),
        );
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
