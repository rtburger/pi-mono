use crate::startup_shell::{ShellUpdateHandle, tool_result_from_user_content, user_message_text};
use crate::{FooterState, StartupShellComponent, StatusHandle};
use pi_agent::{Agent, AgentEvent, ThinkingLevel};
use pi_coding_agent_core::CodingAgentCore;
use pi_events::{AssistantMessage, Message};
use pi_tui::RenderHandle;

pub struct InteractiveCoreBinding {
    agent: Agent,
    listener_id: usize,
}

impl InteractiveCoreBinding {
    pub fn bind(
        core: CodingAgentCore,
        shell: &mut StartupShellComponent,
        render_handle: RenderHandle,
    ) -> Self {
        let update_handle = shell.update_handle_with_render_handle(render_handle.clone());
        let status_handle = shell.status_handle_with_render_handle(render_handle);

        let state = core.state();
        shell.set_footer_state(FooterState {
            model: Some(state.model.clone()),
            thinking_level: thinking_level_label(state.thinking_level).to_owned(),
            context_window: state.model.context_window,
            ..FooterState::default()
        });

        install_shell_callbacks(core.clone(), shell, status_handle.clone());
        sync_existing_state(&core, &update_handle, &status_handle);

        let agent = core.agent();
        let listener_id = agent.subscribe(move |event, _signal| {
            let update_handle = update_handle.clone();
            let status_handle = status_handle.clone();
            Box::pin(async move {
                apply_agent_event(event, &update_handle, &status_handle);
            })
        });

        Self { agent, listener_id }
    }
}

impl Drop for InteractiveCoreBinding {
    fn drop(&mut self) {
        let _ = self.agent.unsubscribe(self.listener_id);
    }
}

fn install_shell_callbacks(
    core: CodingAgentCore,
    shell: &mut StartupShellComponent,
    status_handle: StatusHandle,
) {
    let submit_core = core.clone();
    let submit_status_handle = status_handle.clone();
    shell.set_on_submit(move |value| {
        if value.trim().is_empty() {
            return;
        }

        submit_status_handle.set_message("Working...");
        let core = submit_core.clone();
        let status_handle = submit_status_handle.clone();
        tokio::spawn(async move {
            if let Err(error) = core.prompt_text(value).await {
                status_handle.set_message(format!("Error: {error}"));
            }
        });
    });

    shell.set_on_escape(move || {
        core.abort();
    });
}

fn sync_existing_state(
    core: &CodingAgentCore,
    update_handle: &ShellUpdateHandle,
    status_handle: &StatusHandle,
) {
    let state = core.state();
    for message in &state.messages {
        let Some(message) = message.as_standard_message() else {
            continue;
        };
        apply_existing_message(message, update_handle);
    }

    if state.is_streaming {
        status_handle.set_message("Working...");
        if let Some(streaming_message) = &state.streaming_message
            && let Some(Message::Assistant { .. }) = streaming_message.as_standard_message()
            && let Some(assistant_message) = assistant_message(
                streaming_message
                    .as_standard_message()
                    .expect("assistant message should exist"),
            )
        {
            update_handle.start_assistant_message(assistant_message);
        }
    }
}

fn apply_existing_message(message: &Message, update_handle: &ShellUpdateHandle) {
    match message {
        Message::User { content, .. } => {
            update_handle.append_user_message(user_message_text(content));
        }
        Message::Assistant { .. } => {
            if let Some(assistant_message) = assistant_message(message) {
                update_handle.finish_assistant_message(assistant_message);
            }
        }
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            is_error,
            ..
        } => {
            update_handle.append_tool_result(
                tool_call_id.clone(),
                tool_name.clone(),
                tool_result_from_user_content(content.clone(), serde_json::Value::Null, *is_error),
            );
        }
    }
}

fn apply_agent_event(
    event: AgentEvent,
    update_handle: &ShellUpdateHandle,
    status_handle: &StatusHandle,
) {
    match event {
        AgentEvent::AgentStart => {
            status_handle.set_message("Working...");
        }
        AgentEvent::AgentEnd { .. } => {
            status_handle.clear();
        }
        AgentEvent::MessageStart { message } => {
            if let Some(Message::Assistant { .. }) = message.as_standard_message()
                && let Some(assistant_message) = assistant_message(
                    message
                        .as_standard_message()
                        .expect("assistant message should exist"),
                )
            {
                update_handle.start_assistant_message(assistant_message);
            }
        }
        AgentEvent::MessageUpdate { message, .. } => {
            if let Some(Message::Assistant { .. }) = message.as_standard_message()
                && let Some(assistant_message) = assistant_message(
                    message
                        .as_standard_message()
                        .expect("assistant message should exist"),
                )
            {
                update_handle.update_assistant_message(assistant_message);
            }
        }
        AgentEvent::MessageEnd { message } => {
            let Some(message) = message.as_standard_message() else {
                return;
            };
            match message {
                Message::User { content, .. } => {
                    update_handle.append_user_message(user_message_text(content));
                }
                Message::Assistant { .. } => {
                    if let Some(assistant_message) = assistant_message(message) {
                        update_handle.finish_assistant_message(assistant_message);
                    }
                }
                Message::ToolResult { .. } => {}
            }
        }
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => {
            update_handle.start_tool_execution(tool_call_id, tool_name, args);
        }
        AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            partial_result,
            ..
        } => {
            update_handle.update_tool_execution(
                tool_call_id,
                tool_result_from_user_content(
                    partial_result.content,
                    partial_result.details,
                    false,
                ),
                true,
            );
        }
        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            result,
            is_error,
            ..
        } => {
            update_handle.update_tool_execution(
                tool_call_id,
                tool_result_from_user_content(result.content, result.details, is_error),
                false,
            );
        }
        AgentEvent::TurnStart | AgentEvent::TurnEnd { .. } => {}
    }
}

fn assistant_message(message: &Message) -> Option<AssistantMessage> {
    match message {
        Message::Assistant {
            content,
            api,
            provider,
            model,
            response_id,
            usage,
            stop_reason,
            error_message,
            timestamp,
        } => Some(AssistantMessage {
            role: "assistant".into(),
            content: content.clone(),
            api: api.clone(),
            provider: provider.clone(),
            model: model.clone(),
            response_id: response_id.clone(),
            usage: usage.clone(),
            stop_reason: stop_reason.clone(),
            error_message: error_message.clone(),
            timestamp: *timestamp,
        }),
        _ => None,
    }
}

fn thinking_level_label(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
    }
}
