use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
    UserContent,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default)]
struct RecordedExtensionState {
    call_count: usize,
    contexts: Vec<Context>,
    payloads: Vec<Value>,
}

#[derive(Clone)]
struct RecordingExtensionProvider {
    state: Arc<Mutex<RecordedExtensionState>>,
}

impl AiProvider for RecordingExtensionProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        let state = self.state.clone();
        Box::pin(stream::once(async move {
            let call_index = {
                let state = state.lock().unwrap();
                state.call_count
            };
            let payload = if let Some(on_payload) = options.on_payload.as_ref() {
                on_payload
                    .call(
                        json!({ "provider": "recording-extension", "call": call_index }),
                        model.clone(),
                    )
                    .await
                    .expect("payload hook should succeed")
                    .unwrap_or_else(
                        || json!({ "provider": "recording-extension", "call": call_index }),
                    )
            } else {
                json!({ "provider": "recording-extension", "call": call_index })
            };
            {
                let mut state = state.lock().unwrap();
                state.call_count += 1;
                state.contexts.push(context.clone());
                state.payloads.push(payload.clone());
            }

            let message = if call_index == 0 {
                AssistantMessage {
                    role: String::from("assistant"),
                    content: vec![AssistantContent::ToolCall {
                        id: String::from("tool-1"),
                        name: String::from("dynamic_tool"),
                        arguments: BTreeMap::new(),
                        thought_signature: None,
                    }],
                    api: model.api.clone(),
                    provider: model.provider.clone(),
                    model: model.id.clone(),
                    response_id: None,
                    usage: Usage::default(),
                    stop_reason: StopReason::ToolUse,
                    error_message: None,
                    timestamp: 0,
                }
            } else {
                AssistantMessage {
                    role: String::from("assistant"),
                    content: vec![AssistantContent::Text {
                        text: format!("done: {}", payload["extensionHook"]),
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
                }
            };

            Ok(AssistantEvent::Done {
                reason: message.stop_reason.clone(),
                message,
            })
        }))
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
        "pi-coding-agent-cli-extension-tools-{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn register_recording_extension_provider() -> (Model, String, Arc<Mutex<RecordedExtensionState>>) {
    let api = unique_name("extension-tools-api");
    let provider = unique_name("extension-tools-provider");
    let model_id = unique_name("extension-tools-model");
    let state = Arc::new(Mutex::new(RecordedExtensionState::default()));
    register_provider(
        api.clone(),
        Arc::new(RecordingExtensionProvider {
            state: state.clone(),
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
    (model, api_name, state)
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

fn tool_result_text(context: &Context, tool_name: &str) -> Option<String> {
    context.messages.iter().find_map(|message| match message {
        Message::ToolResult {
            tool_name: current_tool_name,
            content,
            ..
        } if current_tool_name == tool_name => Some(
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
}

#[tokio::test]
async fn rpc_extension_bridge_supports_custom_tools_and_extension_hooks() {
    let (model, api, state) = register_recording_extension_provider();
    let cwd = unique_temp_dir("custom-tools-hooks");
    let extension_path = cwd.join("extension-tools.ts");
    fs::write(
        &extension_path,
        r#"import { Type } from "@sinclair/typebox";

export default function (pi) {
	pi.on("session_start", () => {
		pi.registerTool({
			name: "dynamic_tool",
			label: "Dynamic Tool",
			description: "Tool registered from extension",
			promptSnippet: "Run extension provided behavior",
			promptGuidelines: ["Use dynamic_tool when the user asks for dynamic behavior."],
			parameters: Type.Object({}),
			execute: async () => ({
				content: [{ type: "text", text: "raw extension result" }],
				details: { source: "tool" },
			}),
		});
	});

	pi.on("input", (event) => ({
		action: "transform",
		text: `${event.text} [input hook]`,
	}));

	pi.on("before_provider_request", (event) => ({
		...event.payload,
		extensionHook: true,
	}));

	pi.on("tool_result", (event) => {
		if (event.toolName !== "dynamic_tool") {
			return;
		}
		return {
			content: [{ type: "text", text: "mutated extension result" }],
			details: { source: "tool_result" },
		};
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

    let recorded = state.lock().unwrap().clone();
    assert_eq!(
        recorded.contexts.len(),
        2,
        "contexts: {:?}",
        recorded.contexts
    );
    assert!(
        recorded
            .payloads
            .iter()
            .all(|payload| payload.get("extensionHook") == Some(&Value::Bool(true))),
        "payloads: {:?}",
        recorded.payloads
    );

    let first_context = &recorded.contexts[0];
    let first_tool_names = first_context
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(
        first_tool_names.contains(&"dynamic_tool"),
        "tools: {:?}",
        first_context.tools
    );
    assert_eq!(last_user_text(first_context), "hello [input hook]");
    let first_system_prompt = first_context
        .system_prompt
        .as_deref()
        .expect("expected system prompt");
    assert!(
        first_system_prompt.contains("dynamic_tool: Run extension provided behavior"),
        "system prompt: {first_system_prompt}"
    );
    assert!(
        first_system_prompt.contains("Use dynamic_tool when the user asks for dynamic behavior."),
        "system prompt: {first_system_prompt}"
    );

    let second_context = &recorded.contexts[1];
    assert_eq!(
        tool_result_text(second_context, "dynamic_tool").as_deref(),
        Some("mutated extension result"),
        "context: {:?}",
        second_context
    );
    let tool_result_details = second_context
        .messages
        .iter()
        .find_map(|message| match message {
            Message::ToolResult {
                tool_name, details, ..
            } if tool_name == "dynamic_tool" => details.clone(),
            _ => None,
        });
    assert_eq!(
        tool_result_details,
        Some(json!({ "source": "tool_result" }))
    );

    unregister_provider(&api);
}
