use pi_coding_agent_tui::{
    BuiltInHeaderComponent, KeyHintStyler, KeyId, KeybindingsManager, StartupHeaderComponent,
    StartupHeaderStyler, build_condensed_changelog_notice, build_startup_header_text,
};
use pi_tui::{Component, Terminal, Tui, TuiError};
use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, Clone, Copy, Default)]
struct PlainStyler;

impl KeyHintStyler for PlainStyler {
    fn dim(&self, text: &str) -> String {
        text.to_owned()
    }

    fn muted(&self, text: &str) -> String {
        text.to_owned()
    }
}

impl StartupHeaderStyler for PlainStyler {
    fn accent_bold(&self, text: &str) -> String {
        text.to_owned()
    }
}

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

fn config(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<KeyId>> {
    entries
        .iter()
        .map(|(keybinding, keys)| {
            (
                (*keybinding).to_owned(),
                keys.iter().copied().map(KeyId::from).collect(),
            )
        })
        .collect()
}

#[test]
fn startup_header_text_matches_typescript_instruction_shape() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);

    assert_eq!(
        build_startup_header_text("Pi", "1.2.3", &keybindings, &PlainStyler, false),
        concat!(
            "Pi v1.2.3\n",
            "escape to interrupt\n",
            "ctrl+c to clear\n",
            "ctrl+c twice to exit\n",
            "ctrl+d to exit (empty)\n",
            "ctrl+z to suspend\n",
            "ctrl+k to delete to end\n",
            "shift+tab to cycle thinking level\n",
            "ctrl+p/shift+ctrl+p to cycle models\n",
            "ctrl+l to select model\n",
            "ctrl+o to expand tools\n",
            "ctrl+t to expand thinking\n",
            "ctrl+g for external editor\n",
            "/ for commands\n",
            "! to run bash\n",
            "!! to run bash (no context)\n",
            "alt+enter to queue follow-up\n",
            "alt+up to edit all queued messages\n",
            "ctrl+v to paste image\n",
            "drop files to attach\n\n",
            "Pi can explain its own features and look up its docs. Ask it how to use or extend Pi."
        )
    );
}

#[test]
fn startup_header_text_uses_resolved_keybinding_overrides() {
    let keybindings = KeybindingsManager::new(
        config(&[
            ("app.interrupt", &["ctrl+x"]),
            ("app.model.cycleForward", &["alt+n"]),
            ("app.model.cycleBackward", &["alt+p"]),
        ]),
        None,
    );

    let text = build_startup_header_text("Pi", "1.2.3", &keybindings, &PlainStyler, false);

    assert!(text.contains("ctrl+x to interrupt"));
    assert!(text.contains("alt+n/alt+p to cycle models"));
    assert!(!text.contains("escape to interrupt"));
}

#[test]
fn startup_header_text_is_empty_when_quiet() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);

    assert_eq!(
        build_startup_header_text("Pi", "1.2.3", &keybindings, &PlainStyler, true),
        ""
    );
}

#[test]
fn startup_header_component_renders_through_tui_and_wraps_long_lines() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component = StartupHeaderComponent::new("Pi", "1.2.3", &keybindings, &PlainStyler, false);

    let mut tui = Tui::new(NoopTerminal);
    tui.add_child(Box::new(component));

    let lines = tui.render_for_size(32, 40);

    assert!(
        lines
            .first()
            .is_some_and(|line| line.starts_with("Pi v1.2.3"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Pi can explain its own"))
    );
    assert!(lines.iter().any(|line| line.contains("Ask it how")));
}

#[test]
fn condensed_changelog_notice_uses_latest_version_from_markdown() {
    assert_eq!(
        build_condensed_changelog_notice(
            "# Changelog\n\n## [0.9.0]\n- Added stuff\n",
            "1.2.3",
            &PlainStyler,
        ),
        "Updated to v0.9.0. Use /changelog to view full changelog."
    );
}

#[test]
fn built_in_header_component_renders_spacers_borders_and_condensed_notice() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component = BuiltInHeaderComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainStyler,
        false,
        Some("## [0.9.0]\n- Added stuff\n"),
        true,
    );

    let lines = component.render(60);

    assert_eq!(lines.first().map(String::as_str), Some(""));
    assert!(lines.iter().any(|line| line == &"─".repeat(60)));
    assert!(
        lines
            .iter()
            .any(|line| { line == "Updated to v0.9.0. Use /changelog to view full changelog." })
    );
}

#[test]
fn built_in_header_component_renders_expanded_changelog_markdown() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component = BuiltInHeaderComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainStyler,
        false,
        Some(
            "## [0.9.0]\n\n### Added\n\n- Added **foo**\n- See [docs](docs.md)\n\n```ts\nconsole.log(\"hi\");\n```\n",
        ),
        false,
    );

    let lines = component.render(80);

    assert!(lines.iter().any(|line| line == "What's New"));
    assert!(lines.iter().any(|line| line == "[0.9.0]"));
    assert!(lines.iter().any(|line| line == "### Added"));
    assert!(lines.iter().any(|line| line == "- Added foo"));
    assert!(lines.iter().any(|line| line == "- See docs (docs.md)"));
    assert!(lines.iter().any(|line| line == "```ts"));
    assert!(lines.iter().any(|line| line == "  console.log(\"hi\");"));
    assert!(lines.iter().any(|line| line == "```"));
    assert!(!lines.iter().any(|line| line.contains("Updated to v0.9.0")));
}

#[test]
fn built_in_header_component_in_quiet_mode_only_shows_condensed_notice() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component = BuiltInHeaderComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainStyler,
        true,
        Some("## [0.9.0]\n- Added stuff\n"),
        false,
    );

    let lines = component.render(80);

    assert_eq!(
        lines,
        vec![
            String::new(),
            "Updated to v0.9.0. Use /changelog to view full changelog.".to_owned(),
        ]
    );
}

#[test]
fn built_in_header_component_in_quiet_mode_without_changelog_renders_no_lines() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component =
        BuiltInHeaderComponent::new("Pi", "1.2.3", &keybindings, &PlainStyler, true, None, false);

    assert_eq!(component.render(80), Vec::<String>::new());
}
