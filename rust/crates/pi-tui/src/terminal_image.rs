use std::sync::{Mutex, OnceLock};

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

static CACHED_CAPABILITIES: OnceLock<Mutex<Option<TerminalCapabilities>>> = OnceLock::new();
static CELL_DIMENSIONS: OnceLock<Mutex<CellDimensions>> = OnceLock::new();

fn capabilities_cache() -> &'static Mutex<Option<TerminalCapabilities>> {
    CACHED_CAPABILITIES.get_or_init(|| Mutex::new(None))
}

fn cell_dimensions_state() -> &'static Mutex<CellDimensions> {
    CELL_DIMENSIONS.get_or_init(|| {
        Mutex::new(CellDimensions {
            width_px: 9,
            height_px: 18,
        })
    })
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
