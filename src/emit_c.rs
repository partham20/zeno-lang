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
//! So far it compiles numbers, bools, strings, quantities, functions,
//! control flow, classes, lists, enums and `match`. `try`, `none` and
//! functions used as values are refused with a message, and come next.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ast::*;
use crate::checker::{FunctionKind, Kind, Types};
use crate::error::Error;
use crate::token::Span;
use crate::units;

/// The runtime library, pasted in front of every compiled program.
const RUNTIME: &str = include_str!("runtime.c");

/// Turn a checked program into the text of one C file. `file` is the name of
/// the Zeno source, which the compiled program uses in its error messages.
pub fn emit(program: &Program, types: &Types, file: &str) -> Result<String, Error> {
    let mut emitter = Emitter {
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
        prototypes: Vec::new(),
        definitions: Vec::new(),
        adapters: HashSet::new(),
        returns: None,
    };

    // Classes, enums and functions can be used above where they are written,
    // so learn about all of them before compiling anything. Types go first,
    // because functions mention them.
    for statement in &program.statements {
        match &statement.kind {
            StmtKind::Class(decl) => emitter.register_class(decl, statement.span)?,
            StmtKind::Enum(decl) => emitter.register_enum(decl, statement.span)?,
            _ => {}
        }
    }
    for statement in &program.statements {
        if let StmtKind::Function(decl) = &statement.kind {
            emitter.register(decl, statement.span)?;
        }
    }

    // The statements at the top of the file become the body of C's `main`.
    for statement in &program.statements {
        emitter.stmt(statement)?;
    }
    let main = std::mem::take(&mut emitter.lines);

    // Then each function and method. They are compiled after `main` so that
    // every top-level variable is known by the time a body mentions one.
    for statement in &program.statements {
        match &statement.kind {
            StmtKind::Function(decl) => {
                let entry = &emitter.functions[&decl.name];
                let (c_name, kind) = (entry.c_name.clone(), entry.kind.clone());
                emitter.function(decl, &c_name, &kind, statement.span)?;
            }
            StmtKind::Class(class) => {
                for method in &class.methods {
                    let entry = &emitter.classes[&class.name].methods[&method.name];
                    let (c_name, kind) = (entry.c_name.clone(), entry.kind.clone());
                    emitter.function(method, &c_name, &kind, statement.span)?;
                }
            }
            _ => {}
        }
    }

    // One C file: the runtime, then the program's types, its top-level
    // variables, a first line for every function (so that they can call each
    // other in any order), the functions themselves, and `main`.
    let mut c = String::from(RUNTIME);
    for section in [&emitter.typedefs, &emitter.structs, &emitter.globals, &emitter.prototypes, &emitter.definitions] {
        if !section.is_empty() {
            c.push('\n');
        }
        for item in section {
            c += item;
            c.push('\n');
        }
    }
    c += "\nint main(void) {\n";
    c += &format!("    zeno_file = {};\n", c_string(file));
    c += &render(&main);
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
}

impl Val {
    /// A value that does nothing and depends on nothing: a literal, or a
    /// temporary that already holds its answer.
    fn plain(code: impl Into<String>, kind: Kind) -> Val {
        Val { code: code.into(), kind, reads: false, fails: false, calls: false, pre_fails: false, pre_calls: false }
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
    c_name: String,
    kind: Kind,
}

struct FunctionEntry {
    c_name: String,
    decl: Arc<Function>,
    kind: FunctionKind,
}

/// A class: its fields in order, each with its type and default value, and
/// its methods.
struct ClassEntry {
    fields: Vec<(String, Kind, Option<Expr>)>,
    methods: HashMap<String, FunctionEntry>,
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
    prototypes: Vec<String>,
    definitions: Vec<String>,
    /// The small show-one-item and compare-two-items functions written so
    /// far for lists, so that each is written once.
    adapters: HashSet<String>,
    /// Inside a function: what it returns.
    returns: Option<Kind>,
}

/// The error for a part of Zeno this stage does not handle yet.
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
        Kind::Int => "int64_t ".to_string(),
        Kind::Float | Kind::Quantity(_) => "double ".to_string(),
        Kind::Bool => "bool ".to_string(),
        Kind::Str => "const char *".to_string(),
        Kind::List(_) => "zeno_list *".to_string(),
        Kind::Class(name) | Kind::Enum(name) => format!("z_{name} *"),
        Kind::None => return Err(cannot("a value that is `none`", span)),
        Kind::Other(name) => return Err(cannot(format!("a value of type {name}"), span)),
    })
}

/// A short name for a kind, to build C function names from.
fn key(kind: &Kind) -> String {
    match kind {
        Kind::Int => "int".to_string(),
        Kind::Float => "float".to_string(),
        Kind::Bool => "bool".to_string(),
        Kind::Str => "str".to_string(),
        Kind::None => "none".to_string(),
        Kind::Quantity(unit) => format!("in_{unit}"),
        Kind::List(item) => format!("list_{}", key(item)),
        Kind::Class(name) | Kind::Enum(name) => name.clone(),
        Kind::Other(_) => "other".to_string(),
    }
}

/// C that puts a value into a list slot.
fn to_slot(kind: &Kind, code: &str, span: Span) -> Result<String, Error> {
    let member = match kind {
        Kind::Int => "i",
        Kind::Float | Kind::Quantity(_) => "f",
        Kind::Bool => "b",
        Kind::Str => "s",
        Kind::List(_) | Kind::Class(_) | Kind::Enum(_) => "p",
        Kind::None | Kind::Other(_) => return Err(cannot("a list of these", span)),
    };
    Ok(format!("(zeno_slot){{.{member} = {code}}}"))
}

/// C that takes a value back out of a list slot.
fn from_slot(kind: &Kind, slot: &str, span: Span) -> Result<String, Error> {
    Ok(match kind {
        Kind::Int => format!("{slot}.i"),
        Kind::Float | Kind::Quantity(_) => format!("{slot}.f"),
        Kind::Bool => format!("{slot}.b"),
        Kind::Str => format!("{slot}.s"),
        Kind::List(_) | Kind::Class(_) | Kind::Enum(_) => format!("(({}){slot}.p)", c_type(kind, span)?.trim_end()),
        Kind::None | Kind::Other(_) => return Err(cannot("a list of these", span)),
    })
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
        Ok(temp)
    }

    /// A C name for a new Zeno variable or function. Every Zeno name gets
    /// `z_` in front, so it can never collide with a C keyword or a name
    /// from the C library. A second `x`, in another block, becomes `z2_x`:
    /// that keeps `let x = x + 1` in an inner block from reading itself.
    fn c_name(&mut self, name: &str) -> String {
        let count = self.used.entry(name.to_string()).or_insert(0);
        *count += 1;
        match *count {
            1 => format!("z_{name}"),
            n => format!("z{n}_{name}"),
        }
    }

    fn declare(&mut self, name: &str, kind: Kind) -> String {
        let c_name = self.c_name(name);
        let variable = Variable { c_name: c_name.clone(), kind };
        self.scopes.last_mut().expect("there is always a scope").insert(name.to_string(), variable);
        c_name
    }

    /// Which scope `name` is found in: 0 is the top level.
    fn lookup_depth(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rposition(|scope| scope.contains_key(name))
    }

    fn lookup(&self, name: &str) -> Option<&Variable> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    // ------------------------------------------------------------ Functions

    fn entry(&mut self, decl: &Arc<Function>, c_name: &str, span: Span) -> Result<FunctionEntry, Error> {
        let Some(kind) = self.types.function(decl).cloned() else {
            return Err(cannot(format!("the function `{}`", decl.name), span));
        };
        let c_name = self.c_name(c_name);
        Ok(FunctionEntry { c_name, decl: decl.clone(), kind })
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
        let name = &decl.name;
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
            body += &format!("    {field_type}{field_name};\n");
            params.push(format!("{field_type}{field_name}"));
            fill += &format!("    self->{field_name} = {field_name};\n");
            let label = if index == 0 { format!("{name}({}=", field.name) } else { format!(", {}=", field.name) };
            shown.push(c_string(&label));
            shown.push(self.repr(&Val::plain(format!("self->{field_name}"), kind.clone()), span)?);
            tests.push(self.equal(kind, &format!("a->{field_name}"), &format!("b->{field_name}"), span)?);
        }
        if decl.fields.is_empty() {
            body += "    char nothing; /* C does not allow an empty struct */\n";
            shown.push(c_string(&format!("{name}(")));
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
        self.classes.insert(name.clone(), ClassEntry { fields, methods });
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
    fn function(&mut self, decl: &Function, c_name: &str, kind: &FunctionKind, span: Span) -> Result<(), Error> {
        // A function sees the top-level variables and its own, nothing else.
        self.scopes.truncate(1);
        self.scopes.push(HashMap::new());
        self.lines.clear();
        self.indent = 1;
        // Names only have to be told apart within one C function, so the
        // count of names used starts again from the top-level ones.
        let used = self.used.clone();

        let mut params = Vec::new();
        for (param, param_kind) in decl.params.iter().zip(&kind.params) {
            let param_type = c_type(param_kind, span)?;
            let param_name = self.declare(&param.name, param_kind.clone());
            params.push(format!("{param_type}{param_name}"));
        }
        let return_type = match &kind.returns {
            Kind::None => "void ".to_string(),
            other => c_type(other, span)?,
        };
        let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };

        self.returns = Some(kind.returns.clone());
        for statement in &decl.body {
            self.stmt(statement)?;
        }
        self.returns = None;
        self.scopes.pop();
        self.used = used;

        let body = render(&std::mem::take(&mut self.lines));
        self.helper(format!("static {return_type}{c_name}({params})"), body);
        Ok(())
    }

    // ----------------------------------------------------------- Statements

    fn block(&mut self, statements: &[Stmt]) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        self.indent += 1;
        let result = statements.iter().try_for_each(|statement| self.stmt(statement));
        self.indent -= 1;
        self.scopes.pop();
        result
    }

    fn stmt(&mut self, statement: &Stmt) -> Result<(), Error> {
        let span = statement.span;
        // At the top of the file, outside any function or block.
        let top_level = self.scopes.len() == 1 && self.returns.is_none();
        match &statement.kind {
            StmtKind::Let { name, ty, value, .. } | StmtKind::Const { name, ty, value } => {
                let expected = ty.as_ref().and_then(|ty| self.kind_of_type(ty));
                let val = self.expr_as(value, expected.as_ref())?;
                let c_type = c_type(&val.kind, value.span)?;
                let c_name = self.declare(name, val.kind);
                if top_level {
                    // A top-level variable is one that functions can see: a
                    // C global, given its value when `main` gets this far.
                    self.globals.push(format!("static {c_type}{c_name};"));
                    self.line(format!("{c_name} = {};", val.code));
                } else {
                    self.line(format!("{c_type}{c_name} = {};", val.code));
                }
            }

            StmtKind::Assign { target, op, value } => self.assign(target, *op, value, span)?,

            StmtKind::Expr(expr) => {
                let val = self.expr(expr)?;
                // What is left may still need to run, for what it does.
                if !val.code.is_empty() && (val.fails || val.calls) {
                    self.line(format!("{};", val.code));
                }
            }

            StmtKind::If { condition, then_block, else_block } => {
                let condition = self.expr(condition)?;
                self.line(format!("if ({}) {{", bare(&condition.code)));
                self.block(then_block)?;
                self.else_part(else_block)?;
            }

            StmtKind::While { condition, body } => {
                let mark = self.lines.len();
                let condition = self.expr(condition)?;
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
                self.block(body)?;
                self.line("}");
            }

            StmtKind::For { variable, iterable, body } => {
                self.open_loop(variable, iterable)?;
                self.block(body)?;
                self.line("}");
                self.scopes.pop();
            }

            StmtKind::Match { subject, arms } => self.match_statement(subject, arms, span)?,

            StmtKind::Return(value) => {
                let Some(returns) = self.returns.clone() else {
                    return Err(cannot("`return` outside a function", span));
                };
                match value {
                    Some(value) if returns != Kind::None => {
                        let val = self.expr_as(value, Some(&returns))?;
                        self.line(format!("return {};", val.code));
                    }
                    Some(value) if !matches!(value.kind, ExprKind::None) => {
                        return Err(cannot("returning a value from a function that returns nothing", span))
                    }
                    _ => self.line("return;"),
                }
            }
            StmtKind::Break => self.line("break;"),
            StmtKind::Continue => self.line("continue;"),

            // Compiled separately; see `emit`.
            StmtKind::Function(_) | StmtKind::Class(_) | StmtKind::Enum(_) if top_level => {}
            StmtKind::Function(_) => return Err(cannot("a function inside a function or block", span)),
            StmtKind::Class(_) => return Err(cannot("a class inside a function or block", span)),
            StmtKind::Enum(_) => return Err(cannot("an enum inside a function or block", span)),

            StmtKind::Import { module, names } => {
                let module = module.join(".");
                if !matches!(module.as_str(), "math" | "time") {
                    return Err(cannot(format!("the module `{module}`"), span));
                }
                for name in names {
                    self.imported.insert(name.clone(), module.clone());
                }
            }

            StmtKind::Try { .. } => return Err(cannot("`try`", span)),
            StmtKind::Raise(_) => return Err(cannot("`raise`", span)),
        }
        Ok(())
    }

    /// The kind a written type stands for, where the compiler needs to know
    /// it before it has a value to look at: `let xs: list[int] = []`.
    fn kind_of_type(&self, ty: &Type) -> Option<Kind> {
        let Type::Named { name, args } = ty else {
            return None;
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
            _ => return None,
        })
    }

    /// `expr`, for a place that knows what kind it wants. Only an empty
    /// list needs the help: `[]` alone does not say what it will hold.
    fn expr_as(&mut self, expr: &Expr, expected: Option<&Kind>) -> Result<Val, Error> {
        match (&expr.kind, expected) {
            (ExprKind::List(items), Some(kind @ Kind::List(_))) if items.is_empty() => {
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
        match &target.kind {
            ExprKind::Name(name) => {
                let Some(variable) = self.lookup(name) else {
                    return Err(cannot(format!("an assignment to `{name}`"), target.span));
                };
                let c_name = variable.c_name.clone();
                self.line(format!("{c_name} = {};", val.code));
            }
            // For the other two, Zeno works the new value out first and then
            // looks up where it goes. If working the value out calls a
            // function, pin it down before touching the place.
            ExprKind::Field { object, name } if is_place(object) => {
                let val = if val.calls { self.temp(val, span)? } else { val };
                let object = self.expr(object)?;
                self.line(format!("{}->z_{name} = {};", object.code, val.code));
            }
            ExprKind::Index { object, index } if is_place(object) => {
                let val = self.temp(val, span)?;
                let object = self.expr(object)?;
                let index = self.expr(index)?;
                let slot = to_slot(&val.kind, &val.code, span)?;
                self.line(format!("*zeno_list_at({}, {}, {}) = {slot};", object.code, index.code, at(span)));
            }
            _ => return Err(cannot("an assignment to this", target.span)),
        }
        Ok(())
    }

    /// `match`: each `case` becomes one branch of an `if ... else if` chain.
    /// A pattern gives two things: tests the value must pass (is it this
    /// case?), and new variables for the parts the pattern names.
    fn match_statement(&mut self, subject: &Expr, arms: &[Arm], span: Span) -> Result<(), Error> {
        let subject = self.expr(subject)?;
        let subject = self.temp(subject, span)?;
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
                let c_name = self.declare(&name, kind);
                self.line(format!("{c_type}{c_name} = {code};"));
            }
            let result = arm.body.iter().try_for_each(|statement| self.stmt(statement));
            self.indent -= 1;
            self.scopes.pop();
            result?;
            if tests.is_empty() {
                break; // nothing after a pattern that takes everything can run
            }
        }
        if !arms.is_empty() {
            self.line("}");
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
                if &literal.kind != kind {
                    return Err(cannot("a pattern of a different type from the value", span));
                }
                tests.push(self.equal(kind, code, &literal.code, span)?);
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
            let condition = self.expr(condition)?;
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
    /// caller writes the body, the closing `}`, and drops the scope.
    fn open_loop(&mut self, variable: &str, iterable: &Expr) -> Result<(), Error> {
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
        Ok(())
    }

    /// `for x in items`. The loop walks the list as it was when the loop
    /// began, so a body that adds to the list or removes from it cannot
    /// confuse the loop. That is what `zeno run` does too.
    fn open_list_loop(&mut self, variable: &str, iterable: &Expr) -> Result<(), Error> {
        let span = iterable.span;
        let list = self.expr(iterable)?;
        let Kind::List(item) = list.kind.clone() else {
            return Err(cannot("a `for` loop over anything but a list or `range(...)`", span));
        };
        // A list written out right here is nobody else's, so it needs no copy.
        let fresh = matches!(iterable.kind, ExprKind::List(_) | ExprKind::ListComp { .. });
        let items = self.new_temp();
        let source = if fresh { list.code } else { format!("zeno_list_copy({})", list.code) };
        self.line(format!("zeno_list *{items} = {source};"));

        let position = self.new_temp();
        self.scopes.push(HashMap::new());
        let c_type = c_type(&item, span)?;
        let c_name = self.declare(variable, *item.clone());
        self.line(format!("for (int64_t {position} = 0; {position} < {items}->len; {position}++) {{"));
        self.indent += 1;
        let value = from_slot(&item, &format!("{items}->items[{position}]"), span)?;
        self.line(format!("{c_type}{c_name} = {value};"));
        self.indent -= 1;
        Ok(())
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
        let span = expr.span;
        match &expr.kind {
            ExprKind::Int(n) => match i64::try_from(*n) {
                Ok(n) if n <= i32::MAX as i64 => Ok(Val::plain(n.to_string(), Kind::Int)),
                Ok(n) => Ok(Val::plain(format!("INT64_C({n})"), Kind::Int)),
                Err(_) => Err(cannot("a whole number this big", span)),
            },
            ExprKind::Float(x) => Ok(Val::plain(format!("{x:?}"), Kind::Float)),
            ExprKind::Str(text) => Ok(Val::plain(c_string(text), Kind::Str)),
            ExprKind::Bool(b) => Ok(Val::plain(b.to_string(), Kind::Bool)),
            ExprKind::None => Err(cannot("a value that is `none`", span)),

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
            ExprKind::Name(name) => match self.lookup(name) {
                Some(variable) => {
                    let mut val = Val::plain(variable.c_name.clone(), variable.kind.clone());
                    // Only a top-level variable can be changed by a function
                    // that is called. A local one is out of every other
                    // function's reach, so nothing can disturb it.
                    val.reads = self.lookup_depth(name) == Some(0);
                    Ok(val)
                }
                None if self.functions.contains_key(name) => Err(cannot("a function used as a value", span)),
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
                    val.reads |= matches!(left.kind, Kind::List(_) | Kind::Class(_));
                    return Ok(val);
                }
                self.arithmetic(*op, left, right, span)
            }
            ExprKind::Compare { first, rest } => self.chain(first, rest, span),

            ExprKind::FormatStr(parts) => self.format_string(parts, span),
            ExprKind::Call { callee, args } => self.call(callee, args, span),

            ExprKind::Field { object, name } => self.field(object, name, span),
            ExprKind::Index { object, index } => {
                let mut vals = self.sequence(&[object, index])?;
                let index = vals.pop().expect("two values");
                let list = vals.pop().expect("two values");
                let Kind::List(item) = &list.kind else {
                    return Err(cannot("`[ ]` on anything but a list", span));
                };
                let slot = format!("(*zeno_list_at({}, {}, {}))", list.code, index.code, at(span));
                let mut val = Val::from(from_slot(item, &slot, span)?, *item.clone(), &[&list, &index]).failing();
                val.reads = true;
                Ok(val)
            }
            ExprKind::List(items) => {
                let exprs: Vec<&Expr> = items.iter().collect();
                let vals = self.sequence(&exprs)?;
                let Some(first) = vals.first() else {
                    return Err(cannot(
                        "an empty list with no type beside it. Write `let items: list[int] = []`",
                        span,
                    ));
                };
                if vals.iter().any(|val| val.kind != first.kind) {
                    return Err(cannot("a list whose items are not all one type", span));
                }
                let mut slots = vec![vals.len().to_string()];
                for val in &vals {
                    slots.push(to_slot(&val.kind, &val.code, span)?);
                }
                let parts: Vec<&Val> = vals.iter().collect();
                let kind = Kind::List(Box::new(first.kind.clone()));
                Ok(Val::from(format!("zeno_list_of({})", slots.join(", ")), kind, &parts))
            }
            // `[n * n for n in items if n > 0]` is a loop that fills a list.
            ExprKind::ListComp { element, variable, iterable, condition } => {
                let list = self.new_temp();
                self.line(format!("zeno_list *{list} = zeno_list_new(0);"));
                self.open_loop(variable, iterable)?;
                self.indent += 1;
                if let Some(condition) = condition {
                    let condition = self.expr(condition)?;
                    self.line(format!("if ({}) {{", bare(&condition.code)));
                    self.indent += 1;
                }
                let item = self.expr(element)?;
                self.line(format!("zeno_list_append({list}, {});", to_slot(&item.kind, &item.code, span)?));
                if condition.is_some() {
                    self.indent -= 1;
                    self.line("}");
                }
                self.indent -= 1;
                self.line("}");
                self.scopes.pop();
                let mut val = Val::plain(list, Kind::List(Box::new(item.kind)));
                val.pre_fails = true;
                val.pre_calls = true;
                Ok(val)
            }
            ExprKind::Function(_) => Err(cannot("a function used as a value", span)),
        }
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

            (Add, Str, Str) => Val::from(format!("zeno_join(2, {l}, {r})"), Str, &parts),
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
                if args.iter().any(|arg| arg.name.is_some()) {
                    return Err(cannot("named arguments to a built-in", span));
                }
                let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
                let vals = self.sequence(&exprs)?;
                self.builtin(name, vals, span)
            }
            ExprKind::Field { object, name } => {
                if let ExprKind::Name(owner) = &object.kind {
                    if self.lookup(owner).is_none() {
                        // `math.sqrt(x)`
                        if owner == "math" {
                            return self.math(name, args, span);
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
            _ => Err(cannot("this kind of call", span)),
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
        slots: &[(String, Option<Expr>)],
        receiver: Option<Val>,
        args: &[Arg],
        span: Span,
    ) -> Result<(String, Vec<Val>), Error> {
        let mut places = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            places.push(match &arg.name {
                None => index,
                Some(name) => match slots.iter().position(|(slot, _)| slot == name) {
                    Some(place) => place,
                    None => return Err(cannot(format!("the argument `{name}`"), span)),
                },
            });
        }
        let first = receiver.is_some() as usize;
        let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
        let vals = self.sequence_after(receiver.into_iter().collect(), &exprs)?;

        let mut codes: Vec<Option<String>> = vec![None; slots.len()];
        for (place, val) in places.iter().zip(&vals[first..]) {
            let Some(code) = codes.get_mut(*place) else {
                return Err(cannot("this many arguments", span));
            };
            *code = Some(val.code.clone());
        }
        // An argument that was left out takes the default.
        for (code, (_, default)) in codes.iter_mut().zip(slots) {
            if code.is_none() {
                let default = match default {
                    Some(default) if is_literal(default) => self.expr(default)?,
                    _ => return Err(cannot("a default value that is not a plain literal", span)),
                };
                *code = Some(default.code);
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
        let mut val = Val::from(call, returns, &parts);
        val.fails = true;
        val.calls = true;
        val
    }

    /// A call to one of the program's own functions.
    fn call_function(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let entry = &self.functions[name];
        let (c_name, returns) = (entry.c_name.clone(), entry.kind.returns.clone());
        let slots: Vec<_> = entry.decl.params.iter().map(|param| (param.name.clone(), param.default.clone())).collect();
        let (call, vals) = self.call_c(&c_name, &slots, None, args, span)?;
        Ok(self.called(call, &vals, returns))
    }

    /// `Point(3.0, 4.0)`: a new object.
    fn construct(&mut self, class: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let slots: Vec<_> =
            self.classes[class].fields.iter().map(|(name, _, default)| (name.clone(), default.clone())).collect();
        let (call, vals) = self.call_c(&format!("zeno_new_{class}"), &slots, None, args, span)?;
        let parts: Vec<&Val> = vals.iter().collect();
        Ok(Val::from(call, Kind::Class(class.to_string()), &parts))
    }

    /// `Circle(1.0)`: a new value of an enum.
    fn construct_case(&mut self, owner: &str, index: usize, args: &[Arg], span: Span) -> Result<Val, Error> {
        let (case, fields) = self.enums[owner].cases[index].clone();
        if fields.is_empty() {
            return Err(cannot(format!("`{case}` called like a function: it carries no values"), span));
        }
        let slots: Vec<_> = fields.iter().map(|(name, _)| (name.clone(), None)).collect();
        let (call, vals) = self.call_c(&format!("zeno_new_{owner}_{case}"), &slots, None, args, span)?;
        let parts: Vec<&Val> = vals.iter().collect();
        Ok(Val::from(call, Kind::Enum(owner.to_string()), &parts))
    }

    /// `object.name(args)`: a method of a class, or of a list.
    fn method(&mut self, object: &Expr, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let receiver = self.expr(object)?;
        match receiver.kind.clone() {
            Kind::Class(class) => {
                let Some(entry) = self.classes[&class].methods.get(name) else {
                    return Err(cannot(format!("the method `{name}`"), span));
                };
                let (c_name, returns) = (entry.c_name.clone(), entry.kind.returns.clone());
                // The first parameter is `self`, which the receiver fills.
                let slots: Vec<_> =
                    entry.decl.params.iter().skip(1).map(|param| (param.name.clone(), param.default.clone())).collect();
                let (call, vals) = self.call_c(&c_name, &slots, Some(receiver), args, span)?;
                Ok(self.called(call, &vals, returns))
            }
            Kind::List(item) => {
                let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
                let vals = self.sequence_after(vec![receiver], &exprs)?;
                let parts: Vec<&Val> = vals.iter().collect();
                match (name, vals.as_slice()) {
                    ("append", [list, value]) => {
                        let slot = to_slot(&value.kind, &value.code, span)?;
                        self.line(format!("zeno_list_append({}, {slot});", list.code));
                        let mut val = Val::from(String::new(), Kind::None, &parts);
                        val.pre_calls = true;
                        Ok(val)
                    }
                    ("pop", [list]) => {
                        let slot = format!("zeno_list_pop({}, {})", list.code, at(span));
                        let mut val = Val::from(from_slot(&item, &slot, span)?, *item.clone(), &parts);
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
                    let mut field = Val::from(format!("{}->z_{name}", val.code), kind.clone(), &[&val]);
                    // A function that is called may change the field.
                    field.reads = true;
                    Ok(field)
                }
                None => Err(cannot(format!("a method used as a value: `{name}`"), span)),
            },
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
        Ok(match &val.kind {
            Kind::Str => val.code.clone(),
            Kind::Int => format!("zeno_str_int({})", val.code),
            Kind::Float => format!("zeno_str_float({})", val.code),
            Kind::Bool => format!("zeno_str_bool({})", val.code),
            Kind::Quantity(unit) => format!("zeno_str_quantity({}, {})", val.code, c_string(unit)),
            Kind::Class(name) | Kind::Enum(name) => format!("zeno_show_{name}({})", val.code),
            Kind::List(item) => format!("zeno_list_str({}, {})", val.code, self.repr_adapter(item, span)?),
            Kind::None => return Err(cannot("printing a call that returns nothing", span)),
            Kind::Other(name) => return Err(cannot(format!("printing a value of type {name}"), span)),
        })
    }

    /// The text for a value inside a list or an object. Only a str differs:
    /// there it is shown in quotes.
    fn repr(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        match &val.kind {
            Kind::Str => Ok(format!("zeno_repr_str({})", val.code)),
            _ => self.text(val, span),
        }
    }

    /// A C expression that is true when two values of this kind are equal.
    /// Objects, enum values and lists are equal when their contents are.
    fn equal(&mut self, kind: &Kind, a: &str, b: &str, span: Span) -> Result<String, Error> {
        Ok(match kind {
            Kind::Int | Kind::Float | Kind::Bool | Kind::Quantity(_) => format!("({a} == {b})"),
            Kind::Str => format!("(strcmp({a}, {b}) == 0)"),
            Kind::Class(name) | Kind::Enum(name) => format!("zeno_same_{name}({a}, {b})"),
            Kind::List(item) => format!("zeno_list_eq({a}, {b}, {})", self.eq_adapter(item, span)?),
            Kind::None | Kind::Other(_) => return Err(cannot("comparing these", span)),
        })
    }

    /// `==` and the rest, between two compiled values.
    fn compare(&mut self, op: BinaryOp, left: &Val, right: &Val, span: Span) -> Result<String, Error> {
        let by_contents = matches!(left.kind, Kind::List(_) | Kind::Class(_) | Kind::Enum(_));
        if by_contents && left.kind == right.kind && matches!(op, BinaryOp::Eq | BinaryOp::NotEq) {
            let same = self.equal(&left.kind, &left.code, &right.code, span)?;
            return Ok(if op == BinaryOp::Eq { same } else { format!("(!{same})") });
        }
        comparison(op, left, right, span)
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
                    _ => return Err(cannot("`int` of anything but a number", span)),
                }
            }
            "float" => {
                let val = one()?;
                match &val.kind {
                    Kind::Int | Kind::Float => Val::from(val.double(), Kind::Float, &parts),
                    _ => return Err(cannot("`float` of anything but a number", span)),
                }
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

fn is_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) => true,
        ExprKind::Unit { value, .. } | ExprKind::Unary { op: UnaryOp::Neg, operand: value } => {
            matches!(value.kind, ExprKind::Int(_) | ExprKind::Float(_))
        }
        _ => false,
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

/// A value with a format after the colon, as in `{x:.2f}` or `{n:08b}`.
///
/// The format is read here, while compiling, so the compiled program does
/// not have to read it again every time the line runs. The rules are the
/// ones `format` in `builtins.rs` follows.
fn formatted(val: &Val, spec: &str, plain: String, span: Span) -> Result<String, Error> {
    let bad = || Error::new(format!("`{spec}` is not a format Zeno understands"), span);
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
        return Err(bad());
    }

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
        emit(&program, &types, "test.zeno").map_err(|error| error.message)
    }

    /// Build the C with the machine's C compiler, run it, and return what it
    /// printed, what it wrote as errors, and whether it finished cleanly.
    /// `None` if there is no C compiler here to try it with.
    fn build_and_run(name: &str, c: &str) -> Option<(String, String, bool)> {
        let dir = std::env::temp_dir().join(format!("zeno-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let (c_file, binary) = (dir.join("test.c"), dir.join("test"));
        std::fs::write(&c_file, c).expect("the C file is written");
        let Ok(built) = Command::new("cc").arg("-o").arg(&binary).arg(&c_file).arg("-lm").output() else {
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
        let (interpreted, result) = crate::interpreter::run(&program, true);
        let c = compile(source).expect("the test program compiles");
        let Some((compiled, errors, finished)) = build_and_run(name, &c) else {
            return;
        };
        assert_eq!(compiled, interpreted, "\n{source}");
        match result {
            Ok(()) => assert!(finished, "the compiled program failed: {errors}"),
            // Both must stop with the same error, at the same place.
            Err(error) => {
                assert!(!finished, "the compiled program should have failed");
                let expected = format!(
                    "error: {}\n --> test.zeno:{}:{}\n",
                    error.message, error.span.line, error.span.col
                );
                assert_eq!(errors, expected, "\n{source}");
            }
        }
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
    fn what_is_not_compiled_yet_says_so() {
        let refused = |source: &str| compile(source).err().unwrap_or_else(|| "it compiled".to_string());
        assert_eq!(
            refused("try {\n print(1)\n} except Error {\n print(2)\n}"),
            "`zeno build` cannot compile `try` yet. `zeno run` can still run this program"
        );
        assert_eq!(
            refused("function twice(n: int) -> int { return n * 2 }\nlet f = twice"),
            "`zeno build` cannot compile a function used as a value yet. `zeno run` can still run this program"
        );
    }

    #[test]
    fn simple_code_stays_simple() {
        let c = compile("function area(w: float, h: float) -> float { return w * h }\nlet a = area(2.0, 3.0)").unwrap();
        assert!(c.contains("static double z_area(double z_w, double z_h) {\n    return (z_w * z_h);\n}"));
        assert!(c.contains("    z_a = z_area(2.0, 3.0);\n"));
    }
}
