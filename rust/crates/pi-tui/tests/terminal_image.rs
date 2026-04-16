use base64::{Engine as _, engine::general_purpose::STANDARD};
use pi_tui::{
    CellDimensions, ImageDimensions, ImageProtocol, ImageRenderOptions, allocate_image_id,
    get_capabilities, get_gif_dimensions, get_image_dimensions, get_jpeg_dimensions,
    get_png_dimensions, get_webp_dimensions, image_fallback, is_image_line, render_image,
    reset_capabilities_cache, set_cell_dimensions,
};
use std::{
    ffi::OsString,
    sync::{LazyLock, Mutex, MutexGuard},
};

static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
const GUARDED_ENV_VARS: &[&str] = &[
    "TERM_PROGRAM",
    "TERM",
    "COLORTERM",
    "KITTY_WINDOW_ID",
    "GHOSTTY_RESOURCES_DIR",
    "WEZTERM_PANE",
    "ITERM_SESSION_ID",
];

struct TestGuard {
    _lock: MutexGuard<'static, ()>,
    previous_env: Vec<(&'static str, Option<OsString>)>,
    previous_cell_dimensions: CellDimensions,
}

impl TestGuard {
    fn new() -> Self {
        let lock = TEST_LOCK.lock().expect("terminal image test lock");
        let previous_env = GUARDED_ENV_VARS
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect();
        let previous_cell_dimensions = CellDimensions {
            width_px: 9,
            height_px: 18,
        };
        reset_capabilities_cache();
        set_cell_dimensions(previous_cell_dimensions);

        Self {
            _lock: lock,
            previous_env,
            previous_cell_dimensions,
        }
    }

    fn set_env(&self, name: &str, value: Option<&str>) {
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation through TEST_LOCK.
                unsafe { std::env::set_var(name, value) }
            }
            None => {
                // SAFETY: tests serialize environment mutation through TEST_LOCK.
                unsafe { std::env::remove_var(name) }
            }
        }
        reset_capabilities_cache();
    }

    fn set_cell_dimensions(&self, width_px: u32, height_px: u32) {
        set_cell_dimensions(CellDimensions {
            width_px,
            height_px,
        });
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        for (name, value) in &self.previous_env {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize environment mutation through TEST_LOCK.
                    unsafe { std::env::set_var(name, value) }
                }
                None => {
                    // SAFETY: tests serialize environment mutation through TEST_LOCK.
                    unsafe { std::env::remove_var(name) }
                }
            }
        }
        set_cell_dimensions(self.previous_cell_dimensions);
        reset_capabilities_cache();
    }
}

#[test]
fn detects_kitty_capabilities_after_cache_reset() {
    let guard = TestGuard::new();
    guard.set_env("TERM_PROGRAM", Some("kitty"));

    let capabilities = get_capabilities();

    assert_eq!(capabilities.images, Some(ImageProtocol::Kitty));
    assert!(capabilities.true_color);
    assert!(capabilities.hyperlinks);
}

#[test]
fn parses_dimensions_for_supported_image_formats() {
    let png = STANDARD.encode([
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0, 0x49, 0x48, 0x44, 0x52, 0, 0,
        0, 16, 0, 0, 0, 32,
    ]);
    let jpeg = STANDARD.encode([
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x0a, 0x00, 0x14, 0x03, 0x01, 0x11, 0x00,
        0x02, 0x11, 0x00, 0x03, 0x11, 0x00,
    ]);
    let gif = STANDARD.encode([0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x20, 0x00, 0x10, 0x00]);
    let webp = STANDARD.encode([
        0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x58, 0, 0,
        0, 0, 0, 0, 0, 0, 0x1f, 0x00, 0x00, 0x3f, 0x00, 0x00,
    ]);

    assert_eq!(
        get_png_dimensions(&png),
        Some(ImageDimensions {
            width_px: 16,
            height_px: 32,
        })
    );
    assert_eq!(
        get_jpeg_dimensions(&jpeg),
        Some(ImageDimensions {
            width_px: 20,
            height_px: 10,
        })
    );
    assert_eq!(
        get_gif_dimensions(&gif),
        Some(ImageDimensions {
            width_px: 32,
            height_px: 16,
        })
    );
    assert_eq!(
        get_webp_dimensions(&webp),
        Some(ImageDimensions {
            width_px: 32,
            height_px: 64,
        })
    );
    assert_eq!(
        get_image_dimensions(&png, "image/png; charset=binary"),
        Some(ImageDimensions {
            width_px: 16,
            height_px: 32,
        })
    );
}

#[test]
fn render_image_respects_height_limit_for_kitty() {
    let guard = TestGuard::new();
    guard.set_env("TERM_PROGRAM", Some("kitty"));
    guard.set_cell_dimensions(10, 10);

    let (sequence, result) = render_image(
        "Zm9v",
        ImageDimensions {
            width_px: 100,
            height_px: 100,
        },
        ImageRenderOptions {
            max_width_cells: Some(10),
            max_height_cells: Some(5),
            preserve_aspect_ratio: true,
            image_id: Some(7),
        },
    )
    .expect("kitty render result");

    assert_eq!(result.rows, 5);
    assert_eq!(result.image_id, Some(7));
    assert!(sequence.contains("c=5"), "sequence: {sequence:?}");
    assert!(sequence.contains("r=5"), "sequence: {sequence:?}");
    assert!(sequence.contains("i=7"), "sequence: {sequence:?}");
}

#[test]
fn fallback_and_image_line_detection_match_terminal_sequences() {
    let fallback = image_fallback(
        "image/png",
        Some(ImageDimensions {
            width_px: 64,
            height_px: 32,
        }),
        Some("cat.png"),
    );
    let image_id = allocate_image_id();
    let line = format!("\x1b[2A\x1b_Ga=T,i={image_id};Zm9v\x1b\\");

    assert_eq!(fallback, "[Image: cat.png [image/png] 64x32]");
    assert!(is_image_line(&line));
    assert!(image_id > 0);
}
