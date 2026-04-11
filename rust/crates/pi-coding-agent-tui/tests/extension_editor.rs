use pi_coding_agent_tui::{
    ExtensionEditorComponent, ExternalEditorCommandRunner, ExternalEditorHost, KeyId,
    KeybindingsManager,
};
use pi_tui::Component;
use std::{
    collections::BTreeMap,
    fs, io,
    path::Path,
    sync::{Arc, Mutex},
};

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

#[derive(Clone, Default)]
struct RecordingHost {
    events: Arc<Mutex<Vec<String>>>,
}

impl ExternalEditorHost for RecordingHost {
    fn stop(&self) {
        self.events
            .lock()
            .expect("host events mutex poisoned")
            .push(String::from("stop"));
    }

    fn start(&self) {
        self.events
            .lock()
            .expect("host events mutex poisoned")
            .push(String::from("start"));
    }

    fn request_render(&self) {
        self.events
            .lock()
            .expect("host events mutex poisoned")
            .push(String::from("request_render"));
    }
}

#[derive(Clone, Default)]
struct RecordingRunner {
    calls: Arc<Mutex<Vec<(String, String)>>>,
    replacement: Option<String>,
    exit_code: Option<i32>,
}

impl RecordingRunner {
    fn with_result(replacement: Option<&str>, exit_code: Option<i32>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            replacement: replacement.map(str::to_owned),
            exit_code,
        }
    }
}

impl ExternalEditorCommandRunner for RecordingRunner {
    fn run(&self, command: &str, file_path: &Path) -> io::Result<Option<i32>> {
        let current_text = fs::read_to_string(file_path)?;
        self.calls
            .lock()
            .expect("runner calls mutex poisoned")
            .push((command.to_owned(), current_text));
        if let Some(replacement) = &self.replacement {
            fs::write(file_path, replacement)?;
        }
        Ok(self.exit_code)
    }
}

#[test]
fn extension_editor_renders_title_prefill_and_editor_hints() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let component = ExtensionEditorComponent::new(
        &keybindings,
        "Custom summarization instructions",
        Some("line one\nline two"),
    );

    let lines = component.render(120);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Custom summarization instructions"))
    );
    assert!(lines.iter().any(|line| line.contains("line one")));
    assert!(lines.iter().any(|line| line.contains("line two")));
    assert!(lines.iter().any(|line| line.contains("submit")));
    assert!(lines.iter().any(|line| line.contains("newline")));
    assert!(lines.iter().any(|line| line.contains("cancel")));
}

#[test]
fn extension_editor_submit_flows_through_wrapped_editor() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component = ExtensionEditorComponent::new(&keybindings, "Title", None);
    component.set_on_submit(move |value| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") = Some(value);
    });

    component.handle_input("h");
    component.handle_input("i");
    component.handle_input("\r");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("hi")
    );
    assert_eq!(component.get_text(), "");
}

#[test]
fn extension_editor_cancel_binding_uses_callback_and_preserves_text() {
    let cancel_calls = Arc::new(Mutex::new(0usize));
    let cancel_calls_for_callback = Arc::clone(&cancel_calls);

    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component = ExtensionEditorComponent::new(&keybindings, "Title", Some("draft"));
    component.set_on_cancel(move || {
        *cancel_calls_for_callback
            .lock()
            .expect("cancel mutex poisoned") += 1;
    });

    component.handle_input("\x1b");

    assert_eq!(*cancel_calls.lock().expect("cancel mutex poisoned"), 1);
    assert_eq!(component.get_text(), "draft");
}

#[test]
fn extension_editor_external_editor_binding_invokes_callback_and_does_not_mutate_text() {
    let external_calls = Arc::new(Mutex::new(0usize));
    let external_calls_for_callback = Arc::clone(&external_calls);

    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component = ExtensionEditorComponent::new(&keybindings, "Title", Some("draft"));
    component.set_on_external_editor(move || {
        *external_calls_for_callback
            .lock()
            .expect("external mutex poisoned") += 1;
    });

    component.handle_input("\x07");

    assert_eq!(*external_calls.lock().expect("external mutex poisoned"), 1);
    assert_eq!(component.get_text(), "draft");
}

#[test]
fn extension_editor_default_external_editor_action_updates_text_and_requests_render() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component = ExtensionEditorComponent::new(&keybindings, "Title", Some("draft"));

    let host = RecordingHost::default();
    let host_events = Arc::clone(&host.events);
    let runner = RecordingRunner::with_result(Some("edited from external\n"), Some(0));
    let runner_calls = Arc::clone(&runner.calls);

    component.set_external_editor_host(host);
    component.set_external_editor_command_runner(runner);
    component.set_external_editor_command("mock-editor --wait");

    component.handle_input("\x07");

    assert_eq!(component.get_text(), "edited from external");
    assert_eq!(
        runner_calls
            .lock()
            .expect("runner calls mutex poisoned")
            .as_slice(),
        &[(String::from("mock-editor --wait"), String::from("draft"))]
    );
    assert_eq!(
        host_events
            .lock()
            .expect("host events mutex poisoned")
            .as_slice(),
        [
            String::from("stop"),
            String::from("start"),
            String::from("request_render")
        ]
    );
}

#[test]
fn extension_editor_external_editor_binding_is_consumed_even_without_callback() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component = ExtensionEditorComponent::new(&keybindings, "Title", Some("draft"));
    let runner = RecordingRunner::with_result(None, Some(1));
    component.set_external_editor_command_runner(runner);
    component.set_external_editor_command("mock-editor");

    component.handle_input("\x07");

    assert_eq!(component.get_text(), "draft");
}

#[test]
fn extension_editor_external_editor_callback_takes_precedence_over_default_action() {
    let external_calls = Arc::new(Mutex::new(0usize));
    let external_calls_for_callback = Arc::clone(&external_calls);

    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut component = ExtensionEditorComponent::new(&keybindings, "Title", Some("draft"));
    let runner = RecordingRunner::with_result(Some("edited from external\n"), Some(0));
    let runner_calls = Arc::clone(&runner.calls);
    component.set_external_editor_command_runner(runner);
    component.set_external_editor_command("mock-editor --wait");
    component.set_on_external_editor(move || {
        *external_calls_for_callback
            .lock()
            .expect("external mutex poisoned") += 1;
    });

    component.handle_input("\x07");

    assert_eq!(*external_calls.lock().expect("external mutex poisoned"), 1);
    assert!(
        runner_calls
            .lock()
            .expect("runner calls mutex poisoned")
            .is_empty()
    );
    assert_eq!(component.get_text(), "draft");
}

#[test]
fn extension_editor_uses_configured_cancel_and_external_editor_keybindings() {
    let cancel_calls = Arc::new(Mutex::new(0usize));
    let cancel_calls_for_callback = Arc::clone(&cancel_calls);
    let external_calls = Arc::new(Mutex::new(0usize));
    let external_calls_for_callback = Arc::clone(&external_calls);

    let keybindings = KeybindingsManager::new(
        config(&[
            ("tui.select.cancel", &["ctrl+x"]),
            ("app.editor.external", &["alt+e"]),
        ]),
        None,
    );
    let mut component = ExtensionEditorComponent::new(&keybindings, "Title", Some("draft"));
    component.set_on_cancel(move || {
        *cancel_calls_for_callback
            .lock()
            .expect("cancel mutex poisoned") += 1;
    });
    component.set_on_external_editor(move || {
        *external_calls_for_callback
            .lock()
            .expect("external mutex poisoned") += 1;
    });

    component.handle_input("\x18");
    component.handle_input("\x1be");

    assert_eq!(*cancel_calls.lock().expect("cancel mutex poisoned"), 1);
    assert_eq!(*external_calls.lock().expect("external mutex poisoned"), 1);
    assert_eq!(component.get_text(), "draft");
}
