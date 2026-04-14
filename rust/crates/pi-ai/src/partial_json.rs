use partial_json_fixer::{fix_json_parse, JsonUnit, JsonValue};
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) fn parse_partial_json_map(partial_json: &str) -> BTreeMap<String, Value> {
    let trimmed = partial_json.trim();
    if trimmed.is_empty() {
        return BTreeMap::new();
    }

    let mut value = match fix_json_parse(partial_json) {
        Ok(value) => value,
        Err(_) => return BTreeMap::new(),
    };

    if trimmed.ends_with(':') {
        prune_trailing_placeholder(&mut value);
    }

    match to_serde_value(value) {
        Value::Object(map) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

fn prune_trailing_placeholder(value: &mut JsonValue<'_>) -> bool {
    match value {
        JsonValue::Object(object) => {
            if object
                .values
                .last()
                .is_some_and(|(_, last_value)| is_placeholder_null(last_value))
            {
                object.values.pop();
                return true;
            }
            if let Some((_, last_value)) = object.values.last_mut()
                && prune_trailing_placeholder(last_value)
            {
                return true;
            }
            false
        }
        JsonValue::Array(array) => {
            if array
                .members
                .last()
                .is_some_and(is_placeholder_null)
            {
                array.members.pop();
                return true;
            }
            if let Some(last_value) = array.members.last_mut()
                && prune_trailing_placeholder(last_value)
            {
                return true;
            }
            false
        }
        _ => false,
    }
}

fn is_placeholder_null(value: &JsonValue<'_>) -> bool {
    matches!(value, JsonValue::Null | JsonValue::Unit(JsonUnit::Null))
}

fn to_serde_value(value: JsonValue<'_>) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Unit(unit) => unit_to_value(unit),
        JsonValue::Object(object) => {
            let mut map = serde_json::Map::new();
            for (key, value) in object.values {
                map.insert(unit_to_string(key), to_serde_value(value));
            }
            Value::Object(map)
        }
        JsonValue::Array(array) => {
            Value::Array(array.members.into_iter().map(to_serde_value).collect())
        }
    }
}

fn unit_to_value(unit: JsonUnit<'_>) -> Value {
    match unit {
        JsonUnit::String(text) => Value::String(text.to_string()),
        JsonUnit::Number(number) => serde_json::from_str::<Value>(number).unwrap_or(Value::Null),
        JsonUnit::True => Value::Bool(true),
        JsonUnit::False => Value::Bool(false),
        JsonUnit::Null => Value::Null,
    }
}

fn unit_to_string(unit: JsonUnit<'_>) -> String {
    match unit {
        JsonUnit::String(text) => text.to_string(),
        JsonUnit::Number(number) => number.to_string(),
        JsonUnit::True => "true".into(),
        JsonUnit::False => "false".into(),
        JsonUnit::Null => "null".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_partial_json_map;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn parses_partial_strings_inside_tool_arguments() {
        let actual = parse_partial_json_map(r#"{"path":"src/ma"#);
        let expected = BTreeMap::from([("path".to_string(), json!("src/ma"))]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn prunes_missing_values_at_the_tail_of_nested_objects() {
        let actual = parse_partial_json_map(r#"{"path":{"inner":"#);
        let expected = BTreeMap::from([("path".to_string(), json!({}))]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn preserves_nested_objects_and_arrays_while_pruning_the_missing_tail() {
        let actual = parse_partial_json_map(r#"{"path":[{"inner":"#);
        let expected = BTreeMap::from([("path".to_string(), json!([{}]))]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn prunes_top_level_missing_values() {
        let actual = parse_partial_json_map(r#"{"a":1,"b":"#);
        let expected = BTreeMap::from([("a".to_string(), json!(1))]);

        assert_eq!(actual, expected);
    }
}
