//! Stage 12: the LLVM backend. It turns a checked program into LLVM IR.
//!
//! ```text
//! tokens -> parser -> tree -> checker -> [emit_llvm] -> LLVM IR -> clang -> program
//! ```
//!
//! LLVM is the compiler library behind Clang, Rust and Swift. Its input is
//! a small typed language called IR ("intermediate representation"). Hand
//! LLVM a program in IR and it does the optimising and writes the machine
//! code for whichever processor is asked for. So this stage does what
//! `emit_c` does, one level further down: there is no C compiler between
//! Zeno and LLVM, and nothing of C's rules to work around.
//!
//! IR reads like assembly for a machine that does not exist:
//!
//! ```text
//! define internal i64 @z_double(i64 %arg.n) {
//! entry:
//!   %t1 = mul i64 %arg.n, 2
//!   ret i64 %t1
//! }
//! ```
//!
//! Four things about it shape everything in this file:
//!
//! 1. **Every value has a type, written at every use.** `i64` is a whole
//!    number of 64 bits, `double` a float, `i1` one bit (a bool), `ptr` an
//!    address. The checker's `Types` say which is which, as for the C
//!    backend.
//! 2. **There are no nested expressions.** `a + b * c` is two
//!    instructions, each giving a new name: `%t1 = mul ...` then
//!    `%t2 = add ...`. The order things happen in is the order they are
//!    written, so Zeno's left-to-right rule costs nothing here. (In
//!    `emit_c` it took `sequence` and `settle`.)
//! 3. **A name is given a value once, and never again.** This is called
//!    SSA, "static single assignment". A Zeno `let` is exactly that, so a
//!    `let` *is* an IR value. A `var` changes, so it gets a place in
//!    memory (`alloca`), and is read with `load` and written with
//!    `store`. LLVM's first pass turns those back into plain values where
//!    it can. See `declare`.
//! 4. **There is no `if` and no `while`.** A function is a list of
//!    blocks. Each block has a label, runs straight through, and ends
//!    with exactly one jump: `br` to another block, or `ret`. Loops and
//!    branches are spelled out with those. See `Code`.
//!
//! This first version compiles numbers, bools, text, functions and control
//! flow, which is what the C backend began with. Classes, lists, enums,
//! `match`, `none` and errors are refused with a message that says so.
//!
//! The compiled program still needs the runtime (`runtime.c`) for text,
//! printing and the collector. `zeno build --llvm` writes it out as a
//! second file and hands both to clang. See `runtime`.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use crate::ast::*;
use crate::checker::{FunctionKind, Kind, Types};
use crate::emit_c;
use crate::error::Error;
use crate::token::Span;

// ---------------------------------------------------------------- The runtime

/// The runtime as an LLVM build needs it: the ordinary one, with its
/// functions made visible to another file.
///
/// A C build pastes the runtime in front of the program, and every
/// function in it is `static`, which in C means "only this file may use
/// it". That lets the C compiler drop the ones a program never calls.
/// Here the program is in a second file, of IR, so the word has to go.
/// Only a `static` at the start of a line is removed, which is one that
/// stands outside any function.
pub fn runtime() -> String {
    let mut text = String::new();
    for line in emit_c::runtime(false, true).lines() {
        text += line.strip_prefix("static ").unwrap_or(line);
        text.push('\n');
    }
    text
}

/// Everything in the runtime and the C library that the IR may call, as
/// LLVM wants it declared. Only the ones a program uses are written into
/// its file.
///
/// `signext` and `zeroext` say how a value narrower than a register is
/// widened when it is handed over. C compilers do this without being
/// asked. IR has to say it, or a processor such as RISC-V may read
/// leftover bits.
const DECLARED: &[(&str, &str)] = &[
    // The processor's own overflow flag, as LLVM exposes it: the result
    // and one bit that says whether it fitted.
    ("llvm.sadd.with.overflow.i64", "declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)"),
    ("llvm.ssub.with.overflow.i64", "declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)"),
    ("llvm.smul.with.overflow.i64", "declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)"),
    ("zeno_overflow", "declare void @zeno_overflow(i32 signext, i32 signext)"),
    ("zeno_floordiv", "declare i64 @zeno_floordiv(i64, i64, i32 signext, i32 signext)"),
    ("zeno_mod", "declare i64 @zeno_mod(i64, i64, i32 signext, i32 signext)"),
    ("zeno_pow", "declare i64 @zeno_pow(i64, i64, i32 signext, i32 signext)"),
    ("zeno_abs", "declare i64 @zeno_abs(i64, i32 signext, i32 signext)"),
    ("zeno_range_count", "declare i64 @zeno_range_count(i64, i64, i64, i32 signext, i32 signext)"),
    ("zeno_div", "declare double @zeno_div(double, double, i32 signext, i32 signext)"),
    ("zeno_floordiv_float", "declare double @zeno_floordiv_float(double, double, i32 signext, i32 signext)"),
    ("zeno_mod_float", "declare double @zeno_mod_float(double, double, i32 signext, i32 signext)"),
    ("zeno_int", "declare i64 @zeno_int(double, i32 signext, i32 signext)"),
    ("pow", "declare double @pow(double, double)"),
    ("fabs", "declare double @fabs(double)"),
    ("strcmp", "declare i32 @strcmp(ptr, ptr)"),
    ("zeno_join", "declare ptr @zeno_join(i32 signext, ...)"),
    ("zeno_print", "declare void @zeno_print(i32 signext, ...)"),
    ("zeno_len", "declare i64 @zeno_len(ptr)"),
    ("zeno_char_at", "declare ptr @zeno_char_at(ptr, i64, i32 signext, i32 signext)"),
    ("zeno_str_int", "declare ptr @zeno_str_int(i64)"),
    ("zeno_str_float", "declare ptr @zeno_str_float(double)"),
    ("zeno_str_bool", "declare ptr @zeno_str_bool(i1 zeroext)"),
    ("zeno_fixed", "declare ptr @zeno_fixed(double, i32 signext)"),
    ("zeno_radix", "declare ptr @zeno_radix(i64, i8 signext)"),
    ("zeno_pad", "declare ptr @zeno_pad(ptr, i32 signext, i8 signext, i1 zeroext)"),
    ("zeno_parse_int", "declare i64 @zeno_parse_int(ptr, i32 signext, i32 signext)"),
    ("zeno_parse_float", "declare double @zeno_parse_float(ptr, i32 signext, i32 signext)"),
    ("zeno_upper", "declare ptr @zeno_upper(ptr)"),
    ("zeno_lower", "declare ptr @zeno_lower(ptr)"),
    ("zeno_strip", "declare ptr @zeno_strip(ptr)"),
    ("zeno_replace", "declare ptr @zeno_replace(ptr, ptr, ptr)"),
    ("zeno_starts_with", "declare zeroext i1 @zeno_starts_with(ptr, ptr)"),
    ("zeno_ends_with", "declare zeroext i1 @zeno_ends_with(ptr, ptr)"),
    ("zeno_mark_words", "declare void @zeno_mark_words(ptr, i64)"),
];

// ------------------------------------------------------------------- Pieces

/// A value that has been worked out: how the IR refers to it, and what
/// kind of Zeno value it is. `code` is a name (`%t3`), a constant (`42`,
/// `true`, `1.5`) or the address of a piece of text (`@.str.2`).
#[derive(Clone)]
struct Val {
    code: String,
    kind: Kind,
}

impl Val {
    fn new(code: impl Into<String>, kind: Kind) -> Val {
        Val { code: code.into(), kind }
    }

    /// What a call that gives nothing back leaves behind.
    fn nothing() -> Val {
        Val::new("", Kind::None)
    }
}

/// Where a variable's value is.
#[derive(Clone)]
enum Place {
    /// It never changes, so the variable is simply another name for this
    /// value. Every `let` inside a function, and every parameter.
    Value(String),
    /// It lives in memory at this address, and is read with `load` and
    /// written with `store`. Every `var`, and everything declared at the
    /// top of the file, where functions must be able to find it.
    Slot(String),
}

#[derive(Clone)]
struct Variable {
    place: Place,
    kind: Kind,
}

/// One of the program's functions, as a call needs to know it.
#[derive(Clone)]
struct Entry {
    decl: Arc<Function>,
    kind: FunctionKind,
    /// Its name in the IR, such as `@z_area`.
    name: String,
}

/// A function that has been reached but not yet written out. `sees` is
/// the top of the file as it stood at the function's own line, because
/// those are the variables it can use (decision 3.6d).
struct Waiting {
    decl: Arc<Function>,
    span: Span,
    sees: HashMap<String, Variable>,
}

/// A loop that is being written: where `continue` and `break` jump to.
struct Loop {
    next: String,
    done: String,
}

/// The function being written.
///
/// A function in IR is a list of blocks. A block starts with a label and
/// must end with exactly one instruction that leaves it (`br`, `ret` or
/// `unreachable`), with none after it. `ended` keeps track of that: it is
/// true from the moment the block has been left until a new label opens
/// the next one.
struct Code {
    lines: Vec<String>,
    /// The `alloca`s, which all go at the top of the first block. One
    /// inside a loop would take new space on the stack on every trip.
    slots: Vec<String>,
    /// The label of the block being filled.
    block: String,
    ended: bool,
    temps: usize,
    labels: usize,
    /// How often each variable name has been given a slot, to tell two
    /// variables of one name apart.
    named: HashMap<String, usize>,
    loops: Vec<Loop>,
    /// What the function gives back. `None` for the top of the file.
    returns: Kind,
    in_function: bool,
}

impl Code {
    fn new(returns: Kind, in_function: bool) -> Code {
        Code {
            lines: Vec::new(),
            slots: Vec::new(),
            block: "entry".to_string(),
            ended: false,
            temps: 0,
            labels: 0,
            named: HashMap::new(),
            loops: Vec::new(),
            returns,
            in_function,
        }
    }
}

struct Emitter<'a> {
    types: &'a Types,
    functions: HashMap<String, Entry>,
    waiting: VecDeque<Waiting>,
    /// The variables in view, innermost block last. The first entry is
    /// the top of the file.
    scopes: Vec<HashMap<String, Variable>>,
    code: Code,

    // The finished parts of the file.
    /// Each piece of text the program mentions, and its name in the IR.
    strings: Vec<(String, String)>,
    globals: Vec<String>,
    /// The globals that hold text, which the collector must be shown.
    roots: Vec<String>,
    definitions: Vec<String>,
    /// The names from `DECLARED` that are called.
    used: BTreeSet<&'static str>,
}

/// The error for a part of Zeno this stage does not handle yet.
fn cannot(what: impl AsRef<str>, span: Span) -> Error {
    Error::new(
        format!("`zeno build --llvm` cannot compile {} yet. `zeno build` without `--llvm` can", what.as_ref()),
        span,
    )
}

/// A kind in words, for a message.
fn described(kind: &Kind) -> String {
    match kind {
        Kind::Int => "an int".to_string(),
        Kind::Float => "a float".to_string(),
        Kind::Str => "a str".to_string(),
        Kind::Bool => "a bool".to_string(),
        Kind::None => "a value that is nothing".to_string(),
        Kind::Quantity(unit) => format!("a number with a unit ({unit})"),
        Kind::List(_) => "a list".to_string(),
        Kind::Class(name) => format!("an object of the class `{name}`"),
        Kind::Enum(name) => format!("a value of the enum `{name}`"),
        Kind::Optional(_) => "a value that may be `none`".to_string(),
        Kind::Error => "an error value".to_string(),
        Kind::Function(_) => "a function used as a value".to_string(),
        Kind::Owned(_) => "an owned value".to_string(),
        Kind::Other(name) => format!("a value of type {name}"),
    }
}

/// The IR type that holds a Zeno value of this kind.
fn ir_type(kind: &Kind, span: Span) -> Result<&'static str, Error> {
    Ok(match kind {
        Kind::Int => "i64",
        Kind::Float => "double",
        // One bit.
        Kind::Bool => "i1",
        // Text is the address of its first byte, as in the C backend.
        Kind::Str => "ptr",
        _ => return Err(cannot(described(kind), span)),
    })
}

/// As `ir_type`, for what a function gives back, which may be nothing.
fn return_type(kind: &Kind, span: Span) -> Result<&'static str, Error> {
    match kind {
        Kind::None => Ok("void"),
        _ => ir_type(kind, span),
    }
}

/// What a global holds before the program has given it a value.
fn zero(kind: &Kind) -> &'static str {
    match kind {
        Kind::Float => "0.0",
        Kind::Bool => "false",
        Kind::Str => "null",
        _ => "0",
    }
}

/// `line, col` as the last two arguments of a runtime call that can fail.
fn at(span: Span) -> String {
    format!("i32 {}, i32 {}", span.line, span.col)
}

/// A name in the IR: `sigil` is `%` for something inside a function and
/// `@` for something global. Names with anything but plain letters and
/// digits in them are written in quotes, which IR allows.
fn named(sigil: char, prefix: &str, name: &str) -> String {
    if name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.') {
        return format!("{sigil}{prefix}{name}");
    }
    let mut quoted = String::new();
    for byte in name.bytes() {
        match byte {
            b'"' | b'\\' | 0..=0x1f | 0x7f.. => quoted += &format!("\\{byte:02X}"),
            _ => quoted.push(byte as char),
        }
    }
    format!("{sigil}\"{prefix}{quoted}\"")
}

/// A float as IR writes one. IR wants a decimal point in it, and has no
/// spelling for infinity, so anything unusual is written as the 64 bits
/// that make up the float.
fn float_code(x: f64) -> String {
    if !x.is_finite() {
        return format!("0x{:016X}", x.to_bits());
    }
    let text = format!("{x:?}");
    match text.split_once('e') {
        Some((digits, power)) if !digits.contains('.') => format!("{digits}.0e{power}"),
        _ => text,
    }
}

// ----------------------------------------------------------------- The file

/// Turn a checked program into the text of one LLVM IR file. `file` is
/// the name of the Zeno source, which the compiled program uses in its
/// error messages.
pub fn emit(program: &Program, types: &Types, file: &str) -> Result<String, Error> {
    let mut emitter = Emitter {
        types,
        functions: HashMap::new(),
        waiting: VecDeque::new(),
        scopes: vec![HashMap::new()],
        code: Code::new(Kind::None, false),
        strings: Vec::new(),
        globals: Vec::new(),
        roots: Vec::new(),
        definitions: Vec::new(),
        used: BTreeSet::new(),
    };

    // A function can be called above the line it is written on, so every
    // function is made known before anything is compiled.
    for statement in &program.statements {
        if let StmtKind::Function(decl) = &statement.kind {
            emitter.register(decl, statement.span)?;
        }
    }

    // The top of the file becomes one function, `zeno_program`. Each
    // function it passes on the way is put aside, and written afterwards.
    for statement in &program.statements {
        emitter.statement(statement)?;
    }
    // `noinline` keeps it a function of its own. The collector looks for
    // addresses on the stack below `main`, and this is where they are.
    emitter.finish("define internal void @zeno_program() noinline".to_string());
    while let Some(waiting) = emitter.waiting.pop_front() {
        emitter.function(waiting)?;
    }

    Ok(emitter.file(file))
}

impl Emitter<'_> {
    /// Put the finished parts together.
    fn file(mut self, file: &str) -> String {
        let name = self.string(file);
        let mut out = format!("; {file}, compiled by `zeno build --llvm`.\n");
        out += "; The runtime it calls is in the C file written beside this one.\n\n";

        // The text the program mentions. `[6 x i8]` is six bytes: the
        // characters and the zero byte that ends a C string.
        for (text, name) in &self.strings {
            let mut bytes = String::new();
            for byte in text.bytes() {
                match byte {
                    b'"' | b'\\' | 0..=0x1f | 0x7f.. => bytes += &format!("\\{byte:02X}"),
                    _ => bytes.push(byte as char),
                }
            }
            out += &format!("{name} = private unnamed_addr constant [{} x i8] c\"{bytes}\\00\"\n", text.len() + 1);
        }
        out.push('\n');

        // The variables at the top of the file.
        for global in &self.globals {
            out += global;
            out.push('\n');
        }
        if !self.globals.is_empty() {
            out.push('\n');
        }

        // What lives in the runtime.
        out += "@zeno_file = external global ptr\n";
        out += "@zeno_stack_base = external global ptr\n";
        out += "@zeno_raised = external global ptr\n";
        self.used.insert("zeno_mark_words");
        for (name, declaration) in DECLARED {
            if self.used.contains(name) {
                out += declaration;
                out.push('\n');
            }
        }
        out.push('\n');

        // The functions as they come in the program, then the top of the
        // file, which was written first.
        self.definitions.rotate_left(1);
        for definition in &self.definitions {
            out += definition;
            out.push('\n');
        }

        // The collector calls this to be shown every global that may hold
        // the address of something it looks after. Each is one word (8
        // bytes) wide.
        out += "define void @zeno_roots() {\nentry:\n";
        out += "  call void @zeno_mark_words(ptr @zeno_raised, i64 8)\n";
        for root in &self.roots {
            out += &format!("  call void @zeno_mark_words(ptr {root}, i64 8)\n");
        }
        out += "  ret void\n}\n\n";

        // Where the program starts. `%base` is a variable of `main`, so
        // its address marks where the stack begins, for the collector.
        out += "define i32 @main() {\nentry:\n";
        out += "  %base = alloca ptr\n";
        out += "  store ptr %base, ptr @zeno_stack_base\n";
        out += &format!("  store ptr {name}, ptr @zeno_file\n");
        out += "  call void @zeno_program()\n";
        out += "  ret i32 0\n}\n";
        out
    }

    // ------------------------------------------------- Writing instructions

    /// Write one instruction into the block being filled.
    fn ins(&mut self, text: impl AsRef<str>) {
        self.open();
        self.code.lines.push(format!("  {}", text.as_ref()));
    }

    /// Write an instruction that gives a value, and return the new name
    /// for that value.
    fn set(&mut self, text: impl AsRef<str>) -> String {
        self.code.temps += 1;
        let temp = format!("%t{}", self.code.temps);
        self.ins(format!("{temp} = {}", text.as_ref()));
        temp
    }

    /// A number for a new group of labels, so that `then4`, `else4` and
    /// `end4` can be seen to belong together.
    fn number(&mut self) -> usize {
        self.code.labels += 1;
        self.code.labels
    }

    /// Write the instruction that ends the block.
    fn end(&mut self, text: impl AsRef<str>) {
        self.ins(text);
        self.code.ended = true;
    }

    /// Leave the block for another one, unless it has been left already
    /// (by a `return`, say).
    fn jump(&mut self, to: &str) {
        if !self.code.ended {
            self.end(format!("br label %{to}"));
        }
    }

    /// Leave the block for one of two others.
    fn jump_if(&mut self, test: &str, yes: &str, no: &str) {
        self.end(format!("br i1 {test}, label %{yes}, label %{no}"));
    }

    /// Start the block with this label. A block cannot simply run on into
    /// the next one, so if the one before is still open it jumps here.
    fn place(&mut self, label: &str) {
        self.jump(label);
        self.code.lines.push(String::new());
        self.code.lines.push(format!("{label}:"));
        self.code.block = label.to_string();
        self.code.ended = false;
    }

    /// Make sure there is a block to write into. After a `return` there
    /// is none, and what follows can never run. It still has to be valid
    /// IR, so it gets a block of its own that nothing jumps to. LLVM
    /// throws such a block away.
    fn open(&mut self) {
        if self.code.ended {
            let label = format!("dead{}", self.number());
            self.place(&label);
        }
    }

    /// Call something from `DECLARED` and return its result.
    fn call(&mut self, returns: &str, name: &'static str, args: impl AsRef<str>) -> String {
        self.uses(name);
        self.set(format!("call {returns} @{name}({})", args.as_ref()))
    }

    fn uses(&mut self, name: &'static str) {
        debug_assert!(DECLARED.iter().any(|(declared, _)| *declared == name), "{name} is not declared");
        self.used.insert(name);
    }

    /// The name of a constant holding `text`.
    fn string(&mut self, text: &str) -> String {
        if let Some((_, name)) = self.strings.iter().find(|(known, _)| known == text) {
            return name.clone();
        }
        let name = format!("@.str.{}", self.strings.len() + 1);
        self.strings.push((text.to_string(), name.clone()));
        name
    }

    /// Finish the function being written and add it to the file.
    fn finish(&mut self, first_line: String) {
        if !self.code.ended {
            match self.code.returns {
                Kind::None => self.end("ret void"),
                // The checker made sure every path gives a value back, so
                // the end of the body is never reached.
                _ => self.end("unreachable"),
            }
        }
        let code = std::mem::replace(&mut self.code, Code::new(Kind::None, false));
        let mut text = format!("{first_line} {{\nentry:\n");
        for line in code.slots.iter().chain(&code.lines) {
            text += line;
            text.push('\n');
        }
        text += "}\n";
        self.definitions.push(text);
    }

    // ------------------------------------------------------------ Functions

    fn register(&mut self, decl: &Arc<Function>, span: Span) -> Result<(), Error> {
        let Some(kind) = self.types.function(decl) else {
            return Err(cannot(format!("`{}`, whose types could not be worked out", decl.name), span));
        };
        for param in &kind.params {
            ir_type(param, span)?;
        }
        return_type(&kind.returns, span)?;
        // `z_` keeps the program's names apart from the runtime's and
        // from the C library's: a Zeno function may well be called `main`.
        let entry = Entry { decl: decl.clone(), kind: kind.clone(), name: named('@', "z_", &decl.name) };
        self.functions.insert(decl.name.clone(), entry);
        Ok(())
    }

    /// Write out one function.
    fn function(&mut self, waiting: Waiting) -> Result<(), Error> {
        let Waiting { decl, span, sees } = waiting;
        let entry = self.functions[&decl.name].clone();
        self.code = Code::new(entry.kind.returns.clone(), true);

        // A parameter cannot be changed, so the value that arrives is
        // the variable.
        let mut params = HashMap::new();
        let mut list = Vec::new();
        for (param, kind) in decl.params.iter().zip(&entry.kind.params) {
            let arrives = named('%', "arg.", &param.name);
            list.push(format!("{} {arrives}", ir_type(kind, span)?));
            params.insert(param.name.clone(), Variable { place: Place::Value(arrives), kind: kind.clone() });
        }
        let outer = std::mem::replace(&mut self.scopes, vec![sees, params]);
        let done = self.block(&decl.body);
        self.scopes = outer;
        done?;

        // `internal` is what `static` is in C: nothing outside this file
        // needs the function, so LLVM may inline it and drop it.
        let returns = return_type(&entry.kind.returns, span)?;
        self.finish(format!("define internal {returns} {}({})", entry.name, list.join(", ")));
        Ok(())
    }

    // ----------------------------------------------------------- Statements

    fn block(&mut self, statements: &[Stmt]) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        let done = statements.iter().try_for_each(|statement| self.statement(statement));
        self.scopes.pop();
        done
    }

    /// Is this the top of the file, outside every function and block?
    fn at_the_top(&self) -> bool {
        !self.code.in_function && self.scopes.len() == 1
    }

    fn lookup(&self, name: &str) -> Option<&Variable> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), Error> {
        let span = statement.span;
        self.open();
        match &statement.kind {
            StmtKind::Let { mutable, name, value, .. } => {
                let val = self.expr(value)?;
                self.declare(name, val, *mutable, span)
            }
            StmtKind::Const { name, value, .. } => {
                let val = self.expr(value)?;
                self.declare(name, val, false, span)
            }
            StmtKind::Assign { target, op, value } => self.assign(target, *op, value, span),
            StmtKind::Expr(expr) => self.expr(expr).map(|_| ()),

            StmtKind::Function(decl) => {
                if !self.at_the_top() {
                    return Err(cannot("a function written inside a function or a block", span));
                }
                // It is written out later. It may use the variables
                // declared so far and no others, so a copy of them goes
                // with it.
                self.waiting.push_back(Waiting { decl: decl.clone(), span, sees: self.scopes[0].clone() });
                Ok(())
            }

            StmtKind::If { condition, then_block, else_block } => {
                let test = self.condition(condition)?;
                let n = self.number();
                let (then, otherwise, end) = (format!("then{n}"), format!("else{n}"), format!("end{n}"));
                self.jump_if(&test, &then, if else_block.is_some() { &otherwise } else { &end });
                self.place(&then);
                self.block(then_block)?;
                if let Some(else_block) = else_block {
                    // `place` adds the jump from the end of `then` to `end`.
                    self.jump(&end);
                    self.place(&otherwise);
                    self.block(else_block)?;
                }
                self.place(&end);
                Ok(())
            }

            StmtKind::While { condition, body } => {
                let n = self.number();
                let (test, body_label, done) = (format!("while{n}"), format!("do{n}"), format!("done{n}"));
                // The test is a block of its own, because it is jumped
                // back to after every trip.
                self.place(&test);
                let holds = self.condition(condition)?;
                self.jump_if(&holds, &body_label, &done);
                self.place(&body_label);
                self.code.loops.push(Loop { next: test.clone(), done: done.clone() });
                let result = self.block(body);
                self.code.loops.pop();
                result?;
                self.jump(&test);
                self.place(&done);
                Ok(())
            }

            StmtKind::For { variable, iterable, body } => self.for_loop(variable, iterable, body),

            StmtKind::Return(value) => {
                if !self.code.in_function {
                    return Err(cannot("`return` outside a function", span));
                }
                let val = match value {
                    Some(value) => self.expr(value)?,
                    None => Val::nothing(),
                };
                if val.kind != self.code.returns {
                    return Err(cannot("a `return` of another type than the function gives back", span));
                }
                match val.kind {
                    Kind::None => self.end("ret void"),
                    _ => self.end(format!("ret {} {}", ir_type(&val.kind, span)?, val.code)),
                }
                Ok(())
            }
            StmtKind::Break | StmtKind::Continue => {
                let Some(Loop { next, done }) = self.code.loops.last() else {
                    return Err(cannot("`break` or `continue` outside a loop", span));
                };
                let to = if statement.kind == StmtKind::Break { done.clone() } else { next.clone() };
                self.jump(&to);
                Ok(())
            }

            StmtKind::Class(_) => Err(cannot("a class", span)),
            StmtKind::Enum(_) => Err(cannot("an enum", span)),
            StmtKind::Match { .. } => Err(cannot("`match`", span)),
            StmtKind::Try { .. } => Err(cannot("`try`", span)),
            StmtKind::Raise(_) => Err(cannot("`raise`", span)),
            StmtKind::Import { .. } => Err(cannot("`import`", span)),
        }
    }

    /// `let name = val` or `var name = val`.
    fn declare(&mut self, name: &str, val: Val, mutable: bool, span: Span) -> Result<(), Error> {
        let ty = ir_type(&val.kind, span)?;
        let place = if self.at_the_top() {
            // Functions use the variables at the top of the file, so
            // those live in memory that is there for the whole run.
            // `internal global` is a `static` variable in C.
            let global = named('@', "z_", name);
            self.globals.push(format!("{global} = internal global {ty} {}", zero(&val.kind)));
            if val.kind == Kind::Str {
                self.roots.push(global.clone());
            }
            self.ins(format!("store {ty} {}, ptr {global}", val.code));
            Place::Slot(global)
        } else if mutable {
            // A `var` gets a slot on the stack.
            let count = self.code.named.entry(name.to_string()).or_insert(0);
            *count += 1;
            let slot = match *count {
                1 => named('%', "var.", name),
                count => named('%', "var.", &format!("{name}.{count}")),
            };
            self.code.slots.push(format!("  {slot} = alloca {ty}"));
            self.ins(format!("store {ty} {}, ptr {slot}", val.code));
            Place::Slot(slot)
        } else {
            // A `let` never changes: the value itself is the variable.
            Place::Value(val.code)
        };
        self.scopes.last_mut().expect("a scope").insert(name.to_string(), Variable { place, kind: val.kind });
        Ok(())
    }

    /// `x = value` and `x += value`.
    fn assign(&mut self, target: &Expr, op: Option<BinaryOp>, value: &Expr, span: Span) -> Result<(), Error> {
        let ExprKind::Name(name) = &target.kind else {
            return Err(cannot("changing anything but a variable", span));
        };
        // The new value first, then the old one, as `zeno run` does.
        let mut new = self.expr(value)?;
        let Some(Variable { place: Place::Slot(slot), kind }) = self.lookup(name).cloned() else {
            return Err(cannot(format!("changing `{name}`"), span));
        };
        let ty = ir_type(&kind, span)?;
        if let Some(op) = op {
            let old = Val::new(self.set(format!("load {ty}, ptr {slot}")), kind.clone());
            new = self.arithmetic(op, old, new, span)?;
        }
        if new.kind != kind {
            return Err(cannot(format!("giving `{name}` a value of another type"), span));
        }
        self.ins(format!("store {ty} {}, ptr {slot}", new.code));
        Ok(())
    }

    /// `for n in range(...)`. A range becomes a plain counting loop: no
    /// list of numbers is built.
    fn for_loop(&mut self, variable: &str, iterable: &Expr, body: &[Stmt]) -> Result<(), Error> {
        let span = iterable.span;
        let bounds = match &iterable.kind {
            ExprKind::Call { callee, args }
                if matches!(&callee.kind, ExprKind::Name(name) if name == "range" && self.lookup(name).is_none())
                    && !self.functions.contains_key("range")
                    && args.iter().all(|arg| arg.name.is_none()) =>
            {
                args
            }
            _ => return Err(cannot("a `for` over anything but a `range`", span)),
        };
        // The ends are worked out once, before the loop.
        let mut ends = Vec::new();
        for bound in bounds {
            let val = self.expr(&bound.value)?;
            if val.kind != Kind::Int {
                return Err(cannot("a `range` of anything but ints", span));
            }
            ends.push(val.code);
        }
        // How many trips, and how to get the loop's number from the trip.
        let (start, stop, step) = match ends.as_slice() {
            [stop] => ("0".to_string(), stop.clone(), None),
            [start, stop] => (start.clone(), stop.clone(), None),
            // With a step, count the trips first. Stepping up to `stop`
            // directly could run past the end of what an int holds.
            [start, _, step] => {
                let count = self.call("i64", "zeno_range_count", format!("i64 {}, {}", ends.join(", i64 "), at(span)));
                ("0".to_string(), count, Some((start.clone(), step.clone())))
            }
            _ => return Err(cannot("this `range`", span)),
        };

        // The counter changes, so it has a slot, like a `var`.
        let n = self.number();
        let counter = named('%', "for.", &format!("{variable}.{n}"));
        self.code.slots.push(format!("  {counter} = alloca i64"));
        self.ins(format!("store i64 {start}, ptr {counter}"));

        let (test, body_label, next, done) = (format!("for{n}"), format!("do{n}"), format!("next{n}"), format!("done{n}"));
        self.place(&test);
        let trip = self.set(format!("load i64, ptr {counter}"));
        let more = self.set(format!("icmp slt i64 {trip}, {stop}"));
        self.jump_if(&more, &body_label, &done);

        self.place(&body_label);
        // What the body sees is a value, which it cannot change.
        let number = match step {
            None => trip.clone(),
            Some((start, step)) => {
                let along = self.set(format!("mul i64 {trip}, {step}"));
                self.set(format!("add i64 {start}, {along}"))
            }
        };
        self.scopes.push(HashMap::from([(
            variable.to_string(),
            Variable { place: Place::Value(number), kind: Kind::Int },
        )]));
        self.code.loops.push(Loop { next: next.clone(), done: done.clone() });
        let result = self.block(body);
        self.code.loops.pop();
        self.scopes.pop();
        result?;

        // `continue` comes here: add one and test again. The counter is
        // below `stop`, so adding one cannot overflow.
        self.place(&next);
        let trip = self.set(format!("load i64, ptr {counter}"));
        let up = self.set(format!("add i64 {trip}, 1"));
        self.ins(format!("store i64 {up}, ptr {counter}"));
        self.jump(&test);
        self.place(&done);
        Ok(())
    }

    // ---------------------------------------------------------- Expressions

    /// An expression that must be a bool, for `if` and `while`.
    fn condition(&mut self, expr: &Expr) -> Result<String, Error> {
        let val = self.expr(expr)?;
        match val.kind {
            Kind::Bool => Ok(val.code),
            _ => Err(cannot(format!("{} used as a condition", described(&val.kind)), expr.span)),
        }
    }

    /// Write the instructions that work an expression out, and return
    /// the value they leave.
    fn expr(&mut self, expr: &Expr) -> Result<Val, Error> {
        let span = expr.span;
        Ok(match &expr.kind {
            // The lexer lets one number through that is one too big for an
            // int, and `zeno run` reports it when it gets there.
            ExprKind::Int(n) if *n > i64::MAX as u64 => return Err(cannot("a whole number this big", span)),
            ExprKind::Int(n) => Val::new(n.to_string(), Kind::Int),
            ExprKind::Float(x) => Val::new(float_code(*x), Kind::Float),
            ExprKind::Bool(b) => Val::new(b.to_string(), Kind::Bool),
            ExprKind::Str(text) => Val::new(self.string(text), Kind::Str),
            ExprKind::FormatStr(parts) => self.format_string(parts, span)?,

            ExprKind::Name(name) => match self.lookup(name).cloned() {
                Some(Variable { place: Place::Value(code), kind }) => Val::new(code, kind),
                Some(Variable { place: Place::Slot(slot), kind }) => {
                    let ty = ir_type(&kind, span)?;
                    Val::new(self.set(format!("load {ty}, ptr {slot}")), kind)
                }
                None => return Err(cannot(format!("`{name}` used as a value"), span)),
            },

            ExprKind::Unary { op: UnaryOp::Neg, operand } => match &operand.kind {
                // `-5` is a number, not a sum to be worked out.
                ExprKind::Int(n) if *n <= i64::MAX as u64 => Val::new(format!("-{n}"), Kind::Int),
                _ => {
                    let val = self.expr(operand)?;
                    match val.kind {
                        Kind::Int => Val::new(self.checked("ssub", "0", &val.code, span), Kind::Int),
                        Kind::Float => Val::new(self.set(format!("fneg double {}", val.code)), Kind::Float),
                        _ => return Err(cannot(format!("`-` in front of {}", described(&val.kind)), span)),
                    }
                }
            },
            ExprKind::Unary { op: UnaryOp::Not, operand } => {
                let test = self.condition(operand)?;
                // There is no "not" instruction. Exclusive-or with 1 flips the bit.
                Val::new(self.set(format!("xor i1 {test}, true")), Kind::Bool)
            }

            ExprKind::Binary { op: op @ (BinaryOp::And | BinaryOp::Or), left, right } => {
                self.logic(*op, left, right)?
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.expr(left)?;
                let right = self.expr(right)?;
                match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::FloorDiv
                    | BinaryOp::Mod
                    | BinaryOp::Pow => self.arithmetic(*op, left, right, span)?,
                    _ => Val::new(self.compare(*op, &left, &right, span)?, Kind::Bool),
                }
            }
            ExprKind::Compare { first, rest } => self.chain(first, rest)?,

            ExprKind::Call { callee, args } => self.call_expr(callee, args, span)?,
            ExprKind::Index { object, index } => {
                let text = self.expr(object)?;
                let position = self.expr(index)?;
                if (&text.kind, &position.kind) != (&Kind::Str, &Kind::Int) {
                    return Err(cannot(format!("`[ ]` on {}", described(&text.kind)), span));
                }
                let args = format!("ptr {}, i64 {}, {}", text.code, position.code, at(span));
                Val::new(self.call("ptr", "zeno_char_at", args), Kind::Str)
            }

            ExprKind::None => return Err(cannot("`none`", span)),
            ExprKind::Unit { .. } => return Err(cannot("a number with a unit", span)),
            ExprKind::List(_) | ExprKind::ListComp { .. } => return Err(cannot("a list", span)),
            ExprKind::Function(_) => return Err(cannot("a function used as a value", span)),
            ExprKind::Field { .. } => return Err(cannot("a field", span)),
        })
    }

    /// An int as a float, for arithmetic that mixes the two.
    fn double(&mut self, val: &Val) -> String {
        match val.kind {
            // "signed int to floating point".
            Kind::Int => self.set(format!("sitofp i64 {} to double", val.code)),
            _ => val.code.clone(),
        }
    }

    /// `+`, `-` or `*` on two ints, with the check Zeno promises.
    ///
    /// A plain `add` would wrap round on overflow. This asks for the
    /// result together with the processor's overflow flag, and leaves for
    /// a block that reports the error if the flag is set. The runtime
    /// function does not come back, which is what `unreachable` says.
    fn checked(&mut self, op: &'static str, left: &str, right: &str, span: Span) -> String {
        let name = match op {
            "sadd" => "llvm.sadd.with.overflow.i64",
            "ssub" => "llvm.ssub.with.overflow.i64",
            _ => "llvm.smul.with.overflow.i64",
        };
        let both = self.call("{ i64, i1 }", name, format!("i64 {left}, i64 {right}"));
        let result = self.set(format!("extractvalue {{ i64, i1 }} {both}, 0"));
        let overflowed = self.set(format!("extractvalue {{ i64, i1 }} {both}, 1"));
        let n = self.number();
        let (overflow, fits) = (format!("overflow{n}"), format!("fits{n}"));
        self.jump_if(&overflowed, &overflow, &fits);
        self.place(&overflow);
        self.uses("zeno_overflow");
        self.ins(format!("call void @zeno_overflow({})", at(span)));
        self.end("unreachable");
        self.place(&fits);
        result
    }

    fn arithmetic(&mut self, op: BinaryOp, left: Val, right: Val, span: Span) -> Result<Val, Error> {
        use BinaryOp::*;
        use Kind::{Float, Int, Str};
        let (l, r) = (left.code.clone(), right.code.clone());
        let numbers = matches!(left.kind, Int | Float) && matches!(right.kind, Int | Float);

        Ok(match (op, &left.kind, &right.kind) {
            (Add, Int, Int) => Val::new(self.checked("sadd", &l, &r, span), Int),
            (Sub, Int, Int) => Val::new(self.checked("ssub", &l, &r, span), Int),
            (Mul, Int, Int) => Val::new(self.checked("smul", &l, &r, span), Int),
            // These have more to them than one instruction (`//` rounds
            // down, `%` follows the right side's sign), and the runtime
            // already has them right.
            (FloorDiv | Mod | Pow, Int, Int) => {
                let name = match op {
                    FloorDiv => "zeno_floordiv",
                    Mod => "zeno_mod",
                    _ => "zeno_pow",
                };
                Val::new(self.call("i64", name, format!("i64 {l}, i64 {r}, {}", at(span))), Int)
            }

            (Add, Str, Str) => Val::new(self.join(&[l, r]), Str),

            // What is left is floats, or an int with a float. The float
            // instructions already do what Zeno wants, except for
            // dividing by zero.
            (Add | Sub | Mul, _, _) if numbers => {
                let (l, r) = (self.double(&left), self.double(&right));
                let instruction = match op {
                    Add => "fadd",
                    Sub => "fsub",
                    _ => "fmul",
                };
                Val::new(self.set(format!("{instruction} double {l}, {r}")), Float)
            }
            (Div | FloorDiv | Mod, _, _) if numbers => {
                let (l, r) = (self.double(&left), self.double(&right));
                let name = match op {
                    Div => "zeno_div",
                    FloorDiv => "zeno_floordiv_float",
                    _ => "zeno_mod_float",
                };
                Val::new(self.call("double", name, format!("double {l}, double {r}, {}", at(span))), Float)
            }
            (Pow, _, _) if numbers => {
                let (l, r) = (self.double(&left), self.double(&right));
                Val::new(self.call("double", "pow", format!("double {l}, double {r}")), Float)
            }

            _ => {
                return Err(cannot(
                    format!("`{}` between {} and {}", op.text(), described(&left.kind), described(&right.kind)),
                    span,
                ))
            }
        })
    }

    /// `a and b`, `a or b`. The right side must not run unless it is needed.
    ///
    /// ```text
    ///   br i1 %a, label %and1, label %end1     ; a is false: skip b
    /// and1:
    ///   ...work out b...
    ///   br label %end1
    /// end1:
    ///   %t = phi i1 [ false, %entry ], [ %b, %and1 ]
    /// ```
    ///
    /// `phi` is how a value that depends on the way a block was reached
    /// is written when names can be given a value only once: "false if we
    /// came from `entry`, `%b` if we came from `and1`".
    fn logic(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Result<Val, Error> {
        let is_and = op == BinaryOp::And;
        let first = self.condition(left)?;
        let from = self.code.block.clone();
        let n = self.number();
        let (more, end) = (format!("{}{n}", if is_and { "and" } else { "or" }), format!("end{n}"));
        match is_and {
            true => self.jump_if(&first, &more, &end),
            false => self.jump_if(&first, &end, &more),
        }
        self.place(&more);
        let second = self.condition(right)?;
        // Working out the right side may have ended in another block.
        let after = self.code.block.clone();
        self.place(&end);
        let settled = if is_and { "false" } else { "true" };
        let result = self.set(format!("phi i1 [ {settled}, %{from} ], [ {second}, %{after} ]"));
        Ok(Val::new(result, Kind::Bool))
    }

    /// `a < b`, and `a < b < c`: each piece is worked out once, and only
    /// if the comparisons before it held.
    fn chain(&mut self, first: &Expr, rest: &[(BinaryOp, Expr)]) -> Result<Val, Error> {
        let mut previous = self.expr(first)?;
        let n = if rest.len() > 1 { self.number() } else { 0 };
        let end = format!("end{n}");
        // The blocks that leave early because a comparison failed.
        let mut failed = Vec::new();
        let mut held = String::new();
        for (index, (op, expr)) in rest.iter().enumerate() {
            let next = self.expr(expr)?;
            held = self.compare(*op, &previous, &next, expr.span)?;
            if index + 1 < rest.len() {
                failed.push(self.code.block.clone());
                let more = format!("then{n}.{}", index + 1);
                self.jump_if(&held, &more, &end);
                self.place(&more);
            }
            previous = next;
        }
        // One comparison needs no blocks at all.
        if failed.is_empty() {
            return Ok(Val::new(held, Kind::Bool));
        }
        let last = self.code.block.clone();
        self.place(&end);
        let mut ways: Vec<String> = failed.iter().map(|block| format!("[ false, %{block} ]")).collect();
        ways.push(format!("[ {held}, %{last} ]"));
        Ok(Val::new(self.set(format!("phi i1 {}", ways.join(", "))), Kind::Bool))
    }

    /// One comparison between two values.
    fn compare(&mut self, op: BinaryOp, left: &Val, right: &Val, span: Span) -> Result<String, Error> {
        use BinaryOp::*;
        use Kind::{Bool, Float, Int, Str};
        // `icmp` compares whole numbers, and wants to be told that they
        // are signed (`slt` is "signed less than"). `fcmp` compares
        // floats. Its `o` is for "ordered": false if either side is not
        // a number at all. `!=` is the one that is true then.
        let (whole, float) = match op {
            Eq => ("eq", "oeq"),
            NotEq => ("ne", "une"),
            Lt => ("slt", "olt"),
            LtEq => ("sle", "ole"),
            Gt => ("sgt", "ogt"),
            GtEq => ("sge", "oge"),
            _ => return Err(cannot(format!("`{}`", op.text()), span)),
        };
        Ok(match (&left.kind, &right.kind) {
            (Int, Int) => self.set(format!("icmp {whole} i64 {}, {}", left.code, right.code)),
            (Bool, Bool) if matches!(op, Eq | NotEq) => {
                self.set(format!("icmp {whole} i1 {}, {}", left.code, right.code))
            }
            (Int | Float, Int | Float) => {
                let (l, r) = (self.double(left), self.double(right));
                self.set(format!("fcmp {float} double {l}, {r}"))
            }
            // Two pieces of text are compared by what they say, which
            // `strcmp` answers with a number below, at or above zero.
            (Str, Str) => {
                let order = self.call("i32", "strcmp", format!("ptr {}, ptr {}", left.code, right.code));
                self.set(format!("icmp {whole} i32 {order}, 0"))
            }
            _ => {
                return Err(cannot(
                    format!("`{}` between {} and {}", op.text(), described(&left.kind), described(&right.kind)),
                    span,
                ))
            }
        })
    }

    // ---------------------------------------------------------------- Calls

    fn call_expr(&mut self, callee: &Expr, args: &[Arg], span: Span) -> Result<Val, Error> {
        match &callee.kind {
            ExprKind::Name(name) if self.lookup(name).is_some() => {
                Err(cannot("a call to a function held in a variable", span))
            }
            ExprKind::Name(name) if self.functions.contains_key(name) => self.call_function(name, args, span),
            ExprKind::Name(name) => self.builtin(name, args, span),
            ExprKind::Field { object, name } => self.method(object, name, args, span),
            _ => Err(cannot("this call", span)),
        }
    }

    /// A call to one of the program's own functions.
    fn call_function(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let entry = self.functions[name].clone();
        let params = &entry.decl.params;

        // The arguments are worked out left to right as they are
        // written, and each is put in the place of its parameter.
        let mut given: Vec<Option<Val>> = vec![None; params.len()];
        for (index, arg) in args.iter().enumerate() {
            let place = match &arg.name {
                None => index,
                Some(name) => match params.iter().position(|param| &param.name == name) {
                    Some(place) => place,
                    None => return Err(cannot(format!("the argument `{name}`"), span)),
                },
            };
            let val = self.expr(&arg.value)?;
            match given.get_mut(place) {
                Some(slot) => *slot = Some(val),
                None => return Err(cannot("this many arguments", span)),
            }
        }
        let mut list = Vec::new();
        for ((param, kind), val) in params.iter().zip(&entry.kind.params).zip(given) {
            // An argument that was left out takes the default.
            let val = match (val, &param.default) {
                (Some(val), _) => val,
                (None, Some(default)) if emit_c::is_literal(default) => self.expr(default)?,
                (None, _) => return Err(cannot("a default value that is not a plain literal", span)),
            };
            if val.kind != *kind {
                return Err(cannot(format!("{} given where `{}` wants {}", described(&val.kind), name, described(kind)), span));
            }
            list.push(format!("{} {}", ir_type(kind, span)?, val.code));
        }

        let returns = return_type(&entry.kind.returns, span)?;
        let call = format!("call {returns} {}({})", entry.name, list.join(", "));
        Ok(match entry.kind.returns {
            // Nothing comes back, so there is nothing to name.
            Kind::None => {
                self.ins(call);
                Val::nothing()
            }
            kind => Val::new(self.set(call), kind),
        })
    }

    /// A call to one of the functions Zeno comes with.
    fn builtin(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        if args.iter().any(|arg| arg.name.is_some()) {
            return Err(cannot(format!("`{name}` with a named argument"), span));
        }
        let mut vals = Vec::new();
        for arg in args {
            vals.push(self.expr(&arg.value)?);
        }
        let wrong = || cannot(format!("`{name}` used this way"), span);
        Ok(match (name, vals.as_slice()) {
            ("print", _) => {
                // `zeno_print` takes a count and then that many pieces of
                // text. `(i32, ...)` says the call has a varying number
                // of arguments, which IR must be told at each call.
                let mut list = format!("i32 {}", vals.len());
                for val in &vals {
                    list += &format!(", ptr {}", self.text(val, span)?);
                }
                self.uses("zeno_print");
                self.ins(format!("call void (i32, ...) @zeno_print({list})"));
                Val::nothing()
            }
            ("str", [val]) => Val::new(self.text(val, span)?, Kind::Str),
            ("len", [val]) if val.kind == Kind::Str => {
                Val::new(self.call("i64", "zeno_len", format!("ptr {}", val.code)), Kind::Int)
            }
            ("abs", [val]) => match val.kind {
                Kind::Int => Val::new(self.call("i64", "zeno_abs", format!("i64 {}, {}", val.code, at(span))), Kind::Int),
                Kind::Float => Val::new(self.call("double", "fabs", format!("double {}", val.code)), Kind::Float),
                _ => return Err(wrong()),
            },
            ("int", [val]) => match val.kind {
                Kind::Int => val.clone(),
                Kind::Float => {
                    Val::new(self.call("i64", "zeno_int", format!("double {}, {}", val.code, at(span))), Kind::Int)
                }
                Kind::Str => {
                    Val::new(self.call("i64", "zeno_parse_int", format!("ptr {}, {}", val.code, at(span))), Kind::Int)
                }
                _ => return Err(wrong()),
            },
            ("float", [val]) => match val.kind {
                Kind::Int | Kind::Float => Val::new(self.double(val), Kind::Float),
                Kind::Str => Val::new(
                    self.call("double", "zeno_parse_float", format!("ptr {}, {}", val.code, at(span))),
                    Kind::Float,
                ),
                _ => return Err(wrong()),
            },
            ("min" | "max", [first, rest @ ..])
                if !rest.is_empty()
                    && matches!(first.kind, Kind::Int | Kind::Float)
                    && rest.iter().all(|val| val.kind == first.kind) =>
            {
                // `select` picks one of two values by a bool, with no
                // jump: `b < a ? b : a`. The earlier value is kept when
                // the two are equal.
                let (compare, ty) = match (&first.kind, name) {
                    (Kind::Int, "min") => ("icmp slt", "i64"),
                    (Kind::Int, _) => ("icmp sgt", "i64"),
                    (_, "min") => ("fcmp olt", "double"),
                    _ => ("fcmp ogt", "double"),
                };
                let mut best = first.code.clone();
                for val in rest {
                    let better = self.set(format!("{compare} {ty} {}, {best}", val.code));
                    best = self.set(format!("select i1 {better}, {ty} {}, {ty} {best}", val.code));
                }
                Val::new(best, first.kind.clone())
            }
            _ => return Err(wrong()),
        })
    }

    /// A method of a str, such as `name.upper()`.
    fn method(&mut self, object: &Expr, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let receiver = self.expr(object)?;
        if receiver.kind != Kind::Str {
            return Err(cannot(format!("a method of {}", described(&receiver.kind)), span));
        }
        let mut list = format!("ptr {}", receiver.code);
        for arg in args {
            let val = self.expr(&arg.value)?;
            if val.kind != Kind::Str || arg.name.is_some() {
                return Err(cannot(format!("the str method `{name}` called this way"), span));
            }
            list += &format!(", ptr {}", val.code);
        }
        let (function, returns, kind) = match (name, args.len()) {
            ("upper", 0) => ("zeno_upper", "ptr", Kind::Str),
            ("lower", 0) => ("zeno_lower", "ptr", Kind::Str),
            ("strip", 0) => ("zeno_strip", "ptr", Kind::Str),
            ("replace", 2) => ("zeno_replace", "ptr", Kind::Str),
            ("starts_with", 1) => ("zeno_starts_with", "i1", Kind::Bool),
            ("ends_with", 1) => ("zeno_ends_with", "i1", Kind::Bool),
            _ => return Err(cannot(format!("the str method `{name}` called this way"), span)),
        };
        Ok(Val::new(self.call(returns, function, list), kind))
    }

    // ----------------------------------------------------------------- Text

    /// Several pieces of text made into one.
    fn join(&mut self, pieces: &[String]) -> String {
        self.uses("zeno_join");
        let list: Vec<String> = pieces.iter().map(|piece| format!("ptr {piece}")).collect();
        self.set(format!("call ptr (i32, ...) @zeno_join(i32 {}, {})", pieces.len(), list.join(", ")))
    }

    /// The value as text, for `print`, `str` and f-strings.
    fn text(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        Ok(match &val.kind {
            Kind::Str => val.code.clone(),
            Kind::Int => self.call("ptr", "zeno_str_int", format!("i64 {}", val.code)),
            Kind::Float => self.call("ptr", "zeno_str_float", format!("double {}", val.code)),
            Kind::Bool => self.call("ptr", "zeno_str_bool", format!("i1 {}", val.code)),
            // A call that gave nothing back.
            Kind::None => self.string("none"),
            kind => return Err(cannot(format!("printing {}", described(kind)), span)),
        })
    }

    /// `f"{name} is {age:>3}"`: each piece as text, then all of them joined.
    fn format_string(&mut self, parts: &[FormatPart], span: Span) -> Result<Val, Error> {
        let mut pieces = Vec::new();
        for part in parts {
            pieces.push(match part {
                FormatPart::Text(literal) => self.string(literal),
                FormatPart::Value { value, format } => {
                    let val = self.expr(value)?;
                    match format {
                        Some(spec) => self.formatted(&val, spec, span)?,
                        None => self.text(&val, span)?,
                    }
                }
            });
        }
        Ok(Val::new(
            match pieces.as_slice() {
                [] => self.string(""),
                [only] => only.clone(),
                _ => self.join(&pieces),
            },
            Kind::Str,
        ))
    }

    /// A value with a format after the colon, as in `{x:.2f}` or `{n:08b}`.
    /// The format is read now, by the same function the C backend uses.
    fn formatted(&mut self, val: &Val, spec: &str, span: Span) -> Result<String, Error> {
        let bad = || Error::new(format!("`{spec}` is not a format Zeno understands"), span);
        let emit_c::Format { align, zero, width, precision, kind } = emit_c::read_format(spec).ok_or_else(bad)?;

        let is_number = matches!(val.kind, Kind::Int | Kind::Float);
        let body = match (kind, precision, &val.kind) {
            (Some('f'), _, _) | (None, Some(_), Kind::Float) if is_number => {
                let x = self.double(val);
                self.call("ptr", "zeno_fixed", format!("double {x}, i32 {}", precision.unwrap_or(6)))
            }
            (None, None, _) | (Some('d'), None, Kind::Int) => self.text(val, span)?,
            // A character is handed over as its number: `x` is 120.
            (Some(kind @ ('x' | 'X' | 'b')), None, Kind::Int) => {
                self.call("ptr", "zeno_radix", format!("i64 {}, i8 {}", val.code, kind as u32))
            }
            _ => return Err(bad()),
        };
        if width == 0 {
            return Ok(body);
        }
        // Numbers line up on the right by default, everything else on the left.
        let align = align.unwrap_or(if is_number { '>' } else { '<' });
        let args = format!("ptr {body}, i32 {width}, i8 {}, i1 {}", align as u32, zero && is_number);
        Ok(self.call("ptr", "zeno_pad", args))
    }
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

    /// Build the IR and the runtime with clang, run the program, and
    /// return what it printed, what it wrote as errors, and whether it
    /// finished cleanly. `None` if there is no clang here to try it with.
    fn build_and_run(name: &str, ir: &str) -> Option<(String, String, bool)> {
        let dir = std::env::temp_dir().join(format!("zeno-test-{}-llvm-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let (ir_file, runtime_file, binary) = (dir.join("test.ll"), dir.join("test_runtime.c"), dir.join("test"));
        std::fs::write(&ir_file, ir).expect("the IR file is written");
        std::fs::write(&runtime_file, runtime()).expect("the runtime is written");
        // ZENO_COLLECT_ALWAYS runs the garbage collector before every
        // single allocation, which is the setting most likely to show a
        // value the collector cannot find.
        let built = Command::new("clang")
            .args(["-O2", "-DZENO_COLLECT_ALWAYS", "-Wno-override-module", "-o"])
            .arg(&binary)
            .arg(&ir_file)
            .arg(&runtime_file)
            .arg("-lm")
            .output();
        let Ok(built) = built else {
            eprintln!("no clang found; skipping the compiled half of this test");
            return None;
        };
        let complaints = String::from_utf8_lossy(&built.stderr);
        let complaints: String = complaints.chars().take(1500).collect();
        assert!(built.status.success(), "the IR does not compile:\n{complaints}\n{ir}");
        let ran = Command::new(&binary).output().expect("the program runs");
        let _ = std::fs::remove_dir_all(&dir);
        Some((
            String::from_utf8_lossy(&ran.stdout).into_owned(),
            String::from_utf8_lossy(&ran.stderr).into_owned(),
            ran.status.success(),
        ))
    }

    /// The heart of these tests, as for the C backend: the compiled
    /// program must print exactly what the interpreter and the VM print,
    /// and stop with the same error at the same place.
    fn same_as_interpreter(name: &str, source: &str) {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (interpreted, result) = crate::vm::run_both(&program, source);

        let ir = compile(source).expect("the test program compiles");
        let Some((compiled, errors, finished)) = build_and_run(name, &ir) else {
            return;
        };
        assert_eq!(compiled, interpreted, "\n{source}");
        match &result {
            Ok(()) => assert!(finished, "the compiled program failed: {errors}"),
            Err(error) => {
                assert!(!finished, "the compiled program should have failed");
                let expected =
                    format!("error: {}\n --> test.zeno:{}:{}\n", error.message, error.span.line, error.span.col);
                assert_eq!(errors, expected, "\n{source}");
            }
        }
    }

    #[test]
    fn numbers_match() {
        same_as_interpreter(
            "numbers",
            "print(7 + 2, 7 - 9, 7 * 6, 7 / 2, 7 // 2, -7 // 2, 7 % 3, -7 % 3, 7 % -3, 2 ** 10, 2 ** 0)\n\
             print(1.5 + 2, 3 - 0.5, 2 * 2.5, 7.0 / 2, 7.5 // 2, 7.5 % 2, 2 ** 0.5, 2.0 ** 3)\n\
             print(-(3 + 4), -2.5, - -3, 1e21, 1e-7, 0.1 + 0.2, 100.0, 3.0 * 1e10)\n\
             print(9223372036854775807, -9223372036854775807 - 1)\n\
             print(abs(-5), abs(-2.5), min(3, 1, 2), max(3, 1, 2), min(2.5, 1.5), max(2.5, 1.5))\n\
             print(int(3.9), int(-3.9), int(\"42\"), float(3), float(\"2.5\"), int(7), int(1e30))\n",
        );
    }

    #[test]
    fn comparisons_and_logic_match() {
        same_as_interpreter(
            "logic",
            "print(1 < 2, 2 <= 2, 3 > 4, 4 >= 5, 1 == 1, 1 != 1, 1 == 1.0, 1 < 1.5, 2.5 >= 2)\n\
             print(1 < 2 < 3, 1 < 2 < 2, 3 > 2 > 1 > 0, 1 < 3 > 2, 5 < 4 < 3)\n\
             print(\"apple\" < \"banana\", \"a\" == \"a\", \"a\" != \"b\", \"b\" >= \"a\", true == true, true != false)\n\
             print(true and false, true or false, not true, not (1 > 2), 1 < 2 and 2 < 3 or false)\n\
             function loud(name: str, answer: bool) -> bool {\n\
             \x20   print(f\"asked {name}\")\n\
             \x20   return answer\n\
             }\n\
             function number(name: str, n: int) -> int {\n\
             \x20   print(f\"asked {name}\")\n\
             \x20   return n\n\
             }\n\
             print(loud(\"a\", false) and loud(\"b\", true))\n\
             print(loud(\"c\", true) or loud(\"d\", true))\n\
             print(loud(\"e\", true) and loud(\"f\", false) or loud(\"g\", true))\n\
             print(number(\"h\", 3) < number(\"i\", 2) < number(\"j\", 1))\n\
             print(number(\"k\", 1) < number(\"l\", 2) < number(\"m\", 3))\n",
        );
    }

    #[test]
    fn text_matches() {
        same_as_interpreter(
            "text",
            "let name = \"Zeno\"\n\
             var greeting = \"hello, \" + name\n\
             greeting += \"!\"\n\
             print(greeting, len(greeting), greeting[0], greeting[7], str(12) + str(1.5) + str(true))\n\
             print(name.upper(), name.lower(), \"  padded  \".strip(), name.replace(\"Z\", \"X\"))\n\
             print(name.starts_with(\"Ze\"), name.ends_with(\"x\"))\n\
             print(f\"{name}: {3.14159:.2f} | {42:>6} | {42:<6}| {255:x} {255:X} {5:b} | {7:03}\")\n\
             print(f\"{name:>8} | {name:^8} | {2.5:8.3f} | {10:d} | {true} | {1 + 2}\")\n\
             print(f\"no values\", f\"{1}{2}{3}\", f\"{name}\", \"\")\n\
             print(\"quote \\\" backslash \\\\ tab\\tend\", \"na\u{ef}ve caf\u{e9} \u{2615}\")\n\
             print()\n",
        );
    }

    #[test]
    fn control_flow_matches() {
        same_as_interpreter(
            "control",
            "function classify(n: int) -> str {\n\
             \x20   if n < 0 {\n\
             \x20       return \"negative\"\n\
             \x20   } else if n == 0 {\n\
             \x20       return \"zero\"\n\
             \x20   } else if n < 10 {\n\
             \x20       return \"small\"\n\
             \x20   } else {\n\
             \x20       return \"big\"\n\
             \x20   }\n\
             }\n\
             print(classify(-5), classify(0), classify(7), classify(70))\n\
             function collatz(start: int) -> int {\n\
             \x20   var n = start\n\
             \x20   var steps = 0\n\
             \x20   while n != 1 {\n\
             \x20       if n % 2 == 0 {\n\
             \x20           n = n // 2\n\
             \x20       } else {\n\
             \x20           n = 3 * n + 1\n\
             \x20       }\n\
             \x20       steps += 1\n\
             \x20   }\n\
             \x20   return steps\n\
             }\n\
             print(collatz(27))\n\
             var total = 0\n\
             for i in range(10) {\n\
             \x20   if i == 3 { continue }\n\
             \x20   if i == 8 { break }\n\
             \x20   total += i\n\
             }\n\
             print(total)\n\
             for i in range(2, 5) { print(i) }\n\
             for i in range(10, 0, -3) { print(i) }\n\
             for i in range(0, 10, 4) { print(i) }\n\
             for i in range(5, 5) { print(\"never\") }\n\
             var sum = 0\n\
             for a in range(4) {\n\
             \x20   for b in range(a) {\n\
             \x20       if b == 2 { break }\n\
             \x20       sum += a * b\n\
             \x20   }\n\
             }\n\
             var n = 0\n\
             while true {\n\
             \x20   n += 1\n\
             \x20   if n < 5 { continue }\n\
             \x20   break\n\
             }\n\
             print(sum, n)\n",
        );
    }

    #[test]
    fn functions_match() {
        same_as_interpreter(
            "functions",
            "print(fib(20), power(2), power(2, 5), power(exponent=3, base=4))\n\
             function fib(n: int) -> int {\n\
             \x20   if n < 2 { return n }\n\
             \x20   return fib(n - 1) + fib(n - 2)\n\
             }\n\
             function power(base: int, exponent: int = 2) -> int {\n\
             \x20   var result = 1\n\
             \x20   for i in range(exponent) { result *= base }\n\
             \x20   return result\n\
             }\n\
             function shout(what: str) {\n\
             \x20   print(what.upper() + \"!\")\n\
             \x20   return\n\
             \x20   print(\"never\")\n\
             }\n\
             shout(\"done\")\n\
             print(shout(\"again\"))\n\
             function shadow(n: int) -> int {\n\
             \x20   let x = n\n\
             \x20   var y = 1\n\
             \x20   if n > 0 {\n\
             \x20       let x = n * 2\n\
             \x20       var y = 10\n\
             \x20       y += x\n\
             \x20       return y\n\
             \x20   }\n\
             \x20   return x + y\n\
             }\n\
             print(shadow(4), shadow(0))\n\
             function average(a: float, b: float) -> float { return (a + b) / 2 }\n\
             print(average(1.0, 2.0), average(b=4.0, a=1.5))\n\
             function untyped(a, b) { return a * b + 1 }\n\
             print(untyped(3, 4))\n",
        );
    }

    #[test]
    fn variables_at_the_top_are_shared_with_functions() {
        same_as_interpreter(
            "globals",
            "var counter = 0\n\
             let step = 5\n\
             var log = \"\"\n\
             function bump() -> int {\n\
             \x20   counter += step\n\
             \x20   log += f\"{counter} \"\n\
             \x20   return counter\n\
             }\n\
             bump()\n\
             bump()\n\
             print(counter, bump(), log)\n\
             if counter > 10 {\n\
             \x20   let inside = counter * 2\n\
             \x20   var changing = inside\n\
             \x20   changing += 1\n\
             \x20   print(inside, changing)\n\
             }\n",
        );
        // A function means the built-in `len` when the `len` of the
        // program is declared below it (decision 3.6d).
        same_as_interpreter("below", "function size(text: str) -> int { return len(text) }\nlet len = 5\nprint(size(\"abc\"), len)\n");
    }

    #[test]
    fn text_survives_the_collector() {
        // Text that is held only by a value in the IR, or by a global,
        // while a great deal more is made and thrown away.
        same_as_interpreter(
            "collector",
            "var kept = \"start\"\n\
             function grow(times: int) -> str {\n\
             \x20   var text = \"\"\n\
             \x20   for i in range(times) {\n\
             \x20       let piece = f\"{i},\"\n\
             \x20       text += piece\n\
             \x20       if i % 50 == 0 { kept = text.upper() + \"!\" }\n\
             \x20   }\n\
             \x20   return text\n\
             }\n\
             let made = grow(120)\n\
             var again = \"\"\n\
             for i in range(300) {\n\
             \x20   again = str(i) + made[i % 10]\n\
             }\n\
             print(len(made), len(kept), made[0], kept[5], again)\n",
        );
    }

    #[test]
    fn errors_stop_the_program_the_same_way() {
        let programs = [
            "print(\"before\")\nlet big = 9223372036854775807\nprint(big + 1)\n",
            "let big = 9223372036854775807\nprint(big * 2)\n",
            "var low = -9223372036854775807\nlow -= 2\nprint(low)\n",
            "let low = -9223372036854775807 - 1\nprint(-low)\n",
            "let low = -9223372036854775807 - 1\nprint(abs(low))\n",
            "let zero = 0\nprint(1 // zero)\n",
            "let zero = 0\nprint(1 % zero)\n",
            "let zero = 0\nprint(1 / zero)\n",
            "let zero = 0.0\nprint(1.5 // zero)\n",
            "print(2 ** -1)\n",
            "print(int(\"abc\"))\n",
            "print(float(\"x1\"))\n",
            "let text = \"abc\"\nprint(text[5])\n",
            "for i in range(0, 10, 0) { print(i) }\n",
            "function deep(n: int) -> int {\n    if n == 0 { return 1 // n }\n    return deep(n - 1)\n}\nprint(\"start\")\nprint(deep(3))\n",
        ];
        for (index, program) in programs.iter().enumerate() {
            same_as_interpreter(&format!("error-{index}"), program);
        }
    }

    #[test]
    fn the_examples_it_can_compile_match() {
        same_as_interpreter("hello", include_str!("../examples/hello.zeno"));
        same_as_interpreter("primes", include_str!("../examples/primes.zeno"));
    }

    #[test]
    fn the_runtime_can_be_linked_to() {
        let runtime = runtime();
        assert!(!runtime.lines().any(|line| line.starts_with("static ")));
        // Everything the IR may call is there to be found.
        for (name, _) in DECLARED {
            if name.starts_with("zeno_") {
                assert!(runtime.contains(&format!(" {name}(")) || runtime.contains(&format!("*{name}(")), "{name}");
            }
        }
    }

    #[test]
    fn the_ir_for_a_small_function() {
        // A `let` and a parameter are values. A `var` has a slot.
        let ir = compile(
            "function scale(n: int, by: float) -> float {\n\
             \x20   let twice = by * 2.0\n\
             \x20   var result = twice\n\
             \x20   if n > 0 { result = result * 0.5 }\n\
             \x20   return result\n\
             }\n\
             print(scale(1, 1.5))\n",
        )
        .expect("it compiles");
        let function = "define internal double @z_scale(i64 %arg.n, double %arg.by) {\n\
                        entry:\n\
                        \x20 %var.result = alloca double\n\
                        \x20 %t1 = fmul double %arg.by, 2.0\n\
                        \x20 store double %t1, ptr %var.result\n\
                        \x20 %t2 = icmp sgt i64 %arg.n, 0\n\
                        \x20 br i1 %t2, label %then1, label %end1\n\
                        \n\
                        then1:\n\
                        \x20 %t3 = load double, ptr %var.result\n\
                        \x20 %t4 = fmul double %t3, 0.5\n\
                        \x20 store double %t4, ptr %var.result\n\
                        \x20 br label %end1\n\
                        \n\
                        end1:\n\
                        \x20 %t5 = load double, ptr %var.result\n\
                        \x20 ret double %t5\n\
                        }\n";
        assert!(ir.contains(function), "{ir}");
    }

    #[test]
    fn what_is_not_compiled_yet_says_so() {
        let refused = [
            ("class Point {\n    x: int\n}\nprint(Point(1).x)\n", "a class"),
            ("let items = [1, 2, 3]\nprint(len(items))\n", "a list"),
            ("enum Colour {\n    Red\n    Green\n}\nprint(Colour.Red)\n", "an enum"),
            ("try {\n    print(1 // 0)\n} except ZeroDivisionError {\n    print(\"caught\")\n}\n", "`try`"),
            ("let twice = function(n: int) -> int { return n * 2 }\nprint(twice(2))\n", "a function used as a value"),
            ("function outer() -> int {\n    function inner() -> int { return 1 }\n    return inner()\n}\nprint(outer())\n", "a function written inside a function or a block"),
            ("let wait = 5 ms\nprint(wait)\n", "a number with a unit"),
        ];
        for (source, what) in refused {
            let message = compile(source).expect_err("it should be refused");
            assert_eq!(
                message,
                format!("`zeno build --llvm` cannot compile {what} yet. `zeno build` without `--llvm` can"),
                "\n{source}"
            );
        }
    }
}
