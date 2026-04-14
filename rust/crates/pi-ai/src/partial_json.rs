use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

pub(crate) fn parse_partial_json_map(partial_json: &str) -> BTreeMap<String, Value> {
    let trimmed = partial_json.trim();
    if trimmed.is_empty() {
        return BTreeMap::new();
    }

    let mut parser = Parser::new(trimmed);
    match parser.parse_value() {
        Some(Value::Object(object)) => object.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

struct Parser<'a> {
    source: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, index: 0 }
    }

    fn parse_value(&mut self) -> Option<Value> {
        self.skip_whitespace();
        let ch = self.peek_char()?;
        match ch {
            '"' => self.parse_string().map(Value::String),
            '{' => self.parse_object(),
            '[' => self.parse_array(),
            't' => self.parse_literal("true", Value::Bool(true)),
            'f' => self.parse_literal("false", Value::Bool(false)),
            'n' => self.parse_literal("null", Value::Null),
            _ => self.parse_number().map(Value::Number),
        }
    }

    fn parse_object(&mut self) -> Option<Value> {
        self.consume_char()?;
        let mut object = Map::new();

        loop {
            self.skip_whitespace();
            match self.peek_char() {
                Some('}') => {
                    self.consume_char();
                    return Some(Value::Object(object));
                }
                None => return Some(Value::Object(object)),
                _ => {}
            }

            let key = match self.parse_string() {
                Some(key) => key,
                None => return Some(Value::Object(object)),
            };

            self.skip_whitespace();
            if self.peek_char() != Some(':') {
                return Some(Value::Object(object));
            }
            self.consume_char();

            let value = match self.parse_value() {
                Some(value) => value,
                None => return Some(Value::Object(object)),
            };
            object.insert(key, value);

            self.skip_whitespace();
            match self.peek_char() {
                Some(',') => {
                    self.consume_char();
                }
                Some('}') => {
                    self.consume_char();
                    return Some(Value::Object(object));
                }
                None => return Some(Value::Object(object)),
                _ => return Some(Value::Object(object)),
            }
        }
    }

    fn parse_array(&mut self) -> Option<Value> {
        self.consume_char()?;
        let mut values = Vec::new();

        loop {
            self.skip_whitespace();
            match self.peek_char() {
                Some(']') => {
                    self.consume_char();
                    return Some(Value::Array(values));
                }
                None => return Some(Value::Array(values)),
                _ => {}
            }

            let value = match self.parse_value() {
                Some(value) => value,
                None => return Some(Value::Array(values)),
            };
            values.push(value);

            self.skip_whitespace();
            match self.peek_char() {
                Some(',') => {
                    self.consume_char();
                }
                Some(']') => {
                    self.consume_char();
                    return Some(Value::Array(values));
                }
                None => return Some(Value::Array(values)),
                _ => return Some(Value::Array(values)),
            }
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        let start = self.index;
        self.consume_char()?;

        let mut escaped = false;
        while let Some(ch) = self.peek_char() {
            if ch == '"' && !escaped {
                let end = self.index + ch.len_utf8();
                let slice = &self.source[start..end];
                self.index = end;
                return serde_json::from_str::<String>(slice).ok();
            }

            escaped = if ch == '\\' { !escaped } else { false };
            self.index += ch.len_utf8();
        }

        let candidate = format!("{}\"", &self.source[start..self.index]);
        if let Ok(value) = serde_json::from_str::<String>(&candidate) {
            return Some(value);
        }

        if let Some(last_backslash) = self.source[start..self.index].rfind('\\') {
            let candidate = format!("{}\"", &self.source[start..start + last_backslash]);
            if let Ok(value) = serde_json::from_str::<String>(&candidate) {
                return Some(value);
            }
        }

        None
    }

    fn parse_number(&mut self) -> Option<Number> {
        let start = self.index;
        if self.peek_char() == Some('-') {
            self.consume_char();
        }

        while let Some(ch) = self.peek_char() {
            if matches!(ch, ',' | ']' | '}') {
                break;
            }
            self.index += ch.len_utf8();
        }

        let candidate = self.source[start..self.index].trim_end();
        if candidate == "-" {
            return None;
        }

        if let Ok(Value::Number(number)) = serde_json::from_str::<Value>(candidate) {
            return Some(Self::normalize_number(number));
        }

        if let Some(exponent_index) = candidate.rfind('e') {
            let candidate = candidate[..exponent_index].trim_end();
            if let Ok(Value::Number(number)) = serde_json::from_str::<Value>(candidate) {
                return Some(Self::normalize_number(number));
            }
        }

        None
    }

    fn normalize_number(number: Number) -> Number {
        let Some(value) = number.as_f64() else {
            return number;
        };

        if !value.is_finite() || value.fract() != 0.0 {
            return number;
        }

        if value >= 0.0 {
            let integer = value as u64;
            if (integer as f64) == value {
                return Number::from(integer);
            }
        } else {
            let integer = value as i64;
            if (integer as f64) == value {
                return Number::from(integer);
            }
        }

        number
    }

    fn parse_literal(&mut self, literal: &str, value: Value) -> Option<Value> {
        let remaining = &self.source[self.index..];
        if remaining.starts_with(literal) {
            self.index += literal.len();
            return Some(value);
        }

        if literal.starts_with(remaining) {
            self.index = self.source.len();
            return Some(value);
        }

        None
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek_char(), Some(' ' | '\n' | '\r' | '\t')) {
            self.consume_char();
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.index..].chars().next()
    }

    fn consume_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.index += ch.len_utf8();
        Some(ch)
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
    fn parses_partial_strings_inside_nested_objects_and_arrays() {
        let actual = parse_partial_json_map(r#"{"path":{"inner":""#);
        let expected = BTreeMap::from([("path".to_string(), json!({"inner": ""}))]);

        assert_eq!(actual, expected);

        let actual = parse_partial_json_map(r#"{"path":[{"inner":""#);
        let expected = BTreeMap::from([("path".to_string(), json!([{"inner": ""}]))]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn prunes_missing_values_at_the_tail_of_nested_objects() {
        let actual = parse_partial_json_map(r#"{"path":{"inner":"#);
        let expected = BTreeMap::from([("path".to_string(), json!({}))]);

        assert_eq!(actual, expected);

        let actual = parse_partial_json_map(r#"{"a":1,"b:"#);
        let expected = BTreeMap::from([("a".to_string(), json!(1))]);

        assert_eq!(actual, expected);

        let actual = parse_partial_json_map(r#"{"path":1."#);
        assert_eq!(actual, BTreeMap::new());

        let actual = parse_partial_json_map(r#"{"enabled":tru"#);
        let expected = BTreeMap::from([("enabled".to_string(), json!(true))]);

        assert_eq!(actual, expected);

        let actual = parse_partial_json_map(r#"{"disabled":fals"#);
        let expected = BTreeMap::from([("disabled".to_string(), json!(false))]);

        assert_eq!(actual, expected);

        let actual = parse_partial_json_map(r#"{"value":nul"#);
        let expected = BTreeMap::from([("value".to_string(), json!(null))]);

        assert_eq!(actual, expected);

        let actual = parse_partial_json_map(r#"{"path":1e"#);
        let expected = BTreeMap::from([("path".to_string(), json!(1))]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn preserves_nested_objects_and_arrays_while_pruning_the_missing_tail() {
        let actual = parse_partial_json_map(r#"{"path":[{"inner:"#);
        let expected = BTreeMap::from([("path".to_string(), json!([{}]))]);

        assert_eq!(actual, expected);
    }
}
