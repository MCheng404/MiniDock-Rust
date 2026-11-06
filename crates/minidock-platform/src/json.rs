//! A very small JSON reader.
//!
//! The updater parses exactly one document shape (the GitHub release API) plus
//! one DoH answer. Pulling `serde_json` in would cost ~300 KB of code and a
//! second derive stack for two structs, so this module only does what those
//! two need: objects, arrays, strings, numbers, booleans, null. No writer, no
//! error positions, no 64-bit integers beyond `f64`.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Object member lookup. Duplicate keys keep the first one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_arr(&self) -> Option<&[Self]> {
        match self {
            Self::Arr(items) => Some(items),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON 解析失败")
    }
}

impl std::error::Error for ParseError {}

/// Parse a complete document; trailing whitespace is allowed.
pub fn parse(src: &str) -> Result<Json, ParseError> {
    let mut p = Parser {
        bytes: src.as_bytes(),
        pos: 0,
    };
    let value = p.value(0)?;
    p.skip_ws();
    if p.pos == p.bytes.len() {
        Ok(value)
    } else {
        Err(ParseError)
    }
}

/// Nesting limit: GitHub responses nest ~6 deep; anything past 64 is a bomb.
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_DEPTH {
            return Err(ParseError);
        }
        self.skip_ws();
        match self.peek()? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            _ => self.number(),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.expect(b'{')?;
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek()? == b'}' {
            self.pos += 1;
            return Ok(Json::Obj(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value(depth + 1)?;
            pairs.push((key, value));
            self.skip_ws();
            match self.next()? {
                b',' => continue,
                b'}' => return Ok(Json::Obj(pairs)),
                _ => return Err(ParseError),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek()? == b']' {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.next()? {
                b',' => continue,
                b']' => return Ok(Json::Arr(items)),
                _ => return Err(ParseError),
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let b = self.next()?;
            match b {
                b'"' => return Ok(out),
                b'\\' => match self.next()? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hi = self.hex4()?;
                        // Surrogate pair: recombine in u32, u16 would wrap.
                        let code: u32 = if (0xD800..0xDC00).contains(&hi) {
                            if self.peek()? == b'\\' {
                                self.pos += 1;
                                self.expect(b'u')?;
                                let lo = self.hex4()?;
                                0x1_0000
                                    + ((u32::from(hi) - 0xD800) << 10)
                                    + (u32::from(lo).saturating_sub(0xDC00))
                            } else {
                                0xFFFD
                            }
                        } else {
                            u32::from(hi)
                        };
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    _ => return Err(ParseError),
                },
                _ if b < 0x80 => out.push(b as char),
                _ => {
                    // Multi-byte UTF-8: re-decode from the current offset.
                    self.pos -= 1;
                    let rest =
                        std::str::from_utf8(&self.bytes[self.pos..]).map_err(|_| ParseError)?;
                    let ch = rest.chars().next().ok_or(ParseError)?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u16, ParseError> {
        let mut v: u16 = 0;
        for _ in 0..4 {
            let b = self.next()?;
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(ParseError),
            };
            v = (v << 4) | u16::from(d);
        }
        Ok(v)
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        while let Ok(b) = self.peek() {
            if b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| ParseError)?;
        text.parse::<f64>().map(Json::Num).map_err(|_| ParseError)
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, ParseError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(ParseError)
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Ok(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Result<u8, ParseError> {
        self.bytes.get(self.pos).copied().ok_or(ParseError)
    }

    fn next(&mut self) -> Result<u8, ParseError> {
        let b = self.peek()?;
        self.pos += 1;
        Ok(b)
    }

    fn expect(&mut self, want: u8) -> Result<(), ParseError> {
        if self.next()? == want {
            Ok(())
        } else {
            Err(ParseError)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, Json};

    #[test]
    fn reads_a_github_style_document() {
        let doc = r#"{
            "tag_name": "v1.0.1",
            "draft": false,
            "id": 12345,
            "assets": [
                {"name": "MiniDock-1.0.1-win64.exe", "size": 7782400,
                 "browser_download_url": "https://github.com/a/b/releases/download/v1.0.1/x.exe"}
            ],
            "body": "line\nwith \\\"quote\\\" and \u4e2d\u6587"
        }"#;
        let v = parse(doc).expect("parses");
        assert_eq!(v.get("tag_name").and_then(Json::as_str), Some("v1.0.1"));
        assert_eq!(
            v.get("draft"),
            Some(&Json::Bool(false)),
            "booleans must not be strings"
        );
        let asset = &v.get("assets").and_then(Json::as_arr).expect("assets")[0];
        assert_eq!(asset.get("size").and_then(Json::as_f64), Some(7_782_400.0));
        assert!(v
            .get("body")
            .and_then(Json::as_str)
            .expect("body")
            .contains("中文"));
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(parse(r#"{"a": [1, 2"#).is_err());
        assert!(parse("").is_err());
        assert!(parse(r#"{"a":1} trailing"#).is_err());
    }
}
