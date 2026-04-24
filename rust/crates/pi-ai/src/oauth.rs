use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use parking_lot::Mutex;
use pi_events::Model;
use rand::{RngCore as _, rngs::OsRng};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{
    future::Future,
    io::{Read as _, Write as _},
    pin::Pin,
    sync::{Arc, Once, OnceLock, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[cfg(test)]
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::{oneshot, watch};

const OAUTH_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const CALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(20);

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_CALLBACK_HOST: &str = "127.0.0.1";
const ANTHROPIC_CALLBACK_PORT: u16 = 53692;
const ANTHROPIC_CALLBACK_PATH: &str = "/callback";
const ANTHROPIC_REDIRECT_URI: &str = "http://localhost:53692/callback";
const ANTHROPIC_SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

const OPENAI_CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const OPENAI_CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const OPENAI_CODEX_CALLBACK_HOST: &str = "127.0.0.1";
const OPENAI_CODEX_CALLBACK_PORT: u16 = 1455;
const OPENAI_CODEX_CALLBACK_PATH: &str = "/auth/callback";
const OPENAI_CODEX_SCOPE: &str = "openid profile email offline_access";
const OPENAI_CODEX_AUTH_CLAIM: &str = "https://api.openai.com/auth";

pub type OAuthPromptFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
pub type OAuthCredentialsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<OAuthCredentials, String>> + Send + 'a>>;

type OAuthAuthHandler = dyn Fn(OAuthAuthInfo) -> Result<(), String> + Send + Sync;
type OAuthPromptHandler = dyn Fn(OAuthPrompt) -> OAuthPromptFuture + Send + Sync;
type OAuthProgressHandler = dyn Fn(String) -> Result<(), String> + Send + Sync;
type OAuthManualCodeInputHandler = dyn Fn() -> OAuthPromptFuture + Send + Sync;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthCredentials {
    pub refresh: String,
    pub access: String,
    pub expires: i64,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl OAuthCredentials {
    pub fn new(refresh: impl Into<String>, access: impl Into<String>, expires: i64) -> Self {
        Self {
            refresh: refresh.into(),
            access: access.into(),
            expires,
            extra: serde_json::Map::new(),
        }
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.extra.get(key).and_then(Value::as_str)
    }
}

pub type OAuthProviderId = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthPrompt {
    pub message: String,
    pub placeholder: Option<String>,
    pub allow_empty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthAuthInfo {
    pub url: String,
    pub instructions: Option<String>,
}

#[derive(Clone)]
pub struct OAuthLoginCallbacks {
    pub on_auth: Arc<OAuthAuthHandler>,
    pub on_prompt: Arc<OAuthPromptHandler>,
    pub on_progress: Option<Arc<OAuthProgressHandler>>,
    pub on_manual_code_input: Option<Arc<OAuthManualCodeInputHandler>>,
    pub signal: Option<watch::Receiver<bool>>,
}

impl OAuthLoginCallbacks {
    pub fn new<FA, FP, Fut>(on_auth: FA, on_prompt: FP) -> Self
    where
        FA: Fn(OAuthAuthInfo) -> Result<(), String> + Send + Sync + 'static,
        FP: Fn(OAuthPrompt) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        Self {
            on_auth: Arc::new(on_auth),
            on_prompt: Arc::new(move |prompt| Box::pin(on_prompt(prompt))),
            on_progress: None,
            on_manual_code_input: None,
            signal: None,
        }
    }

    pub fn with_progress<F>(mut self, on_progress: F) -> Self
    where
        F: Fn(String) -> Result<(), String> + Send + Sync + 'static,
    {
        self.on_progress = Some(Arc::new(on_progress));
        self
    }

    pub fn with_manual_code_input<F, Fut>(mut self, on_manual_code_input: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        self.on_manual_code_input = Some(Arc::new(move || Box::pin(on_manual_code_input())));
        self
    }

    pub fn with_signal(mut self, signal: watch::Receiver<bool>) -> Self {
        self.signal = Some(signal);
        self
    }

    pub fn auth(&self, info: OAuthAuthInfo) -> Result<(), String> {
        (self.on_auth)(info)
    }

    pub async fn prompt(&self, prompt: OAuthPrompt) -> Result<String, String> {
        (self.on_prompt)(prompt).await
    }

    pub fn progress(&self, message: impl Into<String>) -> Result<(), String> {
        match &self.on_progress {
            Some(handler) => handler(message.into()),
            None => Ok(()),
        }
    }

    pub async fn manual_code_input(&self) -> Option<Result<String, String>> {
        let handler = self.on_manual_code_input.as_ref()?;
        Some(handler().await)
    }

    pub fn is_aborted(&self) -> bool {
        self.signal
            .as_ref()
            .map(|signal| *signal.borrow())
            .unwrap_or(false)
    }
}

impl std::fmt::Debug for OAuthLoginCallbacks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OAuthLoginCallbacks(..)")
    }
}

pub trait OAuthProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    fn uses_callback_server(&self) -> bool {
        false
    }

    fn login<'a>(&'a self, callbacks: OAuthLoginCallbacks) -> OAuthCredentialsFuture<'a>;

    fn refresh_token<'a>(&'a self, credentials: OAuthCredentials) -> OAuthCredentialsFuture<'a>;

    fn get_api_key(&self, credentials: &OAuthCredentials) -> Result<String, String>;

    fn modify_models(&self, models: Vec<Model>, _credentials: &OAuthCredentials) -> Vec<Model> {
        models
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderInfo {
    pub id: OAuthProviderId,
    pub name: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OAuthApiKeyResult {
    pub new_credentials: OAuthCredentials,
    pub api_key: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OAuthRefreshOverrides<'a> {
    pub anthropic_token_url: Option<&'a str>,
    pub openai_codex_token_url: Option<&'a str>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AnthropicOAuthProvider;

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenAiCodexOAuthProvider;

impl OAuthProvider for AnthropicOAuthProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn name(&self) -> &str {
        "Anthropic (Claude Pro/Max)"
    }

    fn uses_callback_server(&self) -> bool {
        true
    }

    fn login<'a>(&'a self, callbacks: OAuthLoginCallbacks) -> OAuthCredentialsFuture<'a> {
        Box::pin(async move { login_anthropic(callbacks).await })
    }

    fn refresh_token<'a>(&'a self, credentials: OAuthCredentials) -> OAuthCredentialsFuture<'a> {
        Box::pin(async move { refresh_anthropic_token(&credentials.refresh).await })
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> Result<String, String> {
        Ok(credentials.access.clone())
    }
}

impl OAuthProvider for OpenAiCodexOAuthProvider {
    fn id(&self) -> &str {
        "openai-codex"
    }

    fn name(&self) -> &str {
        "ChatGPT Plus/Pro (Codex Subscription)"
    }

    fn uses_callback_server(&self) -> bool {
        true
    }

    fn login<'a>(&'a self, callbacks: OAuthLoginCallbacks) -> OAuthCredentialsFuture<'a> {
        Box::pin(async move { login_openai_codex(callbacks).await })
    }

    fn refresh_token<'a>(&'a self, credentials: OAuthCredentials) -> OAuthCredentialsFuture<'a> {
        Box::pin(async move { refresh_openai_codex_token(&credentials.refresh).await })
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> Result<String, String> {
        Ok(credentials.access.clone())
    }
}

fn built_in_oauth_providers() -> &'static Vec<Arc<dyn OAuthProvider>> {
    static PROVIDERS: OnceLock<Vec<Arc<dyn OAuthProvider>>> = OnceLock::new();
    PROVIDERS.get_or_init(|| {
        vec![
            Arc::new(AnthropicOAuthProvider) as Arc<dyn OAuthProvider>,
            Arc::new(OpenAiCodexOAuthProvider) as Arc<dyn OAuthProvider>,
        ]
    })
}

fn oauth_provider_registry() -> &'static Mutex<Vec<Arc<dyn OAuthProvider>>> {
    static REGISTRY: OnceLock<Mutex<Vec<Arc<dyn OAuthProvider>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

fn ensure_builtin_oauth_providers_registered() {
    static BUILTINS: Once = Once::new();
    BUILTINS.call_once(reset_oauth_providers);
}

fn replace_or_insert_provider(
    providers: &mut Vec<Arc<dyn OAuthProvider>>,
    replacement: Arc<dyn OAuthProvider>,
) {
    if let Some(index) = providers
        .iter()
        .position(|provider| provider.id() == replacement.id())
    {
        providers[index] = replacement;
    } else {
        providers.push(replacement);
    }
}

pub fn get_oauth_provider(id: &str) -> Option<Arc<dyn OAuthProvider>> {
    ensure_builtin_oauth_providers_registered();
    oauth_provider_registry()
        .lock()
        .iter()
        .find(|provider| provider.id() == id)
        .cloned()
}

pub fn register_oauth_provider(provider: Arc<dyn OAuthProvider>) {
    ensure_builtin_oauth_providers_registered();
    replace_or_insert_provider(&mut oauth_provider_registry().lock(), provider);
}

pub fn unregister_oauth_provider(id: &str) {
    ensure_builtin_oauth_providers_registered();
    let mut providers = oauth_provider_registry().lock();
    if let Some(built_in) = built_in_oauth_providers()
        .iter()
        .find(|provider| provider.id() == id)
    {
        replace_or_insert_provider(&mut providers, built_in.clone());
    } else {
        providers.retain(|provider| provider.id() != id);
    }
}

pub fn reset_oauth_providers() {
    let mut providers = oauth_provider_registry().lock();
    providers.clear();
    providers.extend(built_in_oauth_providers().iter().cloned());
}

pub fn get_oauth_providers() -> Vec<Arc<dyn OAuthProvider>> {
    ensure_builtin_oauth_providers_registered();
    oauth_provider_registry().lock().clone()
}

pub fn get_oauth_provider_info_list() -> Vec<OAuthProviderInfo> {
    get_oauth_providers()
        .into_iter()
        .map(|provider| OAuthProviderInfo {
            id: provider.id().to_string(),
            name: provider.name().to_string(),
            available: true,
        })
        .collect()
}

pub async fn refresh_oauth_token(
    provider_id: &str,
    credentials: &OAuthCredentials,
) -> Result<OAuthCredentials, String> {
    refresh_oauth_token_with_overrides(provider_id, credentials, &OAuthRefreshOverrides::default())
        .await
}

pub async fn refresh_oauth_token_with_overrides(
    provider_id: &str,
    credentials: &OAuthCredentials,
    overrides: &OAuthRefreshOverrides<'_>,
) -> Result<OAuthCredentials, String> {
    match provider_id {
        "anthropic" => {
            refresh_anthropic_token_with_url(
                &credentials.refresh,
                overrides.anthropic_token_url.unwrap_or(ANTHROPIC_TOKEN_URL),
            )
            .await
        }
        "openai-codex" => {
            refresh_openai_codex_token_with_url(
                &credentials.refresh,
                overrides
                    .openai_codex_token_url
                    .unwrap_or(OPENAI_CODEX_TOKEN_URL),
            )
            .await
        }
        _ => {
            let provider = get_oauth_provider(provider_id)
                .ok_or_else(|| format!("Unsupported OAuth provider: {provider_id}"))?;
            provider.refresh_token(credentials.clone()).await
        }
    }
}

pub async fn get_oauth_api_key(
    provider_id: &str,
    credentials: &std::collections::BTreeMap<String, OAuthCredentials>,
) -> Result<Option<OAuthApiKeyResult>, String> {
    let Some(provider) = get_oauth_provider(provider_id) else {
        return Err(format!("Unknown OAuth provider: {provider_id}"));
    };

    let Some(mut credentials) = credentials.get(provider_id).cloned() else {
        return Ok(None);
    };

    if now_ms() >= credentials.expires {
        credentials = refresh_oauth_token(provider_id, &credentials).await?;
    }

    let api_key = provider.get_api_key(&credentials)?;
    Ok(Some(OAuthApiKeyResult {
        new_credentials: credentials,
        api_key,
    }))
}

pub async fn login_anthropic(callbacks: OAuthLoginCallbacks) -> Result<OAuthCredentials, String> {
    login_anthropic_with_urls(callbacks, ANTHROPIC_AUTHORIZE_URL, ANTHROPIC_TOKEN_URL).await
}

pub async fn refresh_anthropic_token(refresh_token: &str) -> Result<OAuthCredentials, String> {
    refresh_anthropic_token_with_url(refresh_token, ANTHROPIC_TOKEN_URL).await
}

pub async fn login_openai_codex(
    callbacks: OAuthLoginCallbacks,
) -> Result<OAuthCredentials, String> {
    login_openai_codex_with_originator(callbacks, "pi").await
}

pub async fn login_openai_codex_with_originator(
    callbacks: OAuthLoginCallbacks,
    originator: &str,
) -> Result<OAuthCredentials, String> {
    login_openai_codex_with_url(callbacks, originator, OPENAI_CODEX_TOKEN_URL).await
}

pub async fn refresh_openai_codex_token(refresh_token: &str) -> Result<OAuthCredentials, String> {
    refresh_openai_codex_token_with_url(refresh_token, OPENAI_CODEX_TOKEN_URL).await
}

async fn login_anthropic_with_urls(
    callbacks: OAuthLoginCallbacks,
    authorize_url: &str,
    token_url: &str,
) -> Result<OAuthCredentials, String> {
    ensure_not_aborted(&callbacks)?;

    let (verifier, challenge) = generate_pkce();
    let mut server = start_callback_server(
        ANTHROPIC_CALLBACK_HOST,
        ANTHROPIC_CALLBACK_PORT,
        Arc::new({
            let expected_state = verifier.clone();
            move |url| handle_anthropic_callback(url, &expected_state)
        }),
        false,
    )?;

    let result = async {
        let mut url = Url::parse(authorize_url)
            .map_err(|error| format!("Invalid Anthropic authorize URL: {error}"))?;
        url.query_pairs_mut()
            .append_pair("code", "true")
            .append_pair("client_id", ANTHROPIC_CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", ANTHROPIC_REDIRECT_URI)
            .append_pair("scope", ANTHROPIC_SCOPES)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &verifier);

        callbacks.auth(OAuthAuthInfo {
            url: url.to_string(),
            instructions: Some(
                "Complete login in your browser. If the browser is on another machine, paste the final redirect URL here.".into(),
            ),
        })?;

        let mut code = None;
        let mut state = None;
        let mut redirect_uri_for_exchange = ANTHROPIC_REDIRECT_URI.to_string();
        let callback_receiver = server.take_receiver();

        if callbacks.on_manual_code_input.is_some() {
            let manual = callbacks.manual_code_input();
            let wait_for_code: Pin<
                Box<dyn Future<Output = Result<Option<CallbackAuthorization>, String>> + Send>,
            > = match callback_receiver {
                Some(receiver) => Box::pin(await_callback_code(receiver)),
                None => Box::pin(async { Ok(None) }),
            };
            tokio::pin!(manual);
            tokio::pin!(wait_for_code);

            match tokio::select! {
                manual = &mut manual => ManualOrCallback::Manual(manual),
                callback = &mut wait_for_code => ManualOrCallback::Callback(callback),
            } {
                ManualOrCallback::Manual(Some(Ok(input))) => {
                    server.cancel_wait();
                    let parsed = parse_authorization_input(&input);
                    if parsed.state.as_deref().is_some_and(|value| value != verifier) {
                        return Err("OAuth state mismatch".into());
                    }
                    code = parsed.code;
                    state = parsed.state.or_else(|| Some(verifier.clone()));
                }
                ManualOrCallback::Manual(Some(Err(error))) => {
                    server.cancel_wait();
                    return Err(error);
                }
                ManualOrCallback::Manual(None) => {}
                ManualOrCallback::Callback(callback) => {
                    if let Some(callback) = callback? {
                        code = Some(callback.code);
                        state = callback.state;
                        redirect_uri_for_exchange = ANTHROPIC_REDIRECT_URI.into();
                    }
                }
            }
        } else if let Some(receiver) = callback_receiver {
            if let Some(callback) = await_callback_code(receiver).await? {
                code = Some(callback.code);
                state = callback.state;
                redirect_uri_for_exchange = ANTHROPIC_REDIRECT_URI.into();
            }
        }

        if code.is_none() {
            let input = callbacks
                .prompt(OAuthPrompt {
                    message: "Paste the authorization code or full redirect URL:".into(),
                    placeholder: Some(ANTHROPIC_REDIRECT_URI.into()),
                    allow_empty: false,
                })
                .await?;
            let parsed = parse_authorization_input(&input);
            if parsed.state.as_deref().is_some_and(|value| value != verifier) {
                return Err("OAuth state mismatch".into());
            }
            code = parsed.code;
            state = parsed.state.or_else(|| Some(verifier.clone()));
        }

        let code = code.ok_or_else(|| "Missing authorization code".to_string())?;
        let state = state.ok_or_else(|| "Missing OAuth state".to_string())?;

        callbacks.progress("Exchanging authorization code for tokens...")?;
        exchange_anthropic_authorization_code(
            &code,
            &state,
            &verifier,
            &redirect_uri_for_exchange,
            token_url,
        )
        .await
    }
    .await;

    server.close();
    result
}

async fn refresh_anthropic_token_with_url(
    refresh_token: &str,
    token_url: &str,
) -> Result<OAuthCredentials, String> {
    let client = oauth_http_client()?;
    let response = client
        .post(token_url)
        .header("accept", "application/json")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": ANTHROPIC_CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(|error| format!("Anthropic token refresh request failed: {error}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Anthropic token refresh request failed: {status}: {body}"
        ));
    }

    let payload = serde_json::from_str::<AnthropicTokenResponse>(&body)
        .map_err(|error| format!("Anthropic token refresh returned invalid JSON: {error}"))?;

    Ok(OAuthCredentials::new(
        payload.refresh_token,
        payload.access_token,
        now_ms()
            .saturating_add(payload.expires_in.saturating_mul(1000))
            .saturating_sub(5 * 60 * 1000),
    ))
}

async fn exchange_anthropic_authorization_code(
    code: &str,
    state: &str,
    verifier: &str,
    redirect_uri: &str,
    token_url: &str,
) -> Result<OAuthCredentials, String> {
    let client = oauth_http_client()?;
    let response = client
        .post(token_url)
        .header("accept", "application/json")
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": ANTHROPIC_CLIENT_ID,
            "code": code,
            "state": state,
            "redirect_uri": redirect_uri,
            "code_verifier": verifier,
        }))
        .send()
        .await
        .map_err(|error| format!("Anthropic token exchange request failed: {error}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Anthropic token exchange request failed: {status}: {body}"
        ));
    }

    let payload = serde_json::from_str::<AnthropicTokenResponse>(&body)
        .map_err(|error| format!("Anthropic token exchange returned invalid JSON: {error}"))?;

    Ok(OAuthCredentials::new(
        payload.refresh_token,
        payload.access_token,
        now_ms()
            .saturating_add(payload.expires_in.saturating_mul(1000))
            .saturating_sub(5 * 60 * 1000),
    ))
}

async fn login_openai_codex_with_url(
    callbacks: OAuthLoginCallbacks,
    originator: &str,
    token_url: &str,
) -> Result<OAuthCredentials, String> {
    ensure_not_aborted(&callbacks)?;

    let (verifier, challenge) = generate_pkce();
    let state = create_state();
    let mut server = start_callback_server(
        OPENAI_CODEX_CALLBACK_HOST,
        OPENAI_CODEX_CALLBACK_PORT,
        Arc::new({
            let expected_state = state.clone();
            move |url| handle_openai_codex_callback(url, &expected_state)
        }),
        true,
    )?;

    let result = async {
        let mut url = Url::parse(OPENAI_CODEX_AUTHORIZE_URL)
            .map_err(|error| format!("Invalid OpenAI Codex authorize URL: {error}"))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", OPENAI_CODEX_CLIENT_ID)
            .append_pair("redirect_uri", OPENAI_CODEX_REDIRECT_URI)
            .append_pair("scope", OPENAI_CODEX_SCOPE)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("id_token_add_organizations", "true")
            .append_pair("codex_cli_simplified_flow", "true")
            .append_pair("originator", originator);

        callbacks.auth(OAuthAuthInfo {
            url: url.to_string(),
            instructions: Some("A browser window should open. Complete login to finish.".into()),
        })?;

        let mut code = None;
        let callback_receiver = server.take_receiver();

        if callbacks.on_manual_code_input.is_some() {
            let manual = callbacks.manual_code_input();
            let wait_for_code: Pin<
                Box<dyn Future<Output = Result<Option<CallbackAuthorization>, String>> + Send>,
            > = match callback_receiver {
                Some(receiver) => Box::pin(await_callback_code(receiver)),
                None => Box::pin(async { Ok(None) }),
            };
            tokio::pin!(manual);
            tokio::pin!(wait_for_code);

            match tokio::select! {
                manual = &mut manual => ManualOrCallback::Manual(manual),
                callback = &mut wait_for_code => ManualOrCallback::Callback(callback),
            } {
                ManualOrCallback::Manual(Some(Ok(input))) => {
                    server.cancel_wait();
                    let parsed = parse_authorization_input(&input);
                    if parsed.state.as_deref().is_some_and(|value| value != state) {
                        return Err("State mismatch".into());
                    }
                    code = parsed.code;
                }
                ManualOrCallback::Manual(Some(Err(error))) => {
                    server.cancel_wait();
                    return Err(error);
                }
                ManualOrCallback::Manual(None) => {}
                ManualOrCallback::Callback(callback) => {
                    if let Some(callback) = callback? {
                        code = Some(callback.code);
                    }
                }
            }
        } else if let Some(receiver) = callback_receiver {
            if let Some(callback) = await_callback_code(receiver).await? {
                code = Some(callback.code);
            }
        }

        if code.is_none() {
            let input = callbacks
                .prompt(OAuthPrompt {
                    message: "Paste the authorization code (or full redirect URL):".into(),
                    placeholder: None,
                    allow_empty: false,
                })
                .await?;
            let parsed = parse_authorization_input(&input);
            if parsed.state.as_deref().is_some_and(|value| value != state) {
                return Err("State mismatch".into());
            }
            code = parsed.code;
        }

        let code = code.ok_or_else(|| "Missing authorization code".to_string())?;
        exchange_openai_codex_authorization_code(&code, &verifier, token_url).await
    }
    .await;

    server.close();
    result
}

async fn refresh_openai_codex_token_with_url(
    refresh_token: &str,
    token_url: &str,
) -> Result<OAuthCredentials, String> {
    let client = oauth_http_client()?;
    let response = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|error| format!("OpenAI Codex token refresh request failed: {error}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "OpenAI Codex token refresh failed: {status}: {body}"
        ));
    }

    let payload = serde_json::from_str::<OpenAiCodexTokenResponse>(&body)
        .map_err(|error| format!("OpenAI Codex token refresh returned invalid JSON: {error}"))?;
    oauth_credentials_from_openai_codex_response(payload)
}

async fn exchange_openai_codex_authorization_code(
    code: &str,
    verifier: &str,
    token_url: &str,
) -> Result<OAuthCredentials, String> {
    let client = oauth_http_client()?;
    let response = client
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", OPENAI_CODEX_REDIRECT_URI),
        ])
        .send()
        .await
        .map_err(|error| format!("OpenAI Codex token exchange request failed: {error}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "OpenAI Codex token exchange failed: {status}: {body}"
        ));
    }

    let payload = serde_json::from_str::<OpenAiCodexTokenResponse>(&body)
        .map_err(|error| format!("OpenAI Codex token exchange returned invalid JSON: {error}"))?;
    oauth_credentials_from_openai_codex_response(payload)
}

fn oauth_credentials_from_openai_codex_response(
    payload: OpenAiCodexTokenResponse,
) -> Result<OAuthCredentials, String> {
    let account_id = extract_openai_codex_account_id(&payload.access_token)
        .ok_or_else(|| "Failed to extract accountId from token".to_string())?;

    Ok(OAuthCredentials::new(
        payload.refresh_token,
        payload.access_token,
        now_ms().saturating_add(payload.expires_in.saturating_mul(1000)),
    )
    .with_extra("accountId", Value::String(account_id)))
}

fn oauth_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(OAUTH_HTTP_TIMEOUT)
        .build()
        .map_err(|error| format!("Failed to create HTTP client: {error}"))
}

fn generate_pkce() -> (String, String) {
    let mut verifier_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut verifier_bytes);
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    (verifier, challenge)
}

fn create_state() -> String {
    let mut state_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut state_bytes);
    state_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedAuthorizationInput {
    code: Option<String>,
    state: Option<String>,
}

fn parse_authorization_input(input: &str) -> ParsedAuthorizationInput {
    let value = input.trim();
    if value.is_empty() {
        return ParsedAuthorizationInput::default();
    }

    if let Ok(url) = Url::parse(value) {
        return ParsedAuthorizationInput {
            code: url
                .query_pairs()
                .find_map(|(key, value)| (key == "code").then(|| value.into_owned())),
            state: url
                .query_pairs()
                .find_map(|(key, value)| (key == "state").then(|| value.into_owned())),
        };
    }

    if let Some((code, state)) = value.split_once('#') {
        return ParsedAuthorizationInput {
            code: (!code.is_empty()).then(|| code.to_string()),
            state: (!state.is_empty()).then(|| state.to_string()),
        };
    }

    if value.contains("code=") {
        if let Ok(mut url) = Url::parse("http://localhost/") {
            url.set_query(Some(value.trim_start_matches('?')));
            return ParsedAuthorizationInput {
                code: url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "code").then(|| value.into_owned())),
                state: url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "state").then(|| value.into_owned())),
            };
        }
    }

    ParsedAuthorizationInput {
        code: Some(value.to_string()),
        state: None,
    }
}

#[derive(Debug)]
struct CallbackAuthorization {
    code: String,
    state: Option<String>,
}

enum CallbackHandlerResult {
    Continue {
        status: u16,
        body: String,
    },
    Complete {
        status: u16,
        body: String,
        authorization: CallbackAuthorization,
    },
}

type CallbackHandler = dyn Fn(&Url) -> CallbackHandlerResult + Send + Sync + 'static;

struct CallbackServer {
    receiver: Option<oneshot::Receiver<Option<CallbackAuthorization>>>,
    cancel_sender: Option<mpsc::Sender<()>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl CallbackServer {
    fn take_receiver(&mut self) -> Option<oneshot::Receiver<Option<CallbackAuthorization>>> {
        self.receiver.take()
    }

    fn cancel_wait(&mut self) {
        if let Some(sender) = self.cancel_sender.take() {
            let _ = sender.send(());
        }
    }

    fn shutdown(&mut self) {
        self.cancel_wait();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    fn close(&mut self) {
        self.shutdown();
    }
}

impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn await_callback_code(
    receiver: oneshot::Receiver<Option<CallbackAuthorization>>,
) -> Result<Option<CallbackAuthorization>, String> {
    receiver
        .await
        .map_err(|error| format!("OAuth callback server closed unexpectedly: {error}"))
}

fn start_callback_server(
    host: &str,
    port: u16,
    handler: Arc<CallbackHandler>,
    fallback_on_bind_error: bool,
) -> Result<CallbackServer, String> {
    let listener = match std::net::TcpListener::bind((host, port)) {
        Ok(listener) => listener,
        Err(error) if fallback_on_bind_error => {
            let (sender, receiver) = oneshot::channel();
            let _ = sender.send(None);
            return Ok(CallbackServer {
                receiver: Some(receiver),
                cancel_sender: None,
                handle: None,
            });
        }
        Err(error) => return Err(format!("Failed to bind OAuth callback server: {error}")),
    };

    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Failed to configure OAuth callback server: {error}"))?;

    let (result_sender, result_receiver) = oneshot::channel();
    let (cancel_sender, cancel_receiver) = mpsc::channel();

    let handle = thread::spawn(move || {
        let mut result_sender = Some(result_sender);
        loop {
            if cancel_receiver.try_recv().is_ok() {
                if let Some(sender) = result_sender.take() {
                    let _ = sender.send(None);
                }
                return;
            }

            match listener.accept() {
                Ok((mut stream, _)) => {
                    let target = match read_http_request_target(&mut stream) {
                        Ok(target) => target,
                        Err(error) => {
                            let _ = write_http_response(
                                &mut stream,
                                500,
                                &oauth_error_html(
                                    "Internal error while processing OAuth callback.",
                                    Some(&error),
                                ),
                            );
                            continue;
                        }
                    };

                    let url = match Url::parse(&format!("http://localhost{target}")) {
                        Ok(url) => url,
                        Err(error) => {
                            let _ = write_http_response(
                                &mut stream,
                                500,
                                &oauth_error_html(
                                    "Internal error while processing OAuth callback.",
                                    Some(&error.to_string()),
                                ),
                            );
                            continue;
                        }
                    };

                    match handler(&url) {
                        CallbackHandlerResult::Continue { status, body } => {
                            let _ = write_http_response(&mut stream, status, &body);
                        }
                        CallbackHandlerResult::Complete {
                            status,
                            body,
                            authorization,
                        } => {
                            let _ = write_http_response(&mut stream, status, &body);
                            if let Some(sender) = result_sender.take() {
                                let _ = sender.send(Some(authorization));
                            }
                            return;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(CALLBACK_POLL_INTERVAL);
                }
                Err(_) => {
                    if let Some(sender) = result_sender.take() {
                        let _ = sender.send(None);
                    }
                    return;
                }
            }
        }
    });

    Ok(CallbackServer {
        receiver: Some(result_receiver),
        cancel_sender: Some(cancel_sender),
        handle: Some(handle),
    })
}

fn read_http_request_target(stream: &mut std::net::TcpStream) -> Result<String, String> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 2048];

    loop {
        let bytes_read = stream
            .read(&mut buffer)
            .map_err(|error| format!("Failed to read callback request: {error}"))?;
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() > 16 * 1024 {
            return Err("OAuth callback request was too large".into());
        }
    }

    let request = String::from_utf8(request)
        .map_err(|error| format!("OAuth callback request was not valid UTF-8: {error}"))?;
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "OAuth callback request was missing a request line".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    if method != "GET" {
        return Err(format!("Unsupported callback request method: {method}"));
    }
    let target = parts
        .next()
        .ok_or_else(|| "OAuth callback request was missing a path".to_string())?;
    Ok(target.to_string())
}

fn write_http_response(
    stream: &mut std::net::TcpStream,
    status: u16,
    body: &str,
) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("Failed to write callback response: {error}"))
}

fn handle_anthropic_callback(url: &Url, expected_state: &str) -> CallbackHandlerResult {
    if url.path() != ANTHROPIC_CALLBACK_PATH {
        return CallbackHandlerResult::Continue {
            status: 404,
            body: oauth_error_html("Callback route not found.", None),
        };
    }

    if let Some(error) = url
        .query_pairs()
        .find_map(|(key, value)| (key == "error").then(|| value.into_owned()))
    {
        return CallbackHandlerResult::Continue {
            status: 400,
            body: oauth_error_html(
                "Anthropic authentication did not complete.",
                Some(&format!("Error: {error}")),
            ),
        };
    }

    let code = url
        .query_pairs()
        .find_map(|(key, value)| (key == "code").then(|| value.into_owned()));
    let state = url
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()));

    match (code, state) {
        (Some(code), Some(state)) if state == expected_state => CallbackHandlerResult::Complete {
            status: 200,
            body: oauth_success_html(
                "Anthropic authentication completed. You can close this window.",
            ),
            authorization: CallbackAuthorization {
                code,
                state: Some(state),
            },
        },
        (Some(_), Some(_)) => CallbackHandlerResult::Continue {
            status: 400,
            body: oauth_error_html("State mismatch.", None),
        },
        _ => CallbackHandlerResult::Continue {
            status: 400,
            body: oauth_error_html("Missing code or state parameter.", None),
        },
    }
}

fn handle_openai_codex_callback(url: &Url, expected_state: &str) -> CallbackHandlerResult {
    if url.path() != OPENAI_CODEX_CALLBACK_PATH {
        return CallbackHandlerResult::Continue {
            status: 404,
            body: oauth_error_html("Callback route not found.", None),
        };
    }

    let state = url
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()));
    if state.as_deref() != Some(expected_state) {
        return CallbackHandlerResult::Continue {
            status: 400,
            body: oauth_error_html("State mismatch.", None),
        };
    }

    let Some(code) = url
        .query_pairs()
        .find_map(|(key, value)| (key == "code").then(|| value.into_owned()))
    else {
        return CallbackHandlerResult::Continue {
            status: 400,
            body: oauth_error_html("Missing authorization code.", None),
        };
    };

    CallbackHandlerResult::Complete {
        status: 200,
        body: oauth_success_html("OpenAI authentication completed. You can close this window."),
        authorization: CallbackAuthorization { code, state: None },
    }
}

fn oauth_success_html(message: &str) -> String {
    render_oauth_page(
        "Authentication successful",
        "Authentication successful",
        message,
        None,
    )
}

fn oauth_error_html(message: &str, details: Option<&str>) -> String {
    render_oauth_page(
        "Authentication failed",
        "Authentication failed",
        message,
        details,
    )
}

fn render_oauth_page(title: &str, heading: &str, message: &str, details: Option<&str>) -> String {
    let title = escape_html(title);
    let heading = escape_html(heading);
    let message = escape_html(message);
    let details = details.map(escape_html);

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\" /><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" /><title>{title}</title><style>:root{{--text:#fafafa;--text-dim:#a1a1aa;--page-bg:#09090b;--font-sans:ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,\"Helvetica Neue\",Arial,sans-serif;--font-mono:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,\"Liberation Mono\",\"Courier New\",monospace;}}*{{box-sizing:border-box;}}html{{color-scheme:dark;}}body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:24px;background:var(--page-bg);color:var(--text);font-family:var(--font-sans);text-align:center;}}main{{width:100%;max-width:560px;display:flex;flex-direction:column;align-items:center;justify-content:center;}}h1{{margin:0 0 10px;font-size:28px;line-height:1.15;font-weight:650;color:var(--text);}}p{{margin:0;line-height:1.7;color:var(--text-dim);font-size:15px;}}.details{{margin-top:16px;font-family:var(--font-mono);font-size:13px;color:var(--text-dim);white-space:pre-wrap;word-break:break-word;}}</style></head><body><main><h1>{heading}</h1><p>{message}</p>{}</main></body></html>",
        details
            .map(|details| format!("<div class=\"details\">{details}</div>"))
            .unwrap_or_default()
    )
}

fn escape_html(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn extract_openai_codex_account_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = decode_base64_token_component(payload)?;
    let json = serde_json::from_slice::<Value>(&decoded).ok()?;
    json.get(OPENAI_CODEX_AUTH_CLAIM)?
        .get("chatgpt_account_id")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn decode_base64_token_component(input: &str) -> Option<Vec<u8>> {
    let mut normalized = input.replace('-', "+").replace('_', "/");
    while normalized.len() % 4 != 0 {
        normalized.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(normalized)
        .ok()
}

fn ensure_not_aborted(callbacks: &OAuthLoginCallbacks) -> Result<(), String> {
    if callbacks.is_aborted() {
        Err("OAuth login was aborted".into())
    } else {
        Ok(())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Deserialize)]
struct AnthropicTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct OpenAiCodexTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

enum ManualOrCallback {
    Manual(Option<Result<String, String>>),
    Callback(Result<Option<CallbackAuthorization>, String>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{sync::OnceLock, thread};

    fn registry_lock() -> &'static TokioMutex<()> {
        static LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| TokioMutex::new(()))
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
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
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

    #[test]
    fn parses_authorization_input_from_url_query_fragment_and_raw_code() {
        assert_eq!(
            parse_authorization_input("https://example.com/callback?code=abc&state=xyz"),
            ParsedAuthorizationInput {
                code: Some("abc".into()),
                state: Some("xyz".into()),
            }
        );
        assert_eq!(
            parse_authorization_input("abc#xyz"),
            ParsedAuthorizationInput {
                code: Some("abc".into()),
                state: Some("xyz".into()),
            }
        );
        assert_eq!(
            parse_authorization_input("code=abc&state=xyz"),
            ParsedAuthorizationInput {
                code: Some("abc".into()),
                state: Some("xyz".into()),
            }
        );
        assert_eq!(
            parse_authorization_input("abc"),
            ParsedAuthorizationInput {
                code: Some("abc".into()),
                state: None,
            }
        );
    }

    #[tokio::test]
    async fn built_in_registry_exposes_anthropic_and_openai_codex() {
        let _guard = registry_lock().lock().await;
        reset_oauth_providers();

        let providers = get_oauth_provider_info_list();
        assert_eq!(
            providers,
            vec![
                OAuthProviderInfo {
                    id: "anthropic".into(),
                    name: "Anthropic (Claude Pro/Max)".into(),
                    available: true,
                },
                OAuthProviderInfo {
                    id: "openai-codex".into(),
                    name: "ChatGPT Plus/Pro (Codex Subscription)".into(),
                    available: true,
                },
            ]
        );
    }

    struct TestOAuthProvider {
        id: String,
        name: String,
        refreshed_access: String,
    }

    impl OAuthProvider for TestOAuthProvider {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn login<'a>(&'a self, _callbacks: OAuthLoginCallbacks) -> OAuthCredentialsFuture<'a> {
            Box::pin(async { Err("not implemented".into()) })
        }

        fn refresh_token<'a>(
            &'a self,
            credentials: OAuthCredentials,
        ) -> OAuthCredentialsFuture<'a> {
            Box::pin(async move {
                Ok(OAuthCredentials::new(
                    credentials.refresh,
                    self.refreshed_access.clone(),
                    now_ms() + 3_600_000,
                ))
            })
        }

        fn get_api_key(&self, credentials: &OAuthCredentials) -> Result<String, String> {
            Ok(credentials.access.clone())
        }
    }

    #[tokio::test]
    async fn get_oauth_api_key_refreshes_expired_credentials() {
        let _guard = registry_lock().lock().await;
        reset_oauth_providers();

        let provider_id = "test-refresh-provider";
        register_oauth_provider(Arc::new(TestOAuthProvider {
            id: provider_id.into(),
            name: "Test Provider".into(),
            refreshed_access: "refreshed-access".into(),
        }));

        let credentials = std::collections::BTreeMap::from([(
            provider_id.to_string(),
            OAuthCredentials::new("refresh-token", "expired-access", 0),
        )]);

        let result = get_oauth_api_key(provider_id, &credentials)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.api_key, "refreshed-access");
        assert_eq!(result.new_credentials.access, "refreshed-access");
        assert!(result.new_credentials.expires > now_ms());

        unregister_oauth_provider(provider_id);
    }

    #[tokio::test]
    async fn refresh_oauth_token_with_overrides_refreshes_anthropic_credentials() {
        let (token_url, server) = spawn_single_response_server(
            |request| {
                let request_lower = request.to_ascii_lowercase();
                assert!(request_lower.starts_with("post /token http/1.1"));
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
            },
            json!({
                "access_token": "new-access-token",
                "refresh_token": "new-refresh-token",
                "expires_in": 3600,
            })
            .to_string(),
            "application/json",
        );

        let refreshed = refresh_oauth_token_with_overrides(
            "anthropic",
            &OAuthCredentials::new("refresh-token", "expired-access-token", 0),
            &OAuthRefreshOverrides {
                anthropic_token_url: Some(&token_url),
                ..OAuthRefreshOverrides::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(refreshed.access, "new-access-token");
        assert_eq!(refreshed.refresh, "new-refresh-token");
        assert!(refreshed.expires > now_ms());

        server.join().unwrap();
    }

    #[tokio::test]
    async fn refresh_oauth_token_with_overrides_refreshes_openai_codex_credentials() {
        let (token_url, server) = spawn_single_response_server(
            |request| {
                let request_lower = request.to_ascii_lowercase();
                assert!(request_lower.starts_with("post /token http/1.1"));
                let body = request_body(request);
                assert!(body.contains("grant_type=refresh_token"));
                assert!(body.contains("refresh_token=refresh-token"));
                assert!(body.contains(&format!("client_id={OPENAI_CODEX_CLIENT_ID}")));
            },
            json!({
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

        let refreshed = refresh_oauth_token_with_overrides(
            "openai-codex",
            &OAuthCredentials::new("refresh-token", "expired-access-token", 0),
            &OAuthRefreshOverrides {
                openai_codex_token_url: Some(&token_url),
                ..OAuthRefreshOverrides::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(refreshed.refresh, "new-refresh-token");
        assert_eq!(refreshed.get_string("accountId"), Some("acc_test"));
        assert!(refreshed.expires > now_ms());

        server.join().unwrap();
    }
}
