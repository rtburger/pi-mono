use futures::StreamExt;
use httpmock::prelude::*;
use pi_ai::{StreamOptions, complete, stream_response};
use pi_events::{AssistantEvent, Context, Message, Model, UserContent};

fn model_with(provider: &str, base_url: String, input: Vec<&str>) -> Model {
    Model {
        id: "claude-sonnet-4-20250514".into(),
        name: "claude-sonnet-4-20250514".into(),
        api: "anthropic-messages".into(),
        provider: provider.into(),
        base_url,
        reasoning: true,
        input: input.into_iter().map(str::to_string).collect(),
        context_window: 200_000,
        max_tokens: 8_192,
    }
}

fn model(base_url: String) -> Model {
    model_with("anthropic", base_url, vec!["text"])
}

fn context() -> Context {
    Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: "hi".into() }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

#[tokio::test]
async fn dispatches_anthropic_messages_through_registry() {
    let server = MockServer::start();
    let sse = concat!(
        "event: message_start\n",
        "data: {\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n",
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
            .header("anthropic-version", "2023-06-01");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let mut stream = stream_response(
        model(server.base_url()),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let mut saw_done = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            AssistantEvent::Done { message, .. } => {
                saw_done = true;
                assert_eq!(message.response_id.as_deref(), Some("msg_1"));
            }
            AssistantEvent::Error { error, .. } => {
                panic!("unexpected error: {:?}", error.error_message)
            }
            _ => {}
        }
    }

    mock.assert();
    assert!(saw_done);
}

#[tokio::test]
async fn dispatches_anthropic_oauth_with_claude_code_headers() {
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
            .header("authorization", "Bearer sk-ant-oat-test-key")
            .header("user-agent", "claude-cli/2.1.75")
            .header("x-app", "cli")
            .header(
                "anthropic-beta",
                "claude-code-20250219,oauth-2025-04-20,fine-grained-tool-streaming-2025-05-14,interleaved-thinking-2025-05-14",
            );
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete(
        model(server.base_url()),
        context(),
        StreamOptions {
            api_key: Some("sk-ant-oat-test-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("msg_1"));
}

#[tokio::test]
async fn complete_returns_terminal_error_without_api_key() {
    let response = complete(
        model("https://api.anthropic.com/v1".into()),
        context(),
        StreamOptions::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        response.error_message.as_deref(),
        Some("Anthropic Messages API key is required")
    );
}
