use pi_coding_agent_tools::{create_read_tool, create_write_tool};
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
async fn read_returns_requested_slice_and_continuation_notice() {
    let temp_dir = unique_temp_dir("read-slice");
    fs::write(temp_dir.join("notes.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    let tool = create_read_tool(temp_dir.clone());

    let result = tool
        .execute(
            "tool-1".into(),
            json!({ "path": "notes.txt", "offset": 2, "limit": 2 }),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        result.content,
        vec![UserContent::Text {
            text: "two\nthree\n\n[2 more lines in file. Use offset=4 to continue.]".into(),
        }]
    );
}

#[tokio::test]
async fn read_reports_offset_past_end_of_file() {
    let temp_dir = unique_temp_dir("read-offset");
    fs::write(temp_dir.join("notes.txt"), "one\ntwo\n").unwrap();
    let tool = create_read_tool(temp_dir.clone());

    let error = tool
        .execute(
            "tool-1".into(),
            json!({ "path": "notes.txt", "offset": 5 }),
            None,
        )
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Offset 5 is beyond end of file (3 lines total)"
    );
}

#[tokio::test]
async fn read_returns_image_content_for_supported_image_signatures() {
    let temp_dir = unique_temp_dir("read-image");
    let png_bytes = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    fs::write(temp_dir.join("image.png"), png_bytes).unwrap();
    let tool = create_read_tool(temp_dir.clone());

    let result = tool
        .execute("tool-1".into(), json!({ "path": "image.png" }), None)
        .await
        .unwrap();

    assert_eq!(
        result.content,
        vec![
            UserContent::Text {
                text: "Read image file [image/png]".into(),
            },
            UserContent::Image {
                data: "iVBORw0KGgoBAgME".into(),
                mime_type: "image/png".into(),
            },
        ]
    );
}

#[tokio::test]
async fn write_creates_parent_directories_and_uses_js_string_length() {
    let temp_dir = unique_temp_dir("write");
    let tool = create_write_tool(temp_dir.clone());

    let result = tool
        .execute(
            "tool-1".into(),
            json!({ "path": "nested/out.txt", "content": "a🙂" }),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        result.content,
        vec![UserContent::Text {
            text: "Successfully wrote 3 bytes to nested/out.txt".into(),
        }]
    );
    assert_eq!(
        fs::read_to_string(temp_dir.join("nested/out.txt")).unwrap(),
        "a🙂"
    );
}
