use crate::{
    AfterToolCallContext, AfterToolCallHook, AfterToolCallResult, AgentError, AgentEvent,
    AgentEventStream, AgentLoopConfig, AgentState, AssistantStreamer, BeforeToolCallContext,
    BeforeToolCallHook, BeforeToolCallResult, DefaultAssistantStreamer, agent_loop,
    agent_loop_continue,
};
use futures::StreamExt;
use pi_ai::{AiError, StreamOptions};
use pi_events::{AssistantContent, Message, StopReason, Usage, UserContent};
use std::{
    collections::BTreeMap,
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

struct ActiveRun {
    abort_tx: watch::Sender<bool>,
    done_tx: watch::Sender<bool>,
}

struct AgentInner {
    state: AgentState,
    stream_options: StreamOptions,
    streamer: Arc<dyn AssistantStreamer>,
    listeners: BTreeMap<usize, Listener>,
    before_tool_call: Option<BeforeToolCallHook>,
    after_tool_call: Option<AfterToolCallHook>,
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
                before_tool_call: None,
                after_tool_call: None,
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

    pub async fn prompt(&self, message: Message) -> Result<(), AgentError> {
        self.prompt_messages(vec![message]).await
    }

    pub async fn prompt_messages(&self, messages: Vec<Message>) -> Result<(), AgentError> {
        self.run_prompt(Some(messages)).await
    }

    pub async fn r#continue(&self) -> Result<(), AgentError> {
        self.run_prompt(None).await
    }

    async fn run_prompt(&self, prompts: Option<Vec<Message>>) -> Result<(), AgentError> {
        let (context, model, streamer, stream_options, before_tool_call, after_tool_call) = {
            let mut inner = self.inner.lock().unwrap();
            if inner.active_run.is_some() {
                return Err(match prompts {
                    Some(_) => AgentError::AlreadyProcessingPrompt,
                    None => AgentError::AlreadyProcessingContinue,
                });
            }

            if prompts.is_none() {
                let Some(last_message) = inner.state.messages.last() else {
                    return Err(AgentError::EmptyContext);
                };
                if matches!(last_message, Message::Assistant { .. }) {
                    return Err(AgentError::CannotContinueFromAssistant);
                }
            }

            inner.state.begin_run();
            let (abort_tx, abort_rx) = watch::channel(false);
            let (done_tx, _) = watch::channel(false);
            inner.active_run = Some(ActiveRun { abort_tx, done_tx });

            let mut stream_options = inner.stream_options.clone();
            stream_options.signal = Some(abort_rx);

            (
                inner.state.context_snapshot(),
                inner.state.model.clone(),
                inner.streamer.clone(),
                stream_options,
                inner.before_tool_call.clone(),
                inner.after_tool_call.clone(),
            )
        };

        let mut config = AgentLoopConfig::new(model)
            .with_streamer(streamer)
            .with_stream_options(stream_options);
        if let Some(before_tool_call) = before_tool_call {
            config = config.with_before_tool_call_hook(before_tool_call);
        }
        if let Some(after_tool_call) = after_tool_call {
            config = config.with_after_tool_call_hook(after_tool_call);
        }

        let stream = match prompts {
            Some(prompts) => Ok(agent_loop(prompts, context, config)),
            None => agent_loop_continue(context, config),
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
            inner.state.messages.push(assistant_message.clone());
            inner.state.error_message = assistant_error_message;
            (
                inner.listeners.values().cloned().collect::<Vec<_>>(),
                active_abort_signal(&inner),
            )
        };

        let event = AgentEvent::AgentEnd {
            messages: vec![assistant_message],
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
