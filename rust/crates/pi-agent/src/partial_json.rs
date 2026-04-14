use serde_json::{Map, Value};
use std::char::decode_utf16;
use std::ops::Range;

pub(crate) fn parse_partial_object(
    text: &str,
) -> Option<std::collections::BTreeMap<String, Value>> {
    if text.trim().is_empty() {
        return Some(std::collections::BTreeMap::new());
    }

    let mut parser = PartialJsonParser::new(text);
    parser.skip_whitespace();
    let object = match parser.parse_object()? {
        ParseStatus::Complete(object) | ParseStatus::Partial(object) => object,
    };
    Some(std::collections::BTreeMap::from_iter(object))
}

#[derive(Debug, Clone, PartialEq)]
enum ParseStatus<T> {
    Complete(T),
    Partial(T),
}

impl<T> ParseStatus<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> ParseStatus<U> {
        match self {
            Self::Complete(value) => ParseStatus::Complete(f(value)),
            Self::Partial(value) => ParseStatus::Partial(f(value)),
        }
    }

    fn into_inner(self) -> T {
        match self {
            Self::Complete(value) | Self::Partial(value) => value,
        }
    }
}

struct PartialJsonParser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> PartialJsonParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn consume_if(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_object(&mut self) -> Option<ParseStatus<Map<String, Value>>> {
        if !self.consume_if(b'{') {
            return None;
        }

        let mut object = Map::new();
        self.skip_whitespace();
        if self.is_eof() {
            return Some(ParseStatus::Partial(object));
        }
        if self.consume_if(b'}') {
            return Some(ParseStatus::Complete(object));
        }

        loop {
            self.skip_whitespace();
            if self.is_eof() {
                return Some(ParseStatus::Partial(object));
            }
            if self.consume_if(b'}') {
                return Some(ParseStatus::Complete(object));
            }

            let key = match self.parse_string() {
                Some(ParseStatus::Complete(key)) => key,
                Some(ParseStatus::Partial(_)) | None => return Some(ParseStatus::Partial(object)),
            };

            self.skip_whitespace();
            if !self.consume_if(b':') {
                return Some(ParseStatus::Partial(object));
            }

            self.skip_whitespace();
            if self.is_eof() {
                return Some(ParseStatus::Partial(object));
            }

            let value = match self.parse_value() {
                Some(value) => value,
                None => return Some(ParseStatus::Partial(object)),
            };
            object.insert(key, value.into_inner());

            self.skip_whitespace();
            if self.is_eof() {
                return Some(ParseStatus::Partial(object));
            }
            if self.consume_if(b',') {
                self.skip_whitespace();
                if self.is_eof() {
                    return Some(ParseStatus::Partial(object));
                }
                continue;
            }
            if self.consume_if(b'}') {
                return Some(ParseStatus::Complete(object));
            }
            return Some(ParseStatus::Partial(object));
        }
    }

    fn parse_array(&mut self) -> Option<ParseStatus<Vec<Value>>> {
        if !self.consume_if(b'[') {
            return None;
        }

        let mut array = Vec::new();
        self.skip_whitespace();
        if self.is_eof() {
            return Some(ParseStatus::Partial(array));
        }
        if self.consume_if(b']') {
            return Some(ParseStatus::Complete(array));
        }

        loop {
            self.skip_whitespace();
            if self.is_eof() {
                return Some(ParseStatus::Partial(array));
            }
            if self.consume_if(b']') {
                return Some(ParseStatus::Complete(array));
            }

            let value = match self.parse_value() {
                Some(value) => value,
                None => return Some(ParseStatus::Partial(array)),
            };
            array.push(value.into_inner());

            self.skip_whitespace();
            if self.is_eof() {
                return Some(ParseStatus::Partial(array));
            }
            if self.consume_if(b',') {
                self.skip_whitespace();
                if self.is_eof() {
                    return Some(ParseStatus::Partial(array));
                }
                continue;
            }
            if self.consume_if(b']') {
                return Some(ParseStatus::Complete(array));
            }
            return Some(ParseStatus::Partial(array));
        }
    }

    fn parse_value(&mut self) -> Option<ParseStatus<Value>> {
        self.skip_whitespace();
        match self.peek()? {
            b'{' => self.parse_object().map(|value| value.map(Value::Object)),
            b'[' => self.parse_array().map(|value| value.map(Value::Array)),
            b'"' => self.parse_string().map(|value| value.map(Value::String)),
            b't' | b'f' | b'n' => self.parse_literal(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => None,
        }
    }

    fn parse_string(&mut self) -> Option<ParseStatus<String>> {
        if !self.consume_if(b'"') {
            return None;
        }

        let mut output = String::new();
        let mut segment_start = self.pos;

        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    output.push_str(self.slice(segment_start..self.pos)?);
                    self.pos += 1;
                    return Some(ParseStatus::Complete(output));
                }
                b'\\' => {
                    output.push_str(self.slice(segment_start..self.pos)?);
                    self.pos += 1;
                    if self.is_eof() {
                        return Some(ParseStatus::Partial(output));
                    }

                    let escape = self.peek()?;
                    self.pos += 1;
                    match escape {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{0008}'),
                        b'f' => output.push('\u{000c}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => {
                            if let Some(character) = self.parse_unicode_escape() {
                                output.push(character);
                            } else {
                                return Some(ParseStatus::Partial(output));
                            }
                        }
                        _ => return Some(ParseStatus::Partial(output)),
                    }
                    segment_start = self.pos;
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        output.push_str(self.slice(segment_start..self.pos)?);
        Some(ParseStatus::Partial(output))
    }

    fn parse_unicode_escape(&mut self) -> Option<char> {
        let first = self.parse_u16_hex()?;
        if (0xD800..=0xDBFF).contains(&first)
            && self.pos + 6 <= self.bytes.len()
            && self.bytes[self.pos] == b'\\'
            && self.bytes[self.pos + 1] == b'u'
            && self.bytes[self.pos + 2..self.pos + 6]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            let low_slice = self.slice(self.pos + 2..self.pos + 6)?;
            let second = u16::from_str_radix(low_slice, 16).ok()?;
            if (0xDC00..=0xDFFF).contains(&second) {
                self.pos += 6;
                if let Some(Ok(character)) = decode_utf16([first, second]).next() {
                    return Some(character);
                }
            }
        }

        decode_utf16([first]).next().and_then(Result::ok)
    }

    fn parse_u16_hex(&mut self) -> Option<u16> {
        if self.pos + 4 > self.bytes.len() {
            return None;
        }
        if !self.bytes[self.pos..self.pos + 4]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        let slice = self.slice(self.pos..self.pos + 4)?;
        self.pos += 4;
        u16::from_str_radix(slice, 16).ok()
    }

    fn parse_literal(&mut self) -> Option<ParseStatus<Value>> {
        for (literal, value) in [
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("null", Value::Null),
        ] {
            if self.text[self.pos..].starts_with(literal) {
                self.pos += literal.len();
                return Some(ParseStatus::Complete(value));
            }

            let remaining = &self.text[self.pos..];
            if !remaining.is_empty() && literal.starts_with(remaining) {
                self.pos = self.bytes.len();
                return Some(ParseStatus::Partial(value));
            }
        }

        None
    }

    fn parse_number(&mut self) -> Option<ParseStatus<Value>> {
        let start = self.pos;

        if self.consume_if(b'-') && self.is_eof() {
            self.pos = start;
            return None;
        }

        match self.peek()? {
            b'0' => {
                self.pos += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos = start;
                    return None;
                }
            }
            b'1'..=b'9' => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => {
                self.pos = start;
                return None;
            }
        }

        let mut last_valid = self.pos;

        if self.consume_if(b'.') {
            let fraction_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos > fraction_start {
                last_valid = self.pos;
            } else {
                self.pos = last_valid;
                let value = serde_json::from_str::<Value>(self.slice(start..last_valid)?).ok()?;
                return Some(ParseStatus::Partial(value));
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            let exponent_start = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let digits_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos > digits_start {
                last_valid = self.pos;
            } else {
                self.pos = last_valid;
                let value = serde_json::from_str::<Value>(self.slice(start..last_valid)?).ok()?;
                return Some(if exponent_start == last_valid {
                    ParseStatus::Partial(value)
                } else {
                    ParseStatus::Partial(value)
                });
            }
        }

        self.pos = last_valid;
        let value = serde_json::from_str::<Value>(self.slice(start..last_valid)?).ok()?;
        let status = if self.is_eof() {
            ParseStatus::Partial(value)
        } else {
            ParseStatus::Complete(value)
        };
        Some(status)
    }

    fn slice(&self, range: Range<usize>) -> Option<&'a str> {
        self.text.get(range)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_partial_object;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn parses_partial_object_prefixes_like_ts_streaming_json() {
        let cases = [
            ("{", json!({})),
            ("{\"value\":\"hi", json!({ "value": "hi" })),
            (
                "{\"value\":\"hi\",\"count\":12",
                json!({ "value": "hi", "count": 12 }),
            ),
            (
                "{\"args\":{\"path\":\"/tmp/fi",
                json!({ "args": { "path": "/tmp/fi" } }),
            ),
            ("{\"items\":[1,2", json!({ "items": [1, 2] })),
            ("{\"flag\":tru", json!({ "flag": true })),
            ("{\"flag\":fa", json!({ "flag": false })),
            ("{\"value\":1e", json!({ "value": 1 })),
            ("{\"value\":-1.2e+", json!({ "value": -1.2 })),
            ("{\"nested\":{", json!({ "nested": {} })),
            (
                "{\"nested\":[{\"value\":\"x",
                json!({ "nested": [{ "value": "x" }] }),
            ),
            ("{\"text\":\"a\\", json!({ "text": "a" })),
            ("{\"a\":1,", json!({ "a": 1 })),
            ("{\"n\":01", json!({})),
        ];

        for (input, expected) in cases {
            let parsed = parse_partial_object(input).unwrap();
            assert_eq!(
                parsed,
                BTreeMap::from_iter(expected.as_object().unwrap().clone())
            );
        }
    }
}
