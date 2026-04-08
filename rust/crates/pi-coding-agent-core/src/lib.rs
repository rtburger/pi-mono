pub mod auth;
pub mod bootstrap;
pub mod config_value;
pub mod messages;
pub mod model_registry;
pub mod model_resolver;
pub mod runtime;

pub use auth::{AuthSource, MemoryAuthStorage};
pub use bootstrap::{
    BootstrapDiagnostic, BootstrapDiagnosticLevel, ExistingSessionSelection,
    SessionBootstrapOptions, SessionBootstrapResult, bootstrap_session,
};
pub use messages::{
    BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX, BashExecutionMessage, BranchSummaryMessage,
    COMPACTION_SUMMARY_PREFIX, COMPACTION_SUMMARY_SUFFIX, CompactionSummaryMessage, CustomMessage,
    CustomMessageContent, bash_execution_to_text, convert_to_llm, create_bash_execution_message,
    create_branch_summary_message, create_compaction_summary_message, create_custom_message,
};
pub use model_registry::{ModelRegistry, RequestAuth};
pub use model_resolver::{
    DEFAULT_MODELS, DEFAULT_THINKING_LEVEL, InitialModelOptions, InitialModelResult, ModelCatalog,
    ParseModelPatternOptions, ParsedModelResult, ResolveCliModelResult, ResolveModelScopeResult,
    RestoreModelResult, ScopedModel, default_model_id_for_provider,
    find_exact_model_reference_match, find_initial_model, parse_model_pattern,
    parse_thinking_level, resolve_cli_model, resolve_model_scope, restore_model_from_session,
};
pub use runtime::{
    CodingAgentCore, CodingAgentCoreOptions, CreateCodingAgentCoreResult, create_coding_agent_core,
};

#[derive(Debug, thiserror::Error)]
pub enum CodingAgentCoreError {
    #[error("No model available. Use /login or set an API key environment variable.")]
    NoModelAvailable,
    #[error(transparent)]
    Agent(#[from] pi_agent::AgentError),
    #[error("{0}")]
    Message(String),
}
