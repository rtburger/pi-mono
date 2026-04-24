use async_stream::stream;
use parking_lot::Mutex;
use pi_ai::{AiProvider, AssistantEventStream, StreamOptions, unregister_provider};
use pi_coding_agent_core::{
    AgentSessionOptions, AgentSessionRuntimeRequest, CodingAgentCoreOptions,
    CreateAgentSessionRuntimeFactory, MemoryAuthStorage, NewSessionOptions,
    SessionBootstrapOptions, SessionManager, create_agent_session, create_agent_session_runtime,
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

#[derive(Clone)]
struct RecordingProvider {
    response_text: String,
    contexts: Arc<Mutex<Vec<Context>>>,
}

impl AiProvider for RecordingProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        _options: StreamOptions,
    ) -> AssistantEventStream {
        let contexts = self.contexts.clone();
        let response_text = self.response_text.clone();
        Box::pin(stream! {
            contexts.lock().push(context);
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

#[tokio::test]
async fn create_agent_session_restores_messages_and_persists_new_prompts() {
    let api = unique_name("agent-session-api");
    let provider = unique_name("agent-session-provider");
    let model_id = unique_name("agent-session-model");
    let contexts = Arc::new(Mutex::new(Vec::new()));
    pi_ai::register_provider(
        api.clone(),
        Arc::new(RecordingProvider {
            response_text: String::from("recorded"),
            contexts: contexts.clone(),
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("agent-session-cwd");
    let session_dir = cwd.join("sessions");
    let manager = Arc::new(Mutex::new(
        SessionManager::create(
            cwd.to_string_lossy().as_ref(),
            Some(session_dir.to_string_lossy().as_ref()),
        )
        .unwrap(),
    ));
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        provider.as_str(),
        "token",
    )]));

    let first = create_agent_session(AgentSessionOptions {
        core: CodingAgentCoreOptions {
            auth_source: auth.clone(),
            built_in_models: vec![built_in_model.clone()],
            models_json_path: None,
            cwd: Some(cwd.clone()),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager: Some(manager.clone()),
    })
    .unwrap();

    first.session.prompt_text("first turn").await.unwrap();
    let session_file = manager
        .lock()
        .get_session_file()
        .map(str::to_owned)
        .expect("expected persisted session file");
    assert!(PathBuf::from(&session_file).exists());
    contexts.lock().clear();
    drop(first);

    let reopened = Arc::new(Mutex::new(
        SessionManager::open(
            &session_file,
            Some(session_dir.to_string_lossy().as_ref()),
            None,
        )
        .unwrap(),
    ));
    let restored = create_agent_session(AgentSessionOptions {
        core: CodingAgentCoreOptions {
            auth_source: auth,
            built_in_models: vec![built_in_model],
            models_json_path: None,
            cwd: Some(cwd.clone()),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager: Some(reopened.clone()),
    })
    .unwrap();

    restored.session.prompt_text("second turn").await.unwrap();

    let recorded_contexts = contexts.lock().clone();
    assert_eq!(recorded_contexts.len(), 1);
    let context = &recorded_contexts[0];
    assert_eq!(context.messages.len(), 3, "context: {context:?}");
    match &context.messages[0] {
        Message::User { content, .. } => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("first turn"),
                }]
            );
        }
        other => panic!("expected first user message, got {other:?}"),
    }
    match &context.messages[1] {
        Message::Assistant { content, .. } => {
            assert!(matches!(
                content.as_slice(),
                [AssistantContent::Text { text, .. }] if text == "recorded"
            ));
        }
        other => panic!("expected restored assistant message, got {other:?}"),
    }
    match &context.messages[2] {
        Message::User { content, .. } => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("second turn"),
                }]
            );
        }
        other => panic!("expected second user message, got {other:?}"),
    }

    let entries = reopened.lock().get_entries().to_vec();
    assert!(entries.len() >= 6, "entries: {entries:?}");

    unregister_provider(&api);
}

#[tokio::test]
async fn agent_session_runtime_new_session_recreates_runtime_with_new_session_id() {
    let api = unique_name("agent-session-runtime-api");
    let provider = unique_name("agent-session-runtime-provider");
    let model_id = unique_name("agent-session-runtime-model");
    let contexts = Arc::new(Mutex::new(Vec::new()));
    pi_ai::register_provider(
        api.clone(),
        Arc::new(RecordingProvider {
            response_text: String::from("runtime"),
            contexts,
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("agent-session-runtime-cwd");
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        provider.as_str(),
        "token",
    )]));
    let factory: CreateAgentSessionRuntimeFactory = Arc::new({
        let auth = auth.clone();
        let built_in_model = built_in_model.clone();
        move |request: AgentSessionRuntimeRequest| {
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
        }
    });

    let manager = Arc::new(Mutex::new(SessionManager::in_memory(
        cwd.to_string_lossy().as_ref(),
    )));
    let mut runtime = create_agent_session_runtime(
        factory,
        AgentSessionRuntimeRequest {
            cwd: cwd.clone(),
            session_manager: Some(manager),
        },
    )
    .await
    .unwrap();

    let first_session_id = runtime
        .session()
        .session_id()
        .expect("expected initial session id");
    runtime.session().prompt_text("hello").await.unwrap();
    runtime
        .new_session(NewSessionOptions::default())
        .await
        .unwrap();

    let second_session_id = runtime
        .session()
        .session_id()
        .expect("expected replacement session id");
    assert_ne!(first_session_id, second_session_id);
    assert!(runtime.session().state().messages.is_empty());
    assert_eq!(
        runtime
            .session()
            .session_manager()
            .expect("expected session manager")
            .lock()
            .get_entries()
            .len(),
        2
    );

    unregister_provider(&api);
}
