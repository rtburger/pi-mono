pub mod anthropic_messages;
pub mod models;
pub mod openai_completions;
pub mod openai_responses;

pub use models::{
    built_in_models, get_model, get_models, get_providers, models_are_equal, supports_xhigh,
};

use async_stream::stream;
use futures::{Stream, StreamExt};
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
    UserContent,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    env,
    pin::Pin,
    sync::{
        Arc, Mutex, Once, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::time::{Duration, sleep};

pub type AssistantEventStream = Pin<Box<dyn Stream<Item = Result<AssistantEvent, AiError>> + Send>>;

type Registry = HashMap<String, Arc<dyn AiProvider>>;

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub signal: Option<tokio::sync::watch::Receiver<bool>>,
    pub session_id: Option<String>,
    pub cache_retention: CacheRetention,
    pub api_key: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheRetention {
    None,
    #[default]
    Short,
    Long,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AiError {
    #[error("No API provider registered for api: {0}")]
    UnknownApi(String),
    #[error("No more faux responses queued")]
    NoMoreFauxResponses,
    #[error("Request was aborted")]
    Aborted,
    #[error("{0}")]
    Message(String),
}

pub trait AiProvider: Send + Sync {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream;
}

pub fn register_provider(api: impl Into<String>, provider: Arc<dyn AiProvider>) {
    registry().lock().unwrap().insert(api.into(), provider);
}

pub fn unregister_provider(api: &str) {
    registry().lock().unwrap().remove(api);
}

pub fn get_env_api_key(provider: &str) -> Option<String> {
    match provider {
        "anthropic" => env_var("ANTHROPIC_OAUTH_TOKEN").or_else(|| env_var("ANTHROPIC_API_KEY")),
        "openai" => env_var("OPENAI_API_KEY"),
        _ => None,
    }
}

fn env_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn register_builtin_providers() {
    anthropic_messages::register_anthropic_provider();
    openai_completions::register_openai_completions_provider();
    openai_responses::register_openai_responses_provider();
}

fn ensure_builtin_providers_registered() {
    static BUILTINS: Once = Once::new();
    BUILTINS.call_once(register_builtin_providers);
}

pub fn complete(
    model: Model,
    context: Context,
    options: StreamOptions,
) -> impl std::future::Future<Output = Result<AssistantMessage, AiError>> {
    async move {
        let mut last_message = None;
        let mut events = stream_response(model, context, options)?;
        while let Some(event) = events.next().await {
            match event? {
                AssistantEvent::Done { message, .. } => last_message = Some(message),
                AssistantEvent::Error { error, .. } => last_message = Some(error),
                _ => {}
            }
        }
        last_message.ok_or_else(|| AiError::Message("stream ended without terminal event".into()))
    }
}

pub fn stream_response(
    model: Model,
    context: Context,
    options: StreamOptions,
) -> Result<AssistantEventStream, AiError> {
    ensure_builtin_providers_registered();
    let provider = registry()
        .lock()
        .unwrap()
        .get(&model.api)
        .cloned()
        .ok_or_else(|| AiError::UnknownApi(model.api.clone()))?;
    Ok(provider.stream(model, context, options))
}

#[derive(Debug, Clone)]
pub struct FauxModelDefinition {
    pub id: String,
    pub name: Option<String>,
    pub reasoning: bool,
}

#[derive(Debug, Clone)]
pub enum FauxContentBlock {
    Text(String),
    Thinking(String),
    ToolCall {
        id: String,
        name: String,
        arguments: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone)]
pub struct FauxResponse {
    pub content: Vec<FauxContentBlock>,
    pub stop_reason: StopReason,
    pub error_message: Option<String>,
}

impl FauxResponse {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![FauxContentBlock::Text(text.into())],
            stop_reason: StopReason::Stop,
            error_message: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegisterFauxProviderOptions {
    pub api: String,
    pub provider: String,
    pub models: Vec<FauxModelDefinition>,
    pub token_chunk_chars: usize,
    pub chunk_delay: Duration,
}

impl Default for RegisterFauxProviderOptions {
    fn default() -> Self {
        Self {
            api: "faux".into(),
            provider: "faux".into(),
            models: vec![FauxModelDefinition {
                id: "faux-1".into(),
                name: Some("Faux Model".into()),
                reasoning: false,
            }],
            token_chunk_chars: 4,
            chunk_delay: Duration::from_millis(0),
        }
    }
}

#[derive(Clone)]
pub struct FauxRegistration {
    api: String,
    models: Vec<Model>,
    state: Arc<Mutex<FauxState>>,
}

impl FauxRegistration {
    pub fn models(&self) -> &[Model] {
        &self.models
    }

    pub fn get_model(&self, id: Option<&str>) -> Option<Model> {
        match id {
            Some(id) => self.models.iter().find(|m| m.id == id).cloned(),
            None => self.models.first().cloned(),
        }
    }

    pub fn set_responses(&self, responses: Vec<FauxResponse>) {
        let mut state = self.state.lock().unwrap();
        state.pending = responses.into();
    }

    pub fn pending_response_count(&self) -> usize {
        self.state.lock().unwrap().pending.len()
    }

    pub fn call_count(&self) -> usize {
        self.state.lock().unwrap().call_count
    }

    pub fn unregister(&self) {
        unregister_provider(&self.api);
    }
}

#[derive(Debug, Default)]
struct FauxState {
    pending: VecDeque<FauxResponse>,
    call_count: usize,
    prompt_cache: HashMap<String, String>,
}

#[derive(Clone)]
struct FauxProvider {
    provider_name: String,
    state: Arc<Mutex<FauxState>>,
    token_chunk_chars: usize,
    chunk_delay: Duration,
}

pub fn register_faux_provider(options: RegisterFauxProviderOptions) -> FauxRegistration {
    let api = if options.api == "faux" {
        format!("faux:{}:{}", now_ms(), unique_suffix())
    } else {
        options.api.clone()
    };
    let models: Vec<Model> = options
        .models
        .iter()
        .map(|definition| Model {
            id: definition.id.clone(),
            name: definition
                .name
                .clone()
                .unwrap_or_else(|| definition.id.clone()),
            api: api.clone(),
            provider: options.provider.clone(),
            base_url: "http://localhost:0".into(),
            reasoning: definition.reasoning,
            input: vec!["text".into(), "image".into()],
            context_window: 128_000,
            max_tokens: 16_384,
        })
        .collect();
    let state = Arc::new(Mutex::new(FauxState::default()));
    register_provider(
        api.clone(),
        Arc::new(FauxProvider {
            provider_name: options.provider.clone(),
            state: state.clone(),
            token_chunk_chars: options.token_chunk_chars.max(1),
            chunk_delay: options.chunk_delay,
        }),
    );
    FauxRegistration { api, models, state }
}

impl AiProvider for FauxProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        options: StreamOptions,
    ) -> AssistantEventStream {
        let state = self.state.clone();
        let provider_name = self.provider_name.clone();
        let chunk_chars = self.token_chunk_chars;
        let chunk_delay = self.chunk_delay;
        Box::pin(stream! {
            let response = {
                let mut guard = state.lock().unwrap();
                guard.call_count += 1;
                guard.pending.pop_front()
            };
            let Some(response) = response else {
                let error = build_error_message(&model, &provider_name, AiError::NoMoreFauxResponses, &context, &state, &options);
                yield Ok(AssistantEvent::Error { reason: StopReason::Error, error });
                return;
            };

            let mut partial = AssistantMessage::empty(model.api.clone(), provider_name.clone(), model.id.clone());
            partial.timestamp = now_ms();
            partial.stop_reason = response.stop_reason.clone();

            if is_aborted(&options.signal) {
                let aborted = aborted_message(partial.clone(), &context, &state, &options);
                yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted });
                return;
            }

            yield Ok(AssistantEvent::Start { partial: partial.clone() });

            for (index, block) in response.content.iter().enumerate() {
                if is_aborted(&options.signal) {
                    let aborted = aborted_message(partial.clone(), &context, &state, &options);
                    yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted });
                    return;
                }
                match block {
                    FauxContentBlock::Text(text) => {
                        partial.content.push(AssistantContent::Text { text: String::new(), text_signature: None });
                        yield Ok(AssistantEvent::TextStart { content_index: index, partial: partial.clone() });
                        for chunk in chunk_string(text, chunk_chars) {
                            if chunk_delay.as_millis() > 0 { sleep(chunk_delay).await; }
                            if let Some(AssistantContent::Text { text, .. }) = partial.content.get_mut(index) {
                                text.push_str(&chunk);
                            }
                            yield Ok(AssistantEvent::TextDelta { content_index: index, delta: chunk, partial: partial.clone() });
                            if is_aborted(&options.signal) {
                                let aborted = aborted_message(partial.clone(), &context, &state, &options);
                                yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted });
                                return;
                            }
                        }
                        yield Ok(AssistantEvent::TextEnd { content_index: index, content: text.clone(), partial: partial.clone() });
                    }
                    FauxContentBlock::Thinking(thinking) => {
                        partial.content.push(AssistantContent::Thinking { thinking: String::new(), thinking_signature: None, redacted: false });
                        yield Ok(AssistantEvent::ThinkingStart { content_index: index, partial: partial.clone() });
                        for chunk in chunk_string(thinking, chunk_chars) {
                            if chunk_delay.as_millis() > 0 { sleep(chunk_delay).await; }
                            if let Some(AssistantContent::Thinking { thinking, .. }) = partial.content.get_mut(index) {
                                thinking.push_str(&chunk);
                            }
                            yield Ok(AssistantEvent::ThinkingDelta { content_index: index, delta: chunk, partial: partial.clone() });
                            if is_aborted(&options.signal) {
                                let aborted = aborted_message(partial.clone(), &context, &state, &options);
                                yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted });
                                return;
                            }
                        }
                        yield Ok(AssistantEvent::ThinkingEnd { content_index: index, content: thinking.clone(), partial: partial.clone() });
                    }
                    FauxContentBlock::ToolCall { id, name, arguments } => {
                        partial.content.push(AssistantContent::ToolCall { id: id.clone(), name: name.clone(), arguments: BTreeMap::new(), thought_signature: None });
                        yield Ok(AssistantEvent::ToolCallStart { content_index: index, partial: partial.clone() });
                        let json = serde_json::to_string(arguments).unwrap();
                        for chunk in chunk_string(&json, chunk_chars) {
                            if chunk_delay.as_millis() > 0 { sleep(chunk_delay).await; }
                            yield Ok(AssistantEvent::ToolCallDelta { content_index: index, delta: chunk, partial: partial.clone() });
                            if is_aborted(&options.signal) {
                                let aborted = aborted_message(partial.clone(), &context, &state, &options);
                                yield Ok(AssistantEvent::Error { reason: StopReason::Aborted, error: aborted });
                                return;
                            }
                        }
                        partial.content[index] = AssistantContent::ToolCall { id: id.clone(), name: name.clone(), arguments: arguments.clone(), thought_signature: None };
                        yield Ok(AssistantEvent::ToolCallEnd { content_index: index, tool_call: partial.content[index].clone(), partial: partial.clone() });
                    }
                }
            }

            let final_message = finalize_message(partial, response.error_message, &context, &state, &options);
            match final_message.stop_reason {
                StopReason::Error | StopReason::Aborted => {
                    yield Ok(AssistantEvent::Error { reason: final_message.stop_reason.clone(), error: final_message });
                }
                _ => {
                    yield Ok(AssistantEvent::Done { reason: final_message.stop_reason.clone(), message: final_message });
                }
            }
        })
    }
}

fn build_error_message(
    model: &Model,
    provider_name: &str,
    error: AiError,
    context: &Context,
    state: &Arc<Mutex<FauxState>>,
    options: &StreamOptions,
) -> AssistantMessage {
    finalize_message(
        AssistantMessage {
            role: "assistant".into(),
            content: Vec::new(),
            api: model.api.clone(),
            provider: provider_name.into(),
            model: model.id.clone(),
            response_id: None,
            usage: Usage::default(),
            stop_reason: StopReason::Error,
            error_message: Some(error.to_string()),
            timestamp: now_ms(),
        },
        Some(error.to_string()),
        context,
        state,
        options,
    )
}

fn aborted_message(
    partial: AssistantMessage,
    context: &Context,
    state: &Arc<Mutex<FauxState>>,
    options: &StreamOptions,
) -> AssistantMessage {
    finalize_message(
        AssistantMessage {
            stop_reason: StopReason::Aborted,
            error_message: Some(AiError::Aborted.to_string()),
            ..partial
        },
        Some(AiError::Aborted.to_string()),
        context,
        state,
        options,
    )
}

fn finalize_message(
    mut message: AssistantMessage,
    error_message: Option<String>,
    context: &Context,
    state: &Arc<Mutex<FauxState>>,
    options: &StreamOptions,
) -> AssistantMessage {
    message.error_message = error_message.or(message.error_message.clone());
    message.timestamp = now_ms();
    message.usage = estimate_usage(message.content.as_slice(), context, state, options);
    message
}

fn estimate_usage(
    content: &[AssistantContent],
    context: &Context,
    state: &Arc<Mutex<FauxState>>,
    options: &StreamOptions,
) -> Usage {
    let prompt = serialize_context(context);
    let prompt_tokens = estimate_tokens(&prompt);
    let output_tokens = estimate_tokens(&assistant_content_to_text(content));
    let mut input = prompt_tokens;
    let mut cache_read = 0;
    let mut cache_write = 0;
    if let Some(session_id) = &options.session_id {
        if options.cache_retention != CacheRetention::None {
            let mut guard = state.lock().unwrap();
            if let Some(previous) = guard.prompt_cache.get(session_id).cloned() {
                let prefix = common_prefix_len(&previous, &prompt);
                cache_read = estimate_tokens(&previous[..prefix]);
                cache_write = estimate_tokens(&prompt[prefix..]);
                input = prompt_tokens.saturating_sub(cache_read);
            } else {
                cache_write = prompt_tokens;
            }
            guard.prompt_cache.insert(session_id.clone(), prompt);
        }
    }
    Usage {
        input,
        output: output_tokens,
        cache_read,
        cache_write,
        total_tokens: input + output_tokens + cache_read + cache_write,
        ..Usage::default()
    }
}

fn estimate_tokens(text: &str) -> u64 {
    text.chars().count().div_ceil(4) as u64
}

fn serialize_context(context: &Context) -> String {
    let mut parts = Vec::new();
    if let Some(system_prompt) = &context.system_prompt {
        parts.push(format!("system:{system_prompt}"));
    }
    for message in &context.messages {
        parts.push(match message {
            Message::User { content, .. } => format!("user:{}", user_content_to_text(content)),
            Message::Assistant { content, .. } => {
                format!("assistant:{}", assistant_content_to_text(content))
            }
            Message::ToolResult {
                tool_name, content, ..
            } => {
                format!("toolResult:{tool_name}\n{}", user_content_to_text(content))
            }
        });
    }
    parts.join("\n\n")
}

fn assistant_content_to_text(content: &[AssistantContent]) -> String {
    content
        .iter()
        .map(|block| match block {
            AssistantContent::Text { text, .. } => text.clone(),
            AssistantContent::Thinking { thinking, .. } => thinking.clone(),
            AssistantContent::ToolCall {
                name, arguments, ..
            } => format!("{name}:{}", serde_json::to_string(arguments).unwrap()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn user_content_to_text(content: &[UserContent]) -> String {
    content
        .iter()
        .map(|block| match block {
            UserContent::Text { text } => text.clone(),
            UserContent::Image { data, mime_type } => format!("[image:{mime_type}:{}]", data.len()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(l, r)| l == r).count()
}

fn is_aborted(signal: &Option<tokio::sync::watch::Receiver<bool>>) -> bool {
    signal.as_ref().map(|s| *s.borrow()).unwrap_or(false)
}

fn chunk_string(input: &str, size: usize) -> Vec<String> {
    let chars = input.chars().collect::<Vec<_>>();
    chars
        .chunks(size.max(1))
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .into_iter()
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn unique_suffix() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
