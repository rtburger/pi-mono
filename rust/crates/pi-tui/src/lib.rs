pub mod fuzzy;
pub mod input;
pub mod keybindings;
pub mod keys;
pub mod spacer;
pub mod stdin_buffer;
pub mod terminal;
pub mod terminal_image;
pub mod text;
pub mod tui;
pub mod utils;

pub use fuzzy::{FuzzyMatch, fuzzy_filter, fuzzy_match};
pub use input::Input;
pub use keybindings::{
    KeyId, KeybindingConflict, KeybindingDefinition, KeybindingsConfig, KeybindingsManager,
    TUI_KEYBINDINGS,
};
pub use keys::{
    KeyEventType, decode_kitty_printable, is_key_release, is_key_repeat, is_kitty_protocol_active,
    matches_key, parse_key, set_kitty_protocol_active,
};
pub use spacer::Spacer;
pub use stdin_buffer::{StdinBuffer, StdinBufferEvent, StdinBufferOptions};
pub use terminal::{ProcessTerminal, Terminal};
pub use terminal_image::{
    CellDimensions, ImageProtocol, TerminalCapabilities, detect_capabilities, get_capabilities,
    get_cell_dimensions, reset_capabilities_cache, set_cell_dimensions,
};
pub use text::Text;
pub use tui::{
    CURSOR_MARKER, Component, ComponentId, Container, InputListenerId, InputListenerResult,
    OverlayAnchor, OverlayHandle, OverlayId, OverlayMargin, OverlayOptions, SizeValue, Tui,
};
pub use utils::{
    AnsiCode, ExtractSegmentsResult, SliceWithWidthResult, extract_ansi_code, extract_segments,
    is_punctuation_char, is_whitespace_char, slice_by_column, slice_with_width, truncate_to_width,
    visible_width, wrap_text_with_ansi,
};

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("tui migration pending")]
    Pending,
}
