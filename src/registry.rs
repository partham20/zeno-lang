//! The registry: where a package is found by its name.
//!
//! `needs units from "https://github.com/someone/zeno-units" at "v1.2.0"`
//! says where a package is. `needs units at "v1.2.0"` does not, and the
//! registry is what knows: a list that says, for each name, which git
//! repository the package is and which commit of it each version is.
//!
//! The registry is itself a git repository, and nothing else: no server,
//! no accounts, no uploads. It holds one small file for each package,
//! in a folder called `index`:
//!
//! ```text
//! package units
//! about "Lengths, masses and times that the checker keeps apart"
//! from "https://github.com/someone/zeno-units"
//! owner "someone"
//!
//! version "v1.2.0" commit "9f8e7d6..." dated "2026-11-18"
//! ```
//!
//! The packages stay where their authors keep them. `zeno fetch` gets the
//! registry with git, as it gets everything, reads the file with the
//! package's name, and from there on does what it does for a `needs`
//! line that names the repository itself (`package.rs`).
//!
//! This file is the four things that touch a registry:
//!
//! - `Index`: a copy of a registry on this machine, and looking a name
//!   up in it. `zeno fetch` uses it.
//! - `search`: `zeno search filter`.
//! - `publish`: `zeno publish`, which checks a package and says how to
//!   ask the registry to take it. It changes nothing anywhere.
//! - `accept`: `zeno registry accept`, which is the registry's side. It
//!   checks a package again, from its repository, and writes its file.
//!   Whoever keeps a registry runs it; for Zeno's own, a workflow does.
//!   (`relist`, `zeno registry index`, writes the list of all packages
//!   again after a change by hand.)
//!
//! See docs/registry.md for a walk through all four.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::builtins;
use crate::json::Json;
use crate::lexer;
use crate::modules::shown;
use crate::package::{self, git, Manifest, Source};
use crate::token::{self, Token, TokenKind};
use crate::value::quoted;

/// Zeno's own registry: where a name is looked up unless the program's
/// `zeno.package` names another with a `registry` line.
pub const ZENOS_OWN: &str = "https://github.com/zeno-src/packages";
/// The folder of a registry that has one file for each package.
pub const INDEX: &str = "index";
/// The same list as one file of JSON, which a web page can read.
pub const LIST: &str = "index.json";

// ------------------------------------------------------- What fits a registry

/// Is this how a version in the registry is written? The letter v and
/// three numbers, `v1.2.0`, with no zeros in front of a number, so that
/// a version has one spelling.
pub fn is_version(text: &str) -> bool {
    numbers(text).is_some()
}

/// The three numbers of a version, to put versions in order.
fn numbers(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.strip_prefix('v')?.split('.');
    let mut found = [0; 3];
    for number in &mut found {
        let part = parts.next()?;
        let plain = !part.is_empty() && part.len() <= 9 && part.chars().all(|c| c.is_ascii_digit());
        if !plain || (part.len() > 1 && part.starts_with('0')) {
            return None;
        }
        *number = part.parse().ok()?;
    }
    parts.next().is_none().then_some(found)
}

/// The longest an `about` line can be.
const ABOUT: usize = 120;

/// Is this text fit to stand after `about`? It is shown as one line in a
/// list, so it is one line, and not a long one.
pub fn about_fits(text: &str) -> Result<(), String> {
    let plain = !text.trim().is_empty() && text.trim() == text && !text.chars().any(char::is_control);
    match plain && text.chars().count() <= ABOUT {
        true => Ok(()),
        false => Err(format!(
            "`about` is followed by one line that says what the package is for, in at most {ABOUT} characters, as in \
             `about \"Filters for noisy readings\"`"
        )),
    }
}

/// Can a package of this name be in a registry?
///
/// A name there is small letters, digits and `_`. Every name of the
/// language may have capitals too, but a registry is a folder of files
/// named after packages, and Windows and macOS take `Units` and `units`
/// for one file. It also means nobody can publish a `Units` that is
/// taken for somebody else's `units`.
pub fn name_fits(name: &str) -> Result<(), String> {
    let small = name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    let starts = name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    if !small || !starts || name.len() < 2 || name.len() > 40 {
        return Err(format!(
            "`{name}` cannot be the name of a package in the registry. A name there is 2 to 40 small letters, \
             digits and `_`, and starts with a letter"
        ));
    }
    if token::keyword(name).is_some() {
        return Err(format!("`{name}` is a word of the language, so a package cannot have that name"));
    }
    if builtins::comes_with_zeno(name) {
        return Err(format!("`{name}` is a module that comes with Zeno, so a package cannot have that name"));
    }
    Ok(())
}

/// An address without the `/` at its end, which changes nothing.
fn tidy(address: &str) -> &str {
    address.trim().trim_end_matches('/')
}

/// Do two addresses name one repository? On GitHub and the like, `x.git`
/// is the repository `x`, and capitals do not count. An address is kept
/// as it was given, since on a plain server `x.git` may be the only name
/// that works; the two are only taken for one when they are compared.
fn same_address(a: &str, b: &str) -> bool {
    let plain = |address: &str| {
        let address = tidy(address);
        address.strip_suffix(".git").unwrap_or(address).trim_end_matches('/').to_ascii_lowercase()
    };
    plain(a) == plain(b)
}

/// The address a repository is fetched from by somebody who has no key
/// for it. `origin` is often the address its owner pushes to,
/// `git@github.com:someone/units.git`, which is the same repository as
/// `https://github.com/someone/units.git`.
fn public_address(origin: &str) -> String {
    let origin = tidy(origin);
    let https = |rest: &str| format!("https://{}", rest.replacen(':', "/", 1));
    if let Some(rest) = origin.strip_prefix("ssh://git@") {
        return format!("https://{rest}");
    }
    match origin.strip_prefix("git@") {
        Some(rest) if !origin.contains("://") => https(rest),
        _ => origin.to_string(),
    }
}

// ------------------------------------------------------------ One package

/// What a registry knows about one package: the file `index/<name>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    /// The `about` line of the newest version that was accepted.
    pub about: String,
    /// The git repository the package is.
    pub url: String,
    /// Who published it, and so who may publish its next version. What
    /// this names is up to the registry: in Zeno's own, a GitHub account.
    pub owner: String,
    /// The versions, oldest first.
    pub versions: Vec<Version>,
}

/// One version of a package: a tag of its repository, and the commit the
/// tag named on the day it was published. The commit is what is
/// fetched, so a tag that is moved afterwards moves nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub tag: String,
    pub commit: String,
    /// The day of the commit, `2026-11-18`.
    pub dated: String,
}

/// Read a file of the index. `None` if it is not one.
pub fn parse_entry(source: &str) -> Option<Entry> {
    let tokens = lexer::lex(source).ok()?;
    let text = |token: &Token| match &token.kind {
        TokenKind::Str { value, formatted: false } => Some(value.clone()),
        _ => None,
    };
    let word = |token: &Token, wanted: &str| matches!(&token.kind, TokenKind::Ident(found) if found == wanted);
    let (mut name, mut about, mut url, mut owner) = (None, None, None, None);
    let mut versions = Vec::new();
    for line in tokens.split(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Eof)) {
        match line {
            [] => {}
            [first, Token { kind: TokenKind::Ident(package), .. }] if word(first, "package") => name = Some(package.clone()),
            [first, said] if word(first, "about") => about = Some(text(said)?),
            [first, said] if first.kind == TokenKind::From => url = Some(text(said)?),
            [first, said] if word(first, "owner") => owner = Some(text(said)?),
            [first, tag, second, commit, third, dated] if word(first, "version") && word(second, "commit") && word(third, "dated") => {
                versions.push(Version { tag: text(tag)?, commit: text(commit)?, dated: text(dated)? });
            }
            _ => return None,
        }
    }
    Some(Entry { name: name?, about: about?, url: url?, owner: owner?, versions })
}

/// A file of the index, as text.
pub fn write_entry(entry: &Entry) -> String {
    let mut text = format!(
        "# The package `{}` in this registry. `zeno registry accept` wrote this file:\n\
         # a line for each version, the oldest first. A version that is here\n\
         # stays as it is.\n\
         package {}\nabout {}\nfrom {}\nowner {}\n\n",
        entry.name,
        entry.name,
        quoted(&entry.about),
        quoted(&entry.url),
        quoted(&entry.owner)
    );
    for version in &entry.versions {
        text += &format!("version {} commit {} dated {}\n", quoted(&version.tag), quoted(&version.commit), quoted(&version.dated));
    }
    text
}

/// Every package of a registry as JSON: what its `index.json` holds. A
/// package is one line, so that a new version changes one line.
pub fn write_list(entries: &[Entry]) -> String {
    let lines: Vec<String> = entries
        .iter()
        .map(|entry| {
            let versions = entry.versions.iter().map(|version| {
                Json::object([
                    ("version", Json::text(&version.tag)),
                    ("commit", Json::text(&version.commit)),
                    ("dated", Json::text(&version.dated)),
                ])
            });
            let package = Json::object([
                ("name", Json::text(&entry.name)),
                ("about", Json::text(&entry.about)),
                ("repository", Json::text(&entry.url)),
                ("owner", Json::text(&entry.owner)),
                ("versions", Json::List(versions.collect())),
            ]);
            format!("  {}", package.write())
        })
        .collect();
    match lines.is_empty() {
        true => "{\"packages\": [\n]}\n".to_string(),
        false => format!("{{\"packages\": [\n{}\n]}}\n", lines.join(",\n")),
    }
}

// --------------------------------------------------------------- The index

/// A registry, as a folder on this machine.
pub struct Index {
    /// The folder the registry's repository is in.
    folder: PathBuf,
    /// What to call it in a message: its address, or the folder.
    name: String,
}

/// The folder this machine keeps things in that can be fetched again:
/// the copy of the registry goes there, in `zeno/registry`. `ZENO_CACHE`
/// names another folder.
pub fn cache() -> Result<PathBuf, String> {
    let set = |name: &str| env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    if let Some(folder) = set("ZENO_CACHE") {
        return Ok(folder);
    }
    let found = if cfg!(windows) {
        set("LOCALAPPDATA").map(|folder| folder.join("zeno"))
    } else if cfg!(target_os = "macos") {
        set("HOME").map(|home| home.join("Library/Caches/zeno"))
    } else {
        set("XDG_CACHE_HOME").or_else(|| set("HOME").map(|home| home.join(".cache"))).map(|folder| folder.join("zeno"))
    };
    found.ok_or_else(|| {
        "Zeno keeps a copy of the registry in a folder of your own, and cannot tell where that is on this machine. \
         Set ZENO_CACHE to a folder it may use"
            .to_string()
    })
}

/// A name for a folder, made from an address: its letters and digits,
/// and a number worked out from all of it, so that two addresses never
/// share a folder.
fn folder_name(address: &str) -> String {
    let letters: String = tidy(address)
        .trim_start_matches("https://")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    // FNV-1a: a few lines that turn a text into a number.
    let number = address.bytes().fold(0x811c_9dc5_u32, |hash, byte| (hash ^ byte as u32).wrapping_mul(0x0100_0193));
    format!("{}-{number:08x}", letters.trim_matches('_'))
}

impl Index {
    /// The registry at `address`, as this machine's copy of it. The
    /// first time, the copy is made. With `refresh`, it is brought up to
    /// date. Only the newest state of the registry is kept, not its
    /// history.
    pub fn open(address: &str, cache: Option<&Path>, refresh: bool) -> Result<Index, String> {
        let cache = match cache {
            Some(folder) => folder.to_path_buf(),
            None => self::cache()?,
        };
        let folder = cache.join("registry").join(folder_name(address));
        let cannot = |why: String| format!("cannot reach the registry {address}: {why}");
        if !folder.join(".git").exists() {
            // A copy that was stopped half way is made again.
            let _ = fs::remove_dir_all(&folder);
            if let Some(parent) = folder.parent() {
                fs::create_dir_all(parent).map_err(|err| format!("cannot make the folder `{}`: {err}", shown(parent)))?;
            }
            git(None, &["clone", "--quiet", "--depth", "1", address, &folder.to_string_lossy()]).map_err(cannot)?;
        } else if refresh {
            match git(Some(&folder), &["fetch", "--quiet", "--depth", "1", "origin", "HEAD"]) {
                Ok(_) => {
                    git(Some(&folder), &["checkout", "--quiet", "--force", "--detach", "FETCH_HEAD"]).map_err(cannot)?;
                }
                // A registry nobody has put anything into yet.
                Err(why) if why.contains("couldn't find remote ref") => {}
                Err(why) => return Err(cannot(why)),
            }
        }
        Ok(Index { folder, name: address.to_string() })
    }

    /// The registry in `folder`: a clone that somebody made, to add to.
    pub fn at(folder: &Path) -> Index {
        Index { folder: folder.to_path_buf(), name: format!("in `{}`", shown(folder)) }
    }

    /// What the registry knows about the package `name`, if it has it.
    pub fn entry(&self, name: &str) -> Result<Option<Entry>, String> {
        // A name that is not a registry's name is the name of no file
        // there, whatever is in it: `../x` is not looked for.
        if name_fits(name).is_err() {
            return Ok(None);
        }
        let path = self.folder.join(INDEX).join(name);
        let Ok(source) = fs::read_to_string(&path) else { return Ok(None) };
        match parse_entry(&source) {
            Some(entry) if entry.name == name => Ok(Some(entry)),
            _ => Err(format!("the registry {} has a file `{INDEX}/{name}` that is not written as its files are", self.name)),
        }
    }

    /// Every package of the registry, by name.
    pub fn all(&self) -> Result<Vec<Entry>, String> {
        let mut names: Vec<String> = match fs::read_dir(self.folder.join(INDEX)) {
            Ok(files) => files.flatten().filter_map(|file| file.file_name().into_string().ok()).collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        let mut entries = Vec::new();
        for name in names {
            entries.extend(self.entry(&name)?);
        }
        Ok(entries)
    }

    /// The repository and the commit that are the version `at` of the
    /// package `name`.
    pub fn version(&self, name: &str, at: &str) -> Result<(String, String), String> {
        let Some(entry) = self.entry(name)? else {
            return Err(format!(
                "the registry {} has no package called `{name}`. `zeno search` and a word shows the packages it has",
                self.name
            ));
        };
        match entry.versions.iter().find(|version| version.tag == at) {
            Some(version) => Ok((entry.url.clone(), version.commit.clone())),
            None => Err(format!("the registry has no version `{at}` of `{name}`. It has {}", listed(&entry.versions))),
        }
    }
}

/// "v1.2.0, v1.1.0 and v1.0.0": the versions, the newest first, and not
/// more of them than a message can hold.
fn listed(versions: &[Version]) -> String {
    let tags: Vec<&str> = versions.iter().rev().map(|version| version.tag.as_str()).collect();
    match tags.as_slice() {
        [] => "no version yet".to_string(),
        [only] => only.to_string(),
        [first @ .., last] if tags.len() <= 6 => format!("{} and {last}", first.join(", ")),
        _ => format!("{} and {} older ones", tags[..5].join(", "), tags.len() - 5),
    }
}

/// The registry that the program in `folder` looks names up in: the one
/// its `zeno.package` names, or Zeno's own.
fn registry_of(folder: &Path) -> String {
    package::read_manifest(folder).ok().and_then(|manifest| manifest.registry).unwrap_or_else(|| ZENOS_OWN.to_string())
}

// -------------------------------------------------------------- zeno search

/// `zeno search filter noise`: the packages of the registry that have
/// every one of these words in their name or in their `about` line. With
/// no word, all of them. Gives the lines to print.
///
/// The registry is the one the program in `folder` uses, and the first
/// line says which that is.
pub fn search(words: &[String], folder: &Path, cache: Option<&Path>) -> Result<Vec<String>, String> {
    let address = registry_of(folder);
    let mut said = Vec::new();
    // With no network, the copy from last time is better than nothing,
    // as long as the answer says that this is what it is.
    let index = match Index::open(&address, cache, true) {
        Ok(index) => index,
        Err(why) => match Index::open(&address, cache, false) {
            Ok(index) => {
                said.push("The registry could not be reached, so this is what it had the last time it was.".to_string());
                index
            }
            Err(_) => return Err(why),
        },
    };
    let words: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
    let found: Vec<Entry> = index
        .all()?
        .into_iter()
        .filter(|entry| {
            let text = format!("{} {}", entry.name, entry.about).to_lowercase();
            words.iter().all(|word| text.contains(word.as_str()))
        })
        .collect();
    let packages = |count: usize| crate::error::counted(count, "package");
    said.push(match (found.len(), words.is_empty()) {
        (0, true) => format!("The registry {address} has no packages yet."),
        (0, false) => format!("No package in the registry {address} has `{}` in its name or in what it says it is for.", words.join("` and `")),
        (count, true) => format!("The registry {address} has {}:", packages(count)),
        (count, false) => format!("{} in the registry {address}:", packages(count)),
    });
    for entry in &found {
        let Some(newest) = entry.versions.last() else { continue };
        said.push(String::new());
        said.push(format!("{} {}    {}", entry.name, newest.tag, entry.about));
        said.push(format!("    needs {} at \"{}\"", entry.name, newest.tag));
        said.push(format!("    {}", entry.url));
        if entry.versions.len() > 1 {
            said.push(format!("    older: {}", listed(&entry.versions[..entry.versions.len() - 1])));
        }
    }
    Ok(said)
}

// ------------------------------------------------- What fits to be published

/// Is the package in `folder`, whose `zeno.package` says `manifest`, one
/// that a registry can take? Gives its `about` line.
///
/// `zeno publish` asks this of the folder somebody works in, and
/// `zeno registry accept` asks it again of what the repository holds.
fn fits<'a>(manifest: &'a Manifest, folder: &Path, index: &Index) -> Result<&'a str, String> {
    let name = &manifest.name;
    name_fits(name)?;
    let Some(about) = manifest.about.as_deref() else {
        return Err(format!(
            "`{name}` does not say what it is for. Somebody who searches the registry sees one line for each package: \
             add it to `{}`, as in `about \"Filters for noisy readings\"`",
            package::FILE
        ));
    };
    for need in &manifest.needs {
        match &need.source {
            // A folder beside the package is on its author's machine and
            // on no other.
            Source::Folder(path) if !inside(path) || !folder.join(path).join(package::FILE).is_file() => {
                return Err(format!(
                    "`{name}` needs `{}` in `{path}`, which is not a folder of this package, so nobody who fetches \
                     `{name}` would have it. Publish `{}` too and need it by its name, or keep it in a folder of `{name}`",
                    need.name, need.name
                ));
            }
            Source::Registry { at } => {
                index.version(&need.name, at).map_err(|why| format!("`{name}` needs `{}` at `{at}`, and {why}", need.name))?;
            }
            Source::Folder(_) | Source::Git { .. } => {}
        }
    }
    Ok(about)
}

/// Does this path stay in the folder it is counted from?
fn inside(path: &str) -> bool {
    let mut depth = 0;
    for part in Path::new(path).components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    true
}

// ------------------------------------------------------------- zeno publish

/// `zeno publish`: check that the package in `folder` is ready for the
/// registry, and say how to ask the registry to take it. `tag` says
/// which version, if the commit has more than one.
///
/// It changes nothing, here or anywhere. A registry is added to by
/// whoever keeps it (`accept`); for one on GitHub the asking is a form
/// there, which this gives the address of, already filled in.
pub fn publish(folder: &Path, tag: Option<&str>, cache: Option<&Path>) -> Result<Vec<String>, String> {
    let manifest = package::read_manifest(folder)
        .map_err(|why| format!("{why}. `zeno publish` is run in the folder of a package, where its `{}` is", package::FILE))?;
    let name = &manifest.name;
    let address = manifest.registry.clone().unwrap_or_else(|| ZENOS_OWN.to_string());

    // A version is a commit of a repository, so there has to be one.
    let here = |args: &[&str]| git(Some(folder), args);
    let top = here(&["rev-parse", "--show-toplevel"]).map_err(|_| {
        "this folder is not a git repository. A registry does not hold a package: it says which repository the \
         package is and which commit each version is. Make one (`git init`), commit the files, and push it somewhere \
         anybody can fetch from"
            .to_string()
    })?;
    let same = |a: &Path, b: &Path| matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b);
    if !same(Path::new(&top), if folder.as_os_str().is_empty() { Path::new(".") } else { folder }) {
        return Err(format!(
            "the package is in a folder of the repository `{top}`, and not at its top. A registry reads `{}` at the \
             top of a repository: give the package a repository of its own",
            package::FILE
        ));
    }
    if !here(&["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        return Err("there are changes here that are not committed. A version is a commit: commit them, or put them aside".to_string());
    }
    let commit = here(&["rev-parse", "HEAD"]).map_err(|_| "this repository has no commit yet. Commit the files first".to_string())?;

    // Which version: the tag on this commit.
    let on_commit = here(&["tag", "--points-at", "HEAD"])?;
    let versions: Vec<&str> = on_commit.lines().filter(|tag| is_version(tag)).collect();
    let tag = match (tag, versions.as_slice()) {
        (Some(tag), _) if !is_version(tag) => {
            return Err(format!("`{tag}` is not a version. A version is the letter v and three numbers, as in `zeno publish v1.2.0`"));
        }
        (Some(tag), _) if versions.contains(&tag) => tag,
        (Some(tag), _) => {
            return Err(match here(&["rev-parse", "--verify", "--quiet", &format!("refs/tags/{tag}^{{commit}}")]) {
                Ok(_) => format!("the tag `{tag}` is on another commit than the one this folder is at. `git checkout {tag}` first, so that what is checked is what is published"),
                Err(_) => format!("there is no tag `{tag}` here. Make it, and push it:\n    git tag {tag}\n    git push origin {tag}"),
            });
        }
        (None, [only]) => only,
        (None, []) => {
            return Err(
                "the commit this folder is at has no version. A version is a tag: the letter v and three numbers. \
                 Make one, and push it:\n    git tag v1.0.0\n    git push origin v1.0.0"
                    .to_string(),
            );
        }
        (None, several) => {
            return Err(format!(
                "this commit has {} version tags: {}. Say which to publish, as in `zeno publish {}`",
                several.len(),
                several.join(", "),
                several[several.len() - 1]
            ));
        }
    };

    // Where it is fetched from, and whether the tag is there.
    let origin = here(&["remote", "get-url", "origin"]).map_err(|_| {
        "this repository has no `origin`: it is not pushed anywhere. Push it somewhere anybody can fetch from, \
         with its tag"
            .to_string()
    })?;
    let url = public_address(&origin);
    let on_github = address.starts_with("https://github.com/");
    if on_github && !url.starts_with("https://") {
        return Err(format!(
            "this repository is pushed to `{origin}`, and the registry needs an address that begins `https://`, \
             which anybody can fetch from"
        ));
    }
    let there = here(&["ls-remote", "origin", &format!("refs/tags/{tag}"), &format!("refs/tags/{tag}^{{}}")])
        .map_err(|why| format!("cannot reach `{origin}`: {why}"))?;
    // A tag with a message of its own is two lines, and the one that
    // ends `^{}` is the commit.
    let pushed = there.lines().rev().find_map(|line| line.split_whitespace().next()).map(str::to_string);
    match pushed {
        None => return Err(format!("the tag `{tag}` is not at `{origin}` yet. Push it:\n    git push origin {tag}")),
        Some(pushed) if pushed != commit => {
            return Err(format!(
                "the tag `{tag}` is another commit at `{origin}` than it is here. A version is one commit: \
                 give this commit a new version"
            ));
        }
        Some(_) => {}
    }

    // What the registry has under this name.
    let index = Index::open(&address, cache, true)?;
    fits(&manifest, folder, &index)?;
    if let Some(entry) = index.entry(name)? {
        if !same_address(&entry.url, &url) {
            return Err(taken(&entry));
        }
        if entry.versions.iter().any(|version| version.tag == tag) {
            return Err(format!("`{name}` {tag} is in the registry already. A version that is published stays as it is: give this commit a new version"));
        }
    }

    let short: String = commit.chars().take(7).collect();
    let mut said = vec![format!("`{name}` {tag} ({short}) is ready to be published, from {url}"), String::new()];
    if on_github {
        let form = format!(
            "{}/issues/new?template=publish.yml&title={}&repository={}&version={}",
            tidy(&address),
            in_address(&format!("Publish {name} {tag}")),
            in_address(&url),
            in_address(tag)
        );
        said.push("Open this address, signed in to GitHub, and press the button under the form:".to_string());
        said.push(String::new());
        said.push(format!("    {form}"));
        said.push(String::new());
        said.push("The registry checks the package and answers on that page, most often within a few minutes.".to_string());
    } else {
        said.push(format!("The registry {address} is added to by whoever keeps it. In a clone of it, they run:"));
        said.push(String::new());
        said.push(format!("    zeno registry accept . {url} {tag} --owner NAME"));
        said.push(String::new());
        said.push("with their name for you where NAME stands, and then commit and push what that wrote.".to_string());
    }
    Ok(said)
}

/// The message for a name that another package has.
fn taken(entry: &Entry) -> String {
    format!(
        "the name `{}` is taken in the registry: it is the package at {}. Give this package another name",
        entry.name, entry.url
    )
}

/// A text as it is written inside an address: every character that
/// means something there, or cannot stand there, as `%` and its code.
fn in_address(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out += &format!("%{byte:02X}"),
        }
    }
    out
}

// ----------------------------------------------------- zeno registry accept

/// `zeno registry accept <folder> <address> <tag> --owner <who>`: take
/// the version `tag` of the package in the repository at `address` into
/// the registry in `folder`. Gives the lines to print.
///
/// It fetches the repository, checks the package, and writes two files:
/// the package's own in `index`, and `index.json`. It commits nothing:
/// whoever runs it looks at what it wrote, and commits and pushes.
///
/// `owner` is who asked. The first to publish a name owns it, and only
/// they publish its later versions. Who somebody is, the registry has
/// to know for itself: Zeno's own runs this from a form on GitHub, and
/// passes the account that filled the form in.
pub fn accept(folder: &Path, address: &str, tag: &str, owner: &str) -> Result<Vec<String>, String> {
    if !is_version(tag) {
        return Err(format!(
            "`{tag}` is not a version. A version in a registry is a tag of the package's repository, written as the \
             letter v and three numbers: `v1.2.0`"
        ));
    }
    let plain = |text: &str| !text.is_empty() && !text.starts_with('-') && !text.contains(|c: char| c.is_whitespace() || c.is_control() || "\"'\\`".contains(c));
    if !plain(address) {
        return Err(format!("`{address}` is not the address of a git repository"));
    }
    if !plain(owner) {
        return Err(format!("`{owner}` cannot be the name of an owner: it is one word"));
    }
    if !folder.is_dir() {
        return Err(format!("there is no folder `{}`. `zeno registry accept` is given a clone of the registry to add to", shown(folder)));
    }
    let address = tidy(address);
    let index = Index::at(folder);

    // The repository, at the tag, in a folder that is taken away again.
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.subsec_nanos()).unwrap_or(0);
    let scratch = env::temp_dir().join(format!("zeno-accept-{}-{stamp}", std::process::id()));
    let looked = look(&scratch, address, tag, &index);
    let _ = fs::remove_dir_all(&scratch);
    let (name, about, version) = looked?;

    let entry = match index.entry(&name)? {
        None => Entry { name: name.clone(), about, url: address.to_string(), owner: owner.to_string(), versions: vec![version.clone()] },
        Some(mut entry) => {
            if !same_address(&entry.url, address) {
                return Err(taken(&entry));
            }
            if !entry.owner.eq_ignore_ascii_case(owner) {
                return Err(format!(
                    "`{name}` was published by `{}`, and only they publish its versions. This is asked by `{owner}`",
                    entry.owner
                ));
            }
            if let Some(known) = entry.versions.iter().find(|known| known.tag == tag) {
                return Err(match known.commit == version.commit {
                    true => format!("`{name}` {tag} is in the registry already"),
                    false => format!(
                        "`{name}` {tag} is in the registry already, as the commit `{}`, and the tag now names `{}`. \
                         A version that is published stays as it is: give the new commit a new version",
                        known.commit, version.commit
                    ),
                });
            }
            entry.about = about;
            entry.versions.push(version.clone());
            entry.versions.sort_by_key(|version| numbers(&version.tag));
            entry
        }
    };

    let file = folder.join(INDEX).join(&name);
    fs::create_dir_all(folder.join(INDEX)).map_err(|err| format!("cannot make the folder `{}`: {err}", shown(&folder.join(INDEX))))?;
    fs::write(&file, write_entry(&entry)).map_err(|err| format!("cannot write `{}`: {err}", shown(&file)))?;
    let list = folder.join(LIST);
    fs::write(&list, write_list(&index.all()?)).map_err(|err| format!("cannot write `{}`: {err}", shown(&list)))?;

    let short: String = version.commit.chars().take(7).collect();
    Ok(vec![
        format!("Accepted {name} {tag} ({short}) from {address}"),
        format!("Wrote {} and {}", shown(&file), shown(&list)),
    ])
}

/// `zeno registry index <folder>`: write `index.json` afresh from the
/// files in `index`, after one of them was changed or taken away by
/// hand. Every file is read on the way, so this is also how a registry
/// is checked. Gives the lines to print.
pub fn relist(folder: &Path) -> Result<Vec<String>, String> {
    if !folder.is_dir() {
        return Err(format!("there is no folder `{}`. `zeno registry index` is given a clone of a registry", shown(folder)));
    }
    let entries = Index::at(folder).all()?;
    let list = folder.join(LIST);
    fs::write(&list, write_list(&entries)).map_err(|err| format!("cannot write `{}`: {err}", shown(&list)))?;
    Ok(vec![format!("Wrote {}: {}", shown(&list), crate::error::counted(entries.len(), "package"))])
}

/// Fetch the repository at `address` into `scratch`, at the tag, and
/// check the package it holds. Gives the package's name, its `about`
/// line, and the version the tag is.
fn look(scratch: &Path, address: &str, tag: &str, index: &Index) -> Result<(String, String, Version), String> {
    git(None, &["clone", "--quiet", "--depth", "1", "--branch", tag, address, &scratch.to_string_lossy()])
        .map_err(|why| format!("cannot fetch `{address}` at `{tag}`: {why}"))?;
    let there = |args: &[&str]| git(Some(scratch), args);
    // `--branch` takes a branch of that name too, and a branch moves.
    let commit = there(&["rev-parse", "--verify", "--quiet", &format!("refs/tags/{tag}^{{commit}}")]).map_err(|_| {
        format!("`{tag}` is a branch of `{address}`, and not a tag. A version is a tag, because a tag is meant to stay where it is")
    })?;
    let dated = there(&["log", "-1", "--format=%cs", &commit])?;
    let manifest = package::read_manifest(scratch).map_err(|why| {
        // The scratch folder's name means nothing to whoever reads this.
        let why = why.replace(&format!("{}/", shown(scratch)), "");
        format!("`{address}` at `{tag}` is not a package: {why}. The file is looked for at the top of the repository")
    })?;
    let about = fits(&manifest, scratch, index)?.to_string();
    Ok((manifest.name, about, Version { tag: tag.to_string(), commit, dated }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_v_and_three_numbers() {
        for good in ["v1.2.0", "v0.0.1", "v10.20.30"] {
            assert!(is_version(good), "{good}");
        }
        for bad in ["1.2.0", "v1.2", "v1.2.0.1", "v1.02.0", "v1.2.x", "main", "", "v", "V1.2.0", "v1.2.0-rc1", "v1..0"] {
            assert!(!is_version(bad), "{bad}");
        }
        // Versions are put in order by their numbers, not as text.
        assert!(numbers("v1.10.0") > numbers("v1.9.0"));
    }

    #[test]
    fn a_name_in_a_registry_is_small_letters() {
        for good in ["units", "crc16", "low_pass"] {
            assert_eq!(name_fits(good), Ok(()), "{good}");
        }
        let said = |name: &str| name_fits(name).unwrap_err();
        assert!(said("Units").starts_with("`Units` cannot be the name of a package in the registry"));
        assert!(said("x").contains("2 to 40"));
        assert!(said("_private").contains("starts with a letter"));
        assert_eq!(said("class"), "`class` is a word of the language, so a package cannot have that name");
        assert_eq!(said("math"), "`math` is a module that comes with Zeno, so a package cannot have that name");
    }

    #[test]
    fn an_address_is_tidied_and_made_public() {
        assert_eq!(tidy(" https://example.com/units.git/ "), "https://example.com/units.git");
        assert!(same_address("https://github.com/Someone/Units", "https://github.com/someone/units.git"));
        assert!(!same_address("https://github.com/someone/units", "https://github.com/other/units"));
        assert_eq!(public_address("git@github.com:someone/units.git"), "https://github.com/someone/units.git");
        assert_eq!(public_address("ssh://git@github.com/someone/units"), "https://github.com/someone/units");
        assert_eq!(public_address("https://github.com/someone/units"), "https://github.com/someone/units");
        assert_eq!(public_address("file:///srv/units"), "file:///srv/units");
        assert_eq!(in_address("Publish units v1.2.0"), "Publish%20units%20v1.2.0");
        assert_eq!(in_address("https://a.b/c"), "https%3A%2F%2Fa.b%2Fc");
    }

    #[test]
    fn a_path_stays_inside_or_does_not() {
        assert!(inside("helper") && inside("./lib/helper") && inside("lib/../helper"));
        assert!(!inside("../helper") && !inside("lib/../../helper") && !inside("/srv/helper"));
    }

    fn units() -> Entry {
        Entry {
            name: "units".into(),
            about: "Lengths that say \"metre\"".into(),
            url: "https://example.com/units".into(),
            owner: "someone".into(),
            versions: vec![
                Version { tag: "v1.0.0".into(), commit: "0123abc".into(), dated: "2026-11-01".into() },
                Version { tag: "v1.2.0".into(), commit: "9f8e7d6".into(), dated: "2026-11-18".into() },
            ],
        }
    }

    #[test]
    fn an_entry_is_written_and_read_back() {
        let text = write_entry(&units());
        assert!(text.ends_with(
            "package units\nabout \"Lengths that say \\\"metre\\\"\"\nfrom \"https://example.com/units\"\nowner \"someone\"\n\n\
             version \"v1.0.0\" commit \"0123abc\" dated \"2026-11-01\"\n\
             version \"v1.2.0\" commit \"9f8e7d6\" dated \"2026-11-18\"\n"
        ));
        assert_eq!(parse_entry(&text), Some(units()));
        assert_eq!(parse_entry("package units\n"), None);
        assert_eq!(parse_entry(&format!("{text}something else\n")), None);
    }

    #[test]
    fn the_list_is_json_with_a_line_for_each_package() {
        assert_eq!(write_list(&[]), "{\"packages\": [\n]}\n");
        let text = write_list(&[units(), Entry { name: "wave".into(), versions: Vec::new(), ..units() }]);
        assert_eq!(text.lines().count(), 4);
        let read = Json::parse(&text).expect("it is JSON");
        let packages = read.get("packages").and_then(Json::as_list).expect("a list");
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].get("about").and_then(Json::as_str), Some("Lengths that say \"metre\""));
        assert_eq!(packages[0].at(&["versions"]).and_then(Json::as_list).map(<[Json]>::len), Some(2));
        assert_eq!(packages[1].get("name").and_then(Json::as_str), Some("wave"));
    }

    #[test]
    fn versions_are_listed_newest_first() {
        let version = |tag: &str| Version { tag: tag.into(), commit: String::new(), dated: String::new() };
        assert_eq!(listed(&[]), "no version yet");
        assert_eq!(listed(&[version("v1.0.0")]), "v1.0.0");
        assert_eq!(listed(&[version("v1.0.0"), version("v1.1.0"), version("v2.0.0")]), "v2.0.0, v1.1.0 and v1.0.0");
        let many: Vec<Version> = (0..9).map(|n| version(&format!("v1.{n}.0"))).collect();
        assert_eq!(listed(&many), "v1.8.0, v1.7.0, v1.6.0, v1.5.0, v1.4.0 and 4 older ones");
    }

    #[test]
    fn two_addresses_never_share_a_folder() {
        let own = folder_name(ZENOS_OWN);
        assert!(own.starts_with("github.com_zeno-src_packages-"), "{own}");
        assert_ne!(folder_name("https://example.com/a/b"), folder_name("https://example.com/a_b"));
    }

    /// The whole way round, with git repositories in folders of this
    /// machine: two packages are taken into a registry, a program that
    /// needs one by its name fetches both, and `publish` and `search`
    /// say what they find.
    #[test]
    fn a_package_goes_into_a_registry_and_is_fetched_by_its_name() {
        use crate::package::tests::{repository, scratch};
        use crate::package::{fetch_with, parse_lock, FILE, LOCK};

        let work = scratch("registry");
        let address = |folder: &Path| format!("file://{}", shown(folder));
        let write = |folder: &Path, name: &str, text: &str| fs::write(folder.join(name), text).expect("a file is written");

        // Two packages somebody keeps: `units`, which needs `prefix` by
        // its name, as a published package needs another.
        let (prefix, units) = (work.join("far/prefix"), work.join("far/units"));
        let (prefix_url, units_url) = (address(&prefix), address(&units));
        let files = [(FILE, "package prefix\nabout \"Kilo, mega and the rest\"\n"), ("prefix.zeno", "let kilo = 1000\n")];
        let Some(prefix_first) = repository(&prefix, &files, "one") else {
            eprintln!("no git found; skipping this test");
            return;
        };
        git(Some(&prefix), &["tag", "v1.0.0"]).unwrap();
        let manifest = "package units\nabout \"Lengths, in metres\"\nneeds prefix at \"v1.0.0\"\n";
        let units_first = repository(&units, &[(FILE, manifest), ("units.zeno", "import prefix\nlet km = prefix.kilo\n")], "one").unwrap();
        git(Some(&units), &["tag", "v1.2.0"]).unwrap();

        // A registry: a repository on a server, here a folder, and the
        // clone of it that its keeper works in.
        let (served, keeper, cache) = (work.join("far/registry.git"), work.join("keeper"), work.join("cache"));
        git(None, &["init", "--quiet", "--bare", "--initial-branch=main", &served.to_string_lossy()]).unwrap();
        let registry = address(&served);
        git(None, &["clone", "--quiet", &registry, &keeper.to_string_lossy()]).unwrap();
        let push = |message: &str| {
            repository(&keeper, &[], message).expect("a commit");
            git(Some(&keeper), &["push", "--quiet", "origin", "HEAD:main"]).expect("a push");
        };

        // A program that needs `units` by its name.
        let program = work.join("program");
        fs::create_dir_all(&program).unwrap();
        let needs = |line: &str| write(&program, FILE, &format!("package logger\nregistry \"{registry}\"\n{line}\n"));
        needs("needs units at \"v1.2.0\"");
        let fetch = |update: bool| fetch_with(&program, update, Some(&cache));
        // The registry has nothing in it yet, the first time it is asked and the second.
        for _ in 0..2 {
            let nothing = fetch(false).unwrap_err();
            assert_eq!(nothing, format!("the registry {registry} has no package called `units`. `zeno search` and a word shows the packages it has"));
        }

        // `units` cannot go in before what it needs is there.
        let early = accept(&keeper, &units_url, "v1.2.0", "ada").unwrap_err();
        assert!(early.starts_with("`units` needs `prefix` at `v1.0.0`, and the registry in `"), "{early}");
        assert!(early.contains("has no package called `prefix`"), "{early}");
        let said = accept(&keeper, &prefix_url, "v1.0.0", "ada").expect("it is accepted");
        assert_eq!(said[0], format!("Accepted prefix v1.0.0 ({}) from {prefix_url}", &prefix_first[..7]));
        assert!(said[1].starts_with("Wrote ") && said[1].ends_with("index.json"), "{said:?}");
        accept(&keeper, &format!("{units_url}/"), "v1.2.0", "ada").expect("it is accepted");
        let entry = Index::at(&keeper).entry("units").unwrap().expect("it is there");
        assert_eq!((entry.url.as_str(), entry.owner.as_str(), entry.about.as_str()), (units_url.as_str(), "ada", "Lengths, in metres"));
        assert_eq!(entry.versions.len(), 1);
        assert_eq!((entry.versions[0].tag.as_str(), entry.versions[0].commit.as_str()), ("v1.2.0", units_first.as_str()));
        assert_eq!(entry.versions[0].dated.len(), "2026-11-18".len());
        let list = Json::parse(&fs::read_to_string(keeper.join(LIST)).unwrap()).expect("JSON");
        assert_eq!(list.get("packages").and_then(Json::as_list).map(<[Json]>::len), Some(2));
        push("prefix and units");

        // Now the program's fetch finds both.
        let said = fetch(false).expect("it fetches");
        assert_eq!(said.len(), 4, "{said:?}");
        assert_eq!(said[0], format!("Asked the registry {registry}"));
        assert!(said[1].starts_with("Fetched units v1.2.0 (") && said[2].starts_with("Fetched prefix v1.0.0 ("), "{said:?}");
        assert_eq!(fs::read_to_string(program.join("packages/prefix/prefix.zeno")).unwrap(), "let kilo = 1000\n");
        let lock = fs::read_to_string(program.join(LOCK)).unwrap();
        assert!(
            lock.ends_with(&format!(
                "prefix at \"v1.0.0\" from \"{prefix_url}\" commit \"{prefix_first}\"\nunits at \"v1.2.0\" from \"{units_url}\" commit \"{units_first}\"\n"
            )),
            "{lock}"
        );
        assert!(parse_lock(&lock).iter().all(|pinned| pinned.registry));

        // With the lock, nobody is asked: not the registry, whose copy
        // can be gone, and not the repositories, when the packages are here.
        fs::remove_dir_all(&cache).unwrap();
        let said = fetch(false).expect("it fetches");
        assert!(said.len() == 2 && said.iter().all(|line| line.contains("is already in")), "{said:?}");
        fs::remove_dir_all(program.join("packages")).unwrap();
        let said = fetch(false).expect("it fetches");
        assert!(said.len() == 2 && said.iter().all(|line| line.starts_with("Fetched ")), "{said:?}");
        assert!(!cache.exists());

        // A tag that is moved after it was published moves nothing: the
        // registry names the commit.
        repository(&prefix, &[("prefix.zeno", "let kilo = 1024\n")], "two").unwrap();
        git(Some(&prefix), &["tag", "--force", "v1.0.0"]).unwrap();
        fs::remove_dir_all(program.join("packages")).unwrap();
        let said = fetch(true).expect("it fetches");
        assert_eq!(said[0], format!("Asked the registry {registry}"));
        assert_eq!(fs::read_to_string(program.join("packages/prefix/prefix.zeno")).unwrap(), "let kilo = 1000\n");
        // And the registry does not take the version a second time.
        let moved = accept(&keeper, &prefix_url, "v1.0.0", "ada").unwrap_err();
        assert!(moved.starts_with(&format!("`prefix` v1.0.0 is in the registry already, as the commit `{prefix_first}`")), "{moved}");
        assert!(moved.ends_with("give the new commit a new version"), "{moved}");

        // A version or a name the registry does not have.
        needs("needs units at \"v9.0.0\"");
        assert_eq!(fetch(false).unwrap_err(), "the registry has no version `v9.0.0` of `units`. It has v1.2.0");
        needs("needs unit at \"v1.2.0\"");
        assert!(fetch(false).unwrap_err().contains("has no package called `unit`"));
        // Two needs of one name must mean one thing: here the program
        // names the repository, and `units` names the registry.
        needs(&format!("needs units at \"v1.2.0\"\nneeds prefix from \"{prefix_url}\" at \"v1.0.0\""));
        let two = fetch(false).unwrap_err();
        assert!(two.starts_with("two packages need a package called `prefix`, and do not mean the same one."), "{two}");
        needs("needs units at \"v1.2.0\"");

        // Who may publish what.
        git(Some(&units), &["tag", "v1.3.0"]).unwrap();
        assert_eq!(
            accept(&keeper, &units_url, "v1.3.0", "eve").unwrap_err(),
            "`units` was published by `ada`, and only they publish its versions. This is asked by `eve`"
        );
        let other = work.join("far/other");
        repository(&other, &[(FILE, "package units\nabout \"Another units\"\n")], "one").unwrap();
        git(Some(&other), &["tag", "v1.0.0"]).unwrap();
        assert_eq!(
            accept(&keeper, &address(&other), "v1.0.0", "ada").unwrap_err(),
            format!("the name `units` is taken in the registry: it is the package at {units_url}. Give this package another name")
        );
        accept(&keeper, &units_url, "v1.3.0", "Ada").expect("the owner's next version");
        accept(&keeper, &units_url, "v1.2.0", "ada").expect_err("already there");
        assert_eq!(Index::at(&keeper).version("units", "v1.3.0").unwrap().1, units_first);
        push("units v1.3.0");
        // A file that is changed by hand is found out, and the list is
        // written again from the files.
        let list = fs::read_to_string(keeper.join(LIST)).unwrap();
        fs::remove_file(keeper.join(LIST)).unwrap();
        assert!(relist(&keeper).unwrap()[0].ends_with("index.json: 2 packages"));
        assert_eq!(fs::read_to_string(keeper.join(LIST)).unwrap(), list);
        write(&keeper.join(INDEX), "broken", "package broken\n");
        assert!(relist(&keeper).unwrap_err().ends_with("has a file `index/broken` that is not written as its files are"));
        fs::remove_file(keeper.join(INDEX).join("broken")).unwrap();

        // What is not a version, or not a package.
        git(Some(&other), &["branch", "v2.0.0"]).unwrap();
        let branch = accept(&keeper, &address(&other), "v2.0.0", "ada").unwrap_err();
        assert!(branch.starts_with("`v2.0.0` is a branch of `") && branch.contains("and not a tag"), "{branch}");
        assert!(accept(&keeper, &units_url, "main", "ada").unwrap_err().starts_with("`main` is not a version."));
        assert!(accept(&keeper, &units_url, "v7.7.7", "ada").unwrap_err().starts_with(&format!("cannot fetch `{units_url}` at `v7.7.7`: ")));
        assert_eq!(accept(&keeper, "--upload-pack=x", "v1.0.0", "ada").unwrap_err(), "`--upload-pack=x` is not the address of a git repository");
        let bare = work.join("far/bare");
        repository(&bare, &[("readme.txt", "no package here\n")], "one").unwrap();
        git(Some(&bare), &["tag", "v1.0.0"]).unwrap();
        let none = accept(&keeper, &address(&bare), "v1.0.0", "ada").unwrap_err();
        assert!(none.ends_with("is not a package: there is no file `zeno.package`. The file is looked for at the top of the repository"), "{none}");

        // `zeno search`, in the program's folder, searches its registry.
        let found = search(&["METRES".to_string()], &program, Some(&cache)).expect("it searches");
        assert_eq!(
            found,
            [
                format!("1 package in the registry {registry}:"),
                String::new(),
                "units v1.3.0    Lengths, in metres".to_string(),
                "    needs units at \"v1.3.0\"".to_string(),
                format!("    {units_url}"),
                "    older: v1.2.0".to_string(),
            ]
        );
        assert_eq!(search(&[], &program, Some(&cache)).unwrap()[0], format!("The registry {registry} has 2 packages:"));
        let nothing = search(&["metres".to_string(), "volts".to_string()], &program, Some(&cache)).unwrap();
        assert_eq!(nothing, [format!("No package in the registry {registry} has `metres` and `volts` in its name or in what it says it is for.")]);

        // `zeno publish`: a new package, `wave`, from the first line to
        // the point where the registry can be asked.
        let (wave_served, wave) = (work.join("far/wave.git"), work.join("mine/wave"));
        git(None, &["init", "--quiet", "--bare", "--initial-branch=main", &wave_served.to_string_lossy()]).unwrap();
        let wave_url = address(&wave_served);
        let publish = |tag: Option<&str>| publish(&wave, tag, Some(&cache));
        fs::create_dir_all(&wave).unwrap();
        let header = format!("package wave\nregistry \"{registry}\"\n");
        write(&wave, FILE, &header);
        assert!(publish(None).unwrap_err().starts_with("this folder is not a git repository."));
        repository(&wave, &[("wave.zeno", "let hertz = 50\n")], "one").unwrap();
        assert!(publish(None).unwrap_err().starts_with("the commit this folder is at has no version."));
        git(Some(&wave), &["tag", "v0.1.0"]).unwrap();
        assert!(publish(None).unwrap_err().starts_with("this repository has no `origin`"));
        git(Some(&wave), &["remote", "add", "origin", &wave_url]).unwrap();
        assert_eq!(publish(None).unwrap_err(), format!("the tag `v0.1.0` is not at `{wave_url}` yet. Push it:\n    git push origin v0.1.0"));
        git(Some(&wave), &["push", "--quiet", "origin", "HEAD:main", "v0.1.0"]).unwrap();
        assert!(publish(None).unwrap_err().starts_with("`wave` does not say what it is for."));
        write(&wave, FILE, &format!("{header}about \"Sines and squares\"\nneeds units at \"v1.3.0\"\nneeds helper in \"../helper\"\n"));
        assert!(publish(None).unwrap_err().starts_with("there are changes here that are not committed."));
        repository(&wave, &[], "two").unwrap();
        assert!(publish(None).unwrap_err().starts_with("the commit this folder is at has no version."));
        assert!(publish(Some("v0.1.0")).unwrap_err().starts_with("the tag `v0.1.0` is on another commit than the one this folder is at."));
        assert!(publish(Some("v0.2.0")).unwrap_err().starts_with("there is no tag `v0.2.0` here."));
        assert!(publish(Some("0.2.0")).unwrap_err().starts_with("`0.2.0` is not a version."));
        git(Some(&wave), &["tag", "v0.2.0"]).unwrap();
        git(Some(&wave), &["push", "--quiet", "origin", "HEAD:main", "v0.2.0"]).unwrap();
        let outside = publish(None).unwrap_err();
        assert!(outside.starts_with("`wave` needs `helper` in `../helper`, which is not a folder of this package"), "{outside}");
        write(&wave, FILE, &format!("{header}about \"Sines and squares\"\nneeds units at \"v1.3.0\"\n"));
        repository(&wave, &[], "three").unwrap();
        git(Some(&wave), &["tag", "v0.2.1"]).unwrap();
        git(Some(&wave), &["tag", "v0.3.0"]).unwrap();
        git(Some(&wave), &["push", "--quiet", "origin", "HEAD:main", "v0.2.1", "v0.3.0"]).unwrap();
        assert_eq!(
            publish(None).unwrap_err(),
            "this commit has 2 version tags: v0.2.1, v0.3.0. Say which to publish, as in `zeno publish v0.3.0`"
        );
        let wave_commit = git(Some(&wave), &["rev-parse", "HEAD"]).unwrap();
        let said = publish(Some("v0.3.0")).expect("it is ready");
        assert_eq!(said[0], format!("`wave` v0.3.0 ({}) is ready to be published, from {wave_url}", &wave_commit[..7]));
        assert_eq!(said[4], format!("    zeno registry accept . {wave_url} v0.3.0 --owner NAME"));
        // The keeper runs that line, and then it is published.
        accept(&keeper, &wave_url, "v0.3.0", "ada").expect("it is accepted");
        push("wave v0.3.0");
        assert!(publish(Some("v0.3.0")).unwrap_err().starts_with("`wave` v0.3.0 is in the registry already."));
        assert_eq!(search(&["sines".to_string()], &program, Some(&cache)).unwrap()[2], "wave v0.3.0    Sines and squares");
        let _ = fs::remove_dir_all(&work);
    }

    /// For a registry on GitHub, `publish` ends with the address of a
    /// form. What the address is made of can be checked with no network.
    #[test]
    fn the_form_of_a_registry_on_github_is_filled_in() {
        let form = format!(
            "{}/issues/new?template=publish.yml&title={}&repository={}&version={}",
            tidy(ZENOS_OWN),
            in_address("Publish units v1.2.0"),
            in_address("https://github.com/someone/zeno-units"),
            in_address("v1.2.0")
        );
        assert_eq!(
            form,
            "https://github.com/zeno-src/packages/issues/new?template=publish.yml&title=Publish%20units%20v1.2.0\
             &repository=https%3A%2F%2Fgithub.com%2Fsomeone%2Fzeno-units&version=v1.2.0"
        );
    }
}
