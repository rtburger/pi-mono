use parking_lot::Mutex;
use pi_coding_agent_tui::{CustomEditor, KeyId, KeybindingsManager};
use pi_tui::Component;
use std::{collections::BTreeMap, sync::Arc};

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
fn extension_shortcut_can_consume_or_fall_through() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_for_callback = Arc::clone(&seen);

    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut editor = CustomEditor::new(&keybindings);
    editor.set_on_extension_shortcut(move |data| {
        seen_for_callback.lock().push(data.clone());
        data == "x"
    });

    editor.handle_input("x");
    assert_eq!(editor.get_text(), "");

    editor.handle_input("y");
    assert_eq!(editor.get_text(), "y");
    assert_eq!(
        seen.lock().as_slice(),
        [String::from("x"), String::from("y")]
    );
}

#[test]
fn paste_image_binding_uses_app_keybinding_and_does_not_insert_text() {
    let paste_calls = Arc::new(Mutex::new(0usize));
    let paste_calls_for_callback = Arc::clone(&paste_calls);

    let keybindings =
        KeybindingsManager::new(config(&[("app.clipboard.pasteImage", &["ctrl+x"])]), None);
    let mut editor = CustomEditor::new(&keybindings);
    editor.set_text("draft prompt");
    editor.set_on_paste_image(move || {
        *paste_calls_for_callback.lock() += 1;
    });

    editor.handle_input("\x18");

    assert_eq!(*paste_calls.lock(), 1);
    assert_eq!(editor.get_text(), "draft prompt");
}

#[test]
fn interrupt_binding_prefers_on_escape_and_skips_editor_input() {
    let interrupt_calls = Arc::new(Mutex::new(0usize));
    let interrupt_calls_for_callback = Arc::clone(&interrupt_calls);

    let keybindings = KeybindingsManager::new(config(&[("app.interrupt", &["ctrl+x"])]), None);
    let mut editor = CustomEditor::new(&keybindings);
    editor.set_text("draft prompt");
    editor.set_on_escape(move || {
        *interrupt_calls_for_callback.lock() += 1;
    });

    editor.handle_input("\x18");

    assert_eq!(*interrupt_calls.lock(), 1);
    assert_eq!(editor.get_text(), "draft prompt");
}

#[test]
fn empty_exit_binding_invokes_callback_but_non_empty_falls_through_to_delete_forward() {
    let exit_calls = Arc::new(Mutex::new(0usize));
    let exit_calls_for_callback = Arc::clone(&exit_calls);

    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut editor = CustomEditor::new(&keybindings);
    editor.set_on_ctrl_d(move || {
        *exit_calls_for_callback.lock() += 1;
    });

    editor.set_text("abc");
    editor.handle_input("\x01");
    editor.handle_input("\x04");
    assert_eq!(*exit_calls.lock(), 0);
    assert_eq!(editor.get_text(), "bc");

    editor.set_text("");
    editor.handle_input("\x04");
    assert_eq!(*exit_calls.lock(), 1);
    assert_eq!(editor.get_text(), "");
}

#[test]
fn registered_app_actions_run_before_editor_handling() {
    let follow_up_calls = Arc::new(Mutex::new(0usize));
    let follow_up_calls_for_handler = Arc::clone(&follow_up_calls);

    let keybindings =
        KeybindingsManager::new(config(&[("app.message.followUp", &["ctrl+x"])]), None);
    let mut editor = CustomEditor::new(&keybindings);
    editor.set_text("queued message");
    editor.on_action("app.message.followUp", move || {
        *follow_up_calls_for_handler.lock() += 1;
    });

    editor.handle_input("\x18");

    assert_eq!(*follow_up_calls.lock(), 1);
    assert_eq!(editor.get_text(), "queued message");
}
