use async_stream::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_core::{
    AgentSession, AgentSessionOptions, BranchSummaryDetails, CodingAgentCoreOptions,
    CompactionSettings, MemoryAuthStorage, NavigateTreeOptions, SessionBootstrapOptions,
    SessionManager, create_agent_session,
};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
    UsageCost, UserContent,
};
use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
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
        cost: UsageCost::default(),
    }
}

fn user_message(text: &str, timestamp: u64) -> pi_agent::AgentMessage {
    Message::User {
        content: vec![UserContent::Text {
            text: text.to_owned(),
        }],
        timestamp,
    }
    .into()
}

fn assistant_message(
    model: &Model,
    content: Vec<AssistantContent>,
    stop_reason: StopReason,
    usage: Usage,
    timestamp: u64,
) -> AssistantMessage {
    AssistantMessage {
        role: String::from("assistant"),
        content,
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        response_id: None,
        usage,
        stop_reason,
        error_message: None,
        timestamp,
    }
}

#[derive(Clone)]
struct ScriptedProvider {
    replies: Arc<Mutex<VecDeque<AssistantMessage>>>,
    call_count: Arc<Mutex<usize>>,
}

impl ScriptedProvider {
    fn new(replies: Vec<AssistantMessage>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl AiProvider for ScriptedProvider {
    fn stream(
        &self,
        _model: Model,
        _context: Context,
        _options: StreamOptions,
    ) -> AssistantEventStream {
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("expected scripted reply");
        *self.call_count.lock().unwrap() += 1;
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
    session_manager: Arc<Mutex<SessionManager>>,
) -> AgentSession {
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
            tools: Some(Vec::new()),
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager: Some(session_manager),
    })
    .unwrap()
    .session
}

#[tokio::test]
async fn session_exposes_history_stats_export_and_last_assistant_text() {
    let api = unique_name("session-history-api");
    let provider_name = unique_name("session-history-provider");
    let model_id = unique_name("session-history-model");
    let built_in_model = model(&api, &provider_name, &model_id, 128_000);
    let provider = ScriptedProvider::new(vec![assistant_message(
        &built_in_model,
        vec![AssistantContent::Text {
            text: String::from("answer from history api"),
            text_signature: None,
        }],
        StopReason::Stop,
        usage(42, 8),
        10,
    )]);
    let session_manager = Arc::new(Mutex::new(SessionManager::in_memory(
        "/tmp/session-history",
    )));
    let session = create_session(built_in_model.clone(), provider, session_manager.clone());

    session.prompt_text("hello history").await.unwrap();
    session.set_session_name("named session").unwrap();

    let stats = session.session_stats();
    assert_eq!(stats.session_id, session.session_id());
    assert_eq!(stats.user_messages, 1);
    assert_eq!(stats.assistant_messages, 1);
    assert_eq!(stats.tool_calls, 0);
    assert_eq!(stats.tool_results, 0);
    assert_eq!(stats.total_messages, 2);
    assert_eq!(stats.tokens.input, 42);
    assert_eq!(stats.tokens.output, 8);
    assert_eq!(stats.tokens.total, 50);
    assert_eq!(
        session.last_assistant_text().as_deref(),
        Some("answer from history api")
    );

    let context_usage = session.context_usage().expect("expected context usage");
    assert_eq!(context_usage.context_window, built_in_model.context_window);
    assert_eq!(context_usage.tokens, Some(50));
    assert_eq!(stats.context_usage, Some(context_usage.clone()));

    let fork_messages = session.user_messages_for_forking();
    assert_eq!(fork_messages.len(), 1);
    assert_eq!(fork_messages[0].text, "hello history");

    assert_eq!(
        session_manager
            .lock()
            .unwrap()
            .get_session_name()
            .as_deref(),
        Some("named session")
    );

    let output_path = unique_temp_dir("session-export")
        .join("branch.jsonl")
        .to_string_lossy()
        .into_owned();
    let exported = session.export_to_jsonl(&output_path).unwrap();
    assert_eq!(exported, output_path);
    let jsonl = fs::read_to_string(&output_path).unwrap();
    assert!(jsonl.contains("\"type\":\"session\""), "jsonl: {jsonl}");
    assert!(jsonl.contains("hello history"), "jsonl: {jsonl}");
    assert!(jsonl.contains("named session"), "jsonl: {jsonl}");

    unregister_provider(&api);
}

#[tokio::test]
async fn navigate_tree_generates_branch_summary_and_restores_editor_text() {
    let api = unique_name("navigate-tree-api");
    let provider_name = unique_name("navigate-tree-provider");
    let model_id = unique_name("navigate-tree-model");
    let built_in_model = model(&api, &provider_name, &model_id, 128_000);
    let provider = ScriptedProvider::new(vec![assistant_message(
        &built_in_model,
        vec![AssistantContent::Text {
            text: String::from(
                "## Goal\nReturn later\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] Explored alternate branch\n\n### In Progress\n- [ ] Resume from earlier point\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Branch**: Summarize before rewinding\n\n## Next Steps\n1. Continue from the selected message\n\n## Critical Context\n- kept",
            ),
            text_signature: None,
        }],
        StopReason::Stop,
        usage(10, 10),
        50,
    )]);

    let session_manager = Arc::new(Mutex::new(SessionManager::in_memory("/tmp/navigate-tree")));
    let (user_id, assistant_id, rewind_user_id) = {
        let mut session_manager = session_manager.lock().unwrap();
        let user_id = session_manager
            .append_message(user_message("root message", 1))
            .unwrap();
        let assistant_id = session_manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("root answer"),
                    text_signature: None,
                }],
                api: built_in_model.api.clone(),
                provider: built_in_model.provider.clone(),
                model: built_in_model.id.clone(),
                response_id: None,
                usage: usage(12, 4),
                stop_reason: StopReason::Stop,
                error_message: None,
                timestamp: 2,
            })
            .unwrap();
        let rewind_user_id = session_manager
            .append_message(user_message("message to edit again", 3))
            .unwrap();
        session_manager
            .append_message(Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("alternate branch answer"),
                    text_signature: None,
                }],
                api: built_in_model.api.clone(),
                provider: built_in_model.provider.clone(),
                model: built_in_model.id.clone(),
                response_id: None,
                usage: usage(20, 5),
                stop_reason: StopReason::Stop,
                error_message: None,
                timestamp: 4,
            })
            .unwrap();
        session_manager.append_thinking_level_change("off").unwrap();
        (user_id, assistant_id, rewind_user_id)
    };

    let session = create_session(built_in_model, provider.clone(), session_manager.clone());
    let navigation = session
        .navigate_tree(
            Some(&rewind_user_id),
            NavigateTreeOptions {
                summarize: true,
                custom_instructions: None,
                replace_instructions: false,
                reserve_tokens: None,
                label: Some(String::from("bookmark")),
            },
        )
        .await
        .unwrap();

    assert_eq!(provider.call_count(), 1);
    assert_eq!(
        navigation.editor_text.as_deref(),
        Some("message to edit again")
    );
    let summary_id = navigation
        .summary_entry_id
        .clone()
        .expect("expected summary entry");
    assert!(navigation.old_leaf_id.is_some());

    let manager = session_manager.lock().unwrap();
    assert_eq!(navigation.new_leaf_id.as_deref(), manager.get_leaf_id());
    assert_eq!(manager.get_label(&summary_id), Some("bookmark"));
    assert_ne!(manager.get_leaf_id(), Some(summary_id.as_str()));
    match manager.get_entry(&summary_id) {
        Some(pi_coding_agent_core::SessionEntry::BranchSummary {
            parent_id, details, ..
        }) => {
            assert_eq!(parent_id.as_deref(), Some(assistant_id.as_str()));
            let parsed: BranchSummaryDetails =
                serde_json::from_value(details.clone().expect("expected branch summary details"))
                    .unwrap();
            assert!(parsed.read_files.is_empty());
            assert!(parsed.modified_files.is_empty());
        }
        other => panic!("expected branch summary entry, got {other:?}"),
    }
    drop(manager);

    let roles = session
        .state()
        .messages
        .iter()
        .map(|message| message.role().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["user", "assistant", "branchSummary"]);
    assert_eq!(
        session.last_assistant_text().as_deref(),
        Some("root answer")
    );

    let fork_messages = session.user_messages_for_forking();
    assert_eq!(fork_messages.len(), 2);
    assert!(
        fork_messages
            .iter()
            .any(|message| message.entry_id == user_id)
    );
    assert!(
        fork_messages
            .iter()
            .any(|message| message.entry_id == rewind_user_id)
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn context_usage_is_unknown_immediately_after_compaction() {
    let api = unique_name("context-usage-api");
    let provider_name = unique_name("context-usage-provider");
    let model_id = unique_name("context-usage-model");
    let built_in_model = model(&api, &provider_name, &model_id, 128_000);
    let provider = ScriptedProvider::new(vec![
        assistant_message(
            &built_in_model,
            vec![AssistantContent::Text {
                text: String::from("pre-compaction answer"),
                text_signature: None,
            }],
            StopReason::Stop,
            usage(80, 20),
            10,
        ),
        assistant_message(
            &built_in_model,
            vec![AssistantContent::Text {
                text: String::from(
                    "## Goal\nCompact\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] Captured history\n\n### In Progress\n- [ ] Continue\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Compaction**: Keep the recent branch\n\n## Next Steps\n1. Continue from the compacted state\n\n## Critical Context\n- (none)",
                ),
                text_signature: None,
            }],
            StopReason::Stop,
            usage(10, 10),
            20,
        ),
    ]);
    let session_manager = Arc::new(Mutex::new(SessionManager::in_memory("/tmp/context-usage")));
    let session = create_session(built_in_model, provider, session_manager);
    session.set_compaction_settings(CompactionSettings {
        enabled: true,
        reserve_tokens: 16_384,
        keep_recent_tokens: 1,
    });

    session.prompt_text("compact me").await.unwrap();
    session.compact(None).await.unwrap();

    let usage = session.context_usage().expect("expected context usage");
    assert_eq!(usage.tokens, None);
    assert_eq!(usage.percent, None);
    assert_eq!(session.session_stats().context_usage, Some(usage));

    unregister_provider(&api);
}
