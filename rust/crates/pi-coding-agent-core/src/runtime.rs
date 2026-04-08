use crate::{
    AuthSource, BootstrapDiagnostic, ModelRegistry, SessionBootstrapOptions, bootstrap_session,
};
use pi_agent::{Agent, AgentState, AssistantStreamer};
use pi_ai::{AiError, AssistantEventStream, StreamOptions, stream_response};
use pi_events::{Context, Message, Model};
use std::{path::PathBuf, sync::Arc};

pub struct CodingAgentCoreOptions {
    pub auth_source: Arc<dyn AuthSource>,
    pub built_in_models: Vec<Model>,
    pub models_json_path: Option<PathBuf>,
    pub system_prompt: String,
    pub bootstrap: SessionBootstrapOptions,
    pub stream_options: StreamOptions,
}

pub struct CreateCodingAgentCoreResult {
    pub core: CodingAgentCore,
    pub diagnostics: Vec<BootstrapDiagnostic>,
    pub model_fallback_message: Option<String>,
}

#[derive(Clone)]
pub struct CodingAgentCore {
    agent: Agent,
    model_registry: Arc<ModelRegistry>,
}

impl CodingAgentCore {
    pub fn agent(&self) -> Agent {
        self.agent.clone()
    }

    pub fn model_registry(&self) -> Arc<ModelRegistry> {
        self.model_registry.clone()
    }

    pub fn state(&self) -> AgentState {
        self.agent.state()
    }

    pub async fn prompt_text(
        &self,
        text: impl Into<String>,
    ) -> Result<(), crate::CodingAgentCoreError> {
        self.agent.prompt_text(text).await.map_err(Into::into)
    }

    pub async fn prompt_message(
        &self,
        message: Message,
    ) -> Result<(), crate::CodingAgentCoreError> {
        self.agent.prompt(message).await.map_err(Into::into)
    }

    pub async fn continue_turn(&self) -> Result<(), crate::CodingAgentCoreError> {
        self.agent.r#continue().await.map_err(Into::into)
    }

    pub fn abort(&self) {
        self.agent.abort();
    }

    pub async fn wait_for_idle(&self) {
        self.agent.wait_for_idle().await;
    }
}

pub fn create_coding_agent_core(
    options: CodingAgentCoreOptions,
) -> Result<CreateCodingAgentCoreResult, crate::CodingAgentCoreError> {
    let model_registry = Arc::new(ModelRegistry::new(
        options.auth_source,
        options.built_in_models,
        options.models_json_path,
    ));
    let bootstrap = bootstrap_session(&model_registry, options.bootstrap);

    let Some(model) = bootstrap.model.clone() else {
        return Err(crate::CodingAgentCoreError::NoModelAvailable);
    };

    let mut state = AgentState::new(model);
    state.system_prompt = options.system_prompt;
    state.thinking_level = bootstrap.thinking_level;

    let agent = Agent::with_parts(
        state,
        Arc::new(RegistryBackedStreamer {
            model_registry: model_registry.clone(),
        }),
        options.stream_options,
    );

    Ok(CreateCodingAgentCoreResult {
        core: CodingAgentCore {
            agent,
            model_registry,
        },
        diagnostics: bootstrap.diagnostics,
        model_fallback_message: bootstrap.model_fallback_message,
    })
}

struct RegistryBackedStreamer {
    model_registry: Arc<ModelRegistry>,
}

impl AssistantStreamer for RegistryBackedStreamer {
    fn stream(
        &self,
        model: Model,
        context: Context,
        mut options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError> {
        let auth = self
            .model_registry
            .get_api_key_and_headers(&model)
            .map_err(AiError::Message)?;

        options.api_key = auth.api_key;
        if auth.headers.is_some() || !options.headers.is_empty() {
            let mut merged_headers = auth.headers.unwrap_or_default();
            merged_headers.extend(options.headers);
            options.headers = merged_headers;
        }

        stream_response(model, context, options)
    }
}
