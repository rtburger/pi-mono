use httpmock::prelude::*;
use parking_lot::Mutex;
use pi_ai::{CacheRetention, PayloadHook, StreamOptions, complete};
use pi_events::{Context, Message, Model, UserContent};
use serde_json::{Value, json};
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

static ENV_LOCK: TokioMutex<()> = TokioMutex::const_new(());
const PI_CACHE_RETENTION: &str = "PI_CACHE_RETENTION";

fn base_context() -> Context {
    Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: "hi".into() }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

fn openai_responses_model(base_url: String) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 128_000,
        max_tokens: 64_000,
        compat: None,
    }
}

fn anthropic_model(base_url: String) -> Model {
    Model {
        id: "claude-3-5-haiku-20241022".into(),
        name: "claude-3-5-haiku-20241022".into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url,
        reasoning: false,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 200_000,
        max_tokens: 8_192,
        compat: None,
    }
}

fn capture_payload_hook(captured: Arc<Mutex<Option<Value>>>) -> PayloadHook {
    PayloadHook::new(move |payload, _model| {
        let captured = captured.clone();
        async move {
            *captured.lock() = Some(payload.clone());
            Ok(None)
        }
    })
}

fn captured_payload(captured: &Arc<Mutex<Option<Value>>>) -> Value {
    captured.lock().clone().expect("payload should be captured")
}

fn set_env_var(key: &str, value: &str) {
    unsafe { env::set_var(key, value) };
}

fn restore_env_var(key: &str, previous: Option<String>) {
    match previous {
        Some(value) => unsafe { env::set_var(key, value) },
        None => unsafe { env::remove_var(key) },
    }
}

#[tokio::test]
async fn openai_responses_uses_env_long_cache_retention_when_option_is_omitted() {
    let _env_guard = ENV_LOCK.lock().await;
    let previous = env::var(PI_CACHE_RETENTION).ok();
    set_env_var(PI_CACHE_RETENTION, "long");

    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_cache\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_cache\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_cache\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_cache\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );
    let mock = server.mock(|when, then| {
        when.method(POST).path("/api.openai.com/responses");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let captured = Arc::new(Mutex::new(None));
    let response = complete(
        openai_responses_model(format!("{}/api.openai.com", server.base_url())),
        base_context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            session_id: Some("session-1".into()),
            on_payload: Some(capture_payload_hook(captured.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_cache"));
    let payload = captured_payload(&captured);
    assert_eq!(payload.get("prompt_cache_key"), Some(&json!("session-1")));
    assert_eq!(payload.get("prompt_cache_retention"), Some(&json!("24h")));

    restore_env_var(PI_CACHE_RETENTION, previous);
}

#[tokio::test]
async fn openai_responses_explicit_short_overrides_env_long_cache_retention() {
    let _env_guard = ENV_LOCK.lock().await;
    let previous = env::var(PI_CACHE_RETENTION).ok();
    set_env_var(PI_CACHE_RETENTION, "long");

    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_short\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_short\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_short\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_short\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );
    let mock = server.mock(|when, then| {
        when.method(POST).path("/api.openai.com/responses");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let captured = Arc::new(Mutex::new(None));
    let response = complete(
        openai_responses_model(format!("{}/api.openai.com", server.base_url())),
        base_context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            session_id: Some("session-1".into()),
            cache_retention: Some(CacheRetention::Short),
            on_payload: Some(capture_payload_hook(captured.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_short"));
    let payload = captured_payload(&captured);
    assert_eq!(payload.get("prompt_cache_key"), Some(&json!("session-1")));
    assert!(payload.get("prompt_cache_retention").is_none());

    restore_env_var(PI_CACHE_RETENTION, previous);
}

#[tokio::test]
async fn anthropic_uses_env_long_cache_retention_when_option_is_omitted() {
    let _env_guard = ENV_LOCK.lock().await;
    let previous = env::var(PI_CACHE_RETENTION).ok();
    set_env_var(PI_CACHE_RETENTION, "long");

    let server = MockServer::start();
    let sse = concat!(
        "event: message_start\n",
        "data: {\"message\":{\"id\":\"msg_env\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\n",
        "data: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n"
    );
    let mock = server.mock(|when, then| {
        when.method(POST).path("/api.anthropic.com/messages");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let captured = Arc::new(Mutex::new(None));
    let response = complete(
        anthropic_model(format!("{}/api.anthropic.com", server.base_url())),
        base_context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            on_payload: Some(capture_payload_hook(captured.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("msg_env"));
    let payload = captured_payload(&captured);
    let system = payload
        .get("system")
        .and_then(Value::as_array)
        .expect("system blocks");
    assert_eq!(
        system[0].get("cache_control"),
        Some(&json!({ "type": "ephemeral", "ttl": "1h" }))
    );

    restore_env_var(PI_CACHE_RETENTION, previous);
}

#[tokio::test]
async fn anthropic_explicit_short_overrides_env_long_cache_retention() {
    let _env_guard = ENV_LOCK.lock().await;
    let previous = env::var(PI_CACHE_RETENTION).ok();
    set_env_var(PI_CACHE_RETENTION, "long");

    let server = MockServer::start();
    let sse = concat!(
        "event: message_start\n",
        "data: {\"message\":{\"id\":\"msg_short\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\n",
        "data: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n"
    );
    let mock = server.mock(|when, then| {
        when.method(POST).path("/api.anthropic.com/messages");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let captured = Arc::new(Mutex::new(None));
    let response = complete(
        anthropic_model(format!("{}/api.anthropic.com", server.base_url())),
        base_context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            cache_retention: Some(CacheRetention::Short),
            on_payload: Some(capture_payload_hook(captured.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("msg_short"));
    let payload = captured_payload(&captured);
    let system = payload
        .get("system")
        .and_then(Value::as_array)
        .expect("system blocks");
    assert_eq!(
        system[0].get("cache_control"),
        Some(&json!({ "type": "ephemeral" }))
    );

    restore_env_var(PI_CACHE_RETENTION, previous);
}
