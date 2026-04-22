use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Model, ModelCost, StopReason,
    Usage,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default)]
struct RecordedRequest {
    model: Option<Model>,
    options: Option<StreamOptions>,
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
        options: StreamOptions,
    ) -> AssistantEventStream {
        *self.recorded.lock().unwrap() = RecordedRequest {
            model: Some(model.clone()),
            options: Some(options),
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
    let api = unique_name("extension-provider-api");
    let recorded = Arc::new(Mutex::new(RecordedRequest::default()));
    register_provider(
        api.clone(),
        Arc::new(RecordingProvider {
            response_text: response_text.to_owned(),
            recorded: recorded.clone(),
        }),
    );
    (api, recorded)
}

fn model(api: &str, provider: &str, id: &str, base_url: &str) -> Model {
    Model {
        id: id.to_owned(),
        name: id.to_owned(),
        api: api.to_owned(),
        provider: provider.to_owned(),
        base_url: base_url.to_owned(),
        reasoning: false,
        input: vec![String::from("text")],
        cost: ModelCost {
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

fn write_extension(path: &Path, source: &str) {
    fs::write(path, source).unwrap();
}

#[tokio::test]
async fn print_mode_extension_provider_override_refreshes_current_model() {
    let provider = unique_name("extension-override-provider");
    let model_id = unique_name("extension-override-model");
    let (api, recorded) = register_recording_provider("override ok");
    let built_in_model = model(
        &api,
        &provider,
        &model_id,
        "https://original.example.com/v1",
    );
    let cwd = unique_temp_dir("extension-provider-override");
    let agent_dir = cwd.join("agent");
    fs::create_dir_all(&agent_dir).unwrap();
    let extension_path = cwd.join("override-provider-extension.ts");
    write_extension(
        &extension_path,
        &format!(
            r#"export default function (pi) {{
	pi.registerProvider({provider:?}, {{
		baseUrl: "https://override.example.com/v1",
	}});
}}
"#,
        ),
    );

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
            String::from("hello"),
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
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let recorded = recorded.lock().unwrap().clone();
    let model = recorded.model.expect("expected recorded model");
    let options = recorded.options.expect("expected recorded options");
    assert_eq!(model.provider, provider);
    assert_eq!(model.id, model_id);
    assert_eq!(model.base_url, "https://override.example.com/v1");
    assert_eq!(options.api_key.as_deref(), Some("token"));

    unregister_provider(&api);
}

#[tokio::test]
async fn print_mode_extension_command_can_register_runtime_provider_and_switch_to_it() {
    let bootstrap_provider = unique_name("extension-bootstrap-provider");
    let bootstrap_model_id = unique_name("extension-bootstrap-model");
    let runtime_provider = unique_name("extension-runtime-provider");
    let runtime_model_id = unique_name("extension-runtime-model");
    let (api, recorded) = register_recording_provider("runtime provider ok");
    let built_in_model = model(
        &api,
        &bootstrap_provider,
        &bootstrap_model_id,
        "https://bootstrap.example.com/v1",
    );
    let cwd = unique_temp_dir("extension-runtime-provider");
    let agent_dir = cwd.join("agent");
    fs::create_dir_all(&agent_dir).unwrap();
    let extension_path = cwd.join("runtime-provider-extension.ts");
    write_extension(
        &extension_path,
        &format!(
            r#"const runtimeModel = {{
	id: {runtime_model_id:?},
	name: "Runtime Provider Model",
	api: {api:?},
	provider: {runtime_provider:?},
	baseUrl: "https://runtime.example.com/v1",
	reasoning: false,
	input: ["text"],
	cost: {{ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }},
	contextWindow: 128000,
	maxTokens: 16384,
}};

export default function (pi) {{
	pi.registerCommand("register-runtime-provider", {{
		description: "Register a runtime provider",
		handler: async () => {{
			pi.registerProvider({runtime_provider:?}, {{
				baseUrl: runtimeModel.baseUrl,
				apiKey: "runtime-token",
				api: runtimeModel.api,
				headers: {{ "X-Provider-Header": "runtime" }},
				authHeader: true,
				models: [{{
					id: runtimeModel.id,
					name: runtimeModel.name,
					reasoning: false,
					input: ["text"],
					cost: {{ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }},
					contextWindow: 128000,
					maxTokens: 16384,
				}}],
			}});
		}},
	}});

	pi.registerCommand("use-runtime-provider", {{
		description: "Switch to the runtime provider",
		handler: async () => {{
			const ok = await pi.setModel(runtimeModel);
			if (!ok) {{
				throw new Error("failed to switch to runtime provider");
			}}
		}},
	}});
}}
"#,
        ),
    );

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            bootstrap_provider.clone(),
            String::from("--model"),
            bootstrap_model_id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
            String::from("/register-runtime-provider"),
            String::from("/use-runtime-provider"),
            String::from("hello from runtime provider"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            bootstrap_provider.as_str(),
            "bootstrap-token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let recorded = recorded.lock().unwrap().clone();
    let model = recorded.model.expect("expected recorded model");
    let options = recorded.options.expect("expected recorded options");
    let context = recorded.context.expect("expected recorded context");

    assert_eq!(model.provider, runtime_provider);
    assert_eq!(model.id, runtime_model_id);
    assert_eq!(model.base_url, "https://runtime.example.com/v1");
    assert_eq!(options.api_key.as_deref(), Some("runtime-token"));
    assert_eq!(
        options.headers.get("Authorization").map(String::as_str),
        Some("Bearer runtime-token")
    );
    assert_eq!(
        options.headers.get("X-Provider-Header").map(String::as_str),
        Some("runtime")
    );
    assert!(
        context.messages.iter().any(|message| matches!(
            message,
            pi_events::Message::User { content, .. }
                if content.iter().any(|item| matches!(item, pi_events::UserContent::Text { text } if text == "hello from runtime provider"))
        )),
        "context: {:?}",
        context.messages
    );

    unregister_provider(&api);
}
