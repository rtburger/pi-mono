use super::TextEmitter;
use crate::package_manager::{DefaultPackageManager, ResolveExtensionSourcesOptions};
use pi_coding_agent_core::SourceInfo;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex as AsyncMutex, oneshot},
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionCommandInfo {
    pub name: String,
    pub description: Option<String>,
    pub source_info: SourceInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionResourcePath {
    pub path: String,
    pub extension_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RpcExtensionDiagnostic {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionInitOutput {
    pub extension_count: usize,
    pub commands: Vec<RpcExtensionCommandInfo>,
    #[serde(default)]
    pub skill_paths: Vec<RpcExtensionResourcePath>,
    #[serde(default)]
    pub prompt_paths: Vec<RpcExtensionResourcePath>,
    #[serde(default)]
    pub theme_paths: Vec<RpcExtensionResourcePath>,
    #[serde(default)]
    pub diagnostics: Vec<RpcExtensionDiagnostic>,
}

pub struct RpcExtensionHostStartOptions {
    pub cwd: PathBuf,
    pub agent_dir: Option<PathBuf>,
    pub extension_paths: Vec<String>,
    pub no_extensions: bool,
    pub flag_values: Value,
    pub state: Value,
    pub session_start_reason: String,
    pub previous_session_file: Option<String>,
    pub stdout_emitter: TextEmitter,
    pub stderr_emitter: TextEmitter,
}

pub struct RpcExtensionHostStartResult {
    pub host: Option<RpcExtensionHost>,
    pub init: RpcExtensionInitOutput,
}

type RpcExtensionHostAppRequestFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
type RpcExtensionHostAppRequestHandler =
    Arc<dyn Fn(Value) -> RpcExtensionHostAppRequestFuture + Send + Sync>;

#[derive(Clone)]
pub struct RpcExtensionHost {
    inner: Arc<RpcExtensionHostInner>,
    commands: Arc<Vec<RpcExtensionCommandInfo>>,
}

struct RpcExtensionHostInner {
    stdin: AsyncMutex<ChildStdin>,
    child: AsyncMutex<Option<Child>>,
    pending: Mutex<BTreeMap<String, oneshot::Sender<Result<Value, String>>>>,
    next_request_id: AtomicUsize,
    app_request_handler: Mutex<Option<RpcExtensionHostAppRequestHandler>>,
    stdout_emitter: TextEmitter,
    stderr_emitter: TextEmitter,
}

impl RpcExtensionHost {
    pub async fn start(
        options: RpcExtensionHostStartOptions,
    ) -> Result<RpcExtensionHostStartResult, String> {
        let package_manager = options
            .agent_dir
            .as_ref()
            .map(|agent_dir| DefaultPackageManager::new(options.cwd.clone(), agent_dir.clone()));

        let resolved_configured_extensions = package_manager
            .as_ref()
            .map(|package_manager| package_manager.resolve())
            .transpose()?
            .map(|output| {
                output
                    .resolved
                    .extensions
                    .into_iter()
                    .filter(|resource| resource.enabled)
                    .map(|resource| resource.path)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let resolved_temporary_extensions = package_manager
            .as_ref()
            .filter(|_| !options.extension_paths.is_empty())
            .map(|package_manager| {
                package_manager.resolve_extension_sources(
                    &options.extension_paths,
                    ResolveExtensionSourcesOptions {
                        temporary: true,
                        local: false,
                    },
                )
            })
            .transpose()?
            .map(|output| {
                output
                    .resolved
                    .extensions
                    .into_iter()
                    .filter(|resource| resource.enabled)
                    .map(|resource| resource.path)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let resolved_extension_paths = merge_extension_paths(
            &resolved_temporary_extensions,
            &resolved_configured_extensions,
        );

        let repo_root = repo_root();
        let sidecar_path = sidecar_path();

        let mut command = Command::new("node");
        command
            .arg("--import")
            .arg("tsx")
            .arg(&sidecar_path)
            .current_dir(&repo_root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = command
            .spawn()
            .map_err(|error| format!("Failed to start extension host: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| String::from("Failed to acquire extension host stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| String::from("Failed to acquire extension host stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| String::from("Failed to acquire extension host stderr"))?;

        let inner = Arc::new(RpcExtensionHostInner {
            stdin: AsyncMutex::new(stdin),
            child: AsyncMutex::new(Some(child)),
            pending: Mutex::new(BTreeMap::new()),
            next_request_id: AtomicUsize::new(1),
            app_request_handler: Mutex::new(None),
            stdout_emitter: options.stdout_emitter.clone(),
            stderr_emitter: options.stderr_emitter.clone(),
        });
        spawn_stdout_reader(inner.clone(), stdout);
        spawn_stderr_reader(inner.clone(), stderr);

        let host = Self {
            inner,
            commands: Arc::new(Vec::new()),
        };
        let init_value = host
            .request(
                "init",
                json!({
                    "cwd": options.cwd,
                    "agentDir": options.agent_dir,
                    "extensions": resolved_extension_paths,
                    "noExtensions": options.no_extensions,
                    "flagValues": options.flag_values,
                    "state": options.state,
                    "sessionStartReason": options.session_start_reason,
                    "previousSessionFile": options.previous_session_file,
                }),
            )
            .await?;
        let init: RpcExtensionInitOutput = serde_json::from_value(init_value)
            .map_err(|error| format!("Invalid extension host init response: {error}"))?;

        if init.extension_count == 0 {
            host.shutdown().await?;
            return Ok(RpcExtensionHostStartResult { host: None, init });
        }

        let host = Self {
            inner: host.inner,
            commands: Arc::new(init.commands.clone()),
        };
        Ok(RpcExtensionHostStartResult {
            host: Some(host),
            init,
        })
    }

    pub fn commands(&self) -> Vec<RpcExtensionCommandInfo> {
        (*self.commands).clone()
    }

    pub fn set_app_request_handler<F, Fut>(&self, handler: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, String>> + Send + 'static,
    {
        let handler =
            Arc::new(move |request| Box::pin(handler(request)) as RpcExtensionHostAppRequestFuture);
        *self
            .inner
            .app_request_handler
            .lock()
            .expect("extension host app request handler mutex poisoned") = Some(handler);
    }

    pub fn is_same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.commands.iter().any(|command| command.name == name)
    }

    pub async fn execute_command(&self, name: &str, args: &str) -> Result<bool, String> {
        let value = self
            .request(
                "execute_command",
                json!({
                    "name": name,
                    "args": args,
                }),
            )
            .await?;
        Ok(value
            .get("handled")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    pub async fn before_switch(
        &self,
        reason: &str,
        target_session_file: Option<String>,
    ) -> Result<bool, String> {
        let value = self
            .request(
                "before_switch",
                json!({
                    "reason": reason,
                    "targetSessionFile": target_session_file,
                }),
            )
            .await?;
        Ok(value
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    pub async fn tool_call(
        &self,
        tool_name: &str,
        tool_call_id: &str,
        input: Value,
    ) -> Result<Option<RpcToolCallResult>, String> {
        let value = self
            .request(
                "tool_call",
                json!({
                    "toolName": tool_name,
                    "toolCallId": tool_call_id,
                    "input": input,
                }),
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| format!("Invalid extension tool_call response: {error}"))
    }

    pub async fn update_state(&self, state: Value) -> Result<(), String> {
        let _ = self
            .request("update_state", json!({ "state": state }))
            .await?;
        Ok(())
    }

    pub async fn emit_event(&self, event: Value) -> Result<(), String> {
        self.send_message(json!({ "type": "event", "event": event }))
            .await
    }

    pub async fn deliver_ui_response(&self, response: Value) -> Result<(), String> {
        self.send_message(json!({ "type": "ui_response", "response": response }))
            .await
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        let _ = self.request("shutdown", Value::Null).await;
        if let Some(mut child) = self.inner.child.lock().await.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }

    async fn request(&self, kind: &str, payload: Value) -> Result<Value, String> {
        let request_id = format!(
            "ext-{}",
            self.inner.next_request_id.fetch_add(1, Ordering::Relaxed)
        );
        let mut object = serde_json::Map::new();
        object.insert(String::from("id"), Value::String(request_id.clone()));
        object.insert(String::from("type"), Value::String(kind.to_owned()));
        if let Some(payload_object) = payload.as_object() {
            for (key, value) in payload_object {
                object.insert(key.clone(), value.clone());
            }
        }

        let (tx, rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .expect("extension host pending mutex poisoned")
            .insert(request_id.clone(), tx);
        if let Err(error) = self.send_message(Value::Object(object)).await {
            self.inner
                .pending
                .lock()
                .expect("extension host pending mutex poisoned")
                .remove(&request_id);
            return Err(error);
        }

        rx.await
            .map_err(|_| String::from("Extension host request channel closed"))?
    }

    async fn send_message(&self, payload: Value) -> Result<(), String> {
        let line = serde_json::to_string(&payload)
            .map_err(|error| format!("Failed to serialize extension host request: {error}"))?;
        let mut stdin = self.inner.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("Failed to write to extension host: {error}"))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|error| format!("Failed to write to extension host: {error}"))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("Failed to flush extension host stdin: {error}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RpcToolCallResult {
    #[serde(default)]
    pub block: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

pub fn should_start_extension_host(
    cwd: &Path,
    agent_dir: Option<&Path>,
    extension_paths: &[String],
    no_extensions: bool,
) -> bool {
    if no_extensions {
        return false;
    }
    if !extension_paths.is_empty() {
        return true;
    }

    if cwd.join(".pi").join("extensions").exists()
        || agent_dir
            .map(|agent_dir| agent_dir.join("extensions").exists())
            .unwrap_or(false)
    {
        return true;
    }

    agent_dir.is_some_and(|agent_dir| {
        DefaultPackageManager::new(cwd.to_path_buf(), agent_dir.to_path_buf())
            .has_explicit_extension_configuration()
    })
}

fn merge_extension_paths(primary: &[String], secondary: &[String]) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for path in primary.iter().chain(secondary.iter()) {
        if seen.insert(path.clone()) {
            merged.push(path.clone());
        }
    }

    merged
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn sidecar_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../support/extension-sidecar.mjs")
}

fn spawn_stdout_reader(inner: Arc<RpcExtensionHostInner>, stdout: tokio::process::ChildStdout) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => handle_stdout_line(inner.clone(), line),
                Ok(None) => {
                    reject_all_pending(&inner, "Extension host exited unexpectedly");
                    break;
                }
                Err(error) => {
                    reject_all_pending(&inner, &format!("Extension host read failed: {error}"));
                    break;
                }
            }
        }
    });
}

fn spawn_stderr_reader(inner: Arc<RpcExtensionHostInner>, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if !line.trim().is_empty() {
                        (inner.stderr_emitter)(format!("Warning: extension host: {line}\n"));
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    (inner.stderr_emitter)(format!(
                        "Warning: extension host stderr failed: {error}\n"
                    ));
                    break;
                }
            }
        }
    });
}

fn handle_stdout_line(inner: Arc<RpcExtensionHostInner>, line: String) {
    let value = match serde_json::from_str::<Value>(&line) {
        Ok(value) => value,
        Err(error) => {
            (inner.stderr_emitter)(format!(
                "Warning: extension host emitted invalid JSON: {error}\n"
            ));
            return;
        }
    };

    let message_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if message_type == "response" {
        let Some(request_id) = value.get("id").and_then(Value::as_str) else {
            return;
        };
        let pending = inner
            .pending
            .lock()
            .expect("extension host pending mutex poisoned")
            .remove(request_id);
        if let Some(pending) = pending {
            if value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let data = value.get("data").cloned().unwrap_or(Value::Null);
                let _ = pending.send(Ok(data));
            } else {
                let error = value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| String::from("Extension host request failed"));
                let _ = pending.send(Err(error));
            }
        }
        return;
    }

    if matches!(message_type, "extension_ui_request" | "extension_error") {
        (inner.stdout_emitter)(format!("{line}\n"));
        return;
    }

    if message_type == "app_request" {
        let Some(request_id) = value
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            (inner.stderr_emitter)(String::from(
                "Warning: extension host app request is missing an id\n",
            ));
            return;
        };
        let handler = inner
            .app_request_handler
            .lock()
            .expect("extension host app request handler mutex poisoned")
            .clone();
        tokio::spawn(async move {
            let result = match handler {
                Some(handler) => handler(value).await,
                None => Err(String::from(
                    "Extension app requests are unavailable in this mode",
                )),
            };
            let _ = send_app_response(inner, &request_id, result).await;
        });
        return;
    }

    if message_type == "shutdown_requested" {
        return;
    }

    (inner.stderr_emitter)(format!(
        "Warning: extension host emitted unexpected message: {line}\n"
    ));
}

async fn send_app_response(
    inner: Arc<RpcExtensionHostInner>,
    request_id: &str,
    result: Result<Value, String>,
) -> Result<(), String> {
    let payload = match result {
        Ok(data) => json!({
            "type": "app_response",
            "id": request_id,
            "success": true,
            "data": data,
        }),
        Err(error) => json!({
            "type": "app_response",
            "id": request_id,
            "success": false,
            "error": error,
        }),
    };
    send_inner_message(&inner, &payload).await
}

async fn send_inner_message(
    inner: &Arc<RpcExtensionHostInner>,
    payload: &Value,
) -> Result<(), String> {
    let line = serde_json::to_string(payload)
        .map_err(|error| format!("Failed to serialize extension host request: {error}"))?;
    let mut stdin = inner.stdin.lock().await;
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|error| format!("Failed to write to extension host: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("Failed to write to extension host: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("Failed to flush extension host stdin: {error}"))
}

fn reject_all_pending(inner: &RpcExtensionHostInner, message: &str) {
    let mut pending = inner
        .pending
        .lock()
        .expect("extension host pending mutex poisoned");
    let senders = std::mem::take(&mut *pending);
    drop(pending);
    for (_, sender) in senders {
        let _ = sender.send(Err(message.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::should_start_extension_host;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pi-coding-agent-cli-rpc-extensions-{prefix}-{unique}"
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn starts_extension_host_when_settings_declare_packages() {
        let temp_dir = unique_temp_dir("settings");
        let cwd = temp_dir.join("project");
        let agent_dir = temp_dir.join("agent");
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            cwd.join(".pi").join("settings.json"),
            "{\n  \"packages\": [\"/tmp/demo-pkg\"]\n}\n",
        )
        .unwrap();

        assert!(should_start_extension_host(
            &cwd,
            Some(&agent_dir),
            &[],
            false
        ));
    }
}
