use futures::StreamExt;
use pi_agent::{ProxyStreamConfig, stream_proxy};
use pi_ai::StreamOptions;
use pi_events::{
    AssistantContent, AssistantEvent, Context, Message, Model, StopReason, ToolDefinition,
    UsageCost, UserContent,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};
use tokio::sync::watch;

fn model() -> Model {
    Model {
        id: String::from("mock"),
        name: String::from("Mock"),
        api: String::from("openai-responses"),
        provider: String::from("openai"),
        base_url: String::from("https://api.example.com"),
        reasoning: true,
        input: vec![String::from("text"), String::from("image")],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn context() -> Context {
    Context {
        system_prompt: Some(String::from("sys")),
        messages: vec![
            Message::User {
                content: vec![
                    UserContent::Text {
                        text: String::from("hello"),
                    },
                    UserContent::Image {
                        data: String::from("aGVsbG8="),
                        mime_type: String::from("image/png"),
                    },
                ],
                timestamp: 1,
            },
            Message::Assistant {
                content: vec![AssistantContent::Text {
                    text: String::from("previous"),
                    text_signature: Some(String::from("sig_1")),
                }],
                api: String::from("openai-responses"),
                provider: String::from("openai"),
                model: String::from("mock"),
                response_id: Some(String::from("resp_1")),
                usage: Default::default(),
                stop_reason: StopReason::Stop,
                error_message: None,
                timestamp: 2,
            },
            Message::ToolResult {
                tool_call_id: String::from("tool-0"),
                tool_name: String::from("echo"),
                content: vec![UserContent::Text {
                    text: String::from("tool output"),
                }],
                is_error: false,
                timestamp: 3,
            },
        ],
        tools: vec![ToolDefinition {
            name: String::from("echo"),
            description: String::from("Echo tool"),
            parameters: json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            }),
        }],
    }
}

fn start_server(response: String, captured_request_body: Arc<Mutex<Option<Value>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();

    thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        let mut header_end = None;

        while header_end.is_none() {
            let read = socket.read(&mut buffer).unwrap();
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

        let mut body = request[header_end..].to_vec();
        while body.len() < content_length {
            let read = socket.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..read]);
        }

        *captured_request_body.lock().unwrap() =
            Some(serde_json::from_slice::<Value>(&body).unwrap());

        socket.write_all(response.as_bytes()).unwrap();
    });

    format!("http://{address}")
}

#[tokio::test]
async fn proxy_stream_posts_camel_case_request_and_reconstructs_events() {
    let request_body = Arc::new(Mutex::new(None));
    let sse = concat!(
        "data: {\"type\":\"start\"}\n\n",
        "data: {\"type\":\"text_start\",\"contentIndex\":0}\n\n",
        "data: {\"type\":\"text_delta\",\"contentIndex\":0,\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"text_end\",\"contentIndex\":0,\"contentSignature\":\"sig_final\"}\n\n",
        "data: {\"type\":\"toolcall_start\",\"contentIndex\":1,\"id\":\"tool-1\",\"toolName\":\"echo\"}\n\n",
        "data: {\"type\":\"toolcall_delta\",\"contentIndex\":1,\"delta\":\"{\\\"value\\\":\\\"hi\"}\n\n",
        "data: {\"type\":\"toolcall_delta\",\"contentIndex\":1,\"delta\":\"\\\"}\"}\n\n",
        "data: {\"type\":\"toolcall_end\",\"contentIndex\":1}\n\n",
        "data: {\"type\":\"done\",\"reason\":\"toolUse\",\"usage\":{\"input\":1,\"output\":2,\"cacheRead\":3,\"cacheWrite\":4,\"totalTokens\":10,\"cost\":{\"input\":0.1,\"output\":0.2,\"cacheRead\":0.3,\"cacheWrite\":0.4,\"total\":1.0}}}\n\n"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        sse.len(),
        sse,
    );
    let server = start_server(response, request_body.clone());

    let stream = stream_proxy(
        model(),
        context(),
        StreamOptions {
            temperature: Some(0.2),
            max_tokens: Some(123),
            reasoning_effort: Some(String::from("high")),
            ..StreamOptions::default()
        },
        ProxyStreamConfig::new("token-123", server),
    );

    let events = stream.collect::<Vec<_>>().await;
    let events = events.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    let event_kinds = events
        .iter()
        .map(|event| match event {
            AssistantEvent::Start { .. } => "start",
            AssistantEvent::TextStart { .. } => "text_start",
            AssistantEvent::TextDelta { .. } => "text_delta",
            AssistantEvent::TextEnd { .. } => "text_end",
            AssistantEvent::ThinkingStart { .. } => "thinking_start",
            AssistantEvent::ThinkingDelta { .. } => "thinking_delta",
            AssistantEvent::ThinkingEnd { .. } => "thinking_end",
            AssistantEvent::ToolCallStart { .. } => "toolcall_start",
            AssistantEvent::ToolCallDelta { .. } => "toolcall_delta",
            AssistantEvent::ToolCallEnd { .. } => "toolcall_end",
            AssistantEvent::Done { .. } => "done",
            AssistantEvent::Error { .. } => "error",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        event_kinds,
        vec![
            "start",
            "text_start",
            "text_delta",
            "text_end",
            "toolcall_start",
            "toolcall_delta",
            "toolcall_delta",
            "toolcall_end",
            "done",
        ]
    );

    let body = request_body.lock().unwrap().clone().unwrap();
    assert_eq!(body["model"]["baseUrl"], json!("https://api.example.com"));
    assert!(body["model"].get("base_url").is_none());
    assert_eq!(body["context"]["systemPrompt"], json!("sys"));
    assert!(body["context"].get("system_prompt").is_none());
    assert_eq!(
        body["context"]["messages"][1]["responseId"],
        json!("resp_1")
    );
    assert_eq!(
        body["context"]["messages"][2]["toolCallId"],
        json!("tool-0")
    );
    assert_eq!(
        body["context"]["messages"][0]["content"][1]["mimeType"],
        json!("image/png")
    );
    assert_eq!(body["options"]["maxTokens"], json!(123));
    assert_eq!(body["options"]["reasoning"], json!("high"));
    assert!(body["options"].get("max_tokens").is_none());

    match events.last().unwrap() {
        AssistantEvent::Done { reason, message } => {
            assert_eq!(reason, &StopReason::ToolUse);
            assert_eq!(message.stop_reason, StopReason::ToolUse);
            assert_eq!(message.usage.input, 1);
            assert_eq!(message.usage.output, 2);
            assert_eq!(message.usage.cache_read, 3);
            assert_eq!(message.usage.cache_write, 4);
            assert_eq!(message.usage.total_tokens, 10);
            assert_eq!(
                message.usage.cost,
                UsageCost {
                    input: 0.1,
                    output: 0.2,
                    cache_read: 0.3,
                    cache_write: 0.4,
                    total: 1.0,
                }
            );
            assert_eq!(
                message.content,
                vec![
                    AssistantContent::Text {
                        text: String::from("Hello"),
                        text_signature: Some(String::from("sig_final")),
                    },
                    AssistantContent::ToolCall {
                        id: String::from("tool-1"),
                        name: String::from("echo"),
                        arguments: std::collections::BTreeMap::from_iter([(
                            String::from("value"),
                            json!("hi")
                        )]),
                        thought_signature: None,
                    },
                ]
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn proxy_stream_surfaces_non_ok_json_errors() {
    let request_body = Arc::new(Mutex::new(None));
    let body = "{\"error\":\"denied\"}";
    let response = format!(
        "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body,
    );
    let server = start_server(response, request_body);

    let events = stream_proxy(
        model(),
        context(),
        StreamOptions::default(),
        ProxyStreamConfig::new("token-123", server),
    )
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        AssistantEvent::Error { reason, error } => {
            assert_eq!(reason, &StopReason::Error);
            assert_eq!(error.stop_reason, StopReason::Error);
            assert_eq!(error.error_message.as_deref(), Some("Proxy error: denied"));
        }
        other => panic!("expected error event, got {other:?}"),
    }
}

#[tokio::test]
async fn proxy_stream_respects_pre_aborted_signal() {
    let (_tx, rx) = watch::channel(true);
    let events = stream_proxy(
        model(),
        context(),
        StreamOptions {
            signal: Some(rx),
            ..StreamOptions::default()
        },
        ProxyStreamConfig::new("token-123", "http://127.0.0.1:1"),
    )
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();

    assert_eq!(events.len(), 1);
    match &events[0] {
        AssistantEvent::Error { reason, error } => {
            assert_eq!(reason, &StopReason::Aborted);
            assert_eq!(error.stop_reason, StopReason::Aborted);
            assert_eq!(
                error.error_message.as_deref(),
                Some("Request aborted by user")
            );
        }
        other => panic!("expected aborted error event, got {other:?}"),
    }
}
