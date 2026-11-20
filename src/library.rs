//! The part of Zeno's library that is written in Zeno.
//!
//! Most of what comes with Zeno is written twice or more: in Rust for
//! `zeno run`, in C for a compiled program, and again for every language
//! `zeno build --target` writes. A function in this part is written
//! once, in Zeno, in a file of the folder `library/`. Those files are
//! kept inside the `zeno` program itself, so nothing has to be installed
//! beside it.
//!
//! `modules.rs` treats such a file as one more file of the program: it
//! is read when a file of the program imports its module and names
//! something it declares, its names get the module's name in front
//! (`math.hypot`), and it is joined in ahead of the program's own files.
//! Of its functions, only the ones the program reaches are kept. So a
//! program that names nothing from this part is, to every stage after
//! `modules.rs`, the program it was before there was such a part.
//!
//! See docs/library-in-zeno.md, and chapter 33 of docs/decisions.md.

/// One module's part written in Zeno.
pub struct Part {
    /// The module: `math`.
    pub module: &'static str,
    /// What the file is called where a message names it.
    pub path: &'static str,
    /// The text of the file.
    pub source: &'static str,
    /// What the file declares for programs to use. A file of the
    /// program that spells none of these cannot be using the part, and
    /// the part is then not even read for it. A test holds this list to
    /// the file.
    pub names: &'static [&'static str],
    /// True if the module also has a part that is built in, as `math`
    /// has `math.sqrt`. A name the file does not declare is then left
    /// for the checker, which knows the built-in ones.
    pub built_in_too: bool,
}

/// Every part, in the order they are joined into a program.
pub const PARTS: &[Part] = &[
    Part {
        module: "math",
        path: "library/math.zeno",
        source: include_str!("../library/math.zeno"),
        names: &["hypot", "sinh", "cosh", "tanh", "gcd", "clamp"],
        built_in_too: true,
    },
    Part {
        module: "files",
        path: "library/files.zeno",
        source: include_str!("../library/files.zeno"),
        names: &["join"],
        built_in_too: true,
    },
    Part {
        module: "random",
        path: "library/random.zeno",
        source: include_str!("../library/random.zeno"),
        names: &["seed", "number", "between", "pick"],
        built_in_too: false,
    },
];

/// The part of `module` that is written in Zeno, if it has one.
pub fn part(module: &str) -> Option<&'static Part> {
    PARTS.iter().find(|part| part.module == module)
}

/// True for a module that is written in Zeno and nothing else, such as
/// `random`. No stage after `modules.rs` ever sees its `import` line.
pub fn all_in_zeno(module: &str) -> bool {
    part(module).is_some_and(|part| !part.built_in_too)
}

impl Part {
    /// True if `source`, the text of a file that imports the module,
    /// spells one of the names this part declares. Only then can the
    /// file be using it.
    pub fn spelt_in(&self, source: &str) -> bool {
        self.names.iter().any(|name| source.contains(name))
    }
}
