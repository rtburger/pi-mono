use crate::startup_shell::{ShellUpdateHandle, tool_result_from_user_content, user_message_text};
use crate::{FooterState, StartupShellComponent, StatusHandle};
use pi_agent::{Agent, AgentEvent, ThinkingLevel};
use pi_coding_agent_core::CodingAgentCore;
use pi_events::{AssistantMessage, Message, UserContent};
use pi_tui::RenderHandle;
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

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

        let pending_follow_ups = Arc::new(Mutex::new(Vec::new()));

        install_shell_callbacks(
            core.clone(),
            shell,
            status_handle.clone(),
            update_handle.clone(),
            pending_follow_ups.clone(),
        );
        sync_existing_state(&core, &update_handle, &status_handle);

        let agent = core.agent();
        let listener_id = agent.subscribe(move |event, _signal| {
            let update_handle = update_handle.clone();
            let status_handle = status_handle.clone();
            let pending_follow_ups = pending_follow_ups.clone();
            Box::pin(async move {
                apply_agent_event(event, &update_handle, &status_handle, &pending_follow_ups);
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
    update_handle: ShellUpdateHandle,
    pending_follow_ups: Arc<Mutex<Vec<String>>>,
) {
    let submit_core = core.clone();
    let submit_status_handle = status_handle.clone();
    shell.set_on_submit(move |value| {
        submit_prompt(submit_core.clone(), submit_status_handle.clone(), value);
    });

    let follow_up_core = core.clone();
    let follow_up_update_handle = update_handle.clone();
    let follow_up_messages = pending_follow_ups.clone();
    shell.on_action_with_shell("app.message.followUp", move |shell| {
        let value = shell.input_value();
        if value.trim().is_empty() {
            return;
        }

        if follow_up_core.state().is_streaming {
            shell.clear_input();
            queue_follow_up_message(
                &follow_up_core,
                &follow_up_update_handle,
                &follow_up_messages,
                value,
            );
        } else {
            shell.submit_current_input();
        }
    });

    let dequeue_core = core.clone();
    let dequeue_status_handle = status_handle.clone();
    let dequeue_update_handle = update_handle.clone();
    let dequeue_messages = pending_follow_ups.clone();
    shell.on_action_with_shell("app.message.dequeue", move |shell| {
        let restored = restore_pending_follow_ups_to_shell(
            shell,
            &dequeue_core,
            &dequeue_update_handle,
            &dequeue_messages,
        );
        if restored == 0 {
            dequeue_status_handle.set_message("No queued messages to restore");
        } else {
            let suffix = if restored == 1 { "" } else { "s" };
            dequeue_status_handle.set_message(format!(
                "Restored {restored} queued message{suffix} to editor"
            ));
        }
    });

    let interrupt_core = core.clone();
    let interrupt_update_handle = update_handle;
    let interrupt_messages = pending_follow_ups;
    shell.clear_on_escape();
    shell.on_action_with_shell("app.interrupt", move |shell| {
        if interrupt_core.state().is_streaming && has_pending_follow_ups(&interrupt_messages) {
            restore_pending_follow_ups_to_shell(
                shell,
                &interrupt_core,
                &interrupt_update_handle,
                &interrupt_messages,
            );
        }
        interrupt_core.abort();
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
    pending_follow_ups: &Arc<Mutex<Vec<String>>>,
) {
    match event {
        AgentEvent::AgentStart => {
            status_handle.set_message("Working...");
        }
        AgentEvent::AgentEnd { .. } => {
            pending_follow_ups
                .lock()
                .expect("pending follow-up mutex poisoned")
                .clear();
            update_handle.clear_pending_messages();
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

fn submit_prompt(core: CodingAgentCore, status_handle: StatusHandle, value: String) {
    if value.trim().is_empty() {
        return;
    }

    status_handle.set_message("Working...");
    tokio::spawn(async move {
        if let Err(error) = core.prompt_text(value).await {
            status_handle.set_message(format!("Error: {error}"));
        }
    });
}

fn queue_follow_up_message(
    core: &CodingAgentCore,
    update_handle: &ShellUpdateHandle,
    pending_follow_ups: &Arc<Mutex<Vec<String>>>,
    text: String,
) {
    core.agent().follow_up(Message::User {
        content: vec![UserContent::Text { text: text.clone() }],
        timestamp: now_ms(),
    });

    let follow_up = {
        let mut pending_follow_ups = pending_follow_ups
            .lock()
            .expect("pending follow-up mutex poisoned");
        pending_follow_ups.push(text);
        pending_follow_ups.clone()
    };
    update_handle.set_pending_messages(Vec::new(), follow_up);
}

fn restore_pending_follow_ups_to_shell(
    shell: &mut StartupShellComponent,
    core: &CodingAgentCore,
    update_handle: &ShellUpdateHandle,
    pending_follow_ups: &Arc<Mutex<Vec<String>>>,
) -> usize {
    let follow_up = {
        let mut pending_follow_ups = pending_follow_ups
            .lock()
            .expect("pending follow-up mutex poisoned");
        if pending_follow_ups.is_empty() {
            return 0;
        }
        std::mem::take(&mut *pending_follow_ups)
    };

    core.agent().clear_follow_up_queue();

    let queued_text = follow_up.join("\n\n");
    let current_text = shell.input_value();
    let combined = [queued_text, current_text]
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    shell.set_input_value(combined.clone());
    shell.set_input_cursor(combined.len());
    update_handle.clear_pending_messages();
    follow_up.len()
}

fn has_pending_follow_ups(pending_follow_ups: &Arc<Mutex<Vec<String>>>) -> bool {
    !pending_follow_ups
        .lock()
        .expect("pending follow-up mutex poisoned")
        .is_empty()
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
