use crate::{
    path_utils::resolve_read_path,
    truncate::{DEFAULT_MAX_BYTES, TruncatedBy, TruncationOptions, format_size, truncate_head},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pi_agent::{AgentTool, AgentToolError, AgentToolResult};
use pi_events::{ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::{fs, path::Path, path::PathBuf};

pub fn read_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "read".into(),
        description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path to the file to read (relative or absolute)"},
                "offset": {"type": "number", "description": "Line number to start reading from (1-indexed)"},
                "limit": {"type": "number", "description": "Maximum number of lines to read"}
            },
            "required": ["path"],
            "additionalProperties": false
        }),
    }
}

pub fn create_read_tool(cwd: impl Into<PathBuf>) -> AgentTool {
    let cwd = cwd.into();
    AgentTool::new(
        read_tool_definition(),
        move |_tool_call_id, args, signal| {
            let cwd = cwd.clone();
            async move { execute_read(&cwd, args, signal.as_ref()) }
        },
    )
}

fn execute_read(
    cwd: &Path,
    args: Value,
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<AgentToolResult, AgentToolError> {
    abort_if_requested(signal)?;

    let path = string_arg(&args, &["path", "file_path"])?;
    let offset = optional_non_negative_usize_arg(&args, "offset")?;
    let limit = optional_non_negative_usize_arg(&args, "limit")?;
    let absolute_path = resolve_read_path(&path, cwd);
    let buffer = fs::read(&absolute_path).map_err(io_error)?;

    abort_if_requested(signal)?;

    if let Some(mime_type) = detect_supported_image_mime_type(&buffer) {
        return Ok(AgentToolResult {
            content: vec![
                UserContent::Text {
                    text: format!("Read image file [{mime_type}]"),
                },
                UserContent::Image {
                    data: STANDARD.encode(buffer),
                    mime_type: mime_type.into(),
                },
            ],
            details: Value::Null,
        });
    }

    let text_content = String::from_utf8_lossy(&buffer);
    let all_lines: Vec<&str> = text_content.split('\n').collect();
    let total_file_lines = all_lines.len();
    let start_line = offset.unwrap_or(1).saturating_sub(1);
    let start_line_display = start_line + 1;

    if start_line >= all_lines.len() {
        return Err(AgentToolError::message(format!(
            "Offset {} is beyond end of file ({} lines total)",
            offset.unwrap_or(1),
            all_lines.len()
        )));
    }

    let (selected_content, user_limited_lines) = match limit {
        Some(limit) => {
            let end_line = start_line.saturating_add(limit).min(all_lines.len());
            (
                all_lines[start_line..end_line].join("\n"),
                Some(end_line.saturating_sub(start_line)),
            )
        }
        None => (all_lines[start_line..].join("\n"), None),
    };

    let truncation = truncate_head(&selected_content, TruncationOptions::default());
    let mut output_text = if truncation.first_line_exceeds_limit {
        let first_line_size = format_size(all_lines[start_line].len());
        format!(
            "[Line {start_line_display} is {first_line_size}, exceeds {} limit. Use bash: sed -n '{start_line_display}p' {} | head -c {}]",
            format_size(DEFAULT_MAX_BYTES),
            path,
            DEFAULT_MAX_BYTES
        )
    } else if truncation.truncated {
        let end_line_display = start_line_display + truncation.output_lines.saturating_sub(1);
        let next_offset = end_line_display + 1;
        let mut text = truncation.content.clone();
        match truncation.truncated_by {
            Some(TruncatedBy::Lines) => {
                text.push_str(&format!(
                    "\n\n[Showing lines {start_line_display}-{end_line_display} of {total_file_lines}. Use offset={next_offset} to continue.]"
                ));
            }
            Some(TruncatedBy::Bytes) => {
                text.push_str(&format!(
                    "\n\n[Showing lines {start_line_display}-{end_line_display} of {total_file_lines} ({} limit). Use offset={next_offset} to continue.]",
                    format_size(DEFAULT_MAX_BYTES)
                ));
            }
            None => {}
        }
        text
    } else {
        truncation.content.clone()
    };

    if !truncation.truncated {
        if let Some(user_limited_lines) = user_limited_lines {
            if start_line + user_limited_lines < all_lines.len() {
                let remaining = all_lines.len() - (start_line + user_limited_lines);
                let next_offset = start_line + user_limited_lines + 1;
                output_text.push_str(&format!(
                    "\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]"
                ));
            }
        }
    }

    let details = if truncation.truncated {
        json!({ "truncation": truncation })
    } else {
        Value::Null
    };

    Ok(AgentToolResult {
        content: vec![UserContent::Text { text: output_text }],
        details,
    })
}

fn detect_supported_image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
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

    if let Some(value) = value.as_f64() {
        if value.is_finite() && value >= 0.0 {
            return Ok(Some(value.trunc() as usize));
        }
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
