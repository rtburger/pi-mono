use pi_ai::openai_codex_responses::build_openai_codex_responses_request_params;
use pi_ai::{PayloadHook, StreamOptions, complete};
use pi_events::{
    AssistantContent, Context, Message, Model, ModelCost, StopReason, Usage, UserContent,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

fn responses_model(api: &str) -> Model {
    Model {
        id: "opencode-model".into(),
        name: "OpenCode Model".into(),
        api: api.into(),
        provider: "opencode".into(),
        base_url: "http://127.0.0.1:9".into(),
        reasoning: true,
        input: vec!["text".into()],
        cost: ModelCost::default(),
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    }
}

fn tool_turn_context(api: &str) -> Context {
    Context {
        system_prompt: None,
        messages: vec![
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: "call_opencode|fc_opencode".into(),
                    name: "read".into(),
                    arguments: BTreeMap::from([(String::from("path"), json!("README.md"))]),
                    thought_signature: None,
                }],
                api: api.into(),
                provider: "opencode".into(),
                model: "opencode-model".into(),
                response_id: None,
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 1,
            },
            Message::ToolResult {
                tool_call_id: "call_opencode|fc_opencode".into(),
                tool_name: "read".into(),
                content: vec![UserContent::Text {
                    text: "file contents".into(),
                }],
                details: None,
                is_error: false,
                timestamp: 2,
            },
        ],
        tools: vec![],
    }
}

fn find_function_call(input: &[Value]) -> &Value {
    input
        .iter()
        .find(|item| item.get("type") == Some(&json!("function_call")))
        .expect("expected function_call input item")
}

#[tokio::test]
async fn openai_responses_provider_treats_opencode_tool_ids_as_openai_compatible() {
    let captured = Arc::new(Mutex::new(None::<Value>));
    let hook_capture = captured.clone();
    let hook = PayloadHook::new(move |payload, _model| {
        let hook_capture = hook_capture.clone();
        async move {
            *hook_capture.lock().unwrap() = Some(payload.clone());
            Ok(None)
        }
    });

    let _ = complete(
        responses_model("openai-responses"),
        tool_turn_context("openai-responses"),
        StreamOptions {
            api_key: Some("test-key".into()),
            on_payload: Some(hook),
            ..Default::default()
        },
    )
    .await;

    let payload = captured
        .lock()
        .unwrap()
        .clone()
        .expect("expected captured payload");
    let input = payload
        .get("input")
        .and_then(Value::as_array)
        .expect("expected input array");
    let function_call = find_function_call(input);

    assert_eq!(function_call.get("call_id"), Some(&json!("call_opencode")));
    assert_eq!(function_call.get("id"), Some(&json!("fc_opencode")));
}

#[test]
fn openai_codex_request_params_treat_opencode_tool_ids_as_openai_compatible() {
    let params = build_openai_codex_responses_request_params(
        &responses_model("openai-codex-responses"),
        &tool_turn_context("openai-codex-responses"),
        &Default::default(),
    );

    let serialized = serde_json::to_value(&params).expect("serialize params");
    let input = serialized
        .get("input")
        .and_then(Value::as_array)
        .expect("expected input array");
    let function_call = find_function_call(input);

    assert_eq!(function_call.get("call_id"), Some(&json!("call_opencode")));
    assert_eq!(function_call.get("id"), Some(&json!("fc_opencode")));
}
