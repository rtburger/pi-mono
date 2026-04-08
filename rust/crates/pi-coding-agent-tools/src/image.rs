use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{
    DynamicImage, GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder, imageops::FilterType,
};
use std::io::Cursor;

pub const DEFAULT_MAX_INLINE_IMAGE_BYTES: usize = 4_718_592;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageResizeOptions {
    pub max_width: u32,
    pub max_height: u32,
    pub max_bytes: usize,
    pub jpeg_quality: u8,
}

impl Default for ImageResizeOptions {
    fn default() -> Self {
        Self {
            max_width: 2_000,
            max_height: 2_000,
            max_bytes: DEFAULT_MAX_INLINE_IMAGE_BYTES,
            jpeg_quality: 80,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizedImage {
    pub data: String,
    pub mime_type: String,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub was_resized: bool,
}

pub fn resize_image_bytes(
    bytes: &[u8],
    mime_type: &str,
    options: Option<ImageResizeOptions>,
) -> Option<ResizedImage> {
    let options = options.unwrap_or_default();
    let input_base64 = STANDARD.encode(bytes);
    let input_base64_size = input_base64.len();
    let image = image::load_from_memory(bytes).ok()?;
    let (original_width, original_height) = image.dimensions();

    if original_width <= options.max_width
        && original_height <= options.max_height
        && input_base64_size < options.max_bytes
    {
        return Some(ResizedImage {
            data: input_base64,
            mime_type: mime_type.to_string(),
            original_width,
            original_height,
            width: original_width,
            height: original_height,
            was_resized: false,
        });
    }

    let (target_width, target_height) = clamp_dimensions(
        original_width,
        original_height,
        options.max_width,
        options.max_height,
    );
    let quality_steps = jpeg_quality_steps(options.jpeg_quality);
    let mut current_width = target_width;
    let mut current_height = target_height;

    loop {
        let resized = if current_width == original_width && current_height == original_height {
            image.clone()
        } else {
            image.resize_exact(current_width, current_height, FilterType::Lanczos3)
        };

        for candidate in encode_candidates(&resized, &quality_steps) {
            if candidate.encoded_size < options.max_bytes {
                return Some(ResizedImage {
                    data: candidate.data,
                    mime_type: candidate.mime_type,
                    original_width,
                    original_height,
                    width: current_width,
                    height: current_height,
                    was_resized: true,
                });
            }
        }

        if current_width == 1 && current_height == 1 {
            break;
        }

        let next_width = if current_width == 1 {
            1
        } else {
            ((f64::from(current_width) * 0.75).floor() as u32).max(1)
        };
        let next_height = if current_height == 1 {
            1
        } else {
            ((f64::from(current_height) * 0.75).floor() as u32).max(1)
        };

        if next_width == current_width && next_height == current_height {
            break;
        }

        current_width = next_width;
        current_height = next_height;
    }

    None
}

pub fn format_dimension_note(result: &ResizedImage) -> Option<String> {
    if !result.was_resized {
        return None;
    }

    let scale = f64::from(result.original_width) / f64::from(result.width);
    Some(format!(
        "[Image: original {}x{}, displayed at {}x{}. Multiply coordinates by {:.2} to map to original image.]",
        result.original_width, result.original_height, result.width, result.height, scale
    ))
}

#[derive(Debug)]
struct EncodedCandidate {
    data: String,
    encoded_size: usize,
    mime_type: String,
}

fn clamp_dimensions(
    original_width: u32,
    original_height: u32,
    max_width: u32,
    max_height: u32,
) -> (u32, u32) {
    let mut target_width = original_width;
    let mut target_height = original_height;

    if target_width > max_width {
        target_height = round_div_u64(
            u64::from(target_height) * u64::from(max_width),
            u64::from(target_width),
        ) as u32;
        target_width = max_width;
    }
    if target_height > max_height {
        target_width = round_div_u64(
            u64::from(target_width) * u64::from(max_height),
            u64::from(target_height),
        ) as u32;
        target_height = max_height;
    }

    (target_width.max(1), target_height.max(1))
}

fn round_div_u64(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        return 0;
    }
    (numerator + (denominator / 2)) / denominator
}

fn jpeg_quality_steps(preferred: u8) -> Vec<u8> {
    let mut values = Vec::new();
    for quality in [preferred, 85, 70, 55, 40] {
        if !values.contains(&quality) {
            values.push(quality);
        }
    }
    values
}

fn encode_candidates(image: &DynamicImage, jpeg_qualities: &[u8]) -> Vec<EncodedCandidate> {
    let mut candidates = Vec::new();
    if let Some(candidate) = encode_png(image) {
        candidates.push(candidate);
    }
    for quality in jpeg_qualities {
        if let Some(candidate) = encode_jpeg(image, *quality) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn encode_png(image: &DynamicImage) -> Option<EncodedCandidate> {
    let mut buffer = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png)
        .ok()?;
    let data = STANDARD.encode(&buffer);
    Some(EncodedCandidate {
        encoded_size: data.len(),
        data,
        mime_type: "image/png".into(),
    })
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Option<EncodedCandidate> {
    let mut buffer = Vec::new();
    let rgb_image = DynamicImage::ImageRgb8(image.to_rgb8());
    JpegEncoder::new_with_quality(&mut buffer, quality)
        .encode_image(&rgb_image)
        .ok()?;
    let data = STANDARD.encode(&buffer);
    Some(EncodedCandidate {
        encoded_size: data.len(),
        data,
        mime_type: "image/jpeg".into(),
    })
}
