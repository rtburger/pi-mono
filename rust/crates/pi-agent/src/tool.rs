use pi_events::{ToolDefinition, UserContent};
use serde_json::Value;
use std::{fmt, future::Future, pin::Pin, sync::Arc};
use tokio::sync::watch;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<AgentToolResult, AgentToolError>> + Send>>;

type ToolExecutor =
    Arc<dyn Fn(String, Value, Option<watch::Receiver<bool>>) -> ToolFuture + Send + Sync>;
type ToolArgPreparer = Arc<dyn Fn(Value) -> Value + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentToolResult {
    pub content: Vec<UserContent>,
    pub details: Value,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AgentToolError {
    #[error("{0}")]
    Message(String),
}

impl AgentToolError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

#[derive(Clone)]
pub struct AgentTool {
    pub definition: ToolDefinition,
    executor: ToolExecutor,
    prepare_arguments: Option<ToolArgPreparer>,
}

impl AgentTool {
    pub fn new<F, Fut>(definition: ToolDefinition, executor: F) -> Self
    where
        F: Fn(String, Value, Option<watch::Receiver<bool>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<AgentToolResult, AgentToolError>> + Send + 'static,
    {
        Self {
            definition,
            executor: Arc::new(move |tool_call_id, args, signal| {
                Box::pin(executor(tool_call_id, args, signal))
            }),
            prepare_arguments: None,
        }
    }

    pub fn with_prepare_arguments<F>(mut self, prepare_arguments: F) -> Self
    where
        F: Fn(Value) -> Value + Send + Sync + 'static,
    {
        self.prepare_arguments = Some(Arc::new(prepare_arguments));
        self
    }

    pub fn prepare_arguments(&self, args: Value) -> Value {
        self.prepare_arguments
            .as_ref()
            .map(|prepare_arguments| prepare_arguments(args.clone()))
            .unwrap_or(args)
    }

    pub async fn execute(
        &self,
        tool_call_id: String,
        args: Value,
        signal: Option<watch::Receiver<bool>>,
    ) -> Result<AgentToolResult, AgentToolError> {
        (self.executor)(tool_call_id, args, signal).await
    }
}

impl fmt::Debug for AgentTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentTool")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

impl PartialEq for AgentTool {
    fn eq(&self, other: &Self) -> bool {
        self.definition == other.definition
    }
}
