use crate::{
    AppMode, Args, Diagnostic, DiagnosticKind, ListModels, OverlayAuthSource, PrintModeOptions,
    PrintOutputMode, ProcessFileOptions, build_initial_message, list_models::render_list_models,
    parse_args, process_file_arguments, resolve_app_mode, run_print_mode,
    session_picker::SessionPickerComponent, to_print_output_mode,
};
use pi_agent::ThinkingLevel;
use pi_ai::{StreamOptions, ThinkingBudgets, models_are_equal, supports_xhigh};
use pi_coding_agent_core::{
    AuthSource, BootstrapDiagnosticLevel, CodingAgentCore, CodingAgentCoreError,
    CodingAgentCoreOptions, CustomMessage, CustomMessageContent, ExistingSessionSelection,
    FooterDataProvider, ModelRegistry, NewSessionOptions, ScopedModel, SessionBootstrapOptions,
    SessionEntry, SessionHeader, SessionInfo, SessionManager, create_coding_agent_core,
    find_exact_model_reference_match, get_default_session_dir, parse_thinking_level,
    resolve_cli_model, resolve_model_scope, resolve_prompt_input, restore_model_from_session,
};
use pi_coding_agent_tui::{
    CustomMessageComponent, DEFAULT_APP_KEYBINDINGS, ExternalEditorCommandRunner,
    ExternalEditorHost, FooterStateHandle, InteractiveCoreBinding, KeybindingsManager,
    PlainKeyHintStyler, StartupShellComponent, StatusHandle,
};
use pi_config::{LoadedRuntimeSettings, ThinkingBudgetsSettings, load_runtime_settings};
use pi_events::{AssistantContent, Message, Model, UserContent};
use pi_tui::{
    AutocompleteItem, CombinedAutocompleteProvider, Component, ProcessTerminal, RenderHandle,
    SlashCommand, Terminal, Tui, TuiError, fuzzy_filter, matches_key, truncate_to_width,
};
use std::{
    borrow::Cow,
    cell::Cell,
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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

#[derive(Clone)]
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

#[derive(Debug, Clone)]
enum InteractiveTransitionRequest {
    NewSession,
    ResumePicker,
    ForkPicker,
}

#[derive(Debug, Clone)]
struct BootstrapDefaults {
    provider: String,
    model_id: String,
    thinking_level: ThinkingLevel,
}

impl BootstrapDefaults {
    fn from_model(model: &Model, thinking_level: ThinkingLevel) -> Self {
        Self {
            provider: model.provider.clone(),
            model_id: model.id.clone(),
            thinking_level,
        }
    }
}

#[derive(Clone)]
struct InteractiveSlashCommandContext {
    keybindings: KeybindingsManager,
    runtime_settings: LoadedRuntimeSettings,
    cwd: PathBuf,
    agent_dir: Option<PathBuf>,
}

struct InteractiveIterationOptions {
    parsed: Args,
    stdin_is_tty: bool,
    stdin_content: Option<String>,
    auth_source: Arc<dyn AuthSource>,
    built_in_models: Vec<Model>,
    models_json_path: Option<PathBuf>,
    agent_dir: Option<PathBuf>,
    cwd: PathBuf,
    default_system_prompt: String,
    version: String,
    stream_options: StreamOptions,
    runtime: InteractiveRuntime,
    manager_override: Option<SessionManager>,
    show_resume_picker: bool,
    prefill_input: Option<String>,
    initial_status_message: Option<String>,
    bootstrap_defaults: Option<BootstrapDefaults>,
}

struct InteractiveIterationOutcome {
    exit_code: i32,
    transition: Option<InteractiveTransitionRequest>,
    session_context: Option<InteractiveSessionContext>,
}

struct InteractiveSessionContext {
    manager: Option<SessionManager>,
    session_file: Option<String>,
    session_dir: Option<String>,
    cwd: String,
    model: Model,
    thinking_level: ThinkingLevel,
}

struct InteractiveTransitionPlan {
    manager: Option<SessionManager>,
    cwd: PathBuf,
    prefill_input: Option<String>,
    initial_status_message: Option<String>,
    bootstrap_defaults: Option<BootstrapDefaults>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ForkMessageCandidate {
    entry_id: String,
    parent_id: Option<String>,
    text: String,
}

fn create_session_support(
    parsed: &Args,
    cwd: &Path,
    agent_dir: Option<&Path>,
    resume_session_path: Option<&str>,
    manager_override: Option<SessionManager>,
) -> Result<Option<SessionSupport>, String> {
    if let Some(session_manager) = manager_override {
        return Ok(Some(build_session_support(session_manager)));
    }

    let should_use_session_manager = parsed.no_session
        || parsed.continue_session
        || parsed.session.is_some()
        || parsed.fork.is_some()
        || parsed.session_dir.is_some()
        || resume_session_path.is_some()
        || agent_dir.is_some();
    if !should_use_session_manager {
        return Ok(None);
    }

    let cwd_string = cwd.to_string_lossy().into_owned();
    let session_dir = resolve_session_dir(parsed, cwd, agent_dir);
    let session_manager = if let Some(session) = parsed.session.as_deref() {
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
    } else if let Some(resume_session_path) = resume_session_path {
        SessionManager::open(
            &resolve_session_path(cwd, resume_session_path),
            session_dir.as_deref(),
            None,
        )
        .map_err(|error| error.to_string())?
    } else if parsed.no_session {
        SessionManager::in_memory(&cwd_string)
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

#[derive(Debug, Clone)]
enum ResumePickerOutcome {
    Selected(String),
    Cancelled,
}

async fn select_resume_session(
    tui: &mut Tui<LiveInteractiveTerminal>,
    keybindings: &KeybindingsManager,
    current_sessions: Vec<SessionInfo>,
    all_sessions: Vec<SessionInfo>,
) -> Result<Option<String>, String> {
    let outcome = Arc::new(Mutex::new(None::<ResumePickerOutcome>));
    let mut picker = SessionPickerComponent::new(keybindings, current_sessions, all_sessions);

    let outcome_for_select = Arc::clone(&outcome);
    picker.set_on_select(move |path| {
        *outcome_for_select
            .lock()
            .expect("resume picker outcome mutex poisoned") =
            Some(ResumePickerOutcome::Selected(path));
    });

    let outcome_for_cancel = Arc::clone(&outcome);
    picker.set_on_cancel(move || {
        *outcome_for_cancel
            .lock()
            .expect("resume picker outcome mutex poisoned") = Some(ResumePickerOutcome::Cancelled);
    });

    let picker_id = tui.add_child(Box::new(picker));
    let _ = tui.set_focus_child(picker_id);
    tui.start().map_err(|error| error.to_string())?;

    loop {
        if let Some(outcome) = outcome
            .lock()
            .expect("resume picker outcome mutex poisoned")
            .take()
        {
            tui.clear();
            return Ok(match outcome {
                ResumePickerOutcome::Selected(path) => Some(path),
                ResumePickerOutcome::Cancelled => None,
            });
        }

        tui.drain_terminal_events()
            .map_err(|error| error.to_string())?;
        sleep(Duration::from_millis(16)).await;
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

    let mut current_cwd = cwd;
    let mut use_initial_input = true;
    let mut manager_override = None;
    let mut prefill_input = None;
    let mut initial_status_message = None;
    let mut bootstrap_defaults = None;
    let initial_show_resume_picker = parsed.resume
        && !parsed.no_session
        && parsed.session.is_none()
        && parsed.fork.is_none()
        && !parsed.continue_session;

    loop {
        let parsed_for_iteration = if use_initial_input {
            parsed.clone()
        } else {
            sanitize_interactive_follow_up_args(&parsed)
        };
        let stdin_content_for_iteration = if use_initial_input {
            stdin_content.clone()
        } else {
            None
        };

        let outcome = run_interactive_iteration(InteractiveIterationOptions {
            parsed: parsed_for_iteration,
            stdin_is_tty,
            stdin_content: stdin_content_for_iteration,
            auth_source: auth_source.clone(),
            built_in_models: built_in_models.clone(),
            models_json_path: models_json_path.clone(),
            agent_dir: agent_dir.clone(),
            cwd: current_cwd.clone(),
            default_system_prompt: default_system_prompt.clone(),
            version: version.clone(),
            stream_options: stream_options.clone(),
            runtime: runtime.clone(),
            manager_override: manager_override.take(),
            show_resume_picker: use_initial_input && initial_show_resume_picker,
            prefill_input: prefill_input.take(),
            initial_status_message: initial_status_message.take(),
            bootstrap_defaults: bootstrap_defaults.take(),
        })
        .await;

        let Some(transition) = outcome.transition else {
            return outcome.exit_code;
        };

        let plan = match resolve_interactive_transition(
            transition,
            outcome.session_context,
            &current_cwd,
            agent_dir.as_deref(),
            &runtime,
        )
        .await
        {
            Ok(plan) => plan,
            Err(error) => {
                eprintln!("Error: {error}");
                return 1;
            }
        };

        current_cwd = plan.cwd;
        manager_override = plan.manager;
        prefill_input = plan.prefill_input;
        initial_status_message = plan.initial_status_message;
        bootstrap_defaults = plan.bootstrap_defaults;
        use_initial_input = false;
    }
}

async fn run_interactive_iteration(
    options: InteractiveIterationOptions,
) -> InteractiveIterationOutcome {
    let InteractiveIterationOptions {
        parsed,
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
        runtime,
        manager_override,
        show_resume_picker,
        prefill_input,
        initial_status_message,
        bootstrap_defaults,
    } = options;

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

    let keybindings = create_keybindings_manager(agent_dir.as_deref());
    let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
    let mut tui = Tui::new(terminal.clone());
    let mut resume_picker_was_shown = false;
    let selected_resume_session = if show_resume_picker {
        let cwd_string = cwd.to_string_lossy().into_owned();
        let session_dir = resolve_session_dir(&parsed, &cwd, agent_dir.as_deref());
        let agent_dir_string = agent_dir
            .as_deref()
            .map(|agent_dir| agent_dir.to_string_lossy().into_owned());
        let current_sessions = SessionManager::list(&cwd_string, session_dir.as_deref());
        let all_sessions = SessionManager::list_all(agent_dir_string.as_deref());
        resume_picker_was_shown = true;

        match select_resume_session(&mut tui, &keybindings, current_sessions, all_sessions).await {
            Ok(Some(path)) => Some(path),
            Ok(None) => {
                let _ = tui.stop();
                println!("No session selected");
                return InteractiveIterationOutcome {
                    exit_code: 0,
                    transition: None,
                    session_context: None,
                };
            }
            Err(error) => {
                let _ = tui.stop();
                eprintln!("Error: {error}");
                return InteractiveIterationOutcome {
                    exit_code: 1,
                    transition: None,
                    session_context: None,
                };
            }
        }
    } else {
        None
    };

    let session_support = match create_session_support(
        &parsed,
        &cwd,
        agent_dir.as_deref(),
        selected_resume_session.as_deref(),
        manager_override,
    ) {
        Ok(session_support) => session_support,
        Err(error) => {
            let _ = tui.stop();
            eprintln!("Error: {error}");
            return InteractiveIterationOutcome {
                exit_code: 1,
                transition: None,
                session_context: None,
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
            let _ = tui.stop();
            eprintln!("Error: {error}");
            return InteractiveIterationOutcome {
                exit_code: 1,
                transition: None,
                session_context: None,
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
        let _ = tui.stop();
        eprintln!("Error: {error}");
        return InteractiveIterationOutcome {
            exit_code: 1,
            transition: None,
            session_context: None,
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
            default_provider: bootstrap_defaults
                .as_ref()
                .map(|defaults| defaults.provider.clone()),
            default_model_id: bootstrap_defaults
                .as_ref()
                .map(|defaults| defaults.model_id.clone()),
            default_thinking_level: bootstrap_defaults
                .as_ref()
                .map(|defaults| defaults.thinking_level),
            existing_session: session_support
                .as_ref()
                .map(|session_support| session_support.existing_session.clone())
                .unwrap_or_default(),
        },
        stream_options,
    });

    let created = match created {
        Ok(created) => created,
        Err(CodingAgentCoreError::NoModelAvailable) => {
            let _ = tui.stop();
            eprint!("{}", render_no_models_message(models_json_path.as_deref()));
            return InteractiveIterationOutcome {
                exit_code: 1,
                transition: None,
                session_context: None,
            };
        }
        Err(error) => {
            let _ = tui.stop();
            eprintln!("Error: {error}");
            return InteractiveIterationOutcome {
                exit_code: 1,
                transition: None,
                session_context: None,
            };
        }
    };

    if let Some(session_support) = session_support.as_ref()
        && let Err(error) = apply_session_support(&created.core, session_support)
    {
        let _ = tui.stop();
        eprintln!("Error: {error}");
        return InteractiveIterationOutcome {
            exit_code: 1,
            transition: None,
            session_context: None,
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

    let bootstrap_output = render_bootstrap_diagnostics(&created.diagnostics);
    if !bootstrap_output.is_empty() {
        eprint!("{bootstrap_output}");
    }
    if created
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == BootstrapDiagnosticLevel::Error)
    {
        let _ = tui.stop();
        return InteractiveIterationOutcome {
            exit_code: 1,
            transition: None,
            session_context: None,
        };
    }

    let interactive_session_manager = session_support
        .as_ref()
        .map(|support| support.manager.clone());

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
    if let Some(prefill_input) = prefill_input {
        shell.set_input_value(prefill_input.clone());
        shell.set_input_cursor(prefill_input.len());
    }
    if let Some(initial_status_message) = initial_status_message {
        shell.set_status_message(initial_status_message);
    }

    let exit_requested = Arc::new(AtomicBool::new(false));
    let exit_requested_for_shell = Arc::clone(&exit_requested);
    shell.set_on_exit(move || {
        exit_requested_for_shell.store(true, Ordering::Relaxed);
    });

    let transition_request = Arc::new(Mutex::new(None::<InteractiveTransitionRequest>));
    let slash_command_context = InteractiveSlashCommandContext {
        keybindings: keybindings.clone(),
        runtime_settings: runtime_settings.clone(),
        cwd: cwd.clone(),
        agent_dir: agent_dir.clone(),
    };
    let footer_provider = FooterDataProvider::new(&cwd);
    let render_handle = tui.render_handle();
    if let Some(command) = runtime.extension_editor_command.clone() {
        shell.set_extension_editor_command(command);
    }
    if let Some(runner) = runtime.extension_editor_runner.clone() {
        shell.set_extension_editor_command_runner_arc(runner);
    }
    shell.set_extension_editor_host(terminal.external_editor_host(render_handle.clone()));
    shell.bind_footer_data_provider_with_render_handle(&footer_provider, render_handle.clone());
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, render_handle.clone());
    let status_handle = shell.status_handle_with_render_handle(render_handle.clone());
    let footer_state_handle = shell.footer_state_handle_with_render_handle(render_handle);
    update_interactive_footer_state(
        &footer_state_handle,
        &created.core,
        interactive_session_manager.as_ref(),
    );
    install_interactive_submit_handler(
        &mut shell,
        created.core.clone(),
        created.core.model_registry(),
        interactive_scoped_models,
        interactive_session_manager,
        slash_command_context,
        status_handle,
        footer_state_handle,
        Arc::clone(&exit_requested),
        Arc::clone(&transition_request),
    );
    let shell_id = tui.add_child(Box::new(shell));
    let _ = tui.set_focus_child(shell_id);
    let _ = tui.terminal_mut().set_title("pi");

    if let Err(error) = tui.start() {
        let _ = tui.stop();
        eprintln!("Error: {error}");
        drop(binding);
        return InteractiveIterationOutcome {
            exit_code: 1,
            transition: None,
            session_context: None,
        };
    }
    if resume_picker_was_shown {
        let _ = tui.request_render(false);
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
    let transition = transition_request
        .lock()
        .expect("interactive transition request mutex poisoned")
        .take();
    let final_state = created.core.state();
    let _ = tui
        .terminal_mut()
        .drain_input(Duration::from_millis(1000), Duration::from_millis(50));
    let _ = tui.stop();
    drop(binding);
    drop(tui);
    drop(created);

    let session_context = if transition.is_some() {
        build_interactive_session_context(session_support, &final_state)
    } else {
        None
    };

    InteractiveIterationOutcome {
        exit_code,
        transition,
        session_context,
    }
}

fn create_keybindings_manager(agent_dir: Option<&Path>) -> KeybindingsManager {
    let mut keybindings = match agent_dir {
        Some(agent_dir) => KeybindingsManager::create(agent_dir),
        None => KeybindingsManager::new(BTreeMap::new(), None),
    };
    keybindings.reload();
    keybindings
}

fn sanitize_interactive_follow_up_args(parsed: &Args) -> Args {
    let mut parsed = parsed.clone();
    parsed.continue_session = false;
    parsed.resume = false;
    parsed.session = None;
    parsed.fork = None;
    parsed.messages.clear();
    parsed.file_args.clear();
    parsed
}

fn build_interactive_session_context(
    session_support: Option<SessionSupport>,
    state: &pi_agent::AgentState,
) -> Option<InteractiveSessionContext> {
    let session_support = session_support?;
    let session_file;
    let session_dir;
    let cwd;
    {
        let session_manager = session_support
            .manager
            .lock()
            .expect("session manager mutex poisoned");
        session_file = session_manager.get_session_file().map(str::to_owned);
        session_dir = (!session_manager.get_session_dir().is_empty())
            .then(|| session_manager.get_session_dir().to_owned());
        cwd = session_manager.get_cwd().to_owned();
    }

    let manager = Arc::try_unwrap(session_support.manager)
        .ok()
        .and_then(|manager| manager.into_inner().ok());

    Some(InteractiveSessionContext {
        manager,
        session_file,
        session_dir,
        cwd,
        model: state.model.clone(),
        thinking_level: state.thinking_level,
    })
}

fn restore_session_manager(context: InteractiveSessionContext) -> Result<SessionManager, String> {
    if let Some(manager) = context.manager {
        return Ok(manager);
    }

    if let Some(session_file) = context.session_file {
        return SessionManager::open(&session_file, context.session_dir.as_deref(), None)
            .map_err(|error| error.to_string());
    }

    Ok(SessionManager::in_memory(&context.cwd))
}

async fn resolve_interactive_transition(
    transition: InteractiveTransitionRequest,
    session_context: Option<InteractiveSessionContext>,
    current_cwd: &Path,
    agent_dir: Option<&Path>,
    runtime: &InteractiveRuntime,
) -> Result<InteractiveTransitionPlan, String> {
    match transition {
        InteractiveTransitionRequest::NewSession => {
            let defaults = session_context.as_ref().map(|context| {
                BootstrapDefaults::from_model(&context.model, context.thinking_level)
            });
            let mut manager = match session_context {
                Some(context) => restore_session_manager(context)?,
                None => SessionManager::in_memory(&current_cwd.to_string_lossy()),
            };
            manager.new_session(NewSessionOptions::default());
            Ok(InteractiveTransitionPlan {
                cwd: PathBuf::from(manager.get_cwd()),
                manager: Some(manager),
                prefill_input: None,
                initial_status_message: Some(String::from("New session started")),
                bootstrap_defaults: defaults,
            })
        }
        InteractiveTransitionRequest::ResumePicker => {
            let current_context = session_context;
            let current_cwd_string = current_context
                .as_ref()
                .map(|context| context.cwd.clone())
                .unwrap_or_else(|| current_cwd.to_string_lossy().into_owned());
            let session_dir = current_context
                .as_ref()
                .and_then(|context| context.session_dir.clone());
            let current_sessions =
                SessionManager::list(&current_cwd_string, session_dir.as_deref());
            let agent_dir_string =
                agent_dir.map(|agent_dir| agent_dir.to_string_lossy().into_owned());
            let all_sessions = SessionManager::list_all(agent_dir_string.as_deref());
            let keybindings = create_keybindings_manager(agent_dir);
            let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
            let mut tui = Tui::new(terminal);

            match select_resume_session(&mut tui, &keybindings, current_sessions, all_sessions)
                .await?
            {
                Some(path) => {
                    let manager = SessionManager::open(&path, None, None)
                        .map_err(|error| error.to_string())?;
                    Ok(InteractiveTransitionPlan {
                        cwd: PathBuf::from(manager.get_cwd()),
                        manager: Some(manager),
                        prefill_input: None,
                        initial_status_message: Some(String::from("Resumed session")),
                        bootstrap_defaults: None,
                    })
                }
                None => {
                    let (manager, cwd) = match current_context {
                        Some(context) => {
                            let cwd = PathBuf::from(&context.cwd);
                            (Some(restore_session_manager(context)?), cwd)
                        }
                        None => (None, current_cwd.to_path_buf()),
                    };
                    Ok(InteractiveTransitionPlan {
                        cwd,
                        manager,
                        prefill_input: None,
                        initial_status_message: None,
                        bootstrap_defaults: None,
                    })
                }
            }
        }
        InteractiveTransitionRequest::ForkPicker => {
            let session_context =
                session_context.ok_or_else(|| String::from("Session data unavailable"))?;
            let defaults = BootstrapDefaults::from_model(
                &session_context.model,
                session_context.thinking_level,
            );
            let mut manager = restore_session_manager(session_context)?;
            let candidates = collect_fork_candidates(&manager);
            if candidates.is_empty() {
                return Ok(InteractiveTransitionPlan {
                    cwd: PathBuf::from(manager.get_cwd()),
                    manager: Some(manager),
                    prefill_input: None,
                    initial_status_message: Some(String::from("No messages to fork from")),
                    bootstrap_defaults: None,
                });
            }

            let keybindings = create_keybindings_manager(agent_dir);
            let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
            let mut tui = Tui::new(terminal);
            let selected_entry_id =
                match select_fork_message(&mut tui, &keybindings, candidates.clone()).await? {
                    Some(entry_id) => entry_id,
                    None => {
                        return Ok(InteractiveTransitionPlan {
                            cwd: PathBuf::from(manager.get_cwd()),
                            manager: Some(manager),
                            prefill_input: None,
                            initial_status_message: None,
                            bootstrap_defaults: None,
                        });
                    }
                };

            let selected = candidates
                .into_iter()
                .find(|candidate| candidate.entry_id == selected_entry_id)
                .ok_or_else(|| String::from("Fork selection is no longer available"))?;

            let bootstrap_defaults = if let Some(parent_id) = selected.parent_id.as_deref() {
                manager
                    .create_branched_session(parent_id)
                    .map_err(|error| error.to_string())?;
                None
            } else {
                manager.new_session(NewSessionOptions {
                    id: None,
                    parent_session: manager.get_session_file().map(ToOwned::to_owned),
                });
                Some(defaults)
            };

            Ok(InteractiveTransitionPlan {
                cwd: PathBuf::from(manager.get_cwd()),
                manager: Some(manager),
                prefill_input: Some(selected.text),
                initial_status_message: Some(String::from("Branched to new session")),
                bootstrap_defaults,
            })
        }
    }
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

    if parsed.resume {
        push_line(
            &mut stderr,
            "--resume session picker is only supported in interactive mode in the Rust CLI",
        );
        return RunCommandResult {
            exit_code: 1,
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
    let session_support =
        match create_session_support(&parsed, &cwd, agent_dir.as_deref(), None, None) {
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
        simple_slash_command("settings", "Open settings menu"),
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
        simple_slash_command("scoped-models", "Show scoped models for Ctrl+P cycling"),
        simple_slash_command("export", "Export session as JSONL"),
        simple_slash_command("share", "Share session as a secret GitHub gist"),
        simple_slash_command("copy", "Copy last assistant message to clipboard"),
        simple_slash_command("name", "Set session display name"),
        simple_slash_command("session", "Show session info and stats"),
        simple_slash_command("changelog", "Show changelog entries"),
        simple_slash_command("hotkeys", "Show keyboard shortcuts"),
        simple_slash_command("fork", "Fork from a previous user message"),
        simple_slash_command("tree", "Show or switch the session tree"),
        simple_slash_command("login", "Login with OAuth provider"),
        simple_slash_command("logout", "Logout from OAuth provider"),
        simple_slash_command("new", "Start a new session"),
        simple_slash_command("compact", "Compact the current session context"),
        simple_slash_command("resume", "Resume a different session"),
        simple_slash_command("reload", "Reload keybindings and resources"),
        SlashCommand {
            name: String::from("quit"),
            description: Some(String::from("Quit pi")),
            argument_completions: None,
        },
    ]
}

fn simple_slash_command(name: &str, description: &str) -> SlashCommand {
    SlashCommand {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        argument_completions: None,
    }
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

type ForkSelectCallback = Box<dyn FnMut(String) + Send + 'static>;
type ForkCancelCallback = Box<dyn FnMut() + Send + 'static>;

#[derive(Debug, Clone)]
enum ForkPickerOutcome {
    Selected(String),
    Cancelled,
}

struct ForkMessagePickerComponent {
    keybindings: KeybindingsManager,
    candidates: Vec<ForkMessageCandidate>,
    selected_index: usize,
    on_select: Option<ForkSelectCallback>,
    on_cancel: Option<ForkCancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl ForkMessagePickerComponent {
    fn new(keybindings: &KeybindingsManager, candidates: Vec<ForkMessageCandidate>) -> Self {
        Self {
            keybindings: keybindings.clone(),
            candidates,
            selected_index: 0,
            on_select: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        }
    }

    fn set_on_select<F>(&mut self, on_select: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_select = Some(Box::new(on_select));
    }

    fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn max_visible(&self) -> usize {
        self.viewport_size
            .get()
            .map(|(_, height)| height.saturating_sub(4).max(1))
            .unwrap_or(10)
    }

    fn render_candidates(&self, width: usize) -> Vec<String> {
        if self.candidates.is_empty() {
            return vec![truncate_to_width(
                "No messages to fork from",
                width,
                "...",
                false,
            )];
        }

        let max_visible = self.max_visible();
        let start_index = self
            .selected_index
            .saturating_sub(max_visible / 2)
            .min(self.candidates.len().saturating_sub(max_visible));
        let end_index = (start_index + max_visible).min(self.candidates.len());
        let mut lines = Vec::new();

        for (visible_index, candidate) in self.candidates[start_index..end_index].iter().enumerate()
        {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let suffix = if candidate.parent_id.is_none() {
                " (root)"
            } else {
                ""
            };
            lines.push(truncate_to_width(
                &format!(
                    "{prefix}{}{}",
                    sanitize_display_text(&candidate.text),
                    suffix
                ),
                width,
                "...",
                false,
            ));
        }

        if start_index > 0 || end_index < self.candidates.len() {
            lines.push(truncate_to_width(
                &format!("  ({}/{})", self.selected_index + 1, self.candidates.len()),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for ForkMessagePickerComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(
            "Fork session from user message",
            width,
            "...",
            false,
        ));
        lines.extend(self.render_candidates(width));
        lines.push(truncate_to_width(
            "Enter select  Esc cancel  ↑/↓ navigate",
            width,
            "...",
            false,
        ));
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        if self.matches_binding(data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if self.matches_binding(data, "tui.select.up") {
            if self.candidates.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                self.candidates.len() - 1
            } else {
                self.selected_index - 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            if self.candidates.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index + 1 >= self.candidates.len() {
                0
            } else {
                self.selected_index + 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.pageUp") {
            self.selected_index = self.selected_index.saturating_sub(self.max_visible());
            return;
        }

        if self.matches_binding(data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + self.max_visible())
                .min(self.candidates.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") {
            if let Some(candidate) = self.candidates.get(self.selected_index)
                && let Some(on_select) = &mut self.on_select
            {
                on_select(candidate.entry_id.clone());
            }
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}

async fn select_fork_message(
    tui: &mut Tui<LiveInteractiveTerminal>,
    keybindings: &KeybindingsManager,
    candidates: Vec<ForkMessageCandidate>,
) -> Result<Option<String>, String> {
    let outcome = Arc::new(Mutex::new(None::<ForkPickerOutcome>));
    let mut picker = ForkMessagePickerComponent::new(keybindings, candidates);

    let outcome_for_select = Arc::clone(&outcome);
    picker.set_on_select(move |entry_id| {
        *outcome_for_select
            .lock()
            .expect("fork picker outcome mutex poisoned") =
            Some(ForkPickerOutcome::Selected(entry_id));
    });

    let outcome_for_cancel = Arc::clone(&outcome);
    picker.set_on_cancel(move || {
        *outcome_for_cancel
            .lock()
            .expect("fork picker outcome mutex poisoned") = Some(ForkPickerOutcome::Cancelled);
    });

    let picker_id = tui.add_child(Box::new(picker));
    let _ = tui.set_focus_child(picker_id);
    tui.start().map_err(|error| error.to_string())?;

    loop {
        if let Some(outcome) = outcome
            .lock()
            .expect("fork picker outcome mutex poisoned")
            .take()
        {
            tui.clear();
            return Ok(match outcome {
                ForkPickerOutcome::Selected(entry_id) => Some(entry_id),
                ForkPickerOutcome::Cancelled => None,
            });
        }

        tui.drain_terminal_events()
            .map_err(|error| error.to_string())?;
        sleep(Duration::from_millis(16)).await;
    }
}

fn request_interactive_transition(
    transition: InteractiveTransitionRequest,
    core: &CodingAgentCore,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    status_handle: &StatusHandle,
    transition_request: &Arc<Mutex<Option<InteractiveTransitionRequest>>>,
    exit_requested: &Arc<AtomicBool>,
) -> bool {
    if core.state().is_streaming {
        status_handle.set_message(
            "Session switching while a request is running is not supported in the Rust interactive CLI yet.",
        );
        return true;
    }

    if matches!(transition, InteractiveTransitionRequest::ForkPicker) {
        let Some(session_manager) = session_manager else {
            status_handle.set_message("No messages to fork from");
            return true;
        };
        let session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        if collect_fork_candidates(&session_manager).is_empty() {
            status_handle.set_message("No messages to fork from");
            return true;
        }
    }

    *transition_request
        .lock()
        .expect("interactive transition request mutex poisoned") = Some(transition);
    exit_requested.store(true, Ordering::Relaxed);
    true
}

fn append_transcript_custom_message(
    shell: &mut StartupShellComponent,
    custom_type: &str,
    text: impl Into<String>,
) {
    shell.add_transcript_item(Box::new(CustomMessageComponent::new(CustomMessage {
        custom_type: custom_type.to_owned(),
        content: CustomMessageContent::Text(text.into()),
        display: true,
        details: None,
    })));
}

fn current_session_name(session_manager: Option<&Arc<Mutex<SessionManager>>>) -> Option<String> {
    session_manager.and_then(|session_manager| {
        session_manager
            .lock()
            .expect("session manager mutex poisoned")
            .get_session_name()
    })
}

fn render_session_info_text(
    core: &CodingAgentCore,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
) -> String {
    let state = core.state();
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut tool_results = 0usize;
    let mut tool_calls = 0usize;
    let mut total_input = 0u64;
    let mut total_output = 0u64;
    let mut total_cache_read = 0u64;
    let mut total_cache_write = 0u64;
    let mut total_cost = 0.0f64;

    for agent_message in &state.messages {
        let Some(message) = agent_message.as_standard_message() else {
            continue;
        };
        match message {
            Message::User { .. } => user_messages += 1,
            Message::Assistant { content, usage, .. } => {
                assistant_messages += 1;
                tool_calls += content
                    .iter()
                    .filter(|content| matches!(content, AssistantContent::ToolCall { .. }))
                    .count();
                total_input += usage.input;
                total_output += usage.output;
                total_cache_read += usage.cache_read;
                total_cache_write += usage.cache_write;
                total_cost += usage.cost.total;
            }
            Message::ToolResult { .. } => tool_results += 1,
        }
    }

    let session_file = session_manager.and_then(|session_manager| {
        session_manager
            .lock()
            .expect("session manager mutex poisoned")
            .get_session_file()
            .map(str::to_owned)
    });
    let session_id = session_manager
        .map(|session_manager| {
            session_manager
                .lock()
                .expect("session manager mutex poisoned")
                .get_session_id()
                .to_owned()
        })
        .or_else(|| core.agent().session_id())
        .unwrap_or_else(|| String::from("In-memory"));

    let mut info = String::new();
    push_line(&mut info, "Session Info");
    push_line(&mut info, "");
    if let Some(session_name) = current_session_name(session_manager) {
        push_line(&mut info, &format!("Name: {session_name}"));
    }
    push_line(
        &mut info,
        &format!(
            "File: {}",
            session_file.unwrap_or_else(|| String::from("In-memory"))
        ),
    );
    push_line(&mut info, &format!("ID: {session_id}"));
    push_line(&mut info, "");
    push_line(&mut info, "Messages");
    push_line(&mut info, &format!("User: {user_messages}"));
    push_line(&mut info, &format!("Assistant: {assistant_messages}"));
    push_line(&mut info, &format!("Tool Calls: {tool_calls}"));
    push_line(&mut info, &format!("Tool Results: {tool_results}"));
    push_line(&mut info, &format!("Total: {}", state.messages.len()));
    push_line(&mut info, "");
    push_line(&mut info, "Tokens");
    push_line(&mut info, &format!("Input: {total_input}"));
    push_line(&mut info, &format!("Output: {total_output}"));
    if total_cache_read > 0 {
        push_line(&mut info, &format!("Cache Read: {total_cache_read}"));
    }
    if total_cache_write > 0 {
        push_line(&mut info, &format!("Cache Write: {total_cache_write}"));
    }
    push_line(
        &mut info,
        &format!(
            "Total: {}",
            total_input + total_output + total_cache_read + total_cache_write
        ),
    );
    if total_cost > 0.0 {
        push_line(&mut info, "");
        push_line(&mut info, "Cost");
        push_line(&mut info, &format!("Total: {:.4}", total_cost));
    }
    info.trim_end().to_owned()
}

fn render_runtime_settings_text(context: &InteractiveSlashCommandContext) -> String {
    let mut output = String::new();
    push_line(&mut output, "Settings");
    push_line(&mut output, "");
    let global_settings_path = context
        .agent_dir
        .as_ref()
        .map(|agent_dir| agent_dir.join("settings.json"));
    if let Some(global_settings_path) = global_settings_path {
        push_line(
            &mut output,
            &format!("Global settings: {}", global_settings_path.display()),
        );
    } else {
        push_line(&mut output, "Global settings: unavailable");
    }
    push_line(
        &mut output,
        &format!(
            "Project settings: {}",
            context.cwd.join(".pi/settings.json").display()
        ),
    );
    push_line(&mut output, "");
    push_line(&mut output, "Images");
    push_line(
        &mut output,
        &format!(
            "Auto resize: {}",
            if context.runtime_settings.settings.images.auto_resize_images {
                "on"
            } else {
                "off"
            }
        ),
    );
    push_line(
        &mut output,
        &format!(
            "Block images: {}",
            if context.runtime_settings.settings.images.block_images {
                "on"
            } else {
                "off"
            }
        ),
    );
    push_line(&mut output, "");
    push_line(&mut output, "Editor");
    push_line(
        &mut output,
        &format!(
            "Padding X: {}",
            context.runtime_settings.settings.editor_padding_x
        ),
    );
    push_line(
        &mut output,
        &format!(
            "Autocomplete max visible: {}",
            context.runtime_settings.settings.autocomplete_max_visible
        ),
    );
    push_line(&mut output, "");
    push_line(&mut output, "Thinking budgets");
    push_line(
        &mut output,
        &format!(
            "minimal: {}",
            option_u64_label(context.runtime_settings.settings.thinking_budgets.minimal)
        ),
    );
    push_line(
        &mut output,
        &format!(
            "low: {}",
            option_u64_label(context.runtime_settings.settings.thinking_budgets.low)
        ),
    );
    push_line(
        &mut output,
        &format!(
            "medium: {}",
            option_u64_label(context.runtime_settings.settings.thinking_budgets.medium)
        ),
    );
    push_line(
        &mut output,
        &format!(
            "high: {}",
            option_u64_label(context.runtime_settings.settings.thinking_budgets.high)
        ),
    );

    if !context.runtime_settings.warnings.is_empty() {
        push_line(&mut output, "");
        push_line(&mut output, "Warnings");
        for warning in &context.runtime_settings.warnings {
            push_line(
                &mut output,
                &format!("- ({} settings) {}", warning.scope.label(), warning.message),
            );
        }
    }

    output.trim_end().to_owned()
}

fn option_u64_label(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| String::from("default"))
}

fn render_scoped_models_text(core: &CodingAgentCore, scoped_models: &[ScopedModel]) -> String {
    let mut output = String::new();
    push_line(&mut output, "Scoped Models");
    push_line(&mut output, "");
    let current_model = core.state().model;
    push_line(
        &mut output,
        &format!(
            "Current model: {}/{}",
            current_model.provider, current_model.id
        ),
    );
    push_line(&mut output, "");

    if scoped_models.is_empty() {
        push_line(
            &mut output,
            "No scoped models configured. Model cycling uses all available models.",
        );
        return output.trim_end().to_owned();
    }

    for scoped_model in scoped_models {
        let marker = if models_are_equal(Some(&scoped_model.model), Some(&current_model)) {
            "*"
        } else {
            "-"
        };
        let mut line = format!(
            "{marker} {}/{}",
            scoped_model.model.provider, scoped_model.model.id
        );
        if let Some(thinking_level) = scoped_model.thinking_level {
            line.push_str(&format!(
                " (thinking: {})",
                thinking_level_label(thinking_level)
            ));
        }
        push_line(&mut output, &line);
    }

    output.trim_end().to_owned()
}

fn render_hotkeys_text(keybindings: &KeybindingsManager) -> String {
    let mut output = String::new();
    push_line(&mut output, "Keyboard Shortcuts");
    push_line(&mut output, "");

    let mut current_section = None::<&str>;
    for (keybinding, definition) in DEFAULT_APP_KEYBINDINGS.iter() {
        let section = if keybinding.starts_with("tui.editor.") {
            "Editor"
        } else if keybinding.starts_with("tui.input.") {
            "Input"
        } else if keybinding.starts_with("tui.select.") {
            "Selection"
        } else {
            "Application"
        };

        if current_section != Some(section) {
            if current_section.is_some() {
                push_line(&mut output, "");
            }
            push_line(&mut output, section);
            current_section = Some(section);
        }

        let keys = format_key_ids(&keybindings.get_keys(keybinding));
        let description = definition
            .description
            .as_deref()
            .unwrap_or(keybinding.as_str());
        push_line(&mut output, &format!("{keys}: {description}"));
    }

    output.trim_end().to_owned()
}

fn format_key_ids(keys: &[pi_tui::KeyId]) -> String {
    if keys.is_empty() {
        return String::from("(unbound)");
    }

    keys.iter()
        .map(|key| capitalize_key_id(key.as_str()))
        .collect::<Vec<_>>()
        .join(" / ")
}

fn capitalize_key_id(key: &str) -> String {
    key.split('/')
        .map(|binding| {
            binding
                .split('+')
                .map(|part| {
                    let lower = part.to_ascii_lowercase();
                    match lower.as_str() {
                        "ctrl" => String::from("Ctrl"),
                        "alt" => String::from("Alt"),
                        "shift" => String::from("Shift"),
                        "enter" => String::from("Enter"),
                        "escape" => String::from("Escape"),
                        "backspace" => String::from("Backspace"),
                        "delete" => String::from("Delete"),
                        "pageup" => String::from("PageUp"),
                        "pagedown" => String::from("PageDown"),
                        "left" => String::from("Left"),
                        "right" => String::from("Right"),
                        "up" => String::from("Up"),
                        "down" => String::from("Down"),
                        "tab" => String::from("Tab"),
                        "home" => String::from("Home"),
                        "end" => String::from("End"),
                        _ if part.len() <= 1 => part.to_ascii_uppercase(),
                        _ => {
                            let mut characters = part.chars();
                            let Some(first) = characters.next() else {
                                return String::new();
                            };
                            let mut value = first.to_ascii_uppercase().to_string();
                            value.push_str(characters.as_str());
                            value
                        }
                    }
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn render_changelog_text() -> Result<String, String> {
    let changelog_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../packages/coding-agent/CHANGELOG.md");
    let content = fs::read_to_string(&changelog_path)
        .map_err(|error| format!("{}: {error}", changelog_path.display()))?;

    let mut sections = Vec::<String>::new();
    let mut current = Vec::<String>::new();
    for line in content.lines() {
        if line.starts_with("## ") {
            if !current.is_empty() {
                sections.push(current.join("\n").trim().to_owned());
                current.clear();
            }
        }
        if !current.is_empty() || line.starts_with("## ") {
            current.push(line.to_owned());
        }
    }
    if !current.is_empty() {
        sections.push(current.join("\n").trim().to_owned());
    }

    let mut selected = Vec::<String>::new();
    if let Some(unreleased) = sections
        .iter()
        .find(|section| section.starts_with("## [Unreleased]"))
    {
        selected.push(unreleased.clone());
    }
    if let Some(latest_release) = sections
        .iter()
        .find(|section| section.starts_with("## [") && !section.starts_with("## [Unreleased]"))
    {
        selected.push(latest_release.clone());
    }

    if selected.is_empty() {
        return Ok(String::from("No changelog entries found."));
    }

    Ok(selected.join("\n\n").trim().to_owned())
}

fn render_session_tree_text(session_manager: &SessionManager) -> String {
    let mut output = String::new();
    push_line(&mut output, "Session Tree");
    push_line(&mut output, "");
    push_line(
        &mut output,
        &format!(
            "Current leaf: {}",
            session_manager.get_leaf_id().unwrap_or("root")
        ),
    );
    push_line(&mut output, "");

    let tree = session_manager.get_tree();
    if tree.is_empty() {
        push_line(&mut output, "(empty)");
    } else {
        for (index, node) in tree.iter().enumerate() {
            render_session_tree_node(
                &mut output,
                node,
                "",
                index + 1 == tree.len(),
                session_manager.get_leaf_id(),
            );
        }
    }

    push_line(&mut output, "");
    push_line(&mut output, "Use /tree <entry-id> to switch branches.");
    push_line(&mut output, "Use /tree root to switch to the root.");
    output.trim_end().to_owned()
}

fn render_session_tree_node(
    output: &mut String,
    node: &pi_coding_agent_core::SessionTreeNode,
    prefix: &str,
    is_last: bool,
    current_leaf: Option<&str>,
) {
    if matches!(node.entry, SessionEntry::Label { .. }) {
        for (index, child) in node.children.iter().enumerate() {
            render_session_tree_node(
                output,
                child,
                prefix,
                index + 1 == node.children.len(),
                current_leaf,
            );
        }
        return;
    }

    let marker = if Some(node.entry.id()) == current_leaf {
        '*'
    } else {
        ' '
    };
    let branch = if prefix.is_empty() {
        String::new()
    } else if is_last {
        String::from("└─ ")
    } else {
        String::from("├─ ")
    };
    let label_suffix = node
        .label
        .as_deref()
        .map(|label| format!(" [{label}]"))
        .unwrap_or_default();
    if let Some(description) = describe_session_tree_entry(&node.entry) {
        push_line(
            output,
            &format!("{prefix}{branch}{marker} {description}{label_suffix}"),
        );
    }

    let next_prefix = if prefix.is_empty() {
        if is_last {
            String::from("   ")
        } else {
            String::from("│  ")
        }
    } else if is_last {
        format!("{prefix}   ")
    } else {
        format!("{prefix}│  ")
    };

    for (index, child) in node.children.iter().enumerate() {
        render_session_tree_node(
            output,
            child,
            &next_prefix,
            index + 1 == node.children.len(),
            current_leaf,
        );
    }
}

fn describe_session_tree_entry(entry: &SessionEntry) -> Option<String> {
    let description = match entry {
        SessionEntry::Message { id, message, .. } => match message.as_standard_message() {
            Some(Message::User { content, .. }) => {
                format!(
                    "{id} user: {}",
                    truncate_text_for_tree(&extract_user_text(content), 72)
                )
            }
            Some(Message::Assistant { content, .. }) => format!(
                "{id} assistant: {}",
                truncate_text_for_tree(&extract_assistant_text(content), 72)
            ),
            Some(Message::ToolResult { tool_name, .. }) => {
                format!("{id} tool result: {tool_name}")
            }
            None => format!("{id} custom message"),
        },
        SessionEntry::ThinkingLevelChange {
            id, thinking_level, ..
        } => format!("{id} thinking: {thinking_level}"),
        SessionEntry::ModelChange {
            id,
            provider,
            model_id,
            ..
        } => format!("{id} model: {provider}/{model_id}"),
        SessionEntry::Compaction { id, summary, .. } => {
            format!("{id} compaction: {}", truncate_text_for_tree(summary, 72))
        }
        SessionEntry::BranchSummary { id, summary, .. } => format!(
            "{id} branch summary: {}",
            truncate_text_for_tree(summary, 72)
        ),
        SessionEntry::Custom {
            id, custom_type, ..
        } => format!("{id} custom: {custom_type}"),
        SessionEntry::CustomMessage {
            id,
            custom_type,
            content,
            ..
        } => format!(
            "{id} {custom_type}: {}",
            truncate_text_for_tree(&extract_custom_message_text(content), 72)
        ),
        SessionEntry::SessionInfo { id, name, .. } => format!(
            "{id} session: {}",
            name.clone().unwrap_or_else(|| String::from("(unnamed)"))
        ),
        SessionEntry::Label { .. } => return None,
    };
    Some(description)
}

fn truncate_text_for_tree(text: &str, max: usize) -> String {
    let sanitized = sanitize_display_text(text);
    if sanitized.chars().count() <= max {
        return sanitized;
    }
    let truncated = sanitized
        .chars()
        .take(max.saturating_sub(3))
        .collect::<String>();
    format!("{truncated}...")
}

fn extract_assistant_text(content: &[AssistantContent]) -> String {
    content
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text { text, .. } => Some(text.as_str()),
            AssistantContent::Thinking { .. } | AssistantContent::ToolCall { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_owned()
}

fn extract_custom_message_text(content: &CustomMessageContent) -> String {
    match content {
        CustomMessageContent::Text(text) => text.trim().to_owned(),
        CustomMessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                UserContent::Text { text } => Some(text.as_str()),
                UserContent::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_owned(),
    }
}

fn switch_interactive_tree_branch(
    core: &CodingAgentCore,
    model_registry: &ModelRegistry,
    session_manager: &Arc<Mutex<SessionManager>>,
    branch_ref: &str,
) -> Result<String, String> {
    let (session_context, leaf_id) = {
        let mut session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        if branch_ref.eq_ignore_ascii_case("root") {
            session_manager.reset_leaf();
        } else {
            session_manager
                .branch(branch_ref)
                .map_err(|error| error.to_string())?;
        }
        (
            session_manager.build_session_context(),
            session_manager.get_leaf_id().map(str::to_owned),
        )
    };

    let current_state = core.state();
    let restore_result = session_context.model.as_ref().map(|saved_model| {
        restore_model_from_session(
            &model_registry.catalog(),
            &saved_model.provider,
            &saved_model.model_id,
            Some(&current_state.model),
        )
    });
    let next_model = restore_result
        .as_ref()
        .and_then(|result| result.model.clone())
        .unwrap_or_else(|| current_state.model.clone());
    let next_thinking_level = clamp_interactive_thinking_level(
        parse_thinking_level(&session_context.thinking_level).unwrap_or(ThinkingLevel::Off),
        &next_model,
    );
    let next_messages = session_context.messages;

    core.agent().update_state(move |state| {
        state.messages = next_messages.clone();
        state.model = next_model.clone();
        state.thinking_level = next_thinking_level;
    });

    let mut message = format!("Switched to {}", leaf_id.as_deref().unwrap_or("root"));
    if let Some(fallback_message) = restore_result.and_then(|result| result.fallback_message) {
        message.push_str(". ");
        message.push_str(&fallback_message);
    }
    Ok(message)
}

fn resolve_export_path(
    cwd: &Path,
    session_manager: &SessionManager,
    output_path: Option<&str>,
) -> Result<PathBuf, String> {
    let Some(output_path) = output_path.filter(|path| !path.trim().is_empty()) else {
        return Ok(cwd.join(format!(
            "session-{}.jsonl",
            session_manager.get_session_id()
        )));
    };

    if output_path.ends_with(".html") {
        return Err(String::from(
            "HTML export is not supported in the Rust interactive CLI yet. Use /export <path.jsonl>.",
        ));
    }

    let output_path = Path::new(output_path);
    if output_path.is_absolute() {
        Ok(output_path.to_path_buf())
    } else {
        Ok(cwd.join(output_path))
    }
}

fn export_interactive_session(
    session_manager: &Arc<Mutex<SessionManager>>,
    cwd: &Path,
    output_path: Option<&str>,
) -> Result<String, String> {
    let session_manager = session_manager
        .lock()
        .expect("session manager mutex poisoned");
    let output_path = resolve_export_path(cwd, &session_manager, output_path)?;
    session_manager
        .export_branch_jsonl(&output_path)
        .map_err(|error| error.to_string())
}

fn share_interactive_session(
    session_manager: &Arc<Mutex<SessionManager>>,
    cwd: &Path,
) -> Result<String, String> {
    let temp_file = {
        let session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        env::temp_dir().join(format!(
            "pi-session-{}.jsonl",
            session_manager.get_session_id()
        ))
    };

    export_interactive_session(session_manager, cwd, temp_file.to_str())?;
    let temp_file_string = temp_file.to_string_lossy().into_owned();
    let output = Command::new("gh")
        .args([
            "gist",
            "create",
            "--public=false",
            temp_file_string.as_str(),
        ])
        .output()
        .map_err(|error| format!("Failed to run gh: {error}"))?;
    let _ = fs::remove_file(&temp_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            String::from("gh gist create failed")
        };
        return Err(message);
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| String::from("Failed to parse gist URL from gh output"))
}

fn last_assistant_message_text(core: &CodingAgentCore) -> Option<String> {
    core.state()
        .messages
        .iter()
        .rev()
        .filter_map(|agent_message| {
            let Message::Assistant {
                content,
                stop_reason,
                ..
            } = agent_message.as_standard_message()?
            else {
                return None;
            };
            if *stop_reason == pi_events::StopReason::Aborted && content.is_empty() {
                return None;
            }
            let text = extract_assistant_text(content);
            (!text.is_empty()).then_some(text)
        })
        .next()
}

fn copy_text_to_system_clipboard(text: &str) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        return run_clipboard_command("pbcopy", &[], text);
    }

    if env::var_os("WAYLAND_DISPLAY").is_some()
        && run_clipboard_command("wl-copy", &["--type", "text/plain"], text).is_ok()
    {
        return Ok(());
    }

    if run_clipboard_command("xclip", &["-selection", "clipboard", "-in"], text).is_ok() {
        return Ok(());
    }
    if run_clipboard_command("xsel", &["--clipboard", "--input"], text).is_ok() {
        return Ok(());
    }
    if cfg!(windows) {
        if run_clipboard_command("clip", &[], text).is_ok() {
            return Ok(());
        }
        return run_clipboard_command("clip.exe", &[], text);
    }

    Err(String::from(
        "No clipboard command found (tried wl-copy, xclip, xsel, pbcopy, clip).",
    ))
}

fn run_clipboard_command(command: &str, args: &[&str], text: &str) -> Result<(), String> {
    use std::io::Write as _;

    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{command}: {error}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|error| format!("{command}: {error}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("{command}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        Err(format!("{command} exited with status {}", output.status))
    } else {
        Err(format!("{command}: {stderr}"))
    }
}

fn collect_fork_candidates(session_manager: &SessionManager) -> Vec<ForkMessageCandidate> {
    session_manager
        .get_entries()
        .iter()
        .filter_map(|entry| {
            let SessionEntry::Message {
                id,
                parent_id,
                message,
                ..
            } = entry
            else {
                return None;
            };
            let Message::User { content, .. } = message.as_standard_message()? else {
                return None;
            };
            let text = extract_user_text(content);
            (!text.is_empty()).then(|| ForkMessageCandidate {
                entry_id: id.clone(),
                parent_id: parent_id.clone(),
                text,
            })
        })
        .collect()
}

fn extract_user_text(content: &[UserContent]) -> String {
    content
        .iter()
        .filter_map(|content| match content {
            UserContent::Text { text } => Some(text.as_str()),
            UserContent::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_owned()
}

fn sanitize_display_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() && character != '\n' && character != '\t' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
    slash_command_context: InteractiveSlashCommandContext,
    status_handle: StatusHandle,
    footer_state_handle: FooterStateHandle,
    exit_requested: Arc<AtomicBool>,
    transition_request: Arc<Mutex<Option<InteractiveTransitionRequest>>>,
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

    let new_session_core = core.clone();
    let new_session_status_handle = status_handle.clone();
    let new_session_transition_request = transition_request.clone();
    let new_session_exit_requested = exit_requested.clone();
    shell.on_action("app.session.new", move || {
        request_interactive_transition(
            InteractiveTransitionRequest::NewSession,
            &new_session_core,
            None,
            &new_session_status_handle,
            &new_session_transition_request,
            &new_session_exit_requested,
        );
    });

    let resume_core = core.clone();
    let resume_status_handle = status_handle.clone();
    let resume_transition_request = transition_request.clone();
    let resume_exit_requested = exit_requested.clone();
    shell.on_action("app.session.resume", move || {
        request_interactive_transition(
            InteractiveTransitionRequest::ResumePicker,
            &resume_core,
            None,
            &resume_status_handle,
            &resume_transition_request,
            &resume_exit_requested,
        );
    });

    let fork_core = core.clone();
    let fork_status_handle = status_handle.clone();
    let fork_transition_request = transition_request.clone();
    let fork_exit_requested = exit_requested.clone();
    let fork_session_manager = session_manager.clone();
    shell.on_action("app.session.fork", move || {
        request_interactive_transition(
            InteractiveTransitionRequest::ForkPicker,
            &fork_core,
            fork_session_manager.as_ref(),
            &fork_status_handle,
            &fork_transition_request,
            &fork_exit_requested,
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
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
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
            update_interactive_footer_state(footer_state_handle, core, session_manager);
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
    slash_command_context: &InteractiveSlashCommandContext,
    status_handle: &StatusHandle,
    footer_state_handle: &FooterStateHandle,
    exit_requested: &Arc<AtomicBool>,
    transition_request: &Arc<Mutex<Option<InteractiveTransitionRequest>>>,
) -> bool {
    if text == "/quit" {
        exit_requested.store(true, Ordering::Relaxed);
        return true;
    }

    if text == "/settings" {
        append_transcript_custom_message(
            shell,
            "settings",
            render_runtime_settings_text(slash_command_context),
        );
        return true;
    }

    if text == "/new" {
        return request_interactive_transition(
            InteractiveTransitionRequest::NewSession,
            core,
            None,
            status_handle,
            transition_request,
            exit_requested,
        );
    }

    if text == "/resume" {
        return request_interactive_transition(
            InteractiveTransitionRequest::ResumePicker,
            core,
            None,
            status_handle,
            transition_request,
            exit_requested,
        );
    }

    if text == "/fork" {
        return request_interactive_transition(
            InteractiveTransitionRequest::ForkPicker,
            core,
            session_manager,
            status_handle,
            transition_request,
            exit_requested,
        );
    }

    if text == "/tree" || text.starts_with("/tree ") {
        let Some(session_manager) = session_manager else {
            status_handle.set_message("Session tree is not available in this interactive mode.");
            return true;
        };

        let target = text.strip_prefix("/tree").unwrap_or_default().trim();
        if target.is_empty() {
            let rendered = render_session_tree_text(
                &session_manager
                    .lock()
                    .expect("session manager mutex poisoned"),
            );
            append_transcript_custom_message(shell, "tree", rendered);
            return true;
        }

        if core.state().is_streaming {
            status_handle.set_message(
                "Session switching while a request is running is not supported in the Rust interactive CLI yet.",
            );
            return true;
        }

        match switch_interactive_tree_branch(core, model_registry, session_manager, target) {
            Ok(message) => {
                update_interactive_footer_state(footer_state_handle, core, Some(session_manager));
                status_handle.set_message(message);
            }
            Err(error) => status_handle.set_message(format!("Error: {error}")),
        }
        return true;
    }

    if text == "/scoped-models" || text.starts_with("/scoped-models ") {
        append_transcript_custom_message(
            shell,
            "scoped-models",
            render_scoped_models_text(core, scoped_models),
        );
        return true;
    }

    if text == "/session" {
        append_transcript_custom_message(
            shell,
            "session",
            render_session_info_text(core, session_manager),
        );
        return true;
    }

    if text == "/copy" {
        let Some(text) = last_assistant_message_text(core) else {
            status_handle.set_message("No assistant messages to copy yet.");
            return true;
        };
        match copy_text_to_system_clipboard(&text) {
            Ok(()) => status_handle.set_message("Copied last assistant message to clipboard"),
            Err(error) => status_handle.set_message(format!("Error: {error}")),
        }
        return true;
    }

    if text == "/export" || text.starts_with("/export ") {
        let Some(session_manager) = session_manager else {
            status_handle.set_message("Session export is not available in this interactive mode.");
            return true;
        };
        let output_path = text.strip_prefix("/export").unwrap_or_default().trim();
        match export_interactive_session(
            session_manager,
            &slash_command_context.cwd,
            (!output_path.is_empty()).then_some(output_path),
        ) {
            Ok(path) => status_handle.set_message(format!("Session exported to: {path}")),
            Err(error) => status_handle.set_message(format!("Error: {error}")),
        }
        return true;
    }

    if text == "/share" {
        let Some(session_manager) = session_manager else {
            status_handle.set_message("Session sharing is not available in this interactive mode.");
            return true;
        };
        match share_interactive_session(session_manager, &slash_command_context.cwd) {
            Ok(url) => status_handle.set_message(format!("Shared session: {url}")),
            Err(error) => status_handle.set_message(format!("Error: {error}")),
        }
        return true;
    }

    if text == "/name" || text.starts_with("/name ") {
        let name = text.strip_prefix("/name").unwrap_or_default().trim();
        let current_name = current_session_name(session_manager);
        if name.is_empty() {
            if let Some(current_name) = current_name {
                append_transcript_custom_message(
                    shell,
                    "session",
                    format!("Session name: {current_name}"),
                );
            } else {
                status_handle.set_message("Usage: /name <name>");
            }
            return true;
        }

        let Some(session_manager) = session_manager else {
            status_handle.set_message("Session naming is not supported in this interactive mode.");
            return true;
        };

        match session_manager
            .lock()
            .expect("session manager mutex poisoned")
            .append_session_info(name)
        {
            Ok(_) => {
                footer_state_handle.update(|footer_state| {
                    footer_state.session_name = Some(name.to_owned());
                });
                append_transcript_custom_message(
                    shell,
                    "session",
                    format!("Session name set: {name}"),
                );
            }
            Err(error) => status_handle.set_message(format!("Error: {error}")),
        }
        return true;
    }

    if text == "/changelog" {
        match render_changelog_text() {
            Ok(changelog) => append_transcript_custom_message(shell, "changelog", changelog),
            Err(error) => status_handle.set_message(format!("Error: {error}")),
        }
        return true;
    }

    if text == "/hotkeys" {
        append_transcript_custom_message(
            shell,
            "hotkeys",
            render_hotkeys_text(&slash_command_context.keybindings),
        );
        return true;
    }

    if text == "/login" {
        status_handle
            .set_message("OAuth login is not implemented in the Rust interactive CLI yet.");
        return true;
    }

    if text == "/logout" {
        status_handle
            .set_message("OAuth logout is not implemented in the Rust interactive CLI yet.");
        return true;
    }

    if text == "/compact" || text.starts_with("/compact ") {
        status_handle
            .set_message("Session compaction is not implemented in the Rust interactive CLI yet.");
        return true;
    }

    if text == "/reload" {
        status_handle.set_message(
            "Reloading keybindings, extensions, skills, prompts, and themes is not implemented in the Rust interactive CLI yet.",
        );
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

            update_interactive_footer_state(footer_state_handle, core, session_manager);
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

            update_interactive_footer_state(
                &footer_state_handle_for_select,
                &core,
                session_manager.as_ref(),
            );
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
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
) {
    let state = core.state();
    let session_name = current_session_name(session_manager);
    footer_state_handle.update(|footer_state| {
        footer_state.model = Some(state.model.clone());
        footer_state.context_window = state.model.context_window;
        footer_state.thinking_level = thinking_level_label(state.thinking_level).to_owned();
        footer_state.session_name = session_name.clone();
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
        "  - --continue, --resume, --session, --fork, --no-session, --session-dir",
        "  - --list-models [search]",
        "  - @file text/image preprocessing",
        "",
        "Not yet supported:",
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

    fn test_slash_command_context(
        keybindings: &KeybindingsManager,
        cwd: impl Into<PathBuf>,
    ) -> InteractiveSlashCommandContext {
        InteractiveSlashCommandContext {
            keybindings: keybindings.clone(),
            runtime_settings: LoadedRuntimeSettings::default(),
            cwd: cwd.into(),
            agent_dir: None,
        }
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

    #[test]
    fn collect_fork_candidates_uses_user_messages_only() {
        let mut session_manager = SessionManager::in_memory("/tmp/pi-fork-candidates");
        session_manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("first user message"),
                }],
                timestamp: 1,
            })
            .unwrap();
        session_manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("assistant response"),
                    text_signature: None,
                }],
                api: String::from("faux:test"),
                provider: String::from("faux"),
                model: String::from("model"),
                response_id: None,
                usage: Default::default(),
                stop_reason: pi_events::StopReason::Stop,
                error_message: None,
                timestamp: 2,
            })
            .unwrap();

        let candidates = collect_fork_candidates(&session_manager);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].text, "first user message");
        assert!(candidates[0].parent_id.is_none());
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

    #[test]
    fn slash_commands_name_and_session_update_transcript_and_session_metadata() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-session-commands-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-session-commands-faux-1".into(),
                name: Some("Slash Session Commands Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-session-commands-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-session-commands-cwd");
        let created = create_coding_agent_core(CodingAgentCoreOptions {
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                model.provider.as_str(),
                "token",
            )])),
            built_in_models: vec![model],
            models_json_path: None,
            cwd: Some(cwd.clone()),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        })
        .expect("expected coding agent core");
        let core = created.core;
        let keybindings = create_keybindings_manager(None);
        let mut shell = StartupShellComponent::new(
            "Pi",
            "0.1.0",
            &keybindings,
            &PlainKeyHintStyler,
            true,
            None,
            false,
        );
        let session_manager = Arc::new(Mutex::new(SessionManager::in_memory("/tmp/pi-session")));
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context = test_slash_command_context(&keybindings, cwd);

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/name demo",
            &core,
            core.model_registry().as_ref(),
            &[],
            Some(&session_manager),
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));
        assert_eq!(
            session_manager
                .lock()
                .expect("session manager mutex poisoned")
                .get_session_name()
                .as_deref(),
            Some("demo")
        );

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/session",
            &core,
            core.model_registry().as_ref(),
            &[],
            Some(&session_manager),
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));

        let rendered = shell.render(100).join("\n");
        assert!(
            rendered.contains("Session name set: demo"),
            "output: {rendered}"
        );
        assert!(rendered.contains("Session Info"), "output: {rendered}");
        assert!(rendered.contains("Name: demo"), "output: {rendered}");

        faux.unregister();
    }

    #[test]
    fn interactive_slash_command_catalog_includes_remaining_builtin_commands() {
        let registry = Arc::new(ModelRegistry::new(
            Arc::new(MemoryAuthStorage::default()),
            Vec::new(),
            None,
        ));
        let commands = build_interactive_slash_commands(registry, Vec::new())
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();

        for command in [
            "settings",
            "model",
            "scoped-models",
            "export",
            "share",
            "copy",
            "name",
            "session",
            "changelog",
            "hotkeys",
            "fork",
            "tree",
            "login",
            "logout",
            "new",
            "compact",
            "resume",
            "reload",
            "quit",
        ] {
            assert!(
                commands.iter().any(|current| current == command),
                "missing command {command}: {commands:?}"
            );
        }
    }

    #[test]
    fn slash_commands_render_settings_hotkeys_tree_and_changelog() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-render-commands-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-render-commands-faux-1".into(),
                name: Some("Slash Render Commands Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-render-commands-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-render-commands-cwd");
        let created = create_coding_agent_core(CodingAgentCoreOptions {
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                model.provider.as_str(),
                "token",
            )])),
            built_in_models: vec![model],
            models_json_path: None,
            cwd: Some(cwd.clone()),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        })
        .expect("expected coding agent core");
        let core = created.core;
        let keybindings = create_keybindings_manager(None);
        let mut shell = StartupShellComponent::new(
            "Pi",
            "0.1.0",
            &keybindings,
            &PlainKeyHintStyler,
            true,
            None,
            false,
        );
        let mut manager = SessionManager::in_memory(cwd.to_str().unwrap());
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("hello"),
                }],
                timestamp: 1,
            })
            .unwrap();
        let session_manager = Arc::new(Mutex::new(manager));
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context = test_slash_command_context(&keybindings, cwd);

        for command in [
            "/settings",
            "/scoped-models",
            "/hotkeys",
            "/tree",
            "/changelog",
        ] {
            assert!(handle_interactive_slash_command(
                &mut shell,
                command,
                &core,
                core.model_registry().as_ref(),
                &[],
                Some(&session_manager),
                &slash_command_context,
                &status_handle,
                &footer_state_handle,
                &exit_requested,
                &transition_request,
            ));
        }

        let rendered = shell.render(120).join("\n");
        assert!(rendered.contains("Settings"), "output: {rendered}");
        assert!(rendered.contains("Scoped Models"), "output: {rendered}");
        assert!(
            rendered.contains("Keyboard Shortcuts"),
            "output: {rendered}"
        );
        assert!(rendered.contains("Session Tree"), "output: {rendered}");
        assert!(rendered.contains("## ["), "output: {rendered}");

        faux.unregister();
    }

    #[test]
    fn slash_export_command_writes_jsonl_session_snapshot() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-export-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-export-faux-1".into(),
                name: Some("Slash Export Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-export-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-export-cwd");
        let created = create_coding_agent_core(CodingAgentCoreOptions {
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                model.provider.as_str(),
                "token",
            )])),
            built_in_models: vec![model],
            models_json_path: None,
            cwd: Some(cwd.clone()),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        })
        .expect("expected coding agent core");
        let core = created.core;
        let keybindings = create_keybindings_manager(None);
        let mut shell = StartupShellComponent::new(
            "Pi",
            "0.1.0",
            &keybindings,
            &PlainKeyHintStyler,
            true,
            None,
            false,
        );
        let mut manager = SessionManager::in_memory(cwd.to_str().unwrap());
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("export me"),
                }],
                timestamp: 1,
            })
            .unwrap();
        let session_manager = Arc::new(Mutex::new(manager));
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context = test_slash_command_context(&keybindings, cwd.clone());
        let export_path = cwd.join("session-export.jsonl");
        let command = format!("/export {}", export_path.display());

        assert!(handle_interactive_slash_command(
            &mut shell,
            &command,
            &core,
            core.model_registry().as_ref(),
            &[],
            Some(&session_manager),
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));

        let exported = fs::read_to_string(&export_path).expect("expected exported jsonl");
        assert!(
            exported.contains("\"type\":\"session\""),
            "content: {exported}"
        );
        assert!(
            exported.contains("\"role\":\"user\""),
            "content: {exported}"
        );
        assert!(exported.contains("export me"), "content: {exported}");

        faux.unregister();
    }

    #[test]
    fn tree_slash_command_switches_session_context() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-tree-switch-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-tree-switch-faux-1".into(),
                name: Some("Slash Tree Switch Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-tree-switch-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-tree-switch-cwd");
        let created = create_coding_agent_core(CodingAgentCoreOptions {
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                model.provider.as_str(),
                "token",
            )])),
            built_in_models: vec![model],
            models_json_path: None,
            cwd: Some(cwd.clone()),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        })
        .expect("expected coding agent core");
        let core = created.core;
        let keybindings = create_keybindings_manager(None);
        let mut shell = StartupShellComponent::new(
            "Pi",
            "0.1.0",
            &keybindings,
            &PlainKeyHintStyler,
            true,
            None,
            false,
        );
        let mut manager = SessionManager::in_memory(cwd.to_str().unwrap());
        let root_user_id = manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("root"),
                }],
                timestamp: 1,
            })
            .unwrap();
        manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("assistant root"),
                    text_signature: None,
                }],
                api: String::from("faux:test"),
                provider: String::from("slash-tree-switch-faux"),
                model: String::from("slash-tree-switch-faux-1"),
                response_id: None,
                usage: Default::default(),
                stop_reason: pi_events::StopReason::Stop,
                error_message: None,
                timestamp: 2,
            })
            .unwrap();
        let primary_user_id = manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("primary branch"),
                }],
                timestamp: 3,
            })
            .unwrap();
        manager.branch(&root_user_id).unwrap();
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("alternate branch"),
                }],
                timestamp: 4,
            })
            .unwrap();
        let initial_context = manager.build_session_context();
        core.agent().update_state(move |state| {
            state.messages = initial_context.messages.clone();
        });
        let session_manager = Arc::new(Mutex::new(manager));
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context = test_slash_command_context(&keybindings, cwd);

        assert!(handle_interactive_slash_command(
            &mut shell,
            &format!("/tree {primary_user_id}"),
            &core,
            core.model_registry().as_ref(),
            &[],
            Some(&session_manager),
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));

        let state = core.state();
        let user_messages = state
            .messages
            .iter()
            .filter_map(|message| match message.as_standard_message() {
                Some(Message::User { content, .. }) => Some(extract_user_text(content)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            user_messages
                .iter()
                .any(|message| message == "primary branch")
        );
        assert!(
            !user_messages
                .iter()
                .any(|message| message == "alternate branch")
        );

        faux.unregister();
    }

    #[tokio::test]
    async fn new_session_transition_resets_session_entries_and_preserves_model_defaults() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "new-session-transition-faux".into(),
            models: vec![FauxModelDefinition {
                id: "new-session-transition-faux-1".into(),
                name: Some("New Session Transition Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("new-session-transition-faux-1"))
            .expect("expected faux model");
        let mut session_manager = SessionManager::in_memory("/tmp/pi-new-session-transition");
        session_manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("hello"),
                }],
                timestamp: 1,
            })
            .unwrap();

        let plan = resolve_interactive_transition(
            InteractiveTransitionRequest::NewSession,
            Some(InteractiveSessionContext {
                manager: Some(session_manager),
                session_file: None,
                session_dir: None,
                cwd: String::from("/tmp/pi-new-session-transition"),
                model: model.clone(),
                thinking_level: ThinkingLevel::Off,
            }),
            Path::new("/tmp/pi-new-session-transition"),
            None,
            &InteractiveRuntime::new(Arc::new(|| {
                Box::new(LifecycleScriptedTerminal::new(Vec::new()))
            })),
        )
        .await
        .expect("expected new session transition plan");

        let manager = plan.manager.expect("expected new session manager");
        assert!(manager.get_entries().is_empty());
        assert_eq!(
            plan.initial_status_message.as_deref(),
            Some("New session started")
        );
        let defaults = plan
            .bootstrap_defaults
            .expect("expected preserved bootstrap defaults");
        assert_eq!(defaults.provider, model.provider);
        assert_eq!(defaults.model_id, model.id);
        assert_eq!(defaults.thinking_level, ThinkingLevel::Off);

        faux.unregister();
    }
}
