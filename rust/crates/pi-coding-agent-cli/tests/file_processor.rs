use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use pi_coding_agent_cli::{ProcessFileOptions, process_file_arguments};
use pi_events::UserContent;
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-coding-agent-cli-{prefix}-{unique}"));
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

#[test]
fn process_file_arguments_auto_resizes_large_images_by_default() {
    let cwd = unique_temp_dir("file-processor-auto-resize");
    let image_path = cwd.join("large.png");
    let original_bytes = solid_png(2_100, 2_100);
    fs::write(&image_path, &original_bytes).unwrap();

    let result = process_file_arguments(
        &[format!("@{}", image_path.display())],
        &cwd,
        ProcessFileOptions::default(),
    )
    .unwrap();

    assert!(result.text.contains(&format!(
        "<file name=\"{}\">[Image: original 2100x2100, displayed at 2000x2000. Multiply coordinates by 1.05 to map to original image.]</file>",
        image_path.display()
    )));
    assert_eq!(result.images.len(), 1);
    match &result.images[0] {
        UserContent::Image { data, mime_type } => {
            assert_eq!(mime_type, "image/png");
            assert_ne!(data, &STANDARD.encode(original_bytes));
        }
        other => panic!("expected image attachment, got {other:?}"),
    }
}

#[test]
fn process_file_arguments_can_disable_auto_resize() {
    let cwd = unique_temp_dir("file-processor-no-auto-resize");
    let image_path = cwd.join("large.png");
    let original_bytes = solid_png(2_100, 2_100);
    let original_base64 = STANDARD.encode(&original_bytes);
    fs::write(&image_path, &original_bytes).unwrap();

    let result = process_file_arguments(
        &[format!("@{}", image_path.display())],
        &cwd,
        ProcessFileOptions {
            auto_resize_images: false,
        },
    )
    .unwrap();

    assert_eq!(
        result.text,
        format!("<file name=\"{}\"></file>\n", image_path.display())
    );
    assert_eq!(
        result.images,
        vec![UserContent::Image {
            data: original_base64,
            mime_type: "image/png".into(),
        }]
    );
}
