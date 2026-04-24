use parking_lot::{Mutex, MutexGuard};
use pi_coding_agent_tui::{BorderedLoader, BorderedLoaderOptions, KeybindingsManager, init_theme};
use pi_tui::{Component, visible_width};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

const KEY_ESCAPE: &str = "\x1b";

fn bordered_loader_test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock()
}

fn keybindings() -> KeybindingsManager {
    KeybindingsManager::new(BTreeMap::new(), None)
}

#[test]
fn bordered_loader_renders_borders_and_cancels_on_escape() {
    let _guard = bordered_loader_test_guard();
    let _ = init_theme(None);
    let keybindings = keybindings();
    let aborted = Arc::new(Mutex::new(false));
    let aborted_for_callback = Arc::clone(&aborted);

    let mut loader = BorderedLoader::new(&keybindings, None, "Working...");
    loader.set_on_abort(move || {
        *aborted_for_callback.lock() = true;
    });

    let lines = loader.render(32);
    assert!(lines.first().is_some_and(|line| visible_width(line) == 32));
    assert!(lines.last().is_some_and(|line| visible_width(line) == 32));

    let rendered = lines.join("\n");
    assert!(rendered.contains("Working..."), "lines: {lines:?}");
    assert!(rendered.contains("cancel"), "lines: {lines:?}");
    assert!(!*loader.signal().borrow());
    assert!(!loader.aborted());

    loader.handle_input(KEY_ESCAPE);

    assert!(*loader.signal().borrow());
    assert!(loader.aborted());
    assert!(*aborted.lock());
    loader.dispose();
}

#[test]
fn bordered_loader_without_cancellation_omits_cancel_hint() {
    let _guard = bordered_loader_test_guard();
    let _ = init_theme(None);
    let keybindings = keybindings();

    let mut loader = BorderedLoader::with_options(
        &keybindings,
        None,
        "Still working...",
        BorderedLoaderOptions { cancellable: false },
    );

    let lines = loader.render(28);
    let rendered = lines.join("\n");
    assert!(rendered.contains("Still working..."), "lines: {lines:?}");
    assert!(!rendered.contains("cancel"), "lines: {lines:?}");
    assert!(!loader.aborted());

    loader.handle_input(KEY_ESCAPE);

    assert!(!*loader.signal().borrow());
    assert!(!loader.aborted());
    loader.dispose();
}
