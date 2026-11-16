//! Writes a Zeno program as Go: `zeno build --target go`.
//!
//! The program comes ready from lower.rs, as for JavaScript (emit_js.rs),
//! and every value is a value of runtime.go, which is put in front of the
//! program, so the `.go` file runs by itself:
//!
//! ```text
//! go run hello.go
//! ```
//!
//! Every Zeno value is a `ZV` (Go's `any`), so every variable is one. Go
//! closures share the variables they use, as Zeno's do, and a variable
//! made inside a loop is a new one each round, so a function made in a
//! loop needs nothing more.
//!
//! Three things are Go's own:
//!
//! - Go refuses a variable that nothing reads, so each one is read once
//!   where it is made: `_ = x`.
//! - Go has no `try`. A `try` is a call of `Ztry` with the body and the
//!   handler as functions, which recovers a `ZRaise` panic. A `break`,
//!   `continue` or `return` inside them cannot leave the loop or the
//!   function outside, so each returns how it ended (1, 2, 3 and the
//!   value), and the code after the call does it.
//! - Go's stack overflow cannot be caught, so the runtime counts nested
//!   calls and stops at 1000 with `zeno run`'s message, as `zeno run` does.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId};

/// The runtime every program built for Go carries.
pub const RUNTIME: &str = include_str!("runtime.go");

/// The whole `.go` file: the runtime, then the program.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {source}, written in Go by `zeno build --target go`.");
    let _ = writeln!(out, "// Run it with `go run`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    out.push_str(RUNTIME);
    out.push_str("\n// The program.\n");
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    let _ = writeln!(out, "func main() {{\n\tZstart([]string{{{}}}, func() ZV {{", files.join(", "));
    let mut printer = Printer { program, out: String::new(), depth: 2, frames: Vec::new() };
    printer.block_body(&program.main.body);
    printer.line("return nil");
    out.push_str(&printer.out);
    out.push_str("\t})\n}\n");
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
    /// The loops and `try`s around the statement, in the function it is in.
    frames: Vec<Frame>,
}

/// A Go string literal.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{code:04x}"))
}

impl<'a> Printer<'a> {
    fn name(&self, id: VarId) -> &'a str {
        &self.program.vars[id].ident
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push('\t');
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// The statements of a block, after its variables.
    fn block_body(&mut self, block: &Block) {
        if !block.decls.is_empty() {
            let names: Vec<&str> = block.decls.iter().map(|id| self.name(*id)).collect();
            let line = format!("var {} ZV", names.join(", "));
            self.line(&line);
            self.read_all(&names);
        }
        for statement in &block.stmts {
            self.statement(statement);
        }
    }

    /// `_, _ = a, b`: Go refuses a variable nothing reads.
    fn read_all(&mut self, names: &[&str]) {
        // `_` (as in `for _ in range(3)`) is Go's own blank, read by nothing.
        let names: Vec<&str> = names.iter().copied().filter(|name| *name != "_").collect();
        if names.is_empty() {
            return;
        }
        let blanks = vec!["_"; names.len()];
        let line = format!("{} = {}", blanks.join(", "), names.join(", "));
        self.line(&line);
    }

    fn nested(&mut self, block: &Block) {
        self.depth += 1;
        self.block_body(block);
        self.depth -= 1;
    }

    /// `x := value`, a new variable; nothing for `_`.
    fn bind(&mut self, id: VarId, value: &str) {
        if self.name(id) != "_" {
            let line = format!("{} := {value}", self.name(id));
            self.line(&line);
        }
    }

    /// The innermost loop or `try`, if there is one.
    fn around(&self) -> Option<Frame> {
        self.frames.last().copied()
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Set(id, value) => {
                let line = format!("{} = {}", self.name(*id), self.expr(value));
                self.line(&line);
            }
            Stmt::SetField { object, name, value, line } => {
                let text = format!("Zset_field({}, {}, {}, {line})", self.expr(object), string(name), self.expr(value));
                self.line(&text);
            }
            Stmt::SetIndex { object, index, value, line } => {
                let text = format!("Zset_index({}, {}, {}, {line})", self.expr(object), self.expr(index), self.expr(value));
                self.line(&text);
            }
            Stmt::Expr(expr) => {
                let line = format!("_ = {}", self.expr(expr));
                self.line(&line);
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
                let line = format!("for {} {{", self.cond(condition));
                self.line(&line);
                self.frames.push(Frame::Loop);
                self.nested(body);
                self.frames.pop();
                self.line("}");
            }
            Stmt::For { var, second, items, body } => {
                let items = self.expr(items);
                let item = format!("z_item{}", self.depth);
                let line = format!("for _, {item} := range {items} {{");
                self.line(&line);
                self.depth += 1;
                // A new variable each round, as in Zeno.
                match second {
                    Some(second) => {
                        self.bind(*var, &format!("{item}[0]"));
                        self.bind(*second, &format!("{item}[1]"));
                        let names = [self.name(*var), self.name(*second)];
                        self.read_all(&names);
                    }
                    None => {
                        self.bind(*var, &item);
                        let names = [self.name(*var)];
                        self.read_all(&names);
                    }
                }
                self.frames.push(Frame::Loop);
                self.block_body(body);
                self.frames.pop();
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Break => match self.around() {
                Some(Frame::Try) => self.line("return 1, nil"),
                _ => self.line("break"),
            },
            Stmt::Continue => match self.around() {
                Some(Frame::Try) => self.line("return 2, nil"),
                _ => self.line("continue"),
            },
            Stmt::Return(value) => {
                let value = self.expr(value);
                let line = if self.frames.contains(&Frame::Try) { format!("return 3, {value}") } else { format!("return {value}") };
                self.line(&line);
            }
            Stmt::Raise { value, line } => {
                let text = format!("Zraise({}, {line})", self.expr(value));
                self.line(&text);
            }
            Stmt::Try { body, error, handler } => {
                self.line("if z_how, z_value := Ztry(func() (int, ZV) {");
                self.frames.push(Frame::Try);
                self.nested(body);
                self.depth += 1;
                self.line("return 0, nil");
                self.depth -= 1;
                let line = format!("}}, func({} *ZRaise) (int, ZV) {{", self.name(*error));
                self.line(&line);
                self.nested(handler);
                self.depth += 1;
                self.line("return 0, nil");
                self.depth -= 1;
                self.frames.pop();
                // Do what the body or the handler ended with.
                let mut tail = String::new();
                match self.around() {
                    Some(Frame::Loop) => tail.push_str("z_how == 1 {\n\tbreak\n} else if z_how == 2 {\n\tcontinue\n} else if "),
                    Some(Frame::Try) => {
                        tail.push_str("z_how == 1 || z_how == 2 {\n\treturn z_how, nil\n} else if ")
                    }
                    None => {}
                }
                if self.frames.contains(&Frame::Try) {
                    tail.push_str("z_how == 3 {\n\treturn 3, z_value\n}");
                } else {
                    tail.push_str("z_how == 3 {\n\treturn z_value\n}");
                }
                let mut lines = tail.lines();
                let first = format!("}}); {}", lines.next().unwrap_or(""));
                self.line(&first);
                for rest in lines {
                    self.line(rest);
                }
            }
            Stmt::Reraise(error) => {
                let line = format!("panic({})", self.name(*error));
                self.line(&line);
            }
            Stmt::Fatal { message, line } => {
                let text = format!("Zfatal({}, {line})", string(message));
                self.line(&text);
            }
            Stmt::NoMatch { value, line } => {
                let text = format!("Zno_match({}, {line})", self.expr(value));
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

    fn strings(names: &[String]) -> String {
        let names: Vec<String> = names.iter().map(|name| string(name)).collect();
        format!("[]string{{{}}}", names.join(", "))
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) | Expr::Caught(id) => self.name(*id).to_string(),
            Expr::Int(n) => format!("int64({n})"),
            Expr::Float(x) => float(*x),
            Expr::Str(text) | Expr::Text(text) => string(text),
            Expr::Bool(b) => b.to_string(),
            Expr::None => "nil".to_string(),
            Expr::Missing => "ZMISSING".to_string(),
            Expr::FromCond(test) => self.cond(test),
            Expr::Builtin(name) => format!("Zbuiltin({}, 0)", string(name)),
            Expr::Module(name) => format!("Zmodule({})", string(name)),
            Expr::CallBuiltin { name, args, line } => {
                format!("Zb_{}([]ZV{{{}}}, {line})", name.replace('.', "_"), self.exprs(args))
            }
            Expr::Call { callee, args, line } => format!("Zcall({}, []ZV{{{}}}, {line})", self.expr(callee), self.exprs(args)),
            Expr::CallNamed { callee, args, names, named, line } => format!(
                "Zcall_named({}, []ZV{{{}}}, {}, []ZV{{{}}}, {line})",
                self.expr(callee),
                self.exprs(args),
                Self::strings(names),
                self.exprs(named)
            ),
            Expr::CallDirect { function, args, line } => {
                let args = self.exprs(args);
                if args.is_empty() {
                    format!("Zdirect({}, {line})", self.name(*function))
                } else {
                    format!("Zdirect({}, {line}, {args})", self.name(*function))
                }
            }
            Expr::CallMethod { object, name, args, names, named, line } => {
                if names.is_empty() {
                    format!("Zcall_method({}, {}, []ZV{{{}}}, {line})", self.expr(object), string(name), self.exprs(args))
                } else {
                    format!(
                        "Zcall_method_named({}, {}, []ZV{{{}}}, {}, []ZV{{{}}}, {line})",
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
                format!("Z{name}({args})")
            }
            Expr::Count(n) => n.to_string(),
            Expr::Array(items) => format!("[]ZV{{{}}}", self.exprs(items)),
            Expr::Func(function) => self.function(function),
            Expr::Class(class) => {
                let fields: Vec<String> = class.fields.iter().map(|(name, _)| name.clone()).collect();
                let defaults: Vec<String> = class
                    .fields
                    .iter()
                    .map(|(_, default)| match default {
                        Some(function) => self.function(function),
                        None => "nil".to_string(),
                    })
                    .collect();
                let methods: Vec<String> =
                    class.methods.iter().map(|method| format!("{}: {}", string(&method.name), self.function(method))).collect();
                format!(
                    "Zclass({}, {}, []ZV{{{}}}, map[string]*ZFunc{{{}}})",
                    string(&class.name),
                    Self::strings(&fields),
                    defaults.join(", "),
                    methods.join(", ")
                )
            }
            Expr::Enum(def) => {
                let cases: Vec<String> =
                    def.cases.iter().map(|(name, fields)| format!("{{{}, {}}}", string(name), Self::strings(fields))).collect();
                format!("Zenum({}, []ZCaseDef{{{}}})", string(&def.name), cases.join(", "))
            }
        }
    }

    /// A Zeno function: a Go function of its arguments, as a `*ZFunc` with
    /// its Zeno name and parameters on it.
    fn function(&mut self, function: &Func) -> String {
        let names: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 1, frames: Vec::new() };
        let params: Vec<&str> = function.params.iter().map(|param| inner.name(param.var)).collect();
        for (at, param) in function.params.iter().enumerate() {
            inner.bind(param.var, &format!("z_args[{at}]"));
        }
        if !params.is_empty() {
            inner.read_all(&params);
        }
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if {} == ZMISSING {{", inner.name(param.var));
                inner.line(&line);
                inner.nested(default);
                inner.line("}");
            }
        }
        inner.block_body(&function.body);
        if !matches!(function.body.stmts.last(), Some(Stmt::Return(_))) {
            inner.line("return nil");
        }
        let args = if params.is_empty() { "_" } else { "z_args" };
        let mut text = format!(
            "Zfn({}, {}, []bool{{{}}}, func({args} []ZV) ZV {{\n",
            string(&function.name),
            Self::strings(&names),
            defaults.join(", ")
        );
        text.push_str(&inner.out);
        for _ in 0..self.depth {
            text.push('\t');
        }
        text.push_str("})");
        text
    }

    fn cond(&mut self, test: &Cond) -> String {
        match test {
            Cond::True => "true".to_string(),
            Cond::Truth(Expr::FromCond(inner), _) => self.cond(inner),
            Cond::Truth(value, line) => format!("Ztruth({}, {line})", self.expr(value)),
            Cond::Not(inner) => format!("!{}", self.cond_atom(inner)),
            Cond::And(a, b) => format!("({} && {})", self.cond(a), self.cond(b)),
            Cond::Or(a, b) => format!("({} || {})", self.cond(a), self.cond(b)),
            Cond::Test { name, args, line } => {
                let mut args = self.exprs(args);
                if lower::takes_line(name) {
                    let _ = write!(args, ", {line}");
                }
                format!("Z{name}({args})")
            }
            Cond::IsMissing(id) => format!("({} == ZMISSING)", self.name(*id)),
        }
    }

    fn cond_atom(&mut self, test: &Cond) -> String {
        let text = self.cond(test);
        match test {
            Cond::Not(_) => format!("({text})"),
            _ => text,
        }
    }
}

/// A float literal that Go reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "math.NaN()".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "math.Inf(1)" } else { "math.Inf(-1)" }.to_string()
    } else {
        format!("float64({})", lower::float_literal(x))
    }
}
