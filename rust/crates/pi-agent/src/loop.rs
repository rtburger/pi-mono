use crate::{
    AgentMessage, AgentTool, AgentToolResult, error::AgentError, state::AgentContext,
    tool::AgentToolError, validation::validate_tool_arguments,
};
use async_stream::{stream, try_stream};
use futures::{Stream, StreamExt};
use pi_ai::{
    AiError, AssistantEventStream, SimpleStreamOptions, StreamOptions,
    ThinkingLevel as AiThinkingLevel, stream_simple,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason,
    ToolResultMessage, UserContent,
};
use serde_json::{Map as JsonMap, Value};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};

pub type AgentEventStream = Pin<Box<dyn Stream<Item = Result<AgentEvent, AgentError>> + Send>>;
pub type BeforeToolCallFuture =
    Pin<Box<dyn Future<Output = Option<BeforeToolCallResult>> + Send + 'static>>;
pub type AfterToolCallFuture =
    Pin<Box<dyn Future<Output = Option<AfterToolCallResult>> + Send + 'static>>;
pub type TransformContextFuture = Pin<Box<dyn Future<Output = Vec<AgentMessage>> + Send + 'static>>;
pub type ConvertToLlmFuture = Pin<Box<dyn Future<Output = Vec<Message>> + Send + 'static>>;
type MessageQueueFuture = Pin<Box<dyn Future<Output = Vec<AgentMessage>> + Send + 'static>>;
type MessageQueueHook = Arc<dyn Fn() -> MessageQueueFuture + Send + Sync>;
pub type TransformContextHook = Arc<
    dyn Fn(Vec<AgentMessage>, Option<watch::Receiver<bool>>) -> TransformContextFuture
        + Send
        + Sync,
>;
pub type ConvertToLlmHook = Arc<dyn Fn(Vec<AgentMessage>) -> ConvertToLlmFuture + Send + Sync>;
pub type BeforeToolCallHook = Arc<
    dyn Fn(BeforeToolCallContext, Option<watch::Receiver<bool>>) -> BeforeToolCallFuture
        + Send
        + Sync,
>;
pub type AfterToolCallHook = Arc<
    dyn Fn(AfterToolCallContext, Option<watch::Receiver<bool>>) -> AfterToolCallFuture
        + Send
        + Sync,
>;
pub type SharedToolArgs = Arc<Mutex<Value>>;

#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    AgentStart,
    AgentEnd {
        messages: Vec<AgentMessage>,
    },
    TurnStart,
    TurnEnd {
        message: AssistantMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart {
        message: AgentMessage,
    },
    MessageUpdate {
        message: AgentMessage,
        assistant_event: AssistantEvent,
    },
    MessageEnd {
        message: AgentMessage,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        partial_result: AgentToolResult,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: AgentToolResult,
        is_error: bool,
    },
}

#[derive(Debug, Clone)]
pub struct BeforeToolCallContext {
    pub assistant_message: AssistantMessage,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: SharedToolArgs,
    pub context: AgentContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BeforeToolCallResult {
    pub block: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AfterToolCallContext {
    pub assistant_message: AssistantMessage,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
    pub result: AgentToolResult,
    pub is_error: bool,
    pub context: AgentContext,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AfterToolCallResult {
    pub content: Option<Vec<UserContent>>,
    pub details: Option<Value>,
    pub is_error: Option<bool>,
}

enum ToolExecutionProgress {
    Update(AgentToolResult),
    Complete {
        result: AgentToolResult,
        is_error: bool,
    },
}

pub trait AssistantStreamer: Send + Sync {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError>;
}

impl<F> AssistantStreamer for F
where
    F: Fn(Model, Context, StreamOptions) -> Result<AssistantEventStream, AiError> + Send + Sync,
{
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError> {
        self(model, context, options)
    }
}

#[derive(Clone, Default)]
pub struct DefaultAssistantStreamer;

impl AssistantStreamer for DefaultAssistantStreamer {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError> {
        stream_simple(
            model,
            context,
            map_stream_options_to_simple_options(options),
        )
    }
}

fn map_stream_options_to_simple_options(options: StreamOptions) -> SimpleStreamOptions {
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
        temperature: options.temperature,
        max_tokens: options.max_tokens,
        reasoning,
        thinking_budgets: Default::default(),
        tool_choice: options.tool_choice,
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
pub struct AgentLoopConfig {
    pub model: Model,
    pub stream_options: StreamOptions,
    streamer: Arc<dyn AssistantStreamer>,
    convert_to_llm: Option<ConvertToLlmHook>,
    transform_context: Option<TransformContextHook>,
    before_tool_call: Option<BeforeToolCallHook>,
    after_tool_call: Option<AfterToolCallHook>,
    get_steering_messages: Option<MessageQueueHook>,
    get_follow_up_messages: Option<MessageQueueHook>,
}

impl AgentLoopConfig {
    pub fn new(model: Model) -> Self {
        Self {
            model,
            stream_options: StreamOptions::default(),
            streamer: Arc::new(DefaultAssistantStreamer),
            convert_to_llm: None,
            transform_context: None,
            before_tool_call: None,
            after_tool_call: None,
            get_steering_messages: None,
            get_follow_up_messages: None,
        }
    }

    pub fn with_stream_options(mut self, stream_options: StreamOptions) -> Self {
        self.stream_options = stream_options;
        self
    }

    pub fn with_streamer(mut self, streamer: Arc<dyn AssistantStreamer>) -> Self {
        self.streamer = streamer;
        self
    }

    pub fn with_convert_to_llm_hook(mut self, hook: ConvertToLlmHook) -> Self {
        self.convert_to_llm = Some(hook);
        self
    }

    pub fn with_transform_context_hook(mut self, hook: TransformContextHook) -> Self {
        self.transform_context = Some(hook);
        self
    }

    pub fn with_before_tool_call_hook(mut self, hook: BeforeToolCallHook) -> Self {
        self.before_tool_call = Some(hook);
        self
    }

    pub fn with_after_tool_call_hook(mut self, hook: AfterToolCallHook) -> Self {
        self.after_tool_call = Some(hook);
        self
    }

    pub fn with_convert_to_llm<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(Vec<AgentMessage>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<Message>> + Send + 'static,
    {
        self.convert_to_llm = Some(Arc::new(move |messages| Box::pin(hook(messages))));
        self
    }

    pub fn with_transform_context<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(Vec<AgentMessage>, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<AgentMessage>> + Send + 'static,
    {
        self.transform_context = Some(Arc::new(move |messages, signal| {
            Box::pin(hook(messages, signal))
        }));
        self
    }

    pub fn with_before_tool_call<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(BeforeToolCallContext, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<BeforeToolCallResult>> + Send + 'static,
    {
        self.before_tool_call = Some(Arc::new(move |context, signal| {
            Box::pin(hook(context, signal))
        }));
        self
    }

    pub fn with_after_tool_call<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(AfterToolCallContext, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<AfterToolCallResult>> + Send + 'static,
    {
        self.after_tool_call = Some(Arc::new(move |context, signal| {
            Box::pin(hook(context, signal))
        }));
        self
    }

    pub fn with_get_steering_messages<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<AgentMessage>> + Send + 'static,
    {
        self.get_steering_messages = Some(Arc::new(move || Box::pin(hook())));
        self
    }

    pub fn with_get_follow_up_messages<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<AgentMessage>> + Send + 'static,
    {
        self.get_follow_up_messages = Some(Arc::new(move || Box::pin(hook())));
        self
    }
}

pub fn agent_loop(
    prompts: Vec<AgentMessage>,
    context: AgentContext,
    config: AgentLoopConfig,
) -> AgentEventStream {
    run_loop(prompts, context, config)
}

pub fn agent_loop_continue(
    context: AgentContext,
    config: AgentLoopConfig,
) -> Result<AgentEventStream, AgentError> {
    let Some(last_message) = context.messages.last() else {
        return Err(AgentError::EmptyContext);
    };

    if last_message.is_assistant() {
        return Err(AgentError::CannotContinueFromAssistant);
    }

    Ok(run_loop(Vec::new(), context, config))
}

fn run_loop(
    prompts: Vec<AgentMessage>,
    context: AgentContext,
    config: AgentLoopConfig,
) -> AgentEventStream {
    Box::pin(try_stream! {
        let mut new_messages = Vec::new();
        let mut current_messages = context.messages.clone();
        let tool_definitions = context
            .tools
            .iter()
            .map(|tool| tool.definition.clone())
            .collect::<Vec<_>>();

        yield AgentEvent::AgentStart;
        yield AgentEvent::TurnStart;

        for prompt in prompts {
            current_messages.push(prompt.clone());
            new_messages.push(prompt.clone());
            yield AgentEvent::MessageStart {
                message: prompt.clone(),
            };
            yield AgentEvent::MessageEnd { message: prompt };
        }

        let mut first_turn = true;
        let mut pending_messages = get_pending_messages(&config.get_steering_messages).await;

        'outer: loop {
            let mut has_more_tool_calls = true;

            while has_more_tool_calls || !pending_messages.is_empty() {
                if first_turn {
                    first_turn = false;
                } else {
                    yield AgentEvent::TurnStart;
                }

                if !pending_messages.is_empty() {
                    for message in pending_messages.drain(..) {
                        current_messages.push(message.clone());
                        new_messages.push(message.clone());
                        yield AgentEvent::MessageStart {
                            message: message.clone(),
                        };
                        yield AgentEvent::MessageEnd { message };
                    }
                }

                let llm_source_messages = transform_context(
                    current_messages.clone(),
                    &config.transform_context,
                    config.stream_options.signal.clone(),
                ).await;
                let llm_messages = convert_to_llm(llm_source_messages, &config.convert_to_llm).await;

                let llm_context = Context {
                    system_prompt: Some(context.system_prompt.clone()),
                    messages: llm_messages,
                    tools: tool_definitions.clone(),
                };

                let mut assistant_stream = config.streamer.stream(
                    config.model.clone(),
                    llm_context,
                    config.stream_options.clone(),
                )?;

                let mut inserted_partial = false;
                let final_message = loop {
                    let Some(event_result) = assistant_stream.next().await else {
                        Err(AgentError::MissingTerminalEvent)?
                    };
                    let event = event_result?;
                    match event {
                        AssistantEvent::Start { partial } => {
                            inserted_partial = true;
                            let partial_message = assistant_to_agent_message(&partial);
                            current_messages.push(partial_message.clone());
                            yield AgentEvent::MessageStart {
                                message: partial_message,
                            };
                        }
                        AssistantEvent::TextStart { ref partial, .. }
                        | AssistantEvent::TextDelta { ref partial, .. }
                        | AssistantEvent::TextEnd { ref partial, .. }
                        | AssistantEvent::ThinkingStart { ref partial, .. }
                        | AssistantEvent::ThinkingDelta { ref partial, .. }
                        | AssistantEvent::ThinkingEnd { ref partial, .. }
                        | AssistantEvent::ToolCallStart { ref partial, .. }
                        | AssistantEvent::ToolCallDelta { ref partial, .. }
                        | AssistantEvent::ToolCallEnd { ref partial, .. } => {
                            let partial_message = assistant_to_agent_message(partial);
                            if inserted_partial {
                                if let Some(last_message) = current_messages.last_mut() {
                                    *last_message = partial_message.clone();
                                }
                            }
                            yield AgentEvent::MessageUpdate {
                                message: partial_message,
                                assistant_event: event,
                            };
                        }
                        AssistantEvent::Done { message, .. } => {
                            let final_agent_message = assistant_to_agent_message(&message);
                            if inserted_partial {
                                if let Some(last_message) = current_messages.last_mut() {
                                    *last_message = final_agent_message.clone();
                                }
                            } else {
                                current_messages.push(final_agent_message.clone());
                                yield AgentEvent::MessageStart {
                                    message: final_agent_message.clone(),
                                };
                            }

                            yield AgentEvent::MessageEnd {
                                message: final_agent_message,
                            };
                            break message;
                        }
                        AssistantEvent::Error { error, .. } => {
                            let error_agent_message = assistant_to_agent_message(&error);
                            if inserted_partial {
                                if let Some(last_message) = current_messages.last_mut() {
                                    *last_message = error_agent_message.clone();
                                }
                            } else {
                                current_messages.push(error_agent_message.clone());
                                yield AgentEvent::MessageStart {
                                    message: error_agent_message.clone(),
                                };
                            }

                            yield AgentEvent::MessageEnd {
                                message: error_agent_message,
                            };
                            break error;
                        }
                    }
                };

                new_messages.push(assistant_to_agent_message(&final_message));

                if matches!(final_message.stop_reason, StopReason::Error | StopReason::Aborted) {
                    yield AgentEvent::TurnEnd {
                        message: final_message,
                        tool_results: Vec::new(),
                    };
                    break 'outer;
                }

                let tool_calls = extract_tool_calls(&final_message);
                has_more_tool_calls = !tool_calls.is_empty();

                let mut tool_results = Vec::new();
                let mut tool_result_messages = Vec::new();
                if has_more_tool_calls {
                    let tool_context = current_context_snapshot(&context, &current_messages);

                    for (tool_call_id, tool_name, raw_args) in tool_calls {
                        yield AgentEvent::ToolExecutionStart {
                            tool_call_id: tool_call_id.clone(),
                            tool_name: tool_name.clone(),
                            args: raw_args.clone(),
                        };

                        let prepared_args =
                            prepare_tool_arguments(&context.tools, &tool_name, raw_args.clone());
                        let validated_args = match validate_prepared_tool_arguments(
                            &context.tools,
                            &tool_name,
                            prepared_args,
                        ) {
                            Ok(validated_args) => validated_args,
                            Err(error) => {
                                let result = error_tool_result(tool_error_message(&error));
                                yield AgentEvent::ToolExecutionEnd {
                                    tool_call_id: tool_call_id.clone(),
                                    tool_name: tool_name.clone(),
                                    result: result.clone(),
                                    is_error: true,
                                };

                                let tool_result = build_tool_result_message(
                                    tool_call_id.clone(),
                                    tool_name.clone(),
                                    result,
                                    true,
                                );
                                let tool_result_message = tool_result_to_agent_message(&tool_result);
                                yield AgentEvent::MessageStart {
                                    message: tool_result_message.clone(),
                                };
                                yield AgentEvent::MessageEnd {
                                    message: tool_result_message.clone(),
                                };

                                tool_results.push(tool_result);
                                tool_result_messages.push(tool_result_message);
                                continue;
                            }
                        };
                        let shared_args = Arc::new(Mutex::new(validated_args));

                        let (result, is_error) = if let Some(blocked_result) = run_before_tool_call(
                            &config,
                            &tool_context,
                            &final_message,
                            &tool_call_id,
                            &tool_name,
                            shared_args.clone(),
                        )
                        .await
                        {
                            (blocked_result, true)
                        } else {
                            let execution_args = shared_args.lock().unwrap().clone();
                            let mut execution_stream = execute_tool_call_stream(
                                &context.tools,
                                &tool_call_id,
                                &tool_name,
                                execution_args.clone(),
                                config.stream_options.signal.clone(),
                            );
                            let (mut result, mut is_error) = loop {
                                let Some(progress) = execution_stream.next().await else {
                                    break (
                                        error_tool_result(
                                            "tool execution ended without a completion event".into(),
                                        ),
                                        true,
                                    );
                                };
                                match progress {
                                    ToolExecutionProgress::Update(partial_result) => {
                                        yield AgentEvent::ToolExecutionUpdate {
                                            tool_call_id: tool_call_id.clone(),
                                            tool_name: tool_name.clone(),
                                            args: raw_args.clone(),
                                            partial_result,
                                        };
                                    }
                                    ToolExecutionProgress::Complete { result, is_error } => {
                                        break (result, is_error);
                                    }
                                }
                            };

                            if let Some(after_result) = run_after_tool_call(
                                &config,
                                &tool_context,
                                &final_message,
                                &tool_call_id,
                                &tool_name,
                                execution_args,
                                result.clone(),
                                is_error,
                            )
                            .await
                            {
                                result = AgentToolResult {
                                    content: after_result.content.unwrap_or(result.content),
                                    details: after_result.details.unwrap_or(result.details),
                                };
                                is_error = after_result.is_error.unwrap_or(is_error);
                            }

                            (result, is_error)
                        };

                        yield AgentEvent::ToolExecutionEnd {
                            tool_call_id: tool_call_id.clone(),
                            tool_name: tool_name.clone(),
                            result: result.clone(),
                            is_error,
                        };

                        let tool_result = build_tool_result_message(
                            tool_call_id.clone(),
                            tool_name.clone(),
                            result,
                            is_error,
                        );
                        let tool_result_message = tool_result_to_agent_message(&tool_result);
                        yield AgentEvent::MessageStart {
                            message: tool_result_message.clone(),
                        };
                        yield AgentEvent::MessageEnd {
                            message: tool_result_message.clone(),
                        };

                        tool_results.push(tool_result);
                        tool_result_messages.push(tool_result_message);
                    }
                }

                for message in tool_result_messages {
                    current_messages.push(message.clone());
                    new_messages.push(message);
                }

                yield AgentEvent::TurnEnd {
                    message: final_message,
                    tool_results,
                };

                pending_messages = get_pending_messages(&config.get_steering_messages).await;
            }

            let follow_up_messages = get_pending_messages(&config.get_follow_up_messages).await;
            if !follow_up_messages.is_empty() {
                pending_messages = follow_up_messages;
                continue;
            }

            break;
        }

        yield AgentEvent::AgentEnd {
            messages: new_messages,
        };
    })
}

fn default_convert_to_llm(messages: Vec<AgentMessage>) -> Vec<Message> {
    messages
        .into_iter()
        .filter_map(AgentMessage::into_standard_message)
        .collect()
}

async fn transform_context(
    messages: Vec<AgentMessage>,
    hook: &Option<TransformContextHook>,
    signal: Option<watch::Receiver<bool>>,
) -> Vec<AgentMessage> {
    match hook {
        Some(hook) => hook(messages, signal).await,
        None => messages,
    }
}

async fn convert_to_llm(
    messages: Vec<AgentMessage>,
    hook: &Option<ConvertToLlmHook>,
) -> Vec<Message> {
    match hook {
        Some(hook) => hook(messages).await,
        None => default_convert_to_llm(messages),
    }
}

fn prepare_tool_arguments(tools: &[AgentTool], tool_name: &str, raw_args: Value) -> Value {
    tools
        .iter()
        .find(|tool| tool.definition.name == tool_name)
        .map(|tool| tool.prepare_arguments(raw_args.clone()))
        .unwrap_or(raw_args)
}

fn validate_prepared_tool_arguments(
    tools: &[AgentTool],
    tool_name: &str,
    prepared_args: Value,
) -> Result<Value, AgentToolError> {
    tools
        .iter()
        .find(|tool| tool.definition.name == tool_name)
        .map(|tool| validate_tool_arguments(tool, prepared_args.clone()))
        .unwrap_or(Ok(prepared_args))
}

async fn get_pending_messages(source: &Option<MessageQueueHook>) -> Vec<AgentMessage> {
    match source {
        Some(source) => source().await,
        None => Vec::new(),
    }
}

fn current_context_snapshot(
    base_context: &AgentContext,
    messages: &[AgentMessage],
) -> AgentContext {
    AgentContext {
        system_prompt: base_context.system_prompt.clone(),
        messages: messages.to_vec(),
        tools: base_context.tools.clone(),
    }
}

async fn run_before_tool_call(
    config: &AgentLoopConfig,
    context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_call_id: &str,
    tool_name: &str,
    args: SharedToolArgs,
) -> Option<AgentToolResult> {
    let Some(hook) = &config.before_tool_call else {
        return None;
    };

    let result = hook(
        BeforeToolCallContext {
            assistant_message: assistant_message.clone(),
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            args,
            context: context.clone(),
        },
        config.stream_options.signal.clone(),
    )
    .await;

    match result {
        Some(BeforeToolCallResult {
            block: true,
            reason,
        }) => Some(error_tool_result(
            reason.unwrap_or_else(|| "Tool execution was blocked".into()),
        )),
        _ => None,
    }
}

async fn run_after_tool_call(
    config: &AgentLoopConfig,
    context: &AgentContext,
    assistant_message: &AssistantMessage,
    tool_call_id: &str,
    tool_name: &str,
    args: Value,
    result: AgentToolResult,
    is_error: bool,
) -> Option<AfterToolCallResult> {
    let Some(hook) = &config.after_tool_call else {
        return None;
    };

    hook(
        AfterToolCallContext {
            assistant_message: assistant_message.clone(),
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            args,
            result,
            is_error,
            context: context.clone(),
        },
        config.stream_options.signal.clone(),
    )
    .await
}

fn execute_tool_call_stream(
    tools: &[AgentTool],
    tool_call_id: &str,
    tool_name: &str,
    args: Value,
    signal: Option<watch::Receiver<bool>>,
) -> Pin<Box<dyn Stream<Item = ToolExecutionProgress> + Send>> {
    if is_aborted(&signal) {
        return Box::pin(stream! {
            yield ToolExecutionProgress::Complete {
                result: error_tool_result(AiError::Aborted.to_string()),
                is_error: true,
            };
        });
    }

    let tool = tools
        .iter()
        .find(|tool| tool.definition.name == tool_name)
        .cloned();
    let tool_call_id = tool_call_id.to_string();
    let tool_name = tool_name.to_string();

    Box::pin(stream! {
        let Some(tool) = tool else {
            yield ToolExecutionProgress::Complete {
                result: error_tool_result(format!("Tool {tool_name} not found")),
                is_error: true,
            };
            return;
        };

        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();
        let on_update = Arc::new(move |partial_result| {
            let _ = updates_tx.send(partial_result);
        });

        let mut task = Box::pin(tokio::spawn(async move {
            tool.execute_with_updates(tool_call_id, args, signal, Some(on_update))
                .await
        }));
        let mut updates_closed = false;

        let completion = loop {
            if updates_closed {
                break task.await;
            }

            tokio::select! {
                update = updates_rx.recv(), if !updates_closed => {
                    match update {
                        Some(partial_result) => {
                            yield ToolExecutionProgress::Update(partial_result);
                        }
                        None => {
                            updates_closed = true;
                        }
                    }
                }
                result = &mut task => {
                    break result;
                }
            }
        };

        while let Ok(partial_result) = updates_rx.try_recv() {
            yield ToolExecutionProgress::Update(partial_result);
        }

        match completion {
            Ok(Ok(result)) => {
                yield ToolExecutionProgress::Complete {
                    result,
                    is_error: false,
                };
            }
            Ok(Err(error)) => {
                yield ToolExecutionProgress::Complete {
                    result: error_tool_result(tool_error_message(&error)),
                    is_error: true,
                };
            }
            Err(error) => {
                yield ToolExecutionProgress::Complete {
                    result: error_tool_result(error.to_string()),
                    is_error: true,
                };
            }
        }
    })
}

fn tool_error_message(error: &AgentToolError) -> String {
    match error {
        AgentToolError::Message(message) => message.clone(),
    }
}

fn error_tool_result(message: String) -> AgentToolResult {
    AgentToolResult {
        content: vec![UserContent::Text { text: message }],
        details: Value::Null,
    }
}

fn build_tool_result_message(
    tool_call_id: String,
    tool_name: String,
    result: AgentToolResult,
    is_error: bool,
) -> ToolResultMessage {
    ToolResultMessage {
        role: "toolResult".into(),
        tool_call_id,
        tool_name,
        content: result.content,
        is_error,
        timestamp: now_ms(),
    }
}

fn extract_tool_calls(message: &AssistantMessage) -> Vec<(String, String, Value)> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            AssistantContent::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some((
                id.clone(),
                name.clone(),
                Value::Object(JsonMap::from_iter(arguments.clone())),
            )),
            _ => None,
        })
        .collect()
}

fn is_aborted(signal: &Option<watch::Receiver<bool>>) -> bool {
    signal
        .as_ref()
        .map(|signal| *signal.borrow())
        .unwrap_or(false)
}

fn assistant_to_agent_message(message: &AssistantMessage) -> AgentMessage {
    AgentMessage::from(Message::Assistant {
        content: message.content.clone(),
        api: message.api.clone(),
        provider: message.provider.clone(),
        model: message.model.clone(),
        response_id: message.response_id.clone(),
        usage: message.usage.clone(),
        stop_reason: message.stop_reason.clone(),
        error_message: message.error_message.clone(),
        timestamp: message.timestamp,
    })
}

fn tool_result_to_agent_message(tool_result: &ToolResultMessage) -> AgentMessage {
    AgentMessage::from(Message::ToolResult {
        tool_call_id: tool_result.tool_call_id.clone(),
        tool_name: tool_result.tool_name.clone(),
        content: tool_result.content.clone(),
        is_error: tool_result.is_error,
        timestamp: tool_result.timestamp,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
