use futures::StreamExt;
use httpmock::prelude::*;
use pi_ai::openai_completions::{
    OpenAiCompletionsRequestOptions, build_openai_completions_request_params,
    detect_openai_completions_compat, stream_openai_completions_http,
};
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

fn model(base_url: String) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-completions".into(),
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
        tools: vec![],
    }
}

fn params(model: &Model) -> pi_ai::openai_completions::OpenAiCompletionsRequestParams {
    let compat = detect_openai_completions_compat(model);
    build_openai_completions_request_params(
        model,
        &context(),
        &compat,
        &OpenAiCompletionsRequestOptions::default(),
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
async fn streams_openai_completions_over_http() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .header("authorization", "Bearer test-key")
            .header("accept", "text/event-stream");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let model = model(server.base_url());
    let collected =
        stream_openai_completions_http(model.clone(), params(&model), "test-key".into(), None)
            .collect::<Vec<_>>()
            .await;

    mock.assert();
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("chatcmpl-1"));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn emits_terminal_error_for_http_failure() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(500).body("boom");
    });

    let model = model(server.base_url());
    let collected =
        stream_openai_completions_http(model.clone(), params(&model), "test-key".into(), None)
            .collect::<Vec<_>>()
            .await;

    mock.assert();
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Error { error, .. } => {
            assert!(
                error
                    .error_message
                    .as_deref()
                    .unwrap()
                    .contains("HTTP request failed with status 500")
            );
        }
        other => panic!("expected error event, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatches_openai_completions_through_registry() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
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
                assert_eq!(message.response_id.as_deref(), Some("chatcmpl-1"));
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
        Some("OpenAI Completions API key is required")
    );
}

#[tokio::test]
async fn aborts_while_waiting_for_next_http_body_chunk() {
    let base_url = start_chunked_sse_server(vec![
        (
            concat!(
                "data: {\"id\":\"chatcmpl-abort\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n"
            ),
            0,
        ),
        (
            concat!(
                "data: {\"id\":\"chatcmpl-abort\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":4,\"total_tokens\":9,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
                "data: [DONE]\n\n"
            ),
            250,
        ),
    ])
    .await;

    let model = model(base_url);
    let (tx, rx) = watch::channel(false);

    let mut stream =
        stream_openai_completions_http(model.clone(), params(&model), "test-key".into(), Some(rx));
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
