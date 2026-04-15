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
    time::{Duration, sleep, timeout},
};

fn model_with(id: &str, base_url: String) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 272_000,
        max_tokens: 128_000,
    }
}

fn model(base_url: String) -> Model {
    model_with("gpt-5.2-codex", base_url)
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

fn mock_token() -> String {
    format!(
        "aaa.{}.bbb",
        "eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjX3Rlc3QifX0="
    )
}

fn completed_sse() -> &'static str {
    concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    )
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
async fn dispatches_openai_codex_responses_through_registry_with_codex_headers() {
    let server = MockServer::start();
    let token = mock_token();

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/codex/responses")
            .header("authorization", format!("Bearer {token}"))
            .header("chatgpt-account-id", "acc_test")
            .header("originator", "pi")
            .header("openai-beta", "responses=experimental")
            .header("accept", "text/event-stream");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(completed_sse());
    });

    let mut stream = stream_response(
        model(server.base_url()),
        context(),
        StreamOptions {
            api_key: Some(token),
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
                assert!(message.usage.cost.total > 0.0);
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
async fn completes_after_response_completed_even_when_sse_body_stays_open() {
    let base_url =
        start_chunked_sse_server(vec![(completed_sse(), 0), ("data: [DONE]\n\n", 1_000)]).await;

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
    .expect("timed out waiting for completed Codex SSE stream")
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert_eq!(response.stop_reason, pi_events::StopReason::Stop);
    assert!(response.usage.cost.total > 0.0);
}

#[tokio::test]
async fn sends_session_headers_and_in_memory_prompt_cache_fields() {
    let server = MockServer::start();
    let token = mock_token();

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/codex/responses")
            .header("session_id", "session-1")
            .header("conversation_id", "session-1")
            .body_contains("\"prompt_cache_key\":\"session-1\"")
            .body_contains("\"prompt_cache_retention\":\"in-memory\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(completed_sse());
    });

    let response = complete(
        model(server.base_url()),
        context(),
        StreamOptions {
            api_key: Some(token),
            session_id: Some("session-1".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
}

#[tokio::test]
async fn clamps_minimal_reasoning_effort_for_newer_codex_models() {
    let server = MockServer::start();
    let token = mock_token();

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/codex/responses")
            .body_contains("\"effort\":\"low\"")
            .body_contains("\"summary\":\"auto\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(completed_sse());
    });

    let response = complete(
        model_with("gpt-5.3-codex", server.base_url()),
        context(),
        StreamOptions {
            api_key: Some(token),
            reasoning_effort: Some("minimal".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
}

#[tokio::test]
async fn emits_aborted_terminal_error_before_http_send() {
    let (tx, rx) = watch::channel(false);
    tx.send(true).unwrap();

    let response = complete(
        model("https://chatgpt.com/backend-api".into()),
        context(),
        StreamOptions {
            api_key: Some(mock_token()),
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
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_abort\"}}\n\n",
                "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n"
            ),
            0,
        ),
        (
            concat!(
                "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello there\"}]}}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_abort\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":4,\"total_tokens\":9,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
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
            api_key: Some(mock_token()),
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
        model("https://chatgpt.com/backend-api".into()),
        context(),
        StreamOptions::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        response.error_message.as_deref(),
        Some("OpenAI Codex API key is required")
    );
}
