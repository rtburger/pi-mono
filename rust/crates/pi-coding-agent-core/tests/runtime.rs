use async_stream::stream;
use pi_agent::ThinkingLevel;
use pi_ai::{
    AiProvider, AssistantEventStream, FauxModelDefinition, FauxResponse,
    RegisterFauxProviderOptions, StreamOptions, register_builtin_providers, register_faux_provider,
    register_provider, stream_response, unregister_provider,
};
use pi_coding_agent_core::{
    AuthApiKeyFuture, AuthSource, CodingAgentCoreError, CodingAgentCoreOptions, MemoryAuthStorage,
    SessionBootstrapOptions, create_coding_agent_core,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason,
    UserContent,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-coding-agent-core-runtime-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_models_json(path: &Path, value: serde_json::Value) {
    fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

fn assistant_text(message: &Message) -> Option<&str> {
    match message {
        Message::Assistant { content, .. } => content.iter().find_map(|block| match block {
            AssistantContent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }),
        _ => None,
    }
}

fn unique_name(prefix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{unique}")
}

fn provider_registry_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[derive(Clone)]
struct AsyncOnlyAuthSource {
    provider: String,
    api_key: String,
}

impl AuthSource for AsyncOnlyAuthSource {
    fn has_auth(&self, provider: &str) -> bool {
        provider == self.provider
    }

    fn get_api_key(&self, _provider: &str) -> Option<String> {
        None
    }

    fn get_api_key_for_request<'a>(&'a self, provider: &'a str) -> AuthApiKeyFuture<'a> {
        let api_key = (provider == self.provider).then(|| self.api_key.clone());
        Box::pin(async move { api_key })
    }
}

#[derive(Clone)]
struct RecordingProvider {
    recorded_api_key: Arc<Mutex<Option<String>>>,
}

impl AiProvider for RecordingProvider {
    fn stream(
        &self,
        model: Model,
        _context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        let recorded_api_key = self.recorded_api_key.clone();
        Box::pin(stream! {
            *recorded_api_key.lock().unwrap() = options.api_key.clone();

            let partial = AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
            let mut message = partial.clone();
            message.content.push(AssistantContent::Text {
                text: "async auth ok".into(),
                text_signature: None,
            });
            message.stop_reason = StopReason::Stop;
            message.timestamp = 1;

            yield Ok(AssistantEvent::Start { partial });
            yield Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message,
            });
        })
    }
}

#[derive(Clone)]
struct RecordingContextProvider {
    contexts: Arc<Mutex<Vec<Context>>>,
}

impl AiProvider for RecordingContextProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        _options: StreamOptions,
    ) -> AssistantEventStream {
        let contexts = self.contexts.clone();
        Box::pin(stream! {
            contexts.lock().unwrap().push(context);

            let partial = AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
            let mut message = partial.clone();
            message.content.push(AssistantContent::Text {
                text: "recorded".into(),
                text_signature: None,
            });
            message.stop_reason = StopReason::Stop;
            message.timestamp = 1;

            yield Ok(AssistantEvent::Start { partial });
            yield Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message,
            });
        })
    }
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
        let seen_options = self.seen_options.clone();
        Box::pin(stream! {
            seen_options.lock().unwrap().push(options);

            let partial = AssistantMessage::empty(model.api.clone(), model.provider.clone(), model.id.clone());
            let mut message = partial.clone();
            message.content.push(AssistantContent::Text {
                text: "recorded options".into(),
                text_signature: None,
            });
            message.stop_reason = StopReason::Stop;
            message.timestamp = 1;

            yield Ok(AssistantEvent::Start { partial });
            yield Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message,
            });
        })
    }
}

#[tokio::test]
async fn creates_prompt_capable_core_and_streams_via_faux_provider() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "runtime-faux".into(),
        models: vec![FauxModelDefinition {
            id: "runtime-faux-1".into(),
            name: Some("Runtime Faux".into()),
            reasoning: true,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text("Hello from faux")]);
    let model = faux.get_model(Some("runtime-faux-1")).unwrap();
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        model.provider.clone(),
        "test-token",
    )]));

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: auth,
        built_in_models: vec![model.clone()],
        models_json_path: None,
        cwd: None,
        tools: None,
        system_prompt: "You are a helpful coding assistant".into(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    created.core.prompt_text("Hi").await.unwrap();

    let state = created.core.state();
    assert_eq!(state.system_prompt, "You are a helpful coding assistant");
    assert_eq!(state.model.id, model.id);
    assert_eq!(state.thinking_level, ThinkingLevel::Medium);
    assert_eq!(state.messages.len(), 2);
    let last = state
        .messages
        .last()
        .unwrap()
        .as_standard_message()
        .unwrap();
    assert_eq!(assistant_text(last), Some("Hello from faux"));

    faux.unregister();
}

#[tokio::test]
async fn selected_model_uses_models_json_overrides() {
    let temp_dir = unique_temp_dir("base-url-override");
    let models_json_path = temp_dir.join("models.json");
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "override-faux".into(),
        models: vec![FauxModelDefinition {
            id: "override-faux-1".into(),
            name: Some("Override Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    let model = faux.get_model(Some("override-faux-1")).unwrap();
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                (model.provider.clone()): {
                    "baseUrl": "https://proxy.example.com/v1"
                }
            }
        }),
    );
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        model.provider.clone(),
        "test-token",
    )]));

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: auth,
        built_in_models: vec![model],
        models_json_path: Some(models_json_path),
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    assert_eq!(
        created.core.state().model.base_url,
        "https://proxy.example.com/v1"
    );

    faux.unregister();
}

#[tokio::test]
async fn prompt_materializes_registry_request_auth_failures_as_assistant_error_messages() {
    let temp_dir = unique_temp_dir("request-auth-failure");
    let models_json_path = temp_dir.join("models.json");
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "failure-faux".into(),
        models: vec![FauxModelDefinition {
            id: "failure-faux-1".into(),
            name: Some("Failure Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    let model = faux.get_model(Some("failure-faux-1")).unwrap();
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                (model.provider.clone()): {
                    "baseUrl": "https://proxy.example.com/v1",
                    "apiKey": "!exit 1",
                    "authHeader": true
                }
            }
        }),
    );

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(MemoryAuthStorage::new()),
        built_in_models: vec![model],
        models_json_path: Some(models_json_path),
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    created.core.prompt_text("Hi").await.unwrap();

    let state = created.core.state();
    let last = state
        .messages
        .last()
        .unwrap()
        .as_standard_message()
        .unwrap();
    match last {
        Message::Assistant {
            stop_reason,
            error_message,
            ..
        } => {
            assert_eq!(*stop_reason, StopReason::Error);
            assert!(error_message.as_deref().is_some_and(|message| {
                message.contains("Failed to resolve API key for provider")
            }));
        }
        other => panic!("expected assistant error message, got {other:?}"),
    }

    faux.unregister();
}

#[tokio::test]
async fn runtime_registers_default_coding_tools_and_executes_tool_calls() {
    let temp_dir = unique_temp_dir("tool-runtime");
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "tool-runtime-faux".into(),
        models: vec![FauxModelDefinition {
            id: "tool-runtime-faux-1".into(),
            name: Some("Tool Runtime Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![
        FauxResponse {
            content: vec![pi_ai::FauxContentBlock::ToolCall {
                id: "tool-1".into(),
                name: "write".into(),
                arguments: BTreeMap::from([
                    ("path".into(), json!("notes.txt")),
                    ("content".into(), json!("hello")),
                ]),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
        },
        FauxResponse::text("done"),
    ]);
    let model = faux.get_model(Some("tool-runtime-faux-1")).unwrap();
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        model.provider.clone(),
        "test-token",
    )]));

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: auth,
        built_in_models: vec![model],
        models_json_path: None,
        cwd: Some(temp_dir.clone()),
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    let tool_names = created
        .core
        .state()
        .tools
        .iter()
        .map(|tool| tool.definition.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        tool_names,
        vec![
            String::from("read"),
            String::from("bash"),
            String::from("edit"),
            String::from("write"),
        ]
    );

    created.core.prompt_text("create file").await.unwrap();

    assert_eq!(
        fs::read_to_string(temp_dir.join("notes.txt")).unwrap(),
        "hello"
    );
    let state = created.core.state();
    assert_eq!(state.messages.len(), 4);
    let tool_result = state.messages[2].as_standard_message().unwrap();
    assert_eq!(
        tool_result,
        &Message::ToolResult {
            tool_call_id: "tool-1".into(),
            tool_name: "write".into(),
            content: vec![UserContent::Text {
                text: "Successfully wrote 5 bytes to notes.txt".into(),
            }],
            is_error: false,
            timestamp: match tool_result {
                Message::ToolResult { timestamp, .. } => *timestamp,
                _ => unreachable!(),
            },
        }
    );
    let last = state.messages[3].as_standard_message().unwrap();
    assert_eq!(assistant_text(last), Some("done"));

    faux.unregister();
}

#[tokio::test]
async fn runtime_executes_edit_tool_calls_with_legacy_old_text_arguments() {
    let temp_dir = unique_temp_dir("edit-runtime");
    fs::write(temp_dir.join("notes.txt"), "before\n").unwrap();
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "edit-runtime-faux".into(),
        models: vec![FauxModelDefinition {
            id: "edit-runtime-faux-1".into(),
            name: Some("Edit Runtime Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![
        FauxResponse {
            content: vec![pi_ai::FauxContentBlock::ToolCall {
                id: "tool-1".into(),
                name: "edit".into(),
                arguments: BTreeMap::from([
                    ("path".into(), json!("notes.txt")),
                    ("oldText".into(), json!("before")),
                    ("newText".into(), json!("after")),
                ]),
            }],
            stop_reason: StopReason::ToolUse,
            error_message: None,
        },
        FauxResponse::text("done"),
    ]);
    let model = faux.get_model(Some("edit-runtime-faux-1")).unwrap();
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        model.provider.clone(),
        "test-token",
    )]));

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: auth,
        built_in_models: vec![model],
        models_json_path: None,
        cwd: Some(temp_dir.clone()),
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    created.core.prompt_text("edit file").await.unwrap();

    assert_eq!(
        fs::read_to_string(temp_dir.join("notes.txt")).unwrap(),
        "after\n"
    );
    let state = created.core.state();
    let tool_result = state.messages[2].as_standard_message().unwrap();
    assert_eq!(
        tool_result,
        &Message::ToolResult {
            tool_call_id: "tool-1".into(),
            tool_name: "edit".into(),
            content: vec![UserContent::Text {
                text: "Successfully replaced 1 block(s) in notes.txt.".into(),
            }],
            is_error: false,
            timestamp: match tool_result {
                Message::ToolResult { timestamp, .. } => *timestamp,
                _ => unreachable!(),
            },
        }
    );

    faux.unregister();
}

#[tokio::test]
async fn runtime_can_toggle_block_images_between_requests() {
    let api = unique_name("runtime-block-images-api");
    let provider = unique_name("runtime-block-images-provider");
    let model_id = unique_name("runtime-block-images-model");
    let recorded_contexts = Arc::new(Mutex::new(Vec::new()));
    register_provider(
        api.clone(),
        Arc::new(RecordingContextProvider {
            contexts: recorded_contexts.clone(),
        }),
    );

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.clone(),
            "token",
        )])),
        built_in_models: vec![Model {
            id: model_id.clone(),
            name: "Block Images Model".into(),
            api: api.clone(),
            provider: provider.clone(),
            base_url: "https://example.com/v1".into(),
            reasoning: false,
            input: vec!["text".into(), "image".into()],
            context_window: 128_000,
            max_tokens: 16_384,
        }],
        models_json_path: None,
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    created.core.set_block_images(true);
    created
        .core
        .prompt_message(Message::User {
            content: vec![
                UserContent::Text {
                    text: "before".into(),
                },
                UserContent::Image {
                    data: "image-1".into(),
                    mime_type: "image/png".into(),
                },
                UserContent::Image {
                    data: "image-2".into(),
                    mime_type: "image/png".into(),
                },
                UserContent::Text {
                    text: "after".into(),
                },
            ],
            timestamp: 1,
        })
        .await
        .unwrap();

    created.core.set_block_images(false);
    created
        .core
        .prompt_message(Message::User {
            content: vec![
                UserContent::Text {
                    text: "keep".into(),
                },
                UserContent::Image {
                    data: "image-3".into(),
                    mime_type: "image/png".into(),
                },
            ],
            timestamp: 2,
        })
        .await
        .unwrap();

    let contexts = recorded_contexts.lock().unwrap();
    assert_eq!(contexts.len(), 2);
    assert_eq!(
        contexts[0].messages.last(),
        Some(&Message::User {
            content: vec![
                UserContent::Text {
                    text: "before".into(),
                },
                UserContent::Text {
                    text: "Image reading is disabled.".into(),
                },
                UserContent::Text {
                    text: "after".into(),
                },
            ],
            timestamp: 1,
        })
    );
    assert_eq!(
        contexts[1].messages.last(),
        Some(&Message::User {
            content: vec![
                UserContent::Text {
                    text: "keep".into(),
                },
                UserContent::Image {
                    data: "image-3".into(),
                    mime_type: "image/png".into(),
                },
            ],
            timestamp: 2,
        })
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn runtime_registry_backed_streamer_uses_simple_stream_reasoning_mapping() {
    let _guard = provider_registry_guard();
    let faux = register_faux_provider(RegisterFauxProviderOptions::default());
    let faux_model = faux.get_model(None).expect("faux model");
    let _ = stream_response(faux_model, Context::default(), StreamOptions::default())
        .expect("faux provider should prime builtin registration");
    faux.unregister();

    let seen_options = Arc::new(Mutex::new(Vec::new()));
    register_provider(
        "anthropic-messages",
        Arc::new(RecordingOptionsProvider {
            seen_options: seen_options.clone(),
        }),
    );

    let created = create_coding_agent_core(CodingAgentCoreOptions {
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
            context_window: 200_000,
            max_tokens: 60_000,
        }],
        models_json_path: None,
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions {
            default_thinking_level: Some(ThinkingLevel::High),
            ..Default::default()
        },
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    assert_eq!(created.core.state().thinking_level, ThinkingLevel::High);
    created.core.prompt_text("Hi").await.unwrap();

    let seen_options = seen_options.lock().unwrap().clone();
    assert_eq!(seen_options.len(), 1);
    assert_eq!(seen_options[0].api_key.as_deref(), Some("test-token"));
    assert_eq!(seen_options[0].reasoning_effort.as_deref(), Some("high"));
    assert_eq!(seen_options[0].max_tokens, Some(48_384));

    unregister_provider("anthropic-messages");
    register_builtin_providers();
}

#[tokio::test]
async fn runtime_uses_async_request_auth_resolution() {
    let api = unique_name("runtime-async-auth-api");
    let provider = unique_name("runtime-async-auth-provider");
    let model_id = unique_name("runtime-async-auth-model");
    let recorded_api_key = Arc::new(Mutex::new(None));
    register_provider(
        api.clone(),
        Arc::new(RecordingProvider {
            recorded_api_key: recorded_api_key.clone(),
        }),
    );

    let created = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(AsyncOnlyAuthSource {
            provider: provider.clone(),
            api_key: "refreshed-token".into(),
        }),
        built_in_models: vec![Model {
            id: model_id.clone(),
            name: "Async Auth Model".into(),
            api: api.clone(),
            provider: provider.clone(),
            base_url: "https://example.com/v1".into(),
            reasoning: false,
            input: vec!["text".into()],
            context_window: 128_000,
            max_tokens: 16_384,
        }],
        models_json_path: None,
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    })
    .unwrap();

    created.core.prompt_text("Hi").await.unwrap();

    assert_eq!(
        recorded_api_key.lock().unwrap().as_deref(),
        Some("refreshed-token")
    );
    let state = created.core.state();
    let last = state
        .messages
        .last()
        .unwrap()
        .as_standard_message()
        .unwrap();
    assert_eq!(assistant_text(last), Some("async auth ok"));

    unregister_provider(&api);
}

#[test]
fn returns_no_model_available_when_bootstrap_cannot_select_one() {
    let result = create_coding_agent_core(CodingAgentCoreOptions {
        auth_source: Arc::new(MemoryAuthStorage::new()),
        built_in_models: Vec::new(),
        models_json_path: None,
        cwd: None,
        tools: None,
        system_prompt: String::new(),
        bootstrap: SessionBootstrapOptions::default(),
        stream_options: StreamOptions::default(),
    });

    assert!(matches!(
        result,
        Err(CodingAgentCoreError::NoModelAvailable)
    ));
}
