use crate::truncate::{DEFAULT_MAX_BYTES, TruncationOptions, format_size, truncate_head};
use pi_agent::{AgentTool, AgentToolError, AgentToolResult};
use pi_events::{ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

const DEFAULT_LIMIT: usize = 1000;

pub fn find_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "find".into(),
        description: format!(
            "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore-style hidden directories by skipping .git and node_modules. Output is truncated to {DEFAULT_LIMIT} results or {}KB (whichever is hit first).",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern to match files, e.g. '*.ts' or 'src/**/*.rs'"},
                "path": {"type": "string", "description": "Directory to search in (default: current directory)"},
                "limit": {"type": "number", "description": "Maximum number of results (default: 1000)"}
            },
            "required": ["pattern"],
            "additionalProperties": false
        }),
    }
}

pub fn create_find_tool(cwd: impl Into<PathBuf>) -> AgentTool {
    let cwd = cwd.into();
    AgentTool::new(
        find_tool_definition(),
        move |_tool_call_id, args, signal| {
            let cwd = cwd.clone();
            async move { execute_find(&cwd, args, signal.as_ref()) }
        },
    )
}

fn execute_find(
    cwd: &Path,
    args: Value,
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<AgentToolResult, AgentToolError> {
    abort_if_requested(signal)?;

    let pattern = string_arg(&args, "pattern")?;
    let path = optional_string_arg(&args, "path").unwrap_or_else(|| String::from("."));
    let limit = optional_non_negative_usize_arg(&args, "limit")?.unwrap_or(DEFAULT_LIMIT);
    let search_root = resolve_to_cwd(&path, cwd);

    if !search_root.exists() {
        return Err(AgentToolError::message(format!(
            "Path not found: {}",
            search_root.display()
        )));
    }
    if !search_root.is_dir() {
        return Err(AgentToolError::message(format!(
            "Not a directory: {}",
            search_root.display()
        )));
    }

    let mut results = Vec::new();
    visit_find_paths(
        &search_root,
        &search_root,
        &pattern,
        limit,
        signal,
        &mut results,
    )?;
    results.sort();

    if results.is_empty() {
        return Ok(AgentToolResult {
            content: vec![UserContent::Text {
                text: String::from("No files found matching pattern"),
            }],
            details: Value::Null,
        });
    }

    let result_limit_reached = results.len() >= limit;
    let truncation = truncate_head(
        &results.join("\n"),
        TruncationOptions {
            max_lines: Some(usize::MAX),
            max_bytes: Some(DEFAULT_MAX_BYTES),
        },
    );
    let mut output = truncation.content.clone();
    let mut details = serde_json::Map::new();
    let mut notices = Vec::new();
    if result_limit_reached {
        notices.push(format!(
            "{limit} results limit reached. Use limit={} for more, or refine pattern",
            limit.saturating_mul(2)
        ));
        details.insert(
            String::from("resultLimitReached"),
            Value::from(limit as u64),
        );
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert(
            String::from("truncation"),
            serde_json::to_value(&truncation).expect("find truncation should serialize"),
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

fn visit_find_paths(
    root: &Path,
    current: &Path,
    pattern: &str,
    limit: usize,
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
    results: &mut Vec<String>,
) -> Result<(), AgentToolError> {
    abort_if_requested(signal)?;
    if results.len() >= limit {
        return Ok(());
    }

    let entries = fs::read_dir(current).map_err(io_error)?;
    for entry in entries.flatten() {
        abort_if_requested(signal)?;
        if results.len() >= limit {
            break;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name == ".git" || name == "node_modules" {
                continue;
            }
            visit_find_paths(root, &path, pattern, limit, signal, results)?;
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
        if glob_match(pattern, &relative) || glob_match(pattern, &name) {
            results.push(relative);
        }
    }
    Ok(())
}

fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern.starts_with(b"**/") {
        return glob_match_inner(&pattern[3..], text)
            || (!text.is_empty() && glob_match_inner(pattern, skip_to_next_path_segment(text)));
    }
    match pattern[0] {
        b'*' => {
            glob_match_inner(&pattern[1..], text)
                || (!text.is_empty() && text[0] != b'/' && glob_match_inner(pattern, &text[1..]))
        }
        b'?' => !text.is_empty() && text[0] != b'/' && glob_match_inner(&pattern[1..], &text[1..]),
        byte => !text.is_empty() && byte == text[0] && glob_match_inner(&pattern[1..], &text[1..]),
    }
}

fn skip_to_next_path_segment(text: &[u8]) -> &[u8] {
    match text.iter().position(|byte| *byte == b'/') {
        Some(index) if index + 1 < text.len() => &text[index + 1..],
        _ => &[],
    }
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

fn io_error(error: std::io::Error) -> AgentToolError {
    AgentToolError::message(error.to_string())
}
