use pi_coding_agent_tui::{
    AssistantMessageComponent, DEFAULT_HIDDEN_THINKING_LABEL, KeybindingsManager,
    PlainKeyHintStyler, StartupShellComponent,
};
use pi_events::{AssistantContent, AssistantMessage, StopReason, Usage};
use pi_tui::{Component, Terminal, Tui, TuiError, visible_width};
use std::{collections::BTreeMap, time::Duration};

#[derive(Default)]
struct NoopTerminal;

impl Terminal for NoopTerminal {
    fn start(
        &mut self,
        _on_input: Box<dyn FnMut(String) + Send>,
        _on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn drain_input(&mut self, _max: Duration, _idle: Duration) -> Result<(), TuiError> {
        Ok(())
    }

    fn write(&mut self, _data: &str) -> Result<(), TuiError> {
        Ok(())
    }

    fn columns(&self) -> u16 {
        80
    }

    fn rows(&self) -> u16 {
        24
    }

    fn kitty_protocol_active(&self) -> bool {
        false
    }

    fn move_by(&mut self, _lines: i32) -> Result<(), TuiError> {
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn clear_line(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn clear_from_cursor(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn clear_screen(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn set_title(&mut self, _title: &str) -> Result<(), TuiError> {
        Ok(())
    }
}

fn assistant_message(
    content: Vec<AssistantContent>,
    stop_reason: StopReason,
    error_message: Option<&str>,
) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content,
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "gpt-5-mini".into(),
        response_id: None,
        usage: Usage::default(),
        stop_reason,
        error_message: error_message.map(str::to_string),
        timestamp: 1,
    }
}

#[test]
fn assistant_message_component_renders_text_and_visible_thinking_blocks() {
    let component = AssistantMessageComponent::new(
        Some(assistant_message(
            vec![
                AssistantContent::Thinking {
                    thinking: "Reason through the edge case first.".into(),
                    thinking_signature: None,
                    redacted: false,
                },
                AssistantContent::Text {
                    text: "Final answer with the fix.".into(),
                    text_signature: None,
                },
            ],
            StopReason::Stop,
            None,
        )),
        false,
        DEFAULT_HIDDEN_THINKING_LABEL,
    );

    let lines = component.render(64);

    assert!(lines.iter().all(|line| visible_width(line) <= 64));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Reason through the edge case first."))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Final answer with the fix."))
    );
}

#[test]
fn assistant_message_component_renders_markdown_and_styles_visible_thinking() {
    let component = AssistantMessageComponent::new(
        Some(assistant_message(
            vec![
                AssistantContent::Thinking {
                    thinking: "Need to inspect `src/lib.rs` first.".into(),
                    thinking_signature: None,
                    redacted: false,
                },
                AssistantContent::Text {
                    text: "# Final Plan\n\n- update parser\n- add regression".into(),
                    text_signature: None,
                },
            ],
            StopReason::Stop,
            None,
        )),
        false,
        DEFAULT_HIDDEN_THINKING_LABEL,
    );

    let lines = component.render(72);
    let joined = lines.join("\n");

    assert!(lines.iter().all(|line| visible_width(line) <= 72));
    assert!(joined.contains("Final Plan"), "lines: {lines:?}");
    assert!(joined.contains("update parser"), "lines: {lines:?}");
    assert!(joined.contains("add regression"), "lines: {lines:?}");
    assert!(!joined.contains("# Final Plan"), "lines: {lines:?}");
    assert!(joined.contains("\x1b[3m"), "lines: {lines:?}");
    assert!(joined.contains("src/lib.rs"), "lines: {lines:?}");
}

#[test]
fn assistant_message_component_can_hide_thinking_with_custom_label() {
    let mut component = AssistantMessageComponent::new(
        Some(assistant_message(
            vec![
                AssistantContent::Thinking {
                    thinking: "Private reasoning trace.".into(),
                    thinking_signature: None,
                    redacted: false,
                },
                AssistantContent::Text {
                    text: "Visible answer.".into(),
                    text_signature: None,
                },
            ],
            StopReason::Stop,
            None,
        )),
        true,
        DEFAULT_HIDDEN_THINKING_LABEL,
    );

    let hidden_lines = component.render(60);
    assert!(
        hidden_lines
            .iter()
            .any(|line| line.contains(DEFAULT_HIDDEN_THINKING_LABEL))
    );
    assert!(
        !hidden_lines
            .iter()
            .any(|line| line.contains("Private reasoning trace."))
    );

    component.set_hidden_thinking_label("Internal reasoning hidden");
    let relabeled_lines = component.render(60);
    assert!(
        relabeled_lines
            .iter()
            .any(|line| line.contains("Internal reasoning hidden"))
    );
}

#[test]
fn assistant_message_component_renders_terminal_abort_and_error_without_tool_calls() {
    let aborted = AssistantMessageComponent::new(
        Some(assistant_message(
            vec![AssistantContent::Text {
                text: "Partial answer".into(),
                text_signature: None,
            }],
            StopReason::Aborted,
            Some("Request was aborted"),
        )),
        false,
        DEFAULT_HIDDEN_THINKING_LABEL,
    );
    let aborted_lines = aborted.render(60);
    assert!(
        aborted_lines
            .iter()
            .any(|line| line.contains("Operation aborted"))
    );

    let errored = AssistantMessageComponent::new(
        Some(assistant_message(
            Vec::new(),
            StopReason::Error,
            Some("boom"),
        )),
        false,
        DEFAULT_HIDDEN_THINKING_LABEL,
    );
    let error_lines = errored.render(60);
    assert!(error_lines.iter().any(|line| line.contains("Error: boom")));
}

#[test]
fn assistant_message_component_skips_terminal_error_text_when_tool_calls_are_present() {
    let component = AssistantMessageComponent::new(
        Some(assistant_message(
            vec![AssistantContent::ToolCall {
                id: "call_1".into(),
                name: "edit".into(),
                arguments: BTreeMap::new(),
                thought_signature: None,
            }],
            StopReason::Error,
            Some("tool failure"),
        )),
        false,
        DEFAULT_HIDDEN_THINKING_LABEL,
    );

    let lines = component.render(60);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Error: tool failure"))
    );
    assert!(!lines.iter().any(|line| line.contains("tool failure")));
}

#[test]
fn startup_shell_can_render_assistant_message_component_in_transcript() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.add_transcript_item(Box::new(AssistantMessageComponent::new(
        Some(assistant_message(
            vec![AssistantContent::Text {
                text: "Assistant response".into(),
                text_signature: None,
            }],
            StopReason::Stop,
            None,
        )),
        false,
        DEFAULT_HIDDEN_THINKING_LABEL,
    )));
    shell.set_pending_messages(
        &PlainKeyHintStyler,
        ["queued message"],
        std::iter::empty::<&str>(),
    );

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(64, 20);
    let assistant_line = lines
        .iter()
        .position(|line| line.contains("Assistant response"))
        .expect("assistant message should render");
    let pending_line = lines
        .iter()
        .position(|line| line.contains("Steering: queued message"))
        .expect("pending message should render");
    let prompt_start = lines.len().saturating_sub(3);

    assert!(assistant_line < pending_line);
    assert!(pending_line < prompt_start);
}
