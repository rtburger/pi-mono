use crate::{AgentTool, tool::AgentToolError};
use serde_json::{Number, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidationIssue {
    path: String,
    message: String,
}

pub(crate) fn validate_tool_arguments(
    tool: &AgentTool,
    arguments: Value,
) -> Result<Value, AgentToolError> {
    let original_arguments = arguments.clone();
    let mut validated_arguments = arguments;
    let mut issues = Vec::new();

    validate_schema(
        &tool.definition.parameters,
        &mut validated_arguments,
        "",
        &mut issues,
    );

    if issues.is_empty() {
        Ok(validated_arguments)
    } else {
        Err(AgentToolError::message(format_validation_error(
            &tool.definition.name,
            &original_arguments,
            &issues,
        )))
    }
}

fn validate_schema(
    schema: &Value,
    value: &mut Value,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(schema_object) = schema.as_object() else {
        return;
    };

    match schema_object.get("type") {
        Some(Value::String(kind)) => {
            validate_typed_schema(kind, schema_object, value, path, issues)
        }
        Some(Value::Array(kinds)) => {
            let type_names = kinds.iter().filter_map(Value::as_str).collect::<Vec<_>>();

            if type_names.is_empty() {
                validate_object_keywords(schema_object, value, path, issues);
                return;
            }

            for type_name in &type_names {
                let mut candidate = value.clone();
                let mut candidate_issues = Vec::new();
                validate_typed_schema(
                    type_name,
                    schema_object,
                    &mut candidate,
                    path,
                    &mut candidate_issues,
                );
                if candidate_issues.is_empty() {
                    *value = candidate;
                    return;
                }
            }

            issues.push(ValidationIssue {
                path: display_path(path),
                message: format!("must be {}", type_names.join(" or ")),
            });
        }
        _ => validate_object_keywords(schema_object, value, path, issues),
    }
}

fn validate_typed_schema(
    kind: &str,
    schema_object: &serde_json::Map<String, Value>,
    value: &mut Value,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    match kind {
        "object" => validate_object(schema_object, value, path, issues),
        "array" => validate_array(schema_object, value, path, issues),
        "string" => validate_string(value, path, issues),
        "number" => validate_number(value, path, issues),
        "integer" => validate_integer(value, path, issues),
        "boolean" => validate_boolean(value, path, issues),
        "null" => validate_null(value, path, issues),
        _ => validate_object_keywords(schema_object, value, path, issues),
    }
}

fn validate_object(
    schema_object: &serde_json::Map<String, Value>,
    value: &mut Value,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(object) = value.as_object_mut() else {
        issues.push(ValidationIssue {
            path: display_path(path),
            message: String::from("must be object"),
        });
        return;
    };

    let required = schema_object
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for property in required {
        let Some(property_name) = property.as_str() else {
            continue;
        };
        if !object.contains_key(property_name) {
            issues.push(ValidationIssue {
                path: required_path(path, property_name),
                message: format!("must have required property '{property_name}'"),
            });
        }
    }

    validate_object_keywords(schema_object, value, path, issues);
}

fn validate_object_keywords(
    schema_object: &serde_json::Map<String, Value>,
    value: &mut Value,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };

    let Some(properties) = schema_object.get("properties").and_then(Value::as_object) else {
        return;
    };

    for (property_name, property_schema) in properties {
        let Some(property_value) = object.get_mut(property_name) else {
            continue;
        };
        validate_schema(
            property_schema,
            property_value,
            &join_path(path, property_name),
            issues,
        );
    }
}

fn validate_array(
    schema_object: &serde_json::Map<String, Value>,
    value: &mut Value,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(items) = value.as_array_mut() else {
        issues.push(ValidationIssue {
            path: display_path(path),
            message: String::from("must be array"),
        });
        return;
    };

    let Some(item_schema) = schema_object.get("items") else {
        return;
    };

    for (index, item) in items.iter_mut().enumerate() {
        validate_schema(
            item_schema,
            item,
            &join_path(path, &index.to_string()),
            issues,
        );
    }
}

fn validate_string(value: &mut Value, path: &str, issues: &mut Vec<ValidationIssue>) {
    match value {
        Value::String(_) => {}
        Value::Number(number) => {
            *value = Value::String(number.to_string());
        }
        Value::Bool(boolean) => {
            *value = Value::String(boolean.to_string());
        }
        _ => issues.push(ValidationIssue {
            path: display_path(path),
            message: String::from("must be string"),
        }),
    }
}

fn validate_number(value: &mut Value, path: &str, issues: &mut Vec<ValidationIssue>) {
    match value {
        Value::Number(_) => {}
        Value::String(text) => match text.parse::<f64>() {
            Ok(number) if number.is_finite() => match Number::from_f64(number) {
                Some(number) => *value = Value::Number(number),
                None => issues.push(ValidationIssue {
                    path: display_path(path),
                    message: String::from("must be number"),
                }),
            },
            _ => issues.push(ValidationIssue {
                path: display_path(path),
                message: String::from("must be number"),
            }),
        },
        _ => issues.push(ValidationIssue {
            path: display_path(path),
            message: String::from("must be number"),
        }),
    }
}

fn validate_integer(value: &mut Value, path: &str, issues: &mut Vec<ValidationIssue>) {
    match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => {}
        Value::String(text) => {
            if let Ok(number) = text.parse::<i64>() {
                *value = Value::Number(Number::from(number));
            } else if let Ok(number) = text.parse::<u64>() {
                *value = Value::Number(Number::from(number));
            } else {
                issues.push(ValidationIssue {
                    path: display_path(path),
                    message: String::from("must be integer"),
                });
            }
        }
        _ => issues.push(ValidationIssue {
            path: display_path(path),
            message: String::from("must be integer"),
        }),
    }
}

fn validate_boolean(value: &mut Value, path: &str, issues: &mut Vec<ValidationIssue>) {
    match value {
        Value::Bool(_) => {}
        Value::String(text) if text.eq_ignore_ascii_case("true") => {
            *value = Value::Bool(true);
        }
        Value::String(text) if text.eq_ignore_ascii_case("false") => {
            *value = Value::Bool(false);
        }
        _ => issues.push(ValidationIssue {
            path: display_path(path),
            message: String::from("must be boolean"),
        }),
    }
}

fn validate_null(value: &mut Value, path: &str, issues: &mut Vec<ValidationIssue>) {
    if !value.is_null() {
        issues.push(ValidationIssue {
            path: display_path(path),
            message: String::from("must be null"),
        });
    }
}

fn join_path(path: &str, segment: &str) -> String {
    if path.is_empty() {
        segment.to_string()
    } else {
        format!("{path}/{segment}")
    }
}

fn display_path(path: &str) -> String {
    if path.is_empty() {
        String::from("root")
    } else {
        path.to_string()
    }
}

fn required_path(path: &str, property_name: &str) -> String {
    if path.is_empty() {
        property_name.to_string()
    } else {
        path.to_string()
    }
}

fn format_validation_error(
    tool_name: &str,
    original_arguments: &Value,
    issues: &[ValidationIssue],
) -> String {
    let formatted_issues = issues
        .iter()
        .map(|issue| format!("  - {}: {}", issue.path, issue.message))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Validation failed for tool \"{tool_name}\":\n{formatted_issues}\n\nReceived arguments:\n{}",
        serde_json::to_string_pretty(original_arguments)
            .unwrap_or_else(|_| String::from("<unserializable arguments>"))
    )
}

#[cfg(test)]
mod tests {
    use super::validate_tool_arguments;
    use crate::{AgentTool, AgentToolError};
    use pi_events::ToolDefinition;
    use serde_json::json;

    fn tool(parameters: serde_json::Value) -> AgentTool {
        AgentTool::new(
            ToolDefinition {
                name: String::from("echo"),
                description: String::from("Echo tool"),
                parameters,
            },
            |_tool_call_id, _args, _signal| async move {
                unreachable!("executor is not used in validation tests")
            },
        )
    }

    #[test]
    fn formats_root_required_property_errors_like_typescript() {
        let error = validate_tool_arguments(
            &tool(json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                },
                "required": ["value"]
            })),
            json!({}),
        )
        .unwrap_err();

        assert_eq!(
            error,
            AgentToolError::message(
                "Validation failed for tool \"echo\":\n  - value: must have required property 'value'\n\nReceived arguments:\n{}"
            )
        );
    }

    #[test]
    fn coerces_numbers_and_reports_nested_array_paths() {
        let validated = validate_tool_arguments(
            &tool(json!({
                "type": "object",
                "properties": {
                    "count": { "type": "integer" }
                },
                "required": ["count"]
            })),
            json!({ "count": "42" }),
        )
        .unwrap();
        assert_eq!(validated, json!({ "count": 42 }));

        let error = validate_tool_arguments(
            &tool(json!({
                "type": "object",
                "properties": {
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "oldText": { "type": "string" },
                                "newText": { "type": "string" }
                            },
                            "required": ["oldText", "newText"]
                        }
                    }
                },
                "required": ["edits"]
            })),
            json!({ "edits": [{ "oldText": "before" }] }),
        )
        .unwrap_err();

        assert_eq!(
            error,
            AgentToolError::message(
                "Validation failed for tool \"echo\":\n  - edits/0: must have required property 'newText'\n\nReceived arguments:\n{\n  \"edits\": [\n    {\n      \"oldText\": \"before\"\n    }\n  ]\n}"
            )
        );
    }
}
