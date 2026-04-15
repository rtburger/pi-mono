pub mod args;
pub mod auth;
pub mod file_processor;
pub mod initial_message;
mod list_models;
pub mod print_mode;
pub mod runner;
mod session_picker;
mod tree_picker;

pub use args::{
    AppMode, Args, Diagnostic, DiagnosticKind, ListModels, Mode, PrintOutputMode, ToolName,
    UnknownFlagValue, is_valid_thinking_level, parse_args, parse_thinking_level, resolve_app_mode,
    to_print_output_mode,
};
pub use auth::{EnvAuthSource, OverlayAuthSource};
pub use file_processor::{ProcessFileOptions, ProcessedFiles, process_file_arguments};
pub use initial_message::{InitialMessageResult, build_initial_message};
pub use pi_coding_agent_core::{AuthFileSource, ChainedAuthSource};
pub use print_mode::{PrintModeOptions, PrintModeRunResult, run_print_mode};
pub use runner::{
    RunCommandOptions, RunCommandResult, finalize_system_prompt, run_command,
    run_interactive_command, run_interactive_command_with_terminal,
};
