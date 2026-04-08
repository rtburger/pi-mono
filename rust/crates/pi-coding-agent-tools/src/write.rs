use crate::path_utils::resolve_to_cwd;
use pi_agent::{AgentTool, AgentToolError, AgentToolResult};
use pi_events::{ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::{fs, path::Path, path::PathBuf};

pub fn write_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "write".into(),
        description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path to the file to write (relative or absolute)"},
                "content": {"type": "string", "description": "Content to write to the file"}
            },
            "required": ["path", "content"],
            "additionalProperties": false
        }),
    }
}

pub fn create_write_tool(cwd: impl Into<PathBuf>) -> AgentTool {
    let cwd = cwd.into();
    AgentTool::new(
        write_tool_definition(),
        move |_tool_call_id, args, signal| {
            let cwd = cwd.clone();
            async move { execute_write(&cwd, args, signal.as_ref()) }
        },
    )
}

fn execute_write(
    cwd: &Path,
    args: Value,
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<AgentToolResult, AgentToolError> {
    abort_if_requested(signal)?;

    let path = string_arg(&args, &["path", "file_path"])?;
    let content = string_arg(&args, &["content"])?;
    let absolute_path = resolve_to_cwd(&path, cwd);

    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    abort_if_requested(signal)?;

    fs::write(&absolute_path, content.as_bytes()).map_err(io_error)?;
    abort_if_requested(signal)?;

    Ok(AgentToolResult {
        content: vec![UserContent::Text {
            text: format!(
                "Successfully wrote {} bytes to {}",
                js_string_length(&content),
                path
            ),
        }],
        details: Value::Null,
    })
}

fn js_string_length(value: &str) -> usize {
    value.encode_utf16().count()
}

fn string_arg(args: &Value, keys: &[&str]) -> Result<String, AgentToolError> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            AgentToolError::message(format!(
                "Missing required string argument {}",
                keys.join(" or ")
            ))
        })
}

fn abort_if_requested(
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<(), AgentToolError> {
    if signal.map(|signal| *signal.borrow()).unwrap_or(false) {
        return Err(AgentToolError::message("Operation aborted"));
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> AgentToolError {
    AgentToolError::message(error.to_string())
}
