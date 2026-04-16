mod bash;
mod edit;
mod find;
mod grep;
mod image;
mod ls;
mod path_utils;
mod read;
mod truncate;
mod write;

pub use bash::{bash_tool_definition, create_bash_tool};
pub use edit::{create_edit_tool, edit_tool_definition};
pub use find::{create_find_tool, find_tool_definition};
pub use grep::{create_grep_tool, grep_tool_definition};
pub use image::{
    DEFAULT_MAX_INLINE_IMAGE_BYTES, ImageResizeOptions, ResizedImage, format_dimension_note,
    resize_image_bytes,
};
pub use ls::{create_ls_tool, ls_tool_definition};
pub use path_utils::{resolve_read_path, resolve_to_cwd};
pub use read::{
    create_read_tool, create_read_tool_with_auto_resize_flag, detect_supported_image_mime_type,
    read_tool_definition,
};
pub use truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncationOptions, TruncationResult, format_size,
    truncate_head, truncate_tail,
};
pub use write::{create_write_tool, write_tool_definition};

use pi_agent::AgentTool;
use pi_events::ToolDefinition;
use std::{collections::BTreeMap, path::PathBuf};

pub fn create_coding_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        read_tool_definition(),
        bash_tool_definition(),
        edit_tool_definition(),
        write_tool_definition(),
    ]
}

pub fn create_read_only_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        read_tool_definition(),
        grep_tool_definition(),
        find_tool_definition(),
        ls_tool_definition(),
    ]
}

pub fn create_all_tool_definitions() -> BTreeMap<String, ToolDefinition> {
    BTreeMap::from([
        (String::from("read"), read_tool_definition()),
        (String::from("bash"), bash_tool_definition()),
        (String::from("edit"), edit_tool_definition()),
        (String::from("write"), write_tool_definition()),
        (String::from("grep"), grep_tool_definition()),
        (String::from("find"), find_tool_definition()),
        (String::from("ls"), ls_tool_definition()),
    ])
}

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

pub fn create_read_only_tools(cwd: impl Into<PathBuf>) -> Vec<AgentTool> {
    let cwd = cwd.into();
    vec![
        create_read_tool(cwd.clone()),
        create_grep_tool(cwd.clone()),
        create_find_tool(cwd.clone()),
        create_ls_tool(cwd),
    ]
}

pub fn create_all_tools(cwd: impl Into<PathBuf>) -> BTreeMap<String, AgentTool> {
    let cwd = cwd.into();
    BTreeMap::from([
        (String::from("read"), create_read_tool(cwd.clone())),
        (String::from("bash"), create_bash_tool(cwd.clone())),
        (String::from("edit"), create_edit_tool(cwd.clone())),
        (String::from("write"), create_write_tool(cwd.clone())),
        (String::from("grep"), create_grep_tool(cwd.clone())),
        (String::from("find"), create_find_tool(cwd.clone())),
        (String::from("ls"), create_ls_tool(cwd)),
    ])
}

pub fn create_coding_tools_with_read_auto_resize_flag(
    cwd: impl Into<PathBuf>,
    auto_resize_images: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Vec<AgentTool> {
    let cwd = cwd.into();
    vec![
        create_read_tool_with_auto_resize_flag(cwd.clone(), auto_resize_images),
        create_bash_tool(cwd.clone()),
        create_edit_tool(cwd.clone()),
        create_write_tool(cwd),
    ]
}
