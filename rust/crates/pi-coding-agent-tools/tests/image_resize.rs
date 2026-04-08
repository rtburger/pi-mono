use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use pi_coding_agent_tools::{ImageResizeOptions, format_dimension_note, resize_image_bytes};
use std::io::Cursor;

fn solid_png(width: u32, height: u32) -> Vec<u8> {
    let image = ImageBuffer::from_pixel(width, height, Rgba([255, 0, 0, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn patterned_png(width: u32, height: u32) -> Vec<u8> {
    let image = ImageBuffer::from_fn(width, height, |x, y| {
        Rgba([
            ((x * 37 + y * 11) % 256) as u8,
            ((x * 17 + y * 29) % 256) as u8,
            ((x * 7 + y * 53) % 256) as u8,
            255,
        ])
    });
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

#[test]
fn resize_image_bytes_returns_original_data_when_within_limits() {
    let bytes = solid_png(2, 2);
    let original_base64 = STANDARD.encode(&bytes);

    let result = resize_image_bytes(&bytes, "image/png", None).expect("expected image");

    assert!(!result.was_resized);
    assert_eq!(result.data, original_base64);
    assert_eq!(result.mime_type, "image/png");
    assert_eq!(result.original_width, 2);
    assert_eq!(result.original_height, 2);
    assert_eq!(result.width, 2);
    assert_eq!(result.height, 2);
    assert_eq!(format_dimension_note(&result), None);
}

#[test]
fn resize_image_bytes_resizes_images_exceeding_dimension_limits() {
    let bytes = solid_png(100, 100);
    let result = resize_image_bytes(
        &bytes,
        "image/png",
        Some(ImageResizeOptions {
            max_width: 50,
            max_height: 50,
            max_bytes: 1_048_576,
            jpeg_quality: 80,
        }),
    )
    .expect("expected resized image");

    assert!(result.was_resized);
    assert_eq!(result.original_width, 100);
    assert_eq!(result.original_height, 100);
    assert!(result.width <= 50);
    assert!(result.height <= 50);
    assert_eq!(
        format_dimension_note(&result).as_deref(),
        Some(
            "[Image: original 100x100, displayed at 50x50. Multiply coordinates by 2.00 to map to original image.]"
        )
    );
}

#[test]
fn resize_image_bytes_can_reduce_images_for_byte_limits() {
    let bytes = patterned_png(200, 200);
    let original_base64_len = STANDARD.encode(&bytes).len();
    let result = resize_image_bytes(
        &bytes,
        "image/png",
        Some(ImageResizeOptions {
            max_width: 2_000,
            max_height: 2_000,
            max_bytes: original_base64_len.saturating_sub(100),
            jpeg_quality: 80,
        }),
    )
    .expect("expected resized image");

    assert!(result.was_resized);
    assert!(result.data.len() < original_base64_len);
}

#[test]
fn resize_image_bytes_returns_none_when_limit_is_impossible() {
    let bytes = patterned_png(200, 200);

    let result = resize_image_bytes(
        &bytes,
        "image/png",
        Some(ImageResizeOptions {
            max_width: 2_000,
            max_height: 2_000,
            max_bytes: 1,
            jpeg_quality: 80,
        }),
    );

    assert_eq!(result, None);
}
