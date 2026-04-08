pub mod auth;
pub mod bootstrap;
pub mod config_value;
pub mod model_registry;
pub mod model_resolver;
pub mod runtime;

pub use auth::{AuthSource, MemoryAuthStorage};
pub use bootstrap::{
    BootstrapDiagnostic, BootstrapDiagnosticLevel, ExistingSessionSelection,
    SessionBootstrapOptions, SessionBootstrapResult, bootstrap_session,
};
pub use model_registry::{ModelRegistry, RequestAuth};
pub use model_resolver::{
    DEFAULT_MODELS, DEFAULT_THINKING_LEVEL, InitialModelOptions, InitialModelResult, ModelCatalog,
    ParseModelPatternOptions, ParsedModelResult, ResolveCliModelResult, RestoreModelResult,
    ScopedModel, default_model_id_for_provider, find_exact_model_reference_match,
    find_initial_model, parse_model_pattern, parse_thinking_level, resolve_cli_model,
    restore_model_from_session,
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
