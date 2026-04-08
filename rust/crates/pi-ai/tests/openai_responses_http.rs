use futures::StreamExt;
use httpmock::prelude::*;
use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, OpenAiResponsesParamsOptions,
    build_openai_responses_request_params, stream_openai_responses_http,
};
use pi_events::{AssistantEvent, Context, Message, Model, StopReason, UserContent};
use tokio::sync::watch;

fn model(base_url: String) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn context() -> Context {
    Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: "hi".into() }],
            timestamp: 1,
        }],
    }
}

#[tokio::test]
async fn streams_openai_responses_over_http() {
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
            .header("accept", "text/event-stream");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let model = model(server.base_url());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex", "opencode"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    let collected = stream_openai_responses_http(model, params, "test-key".into(), None)
        .collect::<Vec<_>>()
        .await;

    mock.assert();
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("resp_1"));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn emits_terminal_error_for_http_failure() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/responses");
        then.status(500).body("boom");
    });

    let model = model(server.base_url());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex", "opencode"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    let collected = stream_openai_responses_http(model, params, "test-key".into(), None)
        .collect::<Vec<_>>()
        .await;

    mock.assert();
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Error { error, .. } => {
            assert!(
                error
                    .error_message
                    .as_deref()
                    .unwrap()
                    .contains("HTTP request failed with status 500")
            );
        }
        other => panic!("expected error event, got {other:?}"),
    }
}

#[tokio::test]
async fn passes_runtime_options_into_http_request_body() {
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
            .body_contains("\"max_output_tokens\":123")
            .body_contains("\"temperature\":0.5")
            .body_contains("\"effort\":\"high\"")
            .body_contains("\"summary\":\"detailed\"")
            .body_contains("\"prompt_cache_key\":\"session-1\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let model = model(server.base_url());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex", "opencode"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions {
            max_output_tokens: Some(123),
            temperature: Some(0.5),
            reasoning_effort: Some("high".into()),
            reasoning_summary: Some("detailed".into()),
            session_id: Some("session-1".into()),
            cache_retention: Some("short".into()),
        },
    );

    let collected = stream_openai_responses_http(model, params, "test-key".into(), None)
        .collect::<Vec<_>>()
        .await;

    mock.assert();
    assert!(matches!(
        collected.last().unwrap().as_ref().unwrap(),
        AssistantEvent::Done { .. }
    ));
}

#[tokio::test]
async fn emits_aborted_terminal_error_before_http_send() {
    let model = model("https://api.openai.com/v1".into());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex", "opencode"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );
    let (tx, rx) = watch::channel(false);
    tx.send(true).unwrap();

    let collected = stream_openai_responses_http(model, params, "test-key".into(), Some(rx))
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Error { reason, error } => {
            assert_eq!(*reason, StopReason::Aborted);
            assert_eq!(error.error_message.as_deref(), Some("Request was aborted"));
        }
        other => panic!("expected aborted error event, got {other:?}"),
    }
}
