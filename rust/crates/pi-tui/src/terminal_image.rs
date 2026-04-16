use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    Kitty,
    Iterm2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub images: Option<ImageProtocol>,
    pub true_color: bool,
    pub hyperlinks: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageRenderOptions {
    pub max_width_cells: Option<usize>,
    pub max_height_cells: Option<usize>,
    pub preserve_aspect_ratio: bool,
    pub image_id: Option<u32>,
}

impl Default for ImageRenderOptions {
    fn default() -> Self {
        Self {
            max_width_cells: None,
            max_height_cells: None,
            preserve_aspect_ratio: true,
            image_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageRenderResult {
    pub rows: usize,
    pub image_id: Option<u32>,
}

static CACHED_CAPABILITIES: OnceLock<Mutex<Option<TerminalCapabilities>>> = OnceLock::new();
static CELL_DIMENSIONS: OnceLock<Mutex<CellDimensions>> = OnceLock::new();
static IMAGE_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

const DEFAULT_CELL_DIMENSIONS: CellDimensions = CellDimensions {
    width_px: 9,
    height_px: 18,
};
const KITTY_PREFIX: &str = "\x1b_G";
const ITERM2_PREFIX: &str = "\x1b]1337;File=";
const KITTY_CHUNK_SIZE: usize = 4096;

fn capabilities_cache() -> &'static Mutex<Option<TerminalCapabilities>> {
    CACHED_CAPABILITIES.get_or_init(|| Mutex::new(None))
}

fn cell_dimensions_state() -> &'static Mutex<CellDimensions> {
    CELL_DIMENSIONS.get_or_init(|| Mutex::new(DEFAULT_CELL_DIMENSIONS))
}

pub fn get_cell_dimensions() -> CellDimensions {
    *cell_dimensions_state()
        .lock()
        .expect("cell dimensions mutex poisoned")
}

pub fn set_cell_dimensions(dimensions: CellDimensions) {
    *cell_dimensions_state()
        .lock()
        .expect("cell dimensions mutex poisoned") = dimensions;
}

pub fn detect_capabilities() -> TerminalCapabilities {
    let term_program = std::env::var("TERM_PROGRAM")
        .ok()
        .unwrap_or_default()
        .to_lowercase();
    let term = std::env::var("TERM")
        .ok()
        .unwrap_or_default()
        .to_lowercase();
    let color_term = std::env::var("COLORTERM")
        .ok()
        .unwrap_or_default()
        .to_lowercase();

    if std::env::var("KITTY_WINDOW_ID").is_ok() || term_program == "kitty" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }

    if term_program == "ghostty"
        || term.contains("ghostty")
        || std::env::var("GHOSTTY_RESOURCES_DIR").is_ok()
    {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }

    if std::env::var("WEZTERM_PANE").is_ok() || term_program == "wezterm" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }

    if std::env::var("ITERM_SESSION_ID").is_ok() || term_program == "iterm.app" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Iterm2),
            true_color: true,
            hyperlinks: true,
        };
    }

    if matches!(term_program.as_str(), "vscode" | "alacritty") {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: true,
        };
    }

    let true_color = color_term == "truecolor" || color_term == "24bit";
    TerminalCapabilities {
        images: None,
        true_color,
        hyperlinks: true,
    }
}

pub fn get_capabilities() -> TerminalCapabilities {
    let mut cache = capabilities_cache()
        .lock()
        .expect("capabilities cache mutex poisoned");
    if let Some(capabilities) = *cache {
        return capabilities;
    }
    let capabilities = detect_capabilities();
    *cache = Some(capabilities);
    capabilities
}

pub fn reset_capabilities_cache() {
    *capabilities_cache()
        .lock()
        .expect("capabilities cache mutex poisoned") = None;
}

pub fn is_image_line(line: &str) -> bool {
    line.starts_with(KITTY_PREFIX)
        || line.starts_with(ITERM2_PREFIX)
        || line.contains(KITTY_PREFIX)
        || line.contains(ITERM2_PREFIX)
}

pub fn allocate_image_id() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u32;
    let next = IMAGE_ID_COUNTER.fetch_add(0x9e37_79b9, Ordering::Relaxed);
    let id = nanos ^ next ^ std::process::id();
    if id == 0 { 1 } else { id }
}

pub fn encode_kitty(
    base64_data: &str,
    columns: Option<usize>,
    rows: Option<usize>,
    image_id: Option<u32>,
) -> String {
    let mut params = vec![
        String::from("a=T"),
        String::from("f=100"),
        String::from("q=2"),
    ];

    if let Some(columns) = columns {
        params.push(format!("c={columns}"));
    }
    if let Some(rows) = rows {
        params.push(format!("r={rows}"));
    }
    if let Some(image_id) = image_id {
        params.push(format!("i={image_id}"));
    }

    if base64_data.len() <= KITTY_CHUNK_SIZE {
        return format!("\x1b_G{};{base64_data}\x1b\\", params.join(","));
    }

    let mut chunks = Vec::new();
    let mut offset = 0usize;
    let mut first = true;

    while offset < base64_data.len() {
        let end = (offset + KITTY_CHUNK_SIZE).min(base64_data.len());
        let chunk = &base64_data[offset..end];
        let is_last = end >= base64_data.len();

        if first {
            chunks.push(format!("\x1b_G{},m=1;{chunk}\x1b\\", params.join(",")));
            first = false;
        } else if is_last {
            chunks.push(format!("\x1b_Gm=0;{chunk}\x1b\\"));
        } else {
            chunks.push(format!("\x1b_Gm=1;{chunk}\x1b\\"));
        }

        offset = end;
    }

    chunks.join("")
}

pub fn delete_kitty_image(image_id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={image_id}\x1b\\")
}

pub fn delete_all_kitty_images() -> String {
    String::from("\x1b_Ga=d,d=A\x1b\\")
}

pub fn encode_iterm2(
    base64_data: &str,
    width: Option<&str>,
    height: Option<&str>,
    name: Option<&str>,
    preserve_aspect_ratio: bool,
    inline: bool,
) -> String {
    let mut params = vec![format!("inline={}", if inline { 1 } else { 0 })];

    if let Some(width) = width {
        params.push(format!("width={width}"));
    }
    if let Some(height) = height {
        params.push(format!("height={height}"));
    }
    if let Some(name) = name {
        params.push(format!("name={}", BASE64_STANDARD.encode(name)));
    }
    if !preserve_aspect_ratio {
        params.push(String::from("preserveAspectRatio=0"));
    }

    format!("\x1b]1337;File={}:{}\x07", params.join(";"), base64_data)
}

pub fn calculate_image_rows(
    image_dimensions: ImageDimensions,
    target_width_cells: usize,
    cell_dimensions: CellDimensions,
) -> usize {
    if target_width_cells == 0
        || image_dimensions.width_px == 0
        || image_dimensions.height_px == 0
        || cell_dimensions.width_px == 0
        || cell_dimensions.height_px == 0
    {
        return 1;
    }

    let target_width_px = target_width_cells as f64 * cell_dimensions.width_px as f64;
    let scale = target_width_px / image_dimensions.width_px as f64;
    let scaled_height_px = image_dimensions.height_px as f64 * scale;
    let rows = (scaled_height_px / cell_dimensions.height_px as f64).ceil() as usize;
    rows.max(1)
}

pub fn get_png_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 24 {
        return None;
    }

    if bytes[0..4] != [0x89, 0x50, 0x4e, 0x47] {
        return None;
    }

    Some(ImageDimensions {
        width_px: read_u32_be(&bytes[16..20])?,
        height_px: read_u32_be(&bytes[20..24])?,
    })
}

pub fn get_jpeg_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 2 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }

    let mut offset = 2usize;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }

        let marker = bytes[offset + 1];
        if (0xc0..=0xc2).contains(&marker) {
            return Some(ImageDimensions {
                height_px: read_u16_be(&bytes[offset + 5..offset + 7])? as u32,
                width_px: read_u16_be(&bytes[offset + 7..offset + 9])? as u32,
            });
        }

        let length = read_u16_be(&bytes[offset + 2..offset + 4])? as usize;
        if length < 2 {
            return None;
        }
        offset += 2 + length;
    }

    None
}

pub fn get_gif_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 10 {
        return None;
    }

    let signature = std::str::from_utf8(&bytes[0..6]).ok()?;
    if signature != "GIF87a" && signature != "GIF89a" {
        return None;
    }

    Some(ImageDimensions {
        width_px: read_u16_le(&bytes[6..8])? as u32,
        height_px: read_u16_le(&bytes[8..10])? as u32,
    })
}

pub fn get_webp_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 30 {
        return None;
    }

    if std::str::from_utf8(&bytes[0..4]).ok()? != "RIFF"
        || std::str::from_utf8(&bytes[8..12]).ok()? != "WEBP"
    {
        return None;
    }

    match std::str::from_utf8(&bytes[12..16]).ok()? {
        "VP8 " => Some(ImageDimensions {
            width_px: (read_u16_le(&bytes[26..28])? & 0x3fff) as u32,
            height_px: (read_u16_le(&bytes[28..30])? & 0x3fff) as u32,
        }),
        "VP8L" => {
            let bits = read_u32_le(&bytes[21..25])?;
            Some(ImageDimensions {
                width_px: (bits & 0x3fff) + 1,
                height_px: ((bits >> 14) & 0x3fff) + 1,
            })
        }
        "VP8X" => Some(ImageDimensions {
            width_px: read_u24_le(&bytes[24..27])? + 1,
            height_px: read_u24_le(&bytes[27..30])? + 1,
        }),
        _ => None,
    }
}

pub fn get_image_dimensions(base64_data: &str, mime_type: &str) -> Option<ImageDimensions> {
    match base_mime_type(mime_type).as_str() {
        "image/png" => get_png_dimensions(base64_data),
        "image/jpeg" => get_jpeg_dimensions(base64_data),
        "image/gif" => get_gif_dimensions(base64_data),
        "image/webp" => get_webp_dimensions(base64_data),
        _ => None,
    }
}

pub fn render_image(
    base64_data: &str,
    image_dimensions: ImageDimensions,
    options: ImageRenderOptions,
) -> Option<(String, ImageRenderResult)> {
    let capabilities = get_capabilities();
    let protocol = capabilities.images?;
    let cell_dimensions = get_cell_dimensions();
    let (columns, rows) = resolve_target_size(image_dimensions, cell_dimensions, options);

    match protocol {
        ImageProtocol::Kitty => Some((
            encode_kitty(base64_data, Some(columns), Some(rows), options.image_id),
            ImageRenderResult {
                rows,
                image_id: options.image_id,
            },
        )),
        ImageProtocol::Iterm2 => Some((
            encode_iterm2(
                base64_data,
                Some(&columns.to_string()),
                Some("auto"),
                None,
                options.preserve_aspect_ratio,
                true,
            ),
            ImageRenderResult {
                rows,
                image_id: None,
            },
        )),
    }
}

pub fn image_fallback(
    mime_type: &str,
    dimensions: Option<ImageDimensions>,
    filename: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(filename) = filename.filter(|filename| !filename.is_empty()) {
        parts.push(filename.to_owned());
    }
    parts.push(format!("[{}]", base_mime_type(mime_type)));
    if let Some(dimensions) = dimensions {
        parts.push(format!("{}x{}", dimensions.width_px, dimensions.height_px));
    }
    format!("[Image: {}]", parts.join(" "))
}

fn decode_base64(base64_data: &str) -> Option<Vec<u8>> {
    BASE64_STANDARD.decode(base64_data).ok()
}

fn base_mime_type(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase()
}

fn resolve_target_size(
    image_dimensions: ImageDimensions,
    cell_dimensions: CellDimensions,
    options: ImageRenderOptions,
) -> (usize, usize) {
    let max_width = options.max_width_cells.unwrap_or(80).max(1);
    let max_height = options.max_height_cells.unwrap_or(usize::MAX).max(1);

    if !options.preserve_aspect_ratio {
        let rows =
            calculate_image_rows(image_dimensions, max_width, cell_dimensions).min(max_height);
        return (max_width, rows.max(1));
    }

    if image_dimensions.width_px == 0
        || image_dimensions.height_px == 0
        || cell_dimensions.width_px == 0
        || cell_dimensions.height_px == 0
    {
        return (max_width, 1.min(max_height));
    }

    let width_scale =
        (max_width as f64 * cell_dimensions.width_px as f64) / image_dimensions.width_px as f64;
    let height_scale = if max_height == usize::MAX {
        width_scale
    } else {
        (max_height as f64 * cell_dimensions.height_px as f64) / image_dimensions.height_px as f64
    };
    let scale = width_scale.min(height_scale).max(0.0);
    let scaled_width_px = (image_dimensions.width_px as f64 * scale).ceil();
    let mut columns = (scaled_width_px / cell_dimensions.width_px as f64).ceil() as usize;
    if columns == 0 {
        columns = 1;
    }
    columns = columns.min(max_width);

    let rows = calculate_image_rows(image_dimensions, columns, cell_dimensions).min(max_height);
    (columns, rows.max(1))
}

fn read_u16_be(bytes: &[u8]) -> Option<u16> {
    let bytes = <[u8; 2]>::try_from(bytes).ok()?;
    Some(u16::from_be_bytes(bytes))
}

fn read_u16_le(bytes: &[u8]) -> Option<u16> {
    let bytes = <[u8; 2]>::try_from(bytes).ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn read_u24_le(bytes: &[u8]) -> Option<u32> {
    let bytes = <[u8; 3]>::try_from(bytes).ok()?;
    Some(u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
}

fn read_u32_be(bytes: &[u8]) -> Option<u32> {
    let bytes = <[u8; 4]>::try_from(bytes).ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn read_u32_le(bytes: &[u8]) -> Option<u32> {
    let bytes = <[u8; 4]>::try_from(bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}
