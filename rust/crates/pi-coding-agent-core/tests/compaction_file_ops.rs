use pi_ai::{
    FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions, register_faux_provider,
};
use pi_coding_agent_core::{
    BranchSummaryDetails, BranchSummaryOptions, CompactionDetails, CompactionSettings,
    GeneratedBranchSummary, SessionEntry, compact, generate_branch_summary_with_details,
    prepare_compaction,
};
use pi_events::{AssistantContent, Message, StopReason, Usage, UserContent};
use serde_json::json;
use std::collections::BTreeMap;

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
        provider: String::from("compaction-files-faux"),
        model: String::from("compaction-files-faux-1"),
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

fn assistant_with_tool_calls(timestamp: u64) -> pi_agent::AgentMessage {
    Message::Assistant {
        content: vec![
            AssistantContent::Text {
                text: String::from("working"),
                text_signature: None,
            },
            AssistantContent::ToolCall {
                id: String::from("readme"),
                name: String::from("read"),
                arguments: BTreeMap::from([(String::from("path"), json!("README.md"))]),
                thought_signature: None,
            },
            AssistantContent::ToolCall {
                id: String::from("lib-read"),
                name: String::from("read"),
                arguments: BTreeMap::from([(String::from("path"), json!("src/lib.rs"))]),
                thought_signature: None,
            },
            AssistantContent::ToolCall {
                id: String::from("main-write"),
                name: String::from("write"),
                arguments: BTreeMap::from([(String::from("path"), json!("src/main.rs"))]),
                thought_signature: None,
            },
            AssistantContent::ToolCall {
                id: String::from("lib-edit"),
                name: String::from("edit"),
                arguments: BTreeMap::from([(String::from("path"), json!("src/lib.rs"))]),
                thought_signature: None,
            },
        ],
        api: String::from("faux:test"),
        provider: String::from("compaction-files-faux"),
        model: String::from("compaction-files-faux-1"),
        response_id: None,
        usage: Usage::default(),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp,
    }
    .into()
}

fn tool_result_message(timestamp: u64) -> pi_agent::AgentMessage {
    Message::ToolResult {
        tool_call_id: String::from("result-1"),
        tool_name: String::from("read"),
        content: vec![UserContent::Text {
            text: String::from("tool output"),
        }],
        details: Some(serde_json::Value::Null),
        is_error: false,
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

#[tokio::test]
async fn compact_includes_file_lists_in_summary_and_details() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "compaction-files-faux".into(),
        models: vec![FauxModelDefinition {
            id: "compaction-files-faux-1".into(),
            name: Some("Compaction Files Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text(
        "## Goal\nCompact\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] captured\n\n### In Progress\n- [ ] continue\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Compaction**: keep recent context\n\n## Next Steps\n1. continue\n\n## Critical Context\n- (none)",
    )]);
    let model = faux
        .get_model(Some("compaction-files-faux-1"))
        .expect("expected faux model");

    let entries = vec![
        message_entry("1", None, "2025-01-01T00:00:00Z", user_message("start", 1)),
        message_entry(
            "2",
            Some("1"),
            "2025-01-01T00:00:01Z",
            assistant_with_tool_calls(2),
        ),
        message_entry(
            "3",
            Some("2"),
            "2025-01-01T00:00:02Z",
            user_message("recent", 3),
        ),
        message_entry(
            "4",
            Some("3"),
            "2025-01-01T00:00:03Z",
            assistant_message("done", 4, 80),
        ),
    ];

    let preparation = prepare_compaction(
        &entries,
        CompactionSettings {
            enabled: true,
            reserve_tokens: 16_384,
            keep_recent_tokens: 2,
        },
    )
    .expect("expected compaction preparation");

    assert_eq!(preparation.read_files, vec![String::from("README.md")]);
    assert_eq!(
        preparation.modified_files,
        vec![String::from("src/lib.rs"), String::from("src/main.rs")]
    );

    let result = compact(&preparation, &model, "token", None, None)
        .await
        .expect("expected compaction result");
    let details: CompactionDetails =
        serde_json::from_value(result.details.clone().expect("expected compaction details"))
            .expect("expected compaction details payload");

    assert_eq!(details.read_files, vec![String::from("README.md")]);
    assert_eq!(
        details.modified_files,
        vec![String::from("src/lib.rs"), String::from("src/main.rs")]
    );
    assert!(
        result
            .summary
            .contains("<read-files>\nREADME.md\n</read-files>")
    );
    assert!(
        result
            .summary
            .contains("<modified-files>\nsrc/lib.rs\nsrc/main.rs\n</modified-files>")
    );

    faux.unregister();
}

#[tokio::test]
async fn branch_summary_returns_file_lists_and_skips_tool_result_messages() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "compaction-files-faux".into(),
        models: vec![FauxModelDefinition {
            id: "compaction-files-faux-1".into(),
            name: Some("Compaction Files Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text(
        "## Goal\nBranch\n\n## Constraints & Preferences\n- (none)\n\n## Progress\n### Done\n- [x] investigated\n\n### In Progress\n- [ ] return\n\n### Blocked\n- (none)\n\n## Key Decisions\n- **Branch**: keep the findings\n\n## Next Steps\n1. return to the main branch",
    )]);
    let model = faux
        .get_model(Some("compaction-files-faux-1"))
        .expect("expected faux model");

    let entries = vec![
        SessionEntry::BranchSummary {
            id: String::from("1"),
            parent_id: None,
            timestamp: String::from("2025-01-01T00:00:00Z"),
            from_id: String::from("root"),
            summary: String::from("Earlier branch"),
            details: Some(
                serde_json::to_value(BranchSummaryDetails {
                    read_files: vec![String::from("nested.txt")],
                    modified_files: vec![String::from("nested.rs")],
                })
                .expect("branch summary details should serialize"),
            ),
            from_hook: None,
        },
        message_entry(
            "2",
            Some("1"),
            "2025-01-01T00:00:01Z",
            assistant_with_tool_calls(2),
        ),
        message_entry(
            "3",
            Some("2"),
            "2025-01-01T00:00:02Z",
            tool_result_message(3),
        ),
        message_entry(
            "4",
            Some("3"),
            "2025-01-01T00:00:03Z",
            user_message("wrap up", 4),
        ),
    ];

    let result: GeneratedBranchSummary = generate_branch_summary_with_details(
        &entries,
        &model,
        "token",
        None,
        BranchSummaryOptions::default(),
    )
    .await
    .expect("expected branch summary result");

    assert_eq!(
        result.read_files,
        vec![String::from("README.md"), String::from("nested.txt")]
    );
    assert_eq!(
        result.modified_files,
        vec![
            String::from("nested.rs"),
            String::from("src/lib.rs"),
            String::from("src/main.rs"),
        ]
    );
    assert!(
        result.summary.starts_with(
            "The user explored a different conversation branch before returning here."
        )
    );
    assert!(
        result
            .summary
            .contains("<read-files>\nREADME.md\nnested.txt\n</read-files>")
    );
    assert!(
        result
            .summary
            .contains("<modified-files>\nnested.rs\nsrc/lib.rs\nsrc/main.rs\n</modified-files>")
    );

    faux.unregister();
}
