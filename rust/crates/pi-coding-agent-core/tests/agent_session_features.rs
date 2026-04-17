use async_stream::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_core::{
    AgentSessionEvent, AgentSessionOptions, CodingAgentCoreOptions, CompactionReason,
    CompactionSettings, MemoryAuthStorage, RetrySettings, SessionBootstrapOptions, SessionManager,
    create_agent_session,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Model, StopReason, Usage,
    UserContent,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;

fn unique_name(prefix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{unique}")
}

fn model(api: &str, provider: &str, id: &str, context_window: u64) -> Model {
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
        context_window,
        max_tokens: 16_384,
        compat: None,
    }
}

fn usage(input: u64, output: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read: 0,
        cache_write: 0,
        total_tokens: input + output,
        cost: Default::default(),
    }
}

fn assistant_message(
    model: &Model,
    text: &str,
    stop_reason: StopReason,
    usage: Usage,
    error_message: Option<&str>,
    timestamp: u64,
) -> AssistantMessage {
    AssistantMessage {
        role: String::from("assistant"),
        content: vec![AssistantContent::Text {
            text: text.to_owned(),
            text_signature: None,
        }],
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_id: None,
        usage,
        stop_reason,
        error_message: error_message.map(str::to_owned),
        timestamp,
    }
}

#[derive(Clone)]
struct ScriptedProvider {
    replies: Arc<Mutex<VecDeque<AssistantMessage>>>,
    call_count: Arc<Mutex<usize>>,
    contexts: Arc<Mutex<Vec<Context>>>,
}

impl ScriptedProvider {
    fn new(replies: Vec<AssistantMessage>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            call_count: Arc::new(Mutex::new(0)),
            contexts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }

    fn contexts(&self) -> Vec<Context> {
        self.contexts.lock().unwrap().clone()
    }
}

impl AiProvider for ScriptedProvider {
    fn stream(
        &self,
        _model: Model,
        context: Context,
        _options: StreamOptions,
    ) -> AssistantEventStream {
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("expected scripted reply");
        *self.call_count.lock().unwrap() += 1;
        self.contexts.lock().unwrap().push(context);
        let reason = reply.stop_reason.clone();
        Box::pin(stream! {
            yield Ok(AssistantEvent::Done {
                reason,
                message: reply,
            });
        })
    }
}

fn create_session(
    built_in_model: Model,
    provider: ScriptedProvider,
) -> pi_coding_agent_core::AgentSession {
    register_provider(built_in_model.api.clone(), Arc::new(provider));
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        built_in_model.provider.as_str(),
        "token",
    )]));

    create_agent_session(AgentSessionOptions {
        core: CodingAgentCoreOptions {
            auth_source: auth,
            built_in_models: vec![built_in_model],
            models_json_path: None,
            cwd: None,
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager: Some(Arc::new(Mutex::new(SessionManager::in_memory(
            "/tmp/session",
        )))),
    })
    .unwrap()
    .session
}

fn expect_first_message_role(session: &pi_coding_agent_core::AgentSession, expected_role: &str) {
    let first_role = session
        .state()
        .messages
        .first()
        .map(|message| message.role().to_owned());
    assert_eq!(first_role.as_deref(), Some(expected_role));
}

#[tokio::test]
async fn manual_compaction_emits_events_and_rebuilds_session_context() {
    let api = unique_name("manual-compaction-api");
    let provider_name = unique_name("manual-compaction-provider");
    let model_id = unique_name("manual-compaction-model");
    let built_in_model = model(&api, &provider_name, &model_id, 128_000);
    let provider = ScriptedProvider::new(vec![
        assistant_message(
            &built_in_model,
            "first reply",
            StopReason::Stop,
            usage(80, 20),
            None,
            10,
        ),
        assistant_message(
            &built_in_model,
            "## Goal\nCompact the session\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] Captured earlier work\n\n### In Progress\n- [ ] Continue\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Compaction**: Keep the recent branch\n\n## Next Steps\n1. Continue from the compacted context\n\n## Critical Context\n- (none)",
            StopReason::Stop,
            usage(10, 10),
            None,
            20,
        ),
    ]);
    let provider_handle = provider.clone();
    let session = create_session(built_in_model.clone(), provider);
    session.set_compaction_settings(CompactionSettings {
        enabled: true,
        reserve_tokens: 16_384,
        keep_recent_tokens: 1,
    });

    let events = Arc::new(Mutex::new(Vec::<AgentSessionEvent>::new()));
    let events_clone = events.clone();
    let _unsubscribe = session.subscribe(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    session.prompt_text("hello").await.unwrap();
    let result = session.compact(None).await.unwrap();

    assert!(result.summary.contains("## Goal"));
    expect_first_message_role(&session, "compactionSummary");
    assert_eq!(provider_handle.call_count(), 2);

    let events = events.lock().unwrap().clone();
    assert!(events.contains(&AgentSessionEvent::CompactionStart {
        reason: CompactionReason::Manual,
    }));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentSessionEvent::CompactionEnd {
            reason: CompactionReason::Manual,
            result: Some(_),
            aborted: false,
            will_retry: false,
            error_message: None,
        }
    )));

    unregister_provider(&api);
}

#[tokio::test]
async fn retries_transient_errors_and_waits_for_recovery() {
    let api = unique_name("retry-api");
    let provider_name = unique_name("retry-provider");
    let model_id = unique_name("retry-model");
    let built_in_model = model(&api, &provider_name, &model_id, 128_000);
    let provider = ScriptedProvider::new(vec![
        assistant_message(
            &built_in_model,
            "",
            StopReason::Error,
            Usage::default(),
            Some("overloaded_error"),
            10,
        ),
        assistant_message(
            &built_in_model,
            "recovered",
            StopReason::Stop,
            usage(10, 10),
            None,
            20,
        ),
    ]);
    let provider_handle = provider.clone();
    let session = create_session(built_in_model.clone(), provider);
    session.set_retry_settings(RetrySettings {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 1,
    });

    let retry_events = Arc::new(Mutex::new(Vec::<AgentSessionEvent>::new()));
    let retry_events_clone = retry_events.clone();
    let _unsubscribe = session.subscribe(move |event| {
        retry_events_clone.lock().unwrap().push(event);
    });

    session.prompt_text("retry me").await.unwrap();

    assert_eq!(provider_handle.call_count(), 2);
    assert!(!session.is_retrying());
    let retry_events = retry_events.lock().unwrap().clone();
    assert!(retry_events.iter().any(|event| matches!(
        event,
        AgentSessionEvent::AutoRetryStart {
            attempt: 1,
            max_attempts: 3,
            delay_ms: 1,
            error_message,
        } if error_message == "overloaded_error"
    )));
    assert!(retry_events.iter().any(|event| matches!(
        event,
        AgentSessionEvent::AutoRetryEnd {
            success: true,
            attempt: 1,
            final_error: None,
        }
    )));
    assert_eq!(
        session
            .state()
            .messages
            .last()
            .and_then(|message| message.as_standard_message()),
        Some(&pi_events::Message::Assistant {
            content: vec![AssistantContent::Text {
                text: String::from("recovered"),
                text_signature: None,
            }],
            api: api.clone(),
            provider: provider_name.clone(),
            model: model_id.clone(),
            response_id: None,
            usage: usage(10, 10),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 20,
        })
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn auto_compacts_on_threshold_and_emits_session_events() {
    let api = unique_name("threshold-api");
    let provider_name = unique_name("threshold-provider");
    let model_id = unique_name("threshold-model");
    let built_in_model = model(&api, &provider_name, &model_id, 100);
    let provider = ScriptedProvider::new(vec![
        assistant_message(
            &built_in_model,
            "large reply",
            StopReason::Stop,
            usage(95, 5),
            None,
            10,
        ),
        assistant_message(
            &built_in_model,
            "## Goal\nSummarize\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] threshold reached\n\n### In Progress\n- [ ] Continue\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Compaction**: Run automatically\n\n## Next Steps\n1. Continue\n\n## Critical Context\n- (none)",
            StopReason::Stop,
            usage(10, 10),
            None,
            20,
        ),
    ]);
    let provider_handle = provider.clone();
    let session = create_session(built_in_model.clone(), provider);
    session.set_compaction_settings(CompactionSettings {
        enabled: true,
        reserve_tokens: 10,
        keep_recent_tokens: 1,
    });

    let events = Arc::new(Mutex::new(Vec::<AgentSessionEvent>::new()));
    let events_clone = events.clone();
    let _unsubscribe = session.subscribe(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    session.prompt_text("trigger threshold").await.unwrap();

    assert_eq!(provider_handle.call_count(), 2);
    expect_first_message_role(&session, "compactionSummary");
    let events = events.lock().unwrap().clone();
    assert!(events.contains(&AgentSessionEvent::CompactionStart {
        reason: CompactionReason::Threshold,
    }));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentSessionEvent::CompactionEnd {
            reason: CompactionReason::Threshold,
            result: Some(_),
            aborted: false,
            will_retry: false,
            error_message: None,
        }
    )));

    unregister_provider(&api);
}

#[tokio::test]
async fn overflow_recovery_compacts_and_retries_once() {
    let api = unique_name("overflow-api");
    let provider_name = unique_name("overflow-provider");
    let model_id = unique_name("overflow-model");
    let built_in_model = model(&api, &provider_name, &model_id, 100);
    let provider = ScriptedProvider::new(vec![
        assistant_message(
            &built_in_model,
            "warmup",
            StopReason::Stop,
            usage(20, 10),
            None,
            10,
        ),
        assistant_message(
            &built_in_model,
            "",
            StopReason::Error,
            Usage::default(),
            Some("Input is too long for requested model"),
            20,
        ),
        assistant_message(
            &built_in_model,
            "## Goal\nRecover from overflow\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] Compacted the session\n\n### In Progress\n- [ ] Retry\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Overflow**: Compact before retrying\n\n## Next Steps\n1. Retry the request\n\n## Critical Context\n- (none)",
            StopReason::Stop,
            usage(10, 10),
            None,
            30,
        ),
        assistant_message(
            &built_in_model,
            "recovered after overflow",
            StopReason::Stop,
            usage(10, 10),
            None,
            40,
        ),
    ]);
    let provider_handle = provider.clone();
    let session = create_session(built_in_model.clone(), provider);
    session.set_compaction_settings(CompactionSettings {
        enabled: true,
        reserve_tokens: 10,
        keep_recent_tokens: 1,
    });

    let events = Arc::new(Mutex::new(Vec::<AgentSessionEvent>::new()));
    let events_clone = events.clone();
    let _unsubscribe = session.subscribe(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    session.prompt_text("warm up").await.unwrap();
    session.prompt_text("overflow please").await.unwrap();

    for _ in 0..50 {
        if provider_handle.call_count() >= 4 && !session.state().is_streaming {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(provider_handle.call_count(), 4);
    let contexts = provider_handle.contexts();
    assert_eq!(contexts.len(), 4);
    assert!(contexts[3]
        .messages
        .iter()
        .any(|message| matches!(message, pi_events::Message::User { content, .. } if content.iter().any(|block| matches!(block, UserContent::Text { text } if text.contains("The conversation history before this point was compacted"))))));

    let events = events.lock().unwrap().clone();
    assert!(events.contains(&AgentSessionEvent::CompactionStart {
        reason: CompactionReason::Overflow,
    }));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentSessionEvent::CompactionEnd {
            reason: CompactionReason::Overflow,
            result: Some(_),
            aborted: false,
            will_retry: true,
            error_message: None,
        }
    )));
    assert_eq!(
        session
            .state()
            .messages
            .last()
            .and_then(|message| message.as_standard_message())
            .and_then(|message| match message {
                pi_events::Message::Assistant { content, .. } => content.first(),
                _ => None,
            }),
        Some(&AssistantContent::Text {
            text: String::from("recovered after overflow"),
            text_signature: None,
        })
    );

    unregister_provider(&api);
}
