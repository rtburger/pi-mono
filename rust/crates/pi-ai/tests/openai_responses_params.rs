use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, OpenAiResponsesParamsOptions, ResponsesInputItem,
    build_openai_responses_request_params,
};
use pi_events::{Context, Message, Model, UserContent};
use serde_json::Value;
use std::{fs, path::PathBuf};

fn model(provider: &str, id: &str, reasoning: bool) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "openai-responses".into(),
        provider: provider.into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning,
        input: vec!["text".into(), "image".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn simple_context() -> Context {
    Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: "hi".into() }],
            timestamp: 1,
        }],
    }
}

#[test]
fn omits_reasoning_for_copilot_when_not_requested() {
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("openai_responses_params_copilot.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let params = build_openai_responses_request_params(
        &model("github-copilot", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex", "opencode"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    assert_eq!(params.model, expected["model"].as_str().unwrap());
    assert_eq!(params.stream, expected["stream"].as_bool().unwrap());
    assert_eq!(params.store, expected["store"].as_bool().unwrap());
    assert_eq!(
        params.reasoning.is_some(),
        expected["has_reasoning"].as_bool().unwrap()
    );

    let first_role = match params.input.first().unwrap() {
        ResponsesInputItem::Message { role, .. } => role.as_str(),
        _ => panic!("expected first input item to be a message"),
    };
    assert_eq!(first_role, expected["first_role"].as_str().unwrap());
}

#[test]
fn uses_system_role_for_non_reasoning_models() {
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("openai_responses_params_non_reasoning.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let params = build_openai_responses_request_params(
        &model("openai", "gpt-4o-mini", false),
        &simple_context(),
        &["openai", "openai-codex", "opencode"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    let first_role = match params.input.first().unwrap() {
        ResponsesInputItem::Message { role, .. } => role.as_str(),
        _ => panic!("expected first input item to be a message"),
    };
    assert_eq!(first_role, expected["first_role"].as_str().unwrap());
}

#[test]
fn enables_reasoning_and_prompt_cache_for_openai_when_requested() {
    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex", "opencode"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions {
            reasoning_effort: Some("high".into()),
            reasoning_summary: Some("detailed".into()),
            session_id: Some("session-1".into()),
            cache_retention: Some("long".into()),
            ..Default::default()
        },
    );

    let reasoning = params.reasoning.expect("expected reasoning block");
    assert_eq!(reasoning.effort, "high");
    assert_eq!(reasoning.summary, "detailed");
    assert_eq!(params.prompt_cache_key.as_deref(), Some("session-1"));
    assert_eq!(params.prompt_cache_retention.as_deref(), Some("24h"));
    assert_eq!(
        params.include,
        Some(vec!["reasoning.encrypted_content".into()])
    );
}
