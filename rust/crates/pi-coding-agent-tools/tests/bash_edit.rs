use pi_coding_agent_tools::{create_bash_tool, create_edit_tool};
use pi_events::UserContent;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-coding-agent-tools-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

#[tokio::test]
async fn bash_returns_successful_output() {
    let temp_dir = unique_temp_dir("bash-success");
    let tool = create_bash_tool(temp_dir);

    let result = tool
        .execute("tool-1".into(), json!({ "command": "printf hello" }), None)
        .await
        .unwrap();

    assert_eq!(
        result.content,
        vec![UserContent::Text {
            text: "hello".into(),
        }]
    );
}

#[tokio::test]
async fn bash_reports_non_zero_exit_codes_with_output() {
    let temp_dir = unique_temp_dir("bash-error");
    let tool = create_bash_tool(temp_dir);

    let error = tool
        .execute(
            "tool-1".into(),
            json!({ "command": "printf fail && exit 7" }),
            None,
        )
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "fail\n\nCommand exited with code 7");
}

#[tokio::test]
async fn bash_reports_timeouts() {
    let temp_dir = unique_temp_dir("bash-timeout");
    let tool = create_bash_tool(temp_dir);

    let error = tool
        .execute(
            "tool-1".into(),
            json!({ "command": "sleep 1", "timeout": 0.01 }),
            None,
        )
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "Command timed out after 0.01 seconds");
}

#[tokio::test]
async fn edit_applies_multiple_replacements_against_original_content() {
    let temp_dir = unique_temp_dir("edit-success");
    fs::write(temp_dir.join("file.txt"), "alpha\nbeta\ngamma\n").unwrap();
    let tool = create_edit_tool(temp_dir.clone());

    let result = tool
        .execute(
            "tool-1".into(),
            json!({
                "path": "file.txt",
                "edits": [
                    { "oldText": "beta", "newText": "BETA" },
                    { "oldText": "gamma", "newText": "GAMMA" }
                ]
            }),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        result.content,
        vec![UserContent::Text {
            text: "Successfully replaced 2 block(s) in file.txt.".into(),
        }]
    );
    assert_eq!(
        fs::read_to_string(temp_dir.join("file.txt")).unwrap(),
        "alpha\nBETA\nGAMMA\n"
    );

    let diff = result
        .details
        .get("diff")
        .and_then(|value| value.as_str())
        .expect("successful edit should include diff details");
    assert!(diff.contains("-2 beta"));
    assert!(diff.contains("+2 BETA"));
    assert!(diff.contains("-3 gamma"));
    assert!(diff.contains("+3 GAMMA"));
    assert_eq!(
        result
            .details
            .get("firstChangedLine")
            .and_then(|value| value.as_u64()),
        Some(2)
    );
}

#[tokio::test]
async fn edit_reports_duplicate_matches() {
    let temp_dir = unique_temp_dir("edit-duplicate");
    fs::write(temp_dir.join("file.txt"), "same\nsame\n").unwrap();
    let tool = create_edit_tool(temp_dir);

    let error = tool
        .execute(
            "tool-1".into(),
            json!({
                "path": "file.txt",
                "edits": [{ "oldText": "same", "newText": "different" }]
            }),
            None,
        )
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Found 2 occurrences of the text in file.txt. The text must be unique. Please provide more context to make it unique."
    );
}

#[test]
fn edit_prepare_arguments_supports_legacy_old_text_new_text_fields() {
    let tool = create_edit_tool(PathBuf::from("/tmp"));
    let prepared = tool.prepare_arguments(json!({
        "path": "file.txt",
        "oldText": "before",
        "newText": "after"
    }));

    assert_eq!(
        prepared,
        json!({
            "path": "file.txt",
            "edits": [{ "oldText": "before", "newText": "after" }]
        })
    );
}
