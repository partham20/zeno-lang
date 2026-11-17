//! Writes a Zeno program as Ruby: `zeno build --target ruby`.
//!
//! The program comes ready from lower.rs, as it does for JavaScript
//! (emit_js.rs), and every value is a value of runtime.rb, which is put
//! in front of the program, so the `.rb` file runs by itself:
//!
//! ```text
//! ruby hello.rb
//! ```
//!
//! A Zeno function is a Ruby lambda, which shares the variables around
//! it, as a Zeno function does. Three things are Ruby's own:
//!
//! - Only a lambda or a block has variables of its own in Ruby: an
//!   `if` or a `while` has none. So a `for` is a block handed to
//!   `Z.each`, and each round has its own variables, as in Zeno. A
//!   `while` stays a `while`, unless a function made in its body uses a
//!   variable of the body: then it is a block handed to `Z.loop`, so
//!   that the functions of two rounds do not share one variable.
//! - A name that begins with a capital letter is a constant in Ruby,
//!   and a constant cannot be given a value inside a lambda. `Point`
//!   is written `z_Point`.
//! - `break`, `next` and `return` leave a block, a `begin` and a lambda
//!   as Zeno's `break`, `continue` and `return` do, so a `try` is a
//!   plain `begin` and `rescue`.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId, VarKind};

/// The runtime every program built for Ruby carries.
pub const RUNTIME: &str = include_str!("runtime.rb");

/// The whole `.rb` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    // Ruby reads this first line as an order: every piece of text
    // written in the file is one that cannot change.
    let _ = writeln!(out, "# frozen_string_literal: true");
    let _ = writeln!(out);
    let _ = writeln!(out, "# {source}, written in Ruby by `zeno build --target ruby`.");
    let _ = writeln!(out, "# Run it with `ruby` (3.0 or newer). What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    out.push('\n');
    let mut printer = Printer { program, out: String::new(), depth: 1 };
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    let _ = writeln!(out, "# The program.");
    let _ = writeln!(out, "Z.start([{}], -> {{", files.join(", "));
    printer.statements_of(&program.main.body);
    out.push_str(&printer.out);
    out.push_str("})\n");
    out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
}

/// A Ruby string literal. `#` would begin a value written into the text.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{{{code:x}}}")).replace('#', "\\#")
}

fn strings(names: &[String]) -> String {
    names.iter().map(|name| string(name)).collect::<Vec<_>>().join(", ")
}

impl<'a> Printer<'a> {
    /// A variable. One that begins with a capital letter would be a
    /// constant: no name of the program begins with `z_` (lower.rs).
    fn name(&self, id: VarId) -> String {
        let ident = &self.program.vars[id].ident;
        if ident.starts_with(|c: char| c.is_ascii_uppercase()) {
            format!("z_{ident}")
        } else {
            ident.clone()
        }
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// The statements of a block, after a line that names its variables,
    /// so that Ruby knows each for a variable before a function made in
    /// the block reads it. The error a `try` catches is named where it
    /// is caught.
    fn statements_of(&mut self, block: &Block) {
        let decls: Vec<String> =
            block.decls.iter().filter(|id| self.program.vars[**id].kind != VarKind::Error).map(|id| self.name(*id)).collect();
        // Ruby reads a long row of `a = b = ... = nil` by going one step
        // deeper for each name, so a long row is cut into short ones.
        for row in decls.chunks(20) {
            let line = format!("{} = nil", row.join(" = "));
            self.line(&line);
        }
        for statement in &block.stmts {
            self.statement(statement);
        }
    }

    /// A block one step in.
    fn nested(&mut self, block: &Block) {
        self.depth += 1;
        self.statements_of(block);
        self.depth -= 1;
    }

    /// Does a function made in these statements use a variable they
    /// declare? Then each round of a loop over them needs variables of
    /// its own. The body of a `for` inside them is a block already, and
    /// a `while` inside them is asked for itself.
    fn shares(&self, block: &Block) -> bool {
        block.decls.iter().any(|id| self.program.vars[*id].cell)
            || block.stmts.iter().any(|statement| match statement {
                Stmt::If { then, otherwise, .. } => self.shares(then) || otherwise.as_ref().is_some_and(|block| self.shares(block)),
                Stmt::Try { body, handler, .. } => self.shares(body) || self.shares(handler),
                Stmt::Block(block) => self.shares(block),
                _ => false,
            })
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Set(id, value) => {
                let line = format!("{} = {}", self.name(*id), self.expr(value));
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
                let text = self.expr(expr);
                self.line(&text);
            }
            Stmt::If { condition, then, otherwise } => {
                let line = format!("if {}", self.cond(condition));
                self.line(&line);
                self.nested(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
                    // `elsif`, for an `else` that holds only an `if`.
                    if let ([Stmt::If { condition, then, otherwise: next }], true) = (&block.stmts[..], block.decls.is_empty()) {
                        let line = format!("elsif {}", self.cond(condition));
                        self.line(&line);
                        self.nested(then);
                        otherwise = next.as_ref();
                    } else {
                        self.line("else");
                        self.nested(block);
                        otherwise = None;
                    }
                }
                self.line("end");
            }
            Stmt::While { condition, body } => {
                if self.shares(body) {
                    self.line("Z.loop do");
                    if !matches!(condition, Cond::True) {
                        self.depth += 1;
                        let line = format!("break unless {}", self.cond(condition));
                        self.line(&line);
                        self.depth -= 1;
                    }
                } else {
                    let line = format!("while {}", self.cond(condition));
                    self.line(&line);
                }
                self.nested(body);
                self.line("end");
            }
            Stmt::For { var, second, items, body } => {
                let names = match second {
                    Some(second) => format!("{}, {}", self.name(*var), self.name(*second)),
                    None => self.name(*var),
                };
                let each = if second.is_some() { "each_pair" } else { "each" };
                let over = match items {
                    Expr::Rt { name: "items" | "pairs", args, line } => format!("{}, {line}", self.exprs(args)),
                    other => format!("{}, 0", self.expr(other)),
                };
                let line = format!("Z.{each}({over}) do |{names}|");
                self.line(&line);
                self.nested(body);
                self.line("end");
            }
            Stmt::Break => self.line("break"),
            Stmt::Continue => self.line("next"),
            Stmt::Return(value) => {
                let line = format!("return {}", self.expr(value));
                self.line(&line);
            }
            Stmt::Raise { value, line } => {
                let text = format!("Z.raise_error({}, {line})", self.expr(value));
                self.line(&text);
            }
            Stmt::Try { body, error, handler } => {
                self.line("begin");
                self.nested(body);
                let line = format!("rescue ZRaise => {}", self.name(*error));
                self.line(&line);
                self.nested(handler);
                self.line("end");
            }
            Stmt::Reraise(error) => {
                let line = format!("raise {}", self.name(*error));
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
            Stmt::Block(block) => {
                self.line("begin");
                self.nested(block);
                self.line("end");
            }
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) -> String {
        exprs.iter().map(|expr| self.expr(expr)).collect::<Vec<_>>().join(", ")
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) | Expr::Caught(id) => self.name(*id),
            Expr::Int(n) => n.to_string(),
            Expr::Float(x) => float(*x),
            Expr::Str(text) | Expr::Text(text) => string(text),
            Expr::Bool(b) => b.to_string(),
            Expr::None => "nil".to_string(),
            Expr::Missing => "Z::MISSING".to_string(),
            Expr::FromCond(test) => self.cond(test),
            Expr::Builtin(name) => format!("Z.builtin({})", string(name)),
            Expr::Module(name) => format!("Z.module_of({})", string(name)),
            Expr::CallBuiltin { name, args, line } => {
                format!("Z.b_{}([{}], {line})", name.replace('.', "_"), self.exprs(args))
            }
            Expr::Call { callee, args, line } => format!("Z.call({}, [{}], {line})", self.expr(callee), self.exprs(args)),
            Expr::CallNamed { callee, args, names, named, line } => {
                format!("Z.call_named({}, [{}], [{}], [{}], {line})", self.expr(callee), self.exprs(args), strings(names), self.exprs(named))
            }
            Expr::CallDirect { function, args, line } => {
                let args: String = args.iter().map(|arg| format!(", {}", self.expr(arg))).collect();
                format!("Z.direct({}, {line}{args})", self.name(*function))
            }
            Expr::CallMethod { object, name, args, names, named, line } => {
                if names.is_empty() {
                    format!("Z.call_method({}, {}, [{}], {line})", self.expr(object), string(name), self.exprs(args))
                } else {
                    format!(
                        "Z.call_method_named({}, {}, [{}], [{}], [{}], {line})",
                        self.expr(object),
                        string(name),
                        self.exprs(args),
                        strings(names),
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
            Expr::Func(function) => self.function(function),
            Expr::Class(class) => {
                let defaults: Vec<String> = class
                    .fields
                    .iter()
                    .map(|(_, default)| match default {
                        Some(function) => self.function(function),
                        None => "nil".to_string(),
                    })
                    .collect();
                let fields: Vec<String> = class.fields.iter().map(|(name, _)| name.clone()).collect();
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("{} => {}", string(&method.name), self.function(method))).collect();
                format!("Z.klass({}, [{}], [{}], {{{}}})", string(&class.name), strings(&fields), defaults.join(", "), methods.join(", "))
            }
            Expr::Enum(def) => {
                let cases: Vec<String> = def.cases.iter().map(|(name, fields)| format!("[{}, [{}]]", string(name), strings(fields))).collect();
                format!("Z.enum({}, [{}])", string(&def.name), cases.join(", "))
            }
        }
    }

    /// A Zeno function: a Ruby lambda with its Zeno name and parameters
    /// beside it, so that a call with named arguments can be lined up.
    fn function(&mut self, function: &Func) -> String {
        let names: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 1 };
        let params: Vec<String> = function.params.iter().map(|param| inner.name(param.var)).collect();
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if {}.equal?(Z::MISSING)", inner.name(param.var));
                inner.line(&line);
                inner.nested(default);
                inner.line("end");
            }
        }
        inner.statements_of(&function.body);
        // A function that ends without `return` gives `none`.
        if !matches!(function.body.stmts.last(), Some(Stmt::Return(_))) {
            inner.line("nil");
        }
        let mut text = format!("Z.fn({}, [{}], [{}], ->({}) {{\n", string(&function.name), strings(&names), defaults.join(", "), params.join(", "));
        text.push_str(&inner.out);
        for _ in 0..self.depth {
            text.push_str("  ");
        }
        text.push_str("})");
        text
    }

    fn cond(&mut self, test: &Cond) -> String {
        match test {
            Cond::True => "true".to_string(),
            Cond::Truth(Expr::FromCond(inner), _) => self.cond(inner),
            Cond::Truth(value, line) => format!("Z.truth({}, {line})", self.expr(value)),
            Cond::Not(inner) => format!("!{}", self.cond_atom(inner)),
            Cond::And(a, b) => format!("({} && {})", self.cond(a), self.cond(b)),
            Cond::Or(a, b) => format!("({} || {})", self.cond(a), self.cond(b)),
            Cond::Test { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("Z.{}({args})", step(name))
            }
            Cond::IsMissing(id) => format!("{}.equal?(Z::MISSING)", self.name(*id)),
        }
    }

    /// A test that can stand after `!` as it is.
    fn cond_atom(&mut self, test: &Cond) -> String {
        let text = self.cond(test);
        match test {
            Cond::Not(_) | Cond::True => format!("({text})"),
            _ => text,
        }
    }
}

/// The name a step of lower.rs has in runtime.rb, where it is not the same.
fn step(name: &str) -> &str {
    match name {
        "in" => "is_in",
        "case" => "case_of",
        "format" => "formatted",
        other => other,
    }
}

/// A float literal that Ruby reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "Float::NAN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "Float::INFINITY" } else { "-Float::INFINITY" }.to_string()
    } else {
        lower::float_literal(x)
    }
}
