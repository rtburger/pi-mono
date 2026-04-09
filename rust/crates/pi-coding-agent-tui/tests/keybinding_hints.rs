use pi_coding_agent_tui::{
    KeyHintStyler, KeyId, KeybindingsManager, PlainKeyHintStyler, key_hint, key_text, raw_key_hint,
};
use std::collections::BTreeMap;

struct TestStyler;

impl KeyHintStyler for TestStyler {
    fn dim(&self, text: &str) -> String {
        format!("<dim>{text}</dim>")
    }

    fn muted(&self, text: &str) -> String {
        format!("<muted>{text}</muted>")
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
fn key_text_joins_multiple_keys_with_slashes() {
    let keybindings =
        KeybindingsManager::new(config(&[("app.tools.expand", &["ctrl+o", "alt+o"])]), None);

    assert_eq!(key_text(&keybindings, "app.tools.expand"), "ctrl+o/alt+o");
}

#[test]
fn key_text_returns_empty_string_for_unbound_actions() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);

    assert_eq!(key_text(&keybindings, "app.session.new"), "");
}

#[test]
fn key_hint_combines_dimmed_keys_with_muted_description() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);

    assert_eq!(
        key_hint(
            &keybindings,
            &TestStyler,
            "app.tools.expand",
            "to expand tools"
        ),
        "<dim>ctrl+o</dim><muted> to expand tools</muted>"
    );
}

#[test]
fn raw_key_hint_formats_literal_keys_without_lookup() {
    assert_eq!(
        raw_key_hint(&TestStyler, "/", "for commands"),
        "<dim>/</dim><muted> for commands</muted>"
    );
}

#[test]
fn plain_key_hint_styler_leaves_text_unwrapped() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);

    assert_eq!(
        key_hint(
            &keybindings,
            &PlainKeyHintStyler,
            "app.editor.external",
            "for external editor",
        ),
        "ctrl+g for external editor"
    );
}
