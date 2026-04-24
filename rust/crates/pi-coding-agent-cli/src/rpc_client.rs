use parking_lot::Mutex;
use pi_agent::ThinkingLevel as AgentThinkingLevel;
use pi_coding_agent_core::{ForkMessageCandidate, SessionStats, SourceInfo};
use pi_events::{
    AssistantEvent, AssistantMessage, Message, Model, ModelCompat, ModelCost, ToolResultMessage,
    UserContent,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{Instant, sleep, timeout},
};

pub const DEFAULT_RPC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_RPC_EVENT_TIMEOUT: Duration = Duration::from_secs(60);
const STARTUP_DELAY: Duration = Duration::from_millis(100);

type PendingResponse = oneshot::Sender<Result<RpcResponseEnvelope, String>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RpcClientOptions {
    /// Optional program to spawn. When set together with `cli_path`, the client
    /// executes `program <cli_path> --mode rpc ...`.
    pub program: Option<PathBuf>,
    /// Path to the CLI executable or script.
    pub cli_path: Option<PathBuf>,
    /// Working directory for the spawned process.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables.
    pub env: BTreeMap<String, String>,
    /// Provider override.
    pub provider: Option<String>,
    /// Model override.
    pub model: Option<String>,
    /// Additional CLI arguments.
    pub args: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RpcClientError {
    #[error("RPC client already started")]
    AlreadyStarted,
    #[error("RPC client is not started")]
    NotStarted,
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("Failed to encode or decode RPC JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("RPC process exited immediately with code {code:?}. Stderr: {stderr}")]
    ExitedImmediately { code: Option<i32>, stderr: String },
    #[error("RPC process ended: {message}")]
    ProcessEnded { message: String },
    #[error("Timed out waiting for RPC response to {command}. Stderr: {stderr}")]
    ResponseTimeout { command: String, stderr: String },
    #[error("RPC response channel closed while waiting for {command}")]
    ResponseChannelClosed { command: String },
    #[error("RPC command {command} failed: {message}")]
    Command { command: String, message: String },
    #[error("Unexpected RPC response for {command}: {message}")]
    UnexpectedResponse { command: String, message: String },
    #[error("Timed out waiting for {operation}. Stderr: {stderr}")]
    EventTimeout { operation: String, stderr: String },
    #[error("RPC event stream closed before {operation}")]
    EventStreamClosed { operation: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcThinkingLevel {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "minimal")]
    Minimal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "xhigh")]
    Xhigh,
}

impl From<AgentThinkingLevel> for RpcThinkingLevel {
    fn from(value: AgentThinkingLevel) -> Self {
        match value {
            AgentThinkingLevel::Off => Self::Off,
            AgentThinkingLevel::Minimal => Self::Minimal,
            AgentThinkingLevel::Low => Self::Low,
            AgentThinkingLevel::Medium => Self::Medium,
            AgentThinkingLevel::High => Self::High,
            AgentThinkingLevel::XHigh => Self::Xhigh,
        }
    }
}

impl From<RpcThinkingLevel> for AgentThinkingLevel {
    fn from(value: RpcThinkingLevel) -> Self {
        match value {
            RpcThinkingLevel::Off => Self::Off,
            RpcThinkingLevel::Minimal => Self::Minimal,
            RpcThinkingLevel::Low => Self::Low,
            RpcThinkingLevel::Medium => Self::Medium,
            RpcThinkingLevel::High => Self::High,
            RpcThinkingLevel::Xhigh => Self::XHigh,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcQueueMode {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "one-at-a-time")]
    OneAtATime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcCommandSource {
    #[serde(rename = "extension")]
    Extension,
    #[serde(rename = "prompt")]
    Prompt,
    #[serde(rename = "skill")]
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcCompactionReason {
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "threshold")]
    Threshold,
    #[serde(rename = "overflow")]
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcNotifyType {
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcWidgetPlacement {
    #[serde(rename = "aboveEditor")]
    AboveEditor,
    #[serde(rename = "belowEditor")]
    BelowEditor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcModel {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    pub base_url: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    #[serde(default)]
    pub cost: ModelCost,
    pub context_window: u64,
    pub max_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<ModelCompat>,
}

impl From<Model> for RpcModel {
    fn from(value: Model) -> Self {
        Self {
            id: value.id,
            name: value.name,
            api: value.api,
            provider: value.provider,
            base_url: value.base_url,
            reasoning: value.reasoning,
            input: value.input,
            cost: value.cost,
            context_window: value.context_window,
            max_tokens: value.max_tokens,
            compat: value.compat,
        }
    }
}

impl From<RpcModel> for Model {
    fn from(value: RpcModel) -> Self {
        Self {
            id: value.id,
            name: value.name,
            api: value.api,
            provider: value.provider,
            base_url: value.base_url,
            reasoning: value.reasoning,
            input: value.input,
            cost: value.cost,
            context_window: value.context_window,
            max_tokens: value.max_tokens,
            compat: value.compat,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcSessionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<RpcModel>,
    pub thinking_level: RpcThinkingLevel,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub steering_mode: RpcQueueMode,
    pub follow_up_mode: RpcQueueMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    pub auto_compaction_enabled: bool,
    pub message_count: usize,
    pub pending_message_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCycleModelResult {
    pub model: RpcModel,
    pub thinking_level: RpcThinkingLevel,
    pub is_scoped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcThinkingLevelResult {
    pub level: RpcThinkingLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcCancelledResult {
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcForkResult {
    pub text: String,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcLastAssistantText {
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCompactionResult {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcBashResult {
    pub output: String,
    pub exit_code: i32,
    pub cancelled: bool,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcToolResult {
    pub content: Vec<UserContent>,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcCustomMessage {
    pub role: String,
    pub payload: Value,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcAgentMessage {
    Standard(Message),
    Custom(RpcCustomMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcSlashCommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: RpcCommandSource,
    pub source_info: SourceInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcSessionEvent {
    AgentStart,
    AgentEnd {
        messages: Vec<RpcAgentMessage>,
    },
    TurnStart,
    TurnEnd {
        message: AssistantMessage,
        #[serde(rename = "toolResults")]
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart {
        message: RpcAgentMessage,
    },
    MessageUpdate {
        message: RpcAgentMessage,
        #[serde(rename = "assistantMessageEvent")]
        assistant_message_event: AssistantEvent,
    },
    MessageEnd {
        message: RpcAgentMessage,
    },
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
        #[serde(rename = "partialResult")]
        partial_result: RpcToolResult,
    },
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        result: RpcToolResult,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    QueueUpdate {
        steering: Vec<String>,
        #[serde(rename = "followUp")]
        follow_up: Vec<String>,
    },
    CompactionStart {
        reason: RpcCompactionReason,
    },
    CompactionEnd {
        reason: RpcCompactionReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<RpcCompactionResult>,
        aborted: bool,
        #[serde(rename = "willRetry")]
        will_retry: bool,
        #[serde(rename = "errorMessage")]
        error_message: Option<String>,
    },
    AutoRetryStart {
        attempt: usize,
        #[serde(rename = "maxAttempts")]
        max_attempts: usize,
        #[serde(rename = "delayMs")]
        delay_ms: u64,
        #[serde(rename = "errorMessage")]
        error_message: String,
    },
    AutoRetryEnd {
        success: bool,
        attempt: usize,
        #[serde(rename = "finalError")]
        final_error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcExtensionUiRequest {
    pub id: String,
    #[serde(flatten)]
    pub method: RpcExtensionUiMethod,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum RpcExtensionUiMethod {
    #[serde(rename = "select")]
    Select {
        title: String,
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
    #[serde(rename = "confirm")]
    Confirm {
        title: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
    #[serde(rename = "input")]
    Input {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
    #[serde(rename = "editor")]
    Editor {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefill: Option<String>,
    },
    #[serde(rename = "notify")]
    Notify {
        message: String,
        #[serde(
            rename = "notifyType",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        notify_type: Option<RpcNotifyType>,
    },
    #[serde(rename = "setStatus")]
    SetStatus {
        #[serde(rename = "statusKey")]
        status_key: String,
        #[serde(
            rename = "statusText",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        status_text: Option<String>,
    },
    #[serde(rename = "setWidget")]
    SetWidget {
        #[serde(rename = "widgetKey")]
        widget_key: String,
        #[serde(
            rename = "widgetLines",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        widget_lines: Option<Vec<String>>,
        #[serde(
            rename = "widgetPlacement",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        widget_placement: Option<RpcWidgetPlacement>,
    },
    #[serde(rename = "setTitle")]
    SetTitle { title: String },
    #[serde(rename = "set_editor_text")]
    SetEditorText { text: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RpcOutputEvent {
    Session(RpcSessionEvent),
    ExtensionUiRequest(RpcExtensionUiRequest),
    Unknown(Value),
}

impl RpcOutputEvent {
    fn is_agent_end(&self) -> bool {
        matches!(self, Self::Session(RpcSessionEvent::AgentEnd { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcExtensionUiResponse {
    Value { id: String, value: String },
    Confirmed { id: String, confirmed: bool },
    Cancelled { id: String },
}

impl RpcExtensionUiResponse {
    pub fn value(id: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Value {
            id: id.into(),
            value: value.into(),
        }
    }

    pub fn confirmed(id: impl Into<String>, confirmed: bool) -> Self {
        Self::Confirmed {
            id: id.into(),
            confirmed,
        }
    }

    pub fn cancelled(id: impl Into<String>) -> Self {
        Self::Cancelled { id: id.into() }
    }

    fn into_json(self) -> Value {
        match self {
            Self::Value { id, value } => json!({
                "type": "extension_ui_response",
                "id": id,
                "value": value,
            }),
            Self::Confirmed { id, confirmed } => json!({
                "type": "extension_ui_response",
                "id": id,
                "confirmed": confirmed,
            }),
            Self::Cancelled { id } => json!({
                "type": "extension_ui_response",
                "id": id,
                "cancelled": true,
            }),
        }
    }
}

pub struct RpcEventSubscription {
    handle: JoinHandle<()>,
}

impl Drop for RpcEventSubscription {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct RpcProcess {
    child: Arc<tokio::sync::Mutex<Option<Child>>>,
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
}

pub struct RpcClient {
    options: RpcClientOptions,
    process: Option<RpcProcess>,
    pending_requests: Arc<Mutex<HashMap<String, PendingResponse>>>,
    listeners: Arc<Mutex<Vec<mpsc::UnboundedSender<RpcOutputEvent>>>>,
    stderr: Arc<Mutex<String>>,
    request_id: AtomicU64,
}

impl RpcClient {
    pub fn new(options: RpcClientOptions) -> Self {
        Self {
            options,
            process: None,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            listeners: Arc::new(Mutex::new(Vec::new())),
            stderr: Arc::new(Mutex::new(String::new())),
            request_id: AtomicU64::new(0),
        }
    }

    pub async fn start(&mut self) -> Result<(), RpcClientError> {
        if self.process.is_some() {
            return Err(RpcClientError::AlreadyStarted);
        }

        let mut command = self.build_command();
        command.stdin(std::process::Stdio::piped());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let mut child = command.spawn().map_err(|source| RpcClientError::Io {
            context: String::from("Failed to spawn RPC process"),
            source,
        })?;

        let stdin = child.stdin.take().ok_or_else(|| RpcClientError::Io {
            context: String::from("Failed to capture RPC stdin"),
            source: io::Error::other("stdin pipe missing"),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| RpcClientError::Io {
            context: String::from("Failed to capture RPC stdout"),
            source: io::Error::other("stdout pipe missing"),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| RpcClientError::Io {
            context: String::from("Failed to capture RPC stderr"),
            source: io::Error::other("stderr pipe missing"),
        })?;

        let child = Arc::new(tokio::sync::Mutex::new(Some(child)));
        let stdin = Arc::new(tokio::sync::Mutex::new(Some(stdin)));
        let stdout_task = spawn_stdout_task(
            stdout,
            self.pending_requests.clone(),
            self.listeners.clone(),
        );
        let stderr_task = spawn_stderr_task(stderr, self.stderr.clone());

        self.process = Some(RpcProcess {
            child: child.clone(),
            stdin,
            stdout_task,
            stderr_task,
        });

        sleep(STARTUP_DELAY).await;

        let exit_code = {
            let mut guard = child.lock().await;
            match guard.as_mut() {
                Some(child) => child.try_wait().map_err(|source| RpcClientError::Io {
                    context: String::from("Failed to query RPC process status"),
                    source,
                })?,
                None => None,
            }
        };

        if let Some(status) = exit_code {
            let _ = self.stop().await;
            return Err(RpcClientError::ExitedImmediately {
                code: status.code(),
                stderr: self.stderr(),
            });
        }

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), RpcClientError> {
        let Some(process) = self.process.take() else {
            return Ok(());
        };

        if let Some(stdin) = process.stdin.lock().await.take() {
            drop(stdin);
        }

        let mut child = process.child.lock().await.take();
        if let Some(mut child) = child.take() {
            match timeout(Duration::from_secs(1), child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(source)) => {
                    clear_listeners(&self.listeners);
                    fail_pending_requests(
                        &self.pending_requests,
                        String::from("RPC process wait failed"),
                    );
                    return Err(RpcClientError::Io {
                        context: String::from("Failed while waiting for RPC process to exit"),
                        source,
                    });
                }
                Err(_) => {
                    child.start_kill().map_err(|source| RpcClientError::Io {
                        context: String::from("Failed to terminate RPC process"),
                        source,
                    })?;
                    child.wait().await.map_err(|source| RpcClientError::Io {
                        context: String::from("Failed while waiting for killed RPC process"),
                        source,
                    })?;
                }
            }
        }

        process.stdout_task.await.ok();
        process.stderr_task.await.ok();
        fail_pending_requests(
            &self.pending_requests,
            String::from("RPC client stopped before a response was received"),
        );
        clear_listeners(&self.listeners);
        Ok(())
    }

    pub fn subscribe_events(&self) -> mpsc::UnboundedReceiver<RpcOutputEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.listeners.lock().push(tx);
        rx
    }

    pub fn on_event<F>(&self, mut listener: F) -> RpcEventSubscription
    where
        F: FnMut(RpcOutputEvent) + Send + 'static,
    {
        let mut receiver = self.subscribe_events();
        let handle = tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                listener(event);
            }
        });
        RpcEventSubscription { handle }
    }

    pub fn stderr(&self) -> String {
        self.stderr.lock().clone()
    }

    pub async fn prompt(
        &self,
        message: impl Into<String>,
        images: Option<Vec<UserContent>>,
    ) -> Result<(), RpcClientError> {
        let mut command = command_map("prompt");
        command.insert(String::from("message"), Value::String(message.into()));
        insert_optional_images(&mut command, images)?;
        self.request_unit("prompt", command).await
    }

    pub async fn steer(
        &self,
        message: impl Into<String>,
        images: Option<Vec<UserContent>>,
    ) -> Result<(), RpcClientError> {
        let mut command = command_map("steer");
        command.insert(String::from("message"), Value::String(message.into()));
        insert_optional_images(&mut command, images)?;
        self.request_unit("steer", command).await
    }

    pub async fn follow_up(
        &self,
        message: impl Into<String>,
        images: Option<Vec<UserContent>>,
    ) -> Result<(), RpcClientError> {
        let mut command = command_map("follow_up");
        command.insert(String::from("message"), Value::String(message.into()));
        insert_optional_images(&mut command, images)?;
        self.request_unit("follow_up", command).await
    }

    pub async fn abort(&self) -> Result<(), RpcClientError> {
        self.request_unit("abort", command_map("abort")).await
    }

    pub async fn new_session(
        &self,
        parent_session: Option<String>,
    ) -> Result<RpcCancelledResult, RpcClientError> {
        let mut command = command_map("new_session");
        insert_optional_string(&mut command, "parentSession", parent_session);
        self.request_data("new_session", command).await
    }

    pub async fn get_state(&self) -> Result<RpcSessionState, RpcClientError> {
        self.request_data("get_state", command_map("get_state"))
            .await
    }

    pub async fn set_model(
        &self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<RpcModel, RpcClientError> {
        let mut command = command_map("set_model");
        command.insert(String::from("provider"), Value::String(provider.into()));
        command.insert(String::from("modelId"), Value::String(model_id.into()));
        self.request_data("set_model", command).await
    }

    pub async fn cycle_model(&self) -> Result<Option<RpcCycleModelResult>, RpcClientError> {
        self.request_optional_data("cycle_model", command_map("cycle_model"))
            .await
    }

    pub async fn get_available_models(&self) -> Result<Vec<RpcModel>, RpcClientError> {
        #[derive(Deserialize)]
        struct ResponseData {
            models: Vec<RpcModel>,
        }

        let data: ResponseData = self
            .request_data("get_available_models", command_map("get_available_models"))
            .await?;
        Ok(data.models)
    }

    pub async fn set_thinking_level(
        &self,
        level: impl Into<RpcThinkingLevel>,
    ) -> Result<(), RpcClientError> {
        let mut command = command_map("set_thinking_level");
        command.insert(String::from("level"), serde_json::to_value(level.into())?);
        self.request_unit("set_thinking_level", command).await
    }

    pub async fn cycle_thinking_level(
        &self,
    ) -> Result<Option<RpcThinkingLevelResult>, RpcClientError> {
        self.request_optional_data("cycle_thinking_level", command_map("cycle_thinking_level"))
            .await
    }

    pub async fn set_steering_mode(&self, mode: RpcQueueMode) -> Result<(), RpcClientError> {
        let mut command = command_map("set_steering_mode");
        command.insert(String::from("mode"), serde_json::to_value(mode)?);
        self.request_unit("set_steering_mode", command).await
    }

    pub async fn set_follow_up_mode(&self, mode: RpcQueueMode) -> Result<(), RpcClientError> {
        let mut command = command_map("set_follow_up_mode");
        command.insert(String::from("mode"), serde_json::to_value(mode)?);
        self.request_unit("set_follow_up_mode", command).await
    }

    pub async fn compact(
        &self,
        custom_instructions: Option<String>,
    ) -> Result<RpcCompactionResult, RpcClientError> {
        let mut command = command_map("compact");
        insert_optional_string(&mut command, "customInstructions", custom_instructions);
        self.request_data("compact", command).await
    }

    pub async fn set_auto_compaction(&self, enabled: bool) -> Result<(), RpcClientError> {
        let mut command = command_map("set_auto_compaction");
        command.insert(String::from("enabled"), Value::Bool(enabled));
        self.request_unit("set_auto_compaction", command).await
    }

    pub async fn set_auto_retry(&self, enabled: bool) -> Result<(), RpcClientError> {
        let mut command = command_map("set_auto_retry");
        command.insert(String::from("enabled"), Value::Bool(enabled));
        self.request_unit("set_auto_retry", command).await
    }

    pub async fn abort_retry(&self) -> Result<(), RpcClientError> {
        self.request_unit("abort_retry", command_map("abort_retry"))
            .await
    }

    pub async fn bash(
        &self,
        command_text: impl Into<String>,
    ) -> Result<RpcBashResult, RpcClientError> {
        let mut command = command_map("bash");
        command.insert(String::from("command"), Value::String(command_text.into()));
        self.request_data("bash", command).await
    }

    pub async fn abort_bash(&self) -> Result<(), RpcClientError> {
        self.request_unit("abort_bash", command_map("abort_bash"))
            .await
    }

    pub async fn get_session_stats(&self) -> Result<SessionStats, RpcClientError> {
        self.request_data("get_session_stats", command_map("get_session_stats"))
            .await
    }

    pub async fn export_html(
        &self,
        output_path: Option<String>,
    ) -> Result<RpcExportHtmlResult, RpcClientError> {
        let mut command = command_map("export_html");
        insert_optional_string(&mut command, "outputPath", output_path);
        self.request_data("export_html", command).await
    }

    pub async fn switch_session(
        &self,
        session_path: impl Into<String>,
    ) -> Result<RpcCancelledResult, RpcClientError> {
        let mut command = command_map("switch_session");
        command.insert(
            String::from("sessionPath"),
            Value::String(session_path.into()),
        );
        self.request_data("switch_session", command).await
    }

    pub async fn fork(&self, entry_id: impl Into<String>) -> Result<RpcForkResult, RpcClientError> {
        let mut command = command_map("fork");
        command.insert(String::from("entryId"), Value::String(entry_id.into()));
        self.request_data("fork", command).await
    }

    pub async fn get_fork_messages(&self) -> Result<Vec<ForkMessageCandidate>, RpcClientError> {
        #[derive(Deserialize)]
        struct ResponseData {
            messages: Vec<ForkMessageCandidate>,
        }

        let data: ResponseData = self
            .request_data("get_fork_messages", command_map("get_fork_messages"))
            .await?;
        Ok(data.messages)
    }

    pub async fn get_last_assistant_text(&self) -> Result<Option<String>, RpcClientError> {
        let data: RpcLastAssistantText = self
            .request_data(
                "get_last_assistant_text",
                command_map("get_last_assistant_text"),
            )
            .await?;
        Ok(data.text)
    }

    pub async fn set_session_name(&self, name: impl Into<String>) -> Result<(), RpcClientError> {
        let mut command = command_map("set_session_name");
        command.insert(String::from("name"), Value::String(name.into()));
        self.request_unit("set_session_name", command).await
    }

    pub async fn get_messages(&self) -> Result<Vec<RpcAgentMessage>, RpcClientError> {
        #[derive(Deserialize)]
        struct ResponseData {
            messages: Vec<RpcAgentMessage>,
        }

        let data: ResponseData = self
            .request_data("get_messages", command_map("get_messages"))
            .await?;
        Ok(data.messages)
    }

    pub async fn get_commands(&self) -> Result<Vec<RpcSlashCommand>, RpcClientError> {
        #[derive(Deserialize)]
        struct ResponseData {
            commands: Vec<RpcSlashCommand>,
        }

        let data: ResponseData = self
            .request_data("get_commands", command_map("get_commands"))
            .await?;
        Ok(data.commands)
    }

    pub async fn send_extension_ui_response(
        &self,
        response: RpcExtensionUiResponse,
    ) -> Result<(), RpcClientError> {
        self.send_line(response.into_json()).await
    }

    pub async fn wait_for_idle(&self, timeout_duration: Duration) -> Result<(), RpcClientError> {
        let mut receiver = self.subscribe_events();
        let mut pending_event = None;

        if !self.get_state().await?.is_streaming {
            match timeout(STARTUP_DELAY, receiver.recv()).await {
                Ok(Some(event)) => pending_event = Some(event),
                Ok(None) => {
                    return Err(RpcClientError::EventStreamClosed {
                        operation: String::from("the agent to become idle"),
                    });
                }
                Err(_) => return Ok(()),
            }
        }

        let deadline = Instant::now() + timeout_duration;

        loop {
            let event = if let Some(event) = pending_event.take() {
                event
            } else {
                let remaining =
                    deadline
                        .checked_duration_since(Instant::now())
                        .ok_or_else(|| RpcClientError::EventTimeout {
                            operation: String::from("the agent to become idle"),
                            stderr: self.stderr(),
                        })?;

                timeout(remaining, receiver.recv())
                    .await
                    .map_err(|_| RpcClientError::EventTimeout {
                        operation: String::from("the agent to become idle"),
                        stderr: self.stderr(),
                    })?
                    .ok_or_else(|| RpcClientError::EventStreamClosed {
                        operation: String::from("the agent to become idle"),
                    })?
            };

            if event.is_agent_end() {
                return Ok(());
            }
        }
    }

    pub async fn collect_events(
        &self,
        timeout_duration: Duration,
    ) -> Result<Vec<RpcOutputEvent>, RpcClientError> {
        let mut receiver = self.subscribe_events();
        let deadline = Instant::now() + timeout_duration;
        let mut events = Vec::new();

        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| RpcClientError::EventTimeout {
                    operation: String::from("agent events"),
                    stderr: self.stderr(),
                })?;

            let event = timeout(remaining, receiver.recv())
                .await
                .map_err(|_| RpcClientError::EventTimeout {
                    operation: String::from("agent events"),
                    stderr: self.stderr(),
                })?
                .ok_or_else(|| RpcClientError::EventStreamClosed {
                    operation: String::from("agent events"),
                })?;

            let finished = event.is_agent_end();
            events.push(event);
            if finished {
                return Ok(events);
            }
        }
    }

    pub async fn prompt_and_wait(
        &self,
        message: impl Into<String>,
        images: Option<Vec<UserContent>>,
        timeout_duration: Duration,
    ) -> Result<Vec<RpcOutputEvent>, RpcClientError> {
        let events = self.collect_events(timeout_duration);
        self.prompt(message, images).await?;
        events.await
    }

    async fn request_unit(
        &self,
        command: &str,
        payload: Map<String, Value>,
    ) -> Result<(), RpcClientError> {
        let response = self.send_command(command, payload).await?;
        ensure_success(command, &response)?;
        Ok(())
    }

    async fn request_data<T>(
        &self,
        command: &str,
        payload: Map<String, Value>,
    ) -> Result<T, RpcClientError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self.send_command(command, payload).await?;
        ensure_success(command, &response)?;
        let data = response
            .data
            .ok_or_else(|| RpcClientError::UnexpectedResponse {
                command: command.to_owned(),
                message: String::from("response did not include data"),
            })?;
        serde_json::from_value(data).map_err(RpcClientError::from)
    }

    async fn request_optional_data<T>(
        &self,
        command: &str,
        payload: Map<String, Value>,
    ) -> Result<Option<T>, RpcClientError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self.send_command(command, payload).await?;
        ensure_success(command, &response)?;
        match response.data {
            Some(data) => serde_json::from_value(data)
                .map(Some)
                .map_err(RpcClientError::from),
            None => Ok(None),
        }
    }

    async fn send_command(
        &self,
        command: &str,
        mut payload: Map<String, Value>,
    ) -> Result<RpcResponseEnvelope, RpcClientError> {
        let id = format!(
            "req_{}",
            self.request_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        payload.insert(String::from("id"), Value::String(id.clone()));
        let value = Value::Object(payload);
        let line = serialize_json_line(&value)?;
        let (tx, rx) = oneshot::channel();

        self.pending_requests.lock().insert(id.clone(), tx);

        if let Err(error) = self.write_line(command, &line).await {
            self.pending_requests.lock().remove(&id);
            return Err(error);
        }

        let response = timeout(DEFAULT_RPC_RESPONSE_TIMEOUT, rx)
            .await
            .map_err(|_| {
                self.pending_requests.lock().remove(&id);
                RpcClientError::ResponseTimeout {
                    command: command.to_owned(),
                    stderr: self.stderr(),
                }
            })?
            .map_err(|_| RpcClientError::ResponseChannelClosed {
                command: command.to_owned(),
            })?
            .map_err(|message| RpcClientError::ProcessEnded { message })?;

        if response.command != command {
            return Err(RpcClientError::UnexpectedResponse {
                command: command.to_owned(),
                message: format!(
                    "expected response for {command}, received {}",
                    response.command
                ),
            });
        }

        Ok(response)
    }

    async fn send_line(&self, value: Value) -> Result<(), RpcClientError> {
        let line = serialize_json_line(&value)?;
        self.write_line("extension_ui_response", &line).await
    }

    async fn write_line(&self, command: &str, line: &str) -> Result<(), RpcClientError> {
        let process = self.process.as_ref().ok_or(RpcClientError::NotStarted)?;
        let mut guard = process.stdin.lock().await;
        let stdin = guard.as_mut().ok_or(RpcClientError::NotStarted)?;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|source| RpcClientError::Io {
                context: format!("Failed to write RPC command {command}"),
                source,
            })?;
        stdin.flush().await.map_err(|source| RpcClientError::Io {
            context: format!("Failed to flush RPC command {command}"),
            source,
        })
    }

    fn build_command(&self) -> Command {
        let mut command = if let Some(program) = self.options.program.as_ref() {
            let mut command = Command::new(program);
            if let Some(cli_path) = self.options.cli_path.as_ref() {
                command.arg(cli_path);
            }
            command
        } else if let Some(cli_path) = self.options.cli_path.as_ref() {
            Command::new(cli_path)
        } else {
            Command::new("pi")
        };

        if let Some(cwd) = self.options.cwd.as_ref() {
            command.current_dir(cwd);
        }

        command.arg("--mode").arg("rpc");

        if let Some(provider) = self.options.provider.as_ref() {
            command.arg("--provider").arg(provider);
        }
        if let Some(model) = self.options.model.as_ref() {
            command.arg("--model").arg(model);
        }
        command.args(&self.options.args);

        for (key, value) in &self.options.env {
            command.env(key, value);
        }

        command
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcExportHtmlResult {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RpcResponseEnvelope {
    #[serde(rename = "type")]
    kind: String,
    command: String,
    success: bool,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

fn ensure_success(command: &str, response: &RpcResponseEnvelope) -> Result<(), RpcClientError> {
    if response.kind != "response" {
        return Err(RpcClientError::UnexpectedResponse {
            command: command.to_owned(),
            message: format!("unexpected response type {}", response.kind),
        });
    }

    if response.success {
        return Ok(());
    }

    Err(RpcClientError::Command {
        command: command.to_owned(),
        message: response
            .error
            .clone()
            .unwrap_or_else(|| String::from("unknown RPC error")),
    })
}

fn command_map(kind: &str) -> Map<String, Value> {
    let mut command = Map::new();
    command.insert(String::from("type"), Value::String(kind.to_owned()));
    command
}

fn insert_optional_string(command: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        command.insert(String::from(key), Value::String(value));
    }
}

fn insert_optional_images(
    command: &mut Map<String, Value>,
    images: Option<Vec<UserContent>>,
) -> Result<(), RpcClientError> {
    if let Some(images) = images
        && !images.is_empty()
    {
        command.insert(String::from("images"), serde_json::to_value(images)?);
    }
    Ok(())
}

fn serialize_json_line(value: &Value) -> Result<String, RpcClientError> {
    Ok(format!("{}\n", serde_json::to_string(value)?))
}

fn spawn_stdout_task(
    stdout: ChildStdout,
    pending_requests: Arc<Mutex<HashMap<String, PendingResponse>>>,
    listeners: Arc<Mutex<Vec<mpsc::UnboundedSender<RpcOutputEvent>>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let result = read_lf_lines(stdout, |line| {
            handle_stdout_line(&line, &pending_requests, &listeners);
        })
        .await;

        if let Err(error) = result {
            fail_pending_requests(
                &pending_requests,
                format!("Failed to read RPC stdout: {error}"),
            );
        } else {
            fail_pending_requests(&pending_requests, String::from("RPC stdout closed"));
        }
        clear_listeners(&listeners);
    })
}

fn spawn_stderr_task(stderr: ChildStderr, stderr_buffer: Arc<Mutex<String>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let _ = read_stderr(stderr, stderr_buffer).await;
    })
}

fn handle_stdout_line(
    line: &str,
    pending_requests: &Arc<Mutex<HashMap<String, PendingResponse>>>,
    listeners: &Arc<Mutex<Vec<mpsc::UnboundedSender<RpcOutputEvent>>>>,
) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };

    match value.get("type").and_then(Value::as_str) {
        Some("response") => {
            let id = value.get("id").and_then(Value::as_str).map(str::to_owned);
            let response = serde_json::from_value::<RpcResponseEnvelope>(value.clone())
                .map_err(|error| format!("Malformed RPC response: {error}"));
            if let Some(id) = id
                && let Some(sender) = pending_requests.lock().remove(&id)
            {
                let _ = sender.send(response);
            }
        }
        Some("extension_ui_request") => {
            match serde_json::from_value::<RpcExtensionUiRequest>(value.clone()) {
                Ok(event) => emit_event(listeners, RpcOutputEvent::ExtensionUiRequest(event)),
                Err(_) => emit_event(listeners, RpcOutputEvent::Unknown(value)),
            }
        }
        Some(_) => match serde_json::from_value::<RpcSessionEvent>(value.clone()) {
            Ok(event) => emit_event(listeners, RpcOutputEvent::Session(event)),
            Err(_) => emit_event(listeners, RpcOutputEvent::Unknown(value)),
        },
        None => emit_event(listeners, RpcOutputEvent::Unknown(value)),
    }
}

fn emit_event(
    listeners: &Arc<Mutex<Vec<mpsc::UnboundedSender<RpcOutputEvent>>>>,
    event: RpcOutputEvent,
) {
    let mut listeners = listeners.lock();
    listeners.retain(|listener| listener.send(event.clone()).is_ok());
}

fn fail_pending_requests(
    pending_requests: &Arc<Mutex<HashMap<String, PendingResponse>>>,
    message: String,
) {
    let pending = {
        let mut pending = pending_requests.lock();
        pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>()
    };

    for sender in pending {
        let _ = sender.send(Err(message.clone()));
    }
}

fn clear_listeners(listeners: &Arc<Mutex<Vec<mpsc::UnboundedSender<RpcOutputEvent>>>>) {
    listeners.lock().clear();
}

async fn read_stderr(mut stderr: ChildStderr, stderr_buffer: Arc<Mutex<String>>) -> io::Result<()> {
    let mut chunk = [0u8; 4096];
    loop {
        let read = stderr.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }

        let text = String::from_utf8_lossy(&chunk[..read]).into_owned();
        stderr_buffer.lock().push_str(&text);
        eprint!("{text}");
    }
}

async fn read_lf_lines<R, F>(mut reader: R, mut on_line: F) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    F: FnMut(String),
{
    let mut buffer = Vec::<u8>::new();
    let mut chunk = [0u8; 4096];

    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..read]);
        let mut start = 0usize;
        for index in 0..buffer.len() {
            if buffer[index] == b'\n' {
                emit_jsonl_line(&buffer[start..index], &mut on_line);
                start = index + 1;
            }
        }

        if start > 0 {
            buffer = buffer[start..].to_vec();
        }
    }

    if !buffer.is_empty() {
        emit_jsonl_line(&buffer, &mut on_line);
    }

    Ok(())
}

fn emit_jsonl_line<F>(line: &[u8], on_line: &mut F)
where
    F: FnMut(String),
{
    let line = if line.last() == Some(&b'\r') {
        &line[..line.len().saturating_sub(1)]
    } else {
        line
    };
    on_line(String::from_utf8_lossy(line).into_owned());
}
