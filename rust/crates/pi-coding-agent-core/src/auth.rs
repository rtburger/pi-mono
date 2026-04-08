use crate::config_value::resolve_config_value_uncached;
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub trait AuthSource: Send + Sync {
    fn has_auth(&self, provider: &str) -> bool;
    fn get_api_key(&self, provider: &str) -> Option<String>;
}

#[derive(Default, Clone)]
pub struct MemoryAuthStorage {
    api_keys: Arc<Mutex<HashMap<String, String>>>,
}

impl MemoryAuthStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_api_keys(
        api_keys: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let storage = Self::default();
        {
            let mut guard = storage.api_keys.lock().unwrap();
            for (provider, api_key) in api_keys {
                guard.insert(provider.into(), api_key.into());
            }
        }
        storage
    }

    pub fn set_api_key(&self, provider: impl Into<String>, api_key: impl Into<String>) {
        self.api_keys
            .lock()
            .unwrap()
            .insert(provider.into(), api_key.into());
    }

    pub fn remove_api_key(&self, provider: &str) {
        self.api_keys.lock().unwrap().remove(provider);
    }
}

impl AuthSource for MemoryAuthStorage {
    fn has_auth(&self, provider: &str) -> bool {
        self.api_keys.lock().unwrap().contains_key(provider)
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        self.api_keys.lock().unwrap().get(provider).cloned()
    }
}

#[derive(Debug, Clone)]
pub struct AuthFileSource {
    path: PathBuf,
}

impl AuthFileSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Option<AuthFileData> {
        let content = fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&content).ok()
    }
}

impl AuthSource for AuthFileSource {
    fn has_auth(&self, provider: &str) -> bool {
        self.load().is_some_and(|data| data.contains_key(provider))
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        let mut data = self.load()?;
        let credential = data.remove(provider)?;
        match credential {
            AuthFileCredential::ApiKey { key } => resolve_config_value_uncached(&key),
            AuthFileCredential::OAuth {
                access,
                expires,
                project_id,
            } => {
                let access = access.filter(|value| !value.is_empty())?;
                if expires.is_some_and(|expires| expires <= now_ms()) {
                    return None;
                }
                oauth_api_key(provider, &access, project_id.as_deref())
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct ChainedAuthSource {
    sources: Vec<Arc<dyn AuthSource>>,
}

impl ChainedAuthSource {
    pub fn new(sources: Vec<Arc<dyn AuthSource>>) -> Self {
        Self { sources }
    }
}

impl AuthSource for ChainedAuthSource {
    fn has_auth(&self, provider: &str) -> bool {
        self.sources.iter().any(|source| source.has_auth(provider))
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        self.sources
            .iter()
            .find_map(|source| source.get_api_key(provider))
    }
}

type AuthFileData = HashMap<String, AuthFileCredential>;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum AuthFileCredential {
    #[serde(rename = "api_key")]
    ApiKey { key: String },
    #[serde(rename = "oauth")]
    OAuth {
        access: Option<String>,
        expires: Option<i64>,
        #[serde(rename = "projectId")]
        project_id: Option<String>,
    },
}

fn oauth_api_key(provider: &str, access: &str, project_id: Option<&str>) -> Option<String> {
    match provider {
        "anthropic" | "github-copilot" | "openai-codex" => Some(access.to_string()),
        "google-gemini-cli" | "google-antigravity" => Some(
            serde_json::json!({
                "token": access,
                "projectId": project_id?,
            })
            .to_string(),
        ),
        _ => None,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
