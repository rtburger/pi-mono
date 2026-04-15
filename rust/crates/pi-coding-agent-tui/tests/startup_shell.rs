use pi_coding_agent_core::{FooterDataProvider, FooterDataSnapshot};
use pi_coding_agent_tui::{
    ClipboardImage, ClipboardImageSource, ExternalEditorCommandRunner, ExternalEditorHost,
    FooterState, KeyId, KeybindingsManager, PlainKeyHintStyler, StartupShellComponent,
};
use pi_events::Model;
use pi_tui::{
    AutocompleteProvider, AutocompleteSuggestions, Component, Terminal, Text, Tui, TuiError,
    apply_completion, visible_width,
};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_file(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(path, content).expect("failed to write file");
}

fn create_plain_repo(temp_dir: &Path, repo_name: &str, branch: &str) -> PathBuf {
    let repo_dir = temp_dir.join(repo_name);
    fs::create_dir_all(repo_dir.join(".git")).expect("failed to create repo .git dir");
    write_file(
        repo_dir.join(".git/HEAD"),
        &format!("ref: refs/heads/{branch}\n"),
    );
    repo_dir
}

#[derive(Default)]
struct NoopTerminal;

#[derive(Clone)]
struct RecordingTerminal {
    writes: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone, Default)]
struct RecordingExternalEditorHost {
    events: Arc<Mutex<Vec<String>>>,
}

impl ExternalEditorHost for RecordingExternalEditorHost {
    fn stop(&self) {
        self.events
            .lock()
            .expect("external editor host events mutex poisoned")
            .push(String::from("stop"));
    }

    fn start(&self) {
        self.events
            .lock()
            .expect("external editor host events mutex poisoned")
            .push(String::from("start"));
    }

    fn request_render(&self) {
        self.events
            .lock()
            .expect("external editor host events mutex poisoned")
            .push(String::from("request_render"));
    }
}

#[derive(Clone, Default)]
struct RecordingExternalEditorRunner {
    calls: Arc<Mutex<Vec<(String, String)>>>,
    replacement: Option<String>,
    exit_code: Option<i32>,
}

impl RecordingExternalEditorRunner {
    fn with_result(replacement: Option<&str>, exit_code: Option<i32>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            replacement: replacement.map(str::to_owned),
            exit_code,
        }
    }
}

impl ExternalEditorCommandRunner for RecordingExternalEditorRunner {
    fn run(&self, command: &str, file_path: &Path) -> io::Result<Option<i32>> {
        let current_text = fs::read_to_string(file_path)?;
        self.calls
            .lock()
            .expect("external editor runner calls mutex poisoned")
            .push((command.to_owned(), current_text));
        if let Some(replacement) = &self.replacement {
            fs::write(file_path, replacement)?;
        }
        Ok(self.exit_code)
    }
}

impl RecordingTerminal {
    fn new() -> Self {
        Self {
            writes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn write_count(&self) -> usize {
        self.writes.lock().expect("writes mutex poisoned").len()
    }

    fn writes(&self) -> Vec<String> {
        self.writes.lock().expect("writes mutex poisoned").clone()
    }
}

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

impl Terminal for RecordingTerminal {
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

    fn write(&mut self, data: &str) -> Result<(), TuiError> {
        self.writes
            .lock()
            .expect("writes mutex poisoned")
            .push(data.to_owned());
        Ok(())
    }

    fn columns(&self) -> u16 {
        120
    }

    fn rows(&self) -> u16 {
        6
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

fn model(id: &str, provider: &str, reasoning: bool) -> Model {
    Model {
        id: id.to_owned(),
        name: id.to_owned(),
        api: "openai-responses".to_owned(),
        provider: provider.to_owned(),
        base_url: String::new(),
        reasoning,
        input: vec!["text".to_owned()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 200_000,
        max_tokens: 8_192,
        compat: None,
    }
}

struct StaticClipboardImageSource {
    image: Option<ClipboardImage>,
}

impl ClipboardImageSource for StaticClipboardImageSource {
    fn read_clipboard_image(&self) -> std::io::Result<Option<ClipboardImage>> {
        Ok(self.image.clone())
    }
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
    assert!(lines.len() >= 4);
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

    assert_eq!(lines.len(), 3);
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
    assert!(lines.iter().any(|line| line.contains("hi")));

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
fn startup_shell_supports_multiline_prompt_editing_via_custom_editor_bindings() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

    let keybindings = KeybindingsManager::new(
        config(&[
            ("tui.input.newLine", &["ctrl+x"]),
            ("tui.input.submit", &["ctrl+s"]),
        ]),
        None,
    );
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

    shell.handle_input("h");
    shell.handle_input("i");
    shell.handle_input("\x18");
    shell.handle_input("t");
    shell.handle_input("h");
    shell.handle_input("e");
    shell.handle_input("r");
    shell.handle_input("e");

    assert_eq!(shell.input_value(), "hi\nthere");
    let lines = shell.render(20);
    assert!(lines.iter().any(|line| line.contains("hi")));
    assert!(lines.iter().any(|line| line.contains("there")));

    shell.handle_input("\x13");
    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("hi\nthere")
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

#[test]
fn startup_shell_interrupt_uses_app_keybinding_binding_and_escape_callback() {
    let interrupted = Arc::new(Mutex::new(0usize));
    let interrupted_for_callback = Arc::clone(&interrupted);

    let keybindings = KeybindingsManager::new(config(&[("app.interrupt", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_on_escape(move || {
        *interrupted_for_callback
            .lock()
            .expect("interrupt mutex poisoned") += 1;
    });

    shell.handle_input("\x18");
    assert_eq!(*interrupted.lock().expect("interrupt mutex poisoned"), 1);

    shell.handle_input("\x1b");
    assert_eq!(*interrupted.lock().expect("interrupt mutex poisoned"), 1);
}

#[test]
fn startup_shell_interrupt_cancels_autocomplete_before_escape_callback() {
    struct MultiSuggestionProvider;

    impl AutocompleteProvider for MultiSuggestionProvider {
        fn get_suggestions(
            &self,
            lines: &[String],
            _cursor_line: usize,
            cursor_col: usize,
            force: bool,
        ) -> Option<AutocompleteSuggestions> {
            if !force {
                return None;
            }
            let text = lines.first().map(String::as_str).unwrap_or("");
            let prefix = &text[..cursor_col.min(text.len())];
            (prefix == "src").then(|| AutocompleteSuggestions {
                items: vec![
                    pi_tui::AutocompleteItem {
                        value: String::from("src/"),
                        label: String::from("src/"),
                        description: None,
                    },
                    pi_tui::AutocompleteItem {
                        value: String::from("src.txt"),
                        label: String::from("src.txt"),
                        description: None,
                    },
                ],
                prefix: String::from("src"),
            })
        }

        fn apply_completion(
            &self,
            lines: &[String],
            cursor_line: usize,
            cursor_col: usize,
            item: &pi_tui::AutocompleteItem,
            prefix: &str,
        ) -> pi_tui::CompletionResult {
            apply_completion(lines, cursor_line, cursor_col, item, prefix)
        }
    }

    let interrupted = Arc::new(Mutex::new(0usize));
    let interrupted_for_callback = Arc::clone(&interrupted);

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
    shell.set_autocomplete_provider(Arc::new(MultiSuggestionProvider));
    shell.set_on_escape(move || {
        *interrupted_for_callback
            .lock()
            .expect("interrupt mutex poisoned") += 1;
    });

    shell.handle_input("s");
    shell.handle_input("r");
    shell.handle_input("c");
    shell.handle_input("\t");
    assert!(shell.render(20).iter().any(|line| line.contains("src.txt")));

    shell.handle_input("\x1b");

    assert_eq!(*interrupted.lock().expect("interrupt mutex poisoned"), 0);
    assert_eq!(shell.input_value(), "src");
    assert!(!shell.render(20).iter().any(|line| line.contains("src.txt")));
}

#[test]
fn startup_shell_submit_accepts_autocomplete_before_submitting_prompt() {
    struct ReadmeSuggestionProvider;

    impl AutocompleteProvider for ReadmeSuggestionProvider {
        fn get_suggestions(
            &self,
            lines: &[String],
            _cursor_line: usize,
            cursor_col: usize,
            force: bool,
        ) -> Option<AutocompleteSuggestions> {
            if !force {
                return None;
            }
            let text = lines.first().map(String::as_str).unwrap_or("");
            let prefix = &text[..cursor_col.min(text.len())];
            (prefix == "rea").then(|| AutocompleteSuggestions {
                items: vec![
                    pi_tui::AutocompleteItem {
                        value: String::from("readme.md"),
                        label: String::from("readme.md"),
                        description: None,
                    },
                    pi_tui::AutocompleteItem {
                        value: String::from("readme.txt"),
                        label: String::from("readme.txt"),
                        description: None,
                    },
                ],
                prefix: String::from("rea"),
            })
        }

        fn apply_completion(
            &self,
            lines: &[String],
            cursor_line: usize,
            cursor_col: usize,
            item: &pi_tui::AutocompleteItem,
            prefix: &str,
        ) -> pi_tui::CompletionResult {
            apply_completion(lines, cursor_line, cursor_col, item, prefix)
        }
    }

    let submitted = Arc::new(Mutex::new(Vec::<String>::new()));
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
    shell.set_autocomplete_provider(Arc::new(ReadmeSuggestionProvider));
    shell.set_on_submit(move |value| {
        submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned")
            .push(value);
    });

    shell.handle_input("r");
    shell.handle_input("e");
    shell.handle_input("a");
    shell.handle_input("\t");
    assert!(
        shell
            .render(30)
            .iter()
            .any(|line| line.contains("readme.txt"))
    );

    shell.handle_input("\r");
    assert_eq!(shell.input_value(), "readme.md");
    assert!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .is_empty()
    );

    shell.handle_input("\r");
    assert_eq!(shell.input_value(), "");
    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_slice(),
        [String::from("readme.md")]
    );
}

#[test]
fn startup_shell_clear_binding_clears_input_by_default() {
    let exits = Arc::new(Mutex::new(0usize));
    let exits_for_callback = Arc::clone(&exits);

    let keybindings = KeybindingsManager::new(config(&[("app.clear", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_on_exit(move || {
        *exits_for_callback.lock().expect("exit mutex poisoned") += 1;
    });
    shell.set_input_value("draft prompt");

    shell.handle_input("\x18");

    assert_eq!(shell.input_value(), "");
    assert_eq!(*exits.lock().expect("exit mutex poisoned"), 0);
}

#[test]
fn startup_shell_second_clear_binding_within_window_uses_exit_handler() {
    let exits = Arc::new(Mutex::new(0usize));
    let exits_for_callback = Arc::clone(&exits);

    let keybindings = KeybindingsManager::new(config(&[("app.clear", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_on_exit(move || {
        *exits_for_callback.lock().expect("exit mutex poisoned") += 1;
    });
    shell.set_input_value("draft prompt");

    shell.handle_input("\x18");
    assert_eq!(shell.input_value(), "");
    assert_eq!(*exits.lock().expect("exit mutex poisoned"), 0);

    shell.handle_input("\x18");
    assert_eq!(*exits.lock().expect("exit mutex poisoned"), 1);
}

#[test]
fn startup_shell_extension_shortcut_can_consume_or_fall_through() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_for_callback = Arc::clone(&seen);

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
    shell.set_on_extension_shortcut(move |data| {
        seen_for_callback
            .lock()
            .expect("shortcut mutex poisoned")
            .push(data.clone());
        data == "x"
    });

    shell.handle_input("x");
    assert_eq!(shell.input_value(), "");

    shell.handle_input("y");
    assert_eq!(shell.input_value(), "y");
    assert_eq!(
        seen.lock().expect("shortcut mutex poisoned").as_slice(),
        [String::from("x"), String::from("y")]
    );
}

#[test]
fn startup_shell_extension_shortcut_runs_before_paste_image_binding() {
    let shortcut_calls = Arc::new(Mutex::new(0usize));
    let shortcut_calls_for_callback = Arc::clone(&shortcut_calls);
    let paste_calls = Arc::new(Mutex::new(0usize));
    let paste_calls_for_callback = Arc::clone(&paste_calls);

    let keybindings =
        KeybindingsManager::new(config(&[("app.clipboard.pasteImage", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_on_extension_shortcut(move |data| {
        if data == "\x18" {
            *shortcut_calls_for_callback
                .lock()
                .expect("shortcut mutex poisoned") += 1;
            return true;
        }
        false
    });
    shell.set_on_paste_image(move || {
        *paste_calls_for_callback
            .lock()
            .expect("paste mutex poisoned") += 1;
    });

    shell.handle_input("\x18");

    assert_eq!(*shortcut_calls.lock().expect("shortcut mutex poisoned"), 1);
    assert_eq!(*paste_calls.lock().expect("paste mutex poisoned"), 0);
    assert_eq!(shell.input_value(), "");
}

#[test]
fn startup_shell_paste_image_uses_app_keybinding_binding() {
    let paste_calls = Arc::new(Mutex::new(0usize));
    let paste_calls_for_callback = Arc::clone(&paste_calls);

    let keybindings =
        KeybindingsManager::new(config(&[("app.clipboard.pasteImage", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("draft prompt");
    shell.set_on_paste_image(move || {
        *paste_calls_for_callback
            .lock()
            .expect("paste mutex poisoned") += 1;
    });

    shell.handle_input("\x18");

    assert_eq!(*paste_calls.lock().expect("paste mutex poisoned"), 1);
    assert_eq!(shell.input_value(), "draft prompt");
}

#[test]
fn startup_shell_default_paste_image_writes_temp_file_and_inserts_path() {
    let temp_dir = TestDir::new("startup-shell-paste-image");
    let keybindings =
        KeybindingsManager::new(config(&[("app.clipboard.pasteImage", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("prefix  suffix");
    shell.set_input_cursor("prefix ".len());
    shell.set_clipboard_image_source(
        StaticClipboardImageSource {
            image: Some(ClipboardImage {
                bytes: vec![1, 2, 3],
                mime_type: "image/png".to_owned(),
            }),
        },
        temp_dir.path(),
    );

    shell.handle_input("\x18");

    let written_paths = fs::read_dir(temp_dir.path())
        .expect("temp dir should be readable")
        .map(|entry| entry.expect("dir entry should be readable").path())
        .collect::<Vec<_>>();
    assert_eq!(written_paths.len(), 1);
    let written_path = &written_paths[0];
    assert_eq!(
        fs::read(written_path).expect("written file should be readable"),
        vec![1, 2, 3]
    );

    let expected = format!("prefix {} suffix", written_path.to_string_lossy());
    assert_eq!(shell.input_value(), expected);
}

#[test]
fn startup_shell_default_paste_image_ignores_empty_clipboard() {
    let temp_dir = TestDir::new("startup-shell-empty-paste-image");
    let keybindings =
        KeybindingsManager::new(config(&[("app.clipboard.pasteImage", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("draft prompt");
    shell.set_clipboard_image_source(StaticClipboardImageSource { image: None }, temp_dir.path());

    shell.handle_input("\x18");

    assert_eq!(shell.input_value(), "draft prompt");
    assert!(
        fs::read_dir(temp_dir.path())
            .expect("temp dir should be readable")
            .next()
            .is_none()
    );
}

#[test]
fn startup_shell_can_show_extension_editor_and_restore_hidden_prompt_after_submit() {
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
    shell.set_input_value("hidden prompt");
    shell.show_extension_editor(
        "Extension editor",
        Some("prefill"),
        move |value| {
            *submitted_for_callback
                .lock()
                .expect("submitted mutex poisoned") = Some(value);
        },
        || {},
    );

    assert!(shell.is_showing_extension_editor());
    let lines = shell.render(60);
    assert!(lines.iter().any(|line| line.contains("Extension editor")));
    assert!(!lines.iter().any(|line| line.contains("hidden prompt")));

    shell.handle_input("x");
    shell.handle_input("\r");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("prefillx")
    );
    assert!(!shell.is_showing_extension_editor());
    assert_eq!(shell.input_value(), "hidden prompt");
}

#[test]
fn startup_shell_can_cancel_extension_editor_and_restore_hidden_prompt() {
    let cancelled = Arc::new(Mutex::new(0usize));
    let cancelled_for_callback = Arc::clone(&cancelled);
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
    shell.set_input_value("hidden prompt");
    shell.show_extension_editor(
        "Extension editor",
        Some("prefill"),
        |_| {},
        move || {
            *cancelled_for_callback
                .lock()
                .expect("cancelled mutex poisoned") += 1;
        },
    );

    assert!(shell.is_showing_extension_editor());
    shell.handle_input("x");
    shell.handle_input("\x1b");

    assert_eq!(*cancelled.lock().expect("cancelled mutex poisoned"), 1);
    assert!(!shell.is_showing_extension_editor());
    assert_eq!(shell.input_value(), "hidden prompt");
}

#[test]
fn startup_shell_app_editor_external_opens_prompt_extension_editor_and_restores_edited_prompt() {
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
    let host = RecordingExternalEditorHost::default();
    let host_events = Arc::clone(&host.events);
    let runner =
        RecordingExternalEditorRunner::with_result(Some("edited from external\n"), Some(0));
    let runner_calls = Arc::clone(&runner.calls);
    shell.set_extension_editor_host(host);
    shell.set_extension_editor_command_runner(runner);
    shell.set_extension_editor_command("mock-editor --wait");
    shell.set_input_value("draft prompt");

    shell.handle_input("\x07");

    assert!(shell.is_showing_extension_editor());
    let lines = shell.render(60);
    assert!(lines.iter().any(|line| line.contains("Edit message")));
    assert!(lines.iter().any(|line| line.contains("draft prompt")));

    shell.handle_input("\x07");

    assert_eq!(
        runner_calls
            .lock()
            .expect("external editor runner calls mutex poisoned")
            .as_slice(),
        &[(
            String::from("mock-editor --wait"),
            String::from("draft prompt")
        )]
    );
    assert_eq!(
        host_events
            .lock()
            .expect("external editor host events mutex poisoned")
            .as_slice(),
        [
            String::from("stop"),
            String::from("start"),
            String::from("request_render")
        ]
    );

    shell.handle_input("\r");

    assert!(!shell.is_showing_extension_editor());
    assert_eq!(shell.input_value(), "edited from external");
}

#[test]
fn startup_shell_registered_external_editor_action_overrides_default_prompt_editor() {
    let external_calls = Arc::new(Mutex::new(0usize));
    let external_calls_for_handler = Arc::clone(&external_calls);

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
    shell.set_input_value("draft prompt");
    shell.on_action("app.editor.external", move || {
        *external_calls_for_handler
            .lock()
            .expect("external editor action mutex poisoned") += 1;
    });

    shell.handle_input("\x07");

    assert_eq!(
        *external_calls
            .lock()
            .expect("external editor action mutex poisoned"),
        1
    );
    assert!(!shell.is_showing_extension_editor());
    assert_eq!(shell.input_value(), "draft prompt");
}

#[test]
fn startup_shell_can_show_model_selector_and_restore_hidden_prompt_after_submit() {
    let selected = Arc::new(Mutex::new(None::<String>));
    let selected_for_callback = Arc::clone(&selected);
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
    shell.set_input_value("hidden prompt");
    shell.show_model_selector(
        Some(model("alpha", "openai", true)),
        vec![
            model("alpha", "openai", true),
            model("beta", "anthropic", true),
        ],
        Some("beta"),
        move |model| {
            *selected_for_callback
                .lock()
                .expect("selected mutex poisoned") = Some(model.id);
        },
        || {},
    );

    assert!(shell.is_showing_model_selector());
    let lines = shell.render(60);
    assert!(lines.iter().any(|line| line.contains("Select model")));
    assert!(lines.iter().any(|line| line.contains("beta [anthropic]")));
    assert!(!lines.iter().any(|line| line.contains("hidden prompt")));

    shell.handle_input("\r");

    assert_eq!(
        selected.lock().expect("selected mutex poisoned").as_deref(),
        Some("beta")
    );
    assert!(!shell.is_showing_model_selector());
    assert_eq!(shell.input_value(), "hidden prompt");
}

#[test]
fn startup_shell_can_cancel_model_selector_and_restore_hidden_prompt() {
    let cancelled = Arc::new(Mutex::new(0usize));
    let cancelled_for_callback = Arc::clone(&cancelled);
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
    shell.set_input_value("hidden prompt");
    shell.show_model_selector(
        Some(model("alpha", "openai", true)),
        vec![
            model("alpha", "openai", true),
            model("beta", "anthropic", true),
        ],
        Some("beta"),
        |_| {},
        move || {
            *cancelled_for_callback
                .lock()
                .expect("cancelled mutex poisoned") += 1;
        },
    );

    assert!(shell.is_showing_model_selector());
    shell.handle_input("\x1b");

    assert_eq!(*cancelled.lock().expect("cancelled mutex poisoned"), 1);
    assert!(!shell.is_showing_model_selector());
    assert_eq!(shell.input_value(), "hidden prompt");
}

#[test]
fn startup_shell_paste_image_callback_overrides_default_clipboard_insert() {
    let temp_dir = TestDir::new("startup-shell-paste-image-callback");
    let paste_calls = Arc::new(Mutex::new(0usize));
    let paste_calls_for_callback = Arc::clone(&paste_calls);

    let keybindings =
        KeybindingsManager::new(config(&[("app.clipboard.pasteImage", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("draft prompt");
    shell.set_clipboard_image_source(
        StaticClipboardImageSource {
            image: Some(ClipboardImage {
                bytes: vec![1, 2, 3],
                mime_type: "image/png".to_owned(),
            }),
        },
        temp_dir.path(),
    );
    shell.set_on_paste_image(move || {
        *paste_calls_for_callback
            .lock()
            .expect("paste mutex poisoned") += 1;
    });

    shell.handle_input("\x18");

    assert_eq!(*paste_calls.lock().expect("paste mutex poisoned"), 1);
    assert_eq!(shell.input_value(), "draft prompt");
    assert!(
        fs::read_dir(temp_dir.path())
            .expect("temp dir should be readable")
            .next()
            .is_none()
    );
}

#[test]
fn startup_shell_can_run_registered_app_action_handlers() {
    let clear_calls = Arc::new(Mutex::new(0usize));
    let clear_calls_for_handler = Arc::clone(&clear_calls);

    let keybindings = KeybindingsManager::new(config(&[("app.clear", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.on_action("app.clear", move || {
        *clear_calls_for_handler
            .lock()
            .expect("clear mutex poisoned") += 1;
    });
    shell.set_input_value("draft prompt");

    shell.handle_input("\x18");

    assert_eq!(*clear_calls.lock().expect("clear mutex poisoned"), 1);
    assert_eq!(shell.input_value(), "draft prompt");
}

#[test]
fn startup_shell_shell_aware_action_handler_can_open_model_selector() {
    let keybindings = KeybindingsManager::new(config(&[("app.model.select", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("hidden prompt");
    shell.on_action_with_shell("app.model.select", |shell| {
        shell.show_model_selector(
            Some(model("alpha", "openai", true)),
            vec![
                model("alpha", "openai", true),
                model("beta", "anthropic", true),
            ],
            None,
            |_| {},
            || {},
        );
    });

    shell.handle_input("\x18");

    assert!(shell.is_showing_model_selector());
    assert_eq!(shell.input_value(), "hidden prompt");
    let lines = shell.render(60);
    assert!(lines.iter().any(|line| line.contains("Select model")));
    assert!(!lines.iter().any(|line| line.contains("hidden prompt")));
}

#[test]
fn startup_shell_follow_up_binding_submits_current_input_by_default() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

    let keybindings =
        KeybindingsManager::new(config(&[("app.message.followUp", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("queued message");
    shell.set_on_submit(move |value| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") = Some(value);
    });

    shell.handle_input("\x18");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("queued message")
    );
}

#[test]
fn startup_shell_follow_up_binding_ignores_whitespace_only_input() {
    let submitted = Arc::new(Mutex::new(0usize));
    let submitted_for_callback = Arc::clone(&submitted);

    let keybindings =
        KeybindingsManager::new(config(&[("app.message.followUp", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("   ");
    shell.set_on_submit(move |_| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") += 1;
    });

    shell.handle_input("\x18");

    assert_eq!(*submitted.lock().expect("submitted mutex poisoned"), 0);
}

#[test]
fn startup_shell_registered_follow_up_action_overrides_default_submit() {
    let submitted = Arc::new(Mutex::new(0usize));
    let submitted_for_callback = Arc::clone(&submitted);
    let follow_up_calls = Arc::new(Mutex::new(0usize));
    let follow_up_calls_for_handler = Arc::clone(&follow_up_calls);

    let keybindings =
        KeybindingsManager::new(config(&[("app.message.followUp", &["ctrl+x"])]), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.set_input_value("queued message");
    shell.set_on_submit(move |_| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") += 1;
    });
    shell.on_action("app.message.followUp", move || {
        *follow_up_calls_for_handler
            .lock()
            .expect("follow-up mutex poisoned") += 1;
    });

    shell.handle_input("\x18");

    assert_eq!(
        *follow_up_calls.lock().expect("follow-up mutex poisoned"),
        1
    );
    assert_eq!(*submitted.lock().expect("submitted mutex poisoned"), 0);
}

#[test]
fn startup_shell_exit_handler_only_runs_when_input_is_empty() {
    let exits = Arc::new(Mutex::new(0usize));
    let exits_for_callback = Arc::clone(&exits);

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
    shell.set_on_exit(move || {
        *exits_for_callback.lock().expect("exit mutex poisoned") += 1;
    });

    shell.set_input_value("abc");
    shell.handle_input("\x01");
    shell.handle_input("\x04");
    assert_eq!(shell.input_value(), "bc");
    assert_eq!(*exits.lock().expect("exit mutex poisoned"), 0);

    shell.clear_input();
    shell.handle_input("\x04");
    assert_eq!(*exits.lock().expect("exit mutex poisoned"), 1);
}

#[test]
fn startup_shell_renders_transcript_before_pending_messages_and_prompt() {
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
    shell.add_transcript_item(Box::new(Text::new("first transcript item", 0, 0)));
    shell.add_transcript_item(Box::new(Text::new("second transcript item", 0, 0)));
    shell.set_pending_messages(
        &PlainKeyHintStyler,
        ["queued steering message"],
        ["queued follow-up message"],
    );

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(60, 20);
    let first_transcript = lines
        .iter()
        .position(|line| line.contains("first transcript item"))
        .expect("first transcript line should render");
    let second_transcript = lines
        .iter()
        .position(|line| line.contains("second transcript item"))
        .expect("second transcript line should render");
    let steering = lines
        .iter()
        .position(|line| line.contains("Steering: queued steering message"))
        .expect("steering line should render");
    let follow_up = lines
        .iter()
        .position(|line| line.contains("Follow-up: queued follow-up message"))
        .expect("follow-up line should render");
    let prompt_start = lines.len().saturating_sub(3);

    assert!(first_transcript < second_transcript);
    assert!(second_transcript < steering);
    assert!(steering < follow_up);
    assert!(follow_up < prompt_start);
}

#[test]
fn startup_shell_renders_status_message_between_pending_messages_and_prompt() {
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
    shell.set_pending_messages(
        &PlainKeyHintStyler,
        ["queued steering message"],
        std::iter::empty::<&str>(),
    );
    shell.set_status_message("Working...");

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(40, 10);
    let pending = lines
        .iter()
        .position(|line| line.contains("Steering: queued steering message"))
        .expect("pending line should render");
    let status = lines
        .iter()
        .position(|line| line.contains("Working..."))
        .expect("status line should render");
    let prompt_start = lines.len().saturating_sub(3);

    assert!(pending < status);
    assert!(status < prompt_start);
}

#[test]
fn startup_shell_budgets_transcript_height_for_status_line() {
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
    for index in 1..=6 {
        shell.add_transcript_item(Box::new(Text::new(format!("line {index}"), 0, 0)));
    }
    shell.set_status_message("Working...");
    shell.set_viewport_size(24, 4);

    let lines = shell.render(24);

    assert_eq!(lines.len(), 4);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("line 5") || line.contains("line 6"))
    );
    assert!(lines[0].contains("Working..."));
    assert!(lines[1].chars().all(|character| character == '─'));
    assert!(lines[3].chars().all(|character| character == '─'));
}

#[test]
fn startup_shell_truncates_and_clears_status_message() {
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
    shell.set_status_message("this is a very long working message that must be truncated");

    let lines = shell.render(24);
    assert_eq!(lines.len(), 4);
    assert!(visible_width(&lines[0]) <= 24);
    assert!(lines[0].contains("..."));
    assert!(lines[1].chars().all(|character| character == '─'));
    assert!(lines[3].chars().all(|character| character == '─'));

    shell.clear_status_message();
    let cleared = shell.render(24);
    assert_eq!(cleared.len(), 3);
    assert!(cleared[0].chars().all(|character| character == '─'));
    assert!(cleared[2].chars().all(|character| character == '─'));
}

#[test]
fn startup_shell_status_handle_updates_shell_after_component_is_moved() {
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
    let status_handle = shell.status_handle();

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    status_handle.set_message("Working...");
    let lines = tui.render_for_size(24, 10);
    assert!(lines.iter().any(|line| line.contains("Working...")));

    status_handle.clear();
    let cleared = tui.render_for_size(24, 10);
    assert!(!cleared.iter().any(|line| line.contains("Working...")));
    assert_eq!(cleared.len(), 3);
}

#[test]
fn startup_shell_status_handle_can_queue_tui_rerenders() {
    let terminal = RecordingTerminal::new();
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let render_handle = tui.render_handle();

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
    let status_handle = shell.status_handle_with_render_handle(render_handle);

    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));
    tui.start().expect("start should succeed");
    let writes_before = inspector.write_count();

    status_handle.set_message("Working...");
    tui.drain_terminal_events()
        .expect("queued status rerender should drain successfully");
    assert!(inspector.write_count() > writes_before);
    assert!(
        tui.render_current()
            .iter()
            .any(|line| line.contains("Working..."))
    );

    let writes_after_set = inspector.write_count();
    status_handle.clear();
    tui.drain_terminal_events()
        .expect("queued status clear rerender should drain successfully");
    assert!(inspector.write_count() > writes_after_set);
    assert!(
        !tui.render_current()
            .iter()
            .any(|line| line.contains("Working..."))
    );

    tui.stop().expect("stop should succeed");
}

#[test]
fn startup_shell_clips_transcript_to_remaining_viewport_height() {
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
    for index in 1..=6 {
        shell.add_transcript_item(Box::new(Text::new(format!("line {index}"), 0, 0)));
    }
    shell.set_viewport_size(24, 4);

    let lines = shell.render(24);

    assert_eq!(lines.len(), 4);
    assert!(lines[0].contains("line 6"));
    assert!(lines[1].chars().all(|character| character == '─'));
    assert!(lines[3].chars().all(|character| character == '─'));
}

#[test]
fn startup_shell_can_scroll_transcript_without_hiding_prompt() {
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
    for index in 1..=6 {
        shell.add_transcript_item(Box::new(Text::new(format!("line {index}"), 0, 0)));
    }
    shell.set_viewport_size(24, 4);
    shell.scroll_transcript_up(2);

    let scrolled_up = shell.render(24);
    assert_eq!(shell.transcript_scroll_offset(), 2);
    assert!(scrolled_up[0].contains("line 4"));
    assert!(scrolled_up[1].chars().all(|character| character == '─'));
    assert!(scrolled_up[3].chars().all(|character| character == '─'));

    shell.scroll_transcript_down(1);
    let scrolled_down = shell.render(24);
    assert_eq!(shell.transcript_scroll_offset(), 1);
    assert!(scrolled_down[0].contains("line 5"));
    assert!(scrolled_down[1].chars().all(|character| character == '─'));
    assert!(scrolled_down[3].chars().all(|character| character == '─'));

    shell.scroll_transcript_to_bottom();
    let bottom = shell.render(24);
    assert_eq!(shell.transcript_scroll_offset(), 0);
    assert!(bottom[0].contains("line 6"));
    assert!(bottom[1].chars().all(|character| character == '─'));
    assert!(bottom[3].chars().all(|character| character == '─'));
}

#[test]
fn startup_shell_page_keys_scroll_transcript_by_visible_page() {
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
    for index in 1..=8 {
        shell.add_transcript_item(Box::new(Text::new(format!("line {index}"), 0, 0)));
    }
    shell.set_input_value("draft prompt");
    shell.set_viewport_size(24, 5);

    let bottom = shell.render(24);
    assert!(bottom[0].contains("line 7"));
    assert!(bottom[1].contains("line 8"));
    assert!(bottom[2].chars().all(|character| character == '─'));
    assert!(bottom[4].chars().all(|character| character == '─'));

    shell.handle_input("\x1b[5~");
    let page_up = shell.render(24);
    assert_eq!(shell.transcript_scroll_offset(), 2);
    assert!(page_up[0].contains("line 5"));
    assert!(page_up[1].contains("line 6"));
    assert!(page_up[2].chars().all(|character| character == '─'));
    assert!(page_up[4].chars().all(|character| character == '─'));
    assert_eq!(shell.input_value(), "draft prompt");

    shell.handle_input("\x1b[6~");
    let page_down = shell.render(24);
    assert_eq!(shell.transcript_scroll_offset(), 0);
    assert!(page_down[0].contains("line 7"));
    assert!(page_down[1].contains("line 8"));
    assert!(page_down[2].chars().all(|character| character == '─'));
    assert!(page_down[4].chars().all(|character| character == '─'));
}

#[test]
fn startup_shell_page_keys_use_configured_keybindings() {
    let keybindings = KeybindingsManager::new(
        config(&[
            ("tui.editor.pageUp", &["alt+p"]),
            ("tui.editor.pageDown", &["alt+n"]),
        ]),
        None,
    );
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    for index in 1..=8 {
        shell.add_transcript_item(Box::new(Text::new(format!("line {index}"), 0, 0)));
    }
    shell.set_viewport_size(24, 5);
    let _ = shell.render(24);

    shell.handle_input("\x1bp");
    assert_eq!(shell.transcript_scroll_offset(), 2);

    shell.handle_input("\x1bn");
    assert_eq!(shell.transcript_scroll_offset(), 0);
}

#[test]
fn startup_shell_budgets_transcript_height_for_footer_lines() {
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
    for index in 1..=4 {
        shell.add_transcript_item(Box::new(Text::new(format!("line {index}"), 0, 0)));
    }
    shell.set_footer_state(FooterState {
        cwd: "/tmp/project".to_owned(),
        git_branch: Some("main".to_owned()),
        model: Some(model("gpt-5", "openai", true)),
        thinking_level: "high".to_owned(),
        context_window: 200_000,
        context_percent: Some(12.3),
        ..FooterState::default()
    });
    shell.set_viewport_size(40, 5);

    let lines = shell.render(40);

    assert_eq!(lines.len(), 5);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("line 3") || line.contains("line 4"))
    );
    assert!(lines[0].chars().all(|character| character == '─'));
    assert!(lines[2].chars().all(|character| character == '─'));
    assert!(lines[3].contains("/tmp/project (main)"));
    assert!(lines[4].contains("gpt-5 • high"));
}

#[test]
fn startup_shell_can_apply_footer_data_snapshot_without_overwriting_session_footer_fields() {
    let mut extension_statuses = BTreeMap::new();
    extension_statuses.insert("a-first".to_owned(), "status\none".to_owned());

    let snapshot = FooterDataSnapshot {
        cwd: "/tmp/project".to_owned(),
        git_branch: Some("main".to_owned()),
        available_provider_count: 2,
        extension_statuses,
    };

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
    shell.set_footer_state(FooterState {
        model: Some(model("gpt-5", "openai", true)),
        thinking_level: "high".to_owned(),
        context_window: 200_000,
        context_percent: Some(12.3),
        ..FooterState::default()
    });
    shell.apply_footer_data_snapshot(&snapshot);
    shell.set_viewport_size(40, 6);

    let lines = shell.render(40);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("/tmp/project (main)"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("(openai) gpt-5 • high"))
    );
    assert!(lines.iter().any(|line| line.contains("status one")));
}

#[test]
fn startup_shell_can_bind_live_footer_data_provider_without_manual_snapshot_push() {
    let temp_dir = TestDir::new("startup-shell-footer");
    let first_repo = create_plain_repo(temp_dir.path(), "first", "main");
    let second_repo = create_plain_repo(temp_dir.path(), "second", "feature");
    let provider = FooterDataProvider::new(&first_repo);

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
    shell.set_footer_state(FooterState {
        model: Some(model("gpt-5", "openai", true)),
        thinking_level: "high".to_owned(),
        context_window: 200_000,
        context_percent: Some(12.3),
        ..FooterState::default()
    });
    shell.bind_footer_data_provider(&provider);
    shell.set_viewport_size(120, 6);

    let initial_lines = shell.render(120);
    assert!(
        initial_lines
            .iter()
            .any(|line| line.contains("first (main)"))
    );

    provider.set_cwd(&second_repo);
    let updated_lines = shell.render(120);

    assert!(
        updated_lines
            .iter()
            .any(|line| line.contains("second (feature)"))
    );
    assert!(
        updated_lines
            .iter()
            .any(|line| line.contains("gpt-5 • high"))
    );
}

#[test]
fn startup_shell_live_footer_binding_can_queue_tui_rerenders() {
    let temp_dir = TestDir::new("startup-shell-live-footer-rerender");
    let first_repo = create_plain_repo(temp_dir.path(), "first", "main");
    let second_repo = create_plain_repo(temp_dir.path(), "second", "feature");
    let provider = FooterDataProvider::new(&first_repo);

    let terminal = RecordingTerminal::new();
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let render_handle = tui.render_handle();

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
    shell.set_footer_state(FooterState {
        model: Some(model("gpt-5", "openai", true)),
        thinking_level: "high".to_owned(),
        context_window: 200_000,
        context_percent: Some(12.3),
        ..FooterState::default()
    });
    shell.bind_footer_data_provider_with_render_handle(&provider, render_handle);

    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));
    tui.start().expect("start should succeed");
    tui.drain_terminal_events()
        .expect("initial queued footer render should drain successfully");
    let writes_before = inspector.write_count();

    provider.set_cwd(&second_repo);
    tui.drain_terminal_events()
        .expect("queued footer rerender should drain successfully");

    assert!(inspector.write_count() > writes_before);
    assert!(!inspector.writes().is_empty());
    let lines = tui.render_current();
    assert!(lines.iter().any(|line| line.contains("second (feature)")));

    tui.stop().expect("stop should succeed");
}

#[test]
fn startup_shell_preserves_scrolled_transcript_view_when_new_items_arrive() {
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
    for index in 1..=6 {
        shell.add_transcript_item(Box::new(Text::new(format!("line {index}"), 0, 0)));
    }
    shell.set_viewport_size(24, 4);
    let _ = shell.render(24);
    shell.scroll_transcript_up(2);
    let before = shell.render(24);
    assert!(before[0].contains("line 4"));

    shell.add_transcript_item(Box::new(Text::new("line 7", 0, 0)));
    let after = shell.render(24);

    assert_eq!(shell.transcript_scroll_offset(), 3);
    assert!(after[0].contains("line 4"));
    assert!(!after.iter().any(|line| line.contains("line 7")));
    assert!(after[1].chars().all(|character| character == '─'));
    assert!(after[3].chars().all(|character| character == '─'));
}

#[test]
fn startup_shell_truncates_pending_messages_and_can_remove_or_clear_transcript_items() {
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
    let first_id = shell.add_transcript_item(Box::new(Text::new("first item", 0, 0)));
    shell.add_transcript_item(Box::new(Text::new("second item", 0, 0)));
    shell.set_pending_messages(
        &PlainKeyHintStyler,
        ["this is a very long queued steering message that must be truncated"],
        std::iter::empty::<&str>(),
    );
    assert_eq!(shell.transcript_item_count(), 2);
    assert!(shell.has_pending_messages());
    assert!(shell.remove_transcript_item(first_id));
    assert_eq!(shell.transcript_item_count(), 1);

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(24, 10);
    assert!(lines.iter().all(|line| visible_width(line) <= 24));
    assert!(lines.iter().any(|line| line.contains("second item")));
    assert!(!lines.iter().any(|line| line.contains("first item")));
    assert!(lines.iter().any(|line| line.contains("...")));

    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );
    shell.add_transcript_item(Box::new(Text::new("temporary transcript", 0, 0)));
    shell.set_pending_messages(
        &PlainKeyHintStyler,
        ["temporary queued message"],
        std::iter::empty::<&str>(),
    );
    shell.clear_transcript();
    shell.clear_pending_messages();
    assert_eq!(shell.transcript_item_count(), 0);
    assert!(!shell.has_pending_messages());

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(24, 10);
    assert_eq!(lines.len(), 3);
}
