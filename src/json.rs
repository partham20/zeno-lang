//! JSON: the text an editor and the language server talk in.
//!
//! The language server (`lsp.rs`) gets its requests as JSON and answers
//! in JSON. Zeno uses no library, so this file is a reader and a writer
//! for it. JSON is small: six kinds of value, and this is all of them.

use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Json>),
    /// The members in the order they were written.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Read one JSON value. `None` if the text is not JSON.
    pub fn parse(text: &str) -> Option<Json> {
        let chars: Vec<char> = text.chars().collect();
        let mut reader = Reader { chars: &chars, at: 0 };
        let value = reader.value()?;
        reader.spaces();
        (reader.at == chars.len()).then_some(value)
    }

    /// An object, from its members.
    pub fn object<const N: usize>(members: [(&str, Json); N]) -> Json {
        Json::Object(members.into_iter().map(|(name, value)| (name.to_string(), value)).collect())
    }

    pub fn text(text: impl Into<String>) -> Json {
        Json::Text(text.into())
    }

    pub fn number(number: usize) -> Json {
        Json::Number(number as f64)
    }

    /// The member `name` of an object.
    pub fn get(&self, name: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members.iter().find(|(member, _)| member == name).map(|(_, value)| value),
            _ => None,
        }
    }

    /// A member some way down: `at(&["params", "textDocument", "uri"])`.
    pub fn at(&self, path: &[&str]) -> Option<&Json> {
        path.iter().try_fold(self, |value, name| value.get(name))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_usize(&self) -> Option<usize> {
        match self {
            Json::Number(number) if *number >= 0.0 && number.fract() == 0.0 => Some(*number as usize),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Json]> {
        match self {
            Json::List(items) => Some(items),
            _ => None,
        }
    }

    /// The value as JSON text, on one line.
    pub fn write(&self) -> String {
        let mut out = String::new();
        self.write_into(&mut out);
        out
    }

    fn write_into(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Number(number) if number.fract() == 0.0 && number.abs() < 1e15 => {
                let _ = write!(out, "{}", *number as i64);
            }
            Json::Number(number) => {
                let _ = write!(out, "{number}");
            }
            Json::Text(text) => write_text(text, out),
            Json::List(items) => {
                out.push('[');
                for (nth, item) in items.iter().enumerate() {
                    if nth > 0 {
                        out.push(',');
                    }
                    item.write_into(out);
                }
                out.push(']');
            }
            Json::Object(members) => {
                out.push('{');
                for (nth, (name, value)) in members.iter().enumerate() {
                    if nth > 0 {
                        out.push(',');
                    }
                    write_text(name, out);
                    out.push(':');
                    value.write_into(out);
                }
                out.push('}');
            }
        }
    }
}

/// A text in quotes, with what JSON cannot hold as it is written `\n`,
/// `\"` and so on.
fn write_text(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Reader<'a> {
    chars: &'a [char],
    at: usize,
}

impl Reader<'_> {
    fn spaces(&mut self) {
        while matches!(self.chars.get(self.at), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
    }

    /// Step over `word` if it is what comes next.
    fn word(&mut self, word: &str) -> bool {
        let here = self.chars[self.at..].iter().take(word.len()).collect::<String>() == word;
        if here {
            self.at += word.len();
        }
        here
    }

    fn value(&mut self) -> Option<Json> {
        self.spaces();
        match *self.chars.get(self.at)? {
            'n' => self.word("null").then_some(Json::Null),
            't' => self.word("true").then_some(Json::Bool(true)),
            'f' => self.word("false").then_some(Json::Bool(false)),
            '"' => self.text().map(Json::Text),
            '[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.spaces();
                    if self.word("]") {
                        return Some(Json::List(items));
                    }
                    if !items.is_empty() && !self.word(",") {
                        return None;
                    }
                    items.push(self.value()?);
                }
            }
            '{' => {
                self.at += 1;
                let mut members = Vec::new();
                loop {
                    self.spaces();
                    if self.word("}") {
                        return Some(Json::Object(members));
                    }
                    if !members.is_empty() && !self.word(",") {
                        return None;
                    }
                    self.spaces();
                    let name = self.text()?;
                    self.spaces();
                    if !self.word(":") {
                        return None;
                    }
                    members.push((name, self.value()?));
                }
            }
            _ => {
                let from = self.at;
                while matches!(self.chars.get(self.at), Some(c) if c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')) {
                    self.at += 1;
                }
                self.chars[from..self.at].iter().collect::<String>().parse().ok().map(Json::Number)
            }
        }
    }

    /// A text in quotes.
    fn text(&mut self) -> Option<String> {
        if self.chars.get(self.at) != Some(&'"') {
            return None;
        }
        self.at += 1;
        let mut text = String::new();
        loop {
            let c = *self.chars.get(self.at)?;
            self.at += 1;
            match c {
                '"' => return Some(text),
                '\\' => {
                    let escaped = *self.chars.get(self.at)?;
                    self.at += 1;
                    match escaped {
                        'n' => text.push('\n'),
                        't' => text.push('\t'),
                        'r' => text.push('\r'),
                        'b' => text.push('\u{8}'),
                        'f' => text.push('\u{c}'),
                        'u' => {
                            let mut code = self.four_hex()?;
                            // A character outside the first 65,536 is
                            // written as two: a high half and a low half.
                            if (0xD800..0xDC00).contains(&code) && self.word("\\u") {
                                let low = self.four_hex()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.checked_sub(0xDC00)?);
                            }
                            text.push(char::from_u32(code)?);
                        }
                        other => text.push(other), // `\"`, `\\`, `\/`
                    }
                }
                c => text.push(c),
            }
        }
    }

    fn four_hex(&mut self) -> Option<u32> {
        let digits: String = self.chars.get(self.at..self.at + 4)?.iter().collect();
        self.at += 4;
        u32::from_str_radix(&digits, 16).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn a_value_is_read_and_written_back() {
        let text = r#"{"jsonrpc":"2.0","id":1,"params":{"list":[1,2.5,-3,true,false,null],"text":"a \"b\"\n\\"}}"#;
        let value = Json::parse(text).expect("it is JSON");
        assert_eq!(value.write(), text);
        assert_eq!(value.get("id").and_then(Json::as_usize), Some(1));
        assert_eq!(value.at(&["params", "text"]).and_then(Json::as_str), Some("a \"b\"\n\\"));
        assert_eq!(value.at(&["params", "list"]).and_then(Json::as_list).map(<[Json]>::len), Some(6));
        assert_eq!(value.at(&["params", "nothing", "here"]), None);
    }

    #[test]
    fn spaces_and_escapes_are_read() {
        let value = Json::parse(" { \"a\" : [ ] , \"b\" : \"\\u0041\\ud83d\\ude00\\/\" } ").expect("it is JSON");
        assert_eq!(value, Json::object([("a", Json::List(Vec::new())), ("b", Json::text("A\u{1F600}/"))]));
        // What JSON cannot hold as it is, is written with a backslash.
        assert_eq!(Json::text("tab\there\u{1}").write(), "\"tab\\there\\u0001\"");
        assert_eq!(Json::number(42).write(), "42");
    }

    #[test]
    fn what_is_not_json_is_not_read() {
        for text in ["", "{", "[1,]x", "{\"a\" 1}", "\"open", "nul", "[1 2]", "{} {}"] {
            assert_eq!(Json::parse(text), None, "{text}");
        }
    }
}
