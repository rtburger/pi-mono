use crate::{
    AgentTool, AgentToolResult, error::AgentError, state::AgentContext, tool::AgentToolError,
};
use async_stream::try_stream;
use futures::{Stream, StreamExt};
use pi_ai::{AiError, AssistantEventStream, StreamOptions, stream_response};
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
use tokio::sync::watch;

pub type AgentEventStream = Pin<Box<dyn Stream<Item = Result<AgentEvent, AgentError>> + Send>>;
pub type BeforeToolCallFuture =
    Pin<Box<dyn Future<Output = Option<BeforeToolCallResult>> + Send + 'static>>;
pub type AfterToolCallFuture =
    Pin<Box<dyn Future<Output = Option<AfterToolCallResult>> + Send + 'static>>;
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
        messages: Vec<Message>,
    },
    TurnStart,
    TurnEnd {
        message: AssistantMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart {
        message: Message,
    },
    MessageUpdate {
        message: AssistantMessage,
        assistant_event: AssistantEvent,
    },
    MessageEnd {
        message: Message,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
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
        stream_response(model, context, options)
    }
}

#[derive(Clone)]
pub struct AgentLoopConfig {
    pub model: Model,
    pub stream_options: StreamOptions,
    streamer: Arc<dyn AssistantStreamer>,
    before_tool_call: Option<BeforeToolCallHook>,
    after_tool_call: Option<AfterToolCallHook>,
}

impl AgentLoopConfig {
    pub fn new(model: Model) -> Self {
        Self {
            model,
            stream_options: StreamOptions::default(),
            streamer: Arc::new(DefaultAssistantStreamer),
            before_tool_call: None,
            after_tool_call: None,
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

    pub fn with_before_tool_call_hook(mut self, hook: BeforeToolCallHook) -> Self {
        self.before_tool_call = Some(hook);
        self
    }

    pub fn with_after_tool_call_hook(mut self, hook: AfterToolCallHook) -> Self {
        self.after_tool_call = Some(hook);
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
}

pub fn agent_loop(
    prompts: Vec<Message>,
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

    if matches!(last_message, Message::Assistant { .. }) {
        return Err(AgentError::CannotContinueFromAssistant);
    }

    Ok(run_loop(Vec::new(), context, config))
}

fn run_loop(
    prompts: Vec<Message>,
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

        loop {
            let llm_context = Context {
                system_prompt: Some(context.system_prompt.clone()),
                messages: current_messages.clone(),
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
                        current_messages.push(assistant_to_message(&partial));
                        yield AgentEvent::MessageStart {
                            message: assistant_to_message(&partial),
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
                        if inserted_partial {
                            if let Some(last_message) = current_messages.last_mut() {
                                *last_message = assistant_to_message(partial);
                            }
                        }
                        yield AgentEvent::MessageUpdate {
                            message: partial.clone(),
                            assistant_event: event,
                        };
                    }
                    AssistantEvent::Done { message, .. } => {
                        if inserted_partial {
                            if let Some(last_message) = current_messages.last_mut() {
                                *last_message = assistant_to_message(&message);
                            }
                        } else {
                            current_messages.push(assistant_to_message(&message));
                            yield AgentEvent::MessageStart {
                                message: assistant_to_message(&message),
                            };
                        }

                        yield AgentEvent::MessageEnd {
                            message: assistant_to_message(&message),
                        };
                        break message;
                    }
                    AssistantEvent::Error { error, .. } => {
                        if inserted_partial {
                            if let Some(last_message) = current_messages.last_mut() {
                                *last_message = assistant_to_message(&error);
                            }
                        } else {
                            current_messages.push(assistant_to_message(&error));
                            yield AgentEvent::MessageStart {
                                message: assistant_to_message(&error),
                            };
                        }

                        yield AgentEvent::MessageEnd {
                            message: assistant_to_message(&error),
                        };
                        break error;
                    }
                }
            };

            new_messages.push(assistant_to_message(&final_message));

            if matches!(final_message.stop_reason, StopReason::Error | StopReason::Aborted) {
                yield AgentEvent::TurnEnd {
                    message: final_message,
                    tool_results: Vec::new(),
                };
                break;
            }

            let tool_calls = extract_tool_calls(&final_message);
            if tool_calls.is_empty() {
                yield AgentEvent::TurnEnd {
                    message: final_message,
                    tool_results: Vec::new(),
                };
                break;
            }

            let mut tool_results = Vec::new();
            for (tool_call_id, tool_name, raw_args) in tool_calls {
                yield AgentEvent::ToolExecutionStart {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    args: raw_args.clone(),
                };

                let prepared_args = prepare_tool_arguments(&context.tools, &tool_name, raw_args.clone());
                let shared_args = Arc::new(Mutex::new(prepared_args));

                let before = run_before_tool_call(
                    &config,
                    &context,
                    &final_message,
                    &tool_call_id,
                    &tool_name,
                    shared_args.clone(),
                ).await;
                if let Some(blocked_result) = before {
                    yield AgentEvent::ToolExecutionEnd {
                        tool_call_id: tool_call_id.clone(),
                        tool_name: tool_name.clone(),
                        result: blocked_result.clone(),
                        is_error: true,
                    };

                    let tool_result = build_tool_result_message(
                        tool_call_id.clone(),
                        tool_name.clone(),
                        blocked_result,
                        true,
                    );
                    let tool_result_message = tool_result_to_message(&tool_result);
                    current_messages.push(tool_result_message.clone());
                    new_messages.push(tool_result_message.clone());
                    yield AgentEvent::MessageStart {
                        message: tool_result_message.clone(),
                    };
                    yield AgentEvent::MessageEnd {
                        message: tool_result_message,
                    };
                    tool_results.push(tool_result);
                    continue;
                }

                let execution_args = shared_args.lock().unwrap().clone();
                let (mut result, mut is_error) = execute_tool_call(
                    &context.tools,
                    &tool_call_id,
                    &tool_name,
                    execution_args.clone(),
                    config.stream_options.signal.clone(),
                ).await;

                if let Some(after_result) = run_after_tool_call(
                    &config,
                    &context,
                    &final_message,
                    &tool_call_id,
                    &tool_name,
                    execution_args,
                    result.clone(),
                    is_error,
                ).await {
                    result = AgentToolResult {
                        content: after_result.content.unwrap_or(result.content),
                        details: after_result.details.unwrap_or(result.details),
                    };
                    is_error = after_result.is_error.unwrap_or(is_error);
                }

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
                let tool_result_message = tool_result_to_message(&tool_result);
                current_messages.push(tool_result_message.clone());
                new_messages.push(tool_result_message.clone());
                yield AgentEvent::MessageStart {
                    message: tool_result_message.clone(),
                };
                yield AgentEvent::MessageEnd {
                    message: tool_result_message,
                };
                tool_results.push(tool_result);
            }

            yield AgentEvent::TurnEnd {
                message: final_message,
                tool_results,
            };
            yield AgentEvent::TurnStart;
        }

        yield AgentEvent::AgentEnd {
            messages: new_messages,
        };
    })
}

fn prepare_tool_arguments(tools: &[AgentTool], tool_name: &str, raw_args: Value) -> Value {
    tools
        .iter()
        .find(|tool| tool.definition.name == tool_name)
        .map(|tool| tool.prepare_arguments(raw_args.clone()))
        .unwrap_or(raw_args)
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

async fn execute_tool_call(
    tools: &[AgentTool],
    tool_call_id: &str,
    tool_name: &str,
    args: Value,
    signal: Option<watch::Receiver<bool>>,
) -> (AgentToolResult, bool) {
    if is_aborted(&signal) {
        return (error_tool_result(AiError::Aborted.to_string()), true);
    }

    let Some(tool) = tools.iter().find(|tool| tool.definition.name == tool_name) else {
        return (
            error_tool_result(format!("Tool {tool_name} not found")),
            true,
        );
    };

    match tool.execute(tool_call_id.to_string(), args, signal).await {
        Ok(result) => (result, false),
        Err(error) => (error_tool_result(tool_error_message(&error)), true),
    }
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

fn assistant_to_message(message: &AssistantMessage) -> Message {
    Message::Assistant {
        content: message.content.clone(),
        api: message.api.clone(),
        provider: message.provider.clone(),
        model: message.model.clone(),
        response_id: message.response_id.clone(),
        usage: message.usage.clone(),
        stop_reason: message.stop_reason.clone(),
        error_message: message.error_message.clone(),
        timestamp: message.timestamp,
    }
}

fn tool_result_to_message(tool_result: &ToolResultMessage) -> Message {
    Message::ToolResult {
        tool_call_id: tool_result.tool_call_id.clone(),
        tool_name: tool_result.tool_name.clone(),
        content: tool_result.content.clone(),
        is_error: tool_result.is_error,
        timestamp: tool_result.timestamp,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
