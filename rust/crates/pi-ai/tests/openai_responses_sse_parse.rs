use futures::StreamExt;
use pi_ai::openai_responses::{parse_openai_responses_sse_text, stream_openai_responses_sse_text};
use pi_events::{AssistantEvent, Model};
use std::{fs, path::PathBuf};

fn model() -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn fixture(name: &str) -> Vec<String> {
    serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn parses_sse_text_into_envelopes_and_ignores_done() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n",
        "data: [DONE]\n\n"
    );

    let actual = parse_openai_responses_sse_text(payload)
        .unwrap()
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();

    assert_eq!(actual, fixture("openai_responses_sse_parse.json"));
}

#[tokio::test]
async fn streams_directly_from_sse_text() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );

    let collected = stream_openai_responses_sse_text(model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("resp_1"));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[test]
fn errors_on_invalid_sse_json() {
    let payload = "data: {not-json}\n\n";
    let error = parse_openai_responses_sse_text(payload).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Invalid OpenAI Responses SSE event:")
    );
}
