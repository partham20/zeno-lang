//! Writes a Zeno program as C#: `zeno build --target cs`.
//!
//! The program comes ready from lower.rs, as for Java (emit_java.rs), and
//! the two printers are close. The `.cs` file holds the program, as
//! top-level statements, then runtime.cs, the class `Z`, so the file runs
//! by itself with the `dotnet` of .NET 10 or later:
//!
//! ```text
//! dotnet run hello.cs
//! ```
//!
//! Every Zeno value is an `object`. C# is easier than Java in two ways: a
//! lambda can change a variable outside it, so no variable needs a cell,
//! and a statement that is never reached is only a warning, which the
//! file turns off. The one thing to mind is that C# refuses a name in an
//! inner block that one outside it has, so every name the printer makes
//! has a number of its own.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId, VarKind};

/// The runtime every program built for C# carries.
pub const RUNTIME: &str = include_str!("runtime.cs");

/// The whole `.cs` file: the program, then the runtime.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {source}, written in C# by `zeno build --target cs`.");
    let _ = writeln!(out, "// Run it with `dotnet run`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    // A statement after `return` is a warning in C#, and a Zeno program
    // may have one; so may a variable it never reads.
    out.push_str("#pragma warning disable\n\n");
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    out.push_str("// The program.\n");
    let _ = writeln!(out, "Z.start(new string[] {{{}}}, args, () => {{", files.join(", "));
    let mut printer = Printer { program, out: String::new(), depth: 1, temps: 0 };
    printer.block_body(&program.main.body);
    printer.line("return null;");
    out.push_str(&printer.out);
    out.push_str("});\n\n");
    out.push_str(RUNTIME);
    out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
    /// How many names of its own the printer has made: C# refuses a
    /// name in an inner block, or a lambda, that one outside it has.
    temps: usize,
}

/// A C# string literal.
fn string(text: &str) -> String {
    lower::quoted(text, |code| format!("\\u{code:04x}"))
}

fn strings(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|name| string(name)).collect();
    format!("new string[] {{{}}}", names.join(", "))
}

impl<'a> Printer<'a> {
    fn name(&self, id: VarId) -> &'a str {
        &self.program.vars[id].ident
    }

    fn var(&self, id: VarId) -> String {
        self.name(id).to_string()
    }

    fn temp(&mut self, base: &str) -> String {
        self.temps += 1;
        format!("{base}{}", self.temps)
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// A new variable holding `value`.
    fn bind(&mut self, id: VarId, value: &str) {
        let line = format!("object {} = {value};", self.name(id));
        self.line(&line);
    }

    fn block_body(&mut self, block: &Block) {
        // An error's name is the `catch`'s own, so it is not made here.
        let names: Vec<String> = block
            .decls
            .iter()
            .filter(|id| self.program.vars[**id].kind != VarKind::Error)
            .map(|id| format!("{} = null", self.name(*id)))
            .collect();
        if !names.is_empty() {
            let line = format!("object {};", names.join(", "));
            self.line(&line);
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

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Set(id, value) => {
                let line = format!("{} = {};", self.var(*id), self.expr(value));
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
                let text = self.expr(expr);
                let call = matches!(
                    expr,
                    Expr::Call { .. }
                        | Expr::CallNamed { .. }
                        | Expr::CallDirect { .. }
                        | Expr::CallMethod { .. }
                        | Expr::CallBuiltin { .. }
                        | Expr::Rt { .. }
                );
                let line = if call { format!("{text};") } else { format!("Z.ignore({text});") };
                self.line(&line);
            }
            Stmt::If { condition, then, otherwise } => {
                let line = format!("if ({}) {{", self.cond(condition));
                self.line(&line);
                self.nested(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
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
                        let pair = self.temp("z_pair");
                        let line = format!("foreach (object[] {pair} in {items}) {{");
                        self.line(&line);
                        self.depth += 1;
                        self.bind(*var, &format!("{pair}[0]"));
                        self.bind(*second, &format!("{pair}[1]"));
                    }
                    None => {
                        let item = self.temp("z_item");
                        let line = format!("foreach (object {item} in {items}) {{");
                        self.line(&line);
                        self.depth += 1;
                        self.bind(*var, &item);
                    }
                }
                self.block_body(body);
                self.depth -= 1;
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
                let line = format!("}} catch (Z.Raise {}) {{", self.name(*error));
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
        if exprs.is_empty() {
            "Z.NO_VALUES".to_string()
        } else {
            format!("new object[] {{{}}}", self.exprs(exprs))
        }
    }

    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Var(id) => self.var(*id),
            Expr::Caught(id) => self.name(*id).to_string(),
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
                let sized = ["u8", "u16", "u32", "i8", "i16", "i32"].contains(&name.split('.').next().unwrap_or(""));
                if sized {
                    format!("Z.call_builtin({}, {}, {line})", string(name), self.values(args))
                } else {
                    format!("Z.b_{}({}, {line})", name.replace('.', "_"), self.values(args))
                }
            }
            Expr::Call { callee, args, line } => format!("Z.call({}, {}, {line})", self.expr(callee), self.values(args)),
            Expr::CallNamed { callee, args, names, named, line } => format!(
                "Z.call_named({}, {}, {}, {}, {line})",
                self.expr(callee),
                self.values(args),
                strings(names),
                self.values(named)
            ),
            Expr::CallDirect { function, args, line } => {
                format!("Z.direct({}, {line}, {})", self.var(*function), self.values(args))
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
                    class.methods.iter().map(|method| format!("{}, {}", string(&method.name), self.function(method))).collect();
                format!(
                    "Z.cls({}, {}, new object[] {{{}}}, Z.methods({}))",
                    string(&class.name),
                    strings(&fields),
                    defaults.join(", "),
                    methods.join(", ")
                )
            }
            Expr::Enum(def) => {
                let names: Vec<String> = def.cases.iter().map(|(name, _)| name.clone()).collect();
                let fields: Vec<String> = def
                    .cases
                    .iter()
                    .map(|(_, fields)| {
                        let fields: Vec<String> = fields.iter().map(|field| string(field)).collect();
                        format!("new string[] {{{}}}", fields.join(", "))
                    })
                    .collect();
                format!("Z.enumOf({}, {}, new string[][] {{{}}})", string(&def.name), strings(&names), fields.join(", "))
            }
        }
    }

    /// A Zeno function: a lambda of its arguments, as a `Z.Func` with its
    /// Zeno name and parameters on it.
    fn function(&mut self, function: &Func) -> String {
        let names: Vec<String> = function.params.iter().map(|param| param.name.clone()).collect();
        let defaults: Vec<&str> = function.params.iter().map(|param| if param.default.is_some() { "true" } else { "false" }).collect();
        let args = self.temp("z_args");
        let mut inner = Printer { program: self.program, out: String::new(), depth: self.depth + 1, temps: self.temps };
        for (at, param) in function.params.iter().enumerate() {
            inner.bind(param.var, &format!("{args}[{at}]"));
        }
        for param in &function.params {
            if let Some(default) = &param.default {
                let line = format!("if ({} == Z.MISSING) {{", inner.var(param.var));
                inner.line(&line);
                inner.nested(default);
                inner.line("}");
            }
        }
        inner.block_body(&function.body);
        inner.line("return null;");
        self.temps = inner.temps;
        let mut text = format!(
            "Z.fn({}, {}, new bool[] {{{}}}, (object[] {args}) => {{\n",
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
            Cond::IsMissing(id) => format!("({} == Z.MISSING)", self.var(*id)),
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

/// The name a step of lower.rs has in runtime.cs, where `case` and `in` are C#'s.
fn step(name: &str) -> &str {
    match name {
        "case" => "@case",
        "in" => "@in",
        other => other,
    }
}

/// A float literal that C# reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "double.NaN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "double.PositiveInfinity" } else { "double.NegativeInfinity" }.to_string()
    } else {
        // `1e16` is a double in C#, and so is `2.0`.
        lower::float_literal(x)
    }
}
