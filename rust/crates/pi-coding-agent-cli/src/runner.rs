#[path = "export_html.rs"]
mod export_html;
#[path = "rpc_extensions.rs"]
mod rpc_extensions;

use crate::{
    AppMode, Args, Diagnostic, DiagnosticKind, ListModels, OverlayAuthSource, PrintModeOptions,
    PrintOutputMode, ProcessFileOptions,
    auth::{
        OAuthProviderSummary, list_persisted_oauth_providers, oauth_provider_name,
        oauth_provider_summaries, remove_persisted_oauth_provider, run_terminal_oauth_login,
    },
    build_initial_message,
    list_models::render_list_models,
    parse_args, process_file_arguments, resolve_app_mode,
    resources::{
        ExtensionResourcePath, LoadedCliResources, build_runtime_system_prompt,
        build_selected_tools, extend_cli_resources_from_extensions, load_cli_resources,
        preprocess_prompt_text,
    },
    run_print_mode,
    session_picker::SessionPickerComponent,
    to_print_output_mode,
    tree_picker::{TreePickerComponent, TreePickerItem},
};
use pi_agent::{AgentUnsubscribe, BeforeToolCallResult, ThinkingLevel};
use pi_ai::{
    StreamOptions, ThinkingBudgets, Transport, is_context_overflow, models_are_equal,
    supports_xhigh,
};
use pi_coding_agent_core::{
    AuthSource, BashExecutionMessage, BootstrapDiagnosticLevel, CodingAgentCore,
    CodingAgentCoreError, CodingAgentCoreOptions, CompactionResult, CompactionSettings,
    ContextUsageEstimate, CustomMessage, CustomMessageContent, ExistingSessionSelection,
    FooterDataProvider, ModelRegistry, NewSessionOptions, ScopedModel, SessionBootstrapOptions,
    SessionEntry, SessionHeader, SessionInfo, SessionManager, bash_execution_to_text,
    build_default_pi_system_prompt, calculate_context_tokens, compact,
    create_bash_execution_message, create_coding_agent_core, estimate_context_tokens,
    find_exact_model_reference_match, get_default_session_dir, parse_thinking_level,
    prepare_compaction, resolve_cli_model, resolve_model_scope, restore_model_from_session,
    should_compact,
};
#[cfg(test)]
use pi_coding_agent_tui::PlainKeyHintStyler;
use pi_coding_agent_tui::{
    CustomMessageComponent, DEFAULT_APP_KEYBINDINGS, ExternalEditorCommandRunner,
    ExternalEditorHost, FooterStateHandle, InteractiveCoreBinding, KeybindingsManager,
    StartupShellComponent, StatusHandle, ThemedKeyHintStyler, init_theme, key_hint, key_text,
    set_registered_themes,
};
use pi_config::{LoadedRuntimeSettings, ThinkingBudgetsSettings, load_runtime_settings};
use pi_events::{AssistantContent, Message, Model, UserContent};
use pi_tui::{
    AutocompleteItem, CombinedAutocompleteProvider, Component, Container, Input, ProcessTerminal,
    RenderHandle, SlashCommand, Spacer, Terminal, Text, Tui, TuiError, fuzzy_filter, matches_key,
    truncate_to_width,
};
use std::{
    borrow::Cow,
    cell::Cell,
    collections::BTreeMap,
    env, fs,
    ops::Deref,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{process::Command as TokioCommand, sync::watch, time::sleep};

use rpc_extensions::{
    RpcExtensionCommandInfo, RpcExtensionHost, RpcExtensionHostStartOptions, RpcToolCallResult,
    should_start_extension_host,
};
use serde_json::{Value, json};

const NO_MODELS_ENV_HINT: &str = "  ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY, etc.";
const API_KEY_MODEL_REQUIREMENT: &str =
    "--api-key requires a model to be specified via --model, --provider/--model, or --models";
const FINALIZED_SYSTEM_PROMPT_PREFIX: &str = "\0pi-final-system-prompt\n";
const ROOT_TREE_ENTRY_ID: &str = "root";
const DEFAULT_SHARE_VIEWER_URL: &str = "https://pi.dev/session/";

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
type TextEmitter = Arc<dyn Fn(String) + Send + Sync>;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OAuthPickerMode {
    Login,
    Logout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InteractiveTransitionRequest {
    NewSession,
    ResumePicker,
    ForkPicker,
    TreePicker,
    SettingsPicker,
    OAuthPicker(OAuthPickerMode),
    ScopedModelsPicker { initial_search: Option<String> },
    Reload,
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
    runtime_settings: Arc<Mutex<LoadedRuntimeSettings>>,
    cwd: PathBuf,
    agent_dir: Option<PathBuf>,
    ui_host: Arc<dyn ExternalEditorHost>,
    auth_operation_in_progress: Arc<AtomicBool>,
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
    scoped_models_override: Option<Vec<ScopedModel>>,
    runtime_settings_override: Option<LoadedRuntimeSettings>,
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
    scoped_models: Vec<ScopedModel>,
    available_models: Vec<Model>,
    runtime_settings: LoadedRuntimeSettings,
}

struct InteractiveTransitionPlan {
    manager: Option<SessionManager>,
    cwd: PathBuf,
    prefill_input: Option<String>,
    initial_status_message: Option<String>,
    bootstrap_defaults: Option<BootstrapDefaults>,
    scoped_models: Vec<ScopedModel>,
    runtime_settings: LoadedRuntimeSettings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ForkMessageCandidate {
    entry_id: String,
    parent_id: Option<String>,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SettingsPickerSelection {
    auto_compact: bool,
    auto_resize_images: bool,
    block_images: bool,
    editor_padding_x: usize,
    autocomplete_max_visible: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PersistedScopedModels {
    AllEnabled,
    Explicit(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ScopedModelsPickerSelection {
    enabled_ids: Option<Vec<String>>,
    persisted: Option<PersistedScopedModels>,
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

struct PreparedStartupSession {
    runtime_cwd: PathBuf,
    session_support: Option<SessionSupport>,
}

enum StartupSessionPreparation {
    Ready(PreparedStartupSession),
    Cancelled,
}

async fn select_startup_resume_session(
    parsed: &Args,
    cwd: &Path,
    agent_dir: Option<&Path>,
    terminal_factory: InteractiveTerminalFactory,
) -> Result<Option<String>, String> {
    let keybindings = create_keybindings_manager(agent_dir);
    let terminal = LiveInteractiveTerminal::new((terminal_factory)());
    let mut tui = Tui::new(terminal);
    let cwd_string = cwd.to_string_lossy().into_owned();
    let session_dir = resolve_session_dir(parsed, cwd, agent_dir);
    let agent_dir_string = agent_dir.map(|agent_dir| agent_dir.to_string_lossy().into_owned());
    let current_sessions = SessionManager::list(&cwd_string, session_dir.as_deref());
    let all_sessions = SessionManager::list_all(agent_dir_string.as_deref());
    let outcome =
        select_resume_session(&mut tui, &keybindings, current_sessions, all_sessions).await;
    let _ = tui.stop();
    outcome
}

async fn prepare_startup_session(
    parsed: &Args,
    cwd: &Path,
    agent_dir: Option<&Path>,
    terminal_factory: InteractiveTerminalFactory,
) -> Result<StartupSessionPreparation, String> {
    let show_resume_picker =
        parsed.resume && !parsed.no_session && parsed.session.is_none() && parsed.fork.is_none();

    let selected_resume_session = if show_resume_picker {
        match select_startup_resume_session(parsed, cwd, agent_dir, terminal_factory).await? {
            Some(path) => Some(path),
            None => return Ok(StartupSessionPreparation::Cancelled),
        }
    } else {
        None
    };

    let session_support = create_session_support(
        parsed,
        cwd,
        agent_dir,
        selected_resume_session.as_deref(),
        None,
    )?;
    let runtime_cwd = session_support
        .as_ref()
        .map(|session_support| PathBuf::from(session_support.header.cwd.clone()))
        .unwrap_or_else(|| cwd.to_path_buf());

    Ok(StartupSessionPreparation::Ready(PreparedStartupSession {
        runtime_cwd,
        session_support,
    }))
}

#[derive(Debug, Clone)]
enum TreePickerOutcome {
    Selected(String),
    Cancelled,
}

async fn select_tree_entry(
    tui: &mut Tui<LiveInteractiveTerminal>,
    keybindings: &KeybindingsManager,
    items: Vec<TreePickerItem>,
    initial_selected_id: Option<&str>,
) -> Result<Option<String>, String> {
    let outcome = Arc::new(Mutex::new(None::<TreePickerOutcome>));
    let mut picker = TreePickerComponent::new(keybindings, items, initial_selected_id);

    let outcome_for_select = Arc::clone(&outcome);
    picker.set_on_select(move |entry_id| {
        *outcome_for_select
            .lock()
            .expect("tree picker outcome mutex poisoned") =
            Some(TreePickerOutcome::Selected(entry_id));
    });

    let outcome_for_cancel = Arc::clone(&outcome);
    picker.set_on_cancel(move || {
        *outcome_for_cancel
            .lock()
            .expect("tree picker outcome mutex poisoned") = Some(TreePickerOutcome::Cancelled);
    });

    let picker_id = tui.add_child(Box::new(picker));
    let _ = tui.set_focus_child(picker_id);
    tui.start().map_err(|error| error.to_string())?;

    loop {
        if let Some(outcome) = outcome
            .lock()
            .expect("tree picker outcome mutex poisoned")
            .take()
        {
            tui.clear();
            return Ok(match outcome {
                TreePickerOutcome::Selected(entry_id) => Some(entry_id),
                TreePickerOutcome::Cancelled => None,
            });
        }

        tui.drain_terminal_events()
            .map_err(|error| error.to_string())?;
        sleep(Duration::from_millis(16)).await;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OAuthPickerEntry {
    id: String,
    name: String,
}

#[derive(Debug, Clone)]
enum OAuthPickerOutcome {
    Selected(String),
    Cancelled,
}

struct OAuthPickerComponent {
    keybindings: KeybindingsManager,
    mode: OAuthPickerMode,
    entries: Vec<OAuthPickerEntry>,
    selected_index: usize,
    on_select: Option<Box<dyn FnMut(String) + Send + 'static>>,
    on_cancel: Option<Box<dyn FnMut() + Send + 'static>>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl OAuthPickerComponent {
    fn new(
        keybindings: &KeybindingsManager,
        mode: OAuthPickerMode,
        mut entries: Vec<OAuthPickerEntry>,
    ) -> Self {
        entries.sort_by(|left, right| left.id.cmp(&right.id));
        Self {
            keybindings: keybindings.clone(),
            mode,
            entries,
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

    fn title(&self) -> &'static str {
        match self.mode {
            OAuthPickerMode::Login => "Select provider to login",
            OAuthPickerMode::Logout => "Select provider to logout",
        }
    }

    fn max_visible(&self) -> usize {
        self.viewport_size
            .get()
            .map(|(_, height)| height.saturating_sub(4).max(1))
            .unwrap_or(10)
    }

    fn render_entries(&self, width: usize) -> Vec<String> {
        if self.entries.is_empty() {
            let message = match self.mode {
                OAuthPickerMode::Login => "No OAuth providers available",
                OAuthPickerMode::Logout => "No OAuth providers logged in. Use /login first.",
            };
            return vec![truncate_to_width(message, width, "...", false)];
        }

        let max_visible = self.max_visible();
        let start_index = self
            .selected_index
            .saturating_sub(max_visible / 2)
            .min(self.entries.len().saturating_sub(max_visible));
        let end_index = (start_index + max_visible).min(self.entries.len());
        let mut lines = Vec::new();

        for (visible_index, entry) in self.entries[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            lines.push(truncate_to_width(
                &format!("{prefix}{} [{}]", entry.id, entry.name),
                width,
                "...",
                false,
            ));
        }

        if start_index > 0 || end_index < self.entries.len() {
            lines.push(truncate_to_width(
                &format!("  ({}/{})", self.selected_index + 1, self.entries.len()),
                width,
                "...",
                false,
            ));
        }

        lines
    }

    fn render_hint_line(&self, width: usize) -> String {
        let styler = ThemedKeyHintStyler;
        let hint = format!(
            "{}  {}  {}",
            key_hint(&self.keybindings, &styler, "tui.select.confirm", "select"),
            key_hint(&self.keybindings, &styler, "tui.select.cancel", "cancel"),
            key_hint(&self.keybindings, &styler, "tui.select.down", "navigate"),
        );
        truncate_to_width(&hint, width, "...", false)
    }
}

impl Component for OAuthPickerComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(self.title(), width, "...", false));
        lines.extend(self.render_entries(width));
        lines.push(self.render_hint_line(width));
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
            if self.entries.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                self.entries.len() - 1
            } else {
                self.selected_index - 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            if self.entries.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index + 1 >= self.entries.len() {
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
                .min(self.entries.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") {
            if let Some(entry) = self.entries.get(self.selected_index)
                && let Some(on_select) = &mut self.on_select
            {
                on_select(entry.id.clone());
            }
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}

async fn select_oauth_provider(
    tui: &mut Tui<LiveInteractiveTerminal>,
    keybindings: &KeybindingsManager,
    mode: OAuthPickerMode,
    entries: Vec<OAuthPickerEntry>,
) -> Result<Option<String>, String> {
    let outcome = Arc::new(Mutex::new(None::<OAuthPickerOutcome>));
    let mut picker = OAuthPickerComponent::new(keybindings, mode, entries);

    let outcome_for_select = Arc::clone(&outcome);
    picker.set_on_select(move |provider_id| {
        *outcome_for_select
            .lock()
            .expect("oauth picker outcome mutex poisoned") =
            Some(OAuthPickerOutcome::Selected(provider_id));
    });

    let outcome_for_cancel = Arc::clone(&outcome);
    picker.set_on_cancel(move || {
        *outcome_for_cancel
            .lock()
            .expect("oauth picker outcome mutex poisoned") = Some(OAuthPickerOutcome::Cancelled);
    });

    let picker_id = tui.add_child(Box::new(picker));
    let _ = tui.set_focus_child(picker_id);
    tui.start().map_err(|error| error.to_string())?;

    loop {
        if let Some(outcome) = outcome
            .lock()
            .expect("oauth picker outcome mutex poisoned")
            .take()
        {
            tui.clear();
            return Ok(match outcome {
                OAuthPickerOutcome::Selected(provider_id) => Some(provider_id),
                OAuthPickerOutcome::Cancelled => None,
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

#[derive(Default)]
struct NoopExternalEditorHost;

impl ExternalEditorHost for NoopExternalEditorHost {}

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

pub async fn run_rpc_command(options: RunCommandOptions) -> i32 {
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let stderr = Arc::new(Mutex::new(std::io::stderr()));
    let stdout_emitter: TextEmitter = Arc::new(move |text| {
        use std::io::Write as _;

        let mut stdout = stdout.lock().expect("rpc stdout mutex poisoned");
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    });
    let stderr_emitter: TextEmitter = Arc::new(move |text| {
        use std::io::Write as _;

        let mut stderr = stderr.lock().expect("rpc stderr mutex poisoned");
        let _ = stderr.write_all(text.as_bytes());
        let _ = stderr.flush();
    });

    run_rpc_command_live_with_terminal_factory(
        options,
        stdout_emitter,
        stderr_emitter,
        Arc::new(|| Box::new(ProcessTerminal::new())),
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
        let result = run_command_with_terminal_factory(
            RunCommandOptions {
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
            },
            runtime.terminal_factory.clone(),
        )
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
        let result = run_command_with_terminal_factory(
            RunCommandOptions {
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
            },
            runtime.terminal_factory.clone(),
        )
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
    let mut scoped_models_override = None;
    let mut runtime_settings_override = None;
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

        let default_system_prompt_for_iteration = resolve_interactive_default_system_prompt(
            &default_system_prompt,
            &current_cwd,
            agent_dir.as_deref(),
            &parsed_for_iteration,
        );

        let outcome = run_interactive_iteration(InteractiveIterationOptions {
            parsed: parsed_for_iteration,
            stdin_is_tty,
            stdin_content: stdin_content_for_iteration,
            auth_source: auth_source.clone(),
            built_in_models: built_in_models.clone(),
            models_json_path: models_json_path.clone(),
            agent_dir: agent_dir.clone(),
            cwd: current_cwd.clone(),
            default_system_prompt: default_system_prompt_for_iteration,
            version: version.clone(),
            stream_options: stream_options.clone(),
            runtime: runtime.clone(),
            manager_override: manager_override.take(),
            show_resume_picker: use_initial_input && initial_show_resume_picker,
            prefill_input: prefill_input.take(),
            initial_status_message: initial_status_message.take(),
            bootstrap_defaults: bootstrap_defaults.take(),
            scoped_models_override: scoped_models_override.take(),
            runtime_settings_override: runtime_settings_override.take(),
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
        scoped_models_override = Some(plan.scoped_models);
        runtime_settings_override = Some(plan.runtime_settings);
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
        scoped_models_override,
        runtime_settings_override,
    } = options;

    let runtime_settings = runtime_settings_override.unwrap_or_else(|| {
        agent_dir
            .as_deref()
            .map(|agent_dir| load_runtime_settings(&cwd, agent_dir))
            .unwrap_or_default()
    });
    let shared_runtime_settings = Arc::new(Mutex::new(runtime_settings.clone()));
    eprint!("{}", render_settings_warnings(&runtime_settings.warnings));
    let resources = load_cli_resources(&parsed, &cwd, agent_dir.as_deref());
    for warning in &resources.warnings {
        eprintln!("{warning}");
    }
    set_registered_themes(resources.themes.clone());
    let theme_result = init_theme(runtime_settings.settings.theme.as_deref());
    if let Some(error) = theme_result.error.as_deref() {
        eprintln!("Warning: {error}");
    }
    let (selected_tool_names, selected_tools) = build_selected_tools(
        &parsed,
        &cwd,
        runtime_settings.settings.images.auto_resize_images,
    );

    let scoped_models = if let Some(scoped_models_override) = scoped_models_override {
        scoped_models_override
    } else if let Some(patterns) = parsed.models.as_ref() {
        let registry = ModelRegistry::new(
            auth_source.clone(),
            built_in_models.clone(),
            models_json_path.clone(),
        );
        let resolved = resolve_model_scope(patterns, &registry.get_available());
        eprint!("{}", render_scope_warnings(&resolved.warnings));
        resolved.scoped_models
    } else if let Some(patterns) = runtime_settings.settings.enabled_models.as_ref() {
        if patterns.is_empty() {
            Vec::new()
        } else {
            let registry = ModelRegistry::new(
                auth_source.clone(),
                built_in_models.clone(),
                models_json_path.clone(),
            );
            let resolved = resolve_model_scope(patterns, &registry.get_available());
            eprint!("{}", render_scope_warnings(&resolved.warnings));
            resolved.scoped_models
        }
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
    let mut initial_message = build_initial_message(
        &mut messages,
        (!processed_files.text.is_empty()).then_some(processed_files.text),
        processed_files.images,
        stdin_content,
    );
    initial_message.initial_message = initial_message
        .initial_message
        .as_deref()
        .map(|message| preprocess_prompt_text(message, &resources));
    messages = messages
        .iter()
        .map(|message| preprocess_prompt_text(message, &resources))
        .collect();

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
    apply_runtime_transport_preference(&mut stream_options, &parsed, &runtime_settings);

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(overlay_auth),
        built_in_models,
        models_json_path: models_json_path.clone(),
        cwd: Some(cwd.clone()),
        tools: Some(selected_tools),
        system_prompt: build_runtime_system_prompt(
            &default_system_prompt,
            &parsed,
            &cwd,
            agent_dir.as_deref(),
            &selected_tool_names,
            &resources,
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
        &ThemedKeyHintStyler,
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
            &resources,
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
    let footer_provider = FooterDataProvider::new(&cwd);
    let render_handle = tui.render_handle();
    if let Some(command) = runtime.extension_editor_command.clone() {
        shell.set_extension_editor_command(command);
    }
    if let Some(runner) = runtime.extension_editor_runner.clone() {
        shell.set_extension_editor_command_runner_arc(runner);
    }
    let interactive_host = terminal.external_editor_host(render_handle.clone());
    shell.set_extension_editor_host(interactive_host.clone());
    let slash_command_context = InteractiveSlashCommandContext {
        keybindings: keybindings.clone(),
        runtime_settings: shared_runtime_settings.clone(),
        cwd: cwd.clone(),
        agent_dir: agent_dir.clone(),
        ui_host: Arc::new(interactive_host),
        auth_operation_in_progress: Arc::new(AtomicBool::new(false)),
    };
    shell.bind_footer_data_provider_with_render_handle(&footer_provider, render_handle.clone());
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, render_handle.clone());
    let status_handle = shell.status_handle_with_render_handle(render_handle.clone());
    let footer_state_handle = shell.footer_state_handle_with_render_handle(render_handle.clone());
    update_interactive_footer_state(
        &footer_state_handle,
        &created.core,
        interactive_session_manager.as_ref(),
    );
    footer_state_handle.update(|footer_state| {
        footer_state.auto_compact_enabled = runtime_settings.settings.compaction.enabled;
    });
    let auto_compaction_binding = install_interactive_auto_compaction(
        &created.core,
        interactive_session_manager.as_ref(),
        &status_handle,
        &footer_state_handle,
        shared_runtime_settings.clone(),
    );
    install_interactive_submit_handler(
        &mut shell,
        created.core.clone(),
        created.core.model_registry(),
        interactive_scoped_models.clone(),
        interactive_session_manager.clone(),
        slash_command_context,
        status_handle,
        footer_state_handle,
        Arc::clone(&exit_requested),
        Arc::clone(&transition_request),
        resources.clone(),
        render_handle.clone(),
    );
    let shell_id = tui.add_child(Box::new(shell));
    let _ = tui.set_focus_child(shell_id);
    let _ = tui.terminal_mut().set_title("pi");

    if let Err(error) = tui.start() {
        let _ = tui.stop();
        eprintln!("Error: {error}");
        drop(auto_compaction_binding);
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
    let available_models = created.core.model_registry().get_available();
    let _ = tui
        .terminal_mut()
        .drain_input(Duration::from_millis(1000), Duration::from_millis(50));
    let _ = tui.stop();
    drop(auto_compaction_binding);
    drop(binding);
    drop(tui);
    drop(created);
    drop(interactive_session_manager);

    let session_context = if transition.is_some() {
        build_interactive_session_context(
            session_support,
            &final_state,
            &cwd,
            interactive_scoped_models,
            available_models,
            runtime_settings,
        )
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

fn resolve_interactive_default_system_prompt(
    default_system_prompt: &str,
    cwd: &Path,
    agent_dir: Option<&Path>,
    parsed: &Args,
) -> String {
    let Some(agent_dir) = agent_dir else {
        return default_system_prompt.to_owned();
    };

    finalize_system_prompt(build_default_pi_system_prompt(
        cwd,
        agent_dir,
        parsed.system_prompt.as_deref(),
        parsed.append_system_prompt.as_deref(),
    ))
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
    current_cwd: &Path,
    scoped_models: Vec<ScopedModel>,
    available_models: Vec<Model>,
    runtime_settings: LoadedRuntimeSettings,
) -> Option<InteractiveSessionContext> {
    let snapshot_cwd = current_cwd.to_string_lossy().into_owned();

    let (manager, session_file, session_dir, cwd) = if let Some(session_support) = session_support {
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
            .and_then(|manager| manager.into_inner().ok())
            .or_else(|| {
                session_file
                    .is_none()
                    .then(|| snapshot_session_manager(&cwd, state))
            });

        (manager, session_file, session_dir, cwd)
    } else {
        (
            Some(snapshot_session_manager(&snapshot_cwd, state)),
            None,
            None,
            snapshot_cwd,
        )
    };

    Some(InteractiveSessionContext {
        manager,
        session_file,
        session_dir,
        cwd,
        model: state.model.clone(),
        thinking_level: state.thinking_level,
        scoped_models,
        available_models,
        runtime_settings,
    })
}

fn snapshot_session_manager(cwd: &str, state: &pi_agent::AgentState) -> SessionManager {
    let mut manager = SessionManager::in_memory(cwd);
    let _ = manager.append_model_change(state.model.provider.clone(), state.model.id.clone());
    let _ = manager.append_thinking_level_change(thinking_level_label(state.thinking_level));
    for message in &state.messages {
        let _ = manager.append_message(message.clone());
    }
    manager
}

fn restore_session_manager(context: InteractiveSessionContext) -> Result<SessionManager, String> {
    restore_session_manager_from_parts(
        context.manager,
        context.session_file,
        context.session_dir,
        &context.cwd,
    )
}

fn restore_session_manager_from_parts(
    manager: Option<SessionManager>,
    session_file: Option<String>,
    session_dir: Option<String>,
    cwd: &str,
) -> Result<SessionManager, String> {
    if let Some(manager) = manager {
        return Ok(manager);
    }

    if let Some(session_file) = session_file {
        return SessionManager::open(&session_file, session_dir.as_deref(), None)
            .map_err(|error| error.to_string());
    }

    Ok(SessionManager::in_memory(cwd))
}

struct PreservedInteractiveContext {
    manager: Option<SessionManager>,
    cwd: PathBuf,
    bootstrap_defaults: Option<BootstrapDefaults>,
    scoped_models: Vec<ScopedModel>,
    runtime_settings: LoadedRuntimeSettings,
}

fn preserve_interactive_context(
    session_context: Option<InteractiveSessionContext>,
    current_cwd: &Path,
) -> Result<PreservedInteractiveContext, String> {
    match session_context {
        Some(context) => {
            let manager = restore_session_manager_from_parts(
                context.manager,
                context.session_file,
                context.session_dir,
                &context.cwd,
            )?;
            Ok(PreservedInteractiveContext {
                manager: Some(manager),
                cwd: PathBuf::from(context.cwd),
                bootstrap_defaults: Some(BootstrapDefaults::from_model(
                    &context.model,
                    context.thinking_level,
                )),
                scoped_models: context.scoped_models,
                runtime_settings: context.runtime_settings,
            })
        }
        None => Ok(PreservedInteractiveContext {
            manager: None,
            cwd: current_cwd.to_path_buf(),
            bootstrap_defaults: None,
            scoped_models: Vec::new(),
            runtime_settings: LoadedRuntimeSettings::default(),
        }),
    }
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
            let scoped_models = session_context
                .as_ref()
                .map(|context| context.scoped_models.clone())
                .unwrap_or_default();
            let runtime_settings = session_context
                .as_ref()
                .map(|context| context.runtime_settings.clone())
                .unwrap_or_default();
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
                scoped_models,
                runtime_settings,
            })
        }
        InteractiveTransitionRequest::ResumePicker => {
            let current_context = session_context;
            let current_cwd_string = current_context
                .as_ref()
                .map(|context| context.cwd.clone())
                .unwrap_or_else(|| current_cwd.to_string_lossy().into_owned());
            let current_runtime_settings = current_context
                .as_ref()
                .map(|context| context.runtime_settings.clone())
                .unwrap_or_default();
            let current_scoped_models = current_context
                .as_ref()
                .map(|context| context.scoped_models.clone())
                .unwrap_or_default();
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
                    let next_cwd = PathBuf::from(manager.get_cwd());
                    let runtime_settings = agent_dir
                        .map(|agent_dir| load_runtime_settings(&next_cwd, agent_dir))
                        .unwrap_or(current_runtime_settings);
                    Ok(InteractiveTransitionPlan {
                        cwd: next_cwd,
                        manager: Some(manager),
                        prefill_input: None,
                        initial_status_message: Some(String::from("Resumed session")),
                        bootstrap_defaults: None,
                        scoped_models: current_scoped_models,
                        runtime_settings,
                    })
                }
                None => {
                    let (manager, cwd, scoped_models, runtime_settings) = match current_context {
                        Some(context) => {
                            let cwd = PathBuf::from(&context.cwd);
                            (
                                Some(restore_session_manager_from_parts(
                                    context.manager,
                                    context.session_file,
                                    context.session_dir,
                                    &context.cwd,
                                )?),
                                cwd,
                                context.scoped_models,
                                context.runtime_settings,
                            )
                        }
                        None => (
                            None,
                            current_cwd.to_path_buf(),
                            Vec::new(),
                            LoadedRuntimeSettings::default(),
                        ),
                    };
                    Ok(InteractiveTransitionPlan {
                        cwd,
                        manager,
                        prefill_input: None,
                        initial_status_message: None,
                        bootstrap_defaults: None,
                        scoped_models,
                        runtime_settings,
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
            let scoped_models = session_context.scoped_models.clone();
            let runtime_settings = session_context.runtime_settings.clone();
            let mut manager = restore_session_manager(session_context)?;
            let candidates = collect_fork_candidates(&manager);
            if candidates.is_empty() {
                return Ok(InteractiveTransitionPlan {
                    cwd: PathBuf::from(manager.get_cwd()),
                    manager: Some(manager),
                    prefill_input: None,
                    initial_status_message: Some(String::from("No messages to fork from")),
                    bootstrap_defaults: None,
                    scoped_models,
                    runtime_settings,
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
                            scoped_models,
                            runtime_settings,
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
                scoped_models,
                runtime_settings,
            })
        }
        InteractiveTransitionRequest::TreePicker => {
            let session_context =
                session_context.ok_or_else(|| String::from("Session data unavailable"))?;
            let scoped_models = session_context.scoped_models.clone();
            let runtime_settings = session_context.runtime_settings.clone();
            let mut manager = restore_session_manager(session_context)?;
            if manager.get_entries().is_empty() {
                return Ok(InteractiveTransitionPlan {
                    cwd: PathBuf::from(manager.get_cwd()),
                    manager: Some(manager),
                    prefill_input: None,
                    initial_status_message: Some(String::from("No entries in session")),
                    bootstrap_defaults: None,
                    scoped_models,
                    runtime_settings,
                });
            }

            let current_selection = manager
                .get_leaf_id()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| String::from(ROOT_TREE_ENTRY_ID));
            let items = build_tree_picker_items(&manager);
            let keybindings = create_keybindings_manager(agent_dir);
            let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
            let mut tui = Tui::new(terminal);
            let selected_entry = match select_tree_entry(
                &mut tui,
                &keybindings,
                items,
                Some(current_selection.as_str()),
            )
            .await?
            {
                Some(entry_id) => entry_id,
                None => {
                    return Ok(InteractiveTransitionPlan {
                        cwd: PathBuf::from(manager.get_cwd()),
                        manager: Some(manager),
                        prefill_input: None,
                        initial_status_message: None,
                        bootstrap_defaults: None,
                        scoped_models,
                        runtime_settings,
                    });
                }
            };

            let initial_status_message = if selected_entry == current_selection {
                String::from("Already at this point")
            } else {
                if selected_entry == ROOT_TREE_ENTRY_ID {
                    manager.reset_leaf();
                } else {
                    manager
                        .branch(&selected_entry)
                        .map_err(|error| error.to_string())?;
                }
                String::from("Navigated to selected point")
            };

            Ok(InteractiveTransitionPlan {
                cwd: PathBuf::from(manager.get_cwd()),
                manager: Some(manager),
                prefill_input: None,
                initial_status_message: Some(initial_status_message),
                bootstrap_defaults: None,
                scoped_models,
                runtime_settings,
            })
        }
        InteractiveTransitionRequest::SettingsPicker => {
            let session_context =
                session_context.ok_or_else(|| String::from("Session data unavailable"))?;
            let keybindings = create_keybindings_manager(agent_dir);
            let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
            let mut tui = Tui::new(terminal);
            let selection = select_settings(
                &mut tui,
                &keybindings,
                &session_context.runtime_settings,
                agent_dir.is_some(),
            )
            .await?;

            let current_cwd = PathBuf::from(&session_context.cwd);
            let runtime_settings = if let Some(agent_dir) = agent_dir {
                persist_runtime_settings_changes(
                    &agent_dir.join("settings.json"),
                    &session_context.runtime_settings,
                    &selection,
                )?;
                load_runtime_settings(&current_cwd, agent_dir)
            } else {
                apply_settings_selection(&session_context.runtime_settings, &selection)
            };

            let changed =
                selection != settings_selection_from_loaded(&session_context.runtime_settings);
            let initial_status_message = Some(if changed {
                if agent_dir.is_some() {
                    String::from("Updated settings")
                } else {
                    String::from("Updated session settings")
                }
            } else {
                String::from("Settings unchanged")
            });

            let manager = restore_session_manager_from_parts(
                session_context.manager,
                session_context.session_file,
                session_context.session_dir,
                &session_context.cwd,
            )?;
            Ok(InteractiveTransitionPlan {
                cwd: PathBuf::from(manager.get_cwd()),
                manager: Some(manager),
                prefill_input: None,
                initial_status_message,
                bootstrap_defaults: Some(BootstrapDefaults::from_model(
                    &session_context.model,
                    session_context.thinking_level,
                )),
                scoped_models: session_context.scoped_models,
                runtime_settings,
            })
        }
        InteractiveTransitionRequest::OAuthPicker(mode) => {
            let preserved = preserve_interactive_context(session_context, current_cwd)?;
            let Some(agent_dir) = agent_dir else {
                let message = match mode {
                    OAuthPickerMode::Login => "OAuth login requires an agent directory.",
                    OAuthPickerMode::Logout => "OAuth logout requires an agent directory.",
                };
                return Ok(InteractiveTransitionPlan {
                    cwd: preserved.cwd,
                    manager: preserved.manager,
                    prefill_input: None,
                    initial_status_message: Some(String::from(message)),
                    bootstrap_defaults: preserved.bootstrap_defaults,
                    scoped_models: preserved.scoped_models,
                    runtime_settings: preserved.runtime_settings,
                });
            };

            let auth_path = agent_dir.join("auth.json");
            let entries = match mode {
                OAuthPickerMode::Login => {
                    let entries = oauth_provider_summaries()
                        .into_iter()
                        .map(|provider| OAuthPickerEntry {
                            id: provider.id,
                            name: provider.name,
                        })
                        .collect::<Vec<_>>();
                    if entries.is_empty() {
                        return Ok(InteractiveTransitionPlan {
                            cwd: preserved.cwd,
                            manager: preserved.manager,
                            prefill_input: None,
                            initial_status_message: Some(String::from(
                                "No OAuth providers available",
                            )),
                            bootstrap_defaults: preserved.bootstrap_defaults,
                            scoped_models: preserved.scoped_models,
                            runtime_settings: preserved.runtime_settings,
                        });
                    }
                    entries
                }
                OAuthPickerMode::Logout => {
                    let providers = match list_persisted_oauth_providers(&auth_path) {
                        Ok(providers) => providers,
                        Err(error) => {
                            return Ok(InteractiveTransitionPlan {
                                cwd: preserved.cwd,
                                manager: preserved.manager,
                                prefill_input: None,
                                initial_status_message: Some(format!("Error: {error}")),
                                bootstrap_defaults: preserved.bootstrap_defaults,
                                scoped_models: preserved.scoped_models,
                                runtime_settings: preserved.runtime_settings,
                            });
                        }
                    };
                    if providers.is_empty() {
                        return Ok(InteractiveTransitionPlan {
                            cwd: preserved.cwd,
                            manager: preserved.manager,
                            prefill_input: None,
                            initial_status_message: Some(String::from(
                                "No OAuth providers logged in. Use /login first.",
                            )),
                            bootstrap_defaults: preserved.bootstrap_defaults,
                            scoped_models: preserved.scoped_models,
                            runtime_settings: preserved.runtime_settings,
                        });
                    }
                    providers
                        .into_iter()
                        .map(|provider_id| OAuthPickerEntry {
                            name: oauth_provider_name(&provider_id)
                                .unwrap_or_else(|| provider_id.clone()),
                            id: provider_id,
                        })
                        .collect::<Vec<_>>()
                }
            };

            let keybindings = create_keybindings_manager(Some(agent_dir));
            let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
            let mut tui = Tui::new(terminal);
            let selected = select_oauth_provider(&mut tui, &keybindings, mode, entries).await?;
            let _ = tui.stop();

            let initial_status_message = match selected {
                Some(provider_id) => match mode {
                    OAuthPickerMode::Login => {
                        match run_terminal_oauth_login(
                            auth_path.clone(),
                            provider_id,
                            Arc::new(NoopExternalEditorHost),
                        )
                        .await
                        {
                            Ok(provider_name) => Some(format!("Logged in to {provider_name}")),
                            Err(error) => Some(format!("Error: {error}")),
                        }
                    }
                    OAuthPickerMode::Logout => {
                        let provider_name = oauth_provider_name(&provider_id)
                            .unwrap_or_else(|| provider_id.clone());
                        match remove_persisted_oauth_provider(&auth_path, &provider_id) {
                            Ok(true) => Some(format!("Logged out of {provider_name}")),
                            Ok(false) => {
                                Some(format!("No OAuth credentials stored for {provider_id}"))
                            }
                            Err(error) => Some(format!("Error: {error}")),
                        }
                    }
                },
                None => None,
            };

            Ok(InteractiveTransitionPlan {
                cwd: preserved.cwd,
                manager: preserved.manager,
                prefill_input: None,
                initial_status_message,
                bootstrap_defaults: preserved.bootstrap_defaults,
                scoped_models: preserved.scoped_models,
                runtime_settings: preserved.runtime_settings,
            })
        }
        InteractiveTransitionRequest::ScopedModelsPicker { initial_search } => {
            let session_context =
                session_context.ok_or_else(|| String::from("Session data unavailable"))?;
            let manager = restore_session_manager_from_parts(
                session_context.manager,
                session_context.session_file,
                session_context.session_dir,
                &session_context.cwd,
            )?;
            if session_context.available_models.is_empty() {
                return Ok(InteractiveTransitionPlan {
                    cwd: PathBuf::from(manager.get_cwd()),
                    manager: Some(manager),
                    prefill_input: None,
                    initial_status_message: Some(String::from("No models available")),
                    bootstrap_defaults: Some(BootstrapDefaults::from_model(
                        &session_context.model,
                        session_context.thinking_level,
                    )),
                    scoped_models: session_context.scoped_models,
                    runtime_settings: session_context.runtime_settings,
                });
            }

            let keybindings = create_keybindings_manager(agent_dir);
            let terminal = LiveInteractiveTerminal::new((runtime.terminal_factory)());
            let mut tui = Tui::new(terminal);
            let selection = select_scoped_models(
                &mut tui,
                &keybindings,
                session_context.available_models.clone(),
                &session_context.scoped_models,
                initial_search.as_deref(),
                agent_dir.is_some(),
            )
            .await?;

            if let Some(agent_dir) = agent_dir
                && let Some(persisted) = selection.persisted.as_ref()
            {
                persist_enabled_models_setting(&agent_dir.join("settings.json"), persisted)?;
            }

            let next_scoped_models = scoped_models_from_enabled_ids(
                &session_context.available_models,
                selection.enabled_ids.as_deref(),
            );
            let initial_status_message = Some(scoped_models_status_message(
                &session_context.scoped_models,
                &next_scoped_models,
                selection.persisted.as_ref(),
                agent_dir.is_some(),
            ));

            Ok(InteractiveTransitionPlan {
                cwd: PathBuf::from(manager.get_cwd()),
                manager: Some(manager),
                prefill_input: None,
                initial_status_message,
                bootstrap_defaults: Some(BootstrapDefaults::from_model(
                    &session_context.model,
                    session_context.thinking_level,
                )),
                scoped_models: next_scoped_models,
                runtime_settings: session_context.runtime_settings,
            })
        }
        InteractiveTransitionRequest::Reload => {
            let session_context =
                session_context.ok_or_else(|| String::from("Session data unavailable"))?;
            let manager = restore_session_manager_from_parts(
                session_context.manager,
                session_context.session_file,
                session_context.session_dir,
                &session_context.cwd,
            )?;
            let cwd = PathBuf::from(manager.get_cwd());
            let runtime_settings = agent_dir
                .map(|agent_dir| load_runtime_settings(&cwd, agent_dir))
                .unwrap_or(session_context.runtime_settings.clone());

            Ok(InteractiveTransitionPlan {
                cwd,
                manager: Some(manager),
                prefill_input: None,
                initial_status_message: Some(String::from(
                    "Reloaded keybindings, skills, prompts, and settings",
                )),
                bootstrap_defaults: Some(BootstrapDefaults::from_model(
                    &session_context.model,
                    session_context.thinking_level,
                )),
                scoped_models: session_context.scoped_models,
                runtime_settings,
            })
        }
    }
}

fn settings_selection_from_loaded(loaded: &LoadedRuntimeSettings) -> SettingsPickerSelection {
    SettingsPickerSelection {
        auto_compact: loaded.settings.compaction.enabled,
        auto_resize_images: loaded.settings.images.auto_resize_images,
        block_images: loaded.settings.images.block_images,
        editor_padding_x: loaded.settings.editor_padding_x,
        autocomplete_max_visible: loaded.settings.autocomplete_max_visible,
    }
}

fn apply_settings_selection(
    loaded: &LoadedRuntimeSettings,
    selection: &SettingsPickerSelection,
) -> LoadedRuntimeSettings {
    let mut next = loaded.clone();
    next.settings.compaction.enabled = selection.auto_compact;
    next.settings.images.auto_resize_images = selection.auto_resize_images;
    next.settings.images.block_images = selection.block_images;
    next.settings.editor_padding_x = selection.editor_padding_x;
    next.settings.autocomplete_max_visible = selection.autocomplete_max_visible;
    next
}

fn persist_runtime_settings_changes(
    path: &Path,
    loaded: &LoadedRuntimeSettings,
    selection: &SettingsPickerSelection,
) -> Result<(), String> {
    let previous = settings_selection_from_loaded(loaded);
    if previous == *selection {
        return Ok(());
    }

    update_settings_file(path, |config| {
        if selection.auto_compact != previous.auto_compact {
            set_nested_bool(config, "compaction", "enabled", selection.auto_compact);
        }
        if selection.auto_resize_images != previous.auto_resize_images {
            set_nested_bool(config, "images", "autoResize", selection.auto_resize_images);
        }
        if selection.block_images != previous.block_images {
            set_nested_bool(config, "images", "blockImages", selection.block_images);
        }
        if selection.editor_padding_x != previous.editor_padding_x {
            set_usize(config, "editorPaddingX", selection.editor_padding_x);
        }
        if selection.autocomplete_max_visible != previous.autocomplete_max_visible {
            set_usize(
                config,
                "autocompleteMaxVisible",
                selection.autocomplete_max_visible,
            );
        }
    })
}

fn persist_enabled_models_setting(
    path: &Path,
    persisted: &PersistedScopedModels,
) -> Result<(), String> {
    update_settings_file(path, |config| match persisted {
        PersistedScopedModels::AllEnabled => {
            config.remove("enabledModels");
        }
        PersistedScopedModels::Explicit(enabled_models) => {
            config.insert(
                String::from("enabledModels"),
                Value::Array(enabled_models.iter().cloned().map(Value::String).collect()),
            );
        }
    })
}

fn update_settings_file(
    path: &Path,
    update: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<(), String> {
    let mut config = load_settings_file(path)?;
    update(&mut config);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let rendered =
        serde_json::to_string_pretty(&Value::Object(config)).map_err(|error| error.to_string())?;
    fs::write(path, format!("{rendered}\n")).map_err(|error| error.to_string())
}

fn load_settings_file(path: &Path) -> Result<serde_json::Map<String, Value>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::Map::new());
        }
        Err(error) => return Err(error.to_string()),
    };

    match serde_json::from_str::<Value>(&raw).map_err(|error| error.to_string())? {
        Value::Object(config) => Ok(config),
        _ => Err(format!("{} must contain a JSON object", path.display())),
    }
}

fn set_nested_bool(
    config: &mut serde_json::Map<String, Value>,
    parent_key: &str,
    child_key: &str,
    value: bool,
) {
    ensure_json_object(config, parent_key).insert(child_key.to_owned(), Value::Bool(value));
}

fn set_usize(config: &mut serde_json::Map<String, Value>, key: &str, value: usize) {
    config.insert(key.to_owned(), Value::from(value as u64));
}

fn ensure_json_object<'a>(
    config: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, Value> {
    if !matches!(config.get(key), Some(Value::Object(_))) {
        config.insert(key.to_owned(), Value::Object(serde_json::Map::new()));
    }

    match config.get_mut(key) {
        Some(Value::Object(object)) => object,
        _ => unreachable!("settings parent entry should be an object"),
    }
}

fn scoped_models_status_message(
    previous: &[ScopedModel],
    next: &[ScopedModel],
    persisted: Option<&PersistedScopedModels>,
    can_persist: bool,
) -> String {
    let changed = !same_scoped_models(previous, next);
    let mut message = if changed {
        if next.is_empty() {
            String::from("Model scope cleared")
        } else {
            String::from("Updated session model scope")
        }
    } else {
        String::from("Model scope unchanged")
    };

    if persisted.is_some() {
        if can_persist {
            if changed {
                message.push_str(" and saved to settings");
            } else {
                message = String::from("Saved model scope to settings");
            }
        } else if !changed {
            message = String::from("Settings unavailable; model scope unchanged");
        }
    }

    message
}

fn same_scoped_models(left: &[ScopedModel], right: &[ScopedModel]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.model.provider == right.model.provider
                && left.model.id == right.model.id
                && left.thinking_level == right.thinking_level
        })
}

fn scoped_models_from_enabled_ids(
    models: &[Model],
    enabled_ids: Option<&[String]>,
) -> Vec<ScopedModel> {
    let Some(enabled_ids) = enabled_ids else {
        return Vec::new();
    };
    if enabled_ids.is_empty() || enabled_ids.len() >= models.len() {
        return Vec::new();
    }

    let models_by_id = models
        .iter()
        .map(|model| (full_model_id(model), model.clone()))
        .collect::<BTreeMap<_, _>>();

    enabled_ids
        .iter()
        .filter_map(|id| models_by_id.get(id).cloned())
        .map(|model| ScopedModel {
            model,
            thinking_level: None,
        })
        .collect()
}

fn full_model_id(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

#[derive(Debug, Clone)]
enum SettingsPickerOutcome {
    Closed(SettingsPickerSelection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsPickerItem {
    AutoCompact,
    AutoResizeImages,
    BlockImages,
    EditorPadding,
    AutocompleteMaxVisible,
}

struct SettingsPickerComponent {
    keybindings: KeybindingsManager,
    selection: SettingsPickerSelection,
    selected_index: usize,
    on_close: Option<Box<dyn FnMut(SettingsPickerSelection) + Send + 'static>>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl SettingsPickerComponent {
    const ITEMS: [SettingsPickerItem; 5] = [
        SettingsPickerItem::AutoCompact,
        SettingsPickerItem::AutoResizeImages,
        SettingsPickerItem::BlockImages,
        SettingsPickerItem::EditorPadding,
        SettingsPickerItem::AutocompleteMaxVisible,
    ];

    fn new(keybindings: &KeybindingsManager, selection: SettingsPickerSelection) -> Self {
        Self {
            keybindings: keybindings.clone(),
            selection,
            selected_index: 0,
            on_close: None,
            viewport_size: Cell::new(None),
        }
    }

    fn set_on_close<F>(&mut self, on_close: F)
    where
        F: FnMut(SettingsPickerSelection) + Send + 'static,
    {
        self.on_close = Some(Box::new(on_close));
    }

    fn current_item(&self) -> SettingsPickerItem {
        Self::ITEMS[self.selected_index]
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn close(&mut self) {
        if let Some(on_close) = &mut self.on_close {
            on_close(self.selection.clone());
        }
    }

    fn cycle_selected_value(&mut self, delta: isize) {
        match self.current_item() {
            SettingsPickerItem::AutoCompact => {
                self.selection.auto_compact = !self.selection.auto_compact;
            }
            SettingsPickerItem::AutoResizeImages => {
                self.selection.auto_resize_images = !self.selection.auto_resize_images;
            }
            SettingsPickerItem::BlockImages => {
                self.selection.block_images = !self.selection.block_images;
            }
            SettingsPickerItem::EditorPadding => {
                const VALUES: [usize; 4] = [0, 1, 2, 3];
                self.selection.editor_padding_x =
                    cycle_usize_value(self.selection.editor_padding_x, &VALUES, delta);
            }
            SettingsPickerItem::AutocompleteMaxVisible => {
                const VALUES: [usize; 6] = [3, 5, 7, 10, 15, 20];
                self.selection.autocomplete_max_visible =
                    cycle_usize_value(self.selection.autocomplete_max_visible, &VALUES, delta);
            }
        }
    }

    fn item_label(item: SettingsPickerItem) -> &'static str {
        match item {
            SettingsPickerItem::AutoCompact => "Auto-compact",
            SettingsPickerItem::AutoResizeImages => "Auto-resize images",
            SettingsPickerItem::BlockImages => "Block images",
            SettingsPickerItem::EditorPadding => "Editor padding",
            SettingsPickerItem::AutocompleteMaxVisible => "Autocomplete max items",
        }
    }

    fn item_description(item: SettingsPickerItem) -> &'static str {
        match item {
            SettingsPickerItem::AutoCompact => {
                "Automatically compact the session context when it gets too large"
            }
            SettingsPickerItem::AutoResizeImages => {
                "Resize large images to 2000x2000 before sending them to models"
            }
            SettingsPickerItem::BlockImages => {
                "Prevent images from being sent to the selected model"
            }
            SettingsPickerItem::EditorPadding => {
                "Horizontal padding for the interactive editor (0-3 columns)"
            }
            SettingsPickerItem::AutocompleteMaxVisible => {
                "Maximum visible autocomplete items (3-20)"
            }
        }
    }

    fn item_value(&self, item: SettingsPickerItem) -> String {
        match item {
            SettingsPickerItem::AutoCompact => on_off_label(self.selection.auto_compact),
            SettingsPickerItem::AutoResizeImages => on_off_label(self.selection.auto_resize_images),
            SettingsPickerItem::BlockImages => on_off_label(self.selection.block_images),
            SettingsPickerItem::EditorPadding => self.selection.editor_padding_x.to_string(),
            SettingsPickerItem::AutocompleteMaxVisible => {
                self.selection.autocomplete_max_visible.to_string()
            }
        }
    }

    fn max_visible(&self) -> usize {
        self.viewport_size
            .get()
            .map(|(_, height)| height.saturating_sub(7).max(1))
            .unwrap_or(Self::ITEMS.len())
    }

    fn render_items(&self, width: usize) -> Vec<String> {
        let max_visible = self.max_visible();
        let start_index = self
            .selected_index
            .saturating_sub(max_visible / 2)
            .min(Self::ITEMS.len().saturating_sub(max_visible));
        let end_index = (start_index + max_visible).min(Self::ITEMS.len());
        let mut lines = Vec::new();

        for (visible_index, item) in Self::ITEMS[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let line = format!(
                "{prefix}{:<24} {}",
                Self::item_label(*item),
                self.item_value(*item)
            );
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        lines
    }

    fn render_hint_line(&self, width: usize) -> String {
        let styler = ThemedKeyHintStyler;
        let hint = format!(
            "{}  {}  {}  {}",
            key_hint(&self.keybindings, &styler, "tui.select.up", "navigate"),
            key_hint(
                &self.keybindings,
                &styler,
                "tui.editor.cursorLeft",
                "change"
            ),
            key_hint(&self.keybindings, &styler, "tui.select.confirm", "toggle"),
            key_hint(&self.keybindings, &styler, "tui.select.cancel", "back"),
        );
        truncate_to_width(&hint, width, "...", false)
    }
}

impl Component for SettingsPickerComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let selected = self.current_item();
        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width("Settings", width, "...", false));
        lines.push(truncate_to_width(
            "Saved to global settings when available. Escape closes.",
            width,
            "...",
            false,
        ));
        lines.extend(self.render_items(width));
        lines.push(String::new());
        lines.push(truncate_to_width(
            Self::item_description(selected),
            width,
            "...",
            false,
        ));
        lines.push(self.render_hint_line(width));
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        if self.matches_binding(data, "tui.select.cancel") {
            self.close();
            return;
        }

        if self.matches_binding(data, "tui.select.up") {
            self.selected_index = self.selected_index.saturating_sub(1);
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            self.selected_index =
                (self.selected_index + 1).min(Self::ITEMS.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.select.pageUp") {
            self.selected_index = self.selected_index.saturating_sub(self.max_visible());
            return;
        }

        if self.matches_binding(data, "tui.select.pageDown") {
            self.selected_index =
                (self.selected_index + self.max_visible()).min(Self::ITEMS.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.editor.cursorLeft") {
            self.cycle_selected_value(-1);
            return;
        }

        if self.matches_binding(data, "tui.editor.cursorRight")
            || self.matches_binding(data, "tui.select.confirm")
        {
            self.cycle_selected_value(1);
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}

async fn select_settings(
    tui: &mut Tui<LiveInteractiveTerminal>,
    keybindings: &KeybindingsManager,
    runtime_settings: &LoadedRuntimeSettings,
    _can_persist: bool,
) -> Result<SettingsPickerSelection, String> {
    let outcome = Arc::new(Mutex::new(None::<SettingsPickerOutcome>));
    let mut picker = SettingsPickerComponent::new(
        keybindings,
        settings_selection_from_loaded(runtime_settings),
    );

    let outcome_for_close = Arc::clone(&outcome);
    picker.set_on_close(move |selection| {
        *outcome_for_close
            .lock()
            .expect("settings picker outcome mutex poisoned") =
            Some(SettingsPickerOutcome::Closed(selection));
    });

    let picker_id = tui.add_child(Box::new(picker));
    let _ = tui.set_focus_child(picker_id);
    tui.start().map_err(|error| error.to_string())?;

    loop {
        if let Some(SettingsPickerOutcome::Closed(selection)) = outcome
            .lock()
            .expect("settings picker outcome mutex poisoned")
            .take()
        {
            tui.clear();
            return Ok(selection);
        }

        tui.drain_terminal_events()
            .map_err(|error| error.to_string())?;
        sleep(Duration::from_millis(16)).await;
    }
}

fn on_off_label(enabled: bool) -> String {
    if enabled {
        String::from("on")
    } else {
        String::from("off")
    }
}

fn cycle_usize_value(current: usize, values: &[usize], delta: isize) -> usize {
    let Some(index) = values.iter().position(|value| *value == current) else {
        return values.first().copied().unwrap_or(current);
    };
    let next_index = if delta < 0 {
        if index == 0 {
            values.len().saturating_sub(1)
        } else {
            index - 1
        }
    } else {
        (index + 1) % values.len().max(1)
    };
    values.get(next_index).copied().unwrap_or(current)
}

#[derive(Debug, Clone)]
enum ScopedModelsPickerOutcome {
    Closed(ScopedModelsPickerSelection),
}

#[derive(Clone)]
struct ScopedModelsPickerItem {
    full_id: String,
    model: Model,
    enabled: bool,
}

struct ScopedModelsPickerComponent {
    keybindings: KeybindingsManager,
    models_by_id: BTreeMap<String, Model>,
    all_ids: Vec<String>,
    enabled_ids: Option<Vec<String>>,
    filtered_items: Vec<ScopedModelsPickerItem>,
    selected_index: usize,
    search_input: Input,
    on_close: Option<Box<dyn FnMut(ScopedModelsPickerSelection) + Send + 'static>>,
    viewport_size: Cell<Option<(usize, usize)>>,
    can_persist: bool,
    dirty: bool,
    saved_snapshot: Option<PersistedScopedModels>,
    focused: bool,
}

impl ScopedModelsPickerComponent {
    fn new(
        keybindings: &KeybindingsManager,
        mut models: Vec<Model>,
        scoped_models: &[ScopedModel],
        initial_search: Option<&str>,
        can_persist: bool,
    ) -> Self {
        models.sort_by(|left, right| {
            left.provider
                .cmp(&right.provider)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut models_by_id = BTreeMap::new();
        let mut all_ids = Vec::new();
        for model in models {
            let full_id = full_model_id(&model);
            models_by_id.insert(full_id.clone(), model);
            all_ids.push(full_id);
        }

        let enabled_ids = if scoped_models.is_empty() {
            None
        } else {
            Some(
                scoped_models
                    .iter()
                    .map(|scoped_model| full_model_id(&scoped_model.model))
                    .collect(),
            )
        };

        let mut search_input = Input::with_keybindings(keybindings.deref().clone());
        if let Some(initial_search) = initial_search {
            search_input.set_value(initial_search);
        }

        let mut picker = Self {
            keybindings: keybindings.clone(),
            models_by_id,
            all_ids,
            enabled_ids,
            filtered_items: Vec::new(),
            selected_index: 0,
            search_input,
            on_close: None,
            viewport_size: Cell::new(None),
            can_persist,
            dirty: false,
            saved_snapshot: None,
            focused: false,
        };
        picker.refresh();
        picker
    }

    fn set_on_close<F>(&mut self, on_close: F)
    where
        F: FnMut(ScopedModelsPickerSelection) + Send + 'static,
    {
        self.on_close = Some(Box::new(on_close));
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn close(&mut self) {
        if let Some(on_close) = &mut self.on_close {
            on_close(ScopedModelsPickerSelection {
                enabled_ids: self.enabled_ids.clone(),
                persisted: self.saved_snapshot.clone(),
            });
        }
    }

    fn build_items(&self) -> Vec<ScopedModelsPickerItem> {
        ordered_scoped_model_ids(self.enabled_ids.as_deref(), &self.all_ids)
            .into_iter()
            .filter_map(|id| {
                self.models_by_id
                    .get(&id)
                    .cloned()
                    .map(|model| ScopedModelsPickerItem {
                        enabled: scoped_model_enabled(self.enabled_ids.as_deref(), &id),
                        full_id: id,
                        model,
                    })
            })
            .collect()
    }

    fn refresh(&mut self) {
        let query = self.search_input.get_value();
        let items = self.build_items();
        self.filtered_items = if query.trim().is_empty() {
            items
        } else {
            fuzzy_filter(&items, query, |item| {
                Cow::Owned(format!("{} {}", item.model.id, item.model.provider))
            })
            .into_iter()
            .cloned()
            .collect()
        };
        self.selected_index = self
            .selected_index
            .min(self.filtered_items.len().saturating_sub(1));
    }

    fn max_visible(&self) -> usize {
        self.viewport_size
            .get()
            .map(|(_, height)| height.saturating_sub(9).max(1))
            .unwrap_or(12)
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn render_hint_lines(&self, width: usize) -> Vec<String> {
        let styler = ThemedKeyHintStyler;
        let mut first = vec![
            key_hint(&self.keybindings, &styler, "tui.select.confirm", "toggle"),
            key_hint(&self.keybindings, &styler, "tui.select.up", "navigate"),
            key_hint(&self.keybindings, &styler, "tui.select.cancel", "back"),
        ];
        if self.can_persist {
            first.push(key_hint(
                &self.keybindings,
                &styler,
                "app.scopedModels.save",
                "save",
            ));
        }
        let second = vec![
            key_hint(
                &self.keybindings,
                &styler,
                "app.scopedModels.enableAll",
                "all",
            ),
            key_hint(
                &self.keybindings,
                &styler,
                "app.scopedModels.clearAll",
                "clear",
            ),
            key_hint(
                &self.keybindings,
                &styler,
                "app.scopedModels.toggleProvider",
                "provider",
            ),
            key_hint(
                &self.keybindings,
                &styler,
                "app.scopedModels.moveUp",
                "reorder",
            ),
        ];

        vec![
            truncate_to_width(&first.join("  "), width, "...", false),
            truncate_to_width(&second.join("  "), width, "...", false),
        ]
    }

    fn footer_status_text(&self) -> String {
        let enabled_count = self
            .enabled_ids
            .as_ref()
            .map(|enabled_ids| enabled_ids.len())
            .unwrap_or(self.all_ids.len());
        let all_enabled = self.enabled_ids.is_none();
        let count_text = if all_enabled {
            String::from("all enabled")
        } else {
            format!("{enabled_count}/{} enabled", self.all_ids.len())
        };
        if self.dirty {
            format!("{count_text} (unsaved)")
        } else {
            count_text
        }
    }

    fn render_model_lines(&self, width: usize) -> Vec<String> {
        if self.filtered_items.is_empty() {
            return vec![truncate_to_width("No matching models", width, "...", false)];
        }

        let max_visible = self.max_visible();
        let start_index = self
            .selected_index
            .saturating_sub(max_visible / 2)
            .min(self.filtered_items.len().saturating_sub(max_visible));
        let end_index = (start_index + max_visible).min(self.filtered_items.len());
        let all_enabled = self.enabled_ids.is_none();
        let mut lines = Vec::new();

        for (visible_index, item) in self.filtered_items[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let status = if all_enabled {
                ""
            } else if item.enabled {
                " ✓"
            } else {
                " ✗"
            };
            let line = format!(
                "{prefix}{} [{}]{}",
                item.model.id, item.model.provider, status
            );
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        if start_index > 0 || end_index < self.filtered_items.len() {
            lines.push(truncate_to_width(
                &format!(
                    "  ({}/{})",
                    self.selected_index + 1,
                    self.filtered_items.len()
                ),
                width,
                "...",
                false,
            ));
        }

        if let Some(selected) = self.filtered_items.get(self.selected_index) {
            lines.push(String::new());
            lines.push(truncate_to_width(
                &format!("  Model Name: {}", selected.model.name),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for ScopedModelsPickerComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width("Scoped Models", width, "...", false));
        lines.push(truncate_to_width(
            if self.can_persist {
                "Session-only until saved. Escape closes."
            } else {
                "Session-only. Settings unavailable. Escape closes."
            },
            width,
            "...",
            false,
        ));
        lines.extend(self.search_input.render(width));
        lines.extend(self.render_model_lines(width));
        lines.push(String::new());
        lines.extend(self.render_hint_lines(width));
        lines.push(truncate_to_width(
            &self.footer_status_text(),
            width,
            "...",
            false,
        ));
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.search_input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if self.matches_binding(data, "tui.select.cancel") {
            self.close();
            return;
        }

        if self.matches_binding(data, "app.clear") {
            if self.search_input.get_value().is_empty() {
                self.close();
            } else {
                self.search_input.clear();
                self.refresh();
            }
            return;
        }

        if self.matches_binding(data, "tui.select.up") {
            if self.filtered_items.is_empty() {
                return;
            }
            self.selected_index = self.selected_index.saturating_sub(1);
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            if self.filtered_items.is_empty() {
                return;
            }
            self.selected_index =
                (self.selected_index + 1).min(self.filtered_items.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.select.pageUp") {
            self.selected_index = self.selected_index.saturating_sub(self.max_visible());
            return;
        }

        if self.matches_binding(data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + self.max_visible())
                .min(self.filtered_items.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "app.scopedModels.moveUp")
            || self.matches_binding(data, "app.scopedModels.moveDown")
        {
            if let Some(item) = self.filtered_items.get(self.selected_index) {
                if scoped_model_enabled(self.enabled_ids.as_deref(), &item.full_id) {
                    let delta = if self.matches_binding(data, "app.scopedModels.moveUp") {
                        -1
                    } else {
                        1
                    };
                    self.enabled_ids = move_scoped_model_id(
                        self.enabled_ids.as_deref(),
                        &self.all_ids,
                        &item.full_id,
                        delta,
                    );
                    self.mark_dirty();
                    self.refresh();
                    if delta < 0 {
                        self.selected_index = self.selected_index.saturating_sub(1);
                    } else {
                        self.selected_index = (self.selected_index + 1)
                            .min(self.filtered_items.len().saturating_sub(1));
                    }
                }
            }
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") {
            if let Some(item) = self.filtered_items.get(self.selected_index) {
                self.enabled_ids =
                    toggle_scoped_model_id(self.enabled_ids.as_deref(), &item.full_id);
                self.mark_dirty();
                self.refresh();
            }
            return;
        }

        if self.matches_binding(data, "app.scopedModels.enableAll") {
            let target_ids = if self.search_input.get_value().trim().is_empty() {
                None
            } else {
                Some(
                    self.filtered_items
                        .iter()
                        .map(|item| item.full_id.clone())
                        .collect::<Vec<_>>(),
                )
            };
            self.enabled_ids = enable_scoped_model_ids(
                self.enabled_ids.as_deref(),
                &self.all_ids,
                target_ids.as_deref(),
            );
            self.mark_dirty();
            self.refresh();
            return;
        }

        if self.matches_binding(data, "app.scopedModels.clearAll") {
            let target_ids = if self.search_input.get_value().trim().is_empty() {
                None
            } else {
                Some(
                    self.filtered_items
                        .iter()
                        .map(|item| item.full_id.clone())
                        .collect::<Vec<_>>(),
                )
            };
            self.enabled_ids = clear_scoped_model_ids(
                self.enabled_ids.as_deref(),
                &self.all_ids,
                target_ids.as_deref(),
            );
            self.mark_dirty();
            self.refresh();
            return;
        }

        if self.matches_binding(data, "app.scopedModels.toggleProvider") {
            if let Some(item) = self.filtered_items.get(self.selected_index) {
                let provider = &item.model.provider;
                let provider_ids = self
                    .all_ids
                    .iter()
                    .filter(|id| {
                        self.models_by_id
                            .get(*id)
                            .is_some_and(|model| model.provider == *provider)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let all_enabled = provider_ids
                    .iter()
                    .all(|id| scoped_model_enabled(self.enabled_ids.as_deref(), id));
                self.enabled_ids = if all_enabled {
                    clear_scoped_model_ids(
                        self.enabled_ids.as_deref(),
                        &self.all_ids,
                        Some(provider_ids.as_slice()),
                    )
                } else {
                    enable_scoped_model_ids(
                        self.enabled_ids.as_deref(),
                        &self.all_ids,
                        Some(provider_ids.as_slice()),
                    )
                };
                self.mark_dirty();
                self.refresh();
            }
            return;
        }

        if self.can_persist && self.matches_binding(data, "app.scopedModels.save") {
            self.saved_snapshot = Some(persisted_scoped_models_snapshot(
                self.enabled_ids.as_deref(),
                &self.all_ids,
            ));
            self.dirty = false;
            return;
        }

        self.search_input.handle_input(data);
        self.refresh();
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.search_input.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.search_input.set_viewport_size(width, 1);
    }
}

async fn select_scoped_models(
    tui: &mut Tui<LiveInteractiveTerminal>,
    keybindings: &KeybindingsManager,
    models: Vec<Model>,
    scoped_models: &[ScopedModel],
    initial_search: Option<&str>,
    can_persist: bool,
) -> Result<ScopedModelsPickerSelection, String> {
    let outcome = Arc::new(Mutex::new(None::<ScopedModelsPickerOutcome>));
    let mut picker = ScopedModelsPickerComponent::new(
        keybindings,
        models,
        scoped_models,
        initial_search,
        can_persist,
    );

    let outcome_for_close = Arc::clone(&outcome);
    picker.set_on_close(move |selection| {
        *outcome_for_close
            .lock()
            .expect("scoped models picker outcome mutex poisoned") =
            Some(ScopedModelsPickerOutcome::Closed(selection));
    });

    let picker_id = tui.add_child(Box::new(picker));
    let _ = tui.set_focus_child(picker_id);
    tui.start().map_err(|error| error.to_string())?;

    loop {
        if let Some(ScopedModelsPickerOutcome::Closed(selection)) = outcome
            .lock()
            .expect("scoped models picker outcome mutex poisoned")
            .take()
        {
            tui.clear();
            return Ok(selection);
        }

        tui.drain_terminal_events()
            .map_err(|error| error.to_string())?;
        sleep(Duration::from_millis(16)).await;
    }
}

fn scoped_model_enabled(enabled_ids: Option<&[String]>, id: &str) -> bool {
    enabled_ids.is_none_or(|enabled_ids| enabled_ids.iter().any(|entry| entry == id))
}

fn toggle_scoped_model_id(enabled_ids: Option<&[String]>, id: &str) -> Option<Vec<String>> {
    match enabled_ids {
        None => Some(vec![id.to_owned()]),
        Some(enabled_ids) => {
            let mut next = enabled_ids.to_vec();
            if let Some(index) = next.iter().position(|entry| entry == id) {
                next.remove(index);
            } else {
                next.push(id.to_owned());
            }
            Some(next)
        }
    }
}

fn enable_scoped_model_ids(
    enabled_ids: Option<&[String]>,
    all_ids: &[String],
    target_ids: Option<&[String]>,
) -> Option<Vec<String>> {
    if enabled_ids.is_none() {
        return None;
    }

    let Some(target_ids) = target_ids else {
        return None;
    };

    let mut next = enabled_ids.map_or_else(Vec::new, |enabled_ids| enabled_ids.to_vec());
    for id in target_ids {
        if !next.iter().any(|entry| entry == id) {
            next.push(id.clone());
        }
    }

    if next.len() >= all_ids.len() {
        None
    } else {
        Some(next)
    }
}

fn clear_scoped_model_ids(
    enabled_ids: Option<&[String]>,
    all_ids: &[String],
    target_ids: Option<&[String]>,
) -> Option<Vec<String>> {
    let targets = target_ids
        .map(|target_ids| target_ids.to_vec())
        .unwrap_or_else(|| all_ids.to_vec());
    if targets.is_empty() {
        return enabled_ids.map(|enabled_ids| enabled_ids.to_vec());
    }

    match enabled_ids {
        None => Some(
            all_ids
                .iter()
                .filter(|id| !targets.iter().any(|target| target == *id))
                .cloned()
                .collect(),
        ),
        Some(enabled_ids) => Some(
            enabled_ids
                .iter()
                .filter(|id| !targets.iter().any(|target| target == *id))
                .cloned()
                .collect(),
        ),
    }
}

fn move_scoped_model_id(
    enabled_ids: Option<&[String]>,
    all_ids: &[String],
    id: &str,
    delta: isize,
) -> Option<Vec<String>> {
    let mut next = enabled_ids.map_or_else(|| all_ids.to_vec(), |enabled_ids| enabled_ids.to_vec());
    let Some(index) = next.iter().position(|entry| entry == id) else {
        return if enabled_ids.is_some() {
            Some(next)
        } else {
            None
        };
    };
    let Some(next_index) = index.checked_add_signed(delta) else {
        return Some(next);
    };
    if next_index >= next.len() {
        return Some(next);
    }
    next.swap(index, next_index);
    Some(next)
}

fn persisted_scoped_models_snapshot(
    enabled_ids: Option<&[String]>,
    all_ids: &[String],
) -> PersistedScopedModels {
    match enabled_ids {
        None => PersistedScopedModels::AllEnabled,
        Some(enabled_ids) if enabled_ids.len() >= all_ids.len() => {
            PersistedScopedModels::AllEnabled
        }
        Some(enabled_ids) => PersistedScopedModels::Explicit(enabled_ids.to_vec()),
    }
}

fn ordered_scoped_model_ids(enabled_ids: Option<&[String]>, all_ids: &[String]) -> Vec<String> {
    let Some(enabled_ids) = enabled_ids else {
        return all_ids.to_vec();
    };

    let enabled = enabled_ids.to_vec();
    let mut ordered = enabled.clone();
    ordered.extend(
        all_ids
            .iter()
            .filter(|id| !enabled.iter().any(|enabled_id| enabled_id == *id))
            .cloned(),
    );
    ordered
}

pub fn finalize_system_prompt(prompt: impl Into<String>) -> String {
    let prompt = prompt.into();
    format!("{FINALIZED_SYSTEM_PROMPT_PREFIX}{prompt}")
}

pub async fn run_command(options: RunCommandOptions) -> RunCommandResult {
    run_command_with_terminal_factory(options, Arc::new(|| Box::new(ProcessTerminal::new()))).await
}

async fn run_command_with_terminal_factory(
    options: RunCommandOptions,
    resume_terminal_factory: InteractiveTerminalFactory,
) -> RunCommandResult {
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

    if let Some(export_path) = parsed.export.as_deref() {
        if !parsed.file_args.is_empty() {
            push_line(&mut stderr, "--export does not accept @file arguments");
            return RunCommandResult {
                exit_code: 1,
                stdout,
                stderr,
            };
        }
        if parsed.messages.len() > 1 {
            push_line(
                &mut stderr,
                "--export accepts at most one optional output path argument",
            );
            return RunCommandResult {
                exit_code: 1,
                stdout,
                stderr,
            };
        }

        match export_session_file_to_html(
            &cwd,
            export_path,
            parsed.messages.first().map(String::as_str),
        ) {
            Ok(path) => {
                push_line(&mut stdout, &path);
                return RunCommandResult {
                    exit_code: 0,
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
        }
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
    if matches!(app_mode, AppMode::Interactive) {
        push_line(&mut stderr, &unsupported_app_mode_message(app_mode));
        return RunCommandResult {
            exit_code: 1,
            stdout,
            stderr,
        };
    }

    let startup_cwd = cwd;
    let prepared_session = match prepare_startup_session(
        &parsed,
        &startup_cwd,
        agent_dir.as_deref(),
        resume_terminal_factory.clone(),
    )
    .await
    {
        Ok(StartupSessionPreparation::Ready(prepared_session)) => prepared_session,
        Ok(StartupSessionPreparation::Cancelled) => {
            push_line(&mut stdout, "No session selected");
            return RunCommandResult {
                exit_code: 0,
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

    if app_mode == AppMode::Rpc {
        return run_rpc_command_buffered(
            RpcPreparedOptions {
                parsed,
                initial_stderr: stderr,
                stdin_content,
                auth_source,
                built_in_models,
                models_json_path,
                agent_dir,
                cwd: prepared_session.runtime_cwd,
                default_system_prompt,
                stream_options,
            },
            prepared_session.session_support,
        )
        .await;
    }

    let print_mode = to_print_output_mode(app_mode).expect("print mode expected");
    let runtime_cwd = prepared_session.runtime_cwd;
    let session_support = prepared_session.session_support;

    let runtime_settings = agent_dir
        .as_deref()
        .map(|agent_dir| load_runtime_settings(&runtime_cwd, agent_dir))
        .unwrap_or_default();
    stderr.push_str(&render_settings_warnings(&runtime_settings.warnings));

    let resources = load_cli_resources(&parsed, &runtime_cwd, agent_dir.as_deref());
    for warning in &resources.warnings {
        push_line(&mut stderr, warning);
    }

    let (selected_tool_names, selected_tools) = build_selected_tools(
        &parsed,
        &runtime_cwd,
        runtime_settings.settings.images.auto_resize_images,
    );

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
        &runtime_cwd,
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
    let mut initial_message = build_initial_message(
        &mut messages,
        (!processed_files.text.is_empty()).then_some(processed_files.text),
        processed_files.images,
        stdin_content,
    );
    initial_message.initial_message = initial_message
        .initial_message
        .as_deref()
        .map(|message| preprocess_prompt_text(message, &resources));
    messages = messages
        .iter()
        .map(|message| preprocess_prompt_text(message, &resources))
        .collect();

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
    apply_runtime_transport_preference(&mut stream_options, &parsed, &runtime_settings);

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(overlay_auth),
        built_in_models,
        models_json_path: models_json_path.clone(),
        cwd: Some(runtime_cwd.clone()),
        tools: Some(selected_tools),
        system_prompt: build_runtime_system_prompt(
            &default_system_prompt,
            &parsed,
            &runtime_cwd,
            agent_dir.as_deref(),
            &selected_tool_names,
            &resources,
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

struct RpcPreparedOptions {
    parsed: Args,
    initial_stderr: String,
    stdin_content: Option<String>,
    auth_source: Arc<dyn AuthSource>,
    built_in_models: Vec<Model>,
    models_json_path: Option<PathBuf>,
    agent_dir: Option<PathBuf>,
    cwd: PathBuf,
    default_system_prompt: String,
    stream_options: StreamOptions,
}

struct RpcState {
    core: CodingAgentCore,
    session_manager: Option<Arc<Mutex<SessionManager>>>,
    cwd: PathBuf,
    scoped_models: Vec<ScopedModel>,
    runtime_settings: LoadedRuntimeSettings,
    resources: LoadedCliResources,
    extension_host: Option<RpcExtensionHost>,
    auto_compaction_enabled: bool,
    auto_retry_enabled: bool,
    is_compacting: Arc<AtomicBool>,
    event_unsubscribe: Option<AgentUnsubscribe>,
    bash_abort_tx: Option<tokio::sync::watch::Sender<bool>>,
}

#[derive(Clone)]
struct RpcSnapshot {
    core: CodingAgentCore,
    session_manager: Option<Arc<Mutex<SessionManager>>>,
    cwd: PathBuf,
    scoped_models: Vec<ScopedModel>,
    runtime_settings: LoadedRuntimeSettings,
    resources: LoadedCliResources,
    extension_host: Option<RpcExtensionHost>,
    auto_compaction_enabled: bool,
    is_compacting: Arc<AtomicBool>,
}

#[derive(Clone)]
struct RpcShared {
    options: Arc<RpcPreparedOptions>,
    state: Arc<Mutex<RpcState>>,
    stdout_emitter: TextEmitter,
    stderr_emitter: TextEmitter,
}

impl RpcShared {
    fn emit_stdout(&self, text: impl Into<String>) {
        (self.stdout_emitter)(text.into());
    }

    fn emit_stderr(&self, text: impl Into<String>) {
        (self.stderr_emitter)(text.into());
    }

    fn emit_json(&self, value: &Value) {
        let line = serde_json::to_string(value).expect("rpc json serialization must succeed");
        self.emit_stdout(format!("{line}\n"));
    }

    fn emit_response(&self, id: Option<&str>, command: &str, data: Option<Value>) {
        self.emit_json(&rpc_success_response(id, command, data));
    }

    fn emit_error(&self, id: Option<&str>, command: &str, message: impl Into<String>) {
        self.emit_json(&rpc_error_response(id, command, message));
    }

    fn snapshot(&self) -> RpcSnapshot {
        let state = self.state.lock().expect("rpc state mutex poisoned");
        RpcSnapshot {
            core: state.core.clone(),
            session_manager: state.session_manager.clone(),
            cwd: state.cwd.clone(),
            scoped_models: state.scoped_models.clone(),
            runtime_settings: state.runtime_settings.clone(),
            resources: state.resources.clone(),
            extension_host: state.extension_host.clone(),
            auto_compaction_enabled: state.auto_compaction_enabled,
            is_compacting: state.is_compacting.clone(),
        }
    }

    fn current_core(&self) -> CodingAgentCore {
        self.state
            .lock()
            .expect("rpc state mutex poisoned")
            .core
            .clone()
    }

    async fn replace_state(&self, mut next: RpcState) {
        attach_rpc_event_subscription(&mut next, self.stdout_emitter.clone());

        let previous_extension_host = {
            let mut state = self.state.lock().expect("rpc state mutex poisoned");
            if let Some(unsubscribe) = state.event_unsubscribe.take() {
                let _ = unsubscribe();
            }
            if let Some(bash_abort_tx) = state.bash_abort_tx.take() {
                let _ = bash_abort_tx.send(true);
            }
            let previous_extension_host = state.extension_host.clone();
            *state = next;
            previous_extension_host
        };

        if let Some(extension_host) = previous_extension_host {
            let _ = extension_host.shutdown().await;
        }
    }

    fn abort_active(&self) {
        let snapshot = self.snapshot();
        snapshot.core.abort();
        if let Some(bash_abort_tx) = self
            .state
            .lock()
            .expect("rpc state mutex poisoned")
            .bash_abort_tx
            .clone()
        {
            let _ = bash_abort_tx.send(true);
        }
    }

    async fn shutdown_extension_host(&self) {
        let extension_host = self
            .state
            .lock()
            .expect("rpc state mutex poisoned")
            .extension_host
            .clone();
        if let Some(extension_host) = extension_host {
            let _ = extension_host.shutdown().await;
        }
    }
}

async fn run_rpc_command_live_with_terminal_factory(
    options: RunCommandOptions,
    stdout_emitter: TextEmitter,
    stderr_emitter: TextEmitter,
    resume_terminal_factory: InteractiveTerminalFactory,
) -> i32 {
    let RunCommandOptions {
        args,
        auth_source,
        built_in_models,
        models_json_path,
        agent_dir,
        cwd,
        default_system_prompt,
        stream_options,
        ..
    } = options;

    let parsed = parse_args(&args);
    let initial_stderr = render_parse_diagnostics(&parsed.diagnostics);
    if !initial_stderr.is_empty() {
        stderr_emitter(initial_stderr.clone());
    }
    if parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind == DiagnosticKind::Error)
    {
        return 1;
    }
    if let Some(message) = unsupported_flag_message(&parsed) {
        stderr_emitter(format!("{message}\n"));
        return 1;
    }

    let prepared_session =
        match prepare_startup_session(&parsed, &cwd, agent_dir.as_deref(), resume_terminal_factory)
            .await
        {
            Ok(StartupSessionPreparation::Ready(prepared_session)) => prepared_session,
            Ok(StartupSessionPreparation::Cancelled) => {
                stdout_emitter(String::from("No session selected\n"));
                return 0;
            }
            Err(error) => {
                stderr_emitter(format!("Error: {error}\n"));
                return 1;
            }
        };

    let prepared = RpcPreparedOptions {
        parsed,
        initial_stderr: String::new(),
        stdin_content: None,
        auth_source,
        built_in_models,
        models_json_path,
        agent_dir,
        cwd: prepared_session.runtime_cwd,
        default_system_prompt,
        stream_options,
    };

    run_rpc_processor(
        prepared,
        false,
        stdout_emitter,
        stderr_emitter,
        prepared_session.session_support,
    )
    .await
}

async fn run_rpc_command_buffered(
    options: RpcPreparedOptions,
    initial_session_support: Option<SessionSupport>,
) -> RunCommandResult {
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(options.initial_stderr.clone()));
    let stdout_emitter: TextEmitter = Arc::new({
        let stdout = stdout.clone();
        move |text| {
            stdout
                .lock()
                .expect("rpc stdout buffer mutex poisoned")
                .push_str(&text);
        }
    });
    let stderr_emitter: TextEmitter = Arc::new({
        let stderr = stderr.clone();
        move |text| {
            stderr
                .lock()
                .expect("rpc stderr buffer mutex poisoned")
                .push_str(&text);
        }
    });

    let exit_code = run_rpc_processor(
        options,
        true,
        stdout_emitter,
        stderr_emitter,
        initial_session_support,
    )
    .await;

    RunCommandResult {
        exit_code,
        stdout: stdout
            .lock()
            .expect("rpc stdout buffer mutex poisoned")
            .clone(),
        stderr: stderr
            .lock()
            .expect("rpc stderr buffer mutex poisoned")
            .clone(),
    }
}

async fn run_rpc_processor(
    options: RpcPreparedOptions,
    wait_for_background_tasks: bool,
    stdout_emitter: TextEmitter,
    stderr_emitter: TextEmitter,
    initial_session_support: Option<SessionSupport>,
) -> i32 {
    let buffered_lines = options.stdin_content.as_ref().map(|stdin_content| {
        stdin_content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    });

    let shared = match create_rpc_shared(
        options,
        initial_session_support,
        stdout_emitter,
        stderr_emitter.clone(),
    )
    .await
    {
        Ok(shared) => shared,
        Err(error_output) => {
            if !error_output.is_empty() {
                stderr_emitter(error_output);
            }
            return 1;
        }
    };

    let mut background_tasks = Vec::<tokio::task::JoinHandle<()>>::new();
    if let Some(buffered_lines) = buffered_lines {
        for line in buffered_lines {
            handle_rpc_input_line(shared.clone(), &line, Some(&mut background_tasks)).await;
        }
    } else {
        use std::io::BufRead as _;

        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let line = line.trim().to_owned();
            if line.is_empty() {
                continue;
            }
            let shared = shared.clone();
            tokio::spawn(async move {
                handle_rpc_input_line(shared, &line, None).await;
            });
        }
    }

    if wait_for_background_tasks {
        for task in background_tasks {
            let _ = task.await;
        }
        shared.current_core().wait_for_idle().await;
    } else {
        shared.abort_active();
    }
    shared.shutdown_extension_host().await;

    0
}

async fn create_rpc_shared(
    options: RpcPreparedOptions,
    initial_session_support: Option<SessionSupport>,
    stdout_emitter: TextEmitter,
    stderr_emitter: TextEmitter,
) -> Result<RpcShared, String> {
    if !options.parsed.file_args.is_empty() {
        return Err(String::from(
            "Error: @file arguments are not supported in RPC mode\n",
        ));
    }

    let mut stderr = String::new();
    let runtime_settings = options
        .agent_dir
        .as_deref()
        .map(|agent_dir| load_runtime_settings(&options.cwd, agent_dir))
        .unwrap_or_default();
    stderr.push_str(&render_settings_warnings(&runtime_settings.warnings));

    let scoped_models = resolve_rpc_scoped_models(&options, &runtime_settings, &mut stderr);
    let (mut state, bootstrap_output) = build_rpc_state(
        &options,
        &options.cwd,
        runtime_settings,
        scoped_models,
        initial_session_support,
        None,
        None,
        String::from("startup"),
        None,
        stdout_emitter.clone(),
        stderr_emitter.clone(),
    )
    .await?;
    stderr.push_str(&bootstrap_output);
    attach_rpc_event_subscription(&mut state, stdout_emitter.clone());

    if !stderr.is_empty() {
        stderr_emitter(stderr);
    }

    Ok(RpcShared {
        options: Arc::new(options),
        state: Arc::new(Mutex::new(state)),
        stdout_emitter,
        stderr_emitter,
    })
}

fn resolve_rpc_scoped_models(
    options: &RpcPreparedOptions,
    runtime_settings: &LoadedRuntimeSettings,
    stderr: &mut String,
) -> Vec<ScopedModel> {
    if let Some(patterns) = options.parsed.models.as_ref() {
        let registry = ModelRegistry::new(
            options.auth_source.clone(),
            options.built_in_models.clone(),
            options.models_json_path.clone(),
        );
        let resolved = resolve_model_scope(patterns, &registry.get_available());
        stderr.push_str(&render_scope_warnings(&resolved.warnings));
        return resolved.scoped_models;
    }

    if let Some(patterns) = runtime_settings.settings.enabled_models.as_ref() {
        if patterns.is_empty() {
            return Vec::new();
        }
        let registry = ModelRegistry::new(
            options.auth_source.clone(),
            options.built_in_models.clone(),
            options.models_json_path.clone(),
        );
        let resolved = resolve_model_scope(patterns, &registry.get_available());
        stderr.push_str(&render_scope_warnings(&resolved.warnings));
        return resolved.scoped_models;
    }

    Vec::new()
}

async fn build_rpc_state(
    options: &RpcPreparedOptions,
    cwd: &Path,
    runtime_settings: LoadedRuntimeSettings,
    scoped_models: Vec<ScopedModel>,
    session_support_override: Option<SessionSupport>,
    manager_override: Option<SessionManager>,
    bootstrap_defaults: Option<BootstrapDefaults>,
    session_start_reason: String,
    previous_session_file: Option<String>,
    stdout_emitter: TextEmitter,
    stderr_emitter: TextEmitter,
) -> Result<(RpcState, String), String> {
    let mut resources = load_cli_resources(&options.parsed, cwd, options.agent_dir.as_deref());
    let mut resource_output = String::new();
    for warning in &resources.warnings {
        push_line(&mut resource_output, warning);
    }
    let (selected_tool_names, selected_tools) = build_selected_tools(
        &options.parsed,
        cwd,
        runtime_settings.settings.images.auto_resize_images,
    );

    let session_support = if let Some(session_support_override) = session_support_override {
        Some(session_support_override)
    } else {
        match manager_override {
            Some(manager_override) => Some(build_session_support(manager_override)),
            None => create_session_support(
                &options.parsed,
                cwd,
                options.agent_dir.as_deref(),
                None,
                None,
            )?,
        }
    };

    let overlay_auth = OverlayAuthSource::new(options.auth_source.clone());
    apply_runtime_api_key_override(
        &options.parsed,
        &overlay_auth,
        &options.built_in_models,
        options.models_json_path.as_deref(),
        &scoped_models,
    )?;

    let mut stream_options = options.stream_options.clone();
    if let Some(session_support) = session_support.as_ref() {
        stream_options.session_id = Some(session_support.session_id.clone());
    }
    apply_runtime_transport_preference(&mut stream_options, &options.parsed, &runtime_settings);

    let default_system_prompt = resolve_interactive_default_system_prompt(
        &options.default_system_prompt,
        cwd,
        options.agent_dir.as_deref(),
        &options.parsed,
    );
    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(overlay_auth),
        built_in_models: options.built_in_models.clone(),
        models_json_path: options.models_json_path.clone(),
        cwd: Some(cwd.to_path_buf()),
        tools: Some(selected_tools),
        system_prompt: build_runtime_system_prompt(
            &default_system_prompt,
            &options.parsed,
            cwd,
            options.agent_dir.as_deref(),
            &selected_tool_names,
            &resources,
        ),
        bootstrap: SessionBootstrapOptions {
            cli_provider: options.parsed.provider.clone(),
            cli_model: options.parsed.model.clone(),
            cli_thinking_level: options.parsed.thinking,
            scoped_models: scoped_models.clone(),
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
            return Err(render_no_models_message(
                options.models_json_path.as_deref(),
            ));
        }
        Err(error) => {
            return Err(format!("Error: {error}\n"));
        }
    };

    if let Some(session_support) = session_support.as_ref() {
        apply_session_support(&created.core, session_support)?;
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
    if created
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == BootstrapDiagnosticLevel::Error)
    {
        return Err(bootstrap_output);
    }

    let session_manager = session_support
        .as_ref()
        .map(|support| support.manager.clone());
    let mut extension_host = None;
    if should_start_extension_host(
        cwd,
        options.agent_dir.as_deref(),
        options.parsed.extensions.as_deref().unwrap_or(&[]),
        options.parsed.no_extensions,
    ) {
        let start_result = RpcExtensionHost::start(RpcExtensionHostStartOptions {
            cwd: cwd.to_path_buf(),
            agent_dir: options.agent_dir.clone(),
            extension_paths: options.parsed.extensions.clone().unwrap_or_default(),
            no_extensions: options.parsed.no_extensions,
            flag_values: unknown_flags_to_json(&options.parsed.unknown_flags),
            state: rpc_extension_state_json(
                &created.core,
                session_manager.as_ref(),
                &resources,
                &[],
            ),
            session_start_reason,
            previous_session_file,
            stdout_emitter,
            stderr_emitter,
        })
        .await?;

        let diagnostics_output = render_extension_diagnostics(&start_result.init.diagnostics);
        if start_result
            .init
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == "error")
        {
            return Err(diagnostics_output);
        }
        resource_output.push_str(&diagnostics_output);

        if let Some(host) = start_result.host {
            let skill_paths = start_result
                .init
                .skill_paths
                .into_iter()
                .map(|entry| ExtensionResourcePath {
                    path: entry.path,
                    extension_path: entry.extension_path,
                })
                .collect::<Vec<_>>();
            let prompt_paths = start_result
                .init
                .prompt_paths
                .into_iter()
                .map(|entry| ExtensionResourcePath {
                    path: entry.path,
                    extension_path: entry.extension_path,
                })
                .collect::<Vec<_>>();
            let theme_paths = start_result
                .init
                .theme_paths
                .into_iter()
                .map(|entry| ExtensionResourcePath {
                    path: entry.path,
                    extension_path: entry.extension_path,
                })
                .collect::<Vec<_>>();
            for warning in extend_cli_resources_from_extensions(
                &mut resources,
                cwd,
                &skill_paths,
                &prompt_paths,
                &theme_paths,
            ) {
                push_line(&mut resource_output, &warning);
            }

            let extension_commands = host.commands();
            let extension_system_prompt = build_runtime_system_prompt(
                &default_system_prompt,
                &options.parsed,
                cwd,
                options.agent_dir.as_deref(),
                &selected_tool_names,
                &resources,
            );
            created.core.agent().update_state(|state| {
                state.system_prompt = extension_system_prompt.clone();
            });
            host.update_state(rpc_extension_state_json(
                &created.core,
                session_manager.as_ref(),
                &resources,
                &extension_commands,
            ))
            .await?;

            let before_tool_call_host = host.clone();
            created
                .core
                .agent()
                .set_before_tool_call(move |context, _signal| {
                    let before_tool_call_host = before_tool_call_host.clone();
                    async move {
                        let input = context
                            .args
                            .lock()
                            .expect("rpc before_tool_call args mutex poisoned")
                            .clone();
                        match before_tool_call_host
                            .tool_call(&context.tool_name, &context.tool_call_id, input)
                            .await
                        {
                            Ok(Some(RpcToolCallResult {
                                block: true,
                                reason,
                            })) => Some(BeforeToolCallResult {
                                block: true,
                                reason,
                            }),
                            Ok(_) => None,
                            Err(error) => Some(BeforeToolCallResult {
                                block: true,
                                reason: Some(format!(
                                    "Extension failed, blocking execution: {error}"
                                )),
                            }),
                        }
                    }
                });
            extension_host = Some(host);
        }
    }

    Ok((
        RpcState {
            core: created.core,
            session_manager,
            cwd: cwd.to_path_buf(),
            scoped_models,
            runtime_settings: runtime_settings.clone(),
            resources,
            extension_host,
            auto_compaction_enabled: runtime_settings.settings.compaction.enabled,
            auto_retry_enabled: true,
            is_compacting: Arc::new(AtomicBool::new(false)),
            event_unsubscribe: None,
            bash_abort_tx: None,
        },
        format!("{resource_output}{bootstrap_output}"),
    ))
}

fn attach_rpc_event_subscription(state: &mut RpcState, stdout_emitter: TextEmitter) {
    let extension_host = state.extension_host.clone();
    let unsubscribe = state.core.agent().subscribe(move |event, _signal| {
        let stdout_emitter = stdout_emitter.clone();
        let extension_host = extension_host.clone();
        Box::pin(async move {
            let event_json = rpc_agent_event_to_json(&event);
            let line =
                serde_json::to_string(&event_json).expect("rpc event serialization must succeed");
            stdout_emitter(format!("{line}\n"));
            if let Some(extension_host) = extension_host {
                let _ = extension_host.emit_event(event_json).await;
            }
        })
    });
    state.event_unsubscribe = Some(unsubscribe);
}

async fn handle_rpc_input_line(
    shared: RpcShared,
    line: &str,
    background_tasks: Option<&mut Vec<tokio::task::JoinHandle<()>>>,
) {
    let parsed = match serde_json::from_str::<Value>(line) {
        Ok(parsed) => parsed,
        Err(error) => {
            shared.emit_error(None, "parse", format!("Failed to parse command: {error}"));
            return;
        }
    };

    let Some(command) = parsed.as_object() else {
        shared.emit_error(None, "parse", "RPC command must be a JSON object");
        return;
    };
    if command
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|command_type| command_type == "extension_ui_response")
    {
        let extension_host = shared.snapshot().extension_host;
        if let Some(extension_host) = extension_host {
            let _ = extension_host.deliver_ui_response(parsed.clone()).await;
        }
        return;
    }

    let id = optional_string_field(command, "id");
    let Some(command_type) = optional_string_field(command, "type") else {
        shared.emit_error(id.as_deref(), "parse", "RPC command is missing type");
        return;
    };

    match command_type.as_str() {
        "prompt" => {
            let message = match required_string_field(command, "message") {
                Ok(message) => message,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "prompt", error);
                    return;
                }
            };
            let images = match parse_rpc_images(command, "images") {
                Ok(images) => images,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "prompt", error);
                    return;
                }
            };
            let streaming_behavior = optional_string_field(command, "streamingBehavior");
            let snapshot = shared.snapshot();
            if let Some((command_name, args)) = parse_rpc_extension_command(&message)
                && let Some(extension_host) = snapshot.extension_host.clone()
                && extension_host.has_command(&command_name)
            {
                let task = spawn_rpc_extension_command_task(
                    shared.clone(),
                    id.clone(),
                    command_name,
                    args,
                );
                if let Some(background_tasks) = background_tasks {
                    background_tasks.push(task);
                }
                shared.emit_response(id.as_deref(), "prompt", None);
                return;
            }

            let prepared_message = preprocess_prompt_text(&message, &snapshot.resources);
            if snapshot.core.state().is_streaming {
                if streaming_behavior.as_deref() == Some("steer") {
                    queue_rpc_message(&snapshot.core, "steer", prepared_message, images);
                    shared.emit_response(id.as_deref(), "prompt", None);
                    return;
                }
                if streaming_behavior.as_deref() == Some("followUp") {
                    queue_rpc_message(&snapshot.core, "follow_up", prepared_message, images);
                    shared.emit_response(id.as_deref(), "prompt", None);
                    return;
                }
            }

            let task = spawn_rpc_prompt_task(shared.clone(), id.clone(), prepared_message, images);
            if let Some(background_tasks) = background_tasks {
                background_tasks.push(task);
            }
            shared.emit_response(id.as_deref(), "prompt", None);
        }
        "steer" => {
            let message = match required_string_field(command, "message") {
                Ok(message) => message,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "steer", error);
                    return;
                }
            };
            let images = match parse_rpc_images(command, "images") {
                Ok(images) => images,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "steer", error);
                    return;
                }
            };
            let snapshot = shared.snapshot();
            if let Some((command_name, _)) = parse_rpc_extension_command(&message)
                && let Some(extension_host) = snapshot.extension_host.clone()
                && extension_host.has_command(&command_name)
            {
                shared.emit_error(
                    id.as_deref(),
                    "steer",
                    "Extension commands cannot be queued with steer",
                );
                return;
            }
            queue_rpc_message(
                &shared.current_core(),
                "steer",
                preprocess_prompt_text(&message, &snapshot.resources),
                images,
            );
            shared.emit_response(id.as_deref(), "steer", None);
        }
        "follow_up" => {
            let message = match required_string_field(command, "message") {
                Ok(message) => message,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "follow_up", error);
                    return;
                }
            };
            let images = match parse_rpc_images(command, "images") {
                Ok(images) => images,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "follow_up", error);
                    return;
                }
            };
            let snapshot = shared.snapshot();
            if let Some((command_name, _)) = parse_rpc_extension_command(&message)
                && let Some(extension_host) = snapshot.extension_host.clone()
                && extension_host.has_command(&command_name)
            {
                shared.emit_error(
                    id.as_deref(),
                    "follow_up",
                    "Extension commands cannot be queued with follow_up",
                );
                return;
            }
            queue_rpc_message(
                &shared.current_core(),
                "follow_up",
                preprocess_prompt_text(&message, &snapshot.resources),
                images,
            );
            shared.emit_response(id.as_deref(), "follow_up", None);
        }
        "abort" => {
            shared.current_core().abort();
            shared.emit_response(id.as_deref(), "abort", None);
        }
        "new_session" => {
            let snapshot = shared.snapshot();
            if snapshot.core.state().is_streaming {
                shared.emit_error(
                    id.as_deref(),
                    "new_session",
                    "Cannot create a new session while a request is running",
                );
                return;
            }
            if let Some(extension_host) = snapshot.extension_host.clone() {
                match extension_host.before_switch("new", None).await {
                    Ok(true) => {
                        shared.emit_response(
                            id.as_deref(),
                            "new_session",
                            Some(json!({ "cancelled": true })),
                        );
                        return;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        shared.emit_error(id.as_deref(), "new_session", error);
                        return;
                    }
                }
            }
            let parent_session = optional_string_field(command, "parentSession");
            let mut manager = match recreate_session_manager_from_rpc(&snapshot) {
                Ok(manager) => manager,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "new_session", error);
                    return;
                }
            };
            manager.new_session(NewSessionOptions {
                id: None,
                parent_session,
            });
            let next_cwd = PathBuf::from(manager.get_cwd());
            match build_rpc_state(
                &shared.options,
                &next_cwd,
                snapshot.runtime_settings.clone(),
                snapshot.scoped_models.clone(),
                None,
                Some(manager),
                Some(BootstrapDefaults::from_model(
                    &snapshot.core.state().model,
                    snapshot.core.state().thinking_level,
                )),
                String::from("new"),
                current_rpc_session_file(snapshot.session_manager.as_ref()),
                shared.stdout_emitter.clone(),
                shared.stderr_emitter.clone(),
            )
            .await
            {
                Ok((next_state, bootstrap_output)) => {
                    if !bootstrap_output.is_empty() {
                        shared.emit_stderr(bootstrap_output);
                    }
                    shared.replace_state(next_state).await;
                    shared.emit_response(
                        id.as_deref(),
                        "new_session",
                        Some(json!({ "cancelled": false })),
                    );
                }
                Err(error) => shared.emit_error(id.as_deref(), "new_session", error.trim()),
            }
        }
        "get_state" => {
            shared.emit_response(
                id.as_deref(),
                "get_state",
                Some(rpc_session_state_json(&shared.snapshot())),
            );
        }
        "set_model" => {
            let provider = match required_string_field(command, "provider") {
                Ok(provider) => provider,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "set_model", error);
                    return;
                }
            };
            let model_id = match required_string_field(command, "modelId") {
                Ok(model_id) => model_id,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "set_model", error);
                    return;
                }
            };
            let snapshot = shared.snapshot();
            let available_models = snapshot.core.model_registry().get_available();
            let Some(model) = available_models
                .into_iter()
                .find(|model| model.provider == provider && model.id == model_id)
            else {
                shared.emit_error(
                    id.as_deref(),
                    "set_model",
                    format!("Model not found: {provider}/{model_id}"),
                );
                return;
            };
            match apply_interactive_model_state(
                &snapshot.core,
                &model,
                None,
                snapshot.session_manager.as_ref(),
            ) {
                Ok(()) => {
                    sync_rpc_extension_state(&shared).await;
                    shared.emit_response(
                        id.as_deref(),
                        "set_model",
                        Some(model_to_rpc_json(&snapshot.core.state().model)),
                    )
                }
                Err(error) => shared.emit_error(id.as_deref(), "set_model", error),
            }
        }
        "cycle_model" => {
            let snapshot = shared.snapshot();
            match cycle_interactive_model(
                &snapshot.core,
                snapshot.core.model_registry().as_ref(),
                &snapshot.scoped_models,
                snapshot.session_manager.as_ref(),
                "forward",
            ) {
                Ok(Some(result)) => {
                    sync_rpc_extension_state(&shared).await;
                    shared.emit_response(
                        id.as_deref(),
                        "cycle_model",
                        Some(json!({
                            "model": model_to_rpc_json(&result.model),
                            "thinkingLevel": thinking_level_label(result.thinking_level),
                            "isScoped": !snapshot.scoped_models.is_empty(),
                        })),
                    )
                }
                Ok(None) => shared.emit_response(id.as_deref(), "cycle_model", Some(Value::Null)),
                Err(error) => shared.emit_error(id.as_deref(), "cycle_model", error),
            }
        }
        "get_available_models" => {
            let models = shared.current_core().model_registry().get_available();
            shared.emit_response(
                id.as_deref(),
                "get_available_models",
                Some(json!({
                    "models": models.iter().map(model_to_rpc_json).collect::<Vec<_>>()
                })),
            );
        }
        "set_thinking_level" => {
            let level = match command
                .get("level")
                .and_then(Value::as_str)
                .and_then(parse_thinking_level)
            {
                Some(level) => level,
                None => {
                    shared.emit_error(
                        id.as_deref(),
                        "set_thinking_level",
                        "Invalid thinking level",
                    );
                    return;
                }
            };
            let snapshot = shared.snapshot();
            let model = snapshot.core.state().model;
            match apply_interactive_model_state(
                &snapshot.core,
                &model,
                Some(level),
                snapshot.session_manager.as_ref(),
            ) {
                Ok(()) => {
                    sync_rpc_extension_state(&shared).await;
                    shared.emit_response(id.as_deref(), "set_thinking_level", None)
                }
                Err(error) => shared.emit_error(id.as_deref(), "set_thinking_level", error),
            }
        }
        "cycle_thinking_level" => {
            let snapshot = shared.snapshot();
            match cycle_rpc_thinking_level(&snapshot.core, snapshot.session_manager.as_ref()) {
                Ok(Some(level)) => {
                    sync_rpc_extension_state(&shared).await;
                    shared.emit_response(
                        id.as_deref(),
                        "cycle_thinking_level",
                        Some(json!({ "level": thinking_level_label(level) })),
                    )
                }
                Ok(None) => {
                    shared.emit_response(id.as_deref(), "cycle_thinking_level", Some(Value::Null))
                }
                Err(error) => shared.emit_error(id.as_deref(), "cycle_thinking_level", error),
            }
        }
        "set_steering_mode" => {
            let mode = match command
                .get("mode")
                .and_then(Value::as_str)
                .and_then(queue_mode_from_str)
            {
                Some(mode) => mode,
                None => {
                    shared.emit_error(id.as_deref(), "set_steering_mode", "Invalid queue mode");
                    return;
                }
            };
            shared.current_core().agent().set_steering_mode(mode);
            shared.emit_response(id.as_deref(), "set_steering_mode", None);
        }
        "set_follow_up_mode" => {
            let mode = match command
                .get("mode")
                .and_then(Value::as_str)
                .and_then(queue_mode_from_str)
            {
                Some(mode) => mode,
                None => {
                    shared.emit_error(id.as_deref(), "set_follow_up_mode", "Invalid queue mode");
                    return;
                }
            };
            shared.current_core().agent().set_follow_up_mode(mode);
            shared.emit_response(id.as_deref(), "set_follow_up_mode", None);
        }
        "compact" => {
            let snapshot = shared.snapshot();
            let Some(session_manager) = snapshot.session_manager.as_ref() else {
                shared.emit_error(
                    id.as_deref(),
                    "compact",
                    "Session compaction is unavailable",
                );
                return;
            };
            if snapshot.core.state().is_streaming {
                shared.emit_error(
                    id.as_deref(),
                    "compact",
                    "Wait for the current response to finish before compacting",
                );
                return;
            }
            if snapshot.is_compacting.swap(true, Ordering::Relaxed) {
                shared.emit_error(id.as_deref(), "compact", "Compaction is already running");
                return;
            }
            let custom_instructions = optional_string_field(command, "customInstructions");
            let settings = runtime_compaction_settings(&snapshot.runtime_settings);
            let result = run_interactive_compaction(
                &snapshot.core,
                session_manager,
                &settings,
                custom_instructions.as_deref(),
            )
            .await;
            snapshot.is_compacting.store(false, Ordering::Relaxed);
            match result {
                Ok(Some(result)) => shared.emit_response(
                    id.as_deref(),
                    "compact",
                    Some(compaction_result_to_json(&result)),
                ),
                Ok(None) => shared.emit_error(id.as_deref(), "compact", "Nothing to compact"),
                Err(error) => shared.emit_error(id.as_deref(), "compact", error),
            }
        }
        "set_auto_compaction" => {
            let enabled = command
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            {
                let mut state = shared.state.lock().expect("rpc state mutex poisoned");
                state.auto_compaction_enabled = enabled;
                state.runtime_settings.settings.compaction.enabled = enabled;
            }
            shared.emit_response(id.as_deref(), "set_auto_compaction", None);
        }
        "set_auto_retry" => {
            let enabled = command
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            shared
                .state
                .lock()
                .expect("rpc state mutex poisoned")
                .auto_retry_enabled = enabled;
            shared.emit_response(id.as_deref(), "set_auto_retry", None);
        }
        "abort_retry" => {
            shared.emit_response(id.as_deref(), "abort_retry", None);
        }
        "bash" => {
            let command_text = match required_string_field(command, "command") {
                Ok(command_text) => command_text,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "bash", error);
                    return;
                }
            };
            let cwd = shared.snapshot().cwd;
            let abort_rx = {
                let mut state = shared.state.lock().expect("rpc state mutex poisoned");
                if state.bash_abort_tx.is_some() {
                    shared.emit_error(id.as_deref(), "bash", "A bash command is already running");
                    return;
                }
                let (abort_tx, abort_rx) = tokio::sync::watch::channel(false);
                state.bash_abort_tx = Some(abort_tx);
                abort_rx
            };

            let tool = pi_coding_agent_tools::create_bash_tool(cwd);
            let result = tool
                .execute(
                    String::from("rpc-bash"),
                    json!({ "command": command_text }),
                    Some(abort_rx),
                )
                .await;
            shared
                .state
                .lock()
                .expect("rpc state mutex poisoned")
                .bash_abort_tx = None;
            match result {
                Ok(result) => shared.emit_response(
                    id.as_deref(),
                    "bash",
                    Some(agent_tool_result_to_rpc_bash_json(&result)),
                ),
                Err(error) => shared.emit_error(id.as_deref(), "bash", error.to_string()),
            }
        }
        "abort_bash" => {
            if let Some(abort_tx) = shared
                .state
                .lock()
                .expect("rpc state mutex poisoned")
                .bash_abort_tx
                .clone()
            {
                let _ = abort_tx.send(true);
            }
            shared.emit_response(id.as_deref(), "abort_bash", None);
        }
        "get_session_stats" => {
            shared.emit_response(
                id.as_deref(),
                "get_session_stats",
                Some(rpc_session_stats_json(&shared.snapshot())),
            );
        }
        "export_html" => {
            let snapshot = shared.snapshot();
            let Some(session_manager) = snapshot.session_manager.as_ref() else {
                shared.emit_error(
                    id.as_deref(),
                    "export_html",
                    "Session export is unavailable",
                );
                return;
            };
            let output_path = optional_string_field(command, "outputPath");
            match export_interactive_session(session_manager, &snapshot.cwd, output_path.as_deref())
            {
                Ok(path) => shared.emit_response(
                    id.as_deref(),
                    "export_html",
                    Some(json!({ "path": path })),
                ),
                Err(error) => shared.emit_error(id.as_deref(), "export_html", error),
            }
        }
        "switch_session" => {
            let snapshot = shared.snapshot();
            if snapshot.core.state().is_streaming {
                shared.emit_error(
                    id.as_deref(),
                    "switch_session",
                    "Cannot switch sessions while a request is running",
                );
                return;
            }
            let session_path = match required_string_field(command, "sessionPath") {
                Ok(session_path) => session_path,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "switch_session", error);
                    return;
                }
            };
            let session_dir = snapshot
                .session_manager
                .as_ref()
                .and_then(current_rpc_session_dir);
            let manager = match SessionManager::open(
                &resolve_session_path(&snapshot.cwd, &session_path),
                session_dir.as_deref(),
                None,
            ) {
                Ok(manager) => manager,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "switch_session", error.to_string());
                    return;
                }
            };
            let next_cwd = PathBuf::from(manager.get_cwd());
            let next_runtime_settings = shared
                .options
                .agent_dir
                .as_deref()
                .map(|agent_dir| load_runtime_settings(&next_cwd, agent_dir))
                .unwrap_or(snapshot.runtime_settings.clone());
            match build_rpc_state(
                &shared.options,
                &next_cwd,
                next_runtime_settings,
                snapshot.scoped_models.clone(),
                None,
                Some(manager),
                None,
                String::from("resume"),
                current_rpc_session_file(snapshot.session_manager.as_ref()),
                shared.stdout_emitter.clone(),
                shared.stderr_emitter.clone(),
            )
            .await
            {
                Ok((next_state, bootstrap_output)) => {
                    if !bootstrap_output.is_empty() {
                        shared.emit_stderr(bootstrap_output);
                    }
                    shared.replace_state(next_state).await;
                    shared.emit_response(
                        id.as_deref(),
                        "switch_session",
                        Some(json!({ "cancelled": false })),
                    );
                }
                Err(error) => shared.emit_error(id.as_deref(), "switch_session", error.trim()),
            }
        }
        "fork" => {
            let snapshot = shared.snapshot();
            if snapshot.core.state().is_streaming {
                shared.emit_error(
                    id.as_deref(),
                    "fork",
                    "Cannot fork while a request is running",
                );
                return;
            }
            let entry_id = match required_string_field(command, "entryId") {
                Ok(entry_id) => entry_id,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "fork", error);
                    return;
                }
            };
            let mut manager = match recreate_session_manager_from_rpc(&snapshot) {
                Ok(manager) => manager,
                Err(error) => {
                    shared.emit_error(id.as_deref(), "fork", error);
                    return;
                }
            };
            let candidates = collect_fork_candidates(&manager);
            let Some(selected) = candidates
                .into_iter()
                .find(|candidate| candidate.entry_id == entry_id)
            else {
                shared.emit_error(id.as_deref(), "fork", "Invalid entry ID for forking");
                return;
            };
            let bootstrap_defaults = if let Some(parent_id) = selected.parent_id.as_deref() {
                if let Err(error) = manager.create_branched_session(parent_id) {
                    shared.emit_error(id.as_deref(), "fork", error.to_string());
                    return;
                }
                None
            } else {
                manager.new_session(NewSessionOptions {
                    id: None,
                    parent_session: manager.get_session_file().map(ToOwned::to_owned),
                });
                Some(BootstrapDefaults::from_model(
                    &snapshot.core.state().model,
                    snapshot.core.state().thinking_level,
                ))
            };
            let next_cwd = PathBuf::from(manager.get_cwd());
            match build_rpc_state(
                &shared.options,
                &next_cwd,
                snapshot.runtime_settings.clone(),
                snapshot.scoped_models.clone(),
                None,
                Some(manager),
                bootstrap_defaults,
                String::from("fork"),
                current_rpc_session_file(snapshot.session_manager.as_ref()),
                shared.stdout_emitter.clone(),
                shared.stderr_emitter.clone(),
            )
            .await
            {
                Ok((next_state, bootstrap_output)) => {
                    if !bootstrap_output.is_empty() {
                        shared.emit_stderr(bootstrap_output);
                    }
                    shared.replace_state(next_state).await;
                    shared.emit_response(
                        id.as_deref(),
                        "fork",
                        Some(json!({ "text": selected.text, "cancelled": false })),
                    );
                }
                Err(error) => shared.emit_error(id.as_deref(), "fork", error.trim()),
            }
        }
        "get_fork_messages" => {
            let snapshot = shared.snapshot();
            let messages = snapshot
                .session_manager
                .as_ref()
                .map(|session_manager| {
                    let session_manager = session_manager
                        .lock()
                        .expect("rpc session manager mutex poisoned");
                    collect_fork_candidates(&session_manager)
                        .into_iter()
                        .map(|candidate| {
                            json!({
                                "entryId": candidate.entry_id,
                                "text": candidate.text,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            shared.emit_response(
                id.as_deref(),
                "get_fork_messages",
                Some(json!({ "messages": messages })),
            );
        }
        "get_last_assistant_text" => {
            shared.emit_response(
                id.as_deref(),
                "get_last_assistant_text",
                Some(json!({ "text": last_assistant_message_text(&shared.current_core()) })),
            );
        }
        "set_session_name" => {
            let name = match required_string_field(command, "name") {
                Ok(name) if !name.trim().is_empty() => name,
                _ => {
                    shared.emit_error(
                        id.as_deref(),
                        "set_session_name",
                        "Session name cannot be empty",
                    );
                    return;
                }
            };
            let snapshot = shared.snapshot();
            let Some(session_manager) = snapshot.session_manager.as_ref() else {
                shared.emit_error(
                    id.as_deref(),
                    "set_session_name",
                    "Session naming is unavailable",
                );
                return;
            };
            let append_result = session_manager
                .lock()
                .expect("rpc session manager mutex poisoned")
                .append_session_info(&name);
            match append_result {
                Ok(_) => {
                    sync_rpc_extension_state(&shared).await;
                    shared.emit_response(id.as_deref(), "set_session_name", None)
                }
                Err(error) => {
                    shared.emit_error(id.as_deref(), "set_session_name", error.to_string())
                }
            }
        }
        "get_messages" => {
            let state = shared.current_core().state();
            shared.emit_response(
                id.as_deref(),
                "get_messages",
                Some(json!({
                    "messages": state
                        .messages
                        .iter()
                        .map(rpc_agent_message_to_json)
                        .collect::<Vec<_>>()
                })),
            );
        }
        "get_commands" => {
            let snapshot = shared.snapshot();
            let mut commands = snapshot
                .extension_host
                .as_ref()
                .map(|extension_host| {
                    extension_host
                        .commands()
                        .into_iter()
                        .map(|command| {
                            json!({
                                "name": command.name,
                                "description": command.description,
                                "source": "extension",
                                "sourceInfo": command.source_info,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            commands.extend(snapshot.resources.prompt_templates.iter().map(|template| {
                json!({
                    "name": template.name,
                    "description": template.description,
                    "source": "prompt",
                    "sourceInfo": template.source_info,
                })
            }));
            commands.extend(snapshot.resources.skills.iter().map(|skill| {
                json!({
                    "name": format!("skill:{}", skill.name),
                    "description": skill.description,
                    "source": "skill",
                    "sourceInfo": skill.source_info,
                })
            }));
            shared.emit_response(
                id.as_deref(),
                "get_commands",
                Some(json!({ "commands": commands })),
            );
        }
        other => shared.emit_error(id.as_deref(), other, format!("Unknown command: {other}")),
    }
}

fn optional_string_field(command: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    command
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn required_string_field(
    command: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, String> {
    optional_string_field(command, key)
        .ok_or_else(|| format!("Missing required string field: {key}"))
}

fn parse_rpc_images(
    command: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<UserContent>, String> {
    let Some(images) = command.get(key) else {
        return Ok(Vec::new());
    };
    let Some(images) = images.as_array() else {
        return Err(format!("Field {key} must be an array"));
    };

    images
        .iter()
        .map(|image| {
            let Some(image) = image.as_object() else {
                return Err(String::from("Image entries must be objects"));
            };
            let data = required_string_field(image, "data")?;
            let mime_type = optional_string_field(image, "mimeType")
                .or_else(|| optional_string_field(image, "mime_type"))
                .ok_or_else(|| String::from("Image entries must include mimeType"))?;
            Ok(UserContent::Image { data, mime_type })
        })
        .collect()
}

fn build_rpc_user_message(text: String, images: Vec<UserContent>) -> Message {
    let mut content = Vec::with_capacity(images.len() + 1);
    content.push(UserContent::Text { text });
    content.extend(images);
    Message::User {
        content,
        timestamp: now_ms(),
    }
}

fn queue_rpc_message(core: &CodingAgentCore, kind: &str, text: String, images: Vec<UserContent>) {
    let message = build_rpc_user_message(text, images);
    if kind == "follow_up" {
        core.agent().follow_up(message);
    } else {
        core.agent().steer(message);
    }
}

fn spawn_rpc_prompt_task(
    shared: RpcShared,
    id: Option<String>,
    message: String,
    images: Vec<UserContent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let core = shared.current_core();
        let result = if images.is_empty() {
            core.prompt_text(message)
                .await
                .map_err(|error| error.to_string())
        } else {
            core.prompt_message(build_rpc_user_message(message, images))
                .await
                .map_err(|error| error.to_string())
        };
        if let Err(error) = result {
            shared.emit_error(id.as_deref(), "prompt", error);
        }
    })
}

fn parse_rpc_extension_command(text: &str) -> Option<(String, String)> {
    let command = text.strip_prefix('/')?;
    let (name, args) = match command.split_once(' ') {
        Some((name, args)) => (name, args.trim()),
        None => (command, ""),
    };
    (!name.is_empty()).then(|| (name.to_owned(), args.to_owned()))
}

fn spawn_rpc_extension_command_task(
    shared: RpcShared,
    id: Option<String>,
    command_name: String,
    args: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let extension_host = shared.snapshot().extension_host;
        let Some(extension_host) = extension_host else {
            return;
        };
        match extension_host.execute_command(&command_name, &args).await {
            Ok(true) => {}
            Ok(false) => {
                shared.emit_error(
                    id.as_deref(),
                    "prompt",
                    format!("Unknown extension command: /{command_name}"),
                );
            }
            Err(error) => shared.emit_error(id.as_deref(), "prompt", error),
        }
    })
}

async fn sync_rpc_extension_state(shared: &RpcShared) {
    let snapshot = shared.snapshot();
    let Some(extension_host) = snapshot.extension_host else {
        return;
    };
    let _ = extension_host
        .update_state(rpc_extension_state_json(
            &snapshot.core,
            snapshot.session_manager.as_ref(),
            &snapshot.resources,
            &extension_host.commands(),
        ))
        .await;
}

fn current_rpc_session_file(
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
) -> Option<String> {
    session_manager.and_then(|session_manager| {
        session_manager
            .lock()
            .expect("rpc session manager mutex poisoned")
            .get_session_file()
            .map(str::to_owned)
    })
}

fn unknown_flags_to_json(flags: &BTreeMap<String, crate::UnknownFlagValue>) -> Value {
    let mut values = serde_json::Map::new();
    for (name, value) in flags {
        let value = match value {
            crate::UnknownFlagValue::Bool(value) => Value::Bool(*value),
            crate::UnknownFlagValue::String(value) => Value::String(value.clone()),
        };
        values.insert(name.clone(), value);
    }
    Value::Object(values)
}

fn render_extension_diagnostics(diagnostics: &[rpc_extensions::RpcExtensionDiagnostic]) -> String {
    let mut output = String::new();
    for diagnostic in diagnostics {
        let label = match diagnostic.level.as_str() {
            "error" => "Error",
            _ => "Warning",
        };
        push_line(&mut output, &format!("{label}: {}", diagnostic.message));
    }
    output
}

fn rpc_extension_state_json(
    core: &CodingAgentCore,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    resources: &LoadedCliResources,
    extension_commands: &[RpcExtensionCommandInfo],
) -> Value {
    let state = core.state();
    let active_tools = state
        .tools
        .iter()
        .map(|tool| Value::String(tool.definition.name.clone()))
        .collect::<Vec<_>>();
    let mut commands = extension_commands
        .iter()
        .map(|command| {
            json!({
                "name": command.name,
                "description": command.description,
                "source": "extension",
                "sourceInfo": command.source_info,
            })
        })
        .collect::<Vec<_>>();
    commands.extend(resources.prompt_templates.iter().map(|template| {
        json!({
            "name": template.name,
            "description": template.description,
            "source": "prompt",
            "sourceInfo": template.source_info,
        })
    }));
    commands.extend(resources.skills.iter().map(|skill| {
        json!({
            "name": format!("skill:{}", skill.name),
            "description": skill.description,
            "source": "skill",
            "sourceInfo": skill.source_info,
        })
    }));

    json!({
        "model": model_to_rpc_json(&state.model),
        "thinkingLevel": thinking_level_label(state.thinking_level),
        "isIdle": !state.is_streaming,
        "hasPendingMessages": core.agent().has_queued_messages(),
        "systemPrompt": state.system_prompt,
        "sessionName": current_session_name(session_manager),
        "activeTools": active_tools.clone(),
        "allTools": active_tools,
        "commands": commands,
    })
}

fn queue_mode_from_str(value: &str) -> Option<pi_agent::QueueMode> {
    match value {
        "all" => Some(pi_agent::QueueMode::All),
        "one-at-a-time" => Some(pi_agent::QueueMode::OneAtATime),
        _ => None,
    }
}

fn queue_mode_label(mode: pi_agent::QueueMode) -> &'static str {
    match mode {
        pi_agent::QueueMode::All => "all",
        pi_agent::QueueMode::OneAtATime => "one-at-a-time",
    }
}

fn cycle_rpc_thinking_level(
    core: &CodingAgentCore,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
) -> Result<Option<ThinkingLevel>, String> {
    let state = core.state();
    if !state.model.reasoning {
        return Ok(None);
    }

    let levels = if supports_xhigh(&state.model) {
        vec![
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::XHigh,
        ]
    } else {
        vec![
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
        ]
    };
    let current_index = levels
        .iter()
        .position(|level| *level == state.thinking_level)
        .unwrap_or(0);
    let next_level = levels[(current_index + 1) % levels.len()];
    apply_interactive_model_state(core, &state.model, Some(next_level), session_manager)?;
    Ok(Some(next_level))
}

fn recreate_session_manager_from_rpc(snapshot: &RpcSnapshot) -> Result<SessionManager, String> {
    if let Some(session_manager) = snapshot.session_manager.as_ref() {
        let (session_file, session_dir, cwd) = {
            let session_manager = session_manager
                .lock()
                .expect("rpc session manager mutex poisoned");
            (
                session_manager.get_session_file().map(str::to_owned),
                (!session_manager.get_session_dir().is_empty())
                    .then(|| session_manager.get_session_dir().to_owned()),
                session_manager.get_cwd().to_owned(),
            )
        };

        if let Some(session_file) = session_file {
            return SessionManager::open(&session_file, session_dir.as_deref(), None)
                .map_err(|error| error.to_string());
        }

        return Ok(snapshot_session_manager(&cwd, &snapshot.core.state()));
    }

    Ok(snapshot_session_manager(
        &snapshot.cwd.to_string_lossy(),
        &snapshot.core.state(),
    ))
}

fn current_rpc_session_dir(session_manager: &Arc<Mutex<SessionManager>>) -> Option<String> {
    let session_manager = session_manager
        .lock()
        .expect("rpc session manager mutex poisoned");
    (!session_manager.get_session_dir().is_empty())
        .then(|| session_manager.get_session_dir().to_owned())
}

fn rpc_success_response(id: Option<&str>, command: &str, data: Option<Value>) -> Value {
    let mut response = serde_json::Map::new();
    if let Some(id) = id {
        response.insert(String::from("id"), Value::String(id.to_owned()));
    }
    response.insert(
        String::from("type"),
        Value::String(String::from("response")),
    );
    response.insert(String::from("command"), Value::String(command.to_owned()));
    response.insert(String::from("success"), Value::Bool(true));
    if let Some(data) = data {
        response.insert(String::from("data"), data);
    }
    Value::Object(response)
}

fn rpc_error_response(id: Option<&str>, command: &str, message: impl Into<String>) -> Value {
    let mut response = serde_json::Map::new();
    if let Some(id) = id {
        response.insert(String::from("id"), Value::String(id.to_owned()));
    }
    response.insert(
        String::from("type"),
        Value::String(String::from("response")),
    );
    response.insert(String::from("command"), Value::String(command.to_owned()));
    response.insert(String::from("success"), Value::Bool(false));
    response.insert(String::from("error"), Value::String(message.into()));
    Value::Object(response)
}

fn rpc_session_state_json(snapshot: &RpcSnapshot) -> Value {
    let state = snapshot.core.state();
    json!({
        "model": model_to_rpc_json(&state.model),
        "thinkingLevel": thinking_level_label(state.thinking_level),
        "isStreaming": state.is_streaming,
        "isCompacting": snapshot.is_compacting.load(Ordering::Relaxed),
        "steeringMode": queue_mode_label(snapshot.core.agent().steering_mode()),
        "followUpMode": queue_mode_label(snapshot.core.agent().follow_up_mode()),
        "sessionFile": snapshot.session_manager.as_ref().and_then(|session_manager| {
            session_manager
                .lock()
                .expect("rpc session manager mutex poisoned")
                .get_session_file()
                .map(str::to_owned)
        }),
        "sessionId": snapshot.session_manager.as_ref().map(|session_manager| {
            session_manager
                .lock()
                .expect("rpc session manager mutex poisoned")
                .get_session_id()
                .to_owned()
        }).or_else(|| snapshot.core.agent().session_id()).unwrap_or_else(|| String::from("In-memory")),
        "sessionName": current_session_name(snapshot.session_manager.as_ref()),
        "autoCompactionEnabled": snapshot.auto_compaction_enabled,
        "messageCount": state.messages.len(),
        "pendingMessageCount": if snapshot.core.agent().has_queued_messages() { 1 } else { 0 },
    })
}

fn rpc_session_stats_json(snapshot: &RpcSnapshot) -> Value {
    let state = snapshot.core.state();
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

    json!({
        "sessionFile": snapshot.session_manager.as_ref().and_then(|session_manager| {
            session_manager
                .lock()
                .expect("rpc session manager mutex poisoned")
                .get_session_file()
                .map(str::to_owned)
        }),
        "sessionId": snapshot.session_manager.as_ref().map(|session_manager| {
            session_manager
                .lock()
                .expect("rpc session manager mutex poisoned")
                .get_session_id()
                .to_owned()
        }).or_else(|| snapshot.core.agent().session_id()).unwrap_or_else(|| String::from("In-memory")),
        "userMessages": user_messages,
        "assistantMessages": assistant_messages,
        "toolCalls": tool_calls,
        "toolResults": tool_results,
        "totalMessages": state.messages.len(),
        "tokens": {
            "input": total_input,
            "output": total_output,
            "cacheRead": total_cache_read,
            "cacheWrite": total_cache_write,
            "total": total_input + total_output + total_cache_read + total_cache_write,
        },
        "cost": total_cost,
    })
}

fn model_to_rpc_json(model: &Model) -> Value {
    json!({
        "id": model.id,
        "name": model.name,
        "api": model.api,
        "provider": model.provider,
        "baseUrl": model.base_url,
        "reasoning": model.reasoning,
        "input": model.input,
        "cost": model.cost,
        "contextWindow": model.context_window,
        "maxTokens": model.max_tokens,
        "compat": model.compat,
    })
}

fn compaction_result_to_json(result: &CompactionResult) -> Value {
    json!({
        "summary": result.summary,
        "firstKeptEntryId": result.first_kept_entry_id,
        "tokensBefore": result.tokens_before,
        "details": result.details,
    })
}

fn agent_tool_result_to_rpc_bash_json(result: &pi_agent::AgentToolResult) -> Value {
    let output = result
        .content
        .iter()
        .filter_map(|content| match content {
            UserContent::Text { text } => Some(text.as_str()),
            UserContent::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("");
    let full_output_path = result
        .details
        .get("fullOutputPath")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    json!({
        "output": output,
        "exitCode": 0,
        "cancelled": false,
        "truncated": full_output_path.is_some(),
        "fullOutputPath": full_output_path,
    })
}

fn rpc_agent_event_to_json(event: &pi_agent::AgentEvent) -> Value {
    match event {
        pi_agent::AgentEvent::AgentStart => json!({ "type": "agent_start" }),
        pi_agent::AgentEvent::AgentEnd { messages } => json!({
            "type": "agent_end",
            "messages": messages.iter().map(rpc_agent_message_to_json).collect::<Vec<_>>(),
        }),
        pi_agent::AgentEvent::TurnStart => json!({ "type": "turn_start" }),
        pi_agent::AgentEvent::TurnEnd {
            message,
            tool_results,
        } => json!({
            "type": "turn_end",
            "message": serde_json::to_value(message)
                .expect("assistant message serialization must succeed"),
            "toolResults": tool_results,
        }),
        pi_agent::AgentEvent::MessageStart { message } => json!({
            "type": "message_start",
            "message": rpc_agent_message_to_json(message),
        }),
        pi_agent::AgentEvent::MessageUpdate {
            message,
            assistant_event,
        } => json!({
            "type": "message_update",
            "message": rpc_agent_message_to_json(message),
            "assistantEvent": assistant_event,
        }),
        pi_agent::AgentEvent::MessageEnd { message } => json!({
            "type": "message_end",
            "message": rpc_agent_message_to_json(message),
        }),
        pi_agent::AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => json!({
            "type": "tool_execution_start",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
        }),
        pi_agent::AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            tool_name,
            args,
            partial_result,
        } => json!({
            "type": "tool_execution_update",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
            "partialResult": {
                "content": partial_result.content,
                "details": partial_result.details,
            },
        }),
        pi_agent::AgentEvent::ToolExecutionEnd {
            tool_call_id,
            tool_name,
            result,
            is_error,
        } => json!({
            "type": "tool_execution_end",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "result": {
                "content": result.content,
                "details": result.details,
            },
            "isError": is_error,
        }),
    }
}

fn rpc_agent_message_to_json(message: &pi_agent::AgentMessage) -> Value {
    match message {
        pi_agent::AgentMessage::Standard(message) => {
            serde_json::to_value(message).expect("standard rpc message serialization must succeed")
        }
        pi_agent::AgentMessage::Custom(message) => json!({
            "role": message.role,
            "payload": message.payload,
            "timestamp": message.timestamp,
        }),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

fn apply_runtime_transport_preference(
    stream_options: &mut StreamOptions,
    parsed: &Args,
    runtime_settings: &LoadedRuntimeSettings,
) {
    if let Some(transport) = parsed.transport.or(stream_options.transport) {
        stream_options.transport = Some(transport);
    } else if runtime_settings.settings.transport != Transport::Sse {
        stream_options.transport = Some(runtime_settings.settings.transport);
    }
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

#[cfg(test)]
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

    let mut system_prompt = pi_coding_agent_core::resolve_prompt_input(override_system_prompt)
        .unwrap_or_else(|| default_system_prompt.to_string());

    if let Some(append_system_prompt) =
        pi_coding_agent_core::resolve_prompt_input(append_system_prompt)
    {
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

fn runtime_compaction_settings(runtime_settings: &LoadedRuntimeSettings) -> CompactionSettings {
    CompactionSettings {
        enabled: runtime_settings.settings.compaction.enabled,
        reserve_tokens: runtime_settings.settings.compaction.reserve_tokens,
        keep_recent_tokens: runtime_settings.settings.compaction.keep_recent_tokens,
    }
}

async fn run_interactive_compaction(
    core: &CodingAgentCore,
    session_manager: &Arc<Mutex<SessionManager>>,
    settings: &CompactionSettings,
    custom_instructions: Option<&str>,
) -> Result<Option<CompactionResult>, String> {
    let state = core.state();
    let model = state.model.clone();
    let auth = core
        .model_registry()
        .get_api_key_and_headers(&model)
        .map_err(|error| error.to_string())?;
    let Some(api_key) = auth.api_key else {
        return Err(format!("No API key found for {}.", model.provider));
    };

    let path_entries = {
        let session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        let leaf_id = session_manager.get_leaf_id().map(str::to_owned);
        session_manager.get_branch(leaf_id.as_deref())
    };

    let Some(preparation) = prepare_compaction(&path_entries, settings.clone()) else {
        return Ok(None);
    };

    let result = compact(
        &preparation,
        &model,
        &api_key,
        auth.headers,
        custom_instructions,
    )
    .await?;

    let session_context = {
        let mut session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        session_manager
            .append_compaction(
                result.summary.clone(),
                result.first_kept_entry_id.clone(),
                result.tokens_before,
                result.details.clone(),
                None,
            )
            .map_err(|error| error.to_string())?;
        session_manager.build_session_context()
    };

    let next_messages = session_context.messages;
    core.agent().update_state(move |state| {
        state.messages = next_messages.clone();
    });

    Ok(Some(result))
}

fn strip_trailing_error_assistant(core: &CodingAgentCore) {
    core.agent().update_state(|state| {
        let should_strip = state
            .messages
            .last()
            .and_then(|message| message.as_standard_message())
            .is_some_and(|message| {
                matches!(
                    message,
                    Message::Assistant {
                        stop_reason: pi_events::StopReason::Error,
                        ..
                    }
                )
            });
        if should_strip {
            state.messages.pop();
        }
    });
}

async fn maybe_run_auto_compaction(
    core: &CodingAgentCore,
    session_manager: &Arc<Mutex<SessionManager>>,
    footer_state_handle: &FooterStateHandle,
    settings: &CompactionSettings,
    overflow_recovery_attempted: &AtomicBool,
) -> Result<Option<String>, String> {
    if !settings.enabled {
        return Ok(None);
    }

    let state = core.state();
    let Some(assistant) = state
        .messages
        .iter()
        .rev()
        .filter_map(|message| message.as_standard_message())
        .find_map(standard_message_to_assistant)
    else {
        return Ok(None);
    };

    if assistant.stop_reason == pi_events::StopReason::Aborted {
        return Ok(None);
    }

    let same_model =
        assistant.provider == state.model.provider && assistant.model == state.model.id;
    if !same_model {
        return Ok(None);
    }

    let context_window = state.model.context_window;
    if is_context_overflow(&assistant, Some(context_window)) {
        if overflow_recovery_attempted.swap(true, Ordering::Relaxed) {
            return Ok(Some(String::from(
                "Context overflow recovery failed after one compact-and-retry attempt. Try reducing context or switching to a larger-context model.",
            )));
        }

        let compacted = run_interactive_compaction(core, session_manager, settings, None).await?;
        if compacted.is_none() {
            return Ok(None);
        }

        strip_trailing_error_assistant(core);
        update_interactive_footer_state(footer_state_handle, core, Some(session_manager));
        core.continue_turn()
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Some(String::from(
            "Recovering from context overflow with compaction...",
        )));
    }

    let context_tokens = if assistant.stop_reason == pi_events::StopReason::Error {
        let ContextUsageEstimate {
            tokens,
            last_usage_index,
            ..
        } = estimate_context_tokens(&state.messages);
        if last_usage_index.is_none() {
            return Ok(None);
        }
        tokens
    } else {
        calculate_context_tokens(&assistant.usage)
    };

    if !should_compact(context_tokens, context_window, settings) {
        return Ok(None);
    }

    let compacted = run_interactive_compaction(core, session_manager, settings, None).await?;
    if compacted.is_none() {
        return Ok(None);
    }

    update_interactive_footer_state(footer_state_handle, core, Some(session_manager));
    if core.agent().has_queued_messages() {
        core.continue_turn()
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(Some(String::from(
        "Automatically compacted session context",
    )))
}

fn standard_message_to_assistant(message: &Message) -> Option<pi_events::AssistantMessage> {
    match message {
        Message::Assistant {
            content,
            api,
            provider,
            model,
            response_id,
            usage,
            stop_reason,
            error_message,
            timestamp,
        } => Some(pi_events::AssistantMessage {
            role: String::from("assistant"),
            content: content.clone(),
            api: api.clone(),
            provider: provider.clone(),
            model: model.clone(),
            response_id: response_id.clone(),
            usage: usage.clone(),
            stop_reason: stop_reason.clone(),
            error_message: error_message.clone(),
            timestamp: *timestamp,
        }),
        Message::User { .. } | Message::ToolResult { .. } => None,
    }
}

fn install_interactive_auto_compaction(
    core: &CodingAgentCore,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    status_handle: &StatusHandle,
    footer_state_handle: &FooterStateHandle,
    runtime_settings: Arc<Mutex<LoadedRuntimeSettings>>,
) -> Option<AgentUnsubscribe> {
    let session_manager = session_manager?.clone();
    let core = core.clone();
    let status_handle = status_handle.clone();
    let footer_state_handle = footer_state_handle.clone();
    let compaction_running = Arc::new(AtomicBool::new(false));
    let overflow_recovery_attempted = Arc::new(AtomicBool::new(false));

    Some(core.agent().subscribe(move |event, _signal| {
        let core = core.clone();
        let session_manager = session_manager.clone();
        let status_handle = status_handle.clone();
        let footer_state_handle = footer_state_handle.clone();
        let runtime_settings = runtime_settings.clone();
        let compaction_running = compaction_running.clone();
        let overflow_recovery_attempted = overflow_recovery_attempted.clone();
        Box::pin(async move {
            match event {
                pi_agent::AgentEvent::MessageStart { message }
                    if matches!(message.as_standard_message(), Some(Message::User { .. })) =>
                {
                    overflow_recovery_attempted.store(false, Ordering::Relaxed);
                }
                pi_agent::AgentEvent::AgentEnd { .. } => {
                    if compaction_running.swap(true, Ordering::Relaxed) {
                        return;
                    }

                    let settings = {
                        let runtime_settings = runtime_settings
                            .lock()
                            .expect("interactive runtime settings mutex poisoned")
                            .clone();
                        runtime_compaction_settings(&runtime_settings)
                    };

                    let result = maybe_run_auto_compaction(
                        &core,
                        &session_manager,
                        &footer_state_handle,
                        &settings,
                        &overflow_recovery_attempted,
                    )
                    .await;
                    compaction_running.store(false, Ordering::Relaxed);

                    match result {
                        Ok(Some(message)) => status_handle.set_message(message),
                        Ok(None) => {}
                        Err(error) => status_handle.set_message(format!("Error: {error}")),
                    }
                }
                _ => {}
            }
        })
    }))
}

fn build_interactive_slash_commands(
    model_registry: Arc<ModelRegistry>,
    scoped_models: Vec<ScopedModel>,
    resources: &LoadedCliResources,
) -> Vec<SlashCommand> {
    #[derive(Clone)]
    struct ModelCommandItem {
        id: String,
        provider: String,
        value: String,
    }

    let model_registry_for_arguments = model_registry.clone();
    let scoped_models_for_arguments = scoped_models.clone();
    let oauth_providers = oauth_provider_summaries();
    let oauth_providers_for_login = oauth_providers.clone();
    let oauth_providers_for_logout = oauth_providers.clone();

    let mut commands = vec![
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
        simple_slash_command(
            "scoped-models",
            "Configure scoped models for Ctrl+P cycling",
        ),
        simple_slash_command("export", "Export session (HTML default, or specify .jsonl)"),
        simple_slash_command("share", "Share session with an HTML preview link"),
        simple_slash_command("copy", "Copy last assistant message to clipboard"),
        simple_slash_command("name", "Set session display name"),
        simple_slash_command("session", "Show session info and stats"),
        simple_slash_command("changelog", "Show changelog entries"),
        simple_slash_command("hotkeys", "Show keyboard shortcuts"),
        simple_slash_command("fork", "Fork from a previous user message"),
        simple_slash_command("tree", "Show or switch the session tree"),
        SlashCommand {
            name: String::from("login"),
            description: Some(String::from("Login with OAuth provider")),
            argument_completions: Some(Arc::new(move |prefix| {
                autocomplete_oauth_providers(&oauth_providers_for_login, prefix)
            })),
        },
        SlashCommand {
            name: String::from("logout"),
            description: Some(String::from("Logout from OAuth provider")),
            argument_completions: Some(Arc::new(move |prefix| {
                autocomplete_oauth_providers(&oauth_providers_for_logout, prefix)
            })),
        },
        simple_slash_command("new", "Start a new session"),
        simple_slash_command("compact", "Compact the current session context"),
        simple_slash_command("resume", "Resume a different session"),
        simple_slash_command(
            "reload",
            "Reload keybindings, skills, prompts, and settings",
        ),
        SlashCommand {
            name: String::from("quit"),
            description: Some(String::from("Quit pi")),
            argument_completions: None,
        },
    ];

    commands.extend(
        resources
            .prompt_templates
            .iter()
            .map(|template| SlashCommand {
                name: template.name.clone(),
                description: (!template.description.is_empty())
                    .then_some(template.description.clone()),
                argument_completions: None,
            }),
    );
    commands.extend(resources.skills.iter().map(|skill| SlashCommand {
        name: format!("skill:{}", skill.name),
        description: Some(skill.description.clone()),
        argument_completions: None,
    }));

    commands
}

fn simple_slash_command(name: &str, description: &str) -> SlashCommand {
    SlashCommand {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        argument_completions: None,
    }
}

fn autocomplete_oauth_providers(
    providers: &[OAuthProviderSummary],
    prefix: &str,
) -> Option<Vec<AutocompleteItem>> {
    let filtered = fuzzy_filter(providers, prefix, |provider| {
        Cow::Owned(format!("{} {}", provider.id, provider.name))
    });
    if filtered.is_empty() {
        return None;
    }

    Some(
        filtered
            .into_iter()
            .map(|provider| AutocompleteItem {
                value: provider.id.clone(),
                label: provider.id.clone(),
                description: Some(provider.name.clone()),
            })
            .collect(),
    )
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

    if matches!(&transition, InteractiveTransitionRequest::ForkPicker) {
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

    if matches!(&transition, InteractiveTransitionRequest::TreePicker) {
        let Some(session_manager) = session_manager else {
            status_handle.set_message("Session tree is not available in this interactive mode.");
            return true;
        };
        let session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        if session_manager.get_entries().is_empty() {
            status_handle.set_message("No entries in session");
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

fn current_runtime_settings(context: &InteractiveSlashCommandContext) -> LoadedRuntimeSettings {
    context
        .runtime_settings
        .lock()
        .expect("interactive runtime settings mutex poisoned")
        .clone()
}

fn render_hotkeys_text(keybindings: &KeybindingsManager) -> String {
    let mut output = String::new();
    push_line(&mut output, "Keyboard Shortcuts");
    push_line(&mut output, "");

    let mut current_section = None::<&str>;
    for (keybinding, definition) in DEFAULT_APP_KEYBINDINGS.iter() {
        if keybinding.starts_with("app.scopedModels.") {
            continue;
        }

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

fn build_tree_picker_items(session_manager: &SessionManager) -> Vec<TreePickerItem> {
    let current_leaf = session_manager.get_leaf_id();
    let mut items = vec![TreePickerItem {
        entry_id: String::from(ROOT_TREE_ENTRY_ID),
        display: if current_leaf.is_none() {
            String::from("* root")
        } else {
            String::from("  root")
        },
        search_text: String::from("root session root"),
    }];

    let tree = session_manager.get_tree();
    for (index, node) in tree.iter().enumerate() {
        collect_tree_picker_items(&mut items, node, "", index + 1 == tree.len(), current_leaf);
    }

    items
}

fn collect_tree_picker_items(
    items: &mut Vec<TreePickerItem>,
    node: &pi_coding_agent_core::SessionTreeNode,
    prefix: &str,
    is_last: bool,
    current_leaf: Option<&str>,
) {
    if matches!(node.entry, SessionEntry::Label { .. }) {
        for (index, child) in node.children.iter().enumerate() {
            collect_tree_picker_items(
                items,
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
        items.push(TreePickerItem {
            entry_id: node.entry.id().to_owned(),
            display: format!("{prefix}{branch}{marker} {description}{label_suffix}"),
            search_text: sanitize_display_text(&format!(
                "{} {} {}",
                node.entry.id(),
                description,
                node.label.as_deref().unwrap_or_default()
            )),
        });
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
        collect_tree_picker_items(
            items,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Html,
    Jsonl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SharedSessionLinks {
    gist_url: String,
    preview_url: String,
}

fn resolve_export_target(
    cwd: &Path,
    session_manager: &SessionManager,
    output_path: Option<&str>,
) -> (PathBuf, ExportFormat) {
    let Some(output_path) = output_path.filter(|path| !path.trim().is_empty()) else {
        return (
            cwd.join(export_html::default_html_file_name(session_manager)),
            ExportFormat::Html,
        );
    };

    let resolved_path = {
        let output_path = Path::new(output_path);
        if output_path.is_absolute() {
            output_path.to_path_buf()
        } else {
            cwd.join(output_path)
        }
    };
    let format = if output_path.ends_with(".jsonl") {
        ExportFormat::Jsonl
    } else {
        ExportFormat::Html
    };
    (resolved_path, format)
}

fn export_interactive_session(
    session_manager: &Arc<Mutex<SessionManager>>,
    cwd: &Path,
    output_path: Option<&str>,
) -> Result<String, String> {
    let session_manager = session_manager
        .lock()
        .expect("session manager mutex poisoned");
    let (output_path, format) = resolve_export_target(cwd, &session_manager, output_path);
    match format {
        ExportFormat::Html => export_html::export_session_to_html(&session_manager, &output_path),
        ExportFormat::Jsonl => session_manager
            .export_branch_jsonl(&output_path)
            .map_err(|error| error.to_string()),
    }
}

fn share_viewer_url(gist_id: &str) -> String {
    let base_url = env::var("PI_SHARE_VIEWER_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| String::from(DEFAULT_SHARE_VIEWER_URL));
    format!("{base_url}#{gist_id}")
}

fn parse_gist_id(gist_url: &str) -> Option<&str> {
    gist_url.trim().trim_end_matches('/').rsplit('/').next()
}

fn parse_shared_session_links(output: &[u8]) -> Result<SharedSessionLinks, String> {
    let gist_url = String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| String::from("Failed to parse gist URL from gh output"))?;
    let gist_id = parse_gist_id(&gist_url)
        .filter(|gist_id| !gist_id.is_empty())
        .ok_or_else(|| String::from("Failed to parse gist URL from gh output"))?;

    Ok(SharedSessionLinks {
        preview_url: share_viewer_url(gist_id),
        gist_url,
    })
}

fn share_interactive_session(
    session_manager: &Arc<Mutex<SessionManager>>,
    cwd: &Path,
) -> Result<SharedSessionLinks, String> {
    let temp_file = {
        let session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        env::temp_dir().join(export_html::default_html_file_name(&session_manager))
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

    parse_shared_session_links(&output.stdout)
}

fn export_session_file_to_html(
    cwd: &Path,
    input_path: &str,
    output_path: Option<&str>,
) -> Result<String, String> {
    let input_path = resolve_session_path(cwd, input_path);
    let session_manager =
        SessionManager::open(&input_path, None, None).map_err(|error| error.to_string())?;
    let output_path = if let Some(output_path) = output_path.filter(|path| !path.trim().is_empty())
    {
        let output_path = Path::new(output_path);
        if output_path.is_absolute() {
            output_path.to_path_buf()
        } else {
            cwd.join(output_path)
        }
    } else {
        cwd.join(export_html::default_html_file_name(&session_manager))
    };

    export_html::export_session_to_html(&session_manager, output_path)
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

#[derive(Debug, Clone)]
struct InteractiveBashResult {
    output: String,
    exit_code: Option<i32>,
    cancelled: bool,
    truncated: bool,
    full_output_path: Option<String>,
}

#[derive(Default)]
struct InteractiveBashController {
    abort_tx: Option<watch::Sender<bool>>,
}

#[derive(Debug, Clone)]
enum InteractiveBashTranscriptStatus {
    Running,
    Complete(InteractiveBashResult),
    Error(String),
}

#[derive(Debug, Clone)]
struct InteractiveBashTranscriptState {
    command: String,
    cancel_key_text: String,
    exclude_from_context: bool,
    status: InteractiveBashTranscriptStatus,
}

struct InteractiveBashTranscriptComponent {
    state: Arc<Mutex<InteractiveBashTranscriptState>>,
}

#[derive(Clone)]
struct InteractiveBashTranscriptHandle {
    state: Arc<Mutex<InteractiveBashTranscriptState>>,
    render_handle: RenderHandle,
}

impl InteractiveBashTranscriptComponent {
    fn new(
        command: impl Into<String>,
        cancel_key_text: impl Into<String>,
        exclude_from_context: bool,
        render_handle: RenderHandle,
    ) -> (Self, InteractiveBashTranscriptHandle) {
        let state = Arc::new(Mutex::new(InteractiveBashTranscriptState {
            command: command.into(),
            cancel_key_text: cancel_key_text.into(),
            exclude_from_context,
            status: InteractiveBashTranscriptStatus::Running,
        }));
        (
            Self {
                state: state.clone(),
            },
            InteractiveBashTranscriptHandle {
                state,
                render_handle,
            },
        )
    }
}

impl InteractiveBashTranscriptHandle {
    fn set_complete(&self, result: InteractiveBashResult) {
        self.state
            .lock()
            .expect("interactive bash transcript mutex poisoned")
            .status = InteractiveBashTranscriptStatus::Complete(result);
        self.render_handle.request_render();
    }

    fn set_error(&self, error: impl Into<String>) {
        self.state
            .lock()
            .expect("interactive bash transcript mutex poisoned")
            .status = InteractiveBashTranscriptStatus::Error(error.into());
        self.render_handle.request_render();
    }
}

impl Component for InteractiveBashTranscriptComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let state = self
            .state
            .lock()
            .expect("interactive bash transcript mutex poisoned")
            .clone();
        let label = if state.exclude_from_context {
            "[bash no context]"
        } else {
            "[bash]"
        };
        let body = match state.status {
            InteractiveBashTranscriptStatus::Running => {
                format!(
                    "$ {}\n\nRunning... ({} to cancel)",
                    state.command, state.cancel_key_text
                )
            }
            InteractiveBashTranscriptStatus::Complete(result) => {
                bash_execution_to_text(&BashExecutionMessage {
                    command: state.command,
                    output: result.output,
                    exit_code: result.exit_code.map(i64::from),
                    cancelled: result.cancelled,
                    truncated: result.truncated,
                    full_output_path: result.full_output_path,
                    exclude_from_context: state.exclude_from_context,
                })
            }
            InteractiveBashTranscriptStatus::Error(error) => {
                format!("$ {}\n\nError: {error}", state.command)
            }
        };

        let mut container = Container::new();
        container.add_child(Box::new(Spacer::new(1)));
        container.add_child(Box::new(Text::new(label, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));
        container.add_child(Box::new(Text::new(body, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));
        container.render(width)
    }

    fn invalidate(&mut self) {}
}

fn matches_shell_binding(keybindings: &KeybindingsManager, data: &str, keybinding: &str) -> bool {
    keybindings
        .get_keys(keybinding)
        .iter()
        .any(|key| matches_key(data, key.as_str()))
}

async fn execute_interactive_bash_command(
    cwd: &Path,
    command: &str,
    mut abort_rx: watch::Receiver<bool>,
) -> Result<InteractiveBashResult, String> {
    if !cwd.exists() {
        return Err(format!(
            "Working directory does not exist: {}",
            cwd.display()
        ));
    }

    if *abort_rx.borrow() {
        return Ok(InteractiveBashResult {
            output: String::new(),
            exit_code: None,
            cancelled: true,
            truncated: false,
            full_output_path: None,
        });
    }

    let shell = env::var("SHELL").unwrap_or_else(|_| String::from("sh"));
    let wrapped_command = format!("{{\n{command}\n}} 2>&1");

    let mut command_builder = TokioCommand::new(shell);
    command_builder
        .arg("-lc")
        .arg(wrapped_command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output_future = command_builder
        .spawn()
        .map_err(|error| error.to_string())?
        .wait_with_output();
    tokio::pin!(output_future);

    let abort_future = async {
        while abort_rx.changed().await.is_ok() {
            if *abort_rx.borrow() {
                return;
            }
        }
    };
    tokio::pin!(abort_future);

    let output = tokio::select! {
        output = &mut output_future => output.map_err(|error| error.to_string())?,
        _ = &mut abort_future => {
            return Ok(InteractiveBashResult {
                output: String::new(),
                exit_code: None,
                cancelled: true,
                truncated: false,
                full_output_path: None,
            });
        }
    };

    let mut full_output = String::from_utf8_lossy(&output.stdout).into_owned();
    full_output.push_str(&String::from_utf8_lossy(&output.stderr));
    let full_output = strip_interactive_bash_output(&full_output).replace('\r', "");
    let truncation = pi_coding_agent_tools::truncate_tail(
        &full_output,
        pi_coding_agent_tools::TruncationOptions::default(),
    );
    let full_output_path = if truncation.truncated {
        Some(write_interactive_bash_output(&full_output)?)
    } else {
        None
    };

    Ok(InteractiveBashResult {
        output: if truncation.truncated {
            truncation.content
        } else {
            full_output
        },
        exit_code: output.status.code(),
        cancelled: false,
        truncated: truncation.truncated,
        full_output_path,
    })
}

fn strip_interactive_bash_output(output: &str) -> String {
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
            .expect("interactive bash output should contain a character");
        index += character.len_utf8();

        if character == '\r' || (character.is_control() && character != '\n' && character != '\t') {
            continue;
        }

        result.push(character);
    }

    result
}

fn write_interactive_bash_output(output: &str) -> Result<String, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-bash-{}-{unique}.log", std::process::id()));
    fs::write(&path, output).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

async fn record_interactive_bash_result(
    core: &CodingAgentCore,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    command: &str,
    result: &InteractiveBashResult,
    exclude_from_context: bool,
) -> Result<(), String> {
    if core.state().is_streaming {
        core.wait_for_idle().await;
    }

    let message = create_bash_execution_message(
        command.to_owned(),
        result.output.clone(),
        result.exit_code.map(i64::from),
        result.cancelled,
        result.truncated,
        result.full_output_path.clone(),
        exclude_from_context,
        now_ms(),
    );
    let message_for_state = message.clone();
    core.agent().update_state(move |state| {
        state.messages.push(message_for_state.clone());
    });

    if let Some(session_manager) = session_manager {
        session_manager
            .lock()
            .expect("session manager mutex poisoned")
            .append_message(message)
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn submit_interactive_bash_command(
    shell: &mut StartupShellComponent,
    command: &str,
    exclude_from_context: bool,
    core: &CodingAgentCore,
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    cwd: &Path,
    keybindings: &KeybindingsManager,
    status_handle: &StatusHandle,
    render_handle: &RenderHandle,
    bash_controller: &Arc<Mutex<InteractiveBashController>>,
) {
    if bash_controller
        .lock()
        .expect("interactive bash controller mutex poisoned")
        .abort_tx
        .is_some()
    {
        let raw_command = if exclude_from_context {
            format!("!!{command}")
        } else {
            format!("!{command}")
        };
        shell.set_input_value(raw_command.clone());
        shell.set_input_cursor(raw_command.len());
        status_handle.set_message(format!(
            "A bash command is already running. Press {} to cancel it first.",
            key_text(keybindings, "app.interrupt")
        ));
        return;
    }

    let cancel_key_text = key_text(keybindings, "app.interrupt");
    let (component, component_handle) = InteractiveBashTranscriptComponent::new(
        command,
        cancel_key_text,
        exclude_from_context,
        render_handle.clone(),
    );
    shell.add_transcript_item(Box::new(component));

    let (abort_tx, abort_rx) = watch::channel(false);
    bash_controller
        .lock()
        .expect("interactive bash controller mutex poisoned")
        .abort_tx = Some(abort_tx);

    let core = core.clone();
    let session_manager = session_manager.cloned();
    let cwd = cwd.to_path_buf();
    let command = command.to_owned();
    let status_handle = status_handle.clone();
    let bash_controller = bash_controller.clone();
    tokio::spawn(async move {
        let result = execute_interactive_bash_command(&cwd, &command, abort_rx).await;
        bash_controller
            .lock()
            .expect("interactive bash controller mutex poisoned")
            .abort_tx = None;

        match result {
            Ok(result) => {
                component_handle.set_complete(result.clone());
                if let Err(error) = record_interactive_bash_result(
                    &core,
                    session_manager.as_ref(),
                    &command,
                    &result,
                    exclude_from_context,
                )
                .await
                {
                    status_handle.set_message(format!("Error: {error}"));
                }
            }
            Err(error) => {
                component_handle.set_error(error.clone());
                status_handle.set_message(format!("Error: {error}"));
            }
        }
    });
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
    resources: LoadedCliResources,
    render_handle: RenderHandle,
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

    let tree_core = core.clone();
    let tree_status_handle = status_handle.clone();
    let tree_transition_request = transition_request.clone();
    let tree_exit_requested = exit_requested.clone();
    let tree_session_manager = session_manager.clone();
    shell.on_action("app.session.tree", move || {
        request_interactive_transition(
            InteractiveTransitionRequest::TreePicker,
            &tree_core,
            tree_session_manager.as_ref(),
            &tree_status_handle,
            &tree_transition_request,
            &tree_exit_requested,
        );
    });

    let bash_controller = Arc::new(Mutex::new(InteractiveBashController::default()));
    let interrupt_keybindings = slash_command_context.keybindings.clone();
    let interrupt_bash_controller = bash_controller.clone();
    shell.set_on_extension_shortcut(move |data| {
        if !matches_shell_binding(&interrupt_keybindings, &data, "app.interrupt") {
            return false;
        }

        let abort_tx = interrupt_bash_controller
            .lock()
            .expect("interactive bash controller mutex poisoned")
            .abort_tx
            .clone();
        if let Some(abort_tx) = abort_tx {
            let _ = abort_tx.send(true);
            return true;
        }

        false
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

        if let Some(command) = trimmed.strip_prefix("!!").map(str::trim)
            && !command.is_empty()
        {
            submit_interactive_bash_command(
                shell,
                command,
                true,
                &core,
                session_manager.as_ref(),
                &slash_command_context.cwd,
                &slash_command_context.keybindings,
                &status_handle,
                &render_handle,
                &bash_controller,
            );
            return;
        }

        if let Some(command) = trimmed.strip_prefix('!').map(str::trim)
            && !command.is_empty()
        {
            submit_interactive_bash_command(
                shell,
                command,
                false,
                &core,
                session_manager.as_ref(),
                &slash_command_context.cwd,
                &slash_command_context.keybindings,
                &status_handle,
                &render_handle,
                &bash_controller,
            );
            return;
        }

        status_handle.set_message("Working...");
        let core = core.clone();
        let status_handle = status_handle.clone();
        let prepared = preprocess_prompt_text(&value, &resources);
        tokio::spawn(async move {
            if let Err(error) = core.prompt_text(prepared).await {
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
        return request_interactive_transition(
            InteractiveTransitionRequest::SettingsPicker,
            core,
            session_manager,
            status_handle,
            transition_request,
            exit_requested,
        );
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
            return request_interactive_transition(
                InteractiveTransitionRequest::TreePicker,
                core,
                Some(session_manager),
                status_handle,
                transition_request,
                exit_requested,
            );
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
        if model_registry.get_available().is_empty() {
            status_handle.set_message("No models available");
            return true;
        }

        let initial_search = text
            .strip_prefix("/scoped-models")
            .unwrap_or_default()
            .trim();
        return request_interactive_transition(
            InteractiveTransitionRequest::ScopedModelsPicker {
                initial_search: (!initial_search.is_empty()).then_some(initial_search.to_owned()),
            },
            core,
            session_manager,
            status_handle,
            transition_request,
            exit_requested,
        );
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
            Ok(links) => status_handle.set_message(format!(
                "Share URL: {}\nGist: {}",
                links.preview_url, links.gist_url
            )),
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

    if text == "/login" || text.starts_with("/login ") {
        if core.state().is_streaming {
            status_handle.set_message("Wait for the current response to finish before logging in.");
            return true;
        }

        let provider_id = text.strip_prefix("/login").unwrap_or_default().trim();
        if provider_id.is_empty() {
            if slash_command_context
                .auth_operation_in_progress
                .load(Ordering::Relaxed)
            {
                status_handle.set_message("An OAuth login is already in progress.");
                return true;
            }
            if slash_command_context.agent_dir.is_none() {
                status_handle.set_message("OAuth login requires an agent directory.");
                return true;
            }
            if oauth_provider_summaries().is_empty() {
                status_handle.set_message("No OAuth providers available");
                return true;
            }
            return request_interactive_transition(
                InteractiveTransitionRequest::OAuthPicker(OAuthPickerMode::Login),
                core,
                session_manager,
                status_handle,
                transition_request,
                exit_requested,
            );
        }

        if oauth_provider_name(provider_id).is_none() {
            status_handle.set_message(format!("Unknown OAuth provider: {provider_id}"));
            return true;
        }

        let Some(agent_dir) = slash_command_context.agent_dir.as_ref() else {
            status_handle.set_message("OAuth login requires an agent directory.");
            return true;
        };

        if slash_command_context
            .auth_operation_in_progress
            .swap(true, Ordering::Relaxed)
        {
            status_handle.set_message("An OAuth login is already in progress.");
            return true;
        }

        let provider_id = provider_id.to_owned();
        let auth_path = agent_dir.join("auth.json");
        let status_handle = status_handle.clone();
        let ui_host = slash_command_context.ui_host.clone();
        let auth_operation_in_progress = slash_command_context.auth_operation_in_progress.clone();
        status_handle.set_message(format!("Starting OAuth login for {provider_id}..."));
        tokio::spawn(async move {
            let result = run_terminal_oauth_login(auth_path, provider_id.clone(), ui_host).await;
            auth_operation_in_progress.store(false, Ordering::Relaxed);
            match result {
                Ok(provider_name) => {
                    status_handle.set_message(format!("Logged in to {provider_name}"));
                }
                Err(error) => status_handle.set_message(format!("Error: {error}")),
            }
        });
        return true;
    }

    if text == "/logout" || text.starts_with("/logout ") {
        if core.state().is_streaming {
            status_handle
                .set_message("Wait for the current response to finish before logging out.");
            return true;
        }

        let Some(agent_dir) = slash_command_context.agent_dir.as_ref() else {
            status_handle.set_message("OAuth logout requires an agent directory.");
            return true;
        };

        if slash_command_context
            .auth_operation_in_progress
            .load(Ordering::Relaxed)
        {
            status_handle.set_message("Wait for the current OAuth login to finish.");
            return true;
        }

        let auth_path = agent_dir.join("auth.json");
        let provider_id = text.strip_prefix("/logout").unwrap_or_default().trim();
        if provider_id.is_empty() {
            match list_persisted_oauth_providers(&auth_path) {
                Ok(providers) if providers.is_empty() => {
                    status_handle.set_message("No OAuth providers logged in. Use /login first.");
                    return true;
                }
                Ok(_) => {
                    return request_interactive_transition(
                        InteractiveTransitionRequest::OAuthPicker(OAuthPickerMode::Logout),
                        core,
                        session_manager,
                        status_handle,
                        transition_request,
                        exit_requested,
                    );
                }
                Err(error) => {
                    status_handle.set_message(format!("Error: {error}"));
                    return true;
                }
            }
        }

        let provider_name =
            oauth_provider_name(provider_id).unwrap_or_else(|| provider_id.to_owned());
        match remove_persisted_oauth_provider(&auth_path, provider_id) {
            Ok(true) => status_handle.set_message(format!("Logged out of {provider_name}")),
            Ok(false) => {
                status_handle.set_message(format!("No OAuth credentials stored for {provider_id}"))
            }
            Err(error) => status_handle.set_message(format!("Error: {error}")),
        }
        return true;
    }

    if text == "/compact" || text.starts_with("/compact ") {
        let Some(session_manager) = session_manager else {
            status_handle
                .set_message("Session compaction is not available in this interactive mode.");
            return true;
        };
        if core.state().is_streaming {
            status_handle.set_message("Wait for the current response to finish before compacting.");
            return true;
        }

        let custom_instructions = text.strip_prefix("/compact").unwrap_or_default().trim();
        let custom_instructions =
            (!custom_instructions.is_empty()).then_some(custom_instructions.to_owned());
        let core = core.clone();
        let session_manager = session_manager.clone();
        let status_handle = status_handle.clone();
        let footer_state_handle = footer_state_handle.clone();
        let settings =
            runtime_compaction_settings(&current_runtime_settings(slash_command_context));
        status_handle.set_message("Compacting session context...");
        tokio::spawn(async move {
            match run_interactive_compaction(
                &core,
                &session_manager,
                &settings,
                custom_instructions.as_deref(),
            )
            .await
            {
                Ok(Some(_)) => {
                    update_interactive_footer_state(
                        &footer_state_handle,
                        &core,
                        Some(&session_manager),
                    );
                    status_handle.set_message("Compacted session context");
                }
                Ok(None) => status_handle.set_message("Nothing to compact"),
                Err(error) => status_handle.set_message(format!("Error: {error}")),
            }
        });
        return true;
    }

    if text == "/reload" {
        if core.state().is_streaming {
            status_handle.set_message("Wait for the current response to finish before reloading.");
            return true;
        }
        if slash_command_context
            .auth_operation_in_progress
            .load(Ordering::Relaxed)
        {
            status_handle.set_message("Wait for the current OAuth login to finish.");
            return true;
        }

        return request_interactive_transition(
            InteractiveTransitionRequest::Reload,
            core,
            session_manager,
            status_handle,
            transition_request,
            exit_requested,
        );
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

fn unsupported_flag_message(_parsed: &Args) -> Option<String> {
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
        "  - rpc mode (--mode rpc)",
        "  - interactive mode",
        "  - --provider, --model, --models, --api-key, --system-prompt, --append-system-prompt, --thinking, --transport",
        "  - --continue, --resume, --session, --fork, --no-session, --session-dir",
        "  - --list-models [search]",
        "  - --export <session.jsonl> [out.html]",
        "  - --extension/-e, --no-extensions (RPC extension commands/resources/UI bridge)",
        "  - --skill, --no-skills, --prompt-template, --no-prompt-templates",
        "  - --theme, --no-themes",
        "  - @file text/image preprocessing",
        "",
        "RPC mode limitations:",
        "  - @file arguments are rejected",
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
        FauxModelDefinition, FauxResponse, OAuthCredentials, OAuthCredentialsFuture,
        OAuthLoginCallbacks, OAuthProvider, RegisterFauxProviderOptions, register_faux_provider,
        register_oauth_provider, unregister_oauth_provider,
    };
    use pi_coding_agent_core::MemoryAuthStorage;
    use std::{
        fs, io,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex, OnceLock,
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

    #[derive(Default)]
    struct TestExternalEditorHost;

    impl ExternalEditorHost for TestExternalEditorHost {}

    fn test_slash_command_context(
        keybindings: &KeybindingsManager,
        cwd: impl Into<PathBuf>,
    ) -> InteractiveSlashCommandContext {
        test_slash_command_context_with_agent_dir(keybindings, cwd, None)
    }

    fn test_slash_command_context_with_agent_dir(
        keybindings: &KeybindingsManager,
        cwd: impl Into<PathBuf>,
        agent_dir: Option<PathBuf>,
    ) -> InteractiveSlashCommandContext {
        InteractiveSlashCommandContext {
            keybindings: keybindings.clone(),
            runtime_settings: Arc::new(Mutex::new(LoadedRuntimeSettings::default())),
            cwd: cwd.into(),
            agent_dir,
            ui_host: Arc::new(TestExternalEditorHost),
            auth_operation_in_progress: Arc::new(AtomicBool::new(false)),
        }
    }

    fn oauth_registry_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[derive(Debug)]
    struct TestOAuthProvider {
        id: &'static str,
        name: &'static str,
        access_token: &'static str,
    }

    impl OAuthProvider for TestOAuthProvider {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.name
        }

        fn login<'a>(&'a self, _callbacks: OAuthLoginCallbacks) -> OAuthCredentialsFuture<'a> {
            Box::pin(async move {
                Ok(OAuthCredentials::new(
                    "refresh-token",
                    self.access_token,
                    i64::MAX,
                ))
            })
        }

        fn refresh_token<'a>(
            &'a self,
            credentials: OAuthCredentials,
        ) -> OAuthCredentialsFuture<'a> {
            Box::pin(async move { Ok(credentials) })
        }

        fn get_api_key(&self, credentials: &OAuthCredentials) -> Result<String, String> {
            Ok(credentials.access.clone())
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
    fn interactive_default_system_prompt_reloads_prompt_resources() {
        let cwd = unique_temp_dir("interactive-default-system-prompt-cwd");
        let agent_dir = unique_temp_dir("interactive-default-system-prompt-agent");
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("SYSTEM.md"), "initial system\n").unwrap();
        fs::write(cwd.join("AGENTS.md"), "initial agents\n").unwrap();

        let parsed = Args::default();
        let initial = resolve_interactive_default_system_prompt(
            "cached prompt",
            &cwd,
            Some(agent_dir.as_path()),
            &parsed,
        );

        assert!(initial.contains("initial system\n"), "prompt: {initial}");
        assert!(initial.contains("initial agents\n"), "prompt: {initial}");

        fs::write(cwd.join(".pi").join("SYSTEM.md"), "updated system\n").unwrap();
        fs::write(cwd.join("AGENTS.md"), "updated agents\n").unwrap();

        let reloaded = resolve_interactive_default_system_prompt(
            "cached prompt",
            &cwd,
            Some(agent_dir.as_path()),
            &parsed,
        );

        assert!(reloaded.contains("updated system\n"), "prompt: {reloaded}");
        assert!(reloaded.contains("updated agents\n"), "prompt: {reloaded}");
        assert!(!reloaded.contains("initial system\n"), "prompt: {reloaded}");
        assert!(!reloaded.contains("initial agents\n"), "prompt: {reloaded}");
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

    #[tokio::test]
    async fn run_command_uses_target_session_cwd_for_noninteractive_file_args() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "session-cwd-print-mode-faux".into(),
            models: vec![FauxModelDefinition {
                id: "session-cwd-print-mode-faux-1".into(),
                name: Some("Session Cwd Print Mode Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        faux.set_responses(vec![FauxResponse::text("Cross-project session response")]);
        let model = faux
            .get_model(Some("session-cwd-print-mode-faux-1"))
            .expect("expected faux model");
        let startup_cwd = unique_temp_dir("session-cwd-startup-cwd");
        let selected_cwd = unique_temp_dir("session-cwd-selected-cwd");
        let agent_dir = unique_temp_dir("session-cwd-agent");
        fs::write(selected_cwd.join("context.txt"), "selected cwd file\n").unwrap();

        let agent_dir_string = agent_dir.to_string_lossy().into_owned();
        let selected_cwd_string = selected_cwd.to_string_lossy().into_owned();
        let session_dir = get_default_session_dir(&selected_cwd_string, Some(&agent_dir_string));
        let mut session_manager =
            SessionManager::create(&selected_cwd_string, Some(&session_dir)).unwrap();
        session_manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("resume me"),
                }],
                timestamp: 1,
            })
            .unwrap();
        session_manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("saved response"),
                    text_signature: None,
                }],
                api: String::from("faux:test"),
                provider: model.provider.clone(),
                model: model.id.clone(),
                response_id: None,
                usage: Default::default(),
                stop_reason: pi_events::StopReason::Stop,
                error_message: None,
                timestamp: 2,
            })
            .unwrap();
        let session_file = session_manager
            .get_session_file()
            .map(str::to_owned)
            .expect("expected session file");

        let mut parsed = Args::default();
        parsed.session = Some(session_file.clone());
        let prepared = prepare_startup_session(
            &parsed,
            &startup_cwd,
            Some(agent_dir.as_path()),
            Arc::new(|| Box::new(LifecycleScriptedTerminal::new(Vec::new()))),
        )
        .await
        .expect("startup session preparation should succeed");
        let StartupSessionPreparation::Ready(prepared) = prepared else {
            panic!("expected prepared session support");
        };
        assert_eq!(prepared.runtime_cwd, selected_cwd);

        let result = timeout(
            Duration::from_secs(3),
            run_command(RunCommandOptions {
                args: vec![
                    String::from("--session"),
                    session_file,
                    String::from("--provider"),
                    model.provider.clone(),
                    String::from("--model"),
                    model.id.clone(),
                    String::from("--print"),
                    String::from("@context.txt"),
                    String::from("Use the file"),
                ],
                stdin_is_tty: true,
                stdin_content: None,
                auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                    model.provider.as_str(),
                    "token",
                )])),
                built_in_models: vec![model],
                models_json_path: None,
                agent_dir: Some(agent_dir),
                cwd: startup_cwd,
                default_system_prompt: String::new(),
                version: String::from("0.1.0"),
                stream_options: StreamOptions::default(),
            }),
        )
        .await
        .expect("run_command should complete");

        assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
        assert!(
            result.stdout.contains("Cross-project session response"),
            "stdout: {}\nstderr: {}",
            result.stdout,
            result.stderr
        );

        faux.unregister();
    }

    #[tokio::test]
    async fn run_rpc_command_live_allows_resume_picker_cancellation() {
        let terminal =
            LifecycleScriptedTerminal::new(vec![(Duration::from_millis(5), String::from("\x1b"))]);
        let stdout = Arc::new(Mutex::new(String::new()));
        let stderr = Arc::new(Mutex::new(String::new()));
        let stdout_emitter: TextEmitter = Arc::new({
            let stdout = stdout.clone();
            move |text| {
                stdout
                    .lock()
                    .expect("rpc stdout buffer mutex poisoned")
                    .push_str(&text);
            }
        });
        let stderr_emitter: TextEmitter = Arc::new({
            let stderr = stderr.clone();
            move |text| {
                stderr
                    .lock()
                    .expect("rpc stderr buffer mutex poisoned")
                    .push_str(&text);
            }
        });

        let exit_code = timeout(
            Duration::from_secs(3),
            run_rpc_command_live_with_terminal_factory(
                RunCommandOptions {
                    args: vec![
                        String::from("--mode"),
                        String::from("rpc"),
                        String::from("--resume"),
                    ],
                    stdin_is_tty: true,
                    stdin_content: None,
                    auth_source: Arc::new(MemoryAuthStorage::default()),
                    built_in_models: Vec::new(),
                    models_json_path: None,
                    agent_dir: Some(unique_temp_dir("resume-rpc-agent")),
                    cwd: unique_temp_dir("resume-rpc-cwd"),
                    default_system_prompt: String::new(),
                    version: String::from("0.1.0"),
                    stream_options: StreamOptions::default(),
                },
                stdout_emitter,
                stderr_emitter,
                Arc::new(move || Box::new(terminal.clone())),
            ),
        )
        .await
        .expect("rpc command should complete");

        let stdout = stdout
            .lock()
            .expect("rpc stdout buffer mutex poisoned")
            .clone();
        let stderr = stderr
            .lock()
            .expect("rpc stderr buffer mutex poisoned")
            .clone();
        assert_eq!(exit_code, 0, "stderr: {stderr}");
        assert_eq!(stdout, "No session selected\n");
        assert!(
            !stderr.contains("--resume session picker is only supported"),
            "stderr: {stderr}"
        );

        assert!(stderr.is_empty(), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn rpc_extension_ui_response_resolves_pending_dialog_request() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "rpc-extension-ui-response-faux".into(),
            models: vec![FauxModelDefinition {
                id: "rpc-extension-ui-response-faux-1".into(),
                name: Some("RPC Extension UI Response Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("rpc-extension-ui-response-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("rpc-extension-ui-response");
        let extension_path = cwd.join("rpc-input-extension.ts");
        fs::write(
            &extension_path,
            r#"export default function (pi) {
	pi.registerCommand("rpc-input", {
		description: "Prompt for input",
		handler: async (_args, ctx) => {
			const value = await ctx.ui.input("Enter a value", "type something...");
			ctx.ui.notify(value ? `You entered: ${value}` : "Input cancelled", "info");
		},
	});
}
"#,
        )
        .unwrap();

        let stdout = Arc::new(Mutex::new(String::new()));
        let stderr = Arc::new(Mutex::new(String::new()));
        let stdout_emitter: TextEmitter = Arc::new({
            let stdout = stdout.clone();
            move |text| {
                stdout
                    .lock()
                    .expect("rpc stdout buffer mutex poisoned")
                    .push_str(&text);
            }
        });
        let stderr_emitter: TextEmitter = Arc::new({
            let stderr = stderr.clone();
            move |text| {
                stderr
                    .lock()
                    .expect("rpc stderr buffer mutex poisoned")
                    .push_str(&text);
            }
        });

        let shared = create_rpc_shared(
            RpcPreparedOptions {
                parsed: parse_args(&vec![
                    String::from("--mode"),
                    String::from("rpc"),
                    String::from("--provider"),
                    model.provider.clone(),
                    String::from("--model"),
                    model.id.clone(),
                    String::from("--extension"),
                    extension_path.to_string_lossy().into_owned(),
                ]),
                initial_stderr: String::new(),
                stdin_content: None,
                auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                    model.provider.as_str(),
                    "token",
                )])),
                built_in_models: vec![model.clone()],
                models_json_path: None,
                agent_dir: Some(cwd.join("agent")),
                cwd: cwd.clone(),
                default_system_prompt: String::new(),
                stream_options: StreamOptions::default(),
            },
            None,
            stdout_emitter,
            stderr_emitter,
        )
        .await
        .expect("expected rpc shared state");

        let mut background_tasks = Vec::new();
        handle_rpc_input_line(
            shared.clone(),
            r#"{"id":"cmd-1","type":"prompt","message":"/rpc-input"}"#,
            Some(&mut background_tasks),
        )
        .await;

        let request_id = timeout(Duration::from_secs(2), async {
            loop {
                let stdout = stdout
                    .lock()
                    .expect("rpc stdout buffer mutex poisoned")
                    .clone();
                let lines = stdout
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .collect::<Vec<_>>();
                if let Some(request_id) = lines.iter().find_map(|line| {
                    (line.get("type").and_then(Value::as_str) == Some("extension_ui_request")
                        && line.get("method").and_then(Value::as_str) == Some("input"))
                    .then(|| {
                        line.get("id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .flatten()
                }) {
                    break request_id;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("expected extension input request");

        handle_rpc_input_line(
            shared.clone(),
            &format!(
                "{{\"type\":\"extension_ui_response\",\"id\":\"{request_id}\",\"value\":\"hello from host\"}}"
            ),
            Some(&mut background_tasks),
        )
        .await;

        for task in background_tasks {
            let _ = task.await;
        }
        sleep(Duration::from_millis(50)).await;

        let stdout = stdout
            .lock()
            .expect("rpc stdout buffer mutex poisoned")
            .clone();
        assert!(stdout.contains("Enter a value"), "stdout: {stdout}");
        assert!(
            stdout.contains("You entered: hello from host"),
            "stdout: {stdout}"
        );
        assert!(
            stderr
                .lock()
                .expect("rpc stderr buffer mutex poisoned")
                .is_empty()
        );

        shared.shutdown_extension_host().await;
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
        let commands =
            build_interactive_slash_commands(registry, Vec::new(), &LoadedCliResources::default())
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
    fn settings_and_scoped_models_slash_commands_request_transitions() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-settings-scope-faux".into(),
            models: vec![
                FauxModelDefinition {
                    id: "slash-settings-scope-faux-1".into(),
                    name: Some("Slash Settings Scope Faux 1".into()),
                    reasoning: false,
                },
                FauxModelDefinition {
                    id: "slash-settings-scope-faux-2".into(),
                    name: Some("Slash Settings Scope Faux 2".into()),
                    reasoning: false,
                },
            ],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-settings-scope-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-settings-scope-cwd");
        let created = create_coding_agent_core(CodingAgentCoreOptions {
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                model.provider.as_str(),
                "token",
            )])),
            built_in_models: vec![
                model.clone(),
                faux.get_model(Some("slash-settings-scope-faux-2"))
                    .expect("expected second faux model"),
            ],
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
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context = test_slash_command_context(&keybindings, cwd);

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/settings",
            &core,
            core.model_registry().as_ref(),
            &[],
            None,
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));
        assert!(exit_requested.load(Ordering::Relaxed));
        assert_eq!(
            transition_request
                .lock()
                .expect("interactive transition request mutex poisoned")
                .clone(),
            Some(InteractiveTransitionRequest::SettingsPicker)
        );

        exit_requested.store(false, Ordering::Relaxed);
        *transition_request
            .lock()
            .expect("interactive transition request mutex poisoned") = None;

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/scoped-models beta",
            &core,
            core.model_registry().as_ref(),
            &[],
            None,
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));
        assert!(exit_requested.load(Ordering::Relaxed));
        assert_eq!(
            transition_request
                .lock()
                .expect("interactive transition request mutex poisoned")
                .clone(),
            Some(InteractiveTransitionRequest::ScopedModelsPicker {
                initial_search: Some(String::from("beta")),
            })
        );

        faux.unregister();
    }

    #[test]
    fn slash_commands_render_hotkeys_and_changelog() {
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

        for command in ["/hotkeys", "/changelog"] {
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
        assert!(
            rendered.contains("Keyboard Shortcuts"),
            "output: {rendered}"
        );
        assert!(rendered.contains("## ["), "output: {rendered}");

        faux.unregister();
    }

    #[test]
    fn login_slash_command_requests_oauth_picker_transition() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-login-picker-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-login-picker-faux-1".into(),
                name: Some("Slash Login Picker Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-login-picker-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-login-picker-cwd");
        let agent_dir = unique_temp_dir("slash-login-picker-agent");
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
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context =
            test_slash_command_context_with_agent_dir(&keybindings, cwd, Some(agent_dir));

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/login",
            &core,
            core.model_registry().as_ref(),
            &[],
            None,
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));
        assert!(exit_requested.load(Ordering::Relaxed));
        assert_eq!(
            transition_request
                .lock()
                .expect("interactive transition request mutex poisoned")
                .clone(),
            Some(InteractiveTransitionRequest::OAuthPicker(
                OAuthPickerMode::Login,
            ))
        );

        faux.unregister();
    }

    #[test]
    fn logout_slash_command_requests_oauth_picker_transition() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-logout-picker-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-logout-picker-faux-1".into(),
                name: Some("Slash Logout Picker Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-logout-picker-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-logout-picker-cwd");
        let agent_dir = unique_temp_dir("slash-logout-picker-agent");
        fs::write(
            agent_dir.join("auth.json"),
            serde_json::json!({
                "anthropic": {
                    "type": "oauth",
                    "refresh": "refresh-token",
                    "access": "access-token",
                    "expires": 123
                }
            })
            .to_string(),
        )
        .unwrap();

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
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context =
            test_slash_command_context_with_agent_dir(&keybindings, cwd, Some(agent_dir));

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/logout",
            &core,
            core.model_registry().as_ref(),
            &[],
            None,
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));
        assert!(exit_requested.load(Ordering::Relaxed));
        assert_eq!(
            transition_request
                .lock()
                .expect("interactive transition request mutex poisoned")
                .clone(),
            Some(InteractiveTransitionRequest::OAuthPicker(
                OAuthPickerMode::Logout,
            ))
        );

        faux.unregister();
    }

    #[test]
    fn logout_slash_command_removes_saved_oauth_provider() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-logout-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-logout-faux-1".into(),
                name: Some("Slash Logout Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-logout-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-logout-cwd");
        let agent_dir = unique_temp_dir("slash-logout-agent");
        fs::write(
            agent_dir.join("auth.json"),
            serde_json::json!({
                "anthropic": {
                    "type": "oauth",
                    "refresh": "refresh-token",
                    "access": "access-token",
                    "expires": 123
                }
            })
            .to_string(),
        )
        .unwrap();

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
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context =
            test_slash_command_context_with_agent_dir(&keybindings, cwd, Some(agent_dir.clone()));

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/logout anthropic",
            &core,
            core.model_registry().as_ref(),
            &[],
            None,
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));

        let providers = list_persisted_oauth_providers(&agent_dir.join("auth.json")).unwrap();
        assert!(providers.is_empty(), "providers: {providers:?}");

        let rendered = shell.render(100).join("\n");
        assert!(
            rendered.contains("Logged out of Anthropic"),
            "output: {rendered}"
        );

        faux.unregister();
    }

    #[tokio::test]
    async fn oauth_login_picker_transition_persists_credentials() {
        let _guard = oauth_registry_lock().lock().unwrap();
        register_oauth_provider(Arc::new(TestOAuthProvider {
            id: "zz-transition-oauth",
            name: "Transition OAuth Provider",
            access_token: "transition-access-token",
        }));

        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "oauth-login-transition-faux".into(),
            models: vec![FauxModelDefinition {
                id: "oauth-login-transition-faux-1".into(),
                name: Some("OAuth Login Transition Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("oauth-login-transition-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("oauth-login-transition-cwd");
        let agent_dir = unique_temp_dir("oauth-login-transition-agent");
        let terminal = LifecycleScriptedTerminal::new(vec![
            (Duration::from_millis(5), String::from("\x1b[B")),
            (Duration::from_millis(5), String::from("\x1b[B")),
            (Duration::from_millis(5), String::from("\r")),
        ]);
        let runtime = InteractiveRuntime::new(Arc::new(move || Box::new(terminal.clone())));

        let plan = resolve_interactive_transition(
            InteractiveTransitionRequest::OAuthPicker(OAuthPickerMode::Login),
            Some(InteractiveSessionContext {
                manager: Some(SessionManager::in_memory(cwd.to_str().unwrap())),
                session_file: None,
                session_dir: None,
                cwd: cwd.to_string_lossy().into_owned(),
                model,
                thinking_level: ThinkingLevel::Off,
                scoped_models: Vec::new(),
                available_models: Vec::new(),
                runtime_settings: LoadedRuntimeSettings::default(),
            }),
            &cwd,
            Some(agent_dir.as_path()),
            &runtime,
        )
        .await
        .expect("expected oauth login transition plan");

        assert_eq!(
            plan.initial_status_message.as_deref(),
            Some("Logged in to Transition OAuth Provider")
        );
        let auth: Value =
            serde_json::from_str(&fs::read_to_string(agent_dir.join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(
            auth.pointer("/zz-transition-oauth/access")
                .and_then(Value::as_str),
            Some("transition-access-token")
        );

        faux.unregister();
        unregister_oauth_provider("zz-transition-oauth");
    }

    #[tokio::test]
    async fn oauth_logout_picker_transition_removes_saved_provider() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "oauth-logout-transition-faux".into(),
            models: vec![FauxModelDefinition {
                id: "oauth-logout-transition-faux-1".into(),
                name: Some("OAuth Logout Transition Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("oauth-logout-transition-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("oauth-logout-transition-cwd");
        let agent_dir = unique_temp_dir("oauth-logout-transition-agent");
        fs::write(
            agent_dir.join("auth.json"),
            serde_json::json!({
                "anthropic": {
                    "type": "oauth",
                    "refresh": "refresh-token",
                    "access": "access-token",
                    "expires": 123
                }
            })
            .to_string(),
        )
        .unwrap();
        let terminal =
            LifecycleScriptedTerminal::new(vec![(Duration::from_millis(5), String::from("\r"))]);
        let runtime = InteractiveRuntime::new(Arc::new(move || Box::new(terminal.clone())));

        let plan = resolve_interactive_transition(
            InteractiveTransitionRequest::OAuthPicker(OAuthPickerMode::Logout),
            Some(InteractiveSessionContext {
                manager: Some(SessionManager::in_memory(cwd.to_str().unwrap())),
                session_file: None,
                session_dir: None,
                cwd: cwd.to_string_lossy().into_owned(),
                model,
                thinking_level: ThinkingLevel::Off,
                scoped_models: Vec::new(),
                available_models: Vec::new(),
                runtime_settings: LoadedRuntimeSettings::default(),
            }),
            &cwd,
            Some(agent_dir.as_path()),
            &runtime,
        )
        .await
        .expect("expected oauth logout transition plan");

        assert_eq!(
            plan.initial_status_message.as_deref(),
            Some("Logged out of Anthropic (Claude Pro/Max)")
        );
        let providers = list_persisted_oauth_providers(&agent_dir.join("auth.json")).unwrap();
        assert!(providers.is_empty(), "providers: {providers:?}");

        faux.unregister();
    }

    #[test]
    fn reload_slash_command_requests_reload_transition() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-reload-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-reload-faux-1".into(),
                name: Some("Slash Reload Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-reload-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-reload-cwd");
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
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context = test_slash_command_context(&keybindings, cwd);

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/reload",
            &core,
            core.model_registry().as_ref(),
            &[],
            None,
            &slash_command_context,
            &status_handle,
            &footer_state_handle,
            &exit_requested,
            &transition_request,
        ));
        assert!(exit_requested.load(Ordering::Relaxed));
        assert_eq!(
            transition_request
                .lock()
                .expect("interactive transition request mutex poisoned")
                .clone(),
            Some(InteractiveTransitionRequest::Reload)
        );

        faux.unregister();
    }

    #[tokio::test]
    async fn settings_picker_transition_updates_runtime_settings_file() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "settings-picker-transition-faux".into(),
            models: vec![FauxModelDefinition {
                id: "settings-picker-transition-faux-1".into(),
                name: Some("Settings Picker Transition Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("settings-picker-transition-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("settings-picker-transition-cwd");
        let agent_dir = unique_temp_dir("settings-picker-transition-agent");
        let terminal = LifecycleScriptedTerminal::new(vec![
            (Duration::from_millis(5), String::from("\r")),
            (Duration::from_millis(5), String::from("\x1b")),
        ]);
        let runtime = InteractiveRuntime::new(Arc::new(move || Box::new(terminal.clone())));

        let plan = resolve_interactive_transition(
            InteractiveTransitionRequest::SettingsPicker,
            Some(InteractiveSessionContext {
                manager: Some(SessionManager::in_memory(cwd.to_str().unwrap())),
                session_file: None,
                session_dir: None,
                cwd: cwd.to_string_lossy().into_owned(),
                model,
                thinking_level: ThinkingLevel::Off,
                scoped_models: Vec::new(),
                available_models: Vec::new(),
                runtime_settings: LoadedRuntimeSettings::default(),
            }),
            &cwd,
            Some(agent_dir.as_path()),
            &runtime,
        )
        .await
        .expect("expected settings transition plan");

        assert_eq!(
            plan.initial_status_message.as_deref(),
            Some("Updated settings")
        );
        assert!(!plan.runtime_settings.settings.compaction.enabled);

        let persisted = fs::read_to_string(agent_dir.join("settings.json"))
            .expect("expected persisted settings.json");
        assert!(
            persisted.contains("\"enabled\": false"),
            "settings: {persisted}"
        );

        faux.unregister();
    }

    #[tokio::test]
    async fn scoped_models_picker_transition_updates_scope_and_saves_settings() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "scoped-models-transition-faux".into(),
            models: vec![
                FauxModelDefinition {
                    id: "alpha-model".into(),
                    name: Some("Alpha Model".into()),
                    reasoning: false,
                },
                FauxModelDefinition {
                    id: "beta-model".into(),
                    name: Some("Beta Model".into()),
                    reasoning: false,
                },
            ],
            ..RegisterFauxProviderOptions::default()
        });
        let alpha = faux
            .get_model(Some("alpha-model"))
            .expect("expected alpha faux model");
        let beta = faux
            .get_model(Some("beta-model"))
            .expect("expected beta faux model");
        let cwd = unique_temp_dir("scoped-models-transition-cwd");
        let agent_dir = unique_temp_dir("scoped-models-transition-agent");
        let terminal = LifecycleScriptedTerminal::new(vec![
            (Duration::from_millis(5), String::from("\r")),
            (Duration::from_millis(5), String::from("\x13")),
            (Duration::from_millis(5), String::from("\x1b")),
        ]);
        let runtime = InteractiveRuntime::new(Arc::new(move || Box::new(terminal.clone())));

        let plan = resolve_interactive_transition(
            InteractiveTransitionRequest::ScopedModelsPicker {
                initial_search: None,
            },
            Some(InteractiveSessionContext {
                manager: Some(SessionManager::in_memory(cwd.to_str().unwrap())),
                session_file: None,
                session_dir: None,
                cwd: cwd.to_string_lossy().into_owned(),
                model: alpha.clone(),
                thinking_level: ThinkingLevel::Off,
                scoped_models: Vec::new(),
                available_models: vec![alpha.clone(), beta.clone()],
                runtime_settings: LoadedRuntimeSettings::default(),
            }),
            &cwd,
            Some(agent_dir.as_path()),
            &runtime,
        )
        .await
        .expect("expected scoped models transition plan");

        assert_eq!(plan.scoped_models.len(), 1);
        assert_eq!(plan.scoped_models[0].model.id, alpha.id);
        assert_eq!(
            plan.initial_status_message.as_deref(),
            Some("Updated session model scope and saved to settings")
        );

        let persisted = fs::read_to_string(agent_dir.join("settings.json"))
            .expect("expected persisted settings.json");
        assert!(
            persisted.contains(&format!("\"{}/{}\"", alpha.provider, alpha.id)),
            "settings: {persisted}"
        );

        faux.unregister();
    }

    #[test]
    fn slash_export_command_defaults_to_html_snapshot() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-export-html-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-export-html-faux-1".into(),
                name: Some("Slash Export HTML Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-export-html-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-export-html-cwd");
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
                    text: String::from("export html"),
                }],
                timestamp: 1,
            })
            .unwrap();
        let expected_export_path = cwd.join(export_html::default_html_file_name(&manager));
        let session_manager = Arc::new(Mutex::new(manager));
        let status_handle = shell.status_handle();
        let footer_state_handle = shell.footer_state_handle();
        let exit_requested = Arc::new(AtomicBool::new(false));
        let transition_request = Arc::new(Mutex::new(None));
        let slash_command_context = test_slash_command_context(&keybindings, cwd.clone());

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/export",
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

        let exported = fs::read_to_string(&expected_export_path).expect("expected exported html");
        assert!(exported.contains("<!DOCTYPE html>"), "content: {exported}");
        assert!(
            exported.contains("Current branch snapshot"),
            "content: {exported}"
        );
        assert!(exported.contains("export html"), "content: {exported}");

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

    #[tokio::test]
    async fn run_command_exports_session_file_to_html() {
        let cwd = unique_temp_dir("run-command-export");
        let input_path = cwd.join("source-session.jsonl");
        let mut manager = SessionManager::in_memory(cwd.to_str().unwrap());
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("export from cli"),
                }],
                timestamp: 1,
            })
            .unwrap();
        manager.export_branch_jsonl(&input_path).unwrap();
        let expected_output = cwd.join("pi-session-source-session.html");

        let result = run_command(RunCommandOptions {
            args: vec![
                String::from("--export"),
                input_path.to_string_lossy().into_owned(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::default()),
            built_in_models: Vec::new(),
            models_json_path: None,
            agent_dir: None,
            cwd: cwd.clone(),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        })
        .await;

        assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
        assert_eq!(result.stdout.trim(), expected_output.to_string_lossy());
        let exported = fs::read_to_string(&expected_output).expect("expected exported html");
        assert!(exported.contains("export from cli"), "content: {exported}");
    }

    #[test]
    fn parse_shared_session_links_builds_share_viewer_url() {
        let links = parse_shared_session_links(b"https://gist.github.com/badlogic/abc123\n")
            .expect("expected gist links");

        assert_eq!(links.gist_url, "https://gist.github.com/badlogic/abc123");
        assert_eq!(links.preview_url, "https://pi.dev/session/#abc123");
    }

    #[tokio::test]
    async fn interactive_compaction_helper_appends_compaction_entry_and_rebuilds_state() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "interactive-compaction-faux".into(),
            models: vec![FauxModelDefinition {
                id: "interactive-compaction-faux-1".into(),
                name: Some("Interactive Compaction Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        faux.set_responses(vec![
            FauxResponse::text(
                "## Goal\nCompact the conversation\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] Captured earlier work\n\n### In Progress\n- [ ] Continue after compaction\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Compaction**: Keep recent context\n\n## Next Steps\n1. Continue the task\n\n## Critical Context\n- (none)",
            ),
            FauxResponse::text(
                "## Original Request\nContinue the task\n\n## Early Progress\n- Kept the recent context\n\n## Context for Suffix\n- The latest messages remain in state",
            ),
        ]);
        let model = faux
            .get_model(Some("interactive-compaction-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("interactive-compaction-cwd");
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
        let mut manager = SessionManager::in_memory(cwd.to_str().unwrap());
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("first turn"),
                }],
                timestamp: 1,
            })
            .unwrap();
        manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("first answer"),
                    text_signature: None,
                }],
                api: String::from("faux:test"),
                provider: String::from("interactive-compaction-faux"),
                model: String::from("interactive-compaction-faux-1"),
                response_id: None,
                usage: pi_events::Usage {
                    input: 20_000,
                    output: 1,
                    cache_read: 0,
                    cache_write: 0,
                    total_tokens: 20_001,
                    cost: Default::default(),
                },
                stop_reason: pi_events::StopReason::Stop,
                error_message: None,
                timestamp: 2,
            })
            .unwrap();
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("second turn"),
                }],
                timestamp: 3,
            })
            .unwrap();
        manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("second answer"),
                    text_signature: None,
                }],
                api: String::from("faux:test"),
                provider: String::from("interactive-compaction-faux"),
                model: String::from("interactive-compaction-faux-1"),
                response_id: None,
                usage: Default::default(),
                stop_reason: pi_events::StopReason::Stop,
                error_message: None,
                timestamp: 4,
            })
            .unwrap();
        let initial_context = manager.build_session_context();
        core.agent().update_state(move |state| {
            state.messages = initial_context.messages.clone();
        });
        let session_manager = Arc::new(Mutex::new(manager));

        let result = run_interactive_compaction(
            &core,
            &session_manager,
            &CompactionSettings {
                enabled: true,
                reserve_tokens: 16_384,
                keep_recent_tokens: 4,
            },
            None,
        )
        .await
        .expect("expected compaction result")
        .expect("expected compaction to run");

        assert!(result.summary.contains("## Goal"));
        let entries = session_manager
            .lock()
            .expect("session manager mutex poisoned")
            .get_entries()
            .to_vec();
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry, SessionEntry::Compaction { .. }))
        );

        let state = core.state();
        assert!(matches!(
            state
                .messages
                .first()
                .and_then(|message| message.as_standard_message()),
            None
        ));
        assert!(
            state
                .messages
                .iter()
                .any(|message| matches!(message.role(), "compactionSummary"))
        );

        faux.unregister();
    }

    #[test]
    fn tree_slash_command_requests_tree_picker_transition() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "slash-tree-picker-faux".into(),
            models: vec![FauxModelDefinition {
                id: "slash-tree-picker-faux-1".into(),
                name: Some("Slash Tree Picker Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("slash-tree-picker-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("slash-tree-picker-cwd");
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
                    text: String::from("root"),
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

        assert!(handle_interactive_slash_command(
            &mut shell,
            "/tree",
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

        assert!(exit_requested.load(Ordering::Relaxed));
        assert_eq!(
            transition_request
                .lock()
                .expect("interactive transition request mutex poisoned")
                .clone(),
            Some(InteractiveTransitionRequest::TreePicker)
        );

        faux.unregister();
    }

    #[tokio::test]
    async fn tree_picker_transition_switches_session_context() {
        let faux = register_faux_provider(RegisterFauxProviderOptions {
            provider: "tree-picker-transition-faux".into(),
            models: vec![FauxModelDefinition {
                id: "tree-picker-transition-faux-1".into(),
                name: Some("Tree Picker Transition Faux".into()),
                reasoning: false,
            }],
            ..RegisterFauxProviderOptions::default()
        });
        let model = faux
            .get_model(Some("tree-picker-transition-faux-1"))
            .expect("expected faux model");
        let cwd = unique_temp_dir("tree-picker-transition-cwd");
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
                provider: String::from("tree-picker-transition-faux"),
                model: String::from("tree-picker-transition-faux-1"),
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

        let terminal = LifecycleScriptedTerminal::new(vec![
            (Duration::from_millis(5), String::from("\x1b[A")),
            (Duration::from_millis(5), String::from("\r")),
        ]);
        let runtime = InteractiveRuntime::new(Arc::new(move || Box::new(terminal.clone())));

        let plan = resolve_interactive_transition(
            InteractiveTransitionRequest::TreePicker,
            Some(InteractiveSessionContext {
                manager: Some(manager),
                session_file: None,
                session_dir: None,
                cwd: cwd.to_string_lossy().into_owned(),
                model,
                thinking_level: ThinkingLevel::Off,
                scoped_models: Vec::new(),
                available_models: Vec::new(),
                runtime_settings: LoadedRuntimeSettings::default(),
            }),
            &cwd,
            None,
            &runtime,
        )
        .await
        .expect("expected tree transition plan");

        let manager = plan.manager.expect("expected session manager");
        assert_eq!(manager.get_leaf_id(), Some(primary_user_id.as_str()));
        assert_eq!(
            plan.initial_status_message.as_deref(),
            Some("Navigated to selected point")
        );

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
                scoped_models: Vec::new(),
                available_models: Vec::new(),
                runtime_settings: LoadedRuntimeSettings::default(),
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
