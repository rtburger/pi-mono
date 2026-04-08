use futures::{StreamExt, stream};
use pi_agent::{
    AfterToolCallResult, AgentContext, AgentEvent, AgentLoopConfig, AgentState, AgentTool,
    AgentToolError, AgentToolResult, AssistantStreamer, BeforeToolCallResult, ThinkingLevel,
    agent_loop, agent_loop_continue,
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

fn final_messages(events: &[AgentEvent]) -> Vec<Message> {
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
        vec![user_message("Hi", 10)],
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
    assert!(matches!(messages[0], Message::User { .. }));
    assert!(matches!(messages[1], Message::Assistant { .. }));
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
    assistant_tail_context.messages.push(Message::Assistant {
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
    });
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
    context.messages.push(user_message("resume", 5));

    let stream = agent_loop_continue(context, config.with_streamer(streamer)).unwrap();
    let events = collect_events(stream).await.unwrap();

    let message_end_count = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::MessageEnd { .. }))
        .count();
    assert_eq!(message_end_count, 1);

    let messages = final_messages(&events);
    assert_eq!(messages.len(), 1);
    assert!(matches!(messages[0], Message::Assistant { .. }));
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
        vec![user_message("Hi", 10)],
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
        vec![user_message("run tool", 10)],
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
    assert!(matches!(messages[0], Message::User { .. }));
    assert!(matches!(messages[1], Message::Assistant { .. }));
    assert!(matches!(messages[2], Message::ToolResult { .. }));
    assert!(matches!(messages[3], Message::Assistant { .. }));
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
        vec![user_message("run tool", 10)],
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
        vec![user_message("run tool", 10)],
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
                    Message::ToolResult {
                        content, is_error, ..
                    },
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
        vec![user_message("run tool", 10)],
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
async fn default_streamer_runs_against_pi_ai_faux_provider() {
    let registration = register_faux_provider(RegisterFauxProviderOptions::default());
    registration.set_responses(vec![FauxResponse::text("4")]);
    let model = registration.get_model(None).unwrap();

    let stream = agent_loop(
        vec![user_message("What is 2+2?", 10)],
        AgentContext::new("You are helpful."),
        AgentLoopConfig::new(model),
    );

    let events = collect_events(stream).await.unwrap();
    let messages = final_messages(&events);

    assert_eq!(messages.len(), 2);
    match &messages[1] {
        Message::Assistant { content, .. } => {
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
