use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::{MemoryAuthStorage, SessionManager};
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

fn unique_name(prefix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{unique}")
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "pi-coding-agent-cli-extension-lifecycle-{prefix}-{}-{}",
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
    let api = unique_name("extension-lifecycle-api");
    let provider = unique_name("extension-lifecycle-provider");
    let model_id = unique_name("extension-lifecycle-model");
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
        api: api.clone(),
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

    (model, api, recorded)
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

fn create_session_file(cwd: &PathBuf, prefix: &str) -> (PathBuf, String) {
    let session_dir = cwd.join(format!("{prefix}-sessions"));
    fs::create_dir_all(&session_dir).unwrap();
    let mut manager = SessionManager::create(
        cwd.to_string_lossy().as_ref(),
        Some(session_dir.to_string_lossy().as_ref()),
    )
    .expect("expected session manager");
    let root_user_id = manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("root message"),
            }],
            timestamp: 1,
        })
        .unwrap();
    manager
        .append_message(Message::Assistant {
            content: vec![AssistantContent::Text {
                text: String::from("assistant reply"),
                text_signature: None,
            }],
            api: String::from("faux:test"),
            provider: String::from("faux"),
            model: String::from("model"),
            response_id: None,
            usage: Usage {
                input: 20_000,
                output: 10,
                cache_read: 0,
                cache_write: 0,
                total_tokens: 20_010,
                cost: Default::default(),
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 2,
        })
        .unwrap();
    manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("primary branch"),
            }],
            timestamp: 3,
        })
        .unwrap();
    manager.branch(&root_user_id).unwrap();
    manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("alternate branch"),
            }],
            timestamp: 4,
        })
        .unwrap();
    manager
        .append_message(Message::Assistant {
            content: vec![AssistantContent::Text {
                text: String::from("alternate reply"),
                text_signature: None,
            }],
            api: String::from("faux:test"),
            provider: String::from("faux"),
            model: String::from("model"),
            response_id: None,
            usage: Usage {
                input: 25_000,
                output: 10,
                cache_read: 0,
                cache_write: 0,
                total_tokens: 25_010,
                cost: Default::default(),
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 5,
        })
        .unwrap();
    manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("final alternate turn"),
            }],
            timestamp: 6,
        })
        .unwrap();
    manager
        .append_message(Message::Assistant {
            content: vec![AssistantContent::Text {
                text: String::from("final alternate reply"),
                text_signature: None,
            }],
            api: String::from("faux:test"),
            provider: String::from("faux"),
            model: String::from("model"),
            response_id: None,
            usage: Usage {
                input: 30_000,
                output: 10,
                cache_read: 0,
                cache_write: 0,
                total_tokens: 30_010,
                cost: Default::default(),
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 7,
        })
        .unwrap();
    let session_file = PathBuf::from(
        manager
            .get_session_file()
            .expect("expected persisted session file"),
    );
    (session_file, root_user_id)
}

#[tokio::test]
async fn before_agent_start_modifies_context_for_rpc_prompts() {
    let (model, api, recorded) = register_recording_provider("done");
    let cwd = unique_temp_dir("before-agent-start");
    let extension_path = cwd.join("before-agent-start.ts");
    fs::write(
        &extension_path,
        r#"export default function (pi) {
	pi.on("before_agent_start", (event) => ({
		message: {
			customType: "hook",
			content: "Injected before agent",
			display: false,
		},
		systemPrompt: `${event.systemPrompt}\n\nHOOK SYSTEM`,
	}));
}
"#,
    )
    .unwrap();

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
            "{\"id\":\"req-1\",\"type\":\"prompt\",\"message\":\"hello\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd,
        default_system_prompt: String::from("base system prompt"),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let request = recorded.lock().unwrap().clone();
    let context = request.context.unwrap_or_else(|| {
        panic!(
            "expected recorded context\nstdout: {}\nstderr: {}",
            result.stdout, result.stderr
        )
    });
    assert!(
        context
            .system_prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("HOOK SYSTEM")),
        "context: {context:?}"
    );
    let user_messages = user_texts(&context);
    assert!(user_messages.iter().any(|text| text == "hello"));
    assert!(
        user_messages
            .iter()
            .any(|text| text == "Injected before agent"),
        "user messages: {user_messages:?}"
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn fork_transition_runs_hooks_and_emits_session_start_for_new_host() {
    let (model, api, _recorded) = register_recording_provider("unused");
    let cwd = unique_temp_dir("fork-transition");
    let (session_file, root_user_id) = create_session_file(&cwd, "fork");
    let extension_path = cwd.join("fork-hooks.ts");
    fs::write(
        &extension_path,
        r#"export default function (pi) {
	pi.on("session_before_fork", () => {
		pi.sendMessage({ customType: "hook", content: "before-fork", display: true });
		return { cancel: false };
	});
	pi.on("session_start", (event, ctx) => {
		if (event.reason === "fork") {
			ctx.ui.setTitle(`start:${event.reason}:${event.previousSessionFile ?? ""}`);
		}
	});
}
"#,
    )
    .unwrap();

    let stdin = format!(
        "{{\"id\":\"fork-1\",\"type\":\"fork\",\"entryId\":\"{}\"}}\n",
        root_user_id
    );
    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--session"),
            session_file.to_string_lossy().into_owned(),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(stdin),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("before-fork"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("\"method\":\"setTitle\"") && result.stdout.contains("start:fork:"),
        "stdout: {}",
        result.stdout
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn compact_command_uses_before_compact_hook_and_emits_session_compact() {
    let (model, api, _recorded) = register_recording_provider("unused");
    let cwd = unique_temp_dir("compact-hook");
    let session_dir = cwd.join("compact-sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let mut manager = SessionManager::create(
        cwd.to_string_lossy().as_ref(),
        Some(session_dir.to_string_lossy().as_ref()),
    )
    .expect("expected session manager");
    for (timestamp, text, reply, input_tokens) in [
        (1, "first turn", "first reply", 18_000_u64),
        (3, "second turn", "second reply", 22_000_u64),
        (5, "third turn", "third reply", 24_000_u64),
    ] {
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from(text),
                }],
                timestamp,
            })
            .unwrap();
        manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from(reply),
                    text_signature: None,
                }],
                api: String::from("faux:test"),
                provider: String::from("faux"),
                model: String::from("model"),
                response_id: None,
                usage: Usage {
                    input: input_tokens,
                    output: 10,
                    cache_read: 0,
                    cache_write: 0,
                    total_tokens: input_tokens + 10,
                    cost: Default::default(),
                },
                stop_reason: StopReason::Stop,
                error_message: None,
                timestamp: timestamp + 1,
            })
            .unwrap();
    }
    let session_file = PathBuf::from(
        manager
            .get_session_file()
            .expect("expected persisted session file"),
    );
    let agent_dir = cwd.join("agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("settings.json"),
        "{\n  \"compaction\": { \"enabled\": true, \"keepRecentTokens\": 1 }\n}\n",
    )
    .unwrap();
    let extension_path = cwd.join("compact-hooks.ts");
    fs::write(
        &extension_path,
        r#"export default function (pi) {
	pi.on("session_before_compact", (event) => ({
		compaction: {
			summary: "Hook compaction summary",
			firstKeptEntryId: event.preparation.firstKeptEntryId,
			tokensBefore: event.preparation.tokensBefore,
			details: { source: "hook" },
		},
	}));
	pi.on("session_compact", (event) => {
		pi.sendMessage({
			customType: "hook",
			content: `compact:${event.fromExtension}:${event.compactionEntry.summary}`,
			display: true,
		});
	});
}
"#,
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--session"),
            session_file.to_string_lossy().into_owned(),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"compact-1\",\"type\":\"compact\"}\n",
        )),
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
    let response = result
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value.get("command").and_then(Value::as_str) == Some("compact"))
        .and_then(|value| value.get("data").cloned())
        .unwrap_or_else(|| {
            panic!(
                "expected compact response data\nstdout: {}\nstderr: {}",
                result.stdout, result.stderr
            )
        });
    assert_eq!(
        response.get("summary").and_then(Value::as_str),
        Some("Hook compaction summary")
    );
    assert!(
        result
            .stdout
            .contains("compact:true:Hook compaction summary"),
        "stdout: {}",
        result.stdout
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn tree_navigation_command_uses_before_tree_hook_and_emits_session_tree() {
    let (model, api, _recorded) = register_recording_provider("unused");
    let cwd = unique_temp_dir("tree-hook");
    let (session_file, root_user_id) = create_session_file(&cwd, "tree");
    let extension_path = cwd.join("tree-hooks.ts");
    fs::write(
        &extension_path,
        r#"export default function (pi) {
	pi.registerCommand("go-root", {
		description: "Navigate to root with summary",
		handler: async (_args, ctx) => {
			await ctx.navigateTree("__ROOT_ID__", { summarize: true });
		},
	});
	pi.on("session_before_tree", () => ({
		summary: { summary: "Hook tree summary", details: { source: "hook" } },
	}));
	pi.on("session_tree", (event) => {
		pi.sendMessage({
			customType: "hook",
			content: event.summaryEntry ? event.summaryEntry.summary : "missing",
			display: true,
		});
	});
}
"#
        .replace("__ROOT_ID__", &root_user_id),
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--session"),
            session_file.to_string_lossy().into_owned(),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"prompt\",\"message\":\"/go-root\"}\n",
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
    assert!(
        result.stdout.contains("Hook tree summary"),
        "stdout: {}",
        result.stdout
    );

    unregister_provider(&api);
}
