use pi_coding_agent_core::{MemoryAuthStorage, ModelRegistry, RequestAuth};
use pi_events::{
    Model, ModelCost, ModelRouting, OpenAiCompletionsCompatConfig, OpenAiCompletionsMaxTokensField,
    OpenAiThinkingFormat,
};
use serde_json::json;
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

fn mock_model(provider: &str, id: &str, name: &str) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        api: if provider == "anthropic" {
            "anthropic-messages".into()
        } else {
            "openai-completions".into()
        },
        provider: provider.into(),
        base_url: format!("https://{provider}.example.com/v1"),
        reasoning: false,
        input: vec!["text".into()],
        cost: ModelCost {
            input: 1.0,
            output: 2.0,
            cache_read: 0.5,
            cache_write: 0.25,
        },
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    }
}

fn built_in_models() -> Vec<Model> {
    vec![
        mock_model("anthropic", "claude-sonnet-4-5", "Claude Sonnet 4.5"),
        mock_model("openai", "gpt-4o", "GPT-4o"),
        mock_model("openai", "gpt-5.4", "GPT-5.4"),
    ]
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = env::temp_dir().join(format!("pi-coding-agent-core-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_models_json(path: &Path, value: serde_json::Value) {
    fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

#[test]
fn base_url_override_keeps_built_in_models() {
    let temp_dir = unique_temp_dir("base-url-override");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://proxy.example.com/v1"
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );

    let openai_models: Vec<_> = registry
        .get_all()
        .iter()
        .filter(|model| model.provider == "openai")
        .collect();

    assert_eq!(openai_models.len(), 2);
    assert!(
        openai_models
            .iter()
            .all(|model| model.base_url == "https://proxy.example.com/v1")
    );
}

#[test]
fn custom_models_merge_with_built_ins() {
    let temp_dir = unique_temp_dir("custom-models-merge");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://proxy.example.com/v1",
                    "apiKey": "OPENAI_API_KEY",
                    "api": "openai-completions",
                    "models": [
                        {
                            "id": "custom-openai-model",
                            "name": "Custom OpenAI Model"
                        }
                    ]
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );

    let openai_models: Vec<_> = registry
        .get_all()
        .iter()
        .filter(|model| model.provider == "openai")
        .collect();

    assert_eq!(openai_models.len(), 3);
    assert!(
        openai_models
            .iter()
            .any(|model| model.id == "custom-openai-model")
    );
    assert!(
        openai_models
            .iter()
            .all(|model| model.base_url == "https://proxy.example.com/v1")
    );
}

#[test]
fn custom_model_replaces_built_in_by_provider_and_id() {
    let temp_dir = unique_temp_dir("custom-model-replace");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://replacement.example.com/v1",
                    "apiKey": "OPENAI_API_KEY",
                    "api": "openai-completions",
                    "models": [
                        {
                            "id": "gpt-4o",
                            "name": "Replacement GPT-4o"
                        }
                    ]
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );

    let matching: Vec<_> = registry
        .get_all()
        .iter()
        .filter(|model| model.provider == "openai" && model.id == "gpt-4o")
        .collect();

    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].name, "Replacement GPT-4o");
    assert_eq!(matching[0].base_url, "https://replacement.example.com/v1");
}

#[test]
fn model_override_applies_to_built_in_model() {
    let temp_dir = unique_temp_dir("model-override");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://proxy.example.com/v1",
                    "modelOverrides": {
                        "gpt-4o": {
                            "name": "Overridden GPT-4o",
                            "maxTokens": 4096
                        }
                    }
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );
    let model = registry.find("openai", "gpt-4o").unwrap();

    assert_eq!(model.name, "Overridden GPT-4o");
    assert_eq!(model.max_tokens, 4096);
    assert_eq!(model.base_url, "https://proxy.example.com/v1");
}

#[test]
fn custom_models_capture_cost_and_compat() {
    let temp_dir = unique_temp_dir("custom-cost-compat");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openrouter": {
                    "baseUrl": "https://openrouter.ai/api/v1",
                    "apiKey": "OPENROUTER_API_KEY",
                    "api": "openai-completions",
                    "compat": {
                        "thinkingFormat": "openrouter",
                        "openRouterRouting": {
                            "order": ["anthropic"]
                        }
                    },
                    "models": [
                        {
                            "id": "anthropic/claude-sonnet-4-5",
                            "name": "Claude Sonnet via OpenRouter",
                            "cost": {
                                "input": 0.5,
                                "output": 1.5,
                                "cacheRead": 0.1,
                                "cacheWrite": 0.2
                            },
                            "compat": {
                                "requiresToolResultName": true
                            }
                        }
                    ]
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );
    let model = registry
        .find("openrouter", "anthropic/claude-sonnet-4-5")
        .unwrap();

    assert_eq!(
        model.cost,
        ModelCost {
            input: 0.5,
            output: 1.5,
            cache_read: 0.1,
            cache_write: 0.2,
        }
    );
    assert_eq!(
        model.compat,
        Some(OpenAiCompletionsCompatConfig {
            thinking_format: Some(OpenAiThinkingFormat::OpenRouter),
            open_router_routing: Some(ModelRouting {
                only: None,
                order: Some(vec!["anthropic".into()]),
            }),
            requires_tool_result_name: Some(true),
            ..OpenAiCompletionsCompatConfig::default()
        })
    );
}

#[test]
fn model_overrides_merge_cost_and_provider_compat() {
    let temp_dir = unique_temp_dir("override-cost-compat");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://proxy.example.com/v1",
                    "compat": {
                        "maxTokensField": "max_tokens",
                        "vercelGatewayRouting": {
                            "order": ["openai"]
                        }
                    },
                    "modelOverrides": {
                        "gpt-4o": {
                            "cost": {
                                "output": 9.0
                            },
                            "compat": {
                                "supportsStore": false,
                                "vercelGatewayRouting": {
                                    "only": ["openai"]
                                }
                            }
                        }
                    }
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );
    let model = registry.find("openai", "gpt-4o").unwrap();

    assert_eq!(model.cost.input, 1.0);
    assert_eq!(model.cost.output, 9.0);
    assert_eq!(
        model.compat,
        Some(OpenAiCompletionsCompatConfig {
            max_tokens_field: Some(OpenAiCompletionsMaxTokensField::MaxTokens),
            supports_store: Some(false),
            vercel_gateway_routing: Some(ModelRouting {
                only: Some(vec!["openai".into()]),
                order: Some(vec!["openai".into()]),
            }),
            ..OpenAiCompletionsCompatConfig::default()
        })
    );
}

#[test]
fn refresh_reloads_models_json_from_disk() {
    let temp_dir = unique_temp_dir("refresh");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://first.example.com/v1"
                }
            }
        }),
    );

    let mut registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path.clone()),
    );
    assert_eq!(
        registry.find("openai", "gpt-4o").unwrap().base_url,
        "https://first.example.com/v1"
    );

    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://second.example.com/v1"
                }
            }
        }),
    );
    registry.refresh();

    assert_eq!(
        registry.find("openai", "gpt-4o").unwrap().base_url,
        "https://second.example.com/v1"
    );
}

#[test]
fn invalid_models_json_keeps_built_ins_and_records_error() {
    let temp_dir = unique_temp_dir("invalid-json");
    let models_json_path = temp_dir.join("models.json");
    fs::write(&models_json_path, "{invalid-json").unwrap();

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );

    assert!(
        registry
            .get_error()
            .is_some_and(|error| error.contains("Failed to parse models.json"))
    );
    assert_eq!(registry.get_all().len(), 3);
}

#[test]
fn get_available_does_not_execute_command_backed_api_key_resolution() {
    let temp_dir = unique_temp_dir("get-available");
    let models_json_path = temp_dir.join("models.json");
    let counter_path = temp_dir.join("counter");
    fs::write(&counter_path, "0").unwrap();
    let counter_path = counter_path.display().to_string().replace('\\', "/");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "custom-provider": {
                    "baseUrl": "https://custom.example.com/v1",
                    "apiKey": format!("!sh -c 'count=$(cat \"{counter_path}\"); echo $((count + 1)) > \"{counter_path}\"; echo token'"),
                    "api": "openai-completions",
                    "models": [
                        {
                            "id": "custom-model",
                            "name": "Custom Model"
                        }
                    ]
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );

    let available = registry.get_available();
    let count = fs::read_to_string(temp_dir.join("counter")).unwrap();

    assert!(
        available
            .iter()
            .any(|model| model.provider == "custom-provider")
    );
    assert_eq!(count.trim(), "0");
}

#[test]
fn get_api_key_for_provider_resolves_commands_each_time() {
    let temp_dir = unique_temp_dir("api-key-command");
    let models_json_path = temp_dir.join("models.json");
    let counter_path = temp_dir.join("counter");
    fs::write(&counter_path, "0").unwrap();
    let counter_path = counter_path.display().to_string().replace('\\', "/");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "custom-provider": {
                    "baseUrl": "https://custom.example.com/v1",
                    "apiKey": format!("!sh -c 'count=$(cat \"{counter_path}\"); echo $((count + 1)) > \"{counter_path}\"; cat \"{counter_path}\"'"),
                    "api": "openai-completions",
                    "models": [
                        { "id": "custom-model", "name": "Custom Model" }
                    ]
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );

    let first = registry.get_api_key_for_provider("custom-provider");
    let second = registry.get_api_key_for_provider("custom-provider");

    assert_eq!(first.as_deref(), Some("1"));
    assert_eq!(second.as_deref(), Some("2"));
}

#[test]
fn get_api_key_and_headers_merges_provider_and_model_headers() {
    let temp_dir = unique_temp_dir("request-auth");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://proxy.example.com/v1",
                    "apiKey": "literal-token",
                    "headers": {
                        "X-Provider-Header": "provider-value"
                    },
                    "authHeader": true,
                    "modelOverrides": {
                        "gpt-4o": {
                            "headers": {
                                "X-Model-Header": "model-value"
                            }
                        }
                    }
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );
    let model = registry.find("openai", "gpt-4o").unwrap();

    let auth = registry.get_api_key_and_headers(&model).unwrap();
    assert_eq!(
        auth,
        RequestAuth {
            api_key: Some("literal-token".into()),
            headers: Some(
                [
                    ("Authorization".into(), "Bearer literal-token".into()),
                    ("X-Model-Header".into(), "model-value".into()),
                    ("X-Provider-Header".into(), "provider-value".into()),
                ]
                .into_iter()
                .collect(),
            ),
        }
    );
}

#[test]
fn get_api_key_and_headers_returns_error_for_failed_auth_header_resolution() {
    let temp_dir = unique_temp_dir("request-auth-error");
    let models_json_path = temp_dir.join("models.json");
    write_models_json(
        &models_json_path,
        json!({
            "providers": {
                "openai": {
                    "baseUrl": "https://proxy.example.com/v1",
                    "apiKey": "!exit 1",
                    "authHeader": true
                }
            }
        }),
    );

    let registry = ModelRegistry::new(
        Arc::new(MemoryAuthStorage::new()),
        built_in_models(),
        Some(models_json_path),
    );
    let model = registry.find("openai", "gpt-4o").unwrap();

    let error = registry.get_api_key_and_headers(&model).unwrap_err();
    assert!(error.contains("Failed to resolve API key for provider \"openai\""));
}
