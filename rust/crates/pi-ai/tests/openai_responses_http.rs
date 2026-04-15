use futures::StreamExt;
use httpmock::prelude::*;
use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, OpenAiResponsesParamsOptions,
    build_openai_responses_request_params, stream_openai_responses_http,
};
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
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
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
async fn streams_openai_responses_over_http() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .header("authorization", "Bearer test-key")
            .header("accept", "text/event-stream");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let model = model(server.base_url());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    let collected = stream_openai_responses_http(model, params, "test-key".into(), None)
        .collect::<Vec<_>>()
        .await;

    mock.assert();
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("resp_1"));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn emits_terminal_error_for_http_failure() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/responses");
        then.status(500).body("boom");
    });

    let model = model(server.base_url());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    let collected = stream_openai_responses_http(model, params, "test-key".into(), None)
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
async fn passes_runtime_options_into_http_request_body() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .body_contains("\"max_output_tokens\":123")
            .body_contains("\"temperature\":0.5")
            .body_contains("\"effort\":\"high\"")
            .body_contains("\"summary\":\"detailed\"")
            .body_contains("\"prompt_cache_key\":\"session-1\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let model = model(server.base_url());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions {
            max_output_tokens: Some(123),
            temperature: Some(0.5),
            reasoning_effort: Some("high".into()),
            reasoning_summary: Some("detailed".into()),
            session_id: Some("session-1".into()),
            cache_retention: Some("short".into()),
        },
    );

    let collected = stream_openai_responses_http(model, params, "test-key".into(), None)
        .collect::<Vec<_>>()
        .await;

    mock.assert();
    assert!(matches!(
        collected.last().unwrap().as_ref().unwrap(),
        AssistantEvent::Done { .. }
    ));
}

#[tokio::test]
async fn emits_aborted_terminal_error_before_http_send() {
    let model = model("https://api.openai.com/v1".into());
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );
    let (tx, rx) = watch::channel(false);
    tx.send(true).unwrap();

    let collected = stream_openai_responses_http(model, params, "test-key".into(), Some(rx))
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Error { reason, error } => {
            assert_eq!(*reason, StopReason::Aborted);
            assert_eq!(error.error_message.as_deref(), Some("Request was aborted"));
        }
        other => panic!("expected aborted error event, got {other:?}"),
    }
}

#[tokio::test]
async fn streams_incrementally_across_http_body_chunks() {
    let base_url = start_chunked_sse_server(vec![
        (
            concat!(
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_streamed\"}}\n\n",
                "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
                "data: {\"type\":\"response.output_text.delta\",\"de"
            ),
            0,
        ),
        (
            concat!(
                "lta\":\"Hello\"}\n\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\" world\"}\n\n"
            ),
            25,
        ),
        (
            concat!(
                "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello world\"}]}}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_streamed\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":4,\"total_tokens\":9,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
            ),
            25,
        ),
    ])
    .await;

    let model = model(base_url);
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    let collected = stream_openai_responses_http(model, params, "test-key".into(), None)
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
        vec![
            "start",
            "text_start",
            "text_delta",
            "text_delta",
            "text_end",
            "done"
        ]
    );
    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("resp_streamed"));
            assert_eq!(message.usage.total_tokens, 9);
            assert_eq!(
                message.content,
                vec![AssistantContent::Text {
                    text: "Hello world".into(),
                    text_signature: Some(r#"{"v":1,"id":"msg_1"}"#.into()),
                }]
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
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

    let model = model(base_url);
    let params = build_openai_responses_request_params(
        &model,
        &context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );
    let (tx, rx) = watch::channel(false);

    let mut stream = stream_openai_responses_http(model, params, "test-key".into(), Some(rx));
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
