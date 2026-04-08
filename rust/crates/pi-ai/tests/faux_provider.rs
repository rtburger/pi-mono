use futures::StreamExt;
use pi_ai::{
    CacheRetention, FauxContentBlock, FauxResponse, RegisterFauxProviderOptions, StreamOptions,
    complete, register_faux_provider, stream_response,
};
use pi_events::{AssistantContent, AssistantEvent, Context, Message, StopReason, UserContent};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};
use tokio::sync::watch;
use tokio::time::Duration;

fn user_context(text: &str) -> Context {
    Context {
        system_prompt: Some("Be concise.".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: text.into() }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

#[tokio::test]
async fn completes_with_estimated_usage() {
    let registration = register_faux_provider(RegisterFauxProviderOptions::default());
    registration.set_responses(vec![FauxResponse::text("hello world")]);

    let response = complete(
        registration.get_model(None).unwrap(),
        user_context("hi there"),
        StreamOptions::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        response.content,
        vec![AssistantContent::Text {
            text: "hello world".into(),
            text_signature: None,
        }]
    );
    assert!(response.usage.input > 0);
    assert!(response.usage.output > 0);
    assert_eq!(
        response.usage.total_tokens,
        response.usage.input + response.usage.output
    );
    assert_eq!(registration.call_count(), 1);

    registration.unregister();
}

#[tokio::test]
async fn matches_expected_event_order_fixture() {
    let registration = register_faux_provider(RegisterFauxProviderOptions {
        token_chunk_chars: 64,
        ..Default::default()
    });
    let mut args = BTreeMap::new();
    args.insert("text".into(), json!("hi"));
    registration.set_responses(vec![FauxResponse {
        content: vec![
            FauxContentBlock::Thinking("go".into()),
            FauxContentBlock::Text("ok".into()),
            FauxContentBlock::ToolCall {
                id: "tool-1".into(),
                name: "echo".into(),
                arguments: args,
            },
        ],
        stop_reason: StopReason::ToolUse,
        error_message: None,
    }]);

    let stream = stream_response(
        registration.get_model(None).unwrap(),
        user_context("hi"),
        StreamOptions::default(),
    )
    .unwrap();
    let actual = stream
        .map(|event| match event.unwrap() {
            AssistantEvent::Start { .. } => "start",
            AssistantEvent::ThinkingStart { .. } => "thinking_start",
            AssistantEvent::ThinkingDelta { .. } => "thinking_delta",
            AssistantEvent::ThinkingEnd { .. } => "thinking_end",
            AssistantEvent::TextStart { .. } => "text_start",
            AssistantEvent::TextDelta { .. } => "text_delta",
            AssistantEvent::TextEnd { .. } => "text_end",
            AssistantEvent::ToolCallStart { .. } => "tool_call_start",
            AssistantEvent::ToolCallDelta { .. } => "tool_call_delta",
            AssistantEvent::ToolCallEnd { .. } => "tool_call_end",
            AssistantEvent::Done { .. } => "done",
            AssistantEvent::Error { .. } => "error",
        })
        .collect::<Vec<_>>()
        .await;

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("faux_tool_event_order.json");
    let expected: Vec<String> =
        serde_json::from_str(&fs::read_to_string(fixture_path).unwrap()).unwrap();
    assert_eq!(actual, expected);

    registration.unregister();
}

#[tokio::test]
async fn aborts_mid_stream() {
    let registration = register_faux_provider(RegisterFauxProviderOptions {
        token_chunk_chars: 2,
        chunk_delay: Duration::from_millis(10),
        ..Default::default()
    });
    registration.set_responses(vec![FauxResponse::text("abcdefghijklmnopqrstuvwxyz")]);

    let (tx, rx) = watch::channel(false);
    let mut stream = stream_response(
        registration.get_model(None).unwrap(),
        user_context("hi"),
        StreamOptions {
            signal: Some(rx),
            ..Default::default()
        },
    )
    .unwrap();

    let mut saw_text_delta = false;
    let mut saw_error = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            AssistantEvent::TextDelta { .. } if !saw_text_delta => {
                saw_text_delta = true;
                tx.send(true).unwrap();
            }
            AssistantEvent::Error { reason, .. } => {
                assert_eq!(reason, StopReason::Aborted);
                saw_error = true;
                break;
            }
            _ => {}
        }
    }

    assert!(saw_text_delta);
    assert!(saw_error);
    registration.unregister();
}

#[tokio::test]
async fn simulates_prompt_caching_per_session() {
    let registration = register_faux_provider(RegisterFauxProviderOptions::default());
    registration.set_responses(vec![
        FauxResponse::text("first"),
        FauxResponse::text("second"),
    ]);

    let first = complete(
        registration.get_model(None).unwrap(),
        user_context("hello"),
        StreamOptions {
            session_id: Some("session-1".into()),
            cache_retention: CacheRetention::Short,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(first.usage.cache_read, 0);
    assert!(first.usage.cache_write > 0);

    let mut second_context = user_context("hello");
    second_context.messages.push(Message::Assistant {
        content: first.content.clone(),
        api: first.api.clone(),
        provider: first.provider.clone(),
        model: first.model.clone(),
        response_id: first.response_id.clone(),
        usage: first.usage.clone(),
        stop_reason: first.stop_reason.clone(),
        error_message: first.error_message.clone(),
        timestamp: first.timestamp,
    });
    second_context.messages.push(Message::User {
        content: vec![UserContent::Text {
            text: "follow up".into(),
        }],
        timestamp: 2,
    });

    let second = complete(
        registration.get_model(None).unwrap(),
        second_context,
        StreamOptions {
            session_id: Some("session-1".into()),
            cache_retention: CacheRetention::Short,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(second.usage.cache_read > 0);
    registration.unregister();
}
