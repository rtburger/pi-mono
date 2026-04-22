pub mod autocomplete;
mod box_component;
pub mod dynamic_border;
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

use std::sync::Arc;

pub type ThemeTextStyleFn = dyn Fn(&str) -> String + Send + Sync + 'static;
pub type ThemeSelectionTextStyleFn = dyn Fn(&str, bool) -> String + Send + Sync + 'static;
pub type ThemeHighlightCodeFn = dyn Fn(&str, Option<&str>) -> Vec<String> + Send + Sync + 'static;

pub type SharedThemeTextStyle = Arc<ThemeTextStyleFn>;
pub type SharedThemeSelectionTextStyle = Arc<ThemeSelectionTextStyleFn>;
pub type SharedThemeHighlightCode = Arc<ThemeHighlightCodeFn>;

fn identity_theme_text_style() -> SharedThemeTextStyle {
    Arc::new(str::to_owned)
}

fn identity_theme_selection_text_style() -> SharedThemeSelectionTextStyle {
    Arc::new(|text, _| text.to_owned())
}

/// Interface-style image theme matching the TypeScript `ImageTheme` shape.
#[derive(Clone)]
pub struct ImageThemeSpec {
    pub fallback_color: SharedThemeTextStyle,
}

impl ImageThemeSpec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for ImageThemeSpec {
    fn default() -> Self {
        Self {
            fallback_color: identity_theme_text_style(),
        }
    }
}

impl From<ImageThemeSpec> for image::ImageTheme {
    fn from(spec: ImageThemeSpec) -> Self {
        let ImageThemeSpec { fallback_color } = spec;
        image::ImageTheme::new().with_fallback_color(move |text| fallback_color(text))
    }
}

impl From<&ImageThemeSpec> for image::ImageTheme {
    fn from(spec: &ImageThemeSpec) -> Self {
        spec.clone().into()
    }
}

/// Interface-style default markdown text style matching the TypeScript `DefaultTextStyle` shape.
#[derive(Clone, Default)]
pub struct DefaultTextStyleSpec {
    pub color: Option<SharedThemeTextStyle>,
    pub bg_color: Option<SharedThemeTextStyle>,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
}

impl DefaultTextStyleSpec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl From<DefaultTextStyleSpec> for markdown::DefaultTextStyle {
    fn from(spec: DefaultTextStyleSpec) -> Self {
        let DefaultTextStyleSpec {
            color,
            bg_color,
            bold,
            italic,
            strikethrough,
            underline,
        } = spec;

        let mut style = markdown::DefaultTextStyle::new()
            .with_bold(bold)
            .with_italic(italic)
            .with_strikethrough(strikethrough)
            .with_underline(underline);

        if let Some(color) = color {
            style = style.with_color(move |text| color(text));
        }

        if let Some(bg_color) = bg_color {
            style = style.with_bg_color(move |text| bg_color(text));
        }

        style
    }
}

impl From<&DefaultTextStyleSpec> for markdown::DefaultTextStyle {
    fn from(spec: &DefaultTextStyleSpec) -> Self {
        spec.clone().into()
    }
}

/// Interface-style select list theme matching the TypeScript `SelectListTheme` shape.
#[derive(Clone)]
pub struct SelectListThemeSpec {
    pub selected_prefix: SharedThemeTextStyle,
    pub selected_text: SharedThemeTextStyle,
    pub description: SharedThemeTextStyle,
    pub scroll_info: SharedThemeTextStyle,
    pub no_match: SharedThemeTextStyle,
}

impl SelectListThemeSpec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for SelectListThemeSpec {
    fn default() -> Self {
        Self {
            selected_prefix: identity_theme_text_style(),
            selected_text: identity_theme_text_style(),
            description: identity_theme_text_style(),
            scroll_info: identity_theme_text_style(),
            no_match: identity_theme_text_style(),
        }
    }
}

impl From<SelectListThemeSpec> for select_list::SelectListTheme {
    fn from(spec: SelectListThemeSpec) -> Self {
        let SelectListThemeSpec {
            selected_prefix,
            selected_text,
            description,
            scroll_info,
            no_match,
        } = spec;

        select_list::SelectListTheme::new()
            .with_selected_prefix(move |text| selected_prefix(text))
            .with_selected_text(move |text| selected_text(text))
            .with_description(move |text| description(text))
            .with_scroll_info(move |text| scroll_info(text))
            .with_no_match(move |text| no_match(text))
    }
}

impl From<&SelectListThemeSpec> for select_list::SelectListTheme {
    fn from(spec: &SelectListThemeSpec) -> Self {
        spec.clone().into()
    }
}

/// Interface-style editor theme matching the TypeScript `EditorTheme` shape.
#[derive(Clone)]
pub struct EditorThemeSpec {
    pub border_color: SharedThemeTextStyle,
    pub select_list: SelectListThemeSpec,
}

impl EditorThemeSpec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for EditorThemeSpec {
    fn default() -> Self {
        Self {
            border_color: identity_theme_text_style(),
            select_list: SelectListThemeSpec::default(),
        }
    }
}

impl From<EditorThemeSpec> for editor::EditorTheme {
    fn from(spec: EditorThemeSpec) -> Self {
        let EditorThemeSpec {
            border_color,
            select_list,
        } = spec;

        editor::EditorTheme::new()
            .with_border_color(move |text| border_color(text))
            .with_select_list(select_list.into())
    }
}

impl From<&EditorThemeSpec> for editor::EditorTheme {
    fn from(spec: &EditorThemeSpec) -> Self {
        spec.clone().into()
    }
}

/// Interface-style markdown theme matching the TypeScript `MarkdownTheme` shape.
#[derive(Clone)]
pub struct MarkdownThemeSpec {
    pub heading: SharedThemeTextStyle,
    pub link: SharedThemeTextStyle,
    pub link_url: SharedThemeTextStyle,
    pub code: SharedThemeTextStyle,
    pub code_block: SharedThemeTextStyle,
    pub code_block_border: SharedThemeTextStyle,
    pub quote: SharedThemeTextStyle,
    pub quote_border: SharedThemeTextStyle,
    pub hr: SharedThemeTextStyle,
    pub list_bullet: SharedThemeTextStyle,
    pub bold: SharedThemeTextStyle,
    pub italic: SharedThemeTextStyle,
    pub strikethrough: SharedThemeTextStyle,
    pub underline: SharedThemeTextStyle,
    pub highlight_code: Option<SharedThemeHighlightCode>,
    pub code_block_indent: Option<String>,
}

impl MarkdownThemeSpec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for MarkdownThemeSpec {
    fn default() -> Self {
        Self {
            heading: identity_theme_text_style(),
            link: identity_theme_text_style(),
            link_url: identity_theme_text_style(),
            code: identity_theme_text_style(),
            code_block: identity_theme_text_style(),
            code_block_border: identity_theme_text_style(),
            quote: identity_theme_text_style(),
            quote_border: identity_theme_text_style(),
            hr: identity_theme_text_style(),
            list_bullet: identity_theme_text_style(),
            bold: identity_theme_text_style(),
            italic: identity_theme_text_style(),
            strikethrough: identity_theme_text_style(),
            underline: identity_theme_text_style(),
            highlight_code: None,
            code_block_indent: None,
        }
    }
}

impl From<MarkdownThemeSpec> for markdown::MarkdownTheme {
    fn from(spec: MarkdownThemeSpec) -> Self {
        let MarkdownThemeSpec {
            heading,
            link,
            link_url,
            code,
            code_block,
            code_block_border,
            quote,
            quote_border,
            hr,
            list_bullet,
            bold,
            italic,
            strikethrough,
            underline,
            highlight_code,
            code_block_indent,
        } = spec;

        let mut theme = markdown::MarkdownTheme::new()
            .with_heading(move |text| heading(text))
            .with_link(move |text| link(text))
            .with_link_url(move |text| link_url(text))
            .with_code(move |text| code(text))
            .with_code_block(move |text| code_block(text))
            .with_code_block_border(move |text| code_block_border(text))
            .with_quote(move |text| quote(text))
            .with_quote_border(move |text| quote_border(text))
            .with_hr(move |text| hr(text))
            .with_list_bullet(move |text| list_bullet(text))
            .with_bold(move |text| bold(text))
            .with_italic(move |text| italic(text))
            .with_strikethrough(move |text| strikethrough(text))
            .with_underline(move |text| underline(text));

        if let Some(highlight_code) = highlight_code {
            theme = theme.with_highlight_code(move |code, lang| highlight_code(code, lang));
        }

        if let Some(code_block_indent) = code_block_indent {
            theme = theme.with_code_block_indent(code_block_indent);
        }

        theme
    }
}

impl From<&MarkdownThemeSpec> for markdown::MarkdownTheme {
    fn from(spec: &MarkdownThemeSpec) -> Self {
        spec.clone().into()
    }
}

/// Interface-style settings list theme matching the TypeScript `SettingsListTheme` shape.
#[derive(Clone)]
pub struct SettingsListThemeSpec {
    pub label: SharedThemeSelectionTextStyle,
    pub value: SharedThemeSelectionTextStyle,
    pub description: SharedThemeTextStyle,
    pub cursor: String,
    pub hint: SharedThemeTextStyle,
}

impl SettingsListThemeSpec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for SettingsListThemeSpec {
    fn default() -> Self {
        Self {
            label: identity_theme_selection_text_style(),
            value: identity_theme_selection_text_style(),
            description: identity_theme_text_style(),
            cursor: String::from("→ "),
            hint: identity_theme_text_style(),
        }
    }
}

impl From<SettingsListThemeSpec> for settings_list::SettingsListTheme {
    fn from(spec: SettingsListThemeSpec) -> Self {
        let SettingsListThemeSpec {
            label,
            value,
            description,
            cursor,
            hint,
        } = spec;

        settings_list::SettingsListTheme::new()
            .with_label(move |text, selected| label(text, selected))
            .with_value(move |text, selected| value(text, selected))
            .with_description(move |text| description(text))
            .with_cursor(cursor)
            .with_hint(move |text| hint(text))
    }
}

impl From<&SettingsListThemeSpec> for settings_list::SettingsListTheme {
    fn from(spec: &SettingsListThemeSpec) -> Self {
        spec.clone().into()
    }
}

pub use autocomplete::{
    AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider,
    CompletionResult, SlashCommand, apply_completion,
};
pub use box_component::Box;
pub use dynamic_border::DynamicBorder;
pub use editor::{Editor, EditorCursor, EditorOptions, EditorTheme, TextChunk, word_wrap_line};
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
