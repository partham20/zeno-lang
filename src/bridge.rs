//! Values that live in another language: `import python` (docs/python.md).
//!
//! Zeno starts that language's own interpreter beside the program, once,
//! the first time it is asked for, and the two talk over a socket of this
//! machine: one line of JSON asks, one line answers. `none`, a bool, an
//! int, a float and a str go across as themselves. Anything else stays
//! where it was made, and Zeno holds a `Foreign`: the number the other
//! side keeps it under.
//!
//! Nothing here knows Python but `Lang` and the helper it names
//! (`bridge.py`). Another language is another helper that speaks the
//! same lines.

use std::rc::Rc;

use crate::token::Span;
use crate::value::*;

type Result<T = Value> = std::result::Result<T, Control>;

/// The languages there is a helper for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Python,
}

impl Lang {
    /// As a message names it: "a Python value".
    pub fn name(self) -> &'static str {
        match self {
            Lang::Python => "Python",
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
        link::freed(self.lang, self.id);
    }
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
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(x) if x.is_nan() => out.push_str("[\"f\",\"nan\"]"),
        Value::Float(x) if x.is_infinite() => out.push_str(if *x > 0.0 { "[\"f\",\"inf\"]" } else { "[\"f\",\"-inf\"]" }),
        // `{:?}` always writes a point or an exponent, so that a float
        // arrives as a float.
        Value::Float(x) => out.push_str(&format!("{x:?}")),
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
        text.parse().ok().map(Wire::Int)
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
                    _ => return None,
                }),
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
pub fn module(lang: Lang, name: &str, span: Span) -> Result {
    Ask::new(lang, "import", span).text("name", name).send()
}

/// `thing.name`
pub fn field(foreign: &Foreign, name: &str, span: Span) -> Result {
    Ask::new(foreign.lang, "get", span).on(foreign).text("name", name).send()
}

/// `thing.name = value`
pub fn set_field(foreign: &Foreign, name: &str, value: &Value, span: Span) -> Result<()> {
    Ask::new(foreign.lang, "set", span).on(foreign).text("name", name).value("to", value)?.send().map(|_| ())
}

/// `thing(1, 2, name=3)`
pub fn call(foreign: &Foreign, positional: &[Value], named: &[(String, Value)], span: Span) -> Result {
    let lang = foreign.lang;
    let mut ask = Ask::new(lang, "call", span).on(foreign);
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
    ask.send()
}

/// `thing[index]`
pub fn item(foreign: &Foreign, index: &Value, span: Span) -> Result {
    Ask::new(foreign.lang, "item", span).on(foreign).value("at", index)?.send()
}

/// `thing[index] = value`
pub fn set_item(foreign: &Foreign, index: &Value, value: &Value, span: Span) -> Result<()> {
    Ask::new(foreign.lang, "set_item", span).on(foreign).value("at", index)?.value("to", value)?.send().map(|_| ())
}

/// `len(thing)`
pub fn len(foreign: &Foreign, span: Span) -> Result {
    Ask::new(foreign.lang, "len", span).on(foreign).send()
}

/// `item in thing`
pub fn has(foreign: &Foreign, item: &Value, span: Span) -> Result<bool> {
    match Ask::new(foreign.lang, "has", span).on(foreign).value("item", item)?.send()? {
        Value::Bool(found) => Ok(found),
        _ => Ok(false),
    }
}

/// What a `for` loop steps through: every item, taken now.
pub fn items(foreign: &Foreign, span: Span) -> Result<Vec<Value>> {
    match Ask::new(foreign.lang, "items", span).on(foreign).send()? {
        Value::List(items) => Ok(items.borrow().clone()),
        _ => Ok(Vec::new()),
    }
}

/// `python.copy(thing)`: a list, a tuple or a dict of the other side as
/// a list or a dict of Zeno, item by item.
pub fn copy(foreign: &Foreign, span: Span) -> Result {
    Ask::new(foreign.lang, "copy", span).on(foreign).send()
}

/// How `print` shows it (`repr` false), and how it is shown inside a
/// list (`repr` true). Showing a value cannot fail, so if the other
/// side cannot say, this says what kind of thing it is.
pub fn shown(foreign: &Foreign, repr: bool) -> String {
    let op = if repr { "repr" } else { "str" };
    match Ask::new(foreign.lang, op, Span { line: 0, col: 0 }).on(foreign).send() {
        Ok(Value::Str(text)) => text.to_string(),
        _ => format!("<{} value>", foreign.lang.name()),
    }
}

/// `==`, asked of the other side when at least one of the two is there.
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

// --------------------------------------------------------------- The socket

#[cfg(not(target_arch = "wasm32"))]
mod link {
    use std::cell::RefCell;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{parse, read, Lang, Result, Wire};
    use crate::token::Span;
    use crate::value::{fatal, raise};

    const PYTHON: &str = include_str!("bridge.py");

    /// How long the helper has to start and say hello.
    const PATIENCE: Duration = Duration::from_secs(30);

    struct Link {
        child: Child,
        reader: BufReader<TcpStream>,
        writer: TcpStream,
    }

    thread_local! {
        /// The helper of each language that has been asked for.
        static LINKS: RefCell<Vec<(Lang, Link)>> = const { RefCell::new(Vec::new()) };
        /// What Zeno has let go of since the last line was sent.
        static FREED: RefCell<Vec<(Lang, u64)>> = const { RefCell::new(Vec::new()) };
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
            let Link { mut child, reader, writer } = link;
            // The end of the lines is how the helper knows.
            let _ = writer.shutdown(std::net::Shutdown::Both);
            drop(reader);
            let _ = child.wait();
        }
    }

    /// The programs to try, in order. `ZENO_PYTHON` names one, for a
    /// Python that is not the first one found.
    fn programs(lang: Lang) -> Vec<String> {
        match lang {
            Lang::Python => match std::env::var("ZENO_PYTHON") {
                Ok(named) if !named.is_empty() => vec![named],
                _ if cfg!(windows) => vec!["python".to_string(), "python3".to_string(), "py".to_string()],
                _ => vec!["python3".to_string(), "python".to_string()],
            },
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
        let cannot = |why: String| raise("FileError", format!("cannot start {}: {why}", lang.name()), span);
        let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
            return cannot("this machine gave no socket to talk to it over".to_string());
        };
        let (Ok(address), Ok(())) = (listener.local_addr(), listener.set_nonblocking(true)) else {
            return cannot("this machine gave no socket to talk to it over".to_string());
        };
        let token = secret();
        let programs = programs(lang);
        for program in &programs {
            if !is_python_3(program) {
                continue;
            }
            let started = Command::new(program)
                .args(["-c", PYTHON])
                .env("ZENO_BRIDGE", format!("{} {token} {}", address.port(), first_number()))
                .spawn();
            let mut child = match started {
                Ok(child) => child,
                Err(_) => continue,
            };
            // Wait for it to connect and say the word. A program that
            // ends first was not Python (or not one that works), and
            // the next is tried.
            let began = Instant::now();
            loop {
                if let Ok((stream, _)) = listener.accept() {
                    if let Some(link) = greeted(stream, &token) {
                        let (reader, writer) = link;
                        return Ok(Link { child, reader, writer });
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
        match std::env::var("ZENO_PYTHON") {
            Ok(named) if !named.is_empty() => raise("FileError", format!("there is no {} at `{named}`, which `ZENO_PYTHON` names", lang.name()), span),
            _ => raise(
                "FileError",
                format!(
                    "there is no {} to run. Zeno looked for `{}`. `ZENO_PYTHON` can name one that is somewhere else",
                    lang.name(),
                    programs.join("`, then `")
                ),
                span,
            ),
        }
    }

    /// The first line of a helper is the word it was given.
    fn greeted(stream: TcpStream, token: &str) -> Option<(BufReader<TcpStream>, TcpStream)> {
        stream.set_nonblocking(false).ok()?;
        stream.set_nodelay(true).ok()?;
        stream.set_read_timeout(Some(PATIENCE)).ok()?;
        let mut reader = BufReader::new(stream.try_clone().ok()?);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let Wire::Object(members) = parse(&line)? else { return None };
        let said = members.iter().find(|(name, _)| name == "hello").map(|(_, value)| value);
        if said != Some(&Wire::Text(token.to_string())) {
            return None;
        }
        stream.set_read_timeout(None).ok()?;
        Some((reader, stream))
    }

    /// Send one line and read the answer.
    pub fn ask(lang: Lang, mut line: String, span: Span) -> Result {
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
                return fatal(format!("{} stopped while Zeno was waiting for its answer", lang.name()), span);
            }
            Ok(answer)
        })?;

        let wrong = || fatal(format!("{} answered with a line Zeno cannot read", lang.name()), span);
        let Some(Wire::Object(mut members)) = parse(&answer) else { return wrong() };
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
}

/// A page is inside a browser, which starts no programs.
#[cfg(target_arch = "wasm32")]
mod link {
    use super::{Lang, Result};
    use crate::token::Span;
    use crate::value::raise;

    pub fn freed(_: Lang, _: u64) {}

    pub fn close() {}

    pub fn ask(lang: Lang, _: String, span: Span) -> Result {
        raise("FileError", format!("cannot start {}: a page cannot start a program", lang.name()), span)
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
