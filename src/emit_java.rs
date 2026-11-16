//! Writes a Zeno program as Java: `zeno build --target java`.
//!
//! The program comes ready from lower.rs, as for JavaScript (emit_js.rs).
//! The `.java` file holds the program, in the class `ZenoProgram`, and
//! runtime.java, in the class `Z` beside it, so the file runs by itself
//! with the `java` that comes with every JDK since 11:
//!
//! ```text
//! java hello.java
//! ```
//!
//! Every Zeno value is an `Object`. Three things are Java's own:
//!
//! - A Java lambda can only use a variable that never changes, so a
//!   variable that an inner function uses is kept in a `Z.Cell`, made new
//!   where the variable is (each round of a loop has its own), and the
//!   lambda uses the cell, which never changes.
//! - Java refuses a statement it can prove is never reached, as one after
//!   `return`. Zeno does not, so `return`, `break`, `continue` and
//!   rethrowing are written `if (true) return x;`, which Java does not
//!   look into, and `while (true)` is `while (Z.TRUE)`.
//! - Java reads `\u000a` in a string as a new line in the source, so a
//!   character below a space is written as an octal escape, `\012`.

use std::fmt::Write;

use crate::lower::{self, Block, Cond, Expr, Func, Program, Stmt, VarId, VarKind};

/// The runtime every program built for Java carries.
pub const RUNTIME: &str = include_str!("runtime.java");

/// The whole `.java` file: the program, then the runtime.
pub fn emit(program: &Program, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// {}, written in Java by `zeno build --target java`.", source.replace("\\u", "\\ u"));
    let _ = writeln!(out, "// Run it with `java`. What it prints is what `zeno run` prints.");
    let _ = writeln!(out);
    let files: Vec<String> = program.files.iter().map(|file| string(file)).collect();
    out.push_str("// The program.\nclass ZenoProgram {\n    public static void main(String[] args) {\n");
    let _ = writeln!(out, "        Z.start(new String[] {{{}}}, args, () -> {{", files.join(", "));
    let mut printer = Printer { program, out: String::new(), depth: 3, temps: 0 };
    printer.block_body(&program.main.body);
    printer.line("return null;");
    out.push_str(&printer.out);
    out.push_str("        });\n    }\n}\n\n");
    out.push_str(RUNTIME);
    out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
    depth: usize,
    /// How many names of its own the printer has made: Java refuses a
    /// name in an inner block, or a lambda, that one outside it has.
    temps: usize,
}

/// A Java string literal.
fn string(text: &str) -> String {
    lower::quoted(text, |code| if code < 0x100 { format!("\\{code:03o}") } else { format!("\\u{code:04x}") })
}

fn strings(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|name| string(name)).collect();
    format!("new String[] {{{}}}", names.join(", "))
}

impl<'a> Printer<'a> {
    fn name(&self, id: VarId) -> &'a str {
        &self.program.vars[id].ident
    }

    fn is_cell(&self, id: VarId) -> bool {
        self.program.vars[id].cell
    }

    fn var(&self, id: VarId) -> String {
        if self.is_cell(id) { format!("{}.v", self.name(id)) } else { self.name(id).to_string() }
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

    /// A new variable holding `value`: in a cell if an inner function uses it.
    fn bind(&mut self, id: VarId, value: &str) {
        let line = if self.is_cell(id) {
            format!("final Z.Cell {} = new Z.Cell({value});", self.name(id))
        } else {
            format!("Object {} = {value};", self.name(id))
        };
        self.line(&line);
    }

    fn block_body(&mut self, block: &Block) {
        let plain: Vec<String> = block
            .decls
            .iter()
            .filter(|id| !self.is_cell(**id) && self.program.vars[**id].kind != VarKind::Error)
            .map(|id| format!("{} = null", self.name(*id)))
            .collect();
        if !plain.is_empty() {
            let line = format!("Object {};", plain.join(", "));
            self.line(&line);
        }
        for id in &block.decls {
            if self.is_cell(*id) && self.program.vars[*id].kind != VarKind::Error {
                let line = format!("final Z.Cell {} = new Z.Cell();", self.name(*id));
                self.line(&line);
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
                        let line = format!("for (Object[] {pair} : {items}) {{");
                        self.line(&line);
                        self.depth += 1;
                        self.bind(*var, &format!("{pair}[0]"));
                        self.bind(*second, &format!("{pair}[1]"));
                    }
                    None => {
                        let item = self.temp("z_item");
                        let line = format!("for (Object {item} : {items}) {{");
                        self.line(&line);
                        self.depth += 1;
                        self.bind(*var, &item);
                    }
                }
                self.block_body(body);
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Break => self.line("if (true) break;"),
            Stmt::Continue => self.line("if (true) continue;"),
            Stmt::Return(value) => {
                let line = format!("if (true) return {};", self.expr(value));
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
                let line = format!("if (true) throw {};", self.name(*error));
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
            format!("new Object[] {{{}}}", self.exprs(exprs))
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
                    "Z.cls({}, {}, new Object[] {{{}}}, Z.methods({}))",
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
                        format!("{{{}}}", fields.join(", "))
                    })
                    .collect();
                format!("Z.enumOf({}, {}, new String[][] {{{}}})", string(&def.name), strings(&names), fields.join(", "))
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
            "Z.fn({}, {}, new boolean[] {{{}}}, (Object[] {args}) -> {{\n",
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
            Cond::True => "Z.TRUE".to_string(),
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

/// The name a step of lower.rs has in runtime.java, where `case` is Java's.
fn step(name: &str) -> &str {
    match name {
        "case" => "case_",
        other => other,
    }
}

/// A float literal that Java reads as the same number.
fn float(x: f64) -> String {
    if x.is_nan() {
        "Double.NaN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "Double.POSITIVE_INFINITY" } else { "Double.NEGATIVE_INFINITY" }.to_string()
    } else {
        // `1e16` is a double in Java, and so is `2.0`.
        lower::float_literal(x)
    }
}
