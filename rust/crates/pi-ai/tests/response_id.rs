use futures::StreamExt;
use pi_ai::anthropic_messages::{AnthropicStreamEnvelope, stream_anthropic_sse_events};
use pi_ai::openai_codex_responses::stream_openai_codex_sse_text;
use pi_ai::openai_completions::stream_openai_completions_sse_text;
use pi_ai::openai_responses::{OpenAiResponsesStreamEnvelope, stream_openai_responses_sse_events};
use pi_events::{AssistantEvent, Model};
use serde_json::json;

fn anthropic_model() -> Model {
    Model {
        id: "claude-sonnet-4-5".into(),
        name: "claude-sonnet-4-5".into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url: "https://api.anthropic.com/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
        context_window: 200_000,
        max_tokens: 8_192,
    }
}

fn openai_responses_model() -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: true,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn openai_completions_model() -> Model {
    Model {
        id: "gpt-4o-mini".into(),
        name: "gpt-4o-mini".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: false,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn openai_codex_model() -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url: "https://chatgpt.com/backend-api".into(),
        reasoning: true,
        input: vec!["text".into()],
        context_window: 272_000,
        max_tokens: 128_000,
    }
}

#[tokio::test]
async fn anthropic_done_event_exposes_response_id() {
    let events = vec![
        AnthropicStreamEnvelope {
            event_type: "message_start".into(),
            data: serde_json::from_value(json!({
                "message": {
                    "id": "msg_response_id_1",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "cache_creation_input_tokens": 0
                    }
                }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_start".into(),
            data: serde_json::from_value(json!({
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_delta".into(),
            data: serde_json::from_value(json!({
                "index": 0,
                "delta": { "type": "text_delta", "text": "response id test" }
            }))
            .unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "content_block_stop".into(),
            data: serde_json::from_value(json!({ "index": 0 })).unwrap(),
        },
        AnthropicStreamEnvelope {
            event_type: "message_delta".into(),
            data: serde_json::from_value(json!({
                "delta": { "stop_reason": "end_turn" },
                "usage": { "output_tokens": 3 }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_anthropic_sse_events(anthropic_model(), events, false, vec![])
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("msg_response_id_1"));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_responses_done_event_exposes_response_id() {
    let events = vec![
        OpenAiResponsesStreamEnvelope {
            event_type: "response.created".into(),
            data: serde_json::from_value(json!({
                "response": { "id": "resp_response_id_1" }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.added".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "status": "in_progress",
                    "content": []
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_text.delta".into(),
            data: serde_json::from_value(json!({ "delta": "response id test" })).unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.output_item.done".into(),
            data: serde_json::from_value(json!({
                "item": {
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{ "type": "output_text", "text": "response id test" }]
                }
            }))
            .unwrap(),
        },
        OpenAiResponsesStreamEnvelope {
            event_type: "response.completed".into(),
            data: serde_json::from_value(json!({
                "response": {
                    "id": "resp_response_id_1",
                    "status": "completed",
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 3,
                        "total_tokens": 8,
                        "input_tokens_details": { "cached_tokens": 0 }
                    }
                }
            }))
            .unwrap(),
        },
    ];

    let collected = stream_openai_responses_sse_events(openai_responses_model(), events)
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(message.response_id.as_deref(), Some("resp_response_id_1"));
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_completions_done_event_exposes_response_id() {
    let payload = concat!(
        "data: {\"id\":\"chatcmpl-response-id-1\",\"choices\":[{\"delta\":{\"content\":\"response id test\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-response-id-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let collected = stream_openai_completions_sse_text(openai_completions_model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(
                message.response_id.as_deref(),
                Some("chatcmpl-response-id-1")
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_codex_done_event_exposes_response_id() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_codex_response_id_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"response id test\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"response id test\"}]}}\n\n",
        "data: {\"type\":\"response.done\",\"response\":{\"id\":\"resp_codex_response_id_1\"}}\n\n"
    );

    let collected = stream_openai_codex_sse_text(openai_codex_model(), payload)
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    match collected.last().unwrap().as_ref().unwrap() {
        AssistantEvent::Done { message, .. } => {
            assert_eq!(
                message.response_id.as_deref(),
                Some("resp_codex_response_id_1")
            );
        }
        other => panic!("expected done event, got {other:?}"),
    }
}
