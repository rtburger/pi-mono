use crate::{
    AfterToolCallContext, AfterToolCallHook, AfterToolCallResult, AgentContext, AgentError,
    AgentEvent, AgentEventStream, AgentLoopConfig, AgentMessage, AgentState, AssistantStreamer,
    BeforeToolCallContext, BeforeToolCallHook, BeforeToolCallResult, ConvertToLlmHook,
    DefaultAssistantStreamer, TransformContextHook, agent_loop, agent_loop_continue,
};
use futures::StreamExt;
use pi_ai::{AiError, StreamOptions};
use pi_events::{AssistantContent, Message, Model, StopReason, Usage, UserContent};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

type ListenerFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type Listener = Arc<dyn Fn(AgentEvent, watch::Receiver<bool>) -> ListenerFuture + Send + Sync>;

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
    stream_options: StreamOptions,
    convert_to_llm: Option<ConvertToLlmHook>,
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
    listeners: BTreeMap<usize, Listener>,
    convert_to_llm: Option<ConvertToLlmHook>,
    transform_context: Option<TransformContextHook>,
    before_tool_call: Option<BeforeToolCallHook>,
    after_tool_call: Option<AfterToolCallHook>,
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
        Self::with_parts(
            initial_state,
            Arc::new(DefaultAssistantStreamer),
            StreamOptions::default(),
        )
    }

    pub fn with_parts(
        initial_state: AgentState,
        streamer: Arc<dyn AssistantStreamer>,
        stream_options: StreamOptions,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AgentInner {
                state: initial_state,
                stream_options,
                streamer,
                listeners: BTreeMap::new(),
                convert_to_llm: None,
                transform_context: None,
                before_tool_call: None,
                after_tool_call: None,
                steering_queue: PendingMessageQueue::default(),
                follow_up_queue: PendingMessageQueue::default(),
                active_run: None,
            })),
            next_listener_id: Arc::new(AtomicUsize::new(1)),
        }
    }

    pub fn state(&self) -> AgentState {
        self.inner.lock().unwrap().state.clone()
    }

    pub fn update_state<R>(&self, updater: impl FnOnce(&mut AgentState) -> R) -> R {
        let mut inner = self.inner.lock().unwrap();
        updater(&mut inner.state)
    }

    pub fn subscribe<F, Fut>(&self, listener: F) -> usize
    where
        F: Fn(AgentEvent, watch::Receiver<bool>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.next_listener_id.fetch_add(1, Ordering::Relaxed);
        let listener: Listener = Arc::new(move |event, signal| Box::pin(listener(event, signal)));
        self.inner.lock().unwrap().listeners.insert(id, listener);
        id
    }

    pub fn unsubscribe(&self, id: usize) -> bool {
        self.inner.lock().unwrap().listeners.remove(&id).is_some()
    }

    pub fn set_convert_to_llm<F, Fut>(&self, hook: F)
    where
        F: Fn(Vec<AgentMessage>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<Message>> + Send + 'static,
    {
        let hook: ConvertToLlmHook = Arc::new(move |messages| Box::pin(hook(messages)));
        self.inner.lock().unwrap().convert_to_llm = Some(hook);
    }

    pub fn clear_convert_to_llm(&self) {
        self.inner.lock().unwrap().convert_to_llm = None;
    }

    pub fn set_transform_context<F, Fut>(&self, hook: F)
    where
        F: Fn(Vec<AgentMessage>, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<AgentMessage>> + Send + 'static,
    {
        let hook: TransformContextHook =
            Arc::new(move |messages, signal| Box::pin(hook(messages, signal)));
        self.inner.lock().unwrap().transform_context = Some(hook);
    }

    pub fn clear_transform_context(&self) {
        self.inner.lock().unwrap().transform_context = None;
    }

    pub fn set_before_tool_call<F, Fut>(&self, hook: F)
    where
        F: Fn(BeforeToolCallContext, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<BeforeToolCallResult>> + Send + 'static,
    {
        let hook: BeforeToolCallHook =
            Arc::new(move |context, signal| Box::pin(hook(context, signal)));
        self.inner.lock().unwrap().before_tool_call = Some(hook);
    }

    pub fn clear_before_tool_call(&self) {
        self.inner.lock().unwrap().before_tool_call = None;
    }

    pub fn set_after_tool_call<F, Fut>(&self, hook: F)
    where
        F: Fn(AfterToolCallContext, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<AfterToolCallResult>> + Send + 'static,
    {
        let hook: AfterToolCallHook =
            Arc::new(move |context, signal| Box::pin(hook(context, signal)));
        self.inner.lock().unwrap().after_tool_call = Some(hook);
    }

    pub fn clear_after_tool_call(&self) {
        self.inner.lock().unwrap().after_tool_call = None;
    }

    pub fn set_steering_mode(&self, mode: QueueMode) {
        self.inner.lock().unwrap().steering_queue.set_mode(mode);
    }

    pub fn steering_mode(&self) -> QueueMode {
        self.inner.lock().unwrap().steering_queue.mode()
    }

    pub fn set_follow_up_mode(&self, mode: QueueMode) {
        self.inner.lock().unwrap().follow_up_queue.set_mode(mode);
    }

    pub fn follow_up_mode(&self) -> QueueMode {
        self.inner.lock().unwrap().follow_up_queue.mode()
    }

    pub fn steer<M>(&self, message: M)
    where
        M: Into<AgentMessage>,
    {
        self.inner
            .lock()
            .unwrap()
            .steering_queue
            .enqueue(message.into());
    }

    pub fn follow_up<M>(&self, message: M)
    where
        M: Into<AgentMessage>,
    {
        self.inner
            .lock()
            .unwrap()
            .follow_up_queue
            .enqueue(message.into());
    }

    pub fn clear_steering_queue(&self) {
        self.inner.lock().unwrap().steering_queue.clear();
    }

    pub fn clear_follow_up_queue(&self) {
        self.inner.lock().unwrap().follow_up_queue.clear();
    }

    pub fn clear_all_queues(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.steering_queue.clear();
        inner.follow_up_queue.clear();
    }

    pub fn has_queued_messages(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.steering_queue.has_items() || inner.follow_up_queue.has_items()
    }

    pub fn abort(&self) {
        let abort_tx = {
            let inner = self.inner.lock().unwrap();
            inner.active_run.as_ref().map(|run| run.abort_tx.clone())
        };
        if let Some(abort_tx) = abort_tx {
            let _ = abort_tx.send(true);
        }
    }

    pub async fn wait_for_idle(&self) {
        let done_signal = {
            let inner = self.inner.lock().unwrap();
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
        self.prompt(Message::User {
            content: vec![UserContent::Text { text: text.into() }],
            timestamp: now_ms(),
        })
        .await
    }

    pub async fn prompt<M>(&self, message: M) -> Result<(), AgentError>
    where
        M: Into<AgentMessage>,
    {
        self.run_request(RunRequest::Prompt(vec![message.into()]))
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
            let mut inner = self.inner.lock().unwrap();
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
                        return Err(AgentError::EmptyContext);
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
                stream_options,
                convert_to_llm: inner.convert_to_llm.clone(),
                transform_context: inner.transform_context.clone(),
                before_tool_call: inner.before_tool_call.clone(),
                after_tool_call: inner.after_tool_call.clone(),
            }
        };

        let mut config = self.create_loop_config(
            prepared.model,
            prepared.streamer,
            prepared.stream_options,
            prepared.skip_initial_steering_poll,
        );
        if let Some(convert_to_llm) = prepared.convert_to_llm {
            config = config.with_convert_to_llm_hook(convert_to_llm);
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
        stream_options: StreamOptions,
        skip_initial_steering_poll: bool,
    ) -> AgentLoopConfig {
        let skip_initial_steering_poll = Arc::new(Mutex::new(skip_initial_steering_poll));
        let steering_inner = self.inner.clone();
        let follow_up_inner = self.inner.clone();

        AgentLoopConfig::new(model)
            .with_streamer(streamer)
            .with_stream_options(stream_options)
            .with_get_steering_messages({
                let skip_initial_steering_poll = skip_initial_steering_poll.clone();
                move || {
                    let steering_inner = steering_inner.clone();
                    let skip_initial_steering_poll = skip_initial_steering_poll.clone();
                    async move {
                        let should_skip = {
                            let mut skip_initial_steering_poll =
                                skip_initial_steering_poll.lock().unwrap();
                            let should_skip = *skip_initial_steering_poll;
                            *skip_initial_steering_poll = false;
                            should_skip
                        };

                        if should_skip {
                            Vec::new()
                        } else {
                            let mut inner = steering_inner.lock().unwrap();
                            inner.steering_queue.drain()
                        }
                    }
                }
            })
            .with_get_follow_up_messages(move || {
                let follow_up_inner = follow_up_inner.clone();
                async move {
                    let mut inner = follow_up_inner.lock().unwrap();
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
            let mut inner = self.inner.lock().unwrap();
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
            let mut inner = self.inner.lock().unwrap();
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
            let mut inner = self.inner.lock().unwrap();
            inner.state.finish_run();
            inner.active_run.take().map(|run| run.done_tx)
        };

        if let Some(done_tx) = done_tx {
            let _ = done_tx.send(true);
        }
    }

    fn is_aborted(&self) -> bool {
        let inner = self.inner.lock().unwrap();
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
