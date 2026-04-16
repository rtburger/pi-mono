use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, Transport, register_provider,
    unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use pi_events::{AssistantContent, AssistantEvent, AssistantMessage, Context, Model, StopReason};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-coding-agent-cli-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn provider_registry_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[derive(Clone)]
struct RecordingTransportProvider {
    seen_options: Arc<Mutex<Vec<StreamOptions>>>,
}

impl AiProvider for RecordingTransportProvider {
    fn stream(
        &self,
        model: Model,
        _context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        self.seen_options.lock().unwrap().push(options);

        let partial =
            AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
        let mut message = partial.clone();
        message.content.push(AssistantContent::Text {
            text: "recorded transport".into(),
            text_signature: None,
        });
        message.stop_reason = StopReason::Stop;
        message.timestamp = 1;

        Box::pin(stream::iter(vec![
            Ok(AssistantEvent::Start { partial }),
            Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message,
            }),
        ]))
    }
}

fn codex_model(api: &str) -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "GPT-5.2 Codex".into(),
        api: api.into(),
        provider: "openai-codex".into(),
        base_url: "https://example.invalid/codex".into(),
        reasoning: true,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 200_000,
        max_tokens: 64_000,
        compat: None,
    }
}

#[tokio::test]
async fn run_command_loads_transport_from_settings() {
    let _guard = provider_registry_guard();
    let api = "test-transport-settings-api";
    let seen_options = Arc::new(Mutex::new(Vec::new()));
    register_provider(
        api,
        Arc::new(RecordingTransportProvider {
            seen_options: seen_options.clone(),
        }),
    );

    let cwd = unique_temp_dir("transport-settings-cwd");
    let agent_dir = unique_temp_dir("transport-settings-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"transport":"websocket"}"#,
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            "-p".into(),
            "--provider".into(),
            "openai-codex".into(),
            "--model".into(),
            "gpt-5.2-codex".into(),
            "hello".into(),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            "openai-codex",
            "test-token",
        )])),
        built_in_models: vec![codex_model(api)],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd,
        default_system_prompt: String::new(),
        version: String::from("0.0.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "recorded transport\n");

    let seen_options = seen_options.lock().unwrap().clone();
    assert_eq!(seen_options.len(), 1);
    assert_eq!(seen_options[0].transport, Some(Transport::WebSocket));

    unregister_provider(api);
}

#[tokio::test]
async fn cli_transport_flag_overrides_settings_transport() {
    let _guard = provider_registry_guard();
    let api = "test-transport-cli-api";
    let seen_options = Arc::new(Mutex::new(Vec::new()));
    register_provider(
        api,
        Arc::new(RecordingTransportProvider {
            seen_options: seen_options.clone(),
        }),
    );

    let cwd = unique_temp_dir("transport-cli-cwd");
    let agent_dir = unique_temp_dir("transport-cli-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"transport":"websocket"}"#,
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            "-p".into(),
            "--provider".into(),
            "openai-codex".into(),
            "--model".into(),
            "gpt-5.2-codex".into(),
            "--transport".into(),
            "auto".into(),
            "hello".into(),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            "openai-codex",
            "test-token",
        )])),
        built_in_models: vec![codex_model(api)],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd,
        default_system_prompt: String::new(),
        version: String::from("0.0.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "recorded transport\n");

    let seen_options = seen_options.lock().unwrap().clone();
    assert_eq!(seen_options.len(), 1);
    assert_eq!(seen_options[0].transport, Some(Transport::Auto));

    unregister_provider(api);
}
