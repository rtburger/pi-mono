use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, FauxModelDefinition, FauxResponse,
    RegisterFauxProviderOptions, StreamOptions, register_faux_provider, register_provider,
    unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

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
        *self.recorded.lock().unwrap() = RecordedRequest {
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

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "pi-coding-agent-cli-rpc-{prefix}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[tokio::test]
async fn run_command_rpc_mode_emits_response_before_agent_events() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "rpc-mode-events-faux".into(),
        models: vec![FauxModelDefinition {
            id: "rpc-mode-events-faux-1".into(),
            name: Some("RPC Mode Events Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text("rpc done")]);
    let model = faux
        .get_model(Some("rpc-mode-events-faux-1"))
        .expect("expected faux model");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"req-1\",\"type\":\"prompt\",\"message\":\"hello\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("events"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected valid json line"))
        .collect::<Vec<_>>();

    assert!(lines.len() >= 3, "stdout: {}", result.stdout);
    assert_eq!(
        lines[0].get("type").and_then(Value::as_str),
        Some("response"),
        "stdout: {}",
        result.stdout
    );
    assert_eq!(
        lines[0].get("command").and_then(Value::as_str),
        Some("prompt")
    );
    assert_eq!(
        lines[1].get("type").and_then(Value::as_str),
        Some("agent_start"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        lines
            .iter()
            .any(|line| line.get("type").and_then(Value::as_str) == Some("agent_end")),
        "stdout: {}",
        result.stdout
    );

    faux.unregister();
}

#[tokio::test]
async fn run_command_rpc_mode_updates_and_reports_session_state() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "rpc-mode-state-faux".into(),
        models: vec![
            FauxModelDefinition {
                id: "rpc-mode-state-faux-1".into(),
                name: Some("RPC Mode State Faux 1".into()),
                reasoning: false,
            },
            FauxModelDefinition {
                id: "rpc-mode-state-faux-2".into(),
                name: Some("RPC Mode State Faux 2".into()),
                reasoning: true,
            },
        ],
        ..RegisterFauxProviderOptions::default()
    });
    let initial_model = faux
        .get_model(Some("rpc-mode-state-faux-1"))
        .expect("expected initial faux model");
    let second_model = faux
        .get_model(Some("rpc-mode-state-faux-2"))
        .expect("expected second faux model");

    let stdin_content = [
        "{\"id\":\"req-1\",\"type\":\"get_state\"}",
        "{\"id\":\"req-2\",\"type\":\"set_model\",\"provider\":\"rpc-mode-state-faux\",\"modelId\":\"rpc-mode-state-faux-2\"}",
        "{\"id\":\"req-3\",\"type\":\"set_thinking_level\",\"level\":\"high\"}",
        "{\"id\":\"req-4\",\"type\":\"get_state\"}",
    ]
    .join("\n")
        + "\n";

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            initial_model.provider.clone(),
            String::from("--model"),
            initial_model.id.clone(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(stdin_content),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            initial_model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![initial_model.clone(), second_model.clone()],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("state"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected valid json line"))
        .collect::<Vec<_>>();

    assert_eq!(lines.len(), 4, "stdout: {}", result.stdout);

    let initial_state = lines[0]
        .get("data")
        .and_then(|data| data.get("model"))
        .and_then(|model| model.get("id"))
        .and_then(Value::as_str);
    assert_eq!(initial_state, Some(initial_model.id.as_str()));

    let set_model_id = lines[1]
        .get("data")
        .and_then(|data| data.get("id"))
        .and_then(Value::as_str);
    assert_eq!(set_model_id, Some(second_model.id.as_str()));

    let final_state = &lines[3]["data"];
    assert_eq!(
        final_state
            .get("model")
            .and_then(|model| model.get("id"))
            .and_then(Value::as_str),
        Some(second_model.id.as_str())
    );
    assert_eq!(
        final_state.get("thinkingLevel").and_then(Value::as_str),
        Some("high")
    );

    faux.unregister();
}

fn unique_name(prefix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{unique}")
}

fn register_recording_provider(response_text: &str) -> (String, Arc<Mutex<RecordedRequest>>) {
    let api = unique_name("rpc-extension-api");
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

fn model(api: &str, provider: &str, id: &str) -> Model {
    Model {
        id: id.to_owned(),
        name: id.to_owned(),
        api: api.to_owned(),
        provider: provider.to_owned(),
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

fn last_user_text(context: &Context) -> String {
    context
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::User { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|content| match content {
                        pi_events::UserContent::Text { text } => Some(text.as_str()),
                        pi_events::UserContent::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn extension_sidecar_uses_rust_local_extension_runtime() {
    let sidecar_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../support/extension-sidecar.mjs");
    let source = fs::read_to_string(&sidecar_path).expect("expected extension sidecar source");

    assert!(
        source.contains("./extension-runtime/index.mjs"),
        "sidecar: {source}"
    );
    assert!(
        !source.contains("packages/coding-agent/src/core/extensions/index.ts"),
        "sidecar: {source}"
    );
}

#[tokio::test]
async fn run_command_rpc_mode_loads_extension_commands_and_resources() {
    let provider = unique_name("rpc-extension-provider");
    let model_id = unique_name("rpc-extension-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("rpc-extension-resources");
    let extension_dir = cwd.join("demo-extension");
    fs::create_dir_all(&extension_dir).unwrap();
    fs::create_dir_all(extension_dir.join("skills").join("review-code")).unwrap();
    fs::write(
        extension_dir.join("package.json"),
        r#"{
  "name": "demo-extension",
  "private": true,
  "type": "module",
  "pi": { "extensions": ["./index.ts"] }
}
"#,
    )
    .unwrap();
    fs::write(
        extension_dir.join("index.ts"),
        r#"import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const baseDir = dirname(fileURLToPath(import.meta.url));

export default function (pi) {
	pi.registerCommand("rpc-demo", {
		description: "Demo extension command",
		handler: async () => {},
	});
	pi.on("resources_discover", () => ({
		promptPaths: [join(baseDir, "review.md")],
		skillPaths: [join(baseDir, "skills")],
	}));
}
"#,
    )
    .unwrap();
    fs::write(
        extension_dir.join("review.md"),
        "---\ndescription: Extension review prompt\n---\nReview $1 from extension\n",
    )
    .unwrap();
    fs::write(
        extension_dir.join("skills").join("review-code").join("SKILL.md"),
        "---\ndescription: Review code from extension\n---\n# Review\nRead the target file first.\n",
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--extension"),
            extension_dir.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"get_commands\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected json line"))
        .collect::<Vec<_>>();
    let response = lines
        .iter()
        .find(|line| line.get("command").and_then(Value::as_str) == Some("get_commands"))
        .expect("expected get_commands response");
    let commands = response["data"]["commands"]
        .as_array()
        .expect("expected command array");
    let names = commands
        .iter()
        .filter_map(|command| command.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(names.contains(&"rpc-demo"), "names: {names:?}");
    assert!(names.contains(&"review"), "names: {names:?}");
    assert!(names.contains(&"skill:review-code"), "names: {names:?}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_rpc_mode_expands_prompt_templates_from_extension_resources() {
    let provider = unique_name("rpc-extension-prompt-provider");
    let model_id = unique_name("rpc-extension-prompt-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("rpc-extension-prompt");
    let extension_dir = cwd.join("prompt-extension");
    fs::create_dir_all(&extension_dir).unwrap();
    fs::write(
        extension_dir.join("index.ts"),
        r#"import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const baseDir = dirname(fileURLToPath(import.meta.url));

export default function (pi) {
	pi.on("resources_discover", () => ({
		promptPaths: [join(baseDir, "review.md")],
	}));
}
"#,
    )
    .unwrap();
    fs::write(
        extension_dir.join("review.md"),
        "---\ndescription: Review via extension\n---\nReview $1 carefully\n",
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--extension"),
            extension_dir.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"prompt\",\"message\":\"/review src/lib.rs\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    assert_eq!(last_user_text(&context), "Review src/lib.rs carefully\n");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_rpc_mode_emits_extension_ui_requests() {
    let provider = unique_name("rpc-extension-ui-provider");
    let model_id = unique_name("rpc-extension-ui-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("rpc-extension-ui");
    let extension_path = cwd.join("ui-extension.ts");
    fs::write(
        &extension_path,
        r#"export default function (pi) {
	pi.on("session_start", async (_event, ctx) => {
		ctx.ui.setStatus("demo", "ready");
		ctx.ui.setWidget("demo", ["loaded"]);
		ctx.ui.setTitle("Demo Title");
	});
	pi.registerCommand("rpc-prefill", {
		description: "Prefill editor",
		handler: async (_args, ctx) => {
			ctx.ui.setEditorText("prefilled from extension");
			ctx.ui.notify("Editor prefilled", "info");
		},
	});
}
"#,
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"prompt\",\"message\":\"/rpc-prefill\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected json line"))
        .collect::<Vec<_>>();
    let methods = lines
        .iter()
        .filter_map(|line| {
            (line.get("type").and_then(Value::as_str) == Some("extension_ui_request"))
                .then(|| line.get("method").and_then(Value::as_str))
                .flatten()
        })
        .collect::<Vec<_>>();
    assert!(methods.contains(&"setStatus"), "stdout: {}", result.stdout);
    assert!(methods.contains(&"setWidget"), "stdout: {}", result.stdout);
    assert!(methods.contains(&"setTitle"), "stdout: {}", result.stdout);
    assert!(
        methods.contains(&"set_editor_text"),
        "stdout: {}",
        result.stdout
    );
    assert!(methods.contains(&"notify"), "stdout: {}", result.stdout);

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_rpc_mode_applies_extension_flags() {
    let provider = unique_name("rpc-extension-flag-provider");
    let model_id = unique_name("rpc-extension-flag-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("rpc-extension-flags");
    let extension_path = cwd.join("flag-extension.ts");
    fs::write(
        &extension_path,
        r#"import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const baseDir = dirname(fileURLToPath(import.meta.url));

export default function (pi) {
	pi.registerFlag("demo-flag", {
		type: "boolean",
		description: "Enable demo prompt",
	});
	pi.on("resources_discover", () => {
		if (pi.getFlag("demo-flag") !== true) {
			return {};
		}
		return { promptPaths: [join(baseDir, "flagged.md")] };
	});
}
"#,
    )
    .unwrap();
    fs::write(
        cwd.join("flagged.md"),
        "---\ndescription: Flagged prompt\n---\nFlagged prompt\n",
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
            String::from("--demo-flag"),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"get_commands\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected json line"))
        .collect::<Vec<_>>();
    let response = lines
        .iter()
        .find(|line| line.get("command").and_then(Value::as_str) == Some("get_commands"))
        .expect("expected get_commands response");
    let names = response["data"]["commands"]
        .as_array()
        .expect("expected command array")
        .iter()
        .filter_map(|command| command.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(names.contains(&"flagged"), "names: {names:?}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_rpc_mode_rejects_file_arguments() {
    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("@README.md"),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::new()),
        auth_source: Arc::new(MemoryAuthStorage::default()),
        built_in_models: Vec::new(),
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("file-args"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty(), "stdout: {}", result.stdout);
    assert!(
        result
            .stderr
            .contains("@file arguments are not supported in RPC mode"),
        "stderr: {}",
        result.stderr
    );
}
