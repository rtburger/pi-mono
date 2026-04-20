use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::{MemoryAuthStorage, SessionManager, get_default_session_dir};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
    UserContent,
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
        "pi-coding-agent-cli-extension-commands-{prefix}-{}-{}",
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
    let api = unique_name("extension-command-api");
    let provider = unique_name("extension-command-provider");
    let model_id = unique_name("extension-command-model");
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

fn last_user_text(context: &Context) -> String {
    context
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::User { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|item| match item {
                        UserContent::Text { text } => Some(text.as_str()),
                        UserContent::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

fn user_texts(context: &Context) -> Vec<String> {
    context
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::User { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|item| match item {
                        UserContent::Text { text } => Some(text.as_str()),
                        UserContent::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .collect()
}

fn write_extension(path: &PathBuf, source: &str) {
    fs::write(path, source).unwrap();
}

#[tokio::test]
async fn rpc_extension_command_can_send_user_messages_through_host_control_plane() {
    let (model, api, recorded) = register_recording_provider("handled by extension command");
    let cwd = unique_temp_dir("rpc-send-user-message");
    let extension_path = cwd.join("send-user-message-extension.ts");
    write_extension(
        &extension_path,
        r#"export default function (pi) {
	pi.registerCommand("ask-ext", {
		description: "Send a user message from the extension",
		handler: async () => {
			pi.setSessionName("rpc extension session");
			pi.sendUserMessage("hello from extension host bridge");
		},
	});
}
"#,
    );

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"prompt\",\"message\":\"/ask-ext\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd,
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected valid json line"))
        .collect::<Vec<_>>();
    assert!(
        lines
            .iter()
            .any(|line| line.get("type").and_then(Value::as_str) == Some("agent_start")),
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
    assert!(
        !lines
            .iter()
            .any(|line| line.get("type").and_then(Value::as_str) == Some("extension_error")),
        "stdout: {}",
        result.stdout
    );

    let recorded = recorded.lock().unwrap().clone();
    assert_eq!(recorded.contexts.len(), 1);
    assert_eq!(recorded.contexts[0].messages.len(), 1);
    assert_eq!(
        last_user_text(&recorded.contexts[0]),
        "hello from extension host bridge"
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn rpc_extension_command_can_manage_custom_messages_entries_and_active_tools() {
    let (model, api, recorded) = register_recording_provider("bridge complete");
    let cwd = unique_temp_dir("rpc-bridge-actions");
    let agent_dir = unique_temp_dir("rpc-bridge-actions-agent");
    let session_dir = unique_temp_dir("rpc-bridge-actions-sessions");
    let extension_path = cwd.join("bridge-actions-extension.ts");
    write_extension(
        &extension_path,
        r#"export default function (pi) {
	pi.registerCommand("bridge-actions", {
		description: "Exercise the RPC extension bridge",
		handler: async () => {
			const toolNames = pi.getAllTools().map((tool) => tool.name).join(",");
			pi.appendEntry("tool-catalog", { tools: toolNames });
			pi.setSessionName("configured bridge session");
			pi.sendMessage({
				customType: "note",
				content: "custom context from extension",
				display: true,
				details: { source: "bridge" },
			});
			pi.setActiveTools(["read", "grep", "missing-tool"]);
			pi.sendUserMessage("hello from extension after setup");
		},
	});
}
"#,
    );

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
            "{\"id\":\"cmd-1\",\"type\":\"prompt\",\"message\":\"/bridge-actions\"}\n",
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
    let context = &recorded.contexts[0];
    let tool_names = context
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tool_names, vec!["read", "grep"]);
    assert_eq!(
        user_texts(context),
        vec![
            String::from("custom context from extension"),
            String::from("hello from extension after setup"),
        ]
    );

    let session_files = fs::read_dir(&session_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    assert_eq!(session_files.len(), 1, "files: {session_files:?}");
    let session_contents = fs::read_to_string(&session_files[0]).unwrap();
    assert!(
        session_contents.contains("configured bridge session"),
        "session: {session_contents}"
    );
    assert!(
        session_contents.contains("\"customType\":\"note\""),
        "session: {session_contents}"
    );
    assert!(
        session_contents.contains("\"customType\":\"tool-catalog\""),
        "session: {session_contents}"
    );
    assert!(session_contents.contains("read,bash,edit,write,grep,find,ls"));

    unregister_provider(&api);
}

#[tokio::test]
async fn json_mode_extension_command_can_switch_to_fresh_session_before_prompting() {
    let (model, api, recorded) = register_recording_provider("fresh session reply");
    let cwd = unique_temp_dir("json-session-aware");
    let agent_dir = unique_temp_dir("json-session-aware-agent");
    let extension_path = cwd.join("new-session-extension.ts");
    write_extension(
        &extension_path,
        r#"export default function (pi) {
	pi.registerCommand("restart-ext", {
		description: "Start a fresh session before sending a prompt",
		handler: async (_args, ctx) => {
			await ctx.newSession();
			pi.sendUserMessage("fresh session question");
		},
	});
}
"#,
    );

    let agent_dir_string = agent_dir.to_string_lossy().into_owned();
    let cwd_string = cwd.to_string_lossy().into_owned();
    let session_dir = get_default_session_dir(&cwd_string, Some(&agent_dir_string));
    let mut session_manager = SessionManager::create(&cwd_string, Some(&session_dir)).unwrap();
    session_manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("previous session context"),
            }],
            timestamp: 1,
        })
        .unwrap();
    session_manager
        .append_message(Message::Assistant {
            content: vec![AssistantContent::Text {
                text: String::from("previous reply"),
                text_signature: None,
            }],
            api: model.api.clone(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            response_id: None,
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 2,
        })
        .unwrap();
    let session_file = session_manager
        .get_session_file()
        .map(str::to_owned)
        .expect("expected persisted session file");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("json"),
            String::from("--session"),
            session_file,
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
            String::from("/restart-ext"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd,
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let events = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected valid json line"))
        .collect::<Vec<_>>();
    assert_eq!(
        events[0].get("type").and_then(Value::as_str),
        Some("session")
    );
    assert!(
        events
            .iter()
            .any(|event| event.get("type").and_then(Value::as_str) == Some("agent_start")),
        "stdout: {}",
        result.stdout
    );
    assert!(
        events
            .iter()
            .any(|event| event.get("type").and_then(Value::as_str) == Some("agent_end")),
        "stdout: {}",
        result.stdout
    );
    assert!(
        !events
            .iter()
            .any(|event| event.get("type").and_then(Value::as_str) == Some("extension_error")),
        "stdout: {}",
        result.stdout
    );

    let recorded = recorded.lock().unwrap().clone();
    assert_eq!(recorded.contexts.len(), 1);
    assert_eq!(
        recorded.contexts[0].messages.len(),
        1,
        "context: {:?}",
        recorded.contexts[0]
    );
    assert_eq!(
        last_user_text(&recorded.contexts[0]),
        "fresh session question"
    );

    unregister_provider(&api);
}
