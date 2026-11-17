//! Writes a Zeno program as PHP: `zeno build --target php`.
//!
//! The program comes ready from lower.rs, as it does for JavaScript
//! (emit_js.rs), and every value is a value of runtime.php, which is put
//! in front of the program, so the `.php` file runs by itself:
//!
//! ```text
//! php hello.php
//! ```
//!
//! Two things differ from JavaScript, because PHP is not JavaScript:
//!
//! - A PHP function sees none of the variables around it unless it
//!   names them: `function ($x) use (&$total)`. The `&` makes it the
//!   same variable and not a copy, so that the inner function and the
//!   outer one see each other's changes, as in Zeno.
//! - A PHP variable lasts for its whole function, not for its block. So
//!   a variable that an inner function uses, declared in the body of a
//!   loop, would be one variable for every round, where Zeno gives each
//!   round its own. Such a variable is let go of (`unset`) where its
//!   block begins, which makes the next round's a new one and leaves the
//!   old one with the functions that took it.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId};

/// The runtime every program built for PHP carries.
pub const RUNTIME: &str = include_str!("runtime.php");

/// The whole `.php` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "<?php");
    let _ = writeln!(out, "// {source}, written in PHP by `zeno build --target php`.");
    let _ = writeln!(out, "// Run it with `php` (8.0 or newer). What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    let _ = writeln!(out, "declare(strict_types=1);");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    out.push('\n');
    let mut printer = Printer { program, out: String::new(), depth: 1 };
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    let _ = writeln!(out, "// The program.");
    let _ = writeln!(out, "Z::start([{}], function () {{", files.join(", "));
    printer.function_body(&program.main.body);
    out.push_str(&printer.out);
    out.push_str("});\n");
    out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
}

/// A PHP string literal, in double quotes. A `$` would begin the name
/// of a variable there, so it is escaped.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{{{code:x}}}")).replace('$', "\\$")
}

impl<'a> Printer<'a> {
    /// A variable: `$x`.
    fn name(&self, id: VarId) -> String {
        format!("${}", self.program.vars[id].ident)
    }

    /// Is it a variable that a function written inside this one uses?
    fn shared(&self, id: VarId) -> bool {
        self.program.vars[id].cell
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// The statements of a block, after a line that declares its variables.
    fn block_body(&mut self, block: &Block) {
        // A shared variable is let go of first: what an earlier round of
        // a loop made of it stays with the functions of that round.
        let shared: Vec<String> = block.decls.iter().filter(|id| self.shared(**id)).map(|id| self.name(*id)).collect();
        if !shared.is_empty() {
            let line = format!("unset({});", shared.join(", "));
            self.line(&line);
        }
        self.function_body(block);
    }

    /// The same for the block that is the whole of a function: it runs
    /// once for each call, and each call has variables of its own.
    fn function_body(&mut self, block: &Block) {
        if !block.decls.is_empty() {
            let names: Vec<String> = block.decls.iter().map(|id| self.name(*id)).collect();
            let line = format!("{} = null;", names.join(" = "));
            self.line(&line);
        }
        for statement in &block.stmts {
            self.statement(statement);
        }
    }

    /// `{`, the block, `}` (the `{` is on the line before).
    fn nested(&mut self, block: &Block) {
        self.depth += 1;
        self.block_body(block);
        self.depth -= 1;
    }

    /// A name a loop or a `catch` gives a value to: the variable itself,
    /// or, for a shared one, `spare` and the lines that make the variable
    /// anew from it.
    fn target(&self, id: VarId, spare: &str) -> (String, Option<String>) {
        let name = self.name(id);
        if self.shared(id) { (spare.to_string(), Some(format!("unset({name}); {name} = {spare};"))) } else { (name, None) }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Set(id, value) => {
                let line = format!("{} = {};", self.name(*id), self.expr(value));
                self.line(&line);
            }
            Stmt::SetField { object, name, value, line } => {
                let text = format!("Z::setField({}, {}, {}, {line});", self.expr(object), string(name), self.expr(value));
                self.line(&text);
            }
            Stmt::SetIndex { object, index, value, line } => {
                let text = format!("Z::setIndex({}, {}, {}, {line});", self.expr(object), self.expr(index), self.expr(value));
                self.line(&text);
            }
            Stmt::Expr(expr) => {
                let line = format!("{};", self.expr(expr));
                self.line(&line);
            }
            Stmt::If { condition, then, otherwise } => {
                let line = format!("if ({}) {{", self.cond(condition));
                self.line(&line);
                self.nested(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
                    // `elseif`, for an `else` that holds only an `if`.
                    if let ([Stmt::If { condition, then, otherwise: next }], true) = (&block.stmts[..], block.decls.is_empty()) {
                        let line = format!("}} elseif ({}) {{", self.cond(condition));
                        self.line(&line);
                        self.nested(then);
                        otherwise = next.as_ref();
                    } else {
                        self.line("} else {");
                        self.nested(block);
                        otherwise = None;
                    }
                }
                self.line("}");
            }
            Stmt::While { condition, body } => {
                let line = format!("while ({}) {{", self.cond(condition));
                self.line(&line);
                self.nested(body);
                self.line("}");
            }
            Stmt::For { var, second, items, body } => {
                let items = self.expr(items);
                let (first, first_made) = self.target(*var, "$z_item");
                let (names, second_made) = match second {
                    Some(second) => {
                        let (second, made) = self.target(*second, "$z_second");
                        (format!("[{first}, {second}]"), made)
                    }
                    None => (first, None),
                };
                let line = format!("foreach ({items} as {names}) {{");
                self.line(&line);
                self.depth += 1;
                for made in [first_made, second_made].into_iter().flatten() {
                    self.line(&made);
                }
                self.depth -= 1;
                self.nested(body);
                self.line("}");
            }
            Stmt::Break => self.line("break;"),
            Stmt::Continue => self.line("continue;"),
            Stmt::Return(value) => {
                let line = format!("return {};", self.expr(value));
                self.line(&line);
            }
            Stmt::Raise { value, line } => {
                let text = format!("Z::raise({}, {line});", self.expr(value));
                self.line(&text);
            }
            Stmt::Try { body, error, handler } => {
                self.line("try {");
                self.nested(body);
                // A mistake in the program, or one of PHP's own, is not a
                // `ZRaise`, and goes on past every `except`.
                let (error, made) = self.target(*error, "$z_error");
                let line = format!("}} catch (ZRaise {error}) {{");
                self.line(&line);
                self.depth += 1;
                if let Some(made) = made {
                    self.line(&made);
                }
                self.block_body(handler);
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Reraise(error) => {
                let line = format!("throw {};", self.name(*error));
                self.line(&line);
            }
            Stmt::Fatal { message, line } => {
                let text = format!("Z::fatal({}, {line});", string(message));
                self.line(&text);
            }
            Stmt::NoMatch { value, line } => {
                let text = format!("Z::noMatch({}, {line});", self.expr(value));
                self.line(&text);
            }
            Stmt::Block(block) => self.block_body(block),
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) -> String {
        exprs.iter().map(|expr| self.expr(expr)).collect::<Vec<_>>().join(", ")
    }

    fn strings(names: &[String]) -> String {
        names.iter().map(|name| string(name)).collect::<Vec<_>>().join(", ")
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) | Expr::Caught(id) => self.name(*id),
            // PHP reads the digits of the smallest int as a float, since
            // they do not fit without their `-`.
            Expr::Int(i64::MIN) => "PHP_INT_MIN".to_string(),
            Expr::Int(n) => n.to_string(),
            Expr::Float(x) => float(*x),
            Expr::Str(text) | Expr::Text(text) => string(text),
            Expr::Bool(b) => b.to_string(),
            Expr::None => "null".to_string(),
            Expr::Missing => "Z::$MISSING".to_string(),
            Expr::FromCond(test) => self.cond(test),
            Expr::Builtin(name) => format!("Z::builtin({})", string(name)),
            Expr::Module(name) => format!("Z::module({})", string(name)),
            Expr::CallBuiltin { name, args, line } => {
                format!("Z::b_{}([{}], {line})", name.replace('.', "_"), self.exprs(args))
            }
            Expr::Call { callee, args, line } => format!("Z::call({}, [{}], {line})", self.expr(callee), self.exprs(args)),
            Expr::CallNamed { callee, args, names, named, line } => {
                format!("Z::callNamed({}, [{}], [{}], [{}], {line})", self.expr(callee), self.exprs(args), Self::strings(names), self.exprs(named))
            }
            Expr::CallDirect { function, args, line } => {
                let args: String = args.iter().map(|arg| format!(", {}", self.expr(arg))).collect();
                format!("Z::direct({}, {line}{args})", self.name(*function))
            }
            Expr::CallMethod { object, name, args, names, named, line } => {
                if names.is_empty() {
                    format!("Z::callMethod({}, {}, [{}], {line})", self.expr(object), string(name), self.exprs(args))
                } else {
                    format!(
                        "Z::callMethodNamed({}, {}, [{}], [{}], [{}], {line})",
                        self.expr(object),
                        string(name),
                        self.exprs(args),
                        Self::strings(names),
                        self.exprs(named)
                    )
                }
            }
            Expr::Rt { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("Z::{}({args})", step(name))
            }
            Expr::Count(n) => n.to_string(),
            Expr::Array(items) => format!("[{}]", self.exprs(items)),
            Expr::Func(function) => self.function(function),
            Expr::Class(class) => {
                let fields: Vec<String> = class.fields.iter().map(|(name, _)| string(name)).collect();
                let defaults: Vec<String> = class
                    .fields
                    .iter()
                    .map(|(_, default)| match default {
                        Some(function) => self.function(function),
                        None => "null".to_string(),
                    })
                    .collect();
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("{} => {}", string(&method.name), self.function(method))).collect();
                format!("Z::class_({}, [{}], [{}], [{}])", string(&class.name), fields.join(", "), defaults.join(", "), methods.join(", "))
            }
            Expr::Enum(def) => {
                let cases: Vec<String> =
                    def.cases.iter().map(|(name, fields)| format!("[{}, [{}]]", string(name), Self::strings(fields))).collect();
                format!("Z::enum({}, [{}])", string(&def.name), cases.join(", "))
            }
        }
    }

    /// A Zeno function: a PHP function with its Zeno name and parameters
    /// beside it, so that a call with named arguments can be lined up.
    /// After `use` come the variables around it that it works with.
    fn function(&mut self, function: &Func) -> String {
        let params: Vec<String> = function.params.iter().map(|param| self.name(param.var)).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let names: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 1 };
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if ({} === Z::$MISSING) {{", inner.name(param.var));
                inner.line(&line);
                inner.nested(default);
                inner.line("}");
            }
        }
        inner.function_body(&function.body);
        // A function that ends without `return` gives `none`.
        if !matches!(function.body.stmts.last(), Some(Stmt::Return(_))) {
            inner.line("return null;");
        }
        let captures: Vec<String> = function.captures.iter().map(|id| format!("&{}", self.name(*id))).collect();
        let around = if captures.is_empty() { String::new() } else { format!(" use ({})", captures.join(", ")) };
        let mut text = format!(
            "Z::fn({}, [{}], [{}], function ({}){around} {{\n",
            string(&function.name),
            Self::strings(&names),
            defaults.join(", "),
            params.join(", ")
        );
        text.push_str(&inner.out);
        for _ in 0..self.depth {
            text.push_str("    ");
        }
        text.push_str("})");
        text
    }

    fn cond(&mut self, test: &Cond) -> String {
        match test {
            Cond::True => "true".to_string(),
            Cond::Truth(Expr::FromCond(inner), _) => self.cond(inner),
            Cond::Truth(value, line) => format!("Z::truth({}, {line})", self.expr(value)),
            Cond::Not(inner) => format!("!{}", self.cond_atom(inner)),
            Cond::And(a, b) => format!("({} && {})", self.cond(a), self.cond(b)),
            Cond::Or(a, b) => format!("({} || {})", self.cond(a), self.cond(b)),
            Cond::Test { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("Z::{}({args})", step(name))
            }
            Cond::IsMissing(id) => format!("({} === Z::$MISSING)", self.name(*id)),
        }
    }

    /// A test that can stand after `!` as it is.
    fn cond_atom(&mut self, test: &Cond) -> String {
        let text = self.cond(test);
        match test {
            Cond::Not(_) => format!("({text})"),
            _ => text,
        }
    }
}

/// The name a step of lower.rs has in runtime.php.
fn step(name: &str) -> &str {
    match name {
        "case_field" => "caseField",
        "is_case" => "isCase",
        other => other,
    }
}

/// A float literal that PHP reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "NAN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "INF" } else { "-INF" }.to_string()
    } else {
        lower::float_literal(x)
    }
}
