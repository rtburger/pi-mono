use crate::config_value::resolve_config_value_uncached;
use pi_ai::oauth::{
    OAuthCredentials, OAuthRefreshOverrides, get_oauth_provider, refresh_oauth_token_with_overrides,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    fs,
    future::Future,
    io::ErrorKind,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;

pub type AuthApiKeyFuture<'a> = Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>>;

pub trait AuthSource: Send + Sync {
    fn has_auth(&self, provider: &str) -> bool;
    fn get_api_key(&self, provider: &str) -> Option<String>;

    fn get_api_key_for_request<'a>(&'a self, provider: &'a str) -> AuthApiKeyFuture<'a> {
        Box::pin(async move { self.get_api_key(provider) })
    }

    fn model_base_url(&self, _provider: &str) -> Option<String> {
        None
    }
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
        let root = serde_json::from_str::<Value>(&content).ok()?;
        root.as_object().cloned()
    }

    async fn get_api_key_for_request_impl(&self, provider: &str) -> Option<String> {
        let mut data = self.load()?;
        let entry = data.remove(provider)?;
        match serde_json::from_value::<AuthFileCredential>(entry.clone()).ok()? {
            AuthFileCredential::ApiKey { key } => resolve_config_value_uncached(&key),
            AuthFileCredential::OAuth { access, expires } => {
                access.filter(|value| !value.is_empty())?;
                let credential = AuthFileOAuthCredentialView::from_value(&entry)?;
                if expires.is_none_or(|expires| expires > now_ms()) {
                    return oauth_api_key(provider, &credential.to_oauth_credentials());
                }
                refresh_auth_file_provider_api_key(&self.path, provider).await
            }
        }
    }
}

impl AuthSource for AuthFileSource {
    fn has_auth(&self, provider: &str) -> bool {
        self.load().is_some_and(|data| data.contains_key(provider))
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        let mut data = self.load()?;
        let entry = data.remove(provider)?;
        match serde_json::from_value::<AuthFileCredential>(entry.clone()).ok()? {
            AuthFileCredential::ApiKey { key } => resolve_config_value_uncached(&key),
            AuthFileCredential::OAuth { access, expires } => {
                access.filter(|value| !value.is_empty())?;
                if expires.is_some_and(|expires| expires <= now_ms()) {
                    return None;
                }
                let credential = AuthFileOAuthCredentialView::from_value(&entry)?;
                oauth_api_key(provider, &credential.to_oauth_credentials())
            }
        }
    }

    fn get_api_key_for_request<'a>(&'a self, provider: &'a str) -> AuthApiKeyFuture<'a> {
        Box::pin(async move { self.get_api_key_for_request_impl(provider).await })
    }

    fn model_base_url(&self, _provider: &str) -> Option<String> {
        None
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

    fn get_api_key_for_request<'a>(&'a self, provider: &'a str) -> AuthApiKeyFuture<'a> {
        Box::pin(async move {
            for source in &self.sources {
                if let Some(api_key) = source.get_api_key_for_request(provider).await {
                    return Some(api_key);
                }
            }
            None
        })
    }

    fn model_base_url(&self, provider: &str) -> Option<String> {
        self.sources
            .iter()
            .find_map(|source| source.model_base_url(provider))
    }
}

pub async fn refresh_auth_file_oauth(path: impl AsRef<Path>) {
    let _ = refresh_auth_file_oauth_inner(path.as_ref(), &OAuthRefreshOverrides::default()).await;
}

async fn refresh_auth_file_provider_api_key(path: &Path, provider: &str) -> Option<String> {
    let _lock = acquire_auth_file_lock(path).await.ok()?;
    let content = fs::read_to_string(path).ok()?;
    let mut root = serde_json::from_str::<Value>(&content).ok()?;
    let entries = root.as_object_mut()?;
    let entry = entries.get_mut(provider)?;

    if let Some(current_api_key) = auth_file_api_key(provider, entry) {
        return Some(current_api_key);
    }

    let credential = AuthFileOAuthCredentialView::from_value(entry)?;
    credential.refresh_token()?;
    let refreshed = refresh_auth_provider(
        provider,
        &credential.to_oauth_credentials(),
        &OAuthRefreshOverrides::default(),
    )
    .await
    .ok()?;
    let api_key = oauth_api_key(provider, &refreshed)?;
    apply_refreshed_oauth_credentials(entry, refreshed);

    let serialized = serde_json::to_string_pretty(&root).ok()?;
    fs::write(path, serialized).ok()?;
    Some(api_key)
}

type AuthFileData = Map<String, Value>;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum AuthFileCredential {
    #[serde(rename = "api_key")]
    ApiKey { key: String },
    #[serde(rename = "oauth")]
    OAuth {
        access: Option<String>,
        expires: Option<i64>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct AuthFileOAuthCredentialView {
    #[serde(rename = "type")]
    credential_type: String,
    refresh: Option<String>,
    access: Option<String>,
    expires: Option<i64>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl AuthFileOAuthCredentialView {
    fn from_value(value: &Value) -> Option<Self> {
        let credential = serde_json::from_value::<Self>(value.clone()).ok()?;
        (credential.credential_type == "oauth").then_some(credential)
    }

    fn access_token(&self) -> Option<&str> {
        self.access.as_deref().filter(|value| !value.is_empty())
    }

    fn refresh_token(&self) -> Option<&str> {
        self.refresh.as_deref().filter(|value| !value.is_empty())
    }

    fn to_oauth_credentials(&self) -> OAuthCredentials {
        OAuthCredentials {
            refresh: self.refresh.clone().unwrap_or_default(),
            access: self.access.clone().unwrap_or_default(),
            expires: self.expires.unwrap_or_default(),
            extra: self.extra.clone(),
        }
    }
}

const AUTH_FILE_LOCK_RETRIES: usize = 10;
const AUTH_FILE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(20);

#[cfg(test)]
const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

struct AuthFileLock {
    path: PathBuf,
}

impl Drop for AuthFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

async fn acquire_auth_file_lock(path: &Path) -> Result<AuthFileLock, String> {
    let lock_path = PathBuf::from(format!("{}.lock", path.to_string_lossy()));

    for attempt in 0..AUTH_FILE_LOCK_RETRIES {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => return Ok(AuthFileLock { path: lock_path }),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                if attempt + 1 == AUTH_FILE_LOCK_RETRIES {
                    return Err(format!(
                        "Failed to acquire auth.json lock after {AUTH_FILE_LOCK_RETRIES} attempts"
                    ));
                }
                sleep(AUTH_FILE_LOCK_RETRY_DELAY).await;
            }
            Err(error) => return Err(format!("Failed to acquire auth.json lock: {error}")),
        }
    }

    Err("Failed to acquire auth.json lock".into())
}

fn auth_file_api_key(provider: &str, entry: &Value) -> Option<String> {
    match serde_json::from_value::<AuthFileCredential>(entry.clone()).ok()? {
        AuthFileCredential::ApiKey { key } => resolve_config_value_uncached(&key),
        AuthFileCredential::OAuth { expires, .. } => {
            let credential = AuthFileOAuthCredentialView::from_value(entry)?;
            credential.access_token()?;
            if expires.is_some_and(|expires| expires <= now_ms()) {
                return None;
            }
            oauth_api_key(provider, &credential.to_oauth_credentials())
        }
    }
}

async fn refresh_auth_provider(
    provider: &str,
    credentials: &OAuthCredentials,
    overrides: &OAuthRefreshOverrides<'_>,
) -> Result<OAuthCredentials, String> {
    refresh_oauth_token_with_overrides(provider, credentials, overrides).await
}

async fn refresh_auth_file_oauth_inner(
    path: &Path,
    overrides: &OAuthRefreshOverrides<'_>,
) -> Vec<String> {
    if !path.exists() {
        return Vec::new();
    }

    let _lock = match acquire_auth_file_lock(path).await {
        Ok(lock) => lock,
        Err(error) => return vec![error],
    };

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    let mut root = match serde_json::from_str::<Value>(&content) {
        Ok(root) => root,
        Err(_) => return Vec::new(),
    };

    let Some(entries) = root.as_object_mut() else {
        return Vec::new();
    };

    let mut changed = false;
    let mut errors = Vec::new();

    for (provider, entry) in entries.iter_mut() {
        let Some(credential) = AuthFileOAuthCredentialView::from_value(entry) else {
            continue;
        };
        let Some(expires) = credential.expires else {
            continue;
        };
        if expires > now_ms() {
            continue;
        }

        if credential.refresh_token().is_none() {
            continue;
        }

        let refreshed =
            match refresh_auth_provider(provider, &credential.to_oauth_credentials(), overrides)
                .await
            {
                Ok(refreshed) => Ok(refreshed),
                Err(error) if error.starts_with("Unsupported OAuth provider:") => continue,
                Err(error) => Err(error),
            };

        match refreshed {
            Ok(refreshed) => {
                apply_refreshed_oauth_credentials(entry, refreshed);
                changed = true;
            }
            Err(error) => errors.push(format!(
                "Failed to refresh OAuth token for {provider}: {error}"
            )),
        }
    }

    if changed {
        match serde_json::to_string_pretty(&root) {
            Ok(serialized) => {
                if let Err(error) = fs::write(path, serialized) {
                    errors.push(format!("Failed to write auth.json: {error}"));
                }
            }
            Err(error) => errors.push(format!("Failed to serialize auth.json: {error}")),
        }
    }

    errors
}

fn apply_refreshed_oauth_credentials(entry: &mut Value, refreshed: OAuthCredentials) {
    *entry = oauth_credentials_to_value(refreshed);
}

fn oauth_credentials_to_value(credentials: OAuthCredentials) -> Value {
    let mut object = Map::new();
    object.insert("type".into(), Value::String("oauth".into()));
    object.insert("refresh".into(), Value::String(credentials.refresh));
    object.insert("access".into(), Value::String(credentials.access));
    object.insert(
        "expires".into(),
        Value::Number(serde_json::Number::from(credentials.expires)),
    );
    for (key, value) in credentials.extra {
        object.insert(key, value);
    }
    Value::Object(object)
}

fn oauth_api_key(provider: &str, credentials: &OAuthCredentials) -> Option<String> {
    get_oauth_provider(provider).and_then(|provider| provider.get_api_key(credentials).ok())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pi-auth-unit-{prefix}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn request_body(request: &str) -> &str {
        request.split("\r\n\r\n").nth(1).unwrap_or_default()
    }

    fn spawn_single_response_server<F>(
        assert_request: F,
        response_body: String,
        content_type: &str,
    ) -> (String, thread::JoinHandle<()>)
    where
        F: Fn(&str) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let content_type = content_type.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8192];
            let bytes_read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).into_owned();
            assert_request(&request);

            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body,
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        (format!("http://{address}/token"), handle)
    }

    #[tokio::test]
    async fn refresh_auth_file_oauth_rereads_auth_json_after_waiting_for_lock() {
        let temp_dir = unique_temp_dir("startup-refresh-lock");
        let auth_path = temp_dir.join("auth.json");
        fs::write(
            &auth_path,
            serde_json::json!({
                "anthropic": {
                    "type": "oauth",
                    "refresh": "refresh-token",
                    "access": "expired-access-token",
                    "expires": 0
                }
            })
            .to_string(),
        )
        .unwrap();

        let lock_path = PathBuf::from(format!("{}.lock", auth_path.to_string_lossy()));
        fs::write(&lock_path, "locked").unwrap();

        let auth_path_for_thread = auth_path.clone();
        let lock_path_for_thread = lock_path.clone();
        let updater = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            fs::write(
                &auth_path_for_thread,
                serde_json::json!({
                    "anthropic": {
                        "type": "oauth",
                        "refresh": "refresh-token",
                        "access": "already-refreshed-token",
                        "expires": i64::MAX
                    }
                })
                .to_string(),
            )
            .unwrap();
            fs::remove_file(&lock_path_for_thread).unwrap();
        });

        let errors = refresh_auth_file_oauth_inner(
            &auth_path,
            &OAuthRefreshOverrides {
                anthropic_token_url: Some("http://127.0.0.1:9/token"),
                ..OAuthRefreshOverrides::default()
            },
        )
        .await;

        updater.join().unwrap();

        assert!(errors.is_empty(), "unexpected refresh errors: {errors:?}");

        let refreshed: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert_eq!(
            refreshed
                .pointer("/anthropic/access")
                .and_then(Value::as_str),
            Some("already-refreshed-token")
        );
    }

    #[tokio::test]
    async fn refresh_auth_file_oauth_updates_expired_anthropic_credentials() {
        let (token_url, server) = spawn_single_response_server(
            |request| {
                assert!(
                    request
                        .to_ascii_lowercase()
                        .starts_with("post /token http/1.1")
                );
                let body: Value = serde_json::from_str(request_body(request)).unwrap();
                assert_eq!(
                    body.get("grant_type").and_then(Value::as_str),
                    Some("refresh_token")
                );
                assert_eq!(
                    body.get("refresh_token").and_then(Value::as_str),
                    Some("refresh-token")
                );
                assert_eq!(
                    body.get("client_id").and_then(Value::as_str),
                    Some(ANTHROPIC_CLIENT_ID)
                );
                assert!(body.get("scope").is_none());
            },
            serde_json::json!({
                "access_token": "new-access-token",
                "refresh_token": "new-refresh-token",
                "expires_in": 3600,
            })
            .to_string(),
            "application/json",
        );

        let temp_dir = unique_temp_dir("anthropic-refresh");
        let auth_path = temp_dir.join("auth.json");
        fs::write(
            &auth_path,
            serde_json::json!({
                "anthropic": {
                    "type": "oauth",
                    "refresh": "refresh-token",
                    "access": "expired-access-token",
                    "expires": 0
                }
            })
            .to_string(),
        )
        .unwrap();

        let errors = refresh_auth_file_oauth_inner(
            &auth_path,
            &OAuthRefreshOverrides {
                anthropic_token_url: Some(&token_url),
                ..OAuthRefreshOverrides::default()
            },
        )
        .await;

        assert!(errors.is_empty(), "unexpected refresh errors: {errors:?}");

        let refreshed: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert_eq!(
            refreshed
                .pointer("/anthropic/access")
                .and_then(Value::as_str),
            Some("new-access-token")
        );
        assert_eq!(
            refreshed
                .pointer("/anthropic/refresh")
                .and_then(Value::as_str),
            Some("new-refresh-token")
        );
        assert!(
            refreshed
                .pointer("/anthropic/expires")
                .and_then(Value::as_i64)
                .unwrap()
                > now_ms()
        );

        server.join().unwrap();
    }

    #[tokio::test]
    async fn refresh_auth_file_oauth_updates_expired_openai_codex_credentials() {
        let (token_url, server) = spawn_single_response_server(
            |request| {
                let request_lower = request.to_ascii_lowercase();
                assert!(request_lower.starts_with("post /token http/1.1"));
                let body = request_body(request);
                assert!(body.contains("grant_type=refresh_token"));
                assert!(body.contains("refresh_token=refresh-token"));
                assert!(body.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
            },
            serde_json::json!({
                "access_token": format!(
                    "aaa.{}.bbb",
                    "eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjX3Rlc3QifX0="
                ),
                "refresh_token": "new-refresh-token",
                "expires_in": 3600
            })
            .to_string(),
            "application/json",
        );

        let temp_dir = unique_temp_dir("openai-codex-refresh");
        let auth_path = temp_dir.join("auth.json");
        fs::write(
            &auth_path,
            serde_json::json!({
                "openai-codex": {
                    "type": "oauth",
                    "refresh": "refresh-token",
                    "access": "expired-token",
                    "expires": 0,
                    "accountId": "old-account"
                }
            })
            .to_string(),
        )
        .unwrap();

        let errors = refresh_auth_file_oauth_inner(
            &auth_path,
            &OAuthRefreshOverrides {
                openai_codex_token_url: Some(&token_url),
                ..OAuthRefreshOverrides::default()
            },
        )
        .await;

        assert!(errors.is_empty(), "unexpected refresh errors: {errors:?}");

        let refreshed: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        let credential = refreshed.get("openai-codex").unwrap();
        assert_eq!(
            credential.get("refresh").and_then(Value::as_str),
            Some("new-refresh-token")
        );
        assert_eq!(
            credential.get("accountId").and_then(Value::as_str),
            Some("acc_test")
        );

        server.join().unwrap();
    }
}
