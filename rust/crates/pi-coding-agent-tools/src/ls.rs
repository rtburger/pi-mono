use crate::truncate::{DEFAULT_MAX_BYTES, TruncationOptions, format_size, truncate_head};
use pi_agent::{AgentTool, AgentToolError, AgentToolResult};
use pi_events::{ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

const DEFAULT_LIMIT: usize = 500;

pub fn ls_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "ls".into(),
        description: format!(
            "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to {DEFAULT_LIMIT} entries or {}KB (whichever is hit first).",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory to list (default: current directory)"},
                "limit": {"type": "number", "description": "Maximum number of entries to return (default: 500)"}
            },
            "additionalProperties": false
        }),
    }
}

pub fn create_ls_tool(cwd: impl Into<PathBuf>) -> AgentTool {
    let cwd = cwd.into();
    AgentTool::new(ls_tool_definition(), move |_tool_call_id, args, signal| {
        let cwd = cwd.clone();
        async move { execute_ls(&cwd, args, signal.as_ref()) }
    })
}

fn execute_ls(
    cwd: &Path,
    args: Value,
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<AgentToolResult, AgentToolError> {
    abort_if_requested(signal)?;

    let path = optional_string_arg(&args, "path").unwrap_or_else(|| String::from("."));
    let limit = optional_non_negative_usize_arg(&args, "limit")?.unwrap_or(DEFAULT_LIMIT);
    let directory = resolve_to_cwd(&path, cwd);

    if !directory.exists() {
        return Err(AgentToolError::message(format!(
            "Path not found: {}",
            directory.display()
        )));
    }
    if !directory.is_dir() {
        return Err(AgentToolError::message(format!(
            "Not a directory: {}",
            directory.display()
        )));
    }

    let mut entries = fs::read_dir(&directory)
        .map_err(io_error)?
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
            .cmp(
                &right
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase(),
            )
    });

    let mut lines = Vec::new();
    let mut entry_limit_reached = false;
    for path in entries {
        if lines.len() >= limit {
            entry_limit_reached = true;
            break;
        }
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let suffix = if path.is_dir() { "/" } else { "" };
        lines.push(format!("{name}{suffix}"));
    }

    if lines.is_empty() {
        return Ok(AgentToolResult {
            content: vec![UserContent::Text {
                text: String::from("(empty directory)"),
            }],
            details: Value::Null,
        });
    }

    let truncation = truncate_head(
        &lines.join("\n"),
        TruncationOptions {
            max_lines: Some(usize::MAX),
            max_bytes: Some(DEFAULT_MAX_BYTES),
        },
    );
    let mut output = truncation.content.clone();
    let mut details = serde_json::Map::new();
    let mut notices = Vec::new();
    if entry_limit_reached {
        notices.push(format!(
            "{limit} entries limit reached. Use limit={} for more",
            limit.saturating_mul(2)
        ));
        details.insert(String::from("entryLimitReached"), Value::from(limit as u64));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert(
            String::from("truncation"),
            serde_json::to_value(&truncation).expect("ls truncation should serialize"),
        );
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join(". ")));
    }

    Ok(AgentToolResult {
        content: vec![UserContent::Text { text: output }],
        details: if details.is_empty() {
            Value::Null
        } else {
            Value::Object(details)
        },
    })
}

fn resolve_to_cwd(path: &str, cwd: &Path) -> PathBuf {
    let resolved = Path::new(path);
    if resolved.is_absolute() {
        resolved.to_path_buf()
    } else {
        cwd.join(resolved)
    }
}

fn optional_string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn optional_non_negative_usize_arg(
    args: &Value,
    key: &str,
) -> Result<Option<usize>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if let Some(value) = value.as_u64() {
        return Ok(Some(value as usize));
    }
    if let Some(value) = value.as_f64()
        && value.is_finite()
        && value >= 0.0
    {
        return Ok(Some(value.trunc() as usize));
    }
    Err(AgentToolError::message(format!(
        "Argument \"{key}\" must be a non-negative number"
    )))
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
