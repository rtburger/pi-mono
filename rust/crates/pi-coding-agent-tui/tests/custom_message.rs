use pi_coding_agent_core::{CustomMessage, CustomMessageContent};
use pi_coding_agent_tui::{
    CustomMessageComponent, KeybindingsManager, PlainKeyHintStyler, StartupShellComponent,
};
use pi_events::UserContent;
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

fn string_custom_message() -> CustomMessage {
    CustomMessage {
        custom_type: "note".into(),
        content: CustomMessageContent::Text("Remember to re-run the failing regression.".into()),
        display: true,
        details: None,
    }
}

fn block_custom_message() -> CustomMessage {
    CustomMessage {
        custom_type: "extension".into(),
        content: CustomMessageContent::Blocks(vec![
            UserContent::Text {
                text: "first line".into(),
            },
            UserContent::Image {
                data: "ZmFrZQ==".into(),
                mime_type: "image/png".into(),
            },
            UserContent::Text {
                text: "second line".into(),
            },
        ]),
        display: true,
        details: None,
    }
}

#[test]
fn custom_message_component_renders_label_and_string_content() {
    let component = CustomMessageComponent::new(string_custom_message());

    let lines = component.render(64);

    assert!(lines.iter().all(|line| visible_width(line) <= 64));
    assert!(lines.iter().any(|line| line.contains("[note]")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Remember to re-run the failing regression."))
    );
}

#[test]
fn custom_message_component_renders_only_text_blocks_from_block_content() {
    let component = CustomMessageComponent::new(block_custom_message());

    let lines = component.render(48);

    assert!(lines.iter().all(|line| visible_width(line) <= 48));
    assert!(lines.iter().any(|line| line.contains("[extension]")));
    assert!(lines.iter().any(|line| line.contains("first line")));
    assert!(lines.iter().any(|line| line.contains("second line")));
    assert!(!lines.iter().any(|line| line.contains("image/png")));
    assert!(!lines.iter().any(|line| line.contains("ZmFrZQ==")));
}

#[test]
fn startup_shell_can_render_custom_message_component_in_transcript() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut shell =
        StartupShellComponent::new("Pi", "1.2.3", &keybindings, &PlainKeyHintStyler, true);
    shell.add_transcript_item(Box::new(CustomMessageComponent::new(
        string_custom_message(),
    )));
    shell.set_pending_messages(
        &PlainKeyHintStyler,
        ["queued message"],
        std::iter::empty::<&str>(),
    );

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(72, 20);
    let custom_line = lines
        .iter()
        .position(|line| line.contains("Remember to re-run the failing regression."))
        .expect("custom message should render");
    let pending_line = lines
        .iter()
        .position(|line| line.contains("Steering: queued message"))
        .expect("pending message should render");
    let prompt_start = lines.len().saturating_sub(3);

    assert!(custom_line < pending_line);
    assert!(pending_line < prompt_start);
}
