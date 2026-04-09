pub mod fuzzy;
pub mod keybindings;
pub mod keys;
pub mod stdin_buffer;
pub mod terminal;

pub use fuzzy::{FuzzyMatch, fuzzy_filter, fuzzy_match};
pub use keybindings::{
    KeyId, KeybindingConflict, KeybindingDefinition, KeybindingsConfig, KeybindingsManager,
    TUI_KEYBINDINGS,
};
pub use keys::{
    KeyEventType, decode_kitty_printable, is_key_release, is_key_repeat, is_kitty_protocol_active,
    matches_key, parse_key, set_kitty_protocol_active,
};
pub use stdin_buffer::{StdinBuffer, StdinBufferEvent, StdinBufferOptions};
pub use terminal::{ProcessTerminal, Terminal};

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("tui migration pending")]
    Pending,
}
