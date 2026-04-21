use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Model, StopReason, Usage,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default)]
struct RecordedRequest {
    contexts: Vec<Context>,
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
        self.recorded.lock().unwrap().contexts.push(context);

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
    let path = std::env::temp_dir().join(format!(
        "pi-coding-agent-cli-dynamic-tools-{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn register_recording_provider(
    response_text: &str,
) -> (Model, String, Arc<Mutex<RecordedRequest>>) {
    let api = unique_name("dynamic-tools-api");
    let provider = unique_name("dynamic-tools-provider");
    let model_id = unique_name("dynamic-tools-model");
    let recorded = Arc::new(Mutex::new(RecordedRequest::default()));
    register_provider(
        api.clone(),
        Arc::new(RecordingProvider {
            response_text: response_text.to_owned(),
            recorded: recorded.clone(),
        }),
    );

    let model = Model {
        id: model_id.clone(),
        name: model_id,
        api,
        provider,
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
    };

    let api_name = model.api.clone();
    (model, api_name, recorded)
}

#[tokio::test]
async fn rpc_extension_command_can_register_dynamic_tools_and_activate_them_immediately() {
    let (model, api, recorded) = register_recording_provider("dynamic tools ready");
    let cwd = unique_temp_dir("rpc-refresh-tools");
    let agent_dir = unique_temp_dir("rpc-refresh-tools-agent");
    let session_dir = unique_temp_dir("rpc-refresh-tools-sessions");
    let extension_path = cwd.join("dynamic-tools-extension.ts");
    fs::write(
        &extension_path,
        r#"import { Type } from "@sinclair/typebox";

export default function (pi) {
	pi.registerCommand("add-dynamic-tool", {
		description: "Register a dynamic tool during command execution",
		handler: async () => {
			pi.registerTool({
				name: "echo_dynamic",
				label: "Echo Dynamic",
				description: "Dynamically registered echo tool",
				promptSnippet: "Use echo_dynamic for dynamic echo behavior",
				promptGuidelines: ["Use echo_dynamic when the user asks for dynamic echo behavior."],
				parameters: Type.Object({}),
				execute: async () => ({
					content: [{ type: "text", text: "dynamic tool result" }],
					details: { source: "dynamic" },
				}),
			});

			pi.appendEntry("tool-state", {
				all: pi.getAllTools().map((tool) => tool.name).join(","),
				active: pi.getActiveTools().join(","),
			});
			pi.sendUserMessage("hello after dynamic tool");
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
            String::from("--session-dir"),
            session_dir.to_string_lossy().into_owned(),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"prompt\",\"message\":\"/add-dynamic-tool\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
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
    assert!(
        !result.stdout.contains("\"type\":\"extension_error\""),
        "stdout: {}",
        result.stdout
    );

    let recorded = recorded.lock().unwrap().clone();
    assert_eq!(
        recorded.contexts.len(),
        1,
        "contexts: {:?}",
        recorded.contexts
    );
    let tool_names = recorded.contexts[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(
        tool_names
            .iter()
            .any(|tool_name| *tool_name == "echo_dynamic"),
        "tool names: {tool_names:?}"
    );

    let session_files = fs::read_dir(&session_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    assert_eq!(session_files.len(), 1, "files: {session_files:?}");

    let session_contents = fs::read_to_string(&session_files[0]).unwrap();
    assert!(
        session_contents.contains("\"customType\":\"tool-state\""),
        "session: {session_contents}"
    );
    assert!(
        session_contents.contains("\"all\":\"read,bash,edit,write,grep,find,ls,echo_dynamic\""),
        "session: {session_contents}"
    );
    assert!(
        session_contents.contains("\"active\":\"read,bash,edit,write,echo_dynamic\""),
        "session: {session_contents}"
    );

    unregister_provider(&api);
}
