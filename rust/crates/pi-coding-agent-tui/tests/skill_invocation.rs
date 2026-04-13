use pi_coding_agent_core::{ParsedSkillBlock, parse_skill_block};
use pi_coding_agent_tui::{
    KeybindingsManager, PlainKeyHintStyler, SkillInvocationMessageComponent, StartupShellComponent,
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

fn parsed_skill_block() -> ParsedSkillBlock {
    parse_skill_block(
        "<skill name=\"test-skill\" location=\"/repo/skills/test/SKILL.md\">\nRead the parser docs first.\n\nThen update the failing regression.\n</skill>\n\nPlease update the tests.",
    )
    .expect("skill block should parse")
}

#[test]
fn skill_invocation_component_renders_collapsed_name_with_expand_hint() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component = SkillInvocationMessageComponent::new(parsed_skill_block(), &keybindings);

    let lines = component.render(54);

    assert!(lines.iter().all(|line| visible_width(line) <= 54));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("[skill] test-skill (ctrl+o to expand)"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Please update the tests."))
    );
}

#[test]
fn skill_invocation_component_renders_expanded_skill_content_without_user_message() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component = SkillInvocationMessageComponent::new(parsed_skill_block(), &keybindings);
    component.set_expanded(true);

    let lines = component.render(60);

    assert!(lines.iter().all(|line| visible_width(line) <= 60));
    assert!(lines.iter().any(|line| line.contains("[skill]")));
    assert!(lines.iter().any(|line| line.contains("test-skill")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Read the parser docs first."))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Then update the failing regression."))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Please update the tests."))
    );
}

#[test]
fn startup_shell_can_render_skill_invocation_component_in_transcript() {
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
    shell.add_transcript_item(Box::new(SkillInvocationMessageComponent::new(
        parsed_skill_block(),
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
    let skill_line = lines
        .iter()
        .position(|line| line.contains("[skill] test-skill"))
        .expect("skill invocation should render");
    let pending_line = lines
        .iter()
        .position(|line| line.contains("Steering: queued message"))
        .expect("pending message should render");
    let prompt_start = lines.len().saturating_sub(3);

    assert!(skill_line < pending_line);
    assert!(pending_line < prompt_start);
}
