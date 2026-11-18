//! Packages: modules that one program can take from another folder, or
//! from another person.
//!
//! A package is a folder of `.zeno` files with a file called
//! `zeno.package` in it. That file says what the package is called and
//! which other packages it needs:
//!
//! ```text
//! package thermostat
//!
//! needs filters in "../filters"
//! needs units from "https://github.com/someone/zeno-units" at "v1.2.0"
//! needs crc at "v2.0.1"
//! ```
//!
//! - `needs filters in "../filters"` uses the package in that folder,
//!   where it is. Nothing is copied.
//! - `needs units from "..." at "v1.2.0"` names a git repository and a
//!   tag, branch or commit in it. `zeno fetch` gets it into the folder
//!   `packages/units` beside this file, and writes the commit it got
//!   into `zeno.lock`, so that the next person gets exactly the same.
//! - `needs crc at "v2.0.1"` names a package by its name alone. The
//!   registry (`registry.rs`) knows which git repository `crc` is, and
//!   which commit of it is version 2.0.1. `zeno fetch` asks it, and
//!   then goes on as for the line above.
//!
//! This file reads `zeno.package` and `zeno.lock`, and is `zeno fetch`.
//! How `import` then finds the modules of a package is in `modules.rs`.
//! See docs/using-packages.md for a walk through both, and
//! docs/registry.md for the registry.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::builtins;
use crate::error::Error;
use crate::lexer;
use crate::modules::shown;
use crate::registry;
use crate::token::{Span, Token, TokenKind};

/// The file that makes a folder a package.
pub const FILE: &str = "zeno.package";
/// The folder, beside the program's `zeno.package`, that fetched
/// packages are kept in.
pub const FOLDER: &str = "packages";
/// The file `zeno fetch` writes: which commit each fetched package is.
pub const LOCK: &str = "zeno.lock";

/// What a `zeno.package` says.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub needs: Vec<Need>,
    /// `memory owned` or `memory collector`: how the program frees its
    /// memory when it is built, unless `zeno build` is told otherwise.
    /// `None` when the file does not say, which means `collector`.
    pub memory: Option<Memory>,
    /// `about "..."`: one line that says what the package is for. The
    /// registry shows it to somebody who searches, so a package has to
    /// have one to be published, and needs none otherwise.
    pub about: Option<String>,
    /// `registry "address"`: where the names of `needs units at "v1.2.0"`
    /// lines are looked up, when that is not Zeno's own registry. Like
    /// `memory`, it is the program's line that counts: the one in a
    /// needed package says nothing about the program that needs it.
    pub registry: Option<String>,
}

/// How a built program frees memory. docs/memory-modes.md compares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory {
    /// A garbage collector frees whatever nothing can reach any more.
    /// Lists and objects can be shared freely. `zeno build` with nothing
    /// said.
    Collector,
    /// Every list and object has one owner, and is freed on the line its
    /// owner ends, with no collector in the program: what Rust does, and
    /// what `--no-collector` does on a chip. On a computer the memory
    /// grows as the program needs it.
    Owned,
}


/// One `needs` line.
#[derive(Debug, Clone, PartialEq)]
pub struct Need {
    pub name: String,
    pub source: Source,
    /// Where the line is, for a message about it.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// `needs filters in "../filters"`: a folder on this machine.
    Folder(String),
    /// `needs units from "https://..." at "v1.2.0"`: a git repository,
    /// and a tag, branch or commit in it.
    Git { url: String, at: String },
    /// `needs crc at "v2.0.1"`: a package of the registry, by its name,
    /// and one version of it.
    Registry { at: String },
}

const LINES: &str = "A line of `zeno.package` is one of:\n    \
    package name\n    \
    about \"what the package is for, in one line\"\n    \
    memory owned            (or `memory collector`)\n    \
    needs name in \"folder\"\n    \
    needs name at \"v1.2.0\"   (a version of a package in the registry)\n    \
    needs name from \"address of a git repository\" at \"tag, branch or commit\"\n    \
    registry \"address\"      (a registry other than Zeno's own)";

/// Read a `zeno.package`. `file` is the file's number in the program,
/// which goes into the position of each mistake (see `Span`).
///
/// The words are split by Zeno's own lexer, so a comment starts with `#`
/// and a string is written as in a program.
pub fn parse(source: &str, file: usize) -> Result<Manifest, Vec<Error>> {
    let tokens = lexer::lex_in(source, file)?;
    let mut errors = Vec::new();
    let mut name: Option<String> = None;
    let mut needs: Vec<Need> = Vec::new();
    let mut memory: Option<Memory> = None;
    let mut about: Option<String> = None;
    let mut registry: Option<String> = None;

    for line in tokens.split(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Eof)) {
        let Some(first) = line.first() else { continue };
        let span = first.span;
        let words: Vec<&TokenKind> = line.iter().map(|token| &token.kind).collect();
        let word = |at: usize, wanted: &str| matches!(words.get(at), Some(TokenKind::Ident(found)) if found == wanted);
        match words.as_slice() {
            [_, TokenKind::Ident(package)] if word(0, "package") => {
                if name.is_some() {
                    errors.push(Error::new("a `zeno.package` says `package` once, on its first line", span));
                } else if !needs.is_empty() {
                    errors.push(Error::new("the `package` line comes first, above every `needs`", span));
                } else if builtins::comes_with_zeno(package) {
                    errors.push(Error::new(
                        format!("`{package}` is a module that comes with Zeno, so a package cannot have that name"),
                        span,
                    ));
                }
                name.get_or_insert(package.clone());
            }
            // `owned` is a word of the language, so it comes from the
            // lexer as a keyword, and `collector` as a name.
            [_, mode] if word(0, "memory") => {
                let said = match mode {
                    TokenKind::Owned => Memory::Owned,
                    TokenKind::Ident(word) if word == "collector" => Memory::Collector,
                    _ => {
                        errors.push(Error::new(
                            "a program frees its memory in one of two ways: `memory collector` (the garbage \
                             collector, as when nothing is said) or `memory owned` (one owner for each value, and \
                             no collector)",
                            span,
                        ));
                        continue;
                    }
                };
                if memory.is_some() {
                    errors.push(Error::new("a `zeno.package` says `memory` once", span));
                }
                memory.get_or_insert(said);
            }
            [_, TokenKind::Str { value, formatted: false }] if word(0, "about") => {
                if about.is_some() {
                    errors.push(Error::new("a `zeno.package` says `about` once", span));
                } else if let Err(why) = registry::about_fits(value) {
                    errors.push(Error::new(why, span));
                }
                about.get_or_insert(value.clone());
            }
            [_, TokenKind::Str { value, formatted: false }] if word(0, "registry") => {
                if registry.is_some() {
                    errors.push(Error::new("a `zeno.package` says `registry` once", span));
                } else if value.trim().is_empty() || value.contains(char::is_whitespace) {
                    errors.push(Error::new(
                        "`registry` is followed by the address of a git repository that is a registry, as in \
                         `registry \"https://git.example.com/zeno-packages\"`",
                        span,
                    ));
                }
                registry.get_or_insert(value.clone());
            }
            [_, TokenKind::Ident(need), rest @ ..] if word(0, "needs") => {
                let source = match rest {
                    [TokenKind::In, TokenKind::Str { value, formatted: false }] => Source::Folder(value.clone()),
                    [_, TokenKind::Str { value: at, formatted: false }] if word(2, "at") => {
                        if !registry::is_version(at) {
                            errors.push(Error::new(
                                format!(
                                    "`{at}` is not a version of a package in the registry. One is written as the \
                                     letter v and three numbers, as in `needs {need} at \"v1.2.0\"`. A branch or a \
                                     commit is of a git repository, which this line then names: \
                                     `needs {need} from \"address\" at \"{at}\"`"
                                ),
                                span,
                            ));
                            continue;
                        }
                        Source::Registry { at: at.clone() }
                    }
                    [] => {
                        errors.push(Error::new(
                            format!(
                                "this does not say where `{need}` is. A package in a folder: \
                                 `needs {need} in \"../{need}\"`. A package in the registry, and which version \
                                 of it: `needs {need} at \"v1.0.0\"` (`zeno search {need}` shows the versions there are)"
                            ),
                            span,
                        ));
                        continue;
                    }
                    [TokenKind::From, TokenKind::Str { value: url, formatted: false }, _, TokenKind::Str { value: at, formatted: false }]
                        if word(4, "at") =>
                    {
                        Source::Git { url: url.clone(), at: at.clone() }
                    }
                    [TokenKind::From, TokenKind::Str { .. }] => {
                        errors.push(Error::new(
                            format!(
                                "this does not say which version of `{need}` is needed. Add `at` and a tag, a branch \
                                 or a commit of that repository, as in `needs {need} from \"...\" at \"v1.0.0\"`"
                            ),
                            span,
                        ));
                        continue;
                    }
                    _ => {
                        errors.push(Error::new(format!("this `needs` line is not written as one is. {LINES}"), span));
                        continue;
                    }
                };
                if needs.iter().any(|other| other.name == *need) {
                    errors.push(Error::new(format!("`{need}` is needed twice. One line for each package"), span));
                } else if name.as_deref() == Some(need.as_str()) {
                    errors.push(Error::new(format!("the package `{need}` cannot need itself"), span));
                } else if builtins::comes_with_zeno(need) {
                    errors.push(Error::new(
                        format!("`{need}` is a module that comes with Zeno. `import {need}` gets it, with no `needs` line"),
                        span,
                    ));
                } else {
                    needs.push(Need { name: need.clone(), source, span });
                }
            }
            _ => errors.push(Error::new(format!("this is not a line of a `zeno.package`. {LINES}"), span)),
        }
    }
    let top = Span { line: file * crate::token::LINES_A_FILE + 1, col: 1 };
    match name {
        Some(name) if errors.is_empty() => Ok(Manifest { name, needs, memory, about, registry }),
        Some(_) => Err(errors),
        None => {
            errors.insert(
                0,
                Error::new("a `zeno.package` begins with the word `package` and the package's name, as in `package thermostat`", top),
            );
            Err(errors)
        }
    }
}

// ---------------------------------------------------------------- The lock

/// One line of `zeno.lock`: the commit a fetched package is pinned to.
#[derive(Debug, Clone, PartialEq)]
pub struct Pinned {
    pub name: String,
    pub url: String,
    pub at: String,
    pub commit: String,
    /// The `needs` line named no repository: the registry gave `url`
    /// and `commit` for the version `at`.
    pub registry: bool,
}

/// Read a `zeno.lock`. A line that is not one of its lines is left out:
/// the file is written by `zeno fetch`, and the worst a damaged one can
/// do is make the next `zeno fetch` look a version up again.
///
/// A line begins as the `needs` line it is for, without the `needs`, and
/// then says what `zeno fetch` found:
///
/// ```text
/// units from "https://example.com/units" at "v1.2.0" commit "9f8e7d6..."
/// crc at "v2.0.1" from "https://example.com/crc" commit "0123abc..."
/// ```
///
/// The second is for `needs crc at "v2.0.1"`: the address is something
/// the registry said, so it stands with what was found.
pub fn parse_lock(source: &str) -> Vec<Pinned> {
    let Ok(tokens) = lexer::lex(source) else { return Vec::new() };
    let text = |token: &Token| match &token.kind {
        TokenKind::Str { value, .. } => Some(value.clone()),
        _ => None,
    };
    let word = |token: &Token, wanted: &str| matches!(&token.kind, TokenKind::Ident(found) if found == wanted);
    let mut pinned = Vec::new();
    for line in tokens.split(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Eof)) {
        let [Token { kind: TokenKind::Ident(name), .. }, first, a, second, b, third, commit] = line else { continue };
        let from = |token: &Token| token.kind == TokenKind::From;
        let (url, at, registry) = match () {
            () if from(first) && word(second, "at") => (a, b, false),
            () if word(first, "at") && from(second) => (b, a, true),
            () => continue,
        };
        if !word(third, "commit") {
            continue;
        }
        if let (Some(url), Some(at), Some(commit)) = (text(url), text(at), text(commit)) {
            pinned.push(Pinned { name: name.clone(), url, at, commit, registry });
        }
    }
    pinned
}

/// A `zeno.lock`, as text.
pub fn write_lock(pinned: &[Pinned]) -> String {
    let mut text = String::from(
        "# Written by `zeno fetch`. Each line pins a package this program needs to one\n\
         # commit, so that everyone who builds the program builds the same thing.\n\
         # Keep this file with the program. `zeno fetch --update` writes it afresh.\n\n",
    );
    for package in pinned {
        let Pinned { name, url, at, commit, .. } = package;
        text += &match package.registry {
            false => format!("{name} from \"{url}\" at \"{at}\" commit \"{commit}\"\n"),
            true => format!("{name} at \"{at}\" from \"{url}\" commit \"{commit}\"\n"),
        };
    }
    text
}

// ----------------------------------------------------------------- zeno new

/// `zeno new blinker`: make the folder `blinker` with a program in it
/// that runs and has one test that passes. With `package`, a package:
/// a folder of modules for other programs to need. Gives the lines to
/// print, or what went wrong. Nothing is written into a folder that is
/// there already.
pub fn new(path: &Path, package: bool) -> Result<Vec<String>, String> {
    let what = if package { "package" } else { "program" };
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_string();
    if name.is_empty() {
        return Err("`zeno new` makes a new folder, so it needs a name for it, e.g. `zeno new blinker`".to_string());
    }
    if name.starts_with('-') {
        return Err(format!("`zeno new` has no option `{name}`. It has one, `--package`, as in `zeno new --package filters`"));
    }
    // The name is what other files write after `import`, so it has to
    // be a name of the language.
    let letters = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let starts = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !letters || !starts {
        let mut tidy: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        if !tidy.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            tidy = format!("my_{}", tidy.trim_start_matches('_'));
        }
        return Err(format!(
            "`{name}` cannot be the name of a {what}. A name is letters, digits and `_`, and does not start \
             with a digit, as every name in a Zeno program is. Try `{tidy}`"
        ));
    }
    if crate::token::keyword(&name).is_some() {
        return Err(format!("`{name}` is a word of the language, so a {what} cannot have that name"));
    }
    if builtins::comes_with_zeno(&name) {
        return Err(format!("`{name}` is a module that comes with Zeno, so a {what} cannot have that name"));
    }
    if path.exists() {
        return Err(format!(
            "there is a `{}` here already. `zeno new` makes a new folder, and writes nothing into one that is there",
            shown(&path)
        ));
    }

    let manifest = match package {
        false => format!(
            "# What this program is called, and the packages it needs.\n\
             #\n\
             # A package in another folder is one line:\n\
             #\n\
             #     needs filters in \"../filters\"\n\
             #\n\
             # One from the registry is named by its name and a version of it.\n\
             # `zeno search` lists what there is, and `zeno fetch` gets it:\n\
             #\n\
             #     needs units at \"v1.2.0\"\n\
             #\n\
             # One that is in no registry is named by its git repository:\n\
             #\n\
             #     needs units from \"https://github.com/someone/zeno-units\" at \"v1.2.0\"\n\
             package {name}\n"
        ),
        true => format!(
            "# This file makes the folder a package: a folder of modules that a\n\
             # program in another folder can use. A program that needs it says so\n\
             # in its own `zeno.package`:\n\
             #\n\
             #     needs {name} in \"../{name}\"\n\
             #\n\
             # and then writes `import {name}`.\n\
             package {name}\n"
        ),
    };
    let greeting = |first: &str| {
        format!(
            "{first}\n\
             function hello(who: str) -> str {{\n    return f\"Hello, {{who}}, from {name}.\"\n}}\n\n\
             # A test stands in the file it tests. `zeno test` runs it, `zeno run`\n\
             # goes past it, and a build leaves it out.\n\
             test \"hello says who it greets\" {{\n    assert hello(\"Ada\") == \"Hello, Ada, from {name}.\"\n}}\n"
        )
    };
    let at = |file: &str| shown(&path.join(file));
    let files: Vec<(&str, String, &str)> = match package {
        false => vec![
            (FILE, manifest, "the name of the program, and the packages it needs"),
            (
                "main.zeno",
                "# The program starts here. In this folder:\n#\n#     zeno run main.zeno\n#     zeno test\n\
                 import greeting\n\nprint(greeting.hello(\"world\"))\n"
                    .to_string(),
                "the program",
            ),
            (
                "greeting.zeno",
                greeting("# A module: one file of the program. `main.zeno` imports it, and calls\n# `greeting.hello`.\n"),
                "a module, with a test",
            ),
        ],
        true => vec![
            (FILE, manifest, "the name of the package, and the packages it needs"),
            (
                "module.zeno",
                greeting(&format!(
                    "# The module that has the package's own name. A program that needs the\n\
                     # package gets it with `import {name}`, and calls `{name}.hello`.\n"
                )),
                "the module `import` gets, with a test",
            ),
        ],
    };

    fs::create_dir_all(path).map_err(|why| format!("cannot make the folder `{}`: {why}", shown(&path)))?;
    let mut said = vec![format!("Made the {what} `{name}` in {}", at(""))];
    for (file, text, purpose) in files {
        // The module of a package is called what the package is called.
        let file = if file == "module.zeno" { format!("{name}.zeno") } else { file.to_string() };
        fs::write(path.join(&file), text).map_err(|why| format!("cannot write `{}`: {why}", at(&file)))?;
        said.push(format!("  {file:<width$}  {purpose}", width = name.len().max(8) + 5));
    }
    said.push(match package {
        false => format!("Run it with `zeno run {}`, and its tests with `zeno test {}`.", at("main.zeno"), shown(&path)),
        true => format!("Run its tests with `zeno test {}`.", shown(&path)),
    });
    Ok(said)
}

// --------------------------------------------------------------- zeno fetch

/// `zeno fetch`: get every package the program in `root` needs, and the
/// packages those need, and write `zeno.lock`. Gives the lines to print,
/// or what went wrong.
///
/// With `update`, the lock is not looked at: each `at` is looked up
/// again, which moves a package that follows a branch to the branch's
/// newest commit.
pub fn fetch(root: &Path, update: bool) -> Result<Vec<String>, String> {
    fetch_with(root, update, None)
}

/// `fetch`, with the folder the registry's index is kept in. `None` is
/// the folder this machine keeps such things in (`registry::cache`); a
/// test names one of its own.
pub fn fetch_with(root: &Path, update: bool, cache: Option<&Path>) -> Result<Vec<String>, String> {
    let manifest = read_manifest(root).map_err(|why| match root.as_os_str().is_empty() || root == Path::new(".") {
        true => format!("{why}. `zeno fetch` is run in the folder of a program, where its `{FILE}` is"),
        false => why,
    })?;
    let lock = match update {
        true => Vec::new(),
        false => fs::read_to_string(root.join(LOCK)).map(|text| parse_lock(&text)).unwrap_or_default(),
    };
    // Where a name with no address is looked up: the registry the
    // program names, or Zeno's own. It is asked at most once, and only
    // if the lock does not already say what a name and a version are.
    let registry = manifest.registry.clone().unwrap_or_else(|| registry::ZENOS_OWN.to_string());
    let mut index: Option<registry::Index> = None;

    let mut said = Vec::new();
    let mut pinned: Vec<Pinned> = Vec::new();
    // The folder each package was found in, by its name: a package is
    // looked at once, however many packages need it.
    let mut seen: HashMap<String, PathBuf> = HashMap::from([(manifest.name.clone(), root.to_path_buf())]);
    // Whose needs are still to be looked at: the package's name, its
    // folder and its `needs` lines.
    let mut waiting = vec![(manifest.name.clone(), root.to_path_buf(), manifest.needs)];
    while let Some((needed_by, folder, needs)) = waiting.pop() {
        for need in needs {
            let place = match &need.source {
                Source::Folder(path) => folder.join(path),
                Source::Git { .. } | Source::Registry { .. } => root.join(FOLDER).join(&need.name),
            };
            if let Some(known) = seen.get(&need.name) {
                // Two packages may need the same one, and must mean the same one.
                let same = match (&need.source, pinned.iter().find(|package| package.name == need.name)) {
                    (Source::Git { url, at }, Some(package)) => !package.registry && package.url == *url && package.at == *at,
                    (Source::Registry { at }, Some(package)) => package.registry && package.at == *at,
                    (Source::Folder(_), None) => same_folder(known, &place),
                    _ => false,
                };
                if !same {
                    return Err(format!(
                        "two packages need a package called `{}`, and do not mean the same one. `{needed_by}` \
                         needs it {}, and another needs it from somewhere else or at another version. \
                         A program has one package of each name: make the `needs` lines agree",
                        need.name,
                        described(&need.source)
                    ));
                }
                continue;
            }
            // Which repository, which name for the version, and which
            // commit if something already says: the lock, or the registry.
            let wanted = match &need.source {
                Source::Folder(_) => None,
                Source::Git { url, at } => {
                    let locked = lock.iter().find(|package| !package.registry && package.name == need.name && package.url == *url && package.at == *at);
                    Some((url.clone(), at.clone(), locked.map(|package| (package.commit.clone(), format!("`{LOCK}`"))), false))
                }
                Source::Registry { at } => {
                    let locked = lock.iter().find(|package| package.registry && package.name == need.name && package.at == *at);
                    Some(match locked {
                        Some(package) => (package.url.clone(), at.clone(), Some((package.commit.clone(), format!("`{LOCK}`"))), true),
                        None => {
                            if index.is_none() {
                                index = Some(registry::Index::open(&registry, cache, true)?);
                                said.push(format!("Asked the registry {registry}"));
                            }
                            let (url, commit) = index.as_ref().unwrap().version(&need.name, at)?;
                            (url, at.clone(), Some((commit, "the registry".to_string())), true)
                        }
                    })
                }
            };
            if let Some((url, at, commit, from_registry)) = wanted {
                let (commit, news) = get(&place, &url, &at, commit.as_ref().map(|(commit, who)| (commit.as_str(), who.as_str())))?;
                let short: String = commit.chars().take(7).collect();
                said.push(match news {
                    true => format!("Fetched {} {at} ({short}) into {}", need.name, shown(&place)),
                    false => format!("{} {at} ({short}) is already in {}", need.name, shown(&place)),
                });
                pinned.push(Pinned { name: need.name.clone(), url, at, commit, registry: from_registry });
            }
            let theirs = read_manifest(&place).map_err(|why| format!("`{needed_by}` needs `{}`, and {why}", need.name))?;
            if theirs.name != need.name {
                if !matches!(need.source, Source::Folder(_)) {
                    // Do not leave it lying under a name that is not its own.
                    let _ = fs::remove_dir_all(&place);
                }
                return Err(format!(
                    "`{needed_by}` needs `{}` {}, and the package there is called `{}`. Write `needs {} ...`",
                    need.name,
                    described(&need.source),
                    theirs.name,
                    theirs.name
                ));
            }
            seen.insert(need.name.clone(), place.clone());
            waiting.push((need.name, place, theirs.needs));
        }
    }

    pinned.sort_by(|a, b| a.name.cmp(&b.name));
    let lock_file = root.join(LOCK);
    if pinned.is_empty() {
        // Nothing was fetched, so there is nothing to pin.
        let _ = fs::remove_file(&lock_file);
        said.push(format!("`{}` needs no package that has to be fetched.", manifest.name));
    } else {
        let text = write_lock(&pinned);
        // A fetch that changed nothing leaves the file as it is.
        if fs::read_to_string(&lock_file).ok().as_deref() != Some(text.as_str()) {
            fs::write(&lock_file, text).map_err(|err| format!("cannot write `{}`: {err}", shown(&lock_file)))?;
            said.push(format!("Wrote {}", shown(&lock_file)));
        }
    }
    Ok(said)
}

/// "in `../filters`", or "from `https://...` at `v1.2.0`", or "at
/// `v2.0.1` from the registry".
fn described(source: &Source) -> String {
    match source {
        Source::Folder(path) => format!("in `{path}`"),
        Source::Git { url, at } => format!("from `{url}` at `{at}`"),
        Source::Registry { at } => format!("at `{at}` from the registry"),
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The `zeno.package` of the package in `folder`.
pub fn read_manifest(folder: &Path) -> Result<Manifest, String> {
    let path = folder.join(FILE);
    let text = fs::read_to_string(&path).map_err(|_| format!("there is no file `{}`", shown(&path)))?;
    parse(&text, 0).map_err(|errors| {
        format!("`{}` has a mistake on line {}: {}", shown(&path), errors[0].span.line_in_file(), errors[0].message)
    })
}

/// Run git, and give what it printed.
pub fn git(folder: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    if let Some(folder) = folder {
        command.arg("-C").arg(folder);
    }
    // A package is the same bytes on every machine that fetches it. Git
    // on Windows is usually set to give each line the Windows line end
    // as it writes a file out, and here it is told not to.
    command.args(["-c", "core.autocrlf=false"]);
    // Never stop to ask for a password: say what went wrong instead.
    let ran = command.args(args).env("GIT_TERMINAL_PROMPT", "0").output();
    match ran {
        Ok(output) if output.status.success() => Ok(String::from_utf8_lossy(&output.stdout).trim().to_string()),
        Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        Err(_) => Err("`zeno fetch` gets packages with git, and no `git` was found on this machine. Install git".to_string()),
    }
}

/// Have the repository at `url` in `place`, at the commit that `at`
/// names, or at `pinned` if something already names one: the lock, or
/// the registry. With the commit comes who names it, for a message.
/// Gives the commit, and whether anything had to be fetched.
fn get(place: &Path, url: &str, at: &str, pinned: Option<(&str, &str)>) -> Result<(String, bool), String> {
    let cannot = |why: String| format!("cannot fetch `{url}`: {why}");
    let here_already = place.join(".git").exists();
    if here_already {
        // It is here already. If it is at the pinned commit there is
        // nothing to ask the network for.
        let here = git(Some(place), &["rev-parse", "HEAD"]).map_err(cannot)?;
        if pinned.map(|(commit, _)| commit) == Some(here.as_str()) {
            return Ok((here, false));
        }
        git(Some(place), &["remote", "set-url", "origin", url]).map_err(cannot)?;
        git(Some(place), &["fetch", "--quiet", "--tags", "--force", "origin"]).map_err(cannot)?;
    } else {
        if place.exists() {
            return Err(format!(
                "`{}` is in the way: it is not a package that `zeno fetch` put there. Move it or remove it",
                shown(&place)
            ));
        }
        if let Some(parent) = place.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("cannot make the folder `{}`: {err}", shown(&parent)))?;
        }
        git(None, &["clone", "--quiet", url, &place.to_string_lossy()]).map_err(cannot)?;
    }
    // What `at` names: the pinned commit, or else a tag, a branch of the
    // repository, or a commit, looked for in that order.
    let ways = [format!("refs/tags/{at}^{{commit}}"), format!("refs/remotes/origin/{at}^{{commit}}"), format!("{at}^{{commit}}")];
    let commit = match pinned {
        Some((commit, who)) => {
            // A repository whose history was written again may have lost it.
            if git(Some(place), &["cat-file", "-e", &format!("{commit}^{{commit}}")]).is_err() {
                if !here_already {
                    let _ = fs::remove_dir_all(place);
                }
                return Err(format!(
                    "`{url}` has no commit `{commit}`, which is the one {who} names for `{at}`. Nothing of it was used"
                ));
            }
            commit.to_string()
        }
        None => ways
            .iter()
            .find_map(|way| git(Some(place), &["rev-parse", "--verify", "--quiet", way]).ok())
            .ok_or_else(|| format!("`{url}` has no tag, branch or commit called `{at}`"))?,
    };
    let here = git(Some(place), &["rev-parse", "HEAD"]).map_err(cannot)?;
    if here != commit {
        git(Some(place), &["checkout", "--quiet", "--detach", &commit]).map_err(cannot)?;
    }
    // News is a package that was not here, or is now at another commit.
    Ok((commit.clone(), !here_already || here != commit))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn error(source: &str) -> String {
        parse(source, 0).expect_err("the file should have a mistake")[0].message.clone()
    }

    #[test]
    fn a_package_file_is_read() {
        let manifest = parse(
            "# The thermostat.\npackage thermostat\n\nneeds filters in \"../filters\"\n\
             needs units from \"https://example.com/units\" at \"v1.2.0\"  # a tag\n",
            0,
        )
        .expect("it is read");
        assert_eq!(manifest.name, "thermostat");
        let needs: Vec<(&str, &Source)> = manifest.needs.iter().map(|need| (need.name.as_str(), &need.source)).collect();
        assert_eq!(
            needs,
            [
                ("filters", &Source::Folder("../filters".to_string())),
                ("units", &Source::Git { url: "https://example.com/units".to_string(), at: "v1.2.0".to_string() }),
            ]
        );
        assert_eq!(manifest.needs[1].span.line, 5);
        assert_eq!(parse("package alone", 0).expect("it is read").needs, []);
        // In a program of several files the position says which file.
        assert_eq!(parse("package a\nneeds b in \".\"\n", 3).unwrap().needs[0].span.file(), 3);
    }

    #[test]
    fn a_mistake_in_a_package_file_says_what_to_write() {
        assert_eq!(
            error("needs units in \"../units\"\n"),
            "a `zeno.package` begins with the word `package` and the package's name, as in `package thermostat`"
        );
        assert_eq!(error("package a\npackage b\n"), "a `zeno.package` says `package` once, on its first line");
        assert_eq!(error("package math\n"), "`math` is a module that comes with Zeno, so a package cannot have that name");
        assert_eq!(
            error("package a\nneeds units from \"https://example.com/units\"\n"),
            "this does not say which version of `units` is needed. Add `at` and a tag, a branch or a commit of that \
             repository, as in `needs units from \"...\" at \"v1.0.0\"`"
        );
        assert_eq!(
            error("package a\nneeds units \"../units\"\n"),
            format!("this `needs` line is not written as one is. {LINES}")
        );
        assert_eq!(error("package a\nversion 1.0\n"), format!("this is not a line of a `zeno.package`. {LINES}"));
        assert_eq!(error("package a\nneeds b in \"x\"\nneeds b in \"y\"\n"), "`b` is needed twice. One line for each package");
        assert_eq!(error("package a\nneeds a in \".\"\n"), "the package `a` cannot need itself");
        assert_eq!(
            error("package a\nneeds time in \"t\"\n"),
            "`time` is a module that comes with Zeno. `import time` gets it, with no `needs` line"
        );
    }

    #[test]
    fn a_package_file_says_how_memory_is_freed() {
        let read = |text: &str| parse(text, 0).expect("it is read").memory;
        assert_eq!(read("package a\n"), None);
        assert_eq!(read("package a\nmemory owned\n"), Some(Memory::Owned));
        assert_eq!(read("package a\nmemory collector  # as when nothing is said\n"), Some(Memory::Collector));
        assert_eq!(read("package a\nmemory owned\nneeds b in \"../b\"\n"), Some(Memory::Owned));
        let two_ways = "a program frees its memory in one of two ways: `memory collector` (the garbage collector, \
                        as when nothing is said) or `memory owned` (one owner for each value, and no collector)";
        assert_eq!(error("package a\nmemory manual\n"), two_ways);
        assert_eq!(error("package a\nmemory\n"), format!("this is not a line of a `zeno.package`. {LINES}"));
        assert_eq!(error("package a\nmemory owned\nmemory owned\n"), "a `zeno.package` says `memory` once");
    }

    #[test]
    fn a_package_file_names_a_package_of_the_registry() {
        let manifest = parse(
            "package logger\nabout \"Keeps a log of readings\"\nregistry \"https://git.example.com/zeno-packages\"\n\
             needs units at \"v1.2.0\"\n",
            0,
        )
        .expect("it is read");
        assert_eq!(manifest.about.as_deref(), Some("Keeps a log of readings"));
        assert_eq!(manifest.registry.as_deref(), Some("https://git.example.com/zeno-packages"));
        assert_eq!(manifest.needs[0].source, Source::Registry { at: "v1.2.0".to_string() });
        let plain = parse("package logger\n", 0).expect("it is read");
        assert_eq!((plain.about, plain.registry), (None, None));

        assert_eq!(
            error("package a\nneeds units at \"1.2.0\"\n"),
            "`1.2.0` is not a version of a package in the registry. One is written as the letter v and three numbers, \
             as in `needs units at \"v1.2.0\"`. A branch or a commit is of a git repository, which this line then \
             names: `needs units from \"address\" at \"1.2.0\"`"
        );
        assert_eq!(
            error("package a\nneeds units\n"),
            "this does not say where `units` is. A package in a folder: `needs units in \"../units\"`. A package in \
             the registry, and which version of it: `needs units at \"v1.0.0\"` (`zeno search units` shows the \
             versions there are)"
        );
        assert_eq!(error("package a\nabout \"one\"\nabout \"two\"\n"), "a `zeno.package` says `about` once");
        assert!(error("package a\nabout \"\"\n").starts_with("`about` is followed by one line that says what the package is for"));
        assert!(error(&format!("package a\nabout \"{}\"\n", "long ".repeat(30))).contains("at most 120 characters"));
        assert_eq!(error("package a\nregistry \"x\"\nregistry \"y\"\n"), "a `zeno.package` says `registry` once");
        assert!(error("package a\nregistry \"\"\n").starts_with("`registry` is followed by the address of a git repository"));
        assert_eq!(error("package a\nabout units\n"), format!("this is not a line of a `zeno.package`. {LINES}"));
    }

    #[test]
    fn a_lock_is_written_and_read_back() {
        let pinned = vec![
            Pinned { name: "crc".into(), url: "file:///srv/crc".into(), at: "main".into(), commit: "0123abc".into(), registry: false },
            Pinned { name: "units".into(), url: "https://example.com/units".into(), at: "v1.2.0".into(), commit: "9f8e7d6".into(), registry: false },
            // From the registry: the line begins as its `needs` line does.
            Pinned { name: "wave".into(), url: "https://example.com/wave".into(), at: "v2.0.1".into(), commit: "5a5a5a5".into(), registry: true },
        ];
        let text = write_lock(&pinned);
        assert!(text.ends_with(
            "crc from \"file:///srv/crc\" at \"main\" commit \"0123abc\"\n\
             units from \"https://example.com/units\" at \"v1.2.0\" commit \"9f8e7d6\"\n\
             wave at \"v2.0.1\" from \"https://example.com/wave\" commit \"5a5a5a5\"\n"
        ));
        assert_eq!(parse_lock(&text), pinned);
        assert_eq!(parse_lock("nonsense here\n"), []);
        assert_eq!(parse_lock("wave at \"v2.0.1\" at \"x\" commit \"5a5a5a5\"\nwave from \"x\" at \"v2.0.1\" pinned \"5a5a5a5\"\n"), []);
    }

    /// A folder for one test, empty to begin with.
    pub(crate) fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("zeno-package-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("a temporary folder");
        folder
    }

    /// Make a git repository in `folder` holding these files, with one
    /// commit. `None` if there is no git here to try it with.
    pub(crate) fn repository(folder: &Path, files: &[(&str, &str)], message: &str) -> Option<String> {
        fs::create_dir_all(folder).expect("a folder");
        if !folder.join(".git").exists() {
            git(Some(folder), &["init", "--quiet", "--initial-branch=main"]).ok()?;
        }
        for (name, text) in files {
            fs::write(folder.join(name), text).expect("a file is written");
        }
        git(Some(folder), &["add", "."]).ok()?;
        let me = ["-c", "user.name=Tester", "-c", "user.email=tester@example.com", "-c", "commit.gpgsign=false"];
        git(Some(folder), &[&me[..], &["commit", "--quiet", "-m", message]].concat()).ok()?;
        git(Some(folder), &["rev-parse", "HEAD"]).ok()
    }

    #[test]
    fn fetch_gets_what_is_needed_and_pins_it() {
        let work = scratch("fetch");
        // Two packages somebody else keeps: `units`, which needs `prefix`.
        let (units, prefix) = (work.join("far/units"), work.join("far/prefix"));
        let prefix_url = format!("file://{}", shown(&prefix));
        let units_url = format!("file://{}", shown(&units));
        let Some(prefix_first) = repository(&prefix, &[(FILE, "package prefix\n"), ("prefix.zeno", "let kilo = 1000\n")], "one") else {
            eprintln!("no git found; skipping this test");
            return;
        };
        let units_manifest = format!("package units\nneeds prefix from \"{prefix_url}\" at \"main\"\n");
        let units_first = repository(&units, &[(FILE, &units_manifest), ("units.zeno", "let metre = 1\n")], "one").unwrap();
        git(Some(&units), &["tag", "v1.0.0"]).unwrap();
        // The program, and a package beside it that is used where it is.
        let program = work.join("program");
        fs::create_dir_all(work.join("filters")).unwrap();
        fs::write(work.join("filters").join(FILE), "package filters\n").unwrap();
        fs::create_dir_all(&program).unwrap();
        fs::write(
            program.join(FILE),
            format!("package logger\nneeds filters in \"../filters\"\nneeds units from \"{units_url}\" at \"v1.0.0\"\n"),
        )
        .unwrap();

        let said = fetch(&program, false).expect("it fetches");
        assert_eq!(said.len(), 3, "{said:?}");
        assert!(said[0].starts_with("Fetched units v1.0.0 (") && said[1].starts_with("Fetched prefix main ("), "{said:?}");
        assert_eq!(fs::read_to_string(program.join("packages/units/units.zeno")).unwrap(), "let metre = 1\n");
        assert_eq!(fs::read_to_string(program.join("packages/prefix/prefix.zeno")).unwrap(), "let kilo = 1000\n");
        // The package in a folder is used where it is, and not pinned.
        assert!(!program.join("packages/filters").exists());
        let pinned = parse_lock(&fs::read_to_string(program.join(LOCK)).unwrap());
        assert_eq!(
            pinned,
            [
                Pinned { name: "prefix".into(), url: prefix_url.clone(), at: "main".into(), commit: prefix_first.clone(), registry: false },
                Pinned { name: "units".into(), url: units_url.clone(), at: "v1.0.0".into(), commit: units_first.clone(), registry: false },
            ]
        );

        // `prefix` moves on. The lock keeps the program where it was.
        let prefix_second = repository(&prefix, &[("prefix.zeno", "let kilo = 1000\nlet mega = 1000000\n")], "two").unwrap();
        assert_ne!(prefix_first, prefix_second);
        let said = fetch(&program, false).expect("it fetches");
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("is already in") && said[1].contains("is already in"), "{said:?}");
        assert_eq!(fs::read_to_string(program.join("packages/prefix/prefix.zeno")).unwrap(), "let kilo = 1000\n");
        // Someone who has only the program and its lock gets the same commit.
        fs::remove_dir_all(program.join("packages")).unwrap();
        fetch(&program, false).expect("it fetches");
        assert_eq!(fs::read_to_string(program.join("packages/prefix/prefix.zeno")).unwrap(), "let kilo = 1000\n");
        // `--update` looks `main` up again.
        fetch(&program, true).expect("it fetches");
        assert_eq!(fs::read_to_string(program.join("packages/prefix/prefix.zeno")).unwrap(), "let kilo = 1000\nlet mega = 1000000\n");
        let pinned = parse_lock(&fs::read_to_string(program.join(LOCK)).unwrap());
        assert_eq!(pinned[0].commit, prefix_second);

        // What cannot be fetched says why.
        fs::write(program.join(FILE), format!("package logger\nneeds units from \"{units_url}\" at \"v9\"\n")).unwrap();
        assert_eq!(fetch(&program, false).unwrap_err(), format!("`{units_url}` has no tag, branch or commit called `v9`"));
        fs::write(program.join(FILE), format!("package logger\nneeds unit from \"{units_url}\" at \"v1.0.0\"\n")).unwrap();
        assert_eq!(
            fetch(&program, false).unwrap_err(),
            format!("`logger` needs `unit` from `{units_url}` at `v1.0.0`, and the package there is called `units`. Write `needs units ...`")
        );
        fs::write(program.join(FILE), "package logger\nneeds gone in \"../gone\"\n").unwrap();
        let missing = fetch(&program, false).unwrap_err();
        assert!(missing.starts_with("`logger` needs `gone`, and there is no file `"), "{missing}");
        let _ = fs::remove_dir_all(&work);
    }
}
