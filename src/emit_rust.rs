//! Writes a Zeno program as Rust: `zeno build --target rust`.
//!
//! The program comes ready from lower.rs, as for C++ (emit_cpp.rs). The
//! `.rs` file holds runtime.rs, the module `z`, then the program in
//! `main`, so it builds by itself with `rustc`, and needs no crate:
//!
//! ```text
//! rustc -O --edition 2021 hello.rs && ./hello
//! ```
//!
//! Every Zeno value is a `V` (runtime.rs). Three things are Rust's own:
//!
//! - Rust has no exceptions. Every step that can fail gives a `z::R`, and
//!   the program passes a failure on with `?`.
//! - So a `try` body is a closure, called at once, whose failure the code
//!   after it looks at. A `break`, `continue` or `return` in the body
//!   cannot leave the loop or function outside the closure, so the body
//!   ends with how it ended (`z::Ctl`), and the code after it does that,
//!   as in Go (emit_go.rs).
//! - A Rust closure that outlives the function that made it owns what it
//!   uses. A variable an inner function uses is kept in a `z::Cell`, and
//!   the function is given its own copy of each cell it uses (the
//!   `captures` the lowering lists) where it is made.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId, VarKind};

/// The runtime every program built for Rust carries.
pub const RUNTIME: &str = include_str!("runtime.rs");

/// The whole `.rs` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {source}, written in Rust by `zeno build --target rust`.");
    let _ = writeln!(out, "// Build it with `rustc -O --edition 2021`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    let files: Vec<String> = program.files.iter().map(|file| text(file)).collect();
    out.push_str("\n// The program.\nfn main() {\n");
    let _ = writeln!(out, "    z::start(&[{}], || -> z::R {{", files.join(", "));
    let mut printer = Printer { program, out: String::new(), depth: 2, temps: 0, frames: Vec::new() };
    printer.block_body(&program.main.body);
    printer.line("Ok(V::None)");
    out.push_str(&printer.out);
    out.push_str("    });\n}\n");
    out
}

/// What a `break`, `continue` or `return` would leave.
#[derive(Clone, Copy, PartialEq)]
enum Frame {
    Loop,
    Try,
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
    temps: usize,
    frames: Vec<Frame>,
}

/// A Rust string literal.
fn text(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{{{code:x}}}"))
}

fn names(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|name| text(name)).collect();
    format!("&[{}]", names.join(", "))
}

impl<'a> Printer<'a> {
    fn name(&self, id: VarId) -> &'a str {
        &self.program.vars[id].ident
    }

    fn is_cell(&self, id: VarId) -> bool {
        self.program.vars[id].cell
    }

    fn is_error(&self, id: VarId) -> bool {
        self.program.vars[id].kind == VarKind::Error
    }

    fn var(&self, id: VarId) -> String {
        if self.is_cell(id) { format!("z::get(&{})", self.name(id)) } else { format!("{}.clone()", self.name(id)) }
    }

    fn temp(&mut self, base: &str) -> String {
        self.temps += 1;
        format!("{base}{}", self.temps)
    }

    fn around(&self) -> Option<Frame> {
        self.frames.last().copied()
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// A new variable holding `value`: in a cell if an inner function uses it.
    fn bind(&mut self, id: VarId, value: &str) {
        let line = if self.is_cell(id) {
            format!("let {} = z::cell({value});", self.name(id))
        } else {
            format!("let mut {} = {value};", self.name(id))
        };
        self.line(&line);
    }

    fn block_body(&mut self, block: &Block) {
        // An error's name is made by the `match` of its `try`.
        for id in &block.decls {
            if !self.is_error(*id) {
                self.bind(*id, "V::None");
            }
        }
        for statement in &block.stmts {
            self.statement(statement);
        }
    }

    fn nested(&mut self, block: &Block) {
        self.depth += 1;
        self.block_body(block);
        self.depth -= 1;
    }

    /// `return x` where the printer is: out of a `try` body it is how the
    /// body ended.
    fn give_back(&self, value: &str) -> String {
        if self.frames.contains(&Frame::Try) {
            format!("return Ok(z::Ctl::Return({value}));")
        } else {
            format!("return Ok({value});")
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Set(id, value) => {
                let value = self.expr(value);
                let line = if self.is_cell(*id) {
                    format!("z::set(&{}, {value});", self.name(*id))
                } else {
                    format!("{} = {value};", self.name(*id))
                };
                self.line(&line);
            }
            Stmt::SetField { object, name, value, line } => {
                let code = format!("z::set_field({}, {}, {}, {line})?;", self.expr(object), text(name), self.expr(value));
                self.line(&code);
            }
            Stmt::SetIndex { object, index, value, line } => {
                let code = format!("z::set_index({}, {}, {}, {line})?;", self.expr(object), self.expr(index), self.expr(value));
                self.line(&code);
            }
            Stmt::Expr(expr) => {
                let code = format!("let _ = {};", self.expr(expr));
                self.line(&code);
            }
            Stmt::If { condition, then, otherwise } => {
                let line = format!("if {} {{", self.cond(condition));
                self.line(&line);
                self.nested(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
                    if let ([Stmt::If { condition, then, otherwise: next }], true) = (&block.stmts[..], block.decls.is_empty()) {
                        let line = format!("}} else if {} {{", self.cond(condition));
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
                let line = format!("while {} {{", self.cond(condition));
                self.line(&line);
                self.frames.push(Frame::Loop);
                self.nested(body);
                self.frames.pop();
                self.line("}");
            }
            Stmt::For { var, second, items, body } => {
                let items = self.expr(items);
                match second {
                    Some(second) => {
                        let (a, b) = (self.temp("z_key"), self.temp("z_value"));
                        let line = format!("for ({a}, {b}) in {items} {{");
                        self.line(&line);
                        self.depth += 1;
                        self.bind(*var, &a);
                        self.bind(*second, &b);
                    }
                    None => {
                        let item = self.temp("z_item");
                        let line = format!("for {item} in {items} {{");
                        self.line(&line);
                        self.depth += 1;
                        self.bind(*var, &item);
                    }
                }
                self.frames.push(Frame::Loop);
                self.block_body(body);
                self.frames.pop();
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Break => match self.around() {
                Some(Frame::Try) => self.line("return Ok(z::Ctl::Break);"),
                _ => self.line("break;"),
            },
            Stmt::Continue => match self.around() {
                Some(Frame::Try) => self.line("return Ok(z::Ctl::Continue);"),
                _ => self.line("continue;"),
            },
            Stmt::Return(value) => {
                let value = self.expr(value);
                let line = self.give_back(&value);
                self.line(&line);
            }
            Stmt::Raise { value, line } => {
                let code = format!("let _ = z::raise({}, {line})?;", self.expr(value));
                self.line(&code);
            }
            Stmt::Try { body, error, handler } => {
                let result = self.temp("z_tried");
                let line = format!("let {result} = (|| -> Result<z::Ctl, z::Exc> {{");
                self.line(&line);
                self.frames.push(Frame::Try);
                self.nested(body);
                self.frames.pop();
                self.depth += 1;
                self.line("Ok(z::Ctl::Next)");
                self.depth -= 1;
                self.line("})();");
                let line = format!("match {result} {{");
                self.line(&line);
                self.depth += 1;
                self.line("Ok(z::Ctl::Next) => {}");
                let (leave, again) = match self.around() {
                    Some(Frame::Loop) => ("break;", "continue;"),
                    Some(Frame::Try) => ("return Ok(z::Ctl::Break);", "return Ok(z::Ctl::Continue);"),
                    None => ("{}", "{}"),
                };
                let line = format!("Ok(z::Ctl::Break) => {{ {leave} }}");
                self.line(&line);
                let line = format!("Ok(z::Ctl::Continue) => {{ {again} }}");
                self.line(&line);
                let line = format!("Ok(z::Ctl::Return(z_v)) => {{ {} }}", self.give_back("z_v"));
                self.line(&line);
                let line = format!("Err(z::Exc::Raise({})) => {{", self.name(*error));
                self.line(&line);
                self.nested(handler);
                self.line("}");
                self.line("Err(z_e) => return Err(z_e),");
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Reraise(error) => {
                let line = format!("return Err(z::Exc::Raise({}.clone()));", self.name(*error));
                self.line(&line);
            }
            Stmt::Fatal { message, line } => {
                let code = format!("return z::fatal({}, {line});", text(message));
                self.line(&code);
            }
            Stmt::NoMatch { value, line } => {
                let code = format!("let _ = z::no_match({}, {line})?;", self.expr(value));
                self.line(&code);
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

    fn values(&mut self, exprs: &[Expr]) -> String {
        format!("vec![{}]", self.exprs(exprs))
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) => self.var(*id),
            Expr::Caught(id) => format!("{}.clone()", self.name(*id)),
            Expr::Int(n) => {
                if *n == i64::MIN {
                    "V::Int(i64::MIN)".to_string()
                } else {
                    format!("V::Int({n})")
                }
            }
            Expr::Float(x) => format!("V::Float({})", float(*x)),
            Expr::Str(s) => format!("z::s({})", text(s)),
            Expr::Text(s) => text(s),
            Expr::Bool(b) => format!("V::Bool({b})"),
            Expr::None => "V::None".to_string(),
            Expr::Missing => "V::Missing".to_string(),
            Expr::FromCond(test) => format!("V::Bool({})", self.cond(test)),
            Expr::Builtin(name) => format!("z::builtin({}, 0)?", text(name)),
            Expr::Module(name) => format!("z::module({})", text(name)),
            Expr::CallBuiltin { name, args, line } => {
                let sized = ["u8", "u16", "u32", "i8", "i16", "i32"].contains(&name.split('.').next().unwrap_or(""));
                if sized {
                    format!("z::call_builtin({}, {}, {line})?", text(name), self.values(args))
                } else {
                    format!("z::b_{}({}, {line})?", name.replace('.', "_"), self.values(args))
                }
            }
            Expr::Call { callee, args, line } => format!("z::call({}, {}, {line})?", self.expr(callee), self.values(args)),
            Expr::CallNamed { callee, args, names: given, named, line } => format!(
                "z::call_named({}, {}, {}, {}, {line})?",
                self.expr(callee),
                self.values(args),
                names(given),
                self.values(named)
            ),
            Expr::CallDirect { function, args, line } => {
                format!("z::direct({}, {line}, {})?", self.var(*function), self.values(args))
            }
            Expr::CallMethod { object, name, args, names: given, named, line } => {
                if given.is_empty() {
                    format!("z::call_method({}, {}, {}, {line})?", self.expr(object), text(name), self.values(args))
                } else {
                    format!(
                        "z::call_method_named({}, {}, {}, {}, {}, {line})?",
                        self.expr(object),
                        text(name),
                        self.values(args),
                        names(given),
                        self.values(named)
                    )
                }
            }
            Expr::Rt { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("z::{}({args})?", step(name))
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
                        None => "V::None".to_string(),
                    })
                    .collect();
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("z::s({}), {}", text(&method.name), self.function(method))).collect();
                format!(
                    "z::class({}, {}, vec![{}], vec![{}])",
                    text(&class.name),
                    names(&fields),
                    defaults.join(", "),
                    methods.join(", ")
                )
            }
            Expr::Enum(def) => {
                let cases: Vec<String> = def.cases.iter().map(|(name, _)| name.clone()).collect();
                let fields: Vec<String> = def
                    .cases
                    .iter()
                    .map(|(_, fields)| {
                        let fields: Vec<String> = fields.iter().map(|field| text(field)).collect();
                        format!("vec![{}]", fields.join(", "))
                    })
                    .collect();
                format!("z::enum_of({}, {}, vec![{}])", text(&def.name), names(&cases), fields.join(", "))
            }
        }
    }

    /// A Zeno function: a closure of its arguments, as a `z::Func` with
    /// its Zeno name and parameters on it. It owns a copy of each cell it
    /// uses, made just before it.
    fn function(&mut self, function: &Func) -> String {
        let params: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let args = self.temp("z_args");
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 2, temps: self.temps, frames: Vec::new() };
        for (at, param) in function.params.iter().enumerate() {
            inner.bind(param.var, &format!("{args}[{at}].clone()"));
        }
        for param in &function.params {
            if let Some(default) = &param.default {
                let test = if inner.is_cell(param.var) {
                    format!("if matches!(z::get(&{}), V::Missing) {{", inner.name(param.var))
                } else {
                    format!("if matches!({}, V::Missing) {{", inner.name(param.var))
                };
                inner.line(&test);
                inner.nested(default);
                inner.line("}");
            }
        }
        inner.block_body(&function.body);
        inner.line("Ok(V::None)");
        self.temps = inner.temps;
        let pad = "    ".repeat(self.depth + 1);
        let mut code = format!("z::func({}, {}, &[{}], {{\n", text(&function.name), names(&params), defaults.join(", "));
        let mut captured: Vec<&str> = function.captures.iter().map(|id| self.name(*id)).collect();
        captured.sort_unstable();
        captured.dedup();
        for name in captured {
            let _ = writeln!(code, "{pad}let {name} = {name}.clone();");
        }
        let _ = writeln!(code, "{pad}std::rc::Rc::new(move |{args}: Vec<V>| -> z::R {{");
        code.push_str(&inner.out);
        let _ = writeln!(code, "{pad}}})");
        code.push_str(&"    ".repeat(self.depth));
        code.push_str("})");
        code
    }

    fn cond(&mut self, test: &Cond) -> String {
        match test {
            Cond::True => "true".to_string(),
            Cond::Truth(Expr::FromCond(inner), _) => self.cond(inner),
            Cond::Truth(value, line) => format!("z::truth({}, {line})?", self.expr(value)),
            Cond::Not(inner) => format!("!{}", self.cond_atom(inner)),
            Cond::And(a, b) => format!("({} && {})", self.cond(a), self.cond(b)),
            Cond::Or(a, b) => format!("({} || {})", self.cond(a), self.cond(b)),
            Cond::Test { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("z::{}({args})?", step(name))
            }
            Cond::IsMissing(id) => format!("matches!({}, V::Missing)", self.var(*id)),
        }
    }

    fn cond_atom(&mut self, test: &Cond) -> String {
        let code = self.cond(test);
        match test {
            Cond::Not(_) => format!("({code})"),
            _ => code,
        }
    }
}

/// The name a step of lower.rs has in runtime.rs, where `in` and `mod`
/// are Rust's.
fn step(name: &str) -> &str {
    match name {
        "in" => "r#in",
        "mod" => "r#mod",
        other => other,
    }
}

/// A float literal that Rust reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "f64::NAN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "f64::INFINITY" } else { "f64::NEG_INFINITY" }.to_string()
    } else {
        // `1e16` is a float in Rust; so is `2.0`.
        lower::float_literal(x)
    }
}
