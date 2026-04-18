use pi_ai::{
    OAuthCredentials, OAuthLoginCallbacks, get_env_api_key, get_oauth_provider,
    get_oauth_provider_info_list,
};
use pi_coding_agent_core::AuthSource;
use pi_coding_agent_tui::ExternalEditorHost;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    fs,
    io::{self, ErrorKind, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::spawn_blocking;

const AUTH_FILE_LOCK_RETRIES: usize = 10;
const AUTH_FILE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct OverlayAuthSource {
    base: Arc<dyn AuthSource>,
    runtime_api_keys: Arc<Mutex<HashMap<String, String>>>,
}

impl OverlayAuthSource {
    pub fn new(base: Arc<dyn AuthSource>) -> Self {
        Self {
            base,
            runtime_api_keys: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set_runtime_api_key(&self, provider: impl Into<String>, api_key: impl Into<String>) {
        self.runtime_api_keys
            .lock()
            .unwrap()
            .insert(provider.into(), api_key.into());
    }
}

impl AuthSource for OverlayAuthSource {
    fn has_auth(&self, provider: &str) -> bool {
        self.runtime_api_keys.lock().unwrap().contains_key(provider) || self.base.has_auth(provider)
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        self.runtime_api_keys
            .lock()
            .unwrap()
            .get(provider)
            .cloned()
            .or_else(|| self.base.get_api_key(provider))
    }

    fn get_api_key_for_request<'a>(
        &'a self,
        provider: &'a str,
    ) -> pi_coding_agent_core::AuthApiKeyFuture<'a> {
        Box::pin(async move {
            let runtime_key = self.runtime_api_keys.lock().unwrap().get(provider).cloned();
            match runtime_key {
                Some(runtime_key) => Some(runtime_key),
                None => self.base.get_api_key_for_request(provider).await,
            }
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct EnvAuthSource;

impl EnvAuthSource {
    pub fn new() -> Self {
        Self
    }
}

impl AuthSource for EnvAuthSource {
    fn has_auth(&self, provider: &str) -> bool {
        self.get_api_key(provider).is_some()
    }

    fn get_api_key(&self, provider: &str) -> Option<String> {
        get_env_api_key(provider)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderSummary {
    pub id: String,
    pub name: String,
}

pub fn oauth_provider_summaries() -> Vec<OAuthProviderSummary> {
    get_oauth_provider_info_list()
        .into_iter()
        .filter(|provider| provider.available)
        .map(|provider| OAuthProviderSummary {
            id: provider.id,
            name: provider.name,
        })
        .collect()
}

pub fn oauth_provider_name(provider_id: &str) -> Option<String> {
    get_oauth_provider(provider_id).map(|provider| provider.name().to_owned())
}

pub fn list_persisted_oauth_providers(auth_path: &Path) -> Result<Vec<String>, String> {
    with_auth_file_entries(auth_path, |entries| {
        let providers = entries
            .into_iter()
            .filter_map(|(provider, entry)| {
                (auth_file_entry_type(&entry) == Some("oauth")).then_some(provider)
            })
            .collect::<Vec<_>>();
        Ok((providers, None))
    })
}

pub fn remove_persisted_oauth_provider(auth_path: &Path, provider: &str) -> Result<bool, String> {
    let provider = provider.to_owned();
    with_auth_file_entries(auth_path, move |mut entries| {
        let removed = entries
            .get(&provider)
            .and_then(|entry| auth_file_entry_type(entry))
            .is_some_and(|entry_type| entry_type == "oauth");
        if removed {
            entries.remove(&provider);
            Ok((true, Some(entries)))
        } else {
            Ok((false, None))
        }
    })
}

pub fn persist_oauth_credentials(
    auth_path: &Path,
    provider: &str,
    credentials: &OAuthCredentials,
) -> Result<(), String> {
    let provider = provider.to_owned();
    let credentials = credentials.clone();
    with_auth_file_entries(auth_path, move |mut entries| {
        entries.insert(
            provider.clone(),
            oauth_credentials_to_value(credentials.clone()),
        );
        Ok(((), Some(entries)))
    })
}

pub async fn run_terminal_oauth_login(
    auth_path: PathBuf,
    provider_id: String,
    ui_host: Arc<dyn ExternalEditorHost>,
) -> Result<String, String> {
    let provider = get_oauth_provider(&provider_id)
        .ok_or_else(|| format!("Unknown OAuth provider: {provider_id}"))?;
    let provider_name = provider.name().to_owned();

    ui_host.stop();
    let result = login_from_terminal(&auth_path, &provider_id, &provider_name).await;
    ui_host.start();
    ui_host.request_render();

    result.map(|()| provider_name)
}

async fn login_from_terminal(
    auth_path: &Path,
    provider_id: &str,
    provider_name: &str,
) -> Result<(), String> {
    print_terminal_line("");
    print_terminal_line(&format!("Login to {provider_name}"));
    print_terminal_line("");

    let provider = get_oauth_provider(provider_id)
        .ok_or_else(|| format!("Unknown OAuth provider: {provider_id}"))?;
    let provider_id = provider_id.to_owned();
    let callbacks =
        OAuthLoginCallbacks::new(
            move |info| {
                print_terminal_line(&format!("Open this URL in your browser:\n{}", info.url));
                if let Some(instructions) = info.instructions {
                    print_terminal_line(&instructions);
                }
                let _ = best_effort_open_browser(&info.url);
                Ok(())
            },
            move |prompt| async move {
                prompt_for_terminal_input(prompt.message, prompt.placeholder).await
            },
        )
        .with_progress(|message| {
            print_terminal_line(&message);
            Ok(())
        });

    let credentials = provider
        .login(callbacks)
        .await
        .map_err(|error| format!("{provider_name}: {error}"))?;
    persist_oauth_credentials(auth_path, &provider_id, &credentials)?;
    print_terminal_line(&format!(
        "Login complete. Credentials saved to {}",
        auth_path.display()
    ));
    Ok(())
}

async fn prompt_for_terminal_input(
    message: String,
    placeholder: Option<String>,
) -> Result<String, String> {
    spawn_blocking(move || {
        loop {
            if let Some(placeholder) = placeholder.as_deref().filter(|value| !value.is_empty()) {
                print!("{message} [{placeholder}]: ");
            } else {
                print!("{message}: ");
            }
            io::stdout()
                .flush()
                .map_err(|error| format!("Failed to flush prompt: {error}"))?;

            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .map_err(|error| format!("Failed to read input: {error}"))?;
            let input = trim_line_endings(input);
            if !input.trim().is_empty() {
                return Ok(input);
            }

            print_terminal_line("Input required.");
        }
    })
    .await
    .map_err(|error| format!("OAuth prompt task failed: {error}"))?
}

fn trim_line_endings(mut input: String) -> String {
    while input.ends_with('\n') || input.ends_with('\r') {
        input.pop();
    }
    input
}

fn print_terminal_line(line: &str) {
    println!("{line}");
    let _ = io::stdout().flush();
}

pub(crate) fn best_effort_open_browser(url: &str) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open").arg(url).spawn();
        return Ok(());
    }

    if cfg!(windows) {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
        return Ok(());
    }

    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    Ok(())
}

fn auth_file_entry_type(entry: &Value) -> Option<&str> {
    entry.get("type").and_then(Value::as_str)
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

fn with_auth_file_entries<T>(
    auth_path: &Path,
    mut callback: impl FnMut(Map<String, Value>) -> Result<(T, Option<Map<String, Value>>), String>,
) -> Result<T, String> {
    ensure_auth_file_parent_dir(auth_path)?;

    if !auth_path.exists() {
        let (result, next_entries) = callback(Map::new())?;
        if let Some(entries) = next_entries {
            write_auth_file_entries(auth_path, &entries)?;
        }
        return Ok(result);
    }

    let _lock = acquire_auth_file_lock(auth_path)?;
    let entries = read_auth_file_entries(auth_path)?;
    let (result, next_entries) = callback(entries)?;
    if let Some(entries) = next_entries {
        write_auth_file_entries(auth_path, &entries)?;
    }
    Ok(result)
}

fn ensure_auth_file_parent_dir(auth_path: &Path) -> Result<(), String> {
    let parent = auth_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create {}: {error}", parent.display()))
}

fn read_auth_file_entries(auth_path: &Path) -> Result<Map<String, Value>, String> {
    let content = fs::read_to_string(auth_path)
        .map_err(|error| format!("Failed to read {}: {error}", auth_path.display()))?;
    if content.trim().is_empty() {
        return Ok(Map::new());
    }

    let root = serde_json::from_str::<Value>(&content)
        .map_err(|error| format!("Failed to parse {}: {error}", auth_path.display()))?;
    root.as_object()
        .cloned()
        .ok_or_else(|| format!("{} must contain a JSON object", auth_path.display()))
}

fn write_auth_file_entries(auth_path: &Path, entries: &Map<String, Value>) -> Result<(), String> {
    let rendered = serde_json::to_string_pretty(entries)
        .map_err(|error| format!("Failed to serialize {}: {error}", auth_path.display()))?;
    fs::write(auth_path, rendered)
        .map_err(|error| format!("Failed to write {}: {error}", auth_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(auth_path, permissions).map_err(|error| {
            format!(
                "Failed to set permissions on {}: {error}",
                auth_path.display()
            )
        })?;
    }

    Ok(())
}

struct AuthFileLock {
    path: PathBuf,
}

impl Drop for AuthFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_auth_file_lock(auth_path: &Path) -> Result<AuthFileLock, String> {
    let lock_path = PathBuf::from(format!("{}.lock", auth_path.to_string_lossy()));

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
                std::thread::sleep(AUTH_FILE_LOCK_RETRY_DELAY);
            }
            Err(error) => return Err(format!("Failed to acquire auth.json lock: {error}")),
        }
    }

    Err(String::from("Failed to acquire auth.json lock"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::{
        OAuthCredentialsFuture, OAuthProvider, register_oauth_provider, unregister_oauth_provider,
    };
    use serde_json::json;
    use std::{
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    fn registry_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pi-coding-agent-cli-auth-{prefix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn persisted_oauth_provider_helpers_round_trip_entries() {
        let temp_dir = unique_temp_dir("persist-round-trip");
        let auth_path = temp_dir.join("auth.json");

        persist_oauth_credentials(
            &auth_path,
            "example",
            &OAuthCredentials::new("refresh-token", "access-token", 42)
                .with_extra("accountId", json!("acc_123")),
        )
        .unwrap();

        let providers = list_persisted_oauth_providers(&auth_path).unwrap();
        assert_eq!(providers, vec![String::from("example")]);

        let saved = fs::read_to_string(&auth_path).unwrap();
        assert!(saved.contains("\"type\": \"oauth\""), "content: {saved}");
        assert!(saved.contains("acc_123"), "content: {saved}");

        assert!(remove_persisted_oauth_provider(&auth_path, "example").unwrap());
        assert!(
            list_persisted_oauth_providers(&auth_path)
                .unwrap()
                .is_empty()
        );
        assert!(!remove_persisted_oauth_provider(&auth_path, "missing").unwrap());
    }

    #[derive(Default)]
    struct RecordingUiHost {
        stop_count: AtomicUsize,
        start_count: AtomicUsize,
        render_count: AtomicUsize,
    }

    impl ExternalEditorHost for RecordingUiHost {
        fn stop(&self) {
            self.stop_count.fetch_add(1, Ordering::Relaxed);
        }

        fn start(&self) {
            self.start_count.fetch_add(1, Ordering::Relaxed);
        }

        fn request_render(&self) {
            self.render_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Debug)]
    struct TestOAuthProvider {
        id: &'static str,
        name: &'static str,
    }

    impl OAuthProvider for TestOAuthProvider {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.name
        }

        fn login<'a>(&'a self, callbacks: OAuthLoginCallbacks) -> OAuthCredentialsFuture<'a> {
            Box::pin(async move {
                callbacks.auth(pi_ai::OAuthAuthInfo {
                    url: String::from("https://example.com/login"),
                    instructions: Some(String::from("Finish login in your browser.")),
                })?;
                callbacks.progress("Waiting for browser authentication...")?;
                Ok(OAuthCredentials::new("refresh-token", "access-token", 123))
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

    #[tokio::test]
    async fn terminal_oauth_login_persists_credentials_and_restores_ui_host() {
        let _guard = registry_lock().lock().unwrap();
        let provider_id = "test-cli-oauth";
        register_oauth_provider(Arc::new(TestOAuthProvider {
            id: provider_id,
            name: "Test CLI OAuth",
        }));

        let temp_dir = unique_temp_dir("terminal-login");
        let auth_path = temp_dir.join("auth.json");
        let ui_host = Arc::new(RecordingUiHost::default());

        let provider_name = run_terminal_oauth_login(
            auth_path.clone(),
            String::from(provider_id),
            ui_host.clone(),
        )
        .await
        .unwrap();

        assert_eq!(provider_name, "Test CLI OAuth");
        assert_eq!(ui_host.stop_count.load(Ordering::Relaxed), 1);
        assert_eq!(ui_host.start_count.load(Ordering::Relaxed), 1);
        assert_eq!(ui_host.render_count.load(Ordering::Relaxed), 1);

        let content = fs::read_to_string(&auth_path).unwrap();
        assert!(content.contains("access-token"), "content: {content}");

        unregister_oauth_provider(provider_id);
    }
}
