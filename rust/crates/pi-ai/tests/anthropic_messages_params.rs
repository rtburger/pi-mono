use pi_ai::CacheRetention;
use pi_ai::anthropic_messages::{
    AnthropicContentBlock, AnthropicMessageContent, AnthropicOptions, AnthropicThinkingConfig,
    build_anthropic_request_params, convert_anthropic_messages,
};
use pi_events::{
    AssistantContent, Context, Message, Model, StopReason, ToolDefinition, Usage, UserContent,
};
use serde_json::json;

fn model(id: &str, base_url: &str) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url: base_url.into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 200_000,
        max_tokens: 8_192,
    }
}

fn tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: format!("{name} tool"),
        parameters: json!({
            "type": "object",
            "properties": {
                "task": { "type": "string" }
            },
            "required": ["task"]
        }),
    }
}

fn simple_context() -> Context {
    Context {
        system_prompt: Some("You are concise.".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text {
                text: "Hello".into(),
            }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

#[test]
fn build_params_disables_thinking_when_reasoning_is_off() {
    let params = build_anthropic_request_params(
        &model("claude-sonnet-4-5", "https://api.anthropic.com/v1"),
        &simple_context(),
        false,
        &AnthropicOptions {
            cache_retention: Some(CacheRetention::Short),
            thinking_enabled: Some(false),
            ..AnthropicOptions::default()
        },
    );

    assert_eq!(params.thinking, Some(AnthropicThinkingConfig::Disabled));
    assert_eq!(params.output_config, None);
}

#[test]
fn build_params_adds_long_cache_ttl_only_for_direct_anthropic_base_url() {
    let direct = build_anthropic_request_params(
        &model("claude-sonnet-4-5", "https://api.anthropic.com/v1"),
        &simple_context(),
        false,
        &AnthropicOptions {
            cache_retention: Some(CacheRetention::Long),
            ..AnthropicOptions::default()
        },
    );
    let proxied = build_anthropic_request_params(
        &model("claude-sonnet-4-5", "https://proxy.example.test/v1"),
        &simple_context(),
        false,
        &AnthropicOptions {
            cache_retention: Some(CacheRetention::Long),
            ..AnthropicOptions::default()
        },
    );

    assert_eq!(
        direct.system.as_ref().unwrap()[0]
            .cache_control
            .as_ref()
            .unwrap()
            .ttl
            .as_deref(),
        Some("1h")
    );
    match &direct.messages[0].content {
        AnthropicMessageContent::Blocks(blocks) => match &blocks[0] {
            AnthropicContentBlock::Text { cache_control, .. } => {
                assert_eq!(cache_control.as_ref().unwrap().ttl.as_deref(), Some("1h"));
            }
            other => panic!("expected cached text block, got {other:?}"),
        },
        other => panic!("expected block content, got {other:?}"),
    }

    assert_eq!(
        proxied.system.as_ref().unwrap()[0]
            .cache_control
            .as_ref()
            .unwrap()
            .ttl,
        None
    );
}

#[test]
fn oauth_tool_name_normalization_matches_claude_code_round_trip() {
    let oauth_model = model("claude-sonnet-4-20250514", "https://api.anthropic.com/v1");
    let tools = vec![tool("todowrite"), tool("read"), tool("find"), tool("my_custom_tool")];
    let params = build_anthropic_request_params(
        &oauth_model,
        &Context {
            system_prompt: Some("Use todowrite when asked.".into()),
            messages: vec![Message::User {
                content: vec![UserContent::Text {
                    text: "Add a todo.".into(),
                }],
                timestamp: 1,
            }],
            tools: tools.clone(),
        },
        true,
        &AnthropicOptions::default(),
    );

    let tool_names = params
        .tools
        .as_ref()
        .unwrap()
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"TodoWrite"));
    assert!(tool_names.contains(&"Read"));
    assert!(tool_names.contains(&"find"));
    assert!(tool_names.contains(&"my_custom_tool"));

    let system = params.system.as_ref().unwrap();
    assert_eq!(
        system[0].text,
        "You are Claude Code, Anthropic's official CLI for Claude."
    );
}

#[test]
fn groups_consecutive_tool_results_into_single_user_message() {
    let anthropic_model = model("claude-sonnet-4-5", "https://api.anthropic.com/v1");
    let messages = vec![
        Message::Assistant {
            content: vec![AssistantContent::ToolCall {
                id: "tool_1".into(),
                name: "read".into(),
                arguments: [("path".into(), json!("README.md"))].into_iter().collect(),
                thought_signature: None,
            }],
            api: "anthropic-messages".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            response_id: None,
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 2,
        },
        Message::ToolResult {
            tool_call_id: "tool_1".into(),
            tool_name: "read".into(),
            content: vec![UserContent::Text {
                text: "file text".into(),
            }],
            details: None,
            is_error: false,
            timestamp: 3,
        },
        Message::ToolResult {
            tool_call_id: "tool_2".into(),
            tool_name: "grep".into(),
            content: vec![UserContent::Text {
                text: "grep text".into(),
            }],
            details: None,
            is_error: false,
            timestamp: 4,
        },
    ];

    let converted = convert_anthropic_messages(&messages, &anthropic_model, false, None);

    assert_eq!(converted.len(), 2);
    assert_eq!(converted[0].role, "assistant");
    assert_eq!(converted[1].role, "user");
    match &converted[1].content {
        AnthropicMessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            assert!(matches!(
                blocks[0],
                AnthropicContentBlock::ToolResult { .. }
            ));
            assert!(matches!(
                blocks[1],
                AnthropicContentBlock::ToolResult { .. }
            ));
        }
        other => panic!("expected tool_result blocks, got {other:?}"),
    }
}

#[test]
fn normalizes_foreign_tool_call_ids_for_anthropic_messages() {
    let anthropic_model = model("claude-sonnet-4-5", "https://api.anthropic.com/v1");
    let raw_id = "call_4VnzVawQXPB9MgYib7CiQFEY|I9b95oN1wD/cHXKTw3PpRkL6KkCtzTJhUxMouMWYwHeTo2j3htzfSk7YPx2vifiIM4g3A8XXyOj8q4Bt6SLUG7gqY1E3ELkrkVQNHglRfUmWj84lqxJY+Puieb3VKyX0FB+83TUzn91cDMF/4gzt990IzqVrc+nIb9RRscRD070Du16q1glydVjWR0SBJsE6TbY/esOjFpqplogQqrajm1eI++f3eLi73R6q7hVusY0QbeFySVxABCjhN0lXB04caBe1rzHjYzul6MAXj7uq+0r17VLq+yrtyYhN12wkmFqHeqTyEei6EFPbMy24Nc+IbJlkP0OCg02W+gOnyBFcbi2ctvJFSOhSjt1CqBdqCnnhwUqXjbWiT0wh3DmLScRgTHmGkaI+oAcQQjfic65nxj+TnEkReA==";

    let converted = convert_anthropic_messages(
        &[
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: raw_id.into(),
                    name: "edit".into(),
                    arguments: [("path".into(), json!("src/main.rs"))]
                        .into_iter()
                        .collect(),
                    thought_signature: None,
                }],
                api: "openai-responses".into(),
                provider: "openai-codex".into(),
                model: "gpt-5.3-codex".into(),
                response_id: None,
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 1,
            },
            Message::ToolResult {
                tool_call_id: raw_id.into(),
                tool_name: "edit".into(),
                content: vec![UserContent::Text {
                    text: "done".into(),
                }],
                details: None,
                is_error: false,
                timestamp: 2,
            },
        ],
        &anthropic_model,
        false,
        None,
    );

    match &converted[0].content {
        AnthropicMessageContent::Blocks(blocks) => match &blocks[0] {
            AnthropicContentBlock::ToolUse { id, .. } => {
                assert!(id.len() <= 64);
                assert!(id.chars().all(|character| character.is_ascii_alphanumeric()
                    || character == '_'
                    || character == '-'));
            }
            other => panic!("expected tool_use block, got {other:?}"),
        },
        other => panic!("expected assistant blocks, got {other:?}"),
    }

    match &converted[1].content {
        AnthropicMessageContent::Blocks(blocks) => match &blocks[0] {
            AnthropicContentBlock::ToolResult { tool_use_id, .. } => {
                assert!(tool_use_id.len() <= 64);
                assert!(
                    tool_use_id
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric()
                            || character == '_'
                            || character == '-')
                );
            }
            other => panic!("expected tool_result block, got {other:?}"),
        },
        other => panic!("expected tool_result blocks, got {other:?}"),
    }
}
