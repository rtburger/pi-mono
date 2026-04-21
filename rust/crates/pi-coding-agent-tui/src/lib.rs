mod assistant_message;
mod bash_execution;
mod branch_summary;
mod clipboard_image;
mod compaction_summary;
mod config_selector;
mod custom_editor;
mod custom_message;
mod dialog_countdown;
mod extension_editor;
mod extension_input;
mod extension_selector;
mod footer;
mod interactive_binding;
mod keybinding_hints;
mod keybindings;
mod login_dialog;
mod model_selector;
mod oauth_selector;
mod pending_messages;
mod scoped_models_selector;
mod selector_common;
mod session_selector;
mod session_selector_search;
mod settings_selector;
mod show_images_selector;
mod skill_invocation;
mod startup_header;
mod startup_shell;
mod theme;
mod theme_selector;
mod thinking_selector;
mod tool_execution;
mod transcript;
mod tree_selector;
mod user_message;

pub use assistant_message::{AssistantMessageComponent, DEFAULT_HIDDEN_THINKING_LABEL};
pub use bash_execution::{BashExecutionComponent, BashExecutionHandle};
pub use branch_summary::BranchSummaryMessageComponent;
pub use clipboard_image::{
    ClipboardCommandRunner, ClipboardImage, ClipboardImageSource, ClipboardPlatform, CommandOutput,
    StdClipboardCommandRunner, SystemClipboardImageSource, extension_for_image_mime_type,
    is_wayland_session, paste_clipboard_image_into_shell,
};
pub use compaction_summary::CompactionSummaryMessageComponent;
pub use config_selector::{
    ConfigResourceGroup, ConfigResourceItem, ConfigResourceSubgroup, ConfigResourceType,
    ConfigSelectorComponent,
};
pub use custom_editor::CustomEditor;
pub use custom_message::CustomMessageComponent;
pub use extension_editor::{
    ExtensionEditorComponent, ExternalEditorCommandRunner, ExternalEditorHost,
};
pub use extension_input::ExtensionInputComponent;
pub use extension_selector::ExtensionSelectorComponent;
pub use footer::{FooterComponent, FooterState, FooterStateHandle};
pub use interactive_binding::InteractiveCoreBinding;
pub use keybinding_hints::{KeyHintStyler, PlainKeyHintStyler, key_hint, key_text, raw_key_hint};
pub use keybindings::{
    DEFAULT_APP_KEYBINDINGS, KeybindingsManager, MigrateKeybindingsConfigResult,
    migrate_keybindings_config, migrate_keybindings_file,
};
pub use login_dialog::LoginDialogComponent;
pub use model_selector::ModelSelectorComponent;
pub use oauth_selector::{OAuthProviderItem, OAuthSelectorComponent, OAuthSelectorMode};
pub use pending_messages::PendingMessagesComponent;
pub use pi_tui::{KeyId, KeybindingConflict, KeybindingDefinition, KeybindingsConfig};
pub use scoped_models_selector::{ScopedModelsConfig, ScopedModelsSelectorComponent};
pub use session_selector::{SessionSelectorComponent, SessionSelectorScope};
pub use session_selector_search::{
    MatchResult as SessionSearchMatchResult, NameFilter as SessionNameFilter,
    ParsedSearchQuery as SessionParsedSearchQuery, SearchMode as SessionSearchMode,
    SearchToken as SessionSearchToken, SearchTokenKind as SessionSearchTokenKind,
    SortMode as SessionSortMode, filter_and_sort_sessions as filter_and_sort_sessions_list,
    has_session_name, match_session, parse_search_query,
};
pub use settings_selector::{
    DeliveryMode, DoubleEscapeAction, SettingsChange, SettingsConfig, SettingsSelectorComponent,
};
pub use show_images_selector::ShowImagesSelectorComponent;
pub use skill_invocation::SkillInvocationMessageComponent;
pub use startup_header::{
    BuiltInHeaderComponent, StartupHeaderComponent, StartupHeaderStyler,
    build_condensed_changelog_notice, build_startup_header_text,
};
pub use startup_shell::{ShellUpdateHandle, StartupShellComponent, StatusHandle};
pub use theme::{
    ColorMode, LoadThemesOptions, LoadThemesResult, Theme, ThemeExportColors, ThemeInfo,
    ThemeSelectionResult, ThemedKeyHintStyler, current_theme, current_theme_name,
    get_available_themes, get_available_themes_with_paths, get_resolved_theme_colors,
    get_theme_by_name, get_theme_export_colors, init_theme, is_light_theme, load_theme_from_path,
    load_theme_from_path_with_mode, load_themes, markdown_theme, set_registered_themes, set_theme,
    set_theme_instance,
};
pub use theme_selector::ThemeSelectorComponent;
pub use thinking_selector::ThinkingSelectorComponent;
pub use tool_execution::{
    ToolExecutionComponent, ToolExecutionOptions, ToolExecutionRendererDefinition,
    ToolExecutionResult, ToolRenderContext, ToolRenderResultOptions,
};
pub use transcript::TranscriptComponent;
pub use tree_selector::{TreeFilterMode, TreeSelectorComponent};
pub use user_message::UserMessageComponent;

#[derive(Debug, thiserror::Error)]
pub enum CodingAgentTuiError {
    #[error("coding-agent tui migration pending")]
    Pending,
}
