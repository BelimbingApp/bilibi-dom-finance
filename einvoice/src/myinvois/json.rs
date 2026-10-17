//! Order-preserving, minified JSON for UBL documents (Bilimbi
//! `Bilimbi.Finance.Einvoice.MyInvois.Json`).
//!
//! The authority hashes and signs a document's exact bytes, so an object's
//! key order and a number's written form must survive a read and a write.
//! [`Json`] keeps an object's pairs in order and a number as its decimal
//! text, never through a float or a `serde_json::Value` (whose maps sort
//! keys): `1500.0` stays `1500.0`. The writer is minified and escapes as
//! Bilimbi's writer does: `"` and `\`, the control characters below a space
//! (`\n`, `\r`, `\t`, `\b`, `\f` short, the rest `\u00XX`), nothing else.
use rust_decimal::Decimal;
use std::fmt;

/// A JSON term with ordered object keys and numbers kept as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    /// A number's decimal text, written verbatim.
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Why text is not JSON.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid JSON at byte {at}: {what}")]
pub struct JsonError {
    pub at: usize,
    pub what: &'static str,
}

impl Json {
    /// An ordered object from `(key, value)` pairs, leaving out `None` values
    /// and empty arrays, as the builder's optional elements are.
    pub fn object<'k>(pairs: impl IntoIterator<Item = (&'k str, Option<Json>)>) -> Json {
        Json::Object(
            pairs
                .into_iter()
                .filter_map(|(key, value)| match value {
                    None => None,
                    Some(Json::Array(items)) if items.is_empty() => None,
                    Some(value) => Some((key.to_owned(), value)),
                })
                .collect(),
        )
    }

    /// The UBL JSON form of a basic value, `[{"_": value, attribute: ...}]`;
    /// a `None` attribute is left out.
    pub fn value(value: Json, attributes: &[(&str, Option<&str>)]) -> Json {
        let mut pairs = vec![("_".to_owned(), value)];
        pairs.extend(attributes.iter().filter_map(|(key, value)| {
            value.map(|text| ((*key).to_owned(), Json::String(text.to_owned())))
        }));
        Json::Array(vec![Json::Object(pairs)])
    }

    /// A string term.
    pub fn str(text: &str) -> Json {
        Json::String(text.to_owned())
    }

    /// A text value's UBL form, or `None` for a blank one.
    pub fn text(value: Option<&str>) -> Option<Json> {
        value.map(|text| Json::value(Json::str(text), &[]))
    }

    /// A decimal's UBL form, written as a JSON number in its own decimal text.
    pub fn number(value: Decimal) -> Json {
        Json::Number(value.to_string())
    }

    /// The value under `key` in an object, or `None`.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The first item of the one-item list every UBL element is, under `key`.
    pub fn first(&self, key: &str) -> Option<&Json> {
        match self.get(key)? {
            Json::Array(items) => items.first(),
            _ => None,
        }
    }

    /// Walks a path of UBL elements, each a one-item list, from this object.
    pub fn at(&self, path: &[&str]) -> Option<&Json> {
        path.iter().try_fold(self, |term, key| term.first(key))
    }

    /// The `_` text of the UBL element at `path`.
    pub fn text_at(&self, path: &[&str]) -> Option<&str> {
        match self.at(path)?.get("_")? {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    /// The object without `keys`.
    pub fn without(&self, keys: &[&str]) -> Json {
        match self {
            Json::Object(pairs) => Json::Object(
                pairs
                    .iter()
                    .filter(|(key, _)| !keys.contains(&key.as_str()))
                    .cloned()
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// The object with `pairs` appended after its existing keys.
    pub fn with(&self, pairs: Vec<(&str, Json)>) -> Json {
        match self {
            Json::Object(existing) => {
                let mut all = existing.clone();
                all.extend(
                    pairs
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value)),
                );
                Json::Object(all)
            }
            other => other.clone(),
        }
    }

    /// The object with the one-item list under `key` replaced by `f` of its
    /// first item; unchanged when `key` is absent.
    pub fn update_first(&self, key: &str, f: impl FnOnce(&Json) -> Json) -> Json {
        let Json::Object(pairs) = self else {
            return self.clone();
        };
        let mut f = Some(f);
        Json::Object(
            pairs
                .iter()
                .map(|(k, v)| {
                    if k == key {
                        if let (Json::Array(items), Some(f)) = (v, f.take()) {
                            if let Some(first) = items.first() {
                                let mut items = items.clone();
                                items[0] = f(first);
                                return (k.clone(), Json::Array(items));
                            }
                        }
                    }
                    (k.clone(), v.clone())
                })
                .collect(),
        )
    }

    /// Minified JSON bytes.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        write(self, &mut out);
        out
    }

    /// Reads JSON keeping key order and each number's decimal text.
    pub fn decode(text: &str) -> Result<Json, JsonError> {
        let mut reader = Reader {
            bytes: text.as_bytes(),
            at: 0,
        };
        reader.skip_space();
        let value = reader.value(0)?;
        reader.skip_space();
        if reader.at != reader.bytes.len() {
            return Err(reader.error("trailing characters"));
        }
        Ok(value)
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

fn write(value: &Json, out: &mut String) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(text) => out.push_str(text),
        Json::String(text) => write_string(text, out),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Json::Object(pairs) => {
            out.push('{');
            for (index, (key, value)) in pairs.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write(value, out);
            }
            out.push('}');
        }
    }
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

const MAX_DEPTH: usize = 128;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn error(&self, what: &'static str) -> JsonError {
        JsonError { at: self.at, what }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8, what: &'static str) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(what))
        }
    }

    fn literal(&mut self, word: &[u8], value: Json) -> Result<Json, JsonError> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error("too deeply nested"));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal(b"true", Json::Bool(true)),
            Some(b'f') => self.literal(b"false", Json::Bool(false)),
            Some(b'n') => self.literal(b"null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect(b'{', "expected '{'")?;
        let mut pairs = Vec::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(pairs));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a key"));
            }
            let key = self.string()?;
            self.skip_space();
            self.expect(b':', "expected ':'")?;
            self.skip_space();
            let value = self.value(depth + 1)?;
            pairs.push((key, value));
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(pairs));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect(b'[', "expected '['")?;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        let digits = |reader: &mut Self| {
            let from = reader.at;
            while matches!(reader.peek(), Some(b'0'..=b'9')) {
                reader.at += 1;
            }
            reader.at > from
        };
        if !digits(self) {
            return Err(self.error("expected digits"));
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !digits(self) {
                return Err(self.error("expected digits after '.'"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !digits(self) {
                return Err(self.error("expected digits in the exponent"));
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| self.error("number is not text"))?;
        Ok(Json::Number(text.to_owned()))
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"', "expected '\"'")?;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.at += 1;
            }
            out.push_str(
                std::str::from_utf8(&self.bytes[start..self.at])
                    .map_err(|_| self.error("string is not UTF-8"))?,
            );
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escaped = self.peek().ok_or_else(|| self.error("unexpected end"))?;
                    self.at += 1;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let unit = self.hex4()?;
                            let c = if (0xD800..0xDC00).contains(&unit) {
                                if !self.bytes[self.at..].starts_with(b"\\u") {
                                    return Err(self.error("lone surrogate"));
                                }
                                self.at += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.error("lone surrogate"));
                                }
                                0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)
                            } else {
                                unit
                            };
                            out.push(
                                char::from_u32(c).ok_or_else(|| self.error("bad code point"))?,
                            );
                        }
                        _ => return Err(self.error("bad escape")),
                    }
                }
                Some(_) => return Err(self.error("control character in string")),
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| u32::from_str_radix(d, 16).ok())
            .ok_or_else(|| self.error("bad \\u escape"))?;
        self.at += 4;
        Ok(digits)
    }
}
