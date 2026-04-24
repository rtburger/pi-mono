use async_stream::stream;
use parking_lot::Mutex;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_core::{
    AgentSessionOptions, AgentSessionRuntimeRequest, CodingAgentCoreOptions,
    CreateAgentSessionRuntimeFactory, MemoryAuthStorage, NavigateTreeOptions,
    SessionBootstrapOptions, SessionEntry, SessionManager, create_agent_session,
    create_agent_session_runtime,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason,
    UserContent,
};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

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

#[derive(Clone)]
struct StaticResponseProvider {
    response_text: String,
}

impl AiProvider for StaticResponseProvider {
    fn stream(
        &self,
        model: Model,
        _context: Context,
        _options: StreamOptions,
    ) -> AssistantEventStream {
        let response_text = self.response_text.clone();
        Box::pin(stream! {
            let mut message = AssistantMessage::empty(
                model.api.clone(),
                model.provider.clone(),
                model.id.clone(),
            );
            message.content.push(AssistantContent::Text {
                text: response_text,
                text_signature: None,
            });
            message.stop_reason = StopReason::Stop;
            message.timestamp = 1;
            yield Ok(AssistantEvent::Done {
                reason: StopReason::Stop,
                message,
            });
        })
    }
}

fn create_runtime_factory(
    auth: Arc<MemoryAuthStorage>,
    built_in_model: Model,
) -> CreateAgentSessionRuntimeFactory {
    Arc::new(move |request: AgentSessionRuntimeRequest| {
        let auth = auth.clone();
        let built_in_model = built_in_model.clone();
        Box::pin(async move {
            create_agent_session(AgentSessionOptions {
                core: CodingAgentCoreOptions {
                    auth_source: auth,
                    built_in_models: vec![built_in_model],
                    models_json_path: None,
                    cwd: Some(request.cwd.clone()),
                    tools: None,
                    system_prompt: String::new(),
                    bootstrap: SessionBootstrapOptions::default(),
                    stream_options: StreamOptions::default(),
                },
                session_manager: request.session_manager,
            })
            .map_err(Into::into)
        })
    })
}

#[tokio::test]
async fn agent_session_exposes_session_name_and_tree_helpers() {
    let api = unique_name("runtime-api-parity-session-api");
    let provider = unique_name("runtime-api-parity-session-provider");
    let model_id = unique_name("runtime-api-parity-session-model");
    register_provider(
        api.clone(),
        Arc::new(StaticResponseProvider {
            response_text: String::from("session helper response"),
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runtime-api-parity-session-cwd");
    let manager = Arc::new(Mutex::new(SessionManager::in_memory(
        cwd.to_string_lossy().as_ref(),
    )));
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        provider.as_str(),
        "token",
    )]));

    let created = create_agent_session(AgentSessionOptions {
        core: CodingAgentCoreOptions {
            auth_source: auth,
            built_in_models: vec![built_in_model],
            models_json_path: None,
            cwd: Some(cwd),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager: Some(manager.clone()),
    })
    .unwrap();
    let session = created.session;

    session.prompt_text("first turn").await.unwrap();
    session.set_session_name("named session").unwrap();

    let (expected_leaf_id, expected_tree) = {
        let manager = manager.lock();
        (manager.get_leaf_id().map(str::to_owned), manager.get_tree())
    };

    assert_eq!(session.session_name().as_deref(), Some("named session"));
    assert_eq!(session.leaf_id(), expected_leaf_id);
    assert_eq!(session.session_tree(), expected_tree);

    unregister_provider(&api);
}

#[tokio::test]
async fn agent_session_runtime_delegates_history_helpers() {
    let api = unique_name("runtime-api-parity-runtime-api");
    let provider = unique_name("runtime-api-parity-runtime-provider");
    let model_id = unique_name("runtime-api-parity-runtime-model");
    register_provider(
        api.clone(),
        Arc::new(StaticResponseProvider {
            response_text: String::from("runtime helper response"),
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runtime-api-parity-runtime-cwd");
    let manager = Arc::new(Mutex::new(SessionManager::in_memory(
        cwd.to_string_lossy().as_ref(),
    )));
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        provider.as_str(),
        "token",
    )]));
    let factory = create_runtime_factory(auth, built_in_model);

    let runtime = create_agent_session_runtime(
        factory,
        AgentSessionRuntimeRequest {
            cwd,
            session_manager: Some(manager.clone()),
        },
    )
    .await
    .unwrap();

    runtime
        .session()
        .prompt_text("runtime prompt")
        .await
        .unwrap();
    runtime.set_session_name("runtime session").unwrap();

    let bash = runtime
        .execute_bash("printf 'runtime bash'; exit 7", false)
        .await
        .unwrap();
    assert_eq!(bash.exit_code, Some(7));
    assert_eq!(bash.output, "runtime bash");
    assert!(runtime.session_id().is_some());
    assert!(runtime.session_file().is_none());
    assert_eq!(runtime.session_name().as_deref(), Some("runtime session"));

    let stats = runtime.session_stats();
    assert_eq!(stats.user_messages, 1);
    assert_eq!(stats.assistant_messages, 1);
    assert_eq!(stats.tool_calls, 0);
    assert_eq!(stats.tool_results, 0);
    assert_eq!(stats.total_messages, 3);

    let first_user_id = {
        let manager = manager.lock();
        manager
            .get_entries()
            .iter()
            .find_map(|entry| match entry {
                SessionEntry::Message { id, message, .. } => match message.as_standard_message() {
                    Some(Message::User { .. }) => Some(id.clone()),
                    _ => None,
                },
                _ => None,
            })
            .expect("expected first user message")
    };

    let navigation = runtime
        .navigate_tree(Some(first_user_id.as_str()), NavigateTreeOptions::default())
        .await
        .unwrap();
    assert_eq!(navigation.editor_text.as_deref(), Some("runtime prompt"));
    assert_eq!(runtime.leaf_id(), navigation.new_leaf_id);

    let expected_tree = manager.lock().get_tree();
    assert_eq!(runtime.session_tree(), expected_tree);

    unregister_provider(&api);
}

#[tokio::test]
async fn agent_session_runtime_execute_bash_uses_runtime_cwd_without_session_history() {
    let api = unique_name("runtime-api-parity-ephemeral-bash-api");
    let provider = unique_name("runtime-api-parity-ephemeral-bash-provider");
    let model_id = unique_name("runtime-api-parity-ephemeral-bash-model");
    register_provider(
        api.clone(),
        Arc::new(StaticResponseProvider {
            response_text: String::from("runtime ephemeral bash response"),
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runtime-api-parity-ephemeral-bash-cwd");
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        provider.as_str(),
        "token",
    )]));
    let factory = create_runtime_factory(auth, built_in_model);

    let runtime = create_agent_session_runtime(
        factory,
        AgentSessionRuntimeRequest {
            cwd: cwd.clone(),
            session_manager: None,
        },
    )
    .await
    .unwrap();

    let result = runtime
        .execute_bash("printf '%s' \"$PWD\"", false)
        .await
        .unwrap();

    assert_eq!(result.output, cwd.to_string_lossy());
    assert!(runtime.session().session_manager().is_none());
    assert_eq!(runtime.session_stats().total_messages, 1);

    unregister_provider(&api);
}

#[tokio::test]
async fn agent_session_runtime_import_from_jsonl_updates_public_helpers() {
    let api = unique_name("runtime-api-parity-import-api");
    let provider = unique_name("runtime-api-parity-import-provider");
    let model_id = unique_name("runtime-api-parity-import-model");
    register_provider(
        api.clone(),
        Arc::new(StaticResponseProvider {
            response_text: String::from("runtime import response"),
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runtime-api-parity-import-cwd");
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        provider.as_str(),
        "token",
    )]));
    let factory = create_runtime_factory(auth, built_in_model);

    let mut runtime = create_agent_session_runtime(
        factory,
        AgentSessionRuntimeRequest {
            cwd: cwd.clone(),
            session_manager: None,
        },
    )
    .await
    .unwrap();

    let imported_cwd = unique_temp_dir("runtime-api-parity-imported-cwd");
    let mut imported_manager = SessionManager::in_memory(imported_cwd.to_string_lossy().as_ref());
    imported_manager
        .append_session_info("imported session")
        .unwrap();
    imported_manager
        .append_message(Message::User {
            content: vec![UserContent::Text {
                text: String::from("imported prompt"),
            }],
            timestamp: 1,
        })
        .unwrap();
    imported_manager
        .append_thinking_level_change("off")
        .unwrap();
    let expected_tree = imported_manager.get_tree();
    let import_path = cwd.join("imported.jsonl");
    imported_manager.export_branch_jsonl(&import_path).unwrap();

    runtime
        .import_from_jsonl("imported.jsonl", None)
        .await
        .unwrap();

    assert_eq!(runtime.cwd(), imported_cwd.as_path());
    assert_eq!(runtime.session_name().as_deref(), Some("imported session"));
    assert_eq!(runtime.session_tree(), expected_tree);
    assert_eq!(runtime.session_stats().user_messages, 1);

    unregister_provider(&api);
}
