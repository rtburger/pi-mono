use pi_coding_agent_core::CompactionSummaryMessage;
use pi_coding_agent_tui::{
    CompactionSummaryMessageComponent, KeybindingsManager, PlainKeyHintStyler,
    StartupShellComponent,
};
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

fn compaction_summary_message() -> CompactionSummaryMessage {
    CompactionSummaryMessage {
        summary: String::from("- Preserved the active plan\n- Dropped duplicate tool chatter"),
        tokens_before: 12_345,
    }
}

#[test]
fn compaction_summary_component_renders_collapsed_message_with_expand_hint() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component =
        CompactionSummaryMessageComponent::new(compaction_summary_message(), &keybindings);

    let lines = component.render(56);

    assert!(lines.iter().all(|line| visible_width(line) <= 56));
    assert!(lines.iter().any(|line| line.contains("[compaction]")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Compacted from 12,345 tokens (ctrl+o to expand)"))
    );
}

#[test]
fn compaction_summary_component_renders_expanded_summary_text() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component =
        CompactionSummaryMessageComponent::new(compaction_summary_message(), &keybindings);
    component.set_expanded(true);

    let lines = component.render(64);

    assert!(lines.iter().all(|line| visible_width(line) <= 64));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Compacted from 12,345 tokens"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Preserved the active plan"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Dropped duplicate tool chatter"))
    );
}

#[test]
fn startup_shell_can_render_compaction_summary_component_in_transcript() {
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
    shell.add_transcript_item(Box::new(CompactionSummaryMessageComponent::new(
        compaction_summary_message(),
        &keybindings,
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
    let compaction_line = lines
        .iter()
        .position(|line| line.contains("[compaction]"))
        .expect("compaction summary should render");
    let pending_line = lines
        .iter()
        .position(|line| line.contains("Steering: queued message"))
        .expect("pending message should render");
    let prompt_line = lines
        .iter()
        .position(|line| line.starts_with("> "))
        .expect("prompt should render");

    assert!(compaction_line < pending_line);
    assert!(pending_line < prompt_line);
}
