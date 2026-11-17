//! Writes a Zeno program as JavaScript: `zeno build --target js`.
//!
//! The program comes ready from lower.rs: names made unique, blocks with
//! their variables listed, comprehensions turned into loops. This file
//! only prints it. Every value is a value of runtime.js, which is put in
//! front of the program, so the `.js` file runs by itself under Node:
//!
//! ```text
//! node hello.js
//! ```
//!
//! A Zeno function becomes a JavaScript function. A call of a function
//! known where it is written is `Z.direct(area, 6, r)`: the runtime counts
//! how deep the calls are nested, so 1000 calls inside each other stop the
//! program at line 6 as they do under `zeno run`, and then calls `area(r)`.
//! Everything else goes through the runtime too, which does what `zeno
//! run` does: `Z.add(a, b, 3)` adds as Zeno adds, and stops at line 3 with
//! Zeno's message if the sum is too big for an int.
//!
//! The same JavaScript is what `--target html` puts in a page.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId};

/// The runtime every program built for JavaScript carries.
pub const RUNTIME: &str = include_str!("runtime.js");

/// The whole `.js` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {source}, written in JavaScript by `zeno build --target js`.");
    let _ = writeln!(out, "// Run it with `node`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    out.push('\n');
    out.push_str(&program_body(program));
    out
}

/// A page that runs the program: `zeno build --target html`. `page` is
/// the HTML to put it in (the program's own page, or a plain one with a
/// `<pre id="zeno-output">`). What the program prints goes into that
/// element, and an error that stops it is shown under what it printed.
pub fn page(program: &Program, page: &str) -> String {
    let mut script = String::new();
    script.push_str("<script>\n");
    script.push_str("// Written in JavaScript by `zeno build --target html`.\n");
    script.push_str(RUNTIME);
    script.push_str(
        "Z.setPage(function (text) {\n  \
         let output = document.getElementById(\"zeno-output\");\n  \
         if (!output) {\n    \
         output = document.createElement(\"pre\");\n    \
         output.id = \"zeno-output\";\n    \
         document.body.appendChild(output);\n  \
         }\n  \
         output.textContent += text;\n\
         });\n",
    );
    script.push_str(&program_body(program));
    script.push_str("</script>\n");
    match page.rfind("</body>") {
        Some(at) => format!("{}{script}{}", &page[..at], &page[at..]),
        None => format!("{page}\n{script}"),
    }
}

/// The program alone, with no runtime: a call of `Z.start`.
pub fn program_body(program: &Program) -> String {
    let mut printer = Printer { program, out: String::new(), depth: 0 };
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    let _ = writeln!(printer.out, "// The program.");
    let _ = writeln!(printer.out, "Z.start([{}], function main() {{", files.join(", "));
    printer.depth = 1;
    printer.block_body(&program.main.body);
    printer.out.push_str("});\n");
    printer.out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
}

/// A JavaScript string literal.
///
/// The program may be put in a page, where `</script` in a text would
/// end the script and `<!--` would hide what comes after it. So a `<`
/// before a `/` or a `!` is written as its code.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{code:04x}")).replace("</", "\\x3c/").replace("<!", "\\x3c!")
}

impl<'a> Printer<'a> {
    fn name(&self, id: VarId) -> &'a str {
        &self.program.vars[id].ident
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// The statements of a block, after a line that declares its variables.
    fn block_body(&mut self, block: &Block) {
        if !block.decls.is_empty() {
            let names: Vec<&str> = block.decls.iter().map(|id| self.name(*id)).collect();
            let line = format!("let {};", names.join(", "));
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

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Set(id, value) => {
                let line = format!("{} = {};", self.name(*id), self.expr(value));
                self.line(&line);
            }
            Stmt::SetField { object, name, value, line } => {
                let text = format!("Z.setField({}, {}, {}, {line});", self.expr(object), string(name), self.expr(value));
                self.line(&text);
            }
            Stmt::SetIndex { object, index, value, line } => {
                let text = format!("Z.setIndex({}, {}, {}, {line});", self.expr(object), self.expr(index), self.expr(value));
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
                let names = match second {
                    Some(second) => format!("[{}, {}]", self.name(*var), self.name(*second)),
                    None => self.name(*var).to_string(),
                };
                let line = format!("for (const {names} of {}) {{", self.expr(items));
                self.line(&line);
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
                let text = format!("Z.raise({}, {line});", self.expr(value));
                self.line(&text);
            }
            Stmt::Try { body, error, handler } => {
                self.line("try {");
                self.nested(body);
                let error = self.name(*error).to_string();
                let line = format!("}} catch ({error}) {{");
                self.line(&line);
                self.depth += 1;
                // A mistake in the program, or one of JavaScript's own,
                // goes on past every `except`.
                let line = format!("if (!({error} instanceof Z.Raise)) throw {error};");
                self.line(&line);
                self.block_body(handler);
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Reraise(error) => {
                let line = format!("throw {};", self.name(*error));
                self.line(&line);
            }
            Stmt::Fatal { message, line } => {
                let text = format!("Z.fatal({}, {line});", string(message));
                self.line(&text);
            }
            Stmt::NoMatch { value, line } => {
                let text = format!("Z.noMatch({}, {line});", self.expr(value));
                self.line(&text);
            }
            Stmt::Block(block) => {
                if block.decls.is_empty() {
                    for statement in &block.stmts {
                        self.statement(statement);
                    }
                } else {
                    self.line("{");
                    self.nested(block);
                    self.line("}");
                }
            }
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) -> String {
        exprs.iter().map(|expr| self.expr(expr)).collect::<Vec<_>>().join(", ")
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) => self.name(*id).to_string(),
            Expr::Int(n) if n.unsigned_abs() <= (1u64 << 53) - 1 => n.to_string(),
            Expr::Int(n) => format!("{n}n"),
            Expr::Float(x) => format!("Z.float({})", float(*x)),
            Expr::Str(text) => string(text),
            Expr::Bool(b) => b.to_string(),
            Expr::None => "null".to_string(),
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
                format!("Z.callNamed({}, [{}], [{}], [{}], {line})", self.expr(callee), self.exprs(args), names.join(", "), self.exprs(named))
            }
            Expr::CallDirect { function, args, line } => {
                let args: String = args.iter().map(|arg| format!(", {}", self.expr(arg))).collect();
                format!("Z.direct({}, {line}{args})", self.name(*function))
            }
            Expr::CallMethod { object, name, args, names, named, line } => {
                if names.is_empty() {
                    format!("Z.callMethod({}, {}, [{}], {line})", self.expr(object), string(name), self.exprs(args))
                } else {
                    let names: Vec<String> = names.iter().map(|name| string(name)).collect();
                    format!(
                        "Z.callMethodNamed({}, {}, [{}], [{}], [{}], {line})",
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
            Expr::Text(text) => string(text),
            Expr::Count(n) => n.to_string(),
            Expr::Array(items) => format!("[{}]", self.exprs(items)),
            Expr::Caught(id) => self.name(*id).to_string(),
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
                    class.methods.iter().map(|method| format!("{}: {}", string(&method.name), self.function(method))).collect();
                format!("Z.class({}, [{}], [{}], {{{}}})", string(&class.name), fields.join(", "), defaults.join(", "), methods.join(", "))
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

    /// A Zeno function: a JavaScript function with its Zeno name and
    /// parameters on it, so that a call with named arguments can be lined up.
    fn function(&mut self, function: &Func) -> String {
        let params: Vec<String> = function.params.iter().map(|param| self.name(param.var).to_string()).collect();
        let names: Vec<String> = function.params.iter().map(|param| string(&param.name)).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 1 };
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
            inner.line("return null;");
        }
        let mut text = format!(
            "Z.fn({}, [{}], [{}], function ({}) {{\n",
            string(&function.name),
            names.join(", "),
            defaults.join(", "),
            params.join(", ")
        );
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
            Cond::IsMissing(id) => format!("({} === Z.MISSING)", self.name(*id)),
        }
    }

    /// A test that can stand after `!` as it is.
    fn cond_atom(&mut self, test: &Cond) -> String {
        let text = self.cond(test);
        match test {
            Cond::Truth(..) | Cond::Test { .. } | Cond::True | Cond::And(..) | Cond::Or(..) | Cond::IsMissing(_) => text,
            Cond::Not(_) => format!("({text})"),
        }
    }
}

/// The name a step of lower.rs has in runtime.js.
fn step(name: &str) -> &str {
    match name {
        "case_field" => "caseField",
        "is_case" => "isCase",
        other => other,
    }
}

/// A float literal that JavaScript reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "NaN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else {
        lower::float_literal(x)
    }
}
