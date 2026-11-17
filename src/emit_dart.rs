//! Writes a Zeno program as Dart: `zeno build --target dart`.
//!
//! The program comes ready from lower.rs, as it does for JavaScript
//! (emit_js.rs), and every value is a value of runtime.dart, which is put
//! in front of the program, so the `.dart` file runs by itself:
//!
//! ```text
//! dart run hello.dart
//! ```
//!
//! A Dart function shares the variables around it, as a Zeno function
//! does, and a variable declared in the body of a loop is a new one each
//! round, so a function made in a loop needs nothing more. Two things
//! are Dart's own:
//!
//! - Dart checks types before the program runs, and what a Zeno value
//!   is, is found out while it runs. So every variable is an `Object?`,
//!   every list handed to the runtime says what it holds
//!   (`<Object?>[a, b]`), and every Zeno function takes one list, its
//!   arguments, which its first line takes apart.
//! - `$` in a text begins a value written into it, so it is written `\$`.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId, VarKind};

/// The runtime every program built for Dart carries.
pub const RUNTIME: &str = include_str!("runtime.dart");

/// The whole `.dart` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {source}, written in Dart by `zeno build --target dart`.");
    let _ = writeln!(out, "// Run it with `dart run`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    out.push('\n');
    let mut printer = Printer { program, out: String::new(), depth: 2 };
    let _ = writeln!(out, "// The program.");
    let _ = writeln!(out, "void main(List<String> z_args) {{");
    let _ = writeln!(out, "  Z.start({}, z_args, () {{", strings(&program.files));
    printer.block_body(&program.main.body);
    printer.line("return null;");
    out.push_str(&printer.out);
    out.push_str("  });\n}\n");
    out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
}

/// A Dart string literal.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{{{code:x}}}")).replace('$', "\\$")
}

/// A list of texts, as Dart holds them.
fn strings(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|name| string(name)).collect();
    format!("<String>[{}]", names.join(", "))
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

    /// The statements of a block, after a line that declares its
    /// variables. The error a `try` catches is declared where it is caught.
    fn block_body(&mut self, block: &Block) {
        let names: Vec<&str> =
            block.decls.iter().filter(|id| self.program.vars[**id].kind != VarKind::Error).map(|id| self.name(*id)).collect();
        if !names.is_empty() {
            let line = format!("Object? {};", names.join(", "));
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
                let text = format!("Z.set_field({}, {}, {}, {line});", self.expr(object), string(name), self.expr(value));
                self.line(&text);
            }
            Stmt::SetIndex { object, index, value, line } => {
                let text = format!("Z.set_index({}, {}, {}, {line});", self.expr(object), self.expr(index), self.expr(value));
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
                    format!("{text};")
                } else {
                    format!("Z.ignore({text});")
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
                match second {
                    Some(second) => {
                        let line = format!("for (final z_pair in {items}) {{");
                        self.line(&line);
                        self.depth += 1;
                        let line = format!("Object? {} = z_pair[0], {} = z_pair[1];", self.name(*var), self.name(*second));
                        self.line(&line);
                        self.depth -= 1;
                    }
                    None => {
                        let line = format!("for (Object? {} in {items}) {{", self.name(*var));
                        self.line(&line);
                    }
                }
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
                // A mistake in the program, or one of Dart's own, is not a
                // ZRaise, and goes on past every `except`.
                let line = format!("}} on ZRaise catch ({}) {{", self.name(*error));
                self.line(&line);
                self.nested(handler);
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
                let text = format!("Z.no_match({}, {line});", self.expr(value));
                self.line(&text);
            }
            Stmt::Block(block) => {
                self.line("{");
                self.nested(block);
                self.line("}");
            }
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) -> String {
        exprs.iter().map(|expr| self.expr(expr)).collect::<Vec<_>>().join(", ")
    }

    /// Values for the runtime, as a list that says what it holds.
    fn values(&mut self, exprs: &[Expr]) -> String {
        format!("<Object?>[{}]", self.exprs(exprs))
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) | Expr::Caught(id) => self.name(*id).to_string(),
            Expr::Int(n) => n.to_string(),
            Expr::Float(x) => float(*x),
            Expr::Str(text) | Expr::Text(text) => string(text),
            Expr::Bool(b) => b.to_string(),
            Expr::None => "null".to_string(),
            Expr::Missing => "Z.MISSING".to_string(),
            Expr::FromCond(test) => self.cond(test),
            Expr::Builtin(name) => format!("Z.builtin({})", string(name)),
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
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("{}: {}", string(&method.name), self.function(method))).collect();
                format!(
                    "Z.cls({}, {}, <Object?>[{}], <String, ZFunc>{{{}}})",
                    string(&class.name),
                    strings(&fields),
                    defaults.join(", "),
                    methods.join(", ")
                )
            }
            Expr::Enum(def) => {
                let cases: Vec<String> = def.cases.iter().map(|(name, _)| name.clone()).collect();
                let fields: Vec<String> = def.cases.iter().map(|(_, fields)| strings(fields)).collect();
                format!("Z.enumOf({}, {}, <List<String>>[{}])", string(&def.name), strings(&cases), fields.join(", "))
            }
        }
    }

    /// A Zeno function: a Dart function of one list, its arguments, with
    /// its Zeno name and parameters beside it, so that a call with named
    /// arguments can be lined up.
    fn function(&mut self, function: &Func) -> String {
        let names: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 1 };
        if !function.params.is_empty() {
            let taken: Vec<String> =
                function.params.iter().enumerate().map(|(at, param)| format!("{} = z_a[{at}]", inner.name(param.var))).collect();
            let line = format!("Object? {};", taken.join(", "));
            inner.line(&line);
        }
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if (identical({}, Z.MISSING)) {{", inner.name(param.var));
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
        let mut text =
            format!("Z.fn({}, {}, <bool>[{}], (List<Object?> z_a) {{\n", string(&function.name), strings(&names), defaults.join(", "));
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
            Cond::IsMissing(id) => format!("identical({}, Z.MISSING)", self.name(*id)),
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

/// The name a step of lower.rs has in runtime.dart. `in` and `case`
/// are words of Dart.
fn step(name: &str) -> &str {
    match name {
        "in" => "is_in",
        "case" => "case_of",
        other => other,
    }
}

/// A float literal that Dart reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "double.nan".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "double.infinity" } else { "double.negativeInfinity" }.to_string()
    } else {
        lower::float_literal(x)
    }
}
