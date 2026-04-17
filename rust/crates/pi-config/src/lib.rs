use pi_ai::Transport;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

const CONFIG_DIR_NAME: &str = ".pi";
const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsScope {
    Global,
    Project,
}

impl SettingsScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsWarning {
    pub scope: SettingsScope,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError {
    pub scope: SettingsScope,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSettings {
    pub auto_resize_images: bool,
    pub block_images: bool,
}

impl Default for ImageSettings {
    fn default() -> Self {
        Self {
            auto_resize_images: true,
            block_images: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            reserve_tokens: 16_384,
            keep_recent_tokens: 20_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSummarySettings {
    pub reserve_tokens: u64,
    pub skip_prompt: bool,
}

impl Default for BranchSummarySettings {
    fn default() -> Self {
        Self {
            reserve_tokens: 16_384,
            skip_prompt: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrySettings {
    pub enabled: bool,
    pub max_retries: u64,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetrySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 3,
            base_delay_ms: 2_000,
            max_delay_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSettings {
    pub show_images: bool,
    pub clear_on_shrink: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            show_images: true,
            clear_on_shrink: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownSettings {
    pub code_block_indent: String,
}

impl Default for MarkdownSettings {
    fn default() -> Self {
        Self {
            code_block_indent: String::from("  "),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingBudgetsSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimal: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettings {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_thinking_level: Option<String>,
    pub transport: Transport,
    pub steering_mode: String,
    pub follow_up_mode: String,
    pub theme: Option<String>,
    pub compaction: CompactionSettings,
    pub branch_summary: BranchSummarySettings,
    pub retry: RetrySettings,
    pub hide_thinking_block: bool,
    pub shell_path: Option<String>,
    pub quiet_startup: bool,
    pub shell_command_prefix: Option<String>,
    pub collapse_changelog: bool,
    pub enable_skill_commands: bool,
    pub terminal: TerminalSettings,
    pub images: ImageSettings,
    pub enabled_models: Option<Vec<String>>,
    pub double_escape_action: String,
    pub tree_filter_mode: String,
    pub thinking_budgets: ThinkingBudgetsSettings,
    pub editor_padding_x: usize,
    pub autocomplete_max_visible: usize,
    pub show_hardware_cursor: bool,
    pub markdown: MarkdownSettings,
    pub session_dir: Option<String>,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            default_provider: None,
            default_model: None,
            default_thinking_level: None,
            transport: Transport::Sse,
            steering_mode: String::from("one-at-a-time"),
            follow_up_mode: String::from("one-at-a-time"),
            theme: None,
            compaction: CompactionSettings::default(),
            branch_summary: BranchSummarySettings::default(),
            retry: RetrySettings::default(),
            hide_thinking_block: false,
            shell_path: None,
            quiet_startup: false,
            shell_command_prefix: None,
            collapse_changelog: false,
            enable_skill_commands: true,
            terminal: TerminalSettings::default(),
            images: ImageSettings::default(),
            enabled_models: None,
            double_escape_action: String::from("tree"),
            tree_filter_mode: String::from("default"),
            thinking_budgets: ThinkingBudgetsSettings::default(),
            editor_padding_x: 0,
            autocomplete_max_visible: 5,
            show_hardware_cursor: false,
            markdown: MarkdownSettings::default(),
            session_dir: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilteredPackageSource {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub themes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PackageSource {
    Plain(String),
    Filtered(FilteredPackageSource),
}

impl PackageSource {
    pub fn source(&self) -> &str {
        match self {
            Self::Plain(source) => source,
            Self::Filtered(source) => &source.source,
        }
    }

    pub fn is_filtered(&self) -> bool {
        matches!(self, Self::Filtered(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceSettings {
    pub npm_command: Option<Vec<String>>,
    pub packages: Vec<PackageSource>,
    pub extensions: Vec<String>,
    pub skills: Vec<String>,
    pub prompts: Vec<String>,
    pub themes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedResourceSettings {
    pub global: ResourceSettings,
    pub project: ResourceSettings,
    pub warnings: Vec<SettingsWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedRuntimeSettings {
    pub settings: RuntimeSettings,
    pub warnings: Vec<SettingsWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompactionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_recent_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_prompt: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RetryConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TerminalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_on_shrink: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImageConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_resize: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_images: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_block_indent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_changelog_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steering_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_up_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_summary: Option<BranchSummaryConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_thinking_block: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_startup: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_command_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm_command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapse_changelog: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<Vec<PackageSource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub themes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_skill_commands: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<ImageConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double_escape_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_filter_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<ThinkingBudgetsSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_padding_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autocomplete_max_visible: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_hardware_cursor: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<MarkdownConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_dir: Option<String>,
}

pub trait SettingsStorage: Clone {
    fn read(&self, scope: SettingsScope) -> Result<Option<String>, String>;

    fn update<F>(&self, scope: SettingsScope, update: F) -> Result<(), String>
    where
        F: FnOnce(Option<String>) -> Result<Option<String>, String>;
}

#[derive(Debug, Clone)]
pub struct FileSettingsStorage {
    global_settings_path: PathBuf,
    project_settings_path: PathBuf,
}

impl FileSettingsStorage {
    pub fn new(cwd: impl AsRef<Path>, agent_dir: impl AsRef<Path>) -> Self {
        Self {
            global_settings_path: agent_dir.as_ref().join(SETTINGS_FILE_NAME),
            project_settings_path: cwd.as_ref().join(CONFIG_DIR_NAME).join(SETTINGS_FILE_NAME),
        }
    }

    fn path_for_scope(&self, scope: SettingsScope) -> &Path {
        match scope {
            SettingsScope::Global => &self.global_settings_path,
            SettingsScope::Project => &self.project_settings_path,
        }
    }

    fn lock_path(path: &Path) -> PathBuf {
        PathBuf::from(format!("{}.lock", path.to_string_lossy()))
    }

    fn acquire_lock_sync_with_retry(&self, path: &Path) -> Result<SettingsLock, String> {
        let max_attempts = 10;
        let delay = Duration::from_millis(20);
        let lock_path = Self::lock_path(path);

        for attempt in 1..=max_attempts {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(file) => {
                    return Ok(SettingsLock {
                        _file: file,
                        path: lock_path,
                    });
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && attempt < max_attempts =>
                {
                    sleep(delay);
                }
                Err(error) => return Err(error.to_string()),
            }
        }

        Err(String::from("Failed to acquire settings lock"))
    }
}

impl SettingsStorage for FileSettingsStorage {
    fn read(&self, scope: SettingsScope) -> Result<Option<String>, String> {
        let path = self.path_for_scope(scope);
        match fs::read_to_string(path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn update<F>(&self, scope: SettingsScope, update: F) -> Result<(), String>
    where
        F: FnOnce(Option<String>) -> Result<Option<String>, String>,
    {
        let path = self.path_for_scope(scope);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let _lock = self.acquire_lock_sync_with_retry(path)?;
        let current = match fs::read_to_string(path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };

        if let Some(next) = update(current)? {
            fs::write(path, next).map_err(|error| error.to_string())?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemorySettingsStorage {
    inner: Arc<Mutex<InMemorySettingsStorageInner>>,
}

#[derive(Debug, Default)]
struct InMemorySettingsStorageInner {
    global: Option<String>,
    project: Option<String>,
}

impl InMemorySettingsStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn raw(&self, scope: SettingsScope) -> Option<String> {
        let inner = self
            .inner
            .lock()
            .expect("in-memory settings mutex poisoned");
        match scope {
            SettingsScope::Global => inner.global.clone(),
            SettingsScope::Project => inner.project.clone(),
        }
    }

    pub fn set_raw(&self, scope: SettingsScope, content: Option<String>) {
        let mut inner = self
            .inner
            .lock()
            .expect("in-memory settings mutex poisoned");
        match scope {
            SettingsScope::Global => inner.global = content,
            SettingsScope::Project => inner.project = content,
        }
    }
}

impl SettingsStorage for InMemorySettingsStorage {
    fn read(&self, scope: SettingsScope) -> Result<Option<String>, String> {
        Ok(self.raw(scope))
    }

    fn update<F>(&self, scope: SettingsScope, update: F) -> Result<(), String>
    where
        F: FnOnce(Option<String>) -> Result<Option<String>, String>,
    {
        let current = self.raw(scope);
        if let Some(next) = update(current)? {
            self.set_raw(scope, Some(next));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct SettingsLock {
    _file: fs::File,
    path: PathBuf,
}

impl Drop for SettingsLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub struct SettingsManager<S: SettingsStorage = FileSettingsStorage> {
    storage: S,
    global_settings: Settings,
    project_settings: Settings,
    settings: Settings,
    modified_fields: BTreeSet<String>,
    modified_nested_fields: BTreeMap<String, BTreeSet<String>>,
    modified_project_fields: BTreeSet<String>,
    modified_project_nested_fields: BTreeMap<String, BTreeSet<String>>,
    global_settings_load_error: Option<String>,
    project_settings_load_error: Option<String>,
    errors: Vec<SettingsError>,
}

impl SettingsManager<FileSettingsStorage> {
    pub fn create(cwd: impl AsRef<Path>, agent_dir: impl AsRef<Path>) -> Self {
        Self::from_storage(FileSettingsStorage::new(cwd, agent_dir))
    }
}

impl SettingsManager<InMemorySettingsStorage> {
    pub fn in_memory(settings: Settings) -> Self {
        Self::in_memory_with_scopes(settings, Settings::default())
    }

    pub fn in_memory_with_scopes(global: Settings, project: Settings) -> Self {
        let storage = InMemorySettingsStorage::new();
        if !settings_is_empty(&global) {
            storage.set_raw(
                SettingsScope::Global,
                Some(render_settings(&global).expect("in-memory settings should serialize")),
            );
        }
        if !settings_is_empty(&project) {
            storage.set_raw(
                SettingsScope::Project,
                Some(render_settings(&project).expect("in-memory settings should serialize")),
            );
        }
        Self::from_storage(storage)
    }
}

impl<S: SettingsStorage> SettingsManager<S> {
    pub fn from_storage(storage: S) -> Self {
        let global_load = Self::try_load_from_storage(&storage, SettingsScope::Global);
        let project_load = Self::try_load_from_storage(&storage, SettingsScope::Project);
        let mut errors = Vec::new();
        if let Some(message) = global_load.error.clone() {
            errors.push(SettingsError {
                scope: SettingsScope::Global,
                message,
            });
        }
        if let Some(message) = project_load.error.clone() {
            errors.push(SettingsError {
                scope: SettingsScope::Project,
                message,
            });
        }

        let settings = deep_merge_settings(&global_load.settings, &project_load.settings);

        Self {
            storage,
            global_settings: global_load.settings,
            project_settings: project_load.settings,
            settings,
            modified_fields: BTreeSet::new(),
            modified_nested_fields: BTreeMap::new(),
            modified_project_fields: BTreeSet::new(),
            modified_project_nested_fields: BTreeMap::new(),
            global_settings_load_error: global_load.error,
            project_settings_load_error: project_load.error,
            errors,
        }
    }

    pub fn global_settings(&self) -> Settings {
        self.global_settings.clone()
    }

    pub fn project_settings(&self) -> Settings {
        self.project_settings.clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings.clone()
    }

    pub fn runtime_settings(&self) -> RuntimeSettings {
        resolve_runtime_settings(&self.settings)
    }

    pub fn loaded_runtime_settings(&self) -> LoadedRuntimeSettings {
        let mut loaded = LoadedRuntimeSettings {
            settings: RuntimeSettings::default(),
            warnings: self
                .errors
                .iter()
                .map(|error| SettingsWarning {
                    scope: error.scope,
                    message: error.message.clone(),
                })
                .collect(),
        };

        apply_runtime_scope(&mut loaded, &self.global_settings, SettingsScope::Global);
        apply_runtime_scope(&mut loaded, &self.project_settings, SettingsScope::Project);

        let merged = deep_merge_settings(&self.global_settings, &self.project_settings);
        if merged
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.clear_on_shrink)
            .is_none()
            && std::env::var("PI_CLEAR_ON_SHRINK").ok().as_deref() == Some("1")
        {
            loaded.settings.terminal.clear_on_shrink = true;
        }
        if merged.show_hardware_cursor.is_none()
            && std::env::var("PI_HARDWARE_CURSOR").ok().as_deref() == Some("1")
        {
            loaded.settings.show_hardware_cursor = true;
        }

        loaded
    }

    pub fn loaded_resource_settings(&self) -> LoadedResourceSettings {
        let mut loaded = LoadedResourceSettings {
            global: ResourceSettings::default(),
            project: ResourceSettings::default(),
            warnings: self
                .errors
                .iter()
                .map(|error| SettingsWarning {
                    scope: error.scope,
                    message: error.message.clone(),
                })
                .collect(),
        };

        apply_resource_scope(&mut loaded.global, &self.global_settings);
        apply_resource_scope(&mut loaded.project, &self.project_settings);

        loaded
    }

    pub fn reload(&mut self) {
        let global_load = Self::try_load_from_storage(&self.storage, SettingsScope::Global);
        if let Some(message) = global_load.error {
            self.global_settings_load_error = Some(message.clone());
            self.record_error(SettingsScope::Global, message);
        } else {
            self.global_settings = global_load.settings;
            self.global_settings_load_error = None;
        }

        self.modified_fields.clear();
        self.modified_nested_fields.clear();
        self.modified_project_fields.clear();
        self.modified_project_nested_fields.clear();

        let project_load = Self::try_load_from_storage(&self.storage, SettingsScope::Project);
        if let Some(message) = project_load.error {
            self.project_settings_load_error = Some(message.clone());
            self.record_error(SettingsScope::Project, message);
        } else {
            self.project_settings = project_load.settings;
            self.project_settings_load_error = None;
        }

        self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);
    }

    pub fn apply_overrides(&mut self, overrides: Settings) {
        self.settings = deep_merge_settings(&self.settings, &overrides);
    }

    pub fn flush(&mut self) {}

    pub fn drain_errors(&mut self) -> Vec<SettingsError> {
        std::mem::take(&mut self.errors)
    }

    pub fn last_changelog_version(&self) -> Option<String> {
        self.settings.last_changelog_version.clone()
    }

    pub fn set_last_changelog_version(&mut self, version: impl Into<String>) {
        let version = version.into();
        self.update_global("lastChangelogVersion", None, move |settings| {
            settings.last_changelog_version = Some(version);
        });
    }

    pub fn session_dir(&self) -> Option<String> {
        self.settings.session_dir.clone()
    }

    pub fn set_session_dir(&mut self, session_dir: Option<String>) {
        self.update_global("sessionDir", None, move |settings| {
            settings.session_dir = session_dir;
        });
    }

    pub fn default_provider(&self) -> Option<String> {
        self.settings.default_provider.clone()
    }

    pub fn default_model(&self) -> Option<String> {
        self.settings.default_model.clone()
    }

    pub fn default_thinking_level(&self) -> Option<String> {
        self.settings.default_thinking_level.clone()
    }

    pub fn set_default_provider(&mut self, provider: impl Into<String>) {
        let provider = provider.into();
        self.update_global("defaultProvider", None, move |settings| {
            settings.default_provider = Some(provider);
        });
    }

    pub fn set_default_model(&mut self, model_id: impl Into<String>) {
        let model_id = model_id.into();
        self.update_global("defaultModel", None, move |settings| {
            settings.default_model = Some(model_id);
        });
    }

    pub fn set_default_model_and_provider(
        &mut self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) {
        let provider = provider.into();
        let model_id = model_id.into();
        self.update_global_multiple(
            &[("defaultProvider", None), ("defaultModel", None)],
            move |settings| {
                settings.default_provider = Some(provider);
                settings.default_model = Some(model_id);
            },
        );
    }

    pub fn set_default_thinking_level(&mut self, level: impl Into<String>) {
        let level = level.into();
        self.update_global("defaultThinkingLevel", None, move |settings| {
            settings.default_thinking_level = Some(level);
        });
    }

    pub fn steering_mode(&self) -> String {
        self.settings
            .steering_mode
            .clone()
            .unwrap_or_else(|| String::from("one-at-a-time"))
    }

    pub fn set_steering_mode(&mut self, mode: impl Into<String>) {
        let mode = mode.into();
        self.update_global("steeringMode", None, move |settings| {
            settings.steering_mode = Some(mode);
        });
    }

    pub fn follow_up_mode(&self) -> String {
        self.settings
            .follow_up_mode
            .clone()
            .unwrap_or_else(|| String::from("one-at-a-time"))
    }

    pub fn set_follow_up_mode(&mut self, mode: impl Into<String>) {
        let mode = mode.into();
        self.update_global("followUpMode", None, move |settings| {
            settings.follow_up_mode = Some(mode);
        });
    }

    pub fn theme(&self) -> Option<String> {
        self.settings.theme.clone()
    }

    pub fn set_theme(&mut self, theme: impl Into<String>) {
        let theme = theme.into();
        self.update_global("theme", None, move |settings| {
            settings.theme = Some(theme);
        });
    }

    pub fn transport(&self) -> Transport {
        parse_transport_setting(self.settings.transport.as_deref()).unwrap_or(Transport::Sse)
    }

    pub fn set_transport(&mut self, transport: Transport) {
        self.update_global("transport", None, move |settings| {
            settings.transport = Some(render_transport(transport));
        });
    }

    pub fn compaction_enabled(&self) -> bool {
        self.settings
            .compaction
            .as_ref()
            .and_then(|settings| settings.enabled)
            .unwrap_or(true)
    }

    pub fn compaction_reserve_tokens(&self) -> u64 {
        self.settings
            .compaction
            .as_ref()
            .and_then(|settings| settings.reserve_tokens)
            .unwrap_or(16_384)
    }

    pub fn compaction_keep_recent_tokens(&self) -> u64 {
        self.settings
            .compaction
            .as_ref()
            .and_then(|settings| settings.keep_recent_tokens)
            .unwrap_or(20_000)
    }

    pub fn compaction_settings(&self) -> CompactionSettings {
        CompactionSettings {
            enabled: self.compaction_enabled(),
            reserve_tokens: self.compaction_reserve_tokens(),
            keep_recent_tokens: self.compaction_keep_recent_tokens(),
        }
    }

    pub fn set_compaction_enabled(&mut self, enabled: bool) {
        self.update_global("compaction", Some("enabled"), move |settings| {
            ensure_compaction(settings).enabled = Some(enabled);
        });
    }

    pub fn set_compaction_reserve_tokens(&mut self, reserve_tokens: u64) {
        self.update_global("compaction", Some("reserveTokens"), move |settings| {
            ensure_compaction(settings).reserve_tokens = Some(reserve_tokens);
        });
    }

    pub fn set_compaction_keep_recent_tokens(&mut self, keep_recent_tokens: u64) {
        self.update_global("compaction", Some("keepRecentTokens"), move |settings| {
            ensure_compaction(settings).keep_recent_tokens = Some(keep_recent_tokens);
        });
    }

    pub fn branch_summary_settings(&self) -> BranchSummarySettings {
        BranchSummarySettings {
            reserve_tokens: self
                .settings
                .branch_summary
                .as_ref()
                .and_then(|settings| settings.reserve_tokens)
                .unwrap_or(16_384),
            skip_prompt: self
                .settings
                .branch_summary
                .as_ref()
                .and_then(|settings| settings.skip_prompt)
                .unwrap_or(false),
        }
    }

    pub fn branch_summary_skip_prompt(&self) -> bool {
        self.branch_summary_settings().skip_prompt
    }

    pub fn set_branch_summary_reserve_tokens(&mut self, reserve_tokens: u64) {
        self.update_global("branchSummary", Some("reserveTokens"), move |settings| {
            ensure_branch_summary(settings).reserve_tokens = Some(reserve_tokens);
        });
    }

    pub fn set_branch_summary_skip_prompt(&mut self, skip_prompt: bool) {
        self.update_global("branchSummary", Some("skipPrompt"), move |settings| {
            ensure_branch_summary(settings).skip_prompt = Some(skip_prompt);
        });
    }

    pub fn retry_enabled(&self) -> bool {
        self.settings
            .retry
            .as_ref()
            .and_then(|settings| settings.enabled)
            .unwrap_or(true)
    }

    pub fn retry_settings(&self) -> RetrySettings {
        RetrySettings {
            enabled: self.retry_enabled(),
            max_retries: self
                .settings
                .retry
                .as_ref()
                .and_then(|settings| settings.max_retries)
                .unwrap_or(3),
            base_delay_ms: self
                .settings
                .retry
                .as_ref()
                .and_then(|settings| settings.base_delay_ms)
                .unwrap_or(2_000),
            max_delay_ms: self
                .settings
                .retry
                .as_ref()
                .and_then(|settings| settings.max_delay_ms)
                .unwrap_or(60_000),
        }
    }

    pub fn set_retry_enabled(&mut self, enabled: bool) {
        self.update_global("retry", Some("enabled"), move |settings| {
            ensure_retry(settings).enabled = Some(enabled);
        });
    }

    pub fn set_retry_max_retries(&mut self, max_retries: u64) {
        self.update_global("retry", Some("maxRetries"), move |settings| {
            ensure_retry(settings).max_retries = Some(max_retries);
        });
    }

    pub fn set_retry_base_delay_ms(&mut self, base_delay_ms: u64) {
        self.update_global("retry", Some("baseDelayMs"), move |settings| {
            ensure_retry(settings).base_delay_ms = Some(base_delay_ms);
        });
    }

    pub fn set_retry_max_delay_ms(&mut self, max_delay_ms: u64) {
        self.update_global("retry", Some("maxDelayMs"), move |settings| {
            ensure_retry(settings).max_delay_ms = Some(max_delay_ms);
        });
    }

    pub fn hide_thinking_block(&self) -> bool {
        self.settings.hide_thinking_block.unwrap_or(false)
    }

    pub fn set_hide_thinking_block(&mut self, hide: bool) {
        self.update_global("hideThinkingBlock", None, move |settings| {
            settings.hide_thinking_block = Some(hide);
        });
    }

    pub fn shell_path(&self) -> Option<String> {
        self.settings.shell_path.clone()
    }

    pub fn set_shell_path(&mut self, path: Option<String>) {
        self.update_global("shellPath", None, move |settings| {
            settings.shell_path = path;
        });
    }

    pub fn quiet_startup(&self) -> bool {
        self.settings.quiet_startup.unwrap_or(false)
    }

    pub fn set_quiet_startup(&mut self, quiet: bool) {
        self.update_global("quietStartup", None, move |settings| {
            settings.quiet_startup = Some(quiet);
        });
    }

    pub fn shell_command_prefix(&self) -> Option<String> {
        self.settings.shell_command_prefix.clone()
    }

    pub fn set_shell_command_prefix(&mut self, prefix: Option<String>) {
        self.update_global("shellCommandPrefix", None, move |settings| {
            settings.shell_command_prefix = prefix;
        });
    }

    pub fn npm_command(&self) -> Option<Vec<String>> {
        self.settings.npm_command.clone()
    }

    pub fn set_npm_command(&mut self, command: Option<Vec<String>>) {
        self.update_global("npmCommand", None, move |settings| {
            settings.npm_command = command;
        });
    }

    pub fn collapse_changelog(&self) -> bool {
        self.settings.collapse_changelog.unwrap_or(false)
    }

    pub fn set_collapse_changelog(&mut self, collapse: bool) {
        self.update_global("collapseChangelog", None, move |settings| {
            settings.collapse_changelog = Some(collapse);
        });
    }

    pub fn packages(&self) -> Vec<PackageSource> {
        self.settings.packages.clone().unwrap_or_default()
    }

    pub fn set_packages(&mut self, packages: Vec<PackageSource>) {
        self.update_global("packages", None, move |settings| {
            settings.packages = Some(packages);
        });
    }

    pub fn set_project_packages(&mut self, packages: Vec<PackageSource>) {
        self.update_project("packages", None, move |settings| {
            settings.packages = Some(packages);
        });
    }

    pub fn extension_paths(&self) -> Vec<String> {
        self.settings.extensions.clone().unwrap_or_default()
    }

    pub fn set_extension_paths(&mut self, paths: Vec<String>) {
        self.update_global("extensions", None, move |settings| {
            settings.extensions = Some(paths);
        });
    }

    pub fn set_project_extension_paths(&mut self, paths: Vec<String>) {
        self.update_project("extensions", None, move |settings| {
            settings.extensions = Some(paths);
        });
    }

    pub fn skill_paths(&self) -> Vec<String> {
        self.settings.skills.clone().unwrap_or_default()
    }

    pub fn set_skill_paths(&mut self, paths: Vec<String>) {
        self.update_global("skills", None, move |settings| {
            settings.skills = Some(paths);
        });
    }

    pub fn set_project_skill_paths(&mut self, paths: Vec<String>) {
        self.update_project("skills", None, move |settings| {
            settings.skills = Some(paths);
        });
    }

    pub fn prompt_template_paths(&self) -> Vec<String> {
        self.settings.prompts.clone().unwrap_or_default()
    }

    pub fn set_prompt_template_paths(&mut self, paths: Vec<String>) {
        self.update_global("prompts", None, move |settings| {
            settings.prompts = Some(paths);
        });
    }

    pub fn set_project_prompt_template_paths(&mut self, paths: Vec<String>) {
        self.update_project("prompts", None, move |settings| {
            settings.prompts = Some(paths);
        });
    }

    pub fn theme_paths(&self) -> Vec<String> {
        self.settings.themes.clone().unwrap_or_default()
    }

    pub fn set_theme_paths(&mut self, paths: Vec<String>) {
        self.update_global("themes", None, move |settings| {
            settings.themes = Some(paths);
        });
    }

    pub fn set_project_theme_paths(&mut self, paths: Vec<String>) {
        self.update_project("themes", None, move |settings| {
            settings.themes = Some(paths);
        });
    }

    pub fn enable_skill_commands(&self) -> bool {
        self.settings.enable_skill_commands.unwrap_or(true)
    }

    pub fn set_enable_skill_commands(&mut self, enabled: bool) {
        self.update_global("enableSkillCommands", None, move |settings| {
            settings.enable_skill_commands = Some(enabled);
        });
    }

    pub fn thinking_budgets(&self) -> Option<ThinkingBudgetsSettings> {
        self.settings.thinking_budgets.clone()
    }

    pub fn set_thinking_budgets(&mut self, budgets: Option<ThinkingBudgetsSettings>) {
        self.update_global("thinkingBudgets", None, move |settings| {
            settings.thinking_budgets = budgets;
        });
    }

    pub fn show_images(&self) -> bool {
        self.settings
            .terminal
            .as_ref()
            .and_then(|settings| settings.show_images)
            .unwrap_or(true)
    }

    pub fn set_show_images(&mut self, show: bool) {
        self.update_global("terminal", Some("showImages"), move |settings| {
            ensure_terminal(settings).show_images = Some(show);
        });
    }

    pub fn clear_on_shrink(&self) -> bool {
        if let Some(value) = self
            .settings
            .terminal
            .as_ref()
            .and_then(|settings| settings.clear_on_shrink)
        {
            return value;
        }
        std::env::var("PI_CLEAR_ON_SHRINK").ok().as_deref() == Some("1")
    }

    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        self.update_global("terminal", Some("clearOnShrink"), move |settings| {
            ensure_terminal(settings).clear_on_shrink = Some(enabled);
        });
    }

    pub fn image_auto_resize(&self) -> bool {
        self.settings
            .images
            .as_ref()
            .and_then(|settings| settings.auto_resize)
            .unwrap_or(true)
    }

    pub fn set_image_auto_resize(&mut self, enabled: bool) {
        self.update_global("images", Some("autoResize"), move |settings| {
            ensure_images(settings).auto_resize = Some(enabled);
        });
    }

    pub fn block_images(&self) -> bool {
        self.settings
            .images
            .as_ref()
            .and_then(|settings| settings.block_images)
            .unwrap_or(false)
    }

    pub fn set_block_images(&mut self, blocked: bool) {
        self.update_global("images", Some("blockImages"), move |settings| {
            ensure_images(settings).block_images = Some(blocked);
        });
    }

    pub fn enabled_models(&self) -> Option<Vec<String>> {
        self.settings.enabled_models.clone()
    }

    pub fn set_enabled_models(&mut self, patterns: Option<Vec<String>>) {
        self.update_global("enabledModels", None, move |settings| {
            settings.enabled_models = patterns;
        });
    }

    pub fn double_escape_action(&self) -> String {
        self.settings
            .double_escape_action
            .clone()
            .unwrap_or_else(|| String::from("tree"))
    }

    pub fn set_double_escape_action(&mut self, action: impl Into<String>) {
        let action = action.into();
        self.update_global("doubleEscapeAction", None, move |settings| {
            settings.double_escape_action = Some(action);
        });
    }

    pub fn tree_filter_mode(&self) -> String {
        let Some(mode) = self.settings.tree_filter_mode.as_deref() else {
            return String::from("default");
        };
        match mode {
            "default" | "no-tools" | "user-only" | "labeled-only" | "all" => mode.to_owned(),
            _ => String::from("default"),
        }
    }

    pub fn set_tree_filter_mode(&mut self, mode: impl Into<String>) {
        let mode = mode.into();
        self.update_global("treeFilterMode", None, move |settings| {
            settings.tree_filter_mode = Some(mode);
        });
    }

    pub fn show_hardware_cursor(&self) -> bool {
        self.settings
            .show_hardware_cursor
            .unwrap_or_else(|| std::env::var("PI_HARDWARE_CURSOR").ok().as_deref() == Some("1"))
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        self.update_global("showHardwareCursor", None, move |settings| {
            settings.show_hardware_cursor = Some(enabled);
        });
    }

    pub fn editor_padding_x(&self) -> usize {
        self.settings
            .editor_padding_x
            .filter(|value| value.is_finite())
            .map(clamp_editor_padding_x)
            .unwrap_or(0)
    }

    pub fn set_editor_padding_x(&mut self, padding: f64) {
        let padding = clamp_editor_padding_x(padding) as f64;
        self.update_global("editorPaddingX", None, move |settings| {
            settings.editor_padding_x = Some(padding);
        });
    }

    pub fn autocomplete_max_visible(&self) -> usize {
        self.settings
            .autocomplete_max_visible
            .filter(|value| value.is_finite())
            .map(clamp_autocomplete_max_visible)
            .unwrap_or(5)
    }

    pub fn set_autocomplete_max_visible(&mut self, max_visible: f64) {
        let max_visible = clamp_autocomplete_max_visible(max_visible) as f64;
        self.update_global("autocompleteMaxVisible", None, move |settings| {
            settings.autocomplete_max_visible = Some(max_visible);
        });
    }

    pub fn code_block_indent(&self) -> String {
        self.settings
            .markdown
            .as_ref()
            .and_then(|settings| settings.code_block_indent.clone())
            .unwrap_or_else(|| String::from("  "))
    }

    pub fn set_code_block_indent(&mut self, indent: Option<String>) {
        self.update_global("markdown", Some("codeBlockIndent"), move |settings| {
            ensure_markdown(settings).code_block_indent = indent;
        });
    }

    fn update_global(
        &mut self,
        field: &str,
        nested_key: Option<&str>,
        update: impl FnOnce(&mut Settings),
    ) {
        update(&mut self.global_settings);
        self.mark_modified(field, nested_key);
        self.save_global();
    }

    fn update_global_multiple(
        &mut self,
        fields: &[(&str, Option<&str>)],
        update: impl FnOnce(&mut Settings),
    ) {
        update(&mut self.global_settings);
        for (field, nested_key) in fields {
            self.mark_modified(field, *nested_key);
        }
        self.save_global();
    }

    fn update_project(
        &mut self,
        field: &str,
        nested_key: Option<&str>,
        update: impl FnOnce(&mut Settings),
    ) {
        update(&mut self.project_settings);
        self.mark_project_modified(field, nested_key);
        self.save_project();
    }

    fn mark_modified(&mut self, field: &str, nested_key: Option<&str>) {
        self.modified_fields.insert(field.to_owned());
        if let Some(nested_key) = nested_key {
            self.modified_nested_fields
                .entry(field.to_owned())
                .or_default()
                .insert(nested_key.to_owned());
        }
    }

    fn mark_project_modified(&mut self, field: &str, nested_key: Option<&str>) {
        self.modified_project_fields.insert(field.to_owned());
        if let Some(nested_key) = nested_key {
            self.modified_project_nested_fields
                .entry(field.to_owned())
                .or_default()
                .insert(nested_key.to_owned());
        }
    }

    fn record_error(&mut self, scope: SettingsScope, message: impl Into<String>) {
        self.errors.push(SettingsError {
            scope,
            message: message.into(),
        });
    }

    fn save_global(&mut self) {
        self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);
        if self.global_settings_load_error.is_some() || self.modified_fields.is_empty() {
            return;
        }

        let snapshot = self.global_settings.clone();
        let modified_fields = self.modified_fields.clone();
        let modified_nested_fields = self.modified_nested_fields.clone();
        match Self::persist_scoped_settings(
            &self.storage,
            SettingsScope::Global,
            &snapshot,
            &modified_fields,
            &modified_nested_fields,
        ) {
            Ok(()) => {
                self.modified_fields.clear();
                self.modified_nested_fields.clear();
            }
            Err(error) => self.record_error(SettingsScope::Global, error),
        }
    }

    fn save_project(&mut self) {
        self.settings = deep_merge_settings(&self.global_settings, &self.project_settings);
        if self.project_settings_load_error.is_some() || self.modified_project_fields.is_empty() {
            return;
        }

        let snapshot = self.project_settings.clone();
        let modified_fields = self.modified_project_fields.clone();
        let modified_nested_fields = self.modified_project_nested_fields.clone();
        match Self::persist_scoped_settings(
            &self.storage,
            SettingsScope::Project,
            &snapshot,
            &modified_fields,
            &modified_nested_fields,
        ) {
            Ok(()) => {
                self.modified_project_fields.clear();
                self.modified_project_nested_fields.clear();
            }
            Err(error) => self.record_error(SettingsScope::Project, error),
        }
    }

    fn persist_scoped_settings(
        storage: &S,
        scope: SettingsScope,
        snapshot_settings: &Settings,
        modified_fields: &BTreeSet<String>,
        modified_nested_fields: &BTreeMap<String, BTreeSet<String>>,
    ) -> Result<(), String> {
        let snapshot_object = settings_to_json_object(snapshot_settings)?;
        storage.update(scope, |current| {
            let mut current_object = match current {
                Some(content) => parse_settings_object(&content)?,
                None => Map::new(),
            };

            for field in modified_fields {
                if let Some(nested) = modified_nested_fields.get(field) {
                    let mut merged_nested = current_object
                        .get(field)
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    let snapshot_nested = snapshot_object.get(field).and_then(Value::as_object);
                    for nested_key in nested {
                        if let Some(value) = snapshot_nested.and_then(|value| value.get(nested_key))
                        {
                            merged_nested.insert(nested_key.clone(), value.clone());
                        } else {
                            merged_nested.remove(nested_key);
                        }
                    }
                    current_object.insert(field.clone(), Value::Object(merged_nested));
                    continue;
                }

                if let Some(value) = snapshot_object.get(field) {
                    current_object.insert(field.clone(), value.clone());
                } else {
                    current_object.remove(field);
                }
            }

            Ok(Some(render_json_object(&current_object)?))
        })
    }

    fn load_from_storage(storage: &S, scope: SettingsScope) -> Result<Settings, String> {
        let Some(content) = storage.read(scope)? else {
            return Ok(Settings::default());
        };
        parse_settings(&content)
    }

    fn try_load_from_storage(storage: &S, scope: SettingsScope) -> ScopedLoadResult {
        match Self::load_from_storage(storage, scope) {
            Ok(settings) => ScopedLoadResult {
                settings,
                error: None,
            },
            Err(error) => ScopedLoadResult {
                settings: Settings::default(),
                error: Some(error),
            },
        }
    }
}

struct ScopedLoadResult {
    settings: Settings,
    error: Option<String>,
}

pub fn load_runtime_settings(cwd: &Path, agent_dir: &Path) -> LoadedRuntimeSettings {
    SettingsManager::create(cwd, agent_dir).loaded_runtime_settings()
}

pub fn load_resource_settings(cwd: &Path, agent_dir: &Path) -> LoadedResourceSettings {
    SettingsManager::create(cwd, agent_dir).loaded_resource_settings()
}

fn ensure_compaction(settings: &mut Settings) -> &mut CompactionConfig {
    settings
        .compaction
        .get_or_insert_with(CompactionConfig::default)
}

fn ensure_branch_summary(settings: &mut Settings) -> &mut BranchSummaryConfig {
    settings
        .branch_summary
        .get_or_insert_with(BranchSummaryConfig::default)
}

fn ensure_retry(settings: &mut Settings) -> &mut RetryConfig {
    settings.retry.get_or_insert_with(RetryConfig::default)
}

fn ensure_terminal(settings: &mut Settings) -> &mut TerminalConfig {
    settings
        .terminal
        .get_or_insert_with(TerminalConfig::default)
}

fn ensure_images(settings: &mut Settings) -> &mut ImageConfig {
    settings.images.get_or_insert_with(ImageConfig::default)
}

fn ensure_markdown(settings: &mut Settings) -> &mut MarkdownConfig {
    settings
        .markdown
        .get_or_insert_with(MarkdownConfig::default)
}

fn settings_is_empty(settings: &Settings) -> bool {
    settings_to_json_object(settings)
        .map(|value| value.is_empty())
        .unwrap_or(true)
}

fn settings_to_json_object(settings: &Settings) -> Result<Map<String, Value>, String> {
    match serde_json::to_value(settings).map_err(|error| error.to_string())? {
        Value::Object(object) => Ok(object),
        _ => Ok(Map::new()),
    }
}

fn render_settings(settings: &Settings) -> Result<String, String> {
    render_json_object(&settings_to_json_object(settings)?)
}

fn render_json_object(object: &Map<String, Value>) -> Result<String, String> {
    serde_json::to_string_pretty(&Value::Object(object.clone())).map_err(|error| error.to_string())
}

fn parse_settings(content: &str) -> Result<Settings, String> {
    serde_json::from_value(Value::Object(parse_settings_object(content)?))
        .map_err(|error| error.to_string())
}

fn parse_settings_object(content: &str) -> Result<Map<String, Value>, String> {
    let value = serde_json::from_str::<Value>(content).map_err(|error| error.to_string())?;
    let Value::Object(mut object) = value else {
        return Err(String::from("settings.json must contain a JSON object"));
    };
    migrate_settings_object(&mut object);
    Ok(object)
}

fn migrate_settings_object(settings: &mut Map<String, Value>) {
    if settings.contains_key("queueMode") && !settings.contains_key("steeringMode") {
        if let Some(value) = settings.remove("queueMode") {
            settings.insert(String::from("steeringMode"), value);
        }
    }

    if !settings.contains_key("transport") {
        if let Some(Value::Bool(websockets)) = settings.get("websockets") {
            settings.insert(
                String::from("transport"),
                Value::String(if *websockets {
                    String::from("websocket")
                } else {
                    String::from("sse")
                }),
            );
            settings.remove("websockets");
        }
    }

    let Some(skill_settings) = settings.get("skills").and_then(Value::as_object).cloned() else {
        return;
    };

    if !settings.contains_key("enableSkillCommands") {
        if let Some(value) = skill_settings.get("enableSkillCommands") {
            settings.insert(String::from("enableSkillCommands"), value.clone());
        }
    }

    match skill_settings.get("customDirectories") {
        Some(Value::Array(values)) if !values.is_empty() => {
            settings.insert(String::from("skills"), Value::Array(values.clone()));
        }
        _ => {
            settings.remove("skills");
        }
    }
}

fn deep_merge_settings(base: &Settings, overrides: &Settings) -> Settings {
    let base = settings_to_json_object(base).unwrap_or_default();
    let overrides = settings_to_json_object(overrides).unwrap_or_default();
    let merged = deep_merge_objects(base, overrides);
    serde_json::from_value(Value::Object(merged)).unwrap_or_default()
}

fn deep_merge_objects(
    mut base: Map<String, Value>,
    overrides: Map<String, Value>,
) -> Map<String, Value> {
    for (key, override_value) in overrides {
        match (base.get_mut(&key), override_value) {
            (Some(Value::Object(base_object)), Value::Object(override_object)) => {
                let merged = deep_merge_objects(base_object.clone(), override_object);
                *base_object = merged;
            }
            (_, override_value) => {
                base.insert(key, override_value);
            }
        }
    }
    base
}

fn resolve_runtime_settings(settings: &Settings) -> RuntimeSettings {
    let mut resolved = RuntimeSettings::default();

    resolved.default_provider = settings.default_provider.clone();
    resolved.default_model = settings.default_model.clone();
    resolved.default_thinking_level = settings.default_thinking_level.clone();
    resolved.transport =
        parse_transport_setting(settings.transport.as_deref()).unwrap_or(Transport::Sse);
    resolved.steering_mode = settings
        .steering_mode
        .clone()
        .unwrap_or_else(|| String::from("one-at-a-time"));
    resolved.follow_up_mode = settings
        .follow_up_mode
        .clone()
        .unwrap_or_else(|| String::from("one-at-a-time"));
    resolved.theme = settings.theme.clone();
    resolved.hide_thinking_block = settings.hide_thinking_block.unwrap_or(false);
    resolved.shell_path = settings.shell_path.clone();
    resolved.quiet_startup = settings.quiet_startup.unwrap_or(false);
    resolved.shell_command_prefix = settings.shell_command_prefix.clone();
    resolved.collapse_changelog = settings.collapse_changelog.unwrap_or(false);
    resolved.enable_skill_commands = settings.enable_skill_commands.unwrap_or(true);
    resolved.enabled_models = settings.enabled_models.clone();
    resolved.double_escape_action = settings
        .double_escape_action
        .clone()
        .unwrap_or_else(|| String::from("tree"));
    resolved.tree_filter_mode = match settings.tree_filter_mode.as_deref() {
        Some("default" | "no-tools" | "user-only" | "labeled-only" | "all") => settings
            .tree_filter_mode
            .clone()
            .unwrap_or_else(|| String::from("default")),
        Some(_) | None => String::from("default"),
    };
    resolved.thinking_budgets = settings.thinking_budgets.clone().unwrap_or_default();
    resolved.editor_padding_x = settings
        .editor_padding_x
        .filter(|value| value.is_finite())
        .map(clamp_editor_padding_x)
        .unwrap_or(0);
    resolved.autocomplete_max_visible = settings
        .autocomplete_max_visible
        .filter(|value| value.is_finite())
        .map(clamp_autocomplete_max_visible)
        .unwrap_or(5);
    resolved.show_hardware_cursor = settings
        .show_hardware_cursor
        .unwrap_or_else(|| std::env::var("PI_HARDWARE_CURSOR").ok().as_deref() == Some("1"));
    resolved.session_dir = settings.session_dir.clone();

    if let Some(compaction) = settings.compaction.as_ref() {
        if let Some(enabled) = compaction.enabled {
            resolved.compaction.enabled = enabled;
        }
        if let Some(reserve_tokens) = compaction.reserve_tokens {
            resolved.compaction.reserve_tokens = reserve_tokens;
        }
        if let Some(keep_recent_tokens) = compaction.keep_recent_tokens {
            resolved.compaction.keep_recent_tokens = keep_recent_tokens;
        }
    }

    if let Some(branch_summary) = settings.branch_summary.as_ref() {
        if let Some(reserve_tokens) = branch_summary.reserve_tokens {
            resolved.branch_summary.reserve_tokens = reserve_tokens;
        }
        if let Some(skip_prompt) = branch_summary.skip_prompt {
            resolved.branch_summary.skip_prompt = skip_prompt;
        }
    }

    if let Some(retry) = settings.retry.as_ref() {
        if let Some(enabled) = retry.enabled {
            resolved.retry.enabled = enabled;
        }
        if let Some(max_retries) = retry.max_retries {
            resolved.retry.max_retries = max_retries;
        }
        if let Some(base_delay_ms) = retry.base_delay_ms {
            resolved.retry.base_delay_ms = base_delay_ms;
        }
        if let Some(max_delay_ms) = retry.max_delay_ms {
            resolved.retry.max_delay_ms = max_delay_ms;
        }
    }

    if let Some(terminal) = settings.terminal.as_ref() {
        if let Some(show_images) = terminal.show_images {
            resolved.terminal.show_images = show_images;
        }
        if let Some(clear_on_shrink) = terminal.clear_on_shrink {
            resolved.terminal.clear_on_shrink = clear_on_shrink;
        }
    } else if std::env::var("PI_CLEAR_ON_SHRINK").ok().as_deref() == Some("1") {
        resolved.terminal.clear_on_shrink = true;
    }

    if let Some(images) = settings.images.as_ref() {
        if let Some(auto_resize) = images.auto_resize {
            resolved.images.auto_resize_images = auto_resize;
        }
        if let Some(block_images) = images.block_images {
            resolved.images.block_images = block_images;
        }
    }

    if let Some(markdown) = settings.markdown.as_ref() {
        if let Some(code_block_indent) = markdown.code_block_indent.clone() {
            resolved.markdown.code_block_indent = code_block_indent;
        }
    }

    resolved
}

fn apply_runtime_scope(
    loaded: &mut LoadedRuntimeSettings,
    settings: &Settings,
    scope: SettingsScope,
) {
    if let Some(default_provider) = settings.default_provider.clone() {
        loaded.settings.default_provider = Some(default_provider);
    }
    if let Some(default_model) = settings.default_model.clone() {
        loaded.settings.default_model = Some(default_model);
    }
    if let Some(default_thinking_level) = settings.default_thinking_level.clone() {
        loaded.settings.default_thinking_level = Some(default_thinking_level);
    }
    if let Some(transport) = settings.transport.as_deref() {
        match parse_transport_setting(Some(transport)) {
            Some(transport) => loaded.settings.transport = transport,
            None => loaded.warnings.push(SettingsWarning {
                scope,
                message: format!(
                    "Invalid transport setting \"{transport}\". Valid values: sse, websocket, auto"
                ),
            }),
        }
    }
    if let Some(steering_mode) = settings.steering_mode.clone() {
        loaded.settings.steering_mode = steering_mode;
    }
    if let Some(follow_up_mode) = settings.follow_up_mode.clone() {
        loaded.settings.follow_up_mode = follow_up_mode;
    }
    if let Some(theme) = settings.theme.clone() {
        loaded.settings.theme = Some(theme);
    }
    if let Some(hide_thinking_block) = settings.hide_thinking_block {
        loaded.settings.hide_thinking_block = hide_thinking_block;
    }
    if let Some(shell_path) = settings.shell_path.clone() {
        loaded.settings.shell_path = Some(shell_path);
    }
    if let Some(quiet_startup) = settings.quiet_startup {
        loaded.settings.quiet_startup = quiet_startup;
    }
    if let Some(shell_command_prefix) = settings.shell_command_prefix.clone() {
        loaded.settings.shell_command_prefix = Some(shell_command_prefix);
    }
    if let Some(collapse_changelog) = settings.collapse_changelog {
        loaded.settings.collapse_changelog = collapse_changelog;
    }
    if let Some(enable_skill_commands) = settings.enable_skill_commands {
        loaded.settings.enable_skill_commands = enable_skill_commands;
    }
    if let Some(enabled_models) = settings.enabled_models.clone() {
        loaded.settings.enabled_models = Some(enabled_models);
    }
    if let Some(double_escape_action) = settings.double_escape_action.clone() {
        loaded.settings.double_escape_action = double_escape_action;
    }
    if let Some(tree_filter_mode) = settings.tree_filter_mode.clone() {
        loaded.settings.tree_filter_mode = match tree_filter_mode.as_str() {
            "default" | "no-tools" | "user-only" | "labeled-only" | "all" => tree_filter_mode,
            _ => String::from("default"),
        };
    }
    if let Some(show_hardware_cursor) = settings.show_hardware_cursor {
        loaded.settings.show_hardware_cursor = show_hardware_cursor;
    }
    if let Some(session_dir) = settings.session_dir.clone() {
        loaded.settings.session_dir = Some(session_dir);
    }

    if let Some(compaction) = settings.compaction.as_ref() {
        if let Some(enabled) = compaction.enabled {
            loaded.settings.compaction.enabled = enabled;
        }
        if let Some(reserve_tokens) = compaction.reserve_tokens {
            loaded.settings.compaction.reserve_tokens = reserve_tokens;
        }
        if let Some(keep_recent_tokens) = compaction.keep_recent_tokens {
            loaded.settings.compaction.keep_recent_tokens = keep_recent_tokens;
        }
    }

    if let Some(branch_summary) = settings.branch_summary.as_ref() {
        if let Some(reserve_tokens) = branch_summary.reserve_tokens {
            loaded.settings.branch_summary.reserve_tokens = reserve_tokens;
        }
        if let Some(skip_prompt) = branch_summary.skip_prompt {
            loaded.settings.branch_summary.skip_prompt = skip_prompt;
        }
    }

    if let Some(retry) = settings.retry.as_ref() {
        if let Some(enabled) = retry.enabled {
            loaded.settings.retry.enabled = enabled;
        }
        if let Some(max_retries) = retry.max_retries {
            loaded.settings.retry.max_retries = max_retries;
        }
        if let Some(base_delay_ms) = retry.base_delay_ms {
            loaded.settings.retry.base_delay_ms = base_delay_ms;
        }
        if let Some(max_delay_ms) = retry.max_delay_ms {
            loaded.settings.retry.max_delay_ms = max_delay_ms;
        }
    }

    if let Some(terminal) = settings.terminal.as_ref() {
        if let Some(show_images) = terminal.show_images {
            loaded.settings.terminal.show_images = show_images;
        }
        if let Some(clear_on_shrink) = terminal.clear_on_shrink {
            loaded.settings.terminal.clear_on_shrink = clear_on_shrink;
        }
    }

    if let Some(images) = settings.images.as_ref() {
        if let Some(auto_resize) = images.auto_resize {
            loaded.settings.images.auto_resize_images = auto_resize;
        }
        if let Some(block_images) = images.block_images {
            loaded.settings.images.block_images = block_images;
        }
    }

    if let Some(thinking_budgets) = settings.thinking_budgets.as_ref() {
        if let Some(minimal) = thinking_budgets.minimal {
            loaded.settings.thinking_budgets.minimal = Some(minimal);
        }
        if let Some(low) = thinking_budgets.low {
            loaded.settings.thinking_budgets.low = Some(low);
        }
        if let Some(medium) = thinking_budgets.medium {
            loaded.settings.thinking_budgets.medium = Some(medium);
        }
        if let Some(high) = thinking_budgets.high {
            loaded.settings.thinking_budgets.high = Some(high);
        }
    }

    if let Some(editor_padding_x) = settings.editor_padding_x.filter(|value| value.is_finite()) {
        loaded.settings.editor_padding_x = clamp_editor_padding_x(editor_padding_x);
    }

    if let Some(autocomplete_max_visible) = settings
        .autocomplete_max_visible
        .filter(|value| value.is_finite())
    {
        loaded.settings.autocomplete_max_visible =
            clamp_autocomplete_max_visible(autocomplete_max_visible);
    }

    if let Some(markdown) = settings.markdown.as_ref() {
        if let Some(code_block_indent) = markdown.code_block_indent.clone() {
            loaded.settings.markdown.code_block_indent = code_block_indent;
        }
    }
}

fn apply_resource_scope(target: &mut ResourceSettings, settings: &Settings) {
    if let Some(npm_command) = settings.npm_command.clone() {
        target.npm_command = Some(npm_command);
    }
    if let Some(packages) = settings.packages.clone() {
        target.packages = packages;
    }
    if let Some(extensions) = settings.extensions.clone() {
        target.extensions = extensions;
    }
    if let Some(skills) = settings.skills.clone() {
        target.skills = skills;
    }
    if let Some(prompts) = settings.prompts.clone() {
        target.prompts = prompts;
    }
    if let Some(themes) = settings.themes.clone() {
        target.themes = themes;
    }
}

fn parse_transport_setting(value: Option<&str>) -> Option<Transport> {
    let value = value?;
    match value.trim().to_ascii_lowercase().as_str() {
        "sse" => Some(Transport::Sse),
        "websocket" => Some(Transport::WebSocket),
        "auto" => Some(Transport::Auto),
        _ => None,
    }
}

fn render_transport(transport: Transport) -> String {
    match transport {
        Transport::Sse => String::from("sse"),
        Transport::WebSocket => String::from("websocket"),
        Transport::Auto => String::from("auto"),
    }
}

fn clamp_editor_padding_x(value: f64) -> usize {
    value.floor().clamp(0.0, 3.0) as usize
}

fn clamp_autocomplete_max_visible(value: f64) -> usize {
    value.floor().clamp(3.0, 20.0) as usize
}
