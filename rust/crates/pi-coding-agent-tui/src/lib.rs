mod assistant_message;
mod branch_summary;
mod clipboard_image;
mod compaction_summary;
mod custom_message;
mod footer;
mod keybinding_hints;
mod keybindings;
mod pending_messages;
mod skill_invocation;
mod startup_header;
mod startup_shell;
mod tool_execution;
mod transcript;
mod user_message;

pub use assistant_message::{AssistantMessageComponent, DEFAULT_HIDDEN_THINKING_LABEL};
pub use branch_summary::BranchSummaryMessageComponent;
pub use clipboard_image::{
    ClipboardCommandRunner, ClipboardImage, ClipboardImageSource, ClipboardPlatform, CommandOutput,
    StdClipboardCommandRunner, SystemClipboardImageSource, extension_for_image_mime_type,
    is_wayland_session, paste_clipboard_image_into_shell,
};
pub use compaction_summary::CompactionSummaryMessageComponent;
pub use custom_message::CustomMessageComponent;
pub use footer::{FooterComponent, FooterState};
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
pub use startup_shell::{StartupShellComponent, StatusHandle};
pub use tool_execution::{
    ToolExecutionComponent, ToolExecutionOptions, ToolExecutionRendererDefinition,
    ToolExecutionResult, ToolRenderContext, ToolRenderResultOptions,
};
pub use transcript::TranscriptComponent;
pub use user_message::UserMessageComponent;

#[derive(Debug, thiserror::Error)]
pub enum CodingAgentTuiError {
    #[error("coding-agent tui migration pending")]
    Pending,
}
