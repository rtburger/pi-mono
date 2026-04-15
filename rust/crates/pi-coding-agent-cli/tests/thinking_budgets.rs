use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, FauxModelDefinition, RegisterFauxProviderOptions,
    StreamOptions, register_faux_provider, register_provider, stream_response, unregister_provider,
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
struct RecordingOptionsProvider {
    seen_options: Arc<Mutex<Vec<StreamOptions>>>,
}

impl AiProvider for RecordingOptionsProvider {
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
            text: "recorded options".into(),
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

#[tokio::test]
async fn runner_loads_thinking_budgets_from_settings_for_runtime_requests() {
    let _guard = provider_registry_guard();
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        models: vec![FauxModelDefinition {
            id: "thinking-budgets-faux".into(),
            name: Some("Thinking Budgets Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    let faux_model = faux
        .get_model(Some("thinking-budgets-faux"))
        .expect("faux model");
    let _ = stream_response(faux_model, Context::default(), StreamOptions::default())
        .expect("faux provider should prime builtin registration");
    faux.unregister();

    let cwd = unique_temp_dir("thinking-budgets-cwd");
    let agent_dir = unique_temp_dir("thinking-budgets-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"thinkingBudgets":{"high":2048}}"#,
    )
    .unwrap();

    let seen_options = Arc::new(Mutex::new(Vec::new()));
    register_provider(
        "anthropic-messages",
        Arc::new(RecordingOptionsProvider {
            seen_options: seen_options.clone(),
        }),
    );

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--thinking"),
            String::from("high"),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            String::from("anthropic"),
            "test-token",
        )])),
        built_in_models: vec![Model {
            id: "claude-sonnet-4-20250514".into(),
            name: "Claude Sonnet 4".into(),
            api: "anthropic-messages".into(),
            provider: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            reasoning: true,
            input: vec!["text".into()],
            cost: pi_events::ModelCost {
                input: 1.0,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.1,
            },
            context_window: 200_000,
            max_tokens: 60_000,
            compat: None,
        }],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd,
        default_system_prompt: String::new(),
        version: String::from("0.0.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(
        result.exit_code, 0,
        "stdout: {} stderr: {}",
        result.stdout, result.stderr
    );
    assert_eq!(result.stdout, "recorded options\n");
    assert!(result.stderr.is_empty());

    let seen_options = seen_options.lock().unwrap().clone();
    assert_eq!(seen_options.len(), 1);
    assert_eq!(seen_options[0].reasoning_effort.as_deref(), Some("high"));
    assert_eq!(seen_options[0].max_tokens, Some(34_048));

    unregister_provider("anthropic-messages");
    pi_ai::register_builtin_providers();
}
