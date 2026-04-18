use futures::StreamExt;
use pi_ai::openai_completions::stream_openai_completions_sse_text;
use pi_events::{AssistantEvent, Model, ModelCost};

fn model() -> Model {
    Model {
        id: "gpt-oss-custom".into(),
        name: "gpt-oss-custom".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 0.0,
            output: 1_000_000.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    }
}

#[tokio::test]
async fn streamed_reasoning_tokens_contribute_to_total_tokens_and_cost() {
    let payload = concat!(
        "data: {\"id\":\"chatcmpl-usage-1\",\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-usage-1\",\"choices\":[{\"delta\":{\"content\":\"answer\"}}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12,\"prompt_tokens_details\":{\"cached_tokens\":0},\"completion_tokens_details\":{\"reasoning_tokens\":3}}}\n\n",
        "data: {\"id\":\"chatcmpl-usage-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n"
    );

    let mut stream = stream_openai_completions_sse_text(model(), payload).expect("stream");
    let mut done = None;

    while let Some(event) = stream.next().await {
        match event.expect("assistant event") {
            AssistantEvent::Done { message, .. } => done = Some(message),
            AssistantEvent::Error { error, .. } => panic!("unexpected error: {error:?}"),
            _ => {}
        }
    }

    let message = done.expect("done message");
    assert_eq!(message.usage.input, 10);
    assert_eq!(message.usage.output, 5);
    assert_eq!(message.usage.total_tokens, 15);
    assert_eq!(message.usage.cost.output, 5.0);
    assert_eq!(message.usage.cost.total, 5.0);
}
