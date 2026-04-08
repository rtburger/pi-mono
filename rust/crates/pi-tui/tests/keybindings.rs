use pi_tui::{KeyId, KeybindingsConfig, KeybindingsManager, TUI_KEYBINDINGS};
use std::collections::BTreeMap;

fn config(entries: &[(&str, &[&str])]) -> KeybindingsConfig {
    entries
        .iter()
        .map(|(keybinding, keys)| {
            (
                (*keybinding).to_owned(),
                keys.iter().copied().map(KeyId::from).collect(),
            )
        })
        .collect::<BTreeMap<_, _>>()
}

#[test]
fn does_not_evict_selector_confirm_when_input_submit_is_rebound() {
    let keybindings = KeybindingsManager::new(
        TUI_KEYBINDINGS.as_slice(),
        config(&[("tui.input.submit", &["enter", "ctrl+enter"])]),
    );

    assert_eq!(
        keybindings.get_keys("tui.input.submit"),
        vec![KeyId::from("enter"), KeyId::from("ctrl+enter")]
    );
    assert_eq!(
        keybindings.get_keys("tui.select.confirm"),
        vec![KeyId::from("enter")]
    );
}

#[test]
fn does_not_evict_cursor_bindings_when_another_action_reuses_the_same_key() {
    let keybindings = KeybindingsManager::new(
        TUI_KEYBINDINGS.as_slice(),
        config(&[("tui.select.up", &["up", "ctrl+p"])]),
    );

    assert_eq!(
        keybindings.get_keys("tui.select.up"),
        vec![KeyId::from("up"), KeyId::from("ctrl+p")]
    );
    assert_eq!(
        keybindings.get_keys("tui.editor.cursorUp"),
        vec![KeyId::from("up")]
    );
}

#[test]
fn still_reports_direct_user_binding_conflicts_without_evicting_defaults() {
    let keybindings = KeybindingsManager::new(
        TUI_KEYBINDINGS.as_slice(),
        config(&[
            ("tui.input.submit", &["ctrl+x"]),
            ("tui.select.confirm", &["ctrl+x"]),
        ]),
    );

    assert_eq!(
        keybindings.get_conflicts(),
        vec![pi_tui::KeybindingConflict {
            key: KeyId::from("ctrl+x"),
            keybindings: vec![
                "tui.input.submit".to_owned(),
                "tui.select.confirm".to_owned()
            ],
        }]
    );
    assert_eq!(
        keybindings.get_keys("tui.editor.cursorLeft"),
        vec![KeyId::from("left"), KeyId::from("ctrl+b")]
    );
}
