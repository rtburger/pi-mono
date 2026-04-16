use pi_coding_agent_tools::{create_find_tool, create_grep_tool, create_ls_tool};
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
async fn ls_lists_sorted_entries_and_marks_directories() {
    let temp_dir = unique_temp_dir("ls");
    fs::create_dir_all(temp_dir.join("src")).unwrap();
    fs::write(temp_dir.join("b.txt"), "b").unwrap();
    fs::write(temp_dir.join("A.txt"), "a").unwrap();
    let tool = create_ls_tool(temp_dir.clone());

    let result = tool
        .execute("tool-1".into(), json!({}), None)
        .await
        .unwrap();

    assert_eq!(
        result.content,
        vec![UserContent::Text {
            text: "A.txt\nb.txt\nsrc/".into(),
        }]
    );
}

#[tokio::test]
async fn find_returns_relative_matches() {
    let temp_dir = unique_temp_dir("find");
    fs::create_dir_all(temp_dir.join("src/nested")).unwrap();
    fs::write(temp_dir.join("src/lib.rs"), "lib").unwrap();
    fs::write(temp_dir.join("src/nested/mod.rs"), "mod").unwrap();
    fs::write(temp_dir.join("README.md"), "readme").unwrap();
    let tool = create_find_tool(temp_dir.clone());

    let result = tool
        .execute("tool-1".into(), json!({ "pattern": "src/**/*.rs" }), None)
        .await
        .unwrap();

    assert_eq!(
        result.content,
        vec![UserContent::Text {
            text: "src/lib.rs\nsrc/nested/mod.rs".into(),
        }]
    );
}

#[tokio::test]
async fn grep_returns_matching_lines() {
    let temp_dir = unique_temp_dir("grep");
    fs::create_dir_all(temp_dir.join("src")).unwrap();
    fs::write(temp_dir.join("src/lib.rs"), "alpha\nbeta\nalpha_beta\n").unwrap();
    let tool = create_grep_tool(temp_dir.clone());

    let result = tool
        .execute(
            "tool-1".into(),
            json!({ "pattern": "alpha", "path": "src", "literal": true }),
            None,
        )
        .await
        .unwrap();

    let output = match &result.content[0] {
        UserContent::Text { text } => text.clone(),
        other => panic!("expected text output, got {other:?}"),
    };
    assert!(output.contains("lib.rs:1:alpha"), "output: {output}");
    assert!(output.contains("lib.rs:3:alpha_beta"), "output: {output}");
}
