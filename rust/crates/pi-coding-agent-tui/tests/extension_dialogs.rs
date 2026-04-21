use pi_coding_agent_tui::{
    ExtensionInputComponent, ExtensionSelectorComponent, KeybindingsManager,
};
use pi_tui::Component;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const KEY_DOWN: &str = "\x1b[B";
const KEY_ENTER: &str = "\n";

fn keybindings() -> KeybindingsManager {
    KeybindingsManager::new(BTreeMap::new(), None)
}

#[test]
fn extension_input_submits_entered_value() {
    let keybindings = keybindings();
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

    let mut component = ExtensionInputComponent::new(
        &keybindings,
        "Enter a value",
        None,
        move |value| *submitted_for_callback.lock().unwrap() = Some(value),
        || {},
        None,
        None,
    );

    component.handle_input("h");
    component.handle_input("i");
    component.handle_input(KEY_ENTER);

    assert_eq!(submitted.lock().unwrap().as_deref(), Some("hi"));
    assert!(component.render(80).join("\n").contains("Enter a value"));
}

#[test]
fn extension_selector_navigates_and_selects_option() {
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None::<String>));
    let selected_for_callback = Arc::clone(&selected);

    let mut component = ExtensionSelectorComponent::new(
        &keybindings,
        "Choose action",
        vec![String::from("Allow"), String::from("Block")],
        move |value| *selected_for_callback.lock().unwrap() = Some(value),
        || {},
        None,
        None,
    );

    component.handle_input(KEY_DOWN);
    component.handle_input(KEY_ENTER);

    assert_eq!(selected.lock().unwrap().as_deref(), Some("Block"));
    assert!(component.render(80).join("\n").contains("Choose action"));
}

#[test]
fn extension_input_timeout_invokes_cancel_callback() {
    let keybindings = keybindings();
    let cancelled = Arc::new(Mutex::new(0usize));
    let cancelled_for_callback = Arc::clone(&cancelled);

    let _component = ExtensionInputComponent::new(
        &keybindings,
        "Timed input",
        None,
        |_| {},
        move || *cancelled_for_callback.lock().unwrap() += 1,
        Some(20),
        None,
    );

    thread::sleep(Duration::from_millis(100));

    assert_eq!(*cancelled.lock().unwrap(), 1);
}
