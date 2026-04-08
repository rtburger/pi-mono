use async_stream::try_stream;
use futures::stream;
use pi_agent::{
    Agent, AgentError, AgentEvent, AgentMessage, AgentState, AgentTool, AgentToolError,
    AgentToolResult, CustomAgentMessage, QueueMode,
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
        AgentMessage::Standard(Message::Assistant {
            stop_reason,
            error_message,
            ..
        }) => {
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
    initial_state
        .messages
        .push(user_message("resume", 10).into());
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    agent.r#continue().await.unwrap();

    let state = agent.state();
    assert_eq!(state.messages.len(), 2);
    assert!(is_standard_user_message(&state.messages[0]));
    assert!(is_standard_assistant_message(&state.messages[1]));
}

#[tokio::test]
async fn queue_helpers_do_not_touch_transcript_until_drained() {
    let agent = Agent::new(AgentState::new(model()));

    assert_eq!(agent.steering_mode(), QueueMode::OneAtATime);
    assert_eq!(agent.follow_up_mode(), QueueMode::OneAtATime);

    agent.set_steering_mode(QueueMode::All);
    agent.set_follow_up_mode(QueueMode::All);
    assert_eq!(agent.steering_mode(), QueueMode::All);
    assert_eq!(agent.follow_up_mode(), QueueMode::All);

    agent.steer(user_message("steering", 1));
    agent.follow_up(user_message("follow up", 2));

    assert!(agent.has_queued_messages());
    assert!(agent.state().messages.is_empty());

    agent.clear_steering_queue();
    assert!(agent.has_queued_messages());

    agent.clear_follow_up_queue();
    assert!(!agent.has_queued_messages());
}

#[tokio::test]
async fn continue_drains_all_steering_messages_when_mode_is_all() {
    let response_count = Arc::new(Mutex::new(0usize));
    let streamer = Arc::new({
        let response_count = response_count.clone();
        move |_model: Model,
              _context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let response_count = response_count.clone();
            Ok(Box::pin(try_stream! {
                let next_count = {
                    let mut response_count = response_count.lock().unwrap();
                    *response_count += 1;
                    *response_count
                };
                let message = assistant_message(
                    &format!("processed {next_count}"),
                    StopReason::Stop,
                    20 + next_count as u64,
                );
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message,
                };
            }))
        }
    });

    let mut initial_state = AgentState::new(model());
    initial_state
        .messages
        .push(user_message("initial", 10).into());
    initial_state.messages.push(
        Message::Assistant {
            content: vec![AssistantContent::Text {
                text: "initial response".into(),
                text_signature: None,
            }],
            api: "faux:test".into(),
            provider: "faux".into(),
            model: "mock".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 11,
        }
        .into(),
    );
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    agent.set_steering_mode(QueueMode::All);
    agent.steer(user_message("steering 1", 12));
    agent.steer(user_message("steering 2", 13));

    agent.r#continue().await.unwrap();

    let state = agent.state();
    assert_eq!(*response_count.lock().unwrap(), 1);
    assert_eq!(state.messages.len(), 5);
    assert!(message_has_user_text(&state.messages[2], "steering 1"));
    assert!(message_has_user_text(&state.messages[3], "steering 2"));
    assert!(is_standard_assistant_message(&state.messages[4]));
}

#[tokio::test]
async fn continue_uses_queued_follow_up_messages_from_assistant_tail() {
    let streamer = Arc::new(
        |_model: Model,
         _context: Context,
         _options: StreamOptions|
         -> Result<AssistantEventStream, AiError> {
            let message = assistant_message("processed", StopReason::Stop, 20);
            Ok(Box::pin(try_stream! {
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message,
                };
            }))
        },
    );

    let mut initial_state = AgentState::new(model());
    initial_state
        .messages
        .push(user_message("initial", 10).into());
    initial_state.messages.push(
        Message::Assistant {
            content: vec![AssistantContent::Text {
                text: "initial response".into(),
                text_signature: None,
            }],
            api: "faux:test".into(),
            provider: "faux".into(),
            model: "mock".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 11,
        }
        .into(),
    );
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    agent.follow_up(user_message("queued follow up", 12));

    agent.r#continue().await.unwrap();

    let state = agent.state();
    assert_eq!(state.messages.len(), 4);
    assert!(message_has_user_text(
        &state.messages[2],
        "queued follow up"
    ));
    assert!(is_standard_assistant_message(&state.messages[3]));
}

#[tokio::test]
async fn continue_keeps_steering_queue_one_at_a_time_from_assistant_tail() {
    let response_count = Arc::new(Mutex::new(0usize));
    let streamer = Arc::new({
        let response_count = response_count.clone();
        move |_model: Model,
              _context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let response_count = response_count.clone();
            Ok(Box::pin(try_stream! {
                let next_count = {
                    let mut response_count = response_count.lock().unwrap();
                    *response_count += 1;
                    *response_count
                };
                let message = assistant_message(
                    &format!("processed {next_count}"),
                    StopReason::Stop,
                    20 + next_count as u64,
                );
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message,
                };
            }))
        }
    });

    let mut initial_state = AgentState::new(model());
    initial_state
        .messages
        .push(user_message("initial", 10).into());
    initial_state.messages.push(
        Message::Assistant {
            content: vec![AssistantContent::Text {
                text: "initial response".into(),
                text_signature: None,
            }],
            api: "faux:test".into(),
            provider: "faux".into(),
            model: "mock".into(),
            response_id: None,
            usage: usage(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 11,
        }
        .into(),
    );
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    agent.steer(user_message("steering 1", 12));
    agent.steer(user_message("steering 2", 13));

    agent.r#continue().await.unwrap();

    let state = agent.state();
    assert_eq!(*response_count.lock().unwrap(), 2);
    assert_eq!(state.messages.len(), 6);
    assert!(message_has_user_text(&state.messages[2], "steering 1"));
    assert!(is_standard_assistant_message(&state.messages[3]));
    assert!(message_has_user_text(&state.messages[4], "steering 2"));
    assert!(is_standard_assistant_message(&state.messages[5]));
}

#[tokio::test]
async fn prompt_drains_all_follow_up_messages_when_mode_is_all() {
    let response_count = Arc::new(Mutex::new(0usize));
    let saw_follow_ups_in_context = Arc::new(Mutex::new(false));
    let streamer = Arc::new({
        let response_count = response_count.clone();
        let saw_follow_ups_in_context = saw_follow_ups_in_context.clone();
        move |_model: Model,
              context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            let response_count = response_count.clone();
            let saw_follow_ups_in_context = saw_follow_ups_in_context.clone();
            Ok(Box::pin(try_stream! {
                let next_count = {
                    let mut response_count = response_count.lock().unwrap();
                    *response_count += 1;
                    *response_count
                };
                if next_count == 2 {
                    *saw_follow_ups_in_context.lock().unwrap() = context
                        .messages
                        .iter()
                        .any(|message| matches!(message, Message::User { content, .. } if content.iter().any(|block| matches!(block, UserContent::Text { text } if text == "follow up 1"))))
                        && context
                            .messages
                            .iter()
                            .any(|message| matches!(message, Message::User { content, .. } if content.iter().any(|block| matches!(block, UserContent::Text { text } if text == "follow up 2"))));
                }
                let message = assistant_message(
                    &format!("processed {next_count}"),
                    StopReason::Stop,
                    20 + next_count as u64,
                );
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message,
                };
            }))
        }
    });

    let agent = Agent::with_parts(AgentState::new(model()), streamer, StreamOptions::default());
    agent.set_follow_up_mode(QueueMode::All);
    agent.follow_up(user_message("follow up 1", 11));
    agent.follow_up(user_message("follow up 2", 12));

    agent.prompt_text("start").await.unwrap();

    let state = agent.state();
    assert_eq!(*response_count.lock().unwrap(), 2);
    assert!(*saw_follow_ups_in_context.lock().unwrap());
    assert_eq!(state.messages.len(), 5);
    assert!(is_standard_user_message(&state.messages[0]));
    assert!(is_standard_assistant_message(&state.messages[1]));
    assert!(message_has_user_text(&state.messages[2], "follow up 1"));
    assert!(message_has_user_text(&state.messages[3], "follow up 2"));
    assert!(is_standard_assistant_message(&state.messages[4]));
}

#[tokio::test]
async fn wrapper_forwards_transform_context_and_convert_to_llm_for_custom_messages() {
    let seen_llm_messages = Arc::new(Mutex::new(Vec::new()));
    let streamer = Arc::new({
        let seen_llm_messages = seen_llm_messages.clone();
        move |_model: Model,
              context: Context,
              _options: StreamOptions|
              -> Result<AssistantEventStream, AiError> {
            *seen_llm_messages.lock().unwrap() = context.messages.clone();
            Ok(Box::pin(try_stream! {
                yield AssistantEvent::Done {
                    reason: StopReason::Stop,
                    message: assistant_message("done", StopReason::Stop, 20),
                };
            }))
        }
    });

    let mut initial_state = AgentState::new(model());
    initial_state
        .messages
        .push(user_message("old user", 1).into());
    initial_state.messages.push(
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
    );
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());
    agent.set_transform_context(|messages, _signal| async move {
        messages.into_iter().skip(2).collect()
    });
    agent.set_convert_to_llm(
        |messages| async move { convert_custom_text_messages_to_llm(messages) },
    );

    agent
        .prompt(CustomAgentMessage::new(
            "custom",
            json!({ "text": "from wrapper custom" }),
            10,
        ))
        .await
        .unwrap();

    let llm_messages = seen_llm_messages.lock().unwrap().clone();
    assert_eq!(llm_messages.len(), 1);
    assert!(matches!(
        &llm_messages[0],
        Message::User { content, .. }
            if content.iter().any(|block| matches!(block, UserContent::Text { text } if text == "from wrapper custom"))
    ));

    let state = agent.state();
    assert_eq!(state.messages.len(), 4);
    assert!(
        matches!(&state.messages[2], AgentMessage::Custom(CustomAgentMessage { role, .. }) if role == "custom")
    );
    assert!(is_standard_assistant_message(&state.messages[3]));
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
        AgentMessage::Standard(Message::Assistant {
            stop_reason,
            error_message,
            ..
        }) => {
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
    assert!(is_standard_user_message(&state.messages[0]));
    assert!(is_standard_assistant_message(&state.messages[1]));
    assert!(is_standard_tool_result_message(&state.messages[2]));
    assert!(is_standard_assistant_message(&state.messages[3]));
}

#[tokio::test]
async fn wrapper_forwards_tool_execution_updates_to_listeners() {
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
    initial_state.tools.push(AgentTool::new_with_updates(
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
            }
            Ok(AgentToolResult {
                content: vec![UserContent::Text {
                    text: format!("final: {value}"),
                }],
                details: json!({ "done": true }),
            })
        },
    ));
    let agent = Agent::with_parts(initial_state, streamer, StreamOptions::default());

    let update_snapshots = Arc::new(Mutex::new(Vec::new()));
    let update_snapshots_listener = update_snapshots.clone();
    let update_agent = agent.clone();
    agent.subscribe(move |event, _signal| {
        let update_snapshots_listener = update_snapshots_listener.clone();
        let update_agent = update_agent.clone();
        async move {
            if let AgentEvent::ToolExecutionUpdate { partial_result, .. } = event {
                let pending_ids = update_agent
                    .state()
                    .pending_tool_calls
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                update_snapshots_listener
                    .lock()
                    .unwrap()
                    .push((pending_ids, partial_result));
            }
        }
    });

    agent.prompt_text("run tool").await.unwrap();

    let update_snapshots = update_snapshots.lock().unwrap().clone();
    assert_eq!(update_snapshots.len(), 1);
    assert_eq!(update_snapshots[0].0, vec![String::from("tool-1")]);
    assert_eq!(
        update_snapshots[0].1.content,
        vec![UserContent::Text {
            text: String::from("partial: hello"),
        }]
    );
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
            AgentMessage::Standard(Message::ToolResult {
                content, is_error, ..
            }) => Some((content.clone(), *is_error)),
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
