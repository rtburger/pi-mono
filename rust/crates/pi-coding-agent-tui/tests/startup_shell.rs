use pi_coding_agent_tui::{KeyId, KeybindingsManager, PlainKeyHintStyler, StartupShellComponent};
use pi_tui::{Terminal, Tui, TuiError};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

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
fn startup_shell_renders_header_above_prompt() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        false,
        None,
        false,
    );

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(40, 40);

    assert!(lines.iter().any(|line| line.contains("Pi v1.2.3")));
    assert!(lines.last().is_some_and(|line| line.starts_with("> ")));
}

#[test]
fn quiet_startup_shell_without_changelog_renders_prompt_on_first_line() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(40, 10);

    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("> "));
}

#[test]
fn startup_shell_routes_input_and_submit_through_tui() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

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
    shell.set_on_submit(move |value| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") = Some(value);
    });

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    tui.handle_input("h").expect("input should be handled");
    tui.handle_input("i").expect("input should be handled");

    let lines = tui.render_for_size(20, 10);
    assert!(lines[0].contains("hi"));

    tui.handle_input("\r").expect("submit should be handled");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("hi")
    );
}

#[test]
fn startup_shell_uses_shared_keybindings_for_header_and_input() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

    let keybindings = KeybindingsManager::new(
        config(&[
            ("app.interrupt", &["ctrl+x"]),
            ("tui.input.submit", &["ctrl+s"]),
        ]),
        None,
    );
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        false,
        None,
        false,
    );
    shell.set_on_submit(move |value| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") = Some(value);
    });

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(50, 40);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("ctrl+x to interrupt"))
    );

    tui.handle_input("o").expect("input should be handled");
    tui.handle_input("k").expect("input should be handled");
    tui.handle_input("\x13")
        .expect("custom submit binding should be handled");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("ok")
    );
}
