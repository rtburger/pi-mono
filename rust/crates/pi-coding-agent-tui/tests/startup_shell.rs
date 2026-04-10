use pi_coding_agent_core::{FooterDataProvider, FooterDataSnapshot};
use pi_coding_agent_tui::{
    FooterState, KeyId, KeybindingsManager, PlainKeyHintStyler, StartupShellComponent,
};
use pi_events::Model;
use pi_tui::{Component, Terminal, Text, Tui, TuiError, visible_width};
use std::{
    collections::BTreeMap,
    fs,
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
        context_window: 200_000,
        max_tokens: 8_192,
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
    let prompt = lines
        .iter()
        .position(|line| line.starts_with("> "))
        .expect("prompt should render");

    assert!(first_transcript < second_transcript);
    assert!(second_transcript < steering);
    assert!(steering < follow_up);
    assert!(follow_up < prompt);
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
    assert!(lines[0].contains("line 4"));
    assert!(lines[1].contains("line 5"));
    assert!(lines[2].contains("line 6"));
    assert!(lines[3].starts_with("> "));
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
    assert!(scrolled_up[0].contains("line 2"));
    assert!(scrolled_up[1].contains("line 3"));
    assert!(scrolled_up[2].contains("line 4"));
    assert!(scrolled_up[3].starts_with("> "));

    shell.scroll_transcript_down(1);
    let scrolled_down = shell.render(24);
    assert_eq!(shell.transcript_scroll_offset(), 1);
    assert!(scrolled_down[0].contains("line 3"));
    assert!(scrolled_down[1].contains("line 4"));
    assert!(scrolled_down[2].contains("line 5"));
    assert!(scrolled_down[3].starts_with("> "));

    shell.scroll_transcript_to_bottom();
    let bottom = shell.render(24);
    assert_eq!(shell.transcript_scroll_offset(), 0);
    assert!(bottom[0].contains("line 4"));
    assert!(bottom[1].contains("line 5"));
    assert!(bottom[2].contains("line 6"));
    assert!(bottom[3].starts_with("> "));
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
    assert!(lines[0].contains("line 3"));
    assert!(lines[1].contains("line 4"));
    assert!(lines[2].starts_with("> "));
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
    assert!(before[0].contains("line 2"));
    assert!(before[1].contains("line 3"));
    assert!(before[2].contains("line 4"));

    shell.add_transcript_item(Box::new(Text::new("line 7", 0, 0)));
    let after = shell.render(24);

    assert_eq!(shell.transcript_scroll_offset(), 3);
    assert!(after[0].contains("line 2"));
    assert!(after[1].contains("line 3"));
    assert!(after[2].contains("line 4"));
    assert!(!after.iter().any(|line| line.contains("line 7")));
    assert!(after[3].starts_with("> "));
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
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("> "));
}
