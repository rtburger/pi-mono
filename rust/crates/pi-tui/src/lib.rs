pub mod autocomplete;
mod box_component;
pub mod editor;
pub mod fuzzy;
pub mod image;
pub mod input;
pub mod keybindings;
pub mod keys;
mod kill_ring;
pub mod loader;
pub mod markdown;
pub mod select_list;
pub mod settings_list;
pub mod spacer;
pub mod stdin_buffer;
pub mod terminal;
pub mod terminal_image;
pub mod text;
pub mod truncated_text;
pub mod tui;
mod undo_stack;
pub mod utils;

pub use autocomplete::{
    AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider,
    CompletionResult, SlashCommand, apply_completion,
};
pub use box_component::Box;
pub use editor::{Editor, EditorCursor, EditorOptions, TextChunk, word_wrap_line};
pub use fuzzy::{FuzzyMatch, fuzzy_filter, fuzzy_match};
pub use image::{Image, ImageOptions, ImageTheme};
pub use input::Input;
pub use keybindings::{
    KeyId, KeybindingConflict, KeybindingDefinition, KeybindingsConfig, KeybindingsManager,
    TUI_KEYBINDINGS,
};
pub use keys::{
    KeyEventType, decode_kitty_printable, is_key_release, is_key_repeat, is_kitty_protocol_active,
    matches_key, parse_key, set_kitty_protocol_active,
};
pub use loader::{CancellableLoader, Loader};
pub use markdown::{DefaultTextStyle, Markdown, MarkdownTheme};
pub use select_list::{
    SelectItem, SelectList, SelectListLayoutOptions, SelectListTheme,
    SelectListTruncatePrimaryContext,
};
pub use settings_list::{
    SettingItem, SettingsList, SettingsListOptions, SettingsListTheme, SettingsSubmenuDone,
    SettingsSubmenuFactory,
};
pub use spacer::Spacer;
pub use stdin_buffer::{StdinBuffer, StdinBufferEvent, StdinBufferOptions};
pub use terminal::{ProcessTerminal, Terminal};
pub use terminal_image::{
    CellDimensions, ImageDimensions, ImageProtocol, ImageRenderOptions, ImageRenderResult,
    TerminalCapabilities, allocate_image_id, calculate_image_rows, delete_all_kitty_images,
    delete_kitty_image, detect_capabilities, encode_iterm2, encode_kitty, get_capabilities,
    get_cell_dimensions, get_gif_dimensions, get_image_dimensions, get_jpeg_dimensions,
    get_png_dimensions, get_webp_dimensions, image_fallback, is_image_line, render_image,
    reset_capabilities_cache, set_cell_dimensions,
};
pub use text::Text;
pub use truncated_text::TruncatedText;
pub use tui::{
    CURSOR_MARKER, Component, ComponentId, Container, InputListenerId, InputListenerResult,
    OverlayAnchor, OverlayHandle, OverlayId, OverlayMargin, OverlayOptions, RenderHandle,
    SizeValue, Tui,
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
