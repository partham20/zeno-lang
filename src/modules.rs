//! Programs in several files.
//!
//! `import shapes` at the top of a file brings in the file `shapes.zeno`
//! from the folder of the program's first file, and `shapes.area(2.0)`
//! then calls the `area` that file declares. `from shapes import area`
//! brings in the one name, to be written as plain `area`. These are the
//! two forms that `import math` and `from math import sqrt` already have
//! for the modules that come with Zeno.
//!
//! This stage runs between the parser and the type checker, and it is the
//! only one that knows a program can be several files. It does three
//! things:
//!
//! 1. **Load.** Read the first file, find its imports, read those files,
//!    find their imports, and so on. Each file is read once however many
//!    files import it. Files that import each other are refused.
//! 2. **Rename.** Give every name that an imported file declares at its
//!    top the file's module name in front: `area` in `shapes.zeno` becomes
//!    `shapes.area`, in the file itself and in every file that uses it.
//!    Two files can then each have an `area` without meeting.
//! 3. **Join.** Put the statements of all the files into one program, an
//!    imported file always before the file that imports it.
//!
//! What comes out is one ordinary program. The type checker, the
//! interpreter, the VM and both compilers take it as they take a program
//! written in one file, and need no changes to know about modules. A name
//! with a dot in it, like `shapes.area`, is to them one name like any
//! other. Nobody can write such a name as one word in a source file, so
//! it can never collide with a name somebody did write.
//!
//! See docs/modules.md for a walk through this file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ast::*;
use crate::builtins;
use crate::error::Error;
use crate::lexer;
use crate::parser;
use crate::token::{Span, LINES_A_FILE, MOST_FILES};

/// One source file of a program.
pub struct File {
    /// Where the file is, as it is shown in an error message.
    pub path: String,
    pub source: String,
}

/// What loading a program gives.
pub struct Loaded {
    /// Every file that was read. The first is the file the program starts
    /// in. A `Span` says which of these it is in: see `Span::file`.
    pub files: Vec<File>,
    /// The files joined into one program, or the mistakes that kept them
    /// from being joined.
    pub program: Result<Program, Vec<Error>>,
}

/// Read the program that starts in the file at `path`, whose text is
/// `source`. `read` gives the text of another file, or `None` if there is
/// no such file. (The tests hand in files that exist only in memory.)
pub fn load(path: &str, source: String, read: &dyn Fn(&str) -> Option<String>) -> Loaded {
    let root = Path::new(path).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut loader = Loader {
        read,
        root,
        files: Vec::new(),
        units: Vec::new(),
        numbers: HashMap::new(),
        order: Vec::new(),
        loading: Vec::new(),
        missing: HashSet::new(),
        errors: Vec::new(),
    };
    loader.file(String::new(), path.to_string(), source);
    let program = match loader.errors.is_empty() {
        true => loader.join(),
        false => Err(std::mem::take(&mut loader.errors)),
    };
    Loaded { files: loader.files, program }
}

// ------------------------------------------------------------------- Loading

struct Loader<'a> {
    read: &'a dyn Fn(&str) -> Option<String>,
    /// The folder of the program's first file. Every module is looked for
    /// from here, whichever file imports it, so that one name always
    /// means one file.
    root: PathBuf,
    files: Vec<File>,
    /// What was found in each file, in the order of `files`. `None` for a
    /// file that is still being read, or that has a mistake in it.
    units: Vec<Option<Unit>>,
    /// The number of the file each module is.
    numbers: HashMap<String, usize>,
    /// The files in the order their statements go into the program: each
    /// one after the files it imports.
    order: Vec<usize>,
    /// The modules whose imports are being followed right now, the first
    /// file's first. A module that turns up here a second time is
    /// importing itself, directly or through others.
    loading: Vec<String>,
    /// The modules that were looked for and not found, so that each is
    /// reported once however many lines import it.
    missing: HashSet<String>,
    errors: Vec<Error>,
}

/// One file, parsed.
struct Unit {
    /// `shapes`, or `drivers.uart`. Empty for the program's first file,
    /// which nothing imports and whose names stay as they are written.
    module: String,
    program: Program,
    /// The files this one imports, in the order it lists them.
    imports: Vec<Import>,
}

/// `import shapes` (no names) or `from shapes import area, Point`.
struct Import {
    module: String,
    names: Vec<String>,
    span: Span,
}

impl Loader<'_> {
    /// Read one file and, before going on, every file it imports.
    fn file(&mut self, module: String, path: String, source: String) {
        let number = self.files.len();
        // The first line of this file, for a mistake that is about the
        // file as a whole.
        let top = Span { line: number * LINES_A_FILE + 1, col: 1 };
        let parsed = match source.lines().count() < LINES_A_FILE {
            true => lexer::lex_in(&source, number).and_then(|tokens| parser::parse(tokens).map_err(|error| vec![error])),
            false => Err(vec![Error::new("this file has a million lines or more, and Zeno reads at most 999,999. Split it", top)]),
        };
        self.files.push(File { path, source });
        self.units.push(None);
        let program = match parsed {
            Ok(program) => program,
            Err(errors) => return self.errors.extend(errors),
        };

        // The imports of files are the leading lines of the file. An
        // imported file runs before the file that imports it, wherever
        // the import is written, so the top is the one honest place for it.
        let mut imports = Vec::new();
        let mut below = false;
        for statement in &program.statements {
            let StmtKind::Import { module, names } = &statement.kind else {
                below = true;
                continue;
            };
            let module = module.join(".");
            if builtins::comes_with_zeno(&module) {
                continue;
            }
            if below {
                self.errors.push(Error::new(
                    format!(
                        "`import {module}` goes at the top of the file, above everything that is not an import. \
                         The file `{module}` runs first wherever this line is, so the top is where the line belongs"
                    ),
                    statement.span,
                ));
                continue;
            }
            imports.push(Import { module, names: names.clone(), span: statement.span });
        }

        self.loading.push(module.clone());
        for import in &imports {
            if let Some(first) = self.loading.iter().position(|loading| *loading == import.module) {
                let mut circle: Vec<&str> = self.loading[first..].iter().map(String::as_str).collect();
                circle.push(&import.module);
                self.errors.push(Error::new(in_a_circle(&circle), import.span));
                continue;
            }
            if self.numbers.contains_key(&import.module) || self.missing.contains(&import.module) {
                continue; // Read already, for another line that imports it.
            }
            let mut path = self.root.clone();
            import.module.split('.').for_each(|part| path.push(part));
            let path = format!("{}.zeno", path.display());
            let Some(source) = (self.read)(&path) else {
                self.missing.insert(import.module.clone());
                self.errors.push(Error::new(
                    format!(
                        "there is no module `{}`. `import {}` reads the file `{path}`, and there is no such file. \
                         {}",
                        import.module,
                        import.module,
                        builtins::MODULES
                    ),
                    import.span,
                ));
                continue;
            };
            if self.files.len() >= MOST_FILES {
                self.errors.push(Error::new(format!("a program can have {MOST_FILES} files, and this would be one more"), import.span));
                continue;
            }
            self.numbers.insert(import.module.clone(), self.files.len());
            self.file(import.module.clone(), path, source);
        }
        self.loading.pop();

        // Everything this file imports is in `order` by now, so this
        // file comes after all of it.
        self.order.push(number);
        self.units[number] = Some(Unit { module, program, imports });
    }

    /// Rename the names of every file, and join the files into one program.
    fn join(&mut self) -> Result<Program, Vec<Error>> {
        let mut units: Vec<Unit> = self.units.drain(..).map(|unit| unit.expect("every file was read")).collect();

        // What each module declares at its top: these are the names
        // another file can ask it for.
        let mut declared: HashMap<String, Vec<String>> = HashMap::new();
        let mut cases: HashSet<String> = HashSet::new();
        // The cases of every enum, by the enum's full name.
        let mut enums: HashMap<String, Vec<String>> = HashMap::new();
        for unit in &units {
            let full = |name: &str| match unit.module.is_empty() {
                true => name.to_string(),
                false => format!("{}.{name}", unit.module),
            };
            for statement in &unit.program.statements {
                if let StmtKind::Enum(decl) = &statement.kind {
                    enums.insert(full(&decl.name), decl.variants.iter().map(|case| case.name.clone()).collect());
                }
            }
            if unit.module.is_empty() {
                continue;
            }
            let mut names = Vec::new();
            for (name, _, is_case) in declarations(&unit.program) {
                if is_case {
                    cases.insert(full(&name));
                }
                names.push(name);
            }
            declared.insert(unit.module.clone(), names);
        }

        // What the first file declares, and whether each is an enum
        // case. An imported file must not come to mean one of these by
        // an accident of spelling: see `Renamer::plain`.
        let first_file: HashMap<String, bool> =
            declarations(&units[0].program).into_iter().map(|(name, _, is_case)| (name, is_case)).collect();

        let mut errors = Vec::new();
        for unit in &mut units {
            let mut renamer = Renamer::new(&unit.module, &declared, &cases, &enums, &first_file);
            renamer.imports(&unit.imports);
            renamer.own(&declarations(&unit.program));
            for statement in &mut unit.program.statements {
                renamer.statement(statement, true);
            }
            errors.extend(renamer.errors);
            // The imports of files have done their work.
            unit.program.statements.retain(|statement| match &statement.kind {
                StmtKind::Import { module, .. } => builtins::comes_with_zeno(&module.join(".")),
                _ => true,
            });
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut statements = Vec::new();
        for &number in &self.order {
            statements.append(&mut units[number].program.statements);
        }
        let modules = units
            .iter()
            .zip(&self.files)
            .skip(1)
            .map(|(unit, file)| ModuleFile { path: file.path.clone(), strict: unit.program.strict })
            .collect();
        Ok(Program { strict: units[0].program.strict, statements, modules })
    }
}

/// The error for files that import each other. `circle` begins and ends
/// with the same module: `a`, `b`, `a`.
fn in_a_circle(circle: &[&str]) -> String {
    if circle.len() == 2 {
        return format!("`{}` imports itself. A file cannot import the file it is", circle[0]);
    }
    let steps: Vec<String> = circle.windows(2).map(|pair| format!("`{}` imports `{}`", pair[0], pair[1])).collect();
    let (last, before) = steps.split_last().expect("a circle has a step");
    format!(
        "these files import each other: {}, and {last}. An imported file runs before the file that imports it, \
         and here none of them can be the first. Move what they share into a file of its own, and import that",
        before.join(", ")
    )
}

/// The names a file declares at its top: each with where, and whether it
/// is a case of an enum.
fn declarations(program: &Program) -> Vec<(String, Span, bool)> {
    let mut names = Vec::new();
    for statement in &program.statements {
        let span = statement.span;
        match &statement.kind {
            StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => names.push((name.clone(), span, false)),
            StmtKind::Function(function) => names.push((function.name.clone(), span, false)),
            StmtKind::Class(class) => names.push((class.name.clone(), span, false)),
            StmtKind::Enum(decl) => {
                names.push((decl.name.clone(), span, false));
                names.extend(decl.variants.iter().map(|case| (case.name.clone(), span, true)));
            }
            _ => {}
        }
    }
    names
}

/// "`area`, `Point` and `origin`".
fn listed(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((only, [])) => only.clone(),
        Some((last, before)) => format!("{} and {last}", before.join(", ")),
    }
}

/// The error for a name that a module does not declare.
fn no_such_name(module: &str, name: &str, names: &[String]) -> String {
    match names.is_empty() {
        true => format!("module `{module}` has no `{name}`. Its file declares nothing at the top"),
        false => format!("module `{module}` has no `{name}`. It has {}", listed(names)),
    }
}

// ------------------------------------------------------------------ Renaming

/// Walks one file and gives every name that belongs to a module its full
/// name.
///
/// There are three ways a file reaches such a name, and after this walk
/// all three are the one word `shapes.area`:
///
/// - `shapes.area`, after `import shapes`. The parser read that as the
///   field `area` of a variable `shapes`.
/// - `area`, after `from shapes import area`.
/// - `area`, inside `shapes.zeno` itself.
///
/// A name that a function or a block declares for itself hides a name
/// from the top of the file, as it does everywhere in Zeno. So the walk
/// keeps track of what is declared where it is: `scopes`.
struct Renamer<'a> {
    /// The module this file is. Empty for the program's first file.
    module: &'a str,
    /// What every module declares at its top.
    declared: &'a HashMap<String, Vec<String>>,
    /// The full names of the enum cases that modules declare.
    cases: &'a HashSet<String>,
    /// The cases of every enum of the program, by the enum's full name.
    enums: &'a HashMap<String, Vec<String>>,
    /// What the program's first file declares at its top, and whether
    /// each is an enum case.
    first_file: &'a HashMap<String, bool>,
    /// The modules this file imports whole, with `import shapes`.
    whole: HashSet<String>,
    /// Every way the name of one of those begins: `drivers` and
    /// `drivers.uart` for `import drivers.uart`.
    starts: HashSet<String>,
    /// What a plain name written in this file stands for: the file's own
    /// declarations (when it is a module), and the names it imports with
    /// `from`.
    top: HashMap<String, String>,
    /// The module each name imported with `from` comes from.
    from: HashMap<String, String>,
    /// The names declared by the functions and blocks around the place
    /// the walk is at, the innermost last.
    scopes: Vec<HashSet<String>>,
    errors: Vec<Error>,
}

impl<'a> Renamer<'a> {
    fn new(
        module: &'a str,
        declared: &'a HashMap<String, Vec<String>>,
        cases: &'a HashSet<String>,
        enums: &'a HashMap<String, Vec<String>>,
        first_file: &'a HashMap<String, bool>,
    ) -> Renamer<'a> {
        Renamer {
            module,
            declared,
            cases,
            enums,
            first_file,
            whole: HashSet::new(),
            starts: HashSet::new(),
            top: HashMap::new(),
            from: HashMap::new(),
            scopes: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.errors.push(Error::new(message, span));
    }

    /// Note what the file's imports bring in.
    fn imports(&mut self, imports: &[Import]) {
        for import in imports {
            let module = &import.module;
            let Some(names) = self.declared.get(module) else { continue };
            if import.names.is_empty() {
                // `import a` and `import a.b` in one file would give
                // `a.b.x` two readings.
                let clash = self.whole.iter().find(|other| {
                    other.starts_with(&format!("{module}.")) || module.starts_with(&format!("{other}."))
                });
                if let Some(other) = clash.cloned() {
                    let (short, long) = if other.len() < module.len() { (&other, module) } else { (module, &other) };
                    let rest = &long[short.len() + 1..];
                    self.error(
                        format!(
                            "this file imports `{short}` and `{long}`. `{long}.x` would then have two readings: \
                             `x` of the module `{long}`, and `{rest}.x` of the module `{short}`. \
                             Import one of them with `from`, as in `from {long} import x`"
                        ),
                        import.span,
                    );
                    continue;
                }
                self.whole.insert(module.clone());
                let mut start = String::new();
                for part in module.split('.') {
                    start = if start.is_empty() { part.to_string() } else { format!("{start}.{part}") };
                    self.starts.insert(start.clone());
                }
            }
            for name in &import.names {
                if !names.contains(name) {
                    self.error(no_such_name(module, name, names), import.span);
                    continue;
                }
                match self.from.get(name) {
                    Some(other) if other != module => {
                        let other = other.clone();
                        self.error(
                            format!(
                                "`{name}` is imported twice, from `{other}` and from `{module}`. Import one of the \
                                 two modules whole, as in `import {module}`, and write `{module}.{name}`"
                            ),
                            import.span,
                        );
                    }
                    _ => {
                        self.from.insert(name.clone(), module.clone());
                        self.top.insert(name.clone(), format!("{module}.{name}"));
                    }
                }
            }
        }
    }

    /// Note what the file itself declares at its top.
    fn own(&mut self, declarations: &[(String, Span, bool)]) {
        for (name, span, _) in declarations {
            if let Some(module) = self.from.get(name).cloned() {
                self.error(
                    format!(
                        "`{name}` is imported from `{module}` at the top of this file, so the file cannot declare \
                         a second `{name}`. Give this one another name, or import the module whole with \
                         `import {module}` and write `{module}.{name}` for the other"
                    ),
                    *span,
                );
            } else if self.starts.contains(name) {
                self.error(
                    format!(
                        "this file imports the module `{name}`, so it cannot also declare `{name}`: \
                         `{name}.x` would have two readings. Give this one another name"
                    ),
                    *span,
                );
            } else if !self.module.is_empty() {
                self.top.insert(name.clone(), format!("{}.{name}", self.module));
            }
        }
    }

    /// True if a function or a block around here declares `name`.
    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    fn declare(&mut self, name: &str) {
        self.scopes.last_mut().expect("a scope is open").insert(name.to_string());
    }

    /// A plain name, where one is used: give it its full name if it is a
    /// module's.
    ///
    /// The names of the program's first file stay as they are written,
    /// so in the joined program an imported file could reach one of them
    /// by writing the same word. What `shapes.zeno` means would then
    /// depend on which program imports it. A file sees what it declares
    /// and what it imports, and nothing else, so that is refused here.
    fn plain(&mut self, name: &mut String, span: Span) {
        if self.is_local(name) {
            return;
        }
        if let Some(full) = self.top.get(name.as_str()) {
            *name = full.clone();
        } else if !self.module.is_empty() && self.first_file.contains_key(name.as_str()) {
            self.unseen(name, span);
        }
    }

    fn unseen(&mut self, name: &str, span: Span) {
        self.error(
            format!(
                "this file declares no `{name}`. The program's first file has one, and an imported file \
                 cannot see what the file that imports it declares. Hand the value to the function that \
                 needs it, as an argument"
            ),
            span,
        );
    }

    // ----- Statements -----

    /// The statements between `{` and `}`. Functions, classes and enums
    /// can be used above the line that declares them, so their names are
    /// noted before anything in the block is walked.
    fn block(&mut self, block: &mut Block) {
        let mut scope = HashSet::new();
        for statement in block.iter() {
            match &statement.kind {
                StmtKind::Function(function) => drop(scope.insert(function.name.clone())),
                StmtKind::Class(class) => drop(scope.insert(class.name.clone())),
                StmtKind::Enum(decl) => {
                    scope.insert(decl.name.clone());
                    scope.extend(decl.variants.iter().map(|case| case.name.clone()));
                }
                _ => {}
            }
        }
        self.scopes.push(scope);
        for statement in block {
            self.statement(statement, false);
        }
        self.scopes.pop();
    }

    /// `at_top` is true for a statement at the top of the file: what it
    /// declares is the module's, and gets the module's name in front.
    fn statement(&mut self, statement: &mut Stmt, at_top: bool) {
        let span = statement.span;
        let module = self.module;
        let full = |name: &str| format!("{module}.{name}");
        let renamed = at_top && !module.is_empty();
        match &mut statement.kind {
            StmtKind::Let { name, ty, value, .. } | StmtKind::Const { name, ty, value } => {
                if let Some(ty) = ty {
                    self.ty(ty, span);
                }
                // The value is worked out before the name exists, so a
                // `let x = x + 1` in a block reads the `x` outside it.
                self.expr(value);
                if renamed {
                    *name = full(name);
                } else if !at_top {
                    self.declare(name);
                }
            }
            StmtKind::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
                if let ExprKind::Name(name) = &target.kind {
                    if let Some((owner, _)) = name.rsplit_once('.').filter(|(owner, _)| *owner != module) {
                        self.error(
                            format!(
                                "`{name}` is a variable of the module `{owner}`, and only the file of `{owner}` \
                                 gives it a new value. Give `{owner}` a function that changes it, and call that"
                            ),
                            span,
                        );
                    }
                }
            }
            StmtKind::Expr(expr) | StmtKind::Raise(expr) => self.expr(expr),
            StmtKind::Return(value) => value.iter_mut().for_each(|value| self.expr(value)),

            StmtKind::Function(function) => {
                let function = Arc::make_mut(function);
                if renamed {
                    function.name = full(&function.name);
                }
                self.function(function, span);
            }
            StmtKind::Class(class) => {
                if renamed {
                    class.name = full(&class.name);
                }
                self.scopes.push(class.type_params.iter().cloned().collect());
                for field in &mut class.fields {
                    self.field(field, span);
                }
                for method in &mut class.methods {
                    self.function(Arc::make_mut(method), span);
                }
                self.scopes.pop();
            }
            StmtKind::Enum(decl) => {
                if renamed {
                    decl.name = full(&decl.name);
                }
                for case in &mut decl.variants {
                    if renamed {
                        case.name = full(&case.name);
                    }
                    for field in &mut case.fields {
                        self.field(field, span);
                    }
                }
            }

            StmtKind::If { condition, then_block, else_block } => {
                self.expr(condition);
                self.block(then_block);
                else_block.iter_mut().for_each(|block| self.block(block));
            }
            StmtKind::While { condition, body } => {
                self.expr(condition);
                self.block(body);
            }
            StmtKind::For { variable, iterable, body } => {
                self.expr(iterable);
                self.scopes.push(HashSet::from([variable.clone()]));
                self.block(body);
                self.scopes.pop();
            }
            StmtKind::Match { subject, arms } => {
                self.expr(subject);
                for arm in arms {
                    self.scopes.push(HashSet::new());
                    self.pattern(&mut arm.pattern, span);
                    self.block(&mut arm.body);
                    self.scopes.pop();
                }
            }
            StmtKind::Try { body, handlers } => {
                self.block(body);
                for handler in handlers {
                    if let Some(error) = &mut handler.error {
                        self.named(error, span);
                    }
                    self.scopes.push(handler.binding.iter().cloned().collect());
                    self.block(&mut handler.body);
                    self.scopes.pop();
                }
            }
            StmtKind::Unsafe(body) => self.block(body),
            StmtKind::Break | StmtKind::Continue => {}

            // At the top of the file `Loader::file` has seen to it.
            StmtKind::Import { module, .. } => {
                let module = module.join(".");
                if !at_top && !builtins::comes_with_zeno(&module) {
                    self.error(
                        format!(
                            "`import {module}` goes at the top of the file, not inside a function or a block. \
                             Only a module that comes with Zeno, such as `math`, can be imported in here"
                        ),
                        span,
                    );
                }
            }
        }
    }

    /// A function's types, then its body with the parameters declared.
    fn function(&mut self, function: &mut Function, span: Span) {
        self.scopes.push(function.type_params.iter().cloned().collect());
        for param in &mut function.params {
            if let Some(ty) = &mut param.ty {
                self.ty(ty, span);
            }
            if let Some(default) = &mut param.default {
                self.expr(default);
            }
        }
        if let Some(ty) = &mut function.return_type {
            self.ty(ty, span);
        }
        for error in &mut function.raises {
            self.named(error, span);
        }
        for param in &function.params {
            self.declare(&param.name);
        }
        self.block(&mut function.body);
        self.scopes.pop();
    }

    /// A field of a class, or a value that an enum case carries.
    fn field(&mut self, field: &mut Field, span: Span) {
        if let Some(ty) = &mut field.ty {
            self.ty(ty, span);
        }
        if let Some(default) = &mut field.default {
            self.expr(default);
        }
    }

    fn pattern(&mut self, pattern: &mut Pattern, span: Span) {
        match pattern {
            Pattern::Wildcard | Pattern::Literal(_) => {}
            // `case shapes.Idle`
            Pattern::Name(name) if name.contains('.') => self.case(name, span),
            Pattern::Name(name) => {
                // A plain name is an enum case if it stands for one, and
                // a new variable for the value otherwise.
                let case = self.top.get(name.as_str()).filter(|full| self.cases.contains(*full));
                match case {
                    Some(full) if !self.is_local(name) => *name = full.clone(),
                    _ => {
                        // A case of the first file's would be compared
                        // with, where this file means a new variable.
                        let is_case = self.first_file.get(name.as_str()) == Some(&true);
                        if is_case && !self.module.is_empty() && !self.is_local(name) {
                            let name = name.clone();
                            self.unseen(&name, span);
                        }
                        self.declare(name);
                    }
                }
            }
            Pattern::Variant { name, fields } => {
                self.case(name, span);
                for field in fields {
                    self.pattern(field, span);
                }
            }
        }
    }

    /// The name of an enum case in a pattern. A case can be written with
    /// its enum in front, `Shape.Circle(r)`, as it can in an expression.
    /// Here that becomes the case's own name, which is what the stages
    /// after this one match by.
    fn case(&mut self, name: &mut String, span: Span) {
        if let Some((owner, case)) = name.rsplit_once('.') {
            let mut owner = owner.to_string();
            if !owner.contains('.') && !self.is_local(&owner) {
                if let Some(full) = self.top.get(&owner) {
                    owner = full.clone();
                }
            }
            if self.enums.get(&owner).is_some_and(|cases| cases.iter().any(|known| known == case)) {
                *name = match owner.rsplit_once('.') {
                    Some((module, _)) => format!("{module}.{case}"),
                    None => case.to_string(),
                };
                return;
            }
        }
        self.named(name, span);
    }

    // ----- Names in types -----

    fn ty(&mut self, ty: &mut Type, span: Span) {
        match ty {
            Type::Named { name, args } => {
                self.named(name, span);
                for arg in args {
                    self.ty(arg, span);
                }
            }
            Type::Optional(inner) | Type::Owned(inner) | Type::Take(inner) => self.ty(inner, span),
            Type::Function { params, returns } => {
                for param in params {
                    self.ty(param, span);
                }
                if let Some(returns) = returns {
                    self.ty(returns, span);
                }
            }
        }
    }

    /// A name in one of the places where the parser reads `a.b` as one
    /// name: a type, an enum case in a pattern, an error after `except`
    /// or `raises`.
    fn named(&mut self, name: &mut String, span: Span) {
        let Some((module, member)) = name.rsplit_once('.') else {
            return self.plain(name, span);
        };
        // `shapes.Point` is already the full name. It only has to exist.
        match self.declared.get(module).filter(|_| self.whole.contains(module)) {
            Some(names) if names.iter().any(|declared| declared == member) => {}
            Some(names) => self.error(no_such_name(module, member, names), span),
            None => self.error(
                format!(
                    "this file imports no module `{module}`, so `{name}` means nothing here. \
                     Write `import {module}` at the top of the file"
                ),
                span,
            ),
        }
    }

    // ----- Expressions -----

    /// If `expr` is the name of a module this file imports, or the start
    /// of one (`drivers` of `drivers.uart`), that name.
    fn module_named(&self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::Name(name) if self.starts.contains(name) && !self.is_local(name) => Some(name.clone()),
            ExprKind::Field { object, name } => {
                let before = self.module_named(object)?;
                // After a whole module's name comes one of its names, and
                // that is not part of any module's name.
                let longer = format!("{before}.{name}");
                (!self.whole.contains(&before) && self.starts.contains(&longer)).then_some(longer)
            }
            _ => None,
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        let span = expr.span;
        // `shapes.area`: the parser read a field of a variable `shapes`.
        // It is the name `area` of the module `shapes`.
        if let ExprKind::Field { object, name } = &expr.kind {
            if let Some(module) = self.module_named(object).filter(|module| self.whole.contains(module)) {
                let names = &self.declared[&module];
                match names.contains(name) {
                    true => expr.kind = ExprKind::Name(format!("{module}.{name}")),
                    false => self.error(no_such_name(&module, name, names), span),
                }
                return;
            }
            if let Some(start) = self.module_named(object) {
                if !self.starts.contains(&format!("{start}.{name}")) {
                    return self.error(
                        format!(
                            "this file imports no module `{start}.{name}`. \
                             Write `import {start}.{name}` at the top of the file"
                        ),
                        span,
                    );
                }
            }
        }
        // A module's name with nothing after it.
        if let Some(module) = self.module_named(expr) {
            let example = match self.declared.get(&module).and_then(|names| names.first()) {
                Some(name) => format!(", as in `{module}.{name}`"),
                None => String::new(),
            };
            return self.error(
                format!("`{module}` is a module, not a value. Write `{module}.` and then a name from it{example}"),
                span,
            );
        }

        match &mut expr.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::None => {}
            ExprKind::Name(name) => self.plain(name, span),
            ExprKind::FormatStr(parts) => {
                for part in parts {
                    if let FormatPart::Value { value, .. } = part {
                        self.expr(value);
                    }
                }
            }
            ExprKind::Unit { value, .. } => self.expr(value),
            ExprKind::List(items) => items.iter_mut().for_each(|item| self.expr(item)),
            ExprKind::Dict(entries) => {
                for (key, value) in entries {
                    self.expr(key);
                    self.expr(value);
                }
            }
            ExprKind::ListComp { element, variable, iterable, condition } => {
                self.expr(iterable);
                self.scopes.push(HashSet::from([variable.clone()]));
                self.expr(element);
                if let Some(condition) = condition {
                    self.expr(condition);
                }
                self.scopes.pop();
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Compare { first, rest } => {
                self.expr(first);
                rest.iter_mut().for_each(|(_, operand)| self.expr(operand));
            }
            ExprKind::Function(function) => self.function(Arc::make_mut(function), span),
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                args.iter_mut().for_each(|arg| self.expr(&mut arg.value));
            }
            ExprKind::Field { object, name } => {
                self.expr(object);
                // `Shape.Dot`, where `Shape` is an enum of a module: the
                // case has the module's name in front, like every name
                // the module declares.
                if let ExprKind::Name(owner) = &object.kind {
                    if let Some((module, _)) = owner.rsplit_once('.') {
                        if self.enums.get(owner).is_some_and(|cases| cases.contains(name)) {
                            expr.kind = ExprKind::Name(format!("{module}.{name}"));
                        }
                    }
                }
            }
            ExprKind::Index { object, index } => {
                self.expr(object);
                self.expr(index);
            }
        }
    }
}

// ---------------------------------------------------------------- For tests

/// Programs in several files for the tests of every stage, with no
/// files on disk.
#[cfg(test)]
pub mod testing {
    use super::*;

    /// A test program is one text. A line that begins with `==>` starts
    /// another file, and names it:
    ///
    /// ```text
    /// import shapes
    /// print(shapes.area(2))
    /// ==> shapes.zeno
    /// function area(side: int) -> int { return side * side }
    /// ```
    ///
    /// The text above the first such line is the file the program starts
    /// in, `test.zeno`. A text with no such line is a program in one file.
    pub fn load(source: &str) -> Loaded {
        let mut files: Vec<(String, String)> = vec![("test.zeno".to_string(), String::new())];
        for line in source.split_inclusive('\n') {
            match line.strip_prefix("==> ") {
                Some(name) => files.push((name.trim().to_string(), String::new())),
                None => files.last_mut().expect("there is a file").1 += line,
            }
        }
        let first = files.remove(0).1;
        super::load("test.zeno", first, &|path| {
            files.iter().find(|(name, _)| name == path).map(|(_, source)| source.clone())
        })
    }

    /// The program, joined. A mistake in a test program stops the test.
    pub fn program(source: &str) -> Program {
        let loaded = load(source);
        match loaded.program {
            Ok(program) => program,
            Err(errors) => panic!("the test program has mistakes: {:?}\n{source}", errors[0].message),
        }
    }

    /// Where a compiled program says an error is: `test.zeno:3:5`, or
    /// `shapes.zeno:2:12` for one in an imported file.
    pub fn place(program: &Program, span: Span) -> String {
        let path = match span.file() {
            0 => "test.zeno",
            file => &program.modules[file - 1].path,
        };
        format!("{path}:{}:{}", span.line_in_file(), span.col)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{load, program};
    use crate::printer;
    use std::cell::RefCell;

    /// The first mistake found in a program, and the file and line it is in.
    fn mistake(source: &str) -> (String, usize, usize) {
        let errors = load(source).program.expect_err("the program should have a mistake");
        (errors[0].message.clone(), errors[0].span.file(), errors[0].span.line_in_file())
    }

    fn error(source: &str) -> String {
        mistake(source).0
    }

    /// A small module that most of these tests import.
    const SHAPES: &str = "==> shapes.zeno\n\
        let sides = 4\n\
        var made = 0\n\
        enum Shape {\n    Dot\n    Square(side: int)\n}\n\
        class Point {\n    x: int\n}\n\
        function area(shape: Shape) -> int {\n    made += 1\n    match shape {\n        case Dot { return 0 }\n        case Square(side) { return side * side }\n    }\n}\n";

    #[test]
    fn a_program_in_one_file_is_left_as_it_is() {
        let source = "import math\nlet x = math.sqrt(4.0)\nfunction twice(n: int) -> int { return n * 2 }\nprint(x, twice(2))\n";
        let alone = crate::parser::parse(crate::lexer::lex(source).unwrap()).unwrap();
        assert_eq!(program(source), alone);
    }

    #[test]
    fn the_names_of_an_imported_file_get_its_name_in_front() {
        let joined = program(&format!(
            "import shapes\nfrom shapes import Point, Square\n\
             let p: Point = Point(1)\nlet q: shapes.Point = shapes.Point(2)\n\
             print(shapes.area(Square(3)), shapes.sides, shapes.Shape.Dot)\n{SHAPES}"
        ));
        // The imported file's statements come first, every name it
        // declares at its top with `shapes.` in front. The two imports
        // are gone: they have done their work.
        assert_eq!(
            printer::print(&joined),
            "let shapes.sides = 4\n\
             var shapes.made = 0\n\
             enum shapes.Shape\n    shapes.Dot\n    shapes.Square(side: int)\n\
             class shapes.Point\n    field x: int\n\
             function shapes.area(shape: shapes.Shape) -> int\n    \
                 shapes.made += 1\n    \
                 match shape\n        \
                     case shapes.Dot\n            return 0\n        \
                     case shapes.Square(side)\n            return (side * side)\n\
             let p: shapes.Point = shapes.Point(1)\n\
             let q: shapes.Point = shapes.Point(2)\n\
             print(shapes.area(shapes.Square(3)), shapes.sides, shapes.Dot)\n"
        );
        assert_eq!(joined.modules.len(), 1);
        assert_eq!(joined.modules[0].path, "shapes.zeno");
    }

    #[test]
    fn a_name_declared_nearer_hides_an_imported_one() {
        let joined = program(&format!(
            "import shapes\nfrom shapes import area, sides\n\
             function own(area: int) -> int {{\n    let shapes = [area]\n    return shapes[0] + sides\n}}\n\
             for sides in [1] {{\n    print(sides)\n}}\n\
             print([area for area in [2]], sides)\n{SHAPES}"
        ));
        let text = printer::print(&joined);
        let main = &text[text.find("function own").expect("the first file is there")..];
        assert_eq!(
            main,
            "function own(area: int) -> int\n    let shapes = [area]\n    return (shapes[0] + shapes.sides)\n\
             for sides in [1]\n    print(sides)\n\
             print([area for area in [2]], shapes.sides)\n"
        );
    }

    #[test]
    fn a_case_can_be_written_with_its_enum_in_front() {
        // In a pattern too, for an enum of this file or of another.
        let joined = program(&format!(
            "import shapes\nfrom shapes import Shape\n\
             enum Light {{\n    Off\n    On\n}}\n\
             match Light.On {{\n    case Light.Off {{ print(0) }}\n    case On {{ print(1) }}\n}}\n\
             match shapes.Dot {{\n    case Shape.Dot {{ print(2) }}\n    case shapes.Shape.Square(side) {{ print(side) }}\n}}\n{SHAPES}"
        ));
        let text = printer::print(&joined);
        assert!(text.contains("match Light.On\n    case Off\n        print(0)\n    case On\n        print(1)\n"), "{text}");
        assert!(text.contains("match shapes.Dot\n    case shapes.Dot\n        print(2)\n    case shapes.Square(side)\n"), "{text}");
    }

    #[test]
    fn a_file_is_read_once_and_runs_before_what_imports_it() {
        // `test` imports `a` and `b`, and both import `base`.
        let source = "import a\nimport b\nprint(\"test\")\n\
                      ==> a.zeno\nimport base\nprint(\"a\")\n\
                      ==> b.zeno\nimport base\nimport a\nprint(\"b\")\n\
                      ==> base.zeno\nprint(\"base\")\n";
        assert_eq!(printer::print(&program(source)), "print(\"base\")\nprint(\"a\")\nprint(\"b\")\nprint(\"test\")\n");

        let read = RefCell::new(Vec::new());
        let loaded = super::load("demo/main.zeno", "import a\nimport b\n".to_string(), &|path| {
            read.borrow_mut().push(path.to_string());
            Some(if path.ends_with("base.zeno") { String::new() } else { "import nested.base\n".to_string() })
        });
        assert!(loaded.program.is_ok());
        // A module is looked for from the folder of the first file, and
        // a dot in its name is a folder.
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            *read.borrow(),
            [format!("demo{sep}a.zeno"), format!("demo{sep}nested{sep}base.zeno"), format!("demo{sep}b.zeno")]
        );
        let paths: Vec<&str> = loaded.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths.len(), 4);
        assert_eq!(paths[0], "demo/main.zeno");
    }

    #[test]
    fn a_mistake_is_in_the_file_it_is_in() {
        // The second file's lines are numbered from 1,000,001 inside
        // the compiler. What is shown is the file, and its own line 2.
        let (message, file, line) = mistake("import shapes\n==> shapes.zeno\nlet a = 1\nlet b = = 2\n");
        assert_eq!((message.as_str(), file, line), ("expected an expression, found `=`", 1, 2));

        // The same for a mistake the type checker finds.
        let joined = program("import shapes\nprint(shapes.half(3))\n==> shapes.zeno\n\nfunction half(n: int) -> int {\n    return n / 2\n}\n");
        let errors = crate::checker::check(&joined, false);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "the returned value should be int, but this is float");
        assert_eq!((errors[0].span.file(), errors[0].span.line_in_file(), errors[0].span.col), (1, 3, 12));
        assert_eq!(super::testing::place(&joined, errors[0].span), "shapes.zeno:3:12");
    }

    #[test]
    fn a_line_in_a_message_is_the_file_s_own() {
        let joined = program(
            "import lists\nlists.twice()\n==> lists.zeno\n\
             function lose(ys: take list[int]) {\n}\n\
             function twice() {\n    let mine = [1]\n    lose(mine)\n    print(mine)\n    print(later)\n}\n\
             let later = 4\n",
        );
        let rules = crate::checker::Rules { strict: true, one_owner: true, firmware: true };
        let messages: Vec<String> = crate::checker::check_by(&joined, rules).0.into_iter().map(|error| error.message).collect();
        assert_eq!(
            messages,
            [
                "`mine` was given away on line 5 and holds nothing now. Use it before it goes, or give it only \
                 to what looks: a parameter without `take`",
                "`lists.later` is declared on line 9, which is further down. A line can use only what is declared \
                 above it, also inside a function. Move the declaration of `lists.later` up",
            ]
        );
    }

    #[test]
    fn strict_is_asked_of_one_file() {
        let check = |source: &str, strict: bool| -> Vec<String> {
            crate::checker::check(&program(source), strict).into_iter().map(|error| error.message).collect()
        };
        let loose = "==> loose.zeno\nfunction anything(n) { return n }\n";
        let needs_a_type = |function: &str| {
            format!("strict mode: the parameter `n` of `{function}` needs a type. It could not be worked out from how it is used")
        };
        // The first file is strict, the file it imports is not.
        assert_eq!(check(&format!("strict\nimport loose\nfunction mine(n) {{ return n }}\n{loose}"), false), [needs_a_type("mine")]);
        // The other way round.
        assert_eq!(
            check("import tight\nfunction mine(n) { return n }\n==> tight.zeno\nstrict\nfunction theirs(n) { return n }\n", false),
            [needs_a_type("tight.theirs")]
        );
        // `--strict` and `zeno build` ask it of every file.
        assert_eq!(
            check(&format!("import loose\nfunction mine(n) {{ return n }}\n{loose}"), true),
            [needs_a_type("mine"), needs_a_type("loose.anything")]
        );
    }

    #[test]
    fn what_cannot_be_imported_says_why() {
        assert_eq!(
            error("import shapes\nimport drivers.uart\nimport shapes\n"),
            "there is no module `shapes`. `import shapes` reads the file `shapes.zeno`, and there is no such file. \
             So far Zeno has `math`, `time`, `board`, `files` and `program`"
        );
        assert_eq!(load("import shapes\nimport shapes\nfrom shapes import x\n").program.unwrap_err().len(), 1);
        assert_eq!(
            error(&format!("print(1)\nimport shapes\n{SHAPES}")),
            "`import shapes` goes at the top of the file, above everything that is not an import. \
             The file `shapes` runs first wherever this line is, so the top is where the line belongs"
        );
        assert_eq!(
            error(&format!("function f() {{\n    import math\n    import shapes\n}}\n{SHAPES}")),
            "`import shapes` goes at the top of the file, not inside a function or a block. \
             Only a module that comes with Zeno, such as `math`, can be imported in here"
        );
        assert_eq!(error("import test\n"), "there is no module `test`. `import test` reads the file `test.zeno`, and there is no such file. So far Zeno has `math`, `time`, `board`, `files` and `program`");
        assert_eq!(error("import a\n==> a.zeno\nimport a\n"), "`a` imports itself. A file cannot import the file it is");
        assert_eq!(
            mistake("import a\n==> a.zeno\nimport b\n==> b.zeno\nimport c\n==> c.zeno\n\nimport a\n"),
            (
                "these files import each other: `a` imports `b`, `b` imports `c`, and `c` imports `a`. \
                 An imported file runs before the file that imports it, and here none of them can be the first. \
                 Move what they share into a file of its own, and import that"
                    .to_string(),
                3,
                2
            )
        );
    }

    #[test]
    fn a_name_that_is_not_there_says_what_is() {
        let has = "It has `sides`, `made`, `Shape`, `Dot`, `Square`, `Point` and `area`";
        assert_eq!(error(&format!("from shapes import aera\n{SHAPES}")), format!("module `shapes` has no `aera`. {has}"));
        assert_eq!(error(&format!("import shapes\nprint(shapes.aera(1))\n{SHAPES}")), format!("module `shapes` has no `aera`. {has}"));
        assert_eq!(error(&format!("import shapes\nlet p: shapes.Pint = 1\n{SHAPES}")), format!("module `shapes` has no `Pint`. {has}"));
        assert_eq!(
            error("import empty\nprint(empty.x)\n==> empty.zeno\nprint(1)\n"),
            "module `empty` has no `x`. Its file declares nothing at the top"
        );
        assert_eq!(
            error(&format!("import shapes\nlet s = shapes\n{SHAPES}")),
            "`shapes` is a module, not a value. Write `shapes.` and then a name from it, as in `shapes.sides`"
        );
        assert_eq!(
            error(&format!("let p: shapes.Point = 1\n{SHAPES}")),
            "this file imports no module `shapes`, so `shapes.Point` means nothing here. Write `import shapes` at the top of the file"
        );
        assert_eq!(
            error("import drivers.uart\nprint(drivers.spi.speed)\n==> drivers/uart.zeno\nlet speed = 1\n"),
            "this file imports no module `drivers.spi`. Write `import drivers.spi` at the top of the file"
        );
    }

    #[test]
    fn two_things_cannot_have_one_name() {
        assert_eq!(
            error(&format!("from shapes import area\nlet area = 4\n{SHAPES}")),
            "`area` is imported from `shapes` at the top of this file, so the file cannot declare a second `area`. \
             Give this one another name, or import the module whole with `import shapes` and write `shapes.area` for the other"
        );
        assert_eq!(
            error(&format!("import shapes\nfunction shapes() {{\n}}\n{SHAPES}")),
            "this file imports the module `shapes`, so it cannot also declare `shapes`: \
             `shapes.x` would have two readings. Give this one another name"
        );
        assert_eq!(
            error(&format!("from shapes import area\nfrom other import area\n{SHAPES}==> other.zeno\nlet area = 1\n")),
            "`area` is imported twice, from `shapes` and from `other`. Import one of the two modules whole, \
             as in `import other`, and write `other.area`"
        );
        assert_eq!(
            error("import a\nimport a.b\n==> a.zeno\nlet b = 1\n==> a/b.zeno\nlet x = 2\n"),
            "this file imports `a` and `a.b`. `a.b.x` would then have two readings: `x` of the module `a.b`, \
             and `b.x` of the module `a`. Import one of them with `from`, as in `from a.b import x`"
        );
    }

    #[test]
    fn a_file_sees_what_it_declares_and_what_it_imports() {
        // Only the file of a module gives its variables a new value.
        let belongs = "`shapes.made` is a variable of the module `shapes`, and only the file of `shapes` gives it a new value. \
                       Give `shapes` a function that changes it, and call that";
        assert_eq!(error(&format!("import shapes\nshapes.made = 3\n{SHAPES}")), belongs);
        assert_eq!(error(&format!("from shapes import made\nmade += 1\n{SHAPES}")), belongs);
        // An imported file does not see the file that imports it.
        let unseen = |name: &str| {
            format!(
                "this file declares no `{name}`. The program's first file has one, and an imported file cannot see \
                 what the file that imports it declares. Hand the value to the function that needs it, as an argument"
            )
        };
        let first = "import peek\nlet total = 5\nenum State {\n    Idle\n    Busy\n}\nclass Reading {\n    value: int\n}\n";
        let peek = |body: &str| error(&format!("{first}==> peek.zeno\n{body}"));
        assert_eq!(peek("function show() {\n    print(total)\n}\n"), unseen("total"));
        assert_eq!(peek("function show(r: Reading) {\n}\n"), unseen("Reading"));
        assert_eq!(peek("function show(n: int) {\n    match n {\n        case Idle { print(1) }\n    }\n}\n"), unseen("Idle"));
        // A name of its own with the same spelling is its own.
        assert!(load(&format!("{first}==> peek.zeno\nlet total = 1\nfunction show(State: int) -> int {{\n    return State + total\n}}\n")).program.is_ok());
    }
}
