//! Writes a Zeno program as Python: `zeno build --target py`.
//!
//! The program comes ready from lower.rs, as it does for JavaScript
//! (emit_js.rs), and every value is a value of runtime.py, which is put
//! in front of the program, so the `.py` file runs by itself:
//!
//! ```text
//! python3 hello.py
//! ```
//!
//! Two things differ from JavaScript, because Python is not JavaScript:
//!
//! - A function made inside an expression (`fn(x) { ... }` passed to
//!   `map`) cannot be written there, since a Python `lambda` holds only one
//!   expression. It is written as a `def` on the lines just before the
//!   statement that uses it. Making a function does nothing that can be
//!   seen, so making it a moment earlier changes nothing.
//! - A variable that an inner function uses is kept in a `Cell` (lower.rs
//!   marks it), and the inner function is handed the cell when it is made,
//!   as a parameter of its own: `def f(x, *, total=total)`. Python's own
//!   closures would let every round of a loop share one variable, where
//!   Zeno gives each round its own.
//!
//! The runtime's names are reached through `Z.`, the file's own module,
//! so that no name in the program can hide one of them.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId};

/// The runtime every program built for Python carries.
pub const RUNTIME: &str = include_str!("runtime.py");

/// The whole `.py` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {source}, written in Python by `zeno build --target py`.");
    let _ = writeln!(out, "# Run it with `python3`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    out.push_str("\n\nZ = _sys.modules[__name__]\n\n\n# The program.\n");
    let mut printer = Printer { program, out: String::new(), pending: String::new(), depth: 1, functions: 0 };
    printer.function_body(&program.main);
    out.push_str("def main():\n");
    out.push_str(&printer.out);
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    let _ = writeln!(out, "\n\nZ.start([{}], main)", files.join(", "));
    out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    /// The `def`s of functions made in the statement being written, which
    /// go on the lines before it.
    pending: String,
    depth: usize,
    /// How many functions have been named, so that each has its own name.
    functions: usize,
}

/// A Python string literal.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{code:04x}"))
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("    ");
    }
}

impl<'a> Printer<'a> {
    fn name(&self, id: VarId) -> &'a str {
        &self.program.vars[id].ident
    }

    fn is_cell(&self, id: VarId) -> bool {
        self.program.vars[id].cell
    }

    /// The variable as it is read: `x`, or `x.v` for one in a cell.
    fn var(&self, id: VarId) -> String {
        if self.is_cell(id) { format!("{}.v", self.name(id)) } else { self.name(id).to_string() }
    }

    /// One line, after the functions it uses.
    fn line(&mut self, text: &str) {
        let pending = std::mem::take(&mut self.pending);
        self.out.push_str(&pending);
        indent(&mut self.out, self.depth);
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// The statements of a block, after its variables.
    fn block_body(&mut self, block: &Block) {
        let plain: Vec<&str> = block.decls.iter().filter(|id| !self.is_cell(**id)).map(|id| self.name(*id)).collect();
        if !plain.is_empty() {
            let line = format!("{} = None", plain.join(" = "));
            self.line(&line);
        }
        for id in &block.decls {
            if self.is_cell(*id) {
                let line = format!("{} = Z.Cell()", self.name(*id));
                self.line(&line);
            }
        }
        for statement in &block.stmts {
            self.statement(statement);
        }
    }

    /// A block one step in, with `pass` if nothing else is in it.
    fn nested(&mut self, block: &Block) {
        self.depth += 1;
        let before = self.out.len();
        self.block_body(block);
        if self.out.len() == before {
            self.line("pass");
        }
        self.depth -= 1;
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Set(id, value) => {
                let line = format!("{} = {}", self.var(*id), self.expr(value));
                self.line(&line);
            }
            Stmt::SetField { object, name, value, line } => {
                let text = format!("Z.set_field({}, {}, {}, {line})", self.expr(object), string(name), self.expr(value));
                self.line(&text);
            }
            Stmt::SetIndex { object, index, value, line } => {
                let text = format!("Z.set_index({}, {}, {}, {line})", self.expr(object), self.expr(index), self.expr(value));
                self.line(&text);
            }
            Stmt::Expr(expr) => {
                let line = self.expr(expr);
                self.line(&line);
            }
            Stmt::If { condition, then, otherwise } => {
                let line = format!("if {}:", self.cond(condition));
                self.line(&line);
                self.nested(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
                    // `elif`, for an `else` that holds only an `if` whose
                    // test makes no function.
                    if let ([Stmt::If { condition, then, otherwise: next }], true) = (&block.stmts[..], block.decls.is_empty()) {
                        let test = self.cond(condition);
                        if self.pending.is_empty() {
                            let line = format!("elif {test}:");
                            self.line(&line);
                            self.nested(then);
                            otherwise = next.as_ref();
                            continue;
                        }
                        // The functions must be made where the test is.
                        let pending = std::mem::take(&mut self.pending);
                        self.line("else:");
                        self.depth += 1;
                        self.pending = reindent(&pending, 1);
                        let line = format!("if {test}:");
                        self.line(&line);
                        self.nested(then);
                        if let Some(rest) = next {
                            self.line("else:");
                            self.nested(rest);
                        }
                        self.depth -= 1;
                        otherwise = None;
                    } else {
                        self.line("else:");
                        self.nested(block);
                        otherwise = None;
                    }
                }
            }
            Stmt::While { condition, body } => {
                let line = format!("while {}:", self.cond(condition));
                self.line(&line);
                self.nested(body);
            }
            Stmt::For { var, second, items, body } => {
                let items = self.expr(items);
                let first = if self.is_cell(*var) { "z_item".to_string() } else { self.name(*var).to_string() };
                let names = match second {
                    Some(second) if self.is_cell(*second) => format!("{first}, z_second"),
                    Some(second) => format!("{first}, {}", self.name(*second)),
                    None => first,
                };
                let line = format!("for {names} in {items}:");
                self.line(&line);
                // A variable in a cell gets a new cell each round.
                self.depth += 1;
                if self.is_cell(*var) {
                    let line = format!("{} = Z.Cell(z_item)", self.name(*var));
                    self.line(&line);
                }
                if let Some(second) = second {
                    if self.is_cell(*second) {
                        let line = format!("{} = Z.Cell(z_second)", self.name(*second));
                        self.line(&line);
                    }
                }
                self.depth -= 1;
                self.nested(body);
            }
            Stmt::Break => self.line("break"),
            Stmt::Continue => self.line("continue"),
            Stmt::Return(value) => {
                let line = format!("return {}", self.expr(value));
                self.line(&line);
            }
            Stmt::Raise { value, line } => {
                let text = format!("Z.raise_({}, {line})", self.expr(value));
                self.line(&text);
            }
            Stmt::Try { body, error, handler } => {
                self.line("try:");
                self.nested(body);
                // A mistake in the program, or one of Python's own, goes on
                // past every `except`.
                if self.is_cell(*error) {
                    self.line("except Z.Raise as z_error:");
                    self.depth += 1;
                    let line = format!("{} = Z.Cell(z_error)", self.name(*error));
                    self.line(&line);
                    self.depth -= 1;
                } else {
                    let line = format!("except Z.Raise as {}:", self.name(*error));
                    self.line(&line);
                }
                self.nested(handler);
            }
            Stmt::Reraise(error) => {
                let line = format!("raise {}", self.var(*error));
                self.line(&line);
            }
            Stmt::Fatal { message, line } => {
                let text = format!("Z.fatal({}, {line})", string(message));
                self.line(&text);
            }
            Stmt::NoMatch { value, line } => {
                let text = format!("Z.no_match({}, {line})", self.expr(value));
                self.line(&text);
            }
            Stmt::Block(block) => self.block_body(block),
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) -> String {
        exprs.iter().map(|expr| self.expr(expr)).collect::<Vec<_>>().join(", ")
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) => self.var(*id),
            Expr::Int(n) => n.to_string(),
            Expr::Float(x) => float(*x),
            Expr::Str(text) | Expr::Text(text) => string(text),
            Expr::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Expr::None => "None".to_string(),
            Expr::Missing => "Z.MISSING".to_string(),
            Expr::FromCond(test) => self.cond(test),
            Expr::Builtin(name) => format!("Z.builtin({})", string(name)),
            Expr::Module(name) => format!("Z.module({})", string(name)),
            Expr::CallBuiltin { name, args, line } => {
                format!("Z.b.{}([{}], {line})", name.replace('.', "_"), self.exprs(args))
            }
            Expr::Call { callee, args, line } => format!("Z.call({}, [{}], {line})", self.expr(callee), self.exprs(args)),
            Expr::CallNamed { callee, args, names, named, line } => {
                let names: Vec<String> = names.iter().map(|name| string(name)).collect();
                format!("Z.call_named({}, [{}], [{}], [{}], {line})", self.expr(callee), self.exprs(args), names.join(", "), self.exprs(named))
            }
            Expr::CallDirect { function, args, .. } => format!("{}({})", self.var(*function), self.exprs(args)),
            Expr::CallMethod { object, name, args, names, named, line } => {
                if names.is_empty() {
                    format!("Z.call_method({}, {}, [{}], {line})", self.expr(object), string(name), self.exprs(args))
                } else {
                    let names: Vec<String> = names.iter().map(|name| string(name)).collect();
                    format!(
                        "Z.call_method_named({}, {}, [{}], [{}], [{}], {line})",
                        self.expr(object),
                        string(name),
                        self.exprs(args),
                        names.join(", "),
                        self.exprs(named)
                    )
                }
            }
            Expr::Rt { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("Z.{}({args})", step(name))
            }
            Expr::Count(n) => n.to_string(),
            Expr::Array(items) => format!("[{}]", self.exprs(items)),
            Expr::Caught(id) => self.var(*id),
            Expr::Func(function) => self.function(function),
            Expr::Class(class) => {
                let fields: Vec<String> = class.fields.iter().map(|(name, _)| string(name)).collect();
                let defaults: Vec<String> = class
                    .fields
                    .iter()
                    .map(|(_, default)| match default {
                        Some(function) => self.function(function),
                        None => "None".to_string(),
                    })
                    .collect();
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("{}: {}", string(&method.name), self.function(method))).collect();
                format!("Z.class_({}, [{}], [{}], {{{}}})", string(&class.name), fields.join(", "), defaults.join(", "), methods.join(", "))
            }
            Expr::Enum(def) => {
                let cases: Vec<String> = def
                    .cases
                    .iter()
                    .map(|(name, fields)| {
                        let fields: Vec<String> = fields.iter().map(|field| string(field)).collect();
                        format!("[{}, [{}]]", string(name), fields.join(", "))
                    })
                    .collect();
                format!("Z.enum({}, [{}])", string(&def.name), cases.join(", "))
            }
        }
    }

    /// The body of a function: its parameters put in cells where they must
    /// be, their defaults, then its statements.
    fn function_body(&mut self, function: &Func) {
        for param in &function.params {
            if self.is_cell(param.var) {
                let name = self.name(param.var);
                let line = format!("{name} = Z.Cell({name})");
                self.line(&line);
            }
        }
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if {} is Z.MISSING:", self.var(param.var));
                self.line(&line);
                self.nested(default);
            }
        }
        self.block_body(&function.body);
    }

    /// A Zeno function: a `def` on the lines before the statement, and in
    /// the statement the function with its Zeno name and parameters on it.
    fn function(&mut self, function: &Func) -> String {
        self.functions += 1;
        let ident = format!("z_f{}", self.functions);
        let mut params: Vec<String> = function.params.iter().map(|param| self.name(param.var).to_string()).collect();
        if !function.captures.is_empty() {
            params.push("*".to_string());
            for id in &function.captures {
                let name = self.name(*id);
                params.push(format!("{name}={name}"));
            }
        }
        let mut inner =
            Printer { program: self.program, out: String::new(), pending: String::new(), depth: self.depth + 1, functions: self.functions };
        inner.function_body(function);
        if inner.out.is_empty() {
            inner.line("pass");
        }
        self.functions = inner.functions;
        let mut text = String::new();
        indent(&mut text, self.depth);
        let _ = writeln!(text, "def {ident}({}):", params.join(", "));
        text.push_str(&inner.out);
        self.pending.push_str(&text);
        let names: Vec<String> = function.params.iter().map(|param| string(&param.name)).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "True" } else { "False" }).collect();
        format!("Z.fn({}, [{}], [{}], {ident})", string(&function.name), names.join(", "), defaults.join(", "))
    }

    fn cond(&mut self, test: &Cond) -> String {
        match test {
            Cond::True => "True".to_string(),
            Cond::Truth(Expr::FromCond(inner), _) => self.cond(inner),
            Cond::Truth(value, line) => format!("Z.truth({}, {line})", self.expr(value)),
            Cond::Not(inner) => format!("not {}", self.cond_atom(inner)),
            Cond::And(a, b) => format!("({} and {})", self.cond(a), self.cond(b)),
            Cond::Or(a, b) => format!("({} or {})", self.cond(a), self.cond(b)),
            Cond::Test { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("Z.{}({args})", step(name))
            }
            Cond::IsMissing(id) => format!("({} is Z.MISSING)", self.var(*id)),
        }
    }

    /// A test that can stand after `not` as it is.
    fn cond_atom(&mut self, test: &Cond) -> String {
        let text = self.cond(test);
        match test {
            Cond::Not(_) => format!("({text})"),
            _ => text,
        }
    }
}

/// Lines moved `steps` levels further in.
fn reindent(text: &str, steps: usize) -> String {
    let mut out = String::new();
    for line in text.lines() {
        indent(&mut out, steps);
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The name a step of lower.rs has in runtime.py, where some of Python's
/// own words are taken.
fn step(name: &str) -> &str {
    match name {
        "in" => "in_",
        "pow" => "pow_",
        "list" => "list_",
        "dict" => "dict_",
        "format" => "format_",
        other => other,
    }
}

/// A float literal that Python reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "Z._math.nan".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "Z._math.inf" } else { "(-Z._math.inf)" }.to_string()
    } else {
        lower::float_literal(x)
    }
}
