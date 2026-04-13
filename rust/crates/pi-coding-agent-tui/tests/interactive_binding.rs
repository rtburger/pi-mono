use pi_ai::{
    FauxContentBlock, FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions,
    StreamOptions, register_faux_provider,
};
use pi_coding_agent_core::{
    CodingAgentCoreOptions, MemoryAuthStorage, SessionBootstrapOptions, create_coding_agent_core,
};
use pi_coding_agent_tui::{
    InteractiveCoreBinding, KeybindingsManager, PlainKeyHintStyler, StartupShellComponent,
};
use pi_events::StopReason;
use pi_tui::{Terminal, Tui, TuiError};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-coding-agent-tui-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

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
        20
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

#[tokio::test]
async fn interactive_binding_submits_prompt_and_renders_user_and_assistant_messages() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "interactive-faux".into(),
        models: vec![FauxModelDefinition {
            id: "interactive-faux-1".into(),
            name: Some("Interactive Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text("Hello from faux")]);
    let model = faux.get_model(Some("interactive-faux-1")).unwrap();

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.clone(),
            "test-token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        cwd: Some(unique_temp_dir("interactive-binding-cwd")),
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

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

    let terminal = RecordingTerminal::new();
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, tui.render_handle());
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));
    tui.start().expect("start should succeed");
    let writes_before = inspector.write_count();

    tui.handle_input("h").expect("input should be handled");
    tui.handle_input("i").expect("input should be handled");
    tui.handle_input("\r").expect("submit should be handled");

    tokio::task::yield_now().await;
    created.core.wait_for_idle().await;
    tui.drain_terminal_events()
        .expect("queued interactive updates should drain successfully");

    let lines = tui.render_current();
    assert!(lines.iter().any(|line| line.contains("hi")));
    assert!(lines.iter().any(|line| line.contains("Hello from faux")));
    assert!(lines.len() >= 3);
    assert!(inspector.write_count() > writes_before);

    drop(binding);
    tui.stop().expect("stop should succeed");
    faux.unregister();
}

#[tokio::test]
async fn interactive_shell_external_editor_action_mounts_extension_editor_and_restores_prompt() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "interactive-shell-editor-faux".into(),
        models: vec![FauxModelDefinition {
            id: "interactive-shell-editor-faux-1".into(),
            name: Some("Interactive Shell Editor Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text("Edited prompt received")]);
    let model = faux
        .get_model(Some("interactive-shell-editor-faux-1"))
        .unwrap();

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.clone(),
            "test-token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        cwd: Some(unique_temp_dir("interactive-shell-editor-cwd")),
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

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

    let terminal = RecordingTerminal::new();
    let mut tui = Tui::new(terminal);
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, tui.render_handle());
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));
    tui.start().expect("start should succeed");

    tui.handle_input("h").expect("input should be handled");
    tui.handle_input("i").expect("input should be handled");
    tui.handle_input("\x07")
        .expect("external-editor action should be handled");

    let extension_editor_lines = tui.render_current();
    assert!(
        extension_editor_lines
            .iter()
            .any(|line| line.contains("Edit message"))
    );
    assert!(
        !extension_editor_lines
            .iter()
            .any(|line| line.contains("> hi"))
    );

    tui.handle_input("!")
        .expect("extension editor input should be handled");
    tui.handle_input("\r")
        .expect("extension editor submit should be handled");

    let restored_prompt_lines = tui.render_current();
    assert!(
        restored_prompt_lines
            .iter()
            .any(|line| line.contains("hi!"))
    );

    tui.handle_input("\r")
        .expect("prompt submit should be handled");

    tokio::task::yield_now().await;
    created.core.wait_for_idle().await;
    tui.drain_terminal_events()
        .expect("queued interactive updates should drain successfully");

    let lines = tui.render_current();
    assert!(lines.iter().any(|line| line.contains("hi!")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Edited prompt received"))
    );

    drop(binding);
    tui.stop().expect("stop should succeed");
    faux.unregister();
}

#[tokio::test]
async fn interactive_shell_external_editor_action_respects_registered_override() {
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
    shell.set_input_cursor("draft prompt".len());

    let action_calls = Arc::new(Mutex::new(0usize));
    let action_calls_for_handler = Arc::clone(&action_calls);
    shell.on_action("app.editor.external", move || {
        *action_calls_for_handler
            .lock()
            .expect("action mutex poisoned") += 1;
    });

    let terminal = RecordingTerminal::new();
    let mut tui = Tui::new(terminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));
    tui.start().expect("start should succeed");

    tui.handle_input("\x07")
        .expect("external-editor action should be handled");

    let lines = tui.render_current();
    assert_eq!(*action_calls.lock().expect("action mutex poisoned"), 1);
    assert!(lines.iter().any(|line| line.contains("draft prompt")));
    assert!(!lines.iter().any(|line| line.contains("Edit message")));

    tui.stop().expect("stop should succeed");
}

#[tokio::test]
async fn interactive_binding_renders_tool_execution_updates() {
    let cwd = unique_temp_dir("interactive-tool-binding-cwd");
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "interactive-tool-faux".into(),
        models: vec![FauxModelDefinition {
            id: "interactive-tool-faux-1".into(),
            name: Some("Interactive Tool Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![
        FauxResponse {
            content: vec![FauxContentBlock::ToolCall {
                id: "tool-1".into(),
                name: "write".into(),
                arguments: BTreeMap::from([
                    ("path".into(), json!("notes.txt")),
                    ("content".into(), json!("hello")),
                ]),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
        },
        FauxResponse::text("done"),
    ]);
    let model = faux.get_model(Some("interactive-tool-faux-1")).unwrap();

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.clone(),
            "test-token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        cwd: Some(cwd.clone()),
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

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

    let terminal = RecordingTerminal::new();
    let mut tui = Tui::new(terminal);
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, tui.render_handle());
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));
    tui.start().expect("start should succeed");

    tui.handle_input("b").expect("input should be handled");
    tui.handle_input("u").expect("input should be handled");
    tui.handle_input("i").expect("input should be handled");
    tui.handle_input("l").expect("input should be handled");
    tui.handle_input("d").expect("input should be handled");
    tui.handle_input("\r").expect("submit should be handled");

    tokio::task::yield_now().await;
    created.core.wait_for_idle().await;
    tui.drain_terminal_events()
        .expect("queued interactive updates should drain successfully");

    let lines = tui.render_current();
    assert!(lines.iter().any(|line| line.contains("write notes.txt")));
    assert!(lines.iter().any(|line| line.contains("hello")));
    assert!(lines.iter().any(|line| line.contains("done")));
    assert_eq!(fs::read_to_string(cwd.join("notes.txt")).unwrap(), "hello");

    drop(binding);
    tui.stop().expect("stop should succeed");
    faux.unregister();
}

#[tokio::test]
async fn interactive_binding_replays_existing_state_when_bound_late() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "interactive-replay-faux".into(),
        models: vec![FauxModelDefinition {
            id: "interactive-replay-faux-1".into(),
            name: Some("Interactive Replay Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text("Earlier answer")]);
    let model = faux.get_model(Some("interactive-replay-faux-1")).unwrap();

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.clone(),
            "test-token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        cwd: Some(unique_temp_dir("interactive-replay-cwd")),
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    created.core.prompt_text("earlier prompt").await.unwrap();

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

    let terminal = RecordingTerminal::new();
    let mut tui = Tui::new(terminal);
    let binding =
        InteractiveCoreBinding::bind(created.core.clone(), &mut shell, tui.render_handle());
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));
    tui.start().expect("start should succeed");
    tui.drain_terminal_events()
        .expect("queued replay updates should drain successfully");

    let lines = tui.render_current();
    assert!(lines.iter().any(|line| line.contains("earlier prompt")));
    assert!(lines.iter().any(|line| line.contains("Earlier answer")));

    drop(binding);
    tui.stop().expect("stop should succeed");
    faux.unregister();
}
