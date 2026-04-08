use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use pi_coding_agent_tools::{
    create_read_tool, create_read_tool_with_auto_resize_flag, create_write_tool,
};
use pi_events::UserContent;
use serde_json::json;
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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

fn solid_png(width: u32, height: u32) -> Vec<u8> {
    let image = ImageBuffer::from_pixel(width, height, Rgba([255, 0, 0, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
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
    let png_bytes = solid_png(2, 2);
    let png_base64 = STANDARD.encode(&png_bytes);
    fs::write(temp_dir.join("image.png"), &png_bytes).unwrap();
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
                data: png_base64,
                mime_type: "image/png".into(),
            },
        ]
    );
}

#[tokio::test]
async fn read_auto_resizes_large_images_by_default() {
    let temp_dir = unique_temp_dir("read-auto-resize");
    let image_path = temp_dir.join("large.png");
    let original_bytes = solid_png(2_100, 2_100);
    fs::write(&image_path, &original_bytes).unwrap();
    let tool = create_read_tool(temp_dir.clone());

    let result = tool
        .execute("tool-1".into(), json!({ "path": "large.png" }), None)
        .await
        .unwrap();

    match (&result.content[0], &result.content[1]) {
        (UserContent::Text { text }, UserContent::Image { data, mime_type }) => {
            assert_eq!(mime_type, "image/png");
            assert!(text.contains("Read image file [image/png]"));
            assert!(text.contains(
                "[Image: original 2100x2100, displayed at 2000x2000. Multiply coordinates by 1.05 to map to original image.]"
            ));
            assert_ne!(data, &STANDARD.encode(original_bytes));
        }
        other => panic!("expected text + image content, got {other:?}"),
    }
}

#[tokio::test]
async fn read_tool_with_shared_flag_can_disable_auto_resize_between_calls() {
    let temp_dir = unique_temp_dir("read-shared-flag");
    let image_path = temp_dir.join("large.png");
    let original_bytes = solid_png(2_100, 2_100);
    let original_base64 = STANDARD.encode(&original_bytes);
    fs::write(&image_path, &original_bytes).unwrap();
    let auto_resize_images = Arc::new(AtomicBool::new(true));
    let tool = create_read_tool_with_auto_resize_flag(temp_dir.clone(), auto_resize_images.clone());

    let resized = tool
        .execute("tool-1".into(), json!({ "path": "large.png" }), None)
        .await
        .unwrap();
    assert!(matches!(
        &resized.content[0],
        UserContent::Text { text } if text.contains("[Image: original 2100x2100")
    ));

    auto_resize_images.store(false, Ordering::Relaxed);
    let unresized = tool
        .execute("tool-2".into(), json!({ "path": "large.png" }), None)
        .await
        .unwrap();

    assert_eq!(
        unresized.content,
        vec![
            UserContent::Text {
                text: "Read image file [image/png]".into(),
            },
            UserContent::Image {
                data: original_base64,
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
