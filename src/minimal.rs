//! `zeno build --minimal`: the least C that runs a program.
//!
//! ```text
//! tokens -> parser -> tree -> checker -> emit_c -> [minimal] -> C source
//! ```
//!
//! An ordinary build pastes the whole runtime (src/runtime.c, some three
//! thousand lines) in front of the program, and the program calls into it
//! for every `+`, every `list[i]` and every `print`, so that each can be
//! checked. That is right for a program that is being written. It is a
//! lot to read, and a lot to carry, for one that is finished.
//!
//! A minimal build is put together here from the same pieces, with three
//! things done to them (decisions chapter 30, docs/minimal.md):
//!
//! 1. **The checks are gone.** The parts of the runtime marked
//!    `#ifndef ZENO_MINIMAL` are left out (`resolved`). An index out of
//!    range, an overflow, a division by zero: each does whatever the C
//!    does, as it would in a program written in C.
//! 2. **Nothing carries its place in the source.** A checked build hands
//!    every call that can fail the line and column it was written at,
//!    for the error message. With no checks there is no message, so the
//!    two numbers are taken out of every function and every call
//!    (`without_positions`).
//! 3. **Only what the program uses is written.** The runtime and the
//!    program are cut into their functions, types and variables
//!    (`entries`), and one is kept only if the program reaches it by name
//!    (`assemble`). `print("Hello")` needs `puts` and nothing else, and
//!    that is all its C file holds.
//!
//! None of this reads C the way a C compiler does. It reads it as far as
//! the runtime needs: where a comment, a str and a line for the
//! preprocessor begin and end, where the brackets are, and what the
//! names are. The runtime is one file that is written to be read this
//! way, and the tests at the bottom hold it to that.

use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------- Tokens

/// The kinds of piece C source is cut into here.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    /// A name: `zeno_join`, `int64_t`, `if`.
    Ident,
    /// A number: `16`, `0x80`, `1e9`.
    Number,
    /// One sign: `(`, `;`, `*`.
    Punct,
    /// Blanks and line ends.
    Space,
    /// A comment, with its `/*` and `*/`.
    Comment,
    /// Text in quotes, or one character in single quotes.
    Text,
    /// A line for the preprocessor, such as `#ifdef _WIN32`, without its
    /// line end. A line that ends in `\` goes on to the next.
    Directive,
}

/// Cut C source into its pieces. Put end to end, the pieces are the
/// source again.
fn tokens(text: &str) -> Vec<(Tok, &str)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    // Only blanks so far on this line: a `#` here begins a directive.
    let mut line_start = true;
    while i < bytes.len() {
        let start = i;
        let byte = bytes[i];
        let kind = if byte == b'#' && line_start {
            while i < bytes.len() && bytes[i] != b'\n' {
                // `\` and a line end: the directive goes on.
                if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                i += 1;
            }
            Tok::Directive
        } else if byte.is_ascii_whitespace() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                if bytes[i] == b'\n' {
                    line_start = true;
                }
                i += 1;
            }
            out.push((Tok::Space, &text[start..i]));
            continue;
        } else if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            Tok::Comment
        } else if byte == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            Tok::Comment
        } else if byte == b'"' || byte == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != byte {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            Tok::Text
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            Tok::Ident
        } else if byte.is_ascii_digit() {
            // `1e9`, `0x9E37`, `2.5`: letters and dots belong to it.
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                i += 1;
            }
            Tok::Number
        } else {
            // One sign. A byte of a longer character (there are none
            // outside comments and strs) is taken whole.
            i += 1;
            while i < bytes.len() && !text.is_char_boundary(i) {
                i += 1;
            }
            Tok::Punct
        };
        line_start = false;
        out.push((kind, &text[start..i]));
    }
    out
}

/// The names in a piece of C: every word that is not inside a comment
/// or a str. A directive is looked into as well, since `#ifdef
/// ZENO_GENERATIONS` uses the name it asks about.
fn names_used(text: &str, into: &mut HashSet<String>) {
    for (kind, piece) in tokens(text) {
        match kind {
            Tok::Ident => {
                into.insert(piece.to_string());
            }
            Tok::Directive => {
                // Without its `#`, it is cut up like any other C.
                let rest = piece.trim_start().trim_start_matches('#');
                for (kind, word) in tokens(rest) {
                    if kind == Tok::Ident {
                        into.insert(word.to_string());
                    }
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------- Marked parts

/// The runtime with its marked parts decided.
///
/// `known` says, for each name, whether it counts as defined. A part
/// between `#ifdef NAME` (or `#ifndef NAME`) and its `#endif` is then
/// kept or left out, with the lines of the mark itself always left out,
/// and an `#else` turns the choice over. A mark for any other name
/// (`#ifdef _WIN32`) is left as it is, for the C compiler to decide.
pub fn resolved(text: &str, known: &[(&str, bool)]) -> String {
    /// One `#if` that is open: is it one of ours, and if so, is what it
    /// holds being kept?
    enum Open {
        Ours(bool),
        Theirs,
    }
    let mut out = String::with_capacity(text.len());
    let mut open: Vec<Open> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let keeping = |open: &[Open]| open.iter().all(|part| !matches!(part, Open::Ours(false)));
        let asked = |prefix: &str| {
            let name = trimmed.strip_prefix(prefix)?.trim();
            known.iter().find(|(known, _)| *known == name).map(|(_, defined)| *defined)
        };
        if let Some(defined) = asked("#ifdef ") {
            open.push(Open::Ours(defined));
            continue;
        }
        if let Some(defined) = asked("#ifndef ") {
            open.push(Open::Ours(!defined));
            continue;
        }
        if trimmed.starts_with("#if") {
            if keeping(&open) {
                out += line;
                out.push('\n');
            }
            open.push(Open::Theirs);
            continue;
        }
        if trimmed.starts_with("#else") {
            if let Some(Open::Ours(kept)) = open.last_mut() {
                *kept = !*kept;
                continue;
            }
        }
        if trimmed.starts_with("#endif") {
            if let Some(Open::Ours(_)) = open.pop() {
                continue;
            }
            // One of theirs: its `#endif` is kept if it was.
        }
        if keeping(&open) {
            out += line;
            out.push('\n');
        }
    }
    out
}

/// The runtime with no place in the source handed to anything.
///
/// A function of the runtime that can fail ends its parameters with
/// `int line, int col`, and passes them on as `line, col`. With the
/// checks gone nothing reads them, so both are taken out, here and (by
/// the emitter) in the program's calls. The lines that only say a
/// parameter is not used, `(void)line;`, go with them.
pub fn without_positions(text: &str) -> String {
    let unused: String =
        text.lines().filter(|line| !matches!(line.trim(), "(void)line;" | "(void)col;")).flat_map(|line| [line, "\n"]).collect();
    let toks = tokens(&unused);
    // Does `pattern` begin at `from`, with blanks and comments between
    // its pieces? Gives where it ends.
    let matches = |from: usize, pattern: &[&str]| {
        let mut at = from;
        for (index, wanted) in pattern.iter().enumerate() {
            if index > 0 {
                while toks.get(at).is_some_and(|(kind, _)| matches!(kind, Tok::Space | Tok::Comment)) {
                    at += 1;
                }
            }
            if toks.get(at).map(|(_, piece)| *piece) != Some(*wanted) {
                return None;
            }
            at += 1;
        }
        Some(at)
    };
    let mut out = String::with_capacity(unused.len());
    let mut i = 0;
    while i < toks.len() {
        if let Some(end) = matches(i, &[",", "int", "line", ",", "int", "col"]).or_else(|| matches(i, &[",", "line", ",", "col"])) {
            // `(a, b, int line, int col)` is `(a, b)`.
            i = end;
        } else if let Some(end) = matches(i, &["(", "int", "line", ",", "int", "col", ")"]) {
            out += "(void)";
            i = end;
        } else if let Some(end) = matches(i, &["(", "line", ",", "col", ")"]) {
            out += "()";
            i = end;
        } else {
            out += toks[i].1;
            i += 1;
        }
    }
    out
}

// -------------------------------------------------- Cutting C into items

/// One thing at the top of a C file: a function, a type, a variable, a
/// `#define`, an `#include`.
struct Item {
    text: String,
    /// The names that ask for it. For most items that is the one name it
    /// declares. See `names_of` for the few that are kept another way.
    names: Vec<String>,
    /// Every name it mentions.
    uses: HashSet<String>,
    /// The `#if` parts it is inside, outermost first, by their numbers.
    inside: Vec<usize>,
    /// An `#include` of a header this file does not know the names of.
    /// It is kept when the `#if` part it is in is.
    with_its_part: bool,
}

/// The top of a C file is items, and the lines that open, turn and close
/// an `#if` part around some of them.
enum Entry {
    Item(Item),
    /// `#if`, `#ifdef`, `#ifndef`: the line, the number of the part it
    /// opens, and the names the line asks about.
    Open(String, usize, HashSet<String>),
    /// `#else` or `#elif`.
    Turn(String, usize, HashSet<String>),
    /// `#endif`.
    Close(String, usize),
}

/// The names a C header brings, as far as the runtime and a compiled
/// program use them. An `#include` is kept when one of them is used.
const HEADERS: &[(&str, &[&str])] = &[
    ("errno.h", &["errno", "ENOENT"]),
    ("inttypes.h", &["PRIx64", "PRId64", "PRIu64"]),
    (
        "math.h",
        &[
            "pow", "floor", "ceil", "round", "trunc", "sqrt", "fabs", "fmod", "exp", "log", "log2", "log10", "sin", "cos", "tan",
            "asin", "acos", "atan", "atan2", "hypot", "INFINITY", "NAN", "isnan", "isinf",
        ],
    ),
    ("setjmp.h", &["jmp_buf", "setjmp", "longjmp", "_setjmp", "_longjmp"]),
    ("stdarg.h", &["va_list", "va_start", "va_arg", "va_end"]),
    ("stdbool.h", &["bool", "true", "false"]),
    ("stddef.h", &["size_t", "NULL", "offsetof", "ptrdiff_t", "wchar_t"]),
    (
        "stdint.h",
        &[
            "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "uintptr_t", "intptr_t",
            "INT64_C", "UINT64_C", "UINT32_C", "INT64_MIN", "INT64_MAX", "UINT32_MAX", "UINT64_MAX", "UINTPTR_MAX", "SIZE_MAX",
        ],
    ),
    ("stdio.h", STDIO),
    ("stdlib.h", &["malloc", "calloc", "realloc", "free", "exit", "abort", "atexit", "strtod", "atoi", "llabs"]),
    (
        "string.h",
        &["strlen", "strcmp", "strncmp", "strchr", "strrchr", "strstr", "strpbrk", "memcpy", "memmove", "memset", "memcmp", "memchr"],
    ),
    ("signal.h", &["sigaction", "sigemptyset", "siginfo_t", "SIGSEGV", "SIGBUS", "SA_SIGINFO", "SA_NODEFER", "signal", "SIG_DFL"]),
    ("sys/mman.h", &["mmap", "munmap", "mprotect", "PROT_READ", "PROT_WRITE", "MAP_PRIVATE", "MAP_ANONYMOUS", "MAP_ANON", "MAP_FAILED"]),
    ("time.h", &["nanosleep", "clock_gettime", "timespec", "time_t", "CLOCK_MONOTONIC"]),
    ("malloc.h", &["_aligned_malloc", "_aligned_free"]),
    ("fcntl.h", &["_O_BINARY"]),
    ("io.h", &["_setmode", "_fileno", "_wfindfirst64", "_wfindnext64", "_findclose", "_wfinddata64_t"]),
    // The part of the runtime for folders (decisions 59.20 and 59.21).
    ("dirent.h", &["DIR", "opendir", "readdir", "closedir", "dirent"]),
    ("sys/stat.h", &["stat", "S_ISDIR", "mkdir", "_stat64", "_wstat64", "_S_IFDIR"]),
    ("direct.h", &["_wmkdir"]),
    ("wchar.h", &["wcscmp"]),
];

/// What `<stdio.h>` brings. A program that uses any of it reads or
/// writes, and on Windows that asks for one thing more: see `names_of`.
const STDIO: &[&str] = &[
    "printf", "fprintf", "sprintf", "snprintf", "puts", "putchar", "fputs", "fputc", "fgetc", "fflush", "fopen", "fclose", "fread",
    "fwrite", "fseek", "ftell", "ferror", "remove", "FILE", "stdin", "stdout", "stderr", "EOF", "SEEK_SET", "SEEK_END",
];

/// The functions Microsoft's C library gives a warning for. The two
/// `#define`s at the top of the runtime ask it not to, and are kept when
/// one of these is used.
const WARNED_OF: &[&str] = &["fopen", "sprintf", "strcpy", "strcat", "sscanf", "getenv"];

fn words(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// The names that ask for a directive at the top of the file.
fn names_of_directive(line: &str) -> (Vec<String>, bool) {
    let rest = line.trim_start().trim_start_matches('#').trim_start();
    if let Some(header) = rest.strip_prefix("include") {
        let header = header.trim().trim_start_matches(['<', '"']).trim_end_matches(['>', '"']);
        // The header of a board is where it says that it prints itself
        // (`ZENO_BOARD_PRINTS`), so whatever asks that asks for the header.
        if header == "ZENO_BOARD" {
            return (vec!["ZENO_BOARD_PRINTS".to_string()], true);
        }
        return match HEADERS.iter().find(|(name, _)| *name == header) {
            Some((_, brings)) => (words(brings), false),
            None => (Vec::new(), true),
        };
    }
    if let Some(defined) = rest.strip_prefix("define") {
        let name: String = defined.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        // `_CRT_SECURE_NO_WARNINGS` is not used by name. It is wanted by
        // a program that calls what the warnings are about.
        if name.starts_with("_CRT_") {
            return (words(WARNED_OF), false);
        }
        return (vec![name], false);
    }
    // The one `#error` of the runtime says that a board which stops
    // itself cannot write the report of `--memory`: it goes with the
    // report.
    if rest.starts_with("error") {
        return (vec!["zeno_memory_report".to_string()], false);
    }
    // `#pragma comment(lib, "shell32")` names the library that
    // `CommandLineToArgvW` is in.
    if rest.starts_with("pragma comment") {
        return (vec!["CommandLineToArgvW".to_string()], false);
    }
    // `#pragma STDC FP_CONTRACT OFF` and its like say how floats are
    // worked out: wanted by a program that has a float in it.
    if rest.starts_with("pragma") {
        return (vec!["double".to_string()], false);
    }
    (Vec::new(), true)
}

/// The names that ask for an item which is not a directive. See `Item`.
fn names_of(text: &str) -> Vec<String> {
    let toks: Vec<(Tok, &str)> =
        tokens(text).into_iter().filter(|(kind, _)| !matches!(kind, Tok::Space | Tok::Comment | Tok::Directive)).collect();
    let is = |at: usize, piece: &str| toks.get(at).is_some_and(|(_, found)| *found == piece);
    let ident = |at: usize| toks.get(at).filter(|(kind, _)| *kind == Tok::Ident).map(|(_, piece)| piece.to_string());
    // On Windows the program says, before `main`, that its bytes are to
    // be left alone (see the top of runtime.c). Nothing calls that
    // function: it is wanted by any program that reads or writes.
    if text.contains("__attribute__((constructor))") {
        return words(STDIO);
    }
    if is(0, "typedef") {
        // `typedef void (*zeno_code)(void);` names what is after `(*`.
        // Any other names the last word before its `;`.
        // A struct may have such a thing among its fields, and is
        // still named by its last word.
        let is_struct = toks.iter().any(|(_, piece)| *piece == "{");
        for at in 0..toks.len() {
            if !is_struct && is(at, "(") && is(at + 1, "*") {
                return ident(at + 2).into_iter().collect();
            }
        }
        return (0..toks.len()).rev().find_map(ident).into_iter().collect();
    }
    let mut at = 0;
    // `__attribute__((constructor))` and `__declspec(dllimport)` come
    // first. (`ZENO_NORETURN` in front of a function is one word, and
    // is passed over like `static`.)
    while is(at, "__attribute__") || is(at, "__declspec") {
        at += 1;
        let mut depth = 0;
        while at < toks.len() {
            depth += is(at, "(") as i32 - is(at, ")") as i32;
            at += 1;
            if depth == 0 {
                break;
            }
        }
    }
    // The name is the last word before the first `(`, `=`, `[` or `;`:
    // `static void zeno_free(void *memory) {`, `static size_t zeno_top =
    // 0;`. For `struct z_Point { ... };` it is the word before the `{`.
    let (mut last, mut before_braces) = (None, None);
    while at < toks.len() {
        if is(at, "{") {
            before_braces = last.take();
            let mut depth = 0;
            while at < toks.len() {
                depth += is(at, "{") as i32 - is(at, "}") as i32;
                at += 1;
                if depth == 0 {
                    break;
                }
            }
            continue;
        }
        if is(at, "(") || is(at, "=") || is(at, "[") || is(at, ";") {
            break;
        }
        if let Some(name) = ident(at) {
            last = Some(name);
        }
        at += 1;
    }
    last.or(before_braces).into_iter().collect()
}

/// Does the `#if` at `at` begin an item, and not a part that holds whole
/// items? The runtime has two such:
///
/// ```c
/// #ifndef ZENO_PLAIN_C
/// __attribute__((noinline))
/// #endif
/// static void zeno_mark_stack(void) {
/// ```
///
/// What is between such an `#if` and its `#endif` does not end where an
/// item ends, at a `;` or a `}`.
fn begins_an_item(toks: &[(Tok, &str)], at: usize) -> bool {
    let word = |piece: &str| piece.trim_start().trim_start_matches('#').trim_start().to_string();
    if !word(toks[at].1).starts_with("if") {
        return false;
    }
    let mut depth = 0;
    let mut last = "";
    for (kind, piece) in &toks[at..] {
        match kind {
            Tok::Directive if word(piece).starts_with("if") => depth += 1,
            Tok::Directive if word(piece).starts_with("endif") => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Tok::Directive | Tok::Space | Tok::Comment => {}
            _ => last = piece,
        }
    }
    !matches!(last, "" | ";" | "}")
}

/// Cut the top of a C file into its entries. Comments between items are
/// left out; one inside a function stays with it.
fn entries(text: &str) -> Vec<Entry> {
    let toks = tokens(text);
    let mut out = Vec::new();
    // The `#if` parts that are open, by number, and how many there have been.
    let mut open: Vec<usize> = Vec::new();
    let mut parts = 0;
    let mut i = 0;
    while i < toks.len() {
        let (kind, piece) = toks[i];
        match kind {
            Tok::Space | Tok::Comment => i += 1,
            Tok::Directive if !begins_an_item(&toks, i) => {
                i += 1;
                let rest = piece.trim_start().trim_start_matches('#').trim_start();
                let mut asked = HashSet::new();
                names_used(piece, &mut asked);
                if rest.starts_with("if") {
                    out.push(Entry::Open(piece.to_string(), parts, asked));
                    open.push(parts);
                    parts += 1;
                } else if rest.starts_with("el") {
                    out.push(Entry::Turn(piece.to_string(), *open.last().expect("an #else is inside an #if"), asked));
                } else if rest.starts_with("endif") {
                    out.push(Entry::Close(piece.to_string(), open.pop().expect("an #endif ends an #if")));
                } else {
                    let (names, with_its_part) = names_of_directive(piece);
                    // What the header of a board may count on without
                    // asking: the sized numbers, `bool`, `size_t`, and
                    // the number of the LED (docs/board.md).
                    if rest.starts_with("include") && asked.contains("ZENO_BOARD") {
                        asked.extend(words(&["uint32_t", "bool", "size_t", "ZENO_LED"]));
                    }
                    out.push(Entry::Item(Item { text: piece.to_string(), names, uses: asked, inside: open.clone(), with_its_part }));
                }
            }
            _ => {
                // An item ends at its `;`, or, if it is a function, at the
                // `}` that closes its body. It is a function if its first
                // `{` comes straight after a `)`.
                let start = i;
                let (mut braces, mut brackets) = (0, 0);
                let mut function = None;
                let mut before = "";
                while i < toks.len() {
                    let (kind, piece) = toks[i];
                    i += 1;
                    match (kind, piece) {
                        (Tok::Space | Tok::Comment | Tok::Directive, _) => continue,
                        (Tok::Punct, "{") => {
                            if braces == 0 && brackets == 0 && function.is_none() {
                                function = Some(before == ")");
                            }
                            braces += 1;
                        }
                        (Tok::Punct, "}") => {
                            braces -= 1;
                            if braces == 0 && brackets == 0 && function == Some(true) {
                                break;
                            }
                        }
                        (Tok::Punct, "(") => brackets += 1,
                        (Tok::Punct, ")") => brackets -= 1,
                        (Tok::Punct, ";") if braces == 0 && brackets == 0 => break,
                        _ => {}
                    }
                    before = piece;
                }
                let text: String = toks[start..i].iter().map(|(_, piece)| *piece).collect();
                let mut uses = HashSet::new();
                names_used(&text, &mut uses);
                out.push(Entry::Item(Item { names: names_of(&text), text, uses, inside: open.clone(), with_its_part: false }));
            }
        }
    }
    out
}

// ---------------------------------------------------------- Putting it together

/// What the emitter hands over: the pieces of a C file, as it would have
/// written them one after the other.
pub struct Parts<'a> {
    /// The name of the Zeno file, for the comment at the top.
    pub source: &'a str,
    /// The runtime for this kind of build, with the marks of
    /// `ZENO_FIRMWARE` and its like already decided.
    pub runtime: &'a str,
    /// The program's types and its top-level variables.
    pub declarations: Vec<&'a str>,
    /// Its functions, and the few variables that are written among them.
    pub functions: Vec<&'a str>,
    /// The statements at the top of the file, as C.
    pub program: &'a str,
    /// The build has a collector, if the program turns out to need one.
    pub collector: bool,
    /// The top-level variables the collector starts from.
    pub roots: &'a [String],
    /// The program asks for the words it was started with.
    pub arguments: bool,
    /// `--memory`: the program reports its memory as it ends.
    pub memory_report: bool,
    /// The libraries of C the program declares: their `#include` lines
    /// and declarations, written as they are after the runtime
    /// (decision 44.10). Empty for any other program.
    pub verbatim: &'a str,
}

/// The marks of the runtime that a minimal build decides, beside those of
/// its kind of build. `ZENO_UNCHECKED` is the mark of `--fast`, whose
/// checks are left out here too.
pub fn runtime(of_its_kind: &str, memory_report: bool) -> String {
    let known = [("ZENO_MINIMAL", true), ("ZENO_UNCHECKED", true), ("ZENO_MEMORY_REPORT", memory_report)];
    without_positions(&resolved(of_its_kind, &known))
}

/// Which entries of a file are kept: the ones the program reaches.
struct Kept<'a> {
    all: &'a [Entry],
    /// Which items each name asks for. A name can ask for several: a
    /// function declared in one place and written in another, or written
    /// twice, once for Windows and once for everything else.
    asked_by: HashMap<&'a str, Vec<usize>>,
    /// For each entry: is it kept?
    items: Vec<bool>,
    /// The `#if` parts that have something kept inside them.
    parts: HashSet<usize>,
    /// Names that are used, and whose items have not been looked at yet.
    wanted: Vec<String>,
    seen: HashSet<String>,
}

impl<'a> Kept<'a> {
    fn new(all: &'a [Entry]) -> Self {
        let mut asked_by: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, entry) in all.iter().enumerate() {
            if let Entry::Item(item) = entry {
                for name in &item.names {
                    asked_by.entry(name.as_str()).or_default().push(index);
                }
            }
        }
        Kept { all, asked_by, items: vec![false; all.len()], parts: HashSet::new(), wanted: Vec::new(), seen: HashSet::new() }
    }

    /// These names are used.
    fn want<'b>(&mut self, names: impl IntoIterator<Item = &'b String>) {
        for name in names {
            if self.seen.insert(name.clone()) {
                self.wanted.push(name.clone());
            }
        }
    }

    /// The names in this piece of C are used.
    fn want_all_in(&mut self, text: &str) {
        let mut names = HashSet::new();
        names_used(text, &mut names);
        self.want(&names);
    }

    /// Keep every item a wanted name asks for, and whatever those items
    /// name in their turn, until nothing new is named.
    fn follow(&mut self) {
        let all = self.all;
        while let Some(name) = self.wanted.pop() {
            for index in self.asked_by.get(name.as_str()).cloned().unwrap_or_default() {
                if std::mem::replace(&mut self.items[index], true) {
                    continue;
                }
                let Entry::Item(item) = &all[index] else { continue };
                self.want(&item.uses);
                // The `#if` parts around it are now written, so the
                // names their lines ask about are used as well.
                for part in &item.inside {
                    if !self.parts.insert(*part) {
                        continue;
                    }
                    for entry in all {
                        if let Entry::Open(_, number, asked) | Entry::Turn(_, number, asked) = entry {
                            if number == part {
                                self.want(asked);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Is something by this name kept?
    fn has(&self, name: &str) -> bool {
        self.asked_by.get(name).into_iter().flatten().any(|&index| self.items[index])
    }
}

/// The C written so far. One-line items stand together, and anything
/// longer has an empty line before it and after it.
struct Written {
    c: String,
    last_was_long: bool,
    /// How many `#if` parts what is written next is inside.
    depth: usize,
}

impl Written {
    fn item(&mut self, text: &str) {
        let long = text.trim_end().contains('\n');
        if long || self.last_was_long {
            self.c.push('\n');
        }
        self.c += text.trim_end();
        self.c.push('\n');
        self.last_was_long = long;
    }

    /// What comes next is another kind of thing: an empty line first.
    fn gap(&mut self) {
        self.last_was_long = true;
    }

    /// The kept entries of `all[from..to]`, in the order they stand in.
    fn entries(&mut self, kept: &Kept, from: usize, to: usize) {
        let all = kept.all;
        for index in from..to {
            match &all[index] {
                Entry::Item(item) if kept.items[index] => self.item(&item.text),
                // An `#if` part that is not inside another stands by
                // itself, with an empty line before it and after it.
                Entry::Open(line, part, _) if kept.parts.contains(part) => {
                    if self.depth == 0 {
                        self.gap();
                    }
                    self.depth += 1;
                    self.item(line);
                }
                Entry::Close(line, part) if kept.parts.contains(part) => {
                    self.depth -= 1;
                    self.last_was_long = false;
                    self.item(line);
                    if self.depth == 0 {
                        self.gap();
                    }
                }
                Entry::Turn(line, part, _) if kept.parts.contains(part) => {
                    // An `#else` with nothing kept after it is not written.
                    let branch = all[index + 1..]
                        .iter()
                        .take_while(|later| !matches!(later, Entry::Turn(_, number, _) | Entry::Close(_, number) if number == part))
                        .count();
                    if (index + 1..index + 1 + branch).any(|later| kept.items[later]) {
                        self.item(line);
                    }
                }
                _ => {}
            }
        }
    }
}

/// One C file for a minimal build: what the program uses, and no more.
pub fn assemble(parts: Parts) -> String {
    let mut all = entries(&runtime(parts.runtime, parts.memory_report));
    let runtime_end = all.len();
    all.extend(entries(&parts.declarations.join("\n")));
    let declared = all.len();
    all.extend(entries(&parts.functions.join("\n")));

    // Start from what the top-level code names, and keep following.
    let mut kept = Kept::new(&all);
    kept.want_all_in(parts.program);
    kept.want_all_in(parts.verbatim);
    if parts.arguments {
        kept.want_all_in("zeno_program_start");
    }
    if parts.memory_report {
        kept.want_all_in("atexit(zeno_memory_report);");
    }
    kept.follow();

    // The collector finds the program's variables on the stack, between
    // where `main` began and where the program is now. So a program with
    // a collector in it keeps its top-level code in a function of its
    // own, below `main`, and tells the collector where its other
    // variables are. A program that never asks for memory has no
    // collector in it, and its top-level code is `main` itself.
    let collects = parts.collector && kept.has("zeno_stack_base");
    let mut roots = String::new();
    if collects {
        roots += "static void zeno_roots(void) {\n    zeno_mark_words(&zeno_raised, sizeof zeno_raised);\n";
        for root in parts.roots {
            roots += &format!("    zeno_mark_words(&{root}, sizeof {root});\n");
        }
        roots += "}\n";
        kept.want_all_in(&roots);
        // The top of the program is a function of its own, kept out of
        // `main` in the compiler's word (decision 55.4).
        kept.want_all_in("ZENO_NOINLINE ZENO_PLAIN_C");
        kept.follow();
    }
    // An `#include` of a header whose names are not known here is kept
    // with the `#if` part it is in.
    for (index, entry) in all.iter().enumerate() {
        if let Entry::Item(item) = entry {
            if item.with_its_part && item.inside.last().is_some_and(|part| kept.parts.contains(part)) {
                kept.items[index] = true;
            }
        }
    }

    let mut written = Written {
        c: format!(
            "/* Written by `zeno build --minimal` from {}.\n *\n * The least C that runs the program. Nothing in it checks an index, an\n * overflow or a division by zero: a mistake of that kind does whatever\n * the C does. `zeno build` without `--minimal` writes the C that checks. */\n",
            parts.source
        ),
        last_was_long: true,
        depth: 0,
    };
    written.entries(&kept, 0, runtime_end);
    written.gap();
    if !parts.verbatim.is_empty() {
        written.c += parts.verbatim;
        written.gap();
    }
    written.entries(&kept, runtime_end, declared);

    // The functions. One is declared ahead only if something written
    // before it names it, as a person writing C would do it.
    let functions: Vec<usize> = (declared..all.len()).filter(|&index| kept.items[index]).collect();
    let mut named_so_far: HashSet<&String> = HashSet::new();
    for (index, entry) in all.iter().enumerate().take(declared).skip(runtime_end) {
        if let Entry::Item(item) = entry {
            if kept.items[index] {
                named_so_far.extend(&item.uses);
            }
        }
    }
    written.gap();
    for &index in &functions {
        let Entry::Item(item) = &all[index] else { continue };
        // The first line of a function, which is all of it up to its
        // `{`. A variable has none, and stands where it was written.
        let first_line = item.text.split_once(" {\n").map(|(first_line, _)| first_line).filter(|line| line.ends_with(')'));
        if let Some(first_line) = first_line {
            if item.names.iter().any(|name| named_so_far.contains(name)) {
                written.item(&format!("{first_line};"));
            }
        }
        named_so_far.extend(&item.uses);
    }
    written.gap();
    for &index in &functions {
        let Entry::Item(item) = &all[index] else { continue };
        written.item(&item.text);
    }
    let mut c = written.c;

    let main_line = if parts.arguments { "int main(int argc, char **argv) {\n" } else { "int main(void) {\n" };
    let mut start = String::new();
    if parts.memory_report {
        start += "    atexit(zeno_memory_report);\n";
    }
    if parts.arguments {
        start += "    zeno_program_start(argc, argv);\n";
    }
    c.push('\n');
    if collects {
        c += "ZENO_NOINLINE static void zeno_program(void) {\n";
        c += parts.program;
        c += "}\n\n";
        c += &roots;
        c.push('\n');
        c += main_line;
        c += "    void *base = NULL;\n    zeno_stack_base = &base; /* the stack starts here */\n";
        c += &start;
        c += "#ifndef ZENO_PLAIN_C\n";
        c += "    zeno_program();\n";
        c += "#else\n";
        c += "    void (*volatile program)(void) = zeno_program; /* called so, it stays out of `main` */\n";
        c += "    program();\n";
        c += "#endif\n";
    } else {
        c += main_line;
        c += &start;
        c += parts.program;
    }
    c += "    return 0;\n}\n";
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_is_cut_into_its_pieces_and_put_back() {
        let source = "/* a \"comment\" */\n#ifdef _WIN32\nstatic int x = 'a'; // it's one\n#endif\nconst char *s = \"/* not one */\";\n";
        let toks = tokens(source);
        assert_eq!(toks.iter().map(|(_, piece)| *piece).collect::<String>(), source);
        assert_eq!(toks[0], (Tok::Comment, "/* a \"comment\" */"));
        assert_eq!(toks[2], (Tok::Directive, "#ifdef _WIN32"));
        assert!(toks.contains(&(Tok::Text, "'a'")));
        assert!(toks.contains(&(Tok::Comment, "// it's one")));
        assert!(toks.contains(&(Tok::Text, "\"/* not one */\"")));
        // A `#` that is not first on its line is a sign like any other.
        assert!(!tokens("a = b # c;").iter().any(|(kind, _)| *kind == Tok::Directive));
    }

    #[test]
    fn a_marked_part_is_kept_or_left_out() {
        let source = "a\n#ifndef ZENO_MINIMAL\nchecked\n#else /* ZENO_MINIMAL */\nbare\n#endif /* ZENO_MINIMAL */\n#ifdef _WIN32\nw\n#else\nu\n#endif\nz\n";
        assert_eq!(resolved(source, &[("ZENO_MINIMAL", true)]), "a\nbare\n#ifdef _WIN32\nw\n#else\nu\n#endif\nz\n");
        assert_eq!(resolved(source, &[("ZENO_MINIMAL", false)]), "a\nchecked\n#ifdef _WIN32\nw\n#else\nu\n#endif\nz\n");
        // A part of theirs inside a part of ours that is left out goes with it.
        let nested = "#ifdef ZENO_MEMORY_REPORT\n#ifdef _WIN32\nw\n#else\nu\n#endif\n#endif\nz\n";
        assert_eq!(resolved(nested, &[("ZENO_MEMORY_REPORT", false)]), "z\n");
        assert_eq!(resolved(nested, &[("ZENO_MEMORY_REPORT", true)]), "#ifdef _WIN32\nw\n#else\nu\n#endif\nz\n");
    }

    #[test]
    fn the_place_in_the_source_is_taken_out_of_functions_and_calls() {
        let source = "static void f(int64_t a, int line, int col) {\n    (void)line;\n    (void)col;\n    g(a, \"text\",\n      line, col);\n}\nstatic void h(int line, int col) { f(1, line, col); k(line, col); }\nstatic bool p(const char *t, int line,\n              int col, int64_t *r);\n";
        let bare = "static void f(int64_t a) {\n    g(a, \"text\");\n}\nstatic void h(void) { f(1); k(); }\nstatic bool p(const char *t, int64_t *r);\n";
        assert_eq!(without_positions(source), bare);
        // A line of text being read is called `line` too, and is left alone.
        let reads = "zeno_line_add(&line, &used, &room, c);\n";
        assert_eq!(without_positions(reads), reads);
    }

    #[test]
    fn each_item_is_asked_for_by_its_name() {
        let one = |text: &str| names_of(text).join(" ");
        assert_eq!(one("static void zeno_free(void *memory) { }"), "zeno_free");
        assert_eq!(one("__attribute__((noreturn)) static void zeno_uncaught(void) { }"), "zeno_uncaught");
        assert_eq!(one("ZENO_NORETURN static void zeno_stop(int status) { }"), "zeno_stop");
        assert_eq!(one("static void zeno_roots(void);"), "zeno_roots");
        assert_eq!(one("static size_t zeno_pool_top = 0;"), "zeno_pool_top");
        assert_eq!(one("static const char *const *zeno_modules = NULL;"), "zeno_modules");
        assert_eq!(one("static zeno_spare *zeno_pool_spare[ZENO_POOL_STEPS];"), "zeno_pool_spare");
        assert_eq!(one("static union {\n    char bytes[ZENO_POOL];\n    uint64_t lined_up;\n} zeno_pool;"), "zeno_pool");
        assert_eq!(one("static struct { char *bytes; size_t size; } zeno_pieces[ZENO_PIECES];"), "zeno_pieces");
        assert_eq!(one("typedef struct {\n    int64_t len;\n} zeno_list;"), "zeno_list");
        assert_eq!(one("typedef struct zeno_try {\n    jmp_buf place;\n} zeno_try;"), "zeno_try");
        assert_eq!(one("typedef void (*zeno_code)(void);"), "zeno_code");
        assert_eq!(one("typedef struct zeno_closure {\n    void (*drop)(struct zeno_closure *);\n} zeno_closure;"), "zeno_closure");
        assert_eq!(one("typedef struct z_Point z_Point;"), "z_Point");
        assert_eq!(one("struct z_Point {\n    double z_x;\n};"), "z_Point");
        assert_eq!(one("static z_Shape zeno_case_Shape_Nothing = {.tag = 2};"), "zeno_case_Shape_Nothing");
        assert_eq!(one("__declspec(dllimport) void __stdcall Sleep(unsigned long milliseconds);"), "Sleep");
        assert_eq!(names_of_directive("#define ZENO_PAGE ((size_t)1 << ZENO_PAGE_BITS)").0, ["ZENO_PAGE"]);
        assert_eq!(names_of_directive("#define zeno_setjmp(place) _setjmp(place)").0, ["zeno_setjmp"]);
        assert!(names_of_directive("#include <math.h>").0.contains(&"floor".to_string()));
        // The header of a board comes with what is kept of the board,
        // and with whatever asks whether the board prints by itself.
        assert_eq!(names_of_directive("#include ZENO_BOARD"), (vec!["ZENO_BOARD_PRINTS".to_string()], true));
        assert_eq!(names_of_directive("#include \"vendor.h\""), (Vec::new(), true));
        assert_eq!(names_of_directive("#error \"no report\"").0, ["zeno_memory_report"]);
    }

    #[test]
    fn the_top_of_a_file_is_cut_into_items() {
        let source = "/* about f */\nstatic int f(void) {\n    if (1) { return 2; } /* it's here */\n    return 3;\n}\n\n#ifdef A\nstatic int a = 1; /* one */\n#else\nstatic int a[2] = {1, 2};\n#endif\ntypedef struct {\n#ifdef B\n    int b;\n#endif\n} t;\n";
        let found = entries(source);
        let shown: Vec<String> = found
            .iter()
            .map(|entry| match entry {
                Entry::Item(item) => format!("item {} in {:?}: {}", item.names.join(","), item.inside, item.text.lines().count()),
                Entry::Open(line, part, _) => format!("open {part}: {line}"),
                Entry::Turn(line, part, _) => format!("turn {part}: {line}"),
                Entry::Close(line, part) => format!("close {part}: {line}"),
            })
            .collect();
        assert_eq!(
            shown,
            [
                "item f in []: 4",
                "open 0: #ifdef A",
                "item a in [0]: 1",
                "turn 0: #else",
                "item a in [0]: 1",
                "close 0: #endif",
                "item t in []: 5",
            ]
        );
    }
}
