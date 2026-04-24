use parking_lot::{Mutex, MutexGuard};
use pi_coding_agent_tui::{
    KeybindingsManager, UserMessageSelectorComponent, UserMessageSelectorItem, init_theme,
};
use pi_tui::Component;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

const KEY_ENTER: &str = "\n";
const KEY_ESCAPE: &str = "\x1b";
const KEY_UP: &str = "\x1b[A";

fn selector_test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock()
}

fn keybindings() -> KeybindingsManager {
    KeybindingsManager::new(BTreeMap::new(), None)
}

#[test]
fn user_message_selector_defaults_to_most_recent_message() {
    let _guard = selector_test_guard();
    let _ = init_theme(None);
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None::<String>));
    let mut selector = UserMessageSelectorComponent::new(
        &keybindings,
        vec![
            UserMessageSelectorItem {
                id: String::from("first"),
                text: String::from("first message"),
                is_root: true,
            },
            UserMessageSelectorItem {
                id: String::from("second"),
                text: String::from("second message"),
                is_root: false,
            },
        ],
    );
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |value| *selected.lock() = Some(value));
    }

    let rendered = selector.render(80).join("\n");
    assert!(rendered.contains("Fork session from user message"));
    assert!(rendered.contains("Message 2 of 2"), "output: {rendered}");

    selector.handle_input(KEY_ENTER);

    assert_eq!(*selected.lock(), Some(String::from("second")));
}

#[test]
fn user_message_selector_navigates_and_cancels() {
    let _guard = selector_test_guard();
    let _ = init_theme(None);
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None::<String>));
    let cancelled = Arc::new(Mutex::new(false));
    let mut selector = UserMessageSelectorComponent::new(
        &keybindings,
        vec![
            UserMessageSelectorItem {
                id: String::from("root"),
                text: String::from("root message"),
                is_root: true,
            },
            UserMessageSelectorItem {
                id: String::from("leaf"),
                text: String::from("leaf message"),
                is_root: false,
            },
        ],
    );
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |value| *selected.lock() = Some(value));
    }
    {
        let cancelled = Arc::clone(&cancelled);
        selector.set_on_cancel(move || *cancelled.lock() = true);
    }

    selector.handle_input(KEY_UP);
    selector.handle_input(KEY_ENTER);
    selector.handle_input(KEY_ESCAPE);

    assert_eq!(*selected.lock(), Some(String::from("root")));
    assert!(*cancelled.lock());

    let rendered = selector.render(80).join("\n");
    assert!(rendered.contains("root"), "output: {rendered}");
}
