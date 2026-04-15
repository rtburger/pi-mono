use httpmock::prelude::*;
use pi_ai::openai_completions::{
    OpenAiCompletionsFunctionChoice, OpenAiCompletionsToolChoice,
    OpenAiCompletionsToolChoiceFunction,
};
use pi_ai::openai_responses::OpenAiResponsesServiceTier;
use pi_ai::{
    FauxResponse, PayloadHook, RegisterFauxProviderOptions, SimpleStreamOptions, ThinkingLevel,
    complete_simple, register_faux_provider,
};
use pi_events::{AssistantContent, Context, Message, Model, ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

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

fn context_with_tool() -> Context {
    Context {
        tools: vec![ToolDefinition {
            name: "calculator".into(),
            description: "Calculate a result".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string" }
                },
                "required": ["expression"]
            }),
        }],
        ..base_context()
    }
}

fn openai_responses_model(base_url: String, max_tokens: u64) -> Model {
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
        max_tokens,
        compat: None,
    }
}

fn openai_completions_model(base_url: String, max_tokens: u64) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-completions".into(),
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
        max_tokens,
        compat: None,
    }
}

fn anthropic_model_with_id(id: &str, base_url: String, max_tokens: u64) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 200_000,
        max_tokens,
        compat: None,
    }
}

fn anthropic_model(base_url: String, max_tokens: u64) -> Model {
    anthropic_model_with_id("claude-sonnet-4-20250514", base_url, max_tokens)
}

fn openai_codex_model(base_url: String, max_tokens: u64) -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 272_000,
        max_tokens,
        compat: None,
    }
}

fn openai_codex_token() -> String {
    format!(
        "aaa.{}.bbb",
        "eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjX3Rlc3QifX0="
    )
}

async fn capture_simple_payload_with_context(
    model: Model,
    context: Context,
    mut options: SimpleStreamOptions,
) -> Value {
    let captured = Arc::new(Mutex::new(None));
    let captured_for_hook = Arc::clone(&captured);

    options.api_key = options.api_key.or_else(|| Some("test-key".into()));
    options.on_payload = Some(PayloadHook::new(move |payload, _model| {
        let captured_for_hook = Arc::clone(&captured_for_hook);
        async move {
            *captured_for_hook.lock().unwrap() = Some(payload.clone());
            Ok(Some(payload))
        }
    }));

    let _response = complete_simple(model, context, options).await.unwrap();

    captured
        .lock()
        .unwrap()
        .clone()
        .expect("payload should be captured before request failure")
}

async fn capture_simple_payload(model: Model) -> Value {
    capture_simple_payload_with_context(model, base_context(), SimpleStreamOptions::default()).await
}

#[tokio::test]
async fn simple_openai_responses_clamps_xhigh_and_defaults_max_output_tokens() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .header("authorization", "Bearer test-key")
            .body_contains("\"max_output_tokens\":32000")
            .body_contains("\"effort\":\"high\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        openai_responses_model(server.base_url(), 64_000),
        base_context(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            reasoning: Some(ThinkingLevel::Xhigh),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
}

#[tokio::test]
async fn simple_openai_completions_passes_tool_choice_into_request_body() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .header("authorization", "Bearer test-key")
            .body_contains("\"max_completion_tokens\":16384")
            .body_contains("\"tool_choice\":{")
            .body_contains("\"function\":{\"name\":\"calculator\"}")
            .body_contains("\"tools\":[{");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        openai_completions_model(server.base_url(), 16_384),
        context_with_tool(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            tool_choice: Some(OpenAiCompletionsToolChoice::Function(
                OpenAiCompletionsFunctionChoice {
                    choice_type: "function".into(),
                    function: OpenAiCompletionsToolChoiceFunction {
                        name: "calculator".into(),
                    },
                },
            )),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("chatcmpl-1"));
}

#[tokio::test]
async fn simple_anthropic_adjusts_max_tokens_for_non_adaptive_thinking() {
    let server = MockServer::start();
    let sse = concat!(
        "event: message_start\n",
        "data: {\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
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
        when.method(POST)
            .path("/messages")
            .header("x-api-key", "test-key")
            .body_contains("\"max_tokens\":40000")
            .body_contains("\"thinking\":{")
            .body_contains("\"budget_tokens\":16384")
            .body_contains("\"type\":\"enabled\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        anthropic_model(server.base_url(), 40_000),
        base_context(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            reasoning: Some(ThinkingLevel::High),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("msg_1"));
}

#[tokio::test]
async fn simple_anthropic_disables_thinking_for_budget_reasoning_models_by_default() {
    let payload = capture_simple_payload(anthropic_model_with_id(
        "claude-sonnet-4-5",
        "http://127.0.0.1:9".into(),
        40_000,
    ))
    .await;

    assert_eq!(
        payload.get("thinking"),
        Some(&json!({ "type": "disabled" }))
    );
    assert!(payload.get("output_config").is_none());
}

#[tokio::test]
async fn simple_anthropic_disables_thinking_for_adaptive_reasoning_models_by_default() {
    let payload = capture_simple_payload(anthropic_model_with_id(
        "claude-opus-4-6",
        "http://127.0.0.1:9".into(),
        40_000,
    ))
    .await;

    assert_eq!(
        payload.get("thinking"),
        Some(&json!({ "type": "disabled" }))
    );
    assert!(payload.get("output_config").is_none());
}

#[tokio::test]
async fn simple_openai_responses_passes_service_tier_into_request_body() {
    let payload = capture_simple_payload_with_context(
        openai_responses_model("http://127.0.0.1:9".into(), 64_000),
        base_context(),
        SimpleStreamOptions {
            service_tier: Some(OpenAiResponsesServiceTier::Flex),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(payload.get("service_tier"), Some(&json!("flex")));
}

#[tokio::test]
async fn simple_anthropic_maps_function_tool_choice_to_named_tool() {
    let payload = capture_simple_payload_with_context(
        anthropic_model("http://127.0.0.1:9".into(), 40_000),
        context_with_tool(),
        SimpleStreamOptions {
            tool_choice: Some(OpenAiCompletionsToolChoice::Function(
                OpenAiCompletionsFunctionChoice {
                    choice_type: "function".into(),
                    function: OpenAiCompletionsToolChoiceFunction {
                        name: "calculator".into(),
                    },
                },
            )),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(
        payload.get("tool_choice"),
        Some(&json!({ "type": "tool", "name": "calculator" }))
    );
}

#[tokio::test]
async fn simple_openai_responses_applies_payload_hook_replacement() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_hook\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_hook\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_hook\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_hook\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .header("authorization", "Bearer test-key")
            .body_contains("\"max_output_tokens\":7")
            .body_contains("\"compat_probe\":\"openai-responses\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        openai_responses_model(server.base_url(), 64_000),
        base_context(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            on_payload: Some(PayloadHook::new(|mut payload, _model| async move {
                let object = payload
                    .as_object_mut()
                    .ok_or_else(|| "payload must be a JSON object".to_string())?;
                object.insert("max_output_tokens".into(), json!(7));
                object.insert("compat_probe".into(), json!("openai-responses"));
                Ok(Some(payload))
            })),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_hook"));
}

#[tokio::test]
async fn simple_openai_completions_applies_payload_hook_replacement() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-hook\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-hook\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .header("authorization", "Bearer test-key")
            .body_contains("\"max_completion_tokens\":9")
            .body_contains("\"compat_probe\":\"openai-completions\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        openai_completions_model(server.base_url(), 16_384),
        base_context(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            on_payload: Some(PayloadHook::new(|mut payload, _model| async move {
                let object = payload
                    .as_object_mut()
                    .ok_or_else(|| "payload must be a JSON object".to_string())?;
                object.insert("max_completion_tokens".into(), json!(9));
                object.insert("compat_probe".into(), json!("openai-completions"));
                Ok(Some(payload))
            })),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("chatcmpl-hook"));
}

#[tokio::test]
async fn simple_openai_codex_applies_payload_hook_replacement() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_codex_hook\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_codex_hook\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_codex_hook\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.done\",\"response\":{\"id\":\"resp_codex_hook\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );

    let token = openai_codex_token();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/codex/responses")
            .header("authorization", format!("Bearer {token}").as_str())
            .body_contains("\"text\":{\"verbosity\":\"high\"}")
            .body_contains("\"compat_probe\":\"openai-codex\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        openai_codex_model(server.base_url(), 128_000),
        base_context(),
        SimpleStreamOptions {
            api_key: Some(token),
            on_payload: Some(PayloadHook::new(|mut payload, _model| async move {
                let object = payload
                    .as_object_mut()
                    .ok_or_else(|| "payload must be a JSON object".to_string())?;
                object.insert("text".into(), json!({ "verbosity": "high" }));
                object.insert("compat_probe".into(), json!("openai-codex"));
                Ok(Some(payload))
            })),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_codex_hook"));
}

#[tokio::test]
async fn simple_anthropic_passes_metadata_user_id() {
    let server = MockServer::start();
    let sse = concat!(
        "event: message_start\n",
        "data: {\"message\":{\"id\":\"msg_meta\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
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
        when.method(POST)
            .path("/messages")
            .header("x-api-key", "test-key")
            .body_contains("\"metadata\":{\"user_id\":\"user-123\"}")
            .body_contains("\"compat_probe\":\"anthropic\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        anthropic_model(server.base_url(), 40_000),
        base_context(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            metadata: BTreeMap::from([("user_id".into(), json!("user-123"))]),
            on_payload: Some(PayloadHook::new(|mut payload, _model| async move {
                let object = payload
                    .as_object_mut()
                    .ok_or_else(|| "payload must be a JSON object".to_string())?;
                object.insert("compat_probe".into(), json!("anthropic"));
                Ok(Some(payload))
            })),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("msg_meta"));
}

#[tokio::test]
async fn complete_simple_uses_registered_provider_dispatch() {
    let registration = register_faux_provider(RegisterFauxProviderOptions::default());
    registration.set_responses(vec![FauxResponse::text("Hello from faux")]);
    let model = registration.get_model(None).expect("faux model");

    let response = complete_simple(model, base_context(), SimpleStreamOptions::default())
        .await
        .unwrap();

    assert!(matches!(
        response.content.as_slice(),
        [AssistantContent::Text { text, .. }] if text == "Hello from faux"
    ));

    registration.unregister();
}
