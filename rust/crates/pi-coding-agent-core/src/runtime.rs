use crate::{
    AuthSource, BootstrapDiagnostic, ModelRegistry, SessionBootstrapOptions, bootstrap_session,
    convert_to_llm, filter_blocked_images,
};
use async_stream::stream;
use futures::StreamExt;
use pi_agent::{Agent, AgentState, AgentTool, AssistantStreamer};
use pi_ai::{
    AiError, AssistantEventStream, SimpleStreamOptions, StreamOptions,
    ThinkingLevel as AiThinkingLevel, stream_simple,
};
use pi_coding_agent_tools::create_coding_tools_with_read_auto_resize_flag;
use pi_events::{Context, Message, Model};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct CodingAgentCoreOptions {
    pub auth_source: Arc<dyn AuthSource>,
    pub built_in_models: Vec<Model>,
    pub models_json_path: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub tools: Option<Vec<AgentTool>>,
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
    auto_resize_images: Arc<AtomicBool>,
    block_images: Arc<AtomicBool>,
    thinking_budgets: Arc<Mutex<pi_ai::ThinkingBudgets>>,
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

    pub fn auto_resize_images(&self) -> bool {
        self.auto_resize_images.load(Ordering::Relaxed)
    }

    pub fn set_auto_resize_images(&self, enabled: bool) {
        self.auto_resize_images.store(enabled, Ordering::Relaxed);
    }

    pub fn block_images(&self) -> bool {
        self.block_images.load(Ordering::Relaxed)
    }

    pub fn set_block_images(&self, blocked: bool) {
        self.block_images.store(blocked, Ordering::Relaxed);
    }

    pub fn thinking_budgets(&self) -> pi_ai::ThinkingBudgets {
        self.thinking_budgets.lock().unwrap().clone()
    }

    pub fn set_thinking_budgets(&self, thinking_budgets: pi_ai::ThinkingBudgets) {
        self.agent.set_thinking_budgets(thinking_budgets.clone());
        *self.thinking_budgets.lock().unwrap() = thinking_budgets;
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
    let CodingAgentCoreOptions {
        auth_source,
        built_in_models,
        models_json_path,
        cwd,
        tools,
        system_prompt,
        bootstrap,
        stream_options,
    } = options;

    let model_registry = Arc::new(ModelRegistry::new(
        auth_source,
        built_in_models,
        models_json_path,
    ));
    let bootstrap = bootstrap_session(&model_registry, bootstrap);

    let Some(model) = bootstrap.model.clone() else {
        return Err(crate::CodingAgentCoreError::NoModelAvailable);
    };

    let cwd = match cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()
            .map_err(|error| crate::CodingAgentCoreError::Message(error.to_string()))?,
    };

    let auto_resize_images = Arc::new(AtomicBool::new(true));
    let thinking_budgets = Arc::new(Mutex::new(pi_ai::ThinkingBudgets::default()));

    let mut state = AgentState::new(model);
    state.system_prompt = system_prompt;
    state.thinking_level = bootstrap.thinking_level;
    state.tools = tools.unwrap_or_else(|| {
        create_coding_tools_with_read_auto_resize_flag(cwd, auto_resize_images.clone())
    });

    let agent = Agent::with_parts(
        state,
        Arc::new(RegistryBackedStreamer {
            model_registry: model_registry.clone(),
            thinking_budgets: thinking_budgets.clone(),
        }),
        stream_options,
    );
    let block_images = Arc::new(AtomicBool::new(false));
    let convert_block_images = block_images.clone();
    agent.set_convert_to_llm(move |messages| {
        let convert_block_images = convert_block_images.clone();
        async move {
            let converted = convert_to_llm(messages);
            if convert_block_images.load(Ordering::Relaxed) {
                filter_blocked_images(converted)
            } else {
                converted
            }
        }
    });

    Ok(CreateCodingAgentCoreResult {
        core: CodingAgentCore {
            agent,
            model_registry,
            auto_resize_images,
            block_images,
            thinking_budgets,
        },
        diagnostics: bootstrap.diagnostics,
        model_fallback_message: bootstrap.model_fallback_message,
    })
}

struct RegistryBackedStreamer {
    model_registry: Arc<ModelRegistry>,
    thinking_budgets: Arc<Mutex<pi_ai::ThinkingBudgets>>,
}

impl AssistantStreamer for RegistryBackedStreamer {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> Result<AssistantEventStream, AiError> {
        let model_registry = self.model_registry.clone();
        let thinking_budgets = self.thinking_budgets.clone();
        Ok(Box::pin(stream! {
            let auth = match model_registry.get_api_key_and_headers_async(&model).await {
                Ok(auth) => auth,
                Err(error) => {
                    yield Err(AiError::Message(error));
                    return;
                }
            };

            let mut stream_options = options;
            stream_options.api_key = auth.api_key;
            if auth.headers.is_some() || !stream_options.headers.is_empty() {
                let mut merged_headers = auth.headers.unwrap_or_default();
                merged_headers.extend(stream_options.headers);
                stream_options.headers = merged_headers;
            }

            let thinking_budgets = thinking_budgets.lock().unwrap().clone();
            match stream_simple(model, context, map_stream_options_to_simple_options(stream_options, thinking_budgets)) {
                Ok(mut inner) => {
                    while let Some(event) = inner.next().await {
                        yield event;
                    }
                }
                Err(error) => {
                    yield Err(error);
                }
            }
        }))
    }
}

fn map_stream_options_to_simple_options(
    options: StreamOptions,
    thinking_budgets: pi_ai::ThinkingBudgets,
) -> SimpleStreamOptions {
    let reasoning = options
        .reasoning_effort
        .as_deref()
        .and_then(parse_ai_thinking_level);

    SimpleStreamOptions {
        signal: options.signal,
        session_id: options.session_id,
        cache_retention: options.cache_retention,
        api_key: options.api_key,
        transport: options.transport,
        headers: options.headers,
        metadata: options.metadata,
        on_payload: options.on_payload,
        temperature: options.temperature,
        max_tokens: options.max_tokens,
        reasoning,
        thinking_budgets,
        tool_choice: options.tool_choice,
    }
}

fn parse_ai_thinking_level(value: &str) -> Option<AiThinkingLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "minimal" => Some(AiThinkingLevel::Minimal),
        "low" => Some(AiThinkingLevel::Low),
        "medium" => Some(AiThinkingLevel::Medium),
        "high" => Some(AiThinkingLevel::High),
        "xhigh" => Some(AiThinkingLevel::Xhigh),
        _ => None,
    }
}
