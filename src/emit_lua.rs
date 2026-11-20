//! Writes a Zeno program as Lua: `zeno build --target lua`.
//!
//! The program comes ready from lower.rs, as it does for JavaScript
//! (emit_js.rs), and every value is a value of runtime.lua, which is put
//! in front of the program, so the `.lua` file runs by itself:
//!
//! ```text
//! lua hello.lua
//! ```
//!
//! A Lua function shares the variables around it, as a Zeno function
//! does, and a `local` in the body of a loop is a new variable each
//! round, so a function made in a loop needs nothing more. Four things
//! are Lua's own:
//!
//! - Lua has no `try`. The body of a `try` is a function handed to
//!   `Z.try`, which says whether it raised. A `break`, `continue` or
//!   `return` inside that function cannot leave the loop or the function
//!   outside, so it returns how it ended (1, 2, or 3 and the value), and
//!   the code after the call does it, as in Go (emit_go.rs). The body
//!   runs as a coroutine: Lua's `pcall` can be inside itself about 190
//!   times and no more, and a Zeno `try` up to 999 (runtime.lua).
//! - Lua has no `continue`. It is a `goto` to a label at the end of the
//!   loop's body.
//! - `return` must be the last statement of its block: one that is not
//!   is written `do return x end`.
//! - A Lua function may have 200 local variables and no more. A block
//!   that would pass that keeps its variables in a table: `z_v1.total`.

use std::collections::HashMap;
use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId, VarKind};

/// The runtime every program built for Lua carries.
pub const RUNTIME: &str = include_str!("runtime.lua");

/// How many local variables a function may have before a block keeps
/// its own in a table. Lua's limit is 200, and what an expression is
/// worked out in counts toward a limit of 255 too.
const MOST_LOCALS: usize = 100;

/// The whole `.lua` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "-- {source}, written in Lua by `zeno build --target lua`.");
    let _ = writeln!(out, "-- Run it with `lua` (5.4). What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    out.push('\n');
    let mut printer = Printer::new(program, 1, HashMap::new(), 0);
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    let _ = writeln!(out, "-- The program.");
    let _ = writeln!(out, "Z.start({{{}}}, function()", files.join(", "));
    printer.statements_of(&program.main.body);
    out.push_str(&printer.out);
    out.push_str("end)\n");
    out
}

/// What a `break`, `continue` or `return` would leave.
#[derive(Clone, Copy, PartialEq)]
enum Frame {
    /// A loop, with the number of the label at the end of its body.
    Loop(usize),
    Try,
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
    /// The loops and `try`s around the statement, in the function it is in.
    frames: Vec<Frame>,
    /// How many labels this function has, so that each has its own name.
    labels: usize,
    /// How many local variables are in reach now, in this function.
    locals: usize,
    /// The variables that are kept in a table, with the table's name.
    tabled: HashMap<VarId, String>,
    /// How many such tables the program has, so that each has its own name.
    tables: usize,
}

/// A Lua string literal.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{{{code:x}}}"))
}

fn strings(names: &[String]) -> String {
    names.iter().map(|name| string(name)).collect::<Vec<_>>().join(", ")
}

/// Does a `continue` in these statements go on with the loop they are
/// the body of? One inside a loop of their own does not.
fn continues(block: &Block) -> bool {
    block.stmts.iter().any(|statement| match statement {
        Stmt::Continue => true,
        Stmt::If { then, otherwise, .. } => continues(then) || otherwise.as_ref().is_some_and(continues),
        Stmt::Try { body, handler, .. } => continues(body) || continues(handler),
        Stmt::Block(block) => continues(block),
        _ => false,
    })
}

/// Do these statements, the body of a `try`, hold a `break` or
/// `continue` of a loop outside them, or a `return`?
fn leaves(block: &Block, in_loop: bool) -> (bool, bool) {
    let mut found = (false, false);
    let mut add = |(jump, back): (bool, bool)| found = (found.0 || jump, found.1 || back);
    for statement in &block.stmts {
        match statement {
            Stmt::Break | Stmt::Continue => add((!in_loop, false)),
            Stmt::Return(_) => add((false, true)),
            Stmt::If { then, otherwise, .. } => {
                add(leaves(then, in_loop));
                if let Some(otherwise) = otherwise {
                    add(leaves(otherwise, in_loop));
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => add(leaves(body, true)),
            Stmt::Try { body, handler, .. } => {
                add(leaves(body, in_loop));
                add(leaves(handler, in_loop));
            }
            Stmt::Block(block) => add(leaves(block, in_loop)),
            _ => {}
        }
    }
    found
}

impl<'a> Printer<'a> {
    fn new(program: &'a Program, depth: usize, tabled: HashMap<VarId, String>, tables: usize) -> Self {
        Printer { program, out: String::new(), depth, frames: Vec::new(), labels: 0, locals: 0, tabled, tables }
    }

    /// A variable: `x`, or `z_v1.x` for one kept in a table.
    fn name(&self, id: VarId) -> String {
        let ident = &self.program.vars[id].ident;
        match self.tabled.get(&id) {
            Some(table) => format!("{table}.{ident}"),
            None => ident.clone(),
        }
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// The statements of a block, after a line that declares its
    /// variables. The error a `try` catches is declared where it is caught.
    fn statements_of(&mut self, block: &Block) {
        let before = self.locals;
        let decls: Vec<VarId> = block.decls.iter().copied().filter(|id| self.program.vars[*id].kind != VarKind::Error).collect();
        if self.locals + decls.len() > MOST_LOCALS {
            self.tables += 1;
            let table = format!("z_v{}", self.tables);
            let line = format!("local {table} = {{}}");
            self.line(&line);
            for id in decls {
                self.tabled.insert(id, table.clone());
            }
            self.locals += 1;
        } else if !decls.is_empty() {
            let names: Vec<String> = decls.iter().map(|id| self.name(*id)).collect();
            let line = format!("local {}", names.join(", "));
            self.line(&line);
            self.locals += decls.len();
        }
        for (at, statement) in block.stmts.iter().enumerate() {
            self.statement(statement, at + 1 == block.stmts.len());
        }
        self.locals = before;
    }

    /// A block one step in.
    fn nested(&mut self, block: &Block) {
        self.depth += 1;
        self.statements_of(block);
        self.depth -= 1;
    }

    /// The innermost loop or `try` around the statement.
    fn around(&self) -> Option<Frame> {
        self.frames.last().copied()
    }

    /// `return`, where `last` says that nothing follows it in its block.
    fn give_back(&mut self, values: &str, last: bool) {
        let line = if last { format!("return {values}") } else { format!("do return {values} end") };
        self.line(&line);
    }

    fn leave_loop(&mut self, last: bool) {
        match self.around() {
            Some(Frame::Try) => self.give_back("1", last),
            _ => self.line("break"),
        }
    }

    fn next_round(&mut self, last: bool) {
        match self.around() {
            Some(Frame::Try) => self.give_back("2", last),
            Some(Frame::Loop(label)) => {
                let line = format!("goto z_next_{label}");
                self.line(&line);
            }
            None => {}
        }
    }

    fn leave_function(&mut self, value: &str, last: bool) {
        if self.frames.contains(&Frame::Try) {
            self.give_back(&format!("3, {value}"), last);
        } else {
            self.give_back(value, last);
        }
    }

    /// The body of a loop. A `continue` in it goes to a label after it,
    /// so the body is a block of its own: the label must not be in reach
    /// of the body's variables.
    fn loop_body(&mut self, body: &Block) {
        self.labels += 1;
        let label = self.labels;
        self.frames.push(Frame::Loop(label));
        if continues(body) {
            self.depth += 1;
            self.line("do");
            self.nested(body);
            self.line("end");
            let line = format!("::z_next_{label}::");
            self.line(&line);
            self.depth -= 1;
        } else {
            self.nested(body);
        }
        self.frames.pop();
    }

    fn statement(&mut self, statement: &Stmt, last: bool) {
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
                // Only a call can be a statement in Lua.
                let text = self.expr(expr);
                let line = if matches!(
                    expr,
                    Expr::CallBuiltin { .. }
                        | Expr::Call { .. }
                        | Expr::CallNamed { .. }
                        | Expr::CallDirect { .. }
                        | Expr::CallMethod { .. }
                        | Expr::Rt { .. }
                        | Expr::Builtin(_)
                        | Expr::Module(_)
                        | Expr::Class(_)
                        | Expr::Enum(_)
                        | Expr::Func(_)
                ) {
                    text
                } else {
                    format!("do local _ = {text} end")
                };
                self.line(&line);
            }
            Stmt::If { condition, then, otherwise } => {
                let line = format!("if {} then", self.cond(condition));
                self.line(&line);
                self.nested(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
                    // `elseif`, for an `else` that holds only an `if`.
                    if let ([Stmt::If { condition, then, otherwise: next }], true) = (&block.stmts[..], block.decls.is_empty()) {
                        let line = format!("elseif {} then", self.cond(condition));
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
                let line = format!("while {} do", self.cond(condition));
                self.line(&line);
                self.loop_body(body);
                self.line("end");
            }
            Stmt::For { var, second, items, body } => {
                let items = self.expr(items);
                let names = match second {
                    Some(second) => format!("{}, {}", self.name(*var), self.name(*second)),
                    None => self.name(*var),
                };
                let line = format!("for {names} in {items} do");
                self.line(&line);
                // The loop keeps four variables of its own beside the names.
                self.locals += 6;
                self.loop_body(body);
                self.locals -= 6;
                self.line("end");
            }
            Stmt::Break => self.leave_loop(last),
            Stmt::Continue => self.next_round(last),
            Stmt::Return(value) => {
                let value = self.expr(value);
                self.leave_function(&value, last);
            }
            Stmt::Raise { value, line } => {
                let text = format!("Z.raise({}, {line})", self.expr(value));
                self.line(&text);
            }
            Stmt::Try { body, error, handler } => self.try_statement(body, *error, handler),
            Stmt::Reraise(error) => {
                let line = format!("Z.reraise({})", self.name(*error));
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
                self.line("do");
                self.nested(block);
                self.line("end");
            }
        }
    }

    /// `try`: the body as a function for `Z.try`, then what to do with
    /// how it ended.
    fn try_statement(&mut self, body: &Block, error: VarId, handler: &Block) {
        let (jumps, gives_back) = leaves(body, false);
        let signals = jumps || gives_back;
        self.line("do");
        self.depth += 1;
        self.locals += 3;
        self.line(if signals { "local z_ok, z_how, z_value = Z.try(function()" } else { "local z_ok, z_how = Z.try(function()" });
        self.frames.push(Frame::Try);
        self.nested(body);
        self.frames.pop();
        self.line("end)");
        if signals {
            // Do what the body ended with.
            self.line("if z_ok then");
            self.depth += 1;
            if jumps && self.around().is_some() {
                self.line("if z_how == 1 then");
                self.depth += 1;
                self.leave_loop(true);
                self.depth -= 1;
                // Only a body that holds a `continue` can ask for the next
                // round, and only then does the loop have its label.
                if continues(body) {
                    self.line("elseif z_how == 2 then");
                    self.depth += 1;
                    self.next_round(true);
                    self.depth -= 1;
                }
                self.line("end");
            }
            if gives_back {
                self.line("if z_how == 3 then");
                self.depth += 1;
                self.leave_function("z_value", true);
                self.depth -= 1;
                self.line("end");
            }
            self.depth -= 1;
            self.line("else");
        } else {
            self.line("if not z_ok then");
        }
        self.depth += 1;
        let line = format!("local {} = z_how", self.program.vars[error].ident);
        self.line(&line);
        self.locals += 1;
        self.statements_of(handler);
        self.locals -= 1;
        self.depth -= 1;
        self.line("end");
        self.locals -= 3;
        self.depth -= 1;
        self.line("end");
    }

    fn exprs(&mut self, exprs: &[Expr]) -> String {
        exprs.iter().map(|expr| self.expr(expr)).collect::<Vec<_>>().join(", ")
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) | Expr::Caught(id) => self.name(*id),
            // Lua reads the digits of the smallest int as a float, since
            // they do not fit without their `-`.
            Expr::Int(i64::MIN) => "(-9223372036854775807 - 1)".to_string(),
            Expr::Int(n) => n.to_string(),
            Expr::Float(x) => float(*x),
            Expr::Str(text) | Expr::Text(text) => string(text),
            Expr::Bool(b) => b.to_string(),
            Expr::None => "Z.none".to_string(),
            Expr::Missing => "Z.MISSING".to_string(),
            Expr::FromCond(test) => self.cond(test),
            Expr::Builtin(name) => format!("Z.builtin({})", string(name)),
            Expr::Module(name) => format!("Z.module({})", string(name)),
            Expr::CallBuiltin { name, args, line } => {
                format!("Z.b_{}({{{}}}, {line})", name.replace('.', "_"), self.exprs(args))
            }
            Expr::Call { callee, args, line } => format!("Z.call({}, {{{}}}, {line})", self.expr(callee), self.exprs(args)),
            Expr::CallNamed { callee, args, names, named, line } => {
                format!("Z.call_named({}, {{{}}}, {{{}}}, {{{}}}, {line})", self.expr(callee), self.exprs(args), strings(names), self.exprs(named))
            }
            Expr::CallDirect { function, args, line } => {
                let args: String = args.iter().map(|arg| format!(", {}", self.expr(arg))).collect();
                format!("Z.direct({}, {line}{args})", self.name(*function))
            }
            Expr::CallMethod { object, name, args, names, named, line } => {
                if names.is_empty() {
                    format!("Z.call_method({}, {}, {{{}}}, {line})", self.expr(object), string(name), self.exprs(args))
                } else {
                    format!(
                        "Z.call_method_named({}, {}, {{{}}}, {{{}}}, {{{}}}, {line})",
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
            Expr::Array(items) => format!("{{{}}}", self.exprs(items)),
            Expr::Func(function) => self.function(function),
            Expr::Class(class) => {
                let defaults: Vec<String> = class
                    .fields
                    .iter()
                    .map(|(_, default)| match default {
                        Some(function) => self.function(function),
                        None => "false".to_string(),
                    })
                    .collect();
                let fields: Vec<String> = class.fields.iter().map(|(name, _)| name.clone()).collect();
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("[{}] = {}", string(&method.name), self.function(method))).collect();
                format!("Z.class({}, {{{}}}, {{{}}}, {{{}}})", string(&class.name), strings(&fields), defaults.join(", "), methods.join(", "))
            }
            Expr::Enum(def) => {
                let cases: Vec<String> =
                    def.cases.iter().map(|(name, fields)| format!("{{{}, {{{}}}}}", string(name), strings(fields))).collect();
                format!("Z.enum({}, {{{}}})", string(&def.name), cases.join(", "))
            }
        }
    }

    /// A Zeno function: a Lua function with its Zeno name and parameters
    /// beside it, so that a call with named arguments can be lined up.
    fn function(&mut self, function: &Func) -> String {
        let names: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let mut inner = Printer::new(self.program, self.depth + 1, self.tabled.clone(), self.tables);
        let params: Vec<String> = function.params.iter().map(|param| inner.name(param.var)).collect();
        inner.locals = params.len();
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if {} == Z.MISSING then", inner.name(param.var));
                inner.line(&line);
                inner.nested(default);
                inner.line("end");
            }
        }
        inner.statements_of(&function.body);
        // A function that ends without `return` gives `none`.
        if !matches!(function.body.stmts.last(), Some(Stmt::Return(_))) {
            inner.line("return Z.none");
        }
        self.tables = inner.tables;
        let mut text =
            format!("Z.fn({}, {{{}}}, {{{}}}, function({})\n", string(&function.name), strings(&names), defaults.join(", "), params.join(", "));
        text.push_str(&inner.out);
        for _ in 0..self.depth {
            text.push_str("  ");
        }
        text.push_str("end)");
        text
    }

    fn cond(&mut self, test: &Cond) -> String {
        match test {
            Cond::True => "true".to_string(),
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
            Cond::IsMissing(id) => format!("({} == Z.MISSING)", self.name(*id)),
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

/// The name a step of lower.rs has in runtime.lua. `in` is a word of Lua.
fn step(name: &str) -> &str {
    match name {
        "in" => "is_in",
        other => other,
    }
}

/// A float literal that Lua reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "(0 / 0)".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "(1 / 0)" } else { "(-1 / 0)" }.to_string()
    } else {
        lower::float_literal(x)
    }
}
