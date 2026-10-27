//! Small, tolerant JSON parser for theme and configuration files.
//!
//! Supports the same conveniences as the C# reader: `//` and `/* */`
//! comments, trailing commas and standard JSON escapes. It deliberately does
//! not expose a serde dependency, which keeps startup allocation-free for
//! callers that do not read data files.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    #[allow(dead_code)]
    Bool(bool),
    Number(f64),
    String(String),
    #[allow(dead_code)]
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        let Value::Object(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(value) if value.is_finite() => Some(*value as i64),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(entries) => Some(entries),
            _ => None,
        }
    }
}

pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        position: 0,
    };
    parser.skip_ignored();
    let value = parser.parse_value()?;
    parser.skip_ignored();
    if parser.position != parser.bytes.len() {
        return Err(parser.error("trailing characters"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.position)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let value = self.peek();
        if value.is_some() {
            self.position += 1;
        }
        value
    }

    fn skip_ignored(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.position += 1;
            }
            if self.peek() == Some(b'/') {
                match self.bytes.get(self.position + 1).copied() {
                    Some(b'/') => {
                        self.position += 2;
                        while let Some(byte) = self.peek() {
                            self.position += 1;
                            if byte == b'\n' {
                                break;
                            }
                        }
                    }
                    Some(b'*') => {
                        self.position += 2;
                        while self.position + 1 < self.bytes.len() {
                            if self.bytes[self.position] == b'*'
                                && self.bytes[self.position + 1] == b'/'
                            {
                                self.position += 2;
                                break;
                            }
                            self.position += 1;
                        }
                    }
                    _ => return,
                }
            } else {
                return;
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ignored();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Value::String(self.parse_string()?)),
            Some(b't') => {
                self.expect_word(b"true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.expect_word(b"false")?;
                Ok(Value::Bool(false))
            }
            Some(b'n') => {
                self.expect_word(b"null")?;
                Ok(Value::Null)
            }
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => Err(self.error("expected a value")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn expect_word(&mut self, word: &[u8]) -> Result<(), String> {
        if self.bytes.get(self.position..self.position + word.len()) == Some(word) {
            self.position += word.len();
            Ok(())
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        if self.peek() == Some(b'.') {
            self.position += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position])
            .map_err(|_| self.error("invalid number"))?;
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|_| self.error("invalid number"))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.bump() != Some(b'"') {
            return Err(self.error("expected a string"));
        }
        let mut output = String::new();
        loop {
            let Some(byte) = self.bump() else {
                return Err(self.error("unterminated string"));
            };
            match byte {
                b'"' => return Ok(output),
                b'\\' => {
                    let Some(escape) = self.bump() else {
                        return Err(self.error("unterminated escape"));
                    };
                    match escape {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{8}'),
                        b'f' => output.push('\u{c}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => {
                            let mut scalar = 0u32;
                            for _ in 0..4 {
                                let Some(digit) = self.bump() else {
                                    return Err(self.error("short unicode escape"));
                                };
                                scalar = scalar * 16
                                    + (digit as char).to_digit(16).ok_or_else(|| {
                                        self.error("invalid unicode escape")
                                    })?;
                            }
                            if let Some(ch) = char::from_u32(scalar) {
                                output.push(ch);
                            } else {
                                output.push('\u{FFFD}');
                            }
                        }
                        _ => return Err(self.error("unknown escape")),
                    }
                }
                _ => {
                    // Re-decode UTF-8 properly: copy the full scalar.
                    let first = byte;
                    let length = match first {
                        0x00..=0x7F => 1,
                        0xC0..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF7 => 4,
                        _ => 1,
                    };
                    if length == 1 {
                        output.push(first as char);
                    } else {
                        let end = self.position - 1 + length;
                        if end > self.bytes.len() {
                            return Err(self.error("invalid utf-8"));
                        }
                        let slice = &self.bytes[self.position - 1..end];
                        self.position = end;
                        let text = std::str::from_utf8(slice)
                            .map_err(|_| self.error("invalid utf-8"))?;
                        output.push_str(text);
                    }
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.bump();
        let mut entries = Vec::new();
        loop {
            self.skip_ignored();
            if self.peek() == Some(b'}') {
                self.bump();
                return Ok(Value::Object(entries));
            }
            let key = self.parse_string()?;
            self.skip_ignored();
            if self.bump() != Some(b':') {
                return Err(self.error("expected ':'"));
            }
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ignored();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(Value::Object(entries)),
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.bump();
        let mut values = Vec::new();
        loop {
            self.skip_ignored();
            if self.peek() == Some(b']') {
                self.bump();
                return Ok(Value::Array(values));
            }
            values.push(self.parse_value()?);
            self.skip_ignored();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(Value::Array(values)),
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_and_trailing_commas() {
        let value = parse(
            r##"{
                // a comment
                "id": "test", /* block */
                "colors": { "foreground": "#abc", "count": 12, },
            }"##,
        )
        .unwrap();
        assert_eq!(value.get("id").unwrap().as_str(), Some("test"));
        assert_eq!(
            value.get("colors").unwrap().get("count").unwrap().as_i64(),
            Some(12)
        );
    }

    #[test]
    fn parses_escapes() {
        let value = parse(r#"{"name":"a\u263a\n"}"#).unwrap();
        assert_eq!(value.get("name").unwrap().as_str(), Some("a☺\n"));
    }
}
