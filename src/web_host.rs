//! Zeno in a browser.
//!
//! This file is only built for WebAssembly (`cargo build --release
//! --target wasm32-unknown-unknown`). The result, `zeno.wasm`, is the
//! same lexer, parser, checker and interpreter that `zeno run` uses,
//! with this file where `main` would be. `zeno build --web` puts it
//! in a page beside a program, and `web.js` starts it.
//!
//! WebAssembly can do arithmetic and call functions, and nothing else:
//! it cannot print, read a clock or touch a page. Everything of that
//! kind is asked of the JavaScript that started it. So this file is two
//! short lists.
//!
//! What the page calls (`#[no_mangle]`, so that the name is kept):
//!
//! - `zeno_alloc(length)`: room for that many bytes, for the page to
//!   put text in. Text crosses the line as a place and a length.
//! - `zeno_start(place, length)`: check and run a program. The text
//!   is JSON: `{"main": "counter.zeno", "files": {"counter.zeno": "..."}}`.
//! - `zeno_call(handler)`, `zeno_call_with(handler, place, length)`:
//!   call a function the program gave to `on`, `after`, `every`,
//!   `on_key`, `fetch` or `post`, the second with what the call hands
//!   over, as JSON: the key, the event, or the answer of the network.
//!
//! What it asks of the page (`extern "C"`):
//!
//! - `zeno_host_print(place, length)`: show this text.
//! - `zeno_host_error(place, length)`: show this error. The program
//!   has stopped.
//! - `zeno_host_page(place, length)`: a question about the page or a
//!   change to it, as JSON: `["set_text", 3, "Hello"]`. The answer
//!   comes back as a place: four bytes that say how long it is, and
//!   then that much JSON, in room the page got from `zeno_alloc`.
//! - `zeno_host_input(place, length)`: ask the person for a line.
//!   The answer comes back the same way, and is `null` if they
//!   closed the question.
//! - `zeno_host_now()`: a clock, in milliseconds.
//!
//! See docs/web.md.

use std::alloc::{alloc, dealloc, Layout};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::checker;
use crate::error::Error;
use crate::interpreter::Session;
use crate::json::Json;
use crate::modules;
use crate::page::Page;
use crate::value::Value;

// The page gives these under the name `zeno`, when it starts the program.
#[link(wasm_import_module = "zeno")]
extern "C" {
    fn zeno_host_print(place: *const u8, length: usize);
    fn zeno_host_error(place: *const u8, length: usize);
    fn zeno_host_page(place: *const u8, length: usize) -> *mut u8;
    fn zeno_host_input(place: *const u8, length: usize) -> *mut u8;
    fn zeno_host_now() -> f64;
}

thread_local! {
    /// The program of this page, from when it started until the page is
    /// closed, and its files, to show a line of one in an error.
    static RUNNING: RefCell<Option<(Session, Vec<modules::File>)>> = const { RefCell::new(None) };
}

fn layout(length: usize) -> Layout {
    Layout::from_size_align(length.max(1), 1).expect("a length")
}

/// Room for `length` bytes.
#[no_mangle]
pub extern "C" fn zeno_alloc(length: usize) -> *mut u8 {
    // SAFETY: the layout has a size of at least one byte.
    unsafe { alloc(layout(length)) }
}

/// The text the page put at `place`. The room is given back.
fn taken(place: *mut u8, length: usize) -> String {
    if place.is_null() {
        return String::new();
    }
    // SAFETY: the page got `place` from `zeno_alloc(length)` and filled it.
    let bytes = unsafe { std::slice::from_raw_parts(place, length).to_vec() };
    // SAFETY: the same room, with the same layout, given back once.
    unsafe { dealloc(place, layout(length)) };
    String::from_utf8_lossy(&bytes).into_owned()
}

/// An answer of the page: four bytes of length, and then the JSON.
fn answer(place: *mut u8) -> Json {
    if place.is_null() {
        return Json::Null;
    }
    // SAFETY: the page got `place` from `zeno_alloc(4 + length)` and
    // wrote the length into its first four bytes.
    let length = unsafe { u32::from_le_bytes(*(place as *const [u8; 4])) } as usize;
    // SAFETY: the bytes after the length are the page's answer.
    let bytes = unsafe { std::slice::from_raw_parts(place.add(4), length).to_vec() };
    // SAFETY: the room is given back once, with the layout it was made with.
    unsafe { dealloc(place, layout(4 + length)) };
    Json::parse(&String::from_utf8_lossy(&bytes)).unwrap_or(Json::Null)
}

/// Show text where the page shows what a program prints.
pub fn print(text: &str) {
    // SAFETY: the page reads `text` before this call returns.
    unsafe { zeno_host_print(text.as_ptr(), text.len()) }
}

/// Ask the page, or change it.
pub fn ask(request: &Json) -> Json {
    let request = request.write();
    // SAFETY: the page reads the request before this call returns.
    answer(unsafe { zeno_host_page(request.as_ptr(), request.len()) })
}

/// `input("Name: ")`: ask the person. `None` if they closed the question.
pub fn input(question: &str) -> Option<String> {
    // SAFETY: the page reads the question before this call returns.
    match answer(unsafe { zeno_host_input(question.as_ptr(), question.len()) }) {
        Json::Text(line) => Some(line),
        _ => None,
    }
}

/// A clock that only goes forward, in milliseconds.
pub fn now() -> f64 {
    // SAFETY: it takes nothing and gives a number.
    unsafe { zeno_host_now() }
}

/// Tell the page that the program stopped, and why.
fn stopped(files: &[modules::File], error: &Error) {
    let text = crate::shown(files, error);
    // SAFETY: the page reads `text` before this call returns.
    unsafe { zeno_host_error(text.as_ptr(), text.len()) }
}

/// Check the program and run it. Gives 0 if it ran to its last line,
/// and 1 if something stopped it, which the page has then been told.
#[no_mangle]
pub extern "C" fn zeno_start(place: *mut u8, length: usize) -> u32 {
    let bundle = Json::parse(&taken(place, length)).unwrap_or(Json::Null);
    let main = bundle.get("main").and_then(Json::as_str).unwrap_or("main.zeno").to_string();
    let mut sources = HashMap::new();
    if let Some(Json::Object(files)) = bundle.get("files") {
        for (path, source) in files {
            sources.insert(path.clone(), source.as_str().unwrap_or_default().to_string());
        }
    }
    let first = sources.get(&main).cloned().unwrap_or_default();
    let loaded = modules::load(&main, first, &|path| sources.get(path).cloned());
    let files = loaded.files;
    let program = match loaded.program {
        Ok(program) => program,
        Err(errors) => {
            errors.iter().for_each(|error| stopped(&files, error));
            return 1;
        }
    };
    // The same check as `zeno run`, and for the same reason: the checker
    // also finishes the program (see `checker::checked`).
    let (program, errors, _) = checker::checked(program, checker::Rules::default());
    if !errors.is_empty() {
        errors.iter().for_each(|error| stopped(&files, error));
        return 1;
    }
    let (session, result) = Session::start(program, Page::real(), false);
    let failed = result.as_ref().is_err();
    if let Err(error) = result {
        stopped(&files, &error);
    }
    RUNNING.with(|running| *running.borrow_mut() = Some((session, files)));
    failed as u32
}

/// Call a function of the program with these values.
fn call(handler: u32, arguments: Vec<Value>) -> u32 {
    RUNNING.with(|running| {
        let mut running = running.borrow_mut();
        let Some((session, files)) = running.as_mut() else { return 1 };
        match session.call(handler, arguments) {
            Ok(()) => 0,
            Err(error) => {
                stopped(files, &error);
                1
            }
        }
    })
}

/// The page calls a function that takes nothing: a click, a timer.
#[no_mangle]
pub extern "C" fn zeno_call(handler: u32) -> u32 {
    call(handler, Vec::new())
}

/// The page calls a function and hands something over, written as
/// JSON. One of three things:
///
/// - `{"key": "Enter"}`: the key that was pressed, for `on_key`.
/// - `{"event": {"kind": "click", "target": 4, "name": "<li>", "x": 10,
///   "y": 3, "key": ""}}`: what happened, for `on`. It reaches the
///   function only if the function asks for an `Event`.
/// - `{"reply": [200, "..."]}`: the answer of the network, for `fetch`
///   and `post`.
#[no_mangle]
pub extern "C" fn zeno_call_with(handler: u32, place: *mut u8, length: usize) -> u32 {
    let carried = Json::parse(&taken(place, length)).unwrap_or(Json::Null);
    let text = |json: Option<&Json>| Value::str(json.and_then(Json::as_str).unwrap_or_default().to_string());
    let number = |json: Option<&Json>| match json {
        Some(Json::Number(n)) => *n,
        _ => 0.0,
    };
    if let Some(key) = carried.get("key") {
        return call(handler, vec![text(Some(key))]);
    }
    if let Some(event) = carried.get("event") {
        let name = event.get("name").and_then(Json::as_str).unwrap_or_default();
        let event = crate::value::Event {
            kind: Rc::from(event.get("kind").and_then(Json::as_str).unwrap_or_default()),
            target: Value::Element(number(event.get("target")) as u32, Rc::from(name)),
            x: number(event.get("x")),
            y: number(event.get("y")),
            key: Rc::from(event.get("key").and_then(Json::as_str).unwrap_or_default()),
        };
        return call(handler, vec![Value::Event(Rc::new(event))]);
    }
    if let Some(Json::List(reply)) = carried.get("reply") {
        return call(handler, vec![Value::Int(number(reply.first()) as i64), text(reply.get(1))]);
    }
    call(handler, Vec::new())
}
