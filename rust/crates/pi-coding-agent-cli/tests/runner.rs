use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, built_in_models, get_model,
    register_builtin_providers, register_provider, stream_response, unregister_provider,
};
use pi_coding_agent_cli::{EnvAuthSource, RunCommandOptions, run_command};
use pi_coding_agent_core::{AuthFileSource, ChainedAuthSource, MemoryAuthStorage};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Model, StopReason, Usage,
    UserContent,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

const TINY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACAQMAAABIeJ9nAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGUExURf8AAP///0EdNBEAAAABYktHRAH/Ai3eAAAAB3RJTUUH6gEOADM5Ddoh/wAAAAxJREFUCNdjYGBgAAAABAABJzQnCgAAACV0RVh0ZGF0ZTpjcmVhdGUAMjAyNi0wMS0xNFQwMDo1MTo1NyswMDowMOnKzHgAAAAldEVYdGRhdGU6bW9kaWZ5ADIwMjYtMDEtMTRUMDA6NTE6NTcrMDA6MDCYl3TEAAAAKHRFWHRkYXRlOnRpbWVzdGFtcAAyMDI2LTAxLTE0VDAwOjUxOjU3KzAwOjAwz4JVGwAAAABJRU5ErkJggg==";

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
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{unique}")
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(unique_name(prefix));
    fs::create_dir_all(&path).unwrap();
    path
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
async fn run_command_lists_models_without_entering_print_or_interactive_mode() {
    let result = run_command(RunCommandOptions {
        args: vec![String::from("--list-models")],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([("openai", "token")])),
        built_in_models: vec![model("openai-responses", "openai", "gpt-5.2-codex")],
        models_json_path: None,
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
