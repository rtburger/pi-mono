use async_stream::stream;
use parking_lot::Mutex;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_core::{
    AgentSession, AgentSessionOptions, BashExecutionMessage, CodingAgentCoreOptions,
    MemoryAuthStorage, SessionBootstrapOptions, SessionManager, create_agent_session,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Model, ModelCost, StopReason,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Notify, time::Duration};

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

fn build_session(
    built_in_model: Model,
    cwd: &Path,
    session_manager: Option<Arc<Mutex<SessionManager>>>,
) -> AgentSession {
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        built_in_model.provider.as_str(),
        "token",
    )]));

    create_agent_session(AgentSessionOptions {
        core: CodingAgentCoreOptions {
            auth_source: auth,
            built_in_models: vec![built_in_model],
            models_json_path: None,
            cwd: Some(cwd.to_path_buf()),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager,
    })
    .unwrap()
    .session
}

fn last_bash_message(session: &AgentSession) -> BashExecutionMessage {
    session
        .state()
        .messages
        .iter()
        .rev()
        .find_map(|message| {
            message
                .as_custom_message()
                .and_then(|message| message.decode_typed_payload::<BashExecutionMessage>().ok())
                .flatten()
        })
        .expect("expected bash execution message")
}

#[tokio::test]
async fn execute_bash_returns_exit_code_and_records_result() {
    let cwd = unique_temp_dir("session-bash-exit-code");
    let session_manager = Arc::new(Mutex::new(SessionManager::in_memory(
        cwd.to_string_lossy().as_ref(),
    )));
    let session = build_session(
        model("support-api", "support-provider", "support-model"),
        &cwd,
        Some(session_manager.clone()),
    );

    let result = session
        .execute_bash("printf 'bash output'; exit 7", false)
        .await
        .unwrap();

    assert_eq!(result.output, "bash output");
    assert_eq!(result.exit_code, Some(7));
    assert!(!result.cancelled);
    assert!(!result.truncated);
    assert!(result.full_output_path.is_none());

    let recorded = last_bash_message(&session);
    assert_eq!(recorded.command, "printf 'bash output'; exit 7");
    assert_eq!(recorded.output, "bash output");
    assert_eq!(recorded.exit_code, Some(7));
    assert!(!recorded.cancelled);
    assert!(!recorded.exclude_from_context);

    let restored = session_manager.lock().build_session_context();
    let persisted = restored
        .messages
        .iter()
        .rev()
        .find_map(|message| {
            message
                .as_custom_message()
                .and_then(|message| message.decode_typed_payload::<BashExecutionMessage>().ok())
                .flatten()
        })
        .expect("expected persisted bash execution message");
    assert_eq!(persisted.output, "bash output");
    assert_eq!(persisted.exit_code, Some(7));
}

#[tokio::test]
async fn execute_bash_can_be_aborted_and_records_cancellation() {
    let cwd = unique_temp_dir("session-bash-abort");
    let session = build_session(
        model("abort-api", "abort-provider", "abort-model"),
        &cwd,
        Some(Arc::new(Mutex::new(SessionManager::in_memory(
            cwd.to_string_lossy().as_ref(),
        )))),
    );

    let running_session = session.clone();
    let handle =
        tokio::spawn(async move { running_session.execute_bash("sleep 5", true).await.unwrap() });

    for _ in 0..100 {
        if session.is_bash_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(session.is_bash_running(), "bash command never started");

    session.abort_bash();
    let result = handle.await.unwrap();

    assert!(result.cancelled);
    assert_eq!(result.exit_code, None);
    assert!(!session.is_bash_running());

    let recorded = last_bash_message(&session);
    assert!(recorded.cancelled);
    assert_eq!(recorded.exit_code, None);
    assert!(recorded.exclude_from_context);
}

#[derive(Clone)]
struct BlockingProvider {
    started: Arc<AtomicBool>,
    release: Arc<Notify>,
}

impl AiProvider for BlockingProvider {
    fn stream(
        &self,
        model: Model,
        _context: Context,
        _options: StreamOptions,
    ) -> AssistantEventStream {
        let started = self.started.clone();
        let release = self.release.clone();
        Box::pin(stream! {
            started.store(true, Ordering::SeqCst);

            let partial = AssistantMessage::empty(
                model.api.clone(),
                model.provider.clone(),
                model.id.clone(),
            );
            yield Ok(AssistantEvent::Start {
                partial: partial.clone(),
            });

            release.notified().await;

            let mut message = partial;
            message.content.push(AssistantContent::Text {
                text: String::from("slow response"),
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
async fn execute_bash_queues_messages_until_streaming_turn_finishes() {
    let api = unique_name("bash-queue-api");
    let provider = unique_name("bash-queue-provider");
    let model_id = unique_name("bash-queue-model");
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(Notify::new());
    register_provider(
        api.clone(),
        Arc::new(BlockingProvider {
            started: started.clone(),
            release: release.clone(),
        }),
    );

    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("session-bash-queue");
    let session = build_session(
        built_in_model,
        &cwd,
        Some(Arc::new(Mutex::new(SessionManager::in_memory(
            cwd.to_string_lossy().as_ref(),
        )))),
    );

    let prompt_session = session.clone();
    let prompt_task = tokio::spawn(async move {
        prompt_session.prompt_text("slow prompt").await.unwrap();
    });

    for _ in 0..100 {
        if started.load(Ordering::SeqCst) && session.state().is_streaming {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(started.load(Ordering::SeqCst), "provider never started");
    assert!(
        session.state().is_streaming,
        "session never started streaming"
    );

    let result = session
        .execute_bash("printf 'queued bash'", false)
        .await
        .unwrap();
    assert_eq!(result.output, "queued bash");
    assert!(session.has_pending_bash_messages());
    assert!(
        session
            .state()
            .messages
            .iter()
            .all(|message| message.role() != "bashExecution"),
        "bash message should stay pending while assistant is streaming"
    );

    release.notify_waiters();
    prompt_task.await.unwrap();
    session.wait_for_idle().await;

    assert!(!session.has_pending_bash_messages());
    let recorded = last_bash_message(&session);
    assert_eq!(recorded.command, "printf 'queued bash'");
    assert_eq!(recorded.output, "queued bash");

    unregister_provider(&api);
}
