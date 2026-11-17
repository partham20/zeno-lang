//! Writes a Zeno program as Kotlin: `zeno build --target kotlin`.
//!
//! The program comes ready from lower.rs, as it does for JavaScript
//! (emit_js.rs). The `.kt` file holds the program, in `main`, and
//! runtime.kt after it, in the object `Z`, so the file is compiled and
//! run by itself:
//!
//! ```text
//! kotlinc hello.kt -include-runtime -d hello.jar
//! java -jar hello.jar
//! ```
//!
//! Kotlin runs on Java's machine, and runtime.kt is runtime.java in
//! Kotlin. But a Kotlin function shares the variables around it, as a
//! Zeno function does, and a variable declared in the body of a loop is
//! a new one each round: so nothing is kept in a cell, as Java needs
//! (emit_java.rs). Three things are Kotlin's own:
//!
//! - Kotlin checks types before the program runs, and what a Zeno value
//!   is, is found out while it runs. So every variable is an `Any?`,
//!   and every Zeno function takes one array, its arguments, which its
//!   first lines take apart.
//! - A Zeno function is written `fun(z_a: Array<Any?>): Any? { ... }`,
//!   not as a lambda in braces: `return` in a lambda would leave the
//!   function around it.
//! - `$` in a text begins a value written into it, so it is written `\$`,
//!   and `{ ... }` alone is a lambda, not a block: a block of its own is
//!   `if (true) { ... }`.
//!
//! The file begins with `@file:Suppress("warnings")`, as the C# file
//! turns its warnings off: they are about what the rules write.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId, VarKind};

/// The runtime every program built for Kotlin carries.
pub const RUNTIME: &str = include_str!("runtime.kt");

/// The whole `.kt` file: the program, then the runtime.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {source}, written in Kotlin by `zeno build --target kotlin`.");
    let _ = writeln!(out, "// Compile it with `kotlinc`, and run it with `java`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    // What the rules write is not what a person would: a `var` set once,
    // a `return null` after a `return`. Kotlin warns of each, for every
    // program, so the file says it has heard.
    out.push_str("@file:Suppress(\"warnings\")\n\n");
    out.push_str("// The program.\nfun main(args: Array<String>) {\n");
    let _ = writeln!(out, "    Z.start({}, args, fun(): Any? {{", strings(&program.files));
    let mut printer = Printer { program, out: String::new(), depth: 2 };
    printer.block_body(&program.main.body);
    printer.line("return null");
    out.push_str(&printer.out);
    out.push_str("    })\n}\n\n");
    out.push_str(RUNTIME);
    out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
}

/// A Kotlin string literal.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{code:04x}")).replace('$', "\\$")
}

/// An array of texts, as Kotlin holds them.
fn strings(names: &[String]) -> String {
    if names.is_empty() {
        return "Z.NO_NAMES".to_string();
    }
    let names: Vec<String> = names.iter().map(|name| string(name)).collect();
    format!("arrayOf<String>({})", names.join(", "))
}

impl<'a> Printer<'a> {
    fn name(&self, id: VarId) -> &'a str {
        &self.program.vars[id].ident
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// The statements of a block, after the lines that declare its
    /// variables. The error a `try` catches is declared where it is caught.
    fn block_body(&mut self, block: &Block) {
        for id in &block.decls {
            if self.program.vars[*id].kind != VarKind::Error {
                let line = format!("var {}: Any? = null", self.name(*id));
                self.line(&line);
            }
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
                // A call can be a statement as it is. Anything else is
                // handed to a step that does nothing with it.
                let text = self.expr(expr);
                let line = if matches!(
                    expr,
                    Expr::CallBuiltin { .. }
                        | Expr::Call { .. }
                        | Expr::CallNamed { .. }
                        | Expr::CallDirect { .. }
                        | Expr::CallMethod { .. }
                        | Expr::Rt { .. }
                ) {
                    text
                } else {
                    format!("Z.ignore({text})")
                };
                self.line(&line);
            }
            Stmt::If { condition, then, otherwise } => {
                let line = format!("if ({}) {{", self.cond(condition));
                self.line(&line);
                self.nested(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
                    // `else if`, for an `else` that holds only an `if`.
                    if let ([Stmt::If { condition, then, otherwise: next }], true) = (&block.stmts[..], block.decls.is_empty()) {
                        let line = format!("}} else if ({}) {{", self.cond(condition));
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
                // What the loop steps through cannot be given a new value
                // in Kotlin, so the loop's own names are variables of
                // the body, made from it.
                let (each, taken) = match second {
                    Some(second) => ("z_pair", vec![(*var, "z_pair[0]"), (*second, "z_pair[1]")]),
                    None => ("z_item", vec![(*var, "z_item")]),
                };
                let line = format!("for ({each} in {items}) {{");
                self.line(&line);
                self.depth += 1;
                for (id, value) in taken {
                    let line = format!("var {}: Any? = {value}", self.name(id));
                    self.line(&line);
                }
                self.block_body(body);
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Break => self.line("break"),
            Stmt::Continue => self.line("continue"),
            Stmt::Return(value) => {
                let line = format!("return {}", self.expr(value));
                self.line(&line);
            }
            Stmt::Raise { value, line } => {
                let text = format!("Z.raise({}, {line})", self.expr(value));
                self.line(&text);
            }
            Stmt::Try { body, error, handler } => {
                self.line("try {");
                self.nested(body);
                // A mistake in the program, or one of Kotlin's own, is not
                // a ZRaise, and goes on past every `except`.
                let line = format!("}} catch ({}: ZRaise) {{", self.name(*error));
                self.line(&line);
                self.nested(handler);
                self.line("}");
            }
            Stmt::Reraise(error) => {
                let line = format!("throw {}", self.name(*error));
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
                if block.decls.is_empty() {
                    for statement in &block.stmts {
                        self.statement(statement);
                    }
                } else {
                    self.line("if (true) {");
                    self.nested(block);
                    self.line("}");
                }
            }
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) -> String {
        exprs.iter().map(|expr| self.expr(expr)).collect::<Vec<_>>().join(", ")
    }

    /// Values for the runtime, as an array.
    fn values(&mut self, exprs: &[Expr]) -> String {
        if exprs.is_empty() {
            "Z.NO_VALUES".to_string()
        } else {
            format!("arrayOf<Any?>({})", self.exprs(exprs))
        }
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) | Expr::Caught(id) => self.name(*id).to_string(),
            // Kotlin reads the digits of the smallest int before their
            // `-`, and they do not fit without it.
            Expr::Int(i64::MIN) => "Z.MIN_INT".to_string(),
            Expr::Int(n) => format!("{n}L"),
            Expr::Float(x) => float(*x),
            Expr::Str(text) | Expr::Text(text) => string(text),
            Expr::Bool(b) => b.to_string(),
            Expr::None => "null".to_string(),
            Expr::Missing => "Z.MISSING".to_string(),
            Expr::FromCond(test) => self.cond(test),
            Expr::Builtin(name) => format!("Z.builtin({}, 0)", string(name)),
            Expr::Module(name) => format!("Z.module({})", string(name)),
            Expr::CallBuiltin { name, args, line } => {
                format!("Z.b_{}({}, {line})", name.replace('.', "_"), self.values(args))
            }
            Expr::Call { callee, args, line } => format!("Z.call({}, {}, {line})", self.expr(callee), self.values(args)),
            Expr::CallNamed { callee, args, names, named, line } => {
                format!("Z.call_named({}, {}, {}, {}, {line})", self.expr(callee), self.values(args), strings(names), self.values(named))
            }
            Expr::CallDirect { function, args, line } => {
                format!("Z.direct({}, {line}, {})", self.name(*function), self.values(args))
            }
            Expr::CallMethod { object, name, args, names, named, line } => {
                if names.is_empty() {
                    format!("Z.call_method({}, {}, {}, {line})", self.expr(object), string(name), self.values(args))
                } else {
                    format!(
                        "Z.call_method_named({}, {}, {}, {}, {}, {line})",
                        self.expr(object),
                        string(name),
                        self.values(args),
                        strings(names),
                        self.values(named)
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
            Expr::Array(items) => self.values(items),
            Expr::Func(function) => self.function(function),
            Expr::Class(class) => {
                let fields: Vec<String> = class.fields.iter().map(|(name, _)| name.clone()).collect();
                let defaults: Vec<String> = class
                    .fields
                    .iter()
                    .map(|(_, default)| match default {
                        Some(function) => self.function(function),
                        None => "null".to_string(),
                    })
                    .collect();
                let defaults = if defaults.is_empty() { "Z.NO_VALUES".to_string() } else { format!("arrayOf<Any?>({})", defaults.join(", ")) };
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("{}, {}", string(&method.name), self.function(method))).collect();
                format!("Z.cls({}, {}, {defaults}, Z.methods({}))", string(&class.name), strings(&fields), methods.join(", "))
            }
            Expr::Enum(def) => {
                let cases: Vec<String> = def.cases.iter().map(|(name, _)| name.clone()).collect();
                let fields: Vec<String> = def.cases.iter().map(|(_, fields)| strings(fields)).collect();
                format!("Z.enumOf({}, {}, arrayOf<Array<String>>({}))", string(&def.name), strings(&cases), fields.join(", "))
            }
        }
    }

    /// A Zeno function: a Kotlin function of one array, its arguments,
    /// with its Zeno name and parameters beside it, so that a call with
    /// named arguments can be lined up.
    fn function(&mut self, function: &Func) -> String {
        let names: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 1 };
        for (at, param) in function.params.iter().enumerate() {
            let line = format!("var {}: Any? = z_a[{at}]", inner.name(param.var));
            inner.line(&line);
        }
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if ({} === Z.MISSING) {{", inner.name(param.var));
                inner.line(&line);
                inner.nested(default);
                inner.line("}");
            }
        }
        inner.block_body(&function.body);
        // A function that ends without `return` gives `none`.
        if !matches!(function.body.stmts.last(), Some(Stmt::Return(_))) {
            inner.line("return null");
        }
        let mut text = format!(
            "Z.fn({}, {}, booleanArrayOf({}), fun(z_a: Array<Any?>): Any? {{\n",
            string(&function.name),
            strings(&names),
            defaults.join(", ")
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
            Cond::IsMissing(id) => format!("({} === Z.MISSING)", self.name(*id)),
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

/// The name a step of lower.rs has in runtime.kt. `in` is a word of Kotlin.
fn step(name: &str) -> &str {
    match name {
        "in" => "is_in",
        "case" => "case_of",
        other => other,
    }
}

/// A float literal that Kotlin reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "Z.NAN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "Z.INF" } else { "(-Z.INF)" }.to_string()
    } else {
        lower::float_literal(x)
    }
}
