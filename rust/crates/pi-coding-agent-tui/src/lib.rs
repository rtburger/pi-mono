mod keybinding_hints;
mod keybindings;
mod startup_header;

pub use keybinding_hints::{KeyHintStyler, PlainKeyHintStyler, key_hint, key_text, raw_key_hint};
pub use keybindings::{
    DEFAULT_APP_KEYBINDINGS, KeybindingsManager, MigrateKeybindingsConfigResult,
    migrate_keybindings_config, migrate_keybindings_file,
};
pub use pi_tui::{KeyId, KeybindingConflict, KeybindingDefinition, KeybindingsConfig};
pub use startup_header::{
    BuiltInHeaderComponent, StartupHeaderComponent, StartupHeaderStyler,
    build_condensed_changelog_notice, build_startup_header_text,
};

#[derive(Debug, thiserror::Error)]
pub enum CodingAgentTuiError {
    #[error("coding-agent tui migration pending")]
    Pending,
}
