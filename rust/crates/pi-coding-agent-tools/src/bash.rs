use crate::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncatedBy, TruncationOptions, format_size,
    truncate_tail,
};
use pi_agent::{AgentTool, AgentToolError, AgentToolResult};
use pi_events::{ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::{
    env, fs,
    future::pending,
    path::{Path, PathBuf},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{process::Command, sync::watch, time::Duration};

pub fn bash_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "bash".into(),
        description: format!(
            "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last {DEFAULT_MAX_LINES} lines or {}KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.",
            DEFAULT_MAX_BYTES / 1024
        ),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Bash command to execute"},
                "timeout": {"type": "number", "description": "Timeout in seconds (optional, no default timeout)"}
            },
            "required": ["command"],
            "additionalProperties": false
        }),
    }
}

pub fn create_bash_tool(cwd: impl Into<PathBuf>) -> AgentTool {
    let cwd = cwd.into();
    AgentTool::new(
        bash_tool_definition(),
        move |_tool_call_id, args, signal| {
            let cwd = cwd.clone();
            async move { execute_bash(&cwd, args, signal).await }
        },
    )
}

async fn execute_bash(
    cwd: &Path,
    args: Value,
    signal: Option<watch::Receiver<bool>>,
) -> Result<AgentToolResult, AgentToolError> {
    abort_if_requested(signal.as_ref())?;

    if !cwd.exists() {
        return Err(AgentToolError::message(format!(
            "Working directory does not exist: {}\nCannot execute bash commands.",
            cwd.display()
        )));
    }

    let command = string_arg(&args, &["command"])?;
    let timeout_seconds = optional_non_negative_number_arg(&args, "timeout")?;
    let shell = env::var("SHELL").unwrap_or_else(|_| String::from("sh"));
    let wrapped_command = format!("{{\n{command}\n}} 2>&1");

    let mut command_builder = Command::new(shell);
    command_builder
        .arg("-lc")
        .arg(wrapped_command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = command_builder.spawn().map_err(io_error)?;
    let output_future = child.wait_with_output();
    tokio::pin!(output_future);

    let mut signal = signal;
    let abort_future = async {
        match signal.as_mut() {
            Some(signal) => wait_for_abort(signal).await,
            None => pending::<()>().await,
        }
    };
    tokio::pin!(abort_future);

    let timeout_future = async {
        match timeout_seconds {
            Some(timeout_seconds) if timeout_seconds > 0.0 => {
                tokio::time::sleep(Duration::from_secs_f64(timeout_seconds)).await
            }
            _ => pending::<()>().await,
        }
    };
    tokio::pin!(timeout_future);

    let output = tokio::select! {
        output = &mut output_future => output.map_err(io_error)?,
        _ = &mut abort_future => {
            return Err(AgentToolError::message("Command aborted"));
        }
        _ = &mut timeout_future => {
            return Err(AgentToolError::message(format!(
                "Command timed out after {} seconds",
                format_timeout(timeout_seconds.unwrap_or_default())
            )));
        }
    };

    let mut full_output = String::from_utf8_lossy(&output.stdout).into_owned();
    full_output.push_str(&String::from_utf8_lossy(&output.stderr));
    full_output = full_output.replace('\r', "");

    let truncation = truncate_tail(&full_output, TruncationOptions::default());
    let full_output_path = if truncation.truncated {
        Some(write_full_output(&full_output)?)
    } else {
        None
    };

    let mut output_text = if truncation.truncated {
        if truncation.content.is_empty() {
            String::from("(no output)")
        } else {
            truncation.content.clone()
        }
    } else if full_output.is_empty() {
        String::from("(no output)")
    } else {
        full_output.clone()
    };

    if let Some(full_output_path) = &full_output_path {
        let start_line = truncation
            .total_lines
            .saturating_sub(truncation.output_lines)
            + 1;
        let end_line = truncation.total_lines.max(start_line);
        if truncation.last_line_partial {
            let last_line_size = format_size(
                full_output
                    .split('\n')
                    .next_back()
                    .unwrap_or_default()
                    .len(),
            );
            output_text.push_str(&format!(
                "\n\n[Showing last {} of line {end_line} (line is {last_line_size}). Full output: {full_output_path}]",
                format_size(truncation.output_bytes)
            ));
        } else if matches!(truncation.truncated_by, Some(TruncatedBy::Lines)) {
            output_text.push_str(&format!(
                "\n\n[Showing lines {start_line}-{end_line} of {}. Full output: {full_output_path}]",
                truncation.total_lines
            ));
        } else {
            output_text.push_str(&format!(
                "\n\n[Showing lines {start_line}-{end_line} of {} ({} limit). Full output: {full_output_path}]",
                truncation.total_lines,
                format_size(DEFAULT_MAX_BYTES)
            ));
        }
    }

    let exit_code = output.status.code().unwrap_or(1);
    if exit_code != 0 {
        output_text.push_str(&format!("\n\nCommand exited with code {exit_code}"));
        return Err(AgentToolError::message(output_text));
    }

    Ok(AgentToolResult {
        content: vec![UserContent::Text { text: output_text }],
        details: match full_output_path {
            Some(full_output_path) => json!({
                "truncation": truncation,
                "fullOutputPath": full_output_path,
            }),
            None => Value::Null,
        },
    })
}

async fn wait_for_abort(signal: &mut watch::Receiver<bool>) {
    if *signal.borrow() {
        return;
    }

    while signal.changed().await.is_ok() {
        if *signal.borrow() {
            return;
        }
    }
}

fn write_full_output(output: &str) -> Result<String, AgentToolError> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-bash-{}-{unique}.log", std::process::id()));
    fs::write(&path, output).map_err(io_error)?;
    Ok(path.display().to_string())
}

fn format_timeout(timeout_seconds: f64) -> String {
    if timeout_seconds.fract() == 0.0 {
        format!("{}", timeout_seconds as u64)
    } else {
        let mut text = timeout_seconds.to_string();
        while text.contains('.') && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        text
    }
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

fn optional_non_negative_number_arg(
    args: &Value,
    key: &str,
) -> Result<Option<f64>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };

    let Some(number) = value.as_f64() else {
        return Err(AgentToolError::message(format!(
            "Argument \"{key}\" must be a non-negative number"
        )));
    };

    if number < 0.0 || !number.is_finite() {
        return Err(AgentToolError::message(format!(
            "Argument \"{key}\" must be a non-negative number"
        )));
    }

    Ok(Some(number))
}

fn abort_if_requested(signal: Option<&watch::Receiver<bool>>) -> Result<(), AgentToolError> {
    if signal.map(|signal| *signal.borrow()).unwrap_or(false) {
        return Err(AgentToolError::message("Operation aborted"));
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> AgentToolError {
    AgentToolError::message(error.to_string())
}
