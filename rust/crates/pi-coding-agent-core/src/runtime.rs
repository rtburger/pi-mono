use crate::{
    AuthSource, SessionEntry, SessionManager, bootstrap::BootstrapDiagnostic,
    bootstrap::ExistingSessionSelection, bootstrap::SessionBootstrapOptions, bootstrap_session,
    convert_to_llm, filter_blocked_images, model_resolver::parse_thinking_level,
};
use async_stream::stream;
use futures::{StreamExt, future::BoxFuture};
use pi_agent::{
    Agent, AgentEvent, AgentMessage, AgentState, AgentTool, AgentUnsubscribe, AssistantStreamer,
    ThinkingLevel,
};
use pi_ai::{
    AiError, AssistantEventStream, SimpleStreamOptions, StreamOptions,
    ThinkingLevel as AiThinkingLevel, stream_simple,
};
use pi_coding_agent_tools::create_coding_tools_with_read_auto_resize_flag;
use pi_events::{Context, Message, Model};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct CodingAgentCoreOptions {
    pub auth_source: Arc<dyn AuthSource>,
    pub built_in_models: Vec<Model>,
    pub models_json_path: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub tools: Option<Vec<AgentTool>>,
    pub system_prompt: String,
    pub bootstrap: SessionBootstrapOptions,
    pub stream_options: StreamOptions,
}

pub struct CreateCodingAgentCoreResult {
    pub core: CodingAgentCore,
    pub diagnostics: Vec<BootstrapDiagnostic>,
    pub model_fallback_message: Option<String>,
}

pub struct AgentSessionOptions {
    pub core: CodingAgentCoreOptions,
    pub session_manager: Option<Arc<Mutex<SessionManager>>>,
}

impl From<CodingAgentCoreOptions> for AgentSessionOptions {
    fn from(core: CodingAgentCoreOptions) -> Self {
        Self {
            core,
            session_manager: None,
        }
    }
}

pub struct CreateAgentSessionResult {
    pub session: AgentSession,
    pub diagnostics: Vec<BootstrapDiagnostic>,
    pub model_fallback_message: Option<String>,
}

#[derive(Clone)]
pub struct CodingAgentCore {
    agent: Agent,
    model_registry: Arc<crate::ModelRegistry>,
    auto_resize_images: Arc<AtomicBool>,
    block_images: Arc<AtomicBool>,
    thinking_budgets: Arc<Mutex<pi_ai::ThinkingBudgets>>,
}

impl CodingAgentCore {
    pub fn agent(&self) -> Agent {
        self.agent.clone()
    }

    pub fn model_registry(&self) -> Arc<crate::ModelRegistry> {
        self.model_registry.clone()
    }

    pub fn state(&self) -> AgentState {
        self.agent.state()
    }

    pub fn auto_resize_images(&self) -> bool {
        self.auto_resize_images.load(Ordering::Relaxed)
    }

    pub fn set_auto_resize_images(&self, enabled: bool) {
        self.auto_resize_images.store(enabled, Ordering::Relaxed);
    }

    pub fn block_images(&self) -> bool {
        self.block_images.load(Ordering::Relaxed)
    }

    pub fn set_block_images(&self, blocked: bool) {
        self.block_images.store(blocked, Ordering::Relaxed);
    }

    pub fn thinking_budgets(&self) -> pi_ai::ThinkingBudgets {
        self.thinking_budgets.lock().unwrap().clone()
    }

    pub fn set_thinking_budgets(&self, thinking_budgets: pi_ai::ThinkingBudgets) {
        self.agent.set_thinking_budgets(thinking_budgets.clone());
        *self.thinking_budgets.lock().unwrap() = thinking_budgets;
    }

    pub async fn prompt_text(
        &self,
        text: impl Into<String>,
    ) -> Result<(), crate::CodingAgentCoreError> {
        self.agent.prompt_text(text).await.map_err(Into::into)
    }

    pub async fn prompt_message(
        &self,
        message: Message,
    ) -> Result<(), crate::CodingAgentCoreError> {
        self.agent.prompt(message).await.map_err(Into::into)
    }

    pub async fn continue_turn(&self) -> Result<(), crate::CodingAgentCoreError> {
        self.agent.r#continue().await.map_err(Into::into)
    }

    pub fn abort(&self) {
        self.agent.abort();
    }

    pub async fn wait_for_idle(&self) {
        self.agent.wait_for_idle().await;
    }
}

struct SessionPersistenceSubscription {
    unsubscribe: Mutex<Option<AgentUnsubscribe>>,
}

impl SessionPersistenceSubscription {
    fn new(unsubscribe: AgentUnsubscribe) -> Self {
        Self {
            unsubscribe: Mutex::new(Some(unsubscribe)),
        }
    }
}

impl Drop for SessionPersistenceSubscription {
    fn drop(&mut self) {
        if let Some(unsubscribe) = self.unsubscribe.lock().unwrap().take() {
            let _ = unsubscribe();
        }
    }
}

struct AgentSessionInner {
    core: CodingAgentCore,
    session_manager: Option<Arc<Mutex<SessionManager>>>,
    _session_persistence: Option<Arc<SessionPersistenceSubscription>>,
}

#[derive(Clone)]
pub struct AgentSession {
    inner: Arc<AgentSessionInner>,
}

impl AgentSession {
    fn new(
        core: CodingAgentCore,
        session_manager: Option<Arc<Mutex<SessionManager>>>,
    ) -> Result<Self, crate::CodingAgentCoreError> {
        let session_persistence = if let Some(session_manager) = session_manager.as_ref() {
            let restore = {
                let session_manager = session_manager
                    .lock()
                    .expect("session manager mutex poisoned");
                build_session_restore_state(&session_manager)
            };

            core.agent()
                .set_session_id(Some(restore.session_id.clone()));

            if !restore.restored_messages.is_empty() {
                let restored_messages = restore.restored_messages.clone();
                core.agent().update_state(move |state| {
                    state.messages = restored_messages;
                });
            }

            let state = core.state();
            {
                let mut session_manager = session_manager
                    .lock()
                    .expect("session manager mutex poisoned");
                if restore.has_existing_messages {
                    if !restore.has_thinking_entry {
                        session_manager
                            .append_thinking_level_change(thinking_level_label(
                                state.thinking_level,
                            ))
                            .map_err(|error| {
                                crate::CodingAgentCoreError::Message(error.to_string())
                            })?;
                    }
                } else {
                    session_manager
                        .append_model_change(state.model.provider.clone(), state.model.id.clone())
                        .map_err(|error| crate::CodingAgentCoreError::Message(error.to_string()))?;
                    session_manager
                        .append_thinking_level_change(thinking_level_label(state.thinking_level))
                        .map_err(|error| crate::CodingAgentCoreError::Message(error.to_string()))?;
                }
            }

            let session_manager = session_manager.clone();
            let unsubscribe = core.agent().subscribe(move |event, _signal| {
                let session_manager = session_manager.clone();
                Box::pin(async move {
                    if let AgentEvent::MessageEnd { message } = event {
                        let _ = session_manager
                            .lock()
                            .expect("session manager mutex poisoned")
                            .append_message(message);
                    }
                })
            });

            Some(Arc::new(SessionPersistenceSubscription::new(unsubscribe)))
        } else {
            None
        };

        Ok(Self {
            inner: Arc::new(AgentSessionInner {
                core,
                session_manager,
                _session_persistence: session_persistence,
            }),
        })
    }

    pub fn core(&self) -> CodingAgentCore {
        self.inner.core.clone()
    }

    pub fn agent(&self) -> Agent {
        self.inner.core.agent()
    }

    pub fn model_registry(&self) -> Arc<crate::ModelRegistry> {
        self.inner.core.model_registry()
    }

    pub fn session_manager(&self) -> Option<Arc<Mutex<SessionManager>>> {
        self.inner.session_manager.clone()
    }

    pub fn state(&self) -> AgentState {
        self.inner.core.state()
    }

    pub fn session_id(&self) -> Option<String> {
        self.agent().session_id()
    }

    pub fn session_file(&self) -> Option<String> {
        self.session_manager().and_then(|session_manager| {
            session_manager
                .lock()
                .expect("session manager mutex poisoned")
                .get_session_file()
                .map(str::to_owned)
        })
    }

    pub fn auto_resize_images(&self) -> bool {
        self.inner.core.auto_resize_images()
    }

    pub fn set_auto_resize_images(&self, enabled: bool) {
        self.inner.core.set_auto_resize_images(enabled);
    }

    pub fn block_images(&self) -> bool {
        self.inner.core.block_images()
    }

    pub fn set_block_images(&self, blocked: bool) {
        self.inner.core.set_block_images(blocked);
    }

    pub fn thinking_budgets(&self) -> pi_ai::ThinkingBudgets {
        self.inner.core.thinking_budgets()
    }

    pub fn set_thinking_budgets(&self, thinking_budgets: pi_ai::ThinkingBudgets) {
        self.inner.core.set_thinking_budgets(thinking_budgets);
    }

    pub async fn prompt_text(
        &self,
        text: impl Into<String>,
    ) -> Result<(), crate::CodingAgentCoreError> {
        self.inner.core.prompt_text(text).await
    }

    pub async fn prompt_message(
        &self,
        message: Message,
    ) -> Result<(), crate::CodingAgentCoreError> {
        self.inner.core.prompt_message(message).await
    }

    pub async fn continue_turn(&self) -> Result<(), crate::CodingAgentCoreError> {
        self.inner.core.continue_turn().await
    }

    pub fn abort(&self) {
        self.inner.core.abort();
    }

    pub async fn wait_for_idle(&self) {
        self.inner.core.wait_for_idle().await;
    }
}

pub fn create_agent_session(
    options: AgentSessionOptions,
) -> Result<CreateAgentSessionResult, crate::CodingAgentCoreError> {
    let AgentSessionOptions {
        mut core,
        session_manager,
    } = options;

    if let Some(session_manager) = session_manager.as_ref()
        && core.bootstrap.existing_session == ExistingSessionSelection::default()
    {
        let existing_session = {
            let session_manager = session_manager
                .lock()
                .expect("session manager mutex poisoned");
            build_existing_session_selection(&session_manager)
        };
        core.bootstrap.existing_session = existing_session;
    }

    let created = create_coding_agent_core(core)?;
    let session = AgentSession::new(created.core, session_manager)?;
    Ok(CreateAgentSessionResult {
        session,
        diagnostics: created.diagnostics,
        model_fallback_message: created.model_fallback_message,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum AgentSessionRuntimeError {
    #[error(transparent)]
    Core(#[from] crate::CodingAgentCoreError),
    #[error(transparent)]
    SessionManager(#[from] crate::SessionManagerError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Clone)]
pub struct AgentSessionRuntimeRequest {
    pub cwd: PathBuf,
    pub session_manager: Option<Arc<Mutex<SessionManager>>>,
}

pub type CreateAgentSessionRuntimeFuture =
    BoxFuture<'static, Result<CreateAgentSessionResult, AgentSessionRuntimeError>>;
pub type CreateAgentSessionRuntimeFactory =
    Arc<dyn Fn(AgentSessionRuntimeRequest) -> CreateAgentSessionRuntimeFuture + Send + Sync>;

pub struct AgentSessionRuntime {
    session: AgentSession,
    cwd: PathBuf,
    diagnostics: Vec<BootstrapDiagnostic>,
    model_fallback_message: Option<String>,
    create_runtime: CreateAgentSessionRuntimeFactory,
}

impl AgentSessionRuntime {
    pub fn session(&self) -> AgentSession {
        self.session.clone()
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn diagnostics(&self) -> &[BootstrapDiagnostic] {
        &self.diagnostics
    }

    pub fn model_fallback_message(&self) -> Option<&str> {
        self.model_fallback_message.as_deref()
    }

    pub async fn switch_session(
        &mut self,
        session_path: &str,
        cwd_override: Option<&str>,
    ) -> Result<(), AgentSessionRuntimeError> {
        let manager = SessionManager::open(session_path, None, cwd_override)?;
        let cwd = PathBuf::from(manager.get_cwd());
        self.replace_runtime(cwd, Some(Arc::new(Mutex::new(manager))))
            .await
    }

    pub async fn new_session(
        &mut self,
        options: crate::NewSessionOptions,
    ) -> Result<(), AgentSessionRuntimeError> {
        let session_manager = self.ensure_runtime_session_manager();
        let cwd = {
            let mut session_manager = session_manager
                .lock()
                .expect("session manager mutex poisoned");
            session_manager.new_session(options);
            PathBuf::from(session_manager.get_cwd())
        };
        self.replace_runtime(cwd, Some(session_manager)).await
    }

    pub async fn fork(
        &mut self,
        entry_id: &str,
    ) -> Result<Option<String>, AgentSessionRuntimeError> {
        let session_manager = self.ensure_runtime_session_manager();
        let (selected_text, cwd) = {
            let mut session_manager = session_manager
                .lock()
                .expect("session manager mutex poisoned");
            let selected_entry = session_manager
                .get_entry(entry_id)
                .cloned()
                .ok_or_else(|| {
                    AgentSessionRuntimeError::Message(String::from("Invalid entry ID for forking"))
                })?;
            let SessionEntry::Message {
                message, parent_id, ..
            } = selected_entry
            else {
                return Err(AgentSessionRuntimeError::Message(String::from(
                    "Invalid entry ID for forking",
                )));
            };
            let selected_text = extract_user_message_text(&message).ok_or_else(|| {
                AgentSessionRuntimeError::Message(String::from("Invalid entry ID for forking"))
            })?;

            if let Some(parent_id) = parent_id.as_deref() {
                session_manager.create_branched_session(parent_id)?;
            } else {
                let parent_session = session_manager.get_session_file().map(ToOwned::to_owned);
                session_manager.new_session(crate::NewSessionOptions {
                    id: None,
                    parent_session,
                });
            }

            (selected_text, PathBuf::from(session_manager.get_cwd()))
        };

        self.replace_runtime(cwd, Some(session_manager)).await?;
        Ok(Some(selected_text))
    }

    pub async fn import_from_jsonl(
        &mut self,
        input_path: &str,
        cwd_override: Option<&str>,
    ) -> Result<(), AgentSessionRuntimeError> {
        let resolved_path = resolve_runtime_path(&self.cwd, input_path);
        if !resolved_path.exists() {
            return Err(AgentSessionRuntimeError::Message(format!(
                "File not found: {}",
                resolved_path.display()
            )));
        }

        let session_manager = if let Some(current_manager) = self.session.session_manager() {
            let session_dir = {
                let current_manager = current_manager
                    .lock()
                    .expect("session manager mutex poisoned");
                (!current_manager.get_session_dir().is_empty())
                    .then(|| current_manager.get_session_dir().to_owned())
            };
            if let Some(session_dir) = session_dir {
                fs::create_dir_all(&session_dir)?;
                let destination_path =
                    Path::new(&session_dir).join(resolved_path.file_name().ok_or_else(|| {
                        AgentSessionRuntimeError::Message(String::from("Invalid import file path"))
                    })?);
                if destination_path != resolved_path {
                    fs::copy(&resolved_path, &destination_path)?;
                }
                Arc::new(Mutex::new(SessionManager::open(
                    destination_path.to_string_lossy().as_ref(),
                    Some(&session_dir),
                    cwd_override,
                )?))
            } else {
                Arc::new(Mutex::new(SessionManager::open(
                    resolved_path.to_string_lossy().as_ref(),
                    None,
                    cwd_override,
                )?))
            }
        } else {
            Arc::new(Mutex::new(SessionManager::open(
                resolved_path.to_string_lossy().as_ref(),
                None,
                cwd_override,
            )?))
        };

        let cwd = {
            let session_manager = session_manager
                .lock()
                .expect("session manager mutex poisoned");
            PathBuf::from(session_manager.get_cwd())
        };
        self.replace_runtime(cwd, Some(session_manager)).await
    }

    pub fn dispose(self) {}

    fn ensure_runtime_session_manager(&self) -> Arc<Mutex<SessionManager>> {
        self.session.session_manager().unwrap_or_else(|| {
            Arc::new(Mutex::new(SessionManager::in_memory(
                &self.cwd.to_string_lossy(),
            )))
        })
    }

    async fn replace_runtime(
        &mut self,
        cwd: PathBuf,
        session_manager: Option<Arc<Mutex<SessionManager>>>,
    ) -> Result<(), AgentSessionRuntimeError> {
        let result = (self.create_runtime)(AgentSessionRuntimeRequest {
            cwd: cwd.clone(),
            session_manager,
        })
        .await?;

        self.session = result.session;
        self.cwd = current_runtime_cwd(&self.session, &cwd);
        self.diagnostics = result.diagnostics;
        self.model_fallback_message = result.model_fallback_message;
        Ok(())
    }
}

pub async fn create_agent_session_runtime(
    create_runtime: CreateAgentSessionRuntimeFactory,
    request: AgentSessionRuntimeRequest,
) -> Result<AgentSessionRuntime, AgentSessionRuntimeError> {
    let fallback_cwd = request.cwd.clone();
    let result = create_runtime(request).await?;
    let cwd = current_runtime_cwd(&result.session, &fallback_cwd);
    Ok(AgentSessionRuntime {
        session: result.session,
        cwd,
        diagnostics: result.diagnostics,
        model_fallback_message: result.model_fallback_message,
        create_runtime,
    })
}

pub fn create_coding_agent_core(
    options: CodingAgentCoreOptions,
) -> Result<CreateCodingAgentCoreResult, crate::CodingAgentCoreError> {
    let CodingAgentCoreOptions {
        auth_source,
        built_in_models,
        models_json_path,
        cwd,
        tools,
        system_prompt,
        bootstrap,
        stream_options,
    } = options;

    let model_registry = Arc::new(crate::ModelRegistry::new(
        auth_source,
        built_in_models,
        models_json_path,
    ));
    let bootstrap = bootstrap_session(&model_registry, bootstrap);

    let Some(model) = bootstrap.model.clone() else {
        return Err(crate::CodingAgentCoreError::NoModelAvailable);
    };

    let cwd = match cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()
            .map_err(|error| crate::CodingAgentCoreError::Message(error.to_string()))?,
    };

    let auto_resize_images = Arc::new(AtomicBool::new(true));
    let thinking_budgets = Arc::new(Mutex::new(pi_ai::ThinkingBudgets::default()));

    let mut state = AgentState::new(model);
    state.system_prompt = system_prompt;
    state.thinking_level = bootstrap.thinking_level;
    state.tools = tools.unwrap_or_else(|| {
        create_coding_tools_with_read_auto_resize_flag(cwd, auto_resize_images.clone())
    });

    let agent = Agent::with_parts(
        state,
        Arc::new(RegistryBackedStreamer {
            model_registry: model_registry.clone(),
            thinking_budgets: thinking_budgets.clone(),
        }),
        stream_options,
    );
    let block_images = Arc::new(AtomicBool::new(false));
    let convert_block_images = block_images.clone();
    agent.set_convert_to_llm(move |messages| {
        let convert_block_images = convert_block_images.clone();
        async move {
            let converted = convert_to_llm(messages);
            if convert_block_images.load(Ordering::Relaxed) {
                filter_blocked_images(converted)
            } else {
                converted
            }
        }
    });

    Ok(CreateCodingAgentCoreResult {
        core: CodingAgentCore {
            agent,
            model_registry,
            auto_resize_images,
            block_images,
            thinking_budgets,
        },
        diagnostics: bootstrap.diagnostics,
        model_fallback_message: bootstrap.model_fallback_message,
    })
}

struct RegistryBackedStreamer {
    model_registry: Arc<crate::ModelRegistry>,
    thinking_budgets: Arc<Mutex<pi_ai::ThinkingBudgets>>,
}

impl AssistantStreamer for RegistryBackedStreamer {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError> {
        let model_registry = self.model_registry.clone();
        let thinking_budgets = self.thinking_budgets.clone();
        Ok(Box::pin(stream! {
            let auth = match model_registry.get_api_key_and_headers_async(&model).await {
                Ok(auth) => auth,
                Err(error) => {
                    yield Err(AiError::Message(error));
                    return;
                }
            };

            let mut stream_options = options;
            stream_options.api_key = auth.api_key;
            if auth.headers.is_some() || !stream_options.headers.is_empty() {
                let mut merged_headers = auth.headers.unwrap_or_default();
                merged_headers.extend(stream_options.headers);
                stream_options.headers = merged_headers;
            }

            let thinking_budgets = thinking_budgets.lock().unwrap().clone();
            match stream_simple(model, context, map_stream_options_to_simple_options(stream_options, thinking_budgets)) {
                Ok(mut inner) => {
                    while let Some(event) = inner.next().await {
                        yield event;
                    }
                }
                Err(error) => {
                    yield Err(error);
                }
            }
        }))
    }
}

fn map_stream_options_to_simple_options(
    options: StreamOptions,
    thinking_budgets: pi_ai::ThinkingBudgets,
) -> SimpleStreamOptions {
    let reasoning = options
        .reasoning_effort
        .as_deref()
        .and_then(parse_ai_thinking_level);

    SimpleStreamOptions {
        signal: options.signal,
        session_id: options.session_id,
        cache_retention: options.cache_retention,
        api_key: options.api_key,
        transport: options.transport,
        headers: options.headers,
        metadata: options.metadata,
        on_payload: options.on_payload,
        max_retry_delay_ms: options.max_retry_delay_ms,
        temperature: options.temperature,
        max_tokens: options.max_tokens,
        reasoning,
        thinking_budgets,
        tool_choice: options.tool_choice,
        service_tier: options.service_tier,
    }
}

fn parse_ai_thinking_level(value: &str) -> Option<AiThinkingLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "minimal" => Some(AiThinkingLevel::Minimal),
        "low" => Some(AiThinkingLevel::Low),
        "medium" => Some(AiThinkingLevel::Medium),
        "high" => Some(AiThinkingLevel::High),
        "xhigh" => Some(AiThinkingLevel::Xhigh),
        _ => None,
    }
}

#[derive(Clone)]
struct SessionRestoreState {
    restored_messages: Vec<AgentMessage>,
    has_existing_messages: bool,
    has_thinking_entry: bool,
    existing_session: ExistingSessionSelection,
    session_id: String,
}

fn build_existing_session_selection(session_manager: &SessionManager) -> ExistingSessionSelection {
    build_session_restore_state(session_manager).existing_session
}

fn build_session_restore_state(session_manager: &SessionManager) -> SessionRestoreState {
    let restored_context = session_manager.build_session_context();
    let has_existing_messages = !restored_context.messages.is_empty();
    let has_thinking_entry = session_manager
        .get_branch(session_manager.get_leaf_id())
        .iter()
        .any(|entry| matches!(entry, SessionEntry::ThinkingLevelChange { .. }));

    SessionRestoreState {
        restored_messages: restored_context.messages,
        has_existing_messages,
        has_thinking_entry,
        existing_session: ExistingSessionSelection {
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
        },
        session_id: session_manager.get_session_id().to_owned(),
    }
}

fn current_runtime_cwd(session: &AgentSession, fallback_cwd: &Path) -> PathBuf {
    session
        .session_manager()
        .map(|session_manager| {
            PathBuf::from(
                session_manager
                    .lock()
                    .expect("session manager mutex poisoned")
                    .get_cwd(),
            )
        })
        .unwrap_or_else(|| fallback_cwd.to_path_buf())
}

fn resolve_runtime_path(base: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn extract_user_message_text(message: &AgentMessage) -> Option<String> {
    let Message::User { content, .. } = message.as_standard_message()? else {
        return None;
    };
    let text = content
        .iter()
        .filter_map(|content| match content {
            pi_events::UserContent::Text { text } => Some(text.as_str()),
            pi_events::UserContent::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
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
