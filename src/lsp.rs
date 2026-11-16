//! `zeno lsp`: the language server. An editor starts it, and from then
//! on the editor shows the mistakes in a Zeno file as it is typed, and
//! lays the file out on request.
//!
//! The Language Server Protocol is how an editor and a language talk.
//! The editor starts `zeno lsp` and writes messages to its input: "this
//! file was opened", "this file now reads so", "format this file". The
//! server writes messages back: "these are the mistakes in that file".
//! Every editor that speaks the protocol (VS Code, Neovim, Helix, Zed,
//! Emacs and others) can then work with Zeno, with no code written for
//! any one of them.
//!
//! A message is JSON (`json.rs`), with one header line ahead of it that
//! says how many bytes it is. This file has three parts:
//!
//! 1. **The server**: what it knows (the files that are open) and what
//!    it does with each message. It reads no input and writes no
//!    output itself, so the tests can hand it messages directly.
//! 2. **Checking**: an open file is checked as `zeno check` checks it,
//!    from the text in the editor and not the text on disk.
//! 3. **The wire**: reading a message from the input, and writing one.
//!
//! See docs/lsp.md for a walk through this file.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::checker;
use crate::error::Error;
use crate::format;
use crate::json::Json;
use crate::lexer;
use crate::modules;
use crate::package;
use crate::token::TokenKind;

// ---------------------------------------------------------------- The server

/// One mistake, as an editor shows it.
#[derive(Debug, Clone, PartialEq)]
struct Diagnostic {
    /// Lines and characters, counted from 0 as the protocol counts them.
    line: usize,
    from: usize,
    to: usize,
    message: String,
}

#[derive(Default)]
pub struct Server {
    /// The text of each file that is open in the editor, by its path.
    open: BTreeMap<String, String>,
    /// The address the editor knows each open file by.
    uris: HashMap<String, String>,
    /// The mistakes the editor was last told of, by the file's path.
    /// A file is told of again only when its mistakes have changed.
    shown: BTreeMap<String, Vec<Diagnostic>>,
    /// The folder the editor has open, if it said. A program's folder
    /// is not looked for above it.
    workspace: Option<PathBuf>,
    /// The editor has asked the server to stop.
    stopping: bool,
}

impl Server {
    /// Take one message from the editor, and give the messages to send
    /// back.
    pub fn handle(&mut self, message: &Json) -> Vec<Json> {
        let method = message.get("method").and_then(Json::as_str).unwrap_or("");
        // A request has an `id` and wants an answer. A notification has none.
        let id = message.get("id");
        let params = message.get("params").unwrap_or(&Json::Null);
        let answer = |result: Json| match id {
            Some(id) => vec![Json::object([("jsonrpc", Json::text("2.0")), ("id", id.clone()), ("result", result)])],
            None => Vec::new(),
        };
        let uri = params.at(&["textDocument", "uri"]).and_then(Json::as_str);

        match method {
            "initialize" => {
                self.workspace = params.get("rootUri").and_then(Json::as_str).and_then(path_of).map(PathBuf::from);
                answer(Json::object([
                    (
                        "capabilities",
                        Json::object([
                            // The editor says when a file is opened and
                            // closed, and sends its whole text at every change.
                            ("textDocumentSync", Json::object([("openClose", Json::Bool(true)), ("change", Json::number(1))])),
                            ("documentFormattingProvider", Json::Bool(true)),
                            ("definitionProvider", Json::Bool(true)),
                            ("hoverProvider", Json::Bool(true)),
                        ]),
                    ),
                    ("serverInfo", Json::object([("name", Json::text("zeno")), ("version", Json::text(env!("CARGO_PKG_VERSION")))])),
                ]))
            }
            "shutdown" => {
                self.stopping = true;
                answer(Json::Null)
            }
            "textDocument/didOpen" | "textDocument/didChange" => {
                let text = match method {
                    "textDocument/didOpen" => params.at(&["textDocument", "text"]),
                    // The whole text is sent, as the last change of the list.
                    _ => params.get("contentChanges").and_then(Json::as_list).and_then(|changes| changes.last()?.get("text")),
                };
                if let (Some(uri), Some(text)) = (uri, text.and_then(Json::as_str)) {
                    // Only programs are checked: a `zeno.package` that
                    // is open in the editor is not one.
                    if let Some(path) = path_of(uri).filter(|path| path.ends_with(".zeno")) {
                        self.uris.insert(path.clone(), uri.to_string());
                        self.open.insert(path, text.to_string());
                    }
                }
                self.publish()
            }
            "textDocument/didClose" => {
                if let Some(path) = uri.and_then(path_of) {
                    self.open.remove(&path);
                }
                self.publish()
            }
            // A file was saved: one that is not open may have changed too.
            "textDocument/didSave" => self.publish(),
            "textDocument/formatting" => {
                let source = uri.and_then(path_of).and_then(|path| self.open.get(&path));
                let edits = match source.map(|source| (source, format::format(source))) {
                    // One edit: the whole file, laid out.
                    Some((source, Ok(formatted))) if formatted != *source => vec![Json::object([
                        ("range", range(0, 0, source.lines().count() + 1, 0)),
                        ("newText", Json::text(formatted)),
                    ])],
                    // Laid out already, or it has a mistake, which the
                    // editor is showing.
                    _ => Vec::new(),
                };
                answer(Json::List(edits))
            }
            // "What is the name under the cursor?" and "where is it declared?"
            "textDocument/definition" | "textDocument/hover" => {
                let place = |name: &str| params.at(&["position", name]).and_then(Json::as_usize);
                let meaning = match (uri.and_then(path_of), place("line"), place("character")) {
                    (Some(path), Some(line), Some(character)) => self.meaning(&path, line, character),
                    _ => None,
                };
                answer(match (method, meaning) {
                    ("textDocument/hover", Some(meaning)) => Json::object([(
                        "contents",
                        Json::object([("kind", Json::text("plaintext")), ("value", Json::text(format!("{}: {}", meaning.written, meaning.ty)))]),
                    )]),
                    ("textDocument/definition", Some(Meaning { declared: Some((path, line, character)), .. })) => {
                        let uri = self.uris.get(&path).cloned().unwrap_or_else(|| uri_of(&path));
                        Json::object([("uri", Json::text(uri)), ("range", range(line, character, line, character))])
                    }
                    // Not a name, or one that comes with Zeno.
                    _ => Json::Null,
                })
            }
            _ => match id {
                // A request this server has no answer to gets told so.
                // Anything else the editor mentions in passing is let go.
                Some(id) if !method.is_empty() => vec![Json::object([
                    ("jsonrpc", Json::text("2.0")),
                    ("id", id.clone()),
                    (
                        "error",
                        Json::object([
                            ("code", Json::Number(-32601.0)),
                            ("message", Json::text(format!("`zeno lsp` has no answer to `{method}`"))),
                        ]),
                    ),
                ])],
                _ => Vec::new(),
            },
        }
    }

    /// Check every open file, and tell the editor of each file whose
    /// mistakes are not the ones it was last told of.
    fn publish(&mut self) -> Vec<Json> {
        let mut now: BTreeMap<String, Vec<Diagnostic>> = BTreeMap::new();
        for path in self.open.keys() {
            now.entry(path.clone()).or_default();
            for (file, diagnostic) in self.check(path) {
                let found = now.entry(file).or_default();
                // Two open files can lead to one mistake in a third.
                if !found.contains(&diagnostic) {
                    found.push(diagnostic);
                }
            }
        }
        // A file that had mistakes and has none now is told so.
        for path in self.shown.keys() {
            now.entry(path.clone()).or_default();
        }

        let mut messages = Vec::new();
        for (path, diagnostics) in &now {
            if self.shown.get(path) == Some(diagnostics) {
                continue;
            }
            let uri = self.uris.get(path).cloned().unwrap_or_else(|| uri_of(path));
            let list = diagnostics
                .iter()
                .map(|found| {
                    Json::object([
                        ("range", range(found.line, found.from, found.line, found.to)),
                        ("severity", Json::number(1)), // an error
                        ("source", Json::text("zeno")),
                        ("message", Json::text(found.message.clone())),
                    ])
                })
                .collect();
            messages.push(Json::object([
                ("jsonrpc", Json::text("2.0")),
                ("method", Json::text("textDocument/publishDiagnostics")),
                ("params", Json::object([("uri", Json::text(uri)), ("diagnostics", Json::List(list))])),
            ]));
        }
        now.retain(|path, diagnostics| !diagnostics.is_empty() || self.open.contains_key(path));
        self.shown = now;
        messages
    }

    // ------------------------------------------------------------- Checking

    /// The text of a file: what the editor has, or else what the disk has.
    fn read(&self, path: &str) -> Option<String> {
        self.open.get(path).cloned().or_else(|| std::fs::read_to_string(path).ok())
    }

    /// The folder of the program a file belongs to: the nearest folder,
    /// from the file's own upward, that has a `zeno.package` or a
    /// `main.zeno` in it. With neither anywhere, the file's own folder.
    ///
    /// `zeno run` is given the file a program starts in, and takes that
    /// file's folder. An editor opens any file of a program, so the
    /// server has to work the folder out.
    fn folder_of(&self, path: &str) -> PathBuf {
        let own = Path::new(path).parent().unwrap_or(Path::new("")).to_path_buf();
        let mut folder = own.clone();
        loop {
            let has = |name: &str| self.read(&crate::modules::shown(&folder.join(name))).is_some();
            if has(package::FILE) || has("main.zeno") {
                return folder;
            }
            if self.workspace.as_ref() == Some(&folder) || !folder.pop() || folder.as_os_str().is_empty() {
                return own;
            }
        }
    }

    /// The mistakes in the program that the open file `path` is a file
    /// of, each with the file it is in.
    fn check(&self, path: &str) -> Vec<(String, Diagnostic)> {
        let source = self.open[path].clone();
        let read = |path: &str| self.read(path);
        let loaded = modules::load_in(&self.folder_of(path), path, source, &read);
        let errors = match &loaded.program {
            Ok(program) => checker::check_by(program, checker::Rules { strict: false, one_owner: false, firmware: false }).0,
            Err(errors) => errors.clone(),
        };
        errors
            .iter()
            .filter_map(|error| {
                let file = loaded.files.get(error.span.file())?;
                Some((file.path.clone(), diagnostic(error, &file.source)))
            })
            .collect()
    }
}

/// What a name in an open file means: how it is written there, its
/// type, and where it was declared (the file, and the line and character
/// as the protocol counts them).
struct Meaning {
    written: String,
    ty: String,
    declared: Option<(String, usize, usize)>,
}

impl Server {
    /// The meaning of the name at a place in the open file `path`, if a
    /// name stands there and the checker knows what it is.
    ///
    /// The checker does the work: it notes every place a name is used,
    /// with what the name means there (`checker::Use`). This looks up
    /// the place the editor asked about. Nothing is guessed from how a
    /// name is spelled, so a name a function declares for itself is never
    /// taken for one at the top of the file.
    fn meaning(&self, path: &str, line: usize, character: usize) -> Option<Meaning> {
        let source = self.open.get(path)?;
        // The name under the cursor, and the name its line of dots
        // begins with: `shapes` for the `area` of `shapes.area`.
        let written = lexer::lex_written(source).ok()?;
        let text: Vec<char> = source.lines().nth(line)?.chars().collect();
        let col = 1 + chars_in(&text, character);
        let on = |place: usize| {
            let token = &written.tokens[place];
            let width = written.texts[place].chars().count();
            token.span.line == line + 1 && token.span.col <= col && col < token.span.col + width.max(1)
        };
        let here = (0..written.tokens.len()).find(|&place| on(place))?;
        let TokenKind::Ident(word) = &written.tokens[here].kind else { return None };
        let mut first = here;
        while first >= 2 && written.tokens[first - 1].kind == TokenKind::Dot && matches!(written.tokens[first - 2].kind, TokenKind::Ident(_)) {
            first -= 2;
        }

        let read = |path: &str| self.read(path);
        let loaded = modules::load_in(&self.folder_of(path), path, source.clone(), &read);
        let program = loaded.program.as_ref().ok()?;
        let uses = checker::check_by(program, checker::Rules { strict: false, one_owner: false, firmware: false }).1.uses();
        // The open file is the first file, so its positions are as written.
        // A name of a module is one name in the joined program, and it
        // stands where its line of dots begins.
        let found = [here, first].into_iter().find_map(|place| {
            let at = written.tokens[place].span;
            uses.iter().find(|used| used.at == at && crate::ast::shown_name(&used.name) == word)
        })?;
        let declared = found.declared.and_then(|span| {
            let file = loaded.files.get(span.file())?;
            let line = span.line_in_file().checked_sub(1)?;
            let text: Vec<char> = file.source.lines().nth(line)?.chars().collect();
            let character = text.iter().take(span.col.saturating_sub(1)).map(|c| c.len_utf16()).sum();
            Some((file.path.clone(), line, character))
        });
        Some(Meaning { written: word.clone(), ty: found.ty.clone(), declared })
    }
}

/// How many characters of a line come before the place an editor calls
/// `character`. The editor counts as UTF-16 does, where a character
/// outside the first 65,536 is two.
fn chars_in(line: &[char], character: usize) -> usize {
    let mut units = 0;
    line.iter().take_while(|c| {
        units += c.len_utf16();
        units <= character
    }).count()
}

/// A mistake as an editor shows it: from where it is to the end of the
/// word there.
fn diagnostic(error: &Error, source: &str) -> Diagnostic {
    let line = error.span.line_in_file().saturating_sub(1);
    let text: Vec<char> = source.lines().nth(line).unwrap_or("").chars().collect();
    let from = error.span.col.saturating_sub(1).min(text.len());
    let word = text[from..].iter().take_while(|c| c.is_alphanumeric() || **c == '_').count();
    let to = (from + word.max(1)).min(text.len().max(from + 1));
    // The protocol counts a character as a text in UTF-16 would: most
    // are one, and those outside the first 65,536 are two.
    let utf16 = |upto: usize| text.iter().take(upto).map(|c| c.len_utf16()).sum();
    let message = match &error.note {
        Some(note) => format!("{}\n{note}", error.message),
        None => error.message.clone(),
    };
    Diagnostic { line, from: utf16(from), to: utf16(to), message }
}

fn range(from_line: usize, from: usize, to_line: usize, to: usize) -> Json {
    let place = |line: usize, character: usize| Json::object([("line", Json::number(line)), ("character", Json::number(character))]);
    Json::object([("start", place(from_line, from)), ("end", place(to_line, to))])
}

/// The path of the file an address names: `file:///home/me/a%20b.zeno`
/// is `/home/me/a b.zeno`. `None` for an address that is not a file's.
fn path_of(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut path = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let hex = |byte: u8| (byte as char).to_digit(16);
        match (bytes[at], bytes.get(at + 1).copied().and_then(hex), bytes.get(at + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                path.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, _, _) => {
                path.push(byte);
                at += 1;
            }
        }
    }
    let path = String::from_utf8(path).ok()?;
    // On Windows the address is `file:///C:/work/a.zeno`.
    let drive = path.as_bytes().get(2) == Some(&b':') && path.starts_with('/');
    Some(if drive { path[1..].to_string() } else { path })
}

/// The address of a file, from its path.
fn uri_of(path: &str) -> String {
    let mut uri = String::from("file://");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => uri.push(byte as char),
            b'\\' => uri.push('/'),
            byte => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

// ------------------------------------------------------------------ The wire

/// Read one message: header lines, an empty line, and then as many
/// bytes of JSON as the `Content-Length` header said. `None` when the
/// input has ended.
fn read_message(input: &mut impl BufRead) -> Option<Json> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    // A message that is not JSON is passed over, not the end of the talk.
    Some(Json::parse(&String::from_utf8_lossy(&body)).unwrap_or(Json::Null))
}

fn write_message(output: &mut impl Write, message: &Json) -> io::Result<()> {
    let body = message.write();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

/// `zeno lsp`: answer the editor until it says `exit`.
pub fn run() -> ExitCode {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut server = Server::default();
    while let Some(message) = read_message(&mut input) {
        if message.get("method").and_then(Json::as_str) == Some("exit") {
            break;
        }
        for reply in server.handle(&message) {
            if write_message(&mut output, &reply).is_err() {
                return ExitCode::FAILURE;
            }
        }
    }
    // The protocol: leaving without having been asked to stop is a failure.
    match server.stopping {
        true => ExitCode::SUCCESS,
        false => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> Json {
        Json::parse(text).expect("the test's message is JSON")
    }

    /// The editor opens a file, or changes one that is open.
    fn open(server: &mut Server, name: &str, text: &str) -> Vec<Json> {
        server.handle(&Json::object([
            ("jsonrpc", Json::text("2.0")),
            ("method", Json::text("textDocument/didOpen")),
            ("params", Json::object([("textDocument", Json::object([("uri", Json::text(format!("file:///work/{name}"))), ("text", Json::text(text))]))])),
        ]))
    }

    /// What the server told the editor: each file, and its mistakes as
    /// `line:from-to message`.
    fn told(messages: &[Json]) -> Vec<(String, Vec<String>)> {
        messages
            .iter()
            .map(|message| {
                assert_eq!(message.get("method").and_then(Json::as_str), Some("textDocument/publishDiagnostics"));
                let uri = message.at(&["params", "uri"]).and_then(Json::as_str).unwrap().to_string();
                let found = message.at(&["params", "diagnostics"]).and_then(Json::as_list).unwrap();
                let lines = found
                    .iter()
                    .map(|found| {
                        let number = |path: &[&str]| found.at(path).and_then(Json::as_usize).unwrap();
                        format!(
                            "{}:{}-{} {}",
                            number(&["range", "start", "line"]),
                            number(&["range", "start", "character"]),
                            number(&["range", "end", "character"]),
                            found.get("message").and_then(Json::as_str).unwrap()
                        )
                    })
                    .collect();
                (uri, lines)
            })
            .collect()
    }

    #[test]
    fn the_server_says_what_it_can_do() {
        let mut server = Server::default();
        let reply = server.handle(&message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":"file:///work"}}"#));
        assert_eq!(reply.len(), 1);
        assert_eq!(reply[0].get("id").and_then(Json::as_usize), Some(1));
        let can = reply[0].at(&["result", "capabilities"]).unwrap();
        assert_eq!(can.at(&["textDocumentSync", "change"]).and_then(Json::as_usize), Some(1));
        assert_eq!(can.get("documentFormattingProvider"), Some(&Json::Bool(true)));
        assert_eq!(server.workspace, Some(PathBuf::from("/work")));
        // A notification gets no answer. A request it does not know gets told so.
        assert_eq!(server.handle(&message(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#)), []);
        let unknown = server.handle(&message(r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/rename","params":{}}"#));
        assert_eq!(unknown[0].at(&["error", "code"]), Some(&Json::Number(-32601.0)));
        let stop = server.handle(&message(r#"{"jsonrpc":"2.0","id":3,"method":"shutdown"}"#));
        assert_eq!(stop[0].get("result"), Some(&Json::Null));
        assert!(server.stopping);
    }

    #[test]
    fn mistakes_are_shown_as_the_file_is_typed() {
        let mut server = Server::default();
        // A mistake the type checker finds, with where the word is.
        let said = told(&open(&mut server, "main.zeno", "let count = 3\nprint(cuont + 1)\n"));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0].0, "file:///work/main.zeno");
        assert_eq!(said[0].1.len(), 1);
        assert!(said[0].1[0].starts_with("1:6-11 "), "{said:?}");
        assert!(said[0].1[0].contains("cuont"), "{said:?}");
        // The same text again changes nothing, so nothing is said.
        assert_eq!(open(&mut server, "main.zeno", "let count = 3\nprint(cuont + 1)\n"), []);
        // Fixed: the editor is told the file has no mistakes now.
        let change = Json::parse(
            r#"{"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":"file:///work/main.zeno","version":2},"contentChanges":[{"text":"let count = 3\nprint(count + 1)\n"}]}}"#,
        )
        .unwrap();
        assert_eq!(told(&server.handle(&change)), [("file:///work/main.zeno".to_string(), Vec::new())]);
        // A mistake of syntax is found before the checker runs.
        let said = told(&open(&mut server, "main.zeno", "let a = = 1\n"));
        assert_eq!(said[0].1, ["0:8-9 expected an expression, found `=`"]);
        // A character outside the first 65,536 counts as two.
        let said = told(&open(&mut server, "main.zeno", "print(\"\u{1F600}\", nope)\n"));
        assert!(said[0].1[0].starts_with("0:12-16 "), "{said:?}");
    }

    #[test]
    fn a_program_in_several_files_is_checked_from_the_editor_s_text() {
        let mut server = Server::default();
        // `shapes.zeno` is not on any disk. It is open in the editor.
        assert_eq!(told(&open(&mut server, "shapes.zeno", "function area(side: int) -> int {\n    return side * side\n}\n")).len(), 1);
        let said = told(&open(&mut server, "main.zeno", "import shapes\nprint(shapes.area(\"two\"))\n"));
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].1[0].starts_with("1:18-19 "), "{said:?}");
        // A mistake in the imported file is shown in that file, once,
        // though both open files lead to it.
        let said = told(&open(&mut server, "shapes.zeno", "function area(side: int) -> int {\n    return side * sid\n}\n"));
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(said[0].0, "file:///work/shapes.zeno");
        assert_eq!(said[0].1.len(), 1, "{said:?}");
        assert!(said[0].1[0].starts_with("1:18-21 "), "{said:?}");

        // A file in a folder below is a file of the program above it:
        // the folder that has `main.zeno` is the program's.
        let mut server = Server::default();
        open(&mut server, "main.zeno", "print(\"the program starts here\")\n");
        open(&mut server, "drivers/pins.zeno", "let tx = 4\n");
        let said = told(&open(&mut server, "drivers/uart.zeno", "import drivers.pins\nprint(drivers.pins.tx)\n"));
        assert_eq!(said, [("file:///work/drivers/uart.zeno".to_string(), Vec::new())]);
        // A file that is not a Zeno program is not checked as one.
        assert_eq!(open(&mut server, "zeno.package", "package work\n"), []);
        // When the file is closed the editor is told nothing more of it.
        let close = message(r#"{"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":"file:///work/drivers/uart.zeno"}}}"#);
        assert_eq!(server.handle(&close), []);
        assert!(!server.open.contains_key("/work/drivers/uart.zeno"));
    }

    /// Ask about the place one character into the first `word` on a line
    /// of an open file: `"(p"` asks about the `p` after a bracket.
    fn ask(server: &mut Server, method: &str, name: &str, line: usize, word: &str) -> Json {
        let path = format!("/work/{name}");
        let text: Vec<char> = server.open[&path].lines().nth(line).unwrap().chars().collect();
        let col = text.iter().collect::<String>().find(word).unwrap_or_else(|| panic!("`{word}` is not on line {line}"));
        let character: usize = text.iter().collect::<String>()[..col].chars().map(char::len_utf16).sum();
        let reply = server.handle(&Json::object([
            ("jsonrpc", Json::text("2.0")),
            ("id", Json::number(9)),
            ("method", Json::text(method)),
            (
                "params",
                Json::object([
                    ("textDocument", Json::object([("uri", Json::text(format!("file:///work/{name}")))])),
                    ("position", Json::object([("line", Json::number(line)), ("character", Json::number(character + 1))])),
                ]),
            ),
        ]));
        reply[0].get("result").cloned().unwrap()
    }

    /// Where a "go to declaration" answer points: the file, line and character.
    fn points_to(answer: &Json) -> (String, usize, usize) {
        let number = |name: &str| answer.at(&["range", "start", name]).and_then(Json::as_usize).unwrap();
        (answer.get("uri").and_then(Json::as_str).unwrap().to_string(), number("line"), number("character"))
    }

    #[test]
    fn a_name_says_what_it_is_and_where_it_was_declared() {
        let mut server = Server::default();
        open(&mut server, "shapes.zeno", "class Point {\n    x: int\n}\n\nfunction area(side: int) -> int {\n    return side * side\n}\n");
        open(
            &mut server,
            "main.zeno",
            "import shapes\n\
             let count = 3\n\
             function twice(count: int) -> int {\n    return count * 2\n}\n\
             print(twice(count) + shapes.area(count))\n\
             for step in [1, 2] {\n    print(step)\n}\n\
             let p = shapes.Point(1)\nprint(p.x, \"\u{1F600}\", count)\n",
        );
        let main = "file:///work/main.zeno".to_string();
        // A variable, and a function: each goes to its own line.
        assert_eq!(points_to(&ask(&mut server, "textDocument/definition", "main.zeno", 5, "count")), (main.clone(), 1, 0));
        assert_eq!(points_to(&ask(&mut server, "textDocument/definition", "main.zeno", 5, "twice")), (main.clone(), 2, 0));
        // Inside `twice`, `count` is the parameter and not the variable
        // above it: the checker knows, and nothing is guessed.
        assert_eq!(points_to(&ask(&mut server, "textDocument/definition", "main.zeno", 3, "count")), (main.clone(), 2, 0));
        // The variable of a loop.
        assert_eq!(points_to(&ask(&mut server, "textDocument/definition", "main.zeno", 7, "step")), (main.clone(), 6, 0));
        // A name of a module goes to the module's file.
        assert_eq!(
            points_to(&ask(&mut server, "textDocument/definition", "main.zeno", 5, "area")),
            ("file:///work/shapes.zeno".to_string(), 4, 0)
        );
        assert_eq!(
            points_to(&ask(&mut server, "textDocument/definition", "main.zeno", 9, "Point")),
            ("file:///work/shapes.zeno".to_string(), 0, 0)
        );
        // After a character that counts as two, the place is still found.
        assert_eq!(points_to(&ask(&mut server, "textDocument/definition", "main.zeno", 10, "count")), (main.clone(), 1, 0));

        // What a name is.
        let says = |server: &mut Server, line: usize, word: &str| {
            ask(server, "textDocument/hover", "main.zeno", line, word).at(&["contents", "value"]).and_then(Json::as_str).map(str::to_string)
        };
        assert_eq!(says(&mut server, 5, "count").as_deref(), Some("count: int"));
        assert_eq!(says(&mut server, 10, "(p").as_deref(), Some("p: shapes.Point"));
        assert_eq!(says(&mut server, 5, "twice").as_deref(), Some("twice: function(count: int) -> int"));
        assert_eq!(says(&mut server, 5, "print").as_deref(), Some("print: a function"));
        // A field says what it holds, and a method what it takes.
        assert_eq!(says(&mut server, 10, ".x").as_deref(), Some("x: int"));
        // No answer where the server would have to guess: where a field
        // was declared, a word of the language, a name that comes with
        // Zeno, a module.
        assert_eq!(ask(&mut server, "textDocument/definition", "main.zeno", 10, ".x"), Json::Null);
        assert_eq!(ask(&mut server, "textDocument/definition", "main.zeno", 1, "let"), Json::Null);
        assert_eq!(ask(&mut server, "textDocument/definition", "main.zeno", 5, "print"), Json::Null);
        assert_eq!(ask(&mut server, "textDocument/definition", "main.zeno", 5, "shapes"), Json::Null);
    }

    #[test]
    fn a_file_is_laid_out_on_request() {
        let mut server = Server::default();
        open(&mut server, "main.zeno", "let  a=1\nprint( a )\n");
        let ask = message(r#"{"jsonrpc":"2.0","id":7,"method":"textDocument/formatting","params":{"textDocument":{"uri":"file:///work/main.zeno"},"options":{"tabSize":4,"insertSpaces":true}}}"#);
        let reply = server.handle(&ask);
        let edits = reply[0].get("result").and_then(Json::as_list).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].get("newText").and_then(Json::as_str), Some("let a = 1\nprint(a)\n"));
        assert_eq!(edits[0].at(&["range", "start", "line"]).and_then(Json::as_usize), Some(0));
        assert_eq!(edits[0].at(&["range", "end", "line"]).and_then(Json::as_usize), Some(3));
        // A file that is laid out already, or has a mistake, gets no edit.
        open(&mut server, "main.zeno", "let a = 1\nprint(a)\n");
        assert_eq!(server.handle(&ask)[0].get("result"), Some(&Json::List(Vec::new())));
        open(&mut server, "main.zeno", "let a = = 1\n");
        assert_eq!(server.handle(&ask)[0].get("result"), Some(&Json::List(Vec::new())));
    }

    #[test]
    fn an_address_is_a_path_and_back() {
        assert_eq!(path_of("file:///home/me/a%20b.zeno").as_deref(), Some("/home/me/a b.zeno"));
        assert_eq!(path_of("file:///C:/work/a.zeno").as_deref(), Some("C:/work/a.zeno"));
        assert_eq!(path_of("untitled:Untitled-1"), None);
        assert_eq!(uri_of("/home/me/a b.zeno"), "file:///home/me/a%20b.zeno");
        assert_eq!(uri_of("C:\\work\\a.zeno"), "file:///C:/work/a.zeno");
        assert_eq!(path_of(&uri_of("/tmp/caf\u{e9}/x.zeno")).as_deref(), Some("/tmp/caf\u{e9}/x.zeno"));
    }

    #[test]
    fn a_message_goes_over_the_wire_and_comes_back() {
        let mut wire = Vec::new();
        let sent = Json::object([("jsonrpc", Json::text("2.0")), ("method", Json::text("caf\u{e9}"))]);
        write_message(&mut wire, &sent).unwrap();
        write_message(&mut wire, &Json::Null).unwrap();
        let text = String::from_utf8(wire.clone()).unwrap();
        // The length is in bytes, and `é` is two.
        assert!(text.starts_with("Content-Length: 34\r\n\r\n{\"jsonrpc\""), "{text}");
        let mut input = io::Cursor::new(wire);
        assert_eq!(read_message(&mut input), Some(sent));
        assert_eq!(read_message(&mut input), Some(Json::Null));
        assert_eq!(read_message(&mut input), None);
        // Other headers are allowed, and the name is not case sensitive.
        let mut input = io::Cursor::new(b"Content-Type: x\r\ncontent-length: 2\r\n\r\n{}".to_vec());
        assert_eq!(read_message(&mut input), Some(Json::Object(Vec::new())));
    }
}
