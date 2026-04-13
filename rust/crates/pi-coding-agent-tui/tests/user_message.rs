use pi_coding_agent_core::parse_skill_block;
use pi_coding_agent_tui::{
    KeybindingsManager, PlainKeyHintStyler, SkillInvocationMessageComponent, StartupShellComponent,
    UserMessageComponent,
};
use pi_tui::{Component, Terminal, Tui, TuiError, visible_width};
use std::{collections::BTreeMap, time::Duration};

const OSC133_ZONE_START: &str = "\x1b]133;A\x07";
const OSC133_ZONE_END: &str = "\x1b]133;B\x07";
const OSC133_ZONE_FINAL: &str = "\x1b]133;C\x07";

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

#[test]
fn user_message_component_wraps_rendered_output_in_osc133_prompt_markers() {
    let component = UserMessageComponent::new("Please review the diff before editing files.");

    let lines = component.render(40);

    assert!(!lines.is_empty());
    assert!(lines[0].starts_with(OSC133_ZONE_START));
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains(OSC133_ZONE_END))
    );
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains(OSC133_ZONE_FINAL))
    );
    assert!(lines.iter().all(|line| visible_width(line) <= 40));
}

#[test]
fn startup_shell_can_render_user_message_component_in_transcript() {
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
    shell.add_transcript_item(Box::new(UserMessageComponent::new(
        "Please review the diff before editing files.",
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
    let user_line = lines
        .iter()
        .position(|line| line.contains("Please review the diff before editing files."))
        .expect("user message should render");
    let pending_line = lines
        .iter()
        .position(|line| line.contains("Steering: queued message"))
        .expect("pending message should render");
    let prompt_start = lines.len().saturating_sub(3);

    assert!(user_line < pending_line);
    assert!(pending_line < prompt_start);
}

#[test]
fn parsed_skill_block_and_trailing_user_message_can_render_as_separate_transcript_items() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let skill_block = parse_skill_block(
        "<skill name=\"test-skill\" location=\"/repo/skills/test/SKILL.md\">\nRead the parser docs first.\n\nThen update the failing regression.\n</skill>\n\nPlease update the tests.",
    )
    .expect("skill block should parse");

    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.add_transcript_item(Box::new(SkillInvocationMessageComponent::new(
        skill_block.clone(),
        &keybindings,
    )));
    shell.add_transcript_item(Box::new(UserMessageComponent::new(
        skill_block
            .user_message
            .clone()
            .expect("trailing user message should exist"),
    )));

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(64, 20);
    let skill_line = lines
        .iter()
        .position(|line| line.contains("[skill] test-skill"))
        .expect("skill invocation should render");
    let user_line = lines
        .iter()
        .position(|line| line.contains("Please update the tests."))
        .expect("trailing user message should render separately");
    let prompt_start = lines.len().saturating_sub(3);

    assert!(skill_line < user_line);
    assert!(user_line < prompt_start);
}
