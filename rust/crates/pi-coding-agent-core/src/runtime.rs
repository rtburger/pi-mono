use crate::{
    AuthSource, CompactionResult, CompactionSettings, SessionEntry, SessionManager,
    bootstrap::BootstrapDiagnostic, bootstrap::ExistingSessionSelection,
    bootstrap::SessionBootstrapOptions, bootstrap_session, calculate_context_tokens,
    compact as run_compaction, convert_to_llm, estimate_context_tokens, filter_blocked_images,
    latest_compaction_timestamp, model_resolver::parse_thinking_level, prepare_compaction,
    should_compact,
};
use async_stream::stream;
use futures::{StreamExt, future::BoxFuture};
use pi_agent::{
    Agent, AgentEvent, AgentMessage, AgentState, AgentTool, AgentUnsubscribe, AssistantStreamer,
    ThinkingLevel,
};
use pi_ai::{
    AiError, AssistantEventStream, SimpleStreamOptions, StreamOptions,
    ThinkingLevel as AiThinkingLevel, is_context_overflow, stream_simple,
};
use pi_coding_agent_tools::create_coding_tools_with_read_auto_resize_flag;
use pi_events::{AssistantMessage, Context, Message, Model, StopReason};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, time::sleep};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionReason {
    Manual,
    Threshold,
    Overflow,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentSessionEvent {
    Agent(AgentEvent),
    QueueUpdate {
        steering: Vec<String>,
        follow_up: Vec<String>,
    },
    CompactionStart {
        reason: CompactionReason,
    },
    CompactionEnd {
        reason: CompactionReason,
        result: Option<CompactionResult>,
        aborted: bool,
        will_retry: bool,
        error_message: Option<String>,
    },
    AutoRetryStart {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error_message: String,
    },
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        final_error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrySettings {
    pub enabled: bool,
    pub max_retries: u32,
    pub base_delay_ms: u64,
}

impl Default for RetrySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 3,
            base_delay_ms: 2_000,
        }
    }
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

type SessionEventListener = Arc<dyn Fn(AgentSessionEvent) + Send + Sync>;

#[derive(Debug)]
struct SessionAutomationState {
    compaction_settings: CompactionSettings,
    retry_settings: RetrySettings,
    last_assistant_message: Option<AssistantMessage>,
    overflow_recovery_attempted: bool,
    retry_attempt: u32,
    retry_done_tx: Option<watch::Sender<bool>>,
    retry_cancel_tx: Option<watch::Sender<bool>>,
}

#[derive(Debug, Default, Clone)]
struct SessionQueueState {
    steering: Vec<String>,
    follow_up: Vec<String>,
}

impl Default for SessionAutomationState {
    fn default() -> Self {
        Self {
            compaction_settings: CompactionSettings::default(),
            retry_settings: RetrySettings::default(),
            last_assistant_message: None,
            overflow_recovery_attempted: false,
            retry_attempt: 0,
            retry_done_tx: None,
            retry_cancel_tx: None,
        }
    }
}

struct AgentSessionInner {
    core: CodingAgentCore,
    session_manager: Option<Arc<Mutex<SessionManager>>>,
    session_event_listeners: Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    next_session_listener_id: AtomicUsize,
    automation: Arc<Mutex<SessionAutomationState>>,
    queue_state: Arc<Mutex<SessionQueueState>>,
    _agent_subscription: Arc<SessionPersistenceSubscription>,
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
        if let Some(session_manager) = session_manager.as_ref() {
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
            let mut session_manager = session_manager
                .lock()
                .expect("session manager mutex poisoned");
            if restore.has_existing_messages {
                if !restore.has_thinking_entry {
                    session_manager
                        .append_thinking_level_change(thinking_level_label(state.thinking_level))
                        .map_err(|error| crate::CodingAgentCoreError::Message(error.to_string()))?;
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

        let session_event_listeners = Arc::new(Mutex::new(BTreeMap::new()));
        let automation = Arc::new(Mutex::new(SessionAutomationState::default()));
        let queue_state = Arc::new(Mutex::new(SessionQueueState::default()));
        let unsubscribe = core.agent().subscribe({
            let core = core.clone();
            let session_manager = session_manager.clone();
            let session_event_listeners = session_event_listeners.clone();
            let automation = automation.clone();
            let queue_state = queue_state.clone();
            move |event, _signal| {
                let core = core.clone();
                let session_manager = session_manager.clone();
                let session_event_listeners = session_event_listeners.clone();
                let automation = automation.clone();
                let queue_state = queue_state.clone();
                Box::pin(async move {
                    handle_agent_session_event(
                        core,
                        session_manager,
                        session_event_listeners,
                        automation,
                        queue_state,
                        event,
                    )
                    .await;
                })
            }
        });

        Ok(Self {
            inner: Arc::new(AgentSessionInner {
                core,
                session_manager,
                session_event_listeners,
                next_session_listener_id: AtomicUsize::new(1),
                automation,
                queue_state,
                _agent_subscription: Arc::new(SessionPersistenceSubscription::new(unsubscribe)),
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

    pub fn subscribe<F>(&self, listener: F) -> AgentUnsubscribe
    where
        F: Fn(AgentSessionEvent) + Send + Sync + 'static,
    {
        let id = self
            .inner
            .next_session_listener_id
            .fetch_add(1, Ordering::Relaxed);
        self.inner
            .session_event_listeners
            .lock()
            .unwrap()
            .insert(id, Arc::new(listener));
        let listeners = self.inner.session_event_listeners.clone();
        Box::new(move || listeners.lock().unwrap().remove(&id).is_some())
    }

    pub fn steer(&self, message: Message) {
        enqueue_session_queue_message(
            &self.inner.queue_state,
            &self.inner.session_event_listeners,
            &message,
            "steering",
        );
        self.agent().steer(message);
    }

    pub fn follow_up(&self, message: Message) {
        enqueue_session_queue_message(
            &self.inner.queue_state,
            &self.inner.session_event_listeners,
            &message,
            "follow_up",
        );
        self.agent().follow_up(message);
    }

    pub fn pending_message_count(&self) -> usize {
        let queue_state = self.inner.queue_state.lock().unwrap();
        queue_state.steering.len() + queue_state.follow_up.len()
    }

    pub fn pending_steering_messages(&self) -> Vec<String> {
        self.inner.queue_state.lock().unwrap().steering.clone()
    }

    pub fn pending_follow_up_messages(&self) -> Vec<String> {
        self.inner.queue_state.lock().unwrap().follow_up.clone()
    }

    pub fn retry_settings(&self) -> RetrySettings {
        self.inner.automation.lock().unwrap().retry_settings.clone()
    }

    pub fn set_retry_settings(&self, settings: RetrySettings) {
        self.inner.automation.lock().unwrap().retry_settings = settings;
    }

    pub fn compaction_settings(&self) -> CompactionSettings {
        self.inner
            .automation
            .lock()
            .unwrap()
            .compaction_settings
            .clone()
    }

    pub fn set_compaction_settings(&self, settings: CompactionSettings) {
        self.inner.automation.lock().unwrap().compaction_settings = settings;
    }

    pub fn is_retrying(&self) -> bool {
        self.inner
            .automation
            .lock()
            .unwrap()
            .retry_done_tx
            .as_ref()
            .is_some_and(|done_tx| {
                let receiver = done_tx.subscribe();
                !*receiver.borrow()
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
        let result = self.inner.core.prompt_text(text).await;
        if result.is_ok() {
            self.wait_for_retry().await;
        }
        result
    }

    pub async fn prompt_message(
        &self,
        message: Message,
    ) -> Result<(), crate::CodingAgentCoreError> {
        let result = self.inner.core.prompt_message(message).await;
        if result.is_ok() {
            self.wait_for_retry().await;
        }
        result
    }

    pub async fn continue_turn(&self) -> Result<(), crate::CodingAgentCoreError> {
        let result = self.inner.core.continue_turn().await;
        if result.is_ok() {
            self.wait_for_retry().await;
        }
        result
    }

    pub async fn compact(
        &self,
        custom_instructions: Option<&str>,
    ) -> Result<CompactionResult, crate::CodingAgentCoreError> {
        emit_session_event(
            &self.inner.session_event_listeners,
            AgentSessionEvent::CompactionStart {
                reason: CompactionReason::Manual,
            },
        );

        let result = self.run_manual_compaction(custom_instructions).await;
        match result {
            Ok(result) => {
                emit_session_event(
                    &self.inner.session_event_listeners,
                    AgentSessionEvent::CompactionEnd {
                        reason: CompactionReason::Manual,
                        result: Some(result.clone()),
                        aborted: false,
                        will_retry: false,
                        error_message: None,
                    },
                );
                Ok(result)
            }
            Err(error) => {
                emit_session_event(
                    &self.inner.session_event_listeners,
                    AgentSessionEvent::CompactionEnd {
                        reason: CompactionReason::Manual,
                        result: None,
                        aborted: false,
                        will_retry: false,
                        error_message: Some(format!("Compaction failed: {error}")),
                    },
                );
                Err(error)
            }
        }
    }

    pub fn abort_retry(&self) {
        if let Some(cancel_tx) = self
            .inner
            .automation
            .lock()
            .unwrap()
            .retry_cancel_tx
            .clone()
        {
            let _ = cancel_tx.send(true);
        }
    }

    pub fn abort(&self) {
        self.inner.core.abort();
    }

    pub async fn wait_for_idle(&self) {
        self.inner.core.wait_for_idle().await;
    }

    async fn wait_for_retry(&self) {
        let retry_done = self.inner.automation.lock().unwrap().retry_done_tx.clone();
        let Some(retry_done) = retry_done else {
            return;
        };

        let mut receiver = retry_done.subscribe();
        if !*receiver.borrow() {
            while receiver.changed().await.is_ok() {
                if *receiver.borrow() {
                    break;
                }
            }
        }

        self.agent().wait_for_idle().await;
    }

    async fn run_manual_compaction(
        &self,
        custom_instructions: Option<&str>,
    ) -> Result<CompactionResult, crate::CodingAgentCoreError> {
        let Some(session_manager) = self.session_manager() else {
            return Err(crate::CodingAgentCoreError::Message(String::from(
                "Session compaction is unavailable",
            )));
        };

        self.abort_retry();
        self.abort();
        self.wait_for_idle().await;

        let model = self.state().model;
        let auth = self
            .model_registry()
            .get_api_key_and_headers_async(&model)
            .await
            .map_err(crate::CodingAgentCoreError::Message)?;
        let Some(api_key) = auth.api_key else {
            return Err(crate::CodingAgentCoreError::Message(format!(
                "No API key found for {}.",
                model.provider
            )));
        };

        let path_entries = {
            let session_manager = session_manager
                .lock()
                .expect("session manager mutex poisoned");
            let leaf_id = session_manager.get_leaf_id().map(str::to_owned);
            session_manager.get_branch(leaf_id.as_deref())
        };

        let settings = self.compaction_settings();
        let Some(preparation) = prepare_compaction(&path_entries, settings) else {
            let message = match path_entries.last() {
                Some(SessionEntry::Compaction { .. }) => String::from("Already compacted"),
                _ => String::from("Nothing to compact (session too small)"),
            };
            return Err(crate::CodingAgentCoreError::Message(message));
        };

        let result = run_compaction(
            &preparation,
            &model,
            &api_key,
            auth.headers,
            custom_instructions,
        )
        .await
        .map_err(crate::CodingAgentCoreError::Message)?;

        rebuild_compacted_session(&self.inner.core, &session_manager, &result)
            .map_err(|error| crate::CodingAgentCoreError::Message(error.to_string()))?;

        Ok(result)
    }
}

async fn handle_agent_session_event(
    core: CodingAgentCore,
    session_manager: Option<Arc<Mutex<SessionManager>>>,
    listeners: Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    automation: Arc<Mutex<SessionAutomationState>>,
    queue_state: Arc<Mutex<SessionQueueState>>,
    event: AgentEvent,
) {
    if let AgentEvent::MessageStart { message } = &event
        && matches!(message.as_standard_message(), Some(Message::User { .. }))
    {
        automation.lock().unwrap().overflow_recovery_attempted = false;
        dequeue_session_queue_message(&queue_state, &listeners, message);
    }

    emit_session_event(&listeners, AgentSessionEvent::Agent(event.clone()));

    if let AgentEvent::MessageEnd { message } = &event {
        persist_session_message(session_manager.as_ref(), message);

        if let Some(assistant) = agent_message_to_assistant(message) {
            let mut completed_retry_attempt = None;
            {
                let mut automation = automation.lock().unwrap();
                automation.last_assistant_message = Some(assistant.clone());
                if assistant.stop_reason != StopReason::Error {
                    automation.overflow_recovery_attempted = false;
                }
                if assistant.stop_reason != StopReason::Error && automation.retry_attempt > 0 {
                    completed_retry_attempt = Some(automation.retry_attempt);
                    automation.retry_attempt = 0;
                    resolve_retry_cycle_locked(&mut automation);
                }
            }

            if let Some(attempt) = completed_retry_attempt {
                emit_session_event(
                    &listeners,
                    AgentSessionEvent::AutoRetryEnd {
                        success: true,
                        attempt,
                        final_error: None,
                    },
                );
            }
        }
    }

    if !matches!(event, AgentEvent::AgentEnd { .. }) {
        return;
    }

    let assistant = {
        let mut automation = automation.lock().unwrap();
        automation.last_assistant_message.take()
    };
    let Some(assistant) = assistant else {
        return;
    };

    if is_retryable_error(&assistant, Some(core.state().model.context_window)) {
        if handle_retryable_error(
            core.clone(),
            listeners.clone(),
            automation.clone(),
            assistant.clone(),
        )
        .await
        {
            return;
        }
    } else {
        resolve_retry_cycle_if_pending(&automation);
    }

    if let Some(session_manager) = session_manager {
        maybe_run_session_auto_compaction(core, session_manager, listeners, automation, assistant)
            .await;
    }
}

fn emit_session_event(
    listeners: &Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    event: AgentSessionEvent,
) {
    let callbacks = listeners
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for callback in callbacks {
        callback(event.clone());
    }
}

fn queue_update_event(queue_state: &SessionQueueState) -> AgentSessionEvent {
    AgentSessionEvent::QueueUpdate {
        steering: queue_state.steering.clone(),
        follow_up: queue_state.follow_up.clone(),
    }
}

fn enqueue_session_queue_message(
    queue_state: &Arc<Mutex<SessionQueueState>>,
    listeners: &Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    message: &Message,
    kind: &str,
) {
    let Some(text) = extract_user_text(message) else {
        return;
    };

    let event = {
        let mut queue_state = queue_state.lock().unwrap();
        if kind == "follow_up" {
            queue_state.follow_up.push(text);
        } else {
            queue_state.steering.push(text);
        }
        queue_update_event(&queue_state)
    };
    emit_session_event(listeners, event);
}

fn dequeue_session_queue_message(
    queue_state: &Arc<Mutex<SessionQueueState>>,
    listeners: &Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    message: &AgentMessage,
) {
    let Some(text) = extract_user_message_text(message) else {
        return;
    };

    let event = {
        let mut queue_state = queue_state.lock().unwrap();
        let removed = if let Some(index) =
            queue_state.steering.iter().position(|item| item == &text)
        {
            queue_state.steering.remove(index);
            true
        } else if let Some(index) = queue_state.follow_up.iter().position(|item| item == &text) {
            queue_state.follow_up.remove(index);
            true
        } else {
            false
        };

        removed.then(|| queue_update_event(&queue_state))
    };

    if let Some(event) = event {
        emit_session_event(listeners, event);
    }
}

fn persist_session_message(
    session_manager: Option<&Arc<Mutex<SessionManager>>>,
    message: &AgentMessage,
) {
    let Some(session_manager) = session_manager else {
        return;
    };

    match message.role() {
        "user" | "assistant" | "toolResult" => {
            let _ = session_manager
                .lock()
                .expect("session manager mutex poisoned")
                .append_message(message.clone());
        }
        _ => {}
    }
}

fn agent_message_to_assistant(message: &AgentMessage) -> Option<AssistantMessage> {
    let Message::Assistant {
        content,
        api,
        provider,
        model,
        response_id,
        usage,
        stop_reason,
        error_message,
        timestamp,
    } = message.as_standard_message()?
    else {
        return None;
    };

    Some(AssistantMessage {
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
    })
}

fn resolve_retry_cycle_if_pending(automation: &Arc<Mutex<SessionAutomationState>>) {
    let mut automation = automation.lock().unwrap();
    resolve_retry_cycle_locked(&mut automation);
}

fn resolve_retry_cycle_locked(automation: &mut SessionAutomationState) {
    automation.retry_cancel_tx = None;
    if let Some(done_tx) = automation.retry_done_tx.take() {
        let _ = done_tx.send(true);
    }
}

fn finish_retry_with_failure(
    listeners: &Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    automation: &Arc<Mutex<SessionAutomationState>>,
    attempt: u32,
    final_error: String,
) {
    {
        let mut automation = automation.lock().unwrap();
        automation.retry_attempt = 0;
        resolve_retry_cycle_locked(&mut automation);
    }

    emit_session_event(
        listeners,
        AgentSessionEvent::AutoRetryEnd {
            success: false,
            attempt,
            final_error: Some(final_error),
        },
    );
}

async fn handle_retryable_error(
    core: CodingAgentCore,
    listeners: Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    automation: Arc<Mutex<SessionAutomationState>>,
    message: AssistantMessage,
) -> bool {
    let settings = automation.lock().unwrap().retry_settings.clone();
    if !settings.enabled {
        resolve_retry_cycle_if_pending(&automation);
        return false;
    }

    let mut max_retry_failure = None;
    let (attempt, delay_ms, cancel_tx) = {
        let mut automation = automation.lock().unwrap();
        if automation.retry_done_tx.is_none() {
            let (done_tx, _) = watch::channel(false);
            automation.retry_done_tx = Some(done_tx);
        }

        automation.retry_attempt += 1;
        if automation.retry_attempt > settings.max_retries {
            let attempt = automation.retry_attempt.saturating_sub(1);
            automation.retry_attempt = 0;
            resolve_retry_cycle_locked(&mut automation);
            max_retry_failure = Some(attempt);
            (0, 0, None)
        } else {
            let attempt = automation.retry_attempt;
            let delay_ms = settings
                .base_delay_ms
                .saturating_mul(2_u64.saturating_pow(attempt.saturating_sub(1)));
            let (cancel_tx, _) = watch::channel(false);
            automation.retry_cancel_tx = Some(cancel_tx.clone());
            (attempt, delay_ms, Some(cancel_tx))
        }
    };

    if let Some(attempt) = max_retry_failure {
        emit_session_event(
            &listeners,
            AgentSessionEvent::AutoRetryEnd {
                success: false,
                attempt,
                final_error: message.error_message.clone(),
            },
        );
        return false;
    }

    emit_session_event(
        &listeners,
        AgentSessionEvent::AutoRetryStart {
            attempt,
            max_attempts: settings.max_retries,
            delay_ms,
            error_message: message
                .error_message
                .clone()
                .unwrap_or_else(|| String::from("Unknown error")),
        },
    );

    strip_trailing_error_assistant(&core);

    let listeners_clone = listeners.clone();
    let automation_clone = automation.clone();
    tokio::spawn(async move {
        let mut cancel_rx = cancel_tx.expect("retry cancel tx should exist").subscribe();
        tokio::select! {
            _ = sleep(Duration::from_millis(delay_ms)) => {
                automation_clone.lock().unwrap().retry_cancel_tx = None;
                core.wait_for_idle().await;
                if let Err(error) = core.continue_turn().await {
                    finish_retry_with_failure(&listeners_clone, &automation_clone, attempt, error.to_string());
                }
            }
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    finish_retry_with_failure(&listeners_clone, &automation_clone, attempt, String::from("Retry cancelled"));
                }
            }
        }
    });

    true
}

async fn maybe_run_session_auto_compaction(
    core: CodingAgentCore,
    session_manager: Arc<Mutex<SessionManager>>,
    listeners: Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    automation: Arc<Mutex<SessionAutomationState>>,
    assistant: AssistantMessage,
) {
    let settings = automation.lock().unwrap().compaction_settings.clone();
    if !settings.enabled || assistant.stop_reason == StopReason::Aborted {
        return;
    }

    let state = core.state();
    let same_model =
        assistant.provider == state.model.provider && assistant.model == state.model.id;
    let latest_compaction = {
        let session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        let leaf_id = session_manager.get_leaf_id().map(str::to_owned);
        latest_compaction_timestamp(&session_manager.get_branch(leaf_id.as_deref()))
    };

    if latest_compaction.is_some_and(|timestamp| assistant.timestamp <= timestamp) {
        return;
    }

    if same_model && is_context_overflow(&assistant, Some(state.model.context_window)) {
        let overflow_recovery_attempted = {
            let mut automation = automation.lock().unwrap();
            if automation.overflow_recovery_attempted {
                true
            } else {
                automation.overflow_recovery_attempted = true;
                false
            }
        };

        if overflow_recovery_attempted {
            emit_session_event(
                &listeners,
                AgentSessionEvent::CompactionEnd {
                    reason: CompactionReason::Overflow,
                    result: None,
                    aborted: false,
                    will_retry: false,
                    error_message: Some(String::from(
                        "Context overflow recovery failed after one compact-and-retry attempt. Try reducing context or switching to a larger-context model.",
                    )),
                },
            );
            return;
        }

        strip_trailing_error_assistant(&core);
        run_auto_compaction(
            core,
            session_manager,
            listeners,
            CompactionReason::Overflow,
            settings,
            true,
        )
        .await;
        return;
    }

    let context_tokens = if assistant.stop_reason == StopReason::Error {
        let estimate = estimate_context_tokens(&state.messages);
        let Some(last_usage_index) = estimate.last_usage_index else {
            return;
        };
        if latest_compaction.is_some_and(|timestamp| {
            state
                .messages
                .get(last_usage_index)
                .and_then(agent_message_timestamp)
                .is_some_and(|message_timestamp| message_timestamp <= timestamp)
        }) {
            return;
        }
        estimate.tokens
    } else {
        calculate_context_tokens(&assistant.usage)
    };

    if should_compact(context_tokens, state.model.context_window, &settings) {
        run_auto_compaction(
            core,
            session_manager,
            listeners,
            CompactionReason::Threshold,
            settings,
            false,
        )
        .await;
    }
}

async fn run_auto_compaction(
    core: CodingAgentCore,
    session_manager: Arc<Mutex<SessionManager>>,
    listeners: Arc<Mutex<BTreeMap<usize, SessionEventListener>>>,
    reason: CompactionReason,
    settings: CompactionSettings,
    will_retry: bool,
) {
    emit_session_event(&listeners, AgentSessionEvent::CompactionStart { reason });

    let model = core.state().model;
    let auth = match core
        .model_registry()
        .get_api_key_and_headers_async(&model)
        .await
    {
        Ok(auth) => auth,
        Err(error) => {
            emit_session_event(
                &listeners,
                AgentSessionEvent::CompactionEnd {
                    reason,
                    result: None,
                    aborted: false,
                    will_retry: false,
                    error_message: Some(match reason {
                        CompactionReason::Overflow => {
                            format!("Context overflow recovery failed: {error}")
                        }
                        CompactionReason::Manual | CompactionReason::Threshold => {
                            format!("Auto-compaction failed: {error}")
                        }
                    }),
                },
            );
            return;
        }
    };
    let Some(api_key) = auth.api_key else {
        emit_session_event(
            &listeners,
            AgentSessionEvent::CompactionEnd {
                reason,
                result: None,
                aborted: false,
                will_retry: false,
                error_message: None,
            },
        );
        return;
    };

    let path_entries = {
        let session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        let leaf_id = session_manager.get_leaf_id().map(str::to_owned);
        session_manager.get_branch(leaf_id.as_deref())
    };
    let Some(preparation) = prepare_compaction(&path_entries, settings) else {
        emit_session_event(
            &listeners,
            AgentSessionEvent::CompactionEnd {
                reason,
                result: None,
                aborted: false,
                will_retry: false,
                error_message: None,
            },
        );
        return;
    };

    match run_compaction(&preparation, &model, &api_key, auth.headers, None).await {
        Ok(result) => {
            if rebuild_compacted_session(&core, &session_manager, &result).is_err() {
                emit_session_event(
                    &listeners,
                    AgentSessionEvent::CompactionEnd {
                        reason,
                        result: None,
                        aborted: false,
                        will_retry: false,
                        error_message: Some(match reason {
                            CompactionReason::Overflow => String::from(
                                "Context overflow recovery failed: Failed to persist compaction",
                            ),
                            CompactionReason::Manual | CompactionReason::Threshold => {
                                String::from("Auto-compaction failed: Failed to persist compaction")
                            }
                        }),
                    },
                );
                return;
            }

            emit_session_event(
                &listeners,
                AgentSessionEvent::CompactionEnd {
                    reason,
                    result: Some(result.clone()),
                    aborted: false,
                    will_retry,
                    error_message: None,
                },
            );

            if will_retry {
                strip_trailing_error_assistant(&core);
                tokio::spawn(async move {
                    core.wait_for_idle().await;
                    let _ = core.continue_turn().await;
                });
            } else if core.agent().has_queued_messages() {
                tokio::spawn(async move {
                    core.wait_for_idle().await;
                    let _ = core.continue_turn().await;
                });
            }
        }
        Err(error) => {
            emit_session_event(
                &listeners,
                AgentSessionEvent::CompactionEnd {
                    reason,
                    result: None,
                    aborted: false,
                    will_retry: false,
                    error_message: Some(match reason {
                        CompactionReason::Overflow => {
                            format!("Context overflow recovery failed: {error}")
                        }
                        CompactionReason::Manual | CompactionReason::Threshold => {
                            format!("Auto-compaction failed: {error}")
                        }
                    }),
                },
            );
        }
    }
}

fn rebuild_compacted_session(
    core: &CodingAgentCore,
    session_manager: &Arc<Mutex<SessionManager>>,
    result: &CompactionResult,
) -> Result<(), crate::SessionManagerError> {
    let session_context = {
        let mut session_manager = session_manager
            .lock()
            .expect("session manager mutex poisoned");
        session_manager.append_compaction(
            result.summary.clone(),
            result.first_kept_entry_id.clone(),
            result.tokens_before,
            result.details.clone(),
            None,
        )?;
        session_manager.build_session_context()
    };

    let next_messages = session_context.messages;
    core.agent().update_state(move |state| {
        state.messages = next_messages.clone();
    });
    Ok(())
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
                        stop_reason: StopReason::Error,
                        ..
                    }
                )
            });
        if should_strip {
            state.messages.pop();
        }
    });
}

fn agent_message_timestamp(message: &AgentMessage) -> Option<u64> {
    match message.as_standard_message()? {
        Message::User { timestamp, .. } | Message::Assistant { timestamp, .. } => Some(*timestamp),
        Message::ToolResult { timestamp, .. } => Some(*timestamp),
    }
}

fn is_retryable_error(message: &AssistantMessage, context_window: Option<u64>) -> bool {
    if message.stop_reason != StopReason::Error {
        return false;
    }

    let Some(error_message) = message.error_message.as_deref() else {
        return false;
    };

    if is_context_overflow(message, context_window) {
        return false;
    }

    let lower = error_message.to_ascii_lowercase();
    [
        "overloaded",
        "provider returned error",
        "rate limit",
        "too many requests",
        "429",
        "500",
        "502",
        "503",
        "504",
        "service unavailable",
        "server error",
        "internal error",
        "network error",
        "network_error",
        "connection error",
        "connection refused",
        "other side closed",
        "fetch failed",
        "upstream connect",
        "reset before headers",
        "socket hang up",
        "timed out",
        "timeout",
        "terminated",
        "retry delay",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
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
    extract_user_text(message.as_standard_message()?)
}

fn extract_user_text(message: &Message) -> Option<String> {
    let Message::User { content, .. } = message else {
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
