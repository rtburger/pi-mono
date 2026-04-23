//! Extensions are intentionally unsupported in the Rust CLI rewrite.
//!
//! This module keeps the old RPC extension bridge types available so the rest of
//! the crate can compile while extension support is removed. The Node sidecar
//! and JavaScript runtime have been deleted; any attempt to start the bridge now
//! returns an explicit error.

use super::TextEmitter;
use pi_coding_agent_core::{CustomMessage, ProviderConfigInput, SourceInfo};
use pi_events::UserContent;
use serde::Deserialize;
use serde_json::Value;
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
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
pub struct RpcExtensionToolInfo {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub source_info: SourceInfo,
    #[serde(default)]
    pub prompt_snippet: Option<String>,
    #[serde(default)]
    pub prompt_guidelines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionResourcePath {
    pub path: String,
    pub extension_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionShortcutInfo {
    pub shortcut: String,
    #[serde(default)]
    pub description: Option<String>,
    pub extension_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RpcExtensionDiagnostic {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum RpcExtensionProviderMutation {
    Register {
        name: String,
        config: ProviderConfigInput,
    },
    Unregister {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RpcExtensionToolExecutionResult {
    #[serde(default)]
    pub content: Vec<UserContent>,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcToolResultMutation {
    #[serde(default)]
    pub content: Option<Vec<UserContent>>,
    #[serde(default)]
    pub details: Option<Value>,
    #[serde(default)]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RpcExtensionInputResult {
    pub action: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub images: Option<Vec<UserContent>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcBeforeAgentStartResult {
    #[serde(default)]
    pub messages: Vec<CustomMessage>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RpcBeforeForkResult {
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub skip_conversation_restore: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCompactionResult {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    #[serde(default)]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RpcBeforeCompactResult {
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub compaction: Option<RpcCompactionResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcTreeSummaryResult {
    pub summary: String,
    #[serde(default)]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RpcBeforeTreeResult {
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub summary: Option<RpcTreeSummaryResult>,
    #[serde(default)]
    pub custom_instructions: Option<String>,
    #[serde(default)]
    pub replace_instructions: Option<bool>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcExtensionInitOutput {
    pub extension_count: usize,
    pub commands: Vec<RpcExtensionCommandInfo>,
    #[serde(default)]
    pub tools: Vec<RpcExtensionToolInfo>,
    #[serde(default)]
    pub shortcuts: Vec<RpcExtensionShortcutInfo>,
    #[serde(default)]
    pub skill_paths: Vec<RpcExtensionResourcePath>,
    #[serde(default)]
    pub prompt_paths: Vec<RpcExtensionResourcePath>,
    #[serde(default)]
    pub theme_paths: Vec<RpcExtensionResourcePath>,
    #[serde(default)]
    pub provider_mutations: Vec<RpcExtensionProviderMutation>,
    #[serde(default)]
    pub diagnostics: Vec<RpcExtensionDiagnostic>,
}

#[allow(dead_code)]
pub struct RpcExtensionHostStartOptions {
    pub cwd: PathBuf,
    pub agent_dir: Option<PathBuf>,
    pub extension_paths: Vec<String>,
    pub no_extensions: bool,
    pub flag_values: Value,
    pub keybindings: Value,
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

#[derive(Clone, Default)]
pub struct RpcExtensionHost {
    inner: Arc<()>,
}

impl RpcExtensionHost {
    pub async fn start(
        _options: RpcExtensionHostStartOptions,
    ) -> Result<RpcExtensionHostStartResult, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub fn commands(&self) -> Vec<RpcExtensionCommandInfo> {
        Vec::new()
    }

    pub fn tools(&self) -> Vec<RpcExtensionToolInfo> {
        Vec::new()
    }

    pub fn shortcuts(&self) -> Vec<RpcExtensionShortcutInfo> {
        Vec::new()
    }

    pub fn set_app_request_handler<F, Fut>(&self, _handler: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, String>> + Send + 'static,
    {
    }

    pub fn is_same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn has_command(&self, _name: &str) -> bool {
        false
    }

    pub async fn execute_command(&self, _name: &str, _args: &str) -> Result<bool, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn execute_shortcut(&self, _shortcut: &str) -> Result<bool, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn before_switch(
        &self,
        _reason: &str,
        _target_session_file: Option<String>,
    ) -> Result<bool, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn before_fork(&self, _entry_id: &str) -> Result<RpcBeforeForkResult, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn tool_call(
        &self,
        _tool_name: &str,
        _tool_call_id: &str,
        _input: Value,
    ) -> Result<Option<RpcToolCallResult>, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn execute_tool(
        &self,
        _tool_name: &str,
        _tool_call_id: &str,
        _args: Value,
    ) -> Result<RpcExtensionToolExecutionResult, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn tool_result(
        &self,
        _tool_name: &str,
        _tool_call_id: &str,
        _input: Value,
        _content: Vec<UserContent>,
        _details: Option<Value>,
        _is_error: bool,
    ) -> Result<Option<RpcToolResultMutation>, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn input(
        &self,
        _text: &str,
        _images: &[UserContent],
        _source: &str,
    ) -> Result<RpcExtensionInputResult, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn before_agent_start(
        &self,
        _prompt: &str,
        _images: &[UserContent],
        _system_prompt: &str,
    ) -> Result<Option<RpcBeforeAgentStartResult>, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn before_compact(
        &self,
        _preparation: Value,
        _branch_entries: Value,
        _custom_instructions: Option<String>,
    ) -> Result<RpcBeforeCompactResult, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn before_tree(&self, _preparation: Value) -> Result<RpcBeforeTreeResult, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn before_provider_request(&self, _payload: Value) -> Result<Value, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn request_json(&self, _kind: &str, _payload: Value) -> Result<Value, String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn update_state(&self, _state: Value) -> Result<(), String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn emit_event(&self, _event: Value) -> Result<(), String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn deliver_ui_response(&self, _response: Value) -> Result<(), String> {
        Err(String::from(
            "Extensions are not supported in the Rust CLI rewrite",
        ))
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        Ok(())
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
    _cwd: &Path,
    _agent_dir: Option<&Path>,
    _extension_paths: &[String],
    _no_extensions: bool,
) -> bool {
    false
}
