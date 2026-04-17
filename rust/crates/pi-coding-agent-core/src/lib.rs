pub mod auth;
pub mod bootstrap;
pub mod compaction;
pub mod config_value;
pub mod footer_data;
mod frontmatter;
pub mod messages;
pub mod model_registry;
pub mod model_resolver;
pub mod prompt_templates;
pub mod resource_loader;
pub mod resource_types;
pub mod runtime;
pub mod session_manager;
pub mod skill_block;
pub mod skills;
pub mod system_prompt;

pub use auth::{
    AuthApiKeyFuture, AuthFileSource, AuthSource, ChainedAuthSource, MemoryAuthStorage,
    refresh_auth_file_oauth,
};
pub use bootstrap::{
    BootstrapDiagnostic, BootstrapDiagnosticLevel, ExistingSessionSelection,
    SessionBootstrapOptions, SessionBootstrapResult, bootstrap_session,
};
pub use compaction::{
    BranchSummaryDetails, BranchSummaryOptions, CollectEntriesResult, CompactionDetails,
    CompactionPreparation, CompactionResult, CompactionSettings, ContextUsageEstimate,
    GeneratedBranchSummary, calculate_context_tokens, collect_entries_for_branch_summary, compact,
    estimate_context_tokens, estimate_tokens, generate_branch_summary,
    generate_branch_summary_with_details, latest_compaction_timestamp, prepare_compaction,
    should_compact,
};
pub use footer_data::{BranchChangeSubscription, FooterDataProvider, FooterDataSnapshot};
pub use messages::{
    BLOCKED_IMAGE_PLACEHOLDER, BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX, BashExecutionMessage,
    BranchSummaryMessage, COMPACTION_SUMMARY_PREFIX, COMPACTION_SUMMARY_SUFFIX,
    CompactionSummaryMessage, CustomMessage, CustomMessageContent, bash_execution_to_text,
    convert_to_llm, create_bash_execution_message, create_branch_summary_message,
    create_compaction_summary_message, create_custom_message, filter_blocked_images,
};
pub use model_registry::{ModelRegistry, RequestAuth};
pub use model_resolver::{
    DEFAULT_MODELS, DEFAULT_THINKING_LEVEL, InitialModelOptions, InitialModelResult, ModelCatalog,
    ParseModelPatternOptions, ParsedModelResult, ResolveCliModelResult, ResolveModelScopeResult,
    RestoreModelResult, ScopedModel, default_model_id_for_provider,
    find_exact_model_reference_match, find_initial_model, parse_model_pattern,
    parse_thinking_level, resolve_cli_model, resolve_model_scope, restore_model_from_session,
};
pub use prompt_templates::{
    LoadPromptTemplatesOptions, LoadPromptTemplatesResult, PromptTemplate, expand_prompt_template,
    load_prompt_templates, parse_command_args, substitute_args,
};
pub use resource_loader::{
    DefaultResourceLoader, DefaultResourceLoaderOptions, LoadedResources, ResourcePathEntry,
};
pub use resource_types::{ResourceDiagnostic, SourceInfo};
pub use runtime::{
    AgentSession, AgentSessionEvent, AgentSessionOptions, AgentSessionRuntime,
    AgentSessionRuntimeError, AgentSessionRuntimeRequest, CodingAgentCore, CodingAgentCoreOptions,
    CompactionReason, CreateAgentSessionResult, CreateAgentSessionRuntimeFactory,
    CreateAgentSessionRuntimeFuture, CreateCodingAgentCoreResult, RetrySettings,
    create_agent_session, create_agent_session_runtime, create_coding_agent_core,
};
pub use session_manager::{
    CURRENT_SESSION_VERSION, FileEntry, NewSessionOptions, SessionContext, SessionEntry,
    SessionHeader, SessionInfo, SessionManager, SessionManagerError, SessionModelSelection,
    SessionTreeNode, build_session_context, find_most_recent_session, get_default_session_dir,
    get_latest_compaction_entry, get_sessions_dir, load_entries_from_file, parse_session_entries,
};
pub use skill_block::{ParsedSkillBlock, parse_skill_block};
pub use skills::{
    LoadSkillsOptions, LoadSkillsResult, Skill, expand_skill_command, format_skills_for_prompt,
    load_skills,
};
pub use system_prompt::{
    BuildSystemPromptOptions, ContextFile, LoadedSystemPromptResources,
    build_default_pi_system_prompt, build_system_prompt, discover_append_system_prompt_file,
    discover_system_prompt_file, load_project_context_files, load_system_prompt_resources,
    resolve_prompt_input,
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
