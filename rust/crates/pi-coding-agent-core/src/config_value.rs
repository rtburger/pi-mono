use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};

pub fn resolve_config_value_uncached(config: &str) -> Option<String> {
    if let Some(command) = config.strip_prefix('!') {
        return execute_command_uncached(command);
    }

    std::env::var(config)
        .ok()
        .or_else(|| Some(config.to_string()))
}

pub fn resolve_config_value_or_err(config: &str, description: &str) -> Result<String, String> {
    if let Some(resolved) = resolve_config_value_uncached(config) {
        return Ok(resolved);
    }

    if let Some(command) = config.strip_prefix('!') {
        return Err(format!(
            "Failed to resolve {description} from shell command: {command}"
        ));
    }

    Err(format!("Failed to resolve {description}"))
}

pub fn resolve_headers_or_err(
    headers: Option<&BTreeMap<String, String>>,
    description: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(headers) = headers else {
        return Ok(None);
    };

    let mut resolved = BTreeMap::new();
    for (key, value) in headers {
        resolved.insert(
            key.clone(),
            resolve_config_value_or_err(value, &format!("{description} header \"{key}\""))?,
        );
    }

    Ok((!resolved.is_empty()).then_some(resolved))
}

fn execute_command_uncached(command: &str) -> Option<String> {
    #[cfg(windows)]
    let output = Command::new("cmd")
        .args(["/C", command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    #[cfg(not(windows))]
    let output = Command::new("sh")
        .args(["-lc", command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
