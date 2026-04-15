use httpmock::prelude::*;
use pi_ai::openai_responses::OpenAiResponsesServiceTier;
use pi_ai::{StreamOptions, complete};
use pi_events::{Context, Message, Model, UserContent};

fn model(base_url: String) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url,
        reasoning: false,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 2.0,
            cache_read: 0.5,
            cache_write: 0.0,
        },
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    }
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

fn assert_close(left: f64, right: f64) {
    assert!(
        (left - right).abs() < 1e-12,
        "expected {left} to equal {right}"
    );
}

#[tokio::test]
async fn openai_responses_service_tier_scales_usage_costs() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":3000000,\"output_tokens\":2000000,\"total_tokens\":5000000,\"input_tokens_details\":{\"cached_tokens\":1000000}}}}\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .header("authorization", "Bearer test-key")
            .body_contains("\"service_tier\":\"priority\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete(
        model(server.base_url()),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            service_tier: Some(OpenAiResponsesServiceTier::Priority),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert_close(response.usage.cost.input, 4.0);
    assert_close(response.usage.cost.output, 8.0);
    assert_close(response.usage.cost.cache_read, 1.0);
    assert_close(response.usage.cost.cache_write, 0.0);
    assert_close(response.usage.cost.total, 13.0);
}
