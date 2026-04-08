mod bash;
mod edit;
mod image;
mod path_utils;
mod read;
mod truncate;
mod write;

pub use bash::{bash_tool_definition, create_bash_tool};
pub use edit::{create_edit_tool, edit_tool_definition};
pub use image::{
    DEFAULT_MAX_INLINE_IMAGE_BYTES, ImageResizeOptions, ResizedImage, format_dimension_note,
    resize_image_bytes,
};
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
