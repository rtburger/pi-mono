use crate::{
    AppMode, Args, Diagnostic, DiagnosticKind, ListModels, OverlayAuthSource, PrintModeOptions,
    PrintOutputMode, ProcessFileOptions, build_initial_message, list_models::render_list_models,
    parse_args, process_file_arguments, resolve_app_mode, run_print_mode, to_print_output_mode,
};
use pi_agent::ThinkingLevel;
use pi_ai::{StreamOptions, ThinkingBudgets, models_are_equal, supports_xhigh};
use pi_coding_agent_core::{
    AuthSource, BootstrapDiagnosticLevel, CodingAgentCore, CodingAgentCoreError,
    CodingAgentCoreOptions, ExistingSessionSelection, FooterDataProvider, ModelRegistry,
    ScopedModel, SessionBootstrapOptions, SessionEntry, SessionHeader, SessionManager,
    create_coding_agent_core, find_exact_model_reference_match, get_default_session_dir,
    parse_thinking_level, resolve_cli_model, resolve_model_scope, resolve_prompt_input,
};
use pi_coding_agent_tui::{
    ExternalEditorCommandRunner, ExternalEditorHost, FooterStateHandle, InteractiveCoreBinding,
    KeybindingsManager, PlainKeyHintStyler, StartupShellComponent, StatusHandle,
};
use pi_config::{ThinkingBudgetsSettings, load_runtime_settings};
use pi_events::{Message, Model, UserContent};
use pi_tui::{
    AutocompleteItem, CombinedAutocompleteProvider, ProcessTerminal, RenderHandle, SlashCommand,
    Terminal, Tui, TuiError, fuzzy_filter,
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::time::sleep;

const NO_MODELS_ENV_HINT: &str = "  ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY, etc.";
const API_KEY_MODEL_REQUIREMENT: &str =
    "--api-key requires a model to be specified via --model, --provider/--model, or --models";
const FINALIZED_SYSTEM_PROMPT_PREFIX: &str = "\0pi-final-system-prompt\n";

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

type InteractiveTerminalFactory = Arc<dyn Fn() -> Box<dyn Terminal + Send> + Send + Sync>;
type SharedInputHandler = Arc<Mutex<Box<dyn FnMut(String) + Send>>>;
type SharedResizeHandler = Arc<Mutex<Box<dyn FnMut() + Send>>>;

struct InteractiveRuntime {
    terminal_factory: InteractiveTerminalFactory,
    extension_editor_command: Option<String>,
    extension_editor_runner: Option<Arc<dyn ExternalEditorCommandRunner>>,
}

impl InteractiveRuntime {
    fn new(terminal_factory: InteractiveTerminalFactory) -> Self {
        Self {
            terminal_factory,
            extension_editor_command: None,
            extension_editor_runner: None,
        }
    }
}

#[derive(Clone)]
struct SessionSupport {
    manager: Arc<Mutex<SessionManager>>,
    header: SessionHeader,
    restored_messages: Vec<pi_agent::AgentMessage>,
    has_existing_messages: bool,
    has_thinking_entry: bool,
    existing_session: ExistingSessionSelection,
    session_id: String,
}

fn create_session_support(
    parsed: &Args,
    cwd: &Path,
    agent_dir: Option<&Path>,
) -> Result<Option<SessionSupport>, String> {
    let should_use_session_manager = parsed.no_session
        || parsed.continue_session
        || parsed.session.is_some()
        || parsed.fork.is_some()
        || parsed.session_dir.is_some()
        || agent_dir.is_some();
    if !should_use_session_manager {
        return Ok(None);
    }

    if parsed.resume {
        return Err(String::from(
            "--resume is not supported in the Rust CLI yet. Use --continue or --session <path>.",
        ));
    }

    let cwd_string = cwd.to_string_lossy().into_owned();
    let session_dir = resolve_session_dir(parsed, cwd, agent_dir);
    let session_manager = if parsed.no_session {
        SessionManager::in_memory(&cwd_string)
    } else if let Some(session) = parsed.session.as_deref() {
        SessionManager::open(
            &resolve_session_path(cwd, session),
            session_dir.as_deref(),
            None,
        )
        .map_err(|error| error.to_string())?
    } else if let Some(fork) = parsed.fork.as_deref() {
        SessionManager::fork_from(
            &resolve_session_path(cwd, fork),
            &cwd_string,
            session_dir.as_deref(),
        )
        .map_err(|error| error.to_string())?
    } else if parsed.continue_session {
        SessionManager::continue_recent(&cwd_string, session_dir.as_deref())
            .map_err(|error| error.to_string())?
    } else {
        SessionManager::create(&cwd_string, session_dir.as_deref())
            .map_err(|error| error.to_string())?
    };

    Ok(Some(build_session_support(session_manager)))
}

fn resolve_session_dir(parsed: &Args, cwd: &Path, agent_dir: Option<&Path>) -> Option<String> {
    if let Some(session_dir) = parsed.session_dir.as_deref() {
        return Some(resolve_path_from_cwd(cwd, session_dir));
    }

    agent_dir.map(|agent_dir| {
        get_default_session_dir(&cwd.to_string_lossy(), Some(&agent_dir.to_string_lossy()))
    })
}

fn resolve_session_path(cwd: &Path, path: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path).to_string_lossy().into_owned()
    }
}

fn resolve_path_from_cwd(cwd: &Path, path: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path).to_string_lossy().into_owned()
    }
}

fn build_session_support(session_manager: SessionManager) -> SessionSupport {
    let header = session_manager.get_header().clone();
    let restored_context = session_manager.build_session_context();
    let has_existing_messages = !restored_context.messages.is_empty();
    let has_thinking_entry = session_manager
        .get_branch(session_manager.get_leaf_id())
        .iter()
        .any(|entry| matches!(entry, SessionEntry::ThinkingLevelChange { .. }));
    let existing_session = ExistingSessionSelection {
        has_messages: has_existing_messages,
        saved_model_provider: restored_context
            .model
            .as_ref()
            .map(|model| model.provider.clone()),
        saved_model_id: restored_context
            .model
            .as_ref()
            .map(|model| model.model_id.clone()),
        saved_thinking_level: parse_thinking_level(&restored_context.thinking_level),
        has_thinking_entry,
    };
    let session_id = session_manager.get_session_id().to_owned();

    SessionSupport {
        manager: Arc::new(Mutex::new(session_manager)),
        header,
        restored_messages: restored_context.messages,
        has_existing_messages,
        has_thinking_entry,
        existing_session,
        session_id,
    }
}

fn apply_session_support(
    core: &CodingAgentCore,
    session_support: &SessionSupport,
) -> Result<(), String> {
    core.agent()
        .set_session_id(Some(session_support.session_id.clone()));

    if !session_support.restored_messages.is_empty() {
        let restored_messages = session_support.restored_messages.clone();
        core.agent().update_state(move |state| {
            state.messages = restored_messages;
        });
    }

    let state = core.state();
    {
        let mut session_manager = session_support
            .manager
            .lock()
            .expect("session manager mutex poisoned");
        if session_support.has_existing_messages {
            if !session_support.has_thinking_entry {
                session_manager
                    .append_thinking_level_change(thinking_level_label(state.thinking_level))
                    .map_err(|error| error.to_string())?;
            }
        } else {
            session_manager
                .append_model_change(state.model.provider.clone(), state.model.id.clone())
                .map_err(|error| error.to_string())?;
            session_manager
                .append_thinking_level_change(thinking_level_label(state.thinking_level))
                .map_err(|error| error.to_string())?;
        }
    }

    let session_manager = session_support.manager.clone();
    let _ = core.agent().subscribe(move |event, _signal| {
        let session_manager = session_manager.clone();
        Box::pin(async move {
            if let pi_agent::AgentEvent::MessageEnd { message } = event {
                let _ = session_manager
                    .lock()
                    .expect("session manager mutex poisoned")
                    .append_message(message);
            }
        })
    });

    Ok(())
}

fn session_header_json_line(header: &SessionHeader) -> String {
    serde_json::to_string(&serde_json::json!({
        "type": "session",
        "version": header.version,
        "id": header.id,
        "timestamp": header.timestamp,
        "cwd": header.cwd,
        "parentSession": header.parent_session,
    }))
    .expect("session header serialization should succeed")
        + "\n"
}

#[derive(Clone)]
struct LiveInteractiveTerminal {
    state: Arc<Mutex<LiveInteractiveTerminalState>>,
}

struct LiveInteractiveTerminalState {
    terminal: Box<dyn Terminal + Send>,
    input_handler: Option<SharedInputHandler>,
    resize_handler: Option<SharedResizeHandler>,
    started: bool,
}

impl LiveInteractiveTerminal {
    fn new(terminal: Box<dyn Terminal + Send>) -> Self {
        Self {
            state: Arc::new(Mutex::new(LiveInteractiveTerminalState {
                terminal,
                input_handler: None,
                resize_handler: None,
                started: false,
            })),
        }
    }

    fn external_editor_host(&self, render_handle: RenderHandle) -> LiveExternalEditorHost {
        LiveExternalEditorHost {
            terminal: self.clone(),
            render_handle,
        }
    }

    fn start_inner(state: &mut LiveInteractiveTerminalState) -> Result<(), TuiError> {
        if state.started {
            return Ok(());
        }

        let Some(input_handler) = state.input_handler.as_ref().cloned() else {
            return Ok(());
        };
        let Some(resize_handler) = state.resize_handler.as_ref().cloned() else {
            return Ok(());
        };

        state.terminal.start(
            Box::new(move |data| {
                let mut callback = input_handler
                    .lock()
                    .expect("interactive terminal input handler mutex poisoned");
                (callback)(data);
            }),
            Box::new(move || {
                let mut callback = resize_handler
                    .lock()
                    .expect("interactive terminal resize handler mutex poisoned");
                (callback)();
            }),
        )?;
        state.started = true;
        Ok(())
    }

    fn suspend_for_external_editor(&self) {
        let mut state = self
            .state
            .lock()
            .expect("live interactive terminal mutex poisoned");
        if !state.started {
            return;
        }

        let _ = state.terminal.show_cursor();
        if state.terminal.stop().is_ok() {
            state.started = false;
        }
    }

    fn resume_after_external_editor(&self) {
        let mut state = self
            .state
            .lock()
            .expect("live interactive terminal mutex poisoned");
        if state.started {
            return;
        }

        if Self::start_inner(&mut state).is_ok() && state.started {
            let _ = state.terminal.hide_cursor();
        }
    }
}

impl Terminal for LiveInteractiveTerminal {
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send>,
        on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        let mut state = self
            .state
            .lock()
            .expect("live interactive terminal mutex poisoned");
        state.input_handler = Some(Arc::new(Mutex::new(on_input)));
        state.resize_handler = Some(Arc::new(Mutex::new(on_resize)));
        Self::start_inner(&mut state)
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        let mut state = self
            .state
            .lock()
            .expect("live interactive terminal mutex poisoned");
        if !state.started {
            return Ok(());
        }

        state.terminal.stop()?;
        state.started = false;
        Ok(())
    }

    fn drain_input(&mut self, max: Duration, idle: Duration) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .drain_input(max, idle)
    }

    fn write(&mut self, data: &str) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .write(data)
    }

    fn columns(&self) -> u16 {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .columns()
    }

    fn rows(&self) -> u16 {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .rows()
    }

    fn kitty_protocol_active(&self) -> bool {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .kitty_protocol_active()
    }

    fn move_by(&mut self, lines: i32) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .move_by(lines)
    }

    fn hide_cursor(&mut self) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .show_cursor()
    }

    fn clear_line(&mut self) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .clear_line()
    }

    fn clear_from_cursor(&mut self) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .clear_from_cursor()
    }

    fn clear_screen(&mut self) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .clear_screen()
    }

    fn set_title(&mut self, title: &str) -> Result<(), TuiError> {
        self.state
            .lock()
            .expect("live interactive terminal mutex poisoned")
            .terminal
            .set_title(title)
    }
}

#[derive(Clone)]
struct LiveExternalEditorHost {
    terminal: LiveInteractiveTerminal,
    render_handle: RenderHandle,
}

impl ExternalEditorHost for LiveExternalEditorHost {
    fn stop(&self) {
        self.terminal.suspend_for_external_editor();
    }

    fn start(&self) {
        self.terminal.resume_after_external_editor();
    }

    fn request_render(&self) {
        self.render_handle.request_render();
    }
}

pub async fn run_interactive_command(options: RunCommandOptions) -> i32 {
    run_interactive_command_with_terminal(options, Arc::new(|| Box::new(ProcessTerminal::new())))
        .await
}

pub async fn run_interactive_command_with_terminal(
    options: RunCommandOptions,
    interactive_terminal_factory: InteractiveTerminalFactory,
) -> i32 {
    run_interactive_command_with_runtime(
        options,
        InteractiveRuntime::new(interactive_terminal_factory),
    )
    .await
}

async fn run_interactive_command_with_runtime(
    options: RunCommandOptions,
    runtime: InteractiveRuntime,
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
    let interactive_scoped_models = scoped_models.clone();
    let session_support = match create_session_support(&parsed, &cwd, agent_dir.as_deref()) {
        Ok(session_support) => session_support,
        Err(error) => {
            eprintln!("Error: {error}");
            return 1;
        }
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

    let mut stream_options = stream_options;
    if let Some(session_support) = session_support.as_ref() {
        stream_options.session_id = Some(session_support.session_id.clone());
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
            existing_session: session_support
                .as_ref()
                .map(|session_support| session_support.existing_session.clone())
                .unwrap_or_default(),
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

    if let Some(session_support) = session_support.as_ref()
        && let Err(error) = apply_session_support(&created.core, session_support)
    {
        eprintln!("Error: {error}");
        return 1;
    }

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

    let interactive_session_manager = session_support
        .as_ref()
        .map(|support| support.manager.clone());

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
    shell.set_input_padding_x(runtime_settings.settings.editor_padding_x);
    shell.set_autocomplete_max_visible(runtime_settings.settings.autocomplete_max_visible);
    shell.set_autocomplete_provider(Arc::new(CombinedAutocompleteProvider::new(
        build_interactive_slash_commands(
            created.core.model_registry(),
            interactive_scoped_models.clone(),
        ),
        cwd.clone(),
    )));

    let exit_requested = Arc::new(AtomicBool::new(false));
    let exit_requested_for_shell = Arc::clone(&exit_requested);
    shell.set_on_exit(move || {
        exit_requested_for_shell.store(true, Ordering::Relaxed);
    });

    let footer_provider = FooterDataProvider::new(&cwd);
    let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
    let mut tui = Tui::new(terminal.clone());
    let render_handle = tui.render_handle();
    if let Some(command) = runtime.extension_editor_command {
        shell.set_extension_editor_command(command);
    }
    if let Some(runner) = runtime.extension_editor_runner {
        shell.set_extension_editor_command_runner_arc(runner);
    }
    shell.set_extension_editor_host(terminal.external_editor_host(render_handle.clone()));
    shell.bind_footer_data_provider_with_render_handle(&footer_provider, render_handle.clone());
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, render_handle.clone());
    let status_handle = shell.status_handle_with_render_handle(render_handle.clone());
    let footer_state_handle = shell.footer_state_handle_with_render_handle(render_handle);
    install_interactive_submit_handler(
        &mut shell,
        created.core.clone(),
        created.core.model_registry(),
        interactive_scoped_models,
        interactive_session_manager,
        status_handle,
        footer_state_handle,
        Arc::clone(&exit_requested),
    );
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

pub fn finalize_system_prompt(prompt: impl Into<String>) -> String {
    let prompt = prompt.into();
    format!("{FINALIZED_SYSTEM_PROMPT_PREFIX}{prompt}")
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
    let session_support = match create_session_support(&parsed, &cwd, agent_dir.as_deref()) {
        Ok(session_support) => session_support,
        Err(error) => {
            push_line(&mut stderr, &format!("Error: {error}"));
            return RunCommandResult {
                exit_code: 1,
                stdout,
                stderr,
            };
        }
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

    let mut stream_options = stream_options;
    if let Some(session_support) = session_support.as_ref() {
        stream_options.session_id = Some(session_support.session_id.clone());
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
            existing_session: session_support
                .as_ref()
                .map(|session_support| session_support.existing_session.clone())
                .unwrap_or_default(),
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

    if let Some(session_support) = session_support.as_ref()
        && let Err(error) = apply_session_support(&created.core, session_support)
    {
        push_line(&mut stderr, &format!("Error: {error}"));
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    }

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

    let json_session_header = if print_mode == PrintOutputMode::Json {
        session_support
            .as_ref()
            .map(|session_support| session_header_json_line(&session_support.header))
    } else {
        None
    };

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

    if let Some(header) = json_session_header {
        stdout.push_str(&header);
    }
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
    if let Some(finalized_prompt) =
        default_system_prompt.strip_prefix(FINALIZED_SYSTEM_PROMPT_PREFIX)
    {
        return finalized_prompt.to_owned();
    }

    let mut system_prompt = resolve_prompt_input(override_system_prompt)
        .unwrap_or_else(|| default_system_prompt.to_string());

    if let Some(append_system_prompt) = resolve_prompt_input(append_system_prompt) {
        if !system_prompt.is_empty() && !append_system_prompt.is_empty() {
            system_prompt.push_str("\n\n");
        }
        system_prompt.push_str(&append_system_prompt);
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

fn build_interactive_slash_commands(
    model_registry: Arc<ModelRegistry>,
    scoped_models: Vec<ScopedModel>,
) -> Vec<SlashCommand> {
    #[derive(Clone)]
    struct ModelCommandItem {
        id: String,
        provider: String,
        value: String,
    }

    let model_registry_for_arguments = model_registry.clone();
    let scoped_models_for_arguments = scoped_models.clone();

    vec![
        SlashCommand {
            name: String::from("model"),
            description: Some(String::from("Select model")),
            argument_completions: Some(Arc::new(move |prefix| {
                let models = current_interactive_model_candidates(
                    model_registry_for_arguments.as_ref(),
                    &scoped_models_for_arguments,
                );
                if models.is_empty() {
                    return None;
                }

                let items = models
                    .into_iter()
                    .map(|model| ModelCommandItem {
                        value: format!("{}/{}", model.provider, model.id),
                        id: model.id,
                        provider: model.provider,
                    })
                    .collect::<Vec<_>>();

                let filtered = fuzzy_filter(&items, prefix, |item| {
                    Cow::Owned(format!("{} {}", item.id, item.provider))
                });
                if filtered.is_empty() {
                    return None;
                }

                Some(
                    filtered
                        .into_iter()
                        .map(|item| AutocompleteItem {
                            value: item.value.clone(),
                            label: item.id.clone(),
                            description: Some(item.provider.clone()),
                        })
                        .collect(),
                )
            })),
        },
        SlashCommand {
            name: String::from("quit"),
            description: Some(String::from("Quit pi")),
            argument_completions: None,
        },
    ]
}

fn current_interactive_model_candidates(
    model_registry: &ModelRegistry,
    scoped_models: &[ScopedModel],
) -> Vec<Model> {
    if !scoped_models.is_empty() {
        return scoped_models
            .iter()
            .map(|scoped_model| scoped_model.model.clone())
            .collect();
    }

    model_registry.get_available()
}

#[derive(Debug, Clone)]
struct InteractiveModelCycleResult {
    model: Model,
    thinking_level: ThinkingLevel,
}

fn install_interactive_submit_handler(
    shell: &mut StartupShellComponent,
    core: CodingAgentCore,
    model_registry: Arc<ModelRegistry>,
    scoped_models: Vec<ScopedModel>,
    session_manager: Option<Arc<Mutex<SessionManager>>>,
    status_handle: StatusHandle,
    footer_state_handle: FooterStateHandle,
    exit_requested: Arc<AtomicBool>,
) {
    let cycle_forward_core = core.clone();
    let cycle_forward_model_registry = Arc::clone(&model_registry);
    let cycle_forward_scoped_models = scoped_models.clone();
    let cycle_forward_session_manager = session_manager.clone();
    let cycle_forward_status_handle = status_handle.clone();
    let cycle_forward_footer_state_handle = footer_state_handle.clone();
    shell.on_action("app.model.cycleForward", move || {
        handle_interactive_model_cycle(
            "forward",
            &cycle_forward_core,
            cycle_forward_model_registry.as_ref(),
            &cycle_forward_scoped_models,
            cycle_forward_session_manager.as_ref(),
            &cycle_forward_status_handle,
            &cycle_forward_footer_state_handle,
        );
    });

    let cycle_backward_core = core.clone();
    let cycle_backward_model_registry = Arc::clone(&model_registry);
    let cycle_backward_scoped_models = scoped_models.clone();
    let cycle_backward_session_manager = session_manager.clone();
    let cycle_backward_status_handle = status_handle.clone();
    let cycle_backward_footer_state_handle = footer_state_handle.clone();
    shell.on_action("app.model.cycleBackward", move || {
        handle_interactive_model_cycle(
            "backward",
            &cycle_backward_core,
            cycle_backward_model_registry.as_ref(),
            &cycle_backward_scoped_models,
            cycle_backward_session_manager.as_ref(),
            &cycle_backward_status_handle,
            &cycle_backward_footer_state_handle,
        );
    });

    let action_core = core.clone();
    let action_model_registry = Arc::clone(&model_registry);
    let action_scoped_models = scoped_models.clone();
    let action_session_manager = session_manager.clone();
    let action_status_handle = status_handle.clone();
    let action_footer_state_handle = footer_state_handle.clone();
    shell.on_action_with_shell("app.model.select", move |shell| {
        if action_core.state().is_streaming {
            action_status_handle.set_message(
                "Model switching while a request is running is not supported in the Rust interactive CLI yet.",
            );
            return;
        }

        show_interactive_model_selector(
            shell,
            &action_core,
            action_model_registry.as_ref(),
            &action_scoped_models,
            action_session_manager.as_ref(),
            &action_status_handle,
            &action_footer_state_handle,
            None,
        );
    });

    shell.set_on_submit_with_shell(move |shell, value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }

        if handle_interactive_slash_command(
            shell,
            trimmed,
            &core,
            model_registry.as_ref(),
            &scoped_models,
            session_manager.as_ref(),
            &status_handle,
            &footer_state_handle,
            &exit_requested,
        ) {
            return;
        }

        status_handle.set_message("Working...");
        let core = core.clone();
        let status_handle = status_handle.clone();
        tokio::spawn(async move {
            if let Err(error) = core.prompt_text(value).await {
                status_handle.set_message(format!("Error: {error}"));
            }
        });
    });
}

fn handle_interactive_model_cycle(
    direction: &str,
    core: &CodingAgentCore,
    model_registry: &ModelRegistry,
    scoped_models: &[ScopedModel],
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    status_handle: &StatusHandle,
    footer_state_handle: &FooterStateHandle,
) {
    if core.state().is_streaming {
        status_handle.set_message(
            "Model switching while a request is running is not supported in the Rust interactive CLI yet.",
        );
        return;
    }

    match cycle_interactive_model(
        core,
        model_registry,
        scoped_models,
        session_manager,
        direction,
    ) {
        Ok(Some(result)) => {
            update_interactive_footer_state(footer_state_handle, core);
            let model_name = if result.model.name.is_empty() {
                result.model.id.as_str()
            } else {
                result.model.name.as_str()
            };
            let thinking_suffix =
                if result.model.reasoning && result.thinking_level != ThinkingLevel::Off {
                    format!(
                        " (thinking: {})",
                        thinking_level_label(result.thinking_level)
                    )
                } else {
                    String::new()
                };
            status_handle.set_message(format!("Switched to {model_name}{thinking_suffix}"));
        }
        Ok(None) => {
            let message = if scoped_models.is_empty() {
                "Only one model available"
            } else {
                "Only one model in scope"
            };
            status_handle.set_message(message);
        }
        Err(error) => status_handle.set_message(format!("Error: {error}")),
    }
}

fn handle_interactive_slash_command(
    shell: &mut StartupShellComponent,
    text: &str,
    core: &CodingAgentCore,
    model_registry: &ModelRegistry,
    scoped_models: &[ScopedModel],
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    status_handle: &StatusHandle,
    footer_state_handle: &FooterStateHandle,
    exit_requested: &Arc<AtomicBool>,
) -> bool {
    if text == "/quit" {
        exit_requested.store(true, Ordering::Relaxed);
        return true;
    }

    if text == "/model" || text.starts_with("/model ") {
        let search_term = text.strip_prefix("/model").unwrap_or_default().trim();

        if core.state().is_streaming {
            status_handle.set_message(
                "Model switching while a request is running is not supported in the Rust interactive CLI yet.",
            );
            return true;
        }

        let candidates = current_interactive_model_candidates(model_registry, scoped_models);
        if let Some(model) = (!search_term.is_empty())
            .then(|| find_exact_model_reference_match(search_term, &candidates))
            .flatten()
        {
            if let Err(error) = switch_interactive_model(core, &model, session_manager) {
                status_handle.set_message(format!("Error: {error}"));
                return true;
            }

            update_interactive_footer_state(footer_state_handle, core);
            status_handle.set_message(format!("Model: {}", core.state().model.id));
            return true;
        }

        show_interactive_model_selector(
            shell,
            core,
            model_registry,
            scoped_models,
            session_manager,
            status_handle,
            footer_state_handle,
            (!search_term.is_empty()).then_some(search_term),
        );
        return true;
    }

    false
}

fn show_interactive_model_selector(
    shell: &mut StartupShellComponent,
    core: &CodingAgentCore,
    model_registry: &ModelRegistry,
    scoped_models: &[ScopedModel],
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    status_handle: &StatusHandle,
    footer_state_handle: &FooterStateHandle,
    initial_search: Option<&str>,
) {
    let current_model = Some(core.state().model.clone());
    let models = current_interactive_model_candidates(model_registry, scoped_models);
    let core = core.clone();
    let session_manager = session_manager.cloned();
    let status_handle_for_select = status_handle.clone();
    let footer_state_handle_for_select = footer_state_handle.clone();

    shell.show_model_selector(
        current_model,
        models,
        initial_search,
        move |model| {
            if let Err(error) = switch_interactive_model(&core, &model, session_manager.as_ref()) {
                status_handle_for_select.set_message(format!("Error: {error}"));
                return;
            }

            update_interactive_footer_state(&footer_state_handle_for_select, &core);
            status_handle_for_select.set_message(format!("Model: {}", core.state().model.id));
        },
        || {},
    );
}

fn cycle_interactive_model(
    core: &CodingAgentCore,
    model_registry: &ModelRegistry,
    scoped_models: &[ScopedModel],
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    direction: &str,
) -> Result<Option<InteractiveModelCycleResult>, String> {
    if !scoped_models.is_empty() {
        let scoped_candidates = scoped_models
            .iter()
            .filter(|scoped_model| model_registry.has_configured_auth(&scoped_model.model))
            .collect::<Vec<_>>();
        if scoped_candidates.len() <= 1 {
            return Ok(None);
        }

        let current_model = core.state().model;
        let current_index = scoped_candidates
            .iter()
            .position(|scoped_model| {
                models_are_equal(Some(&scoped_model.model), Some(&current_model))
            })
            .unwrap_or(0);
        let next_index = match direction {
            "backward" => (current_index + scoped_candidates.len() - 1) % scoped_candidates.len(),
            _ => (current_index + 1) % scoped_candidates.len(),
        };
        let next = scoped_candidates[next_index];
        apply_interactive_model_state(core, &next.model, next.thinking_level, session_manager)?;

        let state = core.state();
        return Ok(Some(InteractiveModelCycleResult {
            model: state.model.clone(),
            thinking_level: state.thinking_level,
        }));
    }

    let available_models = model_registry.get_available();
    if available_models.len() <= 1 {
        return Ok(None);
    }

    let current_model = core.state().model;
    let current_index = available_models
        .iter()
        .position(|model| models_are_equal(Some(model), Some(&current_model)))
        .unwrap_or(0);
    let next_index = match direction {
        "backward" => (current_index + available_models.len() - 1) % available_models.len(),
        _ => (current_index + 1) % available_models.len(),
    };
    let next_model = available_models[next_index].clone();
    apply_interactive_model_state(core, &next_model, None, session_manager)?;

    let state = core.state();
    Ok(Some(InteractiveModelCycleResult {
        model: state.model.clone(),
        thinking_level: state.thinking_level,
    }))
}

fn update_interactive_footer_state(
    footer_state_handle: &FooterStateHandle,
    core: &CodingAgentCore,
) {
    let state = core.state();
    footer_state_handle.update(|footer_state| {
        footer_state.model = Some(state.model.clone());
        footer_state.context_window = state.model.context_window;
        footer_state.thinking_level = thinking_level_label(state.thinking_level).to_owned();
    });
}

fn apply_interactive_model_state(
    core: &CodingAgentCore,
    model: &Model,
    thinking_level_override: Option<ThinkingLevel>,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
) -> Result<(), String> {
    if !core.model_registry().has_configured_auth(model) {
        return Err(format!("No API key for {}/{}", model.provider, model.id));
    }

    let state = core.state();
    let next_thinking_level = clamp_interactive_thinking_level(
        thinking_level_override.unwrap_or(state.thinking_level),
        model,
    );

    if let Some(session_manager) = session_manager {
        let mut session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        if !models_are_equal(Some(model), Some(&state.model)) {
            session_manager
                .append_model_change(model.provider.clone(), model.id.clone())
                .map_err(|error| error.to_string())?;
        }
        if next_thinking_level != state.thinking_level {
            session_manager
                .append_thinking_level_change(thinking_level_label(next_thinking_level))
                .map_err(|error| error.to_string())?;
        }
    }

    let next_model = model.clone();
    core.agent().update_state(move |state| {
        state.model = next_model.clone();
        state.thinking_level = next_thinking_level;
    });
    Ok(())
}

fn switch_interactive_model(
    core: &CodingAgentCore,
    model: &Model,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
) -> Result<(), String> {
    apply_interactive_model_state(core, model, None, session_manager)
}

fn clamp_interactive_thinking_level(level: ThinkingLevel, model: &Model) -> ThinkingLevel {
    if !model.reasoning {
        return ThinkingLevel::Off;
    }

    if level == ThinkingLevel::XHigh && !supports_xhigh(model) {
        return ThinkingLevel::High;
    }

    level
}

fn thinking_level_label(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
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
    if parsed.resume {
        return Some(String::from(
            "--resume is not supported in the Rust CLI yet. Use --continue or --session <path>.",
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
        "  - interactive mode",
        "  - --provider, --model, --models, --api-key, --system-prompt, --append-system-prompt, --thinking",
        "  - --continue, --session, --fork, --no-session, --session-dir",
        "  - --list-models [search]",
        "  - @file text/image preprocessing",
        "",
        "Not yet supported:",
        "  - --resume session picker",
        "  - rpc mode",
        "  - export",
    ]
    .join("\n")
}

fn push_line(buffer: &mut String, line: &str) {
    buffer.push_str(line);
    buffer.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::{
        FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions, register_faux_provider,
    };
    use pi_coding_agent_core::MemoryAuthStorage;
    use std::{
        fs, io,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::time::timeout;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pi-coding-agent-cli-{prefix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn strip_terminal_control_sequences(output: &str) -> String {
        let mut result = String::new();
        let bytes = output.as_bytes();
        let mut index = 0usize;

        while index < bytes.len() {
            if bytes[index] == 0x1b {
                match bytes.get(index + 1).copied() {
                    Some(b'[') => {
                        index += 2;
                        while index < bytes.len() {
                            let byte = bytes[index];
                            index += 1;
                            if (0x40..=0x7e).contains(&byte) {
                                break;
                            }
                        }
                        continue;
                    }
                    Some(b']') | Some(b'_') => {
                        index += 2;
                        while index < bytes.len() {
                            if bytes[index] == 0x07 {
                                index += 1;
                                break;
                            }
                            if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                                index += 2;
                                break;
                            }
                            index += 1;
                        }
                        continue;
                    }
                    _ => {
                        index += 1;
                        continue;
                    }
                }
            }

            let character = output[index..]
                .chars()
                .next()
                .expect("terminal output should contain a character");
            index += character.len_utf8();

            if character == '\r'
                || (character.is_control() && character != '\n' && character != '\t')
            {
                continue;
            }

            result.push(character);
        }

        result
    }

    #[test]
    fn resolve_system_prompt_reads_file_inputs_and_uses_blank_line_separator() {
        let temp_dir = unique_temp_dir("resolve-system-prompt");
        let prompt_path = temp_dir.join("SYSTEM.md");
        let append_path = temp_dir.join("APPEND_SYSTEM.md");
        fs::write(&prompt_path, "system from file\n").unwrap();
        fs::write(&append_path, "append from file\n").unwrap();

        let resolved = resolve_system_prompt(
            "default prompt",
            Some(prompt_path.to_str().unwrap()),
            Some(append_path.to_str().unwrap()),
        );

        assert_eq!(resolved, "system from file\n\n\nappend from file\n");
    }

    #[test]
    fn resolve_system_prompt_returns_finalized_prompts_without_reapplying_args() {
        let finalized = finalize_system_prompt("final prompt");
        let resolved = resolve_system_prompt(&finalized, Some("override"), Some("append"));

        assert_eq!(resolved, "final prompt");
    }

    #[derive(Clone)]
    struct LifecycleScriptedTerminal {
        state: Arc<LifecycleScriptedTerminalState>,
    }

    struct LifecycleScriptedTerminalState {
        writes: Mutex<Vec<String>>,
        input_handler: Mutex<Option<SharedInputHandler>>,
        resize_handler: Mutex<Option<SharedResizeHandler>>,
        active: AtomicBool,
        start_count: AtomicUsize,
        stop_count: AtomicUsize,
    }

    impl LifecycleScriptedTerminal {
        fn new(script: Vec<(Duration, String)>) -> Self {
            let state = Arc::new(LifecycleScriptedTerminalState {
                writes: Mutex::new(Vec::new()),
                input_handler: Mutex::new(None),
                resize_handler: Mutex::new(None),
                active: AtomicBool::new(false),
                start_count: AtomicUsize::new(0),
                stop_count: AtomicUsize::new(0),
            });

            let script_state = Arc::clone(&state);
            thread::spawn(move || {
                for (delay, data) in script {
                    thread::sleep(delay);
                    loop {
                        if script_state.active.load(Ordering::Relaxed) {
                            let handler = script_state
                                .input_handler
                                .lock()
                                .expect("scripted terminal input handler mutex poisoned")
                                .clone();
                            if let Some(handler) = handler {
                                let mut callback = handler
                                    .lock()
                                    .expect("scripted terminal input callback mutex poisoned");
                                (callback)(data.clone());
                                break;
                            }
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            });

            Self { state }
        }

        fn output(&self) -> String {
            self.state
                .writes
                .lock()
                .expect("scripted terminal writes mutex poisoned")
                .join("")
        }

        fn start_count(&self) -> usize {
            self.state.start_count.load(Ordering::Relaxed)
        }

        fn stop_count(&self) -> usize {
            self.state.stop_count.load(Ordering::Relaxed)
        }
    }

    impl Terminal for LifecycleScriptedTerminal {
        fn start(
            &mut self,
            on_input: Box<dyn FnMut(String) + Send>,
            on_resize: Box<dyn FnMut() + Send>,
        ) -> Result<(), TuiError> {
            *self
                .state
                .input_handler
                .lock()
                .expect("scripted terminal input handler mutex poisoned") =
                Some(Arc::new(Mutex::new(on_input)));
            *self
                .state
                .resize_handler
                .lock()
                .expect("scripted terminal resize handler mutex poisoned") =
                Some(Arc::new(Mutex::new(on_resize)));
            self.state.active.store(true, Ordering::Relaxed);
            self.state.start_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn stop(&mut self) -> Result<(), TuiError> {
            self.state.active.store(false, Ordering::Relaxed);
            self.state.stop_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn drain_input(&mut self, _max: Duration, _idle: Duration) -> Result<(), TuiError> {
            Ok(())
        }

        fn write(&mut self, data: &str) -> Result<(), TuiError> {
            self.state
                .writes
                .lock()
                .expect("scripted terminal writes mutex poisoned")
                .push(data.to_owned());
            Ok(())
        }

        fn columns(&self) -> u16 {
            100
        }

        fn rows(&self) -> u16 {
            12
        }

        fn kitty_protocol_active(&self) -> bool {
            false
        }

        fn move_by(&mut self, _lines: i32) -> Result<(), TuiError> {
            Ok(())
        }

        fn hide_cursor(&mut self) -> Result<(), TuiError> {
            Ok(())
        }

        fn show_cursor(&mut self) -> Result<(), TuiError> {
            Ok(())
        }

        fn clear_line(&mut self) -> Result<(), TuiError> {
            Ok(())
        }

        fn clear_from_cursor(&mut self) -> Result<(), TuiError> {
            Ok(())
        }

        fn clear_screen(&mut self) -> Result<(), TuiError> {
            Ok(())
        }

        fn set_title(&mut self, _title: &str) -> Result<(), TuiError> {
            Ok(())
        }
    }

    struct ReplacingExternalEditorRunner {
        replacement: String,
    }

    impl ReplacingExternalEditorRunner {
        fn new(replacement: &str) -> Self {
            Self {
                replacement: replacement.to_owned(),
            }
        }
    }

    impl ExternalEditorCommandRunner for ReplacingExternalEditorRunner {
        fn run(&self, _command: &str, file_path: &Path) -> io::Result<Option<i32>> {
            fs::write(file_path, &self.replacement)?;
            Ok(Some(0))
        }
    }

    #[tokio::test]
    async fn interactive_runtime_external_editor_host_stops_and_restarts_live_terminal() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "interactive-external-editor-faux".into(),
            models: vec![FauxModelDefinition {
                id: "interactive-external-editor-faux-1".into(),
                name: Some("Interactive External Editor Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        faux.set_responses(vec![FauxResponse::text("Edited prompt received")]);
        let model = faux
            .get_model(Some("interactive-external-editor-faux-1"))
            .expect("expected faux model");

        let terminal = LifecycleScriptedTerminal::new(vec![
            (Duration::from_millis(5), String::from("h")),
            (Duration::from_millis(5), String::from("i")),
            (Duration::from_millis(5), String::from("\x07")),
            (Duration::from_millis(25), String::from("\x07")),
            (Duration::from_millis(25), String::from("\r")),
            (Duration::from_millis(25), String::from("\r")),
            (Duration::from_millis(80), String::from("\x04")),
        ]);
        let inspector = terminal.clone();

        let exit_code = timeout(
            Duration::from_secs(3),
            run_interactive_command_with_runtime(
                RunCommandOptions {
                    args: vec![
                        String::from("--provider"),
                        model.provider.clone(),
                        String::from("--model"),
                        model.id.clone(),
                    ],
                    stdin_is_tty: true,
                    stdin_content: None,
                    auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                        model.provider.as_str(),
                        "token",
                    )])),
                    built_in_models: vec![model],
                    models_json_path: None,
                    agent_dir: None,
                    cwd: unique_temp_dir("runner-interactive-external-editor"),
                    default_system_prompt: String::new(),
                    version: String::from("0.1.0"),
                    stream_options: StreamOptions::default(),
                },
                InteractiveRuntime {
                    terminal_factory: Arc::new(move || Box::new(terminal.clone())),
                    extension_editor_command: Some(String::from("mock-editor --wait")),
                    extension_editor_runner: Some(Arc::new(ReplacingExternalEditorRunner::new(
                        "edited from external\n",
                    ))),
                },
            ),
        )
        .await
        .expect("interactive runner should complete");

        assert_eq!(exit_code, 0);
        let output = strip_terminal_control_sequences(&inspector.output());
        assert!(output.contains("Edit message"), "output: {output}");
        assert!(output.contains("edited from external"), "output: {output}");
        assert!(
            output.contains("Edited prompt received"),
            "output: {output}"
        );
        assert_eq!(inspector.start_count(), 2, "output: {output}");
        assert_eq!(inspector.stop_count(), 2, "output: {output}");

        faux.unregister();
    }
}
