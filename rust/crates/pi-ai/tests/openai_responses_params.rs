use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, OpenAiResponsesParamsOptions, OpenAiResponsesServiceTier,
    ResponsesInputItem, ResponsesToolDefinition, build_openai_responses_request_params,
};
use pi_events::{Context, Message, Model, ToolDefinition, UserContent};
use serde_json::{Value, json};
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

fn simple_context() -> Context {
    Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: "hi".into() }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

#[test]
fn omits_reasoning_summary_and_include_when_reasoning_not_requested() {
    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );
    let serialized = serde_json::to_value(&params).unwrap();

    assert_eq!(params.model, "gpt-5-mini");
    assert!(params.stream);
    assert!(!params.store);
    assert_eq!(serialized["reasoning"], json!({ "effort": "none" }));
    assert!(serialized.get("include").is_none());

    let first_role = match params.input.first().unwrap() {
        ResponsesInputItem::Message { role, .. } => role.as_str(),
        _ => panic!("expected first input item to be a message"),
    };
    assert_eq!(first_role, "developer");
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
        &["openai", "openai-codex"],
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
fn includes_tools_in_request_params_when_present() {
    let mut context = simple_context();
    context.tools = vec![ToolDefinition {
        name: "calculate".into(),
        description: "Evaluate a math expression".into(),
        parameters: json!({
            "type": "object",
            "properties": { "expression": { "type": "string" } },
            "required": ["expression"]
        }),
    }];

    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &context,
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    match params.tools.as_ref().and_then(|tools| tools.first()) {
        Some(ResponsesToolDefinition::Function {
            name,
            description,
            parameters,
            strict,
        }) => {
            assert_eq!(name, "calculate");
            assert_eq!(description, "Evaluate a math expression");
            assert_eq!(parameters["type"], "object");
            assert!(!strict);
        }
        other => panic!("expected function tool definition, got {other:?}"),
    }
}

#[test]
fn includes_service_tier_when_requested() {
    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions {
            service_tier: Some(OpenAiResponsesServiceTier::Flex),
            ..Default::default()
        },
    );

    assert_eq!(params.service_tier, Some(OpenAiResponsesServiceTier::Flex));
    assert_eq!(
        serde_json::to_value(&params).unwrap()["service_tier"],
        "flex"
    );
}

#[test]
fn includes_reasoning_summary_and_prompt_cache_when_reasoning_effort_is_requested() {
    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions {
            reasoning_effort: Some("high".into()),
            session_id: Some("session-1".into()),
            cache_retention: Some("long".into()),
            ..Default::default()
        },
    );
    let serialized = serde_json::to_value(&params).unwrap();

    assert_eq!(
        serialized["reasoning"],
        json!({
            "effort": "high",
            "summary": "auto"
        })
    );
    assert_eq!(
        serialized["include"],
        json!(["reasoning.encrypted_content"])
    );
    assert_eq!(params.prompt_cache_key.as_deref(), Some("session-1"));
    assert_eq!(params.prompt_cache_retention.as_deref(), Some("24h"));
}

#[test]
fn defaults_reasoning_effort_when_reasoning_summary_is_requested() {
    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions {
            reasoning_summary: Some("detailed".into()),
            ..Default::default()
        },
    );
    let serialized = serde_json::to_value(&params).unwrap();

    assert_eq!(
        serialized["reasoning"],
        json!({
            "effort": "medium",
            "summary": "detailed"
        })
    );
    assert_eq!(
        serialized["include"],
        json!(["reasoning.encrypted_content"])
    );
}
