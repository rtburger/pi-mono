use async_stream::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_core::{
    AgentSessionOptions, AgentSessionRuntimeRequest, CodingAgentCoreOptions,
    CreateAgentSessionRuntimeFactory, MemoryAuthStorage, SessionBootstrapOptions,
    create_agent_session, create_agent_session_runtime,
};
use pi_events::{AssistantContent, AssistantEvent, AssistantMessage, Context, Model, StopReason};
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

#[tokio::test]
async fn agent_session_runtime_reload_preserves_ephemeral_state() {
    let api = unique_name("runtime-reload-api");
    let provider = unique_name("runtime-reload-provider");
    let model_id = unique_name("runtime-reload-model");
    register_provider(
        api.clone(),
        Arc::new(StaticResponseProvider {
            response_text: String::from("reload response"),
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("runtime-reload-cwd");
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

    let mut runtime = create_agent_session_runtime(
        factory,
        AgentSessionRuntimeRequest {
            cwd: cwd.clone(),
            session_manager: None,
        },
    )
    .await
    .unwrap();

    runtime
        .session()
        .prompt_text("hello before reload")
        .await
        .unwrap();

    let before_messages = runtime.session().state().messages;
    assert_eq!(before_messages.len(), 2, "messages: {before_messages:?}");

    runtime.reload().await.unwrap();

    let after_messages = runtime.session().state().messages;
    let after_session_id = runtime.session().session_id();
    assert!(after_session_id.is_some());
    assert_eq!(after_messages.len(), before_messages.len());
    assert!(matches!(
        after_messages[0].as_standard_message(),
        Some(pi_events::Message::User { .. })
    ));
    assert!(matches!(
        after_messages[1].as_standard_message(),
        Some(pi_events::Message::Assistant { .. })
    ));

    unregister_provider(&api);
}
