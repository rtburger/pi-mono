use crate::{
    AfterToolCallContext, AfterToolCallHook, AfterToolCallResult, AgentContext, AgentError,
    AgentEvent, AgentEventStream, AgentLoopConfig, AgentMessage, AgentState, AssistantStreamer,
    BeforeToolCallContext, BeforeToolCallHook, BeforeToolCallResult, ConvertToLlmHook,
    CustomAgentMessage, DefaultAssistantStreamer, GetApiKeyHook, ToolExecutionMode,
    TransformContextHook, agent_loop, agent_loop_continue,
};
use futures::StreamExt;
use parking_lot::Mutex;
use pi_ai::{AiError, PayloadHook, StreamOptions, ThinkingBudgets, Transport};
use pi_events::{AssistantContent, Message, Model, StopReason, Usage, UserContent};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

type ListenerFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type Listener = Arc<dyn Fn(AgentEvent, watch::Receiver<bool>) -> ListenerFuture + Send + Sync>;

pub type AgentUnsubscribe = Box<dyn FnOnce() -> bool + Send + 'static>;

#[derive(Debug, Clone, PartialEq)]
pub enum PromptInput {
    Text(String),
    TextWithImages {
        text: String,
        images: Vec<UserContent>,
    },
    Message(AgentMessage),
    Messages(Vec<AgentMessage>),
}

impl PromptInput {
    fn into_messages(self) -> Vec<AgentMessage> {
        match self {
            Self::Text(text) => vec![
                Message::User {
                    content: vec![UserContent::Text { text }],
                    timestamp: now_ms(),
                }
                .into(),
            ],
            Self::TextWithImages { text, images } => {
                let mut content = Vec::with_capacity(images.len() + 1);
                content.push(UserContent::Text { text });
                content.extend(images);
                vec![
                    Message::User {
                        content,
                        timestamp: now_ms(),
                    }
                    .into(),
                ]
            }
            Self::Message(message) => vec![message],
            Self::Messages(messages) => messages,
        }
    }
}

impl From<&str> for PromptInput {
    fn from(text: &str) -> Self {
        Self::Text(text.to_string())
    }
}

impl From<String> for PromptInput {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<Message> for PromptInput {
    fn from(message: Message) -> Self {
        Self::Message(message.into())
    }
}

impl From<AgentMessage> for PromptInput {
    fn from(message: AgentMessage) -> Self {
        Self::Message(message)
    }
}

impl From<CustomAgentMessage> for PromptInput {
    fn from(message: CustomAgentMessage) -> Self {
        Self::Message(message.into())
    }
}

impl<M> From<Vec<M>> for PromptInput
where
    M: Into<AgentMessage>,
{
    fn from(messages: Vec<M>) -> Self {
        Self::Messages(messages.into_iter().map(Into::into).collect())
    }
}

impl<T> From<(T, Vec<UserContent>)> for PromptInput
where
    T: Into<String>,
{
    fn from((text, images): (T, Vec<UserContent>)) -> Self {
        Self::TextWithImages {
            text: text.into(),
            images,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueMode {
    All,
    #[default]
    OneAtATime,
}

#[derive(Default)]
struct PendingMessageQueue {
    mode: QueueMode,
    messages: VecDeque<AgentMessage>,
}

impl PendingMessageQueue {
    fn enqueue(&mut self, message: AgentMessage) {
        self.messages.push_back(message);
    }

    fn set_mode(&mut self, mode: QueueMode) {
        self.mode = mode;
    }

    fn mode(&self) -> QueueMode {
        self.mode
    }

    fn drain(&mut self) -> Vec<AgentMessage> {
        match self.mode {
            QueueMode::All => self.messages.drain(..).collect(),
            QueueMode::OneAtATime => self.messages.pop_front().into_iter().collect(),
        }
    }

    fn has_items(&self) -> bool {
        !self.messages.is_empty()
    }

    fn clear(&mut self) {
        self.messages.clear();
    }
}

struct ActiveRun {
    abort_tx: watch::Sender<bool>,
    done_tx: watch::Sender<bool>,
}

struct PreparedRun {
    prompts: Option<Vec<AgentMessage>>,
    skip_initial_steering_poll: bool,
    context: AgentContext,
    model: Model,
    streamer: Arc<dyn AssistantStreamer>,
    uses_default_streamer: bool,
    stream_options: StreamOptions,
    thinking_budgets: ThinkingBudgets,
    tool_execution: ToolExecutionMode,
    convert_to_llm: Option<ConvertToLlmHook>,
    get_api_key: Option<GetApiKeyHook>,
    transform_context: Option<TransformContextHook>,
    before_tool_call: Option<BeforeToolCallHook>,
    after_tool_call: Option<AfterToolCallHook>,
}

enum RunRequest {
    Prompt(Vec<AgentMessage>),
    Continue,
}

struct AgentInner {
    state: AgentState,
    stream_options: StreamOptions,
    streamer: Arc<dyn AssistantStreamer>,
    uses_default_streamer: bool,
    thinking_budgets: ThinkingBudgets,
    listeners: BTreeMap<usize, Listener>,
    convert_to_llm: Option<ConvertToLlmHook>,
    get_api_key: Option<GetApiKeyHook>,
    transform_context: Option<TransformContextHook>,
    before_tool_call: Option<BeforeToolCallHook>,
    after_tool_call: Option<AfterToolCallHook>,
    tool_execution: ToolExecutionMode,
    steering_queue: PendingMessageQueue,
    follow_up_queue: PendingMessageQueue,
    active_run: Option<ActiveRun>,
}

#[derive(Clone)]
pub struct Agent {
    inner: Arc<Mutex<AgentInner>>,
    next_listener_id: Arc<AtomicUsize>,
}

impl Agent {
    pub fn new(initial_state: AgentState) -> Self {
        Self::with_parts_internal(
            initial_state,
            Arc::new(DefaultAssistantStreamer::default()),
            StreamOptions::default(),
            true,
        )
    }

    pub fn with_parts(
        initial_state: AgentState,
        streamer: Arc<dyn AssistantStreamer>,
        stream_options: StreamOptions,
    ) -> Self {
        Self::with_parts_internal(initial_state, streamer, stream_options, false)
    }

    fn with_parts_internal(
        initial_state: AgentState,
        streamer: Arc<dyn AssistantStreamer>,
        mut stream_options: StreamOptions,
        uses_default_streamer: bool,
    ) -> Self {
        if stream_options.transport.is_none() {
            stream_options.transport = Some(Transport::Sse);
        }

        Self {
            inner: Arc::new(Mutex::new(AgentInner {
                state: initial_state,
                stream_options,
                streamer,
                uses_default_streamer,
                thinking_budgets: ThinkingBudgets::default(),
                listeners: BTreeMap::new(),
                convert_to_llm: None,
                get_api_key: None,
                transform_context: None,
                before_tool_call: None,
                after_tool_call: None,
                tool_execution: ToolExecutionMode::Parallel,
                steering_queue: PendingMessageQueue::default(),
                follow_up_queue: PendingMessageQueue::default(),
                active_run: None,
            })),
            next_listener_id: Arc::new(AtomicUsize::new(1)),
        }
    }

    pub fn state(&self) -> AgentState {
        self.inner.lock().state.clone()
    }

    pub fn update_state<R>(&self, updater: impl FnOnce(&mut AgentState) -> R) -> R {
        let mut inner = self.inner.lock();
        updater(&mut inner.state)
    }

    pub fn subscribe<F, Fut>(&self, listener: F) -> AgentUnsubscribe
    where
        F: Fn(AgentEvent, watch::Receiver<bool>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.subscribe_id(listener);
        let agent = self.clone();
        Box::new(move || agent.unsubscribe(id))
    }

    pub fn subscribe_id<F, Fut>(&self, listener: F) -> usize
    where
        F: Fn(AgentEvent, watch::Receiver<bool>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.next_listener_id.fetch_add(1, Ordering::Relaxed);
        let listener: Listener = Arc::new(move |event, signal| Box::pin(listener(event, signal)));
        self.inner.lock().listeners.insert(id, listener);
        id
    }

    pub fn unsubscribe(&self, id: usize) -> bool {
        self.inner.lock().listeners.remove(&id).is_some()
    }

    pub fn set_convert_to_llm<F, Fut>(&self, hook: F)
    where
        F: Fn(Vec<AgentMessage>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<Message>> + Send + 'static,
    {
        let hook: ConvertToLlmHook = Arc::new(move |messages| Box::pin(hook(messages)));
        self.inner.lock().convert_to_llm = Some(hook);
    }

    pub fn clear_convert_to_llm(&self) {
        self.inner.lock().convert_to_llm = None;
    }

    pub fn set_get_api_key<F, Fut>(&self, hook: F)
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<String>> + Send + 'static,
    {
        let hook: GetApiKeyHook = Arc::new(move |provider| Box::pin(hook(provider)));
        self.inner.lock().get_api_key = Some(hook);
    }

    pub fn clear_get_api_key(&self) {
        self.inner.lock().get_api_key = None;
    }

    pub fn set_streamer(&self, streamer: Arc<dyn AssistantStreamer>) {
        let mut inner = self.inner.lock();
        inner.streamer = streamer;
        inner.uses_default_streamer = false;
    }

    pub fn clear_streamer(&self) {
        let mut inner = self.inner.lock();
        inner.streamer = Arc::new(DefaultAssistantStreamer::new(
            inner.thinking_budgets.clone(),
        ));
        inner.uses_default_streamer = true;
    }

    pub fn set_transform_context<F, Fut>(&self, hook: F)
    where
        F: Fn(Vec<AgentMessage>, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<AgentMessage>> + Send + 'static,
    {
        let hook: TransformContextHook =
            Arc::new(move |messages, signal| Box::pin(hook(messages, signal)));
        self.inner.lock().transform_context = Some(hook);
    }

    pub fn clear_transform_context(&self) {
        self.inner.lock().transform_context = None;
    }

    pub fn set_before_tool_call<F, Fut>(&self, hook: F)
    where
        F: Fn(BeforeToolCallContext, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<BeforeToolCallResult>> + Send + 'static,
    {
        let hook: BeforeToolCallHook =
            Arc::new(move |context, signal| Box::pin(hook(context, signal)));
        self.inner.lock().before_tool_call = Some(hook);
    }

    pub fn clear_before_tool_call(&self) {
        self.inner.lock().before_tool_call = None;
    }

    pub fn set_after_tool_call<F, Fut>(&self, hook: F)
    where
        F: Fn(AfterToolCallContext, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<AfterToolCallResult>> + Send + 'static,
    {
        let hook: AfterToolCallHook =
            Arc::new(move |context, signal| Box::pin(hook(context, signal)));
        self.inner.lock().after_tool_call = Some(hook);
    }

    pub fn clear_after_tool_call(&self) {
        self.inner.lock().after_tool_call = None;
    }

    pub fn set_thinking_budgets(&self, thinking_budgets: ThinkingBudgets) {
        self.inner.lock().thinking_budgets = thinking_budgets;
    }

    pub fn thinking_budgets(&self) -> ThinkingBudgets {
        self.inner.lock().thinking_budgets.clone()
    }

    pub fn set_tool_execution_mode(&self, tool_execution: ToolExecutionMode) {
        self.inner.lock().tool_execution = tool_execution;
    }

    pub fn tool_execution_mode(&self) -> ToolExecutionMode {
        self.inner.lock().tool_execution
    }

    pub fn set_steering_mode(&self, mode: QueueMode) {
        self.inner.lock().steering_queue.set_mode(mode);
    }

    pub fn steering_mode(&self) -> QueueMode {
        self.inner.lock().steering_queue.mode()
    }

    pub fn set_follow_up_mode(&self, mode: QueueMode) {
        self.inner.lock().follow_up_queue.set_mode(mode);
    }

    pub fn follow_up_mode(&self) -> QueueMode {
        self.inner.lock().follow_up_queue.mode()
    }

    pub fn steer<M>(&self, message: M)
    where
        M: Into<AgentMessage>,
    {
        self.inner.lock().steering_queue.enqueue(message.into());
    }

    pub fn follow_up<M>(&self, message: M)
    where
        M: Into<AgentMessage>,
    {
        self.inner.lock().follow_up_queue.enqueue(message.into());
    }

    pub fn clear_steering_queue(&self) {
        self.inner.lock().steering_queue.clear();
    }

    pub fn clear_follow_up_queue(&self) {
        self.inner.lock().follow_up_queue.clear();
    }

    pub fn clear_all_queues(&self) {
        let mut inner = self.inner.lock();
        inner.steering_queue.clear();
        inner.follow_up_queue.clear();
    }

    pub fn has_queued_messages(&self) -> bool {
        let inner = self.inner.lock();
        inner.steering_queue.has_items() || inner.follow_up_queue.has_items()
    }

    pub fn session_id(&self) -> Option<String> {
        self.inner.lock().stream_options.session_id.clone()
    }

    pub fn set_session_id(&self, session_id: Option<String>) {
        self.inner.lock().stream_options.session_id = session_id;
    }

    pub fn transport(&self) -> Option<Transport> {
        self.inner.lock().stream_options.transport
    }

    pub fn set_transport(&self, transport: Option<Transport>) {
        self.inner.lock().stream_options.transport = transport;
    }

    pub fn on_payload(&self) -> Option<PayloadHook> {
        self.inner.lock().stream_options.on_payload.clone()
    }

    pub fn set_on_payload(&self, on_payload: Option<PayloadHook>) {
        self.inner.lock().stream_options.on_payload = on_payload;
    }

    pub fn max_retry_delay_ms(&self) -> Option<u64> {
        self.inner.lock().stream_options.max_retry_delay_ms
    }

    pub fn set_max_retry_delay_ms(&self, max_retry_delay_ms: Option<u64>) {
        self.inner.lock().stream_options.max_retry_delay_ms = max_retry_delay_ms;
    }

    pub fn signal(&self) -> Option<watch::Receiver<bool>> {
        let inner = self.inner.lock();
        inner
            .active_run
            .as_ref()
            .map(|run| run.abort_tx.subscribe())
    }

    pub fn reset(&self) {
        let mut inner = self.inner.lock();
        inner.state.messages.clear();
        inner.state.finish_run();
        inner.state.error_message = None;
        inner.steering_queue.clear();
        inner.follow_up_queue.clear();
    }

    pub fn abort(&self) {
        let abort_tx = {
            let inner = self.inner.lock();
            inner.active_run.as_ref().map(|run| run.abort_tx.clone())
        };
        if let Some(abort_tx) = abort_tx {
            let _ = abort_tx.send(true);
        }
    }

    pub async fn wait_for_idle(&self) {
        let done_signal = {
            let inner = self.inner.lock();
            inner.active_run.as_ref().map(|run| run.done_tx.subscribe())
        };

        let Some(mut done_signal) = done_signal else {
            return;
        };

        if *done_signal.borrow() {
            return;
        }

        while done_signal.changed().await.is_ok() {
            if *done_signal.borrow() {
                return;
            }
        }
    }

    pub async fn prompt_text(&self, text: impl Into<String>) -> Result<(), AgentError> {
        self.prompt(text.into()).await
    }

    pub async fn prompt_text_with_images(
        &self,
        text: impl Into<String>,
        images: Vec<UserContent>,
    ) -> Result<(), AgentError> {
        self.prompt((text.into(), images)).await
    }

    pub async fn prompt_with_images(
        &self,
        text: impl Into<String>,
        images: Vec<UserContent>,
    ) -> Result<(), AgentError> {
        self.prompt((text.into(), images)).await
    }

    pub async fn prompt<P>(&self, input: P) -> Result<(), AgentError>
    where
        P: Into<PromptInput>,
    {
        self.run_request(RunRequest::Prompt(input.into().into_messages()))
            .await
    }

    pub async fn prompt_messages<I, M>(&self, messages: I) -> Result<(), AgentError>
    where
        I: IntoIterator<Item = M>,
        M: Into<AgentMessage>,
    {
        self.run_request(RunRequest::Prompt(
            messages.into_iter().map(Into::into).collect(),
        ))
        .await
    }

    pub async fn r#continue(&self) -> Result<(), AgentError> {
        self.run_request(RunRequest::Continue).await
    }

    async fn run_request(&self, request: RunRequest) -> Result<(), AgentError> {
        let prepared = {
            let mut inner = self.inner.lock();
            if inner.active_run.is_some() {
                return Err(match request {
                    RunRequest::Prompt(_) => AgentError::AlreadyProcessingPrompt,
                    RunRequest::Continue => AgentError::AlreadyProcessingContinue,
                });
            }

            let mut prompts = None;
            let mut skip_initial_steering_poll = false;

            match request {
                RunRequest::Prompt(messages) => {
                    prompts = Some(messages);
                }
                RunRequest::Continue => {
                    let Some(last_message) = inner.state.messages.last() else {
                        return Err(AgentError::NoMessagesToContinue);
                    };

                    if last_message.is_assistant() {
                        let queued_steering = inner.steering_queue.drain();
                        if !queued_steering.is_empty() {
                            prompts = Some(queued_steering);
                            skip_initial_steering_poll = true;
                        } else {
                            let queued_follow_ups = inner.follow_up_queue.drain();
                            if queued_follow_ups.is_empty() {
                                return Err(AgentError::CannotContinueFromAssistant);
                            }
                            prompts = Some(queued_follow_ups);
                        }
                    }
                }
            }

            inner.state.begin_run();
            let (abort_tx, abort_rx) = watch::channel(false);
            let (done_tx, _) = watch::channel(false);
            inner.active_run = Some(ActiveRun { abort_tx, done_tx });

            let mut stream_options = inner.stream_options.clone();
            stream_options.signal = Some(abort_rx);
            stream_options.reasoning_effort =
                thinking_level_to_reasoning_effort(inner.state.thinking_level)
                    .map(ToOwned::to_owned);

            PreparedRun {
                prompts,
                skip_initial_steering_poll,
                context: inner.state.context_snapshot(),
                model: inner.state.model.clone(),
                streamer: inner.streamer.clone(),
                uses_default_streamer: inner.uses_default_streamer,
                stream_options,
                thinking_budgets: inner.thinking_budgets.clone(),
                tool_execution: inner.tool_execution,
                convert_to_llm: inner.convert_to_llm.clone(),
                get_api_key: inner.get_api_key.clone(),
                transform_context: inner.transform_context.clone(),
                before_tool_call: inner.before_tool_call.clone(),
                after_tool_call: inner.after_tool_call.clone(),
            }
        };

        let mut config = self.create_loop_config(
            prepared.model,
            prepared.streamer,
            prepared.uses_default_streamer,
            prepared.stream_options,
            prepared.thinking_budgets,
            prepared.tool_execution,
            prepared.skip_initial_steering_poll,
        );
        if let Some(convert_to_llm) = prepared.convert_to_llm {
            config = config.with_convert_to_llm_hook(convert_to_llm);
        }
        if let Some(get_api_key) = prepared.get_api_key {
            config = config.with_get_api_key_hook(get_api_key);
        }
        if let Some(transform_context) = prepared.transform_context {
            config = config.with_transform_context_hook(transform_context);
        }
        if let Some(before_tool_call) = prepared.before_tool_call {
            config = config.with_before_tool_call_hook(before_tool_call);
        }
        if let Some(after_tool_call) = prepared.after_tool_call {
            config = config.with_after_tool_call_hook(after_tool_call);
        }

        let stream = match prepared.prompts {
            Some(prompts) => Ok(agent_loop(prompts, prepared.context, config)),
            None => agent_loop_continue(prepared.context, config),
        };

        let result = match stream {
            Ok(stream) => self.consume_stream(stream).await,
            Err(error) => {
                self.finish_run();
                return Err(error);
            }
        };

        match result {
            Ok(()) => {
                self.finish_run();
                Ok(())
            }
            Err(error) => {
                self.handle_run_failure(error).await;
                self.finish_run();
                Ok(())
            }
        }
    }

    fn create_loop_config(
        &self,
        model: Model,
        streamer: Arc<dyn AssistantStreamer>,
        uses_default_streamer: bool,
        stream_options: StreamOptions,
        thinking_budgets: ThinkingBudgets,
        tool_execution: ToolExecutionMode,
        skip_initial_steering_poll: bool,
    ) -> AgentLoopConfig {
        let skip_initial_steering_poll = Arc::new(Mutex::new(skip_initial_steering_poll));
        let steering_inner = self.inner.clone();
        let follow_up_inner = self.inner.clone();

        let mut config = AgentLoopConfig::new(model)
            .with_stream_options(stream_options)
            .with_tool_execution_mode(tool_execution);
        if uses_default_streamer {
            config = config.with_thinking_budgets(thinking_budgets);
        } else {
            config = config.with_streamer(streamer);
        }

        config
            .with_get_steering_messages({
                let skip_initial_steering_poll = skip_initial_steering_poll.clone();
                move || {
                    let steering_inner = steering_inner.clone();
                    let skip_initial_steering_poll = skip_initial_steering_poll.clone();
                    async move {
                        let should_skip = {
                            let mut skip_initial_steering_poll = skip_initial_steering_poll.lock();
                            let should_skip = *skip_initial_steering_poll;
                            *skip_initial_steering_poll = false;
                            should_skip
                        };

                        if should_skip {
                            Vec::new()
                        } else {
                            let mut inner = steering_inner.lock();
                            inner.steering_queue.drain()
                        }
                    }
                }
            })
            .with_get_follow_up_messages(move || {
                let follow_up_inner = follow_up_inner.clone();
                async move {
                    let mut inner = follow_up_inner.lock();
                    inner.follow_up_queue.drain()
                }
            })
    }

    async fn consume_stream(&self, mut stream: AgentEventStream) -> Result<(), AgentError> {
        while let Some(event) = stream.next().await {
            self.process_event(event?).await;
        }
        Ok(())
    }

    async fn process_event(&self, event: AgentEvent) {
        let (listeners, abort_signal) = {
            let mut inner = self.inner.lock();
            inner.state.apply_event(&event);
            (
                inner.listeners.values().cloned().collect::<Vec<_>>(),
                active_abort_signal(&inner),
            )
        };

        for listener in listeners {
            listener(event.clone(), abort_signal.clone()).await;
        }
    }

    async fn handle_run_failure(&self, error: AgentError) {
        let state = self.state();
        let assistant_message = failure_message(
            &state.model,
            error.to_string(),
            is_aborted_error(&error) || self.is_aborted(),
        );
        let assistant_error_message = extract_error_message(&assistant_message);

        let (listeners, abort_signal) = {
            let mut inner = self.inner.lock();
            inner.state.streaming_message = None;
            inner.state.messages.push(assistant_message.clone().into());
            inner.state.error_message = assistant_error_message;
            (
                inner.listeners.values().cloned().collect::<Vec<_>>(),
                active_abort_signal(&inner),
            )
        };

        let event = AgentEvent::AgentEnd {
            messages: vec![assistant_message.into()],
        };
        for listener in listeners {
            listener(event.clone(), abort_signal.clone()).await;
        }
    }

    fn finish_run(&self) {
        let done_tx = {
            let mut inner = self.inner.lock();
            inner.state.finish_run();
            inner.active_run.take().map(|run| run.done_tx)
        };

        if let Some(done_tx) = done_tx {
            let _ = done_tx.send(true);
        }
    }

    fn is_aborted(&self) -> bool {
        let inner = self.inner.lock();
        inner
            .active_run
            .as_ref()
            .map(|run| {
                let signal = run.abort_tx.subscribe();
                *signal.borrow()
            })
            .unwrap_or(false)
    }
}

fn active_abort_signal(inner: &AgentInner) -> watch::Receiver<bool> {
    inner
        .active_run
        .as_ref()
        .expect("agent listener invoked outside active run")
        .abort_tx
        .subscribe()
}

fn failure_message(model: &pi_events::Model, error_message: String, aborted: bool) -> Message {
    Message::Assistant {
        content: vec![AssistantContent::Text {
            text: String::new(),
            text_signature: None,
        }],
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_id: None,
        usage: Usage::default(),
        stop_reason: if aborted {
            StopReason::Aborted
        } else {
            StopReason::Error
        },
        error_message: Some(error_message),
        timestamp: now_ms(),
    }
}

fn extract_error_message(message: &Message) -> Option<String> {
    match message {
        Message::Assistant { error_message, .. } => error_message.clone(),
        _ => None,
    }
}

fn is_aborted_error(error: &AgentError) -> bool {
    matches!(error, AgentError::Ai(AiError::Aborted))
}

fn thinking_level_to_reasoning_effort(
    thinking_level: crate::state::ThinkingLevel,
) -> Option<&'static str> {
    match thinking_level {
        crate::state::ThinkingLevel::Off => None,
        crate::state::ThinkingLevel::Minimal => Some("minimal"),
        crate::state::ThinkingLevel::Low => Some("low"),
        crate::state::ThinkingLevel::Medium => Some("medium"),
        crate::state::ThinkingLevel::High => Some("high"),
        crate::state::ThinkingLevel::XHigh => Some("xhigh"),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_stream::try_stream;
    use pi_ai::{
        AssistantEventStream, FauxResponse, RegisterFauxProviderOptions, register_faux_provider,
    };
    use pi_events::{
        AssistantContent, AssistantEvent, Message, Model, StopReason, Usage, UserContent,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Notify;

    fn model() -> Model {
        Model {
            id: "mock".into(),
            name: "Mock".into(),
            api: "faux:test".into(),
            provider: "faux".into(),
            base_url: "http://localhost".into(),
            reasoning: false,
            input: vec!["text".into()],
            cost: pi_events::ModelCost {
                input: 1.0,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.1,
            },
            context_window: 8192,
            max_tokens: 2048,
            compat: None,
        }
    }

    fn assistant_message(text: &str, stop_reason: StopReason, timestamp: u64) -> Message {
        Message::Assistant {
            content: vec![AssistantContent::Text {
                text: text.to_string(),
                text_signature: None,
            }],
            api: "faux:test".into(),
            provider: "faux".into(),
            model: "mock".into(),
            response_id: None,
            usage: Usage::default(),
            stop_reason,
            error_message: None,
            timestamp,
        }
    }

    fn user_message(text: &str, timestamp: u64) -> Message {
        Message::User {
            content: vec![UserContent::Text {
                text: text.to_string(),
            }],
            timestamp,
        }
    }

    #[tokio::test]
    async fn streamer_can_be_swapped_between_turns_and_cleared_back_to_default() {
        let registration = register_faux_provider(RegisterFauxProviderOptions::default());
        registration.set_responses(vec![FauxResponse::text("from default")]);
        let model = registration.get_model(None).expect("faux model");

        let custom_1_calls = Arc::new(AtomicUsize::new(0));
        let custom_streamer_1 = Arc::new({
            let custom_1_calls = custom_1_calls.clone();
            move |model: Model,
                  _context: pi_events::Context,
                  _options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                custom_1_calls.fetch_add(1, Ordering::Relaxed);

                let mut message = pi_events::AssistantMessage::empty(
                    model.api.clone(),
                    model.provider.clone(),
                    model.id.clone(),
                );
                message.content = vec![AssistantContent::Text {
                    text: "from custom 1".into(),
                    text_signature: None,
                }];
                message.stop_reason = StopReason::Stop;
                message.timestamp = 20;

                Ok(Box::pin(try_stream! {
                    yield AssistantEvent::Done {
                        reason: StopReason::Stop,
                        message,
                    };
                }))
            }
        });

        let custom_2_calls = Arc::new(AtomicUsize::new(0));
        let custom_streamer_2 = Arc::new({
            let custom_2_calls = custom_2_calls.clone();
            move |model: Model,
                  _context: pi_events::Context,
                  _options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                custom_2_calls.fetch_add(1, Ordering::Relaxed);

                let mut message = pi_events::AssistantMessage::empty(
                    model.api.clone(),
                    model.provider.clone(),
                    model.id.clone(),
                );
                message.content = vec![AssistantContent::Text {
                    text: "from custom 2".into(),
                    text_signature: None,
                }];
                message.stop_reason = StopReason::Stop;
                message.timestamp = 21;

                Ok(Box::pin(try_stream! {
                    yield AssistantEvent::Done {
                        reason: StopReason::Stop,
                        message,
                    };
                }))
            }
        });

        let agent = Agent::new(AgentState::new(model));
        agent.set_streamer(custom_streamer_1);
        agent.prompt_text("hello").await.unwrap();
        assert_eq!(custom_1_calls.load(Ordering::Relaxed), 1);
        assert_eq!(registration.call_count(), 0);
        assert!(matches!(
            agent.state().messages.last().and_then(AgentMessage::as_standard_message),
            Some(Message::Assistant { content, .. })
                if matches!(content.as_slice(), [AssistantContent::Text { text, .. }] if text == "from custom 1")
        ));

        agent.set_streamer(custom_streamer_2);
        agent.prompt_text("hello again").await.unwrap();
        assert_eq!(custom_2_calls.load(Ordering::Relaxed), 1);
        assert_eq!(registration.call_count(), 0);
        assert!(matches!(
            agent.state().messages.last().and_then(AgentMessage::as_standard_message),
            Some(Message::Assistant { content, .. })
                if matches!(content.as_slice(), [AssistantContent::Text { text, .. }] if text == "from custom 2")
        ));

        agent.clear_streamer();
        agent.prompt_text("hello from default").await.unwrap();
        assert_eq!(registration.call_count(), 1);
        assert!(matches!(
            agent.state().messages.last().and_then(AgentMessage::as_standard_message),
            Some(Message::Assistant { content, .. })
                if matches!(content.as_slice(), [AssistantContent::Text { text, .. }] if text == "from default")
        ));

        registration.unregister();
    }

    #[tokio::test]
    async fn transport_defaults_to_sse_round_trips_and_is_forwarded_to_streamer() {
        let received_transports = Arc::new(Mutex::new(Vec::new()));
        let streamer = Arc::new({
            let received_transports = received_transports.clone();
            move |_model: Model,
                  _context: pi_events::Context,
                  options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                received_transports.lock().push(options.transport);

                let mut message = pi_events::AssistantMessage::empty("faux:test", "faux", "mock");
                message.content = vec![AssistantContent::Text {
                    text: "ok".into(),
                    text_signature: None,
                }];
                message.stop_reason = StopReason::Stop;
                message.timestamp = 20;

                Ok(Box::pin(try_stream! {
                    yield AssistantEvent::Done {
                        reason: StopReason::Stop,
                        message,
                    };
                }))
            }
        });

        let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
        assert_eq!(agent.transport(), Some(Transport::Sse));

        agent.prompt_text("hello").await.unwrap();

        agent.set_transport(Some(Transport::WebSocket));
        assert_eq!(agent.transport(), Some(Transport::WebSocket));

        agent.prompt_text("hello over websocket").await.unwrap();

        agent.set_transport(None);
        assert_eq!(agent.transport(), None);

        agent
            .prompt_text("hello without transport override")
            .await
            .unwrap();

        let received = received_transports.lock().clone();
        assert_eq!(
            received,
            vec![Some(Transport::Sse), Some(Transport::WebSocket), None,]
        );
    }

    #[tokio::test]
    async fn on_payload_round_trips_and_is_forwarded_to_streamer() {
        let observed_payloads = Arc::new(Mutex::new(Vec::new()));
        let streamer = Arc::new({
            let observed_payloads = observed_payloads.clone();
            move |model: Model,
                  _context: pi_events::Context,
                  options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                let observed_payloads = observed_payloads.clone();
                Ok(Box::pin(try_stream! {
                    let payload = options
                        .on_payload
                        .expect("agent should forward on_payload hook")
                        .call(json!({ "source": "streamer" }), model.clone())
                        .await
                        .map_err(AiError::Message)?;
                    observed_payloads.lock().push(payload);

                    let mut message = pi_events::AssistantMessage::empty("faux:test", "faux", "mock");
                    message.content = vec![AssistantContent::Text {
                        text: "ok".into(),
                        text_signature: None,
                    }];
                    message.stop_reason = StopReason::Stop;
                    message.timestamp = 20;

                    yield AssistantEvent::Done {
                        reason: StopReason::Stop,
                        message,
                    };
                }))
            }
        });

        let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
        assert!(agent.on_payload().is_none());

        agent.set_on_payload(Some(PayloadHook::new(|payload, model| async move {
            Ok(Some(json!({
                "payload": payload,
                "model": model.id,
            })))
        })));
        assert!(agent.on_payload().is_some());

        agent.prompt_text("hello").await.unwrap();

        agent.set_on_payload(Some(PayloadHook::new(|payload, model| async move {
            Ok(Some(json!({
                "kind": "updated",
                "payload": payload,
                "provider": model.provider,
            })))
        })));
        assert!(agent.on_payload().is_some());

        agent.prompt_text("hello again").await.unwrap();

        agent.set_on_payload(None);
        assert!(agent.on_payload().is_none());

        let observed = observed_payloads.lock().clone();
        assert_eq!(
            observed,
            vec![
                Some(json!({
                    "payload": { "source": "streamer" },
                    "model": "mock",
                })),
                Some(json!({
                    "kind": "updated",
                    "payload": { "source": "streamer" },
                    "provider": "faux",
                })),
            ]
        );
    }

    #[tokio::test]
    async fn get_api_key_resolves_per_turn_and_falls_back_to_stream_options_api_key() {
        let received_api_keys = Arc::new(Mutex::new(Vec::new()));
        let requested_providers = Arc::new(Mutex::new(Vec::new()));
        let streamer = Arc::new({
            let received_api_keys = received_api_keys.clone();
            move |_model: Model,
                  _context: pi_events::Context,
                  options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                received_api_keys.lock().push(options.api_key.clone());

                let mut message = pi_events::AssistantMessage::empty("faux:test", "faux", "mock");
                message.content = vec![AssistantContent::Text {
                    text: "ok".into(),
                    text_signature: None,
                }];
                message.stop_reason = StopReason::Stop;
                message.timestamp = 20;

                Ok(Box::pin(try_stream! {
                    yield AssistantEvent::Done {
                        reason: StopReason::Stop,
                        message,
                    };
                }))
            }
        });

        let mut stream_options = StreamOptions::default();
        stream_options.api_key = Some("fallback-key".into());

        let agent = Agent::with_parts(AgentState::new(model()), streamer, stream_options);
        let next_api_key = Arc::new(AtomicUsize::new(1));
        agent.set_get_api_key({
            let requested_providers = requested_providers.clone();
            let next_api_key = next_api_key.clone();
            move |provider| {
                requested_providers.lock().push(provider);
                let suffix = next_api_key.fetch_add(1, Ordering::Relaxed);
                async move { Some(format!("resolved-{suffix}")) }
            }
        });

        agent.prompt_text("hello").await.unwrap();
        agent.prompt_text("hello again").await.unwrap();

        agent.set_get_api_key(|_provider| async move { None });
        agent.prompt_text("hello with fallback").await.unwrap();

        agent.clear_get_api_key();
        agent
            .prompt_text("hello after clearing hook")
            .await
            .unwrap();

        assert_eq!(
            requested_providers.lock().clone(),
            vec!["faux".to_string(), "faux".to_string()]
        );
        assert_eq!(
            received_api_keys.lock().clone(),
            vec![
                Some("resolved-1".to_string()),
                Some("resolved-2".to_string()),
                Some("fallback-key".to_string()),
                Some("fallback-key".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn max_retry_delay_ms_round_trips_and_is_forwarded_to_streamer() {
        let received_max_retry_delays = Arc::new(Mutex::new(Vec::new()));
        let streamer = Arc::new({
            let received_max_retry_delays = received_max_retry_delays.clone();
            move |_model: Model,
                  _context: pi_events::Context,
                  options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                received_max_retry_delays
                    .lock()
                    .push(options.max_retry_delay_ms);

                let mut message = pi_events::AssistantMessage::empty("faux:test", "faux", "mock");
                message.content = vec![AssistantContent::Text {
                    text: "ok".into(),
                    text_signature: None,
                }];
                message.stop_reason = StopReason::Stop;
                message.timestamp = 20;

                Ok(Box::pin(try_stream! {
                    yield AssistantEvent::Done {
                        reason: StopReason::Stop,
                        message,
                    };
                }))
            }
        });

        let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
        assert_eq!(agent.max_retry_delay_ms(), None);

        agent.prompt_text("hello").await.unwrap();

        agent.set_max_retry_delay_ms(Some(1_200));
        assert_eq!(agent.max_retry_delay_ms(), Some(1_200));

        agent
            .prompt_text("hello with max retry delay")
            .await
            .unwrap();

        agent.set_max_retry_delay_ms(Some(2_400));
        assert_eq!(agent.max_retry_delay_ms(), Some(2_400));

        agent
            .prompt_text("hello with updated max retry delay")
            .await
            .unwrap();

        agent.set_max_retry_delay_ms(None);
        assert_eq!(agent.max_retry_delay_ms(), None);

        agent
            .prompt_text("hello without max retry delay")
            .await
            .unwrap();

        let received = received_max_retry_delays.lock().clone();
        assert_eq!(received, vec![None, Some(1_200), Some(2_400), None]);
    }

    #[tokio::test]
    async fn session_id_round_trips_and_is_forwarded_to_streamer() {
        let received_session_ids = Arc::new(Mutex::new(Vec::new()));
        let streamer = Arc::new({
            let received_session_ids = received_session_ids.clone();
            move |_model: Model,
                  _context: pi_events::Context,
                  options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                received_session_ids.lock().push(options.session_id.clone());

                let mut message = pi_events::AssistantMessage::empty("faux:test", "faux", "mock");
                message.content = vec![AssistantContent::Text {
                    text: "ok".into(),
                    text_signature: None,
                }];
                message.stop_reason = StopReason::Stop;
                message.timestamp = 20;

                Ok(Box::pin(try_stream! {
                    yield AssistantEvent::Done {
                        reason: StopReason::Stop,
                        message,
                    };
                }))
            }
        });

        let mut stream_options = StreamOptions::default();
        stream_options.session_id = Some("session-abc".into());

        let agent = Agent::with_parts(AgentState::new(model()), streamer, stream_options);
        assert_eq!(agent.session_id().as_deref(), Some("session-abc"));

        agent.prompt_text("hello").await.unwrap();

        agent.set_session_id(Some("session-def".into()));
        assert_eq!(agent.session_id().as_deref(), Some("session-def"));

        agent.prompt_text("hello again").await.unwrap();

        agent.set_session_id(None);
        assert_eq!(agent.session_id(), None);

        agent.prompt_text("hello without session").await.unwrap();

        let received = received_session_ids.lock().clone();
        assert_eq!(
            received,
            vec![
                Some("session-abc".to_string()),
                Some("session-def".to_string()),
                None,
            ]
        );
    }

    #[tokio::test]
    async fn signal_exposes_active_abort_receiver() {
        let stream_entered = Arc::new(Notify::new());
        let streamer = Arc::new({
            let stream_entered = stream_entered.clone();
            move |_model: Model,
                  _context: pi_events::Context,
                  options: StreamOptions|
                  -> Result<AssistantEventStream, AiError> {
                let mut signal = options.signal.expect("agent should inject an abort signal");
                let stream_entered = stream_entered.clone();
                Ok(Box::pin(try_stream! {
                    stream_entered.notify_waiters();
                    yield AssistantEvent::Start {
                        partial: pi_events::AssistantMessage {
                            role: "assistant".into(),
                            content: vec![AssistantContent::Text {
                                text: String::new(),
                                text_signature: None,
                            }],
                            api: "faux:test".into(),
                            provider: "faux".into(),
                            model: "mock".into(),
                            response_id: None,
                            usage: Usage::default(),
                            stop_reason: StopReason::Stop,
                            error_message: None,
                            timestamp: 20,
                        },
                    };

                    while !*signal.borrow() {
                        signal.changed().await.expect("abort signal sender should stay alive");
                    }

                    let mut aborted = pi_events::AssistantMessage::empty("faux:test", "faux", "mock");
                    aborted.stop_reason = StopReason::Aborted;
                    aborted.error_message = Some("Request was aborted".into());
                    aborted.timestamp = 21;
                    yield AssistantEvent::Error {
                        reason: StopReason::Aborted,
                        error: aborted,
                    };
                }))
            }
        });

        let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
        let prompt_agent = agent.clone();
        let prompt_task = tokio::spawn(async move { prompt_agent.prompt_text("hello").await });

        stream_entered.notified().await;
        let mut signal = agent
            .signal()
            .expect("running agent should expose a signal");
        assert!(!*signal.borrow());

        agent.abort();
        while !*signal.borrow() {
            signal
                .changed()
                .await
                .expect("abort signal receiver should stay connected until the run ends");
        }

        assert!(prompt_task.await.unwrap().is_ok());
        assert!(agent.signal().is_none());
    }

    #[test]
    fn reset_clears_transcript_runtime_state_and_queues() {
        let agent = Agent::new(AgentState::new(model()));
        agent.update_state(|state| {
            state.system_prompt = "system".into();
            state.thinking_level = crate::ThinkingLevel::High;
            state
                .messages
                .push(user_message("kept only until reset", 10).into());
            state.begin_run();
            state.streaming_message =
                Some(assistant_message("partial", StopReason::Stop, 11).into());
            state.pending_tool_calls.insert("tool-1".into());
            state.error_message = Some("boom".into());
        });
        agent.steer(user_message("steering", 12));
        agent.follow_up(user_message("follow-up", 13));

        agent.reset();

        let state = agent.state();
        assert_eq!(state.system_prompt, "system");
        assert_eq!(state.thinking_level, crate::ThinkingLevel::High);
        assert!(state.messages.is_empty());
        assert!(!state.is_streaming);
        assert!(state.streaming_message.is_none());
        assert!(state.pending_tool_calls.is_empty());
        assert!(state.error_message.is_none());
        assert!(!agent.has_queued_messages());
    }
}
