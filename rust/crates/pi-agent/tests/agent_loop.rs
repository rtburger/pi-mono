use futures::{StreamExt, stream};
use pi_agent::{
    AfterToolCallResult, AgentContext, AgentEvent, AgentLoopConfig, AgentMessage, AgentState,
    AgentTool, AgentToolError, AgentToolResult, AssistantStreamer, BeforeToolCallResult,
    CustomAgentMessage, ThinkingLevel, agent_loop, agent_loop_continue,
};
use pi_ai::{
    AiError, AssistantEventStream, FauxResponse, RegisterFauxProviderOptions, StreamOptions,
    register_faux_provider,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason,
    ToolDefinition, Usage, UserContent,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct ScriptedStreamer {
    streams: Arc<Mutex<VecDeque<Vec<Result<AssistantEvent, AiError>>>>>,
}

impl ScriptedStreamer {
    fn new(streams: Vec<Vec<Result<AssistantEvent, AiError>>>) -> Self {
        Self {
            streams: Arc::new(Mutex::new(streams.into())),
        }
    }
}

impl AssistantStreamer for ScriptedStreamer {
    fn stream(
        &self,
        _model: Model,
        _context: Context,
        _options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError> {
        let events = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AiError::Message("no scripted stream remaining".into()))?;
        Ok(Box::pin(stream::iter(events)))
    }
}

fn model() -> Model {
    Model {
        id: "mock".into(),
        name: "Mock".into(),
        api: "faux:test".into(),
        provider: "faux".into(),
        base_url: "http://localhost".into(),
        reasoning: false,
        input: vec!["text".into()],
        context_window: 8192,
        max_tokens: 2048,
    }
}

fn usage() -> Usage {
    Usage::default()
}

fn user_message(text: &str, timestamp: u64) -> Message {
    Message::User {
        content: vec![UserContent::Text {
            text: text.to_string(),
        }],
        timestamp,
    }
}

fn message_has_user_text(message: &AgentMessage, expected: &str) -> bool {
    match message {
        AgentMessage::Standard(Message::User { content, .. }) => {
            content.iter().any(|block| match block {
                UserContent::Text { text } => text == expected,
                _ => false,
            })
        }
        _ => false,
    }
}

fn is_standard_user_message(message: &AgentMessage) -> bool {
    matches!(message, AgentMessage::Standard(Message::User { .. }))
}

fn is_standard_assistant_message(message: &AgentMessage) -> bool {
    matches!(message, AgentMessage::Standard(Message::Assistant { .. }))
}

fn is_standard_tool_result_message(message: &AgentMessage) -> bool {
    matches!(message, AgentMessage::Standard(Message::ToolResult { .. }))
}

fn llm_message_has_user_text(message: &Message, expected: &str) -> bool {
    match message {
        Message::User { content, .. } => content.iter().any(|block| match block {
            UserContent::Text { text } => text == expected,
            _ => false,
        }),
        _ => false,
    }
}

fn convert_custom_text_messages_to_llm(messages: Vec<AgentMessage>) -> Vec<Message> {
    messages
        .into_iter()
        .filter_map(|message| match message {
            AgentMessage::Standard(message) => Some(message),
            AgentMessage::Custom(CustomAgentMessage {
                role,
                payload,
                timestamp,
            }) if role == "custom" => {
                payload
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| Message::User {
                        content: vec![UserContent::Text {
                            text: text.to_string(),
                        }],
                        timestamp,
                    })
            }
            AgentMessage::Custom(_) => None,
        })
        .collect()
}

fn assistant_message(text: &str, stop_reason: StopReason, timestamp: u64) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content: vec![AssistantContent::Text {
            text: text.to_string(),
            text_signature: None,
        }],
        api: "faux:test".into(),
        provider: "faux".into(),
        model: "mock".into(),
        response_id: None,
        usage: usage(),
        stop_reason,
        error_message: None,
        timestamp,
    }
}

fn assistant_tool_call_message(
    tool_call_id: &str,
    tool_name: &str,
    arguments: serde_json::Map<String, Value>,
    timestamp: u64,
) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content: vec![AssistantContent::ToolCall {
            id: tool_call_id.to_string(),
            name: tool_name.to_string(),
            arguments: arguments.into_iter().collect(),
            thought_signature: None,
        }],
        api: "faux:test".into(),
        provider: "faux".into(),
        model: "mock".into(),
        response_id: None,
        usage: usage(),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp,
    }
}

fn echo_tool() -> AgentTool {
    AgentTool::new(
        ToolDefinition {
            name: "echo".into(),
            description: "Echo the provided value".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                },
                "required": ["value"]
            }),
        },
        |_tool_call_id, args, _signal| async move {
            let value = args
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| AgentToolError::message("missing tool arg: value"))?;
            Ok(AgentToolResult {
                content: vec![UserContent::Text {
                    text: format!("echoed: {value}"),
                }],
                details: json!({ "value": value }),
            })
        },
    )
}

async fn collect_events(
    mut stream: pi_agent::AgentEventStream,
) -> Result<Vec<AgentEvent>, pi_agent::AgentError> {
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    Ok(events)
}

fn final_messages(events: &[AgentEvent]) -> Vec<AgentMessage> {
    events
        .iter()
        .find_map(|event| match event {
            AgentEvent::AgentEnd { messages } => Some(messages.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn prompt_loop_emits_user_and_assistant_events() {
    let partial = assistant_message("Hel", StopReason::Stop, 20);
    let final_assistant = assistant_message("Hello", StopReason::Stop, 21);
    let streamer = Arc::new(ScriptedStreamer::new(vec![vec![
        Ok(AssistantEvent::Start {
            partial: partial.clone(),
        }),
        Ok(AssistantEvent::TextStart {
            content_index: 0,
            partial: partial.clone(),
        }),
        Ok(AssistantEvent::TextDelta {
            content_index: 0,
            delta: "lo".into(),
            partial: final_assistant.clone(),
        }),
        Ok(AssistantEvent::TextEnd {
            content_index: 0,
            content: "Hello".into(),
            partial: final_assistant.clone(),
        }),
        Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: final_assistant.clone(),
        }),
    ]])) as Arc<dyn AssistantStreamer>;

    let config = AgentLoopConfig::new(model()).with_streamer(streamer);
    let stream = agent_loop(
        vec![user_message("Hi", 10).into()],
        AgentContext::new("You are helpful."),
        config,
    );

    let events = collect_events(stream).await.unwrap();
    let event_kinds = events
        .iter()
        .map(|event| match event {
            AgentEvent::AgentStart => "agent_start",
            AgentEvent::TurnStart => "turn_start",
            AgentEvent::MessageStart { .. } => "message_start",
            AgentEvent::MessageUpdate { .. } => "message_update",
            AgentEvent::MessageEnd { .. } => "message_end",
            AgentEvent::ToolExecutionStart { .. } => "tool_execution_start",
            AgentEvent::ToolExecutionUpdate { .. } => "tool_execution_update",
            AgentEvent::ToolExecutionEnd { .. } => "tool_execution_end",
            AgentEvent::TurnEnd { .. } => "turn_end",
            AgentEvent::AgentEnd { .. } => "agent_end",
        })
        .collect::<Vec<_>>();

    assert_eq!(
        event_kinds,
        vec![
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "message_start",
            "message_update",
            "message_update",
            "message_update",
            "message_end",
            "turn_end",
            "agent_end",
        ]
    );

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 2);
    assert!(is_standard_user_message(&messages[0]));
    assert!(is_standard_assistant_message(&messages[1]));
}

#[tokio::test]
async fn continue_loop_validates_context_and_skips_existing_user_events() {
    let config = AgentLoopConfig::new(model());

    let empty_error = match agent_loop_continue(AgentContext::new("Test"), config.clone()) {
        Ok(_) => panic!("expected empty-context error"),
        Err(error) => error,
    };
    assert_eq!(
        empty_error.to_string(),
        "Cannot continue: no messages in context"
    );

    let mut assistant_tail_context = AgentContext::new("Test");
    assistant_tail_context.messages.push(
        Message::Assistant {
            content: vec![AssistantContent::Text {
                text: "done".into(),
                text_signature: None,
            }],
            api: "faux:test".into(),
            provider: "faux".into(),
            model: "mock".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 1,
        }
        .into(),
    );
    let assistant_error = match agent_loop_continue(assistant_tail_context, config.clone()) {
        Ok(_) => panic!("expected assistant-tail error"),
        Err(error) => error,
    };
    assert_eq!(
        assistant_error.to_string(),
        "Cannot continue from message role: assistant"
    );

    let streamer = Arc::new(ScriptedStreamer::new(vec![vec![Ok(
        AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("continued", StopReason::Stop, 30),
        },
    )]])) as Arc<dyn AssistantStreamer>;

    let mut context = AgentContext::new("Test");
    context.messages.push(user_message("resume", 5).into());

    let stream = agent_loop_continue(context, config.with_streamer(streamer)).unwrap();
    let events = collect_events(stream).await.unwrap();

    let message_end_count = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::MessageEnd { .. }))
        .count();
    assert_eq!(message_end_count, 1);

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 1);
    assert!(is_standard_assistant_message(&messages[0]));
}

#[tokio::test]
async fn agent_state_reduces_loop_events() {
    let final_assistant = assistant_message("Hello", StopReason::Stop, 21);
    let streamer = Arc::new(ScriptedStreamer::new(vec![vec![
        Ok(AssistantEvent::Start {
            partial: assistant_message("He", StopReason::Stop, 20),
        }),
        Ok(AssistantEvent::TextDelta {
            content_index: 0,
            delta: "llo".into(),
            partial: final_assistant.clone(),
        }),
        Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: final_assistant.clone(),
        }),
    ]])) as Arc<dyn AssistantStreamer>;

    let mut state = AgentState::new(model());
    state.system_prompt = "You are helpful.".into();
    state.thinking_level = ThinkingLevel::Low;
    state.begin_run();

    let stream = agent_loop(
        vec![user_message("Hi", 10).into()],
        state.context_snapshot(),
        AgentLoopConfig::new(state.model.clone()).with_streamer(streamer),
    );

    let events = collect_events(stream).await.unwrap();
    for event in &events {
        state.apply_event(event);
    }
    state.finish_run();

    assert!(!state.is_streaming);
    assert!(state.streaming_message.is_none());
    assert_eq!(state.messages.len(), 2);
    assert!(state.error_message.is_none());
    assert_eq!(state.thinking_level, ThinkingLevel::Low);
}

#[tokio::test]
async fn executes_sequential_tool_calls_and_continues_with_tool_results() {
    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(echo_tool());

    let streamer = Arc::new(ScriptedStreamer::new(vec![
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::ToolUse,
            message: assistant_tool_call_message(
                "tool-1",
                "echo",
                serde_json::Map::from_iter([(
                    String::from("value"),
                    Value::String("hello".into()),
                )]),
                20,
            ),
        })],
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("done", StopReason::Stop, 30),
        })],
    ])) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model()).with_streamer(streamer),
    );

    let events = collect_events(stream).await.unwrap();
    let event_kinds = events
        .iter()
        .map(|event| match event {
            AgentEvent::AgentStart => "agent_start",
            AgentEvent::TurnStart => "turn_start",
            AgentEvent::MessageStart { .. } => "message_start",
            AgentEvent::MessageUpdate { .. } => "message_update",
            AgentEvent::MessageEnd { .. } => "message_end",
            AgentEvent::ToolExecutionStart { .. } => "tool_execution_start",
            AgentEvent::ToolExecutionUpdate { .. } => "tool_execution_update",
            AgentEvent::ToolExecutionEnd { .. } => "tool_execution_end",
            AgentEvent::TurnEnd { .. } => "turn_end",
            AgentEvent::AgentEnd { .. } => "agent_end",
        })
        .collect::<Vec<_>>();

    assert_eq!(
        event_kinds,
        vec![
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "message_start",
            "message_end",
            "tool_execution_start",
            "tool_execution_end",
            "message_start",
            "message_end",
            "turn_end",
            "turn_start",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end",
        ]
    );

    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
            } => Some((tool_call_id, tool_name, result, is_error)),
            _ => None,
        })
        .expect("expected tool execution end event");
    assert_eq!(tool_end.0, "tool-1");
    assert_eq!(tool_end.1, "echo");
    assert!(!tool_end.3);
    assert_eq!(
        tool_end.2.content,
        vec![UserContent::Text {
            text: "echoed: hello".into(),
        }]
    );

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 4);
    assert!(is_standard_user_message(&messages[0]));
    assert!(is_standard_assistant_message(&messages[1]));
    assert!(is_standard_tool_result_message(&messages[2]));
    assert!(is_standard_assistant_message(&messages[3]));
}

#[tokio::test]
async fn emits_tool_execution_update_events_before_completion() {
    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(AgentTool::new_with_updates(
        ToolDefinition {
            name: "echo".into(),
            description: "Echo with streamed updates".into(),
            parameters: json!({ "type": "object" }),
        },
        |_tool_call_id, args, _signal, on_update| async move {
            let value = args
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(on_update) = on_update {
                on_update(AgentToolResult {
                    content: vec![UserContent::Text {
                        text: format!("partial: {value}"),
                    }],
                    details: json!({ "step": 1 }),
                });
                on_update(AgentToolResult {
                    content: vec![UserContent::Text {
                        text: format!("partial: {value}:done"),
                    }],
                    details: json!({ "step": 2 }),
                });
            }
            Ok(AgentToolResult {
                content: vec![UserContent::Text {
                    text: format!("final: {value}"),
                }],
                details: json!({ "done": true }),
            })
        },
    ));

    let streamer = Arc::new(ScriptedStreamer::new(vec![
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::ToolUse,
            message: assistant_tool_call_message(
                "tool-1",
                "echo",
                serde_json::Map::from_iter([(
                    String::from("value"),
                    Value::String("hello".into()),
                )]),
                20,
            ),
        })],
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("done", StopReason::Stop, 30),
        })],
    ])) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model()).with_streamer(streamer),
    );

    let events = collect_events(stream).await.unwrap();
    let update_texts = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionUpdate { partial_result, .. } => Some(
                partial_result
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        UserContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(
        update_texts,
        vec![
            String::from("partial: hello"),
            String::from("partial: hello:done"),
        ]
    );

    let tool_event_sequence = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionStart { .. } => Some("start"),
            AgentEvent::ToolExecutionUpdate { .. } => Some("update"),
            AgentEvent::ToolExecutionEnd { .. } => Some("end"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tool_event_sequence,
        vec!["start", "update", "update", "end"]
    );
}

#[tokio::test]
async fn prepares_arguments_and_allows_before_hook_mutation_without_revalidation() {
    let executed_args = Arc::new(Mutex::new(Vec::new()));
    let tool = AgentTool::new(
        ToolDefinition {
            name: "echo".into(),
            description: "Echo prepared args".into(),
            parameters: json!({ "type": "object" }),
        },
        {
            let executed_args = executed_args.clone();
            move |_tool_call_id, args, _signal| {
                let executed_args = executed_args.clone();
                async move {
                    executed_args.lock().unwrap().push(args.clone());
                    Ok(AgentToolResult {
                        content: vec![UserContent::Text {
                            text: format!("executed: {args}"),
                        }],
                        details: args,
                    })
                }
            }
        },
    )
    .with_prepare_arguments(|args| {
        let alias = args
            .get("alias")
            .and_then(Value::as_str)
            .unwrap_or_default();
        json!({ "value": alias })
    });

    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(tool);

    let streamer = Arc::new(ScriptedStreamer::new(vec![
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::ToolUse,
            message: assistant_tool_call_message(
                "tool-1",
                "echo",
                serde_json::Map::from_iter([(
                    String::from("alias"),
                    Value::String("hello".into()),
                )]),
                20,
            ),
        })],
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("done", StopReason::Stop, 30),
        })],
    ])) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_before_tool_call(|before, _signal| async move {
                *before.args.lock().unwrap() = json!({ "value": 123 });
                None
            }),
    );

    let events = collect_events(stream).await.unwrap();
    assert_eq!(
        executed_args.lock().unwrap().clone(),
        vec![json!({ "value": 123 })]
    );

    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("expected tool execution end event");
    assert_eq!(
        tool_end.content,
        vec![UserContent::Text {
            text: "executed: {\"value\":123}".into(),
        }]
    );
}

#[tokio::test]
async fn validation_failures_become_error_tool_results() {
    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(echo_tool());

    let streamer = Arc::new(ScriptedStreamer::new(vec![
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::ToolUse,
            message: assistant_tool_call_message("tool-1", "echo", serde_json::Map::new(), 20),
        })],
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("done", StopReason::Stop, 30),
        })],
    ])) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model()).with_streamer(streamer),
    );

    let events = collect_events(stream).await.unwrap();
    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                result, is_error, ..
            } => Some((result.clone(), *is_error)),
            _ => None,
        })
        .expect("expected tool execution end event");

    assert!(tool_end.1);
    let error_text = tool_end
        .0
        .content
        .iter()
        .find_map(|content| match content {
            UserContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("expected validation error text");
    assert!(error_text.contains("Validation failed for tool \"echo\""));
    assert!(error_text.contains("value: must have required property 'value'"));
    assert!(error_text.contains("Received arguments:\n{}"));

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 4);
    assert!(is_standard_tool_result_message(&messages[2]));
    assert!(is_standard_assistant_message(&messages[3]));
}

#[tokio::test]
async fn validation_coerces_string_numbers_before_tool_execution() {
    let executed_args = Arc::new(Mutex::new(Vec::new()));
    let tool = AgentTool::new(
        ToolDefinition {
            name: "counter".into(),
            description: "Count things".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "count": { "type": "integer" }
                },
                "required": ["count"]
            }),
        },
        {
            let executed_args = executed_args.clone();
            move |_tool_call_id, args, _signal| {
                let executed_args = executed_args.clone();
                async move {
                    executed_args.lock().unwrap().push(args.clone());
                    Ok(AgentToolResult {
                        content: vec![UserContent::Text {
                            text: format!("counted: {}", args["count"]),
                        }],
                        details: args,
                    })
                }
            }
        },
    );

    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(tool);

    let streamer = Arc::new(ScriptedStreamer::new(vec![
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::ToolUse,
            message: assistant_tool_call_message(
                "tool-1",
                "counter",
                serde_json::Map::from_iter([(String::from("count"), Value::String("42".into()))]),
                20,
            ),
        })],
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("done", StopReason::Stop, 30),
        })],
    ])) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model()).with_streamer(streamer),
    );

    let events = collect_events(stream).await.unwrap();
    assert_eq!(
        executed_args.lock().unwrap().clone(),
        vec![json!({ "count": 42 })]
    );

    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                result, is_error, ..
            } => Some((result.clone(), *is_error)),
            _ => None,
        })
        .expect("expected tool execution end event");
    assert!(!tool_end.1);
    assert_eq!(
        tool_end.0.content,
        vec![UserContent::Text {
            text: String::from("counted: 42"),
        }]
    );
}

#[tokio::test]
async fn after_hook_can_override_tool_result_fields() {
    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(AgentTool::new(
        ToolDefinition {
            name: "echo".into(),
            description: "Echo raw result".into(),
            parameters: json!({ "type": "object" }),
        },
        |_tool_call_id, _args, _signal| async move {
            Ok(AgentToolResult {
                content: vec![UserContent::Text { text: "raw".into() }],
                details: json!({ "raw": true }),
            })
        },
    ));

    let streamer = Arc::new(ScriptedStreamer::new(vec![
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::ToolUse,
            message: assistant_tool_call_message(
                "tool-1",
                "echo",
                serde_json::Map::from_iter([(
                    String::from("value"),
                    Value::String("hello".into()),
                )]),
                20,
            ),
        })],
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("done", StopReason::Stop, 30),
        })],
    ])) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_after_tool_call(|_after, _signal| async move {
                Some(AfterToolCallResult {
                    content: Some(vec![UserContent::Text {
                        text: "audited".into(),
                    }]),
                    details: Some(json!({ "audited": true })),
                    is_error: Some(true),
                })
            }),
    );

    let events = collect_events(stream).await.unwrap();
    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                result, is_error, ..
            } => Some((result.clone(), *is_error)),
            _ => None,
        })
        .expect("expected tool execution end event");

    assert!(tool_end.1);
    assert_eq!(
        tool_end.0.content,
        vec![UserContent::Text {
            text: "audited".into(),
        }]
    );
    assert_eq!(tool_end.0.details, json!({ "audited": true }));

    let tool_result_message = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::MessageEnd {
                message:
                    AgentMessage::Standard(Message::ToolResult {
                        content, is_error, ..
                    }),
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("expected tool result message");
    assert!(tool_result_message.1);
    assert_eq!(
        tool_result_message.0,
        vec![UserContent::Text {
            text: "audited".into(),
        }]
    );
}

#[tokio::test]
async fn before_hook_can_block_tool_execution_with_reason() {
    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(echo_tool());

    let streamer = Arc::new(ScriptedStreamer::new(vec![
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::ToolUse,
            message: assistant_tool_call_message(
                "tool-1",
                "echo",
                serde_json::Map::from_iter([(
                    String::from("value"),
                    Value::String("hello".into()),
                )]),
                20,
            ),
        })],
        vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message: assistant_message("done", StopReason::Stop, 30),
        })],
    ])) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_before_tool_call(|_before, _signal| async move {
                Some(BeforeToolCallResult {
                    block: true,
                    reason: Some("blocked by policy".into()),
                })
            }),
    );

    let events = collect_events(stream).await.unwrap();
    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                result, is_error, ..
            } => Some((result.clone(), *is_error)),
            _ => None,
        })
        .expect("expected tool execution end event");
    assert!(tool_end.1);
    assert_eq!(
        tool_end.0.content,
        vec![UserContent::Text {
            text: "blocked by policy".into(),
        }]
    );
}

#[tokio::test]
async fn steering_messages_are_injected_after_all_tool_results() {
    let mut context = AgentContext::new("You are helpful.");
    context.tools.push(echo_tool());

    let queued_message = user_message("interrupt", 25);
    let steering_polls = Arc::new(Mutex::new(0usize));
    let call_index = Arc::new(Mutex::new(0usize));
    let saw_interrupt_in_context = Arc::new(Mutex::new(false));

    let streamer = Arc::new({
        let call_index = call_index.clone();
        let saw_interrupt_in_context = saw_interrupt_in_context.clone();
        move |_model: Model,
              context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let current_call = {
                let mut call_index = call_index.lock().unwrap();
                let current_call = *call_index;
                if current_call == 1 {
                    *saw_interrupt_in_context.lock().unwrap() = context
                        .messages
                        .iter()
                        .any(|message| llm_message_has_user_text(message, "interrupt"));
                }
                *call_index += 1;
                current_call
            };

            let event = if current_call == 0 {
                AssistantEvent::Done {
                    reason: StopReason::ToolUse,
                    message: AssistantMessage {
                        role: "assistant".into(),
                        content: vec![
                            AssistantContent::ToolCall {
                                id: "tool-1".into(),
                                name: "echo".into(),
                                arguments: serde_json::Map::from_iter([(
                                    String::from("value"),
                                    Value::String("first".into()),
                                )])
                                .into_iter()
                                .collect(),
                                thought_signature: None,
                            },
                            AssistantContent::ToolCall {
                                id: "tool-2".into(),
                                name: "echo".into(),
                                arguments: serde_json::Map::from_iter([(
                                    String::from("value"),
                                    Value::String("second".into()),
                                )])
                                .into_iter()
                                .collect(),
                                thought_signature: None,
                            },
                        ],
                        api: "faux:test".into(),
                        provider: "faux".into(),
                        model: "mock".into(),
                        response_id: None,
                        usage: usage(),
                        stop_reason: StopReason::ToolUse,
                        error_message: None,
                        timestamp: 20,
                    },
                }
            } else {
                AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message: assistant_message("done", StopReason::Stop, 30),
                }
            };

            Ok(Box::pin(stream::iter(vec![Ok(event)])))
        }
    }) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("run tool", 10).into()],
        context,
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_get_steering_messages({
                let steering_polls = steering_polls.clone();
                let queued_message = queued_message.clone();
                move || {
                    let steering_polls = steering_polls.clone();
                    let queued_message = queued_message.clone();
                    async move {
                        let mut steering_polls = steering_polls.lock().unwrap();
                        let current_poll = *steering_polls;
                        *steering_polls += 1;
                        if current_poll == 1 {
                            vec![queued_message.clone().into()]
                        } else {
                            Vec::new()
                        }
                    }
                }
            }),
    );

    let events = collect_events(stream).await.unwrap();
    let event_sequence = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageStart {
                message: AgentMessage::Standard(Message::ToolResult { tool_call_id, .. }),
            } => Some(format!("tool:{tool_call_id}")),
            AgentEvent::MessageStart { message } if message_has_user_text(message, "interrupt") => {
                Some(String::from("interrupt"))
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        event_sequence,
        vec!["tool:tool-1", "tool:tool-2", "interrupt"]
    );
    assert!(*saw_interrupt_in_context.lock().unwrap());

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 6);
    assert!(message_has_user_text(&messages[4], "interrupt"));
}

#[tokio::test]
async fn follow_up_messages_resume_after_agent_would_otherwise_stop() {
    let queued_message = user_message("follow up", 25);
    let follow_up_polls = Arc::new(Mutex::new(0usize));
    let call_index = Arc::new(Mutex::new(0usize));
    let saw_follow_up_in_context = Arc::new(Mutex::new(false));

    let streamer = Arc::new({
        let call_index = call_index.clone();
        let saw_follow_up_in_context = saw_follow_up_in_context.clone();
        move |_model: Model,
              context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let current_call = {
                let mut call_index = call_index.lock().unwrap();
                let current_call = *call_index;
                if current_call == 1 {
                    *saw_follow_up_in_context.lock().unwrap() = context
                        .messages
                        .iter()
                        .any(|message| llm_message_has_user_text(message, "follow up"));
                }
                *call_index += 1;
                current_call
            };

            let event = if current_call == 0 {
                AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message: assistant_message("first", StopReason::Stop, 20),
                }
            } else {
                AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message: assistant_message("second", StopReason::Stop, 30),
                }
            };

            Ok(Box::pin(stream::iter(vec![Ok(event)])))
        }
    }) as Arc<dyn AssistantStreamer>;

    let stream = agent_loop(
        vec![user_message("start", 10).into()],
        AgentContext::new("You are helpful."),
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_get_follow_up_messages({
                let follow_up_polls = follow_up_polls.clone();
                let queued_message = queued_message.clone();
                move || {
                    let follow_up_polls = follow_up_polls.clone();
                    let queued_message = queued_message.clone();
                    async move {
                        let mut follow_up_polls = follow_up_polls.lock().unwrap();
                        let current_poll = *follow_up_polls;
                        *follow_up_polls += 1;
                        if current_poll == 0 {
                            vec![queued_message.clone().into()]
                        } else {
                            Vec::new()
                        }
                    }
                }
            }),
    );

    let events = collect_events(stream).await.unwrap();
    assert!(*saw_follow_up_in_context.lock().unwrap());

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 4);
    assert!(is_standard_user_message(&messages[0]));
    assert!(is_standard_assistant_message(&messages[1]));
    assert!(message_has_user_text(&messages[2], "follow up"));
    assert!(is_standard_assistant_message(&messages[3]));
}

#[tokio::test]
async fn convert_to_llm_can_map_custom_messages_into_llm_requests() {
    let seen_llm_messages = Arc::new(Mutex::new(Vec::new()));
    let streamer = Arc::new({
        let seen_llm_messages = seen_llm_messages.clone();
        move |_model: Model,
              context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            *seen_llm_messages.lock().unwrap() = context.messages.clone();
            Ok(Box::pin(stream::iter(vec![Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message: assistant_message("done", StopReason::Stop, 20),
            })])))
        }
    }) as Arc<dyn AssistantStreamer>;

    let custom_prompt = AgentMessage::from(CustomAgentMessage::new(
        "custom",
        json!({ "text": "from custom" }),
        10,
    ));

    let stream = agent_loop(
        vec![custom_prompt],
        AgentContext::new("You are helpful."),
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_convert_to_llm(|messages| async move {
                convert_custom_text_messages_to_llm(messages)
            }),
    );

    let events = collect_events(stream).await.unwrap();
    let llm_messages = seen_llm_messages.lock().unwrap().clone();
    assert_eq!(llm_messages.len(), 1);
    assert!(llm_message_has_user_text(&llm_messages[0], "from custom"));

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0], AgentMessage::Custom(_)));
    assert!(is_standard_assistant_message(&messages[1]));
}

#[tokio::test]
async fn transform_context_runs_before_convert_to_llm() {
    let seen_converted_messages = Arc::new(Mutex::new(Vec::<AgentMessage>::new()));
    let streamer = Arc::new(
        |_model: Model,
         _context: Context,
         _options: StreamOptions|
         -> Result<AssistantEventStream, AiError> {
            Ok(Box::pin(stream::iter(vec![Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message: assistant_message("done", StopReason::Stop, 30),
            })])))
        },
    ) as Arc<dyn AssistantStreamer>;

    let mut context = AgentContext::new("You are helpful.");
    context.messages = vec![
        user_message("old user", 1).into(),
        Message::Assistant {
            content: vec![AssistantContent::Text {
                text: "old assistant".into(),
                text_signature: None,
            }],
            api: "faux:test".into(),
            provider: "faux".into(),
            model: "mock".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 2,
        }
        .into(),
        user_message("newer user", 3).into(),
        Message::Assistant {
            content: vec![AssistantContent::Text {
                text: "newer assistant".into(),
                text_signature: None,
            }],
            api: "faux:test".into(),
            provider: "faux".into(),
            model: "mock".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 4,
        }
        .into(),
    ];

    let stream = agent_loop(
        vec![user_message("latest", 5).into()],
        context,
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_transform_context(|messages, _signal| async move {
                let keep_from = messages.len().saturating_sub(2);
                messages.into_iter().skip(keep_from).collect()
            })
            .with_convert_to_llm({
                let seen_converted_messages = seen_converted_messages.clone();
                move |messages| {
                    let seen_converted_messages = seen_converted_messages.clone();
                    async move {
                        *seen_converted_messages.lock().unwrap() = messages.clone();
                        convert_custom_text_messages_to_llm(messages)
                    }
                }
            }),
    );

    let _events = collect_events(stream).await.unwrap();
    let seen_converted_messages = seen_converted_messages.lock().unwrap().clone();
    assert_eq!(seen_converted_messages.len(), 2);
    assert!(is_standard_assistant_message(&seen_converted_messages[0]));
    assert!(is_standard_user_message(&seen_converted_messages[1]));
}

#[tokio::test]
async fn continue_allows_custom_tail_when_convert_to_llm_maps_it_to_user() {
    let streamer = Arc::new(
        |_model: Model,
         context: Context,
         _options: StreamOptions|
         -> Result<AssistantEventStream, AiError> {
            assert_eq!(context.messages.len(), 1);
            assert!(llm_message_has_user_text(
                &context.messages[0],
                "resume from custom"
            ));
            Ok(Box::pin(stream::iter(vec![Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message: assistant_message("done", StopReason::Stop, 20),
            })])))
        },
    ) as Arc<dyn AssistantStreamer>;

    let mut context = AgentContext::new("You are helpful.");
    context.messages.push(
        CustomAgentMessage::new("custom", json!({ "text": "resume from custom" }), 10).into(),
    );

    let stream = agent_loop_continue(
        context,
        AgentLoopConfig::new(model())
            .with_streamer(streamer)
            .with_convert_to_llm(|messages| async move {
                convert_custom_text_messages_to_llm(messages)
            }),
    )
    .unwrap();

    let events = collect_events(stream).await.unwrap();
    let messages = final_messages(&events);
    assert_eq!(messages.len(), 1);
    assert!(is_standard_assistant_message(&messages[0]));
}

#[tokio::test]
async fn default_streamer_runs_against_pi_ai_faux_provider() {
    let registration = register_faux_provider(RegisterFauxProviderOptions::default());
    registration.set_responses(vec![FauxResponse::text("4")]);
    let model = registration.get_model(None).unwrap();

    let stream = agent_loop(
        vec![user_message("What is 2+2?", 10).into()],
        AgentContext::new("You are helpful."),
        AgentLoopConfig::new(model),
    );

    let events = collect_events(stream).await.unwrap();
    let messages = final_messages(&events);

    assert_eq!(messages.len(), 2);
    match &messages[1] {
        AgentMessage::Standard(Message::Assistant { content, .. }) => {
            assert_eq!(content.len(), 1);
            assert_eq!(
                content[0],
                AssistantContent::Text {
                    text: "4".into(),
                    text_signature: None,
                }
            );
        }
        other => panic!("expected assistant message, got {other:?}"),
    }

    registration.unregister();
}
