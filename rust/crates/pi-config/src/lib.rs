use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

const CONFIG_DIR_NAME: &str = ".pi";
const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsScope {
    Global,
    Project,
}

impl SettingsScope {
    pub fn label(&self) -> &'static str {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThinkingBudgetsSettings {
    pub minimal: Option<u64>,
    pub low: Option<u64>,
    pub medium: Option<u64>,
    pub high: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettings {
    pub images: ImageSettings,
    pub compaction: CompactionSettings,
    pub thinking_budgets: ThinkingBudgetsSettings,
    pub theme: Option<String>,
    pub editor_padding_x: usize,
    pub autocomplete_max_visible: usize,
    pub enabled_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredPackageSource {
    pub source: String,
    pub extensions: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub prompts: Option<Vec<String>>,
    pub themes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            images: ImageSettings::default(),
            compaction: CompactionSettings::default(),
            thinking_budgets: ThinkingBudgetsSettings::default(),
            theme: None,
            editor_padding_x: 0,
            autocomplete_max_visible: 5,
            enabled_models: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedRuntimeSettings {
    pub settings: RuntimeSettings,
    pub warnings: Vec<SettingsWarning>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawSettings {
    #[serde(default)]
    images: RawImageSettings,
    #[serde(default)]
    compaction: RawCompactionSettings,
    #[serde(default)]
    thinking_budgets: RawThinkingBudgetsSettings,
    theme: Option<String>,
    editor_padding_x: Option<f64>,
    autocomplete_max_visible: Option<f64>,
    enabled_models: Option<Vec<String>>,
    npm_command: Option<Vec<String>>,
    packages: Option<Vec<RawPackageSource>>,
    extensions: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    prompts: Option<Vec<String>>,
    themes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPackageSource {
    Plain(String),
    Filtered {
        source: String,
        extensions: Option<Vec<String>>,
        skills: Option<Vec<String>>,
        prompts: Option<Vec<String>>,
        themes: Option<Vec<String>>,
    },
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawImageSettings {
    auto_resize: Option<bool>,
    block_images: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawCompactionSettings {
    enabled: Option<bool>,
    reserve_tokens: Option<u64>,
    keep_recent_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawThinkingBudgetsSettings {
    minimal: Option<u64>,
    low: Option<u64>,
    medium: Option<u64>,
    high: Option<u64>,
}

pub fn load_runtime_settings(cwd: &Path, agent_dir: &Path) -> LoadedRuntimeSettings {
    let mut loaded = LoadedRuntimeSettings::default();

    apply_settings_file(
        &mut loaded,
        SettingsScope::Global,
        agent_dir.join(SETTINGS_FILE_NAME),
    );
    apply_settings_file(
        &mut loaded,
        SettingsScope::Project,
        cwd.join(CONFIG_DIR_NAME).join(SETTINGS_FILE_NAME),
    );

    loaded
}

pub fn load_resource_settings(cwd: &Path, agent_dir: &Path) -> LoadedResourceSettings {
    let mut loaded = LoadedResourceSettings::default();

    apply_resource_settings_file(
        &mut loaded.global,
        &mut loaded.warnings,
        SettingsScope::Global,
        agent_dir.join(SETTINGS_FILE_NAME),
    );
    apply_resource_settings_file(
        &mut loaded.project,
        &mut loaded.warnings,
        SettingsScope::Project,
        cwd.join(CONFIG_DIR_NAME).join(SETTINGS_FILE_NAME),
    );

    loaded
}

fn apply_settings_file(loaded: &mut LoadedRuntimeSettings, scope: SettingsScope, path: PathBuf) {
    let Some(parsed) = read_settings_file(&scope, &path, &mut loaded.warnings) else {
        return;
    };

    if let Some(auto_resize_images) = parsed.images.auto_resize {
        loaded.settings.images.auto_resize_images = auto_resize_images;
    }
    if let Some(block_images) = parsed.images.block_images {
        loaded.settings.images.block_images = block_images;
    }
    if let Some(enabled) = parsed.compaction.enabled {
        loaded.settings.compaction.enabled = enabled;
    }
    if let Some(reserve_tokens) = parsed.compaction.reserve_tokens {
        loaded.settings.compaction.reserve_tokens = reserve_tokens;
    }
    if let Some(keep_recent_tokens) = parsed.compaction.keep_recent_tokens {
        loaded.settings.compaction.keep_recent_tokens = keep_recent_tokens;
    }

    if let Some(minimal) = parsed.thinking_budgets.minimal {
        loaded.settings.thinking_budgets.minimal = Some(minimal);
    }
    if let Some(low) = parsed.thinking_budgets.low {
        loaded.settings.thinking_budgets.low = Some(low);
    }
    if let Some(medium) = parsed.thinking_budgets.medium {
        loaded.settings.thinking_budgets.medium = Some(medium);
    }
    if let Some(high) = parsed.thinking_budgets.high {
        loaded.settings.thinking_budgets.high = Some(high);
    }

    if let Some(theme) = parsed
        .theme
        .map(|theme| theme.trim().to_owned())
        .filter(|theme| !theme.is_empty())
    {
        loaded.settings.theme = Some(theme);
    }

    if let Some(editor_padding_x) = parsed.editor_padding_x
        && editor_padding_x.is_finite()
    {
        loaded.settings.editor_padding_x = editor_padding_x.floor().clamp(0.0, 3.0) as usize;
    }

    if let Some(autocomplete_max_visible) = parsed.autocomplete_max_visible
        && autocomplete_max_visible.is_finite()
    {
        loaded.settings.autocomplete_max_visible =
            autocomplete_max_visible.floor().clamp(3.0, 20.0) as usize;
    }

    if let Some(enabled_models) = parsed.enabled_models {
        loaded.settings.enabled_models = Some(enabled_models);
    }
}

fn apply_resource_settings_file(
    target: &mut ResourceSettings,
    warnings: &mut Vec<SettingsWarning>,
    scope: SettingsScope,
    path: PathBuf,
) {
    let Some(parsed) = read_settings_file(&scope, &path, warnings) else {
        return;
    };

    target.npm_command = parsed
        .npm_command
        .map(|argv| {
            argv.into_iter()
                .map(|entry| entry.trim().to_owned())
                .filter(|entry| !entry.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|argv| !argv.is_empty());
    target.packages = parsed
        .packages
        .unwrap_or_default()
        .into_iter()
        .map(|package| match package {
            RawPackageSource::Plain(source) => PackageSource::Plain(source),
            RawPackageSource::Filtered {
                source,
                extensions,
                skills,
                prompts,
                themes,
            } => PackageSource::Filtered(FilteredPackageSource {
                source,
                extensions,
                skills,
                prompts,
                themes,
            }),
        })
        .collect();
    target.extensions = parsed.extensions.unwrap_or_default();
    target.skills = parsed.skills.unwrap_or_default();
    target.prompts = parsed.prompts.unwrap_or_default();
    target.themes = parsed.themes.unwrap_or_default();
}

fn read_settings_file(
    scope: &SettingsScope,
    path: &Path,
    warnings: &mut Vec<SettingsWarning>,
) -> Option<RawSettings> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            warnings.push(SettingsWarning {
                scope: scope.clone(),
                message: error.to_string(),
            });
            return None;
        }
    };

    match serde_json::from_str::<RawSettings>(&content) {
        Ok(settings) => Some(settings),
        Err(error) => {
            warnings.push(SettingsWarning {
                scope: scope.clone(),
                message: error.to_string(),
            });
            None
        }
    }
}
