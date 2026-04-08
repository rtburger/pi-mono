use crate::args::PrintOutputMode;
use pi_agent::{AgentEvent, AgentMessage, AgentToolResult};
use pi_coding_agent_core::CodingAgentCore;
use pi_events::{AssistantContent, Message, StopReason, UserContent};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq)]
pub struct PrintModeOptions {
    pub mode: PrintOutputMode,
    pub messages: Vec<String>,
    pub initial_message: Option<String>,
    pub initial_images: Option<Vec<UserContent>>,
}

impl Default for PrintModeOptions {
    fn default() -> Self {
        Self {
            mode: PrintOutputMode::Text,
            messages: Vec::new(),
            initial_message: None,
            initial_images: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrintModeRunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub async fn run_print_mode(
    core: &CodingAgentCore,
    options: PrintModeOptions,
) -> PrintModeRunResult {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit_code = 0;
    let mut subscription = None;
    let json_lines = Arc::new(Mutex::new(Vec::<String>::new()));

    if options.mode == PrintOutputMode::Json {
        let lines = json_lines.clone();
        let subscription_id = core.agent().subscribe(move |event, _signal| {
            let lines = lines.clone();
            async move {
                let line = serde_json::to_string(&agent_event_to_json(&event))
                    .expect("agent event serialization must succeed");
                lines.lock().unwrap().push(line);
            }
        });
        subscription = Some(subscription_id);
    }

    let run_result = run_prompts(core, &options).await;

    if let Some(subscription_id) = subscription {
        core.agent().unsubscribe(subscription_id);
    }

    if options.mode == PrintOutputMode::Json {
        stdout = join_lines(&json_lines.lock().unwrap());
    }

    match run_result {
        Ok(()) => {
            if options.mode == PrintOutputMode::Text {
                let state = core.state();
                if let Some(last_message) = state
                    .messages
                    .last()
                    .and_then(AgentMessage::as_standard_message)
                    && let Message::Assistant {
                        content,
                        stop_reason,
                        error_message,
                        ..
                    } = last_message
                {
                    if matches!(stop_reason, StopReason::Error | StopReason::Aborted) {
                        push_line(
                            &mut stderr,
                            error_message
                                .as_deref()
                                .unwrap_or_else(|| default_request_error(stop_reason)),
                        );
                        exit_code = 1;
                    } else {
                        for block in content {
                            if let AssistantContent::Text { text, .. } = block {
                                push_line(&mut stdout, text);
                            }
                        }
                    }
                }
            }
        }
        Err(error) => {
            push_line(&mut stderr, &error);
            exit_code = 1;
        }
    }

    PrintModeRunResult {
        exit_code,
        stdout,
        stderr,
    }
}

async fn run_prompts(core: &CodingAgentCore, options: &PrintModeOptions) -> Result<(), String> {
    if let Some(initial_message) = options.initial_message.as_ref() {
        prompt_initial_message(core, initial_message, options.initial_images.as_deref())
            .await
            .map_err(|error| error.to_string())?;
    }

    for message in &options.messages {
        core.prompt_text(message)
            .await
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

async fn prompt_initial_message(
    core: &CodingAgentCore,
    initial_message: &str,
    initial_images: Option<&[UserContent]>,
) -> Result<(), pi_coding_agent_core::CodingAgentCoreError> {
    let Some(initial_images) = initial_images else {
        return core.prompt_text(initial_message).await;
    };

    let mut content = Vec::with_capacity(initial_images.len() + 1);
    content.push(UserContent::Text {
        text: initial_message.to_string(),
    });
    content.extend(initial_images.iter().cloned());

    core.prompt_message(Message::User {
        content,
        timestamp: now_ms(),
    })
    .await
}

fn agent_event_to_json(event: &AgentEvent) -> Value {
    match event {
        AgentEvent::AgentStart => json!({ "type": "agent_start" }),
        AgentEvent::AgentEnd { messages } => json!({
            "type": "agent_end",
            "messages": messages.iter().map(agent_message_to_json).collect::<Vec<_>>(),
        }),
        AgentEvent::TurnStart => json!({ "type": "turn_start" }),
        AgentEvent::TurnEnd {
            message,
            tool_results,
        } => json!({
            "type": "turn_end",
            "message": message,
            "toolResults": tool_results,
        }),
        AgentEvent::MessageStart { message } => json!({
            "type": "message_start",
            "message": agent_message_to_json(message),
        }),
        AgentEvent::MessageUpdate {
            message,
            assistant_event,
        } => json!({
            "type": "message_update",
            "message": agent_message_to_json(message),
            "assistantEvent": assistant_event,
        }),
        AgentEvent::MessageEnd { message } => json!({
            "type": "message_end",
            "message": agent_message_to_json(message),
        }),
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => json!({
            "type": "tool_execution_start",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
        }),
        AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            tool_name,
            args,
            partial_result,
        } => json!({
            "type": "tool_execution_update",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
            "partialResult": agent_tool_result_to_json(partial_result),
        }),
        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            tool_name,
            result,
            is_error,
        } => json!({
            "type": "tool_execution_end",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "result": agent_tool_result_to_json(result),
            "isError": is_error,
        }),
    }
}

fn agent_message_to_json(message: &AgentMessage) -> Value {
    match message {
        AgentMessage::Standard(message) => {
            serde_json::to_value(message).expect("standard agent messages must serialize")
        }
        AgentMessage::Custom(message) => json!({
            "role": message.role,
            "payload": message.payload,
            "timestamp": message.timestamp,
        }),
    }
}

fn agent_tool_result_to_json(result: &AgentToolResult) -> Value {
    json!({
        "content": result.content,
        "details": result.details,
    })
}

fn default_request_error(stop_reason: &StopReason) -> &'static str {
    match stop_reason {
        StopReason::Error => "Request error",
        StopReason::Aborted => "Request aborted",
        StopReason::Stop | StopReason::Length | StopReason::ToolUse => "Request error",
    }
}

fn push_line(buffer: &mut String, line: &str) {
    buffer.push_str(line);
    buffer.push('\n');
}

fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        let mut output = lines.join("\n");
        output.push('\n');
        output
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
