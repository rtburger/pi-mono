mod assistant_message;
mod branch_summary;
mod compaction_summary;
mod keybinding_hints;
mod keybindings;
mod pending_messages;
mod skill_invocation;
mod startup_header;
mod startup_shell;
mod transcript;
mod user_message;

pub use assistant_message::{AssistantMessageComponent, DEFAULT_HIDDEN_THINKING_LABEL};
pub use branch_summary::BranchSummaryMessageComponent;
pub use compaction_summary::CompactionSummaryMessageComponent;
pub use keybinding_hints::{KeyHintStyler, PlainKeyHintStyler, key_hint, key_text, raw_key_hint};
pub use keybindings::{
    DEFAULT_APP_KEYBINDINGS, KeybindingsManager, MigrateKeybindingsConfigResult,
    migrate_keybindings_config, migrate_keybindings_file,
};
pub use pending_messages::PendingMessagesComponent;
pub use pi_tui::{KeyId, KeybindingConflict, KeybindingDefinition, KeybindingsConfig};
pub use skill_invocation::SkillInvocationMessageComponent;
pub use startup_header::{
    BuiltInHeaderComponent, StartupHeaderComponent, StartupHeaderStyler,
    build_condensed_changelog_notice, build_startup_header_text,
};
pub use startup_shell::StartupShellComponent;
pub use transcript::TranscriptComponent;
pub use user_message::UserMessageComponent;

#[derive(Debug, thiserror::Error)]
pub enum CodingAgentTuiError {
    #[error("coding-agent tui migration pending")]
    Pending,
}
