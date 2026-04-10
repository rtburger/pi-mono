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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThinkingBudgetsSettings {
    pub minimal: Option<u64>,
    pub low: Option<u64>,
    pub medium: Option<u64>,
    pub high: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeSettings {
    pub images: ImageSettings,
    pub thinking_budgets: ThinkingBudgetsSettings,
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
    thinking_budgets: RawThinkingBudgetsSettings,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawImageSettings {
    auto_resize: Option<bool>,
    block_images: Option<bool>,
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
