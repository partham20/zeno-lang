//! Stage 5: the C backend. It turns a checked program into C source.
//!
//! ```text
//! tokens -> parser -> tree -> checker -> [emit_c] -> C source -> C compiler -> program
//! ```
//!
//! The interpreter walks the tree and *does* what each node says. This stage
//! walks the same tree and *writes down*, in C, what each node says. The two
//! must agree: a program has to print the same thing whether it goes through
//! `zeno run` or `zeno build`. The tests at the bottom hold them to that.
//!
//! Three things make this more than copying the program out with different
//! spelling:
//!
//! 1. **C needs every type; Zeno lets you leave them out.** The checker has
//!    already worked out what every function takes and returns, and hands
//!    that over in `Types`. From there the type of everything else follows:
//!    a literal knows its own type, a variable has the type of its first
//!    value, and `a + b` has the type its two sides give it.
//! 2. **C is looser than Zeno.** Overflow, division by zero and the like are
//!    errors in Zeno and undefined in C. So arithmetic goes through small
//!    checked functions in `runtime.c`.
//! 3. **C does not say which argument of `f(a(), b())` runs first.** Zeno
//!    does: left to right. Where the order can matter, the pieces are worked
//!    out one at a time into temporaries. See `sequence`.
//!
//! It compiles numbers, bools, strings, quantities, functions, control flow,
//! classes, lists, enums and `match`, values that may be `none`, errors
//! (`try`, `except`, `raise`), and functions used as values. The few things
//! it does not compile yet are refused with a message that says so.
//!
//! `zeno build --firmware` compiles errors a second way, as values that
//! each caller tests, with no `setjmp`. Look for "Errors as values".

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::ast::*;
use crate::checker::{FunctionKind, Kind, Types};
use crate::error::Error;
use crate::ints::{self, Width};
use crate::token::Span;
use crate::units;

/// The runtime library, pasted in front of every compiled program.
const RUNTIME: &str = include_str!("runtime.c");

/// The three ways `zeno build` can compile a program.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Build {
    /// `zeno build`: errors jump (`setjmp`), and a collector frees memory.
    Ordinary,
    /// `zeno build --firmware`: errors travel as values that each caller
    /// tests, with no `setjmp`. The collector is still there.
    Firmware,
    /// `zeno build --no-collector`: a firmware build with no collector in
    /// it. Every list and object must be owned. See docs/no-collector.md.
    NoCollector,
}

/// The runtime as one kind of build needs it. The runtime marks what only
/// some builds use with `#ifdef ZENO_FIRMWARE` and `#ifdef
/// ZENO_NO_COLLECTOR`. The choice is made here, so that the C file holds
/// what the program uses and nothing else. (That matters to a reviewer,
/// who must not find `setjmp` anywhere in a firmware build, nor a
/// collector in a build that says it has none.)
pub fn runtime(firmware: bool, collector: bool) -> String {
    let mut text = String::new();
    // One entry for each marked part the line is inside: is it being kept?
    let mut keeping: Vec<bool> = Vec::new();
    for line in RUNTIME.lines() {
        match line.trim() {
            "#ifdef ZENO_FIRMWARE" => keeping.push(firmware),
            "#ifndef ZENO_FIRMWARE" => keeping.push(!firmware),
            "#ifdef ZENO_NO_COLLECTOR" => keeping.push(!collector),
            "#ifndef ZENO_NO_COLLECTOR" => keeping.push(collector),
            "#else /* ZENO_FIRMWARE */" | "#else /* ZENO_NO_COLLECTOR */" => {
                if let Some(kept) = keeping.last_mut() {
                    *kept = !*kept;
                }
            }
            "#endif /* ZENO_FIRMWARE */" | "#endif /* ZENO_NO_COLLECTOR */" => {
                keeping.pop();
            }
            _ if keeping.iter().all(|kept| *kept) => {
                text += line;
                text.push('\n');
            }
            _ => {}
        }
    }
    text
}

/// Turn a checked program into the text of one C file. `file` is the name of
/// the Zeno source, which the compiled program uses in its error messages.
/// `build` says which of the three kinds of build this is.
pub fn emit(program: &Program, types: &Types, file: &str, build: Build) -> Result<String, Error> {
    let firmware = build != Build::Ordinary;
    let collector = build != Build::NoCollector;
    let mut emitter = Emitter {
        firmware,
        collector,
        built: false,
        marks: 0,
        holds: 0,
        raising: None,
        walked: None,
        fallible: false,
        arriving: Vec::new(),
        jumped: HashSet::new(),
        labels: 0,
        types,
        functions: HashMap::new(),
        classes: HashMap::new(),
        enums: HashMap::new(),
        cases: HashMap::new(),
        imported: HashMap::new(),
        scopes: vec![HashMap::new()],
        used: HashMap::new(),
        lines: Vec::new(),
        indent: 1,
        temps: 0,
        typedefs: Vec::new(),
        structs: Vec::new(),
        globals: Vec::new(),
        roots: Vec::new(),
        prototypes: Vec::new(),
        definitions: Vec::new(),
        adapters: HashSet::new(),
        returns: None,
        tries: Vec::new(),
        loops: Vec::new(),
        base: 0,
        loop_scopes: Vec::new(),
        try_scopes: Vec::new(),
        has_try: false,
        volatile: HashSet::new(),
        shared: HashSet::new(),
        closures: 0,
        below: HashSet::new(),
    };

    // Classes, enums and functions can be used above where they are written,
    // so learn about all of them before compiling anything. Types go first,
    // because functions mention them.
    for statement in &program.statements {
        match &statement.kind {
            // A generic class is not compiled. Its copies are, one for
            // each set of types the program uses it with: `Stack[int]`
            // is a class of its own, with its own struct. The checker
            // made the copies, and each is an ordinary class.
            StmtKind::Class(decl) if !decl.type_params.is_empty() => {
                for copy in types.class_copies(decl) {
                    emitter.register_class(copy, statement.span)?;
                }
            }
            StmtKind::Class(decl) => emitter.register_class(decl, statement.span)?,
            StmtKind::Enum(decl) => emitter.register_enum(decl, statement.span)?,
            _ => {}
        }
    }
    for statement in &program.statements {
        match &statement.kind {
            // The same for a generic function: `first[int]`, `first[str]`.
            StmtKind::Function(decl) if !decl.type_params.is_empty() => {
                for copy in types.function_copies(decl) {
                    emitter.register(copy, statement.span)?;
                }
            }
            StmtKind::Function(decl) => emitter.register(decl, statement.span)?,
            _ => {}
        }
    }

    // The statements at the top of the file become the body of C's `main`.
    let top: Vec<&Stmt> = program
        .statements
        .iter()
        .filter(|statement| !matches!(statement.kind, StmtKind::Function(_) | StmtKind::Class(_) | StmtKind::Enum(_)))
        .collect();
    for statement in &top {
        if !firmware {
            changed_in_try(statement, false, &mut emitter.volatile);
        }
        names_in_functions(statement, false, &mut emitter.shared);
    }
    emitter.has_try = has_try(&program.statements);
    for statement in &program.statements {
        emitter.stmt(statement)?;
    }
    // The end of the file is the end of the outermost block.
    emitter.free_owned(0);
    let main = std::mem::take(&mut emitter.lines);

    // Then each function and method. They are compiled after `main` so that
    // every top-level variable is known by the time a body mentions one.
    for (index, statement) in program.statements.iter().enumerate() {
        // A function sees the top-level variables declared above it, and
        // not the ones below.
        emitter.below.clear();
        for later in &program.statements[index + 1..] {
            match &later.kind {
                StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
                    emitter.below.insert(name.clone());
                }
                StmtKind::Import { names, .. } => emitter.below.extend(names.iter().cloned()),
                _ => {}
            }
        }
        // What there is to compile here: the function or class itself,
        // or, if it is generic, each of its copies.
        let (functions, classes): (Vec<&Arc<Function>>, Vec<&Class>) = match &statement.kind {
            StmtKind::Function(decl) if !decl.type_params.is_empty() => {
                (types.function_copies(decl).iter().collect(), Vec::new())
            }
            StmtKind::Function(decl) => (vec![decl], Vec::new()),
            StmtKind::Class(class) if !class.type_params.is_empty() => {
                (Vec::new(), types.class_copies(class).iter().map(|copy| &**copy).collect())
            }
            StmtKind::Class(class) => (Vec::new(), vec![class]),
            _ => continue,
        };
        for decl in functions {
            let entry = &emitter.functions[&decl.name];
            let (c_name, kind, owned) = (entry.c_name.clone(), entry.kind.clone(), entry.owned.clone());
            emitter.function(decl, &c_name, &kind, &owned, statement.span)?;
        }
        for class in classes {
            for method in &class.methods {
                let entry = &emitter.classes[&class.name].methods[&method.name];
                let (c_name, kind, owned) = (entry.c_name.clone(), entry.kind.clone(), entry.owned.clone());
                emitter.function(method, &c_name, &kind, &owned, statement.span)?;
            }
        }
    }

    // One C file: the runtime, then the program's types, its top-level
    // variables, a first line for every function (so that they can call each
    // other in any order), the functions themselves, and `main`.
    let mut c = runtime(firmware, collector);
    for section in [&emitter.typedefs, &emitter.structs, &emitter.globals, &emitter.prototypes, &emitter.definitions] {
        if !section.is_empty() {
            c.push('\n');
        }
        for item in section {
            c += item;
            c.push('\n');
        }
    }
    // The top-level code. It is kept out of `main` (and `noinline` makes
    // sure the C compiler leaves it out) so that every variable of the
    // program lies below `main`'s on the stack, where the collector looks.
    c += "\n__attribute__((noinline)) static void zeno_program(void) {\n";
    c += &render(&main);
    c += "}\n";

    c += "\nint main(void) {\n";
    if collector {
        // The collector starts from the variables. Those on the stack it
        // finds by itself; the top-level ones, and an error on its way to
        // an `except`, are listed here.
        let mut roots = "\nstatic void zeno_roots(void) {\n".to_string();
        roots += "    zeno_mark_words(&zeno_raised, sizeof zeno_raised);\n";
        for root in &emitter.roots {
            roots += &format!("    zeno_mark_words(&{root}, sizeof {root});\n");
        }
        roots += "}\n";
        c.insert_str(c.len() - "\nint main(void) {\n".len(), &roots);
        c += "    void *base = NULL;\n";
        c += "    zeno_stack_base = &base; /* the stack starts here */\n";
    }
    c += "#ifdef ZENO_MEMORY_REPORT\n    atexit(zeno_memory_report);\n#endif\n";
    c += &format!("    zeno_file = {};\n", c_string(file));
    c += "    zeno_program();\n";
    c += "    return 0;\n}\n";
    Ok(c)
}

/// Lines of C, each with how deeply it is indented.
type Lines = Vec<(usize, String)>;

fn render(lines: &Lines) -> String {
    let mut text = String::new();
    for (indent, line) in lines {
        text += &"    ".repeat(*indent);
        text += line;
        text.push('\n');
    }
    text
}

/// A Zeno expression, compiled.
///
/// `code` is a C expression that gives its value. Some expressions cannot be
/// said in one C expression; the statements they needed first have already
/// been written out by the time a `Val` is returned.
///
/// The flags say what running `code` could do, which is what `sequence`
/// needs to know to keep things in order.
#[derive(Clone)]
struct Val {
    code: String,
    kind: Kind,
    /// It looks at a variable, so it must run before anything changes one.
    reads: bool,
    /// It can stop the program with an error.
    fails: bool,
    /// It calls a function, which may print or change variables.
    calls: bool,
    /// The same two questions, about the statements already written out.
    pre_fails: bool,
    pre_calls: bool,
    /// For text: it was there before this statement began (a literal, or
    /// what an unchanging variable holds), so it will still be there
    /// afterwards. Text that is not `fixed` is in the text room: this
    /// statement built it, or a function gave it back, or it was lent
    /// (see `lend`). Only a build with no collector cares: there, such
    /// text is gone when the statement is over, and is copied to be kept.
    fixed: bool,
}

impl Val {
    /// A value that does nothing and depends on nothing: a literal, or a
    /// temporary that already holds its answer.
    fn plain(code: impl Into<String>, kind: Kind) -> Val {
        Val {
            code: code.into(),
            kind,
            reads: false,
            fails: false,
            calls: false,
            pre_fails: false,
            pre_calls: false,
            fixed: false,
        }
    }

    /// A value built out of other values, inheriting what they can do.
    fn from(code: String, kind: Kind, parts: &[&Val]) -> Val {
        let mut val = Val::plain(code, kind);
        for part in parts {
            val.reads |= part.reads;
            val.fails |= part.fails;
            val.calls |= part.calls;
            val.pre_fails |= part.pre_fails;
            val.pre_calls |= part.pre_calls;
        }
        val
    }

    fn failing(mut self) -> Val {
        self.fails = true;
        self
    }

    fn fixed(mut self) -> Val {
        self.fixed = true;
        self
    }

    /// The value as a C double, for arithmetic that mixes ints and floats.
    fn double(&self) -> String {
        match self.kind {
            Kind::Int if self.code.bytes().all(|b| b.is_ascii_digit()) => format!("{}.0", self.code),
            Kind::Int => format!("(double){}", self.code),
            _ => self.code.clone(),
        }
    }

    fn is_number(&self) -> bool {
        matches!(self.kind, Kind::Int | Kind::Float)
    }
}

struct Variable {
    /// The C that reads or writes it. Usually a name, such as `z_count`.
    c_name: String,
    kind: Kind,
    /// Declared with `var`, so it can be given a new value.
    mutable: bool,
    /// A `var` that a function written inside this one also uses does not
    /// live in a C variable: it lives in a box on the heap that both of
    /// them point to, so that a change made by one is seen by the other.
    /// This is the C for the pointer to that box.
    boxed: Option<String>,
    /// Whether anything has looked the variable up. That is how a function
    /// written inside another finds out which outside variables it needs.
    used: Cell<bool>,
    /// It owns the list or object it holds, and frees it when its block
    /// ends. In C it is NULL once the value has been given away.
    owner: bool,
}

struct FunctionEntry {
    c_name: String,
    decl: Arc<Function>,
    kind: FunctionKind,
    /// For each parameter: does the function take over an owned value?
    owned: Vec<bool>,
}

/// `kind` without the checker's note that the value is owned, and whether
/// the note was there. C holds an owned value exactly as it holds a shared
/// one, so everything past this point works with the plain kind. What
/// differs is who frees it, and that is kept beside the kind: see
/// `Variable::owner`.
fn disown(kind: Kind) -> (Kind, bool) {
    match kind {
        Kind::Owned(inner) => (*inner, true),
        kind => (kind, false),
    }
}

/// The same for everything a function takes and gives back.
/// `kind` with every "owned" mark taken off, however deep. Who owns a
/// value does not change how C holds it.
pub fn disowned(kind: Kind) -> Kind {
    match kind {
        Kind::Owned(inner) => disowned(*inner),
        Kind::List(item) => Kind::List(Box::new(disowned(*item))),
        Kind::Optional(inner) => Kind::Optional(Box::new(disowned(*inner))),
        Kind::Function(function) => Kind::Function(Box::new(FunctionKind {
            params: function.params.into_iter().map(disowned).collect(),
            returns: disowned(function.returns),
        })),
        other => other,
    }
}

fn disown_all(kind: FunctionKind) -> (FunctionKind, Vec<bool>) {
    let (params, owned) = kind.params.into_iter().map(disown).unzip();
    (FunctionKind { params, returns: disown(kind.returns).0 }, owned)
}

/// The C statement that frees the owned value in `place`, if `place`
/// still holds one. `texts` says that text is kept by copy (a build with
/// no collector, docs/text.md): then a str has something to free, and a
/// list of them frees its items as well as itself.
fn free_code(kind: &Kind, place: &str, texts: bool) -> String {
    match kind {
        Kind::Class(name) => format!("zeno_drop_{}({place});", ident(name)),
        // An error that an `except` caught, in a build with no collector.
        Kind::Error => format!("zeno_error_drop({place});"),
        kind if is_text(kind) => format!("zeno_text_drop({place});"),
        Kind::List(item) if texts && is_text(item) => format!("zeno_list_drop_text({place});"),
        _ => format!("zeno_list_drop({place});"),
    }
}

/// If `code` is exactly `zeno_text_lend(x)`, the `x`.
fn lent(code: &str) -> Option<&str> {
    let inside = code.strip_prefix("zeno_text_lend(")?.strip_suffix(')')?;
    // The bracket that was taken off the end must be the one that closes
    // the call, and not the end of something written after it.
    let mut depth = 0;
    for byte in inside.bytes() {
        match byte {
            b'(' => depth += 1,
            b')' if depth == 0 => return None,
            b')' => depth -= 1,
            _ => {}
        }
    }
    Some(inside)
}

/// Text, or text that may be `none`.
pub fn is_text(kind: &Kind) -> bool {
    match kind {
        Kind::Str => true,
        Kind::Optional(inner) => **inner == Kind::Str,
        _ => false,
    }
}

/// Is this text written in the program, or `none`? Either can be kept as
/// it is: there is nothing to copy and nothing to free.
pub fn is_written(expr: &Expr) -> bool {
    matches!(expr.kind, ExprKind::Str(_) | ExprKind::None)
}

/// Is there a `try` anywhere in these statements?
pub fn has_try(statements: &[Stmt]) -> bool {
    statements.iter().any(|statement| match &statement.kind {
        StmtKind::Try { .. } => true,
        StmtKind::If { then_block, else_block, .. } => {
            has_try(then_block) || else_block.as_deref().is_some_and(has_try)
        }
        StmtKind::While { body, .. } | StmtKind::For { body, .. } => has_try(body),
        StmtKind::Match { arms, .. } => arms.iter().any(|arm| has_try(&arm.body)),
        _ => false,
    })
}

/// Does this statement always leave the block it is in?
fn leaves(statement: Option<&Stmt>) -> bool {
    matches!(
        statement.map(|statement| &statement.kind),
        Some(StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue | StmtKind::Raise(_))
    )
}

/// A class: its fields in order, each with its type and default value, and
/// its methods.
struct ClassEntry {
    fields: Vec<(String, Kind, Option<Expr>)>,
    /// The fields that are sized whole numbers, such as `level: u8`. The
    /// struct stores each in that many bits. Everywhere else it is an
    /// `int64_t` like any int, so it is made small on the way in and
    /// wide again on the way out.
    sized: HashMap<String, Width>,
    methods: HashMap<String, FunctionEntry>,
    /// The fields that own what they hold: an object of an `owned class`
    /// frees those along with itself.
    owned_fields: Vec<String>,
    /// It is an `owned class`: each object has one owner.
    owned: bool,
}

/// An enum: each case with the names and types of the values it carries.
struct EnumEntry {
    cases: Vec<(String, Vec<(String, Kind)>)>,
}

struct Emitter<'a> {
    types: &'a Types,
    /// The functions, classes and enums of the program, by their Zeno names.
    functions: HashMap<String, FunctionEntry>,
    classes: HashMap<String, ClassEntry>,
    enums: HashMap<String, EnumEntry>,
    /// Each enum case, by its own name: which enum it belongs to, and its
    /// number within it.
    cases: HashMap<String, (String, usize)>,
    /// Names brought in by `from math import sqrt`: the name, and its module.
    imported: HashMap<String, String>,
    /// The variables in sight, innermost block last. The first entry holds
    /// the top-level variables, which become C globals.
    scopes: Vec<HashMap<String, Variable>>,
    /// How many times each Zeno name has been declared, to give each
    /// declaration its own C name.
    used: HashMap<String, usize>,
    /// The C statements written so far for the body being compiled.
    lines: Lines,
    indent: usize,
    temps: usize,
    /// The pieces of the C file, in the order they are written out.
    typedefs: Vec<String>,
    structs: Vec<String>,
    globals: Vec<String>,
    /// The C names of the top-level variables that can hold a pointer: the
    /// places, besides the stack, where the collector starts looking.
    roots: Vec<String>,
    prototypes: Vec<String>,
    definitions: Vec<String>,
    /// The small show-one-item and compare-two-items functions written so
    /// far for lists, so that each is written once.
    adapters: HashSet<String>,
    /// Inside a function: what it returns.
    returns: Option<Kind>,
    /// The `try` blocks being compiled around this point, outermost first,
    /// by the name of the C variable each one keeps its place in.
    tries: Vec<String>,
    /// For each loop around this point, how many `try` blocks were open
    /// when it began. `break` has to close the ones opened since.
    loops: Vec<usize>,
    /// Which of `scopes` is the first that belongs to the function being
    /// compiled. A `return` frees what the owners from there inward hold.
    base: usize,
    /// For each loop around this point, how many scopes there were when it
    /// began. `break` frees what the owners in the newer ones hold.
    loop_scopes: Vec<usize>,
    /// The same for each `try`: an error raised inside one leaves the
    /// scopes opened since it began.
    try_scopes: Vec<usize>,
    /// The function being compiled has a `try` in it. See `stmt`, on `let`.
    has_try: bool,
    /// In the function being compiled: the variables given a new value
    /// inside a `try`. See `changed_in_try`.
    volatile: HashSet<String>,
    /// In the function being compiled: the names that functions written
    /// inside it mention. A `var` among them is kept in a box.
    shared: HashSet<String>,
    /// How many functions with no name have been compiled, to number them.
    closures: usize,
    /// While a function is being compiled: the top-level variables that
    /// are declared below it in the file, which it must not see.
    below: HashSet<String>,

    // For a build with no collector (`zeno build --no-collector`).
    /// False in a build with no collector.
    collector: bool,
    /// The statement being compiled has built text, which lasts until the
    /// statement is over. See `text_begin`.
    built: bool,
    /// How many places in the text room have been noted, to number them.
    marks: usize,
    /// How many values are being held for a moment, to number them. See
    /// `hold`.
    holds: usize,
    /// Where the error that a `raise` is about to send is written. An
    /// error may only be made there.
    raising: Option<Span>,
    /// The list a `for` loop is about to walk, when it is written out
    /// right there: `for pin in [2, 3, 5]`. The loop frees it.
    walked: Option<usize>,

    // The rest is for a firmware build, where errors travel as values.
    /// `zeno build --firmware`, and `--no-collector`, which is one too.
    firmware: bool,
    /// The function being compiled can raise: it returns `true` or `false`
    /// and gives its answer through `zeno_result`.
    fallible: bool,
    /// For each `try` around this point, the kinds of error that can
    /// arrive at its `except` blocks as values.
    arriving: Vec<BTreeSet<String>>,
    /// The `except` labels that some `goto` leads to.
    jumped: HashSet<String>,
    /// How many `except` labels there have been, to number them.
    labels: usize,
}

/// A firmware build calls a function that can raise in a different way
/// from one that cannot, and the type of a function value does not say
/// which it is. So for now only functions that cannot raise may be values.
fn raises_as_value(span: Span) -> Error {
    firmware_cannot(
        "`zeno build --firmware` cannot compile a function that can raise, used as a value, yet. Call it by its name",
        span,
    )
}

/// The error for something a firmware build has to refuse.
fn firmware_cannot(what: impl AsRef<str>, span: Span) -> Error {
    Error::new(format!("{}, or build without `--firmware`", what.as_ref()), span)
}

/// The error for something a build with no collector has to refuse:
/// whatever would make memory that no line of the program gives back.
pub fn without_collector(why: impl AsRef<str>, span: Span) -> Error {
    Error::new(
        format!(
            "{}. `zeno build --no-collector` leaves the collector out, so nothing would free it. \
             docs/no-collector.md lists what such a build accepts",
            why.as_ref()
        ),
        span,
    )
}

/// The error for a part of Zeno this stage does not handle yet.
/// A Zeno name as it can be written inside a C name. Only the copy of a
/// generic needs anything done: `Stack[int]` becomes `Stack_int`, and
/// `Pair[int, list[str]]` becomes `Pair_int_list_str`. Each run of
/// characters that C does not allow in a name turns into one `_`.
pub fn ident(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

/// The name a class is shown by. A copy of a generic class is shown as
/// the class it is a copy of: a `Stack[int]` prints as `Stack(items=[1])`,
/// which is what `zeno run` prints.
pub fn shown_name(name: &str) -> &str {
    name.split('[').next().unwrap_or(name)
}

fn cannot(what: impl AsRef<str>, span: Span) -> Error {
    Error::new(
        format!("`zeno build` cannot compile {} yet. `zeno run` can still run this program", what.as_ref()),
        span,
    )
}

/// The C type that holds a Zeno value of this kind.
///
/// Objects, enum values and lists live on the heap and are handled through
/// pointers, because in Zeno they are shared: `let b = a` makes a second
/// name for the same object, not a copy.
///
/// The text ends ready for a name to follow: `int64_t ` has a space, and
/// `zeno_list *` does not need one.
fn c_type(kind: &Kind, span: Span) -> Result<String, Error> {
    Ok(match kind {
        // A pin is its number.
        Kind::Int | Kind::Pin => "int64_t ".to_string(),
        Kind::Float | Kind::Quantity(_) => "double ".to_string(),
        Kind::Bool => "bool ".to_string(),
        Kind::Str => "const char *".to_string(),
        Kind::List(_) => "zeno_list *".to_string(),
        Kind::Class(name) | Kind::Enum(name) => format!("z_{} *", ident(name)),
        Kind::Error => "zeno_error *".to_string(),
        Kind::Function(_) => "zeno_closure *".to_string(),
        // A pointer can already say "nothing": it is NULL. A number or a
        // bool cannot, so it is paired with a flag.
        Kind::Optional(inner) => match &**inner {
            inner if is_pointer(inner) => c_type(inner, span)?,
            Kind::Int | Kind::Pin => "zeno_opt_int ".to_string(),
            Kind::Float | Kind::Quantity(_) => "zeno_opt_float ".to_string(),
            Kind::Bool => "zeno_opt_bool ".to_string(),
            _ => return Err(cannot("this kind of value that may be `none`", span)),
        },
        Kind::None => {
            return Err(cannot("a variable that only ever holds `none`. Give it a type, as in `x: int or none`", span))
        }
        Kind::Other(name) => return Err(cannot(format!("a value of type {name}"), span)),
        // In C an owned value is the same pointer a shared one would be.
        Kind::Owned(inner) => c_type(inner, span)?,
    })
}

/// Is this a kind that C holds through a pointer?
fn is_pointer(kind: &Kind) -> bool {
    matches!(kind, Kind::Str | Kind::List(_) | Kind::Class(_) | Kind::Enum(_) | Kind::Error | Kind::Function(_))
}

/// The C for "no value" where a `T or none` is wanted, `inner` being the T.
fn nothing(inner: &Kind, span: Span) -> Result<String, Error> {
    Ok(match inner {
        inner if is_pointer(inner) => "NULL".to_string(),
        Kind::Int | Kind::Pin => "(zeno_opt_int){false, 0}".to_string(),
        Kind::Float | Kind::Quantity(_) => "(zeno_opt_float){false, 0.0}".to_string(),
        Kind::Bool => "(zeno_opt_bool){false, false}".to_string(),
        _ => return Err(cannot("this kind of value that may be `none`", span)),
    })
}

/// The one kind that values of kinds `a` and `b` can both be stored as, if
/// there is one: `int` and `none` meet in `int or none`.
fn joined(a: &Kind, b: &Kind) -> Option<Kind> {
    match (a, b) {
        _ if a == b => Some(a.clone()),
        // `[[], [1]]`: the empty list goes along with the other one.
        (empty, Kind::List(_)) if is_empty_list(empty) => Some(b.clone()),
        (Kind::List(_), empty) if is_empty_list(empty) => Some(a.clone()),
        (Kind::None, Kind::Optional(_)) => Some(b.clone()),
        (Kind::Optional(_), Kind::None) => Some(a.clone()),
        (Kind::None, other) | (other, Kind::None) => Some(Kind::Optional(Box::new(other.clone()))),
        (Kind::Optional(inner), other) | (other, Kind::Optional(inner)) if **inner == *other => {
            Some(Kind::Optional(inner.clone()))
        }
        // Two functions that take and give back the same things in C.
        (Kind::Function(x), Kind::Function(y)) => {
            let nowhere = Span { line: 0, col: 0 };
            let (x, y) = (code_type(x, nowhere).ok()?, code_type(y, nowhere).ok()?);
            (x == y).then(|| a.clone())
        }
        _ => None,
    }
}

/// Can a value of this kind be, or contain, a pointer to memory that the
/// collector looks after? Numbers and bools cannot.
fn holds_pointer(kind: &Kind) -> bool {
    match kind {
        Kind::Int | Kind::Float | Kind::Bool | Kind::Pin | Kind::Quantity(_) | Kind::None => false,
        Kind::Optional(inner) => holds_pointer(inner),
        _ => true,
    }
}

/// The kind of `[]` before the place it arrives at has said what it holds.
fn is_empty_list(kind: &Kind) -> bool {
    matches!(kind, Kind::List(item) if matches!(**item, Kind::Other(_)))
}

/// The C return type of a function that gives back a value of this kind.
fn return_type(kind: &Kind, span: Span) -> Result<String, Error> {
    match kind {
        Kind::None => Ok("void ".to_string()),
        other => c_type(other, span),
    }
}

/// The C type of a pointer to the code of a function held as a value. Its
/// first argument is always the function value itself, which is where a
/// function written inside another finds the variables it uses.
fn code_type(kind: &FunctionKind, span: Span) -> Result<String, Error> {
    let mut params = vec!["zeno_closure *".to_string()];
    for param in &kind.params {
        params.push(c_type(param, span)?.trim_end().to_string());
    }
    Ok(format!("{}(*)({})", return_type(&kind.returns, span)?, params.join(", ")))
}

/// C that calls a function held as a value. `function` is mentioned twice,
/// so it must be a plain name.
fn invoke(function: &str, kind: &FunctionKind, args: &[String], span: Span) -> Result<String, Error> {
    let mut all = vec![function.to_string()];
    all.extend(args.iter().cloned());
    Ok(format!("(({}){function}->code)({})", code_type(kind, span)?, all.join(", ")))
}

/// A short name for a kind, to build C function names from.
fn key(kind: &Kind) -> String {
    match kind {
        Kind::Int => "int".to_string(),
        Kind::Float => "float".to_string(),
        Kind::Bool => "bool".to_string(),
        Kind::Pin => "pin".to_string(),
        Kind::Str => "str".to_string(),
        Kind::None => "none".to_string(),
        Kind::Quantity(unit) => format!("in_{unit}"),
        Kind::List(item) => format!("list_{}", key(item)),
        Kind::Class(name) | Kind::Enum(name) => ident(name),
        Kind::Optional(inner) => format!("opt_{}", key(inner)),
        Kind::Error => "error".to_string(),
        Kind::Function(_) => "function".to_string(),
        Kind::Other(_) => "other".to_string(),
        Kind::Owned(inner) => key(inner),
    }
}

/// C that puts a value into a list slot.
fn to_slot(kind: &Kind, code: &str, span: Span) -> Result<String, Error> {
    let member = match kind {
        Kind::Int | Kind::Pin => "i",
        Kind::Float | Kind::Quantity(_) => "f",
        Kind::Bool => "b",
        Kind::Str => "s",
        Kind::List(_) | Kind::Class(_) | Kind::Enum(_) | Kind::Error | Kind::Function(_) => "p",
        Kind::Optional(inner) if is_pointer(inner) => return to_slot(inner, code, span),
        // A number and its flag do not fit in a slot; see `zeno_box_int`.
        Kind::Optional(inner) => return Ok(format!("(zeno_slot){{.p = zeno_box_{}({code})}}", scalar(inner, span)?)),
        Kind::None | Kind::Other(_) | Kind::Owned(_) => return Err(cannot("a list of these", span)),
    };
    Ok(format!("(zeno_slot){{.{member} = {code}}}"))
}

/// C that takes a value back out of a list slot.
fn from_slot(kind: &Kind, slot: &str, span: Span) -> Result<String, Error> {
    Ok(match kind {
        Kind::Int | Kind::Pin => format!("{slot}.i"),
        Kind::Float | Kind::Quantity(_) => format!("{slot}.f"),
        Kind::Bool => format!("{slot}.b"),
        Kind::Str => format!("{slot}.s"),
        Kind::List(_) | Kind::Class(_) | Kind::Enum(_) | Kind::Error | Kind::Function(_) => {
            format!("(({}){slot}.p)", c_type(kind, span)?.trim_end())
        }
        Kind::Optional(inner) if is_pointer(inner) => return from_slot(inner, slot, span),
        Kind::Optional(inner) => format!("zeno_unbox_{}({slot}.p)", scalar(inner, span)?),
        Kind::None | Kind::Other(_) | Kind::Owned(_) => return Err(cannot("a list of these", span)),
    })
}

/// Which of the runtime's three flagged kinds holds an optional `inner`.
fn scalar(inner: &Kind, span: Span) -> Result<&'static str, Error> {
    match inner {
        Kind::Int | Kind::Pin => Ok("int"),
        Kind::Float | Kind::Quantity(_) => Ok("float"),
        Kind::Bool => Ok("bool"),
        _ => Err(cannot("this kind of value that may be `none`", span)),
    }
}

/// `line, col` as the last two arguments of a runtime call that can fail.
fn at(span: Span) -> String {
    format!("{}, {}", span.line, span.col)
}

/// A Zeno string as a C string literal.
fn c_string(text: &str) -> String {
    let mut out = String::from("\"");
    for byte in text.bytes() {
        match byte {
            b'"' => out += "\\\"",
            b'\\' => out += "\\\\",
            b'\n' => out += "\\n",
            b'\t' => out += "\\t",
            // `??` followed by some characters means something else in old C.
            b'?' => out += "\\?",
            // Printable ASCII goes in as it is. Everything else, including
            // the bytes of non-English letters, is written as its number.
            0x20..=0x7e => out.push(byte as char),
            _ => out += &format!("\\{byte:03o}"),
        }
    }
    out.push('"');
    out
}

impl Emitter<'_> {
    // ------------------------------------------------------------- Plumbing

    fn line(&mut self, text: impl Into<String>) {
        self.lines.push((self.indent, text.into()));
    }

    fn new_temp(&mut self) -> String {
        self.temps += 1;
        format!("t{}", self.temps)
    }

    /// Put `val` into a C variable of its own and return that variable.
    fn temp(&mut self, val: Val, span: Span) -> Result<Val, Error> {
        let name = self.new_temp();
        self.line(format!("{}{name} = {};", c_type(&val.kind, span)?, val.code));
        let mut temp = Val::plain(name, val.kind);
        temp.pre_fails = val.pre_fails || val.fails;
        temp.pre_calls = val.pre_calls || val.calls;
        temp.fixed = val.fixed;
        Ok(temp)
    }

    /// A C name for a new Zeno variable or function. Every Zeno name gets
    /// `z_` in front, so it can never collide with a C keyword or a name
    /// from the C library. A second `x`, in another block, becomes `z2_x`:
    /// that keeps `let x = x + 1` in an inner block from reading itself.
    fn c_name(&mut self, name: &str) -> String {
        let name = &ident(name);
        let count = self.used.entry(name.to_string()).or_insert(0);
        *count += 1;
        match *count {
            1 => format!("z_{name}"),
            n => format!("z{n}_{name}"),
        }
    }

    fn declare(&mut self, name: &str, kind: Kind) -> String {
        let c_name = self.c_name(name);
        let variable =
            Variable { c_name: c_name.clone(), kind, mutable: false, boxed: None, used: Cell::new(false), owner: false };
        self.scopes.last_mut().expect("there is always a scope").insert(name.to_string(), variable);
        c_name
    }

    // ------------------------------------------- With no collector to help
    //
    // `zeno build --no-collector` (docs/no-collector.md). Three things
    // need care that a collector would otherwise take: a value that
    // belongs to nobody for a moment, text built while the program runs,
    // and values that must not be kept.

    /// `place` holds an owned value that has no owner just now: it is on
    /// its way into a call, or it is a list still being filled. Until
    /// `let_go`, whatever leaves the block early (a `return`, a failed
    /// call) frees it, as it frees what the block's owners hold.
    fn hold(&mut self, kind: Kind, place: &str) -> String {
        self.holds += 1;
        // No Zeno name has a space in it, so this cannot hide a variable.
        let key = format!("held {}", self.holds);
        let variable =
            Variable { c_name: place.to_string(), kind, mutable: false, boxed: None, used: Cell::new(false), owner: true };
        self.scopes.last_mut().expect("there is always a scope").insert(key.clone(), variable);
        key
    }

    /// The value has arrived where it was going, and has an owner again.
    fn let_go(&mut self, key: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.remove(key).is_some() {
                return;
            }
        }
    }

    /// A statement is about to be compiled. In a build with no collector,
    /// text that the statement builds is written into the text room (see
    /// runtime.c) and stays there until the statement is over. Call
    /// `text_end` with what this returns when it is.
    fn text_begin(&mut self) -> (usize, usize, bool) {
        (self.lines.len(), self.indent, std::mem::replace(&mut self.built, false))
    }

    /// The statement begun at `text_begin` has been compiled. If it built
    /// text, note where the text room's top was before it:
    ///
    /// ```c
    /// size_t m1 = zeno_text_top;
    /// ```
    ///
    /// and return the C statement that puts the top back there, for the
    /// caller to write once nothing needs the text any more.
    fn text_end(&mut self, begin: (usize, usize, bool)) -> Option<String> {
        self.text_mark(begin).map(|mark| format!("zeno_text_top = {mark};"))
    }

    /// `text_end`, giving the name of the C variable that holds the mark
    /// in place of the statement that goes back to it.
    fn text_mark(&mut self, (start, indent, outer): (usize, usize, bool)) -> Option<String> {
        if !std::mem::replace(&mut self.built, outer) {
            return None;
        }
        self.marks += 1;
        let mark = format!("m{}", self.marks);
        self.lines.insert(start, (indent, format!("size_t {mark} = zeno_text_top;")));
        Some(mark)
    }

    // ------------------------------------------------------------ Kept text
    //
    // In a build with no collector, text is kept by copy (docs/text.md).
    // Whatever stores text (a variable, a field, a list) has a copy of
    // its own in the pool and frees it when it is done. Text is never
    // changed once made, so a copy cannot be told from the original, and
    // the program means what it means in every other build.

    /// `val` is about to be stored. If it is text, store a copy that the
    /// place can call its own. `written` says the text is written in the
    /// program, which needs no copy.
    fn keep(&self, mut val: Val, written: bool) -> Val {
        if !self.collector && is_text(&val.kind) && !written {
            // Text that was lent for the statement is copied straight
            // from where it is kept, not from the statement's copy.
            let from = lent(&val.code).unwrap_or(&val.code);
            val.code = format!("zeno_text_keep({from})");
        }
        val
    }

    /// `val` was read from a place that a function could change while
    /// this statement is still using what it read. If it is text, the
    /// statement gets a copy of its own, in the text room.
    fn lend(&self, mut val: Val) -> Val {
        if self.collector || !is_text(&val.kind) {
            return val.fixed();
        }
        val.code = format!("zeno_text_lend({})", val.code);
        val.fixed = false;
        val
    }

    /// The condition of an `if` or a `while`. If working it out builds
    /// text, the answer is taken first and the text given back, before
    /// the body runs.
    fn condition(&mut self, expr: &Expr) -> Result<Val, Error> {
        let begin = self.text_begin();
        let mut val = self.expr(expr)?;
        if let Some(release) = self.text_end(begin) {
            val = self.temp(val, expr.span)?;
            self.line(release);
        }
        Ok(val)
    }

    /// `val` is about to be kept: put in a variable, a field or a list,
    /// given to a function, or returned. In a build with no collector an
    /// error cannot be, because it is freed when the `except` that caught
    /// it ends. (Text can: see `keep`.)
    fn stays(&self, val: &Val, span: Span) -> Result<(), Error> {
        if self.collector {
            return Ok(());
        }
        let kind = match &val.kind {
            Kind::Optional(inner) => &**inner,
            kind => kind,
        };
        match kind {
            Kind::Error => Err(without_collector(
                "an error lasts only until the end of the `except` block that caught it, so it cannot be kept. \
                 Copy out what is needed from it",
                span,
            )),
            Kind::Class(name) if !self.classes[name].owned => Err(without_collector(
                format!(
                    "`{name}` is not an `owned class`, so the only object of it is an error on its way to an \
                     `except`, and that is freed when the `except` block ends. It cannot be kept"
                ),
                span,
            )),
            _ => Ok(()),
        }
    }

    /// A list is being written out, or built by a comprehension. With no
    /// collector it has to start life owned (the checker knows which do),
    /// or be the list a `for` loop walks and then frees.
    fn list_has_owner(&self, expr: &Expr) -> Result<(), Error> {
        if self.collector || self.types.fresh(expr) || self.walked == Some(expr as *const Expr as usize) {
            return Ok(());
        }
        Err(without_collector(
            "this list has no owner. Give it one, as in `let items: owned list[int] = [1, 2, 3]`",
            expr.span,
        ))
    }

    /// Each item of a list sits in one slot. A number that may be `none`
    /// does not fit, and is kept in a small block of its own, which
    /// nothing would free.
    fn fits_a_slot(&self, item: &Kind, span: Span) -> Result<(), Error> {
        match item {
            Kind::Optional(inner) if !self.collector && !is_pointer(inner) => Err(without_collector(
                "a list of numbers that may be `none` keeps each item in a small block of its own",
                span,
            )),
            _ => Ok(()),
        }
    }

    // ---------------------------------------------------------- Owned values
    //
    // An owned value is freed by its owner, at the end of the owner's block
    // (docs/ownership.md). In the C, an owner is an ordinary pointer
    // variable, with one extra rule: when its value is given away, the
    // variable is set to NULL. So "free whatever the owners still hold" is
    // one call per owner, and the call does nothing for NULL. The checker
    // has already made sure a variable is never read after it gave its
    // value away.

    /// The C statements that free what the owners in `scopes[from..]`
    /// still hold, innermost block first.
    fn frees(&self, from: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for scope in self.scopes.iter().skip(from).rev() {
            let mut owners: Vec<&Variable> = scope.values().filter(|variable| variable.owner).collect();
            owners.sort_by(|a, b| a.c_name.cmp(&b.c_name));
            lines.extend(owners.into_iter().map(|owner| free_code(&owner.kind, &owner.c_name, !self.collector)));
        }
        lines
    }

    /// Write those statements out: a block is ending, or being left early.
    fn free_owned(&mut self, from: usize) {
        for line in self.frees(from) {
            self.line(line);
        }
    }

    /// The innermost block has reached its last statement. Unless that
    /// statement left the block by itself (and freed on its way out), free
    /// what the block's owners hold.
    fn end_of_block(&mut self, statements: &[Stmt]) {
        if !leaves(statements.last()) {
            self.free_owned(self.scopes.len() - 1);
        }
    }

    /// Mark the variable just declared as an owner.
    fn owns(&mut self, name: &str) {
        if let Some(variable) = self.scopes.last_mut().and_then(|scope| scope.get_mut(name)) {
            variable.owner = true;
        }
    }

    /// Which scope `name` is found in: 0 is the top level.
    fn lookup_depth(&self, name: &str) -> Option<usize> {
        let depth = self.scopes.iter().rposition(|scope| scope.contains_key(name))?;
        // A top-level variable declared below the function being compiled
        // is not in sight from inside it (decision 3.6c).
        (depth > 0 || !self.below.contains(name)).then_some(depth)
    }

    fn lookup(&self, name: &str) -> Option<&Variable> {
        let depth = self.lookup_depth(name)?;
        let variable = self.scopes[depth].get(name)?;
        variable.used.set(true);
        Some(variable)
    }

    // ------------------------------------------------------------ Functions

    fn entry(&mut self, decl: &Arc<Function>, c_name: &str, span: Span) -> Result<FunctionEntry, Error> {
        let Some(kind) = self.types.function(decl).cloned() else {
            return Err(cannot(format!("the function `{}`", decl.name), span));
        };
        let (kind, owned) = disown_all(kind);
        let c_name = self.c_name(c_name);
        Ok(FunctionEntry { c_name, decl: decl.clone(), kind, owned })
    }

    fn register(&mut self, decl: &Arc<Function>, span: Span) -> Result<(), Error> {
        let entry = self.entry(decl, &decl.name, span)?;
        self.functions.insert(decl.name.clone(), entry);
        Ok(())
    }

    /// Add a C function to the output: its first line, and its body.
    fn helper(&mut self, first_line: String, body: String) {
        self.prototypes.push(format!("{first_line};"));
        self.definitions.push(format!("{first_line} {{\n{body}}}\n"));
    }

    /// A class becomes a C struct, plus three functions the compiler writes
    /// for it: one to make an object, one to show it as text, and one to
    /// compare two of them. Its methods become ordinary C functions that
    /// take the object as their first argument.
    fn register_class(&mut self, decl: &Class, span: Span) -> Result<(), Error> {
        let Some(kinds) = self.types.class(decl).cloned() else {
            return Err(cannot(format!("the class `{}`", decl.name), span));
        };
        let (kinds, owned): (Vec<Kind>, Vec<bool>) = kinds.into_iter().map(disown).unzip();
        let widths = self.types.widths(decl).cloned().unwrap_or_default();
        let mut sized = HashMap::new();
        // `name` goes into C names, `shown` into what the program prints.
        // They differ only for a copy of a generic class.
        let (name, shown_as) = (&ident(&decl.name), shown_name(&decl.name));
        if self.used.contains_key(name) {
            // `Stack[int]` and a class called `Stack_int`, in one program.
            return Err(Error::new(
                format!(
                    "`{}` and another class of this program would both be called `z_{name}` in C. \
                     A copy of a generic class gets such a name: `Stack[int]` becomes `z_Stack_int`. \
                     Rename one of them, or build with `--llvm`",
                    decl.name
                ),
                span,
            ));
        }
        let c_name = self.c_name(name);

        self.typedefs.push(format!("typedef struct {c_name} {c_name};"));
        let mut body = format!("struct {c_name} {{\n");
        let mut params = Vec::new();
        let mut fill = String::new();
        let mut shown = Vec::new();
        let mut tests = Vec::new();
        for (index, (field, kind)) in decl.fields.iter().zip(&kinds).enumerate() {
            let field_type = c_type(kind, span)?;
            let field_name = format!("z_{}", field.name);
            params.push(format!("{field_type}{field_name}"));
            match widths.get(index).copied().flatten() {
                // `level: u8` is one byte of the struct. The number was
                // checked when it became a u8, so the cast loses nothing.
                Some(width) => {
                    body += &format!("    {} {field_name};\n", width.c_type());
                    fill += &format!("    self->{field_name} = ({}){field_name};\n", width.c_type());
                    sized.insert(field.name.clone(), width);
                }
                None if !self.collector && is_text(kind) => {
                    // With no collector the object has a copy of its own.
                    body += &format!("    {field_type}{field_name};\n");
                    fill += &format!("    self->{field_name} = zeno_text_keep({field_name});\n");
                }
                None => {
                    body += &format!("    {field_type}{field_name};\n");
                    fill += &format!("    self->{field_name} = {field_name};\n");
                }
            }
            let label = if index == 0 { format!("{shown_as}({}=", field.name) } else { format!(", {}=", field.name) };
            shown.push(c_string(&label));
            shown.push(self.repr(&Val::plain(format!("self->{field_name}"), kind.clone()), span)?);
            tests.push(self.equal(kind, &format!("a->{field_name}"), &format!("b->{field_name}"), span)?);
        }
        if decl.fields.is_empty() {
            body += "    char nothing; /* C does not allow an empty struct */\n";
            shown.push(c_string(&format!("{shown_as}(")));
            tests.push("true".to_string());
        }
        shown.push(c_string(")"));
        body += "};";
        self.structs.push(body);

        let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };
        self.helper(
            format!("static {c_name} *zeno_new_{name}({params})"),
            format!("    {c_name} *self = ({c_name} *)zeno_alloc(sizeof({c_name}));\n{fill}    return self;\n"),
        );
        self.helper(
            format!("static const char *zeno_show_{name}({c_name} *self)"),
            format!("    return zeno_join({}, {});\n", shown.len(), shown.join(", ")),
        );
        self.helper(
            format!("static bool zeno_same_{name}({c_name} *a, {c_name} *b)"),
            format!("    return {};\n", tests.join(" && ")),
        );
        let mut owned_fields = Vec::new();
        // With no collector every object is freed by a line of the
        // program, and has its own copies of the text in its fields.
        let texts = !self.collector;
        if decl.owned || texts {
            // The end of an object of an owned class: free what its fields
            // own, then the object. NULL means it was given away.
            let mut body = String::from("    if (self == NULL) return;\n");
            for ((field, kind), owned) in decl.fields.iter().zip(&kinds).zip(&owned) {
                if *owned || (texts && is_text(kind)) {
                    body += &format!("    {}\n", free_code(kind, &format!("self->z_{}", field.name), texts));
                }
                if *owned {
                    owned_fields.push(field.name.clone());
                }
            }
            body += "    zeno_free(self);\n";
            self.helper(format!("static void zeno_drop_{name}({c_name} *self)"), body);
            if !decl.owned {
                // Such a class can only be raised (see `construct`). The
                // error that carries the object frees it through this.
                self.helper(
                    format!("static void zeno_drop_any_{name}(void *object)"),
                    format!("    zeno_drop_{name}(({c_name} *)object);\n"),
                );
            }
        }

        let mut methods = HashMap::new();
        for method in &decl.methods {
            let entry = self.entry(method, &format!("{name}_{}", method.name), span)?;
            methods.insert(method.name.clone(), entry);
        }
        let fields = decl
            .fields
            .iter()
            .zip(kinds)
            .map(|(field, kind)| (field.name.clone(), kind, field.default.clone()))
            .collect();
        self.classes.insert(decl.name.clone(), ClassEntry { fields, sized, methods, owned_fields, owned: decl.owned });
        Ok(())
    }

    /// An enum becomes a C struct holding a `tag` that says which case the
    /// value is, and a union with room for whichever case carries the most.
    fn register_enum(&mut self, decl: &Enum, span: Span) -> Result<(), Error> {
        let Some(kinds) = self.types.enumeration(decl).cloned() else {
            return Err(cannot(format!("the enum `{}`", decl.name), span));
        };
        let name = &decl.name;
        let c_name = self.c_name(name);
        self.typedefs.push(format!("typedef struct {c_name} {c_name};"));

        let mut members = String::new();
        let mut bare_cases = Vec::new();
        let mut show = String::from("    switch (self->tag) {\n");
        let mut same = String::from("    if (a->tag != b->tag) return false;\n    switch (a->tag) {\n");
        let mut cases = Vec::new();
        for (index, (case, case_kinds)) in decl.variants.iter().zip(&kinds).enumerate() {
            self.cases.insert(case.name.clone(), (name.clone(), index));
            let fields: Vec<(String, Kind)> =
                case.fields.iter().map(|field| field.name.clone()).zip(case_kinds.iter().cloned()).collect();

            if fields.is_empty() {
                // A case that carries nothing needs no memory of its own:
                // one fixed value stands for every use of it.
                bare_cases.push(format!("static {c_name} zeno_case_{name}_{} = {{.tag = {index}}};", case.name));
                show += &format!("    case {index}: return {};\n", c_string(&case.name));
            } else {
                let mut inside = String::new();
                let mut params = Vec::new();
                let mut fill = String::new();
                let mut shown = Vec::new();
                let mut tests = Vec::new();
                for (position, (field, kind)) in fields.iter().enumerate() {
                    let field_type = c_type(kind, span)?;
                    let place = format!("as.c{index}.z_{field}");
                    inside += &format!("{field_type}z_{field}; ");
                    params.push(format!("{field_type}z_{field}"));
                    fill += &format!("    self->{place} = z_{field};\n");
                    shown.push(if position == 0 { c_string(&format!("{}(", case.name)) } else { c_string(", ") });
                    shown.push(self.repr(&Val::plain(format!("self->{place}"), kind.clone()), span)?);
                    tests.push(self.equal(kind, &format!("a->{place}"), &format!("b->{place}"), span)?);
                }
                members += &format!("        struct {{ {inside}}} c{index};\n");
                self.helper(
                    format!("static {c_name} *zeno_new_{name}_{}({})", case.name, params.join(", ")),
                    format!(
                        "    {c_name} *self = ({c_name} *)zeno_alloc(sizeof({c_name}));\n    self->tag = {index};\n{fill}    return self;\n"
                    ),
                );
                show += &format!(
                    "    case {index}: return zeno_join({}, {}, \")\");\n",
                    shown.len() + 1,
                    shown.join(", ")
                );
                same += &format!("    case {index}: return {};\n", tests.join(" && "));
            }
            cases.push((case.name.clone(), fields));
        }
        let mut body = format!("struct {c_name} {{\n    int tag;\n");
        if !members.is_empty() {
            body += &format!("    union {{\n{members}    }} as;\n");
        }
        body += "};";
        self.structs.push(body);
        self.structs.extend(bare_cases);

        self.helper(
            format!("static const char *zeno_show_{name}({c_name} *self)"),
            format!("{show}    }}\n    return \"\";\n"),
        );
        self.helper(
            format!("static bool zeno_same_{name}({c_name} *a, {c_name} *b)"),
            format!("{same}    }}\n    return true;\n"),
        );
        self.enums.insert(name.clone(), EnumEntry { cases });
        Ok(())
    }

    /// Compile one function or method, and add it to the output.
    fn function(
        &mut self,
        decl: &Function,
        c_name: &str,
        kind: &FunctionKind,
        owned: &[bool],
        span: Span,
    ) -> Result<(), Error> {
        // A function sees the top-level variables and its own, nothing else.
        self.scopes.truncate(1);
        self.scopes.push(HashMap::new());
        self.base = 1;
        self.lines.clear();
        self.indent = 1;
        // Names only have to be told apart within one C function, so the
        // count of names used starts again from the top-level ones.
        let used = self.used.clone();

        // In a firmware build a function that can raise gives back `true`
        // or `false`, and its answer goes out through one more parameter.
        let fallible = self.firmware && !decl.raises.is_empty();
        let mut params = self.parameters(decl, kind, owned, span)?;
        if fallible && kind.returns != Kind::None {
            params.push(format!("{}*zeno_result", c_type(&kind.returns, span)?));
        }
        let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };
        self.body(decl, kind, fallible, span)?;
        self.scopes.pop();
        self.base = 0;
        self.used = used;

        let body = render(&std::mem::take(&mut self.lines));
        let returns = if fallible { "bool ".to_string() } else { return_type(&kind.returns, span)? };
        self.helper(format!("static {returns}{c_name}({params})"), body);
        Ok(())
    }

    /// Declare the parameters of a function in the innermost scope, and
    /// return them as they are written in C.
    ///
    /// A parameter that takes over an owned value is an owner like any
    /// other: the function frees the value when it ends, unless it has
    /// passed it on by then.
    fn parameters(
        &mut self,
        decl: &Function,
        kind: &FunctionKind,
        owned: &[bool],
        span: Span,
    ) -> Result<Vec<String>, Error> {
        // See `stmt`, on `let`, for what `volatile` is doing here.
        let careful = if !self.firmware && has_try(&decl.body) { "volatile " } else { "" };
        let mut params = Vec::new();
        for (index, (param, param_kind)) in decl.params.iter().zip(&kind.params).enumerate() {
            let param_type = c_type(param_kind, span)?;
            let param_name = self.declare(&param.name, param_kind.clone());
            if owned.get(index) == Some(&true) {
                self.owns(&param.name);
                params.push(format!("{param_type}{careful}{param_name}"));
            } else {
                params.push(format!("{param_type}{param_name}"));
            }
        }
        Ok(params)
    }

    /// Compile the statements of a function into `self.lines`.
    fn body(&mut self, decl: &Function, kind: &FunctionKind, fallible: bool, span: Span) -> Result<(), Error> {
        let mut volatile = HashSet::new();
        let mut shared = HashSet::new();
        for statement in &decl.body {
            if !self.firmware {
                changed_in_try(statement, false, &mut volatile);
            }
            names_in_functions(statement, false, &mut shared);
        }
        // What the function around this one was in the middle of is set
        // aside, and put back afterwards.
        let volatile = std::mem::replace(&mut self.volatile, volatile);
        let shared = std::mem::replace(&mut self.shared, shared);
        let tries = std::mem::take(&mut self.tries);
        let loops = std::mem::take(&mut self.loops);
        let loop_scopes = std::mem::take(&mut self.loop_scopes);
        let try_scopes = std::mem::take(&mut self.try_scopes);
        let has_try = std::mem::replace(&mut self.has_try, has_try(&decl.body));
        let returns = self.returns.replace(kind.returns.clone());
        let arriving = std::mem::take(&mut self.arriving);
        let fallible = std::mem::replace(&mut self.fallible, fallible);

        let mut result = decl.body.iter().try_for_each(|statement| self.stmt(statement));
        let ends = matches!(decl.body.last().map(|last| &last.kind), Some(StmtKind::Return(_) | StmtKind::Raise(_)));
        // Reaching the end of the body: free what its owners still hold.
        if result.is_ok() && !ends {
            self.free_owned(self.base);
        }
        // A function that may give back `none` does so if it reaches its
        // end without a `return`.
        if let (Ok(()), Kind::Optional(inner), false) = (&result, &kind.returns, ends) {
            result = nothing(inner, span).map(|none| {
                if self.fallible {
                    self.line(format!("*zeno_result = {none};"));
                    self.line("return true;");
                } else {
                    self.line(format!("return {none};"));
                }
            });
        }
        // Reaching the end of a function that can raise means it did not.
        if result.is_ok() && self.fallible && kind.returns == Kind::None && !ends {
            self.line("return true;");
        }
        self.arriving = arriving;
        self.fallible = fallible;

        self.volatile = volatile;
        self.shared = shared;
        self.tries = tries;
        self.loops = loops;
        self.loop_scopes = loop_scopes;
        self.try_scopes = try_scopes;
        self.has_try = has_try;
        self.returns = returns;
        result
    }

    // ----------------------------------------------------------- Statements

    fn block(&mut self, statements: &[Stmt]) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        self.indent += 1;
        let result = statements.iter().try_for_each(|statement| self.stmt(statement));
        if result.is_ok() {
            self.end_of_block(statements);
        }
        self.indent -= 1;
        self.scopes.pop();
        result
    }

    fn stmt(&mut self, statement: &Stmt) -> Result<(), Error> {
        // The statements that work out one value and are done. In a build
        // with no collector, text they build is given back as they end.
        // (The others give it back at a point of their own choosing.)
        let simple =
            matches!(statement.kind, StmtKind::Let { .. } | StmtKind::Const { .. } | StmtKind::Assign { .. } | StmtKind::Expr(_));
        if self.collector || !simple {
            return self.statement(statement);
        }
        let begin = self.text_begin();
        let result = self.statement(statement);
        if let Some(release) = self.text_end(begin) {
            self.line(release);
        }
        result
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), Error> {
        let span = statement.span;
        // At the top of the file, outside any function or block.
        let top_level = self.scopes.len() == 1 && self.returns.is_none();
        match &statement.kind {
            StmtKind::Let { name, ty, value, .. } | StmtKind::Const { name, ty, value } => {
                let mutable = matches!(statement.kind, StmtKind::Let { mutable: true, .. });
                // The checker knows whether this variable owns its value.
                let owner = self.types.owner(statement);
                let expected = ty.as_ref().and_then(|ty| self.kind_of_type(ty));
                let mut val = self.expr_as(value, expected.as_ref())?;
                self.stays(&val, value.span)?;
                val = self.keep(val, is_written(value));
                if let Some(expected) = &expected {
                    val = self.coerce(val, expected, value.span)?;
                }
                // With no collector a variable has its own copy of its
                // text, and frees it. Text written in the program is not a
                // copy, but a `var` may be given one later.
                let owner = owner || (!self.collector && is_text(&val.kind) && (mutable || !is_written(value)));
                if is_empty_list(&val.kind) {
                    return Err(cannot(
                        "an empty list with no type beside it. Write `let items: list[int] = []`",
                        value.span,
                    ));
                }
                let c_type = c_type(&val.kind, value.span)?;
                if top_level {
                    // A top-level variable is one that functions can see: a
                    // C global, given its value when `main` gets this far.
                    let is_root = holds_pointer(&val.kind);
                    let c_name = self.declare(name, val.kind);
                    self.globals.push(format!("static {c_type}{c_name};"));
                    if is_root {
                        self.roots.push(c_name.clone());
                    }
                    self.line(format!("{c_name} = {};", val.code));
                } else if mutable && self.shared.contains(name) {
                    // A function written inside this one uses the variable
                    // too, and either may change it. So it lives in a box on
                    // the heap, and both hold a pointer to the box.
                    let pointer = self.c_name(name);
                    let size = c_type.trim_end();
                    self.line(format!("{c_type}*{pointer} = ({c_type}*)zeno_alloc(sizeof({size}));"));
                    self.line(format!("*{pointer} = {};", val.code));
                    let variable = Variable {
                        c_name: format!("(*{pointer})"),
                        kind: val.kind,
                        mutable,
                        boxed: Some(pointer),
                        used: Cell::new(false),
                        owner: false,
                    };
                    self.scopes.last_mut().expect("there is always a scope").insert(name.clone(), variable);
                } else {
                    // See `changed_in_try` for what `volatile` is doing here.
                    //
                    // An owner needs the same care for a different reason.
                    // Giving its value away sets it to NULL. If that
                    // happens inside a `try` and an error then jumps back
                    // here, C does not promise the NULL is still there
                    // unless the variable is `volatile`, and the block's
                    // end would free a value that is no longer its own.
                    let changes = mutable && self.volatile.contains(name);
                    let gives = owner && self.has_try && !self.firmware;
                    let careful = if changes || gives { "volatile " } else { "" };
                    let c_name = self.declare(name, val.kind);
                    self.line(format!("{c_type}{careful}{c_name} = {};", val.code));
                }
                if let Some(variable) = self.scopes.last_mut().and_then(|scope| scope.get_mut(name)) {
                    variable.mutable = mutable;
                    variable.owner = owner;
                }
            }

            StmtKind::Assign { target, op, value } => self.assign(target, *op, value, span)?,

            StmtKind::Expr(expr) => {
                let val = self.expr(expr)?;
                if self.types.discarded(expr) {
                    // An owned value that nobody keeps is freed on the spot.
                    self.line(free_code(&val.kind, &val.code, !self.collector));
                } else if !val.code.is_empty() && (val.fails || val.calls) {
                    // What is left may still need to run, for what it does.
                    self.line(format!("{};", val.code));
                }
            }

            StmtKind::If { condition, then_block, else_block } => {
                let condition = self.condition(condition)?;
                self.line(format!("if ({}) {{", bare(&condition.code)));
                self.block(then_block)?;
                self.else_part(else_block)?;
            }

            StmtKind::While { condition, body } => {
                let mark = self.lines.len();
                let condition = self.condition(condition)?;
                let before = self.lines.split_off(mark);
                if before.is_empty() {
                    self.line(format!("while ({}) {{", bare(&condition.code)));
                } else {
                    // The condition needs statements of its own, and they
                    // must run again on every trip round the loop.
                    self.line("while (true) {");
                    self.lines.extend(before.into_iter().map(|(indent, line)| (indent + 1, line)));
                    self.indent += 1;
                    self.line(format!("if (!({})) break;", condition.code));
                    self.indent -= 1;
                }
                self.loop_body(body)?;
                self.line("}");
            }

            StmtKind::For { variable, iterable, body } => {
                let begin = self.text_begin();
                let walked = self.open_loop(variable, iterable)?;
                if let Some(release) = self.text_end(begin) {
                    // Text built to find what to loop over. The loop has
                    // its list or its numbers in hand by now, so the text
                    // is given back before the first trip.
                    let at = self.lines.iter().rposition(|(_, line)| line.starts_with("for (")).expect("a loop");
                    let indent = self.lines[at].0;
                    self.lines.insert(at, (indent, release));
                }
                self.loop_body(body)?;
                self.line("}");
                self.scopes.pop();
                self.close_loop(walked);
            }

            StmtKind::Match { subject, arms } => self.match_statement(subject, arms, span)?,

            StmtKind::Return(value) => {
                let Some(returns) = self.returns.clone() else {
                    return Err(cannot("`return` outside a function", span));
                };
                match value {
                    Some(value) if returns != Kind::None => {
                        let begin = self.text_begin();
                        let val = self.expr_as(value, Some(&returns))?;
                        self.stays(&val, value.span)?;
                        let mut val = self.coerce(val, &returns, span)?;
                        // With no collector, text that a function gives
                        // back travels in the text room (docs/text.md).
                        let text = !self.collector && is_text(&returns) && !is_written(value);
                        self.built |= text;
                        if let Some(mark) = self.text_mark(begin) {
                            // Take the answer before the text it was
                            // worked out from is given back.
                            if text {
                                val.code = format!("zeno_text_return({mark}, {})", val.code);
                                val = self.temp(val, span)?;
                            } else {
                                val = self.temp(val, span)?;
                                self.line(format!("zeno_text_top = {mark};"));
                            }
                        }
                        // Every block of the function ends here. Work the
                        // value out first: it may read what is freed next.
                        let frees = self.frees(self.base);
                        if !frees.is_empty() {
                            if !text {
                                val = self.temp(val, span)?;
                            }
                            frees.into_iter().for_each(|line| self.line(line));
                        }
                        if self.fallible {
                            self.line(format!("*zeno_result = {};", val.code));
                            self.line("return true;");
                        } else if self.tries.is_empty() || self.firmware {
                            self.line(format!("return {};", val.code));
                        } else {
                            // Work the value out while the `try` is still
                            // watching, then leave the `try`, then return.
                            let val = self.temp(val, span)?;
                            self.leave_tries(0);
                            self.line(format!("return {};", val.code));
                        }
                    }
                    Some(value) if !matches!(value.kind, ExprKind::None) => {
                        return Err(cannot("returning a value from a function that returns nothing", span))
                    }
                    _ => {
                        self.free_owned(self.base);
                        self.leave_tries(0);
                        self.line(if self.fallible { "return true;" } else { "return;" });
                    }
                }
            }
            StmtKind::Break | StmtKind::Continue => {
                let Some(&opened_before) = self.loops.last() else {
                    return Err(cannot("`break` or `continue` outside a loop", span));
                };
                // The blocks opened since the loop began end here.
                let outside = self.loop_scopes.last().copied().unwrap_or(self.scopes.len());
                self.free_owned(outside);
                self.leave_tries(opened_before);
                self.line(if statement.kind == StmtKind::Break { "break;" } else { "continue;" });
            }

            // Compiled separately; see `emit`.
            StmtKind::Function(_) | StmtKind::Class(_) | StmtKind::Enum(_) if top_level => {}
            StmtKind::Function(_) => return Err(cannot("a function inside a function or block", span)),
            StmtKind::Class(_) => return Err(cannot("a class inside a function or block", span)),
            StmtKind::Enum(_) => return Err(cannot("an enum inside a function or block", span)),

            StmtKind::Import { module, names } => {
                let module = module.join(".");
                if !matches!(module.as_str(), "math" | "time" | "board") {
                    return Err(cannot(format!("the module `{module}`"), span));
                }
                for name in names {
                    self.imported.insert(name.clone(), module.clone());
                }
            }

            StmtKind::Try { body, handlers } => self.try_statement(body, handlers, span)?,

            StmtKind::Raise(value) if self.firmware => {
                let begin = self.text_begin();
                if !self.collector {
                    // With no collector, an error is freed by the `except`
                    // that catches it. So each error is raised once: it
                    // has to be made right here.
                    let made_here = match &value.kind {
                        ExprKind::Call { callee, .. } => match &callee.kind {
                            ExprKind::Name(name) => {
                                self.lookup(name).is_none()
                                    && (self.classes.contains_key(name) || ERROR_KINDS.contains(&name.as_str()))
                            }
                            _ => false,
                        },
                        _ => false,
                    };
                    if !made_here {
                        return Err(without_collector(
                            "`raise` needs a new error here, such as `raise ValueError(\"too hot\")`. An error that \
                             was caught is freed when its `except` block ends, so it cannot be raised again",
                            value.span,
                        ));
                    }
                    self.raising = Some(value.span);
                }
                // Note the error, then go where a failed call would go.
                let val = self.expr(value)?;
                self.raising = None;
                let (error, kind) = match &val.kind {
                    Kind::Error => match &value.kind {
                        // `raise ValueError("...")` says its kind. An error
                        // held in a variable could be any kind.
                        ExprKind::Call { callee, .. } => match &callee.kind {
                            ExprKind::Name(name) if ERROR_KINDS.contains(&name.as_str()) => (val.code, name.clone()),
                            _ => (val.code, "Error".to_string()),
                        },
                        _ => (val.code, "Error".to_string()),
                    },
                    Kind::Class(name) => {
                        let name = name.clone();
                        let object = self.temp(val, span)?.code;
                        // Showing the object builds text.
                        self.built = !self.collector;
                        let error = match self.collector {
                            true => format!("zeno_error_new({}, zeno_show_{name}({object}), {object})", c_string(&name)),
                            // The `except` that catches it frees the
                            // object, and the text the object keeps.
                            false => format!(
                                "zeno_error_of({}, zeno_show_{name}({object}), {object}, zeno_drop_any_{name})",
                                c_string(&name)
                            ),
                        };
                        (error, name)
                    }
                    _ => return Err(cannot("`raise` with anything but an error or an object", span)),
                };
                let leave = self.failed(&[kind], span)?;
                self.line(format!("zeno_note({error}, {});", at(span)));
                // The error has its own copy of its message by now.
                if let Some(release) = self.text_end(begin) {
                    self.line(release);
                }
                self.line(leave);
            }

            StmtKind::Raise(value) => {
                let val = self.expr(value)?;
                // The jump leaves at least the blocks opened since the
                // nearest `try` in this function, or all of the function's
                // if there is none. Free what their owners hold first.
                // (The functions the jump passes through on its way are
                // not asked: what they own is left to the collector.)
                let inside = self.try_scopes.last().copied().unwrap_or(self.base);
                let frees = self.frees(inside);
                let error = match &val.kind {
                    Kind::Error if frees.is_empty() => val.code,
                    // Making the error may read what is about to be freed.
                    Kind::Error => self.temp(val, span)?.code,
                    // An object of a class can be raised too. It travels
                    // inside an error value whose kind is the class's name.
                    Kind::Class(name) => {
                        let name = name.clone();
                        let object = self.temp(val, span)?.code;
                        format!("zeno_error_new({}, zeno_show_{name}({object}), {object})", c_string(&name))
                    }
                    _ => return Err(cannot("`raise` with anything but an error or an object", span)),
                };
                frees.into_iter().for_each(|line| self.line(line));
                self.line(format!("zeno_raise({error}, {});", at(span)));
            }
        }
        Ok(())
    }

    /// The body of a loop. It notes how many `try` blocks were open when
    /// the loop began, for `break` and `continue` to compare against.
    fn loop_body(&mut self, body: &[Stmt]) -> Result<(), Error> {
        self.loops.push(self.tries.len());
        self.loop_scopes.push(self.scopes.len());
        let result = self.block(body);
        self.loop_scopes.pop();
        self.loops.pop();
        result
    }

    /// Before a `return`, `break` or `continue` jumps out of `try` blocks,
    /// tell the runtime that those blocks are over. `keep` is how many of
    /// the open ones stay open.
    fn leave_tries(&mut self, keep: usize) {
        // In a firmware build a `try` leaves nothing behind to undo.
        if self.firmware {
            return;
        }
        if let Some(outermost) = self.tries.get(keep) {
            self.line(format!("zeno_trying = {outermost}.outer;"));
        }
    }

    /// `try { ... } except Kind as name { ... }`
    ///
    /// ```c
    /// zeno_try t1;                      // a place to come back to
    /// t1.outer = zeno_trying;
    /// zeno_trying = &t1;                // from here on, errors come to t1
    /// if (setjmp(t1.place) == 0) {
    ///     ...the body...
    ///     zeno_trying = t1.outer;       // finished with no error
    /// } else if (zeno_caught("ValueError")) {
    ///     ...the except block...        // an error jumped back here
    /// } else {
    ///     zeno_raise_again();           // not ours: pass it outward
    /// }
    /// ```
    ///
    /// `setjmp` gives 0 when it is first called. If anything inside the
    /// body raises, however deep in function calls, the runtime jumps back
    /// to that same `setjmp`, which this time gives 1.
    fn try_statement(&mut self, body: &[Stmt], handlers: &[Handler], span: Span) -> Result<(), Error> {
        if self.firmware {
            return self.try_with_values(body, handlers, span);
        }
        let place = self.new_temp();
        self.line(format!("zeno_try {place};"));
        self.line(format!("{place}.outer = zeno_trying;"));
        self.line(format!("zeno_trying = &{place};"));
        self.line(format!("if (setjmp({place}.place) == 0) {{"));
        self.tries.push(place.clone());
        self.try_scopes.push(self.scopes.len());
        let result = self.block(body);
        self.try_scopes.pop();
        self.tries.pop();
        result?;
        // Reaching the end of the body means nothing was raised. (If the
        // body always leaves by its last line, that end is never reached.)
        let leaves = matches!(
            body.last().map(|last| &last.kind),
            Some(StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue | StmtKind::Raise(_))
        );
        if !leaves {
            self.indent += 1;
            self.line(format!("zeno_trying = {place}.outer;"));
            self.indent -= 1;
        }

        let mut catches_everything = false;
        for handler in handlers {
            // What the `except` asks for, and what `as name` then holds.
            let (wanted, holds) = match handler.error.as_deref() {
                None | Some("Error") => (None, Kind::Error),
                Some(class) if self.classes.contains_key(class) => (Some(class), Kind::Class(class.to_string())),
                Some(kind) if ERROR_KINDS.contains(&kind) => (Some(kind), Kind::Error),
                Some(other) => return Err(cannot(format!("`except {other}`"), span)),
            };
            self.line(match wanted {
                Some(kind) => format!("}} else if (zeno_caught({})) {{", c_string(kind)),
                None => "} else {".to_string(),
            });
            self.scopes.push(HashMap::new());
            self.indent += 1;
            if let Some(name) = &handler.binding {
                let c_type = c_type(&holds, span)?;
                let value = match holds {
                    Kind::Class(_) => format!("({})zeno_raised->object", c_type.trim_end()),
                    _ => "zeno_raised".to_string(),
                };
                let c_name = self.declare(name, holds);
                self.line(format!("{c_type}{c_name} = {value};"));
            }
            let result = handler.body.iter().try_for_each(|statement| self.stmt(statement));
            if result.is_ok() {
                self.end_of_block(&handler.body);
            }
            self.indent -= 1;
            self.scopes.pop();
            result?;
            if wanted.is_none() {
                catches_everything = true;
                break; // nothing gets past an `except` that takes every error
            }
        }
        if !catches_everything {
            self.line("} else {");
            self.indent += 1;
            self.line("zeno_raise_again();");
            self.indent -= 1;
        }
        self.line("}");
        Ok(())
    }

    // ------------------------------------------- Errors as values (firmware)

    /// Something here can fail in a firmware build: a `raise`, or a call to
    /// a function that can raise, which has just given back `false`. The
    /// error is already noted in `zeno_raised`. Returns the C statement
    /// that sends control where the error goes next:
    ///
    /// - inside a `try`, to its `except` blocks, further down this function;
    /// - in a function that can raise, back to its caller, as `false`;
    /// - at the top level, nobody is left to catch it.
    ///
    /// `kinds` is what may have been raised, so that the `try` around this
    /// point knows what can reach it.
    fn failed(&mut self, kinds: &[String], span: Span) -> Result<String, Error> {
        if let Some(arriving) = self.arriving.last_mut() {
            arriving.extend(kinds.iter().cloned());
        }
        // Leaving this way ends blocks early, like `break` and `return`
        // do: free what their owners hold on the way out.
        let leave = |frees: Vec<String>, jump: String| match frees.is_empty() {
            true => jump,
            false => format!("{{ {} {jump} }}", frees.join(" ")),
        };
        if let Some(label) = self.tries.last() {
            self.jumped.insert(label.clone());
            let inside = self.try_scopes.last().copied().unwrap_or(self.scopes.len());
            return Ok(leave(self.frees(inside), format!("goto {label};")));
        }
        if self.fallible {
            return Ok(leave(self.frees(self.base), "return false;".to_string()));
        }
        if self.returns.is_none() {
            return Ok("zeno_uncaught();".to_string());
        }
        // The checker makes a function say what it raises, so this is a
        // function value, which has no way to say it.
        Err(raises_as_value(span))
    }

    /// A call, in a firmware build, to something that gives back `true` or
    /// `false` and puts its answer where the last argument points:
    ///
    /// ```c
    /// double t3;
    /// if (!z_parse(z_text, &t3)) goto zeno_except_1;   // or: return false;
    /// ```
    ///
    /// `call` is the C call without the answer's pointer.
    fn checked_call(&mut self, call: String, returns: Kind, kinds: &[String], span: Span) -> Result<Val, Error> {
        let leave = self.failed(kinds, span)?;
        let open = call.strip_suffix(')').expect("a call ends with `)`");
        let mut val = if returns == Kind::None {
            self.line(format!("if (!{call}) {leave}"));
            Val::plain(String::new(), Kind::None)
        } else {
            let answer = self.new_temp();
            self.line(format!("{}{answer};", c_type(&returns, span)?));
            let comma = if open.ends_with('(') { "" } else { ", " };
            self.line(format!("if (!{open}{comma}&{answer})) {leave}"));
            // With a collector, text that a function gives back will
            // last. With none it is in the text room, like text this
            // statement built itself.
            let val = Val::plain(answer, returns);
            if !self.collector && is_text(&val.kind) { val } else { val.fixed() }
        };
        val.pre_fails = true;
        val.pre_calls = true;
        Ok(val)
    }

    /// `try` / `except` in a firmware build. Nothing jumps between
    /// functions. Each thing in the body that can fail is followed by a
    /// test, and a failure goes forward to the `except` blocks:
    ///
    /// ```c
    /// {
    ///     double t1;
    ///     if (!z_parse(z_line, &t1)) goto zeno_except_1;
    ///     ...the rest of the body...
    /// }
    /// zeno_except_1: ;
    /// if (zeno_raised != NULL) {                 // did the body fail?
    ///     zeno_error *zeno_caught_1 = zeno_raised;
    ///     zeno_raised = NULL;                    // it is being dealt with
    ///     if (zeno_is(zeno_caught_1, "ValueError")) {
    ///         ...the except block...
    ///     } else {
    ///         zeno_raised = zeno_caught_1;       // not ours: pass it on
    ///         return false;
    ///     }
    /// }
    /// ```
    ///
    /// The `goto` only ever goes forward, to a label in a block around it.
    fn try_with_values(&mut self, body: &[Stmt], handlers: &[Handler], span: Span) -> Result<(), Error> {
        self.labels += 1;
        let (label, caught) = (format!("zeno_except_{}", self.labels), format!("zeno_caught_{}", self.labels));
        self.tries.push(label.clone());
        self.try_scopes.push(self.scopes.len());
        self.arriving.push(BTreeSet::new());
        // With no collector: an error can leave a statement half way
        // through, with its text still in the text room. Whatever the
        // body leaves there is given back where the error is caught.
        let text_top = (!self.collector).then(|| {
            self.marks += 1;
            format!("m{}", self.marks)
        });
        if let Some(mark) = &text_top {
            self.line(format!("size_t {mark} = zeno_text_top;"));
        }
        self.line("{");
        let result = self.block(body);
        self.line("}");
        self.try_scopes.pop();
        self.tries.pop();
        let arrived = self.arriving.pop().unwrap_or_default();
        result?;

        // Only an error that travels as a value can reach an `except`. A
        // division by zero or a bad index stops a firmware build instead,
        // and an `except` waiting for one would wait for ever.
        const WHY: &str = "In a firmware build, an error the program raises itself (and `int` or `float` of text) travels as a value and can be caught. A division by zero, a bad index or an overflow stops the program. Test for it before it happens";
        if arrived.is_empty() {
            return Err(firmware_cannot(format!("nothing in this `try` can be caught. {WHY}"), span));
        }
        let anything = arrived.contains("Error");

        self.line(format!("{label}: ;"));
        if let Some(mark) = &text_top {
            self.line(format!("zeno_text_top = {mark};"));
        }
        // If the body always leaves by its last line, the only way to get
        // here is the `goto`, and there is nothing to ask.
        let leaves = matches!(
            body.last().map(|last| &last.kind),
            Some(StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue | StmtKind::Raise(_))
        );
        self.line(if leaves { "{" } else { "if (zeno_raised != NULL) {" });
        self.indent += 1;
        self.line(format!("zeno_error *{caught} = zeno_raised;"));
        self.line("zeno_raised = NULL;");
        self.line(format!("(void){caught};"));
        let mut handled: Vec<&str> = Vec::new();
        let mut catches_everything = false;
        for (index, handler) in handlers.iter().enumerate() {
            let (wanted, holds) = match handler.error.as_deref() {
                None | Some("Error") => (None, Kind::Error),
                Some(class) if self.classes.contains_key(class) => (Some(class), Kind::Class(class.to_string())),
                Some(kind) if ERROR_KINDS.contains(&kind) => (Some(kind), Kind::Error),
                Some(other) => return Err(cannot(format!("`except {other}`"), span)),
            };
            if let Some(kind) = wanted {
                // An `except` for a class that nothing here raises is idle
                // in every kind of build. One for a built-in kind is most
                // likely waiting for the runtime to raise it.
                if !anything && !arrived.contains(kind) && !self.classes.contains_key(kind) {
                    return Err(firmware_cannot(format!("nothing in this `try` can reach `except {kind}`. {WHY}"), span));
                }
                handled.push(kind);
            }
            let test = wanted.map(|kind| format!("if (zeno_is({caught}, {})) ", c_string(kind))).unwrap_or_default();
            self.line(if index == 0 { format!("{test}{{") } else { format!("}} else {test}{{") });
            self.scopes.push(HashMap::new());
            self.indent += 1;
            if !self.collector {
                // This block is where the error ends its life. However
                // the block is left, the error is freed on the way out.
                self.hold(Kind::Error, &caught);
            }
            if let Some(name) = &handler.binding {
                let c_type = c_type(&holds, span)?;
                let value = match holds {
                    Kind::Class(_) => format!("({}){caught}->object", c_type.trim_end()),
                    _ => caught.clone(),
                };
                let c_name = self.declare(name, holds);
                self.line(format!("{c_type}{c_name} = {value};"));
                self.line(format!("(void){c_name};"));
            }
            let result = handler.body.iter().try_for_each(|statement| self.stmt(statement));
            if result.is_ok() {
                self.end_of_block(&handler.body);
            }
            self.indent -= 1;
            self.scopes.pop();
            result?;
            if wanted.is_none() {
                catches_everything = true;
                break; // nothing gets past an `except` that takes every error
            }
        }
        // What no `except` here takes carries on outward.
        let passed_on: Vec<String> = arrived.iter().filter(|kind| !handled.contains(&kind.as_str())).cloned().collect();
        if !catches_everything {
            let leave = if passed_on.is_empty() {
                // Every kind the body can raise has an `except` above, so
                // this cannot happen. It is written out all the same: C
                // cannot see that, and coding standards want every chain
                // of `else if` to end in an `else`.
                "zeno_uncaught();".to_string()
            } else {
                self.failed(&passed_on, span)?
            };
            self.line("} else {");
            self.indent += 1;
            self.line(format!("zeno_raised = {caught};"));
            self.line(leave);
            self.indent -= 1;
        }
        self.line("}");
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    /// The kind a written type stands for, where the compiler needs to know
    /// it before it has a value to look at: `let xs: list[int] = []`.
    fn kind_of_type(&self, ty: &Type) -> Option<Kind> {
        // Inside a copy of a generic, `T` is whatever the copy was made
        // for, and `Stack[int]` is a copy itself. Only the checker knows
        // those, so its answer is used where it has one.
        if let Some(kind) = self.types.annotation(ty) {
            return Some(disowned(kind.clone()));
        }
        let (name, args) = match ty {
            Type::Named { name, args } => (name, args),
            // Who owns a value does not change how C holds it.
            Type::Owned(inner) | Type::Borrow(inner) => return self.kind_of_type(inner),
            Type::Optional(inner) => return Some(Kind::Optional(Box::new(self.kind_of_type(inner)?))),
            Type::Function { params, returns } => {
                let params = params.iter().map(|param| self.kind_of_type(param)).collect::<Option<Vec<Kind>>>()?;
                let returns = match returns {
                    Some(returns) => self.kind_of_type(returns)?,
                    None => Kind::None,
                };
                return Some(Kind::Function(Box::new(FunctionKind { params, returns })));
            }
        };
        Some(match (name.as_str(), args.as_slice()) {
            ("int", []) => Kind::Int,
            ("float", []) => Kind::Float,
            ("str", []) => Kind::Str,
            ("bool", []) => Kind::Bool,
            ("list", [item]) => Kind::List(Box::new(self.kind_of_type(item)?)),
            (name, []) if self.classes.contains_key(name) => Kind::Class(name.to_string()),
            (name, []) if self.enums.contains_key(name) => Kind::Enum(name.to_string()),
            (name, []) if units::is_unit(name) => Kind::Quantity(name.to_string()),
            ("Pin", []) => Kind::Pin,
            // `u8` and the rest: an int that is known to be small.
            (name, []) if ints::width(name).is_some() => Kind::Int,
            _ => return None,
        })
    }

    /// `expr`, for a place that knows what kind it wants. Only an empty
    /// list needs the help: `[]` alone does not say what it will hold.
    fn expr_as(&mut self, expr: &Expr, expected: Option<&Kind>) -> Result<Val, Error> {
        match (&expr.kind, expected) {
            (ExprKind::List(items), Some(kind @ Kind::List(_))) if items.is_empty() => {
                self.list_has_owner(expr)?;
                Ok(Val::plain("zeno_list_new(0)", kind.clone()))
            }
            _ => self.expr(expr),
        }
    }

    /// `x = v`, `point.x = v`, `items[i] = v`, and the same with `+=`.
    fn assign(&mut self, target: &Expr, op: Option<BinaryOp>, value: &Expr, span: Span) -> Result<(), Error> {
        let val = match op {
            None => self.expr(value)?,
            // `x += 1` is `x = x + 1`.
            Some(op) => {
                let mut vals = self.sequence(&[target, value])?;
                let right = vals.pop().expect("two values");
                let left = vals.pop().expect("two values");
                self.arithmetic(op, left, right, span)?
            }
        };
        self.stays(&val, value.span)?;
        let val = self.keep(val, op.is_none() && is_written(value));
        match &target.kind {
            ExprKind::Name(name) => {
                let Some(variable) = self.lookup(name) else {
                    return Err(cannot(format!("an assignment to `{name}`"), target.span));
                };
                let (c_name, kind, owner) = (variable.c_name.clone(), variable.kind.clone(), variable.owner);
                let mut val = self.coerce(val, &kind, span)?;
                if owner {
                    // The new value is made first, since making it may
                    // read the old one. Then the old one is freed.
                    val = self.temp(val, span)?;
                    self.line(free_code(&kind, &c_name, !self.collector));
                }
                self.line(format!("{c_name} = {};", val.code));
            }
            // For the other two, Zeno works the new value out first and then
            // looks up where it goes. If working the value out calls a
            // function, pin it down before touching the place.
            ExprKind::Field { object, name } if is_place(object) => {
                let val = if val.calls { self.temp(val, span)? } else { val };
                let object = self.expr(object)?;
                let field = match &object.kind {
                    Kind::Class(class) => self.classes[class].fields.iter().find(|(field, _, _)| field == name),
                    _ => None,
                };
                let Some((_, kind, _)) = field else {
                    return Err(cannot("an assignment to this", target.span));
                };
                let kind = kind.clone();
                let mut val = self.coerce(val, &kind, span)?;
                let owned = matches!(&object.kind, Kind::Class(class) if self.classes[class].owned_fields.contains(name))
                    || (!self.collector && is_text(&kind));
                if owned {
                    // As for an owner variable: new value first, then free
                    // the one the field held.
                    val = self.temp(val, span)?;
                    self.line(free_code(&kind, &format!("{}->z_{name}", object.code), !self.collector));
                }
                let sized = match &object.kind {
                    Kind::Class(class) => self.classes[class].sized.get(name).copied(),
                    _ => None,
                };
                match sized {
                    Some(width) => {
                        self.line(format!("{}->z_{name} = ({}){};", object.code, width.c_type(), val.code))
                    }
                    None => self.line(format!("{}->z_{name} = {};", object.code, val.code)),
                }
            }
            ExprKind::Index { object, index } if is_place(object) => {
                // A bare `none` is nothing to pin down, and has no C type
                // until it is known what else the list holds.
                let val = if val.kind == Kind::None { val } else { self.temp(val, span)? };
                let object = self.expr(object)?;
                let index = self.expr(index)?;
                let Kind::List(item) = &object.kind else {
                    return Err(cannot("an assignment to this", target.span));
                };
                let val = self.coerce(val, &item.clone(), span)?;
                self.fits_a_slot(&val.kind, span)?;
                let slot = to_slot(&val.kind, &val.code, span)?;
                if !self.collector && is_text(&val.kind) {
                    // The list is done with the text it held there.
                    let place = self.new_temp();
                    self.line(format!("zeno_slot *{place} = zeno_list_at({}, {}, {});", object.code, index.code, at(span)));
                    self.line(format!("zeno_text_drop({place}->s);"));
                    self.line(format!("*{place} = {slot};"));
                } else {
                    self.line(format!("*zeno_list_at({}, {}, {}) = {slot};", object.code, index.code, at(span)));
                }
            }
            _ => return Err(cannot("an assignment to this", target.span)),
        }
        Ok(())
    }

    /// `match`: each `case` becomes one branch of an `if ... else if` chain.
    /// A pattern gives two things: tests the value must pass (is it this
    /// case?), and new variables for the parts the pattern names.
    fn match_statement(&mut self, subject: &Expr, arms: &[Arm], span: Span) -> Result<(), Error> {
        // With no collector, text built to find the value is needed until
        // a case has been chosen, and is given back as that case begins.
        let begin = self.text_begin();
        let subject = self.expr(subject)?;
        let release = self.text_end(begin);
        let subject = self.temp(subject, span)?;
        let mut chosen = arms.is_empty();
        for (index, arm) in arms.iter().enumerate() {
            let mut tests = Vec::new();
            let mut names = Vec::new();
            self.pattern(&arm.pattern, &subject.code, &subject.kind, &mut tests, &mut names, span)?;
            // The checker has made sure the cases of an enum are all
            // covered. So if the last `case` only asks "is it this case?",
            // the answer is already yes, and it can be a plain `else`.
            let last = index + 1 == arms.len();
            if last && matches!(subject.kind, Kind::Enum(_)) && tests.len() == 1 {
                tests.clear();
            }
            let test = tests.join(" && ");
            self.line(match (index, tests.is_empty()) {
                (0, true) => "if (true) {".to_string(),
                (0, false) => format!("if ({test}) {{"),
                (_, true) => "} else {".to_string(),
                (_, false) => format!("}} else if ({test}) {{"),
            });
            self.scopes.push(HashMap::new());
            self.indent += 1;
            for (name, code, kind) in names {
                let c_type = c_type(&kind, span)?;
                let kept = !self.collector && is_text(&kind);
                let c_name = self.declare(&name, kind);
                if kept {
                    // The name outlives the text room's copy.
                    self.line(format!("{c_type}{c_name} = zeno_text_keep({code});"));
                    self.owns(&name);
                } else {
                    self.line(format!("{c_type}{c_name} = {code};"));
                }
            }
            if let Some(release) = &release {
                self.line(release.clone());
            }
            let result = arm.body.iter().try_for_each(|statement| self.stmt(statement));
            if result.is_ok() {
                self.end_of_block(&arm.body);
            }
            self.indent -= 1;
            self.scopes.pop();
            result?;
            if tests.is_empty() {
                chosen = true;
                break; // nothing after a pattern that takes everything can run
            }
        }
        match release {
            // No case was chosen: the text is given back all the same.
            Some(release) if !chosen => {
                self.line("} else {");
                self.indent += 1;
                self.line(release);
                self.indent -= 1;
                self.line("}");
            }
            Some(release) if arms.is_empty() => self.line(release),
            _ if !arms.is_empty() => self.line("}"),
            _ => {}
        }
        Ok(())
    }

    fn pattern(
        &mut self,
        pattern: &Pattern,
        code: &str,
        kind: &Kind,
        tests: &mut Vec<String>,
        names: &mut Vec<(String, String, Kind)>,
        span: Span,
    ) -> Result<(), Error> {
        match pattern {
            Pattern::Wildcard => {}
            Pattern::Name(name) => match (self.cases.get(name), kind) {
                // A bare enum case, such as `Idle`, is compared.
                (Some((owner, index)), Kind::Enum(subject)) if owner == subject && self.lookup(name).is_none() => {
                    tests.push(format!("{code}->tag == {index}"));
                }
                // Any other name is a new variable holding the value.
                _ => names.push((name.clone(), code.to_string(), kind.clone())),
            },
            Pattern::Literal(literal) => {
                let literal = self.expr(literal)?;
                let Some(common) = joined(kind, &literal.kind) else {
                    return Err(cannot("a pattern of a different type from the value", span));
                };
                let subject = self.coerce(Val::plain(code, kind.clone()), &common, span)?;
                let literal = self.coerce(literal, &common, span)?;
                tests.push(self.equal(&common, &subject.code, &literal.code, span)?);
            }
            Pattern::Variant { name, fields } => {
                let (owner, index) = match (self.cases.get(name), kind) {
                    (Some((owner, index)), Kind::Enum(subject)) if owner == subject => (owner.clone(), *index),
                    _ => return Err(cannot(format!("the pattern `{name}(...)` here"), span)),
                };
                let carried = self.enums[&owner].cases[index].1.clone();
                if carried.len() != fields.len() {
                    return Err(cannot(format!("the pattern `{name}(...)` with this many parts"), span));
                }
                tests.push(format!("{code}->tag == {index}"));
                for (field, (field_name, field_kind)) in fields.iter().zip(&carried) {
                    let place = format!("{code}->as.c{index}.z_{field_name}");
                    self.pattern(field, &place, field_kind, tests, names, span)?;
                }
            }
        }
        Ok(())
    }

    /// What follows the `}` of an `if`: nothing, `else`, or `else if`.
    fn else_part(&mut self, else_block: &Option<Block>) -> Result<(), Error> {
        let Some(block) = else_block else {
            self.line("}");
            return Ok(());
        };
        // The parser stores `else if` as an else block holding one `if`.
        if let [Stmt { kind: StmtKind::If { condition, then_block, else_block }, .. }] = block.as_slice() {
            let mark = self.lines.len();
            let condition = self.condition(condition)?;
            let before = self.lines.split_off(mark);
            if before.is_empty() {
                self.line(format!("}} else if ({}) {{", bare(&condition.code)));
                self.block(then_block)?;
                return self.else_part(else_block);
            }
            // The second condition needs statements of its own. They may
            // only run if the first condition was false, so they go inside
            // the `else`.
            self.line("} else {");
            self.lines.extend(before.into_iter().map(|(indent, line)| (indent + 1, line)));
            self.indent += 1;
            self.line(format!("if ({}) {{", bare(&condition.code)));
            self.block(then_block)?;
            self.else_part(else_block)?;
            self.indent -= 1;
            self.line("}");
            return Ok(());
        }
        self.line("} else {");
        self.block(block)?;
        self.line("}");
        Ok(())
    }

    /// The first line of a `for` loop: `for i in range(...)` or `for x in
    /// items`. It declares the loop variable in a scope of its own; the
    /// caller writes the body, the closing `}`, drops the scope, and gives
    /// what this returns to `close_loop`.
    fn open_loop(&mut self, variable: &str, iterable: &Expr) -> Result<Option<String>, Error> {
        let span = iterable.span;
        let range = match &iterable.kind {
            ExprKind::Call { callee, args }
                if matches!(&callee.kind, ExprKind::Name(name) if name == "range" && self.lookup(name).is_none())
                    && args.iter().all(|arg| arg.name.is_none()) =>
            {
                Some(args)
            }
            _ => None,
        };
        let Some(args) = range else {
            return self.open_list_loop(variable, iterable);
        };
        // A range becomes a plain counting loop: no list of numbers is built.
        let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
        let mut bounds = Vec::new();
        for val in self.sequence(&exprs)? {
            // Worked out once, before the loop, as `range` itself would.
            let is_literal = val.code.bytes().all(|b| b.is_ascii_digit());
            bounds.push(if is_literal { val.code } else { self.temp(val, span)?.code });
        }

        self.scopes.push(HashMap::new());
        let c_name = self.declare(variable, Kind::Int);
        match bounds.as_slice() {
            [stop] => self.line(format!("for (int64_t {c_name} = 0; {c_name} < {stop}; {c_name}++) {{")),
            [start, stop] => {
                self.line(format!("for (int64_t {c_name} = {start}; {c_name} < {stop}; {c_name}++) {{"))
            }
            // With a step, count the trips first. Counting up or down to
            // `stop` directly could run past the end of what an int holds.
            [start, stop, step] => {
                let count = self.new_temp();
                let trip = self.new_temp();
                self.line(format!(
                    "int64_t {count} = zeno_range_count({start}, {stop}, {step}, {});",
                    at(span)
                ));
                self.line(format!("for (int64_t {trip} = 0; {trip} < {count}; {trip}++) {{"));
                self.indent += 1;
                self.line(format!("int64_t {c_name} = {start} + {trip} * {step};"));
                self.indent -= 1;
            }
            _ => return Err(cannot("this `range`", span)),
        }
        Ok(None)
    }

    /// After the `}` of a `for` loop. A loop over a list walks a copy made
    /// for it alone (see `open_list_loop`), and `walked` is that copy.
    /// Nothing else ever saw it, so it can be freed here and now instead
    /// of being left for the collector. (A `return` from inside the loop
    /// skips this line, and frees the copy itself: see `open_list_loop`.)
    fn close_loop(&mut self, walked: Option<String>) {
        if let Some(drop) = walked {
            self.line(drop);
        }
    }

    /// `for x in items`. The loop walks the list as it was when the loop
    /// began, so a body that adds to the list or removes from it cannot
    /// confuse the loop. That is what `zeno run` does too.
    fn open_list_loop(&mut self, variable: &str, iterable: &Expr) -> Result<Option<String>, Error> {
        let span = iterable.span;
        // A list written out right here is nobody else's, so it needs no copy.
        let fresh = matches!(iterable.kind, ExprKind::List(_) | ExprKind::ListComp { .. });
        if fresh {
            self.walked = Some(iterable as *const Expr as usize);
        }
        let list = self.expr(iterable);
        self.walked = None;
        let list = list?;
        let item = match &list.kind {
            Kind::List(item) => (**item).clone(),
            // A str is looped over one character at a time.
            Kind::Str if !self.collector => {
                return Err(without_collector(
                    "a `for` loop over a str makes a list of its characters. \
                     Count instead: `for i in range(len(text))`, and use `text[i]`",
                    span,
                ))
            }
            Kind::Str => Kind::Str,
            _ => return Err(cannot("a `for` loop over anything but a list, a str or `range(...)`", span)),
        };
        let items = self.new_temp();
        let source = match (&list.kind, fresh) {
            (Kind::Str, _) => format!("zeno_chars({})", list.code),
            (_, true) => list.code,
            // With no collector a list of text has its own copies of its
            // items, and so does the copy the loop walks: the body may
            // give the list new items while the loop looks at the old.
            (_, false) if !self.collector && is_text(&item) => format!("zeno_list_copy_text({})", list.code),
            (_, false) => format!("zeno_list_copy({})", list.code),
        };
        self.line(format!("zeno_list *{items} = {source};"));

        let position = self.new_temp();
        self.scopes.push(HashMap::new());
        // The list being walked is the loop's own. A `return` or a failed
        // call inside the loop frees it on the way out; `close_loop` does
        // when the loop ends by itself.
        self.hold(Kind::List(Box::new(item.clone())), &items);
        let c_type = c_type(&item, span)?;
        let c_name = self.declare(variable, item.clone());
        self.line(format!("for (int64_t {position} = 0; {position} < {items}->len; {position}++) {{"));
        self.indent += 1;
        let value = from_slot(&item, &format!("{items}->items[{position}]"), span)?;
        self.line(format!("{c_type}{c_name} = {value};"));
        self.indent -= 1;
        Ok(Some(free_code(&Kind::List(Box::new(item)), &items, !self.collector)))
    }

    // ---------------------------------------------------------- Expressions

    /// Compile several expressions that Zeno works out left to right, and
    /// make sure C does too.
    ///
    /// C leaves the order of `f(a(), b())` and of `a() + b()` up to the
    /// compiler. That only shows when the order matters: when one piece
    /// calls a function (which may print, or change a variable) and the
    /// other reads a variable or does anything else that shows, or when two
    /// pieces can both fail (which error is reported?). In those cases the
    /// earlier piece is worked out into a temporary first. In the common case, such as `a + b * 2`, nothing is
    /// disturbed and the C looks like the Zeno.
    fn sequence(&mut self, exprs: &[&Expr]) -> Result<Vec<Val>, Error> {
        self.sequence_after(Vec::new(), exprs)
    }

    /// `sequence`, continuing from values that are already compiled.
    fn sequence_after(&mut self, mut done: Vec<Val>, exprs: &[&Expr]) -> Result<Vec<Val>, Error> {
        for expr in exprs {
            let mark = self.lines.len();
            let next = self.expr(expr)?;
            self.settle(&mut done, &next, mark, expr.span)?;
            done.push(next);
        }
        Ok(done)
    }

    /// `next` has just been compiled, after the values in `done`. Move into
    /// a temporary every earlier value that `next` could disturb, or that
    /// could disturb `next`. `mark` is how many statements there were
    /// before `next` wrote out its own: the temporaries go there.
    fn settle(&mut self, done: &mut [Val], next: &Val, mark: usize, span: Span) -> Result<(), Error> {
        let fails = next.fails || next.pre_fails;
        let calls = next.calls || next.pre_calls;
        let mut position = mark;
        for earlier in done {
            let disturbed = (calls && (earlier.reads || earlier.fails || earlier.calls))
                || (fails && (earlier.fails || earlier.calls))
                || (next.reads && earlier.calls);
            if disturbed {
                self.temps += 1;
                let name = format!("t{}", self.temps);
                let c_type = c_type(&earlier.kind, span)?;
                self.lines.insert(position, (self.indent, format!("{c_type}{name} = {};", earlier.code)));
                position += 1;
                earlier.pre_fails |= earlier.fails;
                earlier.pre_calls |= earlier.calls;
                earlier.code = name;
                earlier.reads = false;
                earlier.fails = false;
                earlier.calls = false;
            }
        }
        Ok(())
    }

    fn expr(&mut self, expr: &Expr) -> Result<Val, Error> {
        let val = self.value(expr)?;
        // For a build with no collector: text that was not already there
        // has just been built, and the statement will have to give it back.
        let text = match &val.kind {
            Kind::Optional(inner) => **inner == Kind::Str,
            kind => *kind == Kind::Str,
        };
        if text && !val.fixed && !self.collector {
            self.built = true;
        }
        Ok(val)
    }

    fn value(&mut self, expr: &Expr) -> Result<Val, Error> {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Int(n) => match i64::try_from(*n) {
                Ok(n) if n <= i32::MAX as i64 => Ok(Val::plain(n.to_string(), Kind::Int)),
                Ok(n) => Ok(Val::plain(format!("INT64_C({n})"), Kind::Int)),
                Err(_) => Err(cannot("a whole number this big", span)),
            },
            ExprKind::Float(x) => Ok(Val::plain(format!("{x:?}"), Kind::Float)),
            ExprKind::Str(text) => Ok(Val::plain(c_string(text), Kind::Str).fixed()),
            ExprKind::Bool(b) => Ok(Val::plain(b.to_string(), Kind::Bool)),
            // What `none` is in C depends on where it is going, so it is
            // settled there. See `coerce`.
            ExprKind::None => Ok(Val::plain("NULL", Kind::None)),

            ExprKind::Name(name) if self.lookup(name).is_none() && self.cases.contains_key(name) => {
                // A bare enum case, such as `Idle`.
                let (owner, index) = self.cases[name].clone();
                if !self.enums[&owner].cases[index].1.is_empty() {
                    return Err(cannot("an enum case that carries values, used without them", span));
                }
                Ok(Val::plain(format!("(&zeno_case_{owner}_{name})"), Kind::Enum(owner)))
            }
            ExprKind::Name(name)
                if self.lookup(name).is_none() && name == "pi" && self.imported.get(name).is_some_and(|m| m == "math") =>
            {
                Ok(Val::plain(format!("{:?}", std::f64::consts::PI), Kind::Float))
            }
            // `from board import led`
            ExprKind::Name(name)
                if self.lookup(name).is_none() && name == "led" && self.imported.get(name).is_some_and(|m| m == "board") =>
            {
                Ok(Val::plain("ZENO_LED", Kind::Pin))
            }
            ExprKind::Name(name) => match self.lookup(name) {
                // The owner gives its value away here. Take the pointer
                // out and leave NULL behind, so that the end of the owner's
                // block finds nothing to free.
                Some(variable) if self.types.moves(expr) => {
                    let (c_name, kind) = (variable.c_name.clone(), variable.kind.clone());
                    let taken = self.new_temp();
                    self.line(format!("{}{taken} = {c_name};", c_type(&kind, span)?));
                    self.line(format!("{c_name} = NULL;"));
                    // Nothing else on this line can be looking at the
                    // variable (the checker refuses a line that uses an
                    // owner and gives it away), and no other function can
                    // reach it. So no order has to be kept: the temporary
                    // is as plain as a literal.
                    Ok(Val::plain(taken, kind))
                }
                Some(variable) => {
                    let mut val = Val::plain(variable.c_name.clone(), variable.kind.clone());
                    // Only a top-level variable, or one shared with a
                    // function written inside this one, can be changed by a
                    // function that is called. Any other is out of every
                    // other function's reach, so nothing can disturb it.
                    let top_level = self.lookup_depth(name) == Some(0);
                    let reachable = top_level || variable.boxed.is_some();
                    let changeable = reachable && variable.mutable;
                    val.reads = reachable;
                    // Whatever a variable holds was there before this
                    // statement began. If a function could give the
                    // variable new text, what it holds now is lent.
                    let val = self.present(val, expr, changeable)?;
                    Ok(if changeable { self.lend(val) } else { val.fixed() })
                }
                None if self.functions.contains_key(name) => self.function_value(name, span),
                None => Err(cannot(format!("`{name}` used this way"), span)),
            },

            // A quantity is a C double. Its unit has done its work in the
            // checker and costs nothing in the compiled program.
            ExprKind::Unit { value, unit } => {
                let val = self.expr(value)?;
                Ok(Val::from(val.double(), Kind::Quantity(unit.clone()), &[&val]))
            }

            ExprKind::Unary { op: UnaryOp::Not, operand } => {
                let val = self.expr(operand)?;
                Ok(Val::from(format!("(!{})", val.code), Kind::Bool, &[&val]))
            }
            // Every bit turned over. C's `~` on an `int64_t` is the same.
            ExprKind::Unary { op: UnaryOp::BitNot, operand } => {
                let val = self.expr(operand)?;
                if val.kind != Kind::Int {
                    return Err(cannot("`~` on this", span));
                }
                Ok(Val::from(format!("(~{})", val.code), Kind::Int, &[&val]))
            }
            ExprKind::Unary { op: UnaryOp::Neg, operand } => {
                let val = self.expr(operand)?;
                match val.kind {
                    // `-5` written out is just the number minus five.
                    Kind::Int if matches!(operand.kind, ExprKind::Int(_)) => {
                        Ok(Val::plain(format!("-{}", val.code), Kind::Int))
                    }
                    Kind::Int => {
                        Ok(Val::from(format!("zeno_neg({}, {})", val.code, at(span)), Kind::Int, &[&val]).failing())
                    }
                    Kind::Float | Kind::Quantity(_) => {
                        Ok(Val::from(format!("(-{})", val.code), val.kind.clone(), &[&val]))
                    }
                    _ => Err(cannot("`-` on this", span)),
                }
            }

            ExprKind::Binary { op: op @ (BinaryOp::And | BinaryOp::Or), left, right } => {
                self.logic(*op, left, right, span)
            }
            // `x is none`, `x is not none`
            ExprKind::Binary { op: op @ (BinaryOp::Is | BinaryOp::IsNot), left, .. } => {
                let val = self.expr(left)?;
                let is = *op == BinaryOp::Is;
                let code = match &val.kind {
                    Kind::Optional(inner) if is_pointer(inner) => {
                        format!("({} {} NULL)", val.code, if is { "==" } else { "!=" })
                    }
                    Kind::Optional(_) if is => format!("(!{}.present)", val.code),
                    Kind::Optional(_) => format!("({}.present)", val.code),
                    // Anything else is either always `none` or never. It
                    // may still have work to do, such as a call.
                    other => {
                        let always_none = *other == Kind::None;
                        if (val.fails || val.calls) && !val.code.is_empty() {
                            self.line(format!("(void){};", val.code));
                        }
                        let mut answer = Val::plain((always_none == is).to_string(), Kind::Bool);
                        answer.pre_fails = val.pre_fails || val.fails;
                        answer.pre_calls = val.pre_calls || val.calls;
                        return Ok(answer);
                    }
                };
                Ok(Val::from(code, Kind::Bool, &[&val]))
            }
            ExprKind::Binary { op, left, right } => {
                let mut vals = self.sequence(&[left, right])?;
                let right = vals.pop().expect("two values");
                let left = vals.pop().expect("two values");
                if matches!(op, BinaryOp::In | BinaryOp::NotIn) {
                    return self.contains(*op, left, right, span);
                }
                if matches!(op, BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq)
                {
                    let code = self.compare(*op, &left, &right, span)?;
                    let mut val = Val::from(code, Kind::Bool, &[&left, &right]);
                    // Comparing lists or objects looks inside them.
                    val.reads |= !matches!(left.kind, Kind::Int | Kind::Float | Kind::Bool | Kind::Str | Kind::Quantity(_));
                    return Ok(val);
                }
                self.arithmetic(*op, left, right, span)
            }
            ExprKind::Compare { first, rest } => self.chain(first, rest, span),

            ExprKind::FormatStr(parts) => self.format_string(parts, span),
            // A call of a generic goes to one of its copies, and the
            // checker noted which: `first([1, 2])` calls `first[int]`.
            ExprKind::Call { args, .. } if self.types.copy_called(expr).is_some() => {
                let copy = self.types.copy_called(expr).unwrap_or_default().to_string();
                match self.functions.contains_key(&copy) {
                    true => self.call_function(&copy, args, span),
                    false if self.classes.contains_key(&copy) => self.construct(&copy, args, span),
                    false => Err(cannot(format!("`{copy}`, a generic that is not at the top of the file,"), span)),
                }
            }
            ExprKind::Call { callee, args } => self.call(callee, args, span),

            ExprKind::Field { object, name } => self.field(object, name, span),
            ExprKind::Index { object, index } => {
                let mut vals = self.sequence(&[object, index])?;
                let index = vals.pop().expect("two values");
                let list = vals.pop().expect("two values");
                if list.kind == Kind::Str {
                    let code = format!("zeno_char_at({}, {}, {})", list.code, index.code, at(span));
                    return Ok(Val::from(code, Kind::Str, &[&list, &index]).failing());
                }
                let Kind::List(item) = &list.kind else {
                    return Err(cannot("`[ ]` on anything but a list or a str", span));
                };
                let slot = format!("(*zeno_list_at({}, {}, {}))", list.code, index.code, at(span));
                let mut val = Val::from(from_slot(item, &slot, span)?, *item.clone(), &[&list, &index]).failing();
                val.reads = true;
                Ok(self.lend(val))
            }
            ExprKind::List(items) => {
                self.list_has_owner(expr)?;
                let exprs: Vec<&Expr> = items.iter().collect();
                let mut vals = self.sequence(&exprs)?;
                for (val, item) in vals.iter_mut().zip(items) {
                    self.stays(val, item.span)?;
                    if !self.collector && is_text(&val.kind) && !is_written(item) {
                        // Every item has been worked out by now, so each
                        // is copied from what the statement holds.
                        val.code = format!("zeno_text_keep({})", val.code);
                    }
                }
                let Some(first) = vals.first() else {
                    // `[]` alone does not say what it will hold. The place
                    // it arrives at does (`coerce`), and a `let` with no
                    // type beside it says it cannot tell.
                    return Ok(Val::plain("zeno_list_new(0)", Kind::List(Box::new(Kind::Other("nothing yet".to_string())))));
                };
                // `[1, none]` is a list of `int or none`: find the one kind
                // that every item can be stored as.
                let mut item = Some(first.kind.clone());
                for val in &vals {
                    item = item.and_then(|item| joined(&item, &val.kind));
                }
                let Some(item) = item else {
                    return Err(cannot("a list whose items are not all one type", span));
                };
                self.fits_a_slot(&item, span)?;
                let mut slots = vec![vals.len().to_string()];
                for val in &vals {
                    let val = self.coerce(val.clone(), &item, span)?;
                    slots.push(to_slot(&item, &val.code, span)?);
                }
                let parts: Vec<&Val> = vals.iter().collect();
                Ok(Val::from(format!("zeno_list_of({})", slots.join(", ")), Kind::List(Box::new(item)), &parts))
            }
            // `[n * n for n in items if n > 0]` is a loop that fills a list.
            ExprKind::ListComp { element, variable, iterable, condition } => {
                self.list_has_owner(expr)?;
                let list = self.new_temp();
                self.line(format!("zeno_list *{list} = zeno_list_new(0);"));
                // Until it is full, the list is nobody's. If working out
                // an item fails, the failure frees what there is of it.
                let filling = self.hold(Kind::List(Box::new(Kind::None)), &list);
                let filled_from = self.lines.len();
                let walked = self.open_loop(variable, iterable)?;
                self.indent += 1;
                // Text built for one item is given back before the next.
                let begin = self.text_begin();
                if let Some(condition) = condition {
                    let condition = self.condition(condition)?;
                    self.line(format!("if ({}) {{", bare(&condition.code)));
                    self.indent += 1;
                }
                let item = self.expr(element)?;
                self.stays(&item, element.span)?;
                let item = self.keep(item, is_written(element));
                self.fits_a_slot(&item.kind, span)?;
                self.line(format!("zeno_list_append({list}, {});", to_slot(&item.kind, &item.code, span)?));
                if !self.collector && is_text(&item.kind) {
                    // The list has copies of its items, which was not
                    // known when the lines above were written: a failure
                    // half way has to free those as well.
                    let (plain, texts) = (format!("zeno_list_drop({list});"), format!("zeno_list_drop_text({list});"));
                    for (_, line) in &mut self.lines[filled_from..] {
                        *line = line.replace(&plain, &texts);
                    }
                }
                if condition.is_some() {
                    self.indent -= 1;
                    self.line("}");
                }
                if let Some(release) = self.text_end(begin) {
                    self.line(release);
                }
                self.indent -= 1;
                self.line("}");
                self.scopes.pop();
                self.close_loop(walked);
                self.let_go(&filling);
                let mut val = Val::plain(list, Kind::List(Box::new(item.kind)));
                val.pre_fails = true;
                val.pre_calls = true;
                Ok(val)
            }
            ExprKind::Function(decl) => self.closure(decl, span),
        }
    }

    // ------------------------------------------------- Values that may be none

    /// Make `val` fit a place that holds values of kind `to`.
    ///
    /// Nearly always there is nothing to do. The work is at the border of
    /// `T or none`: a plain number going into an `int or none` has a flag
    /// put beside it, and `none` becomes whatever "nothing" is for that
    /// kind.
    fn coerce(&mut self, val: Val, to: &Kind, span: Span) -> Result<Val, Error> {
        if val.kind == *to {
            return Ok(val);
        }
        // An empty list becomes a list of whatever this place holds.
        let mut val = val;
        if is_empty_list(&val.kind) {
            match to {
                Kind::List(_) => return Ok(Val { kind: to.clone(), ..val }),
                Kind::Optional(inner) if matches!(**inner, Kind::List(_)) => val.kind = (**inner).clone(),
                _ => {}
            }
        }
        let code = match (&val.kind, to) {
            (Kind::None, Kind::Optional(inner)) => nothing(inner, span)?,
            (from, Kind::Optional(inner)) if from == &**inner => match from {
                from if is_pointer(from) => val.code.clone(),
                Kind::Int | Kind::Pin => format!("(zeno_opt_int){{true, {}}}", val.code),
                Kind::Float | Kind::Quantity(_) => format!("(zeno_opt_float){{true, {}}}", val.code),
                Kind::Bool => format!("(zeno_opt_bool){{true, {}}}", val.code),
                _ => return Err(cannot("this kind of value that may be `none`", span)),
            },
            (Kind::Function(_), Kind::Function(_)) => match joined(&val.kind, to) {
                Some(_) => val.code.clone(),
                None => {
                    return Err(cannot("a function used where a function of a different shape is wanted", span))
                }
            },
            // Anything else the checker has already agreed to as it is.
            _ => return Ok(val),
        };
        Ok(Val { code, kind: to.clone(), ..val })
    }

    /// A variable has just been read. If the checker has ruled `none` out
    /// at this spot (it is inside `if x is not none { ... }`, say), give
    /// the value inside instead of the `T or none` around it.
    ///
    /// `changeable` is for a variable that other functions can reach and
    /// change. One of them may have set it back to `none` since the check,
    /// so it is looked at once more before it is trusted.
    fn present(&mut self, val: Val, expr: &Expr, changeable: bool) -> Result<Val, Error> {
        let Kind::Optional(inner) = &val.kind else {
            return Ok(val);
        };
        if matches!(self.types.narrowed(expr), None | Some(Kind::None)) {
            return Ok(val);
        }
        let inner = (**inner).clone();
        let again = at(expr.span);
        let code = match (&inner, changeable) {
            (inner, false) if is_pointer(inner) => val.code.clone(),
            (_, false) => format!("{}.value", val.code),
            (Kind::Int | Kind::Pin, true) => format!("zeno_still_int({}, {again})", val.code),
            (Kind::Float | Kind::Quantity(_), true) => format!("zeno_still_float({}, {again})", val.code),
            (Kind::Bool, true) => format!("zeno_still_bool({}, {again})", val.code),
            (inner, true) => {
                format!("(({})zeno_still({}, {again}))", c_type(inner, expr.span)?.trim_end(), val.code)
            }
        };
        Ok(Val { code, kind: inner, fails: val.fails || changeable, ..val })
    }

    /// The name of a C function that shows a `T or none` as text. One is
    /// written for each kind, the first time it is needed.
    fn optional_text(&mut self, inner: &Kind, quoted: bool, span: Span) -> Result<String, Error> {
        let quoted = quoted && *inner == Kind::Str;
        let kind = Kind::Optional(Box::new(inner.clone()));
        let name = format!("zeno_{}_{}", if quoted { "repr" } else { "text" }, key(&kind));
        if self.adapters.insert(name.clone()) {
            let (present, value) = if is_pointer(inner) { ("x != NULL", "x") } else { ("x.present", "x.value") };
            let value = Val::plain(value, inner.clone());
            let shown = if quoted { self.repr(&value, span)? } else { self.text(&value, span)? };
            self.helper(
                format!("static const char *{name}({}x)", c_type(&kind, span)?),
                format!("    return {present} ? {shown} : \"none\";\n"),
            );
        }
        Ok(name)
    }

    /// The same, for comparing two of them: equal when both are `none`, or
    /// when neither is and the values inside are equal.
    fn optional_same(&mut self, inner: &Kind, span: Span) -> Result<String, Error> {
        let kind = Kind::Optional(Box::new(inner.clone()));
        let name = format!("zeno_same_{}", key(&kind));
        if self.adapters.insert(name.clone()) {
            let (a, b, a_value, b_value) = if is_pointer(inner) {
                ("a != NULL", "b != NULL", "a", "b")
            } else {
                ("a.present", "b.present", "a.value", "b.value")
            };
            let same = self.equal(inner, a_value, b_value, span)?;
            let c_type = c_type(&kind, span)?;
            self.helper(
                format!("static bool {name}({c_type}a, {c_type}b)"),
                format!("    if (!({a}) || !({b})) return ({a}) == ({b});\n    return {same};\n"),
            );
        }
        Ok(name)
    }

    // ------------------------------------------------------ Functions as values

    /// A function of the program used as a value: `apply(double, 3)`.
    ///
    /// Every function value is called the same way, with itself as a hidden
    /// first argument. An ordinary function does not take that argument, so
    /// it gets a small wrapper that does, and one fixed value that points
    /// at the wrapper.
    fn function_value(&mut self, name: &str, span: Span) -> Result<Val, Error> {
        let entry = &self.functions[name];
        if self.firmware && !entry.decl.raises.is_empty() {
            return Err(raises_as_value(span));
        }
        let (target, kind) = (entry.c_name.clone(), entry.kind.clone());
        let value = format!("zeno_value_{name}");
        if self.adapters.insert(value.clone()) {
            let mut params = vec!["zeno_closure *zeno_self".to_string()];
            let mut args = Vec::new();
            for (index, param) in kind.params.iter().enumerate() {
                params.push(format!("{}a{index}", c_type(param, span)?));
                args.push(format!("a{index}"));
            }
            let call = format!("{target}({})", args.join(", "));
            let call = if kind.returns == Kind::None { call } else { format!("return {call}") };
            self.helper(
                format!("static {}zeno_wrap_{name}({})", return_type(&kind.returns, span)?, params.join(", ")),
                format!("    (void)zeno_self;\n    {call};\n"),
            );
            self.definitions
                .push(format!("static zeno_closure {value} = {{(zeno_code)zeno_wrap_{name}, {}}};\n", c_string(name)));
        }
        Ok(Val::plain(format!("(&{value})"), Kind::Function(Box::new(kind))))
    }

    /// A function written where a value goes: `function(n) { return n * 2 }`.
    ///
    /// It becomes a C function of its own. If it uses variables from the
    /// function around it, they are packed into a struct made on the heap
    /// at this spot, and the C function receives that struct. That pair, a
    /// function and the variables it took with it, is called a closure.
    ///
    /// ```c
    /// struct zeno_env_1 { zeno_closure base; int64_t z_k; };
    /// static int64_t zeno_fn_1(zeno_closure *zeno_self, int64_t z_n) {
    ///     struct zeno_env_1 *env = (struct zeno_env_1 *)zeno_self;
    ///     return zeno_mul(z_n, env->z_k, 3, 40);
    /// }
    /// ```
    fn closure(&mut self, decl: &Arc<Function>, span: Span) -> Result<Val, Error> {
        let Some(kind) = self.types.function(decl).cloned() else {
            return Err(cannot("this function", span));
        };
        let (kind, owned) = disown_all(kind);
        if self.firmware && !decl.raises.is_empty() {
            return Err(raises_as_value(span));
        }
        if !self.collector {
            return Err(without_collector(
                "a function written where a value goes is kept in a block of its own, with the variables it \
                 uses, and has no owner. Give the function a name at the top of the file and use the name",
                span,
            ));
        }
        self.closures += 1;
        let id = self.closures;

        // Set aside the function being compiled, and start a new one. Every
        // variable in sight of this spot is offered to the new function as
        // `env->name`; the ones it looks up are the ones it needs.
        let mut outside = std::mem::take(&mut self.scopes);
        let globals = outside.remove(0);
        let mut offered: HashMap<String, Variable> = HashMap::new();
        for scope in &outside {
            for (name, variable) in scope {
                let (c_name, boxed) = match variable.boxed {
                    Some(_) => (format!("(*env->z_{name})"), Some(format!("env->z_{name}"))),
                    None => (format!("env->z_{name}"), None),
                };
                let variable = Variable {
                    c_name,
                    kind: variable.kind.clone(),
                    mutable: variable.mutable,
                    boxed,
                    used: Cell::new(false),
                    // The function around this one goes on owning it.
                    owner: false,
                };
                offered.insert(name.clone(), variable);
            }
        }
        self.scopes = vec![globals, offered, HashMap::new()];
        let base = std::mem::replace(&mut self.base, 2);
        let lines = std::mem::take(&mut self.lines);
        let indent = std::mem::replace(&mut self.indent, 1);
        let used = self.used.clone();

        let result = match self.parameters(decl, &kind, &owned, span) {
            Ok(params) => self.body(decl, &kind, false, span).map(|()| params),
            Err(error) => Err(error),
        };
        self.base = base;

        // Put everything back as it was.
        let body = render(&std::mem::replace(&mut self.lines, lines));
        self.indent = indent;
        self.used = used;
        let mut inside = std::mem::take(&mut self.scopes);
        outside.insert(0, inside.remove(0));
        let offered = inside.remove(0);
        self.scopes = outside;
        let params = result?;

        let mut taken: Vec<(&String, &Variable)> = offered.iter().filter(|(_, variable)| variable.used.get()).collect();
        taken.sort_by(|a, b| a.0.cmp(b.0));
        let mut all_params = vec!["zeno_closure *zeno_self".to_string()];
        all_params.extend(params);
        let first_line =
            format!("static {}zeno_fn_{id}({})", return_type(&kind.returns, span)?, all_params.join(", "));
        let name = c_string(&decl.name);
        let kind = Kind::Function(Box::new(kind));

        // The value itself is made here, on the heap, each time this spot
        // is reached: two function values are equal only if they are the
        // very same one, as in `zeno run`.
        let made = self.new_temp();
        let mut fields = String::new();
        let mut fill = Vec::new();
        for (zeno_name, variable) in &taken {
            let field_type = c_type(&variable.kind, span)?;
            let star = if variable.boxed.is_some() { "*" } else { "" };
            fields += &format!("    {field_type}{star}z_{zeno_name};\n");
            // What the variable is called out here, where the function is
            // being made. A boxed one hands over its box, not its value.
            let Some(source) = self.lookup(zeno_name) else {
                return Err(cannot(format!("a function that uses `{zeno_name}` from outside it"), span));
            };
            let source = source.boxed.clone().unwrap_or_else(|| source.c_name.clone());
            fill.push(format!("{made}->z_{zeno_name} = {source};"));
        }
        if taken.is_empty() {
            // It uses nothing from outside: the plain header is all of it.
            self.helper(first_line, format!("    (void)zeno_self;\n{body}"));
            self.line(format!("zeno_closure *{made} = (zeno_closure *)zeno_alloc(sizeof(zeno_closure));"));
            self.line(format!("{made}->code = (zeno_code)zeno_fn_{id};"));
            self.line(format!("{made}->name = {name};"));
            return Ok(Val::plain(made, kind));
        }
        let env = format!("struct zeno_env_{id}");
        self.structs.push(format!("{env} {{\n    zeno_closure base;\n{fields}}};"));
        self.helper(first_line, format!("    {env} *env = ({env} *)zeno_self;\n{body}"));
        self.line(format!("{env} *{made} = ({env} *)zeno_alloc(sizeof({env}));"));
        self.line(format!("{made}->base.code = (zeno_code)zeno_fn_{id};"));
        self.line(format!("{made}->base.name = {name};"));
        for line in fill {
            self.line(line);
        }
        Ok(Val::plain(format!("(&{made}->base)"), kind))
    }

    /// A call to a function held in a variable, or to anything else that
    /// is a function value: `f(3)`, `make_adder(2)(3)`, `handlers[0]()`.
    fn call_value(&mut self, function: Val, args: &[Arg], span: Span) -> Result<Val, Error> {
        let Kind::Function(kind) = function.kind.clone() else {
            return Err(cannot("a call to something that is not a function", span));
        };
        if args.iter().any(|arg| arg.name.is_some()) {
            return Err(cannot("named arguments to a function held in a variable", span));
        }
        if args.len() != kind.params.len() {
            return Err(cannot("a call with this many arguments", span));
        }
        // The C mentions the function twice, so give it a name first.
        let function = if is_c_place(&function.code) { function } else { self.temp(function, span)? };
        let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
        let vals = self.sequence_after(vec![function], &exprs)?;
        let mut codes = Vec::new();
        for (val, param) in vals[1..].iter().zip(&kind.params) {
            self.stays(val, span)?;
            codes.push(self.coerce(val.clone(), param, span)?.code);
        }
        let call = invoke(&vals[0].code, &kind, &codes, span)?;
        Ok(self.called(call, &vals, kind.returns.clone()))
    }

    /// `items.map(f)`, `items.filter(f)`, `items.reduce(f, start)`: each is
    /// a loop that calls `f` on every item. `vals` is the list, the
    /// function, and for `reduce` the starting value.
    ///
    /// They walk the list as it was when they began, as `zeno run` does, so
    /// `f` may change the list without confusing the loop.
    fn over_list(&mut self, name: &str, mut vals: Vec<Val>, item: &Kind, span: Span) -> Result<Val, Error> {
        if !self.collector && name != "reduce" {
            return Err(without_collector(format!("`{name}` makes a new list, which would have no owner"), span));
        }
        let takes = if name == "reduce" { 2 } else { 1 };
        let kind = match vals.get(1).map(|function| &function.kind) {
            Some(Kind::Function(kind)) if kind.params.len() == takes && vals.len() == takes + 1 => (**kind).clone(),
            _ => return Err(cannot(format!("the list method `{name}` called this way"), span)),
        };
        if !is_c_place(&vals[1].code) {
            vals[1] = self.temp(vals[1].clone(), span)?;
        }
        let function = vals[1].code.clone();

        // `reduce` starts from its second argument.
        let total = match vals.get(2) {
            Some(start) => {
                let start = self.coerce(start.clone(), &kind.returns, span)?;
                let total = self.new_temp();
                self.line(format!("{}{total} = {};", c_type(&kind.returns, span)?, start.code));
                total
            }
            None => String::new(),
        };
        let items = self.new_temp();
        let position = self.new_temp();
        // With no collector a list of text is walked as a `for` loop
        // walks one: a copy with copies of the items, which the function
        // that is called cannot pull away.
        let texts = !self.collector && is_text(item);
        let copy = if texts { "zeno_list_copy_text" } else { "zeno_list_copy" };
        self.line(format!("zeno_list *{items} = {copy}({});", vals[0].code));
        let slot = format!("{items}->items[{position}]");
        let each = Val::plain(from_slot(item, &slot, span)?, item.clone());

        let (before, inside, mut result) = match name {
            "map" => {
                let out = self.new_temp();
                let arg = self.coerce(each, &kind.params[0], span)?.code;
                let call = invoke(&function, &kind, &[arg], span)?;
                (
                    format!("zeno_list *{out} = zeno_list_new({items}->len);"),
                    format!("zeno_list_append({out}, {});", to_slot(&kind.returns, &call, span)?),
                    Val::plain(out, Kind::List(Box::new(kind.returns.clone()))),
                )
            }
            "filter" => {
                if kind.returns != Kind::Bool {
                    return Err(cannot("`filter` with a function that does not return a bool", span));
                }
                let out = self.new_temp();
                let arg = self.coerce(each, &kind.params[0], span)?.code;
                let call = invoke(&function, &kind, &[arg], span)?;
                (
                    format!("zeno_list *{out} = zeno_list_new(0);"),
                    format!("if ({call}) zeno_list_append({out}, {slot});"),
                    Val::plain(out, Kind::List(Box::new(item.clone()))),
                )
            }
            _ => {
                let so_far = self.coerce(Val::plain(total.clone(), kind.returns.clone()), &kind.params[0], span)?.code;
                let arg = self.coerce(each, &kind.params[1], span)?.code;
                let call = invoke(&function, &kind, &[so_far, arg], span)?;
                (String::new(), format!("{total} = {call};"), Val::plain(total, kind.returns.clone()))
            }
        };
        if !before.is_empty() {
            self.line(before);
        }
        self.line(format!("for (int64_t {position} = 0; {position} < {items}->len; {position}++) {{"));
        self.indent += 1;
        self.line(inside);
        self.indent -= 1;
        self.line("}");
        // The copy that was walked was nobody else's.
        self.line(free_code(&Kind::List(Box::new(item.clone())), &items, texts));
        result.pre_fails = true;
        result.pre_calls = true;
        Ok(result)
    }

    /// `+ - * / // % **`, on values that are already in order.
    fn arithmetic(&mut self, op: BinaryOp, left: Val, right: Val, span: Span) -> Result<Val, Error> {
        use BinaryOp::*;
        use Kind::{Float, Int, Quantity, Str};
        let parts = [&left, &right];
        let (l, r) = (&left.code, &right.code);
        let (dl, dr) = (left.double(), right.double());
        let symbol = op.text();
        let numbers = left.is_number() && right.is_number();

        let val = match (op, &left.kind, &right.kind) {
            // Whole numbers go through the runtime, which checks them.
            (Add, Int, Int) => Val::from(format!("zeno_add({l}, {r}, {})", at(span)), Int, &parts).failing(),
            (Sub, Int, Int) => Val::from(format!("zeno_sub({l}, {r}, {})", at(span)), Int, &parts).failing(),
            (Mul, Int, Int) => Val::from(format!("zeno_mul({l}, {r}, {})", at(span)), Int, &parts).failing(),
            (FloorDiv, Int, Int) => {
                Val::from(format!("zeno_floordiv({l}, {r}, {})", at(span)), Int, &parts).failing()
            }
            (Mod, Int, Int) => Val::from(format!("zeno_mod({l}, {r}, {})", at(span)), Int, &parts).failing(),
            (Pow, Int, Int) => Val::from(format!("zeno_pow({l}, {r}, {})", at(span)), Int, &parts).failing(),

            // The bits of whole numbers. C's own operators do these three.
            // The two shifts go through the runtime, which checks how far.
            (BitAnd | BitOr | BitXor, Int, Int) => Val::from(format!("({l} {symbol} {r})"), Int, &parts),
            (Shl, Int, Int) => Val::from(format!("zeno_shl({l}, {r}, {})", at(span)), Int, &parts).failing(),
            (Shr, Int, Int) => Val::from(format!("zeno_shr({l}, {r}, {})", at(span)), Int, &parts).failing(),

            (Add, Str, Str) => Val::from(format!("zeno_join(2, {l}, {r})"), Str, &parts),
            (Add, Kind::List(_), Kind::List(_)) if !self.collector => {
                return Err(without_collector("`+` on two lists makes a new list, which would have no owner", span))
            }
            (Add, Kind::List(a), Kind::List(b)) if a == b => {
                Val::from(format!("zeno_list_concat({l}, {r})"), left.kind.clone(), &parts)
            }

            // Quantities. The checker has made sure the two measure the
            // same thing; if the units differ (`1 s + 500 ms`), both sides
            // are scaled to the finer one.
            (Add | Sub, Quantity(a), Quantity(b)) => {
                let (unit, l, r) = in_common(&left, a, &right, b, span)?;
                Val::from(format!("({l} {symbol} {r})"), Quantity(unit), &parts)
            }
            (Mul, Quantity(unit), Int | Float) | (Mul, Int | Float, Quantity(unit)) => {
                Val::from(format!("({dl} * {dr})"), Quantity(unit.clone()), &parts)
            }
            (Div, Quantity(a), Quantity(b)) => {
                let (_, l, r) = in_common(&left, a, &right, b, span)?;
                Val::from(format!("zeno_div({l}, {r}, {})", at(span)), Float, &parts).failing()
            }
            (Div, Quantity(unit), Int | Float) => {
                Val::from(format!("zeno_div({dl}, {dr}, {})", at(span)), Quantity(unit.clone()), &parts).failing()
            }

            // What is left is floats, or an int with a float. C's own float
            // arithmetic already does what Zeno wants, except for dividing
            // by zero.
            (Add | Sub | Mul, _, _) if numbers => Val::from(format!("({dl} {symbol} {dr})"), Float, &parts),
            (Div, _, _) if numbers => {
                Val::from(format!("zeno_div({dl}, {dr}, {})", at(span)), Float, &parts).failing()
            }
            (FloorDiv, _, _) if numbers => {
                Val::from(format!("zeno_floordiv_float({dl}, {dr}, {})", at(span)), Float, &parts).failing()
            }
            (Mod, _, _) if numbers => {
                Val::from(format!("zeno_mod_float({dl}, {dr}, {})", at(span)), Float, &parts).failing()
            }
            (Pow, _, _) if numbers => Val::from(format!("pow({dl}, {dr})"), Float, &parts),

            _ => return Err(cannot(format!("`{symbol}` between these two types"), span)),
        };
        Ok(val)
    }

    /// `a and b`, `a or b`. The right side must not run unless it is needed.
    fn logic(&mut self, op: BinaryOp, left: &Expr, right: &Expr, span: Span) -> Result<Val, Error> {
        let left = self.expr(left)?;
        let mark = self.lines.len();
        let right = self.expr(right)?;
        let before = self.lines.split_off(mark);
        let symbol = if op == BinaryOp::And { "&&" } else { "||" };

        if before.is_empty() {
            // C's `&&` and `||` have the same rule, so this is direct.
            return Ok(Val::from(format!("({} {symbol} {})", left.code, right.code), Kind::Bool, &[&left, &right]));
        }
        // The right side needed statements of its own. They go inside an
        // `if`, so that they only run when the left side did not settle it.
        let result = self.temp(left, span)?;
        let guard = if op == BinaryOp::And { result.code.clone() } else { format!("!{}", result.code) };
        self.line(format!("if ({guard}) {{"));
        self.lines.extend(before.into_iter().map(|(indent, line)| (indent + 1, line)));
        self.indent += 1;
        self.line(format!("{} = {};", result.code, right.code));
        self.indent -= 1;
        self.line("}");
        let mut val = Val::plain(result.code, Kind::Bool);
        val.pre_fails = true;
        val.pre_calls = true;
        Ok(val)
    }

    /// `a < b < c`: each piece is worked out once, and only if the
    /// comparisons before it held.
    fn chain(&mut self, first: &Expr, rest: &[(BinaryOp, Expr)], span: Span) -> Result<Val, Error> {
        // When every piece is a plain name or literal, nothing can go wrong
        // by mentioning the middle one twice: `(a < b && b < c)`.
        if is_plain(first) && rest.iter().all(|(_, expr)| is_plain(expr)) {
            let mut exprs = vec![first];
            exprs.extend(rest.iter().map(|(_, expr)| expr));
            let vals = self.sequence(&exprs)?;
            let mut tests = Vec::new();
            for (index, (op, expr)) in rest.iter().enumerate() {
                tests.push(self.compare(*op, &vals[index], &vals[index + 1], expr.span)?);
            }
            let parts: Vec<&Val> = vals.iter().collect();
            return Ok(Val::from(format!("({})", tests.join(" && ")), Kind::Bool, &parts));
        }

        // Otherwise: one temporary per piece, and an `if` per comparison.
        let result = self.new_temp();
        self.line(format!("bool {result} = false;"));
        let first = self.expr(first)?;
        let mut previous = self.temp(first, span)?;
        let mut opened = 0;
        for (index, (op, expr)) in rest.iter().enumerate() {
            let next = self.expr(expr)?;
            let next = self.temp(next, expr.span)?;
            let test = self.compare(*op, &previous, &next, expr.span)?;
            if index + 1 == rest.len() {
                self.line(format!("{result} = {test};"));
            } else {
                self.line(format!("if ({}) {{", bare(&test)));
                self.indent += 1;
                opened += 1;
            }
            previous = next;
        }
        for _ in 0..opened {
            self.indent -= 1;
            self.line("}");
        }
        let mut val = Val::plain(result, Kind::Bool);
        val.pre_fails = true;
        val.pre_calls = true;
        Ok(val)
    }

    // ---------------------------------------------------------------- Calls

    fn call(&mut self, callee: &Expr, args: &[Arg], span: Span) -> Result<Val, Error> {
        match &callee.kind {
            ExprKind::Name(name) if self.lookup(name).is_none() => {
                if self.functions.contains_key(name) {
                    return self.call_function(name, args, span);
                }
                if self.classes.contains_key(name) {
                    return self.construct(name, args, span);
                }
                if let Some((owner, index)) = self.cases.get(name).cloned() {
                    return self.construct_case(&owner, index, args, span);
                }
                if self.imported.get(name).is_some_and(|module| module == "math") {
                    return self.math(name, args, span);
                }
                if let Some(module) = self.imported.get(name).filter(|module| *module != "math").cloned() {
                    return self.device(&module, name, args, span);
                }
                if args.iter().any(|arg| arg.name.is_some()) {
                    return Err(cannot("named arguments to a built-in", span));
                }
                if ERROR_KINDS.contains(&name.as_str()) && !self.collector && self.raising.take() != Some(span) {
                    return Err(without_collector(
                        format!("this makes a `{name}` that is not raised. An error is freed by the `except` that catches it"),
                        span,
                    ));
                }
                let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
                let vals = self.sequence(&exprs)?;
                // `ValueError("bad")` makes an error value. Its message is
                // whatever it is given, as text.
                if ERROR_KINDS.contains(&name.as_str()) {
                    let message = match vals.as_slice() {
                        [] => "\"\"".to_string(),
                        [message] => self.text(message, span)?,
                        _ => return Err(cannot(format!("`{name}` with more than one message"), span)),
                    };
                    let parts: Vec<&Val> = vals.iter().collect();
                    let code = format!("zeno_error_new({}, {message}, NULL)", c_string(name));
                    return Ok(Val::from(code, Kind::Error, &parts));
                }
                self.builtin(name, vals, span)
            }
            ExprKind::Field { object, name } => {
                if let ExprKind::Name(owner) = &object.kind {
                    if self.lookup(owner).is_none() {
                        // `math.sqrt(x)`
                        if owner == "math" {
                            return self.math(name, args, span);
                        }
                        // `time.sleep(1 s)`, `board.pin(7)`
                        if owner == "time" || owner == "board" {
                            return self.device(owner, name, args, span);
                        }
                        // `u8.wrap(n)`: the low bits of `n`, which cannot fail.
                        if let (Some(width), "wrap", [arg]) = (self.sized_name(owner), name.as_str(), args) {
                            let n = self.expr(&arg.value)?;
                            if n.kind != Kind::Int || arg.name.is_some() {
                                return Err(cannot(format!("`{owner}.wrap` called this way"), span));
                            }
                            let code = format!("zeno_wrap({}, {}, {})", n.code, width.bits, width.signed);
                            return Ok(Val::from(code, Kind::Int, &[&n]));
                        }
                        // `Shape.Circle(1.0)`
                        let case = self.enums.get(owner).and_then(|entry| entry.cases.iter().position(|(case, _)| case == name));
                        if let Some(index) = case {
                            return self.construct_case(owner, index, args, span);
                        }
                    }
                }
                self.method(object, name, args, span)
            }
            // Anything else that is called must be a function value.
            _ => {
                let function = self.expr(callee)?;
                self.call_value(function, args, span)
            }
        }
    }

    /// The size called `name`, if `name` is still the built-in one and not
    /// something of the program's own.
    fn sized_name(&self, name: &str) -> Option<Width> {
        let taken = self.functions.contains_key(name) || self.classes.contains_key(name) || self.enums.contains_key(name);
        ints::width(name).filter(|_| !taken)
    }

    /// A function of the `time` or the `board` module. Each is one call
    /// into the runtime, which passes it on to the board.
    fn device(&mut self, module: &str, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
        let vals = self.sequence(&exprs)?;
        match (module, name, vals.as_slice()) {
            // The board counts in seconds, whatever unit the program wrote.
            ("time", "sleep", [time]) => {
                let scale = match &time.kind {
                    Kind::Quantity(unit) => units::convert(unit, "s"),
                    _ => None,
                };
                let Some(scale) = scale else {
                    return Err(cannot("`sleep` of something that is not a time", span));
                };
                let call = format!("zeno_sleep({})", scaled(&time.code, scale));
                Ok(self.called(call, &vals, Kind::None))
            }
            // The clock moves by itself, so two readings are never one
            // value: it counts as a call, and is not worked out twice.
            ("time", "now", []) => Ok(self.called("zeno_now()".to_string(), &vals, Kind::Quantity("ms".to_string()))),
            ("board", "pin", [number]) if number.kind == Kind::Int => {
                Ok(Val::from(format!("zeno_pin({}, {})", number.code, at(span)), Kind::Pin, &[number]).failing())
            }
            _ => Err(cannot(format!("`{module}.{name}` called this way"), span)),
        }
    }

    fn math(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
        let vals = self.sequence(&exprs)?;
        let [x] = vals.as_slice() else {
            return Err(cannot(format!("`{name}` called this way"), span));
        };
        let d = x.double();
        Ok(match name {
            "sqrt" => Val::from(format!("zeno_sqrt({d}, {})", at(span)), Kind::Float, &[x]).failing(),
            "floor" => Val::from(format!("zeno_whole(floor({d}))"), Kind::Int, &[x]),
            "ceil" => Val::from(format!("zeno_whole(ceil({d}))"), Kind::Int, &[x]),
            "sin" => Val::from(format!("sin({d})"), Kind::Float, &[x]),
            "cos" => Val::from(format!("cos({d})"), Kind::Float, &[x]),
            _ => return Err(cannot(format!("`math.{name}`"), span)),
        })
    }

    /// The C for a call: `c_name(...)` with one argument for each of
    /// `slots`, which are the names and default values of what the function
    /// takes. `receiver` is the object a method is called on, already
    /// compiled; it goes first.
    ///
    /// The arguments are worked out in the order they are written, even
    /// when names put them elsewhere. Returns the call, and the values that
    /// went into it.
    fn call_c(
        &mut self,
        c_name: &str,
        slots: &[(String, Kind, Option<Expr>)],
        owned: &[bool],
        receiver: Option<Val>,
        args: &[Arg],
        span: Span,
    ) -> Result<(String, Vec<Val>), Error> {
        let mut places = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            places.push(match &arg.name {
                None => index,
                Some(name) => match slots.iter().position(|(slot, _, _)| slot == name) {
                    Some(place) => place,
                    None => return Err(cannot(format!("the argument `{name}`"), span)),
                },
            });
        }
        let first = receiver.is_some() as usize;
        // The arguments, left to right, as `sequence` would work them out.
        let mut vals: Vec<Val> = receiver.into_iter().collect();
        let mut held = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let mark = self.lines.len();
            let mut next = self.expr(&arg.value)?;
            self.stays(&next, arg.value.span)?;
            self.settle(&mut vals, &next, mark, arg.value.span)?;
            // An owned value on its way into the call has left its owner
            // and not yet reached the function. In a firmware build a
            // later argument may fail, and the failure leaves this
            // function at once. Hold the value until the call, so that
            // such a failure frees it.
            let taken = owned.get(places[index]).copied().unwrap_or(false);
            let later_work = args[index + 1..].iter().any(|arg| !is_plain(&arg.value));
            if taken && later_work && self.firmware {
                if !self.types.moves(&arg.value) {
                    next = self.temp(next, arg.value.span)?;
                }
                held.push(self.hold(next.kind.clone(), &next.code));
            }
            vals.push(next);
        }
        // From here the function owns what it was given.
        for key in &held {
            self.let_go(key);
        }

        let mut codes: Vec<Option<String>> = vec![None; slots.len()];
        for (place, val) in places.iter().zip(&vals[first..]) {
            let Some((_, kind, _)) = slots.get(*place) else {
                return Err(cannot("this many arguments", span));
            };
            codes[*place] = Some(self.coerce(val.clone(), kind, span)?.code);
        }
        // An argument that was left out takes the default.
        for (code, (_, kind, default)) in codes.iter_mut().zip(slots) {
            if code.is_none() {
                let default = match default {
                    Some(default) if is_literal(default) => self.expr(default)?,
                    _ => return Err(cannot("a default value that is not a plain literal", span)),
                };
                *code = Some(self.coerce(default, kind, span)?.code);
            }
        }
        let mut codes: Vec<String> = codes.into_iter().flatten().collect();
        if first == 1 {
            codes.insert(0, vals[0].code.clone());
        }
        Ok((format!("{c_name}({})", codes.join(", ")), vals))
    }

    /// Turn a call to a function or method into a value.
    fn called(&mut self, call: String, vals: &[Val], returns: Kind) -> Val {
        let parts: Vec<&Val> = vals.iter().collect();
        if returns == Kind::None {
            // Nothing comes back, so the call is a statement by itself.
            self.line(format!("{call};"));
            let mut val = Val::from(String::new(), Kind::None, &parts);
            val.pre_fails = true;
            val.pre_calls = true;
            return val;
        }
        // With no collector, text that comes back is in the text room.
        let mut val = Val::from(call, returns, &parts);
        if self.collector || !is_text(&val.kind) {
            val = val.fixed();
        }
        val.fails = true;
        val.calls = true;
        val
    }

    /// A call to one of the program's own functions.
    fn call_function(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let entry = &self.functions[name];
        let (c_name, returns) = (entry.c_name.clone(), entry.kind.returns.clone());
        let slots: Vec<_> = entry
            .decl
            .params
            .iter()
            .zip(&entry.kind.params)
            .map(|(param, kind)| (param.name.clone(), kind.clone(), param.default.clone()))
            .collect();
        let (raises, owned) = (entry.decl.raises.clone(), entry.owned.clone());
        let (call, vals) = self.call_c(&c_name, &slots, &owned, None, args, span)?;
        if self.firmware && !raises.is_empty() {
            return self.checked_call(call, returns, &raises, span);
        }
        Ok(self.called(call, &vals, returns))
    }

    /// `Point(3.0, 4.0)`: a new object.
    fn construct(&mut self, class: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let entry = &self.classes[class];
        // With no collector, an object needs an owner. The one exception
        // is an error being raised, which the `except` that catches it
        // will free.
        if !self.collector && !entry.owned && self.raising.take() != Some(span) {
            return Err(without_collector(
                format!("`{class}` is not an `owned class`, so this object would have no owner"),
                span,
            ));
        }
        let entry = &self.classes[class];
        let slots = entry.fields.clone();
        let owned: Vec<bool> = slots.iter().map(|(field, _, _)| entry.owned_fields.contains(field)).collect();
        let (call, vals) = self.call_c(&format!("zeno_new_{}", ident(class)), &slots, &owned, None, args, span)?;
        let parts: Vec<&Val> = vals.iter().collect();
        Ok(Val::from(call, Kind::Class(class.to_string()), &parts))
    }

    /// `Circle(1.0)`: a new value of an enum.
    fn construct_case(&mut self, owner: &str, index: usize, args: &[Arg], span: Span) -> Result<Val, Error> {
        let (case, fields) = self.enums[owner].cases[index].clone();
        if fields.is_empty() {
            return Err(cannot(format!("`{case}` called like a function: it carries no values"), span));
        }
        if !self.collector {
            return Err(without_collector(
                format!("`{case}` carries values, so it is kept in a block of its own, and an enum value has no owner"),
                span,
            ));
        }
        let slots: Vec<_> = fields.iter().map(|(name, kind)| (name.clone(), kind.clone(), None)).collect();
        let (call, vals) = self.call_c(&format!("zeno_new_{owner}_{case}"), &slots, &[], None, args, span)?;
        let parts: Vec<&Val> = vals.iter().collect();
        Ok(Val::from(call, Kind::Enum(owner.to_string()), &parts))
    }

    /// `object.name(args)`: a method of a class, or of a list.
    fn method(&mut self, object: &Expr, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let receiver = self.expr(object)?;
        match receiver.kind.clone() {
            Kind::Class(class) => {
                let Some(entry) = self.classes[&class].methods.get(name) else {
                    // Not a method. It may be a field that holds a function.
                    let field = self.classes[&class].fields.iter().find(|(field, _, _)| field == name);
                    let Some((_, kind @ Kind::Function(_), _)) = field else {
                        return Err(cannot(format!("the method `{name}`"), span));
                    };
                    let mut function = Val::from(format!("{}->z_{name}", receiver.code), kind.clone(), &[&receiver]);
                    function.reads = true;
                    return self.call_value(function, args, span);
                };
                let (c_name, returns) = (entry.c_name.clone(), entry.kind.returns.clone());
                // The first parameter is `self`, which the receiver fills.
                let slots: Vec<_> = entry
                    .decl
                    .params
                    .iter()
                    .zip(&entry.kind.params)
                    .skip(1)
                    .map(|(param, kind)| (param.name.clone(), kind.clone(), param.default.clone()))
                    .collect();
                let raises = entry.decl.raises.clone();
                let owned: Vec<bool> = entry.owned.iter().skip(1).copied().collect();
                let (call, vals) = self.call_c(&c_name, &slots, &owned, Some(receiver), args, span)?;
                if self.firmware && !raises.is_empty() {
                    return self.checked_call(call, returns, &raises, span);
                }
                Ok(self.called(call, &vals, returns))
            }
            // A pin of the board: `led.on()`. Each is one call into the
            // runtime, with the pin's number.
            Kind::Pin => {
                if !args.is_empty() {
                    return Err(cannot(format!("`{name}` on a pin with arguments"), span));
                }
                match name {
                    "on" | "off" | "toggle" => {
                        let call = format!("zeno_pin_{name}({})", receiver.code);
                        Ok(self.called(call, &[receiver], Kind::None))
                    }
                    "is_on" => {
                        let mut val = Val::from(format!("zeno_pin_is_on({})", receiver.code), Kind::Bool, &[&receiver]);
                        // What it reads is changed by `on` and `off`.
                        val.reads = true;
                        Ok(val)
                    }
                    _ => Err(cannot(format!("the method `{name}`"), span)),
                }
            }
            Kind::Str => {
                let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
                let vals = self.sequence_after(vec![receiver], &exprs)?;
                let parts: Vec<&Val> = vals.iter().collect();
                let all_text = vals.iter().all(|val| val.kind == Kind::Str);
                if name == "split" && !self.collector {
                    return Err(without_collector("`split` makes a new list, which would have no owner", span));
                }
                let codes: Vec<&str> = vals.iter().map(|val| val.code.as_str()).collect();
                Ok(match (name, codes.as_slice()) {
                    ("upper", [text]) => Val::from(format!("zeno_upper({text})"), Kind::Str, &parts),
                    ("lower", [text]) => Val::from(format!("zeno_lower({text})"), Kind::Str, &parts),
                    ("strip", [text]) => Val::from(format!("zeno_strip({text})"), Kind::Str, &parts),
                    ("split", [text]) => {
                        Val::from(format!("zeno_split_blanks({text})"), Kind::List(Box::new(Kind::Str)), &parts)
                    }
                    ("split", [text, separator]) if all_text => Val::from(
                        format!("zeno_split({text}, {separator}, {})", at(span)),
                        Kind::List(Box::new(Kind::Str)),
                        &parts,
                    )
                    .failing(),
                    ("replace", [text, old, new]) if all_text => {
                        Val::from(format!("zeno_replace({text}, {old}, {new})"), Kind::Str, &parts)
                    }
                    ("starts_with", [text, start]) if all_text => {
                        Val::from(format!("zeno_starts_with({text}, {start})"), Kind::Bool, &parts)
                    }
                    ("ends_with", [text, end]) if all_text => {
                        Val::from(format!("zeno_ends_with({text}, {end})"), Kind::Bool, &parts)
                    }
                    _ => return Err(cannot(format!("the str method `{name}` called this way"), span)),
                })
            }
            Kind::List(item) => {
                let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
                let vals = self.sequence_after(vec![receiver], &exprs)?;
                let parts: Vec<&Val> = vals.iter().collect();
                match (name, vals.as_slice()) {
                    ("map" | "filter" | "reduce", _) => self.over_list(name, vals, &item, span),
                    ("append", [list, value]) => {
                        self.stays(value, span)?;
                        self.fits_a_slot(&item, span)?;
                        let value = self.keep(value.clone(), is_written(&args[0].value));
                        let value = self.coerce(value, &item, span)?;
                        let slot = to_slot(&value.kind, &value.code, span)?;
                        self.line(format!("zeno_list_append({}, {slot});", list.code));
                        let mut val = Val::from(String::new(), Kind::None, &parts);
                        val.pre_calls = true;
                        Ok(val)
                    }
                    ("pop", [list]) => {
                        let slot = format!("zeno_list_pop({}, {})", list.code, at(span));
                        let mut code = from_slot(&item, &slot, span)?;
                        if !self.collector && is_text(&item) {
                            // The list is done with it: the statement gets
                            // it, and the list's copy is freed.
                            code = format!("zeno_text_take({code})");
                        }
                        let mut val = Val::from(code, *item.clone(), &parts);
                        // It changes the list, which is as visible as a call.
                        val.fails = true;
                        val.calls = true;
                        Ok(val)
                    }
                    _ => Err(cannot(format!("the list method `{name}`"), span)),
                }
            }
            _ => Err(cannot(format!("the method `{name}`"), span)),
        }
    }

    /// `object.name`: a field of an object, or a quantity in another unit.
    fn field(&mut self, object: &Expr, name: &str, span: Span) -> Result<Val, Error> {
        if let ExprKind::Name(owner) = &object.kind {
            if self.lookup(owner).is_none() {
                if owner == "math" && name == "pi" {
                    return Ok(Val::plain(format!("{:?}", std::f64::consts::PI), Kind::Float));
                }
                if owner == "board" && name == "led" {
                    return Ok(Val::plain("ZENO_LED", Kind::Pin));
                }
                // `Shape.Idle`
                if self.enums.contains_key(owner) && self.cases.get(name).is_some_and(|(of, _)| of == owner) {
                    let index = self.cases[name].1;
                    if self.enums[owner].cases[index].1.is_empty() {
                        return Ok(Val::plain(format!("(&zeno_case_{owner}_{name})"), Kind::Enum(owner.clone())));
                    }
                }
            }
        }
        let val = self.expr(object)?;
        match &val.kind {
            // `(3 s).ms`: the same quantity in another unit.
            Kind::Quantity(unit) if units::is_unit(name) => match units::convert(unit, name) {
                Some(scale) => Ok(Val::from(scaled(&val.code, scale), Kind::Quantity(name.to_string()), &[&val])),
                None => Err(cannot(format!("`.{name}` on this"), span)),
            },
            Kind::Class(class) => match self.classes[class].fields.iter().find(|(field, _, _)| field == name) {
                Some((_, kind, _)) => {
                    let code = match self.classes[class].sized.contains_key(name) {
                        // A small field is an ordinary int again as soon
                        // as it is read. Left as a `uint32_t`, C would do
                        // the sums that follow without a sign.
                        true => format!("((int64_t){}->z_{name})", val.code),
                        false => format!("{}->z_{name}", val.code),
                    };
                    let mut field = Val::from(code, kind.clone(), &[&val]);
                    // A function that is called may change the field.
                    field.reads = true;
                    Ok(self.lend(field))
                }
                None => Err(cannot(format!("a method used as a value: `{name}`"), span)),
            },
            Kind::Error if matches!(name, "message" | "kind") => {
                Ok(Val::from(format!("{}->{name}", val.code), Kind::Str, &[&val]))
            }
            // `led.number`: a pin is its number already.
            Kind::Pin if name == "number" => Ok(Val { kind: Kind::Int, ..val }),
            _ => Err(cannot(format!("`.{name}` on this"), span)),
        }
    }

    /// `x in items`, `x not in items`, and the same for a piece of a str.
    fn contains(&mut self, op: BinaryOp, left: Val, right: Val, span: Span) -> Result<Val, Error> {
        let code = match (&left.kind, &right.kind) {
            (item, Kind::List(items)) if item == &**items => {
                let same = self.eq_adapter(item, span)?;
                format!("zeno_list_contains({}, {}, {same})", right.code, to_slot(item, &left.code, span)?)
            }
            (Kind::Str, Kind::Str) => format!("(strstr({}, {}) != NULL)", right.code, left.code),
            _ => return Err(cannot("`in` between these two types", span)),
        };
        let code = if op == BinaryOp::NotIn { format!("(!{code})") } else { code };
        let mut val = Val::from(code, Kind::Bool, &[&left, &right]);
        val.reads = true;
        Ok(val)
    }

    // ------------------------------------------------- Showing and comparing

    /// The C expression that turns a value into the text `print` shows.
    fn text(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        // Anything but text has to be turned into text, which builds some.
        if val.kind != Kind::Str && !self.collector {
            self.built = true;
        }
        Ok(match &val.kind {
            Kind::Str => val.code.clone(),
            Kind::Int => format!("zeno_str_int({})", val.code),
            Kind::Float => format!("zeno_str_float({})", val.code),
            Kind::Bool => format!("zeno_str_bool({})", val.code),
            Kind::Pin => format!("zeno_str_pin({})", val.code),
            Kind::Quantity(unit) => format!("zeno_str_quantity({}, {})", val.code, c_string(unit)),
            Kind::Class(name) | Kind::Enum(name) => format!("zeno_show_{}({})", ident(name), val.code),
            Kind::List(item) => format!("zeno_list_str({}, {})", val.code, self.repr_adapter(item, span)?),
            Kind::Error => format!("{}->message", val.code),
            Kind::Function(_) => format!("zeno_show_function({})", val.code),
            Kind::Optional(inner) => format!("{}({})", self.optional_text(inner, false, span)?, val.code),
            // `none` itself, or a call that gave nothing back.
            Kind::None => "\"none\"".to_string(),
            Kind::Other(name) => return Err(cannot(format!("printing a value of type {name}"), span)),
            Kind::Owned(_) => return Err(cannot("printing this", span)),
        })
    }

    /// The text for a value inside a list or an object. Only a str differs:
    /// there it is shown in quotes.
    fn repr(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        if !self.collector {
            self.built = true;
        }
        match &val.kind {
            Kind::Str => Ok(format!("zeno_repr_str({})", val.code)),
            Kind::Optional(inner) => Ok(format!("{}({})", self.optional_text(inner, true, span)?, val.code)),
            _ => self.text(val, span),
        }
    }

    /// A C expression that is true when two values of this kind are equal.
    /// Objects, enum values and lists are equal when their contents are.
    fn equal(&mut self, kind: &Kind, a: &str, b: &str, span: Span) -> Result<String, Error> {
        Ok(match kind {
            Kind::Int | Kind::Float | Kind::Bool | Kind::Pin | Kind::Quantity(_) => format!("({a} == {b})"),
            Kind::Str => format!("(strcmp({a}, {b}) == 0)"),
            Kind::Class(name) | Kind::Enum(name) => format!("zeno_same_{}({a}, {b})", ident(name)),
            Kind::List(item) => format!("zeno_list_eq({a}, {b}, {})", self.eq_adapter(item, span)?),
            Kind::Error => format!("zeno_error_same({a}, {b})"),
            Kind::Optional(inner) => format!("{}({a}, {b})", self.optional_same(inner, span)?),
            Kind::None => "true".to_string(),
            // Two function values are equal only if they are the same one.
            Kind::Function(_) => format!("({a} == {b})"),
            Kind::Other(_) | Kind::Owned(_) => return Err(cannot("comparing these", span)),
        })
    }

    /// `==` and the rest, between two compiled values.
    fn compare(&mut self, op: BinaryOp, left: &Val, right: &Val, span: Span) -> Result<String, Error> {
        let plain = |kind: &Kind| matches!(kind, Kind::Int | Kind::Float | Kind::Bool | Kind::Str | Kind::Quantity(_));
        if plain(&left.kind) && plain(&right.kind) || !matches!(op, BinaryOp::Eq | BinaryOp::NotEq) {
            return comparison(op, left, right, span);
        }
        // Everything else is equal when its contents are. `x == none` and
        // `x == 5`, where `x` may be `none`, first bring both sides to the
        // same kind.
        let Some(kind) = joined(&left.kind, &right.kind) else {
            return Err(cannot(format!("`{}` between these two types", op.text()), span));
        };
        let left = self.coerce(left.clone(), &kind, span)?;
        let right = self.coerce(right.clone(), &kind, span)?;
        let same = self.equal(&kind, &left.code, &right.code, span)?;
        Ok(if op == BinaryOp::Eq { same } else { format!("(!{same})") })
    }

    /// The runtime's list functions do not know what a list holds. To show
    /// a list they are handed a small function that shows one item. This
    /// writes that function for a kind, the first time it is needed, and
    /// returns its name.
    fn repr_adapter(&mut self, kind: &Kind, span: Span) -> Result<String, Error> {
        let name = format!("zeno_item_repr_{}", key(kind));
        if self.adapters.insert(name.clone()) {
            let item = Val::plain(from_slot(kind, "item", span)?, kind.clone());
            let body = self.repr(&item, span)?;
            self.helper(format!("static const char *{name}(zeno_slot item)"), format!("    return {body};\n"));
        }
        Ok(name)
    }

    /// The same, for comparing two items.
    fn eq_adapter(&mut self, kind: &Kind, span: Span) -> Result<String, Error> {
        let name = format!("zeno_item_eq_{}", key(kind));
        if self.adapters.insert(name.clone()) {
            let (a, b) = (from_slot(kind, "a", span)?, from_slot(kind, "b", span)?);
            let body = self.equal(kind, &a, &b, span)?;
            self.helper(format!("static bool {name}(zeno_slot a, zeno_slot b)"), format!("    return {body};\n"));
        }
        Ok(name)
    }

    fn builtin(&mut self, name: &str, vals: Vec<Val>, span: Span) -> Result<Val, Error> {
        let parts: Vec<&Val> = vals.iter().collect();
        let one = || match vals.as_slice() {
            [val] => Ok(val),
            _ => Err(cannot(format!("`{name}` called this way"), span)),
        };
        Ok(match name {
            "print" => {
                // `print` shows its values after all of them are worked out.
                // If one is a list or an object, and another calls
                // something that could change it, finish the calls first.
                let mut vals = vals;
                if vals.iter().any(|val| matches!(val.kind, Kind::List(_) | Kind::Class(_))) {
                    for val in &mut vals {
                        if val.calls {
                            let taken = std::mem::replace(val, Val::plain("", Kind::None));
                            *val = self.temp(taken, span)?;
                        }
                    }
                }
                let parts: Vec<&Val> = vals.iter().collect();
                let mut texts = vec![vals.len().to_string()];
                for val in &vals {
                    texts.push(self.text(val, span)?);
                }
                self.line(format!("zeno_print({});", texts.join(", ")));
                let mut val = Val::from(String::new(), Kind::None, &parts);
                val.pre_calls = true;
                val
            }
            "str" => Val::from(self.text(one()?, span)?, Kind::Str, &parts),
            // `u8(n)`: `n` itself, once the runtime has seen that it fits.
            // The size is handed over as the words for the message.
            _ if ints::width(name).is_some() && matches!(one(), Ok(val) if val.kind == Kind::Int) => {
                let width = ints::width(name).unwrap_or(ints::WIDTHS[0]);
                let article = if width.signed { "an" } else { "a" };
                let code = format!(
                    "zeno_fit({}, INT64_C({}), INT64_C({}), \"{article} {name}\", {})",
                    one()?.code,
                    width.min(),
                    width.max(),
                    at(span)
                );
                Val::from(code, Kind::Int, &parts).failing()
            }
            "len" => match one()? {
                val if val.kind == Kind::Str => Val::from(format!("zeno_len({})", val.code), Kind::Int, &parts),
                val if matches!(val.kind, Kind::List(_)) => {
                    let mut len = Val::from(format!("{}->len", val.code), Kind::Int, &parts);
                    len.reads = true;
                    len
                }
                _ => return Err(cannot("`len` of this", span)),
            },
            "abs" => {
                let val = one()?;
                match &val.kind {
                    Kind::Int => Val::from(format!("zeno_abs({}, {})", val.code, at(span)), Kind::Int, &parts).failing(),
                    Kind::Float | Kind::Quantity(_) => {
                        Val::from(format!("fabs({})", val.code), val.kind.clone(), &parts)
                    }
                    _ => return Err(cannot("`abs` of this", span)),
                }
            }
            "int" => {
                let val = one()?;
                match &val.kind {
                    Kind::Int => Val::from(val.code.clone(), Kind::Int, &parts),
                    Kind::Float => {
                        Val::from(format!("zeno_int({}, {})", val.code, at(span)), Kind::Int, &parts).failing()
                    }
                    Kind::Str => {
                        self.built = !self.collector;
                        let call = format!("zeno_parse_int({}, {})", val.code, at(span));
                        if self.firmware {
                            return self.checked_call(call, Kind::Int, &["ValueError".to_string()], span);
                        }
                        Val::from(call, Kind::Int, &parts).failing()
                    }
                    _ => return Err(cannot("`int` of anything but a number or a str", span)),
                }
            }
            "float" => {
                let val = one()?;
                match &val.kind {
                    Kind::Int | Kind::Float => Val::from(val.double(), Kind::Float, &parts),
                    Kind::Str => {
                        self.built = !self.collector;
                        let call = format!("zeno_parse_float({}, {})", val.code, at(span));
                        if self.firmware {
                            return self.checked_call(call, Kind::Float, &["ValueError".to_string()], span);
                        }
                        Val::from(call, Kind::Float, &parts).failing()
                    }
                    _ => return Err(cannot("`float` of anything but a number or a str", span)),
                }
            }
            // `sum`, `min` and `max` of one list of numbers.
            "sum" | "min" | "max" if matches!(vals.as_slice(), [Val { kind: Kind::List(_), .. }]) => {
                let list = &vals[0];
                let Kind::List(item) = &list.kind else { unreachable!() };
                let code = match (name, &**item) {
                    ("sum", Kind::Int) => format!("zeno_list_sum_int({}, {})", list.code, at(span)),
                    ("sum", Kind::Float) => format!("zeno_list_sum_float({})", list.code),
                    (_, Kind::Int | Kind::Float) => format!(
                        "zeno_list_best_{}({}, {}, {}, {})",
                        key(item),
                        list.code,
                        name == "min",
                        c_string(name),
                        at(span)
                    ),
                    _ => return Err(cannot(format!("`{name}` of a list of anything but ints or floats"), span)),
                };
                let mut val = Val::from(code, (**item).clone(), &parts).failing();
                val.reads = true;
                val
            }
            "min" | "max" => {
                let kind = match vals.first() {
                    Some(first) if vals.len() >= 2 && vals.iter().all(|val| val.kind == first.kind) => {
                        first.kind.clone()
                    }
                    _ => return Err(cannot(format!("`{name}` of values that are not all one type"), span)),
                };
                let helper = match kind {
                    Kind::Int => format!("zeno_{name}"),
                    Kind::Float => format!("zeno_{name}_float"),
                    _ => return Err(cannot(format!("`{name}` of anything but numbers"), span)),
                };
                let mut code = vals[0].code.clone();
                for val in &vals[1..] {
                    code = format!("{helper}({code}, {})", val.code);
                }
                Val::from(code, kind, &parts)
            }
            _ => return Err(cannot(format!("`{name}` used this way"), span)),
        })
    }

    // ------------------------------------------------------------ f-strings

    /// `f"area = {area:.2f}"` becomes one call that joins the pieces.
    fn format_string(&mut self, parts: &[FormatPart], span: Span) -> Result<Val, Error> {
        // Each `{value}` is turned into text as soon as it is worked out,
        // before the next one runs. So the pieces of text are what must be
        // kept in order here, not just the values.
        let mut pieces: Vec<Val> = Vec::new();
        for part in parts {
            match part {
                FormatPart::Text(literal) => pieces.push(Val::plain(c_string(literal), Kind::Str)),
                FormatPart::Value { value, format } => {
                    let mark = self.lines.len();
                    let val = self.expr(value)?;
                    let plain = self.text(&val, span)?;
                    let code = match format {
                        Some(spec) => formatted(&val, spec, plain, span)?,
                        None => plain,
                    };
                    let mut piece = Val::from(code, Kind::Str, &[&val]);
                    // Showing a list or an object looks inside it, and a
                    // later call could change what is there.
                    piece.reads |= matches!(val.kind, Kind::List(_) | Kind::Class(_));
                    self.settle(&mut pieces, &piece, mark, span)?;
                    pieces.push(piece);
                }
            }
        }
        let used: Vec<&Val> = pieces.iter().collect();
        let code = match pieces.as_slice() {
            [] => "\"\"".to_string(),
            [only] => only.code.clone(),
            _ => {
                let codes: Vec<&str> = pieces.iter().map(|piece| piece.code.as_str()).collect();
                format!("zeno_join({}, {})", pieces.len(), codes.join(", "))
            }
        };
        Ok(Val::from(code, Kind::Str, &used))
    }
}

/// `(a < b)` without its outer brackets, for places such as `if (...)` that
/// supply their own.
fn bare(code: &str) -> &str {
    let Some(inner) = code.strip_prefix('(').and_then(|rest| rest.strip_suffix(')')) else {
        return code;
    };
    // Only if the first bracket is the one that closes at the very end:
    // `(a) && (b)` must be left alone.
    let mut depth = 0;
    for byte in inner.bytes() {
        match byte {
            b'(' => depth += 1,
            b')' if depth == 0 => return code,
            b')' => depth -= 1,
            // Brackets inside a string do not count; leave such code as it is.
            b'"' | b'\'' => return code,
            _ => {}
        }
    }
    inner
}

/// Is this expression a name or a literal: something that cannot fail, does
/// nothing, and may safely be mentioned twice?
fn is_plain(expr: &Expr) -> bool {
    matches!(expr.kind, ExprKind::Name(_)) || is_literal(expr)
}

/// Somewhere a value can be stored that has a name: `point`, `self.total`,
/// `a.b.c`. Mentioning it does nothing, so it is safe to mention again.
fn is_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Name(_) => true,
        ExprKind::Field { object, .. } => is_place(object),
        _ => false,
    }
}

pub fn is_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::None => true,
        // A new empty list for each call, as `zeno run` makes one.
        ExprKind::List(items) => items.is_empty(),
        ExprKind::Unit { value, .. } | ExprKind::Unary { op: UnaryOp::Neg, operand: value } => {
            matches!(value.kind, ExprKind::Int(_) | ExprKind::Float(_))
        }
        _ => false,
    }
}

/// Is this piece of C a plain name for a value, safe to mention twice?
/// `z_f`, `env->z_f`, `(*z_f)` and `(&zeno_value_f)` are; a call is not.
fn is_c_place(code: &str) -> bool {
    let inner = code
        .strip_prefix("(*")
        .or_else(|| code.strip_prefix("(&"))
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(code);
    !inner.is_empty() && inner.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'>' | b'.'))
}

/// The built-in kinds of error. An object of any class can be raised too.
pub const ERROR_KINDS: &[&str] = &["Error", "ValueError", "IndexError", "ZeroDivisionError", "OverflowError"];

/// Find the variables that are given a new value inside a `try` block.
///
/// `try` is built on C's `setjmp` and `longjmp`, and C has a rule about
/// them: after the jump back, a local variable that was changed since the
/// `setjmp` may hold either its old value or its new one, unless it was
/// declared `volatile`. (The C compiler may have been keeping the new value
/// in a processor register, and the jump puts the registers back.) So the
/// variables found here are declared `volatile`, which makes C keep them in
/// memory, where the jump cannot touch them.
///
/// Only `var` variables of the function itself matter. Objects and lists
/// live on the heap, which the jump leaves alone.
fn changed_in_try(statement: &Stmt, inside: bool, names: &mut HashSet<String>) {
    let mut each = |block: &[Stmt], inside: bool| {
        for statement in block {
            changed_in_try(statement, inside, names);
        }
    };
    match &statement.kind {
        StmtKind::Assign { target: Expr { kind: ExprKind::Name(name), .. }, .. } if inside => {
            names.insert(name.clone());
        }
        StmtKind::If { then_block, else_block, .. } => {
            each(then_block, inside);
            each(else_block.as_deref().unwrap_or_default(), inside);
        }
        StmtKind::While { body, .. } | StmtKind::For { body, .. } => each(body, inside),
        StmtKind::Match { arms, .. } => arms.iter().for_each(|arm| each(&arm.body, inside)),
        StmtKind::Try { body, handlers } => {
            each(body, true);
            handlers.iter().for_each(|handler| each(&handler.body, inside));
        }
        _ => {}
    }
}

/// Find the names that functions written inside this statement mention.
/// `inside` is whether the walk has already entered such a function.
///
/// A `var` whose name turns up here may be changed by the inner function
/// or by the outer one, and each must see the other's change. See the
/// `boxed` field of `Variable`.
pub fn names_in_functions(statement: &Stmt, inside: bool, names: &mut HashSet<String>) {
    let block = |block: &[Stmt], inside: bool, names: &mut HashSet<String>| {
        for statement in block {
            names_in_functions(statement, inside, names);
        }
    };
    match &statement.kind {
        StmtKind::Let { value, .. } | StmtKind::Const { value, .. } => names_in_expr(value, inside, names),
        StmtKind::Assign { target, value, .. } => {
            names_in_expr(target, inside, names);
            names_in_expr(value, inside, names);
        }
        StmtKind::Expr(expr) | StmtKind::Raise(expr) | StmtKind::Return(Some(expr)) => names_in_expr(expr, inside, names),
        StmtKind::Function(decl) => block(&decl.body, true, names),
        StmtKind::Class(class) => class.methods.iter().for_each(|method| block(&method.body, true, names)),
        StmtKind::If { condition, then_block, else_block } => {
            names_in_expr(condition, inside, names);
            block(then_block, inside, names);
            block(else_block.as_deref().unwrap_or_default(), inside, names);
        }
        StmtKind::While { condition, body } => {
            names_in_expr(condition, inside, names);
            block(body, inside, names);
        }
        StmtKind::For { iterable, body, .. } => {
            names_in_expr(iterable, inside, names);
            block(body, inside, names);
        }
        StmtKind::Match { subject, arms } => {
            names_in_expr(subject, inside, names);
            arms.iter().for_each(|arm| block(&arm.body, inside, names));
        }
        StmtKind::Try { body, handlers } => {
            block(body, inside, names);
            handlers.iter().for_each(|handler| block(&handler.body, inside, names));
        }
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Enum(_) | StmtKind::Import { .. } => {}
    }
}

fn names_in_expr(expr: &Expr, inside: bool, names: &mut HashSet<String>) {
    let mut each = |expr: &Expr| names_in_expr(expr, inside, names);
    match &expr.kind {
        ExprKind::Name(name) if inside => {
            names.insert(name.clone());
        }
        ExprKind::Name(_) | ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::None => {}
        ExprKind::Unit { value: inner, .. } | ExprKind::Unary { operand: inner, .. } | ExprKind::Field { object: inner, .. } => {
            each(inner)
        }
        ExprKind::Binary { left, right, .. } | ExprKind::Index { object: left, index: right } => {
            each(left);
            each(right);
        }
        ExprKind::Compare { first, rest } => {
            each(first);
            rest.iter().for_each(|(_, expr)| each(expr));
        }
        ExprKind::FormatStr(parts) => {
            for part in parts {
                if let FormatPart::Value { value, .. } = part {
                    each(value);
                }
            }
        }
        ExprKind::Call { callee, args } => {
            each(callee);
            args.iter().for_each(|arg| each(&arg.value));
        }
        ExprKind::List(items) => items.iter().for_each(each),
        ExprKind::ListComp { element, iterable, condition, .. } => {
            each(element);
            each(iterable);
            if let Some(condition) = condition {
                each(condition);
            }
        }
        ExprKind::Function(decl) => {
            for statement in &decl.body {
                names_in_functions(statement, true, names);
            }
        }
    }
}

/// C for a quantity after it has been scaled to another unit.
fn scaled(code: &str, scale: units::Scale) -> String {
    if scale.times != 1.0 {
        format!("({code} * {:?})", scale.times)
    } else if scale.over != 1.0 {
        format!("({code} / {:?})", scale.over)
    } else {
        code.to_string()
    }
}

/// Two quantities, both scaled to the unit they share.
fn in_common(left: &Val, a: &str, right: &Val, b: &str, span: Span) -> Result<(String, String, String), Error> {
    match units::common(a, b) {
        Some((unit, l, r)) => Ok((unit.to_string(), scaled(&left.code, l), scaled(&right.code, r))),
        None => Err(cannot(format!("a quantity in {a} with a quantity in {b}"), span)),
    }
}

/// `== != < <= > >=` as a C expression.
fn comparison(op: BinaryOp, left: &Val, right: &Val, span: Span) -> Result<String, Error> {
    let symbol = op.text();
    Ok(match (&left.kind, &right.kind) {
        // C compares strings by where they are in memory. `strcmp` compares
        // what they say.
        (Kind::Str, Kind::Str) => format!("(strcmp({}, {}) {symbol} 0)", left.code, right.code),
        (Kind::Int, Kind::Int) | (Kind::Float, Kind::Float) => {
            format!("({} {symbol} {})", left.code, right.code)
        }
        (Kind::Quantity(a), Kind::Quantity(b)) => {
            let (_, l, r) = in_common(left, a, right, b, span)?;
            format!("({l} {symbol} {r})")
        }
        (Kind::Bool, Kind::Bool) if matches!(op, BinaryOp::Eq | BinaryOp::NotEq) => {
            format!("({} {symbol} {})", left.code, right.code)
        }
        _ if left.is_number() && right.is_number() => format!("({} {symbol} {})", left.double(), right.double()),
        _ => return Err(cannot(format!("`{symbol}` between these two types"), span)),
    })
}

/// The format after the colon in `{x:>8.2f}`, taken apart: line up to the
/// right, eight wide, two decimals, as a float.
pub struct Format {
    /// `<`, `>` or `^`, if one was written.
    pub align: Option<char>,
    /// Fill with zeros instead of spaces.
    pub zero: bool,
    /// How wide to make it. 0 means as wide as it is.
    pub width: usize,
    /// The digits after the `.`, if any.
    pub precision: Option<usize>,
    /// The letter at the end: `f`, `d`, `x`, `X` or `b`.
    pub kind: Option<char>,
}

/// Read a format. `None` means it is not one Zeno understands.
///
/// It is read here, while compiling, so the compiled program does not
/// have to read it again every time the line runs. The rules are the
/// ones `format` in `builtins.rs` follows. The LLVM backend reads formats
/// with this too.
pub fn read_format(spec: &str) -> Option<Format> {
    let mut rest = spec.chars().peekable();

    let align = rest.next_if(|c| matches!(c, '<' | '>' | '^'));
    let zero = rest.next_if_eq(&'0').is_some();
    let mut width = 0;
    while let Some(digit) = rest.peek().and_then(|c| c.to_digit(10)) {
        width = width * 10 + digit as usize;
        rest.next();
    }
    let mut precision = None;
    if rest.next_if_eq(&'.').is_some() {
        let mut digits = 0;
        while let Some(digit) = rest.peek().and_then(|c| c.to_digit(10)) {
            digits = digits * 10 + digit as usize;
            rest.next();
        }
        precision = Some(digits);
    }
    let kind = rest.next();
    if rest.next().is_some() {
        return None;
    }
    Some(Format { align, zero, width, precision, kind })
}

/// A value with a format after the colon, as in `{x:.2f}` or `{n:08b}`.
fn formatted(val: &Val, spec: &str, plain: String, span: Span) -> Result<String, Error> {
    let bad = || Error::new(format!("`{spec}` is not a format Zeno understands"), span);
    let Format { align, zero, width, precision, kind } = read_format(spec).ok_or_else(bad)?;

    let is_number = val.is_number();
    let body = match (kind, precision, &val.kind) {
        (Some('f'), _, _) | (None, Some(_), Kind::Float) if is_number => {
            format!("zeno_fixed({}, {})", val.double(), precision.unwrap_or(6))
        }
        (None, None, _) | (Some('d'), None, Kind::Int) => plain,
        (Some(kind @ ('x' | 'X' | 'b')), None, Kind::Int) => format!("zeno_radix({}, '{kind}')", val.code),
        _ => return Err(bad()),
    };
    if width == 0 {
        return Ok(body);
    }
    // Numbers line up on the right by default, everything else on the left.
    let align = align.unwrap_or(if is_number { '>' } else { '<' });
    Ok(format!("zeno_pad({body}, {width}, '{align}', {})", zero && is_number))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn compile(source: &str) -> Result<String, String> {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        emit(&program, &types, "test.zeno", Build::Ordinary).map_err(|error| error.message)
    }

    /// The same, as `zeno build --firmware` compiles it.
    fn compile_firmware(source: &str) -> Result<String, String> {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        emit(&program, &types, "test.zeno", Build::Firmware).map_err(|error| error.message)
    }

    /// Build the C with the machine's C compiler, run it, and return what it
    /// printed, what it wrote as errors, and whether it finished cleanly.
    /// `None` if there is no C compiler here to try it with.
    fn build_and_run(name: &str, c: &str, flags: &[&str]) -> Option<(String, String, bool)> {
        let dir = std::env::temp_dir().join(format!("zeno-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let (c_file, binary) = (dir.join("test.c"), dir.join("test"));
        std::fs::write(&c_file, c).expect("the C file is written");
        let Ok(built) = Command::new("cc").args(flags).arg("-o").arg(&binary).arg(&c_file).arg("-lm").output() else {
            eprintln!("no C compiler found; skipping the compiled half of this test");
            return None;
        };
        let complaints = String::from_utf8_lossy(&built.stderr);
        let complaints: String = complaints.chars().take(1500).collect();
        assert!(built.status.success(), "the C does not compile:\n{complaints}");
        let ran = Command::new(&binary).output().expect("the program runs");
        let _ = std::fs::remove_dir_all(&dir);
        Some((
            String::from_utf8_lossy(&ran.stdout).into_owned(),
            String::from_utf8_lossy(&ran.stderr).into_owned(),
            ran.status.success(),
        ))
    }

    /// The heart of these tests: the compiled program must print exactly
    /// what the interpreter prints.
    fn same_as_interpreter(name: &str, source: &str) {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (interpreted, result) = crate::vm::run_both(&program, source);

        // Both kinds of build must agree with the interpreter: the
        // ordinary one, and `--firmware`, where errors travel as values.
        for firmware in [false, true] {
            let c = if firmware {
                match compile_firmware(source) {
                    Ok(c) => c,
                    // A firmware build refuses a few things on purpose,
                    // and says so. Any other refusal is a failure.
                    Err(message) => {
                        assert!(message.contains("--firmware"), "the firmware build failed: {message}\n{source}");
                        continue;
                    }
                }
            } else {
                compile(source).expect("the test program compiles")
            };
            let name = if firmware { format!("{name}-firmware") } else { name.to_string() };
            // ZENO_COLLECT_ALWAYS runs the garbage collector before every
            // single allocation. If the collector ever frees something that
            // is still in use, this is the setting most likely to show it.
            let Some((compiled, errors, finished)) = build_and_run(&name, &c, &["-DZENO_COLLECT_ALWAYS"]) else {
                return;
            };
            assert_eq!(compiled, interpreted, "\nfirmware: {firmware}\n{source}");
            match &result {
                Ok(()) => assert!(finished, "the compiled program failed (firmware: {firmware}): {errors}"),
                // Both must stop with the same error, at the same place.
                Err(error) => {
                    assert!(!finished, "the compiled program should have failed");
                    let expected =
                        format!("error: {}\n --> test.zeno:{}:{}\n", error.message, error.span.line, error.span.col);
                    assert_eq!(errors, expected, "\nfirmware: {firmware}\n{source}");
                }
            }
        }
        // The LLVM backend is held to every program the C backend is.
        crate::emit_llvm::tests::same_as_interpreter(&format!("{name}-llvm"), source);
    }

    #[test]
    fn arithmetic_matches() {
        same_as_interpreter(
            "arithmetic",
            "print(1 + 2 * 3, 7 // 2, -7 // 2, 7 % -3, -7 % 3, 2 ** 10, 7 / 2, 2 ** 0.5)\n\
             print(1.5 + 1, 3 * 0.5, 7.5 // 2, -7.5 % 2, 10 - 2.5, -(3 + 4), 0 ** 0, (-1) ** 7)\n\
             print(1 == 1.0, 2 < 3.5, \"a\" < \"b\", \"a\" == \"a\", not true, true and false, true or false)\n\
             print(abs(-3), abs(-2.5), min(3, 1, 2), max(1.5, 2.5), int(3.9), int(-3.9), float(2))",
        );
    }

    #[test]
    fn floats_print_the_same() {
        same_as_interpreter(
            "floats",
            "print(0.1 + 0.2, 100.0, 1e16, 1.5e-7, 0.0001, 123456.789, 1e15, 5e-324, 1.7976931348623157e308)\n\
             print(3.0, -0.5, 1 / 3, 2.0 ** 70, 12345678912345678.0, 0.1 * 3, -0.0)",
        );
    }

    #[test]
    fn strings_and_formats_match() {
        same_as_interpreter(
            "strings",
            "let name = \"Zeno\"\nlet pi = 3.14159\n\
             print(f\"Hello, {name}!\", name + \"!\", len(\"h\u{e9}llo\"), str(42) + str(1.5) + str(true))\n\
             print(f\"{3:5}|{pi:.2f}|{255:x}|{255:X}|{5:08b}|{name:>6}|{name:^8}|{2.5:8.1f}|{7:<4}|{-3:05}|{pi:10}|\")\n\
             print(f\"{2:.3f} {name:2} {-255:x} {1.0} {none_yet()}\")\n\
             function none_yet() -> str { return \"tab\\there \\\"quoted\\\" ??= done\" }",
        );
    }

    #[test]
    fn control_flow_matches() {
        same_as_interpreter(
            "control",
            "var total = 0\nfor i in range(10) {\n if i % 2 == 0 { continue }\n if i > 7 { break }\n total += i\n}\nprint(total)\n\
             for i in range(10, 0, -3) { print(i) }\nfor i in range(2, 5) { print(i) }\nfor i in range(5, 2) { print(i) }\n\
             var n = 27\nvar steps = 0\nwhile n != 1 {\n if n % 2 == 0 { n = n // 2 } else { n = 3 * n + 1 }\n steps += 1\n}\nprint(steps)\n\
             let x = 5\nif x < 3 { print(\"small\") } else if x < 10 { print(\"medium\") } else { print(\"big\") }\n\
             print(0 <= x < 10, 0 < x < 3, 1 < 2 < 3 < 4)\n\
             if x > 0 {\n let x = x + 1\n print(x)\n}\nprint(x)",
        );
    }

    #[test]
    fn functions_match() {
        same_as_interpreter(
            "functions",
            "let base = 10\n\
             function add(a, b) { return a + b }\n\
             function fact(n) {\n if n <= 1 { return 1 }\n return n * fact(n - 1)\n}\n\
             function greet(who: str, excited: bool = false) -> str {\n if excited { return f\"Hello, {who}!\" }\n return f\"Hello, {who}\"\n}\n\
             function area(w, h) { return w * h }\n\
             function show(label: str) { print(label, base) }\n\
             function average(a, b) {\n if a == b { return float(a) }\n return (a + b) / 2\n}\n\
             function wait(d: ms) -> ms { return d * 2 }\n\
             print(add(1, 2), fact(10), greet(\"Zeno\"), greet(excited=true, who=\"you\"), area(2.5, 4.0))\n\
             show(\"base is\")\nprint(average(2, 2), average(1, 2), wait(250 ms), 3 s / 2, 1.5 V + 1 V)",
        );
    }

    #[test]
    fn units_convert_the_same_way() {
        same_as_interpreter(
            "units",
            "let t = 3 s\nlet d = 250 ms\n\
             print(t + d, d + t, t - d, t.ms, d.s, (90 min).h, (1 kWh).J, t.ms.us)\n\
             print(1 s == 1000 ms, 1 s != 1000 ms, t > d, 999 ms < 1 s, 1 h / 1 min, 2 * t + 0.5 * d)\n\
             function wait(d: ms) -> ms { return d }\nprint(wait(t.ms), wait((2 min).ms))",
        );
    }

    #[test]
    fn classes_match() {
        same_as_interpreter(
            "classes",
            "import math\n\
             class Point {\n x: float\n y: float\n\
              function length(self) -> float { return math.sqrt(self.x * self.x + self.y * self.y) }\n\
              function scaled(self, by: float) -> Point { return Point(self.x * by, self.y * by) }\n\
              function move(self, dx: float) { self.x += dx }\n}\n\
             class Counter {\n count = 0\n name: str = \"c\"\n\
              function bump(self) -> int {\n  self.count += 1\n  return self.count\n }\n}\n\
             class Line {\n start: Point\n end: Point\n}\n\
             let p = Point(3.0, 4.0)\nlet q = Point(y=4.0, x=3.0)\n\
             print(p, p.length(), p == q, p != q, p.scaled(2.0), p.x + q.y)\n\
             p.move(1.5)\nprint(p, p == q, Line(p, q), Line(p, q).end.y)\n\
             let c = Counter()\nlet d = c\n\
             print(c.bump(), d.bump(), c.count, c, Counter(5, \"fi\\\"ve\"))\n\
             c.count = 10\nprint(d.count, c.bump() + c.bump(), c.bump() - d.count, c)\n\
             print(f\"{c} {c.bump()} {c}\")\nprint(c, c.bump(), c)",
        );
    }

    #[test]
    fn lists_match() {
        same_as_interpreter(
            "lists",
            "let xs = [3, 1, 2]\nvar ys: list[int] = []\n\
             for x in xs { ys.append(x * 10) }\n\
             print(xs, ys, len(xs), xs[0] + ys[2], xs + ys, xs == [3, 1, 2], xs != ys, 2 in xs, 5 not in xs)\n\
             ys[1] = 99\nprint(ys, ys.pop(), ys)\nprint(f\"{ys} {ys.pop()} {ys}\")\n\
             let names = [\"a\", \"b c\"]\nlet grid = [[1, 2], [3]]\nlet halves = [0.5, 1.0]\n\
             print(names, grid, halves, [true, false], [250 ms, 500 ms], grid[1][0], \"b c\" in names, \"b\" in \"abc\")\n\
             let squares = [n * n for n in range(6) if n % 2 == 0]\n\
             let doubled = [x * 2 for x in xs]\nprint(squares, doubled, [len(name) for name in names])\n\
             var total = 0\nfor x in xs {\n xs.append(x)\n total += x\n}\nprint(total, xs)\n\
             function first(items: list[str]) -> str { return items[0] }\n\
             function build(n: int) -> list[int] {\n var out: list[int] = []\n for i in range(n) { out.append(i) }\n return out\n}\n\
             print(first(names), build(4), build(0), len(build(3)))",
        );
        same_as_interpreter("index", "let xs = [1, 2]\nlet i = 2\nprint(xs[i])");
        same_as_interpreter("negative-index", "let xs = [1, 2]\nlet i = -1\nxs[i] = 5");
        same_as_interpreter("pop", "var xs: list[int] = []\nprint(xs.pop())");
    }

    #[test]
    fn enums_and_match_match() {
        same_as_interpreter(
            "enums",
            "enum Shape {\n Circle(radius: float)\n Rect(width: float, height: float)\n Dot\n}\n\
             enum Light {\n Red\n Green\n}\n\
             function area(shape: Shape) -> float {\n match shape {\n\
               case Circle(r) { return 3.0 * r * r }\n\
               case Rect(w, h) { return w * h }\n\
               case Dot { return 0.0 }\n }\n}\n\
             function next(light: Light) -> Light {\n match light {\n  case Red { return Green }\n  case Green { return Red }\n }\n}\n\
             function name(n: int) -> str {\n match n {\n  case 0 { return \"zero\" }\n  case 1 { return \"one\" }\n  case other { return f\"many ({other})\" }\n }\n}\n\
             let shapes = [Circle(1.0), Rect(2.0, 3.0), Dot, Shape.Rect(1.0, 1.0), Shape.Dot]\n\
             for shape in shapes { print(shape, area(shape)) }\n\
             print(shapes, Circle(1.0) == Circle(1.0), Circle(1.0) == Dot, Dot != Dot, next(Red), next(next(Red)) == Red)\n\
             print(name(0), name(1), name(7))\n\
             match shapes[1] {\n case Rect(w, 3.0) { print(\"three high\", w) }\n case _ { print(\"no\") }\n}\n\
             match \"b\" {\n case \"a\" { print(1) }\n case \"b\" { print(2) }\n case _ { print(3) }\n}",
        );
    }

    #[test]
    fn things_happen_in_the_order_zeno_says() {
        // C does not promise an order for the pieces of `a() + b()` or of
        // `f(a(), b())`. Zeno does: left to right.
        same_as_interpreter(
            "order",
            "var count = 0\n\
             function next(label: int) -> int {\n print(label)\n count += 1\n return count\n}\n\
             function three(a: int, b: int, c: int) -> int { return a * 100 + b * 10 + c }\n\
             print(next(1) - next(2))\n\
             print(three(next(3), next(4), next(5)))\n\
             print(count + next(6), count)\n\
             print(f\"{next(7)} {next(8)}\")\n\
             print(next(9) > 100 and next(-1) > 0, next(10) > 0 or next(-1) > 0)\n\
             print(0 < next(11) < next(12) < 5, 100 < next(13) < next(-1))\n\
             var i = 0\nwhile next(14) < 20 and i < 2 { i += 1 }\n\
             if count > 100 { print(\"no\") } else if next(15) > 0 and next(16) > 0 { print(\"yes\") }",
        );
    }

    #[test]
    fn a_function_does_not_see_what_is_declared_below_it() {
        // `len` inside `size` is the built-in. The variable of the same
        // name further down the file is not in sight from there.
        same_as_interpreter(
            "below",
            "function size(items: list[int]) -> int { return len(items) }\n\
             print(size([1, 2]))\nlet len = 5\nprint(size([1, 2, 3]), len)",
        );
    }

    #[test]
    fn errors_stop_the_program_the_same_way() {
        same_as_interpreter("overflow", "var n = 9223372036854775807\nprint(\"before\")\nn += 1\nprint(n)");
        same_as_interpreter("zero", "let z = 0\nprint(1 // z)");
        same_as_interpreter("zero-float", "let z = 0.0\nprint(1 / z)");
        same_as_interpreter("power", "let p = -1\nprint(2 ** p)");
        same_as_interpreter("big-power", "print(3 ** 50)");
        same_as_interpreter("root", "import math\nprint(math.sqrt(-4.0), math.pi)");
        same_as_interpreter("step", "let s = 0\nfor i in range(1, 5, s) { print(i) }");
        // Which of two errors is reported is also a matter of order.
        same_as_interpreter("first-error", "let z = 0\nlet big = 9223372036854775807\nprint((1 // z) + (big * 2))");
    }

    #[test]
    fn values_that_may_be_none_match() {
        same_as_interpreter("none", r##"function find(id: int) -> str or none {
    if id == 1 { return "one" }
}
print(find(1), find(2))
let a = find(1)
print(a == "one", a == none, a is none)
var n: int or none = 5
if n is not none {
    print(n + 1)
}
function get() -> int or none { return 3 }
let g = get()
if g is none { print("none") } else { print(g * 2) }
let names = ["a", none, "c"]
print(names, names[1] is none, names[0] == "a")
var best: int or none = none
for x in [3, 9, 4] {
    if best is none or x > best { best = x }
}
print(best, best == 9, best != none)
function half(n: int) -> float or none {
    if n % 2 == 0 { return n / 2 }
    return none
}
print(half(4), half(3), half(3) is none, half(4) == 2.0)
function flag(n: int) -> bool or none {
    if n > 0 { return true }
    if n < 0 { return false }
}
print(flag(1), flag(0), flag(-1))
let w: ms or none = 250 ms
print(w)
if w is not none { print(w + 1 s) }
"##);
        same_as_interpreter("none-in-objects", r##"function f(x: int or none) -> int {
    match x {
        case none { return 0 }
        case other { return 1 }
    }
}
print(f(none), f(1))
let e = ValueError("a")
let e2 = ValueError("a")
print(e == e2, e == e)
function show(u: str or none = none) { print(u) }
show()
show("x")
class Node { value: int
 next: Node or none = none }
let a = Node(1)
let b = Node(2, a)
print(b, a.next is none, b.next is not none)
let nx = b.next
if nx is not none { print(nx.value) }
print(f"{a.next} {nx}")
"##);
    }

    #[test]
    fn a_shared_variable_is_looked_at_again_after_a_check() {
        // `clear()` sets `n` back to none after the check. The compiled
        // program must stop, not carry on with a number that is not there.
        let source = "var n: int or none = 5\nfunction clear() { n = none }\nif n is not none {\n clear()\n print(n + 1)\n}";
        let c = compile(source).unwrap();
        assert!(c.contains("zeno_still_int(z_n, 5, 8)"));
        let Some((printed, errors, finished)) = build_and_run("gone", &c, &[]) else {
            return;
        };
        assert_eq!(printed, "");
        assert!(!finished);
        assert_eq!(
            errors,
            "error: this was checked with `is not none`, but it has been set to `none` since\n --> test.zeno:5:8\n"
        );
        // A variable nothing else can reach needs no second look.
        let c = compile("function f(n: int or none) -> int {\n if n is not none { return n + 1 }\n return 0\n}\nprint(f(1))").unwrap();
        assert!(c.contains("return zeno_add(z_n.value, 1, "));
    }

    #[test]
    fn errors_are_raised_and_caught_the_same_way() {
        same_as_interpreter("errors", r##"function parse_port(text: str) -> int raises ValueError {
    let port = int(text)
    if not 1 <= port <= 65535 {
        raise ValueError("port out of range")
    }
    return port
}
for text in ["8080", "0", "abc", " 1_000 ", "", "-5", "99999999999999999999", "+7", "_-5", "1__2", "-"] {
    try {
        print("port", parse_port(text))
    } except ValueError as err {
        print(f"error: {err}", err.kind, err.message == str(err))
    }
}
for text in ["2.5", " 1e3 ", "abc", "0x10", "inf", "-.5", "1.", "", "nan", "1e", "+3", "1_0"] {
    try { print(float(text)) } except ValueError as err { print(err) }
}
function depth(n: int) -> int {
    if n == 0 { return 1 // n }
    return depth(n - 1) + 1
}
function safe(n: int) -> int {
    var result = -1
    try {
        result = 10
        result = depth(n)
        result = 20
    } except ZeroDivisionError {
        result += 100
    }
    return result
}
print(safe(5))
function find(xs: list[int], want: int) -> int {
    var i = 0
    while i < 10 {
        try {
            if xs[i] == want { return i }
            if xs[i] < 0 { break }
            i += 1
            continue
        } except IndexError as e {
            print("ran off the end:", e)
            return -1
        }
    }
    return -2
}
print(find([5, 6, 7], 7), find([5, 6], 9), find([1, -1, 3], 3))
try {
    try {
        print([1, 2][5])
    } except ValueError {
        print("wrong one")
    }
} except IndexError as e {
    print("outer caught:", e)
}
try {
    try { raise ValueError("inner") } except ValueError as e { raise IndexError(f"from {e}") }
} except Error as e {
    print(e.kind, e)
}
var n = 0
try {
    n = 1
    print(9223372036854775807 + n)
    n = 2
} except OverflowError as e {
    print("overflow", n, e)
} except {
    print("other")
}
print(n)
let big = [1, 2, 3]
try { print(big.pop(), big.pop(), big.pop(), big.pop()) } except IndexError as e { print(e, big) }
print(parse_port("70000"))
"##);
        same_as_interpreter("error-values", r##"class Oops { code: int }
function f(n: int) -> int raises Oops, ValueError {
    if n == 0 { raise Oops(3) }
    if n == 1 { raise ValueError("one") }
    return n
}
try { print(f(0)) } except Oops as e { print("oops", e.code, e) }
try { print(f(1)) } except Error as e { print(e, e.kind, e.message) }
try { print(f(0)) } except Error as e { print(e) }
try { print(1 // 0) } except ZeroDivisionError as e { print(e.kind, e) }
try { print(f(1)) } except { print("any") }
let e = ValueError("made")
print(e, [e])
try { raise e } except ValueError as x { print(x == e) }
print(f(0))
"##);
        same_as_interpreter("raised-objects", r##"class Timeout {
    after: ms
    tries: int = 1
}
function wait(n: int) raises Timeout, ValueError {
    if n > 2 { raise Timeout(n * 100 ms, n) }
    if n < 0 { raise ValueError("negative") }
    print("waited", n)
}
for n in [1, 3, -1] {
    try {
        wait(n)
    } except Timeout as t {
        print("timeout", t.after, t.tries, t)
        t.tries = 0
    } except Error as e {
        print("error", e.kind, e.message, e)
    }
}
try { wait(5) } except Error as e { print(e.kind, e.message, e == e) }
try {
    try { wait(9) } except Error as e {
        print("passing on", e.kind)
        raise e
    }
} except Timeout as t {
    print("got it back", t.after)
}
let s = "  Hello, Wörld \t"
print(s.upper(), s.lower(), f"[{s.strip()}]", s.split(), "a,b,,c".split(","), "abc".replace("b", "--"), "abc".replace("", "."))
print("hello".starts_with("he"), "hello".ends_with("lo"), "hello".starts_with("lo"), "é".upper(), "x".replace("", "é"))
print(sum([1, 2, 3]), sum([0.5, 0.25]), min([3, 1, 2]), max([1.5, 0.5]), len("a b".split()))
try { print("a b".split("")) } except ValueError as e { print(e) }
let empty: list[int] = []
try { print(min(empty)) } except ValueError as e { print(e) }
print(sum(empty))
wait(7)
"##);
    }

    #[test]
    fn functions_as_values_match() {
        same_as_interpreter("closures", r##"function twice(f: function(int) -> int, x: int) -> int { return f(f(x)) }
print(twice(function(n) { return n * 3 }, 2))
function make(k: int) { return function(n: int) { return n * k } }
let triple = make(3)
print(triple(5), make(2)(4))
function counter() {
    var count = 0
    return function() {
        count += 1
        return count
    }
}
let c = counter()
let d = counter()
print(c(), c(), c(), d())
let fs: list[function(int) -> int] = [triple, make(10)]
print(fs[1](2), fs[0](2))
function apply(f: function(int) -> int, x: int) -> int { return f(x) }
function double(n: int) -> int { return n * 2 }
print(apply(triple, 1), apply(double, 21), twice(double, 1))
print(triple, double, [double])
let xs = [1, 2, 3, 4]
print(xs.map(double), xs.filter(function(n) { return n % 2 == 0 }), xs.reduce(function(a, b) { return a + b }, 0))
print(xs.map(function(n) { return f"<{n}>" }), xs.map(function(n) { return n / 2 }))
function both() {
    var total = 0
    let add = function(n: int) { total += n }
    add(5)
    total += 1
    add(10)
    return total
}
print(both())
class Button {
    label: str
    on_press: function(str) -> str
}
let b = Button("ok", function(s) { return s + "!" })
print(b.on_press(b.label))
function compose(f: function(int) -> int, g: function(int) -> int) {
    return function(x: int) { return g(f(x)) }
}
print(compose(double, triple)(1))
function adders() {
    var out: list[function(int) -> int] = []
    for i in range(3) {
        out.append(function(n: int) { return n + i })
    }
    return out
}
for f in adders() { print(f(10)) }
"##);
        same_as_interpreter("closures-and-try", r##"var n: int or none = 5
if n is not none {
    n += 1
    print(n)
}
function bump(x: int or none) -> int or none {
    var y = x
    if y is not none {
        y += 10
        y = y * 2
    }
    return y
}
print(bump(1), bump(none))
function outer(a: int) -> function(int) -> function(int) -> int {
    return function(b: int) {
        return function(c: int) { return a * 100 + b * 10 + c }
    }
}
print(outer(1)(2)(3))
class Acc {
    total: int = 0
    function adder(self) -> function(int) -> int {
        return function(n: int) {
            self.total += n
            return self.total
        }
    }
}
let acc = Acc()
let add = acc.adder()
print(add(5), add(6), acc)
function careful(text: str or none) -> function() -> str {
    if text is none { return function() { return "nothing" } }
    return function() { return text.upper() }
}
print(careful(none)(), careful("abc")())
function tries(xs: list[int]) -> list[str] {
    var log: list[str] = []
    let attempt = function(i: int) -> int {
        try {
            return xs[i]
        } except IndexError {
            log.append(f"miss {i}")
            return -1
        }
    }
    var hits = 0
    for i in range(4) {
        try {
            if attempt(i) < 0 { continue }
            hits += 1
            if hits == 2 { break }
        } except Error {
            log.append("never")
        }
    }
    log.append(f"hits {hits}")
    return log
}
print(tries([7, 8, 9]), tries([7]))
for round in range(2) {
    var count = 0
    let step = function() { count += round + 1 }
    try {
        step()
        count += 10
        print(1 // round)
        count += 100
    } except ZeroDivisionError {
        count += 1000
    }
    step()
    print(round, count)
}
function loop_out() -> int {
    var i = 0
    while true {
        try {
            try {
                i += 1
                if i == 3 { break }
                if i == 1 { continue }
                print([0][i])
            } except ValueError { print("no") }
        } except IndexError as e {
            print("caught", i, e)
        }
    }
    try { print([1][0]) } except IndexError { print("no") }
    return i
}
print(loop_out())
print(1 // 0)
"##);
    }

    #[test]
    fn a_new_value_changes_what_is_known_about_none() {
        // Walking a linked list, filling in a default, keeping the last
        // match: each gives the variable a new value after a check.
        same_as_interpreter("walk", r##"class Node {
    value: int
    next: Node or none = none
}
function total(head: Node or none) -> int {
    var node = head
    var sum = 0
    while node is not none {
        sum += node.value
        node = node.next
    }
    return sum
}
let list = Node(1, Node(2, Node(3)))
print(total(list), total(none))
function name_of(given: str or none) -> str {
    var name = given
    if name is none {
        name = "anon"
    }
    return name
}
print(name_of(none), name_of("zeno"))
function last_even(xs: list[int]) -> int or none {
    var found: int or none = none
    for x in xs {
        if x % 2 == 0 { found = x }
    }
    return found
}
print(last_even([1, 2, 3, 4, 5]), last_even([1, 3]))
var top: int or none = none
top = 5
print(top + 1)
function longest(words: list[str]) -> str or none {
    var best: str or none = none
    for word in words {
        if best is none or len(word) > len(best) {
            best = word
        }
    }
    return best
}
print(longest(["a", "ccc", "bb"]), longest([]))
"##);
    }

    #[test]
    fn an_empty_list_is_a_list_of_whatever_it_arrives_at() {
        same_as_interpreter("empty", r##"class Bag {
    items: list[int] = []
    name: str or none = none
}
function count(xs: list[str]) -> int { return len(xs) }
function maybe(flag: bool) -> list[int] or none {
    if flag { return [] }
    return none
}
let b = Bag()
b.items.append(4)
print(b, count([]), maybe(true), maybe(false))
var grid: list[list[int]] = [[], [1, 2]]
grid.append([])
grid[1] = []
print(grid, len([]))
var xs: list[int] = [1]
xs = []
print(xs, [] == xs)
let c = Bag()
c.items.append(9)
print(b.items, c.items)
function push(x: int, into: list[int] = []) -> list[int] {
    into.append(x)
    return into
}
print(push(1), push(2), push(3, [0]))
"##);
        let message = compile("print([])").err().unwrap_or_else(|| "it compiled".to_string());
        assert!(message.contains("cannot compile a list of these yet"), "{message}");
    }

    #[test]
    fn the_new_pieces_work_together() {
        same_as_interpreter("optional-fields", r##"class Node {
    value: int
    weight: float or none = none
    next: Node or none = none
}
let a = Node(1, 2.5)
let b = Node(1, 2.5)
let c = Node(1, none, a)
print(a == b, a == c, c == Node(1, none, b), c, [a.weight, c.weight] == [2.5, none] )
"##);
        same_as_interpreter("together", r##"function countdown(start: int or none) -> int {
    var x = start
    var steps = 0
    while x is not none and x > 0 {
        x = x - 1
        steps += 1
    }
    return steps
}
print(countdown(3), countdown(none))
let fs = [function() { return n * n } for n in range(4)]
print([f() for f in fs])
enum Reading {
    Value(v: float)
    Fault(code: int)
}
function read(i: int) -> Reading or none {
    if i == 0 { return Value(1.5) }
    if i == 1 { return Fault(7) }
    return none
}
for i in range(3) {
    let r = read(i)
    var seen = 0
    try {
        if r is none { raise ValueError("nothing to read") }
        seen = 1
        match r {
            case Value(v) { print("value", v) }
            case Fault(code) {
                seen = 2
                raise IndexError(f"fault {code}")
            }
        }
        seen = 3
    } except ValueError as e {
        print("value error:", e, seen)
    } except IndexError as e {
        print("index error:", e, seen)
    }
    print("seen", seen)
}
let gaps = [21.5, none, 22.0]
var total = 0.0
for g in gaps {
    if g is not none { total += g }
}
gaps[1] = 30.0
gaps.append(none)
print(gaps, total, gaps[1] == 30.0, gaps[3] is none, [1, none] == [1, none], [true, none], gaps == [21.5, 30.0, 22.0, none])
let words = "the quick  brown fox".split()
print(words.map(function(w) { return w.upper() }), words.filter(function(w) { return len(w) > 3 }).map(len_of))
function len_of(w: str) -> int { return len(w) }
print(words.reduce(function(acc: str, w: str) { return acc + w[0] }, ""))
let word = "héllo"
print(word[1], word[4], [c.upper() for c in word], len(word))
for ch in "añb" { print(ch) }
print(word[5])
"##);
    }

    #[test]
    fn the_examples_compile_and_match() {
        // `blink.zeno` needs a `board` module, which only exists on a board.
        for (name, source) in [
            ("hello", include_str!("../examples/hello.zeno")),
            ("tour", include_str!("../examples/tour.zeno")),
            ("styles", include_str!("../examples/styles.zeno")),
            ("primes", include_str!("../examples/primes.zeno")),
            ("shapes", include_str!("../examples/shapes.zeno")),
            ("sensor", include_str!("../examples/sensor.zeno")),
            ("memory", include_str!("../examples/memory.zeno")),
            ("buffer", include_str!("../examples/buffer.zeno")),
            ("signal", include_str!("../examples/signal.zeno")),
            ("packet", include_str!("../examples/packet.zeno")),
            ("generics", include_str!("../examples/generics.zeno")),
        ] {
            same_as_interpreter(name, source);
        }
    }

    // ---------------------------------------------- Errors as values (firmware)

    /// `same_as_interpreter`, for a program that a firmware build must
    /// accept, so that the firmware half of the comparison really runs.
    fn firmware_as_interpreter(name: &str, source: &str) {
        if let Err(message) = compile_firmware(source) {
            panic!("the firmware build refused this: {message}");
        }
        same_as_interpreter(name, source);
        // The LLVM backend must accept it for firmware too.
        crate::emit_llvm::tests::firmware_as_interpreter(&format!("{name}-llvm"), source);
    }

    #[test]
    fn a_firmware_build_has_no_setjmp() {
        let source = include_str!("../examples/sensor.zeno");
        let c = compile_firmware(source).unwrap();
        for word in ["setjmp", "longjmp", "jmp_buf", "ZENO_FIRMWARE"] {
            assert!(!c.contains(word), "`{word}` is in the C of a firmware build");
        }
        // A function that can raise says whether it worked, and gives its
        // answer through a pointer. Its caller tests what came back.
        assert!(c.contains("static bool z_parse_line(const char *z_line, zeno_opt_float *zeno_result) {"));
        assert!(c.contains("if (!zeno_parse_float(z_text, 19, 17, &t"));
        assert!(c.contains("if (!z_parse_line(z_line, &t"));
        assert!(c.contains(")) goto zeno_except_"));
        assert!(c.contains("if (zeno_is(zeno_caught_"));
        // A function that cannot raise is exactly as before.
        assert!(c.contains("static zeno_closure *z_in_range(double z_low, double z_high) {"));
        // And an ordinary build has none of this.
        let c = compile(source).unwrap();
        assert!(c.contains("setjmp") && !c.contains("zeno_except_") && !c.contains("ZENO_FIRMWARE"));
        assert!(c.contains("static zeno_opt_float z_parse_line(const char *z_line) {"));
    }

    #[test]
    fn errors_as_values_match() {
        // Errors the program raises, through functions, methods, loops,
        // nested `try` blocks, conditions and recursion, and one left
        // uncaught at the end.
        firmware_as_interpreter("values-a", r##"class Timeout {
    after: int
    tries: int
}
class Sensor {
    name: str
    offset: float
    function read(self, raw: str) -> float raises ValueError {
        let value = float(raw) + self.offset
        if value > 100.0 {
            raise ValueError(f"{self.name}: {value} is too hot")
        }
        return value
    }
    function check(self, raw: str) raises ValueError {
        self.read(raw)
    }
}
function parse_port(text: str) -> int raises ValueError {
    let port = int(text)
    if not 1 <= port <= 65535 {
        raise ValueError("port out of range")
    }
    return port
}
function note(what: str) -> int {
    print("  note", what)
    return 1
}
function first_port(texts: list[str]) -> int or none raises ValueError {
    for text in texts {
        if text == "skip" { continue }
        if text == "stop" { break }
        return parse_port(text)
    }
}
function wait(n: int) raises Timeout {
    if n > 3 {
        raise Timeout(n * 100, n)
    }
    print("  waited", n)
}
function depth(n: int) -> int raises ValueError {
    if n == 0 { raise ValueError("bottom") }
    return depth(n - 1) + 1
}
function safe(n: int) -> int {
    var result = -1
    try {
        result = 10
        result = depth(n)
        result = 20
    } except ValueError {
        result += 100
    }
    return result
}
function find(texts: list[str], want: int) -> int {
    var i = 0
    while i < len(texts) {
        try {
            if parse_port(texts[i]) == want { return i }
            if texts[i] == "9" { break }
            i += 1
            continue
        } except ValueError as e {
            print("  bad entry:", e)
            return -1
        }
    }
    return -2
}
function relay(n: int) raises Timeout, ValueError {
    try {
        wait(n)
        parse_port("x")
    } except ValueError as e {
        print("  relay saw", e)
        raise e
    }
}
function total(texts: list[str]) -> int raises ValueError {
    var sum = 0
    for text in texts {
        sum += parse_port(text) + note(text) * parse_port(text)
    }
    return sum
}

for text in ["8080", "0", "abc", " 1_000 ", "", "-5", "99999999999999999999"] {
    try {
        print("port", parse_port(text))
    } except ValueError as err {
        print(f"error: {err}", err.kind, err.message == str(err))
    }
}
for text in ["2.5", " 1e3 ", "abc", "0x10", ""] {
    try { print(float(text)) } except ValueError as err { print(err) }
}
print(safe(5), safe(0))
print(find(["5", "6", "7"], 7), find(["5", "x"], 9), find(["1", "9", "3"], 3), find(["1"], 3))
let s = Sensor("probe", 0.5)
try {
    print(s.read("20"))
    s.check("21")
    print(s.read("99.9"))
    print("not reached")
} except ValueError as e {
    print("caught:", e)
}
try {
    try {
        print(parse_port("7"), parse_port("70000"))
    } except Timeout {
        print("wrong one")
    }
} except ValueError as e {
    print("outer caught:", e)
}
try {
    try { raise ValueError("inner") } except ValueError as e { raise IndexError(f"from {e}") }
} except Error as e {
    print(e.kind, e)
}
for n in [2, 5] {
    try {
        relay(n)
    } except Timeout as t {
        print("timeout", t.after, t.tries, t)
    } except Error as e {
        print("other", e.kind, e)
    }
}
try {
    print(first_port(["skip", "80"]), first_port(["skip", "stop", "81"]), first_port([]))
    print(first_port(["skip", "zero"]))
} except {
    print("any")
}
var count = 0
try {
    if parse_port("1") == 1 and parse_port("2") == 2 or parse_port("bad") == 3 {
        count += 1
    }
    while parse_port(str(count + 1)) < 4 {
        count += 1
    }
    print(f"count {count} port {parse_port('44')} then {parse_port('nope')}")
} except ValueError as e {
    print("stopped at", count, e)
}
try {
    print(total(["1", "2"]))
    print(total(["3", "0", "5"]))
} except ValueError as e {
    print(e)
}
function chain(n: int) -> int raises ValueError {
    try {
        return parse_port(str(n))
    } except ValueError as e {
        if n < -1 {
            raise ValueError(f"gave up at {n}")
        }
        return chain(n - 1)
    }
}
try { print(chain(3), chain(0)) } except ValueError as e { print(e) }
let e = ValueError("kept")
try { raise e } except ValueError as x { print(x == e) }
print(parse_port("70000"))
print("never printed")
"##);
    }

    #[test]
    fn errors_as_values_match_in_methods_loops_and_function_values() {
        firmware_as_interpreter("values-b", r##"enum Reply {
    Ok(value: int)
    Missing
}
class Bad {
    why: str
}
class Parser {
    seen: int = 0
    function one(self, text: str) -> int raises ValueError {
        self.seen += 1
        return int(text)
    }
    function many(self, texts: list[str]) -> list[int] raises ValueError {
        return [self.one(text) for text in texts]
    }
    function reply(self, text: str) -> Reply raises Bad {
        if text == "" { return Missing }
        if text == "!" { raise Bad("bang") }
        try {
            return Ok(self.one(text))
        } except ValueError as e {
            raise Bad(f"not a number: {text}")
        }
    }
}
function describe(p: Parser, text: str) -> str raises Bad {
    match p.reply(text) {
        case Ok(value) { return f"ok {value}" }
        case Missing { return "missing" }
    }
}
function numbers(text: str) -> list[int] raises ValueError {
    var out: list[int] = []
    for piece in text.split(",") {
        out.append(int(piece))
    }
    return out
}
function first_good(texts: list[str]) -> int or none {
    for text in texts {
        try {
            return int(text)
        } except ValueError {
            continue
        }
    }
    return none
}
function count_good(texts: list[str]) -> int {
    var good = 0
    var i = 0
    while true {
        try {
            if i >= len(texts) { break }
            let text = texts[i]
            i += 1
            int(text)
            good += 1
        } except ValueError {
            try {
                float(texts[i - 1])
                good += 10
            } except ValueError {
                if texts[i - 1] == "end" { break }
            }
        }
    }
    return good
}
let p = Parser()
try {
    print(p.many(["1", "2", "3"]), p.seen)
    print(p.many(["4", "x", "6"]), p.seen)
} except ValueError as e {
    print("stopped:", e, p.seen)
}
for text in ["12", "", "zz", "!"] {
    try {
        print(describe(p, text))
    } except Bad as bad {
        print("bad:", bad.why, bad)
    }
}
try {
    for n in numbers("3,4,5") { print(n) }
    for n in numbers("6,seven") { print(n) }
} except ValueError as e { print(e) }
print(first_good(["a", "b", "7", "8"]), first_good(["a"]), first_good([]))
print(count_good(["1", "2.5", "x", "3", "end", "4"]))
let twice = function(text: str) -> int {
    try {
        return int(text) * 2
    } except ValueError {
        return -1
    }
}
print(twice("21"), twice("no"), ["1", "b", "3"].map(twice))
function boom() raises Bad {
    raise Bad("at the top")
}
print("before")
boom()
print("after")
"##);
    }

    #[test]
    fn what_a_firmware_build_refuses_says_so() {
        let refused = |source: &str| compile_firmware(source).err().unwrap_or_else(|| "it compiled".to_string());
        // A division by zero stops a firmware build. An `except` for it
        // would never run, so it is refused, not left to mislead.
        let message = refused("let zero = 0\ntry { print(1 // zero) } except ZeroDivisionError { print(\"no\") }");
        assert!(message.starts_with("nothing in this `try` can be caught. In a firmware build"), "{message}");
        assert!(message.ends_with("Test for it before it happens, or build without `--firmware`"), "{message}");
        let parse = "function parse(text: str) -> int raises ValueError { return int(text) }\n";
        let message = refused(&format!("{parse}try {{ print(parse(\"1\")) }} except IndexError {{ print(\"no\") }}"));
        assert!(message.starts_with("nothing in this `try` can reach `except IndexError`."), "{message}");
        // The type of a function value does not say whether it can raise.
        let as_value = "`zeno build --firmware` cannot compile a function that can raise, used as a value, yet. Call it by its name, or build without `--firmware`";
        assert_eq!(refused(&format!("{parse}print([\"1\"].map(parse))")), as_value);
        assert_eq!(
            refused("let f = function(text: str) -> int raises ValueError { return int(text) }\nprint(1)"),
            as_value
        );
        // All four are fine in an ordinary build.
        assert!(compile(&format!("{parse}print([\"1\"].map(parse))")).is_ok());
    }

    #[test]
    fn a_fault_stops_a_firmware_build_where_an_ordinary_one_catches_it() {
        // The one place the two kinds of build differ, on purpose. The
        // `except` here takes anything. An ordinary build catches the
        // division by zero with it. A firmware build catches the errors
        // the program raises, and stops at the division.
        let source = "function parse(text: str) -> int raises ValueError { return int(text) }\nlet zero = 0\ntry {\n print(parse(\"1\"))\n print(1 // zero)\n} except {\n print(\"caught\")\n}\ntry { print(parse(\"x\")) } except { print(\"caught too\") }";
        let Some((printed, _, finished)) = build_and_run("fault", &compile(source).unwrap(), &[]) else {
            return;
        };
        assert_eq!(printed, "1\ncaught\ncaught too\n");
        assert!(finished);
        let (printed, errors, finished) = build_and_run("fault-firmware", &compile_firmware(source).unwrap(), &[]).unwrap();
        assert_eq!(printed, "1\n");
        assert!(!finished);
        assert_eq!(errors, "error: uncaught ZeroDivisionError: division by zero\n --> test.zeno:5:8\n");
    }

    // ------------------------------------------------------------ Memory

    /// Build with the report switched on and a low first limit, so that
    /// the collector runs often, and at -O2, which is what `zeno build`
    /// uses and where the C compiler is most inventive about where it
    /// keeps a pointer. Gives back what the program printed and the three
    /// numbers of the report: bytes made, most in use at once, collections.
    fn memory_of(name: &str, source: &str) -> Option<(String, [usize; 3])> {
        let c = compile(source).expect("the test program compiles");
        let flags = ["-O2", "-DZENO_MEMORY_REPORT", "-DZENO_FIRST_COLLECTION=16384"];
        let (printed, errors, finished) = build_and_run(name, &c, &flags)?;
        assert!(finished, "the compiled program failed: {errors}");
        let report = errors.lines().find(|line| line.starts_with("memory:")).expect("a memory report");
        let numbers: Vec<usize> = report
            .split(|c: char| !c.is_ascii_digit())
            .filter(|word| !word.is_empty())
            .map(|word| word.parse().expect("a number"))
            .collect();
        Some((printed, [numbers[1], numbers[2], numbers[3]]))
    }

    fn interpreted(source: &str) -> String {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (printed, result) = crate::vm::run_both(&program, source);
        assert!(result.is_ok(), "the interpreter failed");
        printed
    }

    #[test]
    fn memory_that_is_no_longer_used_is_given_back() {
        // Every trip round the loop makes text, objects, a list, a ring of
        // objects that point at each other, and a function value, and
        // keeps almost none of it.
        let source = r##"class Point {
    x: int
    label: str
}
class Node {
    value: int
    next: Node or none = none
    back: Node or none = none
}
function ring(n: int) -> Node {
    let first = Node(0)
    var last = first
    for i in range(1, n) {
        let node = Node(i)
        last.next = node
        node.back = last
        last = node
    }
    last.next = first
    first.back = last
    return first
}
function adder(n: int) -> function(int) -> int {
    return function(x: int) -> int { return x + n }
}
var kept: list[str] = []
var total = 0
for i in range(20000) {
    let text = f"item {i} of many"
    let pair = [Point(i, text), Point(i + 1, text.upper())]
    let add = adder(i)
    total += len(text) + pair[1].x + ring(5).value + add(1) - i
    if i % 5000 == 0 {
        kept.append(text)
    }
}
print(total, kept)
"##;
        let Some((printed, [made, most, collections])) = memory_of("garbage", source) else {
            return;
        };
        assert_eq!(printed, interpreted(source));
        assert!(made > 4_000_000, "the program made {made} bytes");
        assert!(collections > 20, "the collector ran {collections} times");
        // Without a collector `most` would equal `made`.
        assert!(most < 200_000, "at most {most} bytes were in use at once, of {made} made");
    }

    #[test]
    fn memory_that_is_still_used_is_kept() {
        // The other half: things the program holds on to must come through
        // every collection untouched. A long chain (which also
        // shows the collector follows pointers without using the stack to
        // do it), a list in a top-level variable, a list reachable only
        // through an object, text in a list, and a function value that
        // remembers a variable.
        let source = r##"class Node {
    value: int
    next: Node or none = none
}
class Store {
    names: list[str]
    sizes: list[int or none]
}
function counter() {
    var count = 0
    return function() {
        count += 1
        return count
    }
}
function chain(n: int) -> Node or none {
    var head: Node or none = none
    for i in range(n) {
        head = Node(i, head)
    }
    return head
}
function churn(n: int) -> int {
    var seen = 0
    for i in range(n) {
        let scrap = [f"scrap {i}", f"{i * 2}"]
        seen += len(scrap[0]) + len(scrap)
    }
    return seen
}
let store = Store([], [])
let tick = counter()
let head = chain(30000)
var squares: list[int] = []
for i in range(2000) {
    squares.append(i * i)
    if i % 100 == 0 {
        store.names.append(f"name {i}")
        if i % 200 == 0 {
            store.sizes.append(none)
        } else {
            store.sizes.append(i)
        }
        tick()
    }
}
print(churn(30000))
var count = 0
var total = 0
var node = head
while node is not none {
    count += 1
    total += node.value
    node = node.next
}
print(count, total, len(squares), squares[1999], sum(squares))
print(store.names, store.sizes, tick())
"##;
        let Some((printed, [_, _, collections])) = memory_of("kept", source) else {
            return;
        };
        assert_eq!(printed, interpreted(source));
        // Fewer runs than in the test above, because the limit grows with
        // what is kept.
        assert!(collections > 5, "the collector ran {collections} times");
    }

    #[test]
    fn an_error_on_its_way_to_an_except_is_kept() {
        // Between `raise` and `except` the error is in no variable of the
        // program's, only in the runtime's own. The handlers here make
        // garbage before they look at what they caught.
        same_as_interpreter("raised", r##"class Fault {
    code: int
    notes: list[str]
}
function risky(i: int) -> int raises Fault {
    if i % 3 == 0 {
        raise Fault(i, [f"note {i}", f"again {i}"])
    }
    return i
}
var total = 0
var notes: list[str] = []
for i in range(30) {
    try {
        total += risky(i)
    } except Fault as fault {
        let filler = [f"filler {i}", f"more {i}"]
        total += fault.code + len(filler)
        notes.append(fault.notes[1])
    }
}
print(total, notes)
try {
    let n = int("not a number")
    print(n)
} except ValueError as err {
    let filler = [f"{total}", f"{len(notes)}"]
    print(err, filler)
}
"##);
    }

    // ----------------------------------------------------------- Owned values

    #[test]
    fn owned_values_match() {
        // Making, lending, giving away, replacing, and leaving a block
        // early with `continue`.
        same_as_interpreter(
            "owned",
            r##"owned class Packet {
    id: int
    data: owned list[int]

    function size(self) -> int {
        return len(self.data)
    }

    function add(self, value: int) {
        self.data.append(value)
    }
}

function total(xs: borrow list[int]) -> int {
    var sum_of = 0
    for x in xs {
        sum_of += x
    }
    return sum_of
}

function make(id: int, count: int) -> Packet {
    let data: owned list[int] = [n * id for n in range(count)]
    return Packet(id, data)
}

function send(p: Packet) -> int {
    print(f"sending {p.id} with {p.size()} items, total {total(p.data)}")
    return p.size()
}

function describe(p: borrow Packet) -> str {
    return f"packet {p.id}: {p.data}"
}

let first = make(1, 4)
first.add(99)
print(describe(first))
print(first)
let sent = send(first)
print(sent)

var buffer: owned list[int] = [1, 2, 3]
print(total(buffer), len(buffer), buffer[0])
buffer = [4, 5]
print(buffer)
let shared = [7, 8, 9]
print(total(shared))

for i in range(3) {
    let p = make(i + 1, 3)
    if i == 1 {
        print(send(p))
        continue
    }
    print(describe(p))
}
"##,
        );
    }

    #[test]
    fn owned_values_match_when_errors_and_early_exits_end_their_blocks() {
        // Owners in scope when a function raises, when a `try` catches,
        // and when `return`, `break` and `continue` leave from deep inside.
        same_as_interpreter(
            "owned-exits",
            r##"# Errors, early exits and owners.
owned class Frame {
    id: int
    bytes: owned list[int]

    function checksum(self) -> int {
        var total = 0
        for b in self.bytes {
            total = (total + b) % 256
        }
        return total
    }
}

owned class Link {
    name: str
    last: Frame
    sent: int
}

class Timeout {
    after: int
}

function build(id: int, size: int) -> Frame {
    return Frame(id, [(id * 7 + n) % 256 for n in range(size)])
}

function transmit(frame: Frame) -> int raises Timeout {
    if frame.id % 4 == 3 {
        raise Timeout(frame.id)
    }
    return frame.checksum()
}

function parse(text: str, into: borrow list[int]) raises ValueError {
    for part in text.split(",") {
        into.append(int(part))
    }
}

function longest(a: borrow list[int], b: borrow list[int]) -> int {
    if len(a) > len(b) {
        return len(a)
    }
    return len(b)
}

function first_big(limit: int) -> Frame {
    var n = 0
    while true {
        let candidate = build(n, n)
        if candidate.checksum() > limit {
            return candidate
        }
        n += 1
    }
}

function drain(frames: int) -> int {
    var good = 0
    for id in range(frames) {
        let frame = build(id, 5)
        if id % 5 == 4 {
            continue
        }
        try {
            let sum = transmit(frame)
            good += 1
            print(f"frame {id} ok, checksum {sum}")
        } except Timeout as t {
            print(f"frame {t.after} timed out")
        }
    }
    return good
}

print(drain(10))

let numbers: owned list[int] = []
try {
    parse("1,2,3", numbers)
    parse("4,five,6", numbers)
} except ValueError as e {
    print("bad:", e.message)
}
print(numbers, longest(numbers, [1, 2]))

let big = first_big(100)
print(big.id, big.checksum(), len(big.bytes))

var link = Link("uart0", build(1, 3), 0)
print(link)
link.last = build(2, 4)
link.sent += 1
print(link.last.checksum(), link)
link = Link("uart1", build(9, 2), 0)
print(link.name, link.last)

function swap_in(link: Link, frame: Frame) -> Link {
    link.last = frame
    link.sent += 1
    return link
}
let again = swap_in(link, build(5, 1))
print(again)

var kept: owned list[str] = ["a"]
for round in range(4) {
    let next: owned list[str] = [s + str(round) for s in kept]
    next.append("x")
    kept = next
}
print(kept)
"##,
        );
        // A value moved into a function that frees it and then raises;
        // owners in `match` cases, in a function value, in nested loops.
        same_as_interpreter(
            "owned-raises",
            r##"owned class Buf {
    tag: str
    data: owned list[int]
}

class Refused {
    why: str
}

enum Mode {
    Fast
    Slow(factor: int)
}

function fresh(tag: str, n: int) -> Buf {
    return Buf(tag, [i for i in range(n)])
}

# Takes the buffer, frees it itself, and then raises.
function swallow(b: Buf, fail: bool) -> int raises Refused {
    let size = len(b.data)
    if fail {
        let inner = b
        print("swallowed", inner.tag)
        raise Refused("too big: " + str(size))
    }
    return size
}

function attempt(n: int, fail: bool) -> str {
    let b = fresh("b" + str(n), n)
    let spare = fresh("spare", 2)
    var result = "none"
    try {
        let got = swallow(b, fail)
        result = "took " + str(got)
    } except Refused as r {
        result = r.why
    }
    return result + " / " + spare.tag
}

function takes_and_tries(b: Buf, fail: bool) -> int {
    try {
        return swallow(b, fail)
    } except Refused {
        return -1
    }
}

function regrow(b: Buf) -> Buf {
    b.data.append(len(b.data))
    return b
}

function by_mode(mode: Mode) -> int {
    match mode {
        case Fast {
            let b = fresh("fast", 3)
            return len(b.data)
        }
        case Slow(factor) {
            let b = fresh("slow", factor)
            if factor > 3 {
                return -len(b.data)
            }
            print("slow", b.tag)
        }
    }
    return 0
}

function counter() -> function() -> int {
    var count = 0
    return function() -> int {
        let scratch: owned list[int] = [count, count + 1]
        count += len(scratch)
        return count
    }
}

function search(limit: int) -> int {
    var found = -1
    for i in range(limit) {
        let outer = fresh("o", i)
        var j = 0
        while j < 3 {
            let inner = fresh("i", j)
            j += 1
            if len(inner.data) + len(outer.data) == 4 {
                found = i * 10 + j
                break
            }
            if j == 2 {
                continue
            }
            inner.data.append(1)
        }
        if found >= 0 {
            break
        }
    }
    return found
}

print(attempt(3, false))
print(attempt(4, true))
print(takes_and_tries(fresh("t", 5), false), takes_and_tries(fresh("t", 6), true))

var b = fresh("grow", 1)
for i in range(3) {
    b = regrow(b)
}
print(b)

print(by_mode(Fast), by_mode(Slow(2)), by_mode(Slow(5)))
let next = counter()
print(next(), next(), next())
print(search(6))

fresh("nobody", 3)
Buf("nobody either", [])

if len(b.data) > 2 {
    let local = fresh("local", 2)
    try {
        print(swallow(local, true))
    } except Refused as r {
        print("top level:", r.why)
    }
}

var count = 0
function bump() -> int {
    var count = 10
    let adder = function() -> int {
        count += 1
        return count
    }
    return adder()
}
var scratch: owned list[int] = [1]
scratch = [n for n in range(bump())]
print(len(scratch), count)
"##,
        );
    }

    #[test]
    fn an_owned_value_is_freed_without_the_collector() {
        // The same work twice. With shared lists the collector has to run
        // to get the memory back. With owned lists it never needs to.
        let work = |list_type: &str| {
            format!(
                "function work(rounds: int) -> int {{
    var total = 0
    for round in range(rounds) {{
        let samples: {list_type} = [n * round for n in range(500)]
        total += samples[499] % 7
    }}
    return total
}}
print(work(400))
"
            )
        };
        let Some((shared_printed, [_, shared_most, shared_collections])) = memory_of("shared-work", &work("list[int]"))
        else {
            return;
        };
        let Some((owned_printed, [made, owned_most, owned_collections])) =
            memory_of("owned-work", &work("owned list[int]"))
        else {
            return;
        };
        assert_eq!(owned_printed, shared_printed);
        assert_eq!(owned_printed, interpreted(&work("owned list[int]")));
        assert!(made > 2_000_000, "the program made {made} bytes");
        assert!(shared_collections > 20, "with shared lists the collector ran {shared_collections} times");
        assert_eq!(owned_collections, 0, "with owned lists the collector should have nothing to do");
        assert!(owned_most < 8_192, "at most {owned_most} bytes were in use at once");
        assert!(owned_most < shared_most);
    }

    #[test]
    fn the_c_for_an_owned_value() {
        let source = "owned class Frame {
    id: int
    bytes: owned list[int]
}
class Refused {
    id: int
}
function check(frame: borrow Frame) -> int raises Refused {
    if frame.id > 9 {
        raise Refused(frame.id)
    }
    return len(frame.bytes)
}
function send(frame: Frame) -> int raises Refused {
    let size = check(frame)
    return size + 1
}
function main() -> int raises Refused {
    let frame = Frame(1, [1, 2, 3])
    return send(frame)
}
";
        let c = compile(source).expect("the program compiles");
        // An object of an owned class frees what its fields own, then itself.
        assert!(c.contains("static void zeno_drop_Frame(z_Frame *self) {\n    if (self == NULL) return;\n    zeno_list_drop(self->z_bytes);\n    zeno_free(self);\n}"), "{c}");
        // Giving a value away takes the pointer and leaves NULL behind.
        assert!(c.contains(" = z_frame;\n    z_frame = NULL;\n"), "{c}");
        // The one that took it over frees it before it returns.
        assert!(c.contains("    zeno_drop_Frame(z_frame);\n    return t"), "{c}");
        // A borrowed value is an ordinary pointer, and nothing frees it.
        let check = c.split("static int64_t z_check(z_Frame *z_frame) {").nth(1).expect("the C for `check`");
        assert!(!check.split("\n}\n").next().expect("its body").contains("zeno_drop"), "{c}");

        // In a firmware build, a failed call frees on its way out too.
        let c = compile_firmware(source).expect("the program compiles for firmware");
        assert!(c.contains(")) { zeno_drop_Frame(z_frame); return false; }"), "{c}");
    }

    // ------------------------------------------------ A build with no collector

    /// The same, as `zeno build --no-collector` compiles it.
    fn compile_no_collector(source: &str) -> Result<String, String> {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        emit(&program, &types, "test.zeno", Build::NoCollector).map_err(|error| error.message)
    }

    /// A program that a build with no collector must accept. Built that
    /// way it must print what the interpreter prints, and it must end
    /// with every block it made given back: with no collector, a block
    /// that is not freed by some line of the program is never freed.
    /// Returns the most text that was in the text room at once.
    fn no_collector_as_interpreter(name: &str, source: &str) -> Option<usize> {
        let c = match compile_no_collector(source) {
            Ok(c) => c,
            Err(message) => panic!("the build with no collector refused this: {message}"),
        };
        // The point of the build: none of this is in the C file.
        for gone in ["zeno_collect", "zeno_roots", "zeno_stack_base", "zeno_block_count", "setjmp"] {
            assert!(!c.contains(gone), "`{gone}` is still in a build with no collector");
        }
        // Nor is the heap: lists, objects and errors come from the pool.
        assert_eq!(crate::emit_llvm::tests::asks_the_heap(&c), None, "a build with no collector uses the heap");
        let name = format!("{name}-no-collector");
        let (compiled, errors, finished) = build_and_run(&name, &c, &["-O1", "-DZENO_MEMORY_REPORT"])?;
        assert!(finished, "the compiled program failed: {errors}");
        assert_eq!(compiled, interpreted(source), "\n{source}");
        assert!(errors.contains("no collector, 0 blocks left at the end\n"), "blocks were never freed: {errors}");
        let text = errors.lines().find(|line| line.starts_with("text:")).expect("a line about the text room");
        let most = text.split(|c: char| !c.is_ascii_digit()).find(|word| !word.is_empty()).expect("a number");
        // A program with no shared values is still a Zeno program: the
        // two builds that do have a collector must agree as well.
        same_as_interpreter(&name, source);
        // And the build with no collector through LLVM must use memory
        // exactly as this one did: the same report, line for line.
        crate::emit_llvm::tests::no_collector_as_c(&name, source, &compiled, &errors);
        Some(most.parse().expect("a number"))
    }

    #[test]
    fn with_no_collector_errors_and_early_exits_free_everything() {
        no_collector_as_interpreter("errors-and-exits", NO_COLLECTOR_ERRORS);
    }

    #[test]
    fn with_no_collector_a_failure_half_way_frees_everything() {
        no_collector_as_interpreter("half-way", NO_COLLECTOR_HALF_WAY);
    }

    /// Errors, early exits and text.
    const NO_COLLECTOR_ERRORS: &str = r#"# Errors, early exits and text, with no collector to tidy up after them.
owned class Packet {
    id: int
    name: str
    bytes: owned list[int]

    function total(self) -> int {
        var sum_ = 0
        for b in self.bytes {
            sum_ += b
        }
        return sum_
    }

    function first_over(self, limit: int) -> int {
        for b in self.bytes {
            if b > limit {
                return b              # leaves from inside the loop
            }
        }
        return -1
    }

    function check(self) -> int raises BadPacket {
        for b in self.bytes {
            if b < 0 {
                raise BadPacket(self.id, "negative byte")
            }
        }
        return len(self.bytes)
    }
}

class BadPacket {
    id: int
    why: str

    function label(self) -> str {
        if self.id > 100 {
            return "big"
        }
        return "small"
    }
}

class Fatal {
    code: int
}

function make(id: int, size: int) -> Packet {
    let bytes: owned list[int] = [(n * 7 + id) % 50 - (id % 3) for n in range(size)]
    return Packet(id, "pkt", bytes)
}

function risky(n: int) -> int raises BadPacket {
    if n % 4 == 3 {
        raise BadPacket(n, "risky")
    }
    return n * 2
}

function consume(p: Packet, extra: int) -> int {
    return p.total() + extra
}

function name_of(code: int) -> str {
    if code == 0 {
        return "idle"
    } else if code == 1 {
        return "busy"
    }
    return "unknown"
}

function parse(text: str) -> int raises ValueError {
    let n = int(text)
    if n > 1000 {
        raise ValueError(f"{n} is more than {1000}")
    }
    return n
}

function squares(count: int) -> owned list[int] raises BadPacket {
    return [risky(n) for n in range(count)]
}

function process(id: int) -> int raises BadPacket, Fatal {
    let p = make(id, 6)
    if id == 9 {
        raise Fatal(id)                    # p is freed on the way out
    }
    let checked = p.check()
    return consume(p, risky(id)) + checked # p is held while risky runs
}

var good = 0
var bad = 0
for id in range(12) {
    try {
        good += process(id)
    } except BadPacket as e {
        bad += 1
        print(f"bad packet {e.id}: {e.why} ({e.label()})")
        if e.id == 7 {
            continue
        }
        print("  noted")
    } except Fatal as f {
        print(f"fatal {f.code}")
    }
}
print(good, bad)

# An error raised while another is being handled.
function twice(n: int) -> int raises BadPacket, ValueError {
    try {
        return risky(n)
    } except BadPacket as first {
        if first.id > 10 {
            raise ValueError(f"gave up on {first.id}")
        }
        return risky(n + 4)
    }
}

for n in [3, 5, 11, 2] {
    try {
        print(twice(n))
    } except BadPacket as e {
        print(f"still bad: {e.id}")
    } except ValueError as e {
        print(f"value error: {e}")
    }
}

# A list that fails while it is being filled.
for count in [3, 5] {
    try {
        let s = squares(count)
        print(s, len(s))
    } except BadPacket as e {
        print(f"squares({count}) stopped at {e.id}")
    }
}

# Text: built, compared and printed, never kept.
let words: owned list[str] = ["On", "off", " ON ", "7", "x9", "2000"]
var on = 0
for word in words {
    if word.strip().upper() == "ON" {
        on += 1
    } else if word.lower() == "off" {
        print("off seen")
    } else {
        try {
            print(f"number {parse(word):>6} from {word}")
        } except ValueError as e {
            print(f"not usable: {e}")
        }
    }
}
print(f"on {on} times, {name_of(1)} {name_of(5)}")

var i = 0
while str(i * i) != "49" {
    i += 1
}
print(i, 3.5, 2.0 / 3.0, f"{i / 3:.3f}", true, none)

let label = name_of(0)
var count_e = 0
for k in range(len(label)) {
    if label[k] == "e" or label[k] == "i" {
        count_e += 1
    }
}
print(label, count_e, len(label + "!"), "dl" in label, label < "idler")

# Moves into calls that come to nothing, breaks and whiles.
var round = 0
while true {
    let p = make(round, 3 + round)
    if round == 4 {
        break
    }
    round += 1
    if p.first_over(40) > 0 {
        continue
    }
    print(consume(p, round))
}
print(round)
"#;

    /// Failures that happen while a list is being filled, while owned
    /// values are on their way into a call, and while text is being built.
    const NO_COLLECTOR_HALF_WAY: &str = r#"# Failures that happen half way through something.
owned class Batch {
    tag: str
    values: owned list[int]

    function sum_until(self, stop: int) -> int raises TooBig {
        var total = 0
        for v in self.values {
            if v == stop {
                raise TooBig(v, self.tag)      # the loop's list is freed too
            }
            total += v
        }
        return total
    }

    function grow(self, by: int) {
        for n in range(by) {
            self.values.append(n)
        }
    }
}

class TooBig {
    value: int
    tag: str
}

enum Level {
    Low
    High
}

function level(n: int) -> Level {
    if n > 5 {
        return High
    }
    return Low
}

function check(n: int) -> int raises TooBig {
    if n >= 6 {
        raise TooBig(n, "check")
    }
    return n
}

function batch(tag: str, size: int) -> Batch raises TooBig {
    return Batch(tag, [check(n) for n in range(size)])
}

function join(a: Batch, b: Batch, extra: int) -> int {
    return len(a.values) + len(b.values) + extra
}

function total(xs: borrow list[int]) -> int {
    var t = 0
    for x in xs {
        t += x
    }
    return t
}

function describe(n: int) -> int raises TooBig {
    # Text is in the making when `check` fails.
    print(f"n is {n}, checked {check(n)}, level {level(n)}")
    return n
}

function deep(n: int) -> int {
    if n == 0 {
        return 0
    }
    print(f"depth {n}: {deep(n - 1)}")
    return n
}

var failures = 0
for round in range(300) {
    try {
        let a = batch("a", round % 9)
        let b = batch("b", 3)
        # `a` and `b` are on their way into `join` when `check` fails.
        failures += join(a, b, check(round % 8)) * 0
    } except TooBig as e {
        failures += 1
    }
    try {
        let c = batch("c", 5)
        c.grow(round % 4)
        failures += c.sum_until(round % 7) * 0
    } except TooBig as e {
        if e.tag != "c" {
            print("wrong tag")
        }
        failures += 1
    }
    try {
        if round % 50 == 0 {
            failures += describe((round // 50) * 2) * 0
        }
    } except TooBig as e {
        print(f"round {round}: {e.value} from {e.tag}")
    }
}
print(failures)
print(deep(4))

let big: owned list[int] = [n * n for n in range(40)]
print(total(big), big.reduce(larger, 0), level(3), level(9) == High)
match level(total(big)) {
    case Low {
        print("low")
    }
    case High {
        print("high")
    }
}

function larger(a: int, b: int) -> int {
    if a > b {
        return a
    }
    return b
}

function first_even(xs: borrow list[int]) -> int or none {
    for x in xs {
        if x % 2 == 0 and x > 0 {
            return x
        }
    }
    return none
}

let found = first_even(big)
if found is not none {
    print(f"first even square: {found}")
}
let odd: owned list[int] = [1, 3, 5]
print(first_even(odd) is none)
"#;

    #[test]
    fn with_no_collector_text_is_given_back_after_each_statement() {
        // Ten thousand lines of text are built, a few bytes at a time,
        // in a loop, a comprehension, a condition and a `while`. If any
        // statement left its text behind, the room would overflow.
        let source = r#"owned class Reading {
    tick: int
    celsius: float
}

function label(tick: int) -> str {
    if tick % 2 == 0 {
        return "even"
    }
    return "odd"
}

function width(n: int) -> int {
    return len(str(n))
}

var shown = 0
var longest = 0
for tick in range(2000) {
    let reading = Reading(tick, 20.0 + tick % 7)
    if f"{reading.tick}" == "1999" or tick % 500 == 0 {
        print(f"tick {reading.tick:>5}: {reading.celsius:.1f} C, {label(tick)}", reading)
        shown += 1
    }
    let widths: owned list[int] = [width(n) + len(f"<{n}>") for n in range(tick % 5) if str(n) != "3"]
    longest = max(longest, len(widths), width(tick))
}
var n = 1
while len(str(n)) < 6 {
    n *= 10
}
print(shown, longest, n, label(shown).upper() + "!")
"#;
        let Some(most_text) = no_collector_as_interpreter("text", source) else { return };
        assert!(most_text < 200, "text is piling up in the text room: {most_text} bytes");

        // What the C for one such statement looks like.
        let c = compile_no_collector("let count = 3\nprint(f\"count: {count}\")\nprint(\"done\")").unwrap();
        assert!(
            c.contains(
                "    size_t m1 = zeno_text_top;\n    \
                 zeno_print(1, zeno_join(2, \"count: \", zeno_str_int(z_count)));\n    \
                 zeno_text_top = m1;\n    \
                 zeno_print(1, \"done\");\n"
            ),
            "{c}"
        );
    }

    #[test]
    fn with_no_collector_the_text_room_has_an_end() {
        let source = "let squares: owned list[int] = [n * n for n in range(40)]\nprint(\"before\")\nprint(squares)";
        let c = compile_no_collector(source).unwrap();
        let Some((printed, errors, finished)) = build_and_run("small-room", &c, &["-DZENO_TEXT_ROOM=64"]) else {
            return;
        };
        assert!(!finished);
        assert_eq!(printed, "before\n");
        assert_eq!(errors, "error: a statement built more text than there is room for (ZENO_TEXT_ROOM is 64 bytes)\n");
        // Showing a list gives back each half-built piece as it goes, so
        // the room needs about twice the finished text, not far more.
        let Some((printed, _, finished)) = build_and_run("room", &c, &["-DZENO_TEXT_ROOM=400"]) else { return };
        assert!(finished);
        assert_eq!(printed, interpreted(source));
    }

    /// The number in the line `pool: 2104 bytes taken, in a pool of 32768`
    /// of a memory report: how much of the pool was ever cut into blocks.
    fn pool_needed(report: &str) -> usize {
        let line = report.lines().find(|line| line.starts_with("pool:")).expect("a line about the pool");
        let number = line.split(|c: char| !c.is_ascii_digit()).find(|word| !word.is_empty()).expect("a number");
        number.parse().expect("a number")
    }

    #[test]
    fn with_no_collector_the_pool_has_an_end() {
        let source = "var items: owned list[int] = []\nprint(\"before\")\nfor n in range(100) { items.append(n) }\nprint(len(items))";
        let c = compile_no_collector(source).unwrap();
        // A pool that is too small stops the program, and says what to change.
        let Some((printed, errors, finished)) = build_and_run("small-pool", &c, &["-DZENO_POOL=1024"]) else {
            return;
        };
        assert!(!finished);
        assert_eq!(printed, "before\n");
        assert_eq!(errors, "error: the memory pool is full (ZENO_POOL is 1024 bytes)\n");
        // The report says how much of the pool the program took...
        let Some((_, report, finished)) = build_and_run("pool", &c, &["-DZENO_MEMORY_REPORT"]) else { return };
        assert!(finished);
        let needed = pool_needed(&report);
        assert!(needed > 1024 && needed < 4096, "{report}");
        // ...and that is exact: it runs in a pool of that size, and not
        // in one that is a byte smaller.
        let exact = format!("-DZENO_POOL={needed}");
        let Some((printed, _, finished)) = build_and_run("exact-pool", &c, &[&exact]) else { return };
        assert!(finished);
        assert_eq!(printed, interpreted(source));
        let short = format!("-DZENO_POOL={}", needed - 1);
        let Some((_, errors, finished)) = build_and_run("short-pool", &c, &[&short]) else { return };
        assert!(!finished);
        assert_eq!(errors, format!("error: the memory pool is full (ZENO_POOL is {} bytes)\n", needed - 1));
    }

    #[test]
    fn with_no_collector_the_pool_does_not_fill_up_over_time() {
        // Lists of forty different lengths, and an object for each, made
        // and given back over and over. A pool that broke up into pieces,
        // or lost a block now and then, would need more for more trips.
        let program = |trips: usize| {
            format!(
                "owned class Frame {{
    id: int
    bytes: owned list[int]
}}

var total = 0
for trip in range({trips}) {{
    let bytes: owned list[int] = [n for n in range(trip % 40)]
    let frame = Frame(trip, bytes)
    total = total + len(frame.bytes)
}}
print(total)
"
            )
        };
        let mut needed = Vec::new();
        for trips in [80, 4000] {
            let source = program(trips);
            let c = compile_no_collector(&source).unwrap();
            let name = format!("pool-{trips}-trips");
            let Some((printed, report, finished)) = build_and_run(&name, &c, &["-DZENO_MEMORY_REPORT"]) else { return };
            assert!(finished, "{report}");
            assert_eq!(printed, interpreted(&source));
            assert!(report.contains("no collector, 0 blocks left at the end\n"), "{report}");
            needed.push(pool_needed(&report));
        }
        assert_eq!(needed[0], needed[1], "fifty times the trips needed a bigger pool");
        assert!(needed[0] < 2048, "{needed:?}");
    }

    #[test]
    fn the_pool_gives_a_block_back_to_its_own_size() {
        // The pool on its own, driven from C: what the comments in
        // runtime.c say it does.
        let c = runtime(true, false)
            + r#"
int main(void) {
    char *a = zeno_alloc(24);  /* fits the 32-byte size */
    char *b = zeno_alloc(100); /* fits the 128-byte size */
    /* Lined up for an int or a float, and 8 bytes of note in front of each. */
    printf("%d %d\n", (int)((uintptr_t)a % 8), (int)(b - a));
    memset(a, 'x', 24);
    zeno_free(a);
    /* The next request of that size gets the same place, wiped. */
    char *c = zeno_alloc(17);
    printf("%d %d\n", c == a, c[0] == 0 && c[23] == 0);
    /* Another size does not: it is cut from what has never been used. */
    char *d = zeno_alloc(16);
    printf("%d %d\n", d == b + 128 + 8, (int)zeno_pool_top);
    /* NULL is nothing to give back. A block given back twice is caught. */
    zeno_free(NULL);
    zeno_free(d);
    zeno_free(d);
    printf("not reached\n");
    return 0;
}
"#;
        let Some((printed, errors, finished)) = build_and_run("pool-alone", &c, &[]) else { return };
        assert!(!finished);
        assert_eq!(printed, "0 40\n1 1\n1 200\n");
        assert_eq!(errors, "error: a block of memory was given back twice\n");
    }

    // ------------------------------------------------------------ Kept text

    #[test]
    fn with_no_collector_text_is_kept_by_copy() {
        // Every line that used to be refused because "this text is built
        // while the program runs", and the lines beside them: text kept
        // in a variable, a field and a list, given to a function, given
        // back by one, matched on, and read from a place that a function
        // changes in the middle of the statement.
        no_collector_as_interpreter("text-kept", r#"# The ten lines that used to be refused, and their neighbours.
owned class Packet {
    id: int
    name: str
    bytes: owned list[int]
}
class Bad {
    id: int
    why: str
}
function risky(n: int) -> int raises Bad {
    if n > 2 { raise Bad(n, "big " + str(n)) }
    return n
}
function name_len(text: str) -> int { return len(text) }
function f(n: int) -> str { return str(n) }
function join(a: str, b: str) -> str { return a + b }
function longer(a: str, b: str) -> str {
    if len(a) > len(b) { return a }
    return b
}
var state = "start"
function advance() -> str {
    state = state + "+"
    return state
}
function echo(text: str) -> str {
    state = "changed by echo"
    return text
}
let x = 7
let s = f"{x}"
print(s)
print(f(1))
let p = Packet(1, "a", [1])
p.name = "a" + "b"
print(p, p.name)
let w: owned list[str] = ["a"]
w.append(str(x))
w.append(p.name)
w[0] = w[1] + w[2]
print(w, len(w), "7" in w, "8" in w)
let v: owned list[str] = [str(n) for n in range(3)]
print(v)
print(name_len(str(x)))
let c = "abc"[0]
print(c)
var t = "a"
t += "b"
t = t + t
print(t)
var keep = ""
try { print(int("x")) } except ValueError as e { keep = str(e) }
print(keep)
try { print(risky(5)) } except Bad as e { keep = e.why + "!" }
print(keep)
for i in range(len(str(x * 100))) { print(i) }
print(state, advance(), state)
print(echo(state), state)
print(join(f(1), f(2)), join(join("a", f(3)), longer(t, state)))
print(longer(state, t), longer(p.name, w[0]))
function fold(a: str, b: str) -> str { return a + "," + b }
print(v.reduce(fold, ""))
var maybe: str or none = none
print(maybe)
maybe = f"{x}!"
if maybe is not none { print(maybe + "?") }
maybe = none
print(maybe)
function first(names: borrow list[str]) -> str { return names[0] }
function pick(n: int) -> str or none {
    if n > 1 { return f"n{n}" }
    return none
}
print(first(w), first(v), pick(1), pick(2))
let chosen = pick(3)
print(chosen)
match f(42) {
    case "42" { print("forty-two") }
    case _ { print("other") }
}
match join("a", "b") {
    case "x" { print("x") }
    case other { print("got", other, other + other) }
}
var count = 0
while count < 3 {
    let label = f"trip {count}"
    if count == 1 {
        count += 1
        continue
    }
    print(label)
    count += 1
}
print(w.pop(), w)
"#);
    }

    #[test]
    fn with_no_collector_kept_text_is_freed_on_every_way_out() {
        // A raised object that keeps text, a function that fails half way
        // with text in hand, `break`, `continue` and an early `return`
        // inside loops over text, and a function that calls itself.
        no_collector_as_interpreter("text-ways-out", r#"# Errors, failures half way, and text in the hard places.
class Refused {
    channel: int
    reason: str
}

owned class Reading {
    name: str
    values: owned list[int]
    note: str or none

    function tag(self) -> str {
        if self.note is none { return self.name }
        return f"{self.name} [{self.note}]"
    }
}

function name_of(channel: int) -> str raises Refused {
    if channel % 5 == 4 { raise Refused(channel, f"channel {channel} is off") }
    if channel == 0 { return "zero" }
    return f"ch{channel}"
}

function read(channel: int) -> Reading raises Refused {
    let name = name_of(channel)
    let values: owned list[int] = [channel, channel * 2]
    let other = name_of(channel + 1)
    return Reading(name + "/" + other, values, none)
}

function all_names(count: int) -> int raises Refused {
    let names: owned list[str] = [name_of(n) for n in range(count)]
    var longest = ""
    for name in names {
        if len(name) > len(longest) { longest = name }
        if name == "ch2" { return len(longest) }
    }
    print(names)
    return len(names)
}

function repeat(text: str, times: int) -> str {
    if times == 0 { return "" }
    return text + repeat(text, times - 1)
}

var kept: owned list[str] = []
var last = "none yet"
for channel in range(12) {
    try {
        let reading = read(channel)
        reading.note = f"round {channel}"
        print(reading.tag(), reading)
        kept.append(reading.name)
        last = reading.tag()
    } except Refused as refused {
        print("refused:", refused.reason, refused.channel)
        last = refused.reason
        kept.append(refused.reason.upper())
        if channel > 8 { break }
        continue
    }
}
print(last)
print(kept)
for count in [2, 3, 6] {
    try {
        print(all_names(count))
    } except Refused as refused {
        print(refused)
    }
}
print(repeat("ab", 4), len(repeat("xyz", 30)))
var total = ""
for n in range(5) {
    total = total + str(n)
    total += "-"
}
print(total)
var trips = 0
while trips < 3 {
    total = total.replace(str(trips), "") + "!"
    trips += 1
    print(total)
}
"#);
    }

    #[test]
    fn with_no_collector_objects_and_lists_keep_their_own_text() {
        // Text in the fields of an object and in the items of a list,
        // `none` among them, and a loop whose body gives the list it is
        // walking a new item.
        no_collector_as_interpreter("text-inside", r#"owned class Packet {
    id: int
    name: str
    tags: owned list[str]

    function rename(self, name: str) {
        self.name = name + "/" + self.name
        self.tags.append(self.name)
    }
    function label(self) -> str { return f"{self.id}:{self.name}" }
}
function make(id: int) -> Packet {
    let tags: owned list[str] = [f"t{id}", "plain"]
    return Packet(id, f"p{id}", tags)
}
var gone = make(9)
for id in range(4) {
    let packet = make(id)
    packet.rename(str(id * 10))
    print(packet.label())
    print(packet.tags)
    print(len(packet.name))
    print(packet)
    if id == 2 { gone = packet }
}
print(gone.label())
var notes: owned list[str or none] = [none, "a", f"{1 + 1}"]
notes.append(none)
notes.append(gone.name)
notes[0] = gone.label()
notes[1] = none
print(notes)
for note in notes {
    if note is not none { print(note + "!") }
}
let names: owned list[str] = ["zero", gone.name, f"two {gone.id}"]
for name in names {
    names[1] = name + "?"
    print(name)
}
print(names)
print(names.pop())
print(names)
"#);
    }

    #[test]
    fn with_no_collector_kept_text_does_not_fill_the_pool_over_time() {
        // Each trip builds text, keeps some of it and lets the rest go.
        // A hundred times the trips makes a hundred times the text and
        // needs no more of the pool: every copy is given back.
        let program = |trips: usize| r#"owned class Log {
    last: str
    lines: owned list[str]

    function add(self, line: str) {
        self.last = line
        if len(self.lines) < 4 {
            self.lines.append(line)
        } else {
            self.lines[len(self.lines) - 1] = line
        }
    }
}
function line_for(n: int) -> str {
    if n % 7 == 0 { return "seven" }
    return f"line {n}"
}
let log = Log("", [])
var longest = ""
var total = 0
for n in range({trips}) {
    let line = line_for(n)
    log.add(line)
    if len(line) > len(longest) { longest = line }
    match line_for(n + 1) {
        case "seven" { total += 7 }
        case other { total += len(other) }
    }
}
print(log.last, longest, total)
print(log.lines)
"#.replace("{trips}", &trips.to_string());
        let mut needed = Vec::new();
        for trips in [50, 5000] {
            let source = program(trips);
            let c = compile_no_collector(&source).unwrap();
            let name = format!("text-{trips}-trips");
            let Some((printed, report, finished)) = build_and_run(&name, &c, &["-DZENO_MEMORY_REPORT"]) else { return };
            assert!(finished, "{report}");
            assert_eq!(printed, interpreted(&source));
            assert!(report.contains("no collector, 0 blocks left at the end\n"), "{report}");
            crate::emit_llvm::tests::no_collector_as_c(&name, &source, &printed, &report);
            needed.push(pool_needed(&report));
        }
        assert_eq!(needed[0], needed[1], "a hundred times the trips needed a bigger pool");
        assert!(needed[0] < 512, "{needed:?}");
    }

    #[test]
    fn the_c_for_kept_text() {
        let c = compile_no_collector(
            "function label(channel: int) -> str {
    return f\"ch{channel}\"
}

var best = \"none yet\"
for channel in range(3) {
    let name = label(channel)
    if channel == 1 {
        best = name
    }
}
print(best)
",
        )
        .unwrap();
        // A function's answer travels in the text room: the runtime moves
        // it down to where the room's top was when the `return` began.
        let label = c.split("static const char *z_label(int64_t z2_channel) {\n").nth(1).expect("the function");
        let label = label.split("\n}\n").next().unwrap();
        assert_eq!(
            label,
            "    size_t m3 = zeno_text_top;
    const char *t2 = zeno_text_return(m3, zeno_join(2, \"ch\", zeno_str_int(z2_channel)));
    return t2;"
        );
        // Whatever keeps text has a copy of its own, and frees it: `name`
        // at the end of each trip, `best` when it is given other text and
        // when the program ends. Reading `best`, which a function could
        // change, lends the statement a copy.
        let program = c.split("static void zeno_program(void) {\n").nth(1).expect("the program");
        let program = program.split("\n}\n").next().unwrap();
        assert_eq!(
            program,
            "    z_best = \"none yet\";
    for (int64_t z_channel = 0; z_channel < 3; z_channel++) {
        size_t m1 = zeno_text_top;
        const char *z_name = zeno_text_keep(z_label(z_channel));
        zeno_text_top = m1;
        if (z_channel == 1) {
            const char *t1 = zeno_text_keep(z_name);
            zeno_text_drop(z_best);
            z_best = t1;
        }
        zeno_text_drop(z_name);
    }
    size_t m2 = zeno_text_top;
    zeno_print(1, zeno_text_lend(z_best));
    zeno_text_top = m2;
    zeno_text_drop(z_best);"
        );
        // With a collector none of it is there: text is shared, as ever.
        let shared = compile("var best = \"a\"\nbest = best + \"b\"\nprint(best)").unwrap();
        for gone in ["zeno_text_keep", "zeno_text_drop", "zeno_text_lend", "zeno_text_return"] {
            assert!(!shared.contains(gone), "{gone}");
        }
    }

    #[test]
    fn the_three_places_text_can_be() {
        // The runtime on its own, driven from C: what the comments in
        // runtime.c say about text written in the program, text in the
        // text room and text in the pool.
        let c = runtime(true, false)
            + r#"
int main(void) {
    atexit(zeno_memory_report);
    const char *written = "written";
    /* Text written in the program is kept, lent and freed as it is. */
    printf("%d %d\n", zeno_text_keep(written) == written, zeno_text_lend(written) == written);
    zeno_text_drop(written);
    zeno_text_drop(NULL);
    /* Text in the room is copied into the pool to be kept. */
    size_t mark = zeno_text_top;
    const char *built = zeno_join(2, "bu", "ilt");
    const char *kept = zeno_text_keep(built);
    printf("%d %d %d %s\n", zeno_in_room(built), zeno_in_pool(kept), kept != built, kept);
    zeno_text_top = mark;
    /* Kept text is lent as a copy in the room, which outlives the kept one. */
    const char *lent = zeno_text_lend(kept);
    zeno_text_drop(kept);
    printf("%d %s\n", zeno_in_room(lent), lent);
    zeno_text_top = mark;
    /* A function's answer is moved down to where its `return` began. */
    size_t began = zeno_text_top;
    zeno_join(2, "some", "thing in the way");
    const char *answer = zeno_text_return(began, zeno_join(2, "ans", "wer"));
    printf("%d %d %s\n", answer == zeno_text_room + began, (int)(zeno_text_top - began), answer);
    /* The caller's own text, lower in the room, comes back as it is. */
    size_t inner = zeno_text_top;
    printf("%d %d\n", zeno_text_return(inner, answer) == answer, zeno_text_return(inner, written) == written);
    /* A list of text frees its copies with itself, and nothing is left. */
    zeno_list *list = zeno_list_of(2, (zeno_slot){.s = zeno_text_keep(answer)}, (zeno_slot){.s = written});
    zeno_list *copy = zeno_list_copy_text(list);
    printf("%d %d\n", copy->items[0].s != list->items[0].s, copy->items[1].s == written);
    zeno_list_drop_text(copy);
    zeno_list_drop_text(list);
    return 0;
}
"#;
        let Some((printed, errors, finished)) = build_and_run("text-alone", &c, &["-DZENO_MEMORY_REPORT"]) else { return };
        assert!(finished, "{errors}");
        assert_eq!(printed, "1 1\n1 1 1 built\n1 built\n1 7 answer\n1 1\n1 1\n");
        assert!(errors.contains("no collector, 0 blocks left at the end\n"), "{errors}");
    }

    #[test]
    fn what_a_build_with_no_collector_refuses_says_so() {
        const SETTING: &str = r#"owned class Packet {
    id: int
    name: str
    bytes: owned list[int]
}
class Bad {
    id: int
    why: str
}
enum Shape {
    Dot
    Circle(r: float)
}
function risky(n: int) -> int raises Bad {
    if n > 2 { raise Bad(n, "big") }
    return n
}
function name_len(text: str) -> int { return len(text) }
function look(xs: borrow list[int]) -> int { return len(xs) }
function double(n: int) -> int { return n * 2 }
function add(a: int, b: int) -> int { return a + b }
let x = 7
"#;
        let refused = |body: &str, why: &str| {
            let source = format!("{SETTING}{body}\n");
            let message = compile_no_collector(&source).err().unwrap_or_else(|| "it compiled".to_string());
            assert!(message.contains(why), "{body}\n{message}");
            assert!(message.contains("`zeno build --no-collector` leaves the collector out"), "{message}");
            // Each of these is fine where there is a collector.
            assert!(compile(&source).is_ok(), "{body}");
            // The LLVM backend refuses the same program in the same words.
            crate::emit_llvm::tests::refuses_as_c(&source, &message);
        };

        // (Text is not in this table: it is kept by copy. See
        // `with_no_collector_text_is_kept_by_copy`.)

        // An error is freed when its `except` ends, so it cannot be kept
        // or raised again, and one can only be made by `raise`.
        let caught = "try { print(risky(5)) } except Bad as e";
        refused(&format!("{caught} {{ let again = e\nprint(again.id) }}"), "It cannot be kept");
        refused(&format!("var last: Bad or none = none\n{caught} {{ last = e }}"), "It cannot be kept");
        refused(
            &format!("function pass_on(b: Bad) -> int {{ return b.id }}\n{caught} {{ print(pass_on(e)) }}"),
            "It cannot be kept",
        );
        refused("try { print(risky(5)) } except Error as e { let again = e\nprint(again) }", "so it cannot be kept");
        refused(
            &format!("function f() -> int raises Bad {{ try {{ return risky(5) }} except Bad as e {{ raise e }} }}\n{caught} {{ print(f()) }}"),
            "`raise` needs a new error here",
        );
        refused("let b = Bad(1, \"x\")", "this object would have no owner");
        refused("print(Bad(1, \"x\").id)", "this object would have no owner");
        refused("print(ValueError(\"x\"))", "this makes a `ValueError` that is not raised");

        // Whatever else makes a block that nothing owns.
        refused("let xs = [1, 2, 3]\nprint(len(xs))", "this list has no owner");
        refused("print(look([1, 2]))", "this list has no owner");
        refused("print(x in [1, 2])", "this list has no owner");
        refused("let xs: list[int] = []\nprint(len(xs))", "this list has no owner");
        refused("let s = Circle(1.0)", "`Circle` carries values");
        refused("let f = function(n: int) { return n + x }\nprint(f(1))", "a function written where a value goes");
        refused("let a: owned list[int] = [1]\nlet b: owned list[int] = [2]\nprint(a + b)", "`+` on two lists");
        refused("print(\"a b\".split())", "`split` makes a new list");
        refused("let a: owned list[int] = [1]\nprint(a.map(double))", "`map` makes a new list");
        refused("for ch in \"abc\" { print(ch) }", "a `for` loop over a str");
        refused("let m: owned list[int or none] = [1, none]", "numbers that may be `none`");

        // What sits right beside those refusals, and is accepted.
        let accepted = format!(
            "{SETTING}{}",
            r#"var why = ""
try {
    print(risky(5))
} except Bad as e {
    print(e.id, e.why, e)
    why = e.why                       # a copy, which outlives the error
}
print(why)
for pin in [2, 3, 5] {                # a list the loop frees when it ends
    print(pin)
}
let a: owned list[int] = [1, 2, 3]
print(a.reduce(add, 0), look(a), a, Dot == Dot, 3 s, (1500 ms).s)
var best: int or none = none
best = 4
if best is not none {
    print(best + 1)
}
try {
    print(int("12"), float("1.5"))
    print(int("zz"))
} except ValueError as e {
    print(e)
}
"#
        );
        no_collector_as_interpreter("accepted", &accepted);
    }

    #[test]
    fn the_owned_example_builds_with_no_collector() {
        no_collector_as_interpreter("buffer", include_str!("../examples/buffer.zeno"));
        // Simple code stays simple here too: nothing is noted or freed
        // for a program that makes no memory.
        let c = compile_no_collector("var total = 0\nfor i in range(10) {\n    total += i\n}\nlet done = total > 40").unwrap();
        let program = c.split("static void zeno_program(void) {\n").nth(1).expect("the program");
        assert!(!program.contains("zeno_text_top") && !program.contains("drop"), "{program}");
    }

    #[test]
    fn what_is_not_compiled_yet_says_so() {
        let refused = |source: &str| compile(source).err().unwrap_or_else(|| "it compiled".to_string());
        assert_eq!(
            refused("function outer() -> int {\n function inner() -> int { return 1 }\n return inner()\n}"),
            "`zeno build` cannot compile a function inside a function or block yet. `zeno run` can still run this program"
        );
    }

    // ----- The bits of whole numbers -----

    #[test]
    fn bit_operators_match() {
        same_as_interpreter(
            "bits",
            "let flags = 0b1010_0101\nlet mask = 0x0F\n\
             print(flags & mask, flags | mask, flags ^ mask, ~flags, ~0, ~-1)\n\
             print(1 << 5, 1 << 62, 0xFF >> 4, -8 >> 1, -1 >> 63, -7 >> 1, 5 >> 0, 3 << 0, -1 << 63, -2 << 62)\n\
             print(flags & mask == 5, 1 << 2 + 1, 6 | 1 ^ 3 & 2, 1 | 2 == 3, -1 & 0xFF, ~5 & 0xFF)\n\
             print(f\"{flags & ~mask:08b} {1 << 12:x}\")\n\
             var reg = 0\nreg |= 1 << 3\nreg |= 0x81\nreg &= ~0x01\nreg ^= 0xFF\nreg <<= 4\nreg >>= 2\nprint(reg)\n\
             function bit(n: int, at: int) -> bool { return (n >> at) & 1 == 1 }\n\
             print(bit(5, 0), bit(5, 1), bit(5, 2))\n",
        );
    }

    #[test]
    fn a_shift_too_far_is_an_error() {
        let shift = "function shift(n: int, by: int) -> int { return n << by }\n\
                     function back(n: int, by: int) -> int { return n >> by }\n";
        same_as_interpreter("shift-64", &format!("{shift}print(shift(1, 63), back(-1, 63))\nprint(shift(1, 64))\n"));
        same_as_interpreter("shift-minus", &format!("{shift}print(back(8, -1))\n"));
        same_as_interpreter("shift-big", &format!("{shift}print(shift(3, 61))\nprint(shift(3, 62))\n"));
        same_as_interpreter("shift-small", &format!("{shift}print(shift(-3, 61))\nprint(shift(-3, 62))\n"));
    }

    // ----- Sized whole numbers -----

    const SIZED: &str = r#"class Packet {
    kind: u8
    length: u16
    offset: i8 = -1
    total: i32 = 0
    big: u32 = 4294967295
    name: str = "p"
}
function checksum(data: list[u8]) -> u8 {
    var total = 0
    for byte in data { total += byte }
    return u8.wrap(total)
}
function double(n: int) -> int { return n * 2 }
function small(n: int) -> i8 { return i8(n) }
function level(by: u8 = 7) -> u8 { return by }
let a: u8 = 200
let b: u8 = 100
print(a + b, a - b, b - a, a * b, a // b, a % b, -a, ~a, a << 8, a >> 3, a & b, a | b, a ^ b, a ** 2)
print(a == 200, a != b, a < b, a >= b, a == b + 100, a in [100, 200], f"{a} {a:x} {a:08b}", str(a), abs(small(-5)), min(a, b), max(a, 1000), float(a), int(a))
print(u8.wrap(a + b), i8.wrap(a), i16.wrap(0xFFFF), u16.wrap(-1), u32.wrap(-1), i32.wrap(0x80000000), u8.wrap(256))
print(u8(255), u16(65535), u32(4294967295), i8(-128), i16(-32768), i32(-2147483648), small(127), level(), level(9), level(by=b))
print(double(a), double(small(-3)))
let bytes: list[u8] = [1, 2, 0xFF]
bytes.append(7)
bytes.append(a)
bytes[0] = 16
print(bytes, sum(bytes), max(bytes), min(bytes), len(bytes), checksum(bytes), bytes[1] << 4 | bytes[0], a in bytes, 3 in bytes)
let doubled = [double(n) for n in bytes]
let wide: list[int] = [int(n) for n in bytes]
print(doubled, wide, wide == doubled)
var maybe: u8 or none = none
print(maybe)
maybe = 5
if maybe is not none { print(maybe + 1) }
let p = Packet(kind=3, length=1024)
let q = Packet(255, 65535, -128, -2147483648, 0)
print(p, q, p == q, p == Packet(3, 1024), q.big, ~q.big, -p.big, p.big < -1, q.offset < q.length, p.offset + p.big)
p.kind = u8(p.kind + 250)
p.length = u16.wrap(-2)
p.offset = 127
p.total = i32(p.offset * -1000)
q.big = u32.wrap(-1)
print(p, q, p.kind + q.kind, p.length >> 8)
var count: u8 = 254
for i in range(3) {
    count = u8.wrap(count + 1)
    print(count)
}
match a {
    case 200 { print("two hundred") }
    case _ { print("other") }
}
"#;

    #[test]
    fn sized_ints_match() {
        same_as_interpreter("sized", SIZED);
        same_as_interpreter("packet", include_str!("../examples/packet.zeno"));
    }

    #[test]
    fn a_number_too_big_for_its_size_is_an_error() {
        let small = "function small(n: int) -> i8 { return i8(n) }\nfunction byte(n: int) -> u8 { return u8(n) }\n";
        same_as_interpreter("sized-fits", &format!("{small}print(small(-128), small(127), byte(0), byte(255))\nprint(byte(256))\n"));
        same_as_interpreter("sized-below", &format!("{small}print(small(-129))\n"));
        same_as_interpreter("sized-minus", &format!("{small}print(byte(-1))\n"));
        same_as_interpreter(
            "sized-caught",
            &format!("{small}try {{ print(byte(300)) }} except OverflowError as e {{ print(e.kind, e.message) }}\nprint(small(5))\n"),
        );
        same_as_interpreter("sized-field", "class Reg {\n    value: u8\n}\nlet r = Reg(value=200)\nr.value = u8(r.value + 100)\n");
    }

    #[test]
    fn a_sized_field_is_small_in_the_struct() {
        let source = "class Packet {\n    kind: u8\n    length: u16\n    offset: i8\n    total: i32\n    big: u32\n    plain: int\n}\n\
                      let p = Packet(3, 1024, -1, 0, 0, 0)\np.kind = 9\nprint(p.kind + p.offset)\n";
        let c = compile(source).unwrap();
        assert!(
            c.contains(
                "struct z_Packet {\n    uint8_t z_kind;\n    uint16_t z_length;\n    int8_t z_offset;\n    \
                 int32_t z_total;\n    uint32_t z_big;\n    int64_t z_plain;\n};"
            ),
            "{c}"
        );
        // Small going in, an ordinary int coming out.
        assert!(c.contains("    self->z_kind = (uint8_t)z_kind;\n"), "{c}");
        assert!(c.contains("    z_p->z_kind = (uint8_t)9;\n"), "{c}");
        assert!(c.contains("zeno_add(((int64_t)z_p->z_kind), ((int64_t)z_p->z_offset), 11, 7)"), "{c}");
        // A sized number outside an object is an int64_t like any other.
        let c = compile("function next(n: u8) -> u8 { return u8.wrap(n + 1) }\nlet a: u8 = next(255)\n").unwrap();
        assert!(c.contains("static int64_t z_next(int64_t z_n) {\n    return zeno_wrap(zeno_add(z_n, 1, 1, 45), 8, false);\n}"), "{c}");
    }

    // ----- Generics -----

    /// Generic functions and classes, used with ints, floats, strs, a
    /// u8, an enum, lists and objects, with errors, defaults, functions
    /// as arguments, and one generic calling another.
    const GENERIC: &str = r#"function first[T](items: list[T]) -> T {
    return items[0]
}

function largest[T](items: list[T]) -> T {
    var best = items[0]
    for item in items {
        if item > best {
            best = item
        }
    }
    return best
}

function pair[A, B](a: A, b: B) -> str {
    return f"({a}, {b})"
}

function apply[T, U](value: T, f: function(T) -> U) -> U {
    return f(value)
}

class Stack[T] {
    items: list[T]

    function push(self, item: T) {
        self.items.append(item)
    }

    function pop(self) -> T {
        return self.items.pop()
    }

    function size(self) -> int {
        return len(self.items)
    }
}

class Pair[A, B] {
    left: A
    right: B
}

function swap[A, B](p: Pair[A, B]) -> Pair[B, A] {
    return Pair(p.right, p.left)
}

print(first([1, 2, 3]), first(["a", "b"]), first([1.5]))
print(largest([3, 9, 2]), largest(["pear", "apple"]), largest([2.5, 1.5]))
print(pair(1, "one"), pair(true, 2.5))
print(apply(5, function(n: int) -> int { return n * 2 }), apply("hi", function(s) { return s.upper() }))

let numbers: Stack[int] = Stack(items=[])
numbers.push(1)
numbers.push(2)
let words = Stack(items=["a"])
words.push("b")
print(numbers.pop(), numbers.size(), words.pop(), words, numbers)
let p = Pair(1, "x")
let q = swap(p)
print(p, q, q.left, q.right + 1)
enum Colour {
    Red
    Green
}

class Cell[T] {
    value: T
    count: u8 = 0

    function set(self, value: T) {
        self.value = value
        self.count = u8.wrap(self.count + 1)
    }
}

function reversed[T](items: list[T]) -> list[T] {
    var out: list[T] = []
    var at = len(items) - 1
    while at >= 0 {
        out.append(items[at])
        at -= 1
    }
    return out
}

function find[T](items: list[T], wanted: T) -> int or none {
    var at = 0
    for item in items {
        if item == wanted {
            return at
        }
        at += 1
    }
    return none
}

function first_or[T](items: list[T], otherwise: T) -> T {
    if len(items) == 0 {
        return otherwise
    }
    return items[0]
}

function checked[T](items: list[T], at: int) -> T raises ValueError {
    if at < 0 or at >= len(items) {
        raise ValueError(f"no item {at}")
    }
    return items[at]
}

function map[T, U](items: list[T], f: function(T) -> U) -> list[U] {
    var out: list[U] = []
    for item in items {
        out.append(f(item))
    }
    return out
}

function keep[T](items: list[T], want: function(T) -> bool) -> list[T] {
    return [item for item in items if want(item)]
}

function make[T](value: T, times: int = 2) -> list[T] {
    var out: list[T] = []
    for i in range(times) {
        out.append(value)
    }
    return out
}

let small: u8 = 200
let cell = Cell(small)
cell.set(7)
let named = Cell("a", 3)
named.set("b")
print(cell, named, cell.value + 1, cell == Cell(small), cell == cell)
print(reversed([1, 2, 3]), reversed(["x", "y"]), reversed([Colour.Red, Colour.Green]))
print(find([10, 20, 30], 20), find(["a"], "z"), find([2.5], 2.5))
let nothing: list[str] = []
print(first_or(nothing, "none there"), first_or([4], 0))
try {
    print(checked([1, 2], 1))
    print(checked(["a"], 5))
} except ValueError as e {
    print("caught", e)
}
print(map([1, 2, 3], function(n: int) -> str { return f"<{n}>" }), map(["a", "bb"], function(s: str) -> int { return len(s) }))
print(keep([1, 2, 3, 4], function(n: int) -> bool { return n % 2 == 0 }))
print(make("z"), make(1.5, times=3), make(Cell(1), 1))
let cells: list[Cell[int]] = []
cells.append(Cell(5))
print(cells, len(cells))
let maybe: Cell[str] or none = none
print(maybe)

# A function written inside a generic is copied with it: `both` takes
# an int in one copy and a str in the other.
function doubled[T](items: list[T], twice: function(T) -> T) -> list[T] {
    let both = function(item: T) -> T {
        return twice(twice(item))
    }
    return items.map(both)
}
print(doubled([1, 2], function(n: int) -> int { return n * 2 }))
print(doubled(["a"], function(s: str) -> str { return s + s }))
"#;

    /// An owned class that is generic: each copy frees its own objects.
    const GENERIC_OWNED: &str = r#"owned class Box[T] {
    value: T
    function get(self) -> T {
        return self.value
    }
}
function show[T](box: borrow Box[T]) -> T {
    return box.get()
}
function open[T](box: Box[T]) -> T {
    return box.value
}
function boxed[T](value: T) -> Box[T] {
    return Box(value)
}
let a = Box(5)
let b = Box(2.5)
print(a.get() + 1, show(b), a.value)
let c = boxed("in")
print(show(c))
print(open(c))
print(open(b))
let d: Box[int] = Box(9)
print(d.value)
"#;

    #[test]
    fn generics_match() {
        same_as_interpreter("generic", GENERIC);
        no_collector_as_interpreter("generic-owned", GENERIC_OWNED);
    }

    #[test]
    fn a_generic_is_compiled_once_for_each_set_of_types() {
        let source = "function first[T](items: list[T]) -> T {\n    return items[0]\n}\n\
                      class Cell[T] {\n    value: T\n    function get(self) -> T {\n        return self.value\n    }\n}\n\
                      function unused[T](x: T) -> T {\n    return x\n}\n\
                      let small: u8 = 7\n\
                      print(first([1, 2]), first([\"a\"]), first([3]))\n\
                      print(Cell(1.5).get(), Cell(small), Cell([1]))\n";
        let c = compile(source).unwrap();
        // Two copies of `first`, though it is called three times: an
        // ordinary C function each, with the type filled in.
        assert!(c.contains("static int64_t z_first_int(zeno_list *z_items) {\n"), "{c}");
        assert!(c.contains("static const char *z_first_str(zeno_list *z_items) {\n"), "{c}");
        assert_eq!(c.matches("z_first_int(zeno_list *z_items) {").count(), 1, "{c}");
        assert!(c.contains(" = z_first_int(zeno_list_of(2, (zeno_slot){.i = 1}, (zeno_slot){.i = 2}));"), "{c}");
        // Three structs for `Cell`, each as big as what it holds. A
        // `Cell[u8]` is one byte.
        assert!(c.contains("struct z_Cell_float {\n    double z_value;\n};"), "{c}");
        assert!(c.contains("struct z_Cell_u8 {\n    uint8_t z_value;\n};"), "{c}");
        assert!(c.contains("struct z_Cell_list_int {\n    zeno_list *z_value;\n};"), "{c}");
        assert!(c.contains("static double z_Cell_float_get(z_Cell_float *z_self) {\n    return z_self->z_value;\n}"), "{c}");
        // It still prints as a `Cell`.
        assert!(c.contains("zeno_join(3, \"Cell(value=\", "), "{c}");
        // Nothing is written for the generic itself, or for one that
        // nothing calls.
        assert!(!c.contains("z_first(") && !c.contains("z_Cell ") && !c.contains("z_unused"), "{c}");
    }

    #[test]
    fn what_a_generic_cannot_be_compiled_to_says_so() {
        let refused = |source: &str| compile(source).err().unwrap_or_else(|| "it compiled".to_string());
        assert_eq!(
            refused("class Stack[T] {\n    items: list[T]\n}\nclass Stack_int {\n    n: int\n}\nprint(Stack([1]), Stack_int(2))\n"),
            "`Stack_int` and another class of this program would both be called `z_Stack_int` in C. \
             A copy of a generic class gets such a name: `Stack[int]` becomes `z_Stack_int`. \
             Rename one of them, or build with `--llvm`"
        );
        assert_eq!(
            refused("function outer() -> int {\n    function id[T](x: T) -> T {\n        return x\n    }\n    return id(4)\n}\nprint(outer())\n"),
            "`zeno build` cannot compile a function inside a function or block yet. `zeno run` can still run this program"
        );
    }

    // ----- The board and the clock -----

    const PINS: &str = "import board\nimport time\nfrom board import led, pin\nfrom time import sleep, now\n\
        function flash(light: Pin, times: int) {\n    for i in range(times) {\n        light.toggle()\n        sleep(1 ms)\n    }\n}\n\
        class Lamp {\n    light: Pin\n    name: str\n}\n\
        let start = now()\nled.on()\nled.on()\n\
        print(led, led.is_on(), led.number, led == pin(13), led != pin(2))\n\
        flash(led, 3)\n\
        let relay = board.pin(7)\nrelay.on()\nrelay.off()\nrelay.off()\n\
        let all = [led, relay, pin(0)]\nfor p in all { p.on() }\n\
        print(all, relay in all, pin(5) in all, f\"{relay} is {relay.is_on()}\")\n\
        let lamp = Lamp(board.led, \"front\")\nprint(lamp, lamp.light.is_on())\n\
        var spare: Pin or none = none\nprint(spare)\nspare = pin(63)\n\
        if spare is not none { spare.toggle() }\nprint(spare, spare == pin(63))\n\
        time.sleep(0.001 s)\nsleep(0 ms)\nsleep(-5 s)\n\
        print(now() >= start, time.now() - start < 10 s)\n";

    #[test]
    fn pins_and_the_clock_match() {
        same_as_interpreter("pins", PINS);
    }

    #[test]
    fn a_pin_the_board_does_not_have_is_an_error() {
        same_as_interpreter(
            "no-pin",
            "from board import pin\nfunction pick(n: int) -> Pin { return pin(n) }\n\
             try {\n    pick(64).on()\n} except ValueError as error {\n    print(\"no:\", error.message)\n}\n\
             pick(2).on()\npick(-1).on()\n",
        );
    }

    /// The same program on a board of its own: the four functions that
    /// stand between a Zeno program and the hardware, written again in a
    /// header. This is how a real board is put underneath.
    #[test]
    fn a_board_of_its_own_can_be_put_underneath() {
        let source = "from board import led, pin\nfrom time import sleep, now\n\
                      let before = now()\nled.toggle()\nsleep(250 ms)\nled.toggle()\npin(1).on()\n\
                      print(pin(1).is_on(), now() - before)\nprint(pin(2))\n";
        // A board with two pins that writes down every call, and a clock
        // that moves only when the program sleeps. Its LED is on pin 0.
        let header = "#define ZENO_PINS 2\n\
                      static bool my_levels[ZENO_PINS];\n\
                      static double my_clock = 0.0;\n\
                      static int my_pin(int pin) { return pin == ZENO_LED ? 0 : pin; }\n\
                      static bool zeno_board_read(int pin) { return my_levels[my_pin(pin)]; }\n\
                      static void zeno_board_write(int pin, bool level) {\n\
                      \x20   my_levels[my_pin(pin)] = level;\n\
                      \x20   printf(\"write %d %d\\n\", my_pin(pin), (int)level);\n\
                      }\n\
                      static void zeno_board_sleep(double seconds) { my_clock += seconds * 1000.0; }\n\
                      static double zeno_board_now(void) { return my_clock; }\n";
        let dir = std::env::temp_dir().join(format!("zeno-test-{}-my-board", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let file = dir.join("my_board.h");
        std::fs::write(&file, header).expect("the header is written");
        let flag = format!("-DZENO_BOARD=\"{}\"", file.display());
        for c in [compile(source).unwrap(), compile_firmware(source).unwrap(), compile_no_collector(source).unwrap()] {
            let Some((printed, errors, finished)) = build_and_run("board-header", &c, &[&flag]) else {
                return;
            };
            // The LED is pin 13 to the program, and pin 0 to this board.
            assert_eq!(printed, "write 0 1\nwrite 0 0\nwrite 1 1\ntrue 250 ms\n");
            assert!(!finished);
            assert_eq!(errors, "error: uncaught ValueError: this board has no pin 2. Its pins are 0 to 1\n --> test.zeno:9:7\n");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blink_is_three_lines_of_c_and_needs_no_collector() {
        let c = compile_no_collector(include_str!("../examples/blink.zeno")).unwrap();
        let program = c.split("static void zeno_program(void) {\n").nth(1).expect("the program");
        let body = program.split("\n}\n").next().unwrap();
        assert_eq!(
            body,
            "    while (true) {\n        zeno_pin_toggle(ZENO_LED);\n        zeno_sleep((500.0 / 1000.0));\n    }"
        );
        no_collector_as_interpreter("signal", include_str!("../examples/signal.zeno"));
    }

    #[test]
    fn simple_code_stays_simple() {
        let c = compile("function area(w: float, h: float) -> float { return w * h }\nlet a = area(2.0, 3.0)").unwrap();
        assert!(c.contains("static double z_area(double z_w, double z_h) {\n    return (z_w * z_h);\n}"));
        assert!(c.contains("    z_a = z_area(2.0, 3.0);\n"));
    }
}
