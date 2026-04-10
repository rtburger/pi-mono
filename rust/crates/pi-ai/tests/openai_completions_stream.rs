use futures::StreamExt;
use pi_ai::openai_completions::{
    parse_openai_completions_sse_text, stream_openai_completions_sse_text,
};
use pi_events::{AssistantContent, AssistantEvent, Model, StopReason};

fn model() -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

#[test]
fn parses_sse_text_and_ignores_done_sentinel() {
    let payload = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: [DONE]\n\n"
    );

    let chunks = parse_openai_completions_sse_text(payload).unwrap();
    assert_eq!(chunks.len(), 1);
}

#[tokio::test]
async fn streams_text_response_events() {
    let payload = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let collected = stream_openai_completions_sse_text(model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    let actual = collected
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
        actual,
        vec!["start", "text_start", "text_delta", "text_end", "done"]
    );
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("chatcmpl-1"));
            assert_eq!(message.usage.total_tokens, 8);
            assert_eq!(
                message.content,
                vec![AssistantContent::Text {
                    text: "Hello".into(),
                    text_signature: None,
                }]
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn streams_tool_call_response_events() {
    let payload = concat!(
        "data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_1\",\"function\":{\"name\":\"edit\",\"arguments\":\"{\\\"path\\\":\\\"src/main.rs\\\"}\"}}]}}]}\n\n",
        "data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let collected = stream_openai_completions_sse_text(model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    let names = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start",
            AssistantEvent::ToolCallStart { .. } => "tool_call_start",
            AssistantEvent::ToolCallDelta { .. } => "tool_call_delta",
            AssistantEvent::ToolCallEnd { .. } => "tool_call_end",
            AssistantEvent::Done { .. } => "done",
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "start",
            "tool_call_start",
            "tool_call_delta",
            "tool_call_end",
            "done"
        ]
    );
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { reason, message } => {
            assert_eq!(*reason, StopReason::ToolUse);
            assert!(matches!(
                message.content[0],
                AssistantContent::ToolCall { .. }
            ));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn streams_reasoning_response_events() {
    let payload = concat!(
        "data: {\"id\":\"chatcmpl-3\",\"choices\":[{\"delta\":{\"reasoning_content\":\"I reasoned\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-3\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let collected = stream_openai_completions_sse_text(model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    let names = collected
        .iter()
        .map(|event| match event.as_ref().unwrap() {
            AssistantEvent::Start { .. } => "start",
            AssistantEvent::ThinkingStart { .. } => "thinking_start",
            AssistantEvent::ThinkingDelta { .. } => "thinking_delta",
            AssistantEvent::ThinkingEnd { .. } => "thinking_end",
            AssistantEvent::Done { .. } => "done",
            other => panic!("unexpected event: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "start",
            "thinking_start",
            "thinking_delta",
            "thinking_end",
            "done"
        ]
    );
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(
                message.content,
                vec![AssistantContent::Thinking {
                    thinking: "I reasoned".into(),
                    thinking_signature: Some("reasoning_content".into()),
                    redacted: false,
                }]
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}
