use futures::{SinkExt, StreamExt};
use pi_ai::{StreamOptions, Transport, complete};
use pi_events::{Context, Message, Model, UserContent};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant, timeout},
};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        Message as WebSocketMessage,
        handshake::server::{Request, Response},
    },
};

fn model(base_url: String) -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 272_000,
        max_tokens: 128_000,
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

async fn write_http_response(socket: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

async fn send_completed_websocket_response(
    websocket: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    response_id: &str,
    message_id: &str,
) {
    for message in [
        serde_json::json!({"type":"response.created","response":{"id":response_id}}),
        serde_json::json!({"type":"response.output_item.added","item":{"type":"message","id":message_id,"role":"assistant","status":"in_progress","content":[]}}),
        serde_json::json!({"type":"response.output_text.delta","delta":"Hello"}),
        serde_json::json!({"type":"response.output_item.done","item":{"type":"message","id":message_id,"role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello"}]}}),
        serde_json::json!({"type":"response.completed","response":{"id":response_id,"status":"completed","usage":{"input_tokens":5,"output_tokens":3,"total_tokens":8,"input_tokens_details":{"cached_tokens":0}}}}),
    ] {
        websocket
            .send(WebSocketMessage::Text(message.to_string().into()))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn uses_websocket_transport_for_codex_when_requested() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = accept_hdr_async(stream, |request: &Request, response: Response| {
            assert_eq!(request.uri().path(), "/codex/responses");
            assert_eq!(request.headers().get("authorization").unwrap(), "Bearer aaa.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjX3Rlc3QifX0=.bbb");
            assert_eq!(request.headers().get("chatgpt-account-id").unwrap(), "acc_test");
            assert_eq!(request.headers().get("originator").unwrap(), "pi");
            assert_eq!(request.headers().get("openai-beta").unwrap(), "responses_websockets=2026-02-06");
            assert_eq!(request.headers().get("x-client-request-id").unwrap(), "session-ws");
            assert_eq!(request.headers().get("session_id").unwrap(), "session-ws");
            Ok(response)
        })
        .await
        .unwrap();

        let request = websocket.next().await.unwrap().unwrap();
        let WebSocketMessage::Text(request) = request else {
            panic!("expected websocket text request");
        };
        let payload: serde_json::Value = serde_json::from_str(request.as_ref()).unwrap();
        assert_eq!(
            payload.get("type").and_then(|value| value.as_str()),
            Some("response.create")
        );
        assert_eq!(
            payload
                .get("prompt_cache_key")
                .and_then(|value| value.as_str()),
            Some("session-ws")
        );
        assert_eq!(
            payload
                .get("prompt_cache_retention")
                .and_then(|value| value.as_str()),
            Some("in-memory")
        );

        for message in [
            serde_json::json!({"type":"response.created","response":{"id":"resp_ws"}}),
            serde_json::json!({"type":"response.output_item.added","item":{"type":"message","id":"msg_ws","role":"assistant","status":"in_progress","content":[]}}),
            serde_json::json!({"type":"response.output_text.delta","delta":"Hello"}),
            serde_json::json!({"type":"response.output_item.done","item":{"type":"message","id":"msg_ws","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello"}]}}),
            serde_json::json!({"type":"response.completed","response":{"id":"resp_ws","status":"completed","usage":{"input_tokens":5,"output_tokens":3,"total_tokens":8,"input_tokens_details":{"cached_tokens":0}}}}),
        ] {
            websocket
                .send(WebSocketMessage::Text(message.to_string().into()))
                .await
                .unwrap();
        }

        websocket.close(None).await.unwrap();
    });

    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some(mock_token()),
            session_id: Some("session-ws".into()),
            transport: Some(Transport::WebSocket),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_ws"));
    server.await.unwrap();
}

#[tokio::test]
async fn auto_transport_falls_back_to_sse_when_websocket_connect_fails_before_start() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut websocket_attempt, _) = listener.accept().await.unwrap();
        let (request_line, _) = read_http_request(&mut websocket_attempt).await;
        assert!(request_line.starts_with("GET /codex/responses HTTP/1.1"));
        write_http_response(
            &mut websocket_attempt,
            "404 Not Found",
            "text/plain",
            "not a websocket",
        )
        .await;

        let (mut sse_request, _) = listener.accept().await.unwrap();
        let (request_line, body) = read_http_request(&mut sse_request).await;
        assert!(request_line.starts_with("POST /codex/responses HTTP/1.1"));
        assert!(body.contains("\"model\":\"gpt-5.2-codex\""));
        write_http_response(
            &mut sse_request,
            "200 OK",
            "text/event-stream",
            completed_sse(),
        )
        .await;
    });

    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some(mock_token()),
            transport: Some(Transport::Auto),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    server.await.unwrap();
}

#[tokio::test]
async fn explicit_websocket_transport_does_not_fallback_to_sse() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut websocket_attempt, _) = listener.accept().await.unwrap();
        let (request_line, _) = read_http_request(&mut websocket_attempt).await;
        assert!(request_line.starts_with("GET /codex/responses HTTP/1.1"));
        write_http_response(
            &mut websocket_attempt,
            "404 Not Found",
            "text/plain",
            "not a websocket",
        )
        .await;
    });

    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some(mock_token()),
            transport: Some(Transport::WebSocket),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.stop_reason, pi_events::StopReason::Error);
    assert!(
        response
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("WebSocket connection failed")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn reuses_cached_websocket_for_same_session_id() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = accept_hdr_async(stream, |request: &Request, response: Response| {
            assert_eq!(request.uri().path(), "/codex/responses");
            assert_eq!(
                request.headers().get("x-client-request-id").unwrap(),
                "session-cache"
            );
            assert_eq!(
                request.headers().get("session_id").unwrap(),
                "session-cache"
            );
            Ok(response)
        })
        .await
        .unwrap();

        let first_request = timeout(Duration::from_secs(1), websocket.next())
            .await
            .expect("timed out waiting for first websocket request")
            .unwrap()
            .unwrap();
        assert!(matches!(first_request, WebSocketMessage::Text(_)));
        send_completed_websocket_response(&mut websocket, "resp_ws_1", "msg_ws_1").await;

        let second_request = timeout(Duration::from_secs(1), websocket.next())
            .await
            .expect("timed out waiting for reused websocket request")
            .unwrap()
            .unwrap();
        assert!(matches!(second_request, WebSocketMessage::Text(_)));
        send_completed_websocket_response(&mut websocket, "resp_ws_2", "msg_ws_2").await;

        websocket.close(None).await.unwrap();
    });

    let first = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some(mock_token()),
            session_id: Some("session-cache".into()),
            transport: Some(Transport::WebSocket),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(first.response_id.as_deref(), Some("resp_ws_1"));

    let second = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some(mock_token()),
            session_id: Some("session-cache".into()),
            transport: Some(Transport::WebSocket),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(second.response_id.as_deref(), Some("resp_ws_2"));

    server.await.unwrap();
}

#[tokio::test]
async fn retries_retryable_http_failures_before_succeeding() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first_request, _) = listener.accept().await.unwrap();
        let (first_line, _) = read_http_request(&mut first_request).await;
        assert!(first_line.starts_with("POST /codex/responses HTTP/1.1"));
        write_http_response(
            &mut first_request,
            "429 Too Many Requests",
            "text/plain",
            "rate limit",
        )
        .await;

        let (mut second_request, _) = listener.accept().await.unwrap();
        let (second_line, _) = read_http_request(&mut second_request).await;
        assert!(second_line.starts_with("POST /codex/responses HTTP/1.1"));
        write_http_response(
            &mut second_request,
            "200 OK",
            "text/event-stream",
            completed_sse(),
        )
        .await;
    });

    let started = Instant::now();
    let response = complete(
        model(format!("http://{address}")),
        context(),
        StreamOptions {
            api_key: Some(mock_token()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
    assert!(started.elapsed() >= Duration::from_millis(900));
    server.await.unwrap();
}
