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
pub struct LoadedImageSettings {
    pub settings: ImageSettings,
    pub warnings: Vec<SettingsWarning>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawSettings {
    #[serde(default)]
    images: RawImageSettings,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawImageSettings {
    auto_resize: Option<bool>,
    block_images: Option<bool>,
}

pub fn load_image_settings(cwd: &Path, agent_dir: &Path) -> LoadedImageSettings {
    let mut loaded = LoadedImageSettings::default();

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

fn apply_settings_file(loaded: &mut LoadedImageSettings, scope: SettingsScope, path: PathBuf) {
    let Some(parsed) = read_settings_file(&scope, &path, &mut loaded.warnings) else {
        return;
    };

    if let Some(auto_resize_images) = parsed.images.auto_resize {
        loaded.settings.auto_resize_images = auto_resize_images;
    }
    if let Some(block_images) = parsed.images.block_images {
        loaded.settings.block_images = block_images;
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
