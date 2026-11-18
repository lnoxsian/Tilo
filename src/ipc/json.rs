//! Lightweight, zero-dependency JSON serialization and parsing for Tilo IPC.

use std::fmt::Write;

/// A lightweight JSON value representation.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(i64),
    Float(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Creates an empty JSON object.
    pub fn object() -> Self {
        JsonValue::Object(Vec::new())
    }

    /// Creates an empty JSON array.
    pub fn array() -> Self {
        JsonValue::Array(Vec::new())
    }

    /// Appends a value to an array. Panics if not an array.
    pub fn push(&mut self, val: JsonValue) {
        match self {
            JsonValue::Array(arr) => arr.push(val),
            _ => panic!("push called on non-array JsonValue"),
        }
    }

    /// Inserts a key-value pair into an object. Panics if not an object.
    pub fn insert(&mut self, key: impl Into<String>, val: JsonValue) {
        match self {
            JsonValue::Object(obj) => obj.push((key.into(), val)),
            _ => panic!("insert called on non-object JsonValue"),
        }
    }

    /// Serializes this JSON value into a string.
    pub fn to_json_string(&self) -> String {
        let mut buf = String::with_capacity(128);
        self.write_json(&mut buf);
        buf
    }

    fn write_json(&self, buf: &mut String) {
        match self {
            JsonValue::Null => buf.push_str("null"),
            JsonValue::Bool(b) => buf.push_str(if *b { "true" } else { "false" }),
            JsonValue::Number(n) => {
                let _ = write!(buf, "{}", n);
            }
            JsonValue::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    buf.push_str("null");
                } else {
                    let _ = write!(buf, "{:.4}", f);
                }
            }
            JsonValue::Str(s) => escape_str(s, buf),
            JsonValue::Array(arr) => {
                buf.push('[');
                for (i, item) in arr.iter().enumerate() {
                    if i > 0 {
                        buf.push(',');
                    }
                    item.write_json(buf);
                }
                buf.push(']');
            }
            JsonValue::Object(obj) => {
                buf.push('{');
                for (i, (key, val)) in obj.iter().enumerate() {
                    if i > 0 {
                        buf.push(',');
                    }
                    escape_str(key, buf);
                    buf.push(':');
                    val.write_json(buf);
                }
                buf.push('}');
            }
        }
    }

    /// Simple JSON parser for basic values, arrays, and objects.
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut parser = JsonParser::new(input.trim());
        let val = parser.parse_value()?;
        parser.skip_whitespace();
        if !parser.is_eof() {
            return Err("Unexpected trailing characters after JSON".to_string());
        }
        Ok(val)
    }

    /// Helper to get a string slice if this is a Str.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Helper to get an array slice if this is an Array.
    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(arr) => Some(arr),
            _ => None,
        }
    }
}

fn escape_str(s: &str, buf: &mut String) {
    buf.push('"');
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            '\u{0008}' => buf.push_str("\\b"),
            '\u{000C}' => buf.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(buf, "\\u{:04x}", c as u32);
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

struct JsonParser<'a> {
    chars: Vec<char>,
    pos: usize,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
            _marker: std::marker::PhantomData,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_whitespace();
        let c = self.peek().ok_or("Unexpected end of input")?;
        match c {
            'n' => self.parse_null(),
            't' | 'f' => self.parse_bool(),
            '"' => self.parse_string().map(JsonValue::Str),
            '[' => self.parse_array(),
            '{' => self.parse_object(),
            '-' | '0'..='9' => self.parse_number(),
            other => Err(format!("Unexpected character '{}' at position {}", other, self.pos)),
        }
    }

    fn parse_null(&mut self) -> Result<JsonValue, String> {
        if self.match_keyword("null") {
            Ok(JsonValue::Null)
        } else {
            Err("Expected 'null'".to_string())
        }
    }

    fn parse_bool(&mut self) -> Result<JsonValue, String> {
        if self.match_keyword("true") {
            Ok(JsonValue::Bool(true))
        } else if self.match_keyword("false") {
            Ok(JsonValue::Bool(false))
        } else {
            Err("Expected boolean ('true' or 'false')".to_string())
        }
    }

    fn match_keyword(&mut self, kw: &str) -> bool {
        let kw_chars: Vec<char> = kw.chars().collect();
        if self.pos + kw_chars.len() <= self.chars.len()
            && &self.chars[self.pos..self.pos + kw_chars.len()] == kw_chars.as_slice()
        {
            self.pos += kw_chars.len();
            true
        } else {
            false
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.next() != Some('"') {
            return Err("Expected string opening quote".to_string());
        }
        let mut s = String::new();
        while let Some(c) = self.next() {
            match c {
                '"' => return Ok(s),
                '\\' => {
                    let esc = self.next().ok_or("Unterminated escape in string")?;
                    match esc {
                        '"' => s.push('"'),
                        '\\' => s.push('\\'),
                        '/' => s.push('/'),
                        'b' => s.push('\u{0008}'),
                        'f' => s.push('\u{000C}'),
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        'u' => {
                            let mut hex = String::with_capacity(4);
                            for _ in 0..4 {
                                hex.push(self.next().ok_or("Incomplete unicode escape")?);
                            }
                            let code = u32::from_str_radix(&hex, 16)
                                .map_err(|_| "Invalid hex in unicode escape")?;
                            let ch = char::from_u32(code)
                                .ok_or("Invalid unicode codepoint")?;
                            s.push(ch);
                        }
                        other => return Err(format!("Unknown escape sequence \\{}", other)),
                    }
                }
                other => s.push(other),
            }
        }
        Err("Unterminated string".to_string())
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let mut num_str = String::new();
        let mut is_float = false;

        while let Some(c) = self.peek() {
            if c == '-' || c == '+' || c.is_ascii_digit() {
                num_str.push(c);
                self.pos += 1;
            } else if c == '.' || c == 'e' || c == 'E' {
                is_float = true;
                num_str.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }

        if is_float {
            num_str
                .parse::<f64>()
                .map(JsonValue::Float)
                .map_err(|e| format!("Invalid float number '{}': {}", num_str, e))
        } else {
            num_str
                .parse::<i64>()
                .map(JsonValue::Number)
                .map_err(|e| format!("Invalid integer number '{}': {}", num_str, e))
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        if self.next() != Some('[') {
            return Err("Expected '['".to_string());
        }
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }

        loop {
            let item = self.parse_value()?;
            items.push(item);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                other => return Err(format!("Expected ',' or ']' in array, got {:?}", other)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        if self.next() != Some('{') {
            return Err("Expected '{'".to_string());
        }
        let mut obj = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(JsonValue::Object(obj));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.next() != Some(':') {
                return Err("Expected ':' after object key".to_string());
            }
            let val = self.parse_value()?;
            obj.push((key, val));
            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(obj));
                }
                other => return Err(format!("Expected ',' or '}}' in object, got {:?}", other)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_value_serialization() {
        let mut obj = JsonValue::object();
        obj.insert("success", JsonValue::Bool(true));
        obj.insert("code", JsonValue::Number(200));
        obj.insert("message", JsonValue::Str("hello \"world\"\n".to_string()));

        let mut arr = JsonValue::array();
        arr.push(JsonValue::Number(1));
        arr.push(JsonValue::Number(2));
        obj.insert("items", arr);

        let json = obj.to_json_string();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"code\":200"));
        assert!(json.contains("\"message\":\"hello \\\"world\\\"\\n\""));
        assert!(json.contains("\"items\":[1,2]"));
    }

    #[test]
    fn test_json_parser_roundtrip() {
        let json = r#"{"name":"tilo","count":42,"tags":["bsp","tabbed"],"active":true,"meta":null}"#;
        let val = JsonValue::parse(json).expect("Parse error");
        assert_eq!(
            val.to_json_string(),
            r#"{"name":"tilo","count":42,"tags":["bsp","tabbed"],"active":true,"meta":null}"#
        );
    }
}
