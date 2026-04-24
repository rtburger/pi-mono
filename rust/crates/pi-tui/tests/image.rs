use parking_lot::{Mutex, MutexGuard};
use pi_tui::{
    CellDimensions, Component, Image, ImageDimensions, ImageOptions, ImageTheme,
    reset_capabilities_cache, set_cell_dimensions,
};
use std::{ffi::OsString, sync::LazyLock};

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
        let lock = TEST_LOCK.lock();
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
fn image_renders_fallback_text_when_terminal_has_no_image_support() {
    let guard = TestGuard::new();
    guard.set_env("TERM_PROGRAM", Some("vscode"));

    let image = Image::with_dimensions(
        "Zm9v",
        "image/png",
        ImageTheme::new().with_fallback_color(|text| format!("<{text}>")),
        ImageOptions {
            filename: Some(String::from("cat.png")),
            ..ImageOptions::default()
        },
        ImageDimensions {
            width_px: 320,
            height_px: 200,
        },
    );

    let lines = image.render(120);

    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "<[Image: cat.png [image/png] 320x200]>");
}

#[test]
fn image_renders_kitty_sequence_with_reserved_rows() {
    let guard = TestGuard::new();
    guard.set_env("TERM_PROGRAM", Some("kitty"));
    guard.set_cell_dimensions(10, 10);

    let image = Image::with_dimensions(
        "Zm9v",
        "image/png",
        ImageTheme::default(),
        ImageOptions {
            image_id: Some(42),
            ..ImageOptions::default()
        },
        ImageDimensions {
            width_px: 100,
            height_px: 20,
        },
    );

    let lines = image.render(12);

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "");
    assert!(
        lines[1].starts_with("\x1b[1A\x1b_G"),
        "line: {:?}",
        lines[1]
    );
    assert!(lines[1].contains("c=10"), "line: {:?}", lines[1]);
    assert!(lines[1].contains("r=2"), "line: {:?}", lines[1]);
    assert!(lines[1].contains("i=42"), "line: {:?}", lines[1]);
    assert_eq!(image.image_id(), Some(42));
}

#[test]
fn clearing_cache_allows_image_to_switch_from_fallback_to_inline_render() {
    let guard = TestGuard::new();
    let image = Image::with_dimensions(
        "Zm9v",
        "image/png",
        ImageTheme::default(),
        ImageOptions::default(),
        ImageDimensions {
            width_px: 100,
            height_px: 100,
        },
    );

    guard.set_env("TERM_PROGRAM", Some("vscode"));
    let fallback = image.render(20);
    assert_eq!(fallback.len(), 1);
    assert!(fallback[0].contains("[Image:"));

    guard.set_env("TERM_PROGRAM", Some("kitty"));
    image.clear_cache();
    let inline = image.render(20);

    assert!(inline.len() > 1);
    assert!(inline.iter().any(|line| line.contains("\x1b_G")));
}
