use futures::StreamExt;
use pi_ai::openai_codex_responses::stream_openai_codex_sse_text;
use pi_events::{AssistantContent, AssistantEvent, Model, StopReason};

fn model() -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url: "https://chatgpt.com/backend-api".into(),
        reasoning: true,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 272_000,
        max_tokens: 128_000,
        compat: None,
    }
}

#[tokio::test]
async fn streams_codex_done_event_as_terminal_stop_response() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.content_part.added\",\"part\":{\"type\":\"output_text\",\"text\":\"\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.done\",\"response\":{\"id\":\"resp_1\"}}\n\n"
    );

    let collected = stream_openai_codex_sse_text(model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    let names = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start",
            AssistantEvent::TextStart { .. } => "text_start",
            AssistantEvent::TextDelta { .. } => "text_delta",
            AssistantEvent::TextEnd { .. } => "text_end",
            AssistantEvent::Done { .. } => "done",
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec!["start", "text_start", "text_delta", "text_end", "done"]
    );
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { reason, message } => {
            assert_eq!(*reason, StopReason::Stop);
            assert_eq!(message.response_id.as_deref(), Some("resp_1"));
            assert_eq!(
                message.content,
                vec![AssistantContent::Text {
                    text: "Hello".into(),
                    text_signature: Some(r#"{"v":1,"id":"msg_1"}"#.into()),
                }]
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn maps_codex_incomplete_event_to_length_stop_reason() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_2\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_2\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_2\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.incomplete\",\"response\":{\"id\":\"resp_2\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n"
    );

    let collected = stream_openai_codex_sse_text(model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { reason, message } => {
            assert_eq!(*reason, StopReason::Length);
            assert_eq!(message.stop_reason, StopReason::Length);
            assert_eq!(message.response_id.as_deref(), Some("resp_2"));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}
