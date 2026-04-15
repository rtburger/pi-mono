use futures::StreamExt;
use httpmock::prelude::*;
use pi_ai::{StreamOptions, complete, stream_response};
use pi_events::{
    AssistantContent, AssistantEvent, Context, Message, Model, StopReason, UserContent,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
    time::{Duration, sleep},
};

fn model_with(provider: &str, base_url: String, input: Vec<&str>) -> Model {
    Model {
        id: "claude-sonnet-4-20250514".into(),
        name: "claude-sonnet-4-20250514".into(),
        api: "anthropic-messages".into(),
        provider: provider.into(),
        base_url,
        reasoning: true,
        input: input.into_iter().map(str::to_string).collect(),
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 200_000,
        max_tokens: 8_192,
        compat: None,
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

async fn start_chunked_sse_server(chunks: Vec<(&'static str, u64)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        let mut header_end = None;

        while header_end.is_none() {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..read]);
            header_end = request.windows(4).position(|window| window == b"\r\n\r\n");
        }

        let header_end = header_end.unwrap() + 4;
        let header_text = String::from_utf8_lossy(&request[..header_end]);
        let content_length = header_text
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let mut body_received = request.len().saturating_sub(header_end);
        while body_received < content_length {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            body_received += read;
        }

        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();

        for (chunk, delay_ms) in chunks {
            if delay_ms > 0 {
                sleep(Duration::from_millis(delay_ms)).await;
            }
            socket
                .write_all(format!("{:X}\r\n", chunk.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(chunk.as_bytes()).await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
        }

        socket.write_all(b"0\r\n\r\n").await.unwrap();
    });

    format!("http://{address}")
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
async fn emits_aborted_terminal_error_before_http_send() {
    let (tx, rx) = watch::channel(false);
    tx.send(true).unwrap();

    let response = complete(
        model("https://api.anthropic.com/v1".into()),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            signal: Some(rx),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.stop_reason, StopReason::Aborted);
    assert_eq!(
        response.error_message.as_deref(),
        Some("Request was aborted")
    );
}

#[tokio::test]
async fn aborts_while_waiting_for_next_http_body_chunk() {
    let base_url = start_chunked_sse_server(vec![
        (
            concat!(
                "event: message_start\n",
                "data: {\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n",
                "event: content_block_start\n",
                "data: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n"
            ),
            0,
        ),
        (
            concat!(
                "event: content_block_stop\n",
                "data: {\"index\":0}\n\n",
                "event: message_delta\n",
                "data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n"
            ),
            250,
        ),
    ])
    .await;

    let (tx, rx) = watch::channel(false);
    let mut stream = stream_response(
        model(base_url),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            signal: Some(rx),
            ..Default::default()
        },
    )
    .unwrap();
    let mut names = Vec::new();

    while let Some(event) = stream.next().await {
        match event.unwrap() {
            AssistantEvent::Start { .. } => names.push("start"),
            AssistantEvent::TextStart { .. } => names.push("text_start"),
            AssistantEvent::TextDelta { .. } => {
                names.push("text_delta");
                tx.send(true).unwrap();
            }
            AssistantEvent::Error { reason, error } => {
                names.push("error");
                assert_eq!(reason, StopReason::Aborted);
                assert_eq!(error.error_message.as_deref(), Some("Request was aborted"));
                assert_eq!(
                    error.content,
                    vec![AssistantContent::Text {
                        text: "Hello".into(),
                        text_signature: None,
                    }]
                );
                break;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    assert_eq!(names, vec!["start", "text_start", "text_delta", "error"]);
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
