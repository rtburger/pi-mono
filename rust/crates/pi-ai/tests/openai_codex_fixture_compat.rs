use futures::StreamExt;
use pi_ai::openai_codex_responses::stream_openai_codex_sse_text;
use pi_ai::{StreamOptions, complete};
use pi_events::{
    AssistantContent, AssistantEvent, Context, Message, Model, StopReason, UserContent,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::{Duration, sleep, timeout},
};

const COMPLETED_TERMINAL_FIXTURE: &str =
    include_str!("fixtures/openai_codex_sse_completed_terminal.sse");

fn model(base_url: String) -> Model {
    Model {
        id: "gpt-5.1-codex".into(),
        name: "gpt-5.1-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        context_window: 400_000,
        max_tokens: 128_000,
    }
}

fn context() -> Context {
    Context {
        system_prompt: Some("You are a helpful assistant.".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text {
                text: "Say hello".into(),
            }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

fn mock_token() -> String {
    format!(
        "aaa.{}.bbb",
        "eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjX3Rlc3QifX0="
    )
}

async fn start_chunked_sse_server(chunks: Vec<(String, u64)>) -> String {
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
async fn ts_completed_fixture_streams_to_terminal_done() {
    let events = stream_openai_codex_sse_text(
        model("https://chatgpt.com/backend-api".into()),
        COMPLETED_TERMINAL_FIXTURE,
    )
    .unwrap()
    .collect::<Vec<_>>()
    .await;

    let names = events
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

    match events.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { reason, message } => {
            assert_eq!(*reason, StopReason::Stop);
            assert_eq!(message.stop_reason, StopReason::Stop);
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
async fn ts_completed_fixture_finishes_before_delayed_done_marker() {
    let base_url = start_chunked_sse_server(vec![
        (COMPLETED_TERMINAL_FIXTURE.to_string(), 0),
        ("data: [DONE]\n\n".to_string(), 1_000),
    ])
    .await;

    let response = timeout(
        Duration::from_millis(250),
        complete(
            model(base_url),
            context(),
            StreamOptions {
                api_key: Some(mock_token()),
                ..Default::default()
            },
        ),
    )
    .await
    .expect("timed out waiting for Codex terminal completion from TS fixture")
    .unwrap();

    assert_eq!(response.stop_reason, StopReason::Stop);
    assert_eq!(
        response.content,
        vec![AssistantContent::Text {
            text: "Hello".into(),
            text_signature: Some(r#"{"v":1,"id":"msg_1"}"#.into()),
        }]
    );
}
