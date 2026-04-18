use pi_coding_agent_core::{
    SessionEntry, SessionManager, TreeNavigationSummary, apply_tree_navigation,
    build_session_context, prepare_tree_navigation,
};
use pi_events::{AssistantContent, Message, StopReason, Usage, UsageCost, UserContent};
use serde_json::json;

fn user_message(text: &str, timestamp: u64) -> pi_agent::AgentMessage {
    Message::User {
        content: vec![UserContent::Text {
            text: text.to_owned(),
        }],
        timestamp,
    }
    .into()
}

fn assistant_message(text: &str, timestamp: u64) -> pi_agent::AgentMessage {
    Message::Assistant {
        content: vec![AssistantContent::Text {
            text: text.to_owned(),
            text_signature: None,
        }],
        api: String::from("faux:test"),
        provider: String::from("openai"),
        model: String::from("gpt-test"),
        response_id: None,
        usage: Usage {
            input: 1,
            output: 1,
            cache_read: 0,
            cache_write: 0,
            total_tokens: 2,
            cost: UsageCost::default(),
        },
        stop_reason: StopReason::Stop,
        error_message: None,
        timestamp,
    }
    .into()
}

#[test]
fn prepare_tree_navigation_rewinds_to_parent_for_user_targets() {
    let mut session = SessionManager::in_memory("/tmp/tree-navigation-user");
    let u1 = session.append_message(user_message("first", 1)).unwrap();
    let a1 = session.append_message(assistant_message("one", 2)).unwrap();
    let u2 = session.append_message(user_message("second", 3)).unwrap();
    let a2 = session.append_message(assistant_message("two", 4)).unwrap();

    let preparation = prepare_tree_navigation(&session, Some(&u2)).unwrap();

    assert_eq!(preparation.old_leaf_id.as_deref(), Some(a2.as_str()));
    assert_eq!(preparation.target_id.as_deref(), Some(u2.as_str()));
    assert_eq!(preparation.common_ancestor_id.as_deref(), Some(u2.as_str()));
    assert_eq!(preparation.new_leaf_id.as_deref(), Some(a1.as_str()));
    assert_eq!(preparation.editor_text.as_deref(), Some("second"));
    assert_eq!(preparation.entries_to_summarize.len(), 1);
    assert_eq!(preparation.entries_to_summarize[0].id(), a2.as_str());

    let result = apply_tree_navigation(&mut session, &preparation, None, None).unwrap();
    assert_eq!(result.old_leaf_id.as_deref(), Some(a2.as_str()));
    assert_eq!(result.new_leaf_id.as_deref(), Some(a1.as_str()));
    assert_eq!(result.editor_text.as_deref(), Some("second"));
    assert!(result.summary_entry_id.is_none());
    assert_eq!(session.get_leaf_id(), Some(a1.as_str()));

    let context = session.build_session_context();
    assert_eq!(context.messages.len(), 2);
    assert!(matches!(
        context.messages[0].as_standard_message(),
        Some(Message::User { .. })
    ));
    assert!(matches!(
        context.messages[1].as_standard_message(),
        Some(Message::Assistant { .. })
    ));

    let _ = u1;
}

#[test]
fn apply_tree_navigation_attaches_summary_and_label_to_selected_position() {
    let mut session = SessionManager::in_memory("/tmp/tree-navigation-summary");
    let _u1 = session.append_message(user_message("first", 1)).unwrap();
    let a1 = session.append_message(assistant_message("one", 2)).unwrap();
    let _u2 = session.append_message(user_message("second", 3)).unwrap();
    let _a2 = session.append_message(assistant_message("two", 4)).unwrap();

    let preparation = prepare_tree_navigation(&session, Some(&a1)).unwrap();
    assert_eq!(preparation.new_leaf_id.as_deref(), Some(a1.as_str()));
    assert!(preparation.editor_text.is_none());
    assert_eq!(preparation.entries_to_summarize.len(), 2);

    let result = apply_tree_navigation(
        &mut session,
        &preparation,
        Some(TreeNavigationSummary {
            summary: String::from("Branch summary"),
            details: Some(json!({ "readFiles": ["README.md"] })),
            from_hook: None,
        }),
        Some("return-here"),
    )
    .unwrap();

    let summary_id = result
        .summary_entry_id
        .clone()
        .expect("expected summary entry id");
    assert_eq!(session.get_label(&summary_id), Some("return-here"));
    assert_eq!(result.new_leaf_id.as_deref(), session.get_leaf_id());
    assert_ne!(result.new_leaf_id.as_deref(), Some(summary_id.as_str()));

    match session.get_entry(&summary_id) {
        Some(SessionEntry::BranchSummary {
            parent_id, summary, ..
        }) => {
            assert_eq!(parent_id.as_deref(), Some(a1.as_str()));
            assert_eq!(summary, "Branch summary");
        }
        other => panic!("expected branch summary entry, got {other:?}"),
    }

    let context = build_session_context(session.get_entries(), session.get_leaf_id());
    assert!(
        context
            .messages
            .iter()
            .any(|message| message.role() == "branchSummary")
    );
}

#[test]
fn prepare_tree_navigation_to_root_collects_full_current_branch() {
    let mut session = SessionManager::in_memory("/tmp/tree-navigation-root");
    let u1 = session.append_message(user_message("first", 1)).unwrap();
    let a1 = session.append_message(assistant_message("one", 2)).unwrap();
    let _u2 = session.append_message(user_message("second", 3)).unwrap();

    let preparation = prepare_tree_navigation(&session, None).unwrap();
    assert_eq!(preparation.old_leaf_id.as_deref(), session.get_leaf_id());
    assert!(preparation.target_id.is_none());
    assert!(preparation.common_ancestor_id.is_none());
    assert!(preparation.editor_text.is_none());
    assert!(preparation.new_leaf_id.is_none());
    assert_eq!(preparation.entries_to_summarize.len(), 3);
    assert_eq!(preparation.entries_to_summarize[0].id(), u1.as_str());
    assert_eq!(preparation.entries_to_summarize[1].id(), a1.as_str());

    let result = apply_tree_navigation(
        &mut session,
        &preparation,
        Some(TreeNavigationSummary {
            summary: String::from("Left this branch"),
            details: None,
            from_hook: None,
        }),
        None,
    )
    .unwrap();

    let summary_id = result.summary_entry_id.expect("expected summary entry id");
    match session.get_entry(&summary_id) {
        Some(SessionEntry::BranchSummary { parent_id, .. }) => {
            assert!(parent_id.is_none());
        }
        other => panic!("expected branch summary entry, got {other:?}"),
    }
    assert_eq!(session.get_leaf_id(), Some(summary_id.as_str()));
}
