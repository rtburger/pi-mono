use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::stream;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, built_in_models, get_model,
    register_builtin_providers, register_provider, stream_response, unregister_provider,
};
use pi_coding_agent_cli::{
    EnvAuthSource, RunCommandOptions, run_command, run_interactive_command_with_terminal,
};
use pi_coding_agent_core::{AuthFileSource, ChainedAuthSource, MemoryAuthStorage};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Model, StopReason, Usage,
    UserContent,
};
use pi_tui::{Terminal, TuiError};
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const TINY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACAQMAAABIeJ9nAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGUExURf8AAP///0EdNBEAAAABYktHRAH/Ai3eAAAAB3RJTUUH6gEOADM5Ddoh/wAAAAxJREFUCNdjYGBgAAAABAABJzQnCgAAACV0RVh0ZGF0ZTpjcmVhdGUAMjAyNi0wMS0xNFQwMDo1MTo1NyswMDowMOnKzHgAAAAldEVYdGRhdGU6bW9kaWZ5ADIwMjYtMDEtMTRUMDA6NTE6NTcrMDA6MDCYl3TEAAAAKHRFWHRkYXRlOnRpbWVzdGFtcAAyMDI2LTAxLTE0VDAwOjUxOjU3KzAwOjAwz4JVGwAAAABJRU5ErkJggg==";
static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default)]
struct RecordedRequest {
    context: Option<Context>,
    model: Option<Model>,
    api_key: Option<String>,
}

#[derive(Clone)]
struct RecordingProvider {
    response_text: String,
    recorded: Arc<Mutex<RecordedRequest>>,
}

impl AiProvider for RecordingProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        *self.recorded.lock().unwrap() = RecordedRequest {
            context: Some(context),
            model: Some(model.clone()),
            api_key: options.api_key,
        };

        let message = AssistantMessage {
            role: String::from("assistant"),
            content: vec![AssistantContent::Text {
                text: self.response_text.clone(),
                text_signature: None,
            }],
            api: model.api.clone(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            response_id: None,
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };

        Box::pin(stream::iter(vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message,
        })]))
    }
}

fn unique_name(prefix: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{timestamp}-{counter}")
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(unique_name(prefix));
    fs::create_dir_all(&path).unwrap();
    path
}

fn solid_png(width: u32, height: u32) -> Vec<u8> {
    let image = ImageBuffer::from_pixel(width, height, Rgba([255, 0, 0, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn register_recording_provider(response_text: &str) -> (String, Arc<Mutex<RecordedRequest>>) {
    let api = unique_name("recording-api");
    let recorded = Arc::new(Mutex::new(RecordedRequest::default()));
    register_provider(
        api.clone(),
        Arc::new(RecordingProvider {
            response_text: response_text.to_string(),
            recorded: recorded.clone(),
        }),
    );
    (api, recorded)
}

fn model(api: &str, provider: &str, id: &str) -> Model {
    Model {
        id: id.to_string(),
        name: id.to_string(),
        api: api.to_string(),
        provider: provider.to_string(),
        base_url: String::from("https://example.invalid/v1"),
        reasoning: false,
        input: vec![String::from("text"), String::from("image")],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

#[derive(Clone)]
struct ScriptedTerminal {
    writes: Arc<Mutex<Vec<String>>>,
    script: Arc<Vec<(Duration, TerminalAction)>>,
}

#[derive(Clone)]
enum TerminalAction {
    Input(String),
}

impl ScriptedTerminal {
    fn new(script: Vec<(Duration, TerminalAction)>) -> Self {
        Self {
            writes: Arc::new(Mutex::new(Vec::new())),
            script: Arc::new(script),
        }
    }

    fn output(&self) -> String {
        self.writes.lock().unwrap().join("")
    }
}

impl Terminal for ScriptedTerminal {
    fn start(
        &mut self,
        mut on_input: Box<dyn FnMut(String) + Send>,
        _on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        let script = Arc::clone(&self.script);
        thread::spawn(move || {
            for (delay, action) in script.iter() {
                thread::sleep(*delay);
                match action {
                    TerminalAction::Input(data) => on_input(data.clone()),
                }
            }
        });
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn drain_input(&mut self, _max: Duration, _idle: Duration) -> Result<(), TuiError> {
        Ok(())
    }

    fn write(&mut self, data: &str) -> Result<(), TuiError> {
        self.writes.lock().unwrap().push(data.to_owned());
        Ok(())
    }

    fn columns(&self) -> u16 {
        100
    }

    fn rows(&self) -> u16 {
        12
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

fn strip_terminal_control_sequences(output: &str) -> String {
    let mut result = String::new();
    let bytes = output.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            match bytes.get(index + 1).copied() {
                Some(b'[') => {
                    index += 2;
                    while index < bytes.len() {
                        let byte = bytes[index];
                        index += 1;
                        if (0x40..=0x7e).contains(&byte) {
                            break;
                        }
                    }
                    continue;
                }
                Some(b']') | Some(b'_') => {
                    index += 2;
                    while index < bytes.len() {
                        if bytes[index] == 0x07 {
                            index += 1;
                            break;
                        }
                        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                    continue;
                }
                _ => {
                    index += 1;
                    continue;
                }
            }
        }

        let character = output[index..]
            .chars()
            .next()
            .expect("terminal output should contain a character");
        index += character.len_utf8();

        if character == '\r' || (character.is_control() && character != '\n' && character != '\t') {
            continue;
        }

        result.push(character);
    }

    result
}

#[tokio::test]
async fn run_command_applies_cli_api_key_override_to_stream_options() {
    let provider = unique_name("provider");
    let model_id = unique_name("model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--api-key"),
            String::from("cli-token"),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(EnvAuthSource::new()),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("runner-api-key"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "done\n");
    assert!(result.stderr.is_empty());
    assert_eq!(
        recorded.lock().unwrap().api_key.as_deref(),
        Some("cli-token")
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_uses_pi_ai_built_in_model_catalog() {
    register_builtin_providers();
    let _ = stream_response(
        get_model("openai", "gpt-5.4").expect("expected built-in openai model"),
        Context::default(),
        StreamOptions::default(),
    );

    let recorded = Arc::new(Mutex::new(RecordedRequest::default()));
    register_provider(
        "openai-responses",
        Arc::new(RecordingProvider {
            response_text: String::from("catalog"),
            recorded: recorded.clone(),
        }),
    );

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            String::from("openai"),
            String::from("--model"),
            String::from("gpt-5.4"),
            String::from("--api-key"),
            String::from("cli-token"),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(EnvAuthSource::new()),
        built_in_models: built_in_models().to_vec(),
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("runner-built-in-catalog"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(
        result.exit_code, 0,
        "stderr: {} stdout: {}",
        result.stderr, result.stdout
    );
    assert_eq!(result.stdout, "catalog\n");
    assert!(result.stderr.is_empty());

    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    assert_eq!(request.api_key.as_deref(), Some("cli-token"));
    assert_eq!(
        get_model("openai", "gpt-5.4")
            .as_ref()
            .map(|model| model.api.as_str()),
        Some("openai-responses")
    );
    assert_eq!(context.messages.len(), 1);

    unregister_provider("openai-responses");
    register_builtin_providers();
}

#[tokio::test]
async fn run_command_merges_stdin_text_file_args_and_image_attachments() {
    let provider = unique_name("provider");
    let model_id = unique_name("model");
    let (api, recorded) = register_recording_provider("ok");
    let built_in_model = model(&api, &provider, &model_id);
    let temp_dir = unique_temp_dir("runner-files");
    let text_path = temp_dir.join("notes.txt");
    let image_path = temp_dir.join("screenshot.png");
    fs::write(&text_path, "hello from file\n").unwrap();
    fs::write(&image_path, STANDARD.decode(TINY_PNG).unwrap()).unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            format!("@{}", text_path.display()),
            format!("@{}", image_path.display()),
            String::from("Explain"),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from("stdin text\n")),
        auth_source: Arc::new(EnvAuthSource::new()),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: None,
        cwd: temp_dir.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "ok\n");

    let recorded = recorded.lock().unwrap().clone();
    let context = recorded.context.expect("expected recorded context");
    assert_eq!(context.messages.len(), 1);
    match &context.messages[0] {
        pi_events::Message::User { content, .. } => {
            assert_eq!(content.len(), 2);
            match &content[0] {
                UserContent::Text { text } => {
                    assert!(text.starts_with("stdin text"));
                    assert!(text.contains(&format!(
                        "<file name=\"{}\">\nhello from file\n\n</file>\n",
                        text_path.display()
                    )));
                    assert!(text.contains(&format!(
                        "<file name=\"{}\"></file>\nExplain",
                        image_path.display()
                    )));
                }
                other => panic!("expected text block, got {other:?}"),
            }
            match &content[1] {
                UserContent::Image { mime_type, data } => {
                    assert_eq!(mime_type, "image/png");
                    assert!(!data.is_empty());
                }
                other => panic!("expected image block, got {other:?}"),
            }
        }
        other => panic!("expected user message, got {other:?}"),
    }

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_loads_block_images_setting_from_global_settings() {
    let provider = unique_name("provider");
    let model_id = unique_name("model");
    let (api, recorded) = register_recording_provider("blocked");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runner-block-images-cwd");
    let agent_dir = unique_temp_dir("runner-block-images-agent");
    let image_path = cwd.join("screenshot.png");
    fs::write(&image_path, STANDARD.decode(TINY_PNG).unwrap()).unwrap();
    fs::write(
        agent_dir.join("settings.json"),
        serde_json::json!({
            "images": {
                "blockImages": true
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            format!("@{}", image_path.display()),
            String::from("Explain"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd,
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "blocked\n");
    assert!(result.stderr.is_empty());

    let recorded = recorded.lock().unwrap().clone();
    let context = recorded.context.expect("expected recorded context");
    match &context.messages[0] {
        pi_events::Message::User { content, .. } => {
            assert_eq!(content.len(), 2);
            match &content[0] {
                UserContent::Text { text } => {
                    assert!(text.contains("<file name="));
                    assert!(text.contains("Explain"));
                }
                other => panic!("expected text block, got {other:?}"),
            }
            assert_eq!(
                content[1],
                UserContent::Text {
                    text: String::from("Image reading is disabled."),
                }
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_loads_auto_resize_setting_from_global_settings() {
    let provider = unique_name("provider");
    let model_id = unique_name("model");
    let (api, recorded) = register_recording_provider("resized");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runner-auto-resize-cwd");
    let agent_dir = unique_temp_dir("runner-auto-resize-agent");
    let image_path = cwd.join("large.png");
    let original_bytes = solid_png(2_100, 2_100);
    let original_base64 = STANDARD.encode(&original_bytes);
    fs::write(&image_path, &original_bytes).unwrap();
    fs::write(
        agent_dir.join("settings.json"),
        serde_json::json!({
            "images": {
                "autoResize": false
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            format!("@{}", image_path.display()),
            String::from("Explain"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd,
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "resized\n");
    assert!(result.stderr.is_empty());

    let recorded = recorded.lock().unwrap().clone();
    let context = recorded.context.expect("expected recorded context");
    match &context.messages[0] {
        pi_events::Message::User { content, .. } => {
            assert_eq!(content.len(), 2);
            match &content[0] {
                UserContent::Text { text } => {
                    assert!(!text.contains("[Image:"));
                    assert!(
                        text.contains(&format!("<file name=\"{}\"></file>", image_path.display()))
                    );
                }
                other => panic!("expected text block, got {other:?}"),
            }
            assert_eq!(
                content[1],
                UserContent::Image {
                    data: original_base64,
                    mime_type: String::from("image/png"),
                }
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_rejects_api_key_without_model() {
    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--api-key"),
            String::from("cli-token"),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(EnvAuthSource::new()),
        built_in_models: Vec::new(),
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("runner-api-key-error"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty());
    assert!(result.stderr.contains(
        "--api-key requires a model to be specified via --model, --provider/--model, or --models"
    ));
}

#[tokio::test]
async fn run_command_rejects_interactive_mode_for_now() {
    let result = run_command(RunCommandOptions {
        args: vec![String::from("hello")],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(EnvAuthSource::new()),
        built_in_models: Vec::new(),
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("runner-interactive"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty());
    assert!(
        result
            .stderr
            .contains("Interactive mode is not supported in the Rust CLI yet")
    );
}

#[tokio::test]
async fn run_interactive_command_renders_live_transcript_and_exits() {
    let provider = unique_name("interactive-provider");
    let model_id = unique_name("interactive-model");
    let (api, _recorded) = register_recording_provider("interactive-done");
    let built_in_model = model(&api, &provider, &model_id);
    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("h")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("i")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(80),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![built_in_model],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-live"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = inspector.output();
    assert!(output.contains("interactive-done"), "output: {output}");
    assert!(output.contains("hi"), "output: {output}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_loads_editor_padding_setting_for_prompt() {
    let provider = unique_name("interactive-provider");
    let model_id = unique_name("interactive-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runner-interactive-editor-padding-cwd");
    let agent_dir = unique_temp_dir("runner-interactive-editor-padding-agent");

    fs::write(
        agent_dir.join("settings.json"),
        serde_json::json!({
            "editorPaddingX": 3
        })
        .to_string(),
    )
    .unwrap();

    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("h")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("i")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("\x7f")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\x7f")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![built_in_model],
            models_json_path: None,
            agent_dir: Some(agent_dir),
            cwd,
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("   hi"), "output: {output}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_loads_autocomplete_max_visible_setting_for_prompt() {
    let provider = unique_name("interactive-provider");
    let model_id = unique_name("interactive-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runner-interactive-autocomplete-cwd");
    let agent_dir = unique_temp_dir("runner-interactive-autocomplete-agent");
    for index in 1..=4 {
        fs::write(cwd.join(format!("readme-{index}.md")), String::new()).unwrap();
    }
    fs::write(
        agent_dir.join("settings.json"),
        serde_json::json!({
            "autocompleteMaxVisible": 3
        })
        .to_string(),
    )
    .unwrap();

    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("r")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("e")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\t")),
        ),
        (
            Duration::from_millis(80),
            TerminalAction::Input(String::from("\x7f")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\x7f")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![built_in_model],
            models_json_path: None,
            agent_dir: Some(agent_dir),
            cwd,
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = inspector.output();
    assert!(output.contains("readme-1.md"), "output: {output}");
    assert!(output.contains("readme-2.md"), "output: {output}");
    assert!(output.contains("readme-3.md"), "output: {output}");
    assert!(!output.contains("readme-4.md"), "output: {output}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_auto_triggers_attachment_autocomplete_in_prompt() {
    let provider = unique_name("interactive-provider");
    let model_id = unique_name("interactive-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runner-interactive-attachment-autocomplete");
    fs::write(cwd.join("attachment-target.txt"), String::new()).unwrap();
    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("@")),
        ),
        (
            Duration::from_millis(30),
            TerminalAction::Input(String::from("\x7f")),
        ),
        (
            Duration::from_millis(30),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![built_in_model],
            models_json_path: None,
            agent_dir: None,
            cwd,
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("attachment-target.txt"), "output: {output}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_updates_quoted_attachment_autocomplete_in_prompt() {
    let provider = unique_name("interactive-provider");
    let model_id = unique_name("interactive-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runner-interactive-quoted-attachment-autocomplete");
    fs::create_dir_all(cwd.join("my folder")).unwrap();
    fs::write(cwd.join("my folder/target.txt"), String::new()).unwrap();
    fs::write(cwd.join("my folder/other.txt"), String::new()).unwrap();
    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(1),
            TerminalAction::Input(String::from("@")),
        ),
        (
            Duration::from_millis(1),
            TerminalAction::Input(String::from("m")),
        ),
        (
            Duration::from_millis(1),
            TerminalAction::Input(String::from("y")),
        ),
        (
            Duration::from_millis(1),
            TerminalAction::Input(String::from("\t")),
        ),
        (
            Duration::from_millis(1),
            TerminalAction::Input(String::from("t")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(10),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(80),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![built_in_model],
            models_json_path: None,
            agent_dir: None,
            cwd,
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("target.txt"), "output: {output}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_renders_slash_command_autocomplete_for_supported_commands() {
    let provider = unique_name("interactive-provider");
    let model_id = unique_name("interactive-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("/")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("m")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\t")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("\x03")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![built_in_model],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-slash-autocomplete"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("model — Select model"), "output: {output}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_renders_model_argument_autocomplete_in_prompt() {
    let initial_provider = unique_name("p1");
    let suggestion_provider = unique_name("p2");
    let initial_model_id = unique_name("a");
    let suggestion_model_id = unique_name("b");
    let (api, _recorded) = register_recording_provider("unused");
    let partial_model_id = &suggestion_model_id[..suggestion_model_id.len().min(1)];
    let mut script = format!("/model {partial_model_id}")
        .chars()
        .map(|character| {
            (
                Duration::from_millis(5),
                TerminalAction::Input(character.to_string()),
            )
        })
        .collect::<Vec<_>>();
    script.extend([
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\x03")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let terminal = ScriptedTerminal::new(script);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                initial_provider.clone(),
                String::from("--model"),
                initial_model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([
                (initial_provider.as_str(), "token-a"),
                (suggestion_provider.as_str(), "token-b"),
            ])),
            built_in_models: vec![
                model(&api, &initial_provider, &initial_model_id),
                model(&api, &suggestion_provider, &suggestion_model_id),
            ],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-model-autocomplete"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(
        output.contains(&format!("{suggestion_model_id} — {suggestion_provider}")),
        "output: {output}"
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_scopes_model_autocomplete_and_switching_to_scoped_models() {
    let provider = unique_name("interactive-provider");
    let initial_model_id = unique_name("alpha-scoped-model");
    let scoped_target_model_id = unique_name("beta-scoped-model");
    let unscoped_model_id = unique_name("beta-global-model");
    let (api, recorded) = register_recording_provider("interactive-scoped");
    let mut script = String::from("/model beta")
        .chars()
        .map(|character| {
            (
                Duration::from_millis(5),
                TerminalAction::Input(character.to_string()),
            )
        })
        .collect::<Vec<_>>();
    script.extend([
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("h")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("i")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(80),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let terminal = ScriptedTerminal::new(script);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--models"),
                format!("{initial_model_id},{scoped_target_model_id}"),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![
                model(&api, &provider, &initial_model_id),
                model(&api, &provider, &scoped_target_model_id),
                model(&api, &provider, &unscoped_model_id),
            ],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-scoped-model-autocomplete"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(
        output.contains(&format!("{scoped_target_model_id} — {provider}")),
        "output: {output}"
    );
    assert!(!output.contains(&unscoped_model_id), "output: {output}");
    assert!(output.contains("Model:"), "output: {output}");
    assert!(output.contains(&scoped_target_model_id), "output: {output}");
    assert!(output.contains("interactive-scoped"), "output: {output}");

    let request = recorded.lock().unwrap().clone();
    assert_eq!(
        request.model.as_ref().map(|model| model.id.as_str()),
        Some(scoped_target_model_id.as_str())
    );
    let context = request.context.expect("expected recorded context");
    match context.messages.first() {
        Some(pi_events::Message::User { content, .. }) => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("hi"),
                }]
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_executes_quit_slash_command_without_prompting_model() {
    let provider = unique_name("interactive-provider");
    let model_id = unique_name("interactive-model");
    let (api, recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("/")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("q")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("u")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("i")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("t")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![built_in_model],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-quit-command"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    assert!(recorded.lock().unwrap().context.is_none());
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("/quit"), "output: {output}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_uses_model_selector_fallback_for_non_exact_model_command() {
    let provider = unique_name("interactive-provider");
    let initial_model_id = unique_name("alpha-model");
    let selected_model_id = unique_name("beta-selector-model");
    let (api, recorded) = register_recording_provider("interactive-selector");
    let mut script = String::from("/model beta")
        .chars()
        .map(|character| {
            (
                Duration::from_millis(5),
                TerminalAction::Input(character.to_string()),
            )
        })
        .collect::<Vec<_>>();
    script.extend([
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\x1b")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("h")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("i")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(80),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let terminal = ScriptedTerminal::new(script);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                initial_model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![
                model(&api, &provider, &initial_model_id),
                model(&api, &provider, &selected_model_id),
            ],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-model-selector-command"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("Select model"), "output: {output}");
    assert!(
        output.contains(&format!("{selected_model_id} [{provider}]")),
        "output: {output}"
    );
    assert!(output.contains("Model:"), "output: {output}");
    assert!(output.contains(&selected_model_id), "output: {output}");
    assert!(output.contains("interactive-selector"), "output: {output}");

    let request = recorded.lock().unwrap().clone();
    assert_eq!(
        request.model.as_ref().map(|model| model.id.as_str()),
        Some(selected_model_id.as_str())
    );
    let context = request.context.expect("expected recorded context");
    match context.messages.first() {
        Some(pi_events::Message::User { content, .. }) => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("hi"),
                }]
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_opens_model_selector_from_app_model_select_keybinding() {
    let provider = unique_name("interactive-provider");
    let initial_model_id = unique_name("alpha-model");
    let selected_model_id = unique_name("beta-keybinding-model");
    let (api, recorded) = register_recording_provider("interactive-keybinding-selector");
    let terminal = ScriptedTerminal::new(vec![
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\x0c")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\x1b[B")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("h")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("i")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(80),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                initial_model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![
                model(&api, &provider, &initial_model_id),
                model(&api, &provider, &selected_model_id),
            ],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-model-selector-keybinding"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("Select model"), "output: {output}");
    assert!(
        output.contains(&format!("{selected_model_id} [{provider}]")),
        "output: {output}"
    );
    assert!(output.contains("Model:"), "output: {output}");
    assert!(output.contains(&selected_model_id), "output: {output}");
    assert!(
        output.contains("interactive-keybinding-selector"),
        "output: {output}"
    );

    let request = recorded.lock().unwrap().clone();
    assert_eq!(
        request.model.as_ref().map(|model| model.id.as_str()),
        Some(selected_model_id.as_str())
    );
    let context = request.context.expect("expected recorded context");
    match context.messages.first() {
        Some(pi_events::Message::User { content, .. }) => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("hi"),
                }]
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }

    unregister_provider(&api);
}

#[tokio::test]
async fn run_interactive_command_switches_models_via_model_slash_command() {
    let provider = unique_name("interactive-provider");
    let initial_model_id = unique_name("alpha-model");
    let switched_model_id = unique_name("beta-model");
    let (api, recorded) = register_recording_provider("interactive-switched");
    let partial_model_id = &switched_model_id[..switched_model_id.len().min(4)];
    let mut script = format!("/model {partial_model_id}")
        .chars()
        .map(|character| {
            (
                Duration::from_millis(5),
                TerminalAction::Input(character.to_string()),
            )
        })
        .collect::<Vec<_>>();
    script.extend([
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(25),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(40),
            TerminalAction::Input(String::from("h")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("i")),
        ),
        (
            Duration::from_millis(5),
            TerminalAction::Input(String::from("\r")),
        ),
        (
            Duration::from_millis(80),
            TerminalAction::Input(String::from("\x04")),
        ),
    ]);
    let terminal = ScriptedTerminal::new(script);
    let inspector = terminal.clone();

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                initial_model_id.clone(),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![
                model(&api, &provider, &initial_model_id),
                model(&api, &provider, &switched_model_id),
            ],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("runner-interactive-model-command"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let output = strip_terminal_control_sequences(&inspector.output());
    assert!(output.contains("Model:"), "output: {output}");
    assert!(output.contains(&switched_model_id), "output: {output}");
    assert!(output.contains("interactive-switched"), "output: {output}");

    let request = recorded.lock().unwrap().clone();
    assert_eq!(
        request.model.as_ref().map(|model| model.id.as_str()),
        Some(switched_model_id.as_str())
    );
    let context = request.context.expect("expected recorded context");
    match context.messages.first() {
        Some(pi_events::Message::User { content, .. }) => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("hi"),
                }]
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_lists_models_without_entering_print_or_interactive_mode() {
    let result = run_command(RunCommandOptions {
        args: vec![String::from("--list-models")],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([("openai", "token")])),
        built_in_models: vec![model("openai-responses", "openai", "gpt-5.2-codex")],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("runner-list-models"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert!(result.stderr.is_empty());
    assert!(result.stdout.contains("provider"));
    assert!(result.stdout.contains("gpt-5.2-codex"));
    assert!(!result.stdout.contains("Interactive mode is not supported"));
}

#[tokio::test]
async fn run_command_uses_first_scoped_model_when_models_flag_is_provided() {
    let provider = unique_name("provider");
    let second_provider = unique_name("provider");
    let (api, recorded) = register_recording_provider("scoped");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--models"),
            String::from("beta-model,alpha-model:high"),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([
            (provider.as_str(), "token-a"),
            (second_provider.as_str(), "token-b"),
        ])),
        built_in_models: vec![
            model(&api, &provider, "alpha-model"),
            model(&api, &second_provider, "beta-model"),
        ],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("runner-model-scope"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "scoped\n");
    assert!(result.stderr.is_empty());
    assert_eq!(
        recorded
            .lock()
            .unwrap()
            .model
            .as_ref()
            .map(|model| (model.provider.as_str(), model.id.as_str())),
        Some((second_provider.as_str(), "beta-model"))
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_applies_cli_api_key_override_when_models_flag_selects_initial_model() {
    let provider = unique_name("provider");
    let (api, recorded) = register_recording_provider("scoped");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--models"),
            String::from("alpha-model"),
            String::from("--api-key"),
            String::from("cli-token"),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token-a",
        )])),
        built_in_models: vec![model(&api, &provider, "alpha-model")],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("runner-model-scope-api-key"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "scoped\n");
    assert!(result.stderr.is_empty());
    assert_eq!(
        recorded.lock().unwrap().api_key.as_deref(),
        Some("cli-token")
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_uses_auth_json_api_keys_for_initial_model_selection() {
    let provider = unique_name("provider");
    let model_id = unique_name("model");
    let (api, recorded) = register_recording_provider("auth-file");
    let temp_dir = unique_temp_dir("runner-auth-file");
    let auth_path = temp_dir.join("auth.json");
    fs::write(
        &auth_path,
        serde_json::json!({
            provider.clone(): {
                "type": "api_key",
                "key": "stored-token"
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![String::from("-p"), String::from("hello")],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(ChainedAuthSource::new(vec![Arc::new(AuthFileSource::new(
            auth_path,
        ))])),
        built_in_models: vec![model(&api, &provider, &model_id)],
        models_json_path: None,
        agent_dir: None,
        cwd: temp_dir,
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "auth-file\n");
    assert!(result.stderr.is_empty());
    assert_eq!(
        recorded.lock().unwrap().api_key.as_deref(),
        Some("stored-token")
    );

    unregister_provider(&api);
}
