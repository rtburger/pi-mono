mod bash;
mod edit;
mod path_utils;
mod read;
mod truncate;
mod write;

pub use bash::{bash_tool_definition, create_bash_tool};
pub use edit::{create_edit_tool, edit_tool_definition};
pub use read::{create_read_tool, read_tool_definition};
pub use truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncationOptions, TruncationResult, format_size,
    truncate_head, truncate_tail,
};
pub use write::{create_write_tool, write_tool_definition};

use pi_agent::AgentTool;
use std::path::PathBuf;

pub fn create_read_write_tools(cwd: impl Into<PathBuf>) -> Vec<AgentTool> {
    let cwd = cwd.into();
    vec![create_read_tool(cwd.clone()), create_write_tool(cwd)]
}

pub fn create_coding_tools(cwd: impl Into<PathBuf>) -> Vec<AgentTool> {
    let cwd = cwd.into();
    vec![
        create_read_tool(cwd.clone()),
        create_bash_tool(cwd.clone()),
        create_edit_tool(cwd.clone()),
        create_write_tool(cwd),
    ]
}
