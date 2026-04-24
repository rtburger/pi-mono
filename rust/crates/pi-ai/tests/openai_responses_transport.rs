use httpdate::fmt_http_date;
use pi_ai::{StreamOptions, complete};
use pi_events::{Context, Message, Model, StopReason, UserContent};
use std::time::{Duration, SystemTime};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    time::{Instant, sleep, timeout},
};

fn model(base_url: String) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into(), "image".into()],
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

fn completed_sse() -> &'static str {
    concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    )
}

async fn read_http_request(socket: &mut TcpStream) -> (String, String) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut header_end = None;

    while header_end.is_none() {
        let read = socket.read(&mut buffer).await.unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        header_end = request.windows(4).position(|window| window == b"\r\n\r\n");
    }

    let header_end = header_end.map(|index| index + 4).unwrap_or(request.len());
    let header_text = String::from_utf8_lossy(&request[..header_end]).into_owned();
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

    let mut body = request[header_end..].to_vec();
    while body.len() < content_length {
        let read = socket.read(&mut buffer).await.unwrap();
        if read == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..read]);
    }

    (
        header_text,
        String::from_utf8_lossy(&body[..content_length.min(body.len())]).into_owned(),
    )
}

async fn write_http_response(
    socket: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    extra_headers: &[(String, String)],
) {
    let extra_headers = extra_headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n{extra_headers}\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn retries_429_retry_after_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first_request, _) = listener.accept().await.unwrap();
        let (first_line, body) = read_http_request(&mut first_request).await;
        assert!(first_line.starts_with("POST /responses HTTP/1.1"));
        assert!(body.contains("\"model\":\"gpt-5-mini\""));
        write_http_response(
            &mut first_request,
            "429 Too Many Requests",
            "text/plain",
            "retry later",
            &[("retry-after".into(), "1".into())],
        )
        .await;

        let (mut second_request, _) = listener.accept().await.unwrap();
        let (second_line, _) = read_http_request(&mut second_request).await;
        assert!(second_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut second_request,
            "200 OK",
            "text/event-stream",
            completed_sse(),
            &[],
        )
        .await;
    });

    let started = Instant::now();
    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert!(started.elapsed() >= Duration::from_millis(900));
    server.await.unwrap();
}

#[tokio::test]
async fn retries_429_retry_after_http_date() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let retry_after = fmt_http_date(SystemTime::now() + Duration::from_secs(2));
    let server = tokio::spawn(async move {
        let (mut first_request, _) = listener.accept().await.unwrap();
        let (first_line, _) = read_http_request(&mut first_request).await;
        assert!(first_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut first_request,
            "429 Too Many Requests",
            "text/plain",
            "retry later",
            &[("retry-after".into(), retry_after)],
        )
        .await;

        let (mut second_request, _) = listener.accept().await.unwrap();
        let (second_line, _) = read_http_request(&mut second_request).await;
        assert!(second_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut second_request,
            "200 OK",
            "text/event-stream",
            completed_sse(),
            &[],
        )
        .await;
    });

    let started = Instant::now();
    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert!(started.elapsed() >= Duration::from_millis(900));
    server.await.unwrap();
}

#[tokio::test]
async fn retries_5xx_with_backoff() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first_request, _) = listener.accept().await.unwrap();
        let (first_line, _) = read_http_request(&mut first_request).await;
        assert!(first_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut first_request,
            "503 Service Unavailable",
            "text/plain",
            "temporary outage",
            &[],
        )
        .await;

        let (mut second_request, _) = listener.accept().await.unwrap();
        let (second_line, _) = read_http_request(&mut second_request).await;
        assert!(second_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut second_request,
            "200 OK",
            "text/event-stream",
            completed_sse(),
            &[],
        )
        .await;
    });

    let started = Instant::now();
    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert!(started.elapsed() >= Duration::from_millis(300));
    server.await.unwrap();
}

#[tokio::test]
async fn does_not_retry_non_429_4xx() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first_request, _) = listener.accept().await.unwrap();
        let (first_line, _) = read_http_request(&mut first_request).await;
        assert!(first_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut first_request,
            "400 Bad Request",
            "text/plain",
            "bad request",
            &[],
        )
        .await;

        assert!(
            timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_err()
        );
    });

    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.stop_reason, StopReason::Error);
    assert!(
        response
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("400 Bad Request: bad request")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn retries_network_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (first_request, _) = listener.accept().await.unwrap();
        drop(first_request);

        let (mut second_request, _) = listener.accept().await.unwrap();
        let (second_line, _) = read_http_request(&mut second_request).await;
        assert!(second_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut second_request,
            "200 OK",
            "text/event-stream",
            completed_sse(),
            &[],
        )
        .await;
    });

    let started = Instant::now();
    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert!(started.elapsed() >= Duration::from_millis(300));
    server.await.unwrap();
}

#[tokio::test]
async fn surfaces_last_error_after_max_attempts() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for body in ["fail-1", "fail-2", "fail-3"] {
            let (mut request, _) = listener.accept().await.unwrap();
            let (request_line, _) = read_http_request(&mut request).await;
            assert!(request_line.starts_with("POST /responses HTTP/1.1"));
            write_http_response(
                &mut request,
                "503 Service Unavailable",
                "text/plain",
                body,
                &[],
            )
            .await;
        }
    });

    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.stop_reason, StopReason::Error);
    assert!(
        response
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("503 Service Unavailable: fail-3")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn aborts_during_retry_wait_without_retrying_again() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first_request, _) = listener.accept().await.unwrap();
        let (first_line, _) = read_http_request(&mut first_request).await;
        assert!(first_line.starts_with("POST /responses HTTP/1.1"));
        write_http_response(
            &mut first_request,
            "429 Too Many Requests",
            "text/plain",
            "retry later",
            &[("retry-after".into(), "1".into())],
        )
        .await;

        assert!(
            timeout(Duration::from_millis(400), listener.accept())
                .await
                .is_err()
        );
    });

    let (abort_tx, abort_rx) = watch::channel(false);
    tokio::spawn(async move {
        sleep(Duration::from_millis(100)).await;
        let _ = abort_tx.send(true);
    });

    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some("test-key".into()),
            signal: Some(abort_rx),
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
    server.await.unwrap();
}
