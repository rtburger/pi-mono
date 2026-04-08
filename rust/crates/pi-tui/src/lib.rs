pub mod fuzzy;
pub mod keybindings;

pub use fuzzy::{FuzzyMatch, fuzzy_filter, fuzzy_match};
pub use keybindings::{
    KeyId, KeybindingConflict, KeybindingDefinition, KeybindingsConfig, KeybindingsManager,
    TUI_KEYBINDINGS,
};

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("tui migration pending")]
    Pending,
}
