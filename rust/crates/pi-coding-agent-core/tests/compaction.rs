use pi_ai::{
    FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions, register_faux_provider,
};
use pi_coding_agent_core::{
    BranchSummaryOptions, CompactionSettings, SessionEntry, build_session_context,
    collect_entries_for_branch_summary, compact, estimate_context_tokens, generate_branch_summary,
    prepare_compaction,
};
use pi_events::{AssistantContent, Message, StopReason, Usage, UserContent};

fn user_message(text: &str, timestamp: u64) -> pi_agent::AgentMessage {
    Message::User {
        content: vec![UserContent::Text {
            text: text.to_owned(),
        }],
        timestamp,
    }
    .into()
}

fn assistant_message(text: &str, timestamp: u64, input_tokens: u64) -> pi_agent::AgentMessage {
    Message::Assistant {
        content: vec![AssistantContent::Text {
            text: text.to_owned(),
            text_signature: None,
        }],
        api: String::from("faux:test"),
        provider: String::from("compaction-faux"),
        model: String::from("compaction-faux-1"),
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
        timestamp,
    }
    .into()
}

fn message_entry(
    id: &str,
    parent_id: Option<&str>,
    timestamp: &str,
    message: pi_agent::AgentMessage,
) -> SessionEntry {
    SessionEntry::Message {
        id: id.to_owned(),
        parent_id: parent_id.map(ToOwned::to_owned),
        timestamp: timestamp.to_owned(),
        message,
    }
}

#[test]
fn prepare_compaction_keeps_recent_messages_and_carries_previous_summary() {
    let entries = vec![
        message_entry("1", None, "2025-01-01T00:00:00Z", user_message("start", 1)),
        message_entry(
            "2",
            Some("1"),
            "2025-01-01T00:00:01Z",
            assistant_message("a", 2, 5_000),
        ),
        message_entry(
            "3",
            Some("2"),
            "2025-01-01T00:00:02Z",
            user_message("middle", 3),
        ),
        message_entry(
            "4",
            Some("3"),
            "2025-01-01T00:00:03Z",
            assistant_message("b", 4, 12_000),
        ),
        SessionEntry::Compaction {
            id: String::from("5"),
            parent_id: Some(String::from("4")),
            timestamp: String::from("2025-01-01T00:00:04Z"),
            summary: String::from("Older summary"),
            first_kept_entry_id: String::from("3"),
            tokens_before: 10_000,
            details: None,
            from_hook: None,
        },
        message_entry(
            "6",
            Some("5"),
            "2025-01-01T00:00:05Z",
            user_message("recent", 5),
        ),
        message_entry(
            "7",
            Some("6"),
            "2025-01-01T00:00:06Z",
            assistant_message("c", 6, 25_000),
        ),
    ];

    let preparation = prepare_compaction(
        &entries,
        CompactionSettings {
            enabled: true,
            reserve_tokens: 16_384,
            keep_recent_tokens: 4,
        },
    )
    .expect("expected compaction preparation");

    assert_eq!(preparation.first_kept_entry_id, "6");
    assert_eq!(
        preparation.previous_summary.as_deref(),
        Some("Older summary")
    );
    assert_eq!(preparation.messages_to_summarize.len(), 2);
    assert!(preparation.turn_prefix_messages.is_empty());
    assert!(preparation.tokens_before > 0);
}

#[test]
fn estimate_context_tokens_uses_last_assistant_usage_plus_trailing_messages() {
    let messages = vec![
        user_message("hello", 1),
        assistant_message("world", 2, 200),
        user_message("follow up", 3),
    ];

    let estimate = estimate_context_tokens(&messages);
    assert_eq!(estimate.usage_tokens, 210);
    assert_eq!(estimate.last_usage_index, Some(1));
    assert!(estimate.trailing_tokens > 0);
    assert_eq!(
        estimate.tokens,
        estimate.usage_tokens + estimate.trailing_tokens
    );
}

#[tokio::test]
async fn compact_generates_summary_and_builds_compaction_context() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "compaction-faux".into(),
        models: vec![FauxModelDefinition {
            id: "compaction-faux-1".into(),
            name: Some("Compaction Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text(
        "## Goal\nCompact the session\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] Captured the earlier turns\n\n### In Progress\n- [ ] Keep working\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Compaction**: Keep only the recent branch\n\n## Next Steps\n1. Continue from the preserved context\n\n## Critical Context\n- (none)",
    )]);
    let model = faux
        .get_model(Some("compaction-faux-1"))
        .expect("expected faux model");

    let entries = vec![
        message_entry("1", None, "2025-01-01T00:00:00Z", user_message("one", 1)),
        message_entry(
            "2",
            Some("1"),
            "2025-01-01T00:00:01Z",
            assistant_message("two", 2, 40_000),
        ),
        message_entry(
            "3",
            Some("2"),
            "2025-01-01T00:00:02Z",
            user_message("three", 3),
        ),
        message_entry(
            "4",
            Some("3"),
            "2025-01-01T00:00:03Z",
            assistant_message("four", 4, 50_000),
        ),
    ];

    let preparation = prepare_compaction(
        &entries,
        CompactionSettings {
            enabled: true,
            reserve_tokens: 16_384,
            keep_recent_tokens: 4,
        },
    )
    .expect("expected compaction preparation");
    let result = compact(&preparation, &model, "token", None, None)
        .await
        .expect("expected compaction result");

    let compacted_entries = [
        entries.clone(),
        vec![SessionEntry::Compaction {
            id: String::from("5"),
            parent_id: Some(String::from("4")),
            timestamp: String::from("2025-01-01T00:00:04Z"),
            summary: result.summary.clone(),
            first_kept_entry_id: result.first_kept_entry_id.clone(),
            tokens_before: result.tokens_before,
            details: result.details.clone(),
            from_hook: None,
        }],
    ]
    .concat();
    let context = build_session_context(&compacted_entries, Some("5"));

    assert!(result.summary.contains("## Goal"));
    assert!(
        context
            .messages
            .iter()
            .any(|message| message.role() == "compactionSummary")
    );

    faux.unregister();
}

#[tokio::test]
async fn branch_summary_collects_abandoned_entries_and_generates_summary() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "compaction-faux".into(),
        models: vec![FauxModelDefinition {
            id: "compaction-faux-1".into(),
            name: Some("Compaction Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text(
        "## Goal\nTry an alternate branch\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] Investigated the alternate path\n\n### In Progress\n- [ ] Return to the main branch\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Branch**: Return to the original task\n\n## Next Steps\n1. Continue from the selected entry",
    )]);
    let model = faux
        .get_model(Some("compaction-faux-1"))
        .expect("expected faux model");

    let mut session = pi_coding_agent_core::SessionManager::in_memory("/tmp/branch-summary");
    let _root = session.append_message(user_message("root", 1)).unwrap();
    let reply = session
        .append_message(assistant_message("reply", 2, 1_000))
        .unwrap();
    let main = session
        .append_message(user_message("main branch", 3))
        .unwrap();
    session.branch(&reply).unwrap();
    let alt = session
        .append_message(user_message("alternate branch", 4))
        .unwrap();
    let _alt_reply = session
        .append_message(assistant_message("alt reply", 5, 1_000))
        .unwrap();

    let collected = collect_entries_for_branch_summary(&session, session.get_leaf_id(), &main);
    assert_eq!(
        collected.common_ancestor_id.as_deref(),
        Some(reply.as_str())
    );
    assert_eq!(collected.entries.len(), 2);
    assert_eq!(collected.entries[0].id(), alt.as_str());

    let summary = generate_branch_summary(
        &collected.entries,
        &model,
        "token",
        None,
        BranchSummaryOptions::default(),
    )
    .await
    .expect("expected branch summary");

    assert!(summary.starts_with("The user explored a different conversation branch"));
    assert!(summary.contains("## Goal"));

    faux.unregister();
}
