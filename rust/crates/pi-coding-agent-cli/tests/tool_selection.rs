use futures::stream;
use parking_lot::Mutex;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_interactive_command_with_terminal};
use pi_coding_agent_core::MemoryAuthStorage;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Model, StopReason, Usage,
};
use pi_tui::{Terminal, TuiError};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default)]
struct RecordedRequest {
    context: Option<Context>,
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
        _options: StreamOptions,
    ) -> AssistantEventStream {
        *self.recorded.lock() = RecordedRequest {
            context: Some(context),
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

#[derive(Clone)]
struct ScriptedTerminal {
    script: Arc<Vec<(Duration, String)>>,
}

impl ScriptedTerminal {
    fn new(script: Vec<(Duration, String)>) -> Self {
        Self {
            script: Arc::new(script),
        }
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
            for (delay, input) in script.iter() {
                thread::sleep(*delay);
                on_input(input.clone());
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

    fn write(&mut self, _data: &str) -> Result<(), TuiError> {
        Ok(())
    }

    fn columns(&self) -> u16 {
        100
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

fn register_recording_provider(response_text: &str) -> (String, Arc<Mutex<RecordedRequest>>) {
    let api = unique_name("tool-selection-api");
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
        input: vec![String::from("text")],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    }
}

#[tokio::test]
async fn interactive_runner_accepts_read_only_tool_selection_flags() {
    let provider = unique_name("tool-selection-provider");
    let model_id = unique_name("tool-selection-model");
    let (api, recorded) = register_recording_provider("done");
    let terminal = ScriptedTerminal::new(vec![
        (Duration::from_millis(5), String::from("h")),
        (Duration::from_millis(5), String::from("i")),
        (Duration::from_millis(5), String::from("\r")),
        (Duration::from_millis(80), String::from("\x04")),
    ]);

    let exit_code = run_interactive_command_with_terminal(
        RunCommandOptions {
            args: vec![
                String::from("--provider"),
                provider.clone(),
                String::from("--model"),
                model_id.clone(),
                String::from("--no-tools"),
                String::from("--tools"),
                String::from("read,grep,find,ls"),
            ],
            stdin_is_tty: true,
            stdin_content: None,
            auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
                provider.as_str(),
                "token",
            )])),
            built_in_models: vec![model(&api, &provider, &model_id)],
            models_json_path: None,
            agent_dir: None,
            cwd: unique_temp_dir("interactive-tool-selection"),
            default_system_prompt: String::new(),
            version: String::from("0.1.0"),
            stream_options: StreamOptions::default(),
        },
        Arc::new(move || Box::new(terminal.clone())),
    )
    .await;

    assert_eq!(exit_code, 0);
    let request = recorded.lock().clone();
    let context = request.context.expect("expected recorded context");
    let tool_names = context
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tool_names, vec!["read", "grep", "find", "ls"]);

    unregister_provider(&api);
}
