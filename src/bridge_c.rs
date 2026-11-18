//! `import c`: a library of C, or of Go or Rust built with a C
//! interface, used from `zeno run` (docs/c.md, decisions chapter 43).
//!
//! A program says which functions it uses, as C declares them:
//!
//! ```zeno
//! let z = c.library("z", "unsigned long crc32(unsigned long crc, const unsigned char *buf, unsigned int len);")
//! ```
//!
//! Zeno reads those declarations, and writes a small library of C of
//! its own, the stubs: each declared function wrapped in one that takes
//! its arguments as an array of slots, and a table of what each slot
//! holds. The C compiler builds the stubs with the library they call,
//! and the C helper (`bridge.c`), a program beside Zeno as Python is
//! for `import python`, loads them. So the C compiler, which knows how
//! each machine passes an `unsigned long` or a `float`, writes every
//! call, and Zeno only says what was declared.
//!
//! What is built is kept in the folder this machine has for what can be
//! made again (`registry::cache`), under a number worked out from what
//! it was built from, and the next run uses it as it is.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::token::Span;
use crate::value::*;

type Result<T = Value> = std::result::Result<T, Control>;

const HELPER: &str = include_str!("bridge.c");

// ----------------------------------------------------------- Declarations

/// A type as a declaration wrote it.
#[derive(Debug, Clone, PartialEq)]
struct Type {
    /// As C writes it: `const unsigned char *`.
    text: String,
    /// What is left with no `*` and no `const`: `unsigned char`.
    base: String,
    /// How many `*`.
    stars: usize,
    /// Whether what it points to is `const`.
    konst: bool,
}

/// One function, as it was declared.
#[derive(Debug, Clone, PartialEq)]
struct Declared {
    name: String,
    /// The declaration, with its spaces made single: how `print` shows
    /// the function.
    text: String,
    gives: Type,
    takes: Vec<Type>,
}

/// The words of C that are a type or part of one, and so are never the
/// name of a parameter.
const TYPE_WORDS: &[&str] = &[
    "void", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "_Bool", "bool", "const", "volatile",
    "restrict", "struct", "enum", "union", "__restrict", "__restrict__",
];

/// Comments taken out: `/* ... */` and `// ...`.
fn without_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
            out.push(' ');
        } else if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |end| &after[end..]);
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The words and stars of a type, in order: `const char *` is
/// `["const", "char", "*"]`.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        if c == '*' {
            out.push("*".to_string());
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// A type from its tokens.
fn type_of(tokens: &[String]) -> Type {
    let stars = tokens.iter().filter(|t| *t == "*").count();
    let last_star = tokens.iter().rposition(|t| t == "*");
    // `const char *` and `char const *` point to what is constant;
    // `char *const` is a pointer that is.
    let konst = match last_star {
        Some(last) => {
            let from = tokens[..last].iter().rposition(|t| t == "*").map_or(0, |at| at + 1);
            tokens[from..last].iter().any(|t| t == "const")
        }
        None => false,
    };
    let first_star = tokens.iter().position(|t| t == "*").unwrap_or(tokens.len());
    let base: Vec<&str> = tokens[..first_star]
        .iter()
        .map(String::as_str)
        .filter(|t| !matches!(*t, "const" | "volatile" | "restrict" | "register" | "__restrict" | "__restrict__"))
        .collect();
    let mut text = String::new();
    for token in tokens {
        if token == "*" {
            if !text.ends_with('*') {
                text.push(' ');
            }
            text.push('*');
        } else {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(token);
        }
    }
    Type { text, base: base.join(" "), stars, konst }
}

/// Is this word a name a program could give, and not a word of a type?
fn is_name(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && !TYPE_WORDS.contains(&word)
}

/// One parameter: its type, with its name taken off.
fn parameter(text: &str, function: &str) -> std::result::Result<Type, String> {
    let mut text = text.trim().to_string();
    if text.contains('(') {
        return Err(format!(
            "`{function}` takes a function (`{text}`), and a function of Zeno cannot be given to C yet"
        ));
    }
    // `int values[]` and `int values[8]` are pointers.
    let mut stars = 0;
    while let Some(open) = text.find('[') {
        let close = text[open..].find(']').map_or(text.len(), |at| open + at + 1);
        text.replace_range(open..close, "");
        stars += 1;
    }
    let mut tokens = tokens(&text);
    let words: Vec<usize> = (0..tokens.len()).filter(|&at| tokens[at] != "*").collect();
    if let [.., before, last] = words[..] {
        let follows_tag = matches!(tokens[before].as_str(), "struct" | "enum" | "union");
        if is_name(&tokens[last]) && !follows_tag && last == tokens.len() - 1 {
            tokens.pop();
        }
    }
    for _ in 0..stars {
        tokens.push("*".to_string());
    }
    if tokens.is_empty() {
        return Err(format!("a parameter of `{function}` has no type"));
    }
    Ok(type_of(&tokens))
}

/// One declaration, with no `;`.
fn declaration(text: &str) -> std::result::Result<Declared, String> {
    let shown = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.contains('{') {
        return Err(format!("`{shown}` has a body. `c.library` takes declarations, each ending with `;`"));
    }
    let Some(open) = text.find('(') else {
        return Err(format!("`{shown}` is not the declaration of a function: it has no `(`"));
    };
    let mut depth = 0;
    let mut close = None;
    for (at, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + at);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else { return Err(format!("`{shown}` has a `(` that is never closed")) };
    let head = tokens(&text[..open]);
    let Some(name) = head.last().filter(|word| is_name(word)).cloned() else {
        return Err(format!("`{shown}` has no name before its `(`"));
    };
    let mut gives: Vec<String> = head[..head.len() - 1].to_vec();
    gives.retain(|word| !matches!(word.as_str(), "extern" | "static" | "inline" | "__inline" | "__inline__"));
    if gives.is_empty() {
        return Err(format!("`{shown}` says nothing of what `{name}` gives. A function that gives nothing gives `void`"));
    }
    let inside = text[open + 1..close].trim();
    let mut takes = Vec::new();
    if !inside.is_empty() && inside != "void" {
        let mut depth = 0;
        let mut from = 0;
        let mut parts = Vec::new();
        for (at, c) in inside.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(&inside[from..at]);
                    from = at + 1;
                }
                _ => {}
            }
        }
        parts.push(&inside[from..]);
        for part in parts {
            if part.trim() == "..." {
                return Err(format!(
                    "`{name}` takes any number of arguments (`...`), and Zeno cannot know their types. Declare it with the ones the program gives, under another name of C, or write a function of C that calls it"
                ));
            }
            takes.push(parameter(part, &name)?);
        }
    }
    let gives = type_of(&gives);
    for of in std::iter::once(&gives).chain(&takes) {
        if of.stars == 0 && (of.base.starts_with("struct ") || of.base.starts_with("union ")) {
            return Err(format!(
                "`{name}` takes or gives a whole `{}`, not a pointer to one, and Zeno cannot hand C a struct yet. Write a function of C that takes a pointer to it",
                of.base
            ));
        }
    }
    Ok(Declared { name, text: shown, gives, takes })
}

/// Is this statement a type for the declarations that follow, and not a
/// function: `typedef struct counter counter`, `struct point { int x; }`,
/// `enum mode { OFF, ON }`? It is given to C as it is.
fn is_a_type(statement: &str) -> bool {
    let words = tokens(statement);
    match words.first().map(String::as_str) {
        Some("typedef") => true,
        Some("struct" | "union" | "enum") => match (statement.find('{'), statement.find('(')) {
            (_, None) => true,
            (Some(brace), Some(open)) => brace < open,
            (None, Some(_)) => false,
        },
        _ => false,
    }
}

/// All that a program gave `c.library`: the lines that begin with `#`
/// and the types, as they are, and each function.
fn declarations(text: &str) -> std::result::Result<(Vec<String>, Vec<Declared>), String> {
    let text = without_comments(text);
    let mut lines = Vec::new();
    let mut rest = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            lines.push(line.trim().to_string());
        } else {
            rest.push_str(line);
            rest.push('\n');
        }
    }
    let mut found: Vec<Declared> = Vec::new();
    let mut depth = 0;
    let mut from = 0;
    for (at, c) in rest.char_indices() {
        match c {
            '(' | '{' => depth += 1,
            ')' => depth -= 1,
            // A function with its body ends at its `}`, and is refused
            // for it; a struct ends at the `;` after its `}`.
            '}' => {
                depth -= 1;
                let one = rest[from..=at].trim();
                if depth == 0 && !is_a_type(one) {
                    declaration(one)?;
                }
            }
            ';' if depth == 0 => {
                let one = rest[from..at].trim();
                from = at + 1;
                if one.is_empty() {
                    continue;
                }
                if is_a_type(one) {
                    lines.push(format!("{};", one.split_whitespace().collect::<Vec<_>>().join(" ")));
                    continue;
                }
                let declared = declaration(one)?;
                if found.iter().any(|other| other.name == declared.name) {
                    return Err(format!("`{}` is declared twice", declared.name));
                }
                found.push(declared);
            }
            _ => {}
        }
    }
    let left = rest[from..].trim();
    if !left.is_empty() {
        let shown = left.split_whitespace().collect::<Vec<_>>().join(" ");
        return Err(format!("`{shown}` has no `;` after it. Each declaration ends with one"));
    }
    if found.is_empty() {
        return Err("`c.library` was given no declarations. It takes the functions the program uses, as C declares them: \"int abs(int n);\"".to_string());
    }
    Ok((lines, found))
}

// ----------------------------------------------------------- The stubs

/// A string of C that holds this text.
fn c_text(text: &str) -> String {
    let mut out = String::from("\"");
    for byte in text.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b' '..=b'~' => out.push(byte as char),
            other => out.push_str(&format!("\\{other:03o}")),
        }
    }
    out.push('"');
    out
}

/// What a slot of this type holds, as the table of the stubs says it.
fn kind(of: &Type) -> String {
    if of.stars == 0 {
        if of.base == "void" {
            return "{'v', 0, 0, 0, 0}".to_string();
        }
        return format!("{{ZCODE({0}), ZSIZE({0}), 0, 0, 0}}", of.base);
    }
    let konst = u8::from(of.konst);
    if of.stars == 1 && of.base == "char" {
        return format!("{{'s', sizeof(void *), {konst}, 'u', 1}}");
    }
    if of.stars > 1 {
        return format!("{{'p', sizeof(void *), {konst}, 'p', sizeof(void *)}}");
    }
    format!("{{'p', sizeof(void *), {konst}, ZCODE({0}), ZSIZE({0})}}", of.base)
}

/// The argument in slot `at`, as the type the function takes.
fn argument(of: &Type, at: usize) -> String {
    if of.stars > 0 {
        format!("({}) a[{at}].p", of.text)
    } else {
        format!("ZA({}, a[{at}])", of.text)
    }
}

/// The library of C that Zeno writes for a `c.library`.
fn stubs(lines: &[String], declared: &[Declared]) -> String {
    let mut out = String::from(
        "/* Written by Zeno for c.library (docs/c.md). */\n\
         #include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n",
    );
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("\n/* As the program declared them. */\n");
    for function in declared {
        out.push_str(&function.text);
        out.push_str(";\n");
    }
    out.push_str(STUB_TOP);
    for (number, function) in declared.iter().enumerate() {
        let args: Vec<String> = function.takes.iter().enumerate().map(|(at, of)| argument(of, at)).collect();
        let call = format!("{}({})", function.name, args.join(", "));
        let body = if function.gives.stars > 0 {
            format!("r->p = (void *) {call};")
        } else if function.gives.base == "void" {
            format!("(void) a; (void) r; {call};")
        } else {
            format!("ZR({}, r, {call});", function.gives.text)
        };
        out.push_str(&format!("static void zeno_call_{number}(zeno_slot *a, zeno_slot *r) {{ (void) a; {body} }}\n"));
        let kinds: Vec<String> = std::iter::once(&function.gives).chain(&function.takes).map(kind).collect();
        out.push_str(&format!("static const zeno_kind zeno_kinds_{number}[] = {{{}}};\n", kinds.join(", ")));
        let types: Vec<String> = std::iter::once(&function.gives).chain(&function.takes).map(|of| c_text(&of.text)).collect();
        out.push_str(&format!("static const char *const zeno_types_{number}[] = {{{}}};\n", types.join(", ")));
    }
    out.push_str("static const zeno_function zeno_functions[] = {\n");
    for (number, function) in declared.iter().enumerate() {
        out.push_str(&format!(
            "    {{{}, {}, {}, zeno_kinds_{number}, zeno_types_{number}, zeno_call_{number}}},\n",
            c_text(&function.name),
            c_text(&function.text),
            function.takes.len()
        ));
    }
    out.push_str(&format!(
        "}};\nZENO_EXPORT const zeno_table *zeno_library(void) {{\n    static const zeno_table table = {{{}, zeno_functions}};\n    return &table;\n}}\n",
        declared.len()
    ));
    out
}

/// What every set of stubs begins with. The four types are the ones
/// `bridge.c` reads. What a type is (a float, a whole number with a
/// sign or without, its size) is worked out by the C compiler, from the
/// type the declaration names, so that a name a header gives a type
/// (`uLong`, `size_t`) is known as well as `int` is.
const STUB_TOP: &str = r#"
typedef union { int64_t i; uint64_t u; double d; void *p; } zeno_slot;
typedef struct { char code; unsigned char size; unsigned char konst; char pointee; unsigned char pointee_size; } zeno_kind;
typedef struct {
    const char *name;
    const char *declared;
    int arity;
    const zeno_kind *kinds;
    const char *const *types;
    void (*call)(zeno_slot *, zeno_slot *);
} zeno_function;
typedef struct { int count; const zeno_function *functions; } zeno_table;

#ifdef _WIN32
#define ZENO_EXPORT __declspec(dllexport)
#else
#define ZENO_EXPORT __attribute__((visibility("default")))
#endif

#define ZCODE(T) _Generic((T *) 0, \
    _Bool *: 'b', char *: ((char) -1 < 0 ? 'i' : 'u'), signed char *: 'i', unsigned char *: 'u', \
    short *: 'i', unsigned short *: 'u', int *: 'i', unsigned int *: 'u', long *: 'i', unsigned long *: 'u', \
    long long *: 'i', unsigned long long *: 'u', float *: 'f', double *: 'f', long double *: 'f', default: 'o')
#define ZSIZE(T) _Generic((T *) 0, \
    _Bool *: sizeof(_Bool), char *: 1, signed char *: 1, unsigned char *: 1, \
    short *: sizeof(short), unsigned short *: sizeof(short), int *: sizeof(int), unsigned int *: sizeof(int), \
    long *: sizeof(long), unsigned long *: sizeof(long), long long *: sizeof(long long), \
    unsigned long long *: sizeof(long long), float *: 4, double *: 8, long double *: 8, default: 0)
#define ZA(T, s) _Generic((T *) 0, float *: (T) (s).d, double *: (T) (s).d, long double *: (T) (s).d, default: (T) (s).u)
#define ZR(T, r, e) _Generic((T *) 0, \
    float *: (void) ((r)->d = (e)), double *: (void) ((r)->d = (e)), long double *: (void) ((r)->d = (double) (e)), \
    default: (void) ((r)->u = (uint64_t) (e)))

"#;

// ----------------------------------------------------------- Building

/// The C compilers to try: `CC`, as for `zeno build`, or else the first
/// of these that is installed.
fn compilers() -> Vec<String> {
    match std::env::var("CC") {
        Ok(named) if !named.is_empty() => vec![named],
        _ => vec!["cc".to_string(), "gcc".to_string(), "clang".to_string()],
    }
}

fn number_of(parts: &[&[u8]]) -> u64 {
    let mut number: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.iter().chain(&[0]) {
            number = (number ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    number
}

fn cache() -> PathBuf {
    crate::registry::cache().unwrap_or_else(|_| std::env::temp_dir().join("zeno-cache"))
}

/// A folder for one build, with a name no other run has.
fn making(base: &Path, name: &str) -> PathBuf {
    use std::hash::{BuildHasher, Hasher};
    let word = std::collections::hash_map::RandomState::new().build_hasher().finish();
    base.join(format!("{name}.{}.{word:016x}", std::process::id()))
}

/// Run a compiler, quietly. What it said, if it failed; `None` if there
/// is no such program.
fn compile(compiler: &str, args: &[String]) -> Option<std::result::Result<(), String>> {
    let output = Command::new(compiler).args(args).stdin(Stdio::null()).output().ok()?;
    if output.status.success() {
        Some(Ok(()))
    } else {
        Some(Err(String::from_utf8_lossy(&output.stderr).into_owned()))
    }
}

/// Make `from` the folder `to`, which another run may have made first.
fn settle(from: &Path, to: &Path, file: &str) -> PathBuf {
    if std::fs::rename(from, to).is_ok() {
        return to.join(file);
    }
    if to.join(file).is_file() {
        let _ = std::fs::remove_dir_all(from);
        return to.join(file);
    }
    from.join(file)
}

/// The C helper, built once with the C compiler and kept.
pub fn helper() -> std::result::Result<String, String> {
    let file = if cfg!(windows) { "zeno-c-helper.exe" } else { "zeno-c-helper" };
    let compilers = compilers();
    let base = cache();
    for compiler in &compilers {
        let name = format!("c-bridge-{:016x}", number_of(&[HELPER.as_bytes(), compiler.as_bytes()]));
        let built = base.join(&name);
        if built.join(file).is_file() {
            return Ok(built.join(file).to_string_lossy().into_owned());
        }
        let folder = making(&base, &name);
        if std::fs::create_dir_all(&folder).is_err() || std::fs::write(folder.join("bridge.c"), HELPER).is_err() {
            return Err(format!("cannot write the C helper in `{}`", base.display()));
        }
        let mut args = vec!["-O2".to_string(), "-o".to_string(), folder.join(file).to_string_lossy().into_owned()];
        args.push(folder.join("bridge.c").to_string_lossy().into_owned());
        let libraries: Vec<Vec<String>> = if cfg!(windows) {
            vec![vec!["-lws2_32".to_string()]]
        } else {
            // `dlopen` is in `libdl` on older systems, and in the C
            // library itself on newer ones.
            vec![vec!["-ldl".to_string()], vec![]]
        };
        let mut said = String::new();
        for extra in libraries {
            let all: Vec<String> = args.iter().cloned().chain(extra).collect();
            match compile(compiler, &all) {
                // There is no such compiler: the next is tried.
                None => break,
                Some(Ok(())) => {
                    let _ = std::fs::remove_file(folder.join("bridge.c"));
                    return Ok(settle(&folder, &built, file).to_string_lossy().into_owned());
                }
                Some(Err(text)) => said = text,
            }
        }
        let _ = std::fs::remove_dir_all(&folder);
        if !said.is_empty() {
            return Err(format!("`{compiler}` could not build the C helper. That is a bug in Zeno. It said:\n{}", first_lines(&said, 12)));
        }
    }
    Err(format!(
        "there is no C compiler to build with. Zeno looked for `{}`. `CC` can name one",
        compilers.join("`, then `")
    ))
}

fn first_lines(text: &str, count: usize) -> String {
    text.lines().take(count).collect::<Vec<_>>().join("\n")
}

/// What a word given as the library is: a library by its name, a file,
/// or a word for the compiler.
enum Part {
    /// `"z"`: `-lz`.
    Named(String),
    /// A library, an object file or C source, by its path.
    File(PathBuf),
    /// `"-lpthread"`, as it is.
    Flag(String),
}

fn parts(words: &[String], home: &Path, span: Span) -> Result<Vec<Part>> {
    let mut out = Vec::new();
    for word in words {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        if word.starts_with('-') {
            out.push(Part::Flag(word.to_string()));
            continue;
        }
        let lower = word.to_ascii_lowercase();
        let is_file = word.contains('/')
            || word.contains('\\')
            || [".c", ".so", ".a", ".o", ".dylib", ".dll", ".lib", ".obj"].iter().any(|end| lower.ends_with(end))
            || lower.contains(".so.");
        if !is_file {
            out.push(Part::Named(word.to_string()));
            continue;
        }
        let path = home.join(word);
        if !path.is_file() {
            return raise("FileError", format!("there is no file `{word}`, from the folder `{}`", home.display()), span);
        }
        out.push(Part::File(path.canonicalize().unwrap_or(path)));
    }
    Ok(out)
}

/// What a value given to `c.library` says, as words: a str, or a list
/// of them.
fn words(value: &Value, what: &str, span: Span) -> Result<Vec<String>> {
    match value {
        Value::Str(text) => Ok(vec![text.to_string()]),
        Value::List(items) => items
            .borrow()
            .iter()
            .map(|item| match item {
                Value::Str(text) => Ok(text.to_string()),
                other => raise("Error", format!("{what} is a str or a list of str, and this list holds {}", other.type_name()), span),
            })
            .collect(),
        Value::None if what.starts_with("the library") => Ok(Vec::new()),
        other => raise("Error", format!("{what} is a str or a list of str, and this is {}", other.type_name()), span),
    }
}

/// `c.library(library, declarations)`: the stubs built, and loaded.
#[cold]
#[inline(never)]
pub fn library(library: &Value, declared: &Value, span: Span) -> Result {
    let words_given = words(library, "the library given to `c.library`", span)?;
    let text = words(declared, "what `c.library` is given to declare", span)?.join("\n");
    let (lines, functions) = match declarations(&text) {
        Ok(found) => found,
        Err(why) => return raise("ValueError", why, span),
    };
    let home = crate::bridge::home().or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("."));
    let parts = parts(&words_given, &home, span)?;
    let label = words_given.first().cloned().unwrap_or_else(|| "c".to_string());
    let source = stubs(&lines, &functions);

    let file = if cfg!(windows) { "zeno-library.dll" } else { "zeno-library.so" };
    let mut args: Vec<String> = Vec::new();
    let mut folders: Vec<PathBuf> = vec![home.clone()];
    let mut said: Vec<Vec<u8>> = vec![source.clone().into_bytes()];
    for part in &parts {
        match part {
            Part::Named(name) => args.push(format!("-l{name}")),
            Part::Flag(flag) => args.push(flag.clone()),
            Part::File(path) => {
                args.push(path.to_string_lossy().into_owned());
                if let Some(folder) = path.parent() {
                    if !folders.iter().any(|f| f == folder) {
                        folders.push(folder.to_path_buf());
                    }
                }
                // A file that changed is built again.
                let changed = std::fs::metadata(path)
                    .ok()
                    .and_then(|data| data.modified().ok().map(|when| (data.len(), when)))
                    .map(|(size, when)| format!("{size} {when:?}"))
                    .unwrap_or_default();
                said.push(changed.into_bytes());
            }
        }
    }
    for arg in &args {
        said.push(arg.clone().into_bytes());
    }
    for folder in &folders {
        said.push(folder.to_string_lossy().into_owned().into_bytes());
    }

    let base = cache();
    let mut tried = Vec::new();
    for compiler in compilers() {
        let mut all: Vec<&[u8]> = said.iter().map(Vec::as_slice).collect();
        all.push(compiler.as_bytes());
        let name = format!("c-library-{:016x}", number_of(&all));
        let built = base.join(&name);
        if built.join(file).is_file() {
            return crate::bridge::c_library(&built.join(file).to_string_lossy(), &label, span);
        }
        let folder = making(&base, &name);
        if std::fs::create_dir_all(&folder).is_err() || std::fs::write(folder.join("zeno-library.c"), &source).is_err() {
            return raise("FileError", format!("cannot write what `c.library` builds, in `{}`", base.display()), span);
        }
        let mut command: Vec<String> = vec!["-shared".to_string()];
        if !cfg!(windows) {
            command.push("-fPIC".to_string());
        }
        command.extend(["-O1".to_string(), "-o".to_string(), folder.join(file).to_string_lossy().into_owned()]);
        command.push(folder.join("zeno-library.c").to_string_lossy().into_owned());
        // A header, or a library, beside the program or beside a file
        // it names, is found there.
        for folder in &folders {
            command.push(format!("-I{}", folder.display()));
            command.push(format!("-L{}", folder.display()));
        }
        command.extend(args.iter().cloned());
        if !cfg!(windows) {
            // Where a library that is a file of its own is found, when
            // the helper loads the stubs.
            for folder in &folders {
                command.push(format!("-Wl,-rpath,{}", folder.display()));
            }
        }
        match compile(&compiler, &command) {
            None => {
                let _ = std::fs::remove_dir_all(&folder);
                tried.push(compiler);
                continue;
            }
            Some(Ok(())) => {
                let path = settle(&folder, &built, file);
                return crate::bridge::c_library(&path.to_string_lossy(), &label, span);
            }
            Some(Err(text)) => {
                let _ = std::fs::remove_dir_all(&folder);
                return raise_build_error(&text, &folder, &label, span);
            }
        }
    }
    raise(
        "FileError",
        format!("there is no C compiler to build with. Zeno looked for `{}`. `CC` can name one", tried.join("`, then `")),
        span,
    )
}

/// What the C compiler said, as an error of Zeno.
fn raise_build_error(said: &str, folder: &Path, label: &str, span: Span) -> Result {
    for line in said.lines() {
        // GNU ld: "cannot find -lfoo"; the linker of macOS: "library
        // not found for -lfoo".
        for sign in ["cannot find -l", "library not found for -l"] {
            if let Some(at) = line.find(sign) {
                let name: String = line[at + sign.len()..].chars().take_while(|c| !c.is_whitespace() && *c != ':' && *c != '\'').collect();
                return raise(
                    "FileError",
                    format!(
                        "C has no library `{name}`. Is it installed, with what building with it needs (on Debian and Ubuntu, its package that ends in `-dev`)?"
                    ),
                    span,
                );
            }
        }
    }
    let shown = folder.join("zeno-library.c").to_string_lossy().into_owned();
    let said = said.replace(&shown, "the declarations");
    raise("Error", format!("C could not build what was declared for `{label}`. The C compiler said:\n{}", first_lines(said.trim(), 15)), span)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declaration_is_read_as_c_reads_it() {
        let (lines, found) = declarations(
            "#include <zlib.h>\n\
             /* the sum */ unsigned long crc32(unsigned long crc, const unsigned char *buf, unsigned int len);\n\
             int abs(int);   double frexp(double x, int *exponent); // two\n\
             void free(void *); char *getenv(const char *name); size_t strlen(const char*);\n\
             int main2(int argc, char *const argv[]); struct tm *gmtime(const time_t *t); const char *zlibVersion(void);\n\
             typedef struct counter counter; struct point { int x; int y; };\n\
             enum mode { OFF, ON }; typedef void (*handler)(int);",
        )
        .expect("they are declarations");
        // A type is given to C as it is, after the lines of `#`.
        assert_eq!(
            lines,
            [
                "#include <zlib.h>",
                "typedef struct counter counter;",
                "struct point { int x; int y; };",
                "enum mode { OFF, ON };",
                "typedef void (*handler)(int);"
            ]
        );
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["crc32", "abs", "frexp", "free", "getenv", "strlen", "main2", "gmtime", "zlibVersion"]);
        let crc = &found[0];
        assert_eq!(crc.gives.text, "unsigned long");
        assert_eq!(crc.takes.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["unsigned long", "const unsigned char *", "unsigned int"]);
        assert!(crc.takes[1].konst && crc.takes[1].stars == 1 && crc.takes[1].base == "unsigned char");
        assert_eq!(crc.text, "unsigned long crc32(unsigned long crc, const unsigned char *buf, unsigned int len)");
        assert_eq!(found[1].takes[0].text, "int");
        assert!(!found[2].takes[1].konst && found[2].takes[1].base == "int");
        assert_eq!(found[4].gives.text, "char *");
        assert_eq!(found[5].takes[0].text, "const char *");
        // `char *const argv[]`: the pointers are constant, and are what
        // the parameter points to.
        assert_eq!(found[6].takes[1].stars, 2);
        assert!(found[6].takes[1].konst);
        assert_eq!(found[7].gives.base, "struct tm");
        assert_eq!(found[7].takes[0].base, "time_t");
        assert!(found[8].takes.is_empty());
        assert!(found[8].gives.konst && found[8].gives.base == "char");

        for (wrong, says) in [
            ("int printf(const char *format, ...);", "any number of arguments"),
            ("void qsort(void *base, size_t n, size_t size, int (*compare)(const void *, const void *));", "a function of Zeno cannot be given"),
            ("int f(int x) { return x; }", "has a body"),
            ("int f(int x)", "has no `;`"),
            ("abs(int x);", "says nothing of what `abs` gives"),
            ("int x;", "not the declaration of a function"),
            ("struct point { int x; int y; }; struct point middle(struct point a);", "a whole `struct point`"),
            ("int f(void); long f(int);", "declared twice"),
            ("", "no declarations"),
        ] {
            let error = declarations(wrong).expect_err(wrong);
            assert!(error.contains(says), "{wrong}: {error}");
        }
    }
}
