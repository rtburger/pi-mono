use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, OpenAiResponsesParamsOptions, OpenAiResponsesReasoning,
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
fn defaults_reasoning_for_openai_reasoning_models_when_not_requested() {
    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions::default(),
        OpenAiResponsesParamsOptions::default(),
    );

    assert_eq!(params.model, "gpt-5-mini");
    assert!(params.stream);
    assert!(!params.store);
    assert_eq!(
        params.reasoning,
        Some(OpenAiResponsesReasoning {
            effort: "none".into(),
            summary: "auto".into(),
        })
    );

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
fn enables_reasoning_and_prompt_cache_for_openai_when_requested() {
    let params = build_openai_responses_request_params(
        &model("openai", "gpt-5-mini", true),
        &simple_context(),
        &["openai", "openai-codex"],
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
