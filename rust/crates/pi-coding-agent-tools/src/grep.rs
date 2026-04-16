use crate::truncate::{DEFAULT_MAX_BYTES, TruncationOptions, format_size, truncate_head};
use pi_agent::{AgentTool, AgentToolError, AgentToolResult};
use pi_events::{ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const DEFAULT_LIMIT: usize = 100;
const DEFAULT_LINE_LIMIT: usize = 300;

pub fn grep_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "grep".into(),
        description: format!(
            "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore when ripgrep is available. Output is truncated to {DEFAULT_LIMIT} matches or {}KB (whichever is hit first).",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Search pattern (regex or literal string)"},
                "path": {"type": "string", "description": "Directory or file to search (default: current directory)"},
                "glob": {"type": "string", "description": "Filter files by glob pattern"},
                "ignoreCase": {"type": "boolean", "description": "Case-insensitive search"},
                "literal": {"type": "boolean", "description": "Treat pattern as a literal string"},
                "context": {"type": "number", "description": "Number of context lines to include"},
                "limit": {"type": "number", "description": "Maximum number of matches to return (default: 100)"}
            },
            "required": ["pattern"],
            "additionalProperties": false
        }),
    }
}

pub fn create_grep_tool(cwd: impl Into<PathBuf>) -> AgentTool {
    let cwd = cwd.into();
    AgentTool::new(
        grep_tool_definition(),
        move |_tool_call_id, args, signal| {
            let cwd = cwd.clone();
            async move { execute_grep(&cwd, args, signal.as_ref()) }
        },
    )
}

fn execute_grep(
    cwd: &Path,
    args: Value,
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<AgentToolResult, AgentToolError> {
    abort_if_requested(signal)?;

    let pattern = string_arg(&args, "pattern")?;
    let path = optional_string_arg(&args, "path").unwrap_or_else(|| String::from("."));
    let glob = optional_string_arg(&args, "glob");
    let ignore_case = args
        .get("ignoreCase")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let literal = args
        .get("literal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let context = optional_non_negative_usize_arg(&args, "context")?.unwrap_or(0);
    let limit = optional_non_negative_usize_arg(&args, "limit")?.unwrap_or(DEFAULT_LIMIT);
    let search_root = resolve_to_cwd(&path, cwd);

    if !search_root.exists() {
        return Err(AgentToolError::message(format!(
            "Path not found: {}",
            search_root.display()
        )));
    }

    let mut command = Command::new("rg");
    command
        .arg("--line-number")
        .arg("--color=never")
        .arg("--hidden");
    if ignore_case {
        command.arg("--ignore-case");
    }
    if literal {
        command.arg("--fixed-strings");
    }
    if context > 0 {
        command.arg("--context").arg(context.to_string());
    }
    if let Some(glob) = glob.as_deref() {
        command.arg("--glob").arg(glob);
    }
    command.arg(pattern).arg(&search_root);

    let output = command
        .output()
        .map_err(|error| AgentToolError::message(format!("Failed to run ripgrep (rg): {error}")))?;

    abort_if_requested(signal)?;

    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(AgentToolError::message(if stderr.is_empty() {
            format!("ripgrep exited with code {}", output.status)
        } else {
            stderr
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let mut lines = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(limit.max(DEFAULT_LINE_LIMIT))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut match_limit_reached = false;
    if lines.len() > limit {
        lines.truncate(limit);
        match_limit_reached = true;
    }
    if lines.is_empty() {
        return Ok(AgentToolResult {
            content: vec![UserContent::Text {
                text: String::from("No matches found"),
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
    if match_limit_reached {
        notices.push(format!(
            "{limit} matches limit reached. Use limit={} for more, or refine pattern",
            limit.saturating_mul(2)
        ));
        details.insert(String::from("matchLimitReached"), Value::from(limit as u64));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert(
            String::from("truncation"),
            serde_json::to_value(&truncation).expect("grep truncation should serialize"),
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

fn string_arg(args: &Value, key: &str) -> Result<String, AgentToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| AgentToolError::message(format!("Missing required string argument {key}")))
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
