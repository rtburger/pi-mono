use async_stream::try_stream;
use futures::stream;
use pi_agent::{
    Agent, AgentError, AgentEvent, AgentState, AgentTool, AgentToolError, AgentToolResult,
};
use pi_ai::{AiError, AssistantEventStream, StreamOptions};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason,
    ToolDefinition, Usage, UserContent,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, time::sleep};

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

#[tokio::test]
async fn prompt_waits_for_async_agent_end_listeners_and_wait_for_idle() {
    let listener_entered = Arc::new(Notify::new());
    let release_listener = Arc::new(Notify::new());
    let streamer = Arc::new(
        |_model: Model,
         _context: Context,
         _options: StreamOptions|
         -> Result<AssistantEventStream, AiError> {
            let message = assistant_message("ok", StopReason::Stop, 20);
            Ok(Box::pin(try_stream! {
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message,
                };
            }))
        },
    );

    let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
    let entered = listener_entered.clone();
    let release = release_listener.clone();
    agent.subscribe(move |event, _signal| {
        let entered = entered.clone();
        let release = release.clone();
        async move {
            if matches!(event, AgentEvent::AgentEnd { .. }) {
                entered.notify_waiters();
                release.notified().await;
            }
        }
    });

    let prompt_agent = agent.clone();
    let prompt_task = tokio::spawn(async move { prompt_agent.prompt_text("hello").await });

    listener_entered.notified().await;
    assert!(agent.state().is_streaming);

    let idle_agent = agent.clone();
    let idle_task = tokio::spawn(async move {
        idle_agent.wait_for_idle().await;
    });

    sleep(Duration::from_millis(10)).await;
    assert!(!prompt_task.is_finished());
    assert!(!idle_task.is_finished());

    release_listener.notify_waiters();

    assert!(prompt_task.await.unwrap().is_ok());
    idle_task.await.unwrap();
    assert!(!agent.state().is_streaming);
    assert_eq!(agent.state().messages.len(), 2);
}

#[tokio::test]
async fn abort_marks_signal_and_records_aborted_message() {
    let streamer = Arc::new(
        |_model: Model,
         _context: Context,
         options: StreamOptions|
         -> Result<AssistantEventStream, AiError> {
            let mut signal = options.signal.expect("agent should inject abort signal");
            let partial = assistant_message("", StopReason::Stop, 20);
            Ok(Box::pin(try_stream! {
                yield AssistantEvent::Start {
                    partial: partial.clone(),
                };

                while !*signal.borrow() {
                    signal.changed().await.expect("abort signal sender should stay alive");
                }

                let mut aborted = assistant_message("", StopReason::Aborted, 21);
                aborted.error_message = Some("Request was aborted".into());
                yield AssistantEvent::Error {
                    reason: StopReason::Aborted,
                    error: aborted,
                };
            }))
        },
    );

    let abort_seen = Arc::new(Mutex::new(false));
    let abort_seen_listener = abort_seen.clone();
    let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
    agent.subscribe(move |event, signal| {
        let abort_seen_listener = abort_seen_listener.clone();
        async move {
            if matches!(event, AgentEvent::AgentStart) {
                *abort_seen_listener.lock().unwrap() = *signal.borrow();
            }
        }
    });

    let prompt_agent = agent.clone();
    let prompt_task = tokio::spawn(async move { prompt_agent.prompt_text("hello").await });

    sleep(Duration::from_millis(10)).await;
    agent.abort();

    assert!(prompt_task.await.unwrap().is_ok());
    let state = agent.state();
    assert!(!state.is_streaming);
    assert_eq!(state.messages.len(), 2);
    match state.messages.last().unwrap() {
        Message::Assistant {
            stop_reason,
            error_message,
            ..
        } => {
            assert_eq!(stop_reason, &StopReason::Aborted);
            assert_eq!(error_message.as_deref(), Some("Request was aborted"));
        }
        other => panic!("expected assistant message, got {other:?}"),
    }
    assert!(!*abort_seen.lock().unwrap());
}

#[tokio::test]
async fn continue_uses_existing_state_and_updates_transcript() {
    let streamer = Arc::new(
        |_model: Model,
         _context: Context,
         _options: StreamOptions|
         -> Result<AssistantEventStream, AiError> {
            let message = assistant_message("continued", StopReason::Stop, 20);
            Ok(Box::pin(try_stream! {
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message,
                };
            }))
        },
    );

    let mut initial_state = AgentState::new(model());
    initial_state.messages.push(user_message("resume", 10));
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    agent.r#continue().await.unwrap();

    let state = agent.state();
    assert_eq!(state.messages.len(), 2);
    assert!(matches!(state.messages[0], Message::User { .. }));
    assert!(matches!(state.messages[1], Message::Assistant { .. }));
}

#[tokio::test]
async fn prompt_materializes_internal_stream_errors_as_assistant_failure_messages() {
    let streamer = Arc::new(
        |_model: Model,
         _context: Context,
         _options: StreamOptions|
         -> Result<AssistantEventStream, AiError> {
            Ok(Box::pin(stream::once(async {
                Err(AiError::Message("boom".into()))
            })))
        },
    );

    let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
    agent.prompt_text("hello").await.unwrap();

    let state = agent.state();
    assert_eq!(state.messages.len(), 2);
    match state.messages.last().unwrap() {
        Message::Assistant {
            stop_reason,
            error_message,
            ..
        } => {
            assert_eq!(stop_reason, &StopReason::Error);
            assert_eq!(error_message.as_deref(), Some("boom"));
        }
        other => panic!("expected assistant failure message, got {other:?}"),
    }
    assert_eq!(state.error_message.as_deref(), Some("boom"));
}

#[tokio::test]
async fn rejects_prompt_and_continue_while_active() {
    let blocker = Arc::new(Notify::new());
    let streamer = Arc::new({
        let blocker = blocker.clone();
        move |_model: Model,
              _context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let blocker = blocker.clone();
            let message = assistant_message("done", StopReason::Stop, 20);
            Ok(Box::pin(try_stream! {
                blocker.notified().await;
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message,
                };
            }))
        }
    });

    let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
    let running_agent = agent.clone();
    let running_prompt = tokio::spawn(async move { running_agent.prompt_text("first").await });

    sleep(Duration::from_millis(10)).await;
    assert_eq!(
        agent.prompt_text("second").await.unwrap_err(),
        AgentError::AlreadyProcessingPrompt
    );
    assert_eq!(
        agent.r#continue().await.unwrap_err(),
        AgentError::AlreadyProcessingContinue
    );

    blocker.notify_waiters();
    assert!(running_prompt.await.unwrap().is_ok());
}

#[tokio::test]
async fn wrapper_runs_tool_flow_and_tracks_pending_tool_calls() {
    let calls = Arc::new(Mutex::new(VecDeque::from([
        assistant_tool_call_message(
            "tool-1",
            "echo",
            serde_json::Map::from_iter([(String::from("value"), Value::String("hello".into()))]),
            20,
        ),
        assistant_message("done", StopReason::Stop, 30),
    ])));

    let streamer = Arc::new({
        let calls = calls.clone();
        move |_model: Model,
              _context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let message = calls
                .lock()
                .unwrap()
                .pop_front()
                .expect("expected scripted assistant message");
            let reason = message.stop_reason.clone();
            Ok(Box::pin(try_stream! {
                yield AssistantEvent::Done { reason, message };
            }))
        }
    });

    let mut initial_state = AgentState::new(model());
    initial_state.tools.push(echo_tool());
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    let pending_snapshots = Arc::new(Mutex::new(Vec::new()));
    let pending_agent = agent.clone();
    let pending_snapshots_listener = pending_snapshots.clone();
    agent.subscribe(move |event, _signal| {
        let pending_agent = pending_agent.clone();
        let pending_snapshots_listener = pending_snapshots_listener.clone();
        async move {
            if matches!(
                event,
                AgentEvent::ToolExecutionStart { .. } | AgentEvent::ToolExecutionEnd { .. }
            ) {
                pending_snapshots_listener.lock().unwrap().push(
                    pending_agent
                        .state()
                        .pending_tool_calls
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            }
        }
    });

    agent.prompt_text("run tool").await.unwrap();

    let state = agent.state();
    assert_eq!(
        pending_snapshots.lock().unwrap().clone(),
        vec![vec![String::from("tool-1")], Vec::<String>::new()]
    );
    assert!(state.pending_tool_calls.is_empty());
    assert_eq!(state.messages.len(), 4);
    assert!(matches!(state.messages[0], Message::User { .. }));
    assert!(matches!(state.messages[1], Message::Assistant { .. }));
    assert!(matches!(state.messages[2], Message::ToolResult { .. }));
    assert!(matches!(state.messages[3], Message::Assistant { .. }));
}

#[tokio::test]
async fn wrapper_forwards_before_and_after_tool_hooks() {
    let calls = Arc::new(Mutex::new(VecDeque::from([
        assistant_tool_call_message(
            "tool-1",
            "echo",
            serde_json::Map::from_iter([(String::from("alias"), Value::String("hello".into()))]),
            20,
        ),
        assistant_message("done", StopReason::Stop, 30),
    ])));

    let streamer = Arc::new({
        let calls = calls.clone();
        move |_model: Model,
              _context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let message = calls
                .lock()
                .unwrap()
                .pop_front()
                .expect("expected scripted assistant message");
            let reason = message.stop_reason.clone();
            Ok(Box::pin(try_stream! {
                yield AssistantEvent::Done { reason, message };
            }))
        }
    });

    let mut initial_state = AgentState::new(model());
    initial_state.tools.push(
        AgentTool::new(
            ToolDefinition {
                name: "echo".into(),
                description: "Echo prepared args".into(),
                parameters: json!({ "type": "object" }),
            },
            |_tool_call_id, args, _signal| async move {
                let value = args
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AgentToolError::message("missing tool arg: value"))?;
                Ok(AgentToolResult {
                    content: vec![UserContent::Text {
                        text: format!("raw: {value}"),
                    }],
                    details: json!({ "value": value }),
                })
            },
        )
        .with_prepare_arguments(|args| {
            let alias = args
                .get("alias")
                .and_then(Value::as_str)
                .unwrap_or_default();
            json!({ "value": alias })
        }),
    );
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    agent.set_before_tool_call(|before, _signal| async move {
        *before.args.lock().unwrap() = json!({ "value": "mutated" });
        None
    });
    agent.set_after_tool_call(|_after, _signal| async move {
        Some(pi_agent::AfterToolCallResult {
            content: Some(vec![UserContent::Text {
                text: "audited".into(),
            }]),
            details: Some(json!({ "audited": true })),
            is_error: Some(true),
        })
    });

    agent.prompt_text("run tool").await.unwrap();

    let state = agent.state();
    let tool_result = state
        .messages
        .iter()
        .find_map(|message| match message {
            Message::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("expected tool result message");

    assert!(tool_result.1);
    assert_eq!(
        tool_result.0,
        vec![UserContent::Text {
            text: "audited".into(),
        }]
    );
}
