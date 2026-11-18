//! Values that live in another language: `import python`, `import node`,
//! `import java`, `import ruby`, `import php`, `import lua` and
//! `import csharp` (docs/python.md, docs/node.md, docs/java.md,
//! docs/ruby.md, docs/php.md, docs/lua.md, docs/csharp.md).
//!
//! Zeno starts that language's own interpreter beside the program, once,
//! the first time it is asked for, and the two talk over a socket of this
//! machine (for Lua, two named pipes): one line of JSON asks, one line
//! answers. `none`, a bool, an int, a float and a str go across as
//! themselves. Anything else stays
//! where it was made, and Zeno holds a `Foreign`: the number the other
//! side keeps it under.
//!
//! Nothing here knows a language but `Lang` and the helper it names
//! (`bridge.py`, `bridge.js`, `bridge.java`, `bridge.rb`, `bridge.php`,
//! `bridge.lua`, `bridge.cs`). Another language is another helper that
//! speaks the same lines.

use std::rc::Rc;

use crate::token::Span;
use crate::value::*;

type Result<T = Value> = std::result::Result<T, Control>;

/// The languages there is a helper for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Python,
    Node,
    Java,
    Ruby,
    Php,
    Lua,
    CSharp,
    C,
}

impl Lang {
    /// The language, as a message names it: "a Python value".
    pub fn name(self) -> &'static str {
        match self {
            Lang::Python => "Python",
            Lang::Node => "JavaScript",
            Lang::Java => "Java",
            Lang::Ruby => "Ruby",
            Lang::Php => "PHP",
            Lang::Lua => "Lua",
            Lang::CSharp => "C#",
            Lang::C => "C",
        }
    }

    /// The program that runs it, which is what is started and can be
    /// missing.
    pub fn runtime(self) -> &'static str {
        match self {
            Lang::Python => "Python",
            Lang::Node => "Node",
            Lang::Java => "Java",
            Lang::Ruby => "Ruby",
            Lang::Php => "PHP",
            Lang::Lua => "Lua",
            Lang::CSharp => ".NET",
            Lang::C => "C",
        }
    }
}

/// A value that stays in the other language. When the last holder of
/// one lets go, the other side is told with the next line it is sent,
/// and lets go too.
pub struct Foreign {
    pub lang: Lang,
    pub id: u64,
}

impl Drop for Foreign {
    fn drop(&mut self) {
        let_go(self.lang, self.id);
    }
}

// Everything below that the rest of Zeno calls is marked `#[cold]` and
// `#[inline(never)]`. The places that call it are the ones every
// program runs through: reading an item, comparing, letting go of a
// value. Left to itself the compiler folds this file into them, and
// then their code moves whenever this file changes: adding Java here
// made a sort on the VM a tenth slower, with no line of the VM touched
// (decision 29.11). Kept out, a program that asks for no other
// language runs the code it ran before.
#[cold]
#[inline(never)]
fn let_go(lang: Lang, id: u64) {
    link::freed(lang, id);
}

// ----------------------------------------------------------------- The lines

/// A value as a line holds it.
#[derive(Debug, PartialEq)]
enum Wire {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<Wire>),
    Object(Vec<(String, Wire)>),
}

/// How deep a list may hold lists on its way across. A list that holds
/// itself would otherwise never end.
const DEEPEST: usize = 200;

/// Write a Zeno value as the other side reads it.
fn write(lang: Lang, value: &Value, depth: usize, out: &mut String, span: Span) -> Result<()> {
    if depth > DEEPEST {
        return fatal(format!("this is nested more than {DEEPEST} deep, or holds itself, and cannot be given to {}", lang.name()), span);
    }
    match value {
        Value::None => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        // A number of JavaScript holds whole numbers exactly up to 2^53.
        // A bigger int goes as text, and arrives as a BigInt.
        Value::Int(n) if lang == Lang::Node && n.unsigned_abs() > (1 << 53) => out.push_str(&format!("[\"i\",\"{n}\"]")),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(x) if x.is_nan() => out.push_str("[\"f\",\"nan\"]"),
        Value::Float(x) if x.is_infinite() => out.push_str(if *x > 0.0 { "[\"f\",\"inf\"]" } else { "[\"f\",\"-inf\"]" }),
        // `{:?}` always writes a point or an exponent, so that a float
        // arrives as a float.
        Value::Float(x) => out.push_str(&format!("{x:?}")),
        Value::F32(x) => return write(lang, &Value::Float(f64::from(*x)), depth, out, span),
        Value::U64(n) if *n > i64::MAX as u64 => out.push_str(&format!("[\"u\",\"{n}\"]")),
        Value::U64(n) => out.push_str(&n.to_string()),
        Value::Str(text) => write_text(text, out),
        Value::List(items) => {
            out.push_str("[\"l\",[");
            for (at, item) in items.borrow().iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                write(lang, item, depth + 1, out, span)?;
            }
            out.push_str("]]");
        }
        Value::Dict(dict) => {
            out.push_str("[\"d\",[");
            for (at, (key, item)) in dict.borrow().entries().iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                out.push('[');
                write(lang, key, depth + 1, out, span)?;
                out.push(',');
                write(lang, item, depth + 1, out, span)?;
                out.push(']');
            }
            out.push_str("]]");
        }
        Value::Foreign(foreign) if foreign.lang == lang => out.push_str(&format!("[\"p\",{}]", foreign.id)),
        Value::Foreign(foreign) => {
            return fatal(format!("a {} value cannot be given to {}. Each stays where it was made", foreign.lang.name(), lang.name()), span);
        }
        Value::Function(_) | Value::Builtin(_) | Value::Method(..) | Value::BoundMethod(..) => {
            return fatal(format!("a function of Zeno cannot be given to {} yet", lang.name()), span);
        }
        other => return fatal(format!("{} cannot be given to {}", other.type_name(), lang.name()), span),
    }
    Ok(())
}

fn write_text(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Read a line of JSON. `None` if it is not one: the other side is not
/// the helper, or has stopped in the middle of a line.
fn parse(line: &str) -> Option<Wire> {
    let mut reader = Reader { bytes: line.as_bytes(), at: 0 };
    let value = reader.value(0)?;
    reader.blanks();
    (reader.at == reader.bytes.len()).then_some(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn blanks(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.at += 1;
        }
    }

    fn word(&mut self, word: &str, value: Wire) -> Option<Wire> {
        self.bytes[self.at..].starts_with(word.as_bytes()).then(|| {
            self.at += word.len();
            value
        })
    }

    fn value(&mut self, depth: usize) -> Option<Wire> {
        if depth > 4 * DEEPEST {
            return None;
        }
        self.blanks();
        match *self.bytes.get(self.at)? {
            b'n' => self.word("null", Wire::Null),
            b't' => self.word("true", Wire::Bool(true)),
            b'f' => self.word("false", Wire::Bool(false)),
            b'"' => self.text().map(Wire::Text),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.blanks();
                    if self.bytes.get(self.at) == Some(&b']') {
                        self.at += 1;
                        return Some(Wire::List(items));
                    }
                    if !items.is_empty() {
                        if self.bytes.get(self.at) != Some(&b',') {
                            return None;
                        }
                        self.at += 1;
                    }
                    items.push(self.value(depth + 1)?);
                }
            }
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                loop {
                    self.blanks();
                    if self.bytes.get(self.at) == Some(&b'}') {
                        self.at += 1;
                        return Some(Wire::Object(members));
                    }
                    if !members.is_empty() {
                        if self.bytes.get(self.at) != Some(&b',') {
                            return None;
                        }
                        self.at += 1;
                        self.blanks();
                    }
                    let name = self.text()?;
                    self.blanks();
                    if self.bytes.get(self.at) != Some(&b':') {
                        return None;
                    }
                    self.at += 1;
                    members.push((name, self.value(depth + 1)?));
                }
            }
            _ => self.number(),
        }
    }

    /// A whole number that fits is an int. Anything with a point or an
    /// exponent is a float.
    fn number(&mut self) -> Option<Wire> {
        let start = self.at;
        while matches!(self.bytes.get(self.at), Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
        if text.contains(['.', 'e', 'E']) {
            return text.parse().ok().map(Wire::Float);
        }
        // A whole number too big for an int is a float.
        match text.parse() {
            Ok(n) => Some(Wire::Int(n)),
            Err(_) => text.parse().ok().map(Wire::Float),
        }
    }

    fn hex(&mut self) -> Option<u32> {
        let digits = std::str::from_utf8(self.bytes.get(self.at..self.at + 4)?).ok()?;
        self.at += 4;
        u32::from_str_radix(digits, 16).ok()
    }

    fn text(&mut self) -> Option<String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return None;
        }
        self.at += 1;
        let mut text = Vec::new();
        loop {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            match byte {
                b'"' => return String::from_utf8(text).ok(),
                b'\\' => {
                    let escaped = *self.bytes.get(self.at)?;
                    self.at += 1;
                    let c = match escaped {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let first = self.hex()?;
                            // A character past U+FFFF is written as two
                            // halves. Half of one alone is not a
                            // character, and is shown as the mark for
                            // "not a character".
                            let code = match first {
                                0xD800..=0xDBFF if self.bytes[self.at..].starts_with(b"\\u") => {
                                    let before = self.at;
                                    self.at += 2;
                                    match self.hex()? {
                                        second @ 0xDC00..=0xDFFF => 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00),
                                        _ => {
                                            self.at = before;
                                            0xFFFD
                                        }
                                    }
                                }
                                _ => first,
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        other => other as char,
                    };
                    text.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
                byte => text.push(byte),
            }
        }
    }
}

/// A value the other side sent, as a value of Zeno.
fn read(lang: Lang, wire: Wire) -> Option<Value> {
    Some(match wire {
        Wire::Null => Value::None,
        Wire::Bool(b) => Value::Bool(b),
        Wire::Int(n) => Value::Int(n),
        Wire::Float(x) => Value::Float(x),
        Wire::Text(text) => Value::str(text),
        Wire::Object(_) => return None,
        Wire::List(mut tagged) => {
            if tagged.len() != 2 {
                return None;
            }
            let inside = tagged.pop()?;
            let Wire::Text(tag) = tagged.pop()? else { return None };
            match (tag.as_str(), inside) {
                ("p", Wire::Int(id)) => Value::Foreign(Rc::new(Foreign { lang, id: id as u64 })),
                ("f", Wire::Text(which)) => Value::Float(match which.as_str() {
                    "nan" => f64::NAN,
                    "inf" => f64::INFINITY,
                    "-inf" => f64::NEG_INFINITY,
                    other => other.parse().ok()?,
                }),
                ("i", Wire::Text(digits)) => Value::Int(digits.parse().ok()?),
                ("u", Wire::Text(digits)) => Value::U64(digits.parse().ok()?),
                ("l", Wire::List(items)) => {
                    Value::list(items.into_iter().map(|item| read(lang, item)).collect::<Option<Vec<Value>>>()?)
                }
                ("d", Wire::List(pairs)) => {
                    let mut dict = Dict::default();
                    for pair in pairs {
                        let Wire::List(mut pair) = pair else { return None };
                        if pair.len() != 2 {
                            return None;
                        }
                        let item = read(lang, pair.pop()?)?;
                        let key = Key::of(&read(lang, pair.pop()?)?)?;
                        dict.set(key, item);
                    }
                    Value::Dict(Rc::new(std::cell::RefCell::new(dict)))
                }
                _ => return None,
            }
        }
    })
}

// ------------------------------------------------------------ What Zeno asks

/// One thing asked of the other side: `{"op": "get", "on": ..., "name": ...}`.
struct Ask {
    lang: Lang,
    line: String,
    span: Span,
}

impl Ask {
    fn new(lang: Lang, op: &str, span: Span) -> Ask {
        let mut line = String::from("{\"op\":");
        write_text(op, &mut line);
        Ask { lang, line, span }
    }

    fn text(mut self, name: &str, text: &str) -> Ask {
        self.line.push_str(&format!(",\"{name}\":"));
        write_text(text, &mut self.line);
        self
    }

    fn value(mut self, name: &str, value: &Value) -> Result<Ask> {
        self.line.push_str(&format!(",\"{name}\":"));
        write(self.lang, value, 0, &mut self.line, self.span)?;
        Ok(self)
    }

    fn on(self, foreign: &Foreign) -> Ask {
        let id = foreign.id;
        let mut ask = self;
        ask.line.push_str(&format!(",\"on\":[\"p\",{id}]"));
        ask
    }

    fn send(self) -> Result {
        link::ask(self.lang, self.line, self.span)
    }
}

/// `python.module("json")`: the module, which stays in Python.
#[cold]
#[inline(never)]
pub fn module(lang: Lang, name: &str, span: Span) -> Result {
    Ask::new(lang, "import", span).text("name", name).send()
}

/// `node.global("JSON")`: something JavaScript has without asking for
/// a module.
#[cold]
#[inline(never)]
pub fn global(lang: Lang, name: &str, span: Span) -> Result {
    Ask::new(lang, "global", span).text("name", name).send()
}

/// `ruby.symbol("name")`: what `:name` is in Ruby.
#[cold]
#[inline(never)]
pub fn symbol(lang: Lang, name: &str, span: Span) -> Result {
    Ask::new(lang, "symbol", span).text("name", name).send()
}

/// What a language has by a name, of a kind Python has no word for:
/// `php.callable("strrev")` asks for `function`, `php.constant("M_PI")`
/// for `constant`.
#[cold]
#[inline(never)]
pub fn named(lang: Lang, op: &str, name: &str, span: Span) -> Result {
    Ask::new(lang, op, span).text("name", name).send()
}

/// `php.variable(value)`: a variable of PHP that holds the value, for
/// a function that changes what it is given.
#[cold]
#[inline(never)]
pub fn variable(lang: Lang, value: &Value, span: Span) -> Result {
    Ask::new(lang, "variable", span).value("value", value)?.send()
}

/// `java.jar("lib")`: a jar, or a folder of them, where Java then
/// finds classes.
#[cold]
#[inline(never)]
pub fn jar(lang: Lang, path: &str, span: Span) -> Result<()> {
    Ask::new(lang, "jar", span).text("name", path).send().map(|_| ())
}

/// `thing.name`
#[cold]
#[inline(never)]
pub fn field(foreign: &Foreign, name: &str, span: Span) -> Result {
    Ask::new(foreign.lang, "get", span).on(foreign).text("name", name).send()
}

/// `thing.name = value`
#[cold]
#[inline(never)]
pub fn set_field(foreign: &Foreign, name: &str, value: &Value, span: Span) -> Result<()> {
    Ask::new(foreign.lang, "set", span).on(foreign).text("name", name).value("to", value)?.send().map(|_| ())
}

/// `thing(1, 2, name=3)`
#[cold]
#[inline(never)]
pub fn call(foreign: &Foreign, positional: &[Value], named: &[(String, Value)], span: Span) -> Result {
    called("call", foreign, positional, named, span)
}

/// `node.new(Class, 1, 2)`: what `new Class(1, 2)` is in JavaScript.
#[cold]
#[inline(never)]
pub fn new(foreign: &Foreign, positional: &[Value], span: Span) -> Result {
    called("new", foreign, positional, &[], span)
}

fn called(op: &str, foreign: &Foreign, positional: &[Value], named: &[(String, Value)], span: Span) -> Result {
    let lang = foreign.lang;
    let mut ask = Ask::new(lang, op, span).on(foreign);
    ask.line.push_str(",\"args\":[");
    for (at, arg) in positional.iter().enumerate() {
        if at > 0 {
            ask.line.push(',');
        }
        write(lang, arg, 0, &mut ask.line, span)?;
    }
    ask.line.push_str("],\"named\":[");
    for (at, (name, arg)) in named.iter().enumerate() {
        if at > 0 {
            ask.line.push(',');
        }
        ask.line.push('[');
        write_text(name, &mut ask.line);
        ask.line.push(',');
        write(lang, arg, 0, &mut ask.line, span)?;
        ask.line.push(']');
    }
    ask.line.push(']');
    if lang != Lang::C {
        return ask.send();
    }
    // A list given to C for a pointer is the memory it points to. What
    // the function wrote there comes back with the answer, and the
    // list is made to hold it.
    let mut members = link::ask_members(lang, ask.line, span)?;
    if let Some(at) = members.iter().position(|(name, _)| name == "back") {
        let (_, back) = members.remove(at);
        let Wire::List(back) = back else { return fatal("C answered with a line Zeno cannot read".to_string(), span) };
        for pair in back {
            let Wire::List(mut pair) = pair else { continue };
            let (Some(items), Some(Wire::Int(index))) = (pair.pop(), pair.pop()) else { continue };
            let (Some(Value::List(list)), Some(Value::List(new))) = (positional.get(index as usize), read(lang, items)) else { continue };
            let new = std::mem::take(&mut *new.borrow_mut());
            let mut old = list.borrow_mut();
            // An f32 or a u64 stays one.
            let kept: Vec<Value> = new
                .into_iter()
                .enumerate()
                .map(|(at, value)| match (old.get(at), value) {
                    (Some(Value::F32(_)), Value::Float(x)) => Value::F32(x as f32),
                    (Some(Value::U64(_)), Value::Int(n)) if n >= 0 => Value::U64(n as u64),
                    (_, value) => value,
                })
                .collect();
            *old = kept;
        }
    }
    link::answer(lang, members, span)
}

/// `c.library("z", declarations)`: the stubs that call the declared
/// functions, already built (`bridge_c.rs`), loaded by the C helper.
#[cold]
#[inline(never)]
pub fn c_library(stubs: &str, label: &str, span: Span) -> Result {
    Ask::new(Lang::C, "import", span).text("name", stubs).text("label", label).send()
}

/// The folder of the program that is run, where that is a file.
pub fn home() -> Option<std::path::PathBuf> {
    link::home()
}

/// `thing[index]`
#[cold]
#[inline(never)]
pub fn item(foreign: &Foreign, index: &Value, span: Span) -> Result {
    Ask::new(foreign.lang, "item", span).on(foreign).value("at", index)?.send()
}

/// `thing[index] = value`
#[cold]
#[inline(never)]
pub fn set_item(foreign: &Foreign, index: &Value, value: &Value, span: Span) -> Result<()> {
    Ask::new(foreign.lang, "set_item", span).on(foreign).value("at", index)?.value("to", value)?.send().map(|_| ())
}

/// `len(thing)`
#[cold]
#[inline(never)]
pub fn len(foreign: &Foreign, span: Span) -> Result {
    Ask::new(foreign.lang, "len", span).on(foreign).send()
}

/// `item in thing`
#[cold]
#[inline(never)]
pub fn has(foreign: &Foreign, item: &Value, span: Span) -> Result<bool> {
    match Ask::new(foreign.lang, "has", span).on(foreign).value("item", item)?.send()? {
        Value::Bool(found) => Ok(found),
        _ => Ok(false),
    }
}

/// What a `for` loop steps through: every item, taken now.
#[cold]
#[inline(never)]
pub fn items(foreign: &Foreign, span: Span) -> Result<Vec<Value>> {
    match Ask::new(foreign.lang, "items", span).on(foreign).send()? {
        Value::List(items) => Ok(items.borrow().clone()),
        _ => Ok(Vec::new()),
    }
}

/// `python.copy(thing)`: a list, a tuple or a dict of the other side as
/// a list or a dict of Zeno, item by item.
#[cold]
#[inline(never)]
pub fn copy(foreign: &Foreign, span: Span) -> Result {
    Ask::new(foreign.lang, "copy", span).on(foreign).send()
}

/// How `print` shows it (`repr` false), and how it is shown inside a
/// list (`repr` true). Showing a value cannot fail, so if the other
/// side cannot say, this says what kind of thing it is.
#[cold]
#[inline(never)]
pub fn shown(foreign: &Foreign, repr: bool) -> String {
    let op = if repr { "repr" } else { "str" };
    match Ask::new(foreign.lang, op, Span { line: 0, col: 0 }).on(foreign).send() {
        Ok(Value::Str(text)) => text.to_string(),
        _ => format!("<{} value>", foreign.lang.name()),
    }
}

/// `==`, asked of the other side when at least one of the two is there.
#[cold]
#[inline(never)]
pub fn equal(foreign: &Foreign, other: &Value) -> bool {
    if let Value::Foreign(other) = other {
        if other.lang == foreign.lang && other.id == foreign.id {
            return true;
        }
    }
    let Ok(ask) = Ask::new(foreign.lang, "equal", Span { line: 0, col: 0 }).on(foreign).value("to", other) else {
        return false;
    };
    matches!(ask.send(), Ok(Value::Bool(true)))
}

/// The other language's interpreter is told that the program is over,
/// and waited for, so that its last lines are on the screen before
/// Zeno's own end.
pub fn close() {
    link::close();
}

/// Where the program that is run is. A module of the other language is
/// looked for from there, as it would be for a program of that language
/// in the same folder.
pub fn program_at(path: &str) {
    link::program_at(path);
}

// --------------------------------------------------------------- The socket

#[cfg(not(target_arch = "wasm32"))]
mod link {
    use std::cell::RefCell;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{parse, read, Lang, Result, Wire};
    use crate::token::Span;
    use crate::value::{fatal, raise};

    const PYTHON: &str = include_str!("bridge.py");
    const NODE: &str = include_str!("bridge.js");
    const JAVA: &str = include_str!("bridge.java");
    const RUBY: &str = include_str!("bridge.rb");
    const PHP: &str = include_str!("bridge.php");
    const LUA: &str = include_str!("bridge.lua");
    const CSHARP: &str = include_str!("bridge.cs");

    /// How the helper of a language is handed to its program: the
    /// option that says "the program is the next word", and the helper.
    /// Java takes no program as a word, so its helper is a file
    /// (`java_helper`).
    fn helper(lang: Lang, program: &str) -> Vec<String> {
        match lang {
            Lang::Python => vec!["-c".to_string(), PYTHON.to_string()],
            Lang::Node => vec!["-e".to_string(), NODE.to_string()],
            Lang::Java => java_helper(program),
            Lang::Ruby => vec!["-e".to_string(), RUBY.to_string()],
            // `-r` takes the program without the `<?php` a file begins with.
            Lang::Php => vec!["-r".to_string(), PHP.trim_start_matches("<?php").to_string()],
            // Lua takes no program that begins with `-`, as a comment does.
            Lang::Lua => vec!["-e".to_string(), format!(" {LUA}")],
            Lang::CSharp => csharp_helper(program),
            // The helper is a program of its own (`programs`).
            Lang::C => Vec::new(),
        }
    }

    /// The words that start the Java helper.
    ///
    /// Java runs classes, which are made from the source by `javac`.
    /// That takes about a second, so it is done once: the classes are
    /// kept in the folder this machine has for what can be made again
    /// (`registry::cache`), under a number worked out from the source,
    /// and every later run starts them as they are. Where there is no
    /// `javac`, or no such folder, Java is given the source itself,
    /// which it compiles each time it is started (Java 11 and later).
    fn java_helper(program: &str) -> Vec<String> {
        let mut number: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in JAVA.bytes() {
            number = (number ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let name = format!("java-bridge-{number:016x}");
        let (base, shared) = match crate::registry::cache() {
            Ok(folder) => (folder, true),
            // A folder nobody else can have made, used this once and
            // taken away when the program is over.
            Err(_) => {
                let folder = std::env::temp_dir().join(format!("zeno-{}-{}", std::process::id(), secret()));
                SCRAPS.with(|scraps| scraps.borrow_mut().push(folder.clone()));
                (folder, false)
            }
        };
        let classes = base.join(&name);
        let run = |folder: &std::path::Path| vec!["-cp".to_string(), folder.to_string_lossy().into_owned(), "ZenoBridge".to_string()];
        if shared && classes.join("ZenoBridge.class").is_file() {
            return run(&classes);
        }
        // The source has a place of its own, the same for every run.
        let source = base.join(format!("{name}-source")).join("ZenoBridge.java");
        let written = std::fs::read(&source).is_ok_and(|there| there == JAVA.as_bytes())
            || (source.parent().is_some_and(|folder| std::fs::create_dir_all(folder).is_ok()) && std::fs::write(&source, JAVA).is_ok());
        let as_source = vec![source.to_string_lossy().into_owned()];
        if !written {
            return as_source;
        }
        // The classes are made in a folder of this run's own, and given
        // their name when they are whole, so that two runs at once
        // never share half of them.
        let making = base.join(format!("{name}.{}", std::process::id()));
        // The `javac` that came with this `java`.
        let javac = match std::path::Path::new(program).parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder.join("javac").to_string_lossy().into_owned(),
            _ => "javac".to_string(),
        };
        let compiled = std::fs::create_dir_all(&making).is_ok()
            && Command::new(javac)
                .args(["-nowarn", "--release", "11", "-encoding", "UTF-8", "-d"])
                .arg(&making)
                .arg(&source)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
        if !compiled {
            let _ = std::fs::remove_dir_all(&making);
            return as_source;
        }
        if std::fs::rename(&making, &classes).is_ok() {
            return run(&classes);
        }
        // Another run got there first, and its classes are the same.
        if classes.join("ZenoBridge.class").is_file() {
            let _ = std::fs::remove_dir_all(&making);
            return run(&classes);
        }
        run(&making)
    }

    /// The words that start the .NET helper.
    ///
    /// .NET runs a program that `dotnet build` made from a project, and
    /// that takes some seconds, so it is done once, as for Java: the
    /// project and what is built from it are kept in the folder this
    /// machine has for what can be made again (`registry::cache`), under
    /// a number worked out from the source, and every later run starts
    /// what was built. It is built for the .NET that builds it, and runs
    /// on that one and every later one.
    fn csharp_helper(program: &str) -> Vec<String> {
        let project = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    \
                       <TargetFramework>TARGET</TargetFramework>\n    <RollForward>Major</RollForward>\n    \
                       <AssemblyName>ZenoBridge</AssemblyName>\n    <RootNamespace>Zeno</RootNamespace>\n    \
                       <ImplicitUsings>disable</ImplicitUsings>\n    <Nullable>disable</Nullable>\n    \
                       <SatelliteResourceLanguages>en</SatelliteResourceLanguages>\n    \
                       <GenerateDocumentationFile>false</GenerateDocumentationFile>\n    <NoWarn>$(NoWarn);CS8632</NoWarn>\n  \
                       </PropertyGroup>\n</Project>\n";
        let mut number: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in CSHARP.bytes().chain(project.bytes()) {
            number = (number ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let name = format!("csharp-bridge-{number:016x}");
        let base = match crate::registry::cache() {
            Ok(folder) => folder,
            Err(_) => {
                let folder = std::env::temp_dir().join(format!("zeno-{}-{}", std::process::id(), secret()));
                SCRAPS.with(|scraps| scraps.borrow_mut().push(folder.clone()));
                folder
            }
        };
        let built = base.join(&name);
        let run = |folder: &std::path::Path| vec![folder.join("ZenoBridge.dll").to_string_lossy().into_owned()];
        if built.join("ZenoBridge.dll").is_file() {
            return run(&built);
        }
        let quiet = |command: &mut Command| {
            command
                .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
                .env("DOTNET_NOLOGO", "1")
                .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
                .stdin(Stdio::null());
        };
        // The version of the SDK says which .NET to build for: 8.0.100
        // builds for `net8.0`.
        let mut asked = Command::new(program);
        asked.arg("--version");
        quiet(&mut asked);
        let major = asked
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|said| said.status.success())
            .and_then(|said| String::from_utf8_lossy(&said.stdout).trim().split('.').next().and_then(|major| major.parse::<u32>().ok()))
            .unwrap_or(8)
            .max(6);
        // `dotnet build` keeps what it is in the middle of in the folder
        // of the project, so each build has a folder of its own: two
        // runs at once, or two tests, never build in one. It goes when
        // the program is over.
        let unique = secret();
        let source = base.join(format!("{name}-source-{unique}"));
        SCRAPS.with(|scraps| scraps.borrow_mut().push(source.clone()));
        let written = std::fs::create_dir_all(&source).is_ok()
            && std::fs::write(source.join("ZenoBridge.csproj"), project.replace("TARGET", &format!("net{major}.0"))).is_ok()
            && std::fs::write(source.join("Bridge.cs"), CSHARP).is_ok();
        // Where it cannot be built, `dotnet run` builds it and says why
        // it could not: no SDK, which is what builds.
        let as_source = vec!["run".to_string(), "--project".to_string(), source.to_string_lossy().into_owned()];
        if !written {
            return as_source;
        }
        let making = base.join(format!("{name}.{unique}"));
        // No server of the build is left running after it.
        let mut build = Command::new(program);
        build.arg("build").arg(source.join("ZenoBridge.csproj")).args(["-c", "Release", "--nologo", "-v", "q", "-nodeReuse:false", "-p:UseSharedCompilation=false", "-o"]).arg(&making);
        quiet(&mut build);
        let compiled = build.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
            && making.join("ZenoBridge.dll").is_file();
        if !compiled {
            let _ = std::fs::remove_dir_all(&making);
            return as_source;
        }
        let _ = std::fs::remove_dir_all(&source);
        if std::fs::rename(&making, &built).is_ok() {
            return run(&built);
        }
        if built.join("ZenoBridge.dll").is_file() {
            let _ = std::fs::remove_dir_all(&making);
            return run(&built);
        }
        run(&making)
    }

    /// The variable of the environment that names the program to use.
    fn named_by(lang: Lang) -> &'static str {
        match lang {
            Lang::Python => "ZENO_PYTHON",
            Lang::Node => "ZENO_NODE",
            Lang::Java => "ZENO_JAVA",
            Lang::Ruby => "ZENO_RUBY",
            Lang::Php => "ZENO_PHP",
            Lang::Lua => "ZENO_LUA",
            Lang::CSharp => "ZENO_DOTNET",
            Lang::C => "CC",
        }
    }

    /// How long the helper has to start and say hello.
    const PATIENCE: Duration = Duration::from_secs(30);

    struct Link {
        child: Child,
        reader: BufReader<Box<dyn Read>>,
        writer: Box<dyn Write>,
        /// The socket the two ends are, where they are one.
        socket: Option<TcpStream>,
    }

    thread_local! {
        /// The helper of each language that has been asked for.
        static LINKS: RefCell<Vec<(Lang, Link)>> = const { RefCell::new(Vec::new()) };
        /// What Zeno has let go of since the last line was sent.
        static FREED: RefCell<Vec<(Lang, u64)>> = const { RefCell::new(Vec::new()) };
        /// Folders made for this run only.
        static SCRAPS: RefCell<Vec<std::path::PathBuf>> = const { RefCell::new(Vec::new()) };
    }

    /// The folder of the program that is run, where that is a file. It
    /// is said before the program starts, on another thread than the
    /// one the program runs on.
    static HOME: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

    pub fn home() -> Option<std::path::PathBuf> {
        HOME.lock().ok().and_then(|home| home.clone())
    }

    pub fn program_at(path: &str) {
        let folder = std::fs::canonicalize(path).ok().and_then(|file| file.parent().map(|folder| crate::bridge_c::plain(folder.to_path_buf())));
        if let Ok(mut home) = HOME.lock() {
            *home = folder;
        }
    }

    pub fn freed(lang: Lang, id: u64) {
        // When the program is over these are gone already, and so is
        // the helper.
        let _ = FREED.try_with(|freed| {
            if let Ok(mut freed) = freed.try_borrow_mut() {
                freed.push((lang, id));
            }
        });
    }

    pub fn close() {
        let links = LINKS.with(|links| std::mem::take(&mut *links.borrow_mut()));
        for (_, link) in links {
            let Link { mut child, reader, writer, socket } = link;
            // The end of the lines is how the helper knows.
            if let Some(socket) = socket {
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
            drop(writer);
            drop(reader);
            let _ = child.wait();
        }
        for folder in SCRAPS.with(|scraps| std::mem::take(&mut *scraps.borrow_mut())) {
            let _ = std::fs::remove_dir_all(folder);
        }
    }

    /// The programs to try, in order. `ZENO_PYTHON` names one, for a
    /// Python that is not the first one found, and `ZENO_NODE` a Node.
    fn programs(lang: Lang) -> Vec<String> {
        match (std::env::var(named_by(lang)), lang) {
            (Ok(named), _) if !named.is_empty() => vec![named],
            (_, Lang::Python) if cfg!(windows) => vec!["python".to_string(), "python3".to_string(), "py".to_string()],
            (_, Lang::Python) => vec!["python3".to_string(), "python".to_string()],
            (_, Lang::Node) => vec!["node".to_string()],
            (_, Lang::Ruby) => vec!["ruby".to_string()],
            (_, Lang::Php) => vec!["php".to_string()],
            // Some systems name Lua by its version only.
            (_, Lang::Lua) => vec!["lua".to_string(), "lua5.4".to_string(), "luajit".to_string()],
            // `DOTNET_ROOT` is where .NET is when it is not on the path,
            // and `~/.dotnet` is where Microsoft's script puts it.
            // The helper, built by the C compiler. Where it cannot be,
            // there is nothing to start, and `start` says why.
            (_, Lang::C) => crate::bridge_c::helper().into_iter().collect(),
            (_, Lang::CSharp) => {
                let file = if cfg!(windows) { "dotnet.exe" } else { "dotnet" };
                let mut names = vec!["dotnet".to_string()];
                for folder in [std::env::var_os("DOTNET_ROOT"), std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|home| std::path::Path::new(&home).join(".dotnet").into_os_string())]
                    .into_iter()
                    .flatten()
                    .filter(|folder| !folder.is_empty())
                {
                    let path = std::path::Path::new(&folder).join(file);
                    if path.is_file() {
                        names.push(path.to_string_lossy().into_owned());
                    }
                }
                names
            }
            // `JAVA_HOME` is where the tools of Java look for it.
            (_, Lang::Java) => {
                let mut names = vec!["java".to_string()];
                if let Some(home) = std::env::var_os("JAVA_HOME").filter(|home| !home.is_empty()) {
                    names.push(std::path::Path::new(&home).join("bin").join("java").to_string_lossy().into_owned());
                }
                names
            }
        }
    }

    /// A word nobody else on this machine can guess, so that only the
    /// helper Zeno started is listened to.
    fn secret() -> String {
        use std::hash::{BuildHasher, Hasher};
        let mut word = String::new();
        for _ in 0..2 {
            // Each `RandomState` is seeded by the system.
            word += &format!("{:016x}", std::collections::hash_map::RandomState::new().build_hasher().finish());
        }
        word
    }

    /// The first number a new helper keeps a value under. A helper that
    /// is started after another has stopped begins far above it, so
    /// that a value the old one kept is never taken for one of the new.
    fn first_number() -> u64 {
        thread_local! {
            static STARTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        }
        STARTED.with(|started| {
            started.set(started.get() + 1);
            (started.get() - 1) << 40
        })
    }

    /// On Windows `python` can be a stand-in that only says where to get
    /// Python, and elsewhere `python` can be Python 2. Such a name is
    /// asked first, quietly. `python3` is what it says it is.
    fn is_python_3(program: &str) -> bool {
        if !cfg!(windows) && program != "python" {
            return true;
        }
        Command::new(program)
            .args(["-c", "import sys; sys.exit(0 if sys.version_info >= (3, 6) else 1)"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn start(lang: Lang, span: Span) -> Result<Link> {
        let cannot = |why: String| raise("FileError", format!("cannot start {}: {why}", lang.runtime()), span);
        // Lua comes with no sockets, and opens a named pipe as a file.
        // Windows has no named pipes that a file can be, and there, or
        // where no pipe can be made, Lua needs LuaSocket.
        let piped = lang == Lang::Lua && cfg!(unix);
        // C has no interpreter to start: the helper is built first.
        if lang == Lang::C {
            if let Err(why) = crate::bridge_c::helper() {
                return raise("FileError", why, span);
            }
        }
        let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
            return cannot("this machine gave no socket to talk to it over".to_string());
        };
        let (Ok(address), Ok(())) = (listener.local_addr(), listener.set_nonblocking(true)) else {
            return cannot("this machine gave no socket to talk to it over".to_string());
        };
        let token = secret();
        let programs = programs(lang);
        for program in &programs {
            if lang == Lang::Python && !is_python_3(program) {
                continue;
            }
            let pipes = if piped { Pipes::make() } else { None };
            let place = match &pipes {
                Some(pipes) => format!("pipe {}|{}", pipes.to_zeno.display(), pipes.from_zeno.display()),
                None => address.port().to_string(),
            };
            let mut command = Command::new(program);
            command.args(helper(lang, program)).env("ZENO_BRIDGE", format!("{place} {token} {}", first_number()));
            // Never the folder of a Zeno that started this one.
            match HOME.lock().ok().and_then(|home| home.clone()) {
                Some(folder) => command.env("ZENO_PROGRAM", folder),
                None => command.env_remove("ZENO_PROGRAM"),
            };
            let started = command.spawn();
            let mut child = match started {
                Ok(child) => child,
                Err(_) => continue,
            };
            if let Some(pipes) = pipes {
                match pipes.open(&mut child, &token) {
                    Opened::Linked(reader, writer) => return Ok(Link { child, reader, writer, socket: None }),
                    Opened::Gone => continue,
                    Opened::Silent => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return cannot(format!("`{program}` did not answer"));
                    }
                }
            }
            // Wait for it to connect and say the word. A program that
            // ends first was not Python (or not one that works), and
            // the next is tried.
            let began = Instant::now();
            loop {
                if let Ok((stream, _)) = listener.accept() {
                    if let Some(socket) = greeted(stream, &token) {
                        if let (Ok(reading), Ok(writing)) = (socket.try_clone(), socket.try_clone()) {
                            let reader = BufReader::new(Box::new(reading) as Box<dyn Read>);
                            return Ok(Link { child, reader, writer: Box::new(writing), socket: Some(socket) });
                        }
                    }
                    continue;
                }
                if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
                    break;
                }
                if began.elapsed() > PATIENCE {
                    let _ = child.kill();
                    let _ = child.wait();
                    return cannot(format!("`{program}` did not answer"));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let variable = named_by(lang);
        match std::env::var(variable) {
            Ok(named) if !named.is_empty() => {
                raise("FileError", format!("there is no {} at `{named}`, which `{variable}` names", lang.runtime()), span)
            }
            _ => raise(
                "FileError",
                format!(
                    "there is no {} to run. Zeno looked for `{}`. `{variable}` can name one that is somewhere else",
                    lang.runtime(),
                    programs.join("`, then `")
                ),
                span,
            ),
        }
    }

    /// Is this the first line of the helper, with the word it was given?
    fn says_hello(line: &str, token: &str) -> bool {
        let Some(Wire::Object(members)) = parse(line) else { return false };
        let said = members.iter().find(|(name, _)| name == "hello").map(|(_, value)| value);
        said == Some(&Wire::Text(token.to_string()))
    }

    /// The socket a helper connected over, once it said the word.
    fn greeted(stream: TcpStream, token: &str) -> Option<TcpStream> {
        stream.set_nonblocking(false).ok()?;
        stream.set_nodelay(true).ok()?;
        stream.set_read_timeout(Some(PATIENCE)).ok()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        if !says_hello(&line, token) {
            return None;
        }
        stream.set_read_timeout(None).ok()?;
        Some(stream)
    }

    /// Two named pipes in a folder made for them: one the helper
    /// writes to Zeno, one Zeno writes to the helper. They are files
    /// only until both ends have them open.
    struct Pipes {
        folder: std::path::PathBuf,
        to_zeno: std::path::PathBuf,
        from_zeno: std::path::PathBuf,
    }

    enum Opened {
        Linked(BufReader<Box<dyn Read>>, Box<dyn Write>),
        /// The helper ended before it opened both, or said no word.
        Gone,
        /// It is still there and has opened nothing.
        Silent,
    }

    /// The folder goes with them, whatever happened.
    impl Drop for Pipes {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }

    impl Pipes {
        fn make() -> Option<Pipes> {
            thread_local! {
                static MADE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
            }
            let number = MADE.with(|made| {
                made.set(made.get() + 1);
                made.get()
            });
            let folder = std::env::temp_dir().join(format!("zeno-{}-{number}-{}", std::process::id(), &secret()[..8]));
            std::fs::create_dir(&folder).ok()?;
            let pipes = Pipes { to_zeno: folder.join("to-zeno"), from_zeno: folder.join("from-zeno"), folder };
            // Only this user may open them.
            let made = Command::new("mkfifo")
                .arg("-m")
                .arg("600")
                .arg(&pipes.to_zeno)
                .arg(&pipes.from_zeno)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            // Made or not, a `Pipes` that is dropped takes its folder.
            made.then_some(pipes)
        }

        /// Open Zeno's ends, which waits for the helper to open its
        /// own: the pipe to Zeno first, then the other, as the helper
        /// does. The opening is done on a thread of its own, so that
        /// a helper that ends first is seen.
        fn open(self, child: &mut Child, token: &str) -> Opened {
            let (to_zeno, from_zeno) = (self.to_zeno.clone(), self.from_zeno.clone());
            let (told, waiting) = std::sync::mpsc::channel();
            let opener = std::thread::spawn(move || {
                let reading = std::fs::File::open(&to_zeno);
                let writing = std::fs::OpenOptions::new().write(true).open(&from_zeno);
                let _ = told.send((reading, writing));
            });
            let began = Instant::now();
            let opened = loop {
                if let Ok(opened) = waiting.recv_timeout(Duration::from_millis(2)) {
                    break Some(opened);
                }
                let ended = matches!(child.try_wait(), Ok(Some(_)) | Err(_));
                if ended || began.elapsed() > PATIENCE {
                    // Open the helper's ends here, which lets the
                    // thread's opens finish.
                    let _ = std::fs::OpenOptions::new().write(true).open(&self.to_zeno);
                    let _ = std::fs::File::open(&self.from_zeno);
                    let _ = opener.join();
                    return if ended { Opened::Gone } else { Opened::Silent };
                }
            };
            let _ = opener.join();
            // Both ends are open, and the names are not needed.
            drop(self);
            let Some((Ok(reading), Ok(writing))) = opened else { return Opened::Gone };
            let mut reader = BufReader::new(Box::new(reading) as Box<dyn Read>);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || !says_hello(&line, token) {
                return Opened::Gone;
            }
            Opened::Linked(reader, Box::new(writing))
        }
    }

    /// Send one line and read the answer.
    pub fn ask(lang: Lang, line: String, span: Span) -> Result {
        let members = ask_members(lang, line, span)?;
        answer(lang, members, span)
    }

    /// What the answer holds: the value, or the error.
    pub(super) fn answer(lang: Lang, mut members: Vec<(String, Wire)>, span: Span) -> Result {
        let wrong = || fatal(format!("{} answered with a line Zeno cannot read", lang.runtime()), span);
        let Some((name, value)) = members.pop() else { return wrong() };
        match (name.as_str(), value) {
            ("ok", value) => match read(lang, value) {
                Some(value) => Ok(value),
                None => wrong(),
            },
            ("err", Wire::List(mut said)) if said.len() == 2 => match (said.pop(), said.pop()) {
                (Some(Wire::Text(message)), Some(Wire::Text(kind))) => raise(&kind, message, span),
                _ => wrong(),
            },
            _ => wrong(),
        }
    }

    /// Send the line, and read the line that answers it.
    pub(super) fn ask_members(lang: Lang, mut line: String, span: Span) -> Result<Vec<(String, Wire)>> {
        // What Zeno printed is on the screen before the other side prints.
        let _ = std::io::stdout().flush();
        let freed: Vec<u64> = FREED.with(|freed| {
            let mut freed = freed.borrow_mut();
            let (gone, kept) = std::mem::take(&mut *freed).into_iter().partition(|(of, _)| *of == lang);
            *freed = kept;
            gone.into_iter().map(|(_, id)| id).collect::<Vec<u64>>()
        });
        if !freed.is_empty() {
            let ids: Vec<String> = freed.iter().map(u64::to_string).collect();
            line.push_str(&format!(",\"free\":[{}]", ids.join(",")));
        }
        line.push_str("}\n");

        let answer = LINKS.with(|links| -> Result<String> {
            let mut links = links.borrow_mut();
            if !links.iter().any(|(of, _)| *of == lang) {
                let link = start(lang, span)?;
                links.push((lang, link));
            }
            let at = links.iter().position(|(of, _)| *of == lang).unwrap_or(0);
            let link = &mut links[at].1;
            let mut answer = String::new();
            let sent = link.writer.write_all(line.as_bytes());
            if sent.is_err() || !matches!(link.reader.read_line(&mut answer), Ok(n) if n > 0) {
                // It is gone: it was stopped, or it crashed. The next
                // thing asked starts a new one, which knows nothing of
                // what the old one kept.
                let (_, mut gone) = links.remove(at);
                let _ = gone.child.wait();
                return fatal(format!("{} stopped while Zeno was waiting for its answer", lang.runtime()), span);
            }
            Ok(answer)
        })?;

        match parse(&answer) {
            Some(Wire::Object(members)) => Ok(members),
            _ => fatal(format!("{} answered with a line Zeno cannot read", lang.runtime()), span),
        }
    }
}

/// A page is inside a browser, which starts no programs.
#[cfg(target_arch = "wasm32")]
mod link {
    use super::{Lang, Result};
    use crate::token::Span;
    use crate::value::raise;

    pub fn freed(_: Lang, _: u64) {}

    pub fn home() -> Option<std::path::PathBuf> {
        None
    }

    pub(super) fn ask_members(lang: Lang, _: String, span: Span) -> Result<Vec<(String, super::Wire)>> {
        raise("FileError", format!("cannot start {}: a page cannot start a program", lang.runtime()), span)
    }

    pub(super) fn answer(lang: Lang, _: Vec<(String, super::Wire)>, span: Span) -> Result {
        raise("FileError", format!("cannot start {}: a page cannot start a program", lang.runtime()), span)
    }

    pub fn close() {}

    pub fn program_at(_: &str) {}

    pub fn ask(lang: Lang, _: String, span: Span) -> Result {
        raise("FileError", format!("cannot start {}: a page cannot start a program", lang.runtime()), span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_read_back_as_it_was_written() {
        let line = r#"{"ok": ["l", [1, -2.5, 1e21, "a\"b\\ é 😀 \ud83d", null, true, ["d", [["k", ["p", 7]], [3, ["f", "-inf"]]]]]]}"#;
        let Some(Wire::Object(mut members)) = parse(line) else { panic!("the line is JSON") };
        let value = read(Lang::Python, members.pop().expect("one member").1).expect("a value");
        let Value::List(items) = &value else { panic!("a list") };
        let items = items.borrow();
        assert!(matches!(items[0], Value::Int(1)));
        assert!(matches!(items[1], Value::Float(x) if x == -2.5));
        assert!(matches!(items[2], Value::Float(x) if x == 1e21));
        assert_eq!(items[3].to_string(), "a\"b\\ \u{e9} \u{1F600} \u{FFFD}");
        assert!(matches!(items[4], Value::None));
        let Value::Dict(dict) = &items[6] else { panic!("a dict") };
        let dict = dict.borrow();
        assert!(matches!(dict.get(&Key::Str(Rc::from("k"))), Some(Value::Foreign(foreign)) if foreign.id == 7));
        assert!(matches!(dict.get(&Key::Int(3)), Some(Value::Float(x)) if *x == f64::NEG_INFINITY));

        // What is not a line of the helper is not read as one.
        for bad in ["", "{", "[1, 2", "{\"ok\": [1, 2, 3]}x", "nul"] {
            assert!(parse(bad).is_none() || bad.is_empty(), "{bad}");
        }
        assert!(read(Lang::Python, Wire::List(vec![Wire::Int(1), Wire::Int(2)])).is_none());
    }

    #[test]
    fn a_value_is_written_as_the_other_side_reads_it() {
        let mut dict = Dict::default();
        dict.set(Key::Str(Rc::from("a b")), Value::Float(2.0));
        dict.set(Key::Int(4), Value::None);
        let value = Value::list(vec![
            Value::Int(-3),
            Value::Float(1.0),
            Value::Float(f64::NAN),
            Value::str("tab\there \"q\" \u{e9}\n"),
            Value::Bool(false),
            Value::Dict(Rc::new(std::cell::RefCell::new(dict))),
        ]);
        let mut line = String::new();
        assert!(write(Lang::Python, &value, 0, &mut line, Span { line: 0, col: 0 }).is_ok());
        assert_eq!(
            line,
            "[\"l\",[-3,1.0,[\"f\",\"nan\"],\"tab\\u0009here \\\"q\\\" \u{e9}\\n\",false,[\"d\",[[\"a b\",2.0],[4,null]]]]]"
        );
        // And it is JSON, which reads back as the same value.
        let back = read(Lang::Python, parse(&line).expect("JSON")).expect("a value");
        assert_eq!(back.to_string(), value.to_string());

        // A list that holds itself has no end to write.
        let holds_itself = Value::list(Vec::new());
        if let Value::List(items) = &holds_itself {
            items.borrow_mut().push(holds_itself.clone());
        }
        let mut line = String::new();
        assert!(write(Lang::Python, &holds_itself, 0, &mut line, Span { line: 0, col: 0 }).is_err());
        if let Value::List(items) = &holds_itself {
            items.borrow_mut().clear();
        }
    }
}
