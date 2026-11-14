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
//! This first slice compiles numbers, bools, strings, quantities, functions
//! and control flow. Classes, lists, enums, `match`, `try` and functions as
//! values are refused with a message, and are the next slices.

use std::collections::HashMap;
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
        scopes: vec![HashMap::new()],
        used: HashMap::new(),
        lines: Vec::new(),
        indent: 1,
        temps: 0,
        globals: Vec::new(),
        returns: None,
    };

    // Functions can be called above where they are written, so learn about
    // all of them before compiling anything.
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

    // Then each function. They are compiled after `main` so that every
    // top-level variable is known by the time a function body mentions one.
    let mut prototypes = Vec::new();
    let mut definitions = Vec::new();
    for statement in &program.statements {
        if let StmtKind::Function(decl) = &statement.kind {
            let (prototype, body) = emitter.function(decl, statement.span)?;
            prototypes.push(format!("{prototype};"));
            definitions.push(format!("{prototype} {{\n{body}}}\n"));
        }
    }

    let mut c = String::from(RUNTIME);
    c.push('\n');
    for global in &emitter.globals {
        c += global;
        c.push('\n');
    }
    if !emitter.globals.is_empty() {
        c.push('\n');
    }
    for prototype in &prototypes {
        c += prototype;
        c.push('\n');
    }
    if !prototypes.is_empty() {
        c.push('\n');
    }
    for definition in &definitions {
        c += definition;
        c.push('\n');
    }
    c += "int main(void) {\n";
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

struct Emitter<'a> {
    types: &'a Types,
    /// The functions of the program, by their Zeno names.
    functions: HashMap<String, FunctionEntry>,
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
    /// Declarations of the C globals.
    globals: Vec<String>,
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
fn c_type(kind: &Kind, span: Span) -> Result<&'static str, Error> {
    Ok(match kind {
        Kind::Int => "int64_t",
        Kind::Float | Kind::Quantity(_) => "double",
        Kind::Bool => "bool",
        Kind::Str => "const char *",
        Kind::None => return Err(cannot("a value that is `none`", span)),
        Kind::Other(name) => return Err(cannot(format!("a value of type {name}"), span)),
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
        self.line(format!("{} {name} = {};", c_type(&val.kind, span)?, val.code));
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

    fn register(&mut self, decl: &Arc<Function>, span: Span) -> Result<(), Error> {
        let Some(kind) = self.types.function(decl).cloned() else {
            return Err(cannot(format!("the function `{}`", decl.name), span));
        };
        let c_name = self.c_name(&decl.name);
        self.functions.insert(decl.name.clone(), FunctionEntry { c_name, decl: decl.clone(), kind });
        Ok(())
    }

    /// Compile one function. Returns its first line, such as
    /// `static int64_t z_add(int64_t z_a, int64_t z_b)`, and its body.
    fn function(&mut self, decl: &Arc<Function>, span: Span) -> Result<(String, String), Error> {
        let entry = &self.functions[&decl.name];
        let (c_name, kind) = (entry.c_name.clone(), entry.kind.clone());

        // A function sees the top-level variables and its own, nothing else.
        self.scopes.truncate(1);
        self.scopes.push(HashMap::new());
        self.lines.clear();
        self.indent = 1;

        let mut params = Vec::new();
        for (param, param_kind) in decl.params.iter().zip(&kind.params) {
            let param_type = c_type(param_kind, span)?;
            let param_name = self.declare(&param.name, param_kind.clone());
            params.push(format!("{param_type} {param_name}"));
        }
        let return_type = match &kind.returns {
            Kind::None => "void",
            other => c_type(other, span)?,
        };
        let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };

        self.returns = Some(kind.returns.clone());
        for statement in &decl.body {
            self.stmt(statement)?;
        }
        self.returns = None;
        self.scopes.pop();

        let body = render(&std::mem::take(&mut self.lines));
        Ok((format!("static {return_type} {c_name}({params})"), body))
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
        match &statement.kind {
            StmtKind::Let { name, value, .. } | StmtKind::Const { name, value, .. } => {
                let val = self.expr(value)?;
                let c_type = c_type(&val.kind, value.span)?;
                // At the top of the file, outside any function or block, a
                // variable is one that functions can see: a C global.
                let global = self.scopes.len() == 1 && self.returns.is_none();
                let c_name = self.declare(name, val.kind);
                if global {
                    self.globals.push(format!("static {c_type} {c_name};"));
                    self.line(format!("{c_name} = {};", val.code));
                } else {
                    self.line(format!("{c_type} {c_name} = {};", val.code));
                }
            }

            StmtKind::Assign { target, op, value } => {
                let ExprKind::Name(name) = &target.kind else {
                    return Err(cannot("an assignment to anything but a variable", target.span));
                };
                let Some(variable) = self.lookup(name) else {
                    return Err(cannot(format!("an assignment to `{name}`"), target.span));
                };
                let c_name = variable.c_name.clone();
                let val = match op {
                    None => self.expr(value)?,
                    // `x += 1` is `x = x + 1`.
                    Some(op) => {
                        let mut vals = self.sequence(&[target, value])?;
                        let right = vals.pop().expect("two values");
                        let left = vals.pop().expect("two values");
                        self.arithmetic(*op, left, right, span)?
                    }
                };
                self.line(format!("{c_name} = {};", val.code));
            }

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

            StmtKind::For { variable, iterable, body } => self.for_range(variable, iterable, body)?,

            StmtKind::Return(value) => {
                let Some(returns) = self.returns.clone() else {
                    return Err(cannot("`return` outside a function", span));
                };
                match value {
                    Some(value) if returns != Kind::None => {
                        let val = self.expr(value)?;
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
            StmtKind::Function(_) if self.scopes.len() == 1 && self.returns.is_none() => {}
            StmtKind::Function(_) => return Err(cannot("a function inside a function or block", span)),

            StmtKind::Import { module, .. } => {
                if !matches!(module.join(".").as_str(), "math" | "time") {
                    return Err(cannot(format!("the module `{}`", module.join(".")), span));
                }
            }

            StmtKind::Class(_) => return Err(cannot("classes", span)),
            StmtKind::Enum(_) => return Err(cannot("enums", span)),
            StmtKind::Match { .. } => return Err(cannot("`match`", span)),
            StmtKind::Try { .. } => return Err(cannot("`try`", span)),
            StmtKind::Raise(_) => return Err(cannot("`raise`", span)),
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

    /// `for i in range(...)`. Without lists, a range is the only thing there
    /// is to loop over, and it becomes a plain counting loop: no list of
    /// numbers is ever built.
    fn for_range(&mut self, variable: &str, iterable: &Expr, body: &[Stmt]) -> Result<(), Error> {
        let span = iterable.span;
        let args = match &iterable.kind {
            ExprKind::Call { callee, args }
                if matches!(&callee.kind, ExprKind::Name(name) if name == "range" && self.lookup(name).is_none())
                    && args.iter().all(|arg| arg.name.is_none()) =>
            {
                args
            }
            _ => return Err(cannot("a `for` loop over anything but `range(...)`", span)),
        };
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
        self.block(body)?;
        self.line("}");
        self.scopes.pop();
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
        let mut done: Vec<Val> = Vec::new();
        for expr in exprs {
            let mark = self.lines.len();
            let next = self.expr(expr)?;
            let fails = next.fails || next.pre_fails;
            let calls = next.calls || next.pre_calls;

            // Temporaries go before whatever statements `next` wrote out.
            let mut position = mark;
            for earlier in &mut done {
                let disturbed = (calls && (earlier.reads || earlier.fails || earlier.calls))
                    || (fails && (earlier.fails || earlier.calls))
                    || (next.reads && earlier.calls);
                if disturbed {
                    self.temps += 1;
                    let name = format!("t{}", self.temps);
                    let c_type = c_type(&earlier.kind, expr.span)?;
                    self.lines.insert(position, (self.indent, format!("{c_type} {name} = {};", earlier.code)));
                    position += 1;
                    earlier.pre_fails |= earlier.fails;
                    earlier.pre_calls |= earlier.calls;
                    earlier.code = name;
                    earlier.reads = false;
                    earlier.fails = false;
                    earlier.calls = false;
                }
            }
            done.push(next);
        }
        Ok(done)
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
                if matches!(op, BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq)
                {
                    let code = comparison(*op, &left, &right, span)?;
                    return Ok(Val::from(code, Kind::Bool, &[&left, &right]));
                }
                self.arithmetic(*op, left, right, span)
            }
            ExprKind::Compare { first, rest } => self.chain(first, rest, span),

            ExprKind::FormatStr(parts) => self.format_string(parts, span),
            ExprKind::Call { callee, args } => self.call(callee, args, span),

            ExprKind::Field { object, name } => match (&object.kind, name.as_str()) {
                (ExprKind::Name(module), "pi") if module == "math" && self.lookup(module).is_none() => {
                    Ok(Val::plain(format!("{:?}", std::f64::consts::PI), Kind::Float))
                }
                // `(3 s).ms`: the same quantity in another unit.
                _ if units::is_unit(name) => {
                    let val = self.expr(object)?;
                    let scale = match &val.kind {
                        Kind::Quantity(unit) => units::convert(unit, name),
                        _ => None,
                    };
                    match scale {
                        Some(scale) => Ok(Val::from(scaled(&val.code, scale), Kind::Quantity(name.clone()), &[&val])),
                        None => Err(cannot(format!("`.{name}` on this"), span)),
                    }
                }
                _ => Err(cannot(format!("`.{name}`"), span)),
            },

            ExprKind::List(_) | ExprKind::ListComp { .. } | ExprKind::Index { .. } => Err(cannot("lists", span)),
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
                tests.push(comparison(*op, &vals[index], &vals[index + 1], expr.span)?);
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
            let test = comparison(*op, &previous, &next, expr.span)?;
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
                if args.iter().any(|arg| arg.name.is_some()) {
                    return Err(cannot("named arguments to a built-in", span));
                }
                let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
                let vals = self.sequence(&exprs)?;
                self.builtin(name, vals, span)
            }
            // `math.sqrt(x)`
            ExprKind::Field { object, name } => match &object.kind {
                ExprKind::Name(module) if module == "math" && self.lookup(module).is_none() => {
                    let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
                    let vals = self.sequence(&exprs)?;
                    let [x] = vals.as_slice() else {
                        return Err(cannot(format!("`math.{name}` called this way"), span));
                    };
                    let d = x.double();
                    Ok(match name.as_str() {
                        "sqrt" => Val::from(format!("zeno_sqrt({d}, {})", at(span)), Kind::Float, &[x]).failing(),
                        "floor" => Val::from(format!("zeno_whole(floor({d}))"), Kind::Int, &[x]),
                        "ceil" => Val::from(format!("zeno_whole(ceil({d}))"), Kind::Int, &[x]),
                        "sin" => Val::from(format!("sin({d})"), Kind::Float, &[x]),
                        "cos" => Val::from(format!("cos({d})"), Kind::Float, &[x]),
                        _ => return Err(cannot(format!("`math.{name}`"), span)),
                    })
                }
                _ => Err(cannot(format!("the method `{name}`"), span)),
            },
            _ => Err(cannot("this kind of call", span)),
        }
    }

    /// A call to one of the program's own functions.
    fn call_function(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let entry = &self.functions[name];
        let (c_name, decl, kind) = (entry.c_name.clone(), entry.decl.clone(), entry.kind.clone());

        // Which parameter each argument is for. The arguments are worked out
        // in the order they are written, even when names put them elsewhere.
        let mut slots = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let slot = match &arg.name {
                None => index,
                Some(name) => match decl.params.iter().position(|param| &param.name == name) {
                    Some(slot) => slot,
                    None => return Err(cannot(format!("the argument `{name}`"), span)),
                },
            };
            slots.push(slot);
        }
        let exprs: Vec<&Expr> = args.iter().map(|arg| &arg.value).collect();
        let vals = self.sequence(&exprs)?;

        let mut codes: Vec<Option<String>> = vec![None; decl.params.len()];
        for (slot, val) in slots.iter().zip(&vals) {
            let Some(code) = codes.get_mut(*slot) else {
                return Err(cannot("this many arguments", span));
            };
            *code = Some(val.code.clone());
        }
        // An argument that was left out takes the parameter's default.
        for (code, param) in codes.iter_mut().zip(&decl.params) {
            if code.is_none() {
                let default = match &param.default {
                    Some(default) if is_literal(default) => self.expr(default)?,
                    _ => return Err(cannot("a default value that is not a plain literal", span)),
                };
                *code = Some(default.code);
            }
        }
        let codes: Vec<String> = codes.into_iter().flatten().collect();
        let call = format!("{c_name}({})", codes.join(", "));

        let parts: Vec<&Val> = vals.iter().collect();
        if kind.returns == Kind::None {
            // Nothing comes back, so the call is a statement by itself.
            self.line(format!("{call};"));
            let mut val = Val::from(String::new(), Kind::None, &parts);
            val.pre_fails = true;
            val.pre_calls = true;
            return Ok(val);
        }
        let mut val = Val::from(call, kind.returns, &parts);
        val.fails = true;
        val.calls = true;
        Ok(val)
    }

    fn builtin(&mut self, name: &str, vals: Vec<Val>, span: Span) -> Result<Val, Error> {
        let parts: Vec<&Val> = vals.iter().collect();
        let one = || match vals.as_slice() {
            [val] => Ok(val),
            _ => Err(cannot(format!("`{name}` called this way"), span)),
        };
        Ok(match name {
            "print" => {
                let mut texts = vec![vals.len().to_string()];
                for val in &vals {
                    texts.push(text(val, span)?);
                }
                self.line(format!("zeno_print({});", texts.join(", ")));
                let mut val = Val::from(String::new(), Kind::None, &parts);
                val.pre_calls = true;
                val
            }
            "str" => Val::from(text(one()?, span)?, Kind::Str, &parts),
            "len" => match one()? {
                val if val.kind == Kind::Str => Val::from(format!("zeno_len({})", val.code), Kind::Int, &parts),
                _ => return Err(cannot("`len` of anything but a str", span)),
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
        let exprs: Vec<&Expr> = parts
            .iter()
            .filter_map(|part| match part {
                FormatPart::Value { value, .. } => Some(value),
                FormatPart::Text(_) => None,
            })
            .collect();
        let vals = self.sequence(&exprs)?;

        let mut pieces = Vec::new();
        let mut next = vals.iter();
        for part in parts {
            match part {
                FormatPart::Text(literal) => pieces.push(c_string(literal)),
                FormatPart::Value { format, .. } => {
                    let val = next.next().expect("one value for each `{}`");
                    pieces.push(match format {
                        Some(spec) => formatted(val, spec, span)?,
                        None => text(val, span)?,
                    });
                }
            }
        }
        let used: Vec<&Val> = vals.iter().collect();
        let code = match pieces.as_slice() {
            [] => "\"\"".to_string(),
            [only] => only.clone(),
            _ => format!("zeno_join({}, {})", pieces.len(), pieces.join(", ")),
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

/// The C expression that turns a value into the text `print` shows for it.
fn text(val: &Val, span: Span) -> Result<String, Error> {
    Ok(match &val.kind {
        Kind::Str => val.code.clone(),
        Kind::Int => format!("zeno_str_int({})", val.code),
        Kind::Float => format!("zeno_str_float({})", val.code),
        Kind::Bool => format!("zeno_str_bool({})", val.code),
        Kind::Quantity(unit) => format!("zeno_str_quantity({}, {})", val.code, c_string(unit)),
        Kind::None => return Err(cannot("printing a call that returns nothing", span)),
        Kind::Other(name) => return Err(cannot(format!("printing a value of type {name}"), span)),
    })
}

/// A value with a format after the colon, as in `{x:.2f}` or `{n:08b}`.
///
/// The format is read here, while compiling, so the compiled program does
/// not have to read it again every time the line runs. The rules are the
/// ones `format` in `builtins.rs` follows.
fn formatted(val: &Val, spec: &str, span: Span) -> Result<String, Error> {
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
        (None, None, _) | (Some('d'), None, Kind::Int) => text(val, span)?,
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
        assert!(built.status.success(), "the C does not compile:\n{}", String::from_utf8_lossy(&built.stderr));
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
        assert_eq!(
            compile("let xs = [1, 2, 3]").unwrap_err(),
            "`zeno build` cannot compile lists yet. `zeno run` can still run this program"
        );
        assert_eq!(
            compile("class Point {\n x: int\n}").unwrap_err(),
            "`zeno build` cannot compile classes yet. `zeno run` can still run this program"
        );
    }

    #[test]
    fn simple_code_stays_simple() {
        let c = compile("function area(w: float, h: float) -> float { return w * h }\nlet a = area(2.0, 3.0)").unwrap();
        assert!(c.contains("static double z_area(double z_w, double z_h) {\n    return (z_w * z_h);\n}"), "{c}");
        assert!(c.contains("    z_a = z_area(2.0, 3.0);\n"), "{c}");
    }
}
