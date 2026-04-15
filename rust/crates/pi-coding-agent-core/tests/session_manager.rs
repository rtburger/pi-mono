use pi_coding_agent_core::{
    CURRENT_SESSION_VERSION, FileEntry, NewSessionOptions, SessionEntry, SessionManager,
    build_session_context, find_most_recent_session, load_entries_from_file,
};
use pi_events::{AssistantContent, Message, StopReason, Usage, UsageCost, UserContent};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pi-coding-agent-core-{prefix}-{timestamp}-{counter}"
    ));
    fs::create_dir_all(&path).unwrap();
    path
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

fn assistant_message(text: &str, timestamp: u64) -> pi_agent::AgentMessage {
    Message::Assistant {
        content: vec![AssistantContent::Text {
            text: text.to_owned(),
            text_signature: None,
        }],
        api: String::from("openai-responses"),
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

fn entry_message(
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
fn build_session_context_handles_compaction_and_branch_paths() {
    let entries = vec![
        entry_message("1", None, "2025-01-01T00:00:00Z", user_message("start", 1)),
        entry_message(
            "2",
            Some("1"),
            "2025-01-01T00:00:01Z",
            assistant_message("r1", 2),
        ),
        entry_message(
            "3",
            Some("2"),
            "2025-01-01T00:00:02Z",
            user_message("q2", 3),
        ),
        entry_message(
            "4",
            Some("3"),
            "2025-01-01T00:00:03Z",
            assistant_message("r2", 4),
        ),
        SessionEntry::Compaction {
            id: String::from("5"),
            parent_id: Some(String::from("4")),
            timestamp: String::from("2025-01-01T00:00:04Z"),
            summary: String::from("Compacted history"),
            first_kept_entry_id: String::from("3"),
            tokens_before: 1000,
            details: None,
            from_hook: None,
        },
        entry_message(
            "6",
            Some("5"),
            "2025-01-01T00:00:05Z",
            user_message("q3", 5),
        ),
        entry_message(
            "7",
            Some("6"),
            "2025-01-01T00:00:06Z",
            assistant_message("r3", 6),
        ),
        SessionEntry::BranchSummary {
            id: String::from("8"),
            parent_id: Some(String::from("3")),
            timestamp: String::from("2025-01-01T00:00:07Z"),
            from_id: String::from("4"),
            summary: String::from("Tried wrong approach"),
            details: None,
            from_hook: None,
        },
        entry_message(
            "9",
            Some("8"),
            "2025-01-01T00:00:08Z",
            user_message("better path", 7),
        ),
    ];

    let main = build_session_context(&entries, Some("7"));
    assert_eq!(main.messages.len(), 5);
    match &main.messages[0] {
        pi_agent::AgentMessage::Custom(message) => {
            assert_eq!(message.role, "compactionSummary");
        }
        other => panic!("expected compaction summary, got {other:?}"),
    }
    match &main.messages[1] {
        pi_agent::AgentMessage::Standard(Message::User { content, .. }) => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("q2")
                }]
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }
    assert_eq!(
        main.model.as_ref().map(|model| model.provider.as_str()),
        Some("openai")
    );
    assert_eq!(
        main.model.as_ref().map(|model| model.model_id.as_str()),
        Some("gpt-test")
    );

    let branch = build_session_context(&entries, Some("9"));
    assert_eq!(branch.messages.len(), 5);
    match &branch.messages[3] {
        pi_agent::AgentMessage::Custom(message) => {
            assert_eq!(message.role, "branchSummary");
        }
        other => panic!("expected branch summary, got {other:?}"),
    }
    match &branch.messages[4] {
        pi_agent::AgentMessage::Standard(Message::User { content, .. }) => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("better path")
                }]
            );
        }
        other => panic!("expected final user message, got {other:?}"),
    }
}

#[test]
fn session_manager_tracks_tree_and_labels() {
    let mut session = SessionManager::in_memory("/tmp/project");
    let id1 = session.append_message(user_message("first", 1)).unwrap();
    let id2 = session
        .append_message(assistant_message("second", 2))
        .unwrap();
    let id3 = session.append_message(user_message("third", 3)).unwrap();
    session.branch(&id2).unwrap();
    let branch_id = session.append_message(user_message("branch", 4)).unwrap();
    let _ = session
        .append_label_change(&id1, Some(String::from("start")))
        .unwrap();
    let _ = session
        .append_label_change(&branch_id, Some(String::from("branch-point")))
        .unwrap();

    let tree = session.get_tree();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].entry.id(), id1);
    assert_eq!(tree[0].label.as_deref(), Some("start"));

    let node2 = &tree[0].children[0];
    assert_eq!(node2.entry.id(), id2);
    assert_eq!(node2.children.len(), 2);
    let child_ids = node2
        .children
        .iter()
        .map(|child| child.entry.id().to_owned())
        .collect::<Vec<_>>();
    assert!(child_ids.contains(&id3));
    assert!(child_ids.contains(&branch_id));
    let branch_node = node2
        .children
        .iter()
        .find(|child| child.entry.id() == branch_id)
        .unwrap();
    assert_eq!(branch_node.label.as_deref(), Some("branch-point"));

    let context = session.build_session_context();
    assert_eq!(context.messages.len(), 3);
    match &context.messages[2] {
        pi_agent::AgentMessage::Standard(Message::User { content, .. }) => {
            assert_eq!(
                content,
                &vec![UserContent::Text {
                    text: String::from("branch")
                }]
            );
        }
        other => panic!("expected branch user message, got {other:?}"),
    }
}

#[test]
fn create_branched_session_preserves_only_labels_on_path() {
    let mut session = SessionManager::in_memory("/tmp/project");
    let id1 = session.append_message(user_message("first", 1)).unwrap();
    let id2 = session
        .append_message(assistant_message("second", 2))
        .unwrap();
    let id3 = session.append_message(user_message("third", 3)).unwrap();
    session
        .append_label_change(&id1, Some(String::from("first-label")))
        .unwrap();
    session
        .append_label_change(&id2, Some(String::from("second-label")))
        .unwrap();
    session
        .append_label_change(&id3, Some(String::from("third-label")))
        .unwrap();

    let result = session.create_branched_session(&id2).unwrap();
    assert!(result.is_none());
    assert_eq!(session.get_entries().len(), 4);
    assert_eq!(session.get_label(&id1), Some("first-label"));
    assert_eq!(session.get_label(&id2), Some("second-label"));
    assert_eq!(session.get_label(&id3), None);
}

#[test]
fn load_entries_and_find_most_recent_session_ignore_invalid_files() {
    let temp_dir = unique_temp_dir("session-load");
    let invalid_path = temp_dir.join("invalid.jsonl");
    let older_path = temp_dir.join("older.jsonl");
    let newer_path = temp_dir.join("newer.jsonl");

    fs::write(&invalid_path, "{\"type\":\"message\"}\n").unwrap();
    fs::write(
        &older_path,
        "{\"type\":\"session\",\"id\":\"older\",\"timestamp\":\"2025-01-01T00:00:00Z\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();
    thread::sleep(Duration::from_millis(10));
    fs::write(
        &newer_path,
        "{\"type\":\"session\",\"id\":\"newer\",\"timestamp\":\"2025-01-01T00:00:00Z\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();

    let invalid_entries = load_entries_from_file(&invalid_path);
    assert!(invalid_entries.is_empty());

    let newer_entries = load_entries_from_file(&newer_path);
    assert_eq!(newer_entries.len(), 1);
    assert!(matches!(newer_entries.first(), Some(FileEntry::Session(_))));
    assert_eq!(
        find_most_recent_session(&temp_dir).as_deref(),
        Some(newer_path.to_string_lossy().as_ref())
    );

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn open_recovers_corrupted_file_and_preserves_explicit_path() {
    let temp_dir = unique_temp_dir("session-recover");
    let file_path = temp_dir.join("corrupted.jsonl");
    fs::write(&file_path, "garbage\n").unwrap();

    let session = SessionManager::open(
        file_path.to_string_lossy().as_ref(),
        Some(temp_dir.to_string_lossy().as_ref()),
        None,
    )
    .unwrap();

    assert_eq!(
        session.get_session_file(),
        Some(file_path.to_string_lossy().as_ref())
    );
    let content = fs::read_to_string(&file_path).unwrap();
    let lines = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let header: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(
        header.get("type").and_then(|value| value.as_str()),
        Some("session")
    );

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn persisted_sessions_defer_file_creation_until_assistant_message() {
    let temp_dir = unique_temp_dir("session-persist");
    let mut session = SessionManager::create(
        temp_dir.to_string_lossy().as_ref(),
        Some(temp_dir.to_string_lossy().as_ref()),
    )
    .unwrap();
    let session_file = PathBuf::from(session.get_session_file().unwrap());

    session.append_message(user_message("hello", 1)).unwrap();
    assert!(!session_file.exists());

    session
        .append_custom_entry("state", Some(json!({ "ok": true })))
        .unwrap();
    assert!(!session_file.exists());

    session.append_message(assistant_message("hi", 2)).unwrap();
    assert!(session_file.exists());
    let content = fs::read_to_string(&session_file).unwrap();
    let lines = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 4);

    let records = lines
        .into_iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.get("type").and_then(|value| value.as_str()) == Some("session"))
            .count(),
        1
    );

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn open_migrates_v1_session_files_to_current_format() {
    let temp_dir = unique_temp_dir("session-migrate");
    let file_path = temp_dir.join("v1.jsonl");
    fs::write(
        &file_path,
        concat!(
            "{\"type\":\"session\",\"id\":\"sess-1\",\"timestamp\":\"2025-01-01T00:00:00Z\",\"cwd\":\"/tmp\"}\n",
            "{\"type\":\"message\",\"timestamp\":\"2025-01-01T00:00:01Z\",\"message\":{\"role\":\"user\",\"content\":\"hi\",\"timestamp\":1}}\n",
            "{\"type\":\"message\",\"timestamp\":\"2025-01-01T00:00:02Z\",\"message\":{\"role\":\"hookMessage\",\"customType\":\"x\",\"content\":\"hello\",\"display\":true,\"timestamp\":2}}\n"
        ),
    )
    .unwrap();

    let session = SessionManager::open(
        file_path.to_string_lossy().as_ref(),
        Some(temp_dir.to_string_lossy().as_ref()),
        None,
    )
    .unwrap();
    assert_eq!(session.get_entries().len(), 2);

    let rewritten = fs::read_to_string(&file_path).unwrap();
    let rewritten_entries = load_entries_from_file(&file_path);
    match rewritten_entries.first() {
        Some(FileEntry::Session(header)) => {
            assert_eq!(header.version, Some(CURRENT_SESSION_VERSION));
        }
        other => panic!("expected session header, got {other:?}"),
    }
    match &session.get_entries()[1] {
        SessionEntry::Message { message, .. } => match message {
            pi_agent::AgentMessage::Custom(message) => {
                assert_eq!(message.role, "custom");
            }
            other => panic!("expected migrated custom message, got {other:?}"),
        },
        other => panic!("expected message entry, got {other:?}"),
    }
    assert!(rewritten.contains("\"version\":3"));

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn list_uses_last_message_timestamp_for_modified_and_reads_session_name() {
    let temp_dir = unique_temp_dir("session-list");
    let file_path = temp_dir.join("session.jsonl");
    let mut session = SessionManager::open(
        file_path.to_string_lossy().as_ref(),
        Some(temp_dir.to_string_lossy().as_ref()),
        None,
    )
    .unwrap();
    session
        .append_message(assistant_message("hi", 100))
        .unwrap();
    session.append_session_info("named session").unwrap();
    session
        .append_message(assistant_message("later", 500))
        .unwrap();

    let sessions = SessionManager::list("/tmp", Some(temp_dir.to_string_lossy().as_ref()));
    assert_eq!(sessions.len(), 1);
    let info = &sessions[0];
    assert_eq!(info.name.as_deref(), Some("named session"));
    assert_eq!(
        info.modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        500
    );
    assert_eq!(info.first_message, "(no messages)");
    assert_eq!(info.message_count, 2);

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn new_session_uses_custom_id_when_provided() {
    let mut session = SessionManager::in_memory("/tmp/project");
    session.new_session(NewSessionOptions {
        id: Some(String::from("custom-id")),
        parent_session: None,
    });

    assert_eq!(session.get_session_id(), "custom-id");
    assert_eq!(session.get_header().id, "custom-id");
}
