use futures::StreamExt;
use httpmock::prelude::*;
use pi_ai::{StreamOptions, complete, stream_response};
use pi_events::{AssistantEvent, Context, Message, Model, UserContent};

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
async fn dispatches_openai_responses_through_registry() {
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
            .header("authorization", "Bearer test-key");
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
                assert_eq!(message.response_id.as_deref(), Some("resp_1"));
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
async fn complete_returns_terminal_error_without_api_key() {
    let response = complete(
        model("https://api.openai.com/v1".into()),
        context(),
        StreamOptions::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        response.error_message.as_deref(),
        Some("OpenAI Responses API key is required")
    );
}
