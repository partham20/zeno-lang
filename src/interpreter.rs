//! The interpreter: runs a program by walking its syntax tree.
//!
//! There are two central functions, one for each kind of tree node:
//!
//! - `exec` runs a statement. It returns nothing, but changes things:
//!   declares a variable, prints, loops.
//! - `eval` works out an expression and returns its `Value`.
//!
//! Both are a `match` on the node kind, and both call themselves for the
//! nodes nested inside. That is all a "tree-walking" interpreter is.
//!
//! Types written in the source (`x: int`) are not checked here. The
//! interpreter finds mistakes only when it reaches them; the type checker,
//! the next stage, will find them before the program runs.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::builtins;
use crate::error::Error;
use crate::token::Span;
use crate::units;
use crate::value::*;

/// How deep function calls may nest before we assume runaway recursion.
const MAX_DEPTH: usize = 1000;
/// The interpreter calls itself for every nested node, so deep Zeno
/// recursion needs a deep Rust stack. It runs on its own thread to get one.
const STACK_SIZE: usize = 256 * 1024 * 1024;

/// Run a program. With `capture` the printed text is returned instead of
/// going to the screen, which is how the tests check output.
pub fn run(program: &Program, capture: bool) -> (String, Result<(), Error>) {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || {
                let mut interpreter = Interpreter { output: capture.then(String::new), depth: 0 };
                // Built-ins live in the outermost scope; the program gets its own inside it.
                let env = Scope::child(&builtins::globals());
                let result = interpreter
                    .run_statements(&program.statements, &env)
                    .map_err(top_level_error);
                (interpreter.output.unwrap_or_default(), result)
            })
            .expect("could not start the interpreter thread")
            .join()
            .expect("the interpreter crashed")
    })
}

/// What to report when something escapes the whole program.
fn top_level_error(control: Control) -> Error {
    match control {
        Control::Fatal(error) => error,
        Control::Raise(value, span) => {
            Error::new(format!("uncaught {}: {value}", error_kind(&value)), span)
        }
        Control::Break(span) => Error::new("`break` outside a loop", span),
        Control::Continue(span) => Error::new("`continue` outside a loop", span),
        Control::Return(_, span) => Error::new("`return` outside a function", span),
    }
}

/// The name `except` matches against: the error's kind, or an object's class.
fn error_kind(value: &Value) -> String {
    match value {
        Value::Error(error) => error.kind.clone(),
        Value::Instance(instance) => instance.class.name.clone(),
        _ => "Error".to_string(),
    }
}

// --------------------------------------------------------------------- Scopes

/// A scope is one set of variables. Scopes nest: each `{ }` block gets a new
/// one whose `parent` is the scope around it. Looking up a name walks outward
/// until it is found.
pub struct Scope {
    variables: RefCell<HashMap<String, Variable>>,
    parent: Option<Env>,
}

/// Scopes are shared (`Rc`) because a function keeps hold of the scope it
/// was defined in for as long as the function exists.
pub type Env = Rc<Scope>;

struct Variable {
    value: Value,
    /// True for `var`, false for `let`, `const`, parameters and loop variables.
    mutable: bool,
}

enum AssignError {
    NotDeclared,
    Immutable,
}

impl Scope {
    pub fn root() -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: None })
    }

    pub fn child(parent: &Env) -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: Some(parent.clone()) })
    }

    /// Create a variable in this scope. Returns false if the name is taken here.
    pub fn declare(&self, name: &str, value: Value, mutable: bool) -> bool {
        let mut variables = self.variables.borrow_mut();
        if variables.contains_key(name) {
            return false;
        }
        variables.insert(name.to_string(), Variable { value, mutable });
        true
    }

    fn get(&self, name: &str) -> Option<Value> {
        if let Some(variable) = self.variables.borrow().get(name) {
            return Some(variable.value.clone());
        }
        self.parent.as_ref()?.get(name)
    }

    fn assign(&self, name: &str, value: Value) -> Result<(), AssignError> {
        if let Some(variable) = self.variables.borrow_mut().get_mut(name) {
            if !variable.mutable {
                return Err(AssignError::Immutable);
            }
            variable.value = value;
            return Ok(());
        }
        match &self.parent {
            Some(parent) => parent.assign(name, value),
            None => Err(AssignError::NotDeclared),
        }
    }
}

// ---------------------------------------------------------------- Interpreter

pub struct Interpreter {
    /// `Some` collects printed text; `None` prints straight to the screen.
    output: Option<String>,
    /// How many function calls are currently in progress.
    depth: usize,
}

impl Interpreter {
    pub fn write_line(&mut self, text: &str) {
        match &mut self.output {
            Some(buffer) => {
                buffer.push_str(text);
                buffer.push('\n');
            }
            None => println!("{text}"),
        }
    }

    fn declare(
        &self,
        env: &Env,
        name: &str,
        value: Value,
        mutable: bool,
        span: Span,
    ) -> Result<(), Control> {
        if env.declare(name, value, mutable) {
            Ok(())
        } else {
            fatal(format!("`{name}` is already declared in this block"), span)
        }
    }

    // ------------------------------------------------------------ Statements

    /// Run a `{ }` block in a fresh scope of its own.
    fn exec_block(&mut self, statements: &[Stmt], env: &Env) -> Result<(), Control> {
        self.run_statements(statements, &Scope::child(env))
    }

    /// Run statements in the given scope, one after another.
    fn run_statements(&mut self, statements: &[Stmt], env: &Env) -> Result<(), Control> {
        self.hoist(statements, env)?;
        for statement in statements {
            self.exec(statement, env)?;
        }
        Ok(())
    }

    /// Declare the block's functions, classes and enums before running any
    /// of it, so they can be used above the line where they are written.
    fn hoist(&mut self, statements: &[Stmt], env: &Env) -> Result<(), Control> {
        for statement in statements {
            let span = statement.span;
            match &statement.kind {
                StmtKind::Function(decl) => {
                    let closure = Closure { decl: decl.clone(), env: env.clone() };
                    self.declare(env, &decl.name, Value::Function(Rc::new(closure)), false, span)?;
                }
                StmtKind::Class(class) => {
                    let methods = class
                        .methods
                        .iter()
                        .map(|decl| {
                            let closure = Closure { decl: decl.clone(), env: env.clone() };
                            (decl.name.clone(), Rc::new(closure))
                        })
                        .collect();
                    let def = ClassDef {
                        name: class.name.clone(),
                        fields: class.fields.clone(),
                        methods,
                        env: env.clone(),
                    };
                    self.declare(env, &class.name, Value::Class(Rc::new(def)), false, span)?;
                }
                StmtKind::Enum(decl) => {
                    let def = Rc::new(EnumDef { name: decl.name.clone(), cases: decl.variants.clone() });
                    self.declare(env, &decl.name, Value::Enum(def.clone()), false, span)?;
                    // Each case is also a name of its own, so you can write
                    // `Circle(1.0)` as well as `Shape.Circle(1.0)`.
                    for (index, case) in decl.variants.iter().enumerate() {
                        self.declare(env, &case.name, case_value(&def, index), false, span)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn exec(&mut self, statement: &Stmt, env: &Env) -> Result<(), Control> {
        let span = statement.span;

        match &statement.kind {
            StmtKind::Let { mutable, name, value, .. } => {
                let value = self.eval(value, env)?;
                self.declare(env, name, value, *mutable, span)
            }
            StmtKind::Const { name, value, .. } => {
                let value = self.eval(value, env)?;
                self.declare(env, name, value, false, span)
            }
            StmtKind::Assign { target, op, value } => {
                let mut new = self.eval(value, env)?;
                if let Some(op) = op {
                    // `x += 1` is `x = x + 1`.
                    let old = self.eval(target, env)?;
                    new = self.binary(*op, old, new, span)?;
                }
                self.assign(target, new, env)
            }
            StmtKind::Expr(expr) => {
                self.eval(expr, env)?;
                Ok(())
            }

            // Already declared by `hoist`.
            StmtKind::Function(_) | StmtKind::Class(_) | StmtKind::Enum(_) => Ok(()),

            StmtKind::If { condition, then_block, else_block } => {
                if self.condition(condition, env)? {
                    self.exec_block(then_block, env)
                } else if let Some(else_block) = else_block {
                    self.exec_block(else_block, env)
                } else {
                    Ok(())
                }
            }
            StmtKind::While { condition, body } => {
                while self.condition(condition, env)? {
                    match self.exec_block(body, env) {
                        Ok(()) | Err(Control::Continue(_)) => {}
                        Err(Control::Break(_)) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            StmtKind::For { variable, iterable, body } => {
                let iterable_span = iterable.span;
                let iterable = self.eval(iterable, env)?;
                for item in self.items(&iterable, iterable_span)? {
                    // A fresh scope and a fresh loop variable for every round.
                    let scope = Scope::child(env);
                    scope.declare(variable, item, false);
                    match self.run_statements(body, &scope) {
                        Ok(()) | Err(Control::Continue(_)) => {}
                        Err(Control::Break(_)) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            StmtKind::Match { subject, arms } => {
                let value = self.eval(subject, env)?;
                for arm in arms {
                    // The pattern's variables live in the arm's own scope.
                    let scope = Scope::child(env);
                    if self.matches(&arm.pattern, &value, &scope, span)? {
                        return self.run_statements(&arm.body, &scope);
                    }
                }
                fatal(format!("no `case` matches {}", value.repr()), span)
            }
            StmtKind::Try { body, handlers } => {
                let (error, raised_at) = match self.exec_block(body, env) {
                    Err(Control::Raise(error, span)) => (error, span),
                    other => return other,
                };
                let kind = error_kind(&error);
                for handler in handlers {
                    // A bare `except` and `except Error` both catch everything.
                    let catches = match &handler.error {
                        None => true,
                        Some(name) => name == "Error" || *name == kind,
                    };
                    if catches {
                        let scope = Scope::child(env);
                        if let Some(binding) = &handler.binding {
                            scope.declare(binding, error, false);
                        }
                        return self.run_statements(&handler.body, &scope);
                    }
                }
                Err(Control::Raise(error, raised_at))
            }

            StmtKind::Return(value) => {
                let value = match value {
                    Some(value) => self.eval(value, env)?,
                    None => Value::None,
                };
                Err(Control::Return(value, span))
            }
            StmtKind::Raise(value) => {
                let error = self.eval(value, env)?;
                match error {
                    Value::Error(_) | Value::Instance(_) => Err(Control::Raise(error, span)),
                    other => fatal(
                        format!("only an error can be raised, and this is {}", other.type_name()),
                        span,
                    ),
                }
            }
            StmtKind::Break => Err(Control::Break(span)),
            StmtKind::Continue => Err(Control::Continue(span)),

            StmtKind::Import { module, names } => {
                let path = module.join(".");
                let Some(found) = builtins::module(&path) else {
                    return fatal(
                        format!("there is no module `{path}`. So far Zeno has `math` and `time`"),
                        span,
                    );
                };
                if names.is_empty() {
                    // `import math` makes the name `math`.
                    return self.declare(env, found.name, Value::Module(found.clone()), false, span);
                }
                for name in names {
                    let Some(member) = found.members.get(name.as_str()) else {
                        return fatal(format!("module `{path}` has no `{name}`"), span);
                    };
                    self.declare(env, name, member.clone(), false, span)?;
                }
                Ok(())
            }
        }
    }

    /// Work out a condition. It has to be a real bool; see docs/decisions.md.
    fn condition(&mut self, condition: &Expr, env: &Env) -> Result<bool, Control> {
        match self.eval(condition, env)? {
            Value::Bool(b) => Ok(b),
            other => fatal(
                format!("a condition must be true or false, but this is {}", other.type_name()),
                condition.span,
            ),
        }
    }

    /// The things a `for` loop or comprehension steps through.
    fn items(&mut self, value: &Value, span: Span) -> Result<Vec<Value>, Control> {
        match value {
            // A copy, so changing the list inside the loop cannot upset the loop.
            Value::List(items) => Ok(items.borrow().clone()),
            Value::Str(text) => Ok(text.chars().map(|c| Value::str(c.to_string())).collect()),
            other => fatal(format!("cannot loop over {}", other.type_name()), span),
        }
    }

    fn assign(&mut self, target: &Expr, value: Value, env: &Env) -> Result<(), Control> {
        let span = target.span;
        match &target.kind {
            ExprKind::Name(name) => match env.assign(name, value) {
                Ok(()) => Ok(()),
                Err(AssignError::NotDeclared) => fatal(
                    format!("`{name}` is not declared. Create it with `let` or `var` first"),
                    span,
                ),
                Err(AssignError::Immutable) => fatal(
                    format!("`{name}` cannot be changed. Declare it with `var` to allow that"),
                    span,
                ),
            },
            ExprKind::Field { object, name } => match self.eval(object, env)? {
                Value::Instance(instance) => {
                    match instance.fields.borrow_mut().get_mut(name) {
                        Some(slot) => *slot = value,
                        None => {
                            return fatal(
                                format!("`{}` has no field `{name}`", instance.class.name),
                                span,
                            )
                        }
                    }
                    Ok(())
                }
                other => fatal(format!("cannot set a field on {}", other.type_name()), span),
            },
            ExprKind::Index { object, index } => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                match object {
                    Value::List(items) => {
                        let len = items.borrow().len();
                        let i = self.index(&index, len, "list", span)?;
                        items.borrow_mut()[i] = value;
                        Ok(())
                    }
                    other => fatal(format!("cannot set an item of {}", other.type_name()), span),
                }
            }
            // The parser only lets the three kinds above through.
            _ => fatal("this cannot be assigned to", span),
        }
    }

    /// Does `value` fit `pattern`? Names in the pattern are declared in `scope`.
    fn matches(
        &mut self,
        pattern: &Pattern,
        value: &Value,
        scope: &Env,
        span: Span,
    ) -> Result<bool, Control> {
        match pattern {
            Pattern::Wildcard => Ok(true),
            Pattern::Literal(literal) => {
                let literal = self.eval(literal, scope)?;
                Ok(equal(&literal, value))
            }
            Pattern::Name(name) => {
                // A name that is already a bare enum case, like `Idle`, is
                // compared. Any other name becomes a new variable holding the value.
                if let Some(Value::Case(case)) = scope.get(name) {
                    if case.fields.is_empty() && case.name() == name {
                        return Ok(equal(&Value::Case(case), value));
                    }
                }
                scope.declare(name, value.clone(), false);
                Ok(true)
            }
            Pattern::Variant { name, fields } => {
                let Some(Value::CaseConstructor(def, index)) = scope.get(name) else {
                    return fatal(format!("`{name}` is not an enum case that carries values"), span);
                };
                let Value::Case(case) = value else {
                    return Ok(false);
                };
                if !Rc::ptr_eq(&case.def, &def) || case.index != index {
                    return Ok(false);
                }
                if fields.len() != case.fields.len() {
                    return fatal(
                        format!(
                            "`{name}` carries {} value(s), but this pattern has {}",
                            case.fields.len(),
                            fields.len()
                        ),
                        span,
                    );
                }
                for (pattern, value) in fields.iter().zip(&case.fields) {
                    if !self.matches(pattern, value, scope, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    // ----------------------------------------------------------- Expressions

    fn eval(&mut self, expr: &Expr, env: &Env) -> Result<Value, Control> {
        let span = expr.span;

        match &expr.kind {
            ExprKind::Int(n) => match i64::try_from(*n) {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => fatal("this number is too big for an int", span),
            },
            ExprKind::Float(x) => Ok(Value::Float(*x)),
            ExprKind::Str(text) => Ok(Value::str(text)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::None => Ok(Value::None),

            ExprKind::FormatStr(parts) => {
                let mut text = String::new();
                for part in parts {
                    match part {
                        FormatPart::Text(plain) => text += plain,
                        FormatPart::Value { value, format } => {
                            let value = self.eval(value, env)?;
                            text += &builtins::format(&value, format.as_deref(), span)?;
                        }
                    }
                }
                Ok(Value::str(text))
            }

            ExprKind::Name(name) => match env.get(name) {
                Some(value) => Ok(value),
                None => fatal(format!("`{name}` is not defined"), span),
            },

            ExprKind::Unit { value, unit } => match self.eval(value, env)?.number() {
                _ if !units::is_unit(unit) => fatal(crate::units::unknown(unit), span),
                Some(number) => Ok(Value::Quantity(number, Rc::from(unit.as_str()))),
                None => fatal("a unit can only follow a number", span),
            },

            ExprKind::List(items) => {
                let mut values = Vec::new();
                for item in items {
                    values.push(self.eval(item, env)?);
                }
                Ok(Value::list(values))
            }
            ExprKind::ListComp { element, variable, iterable, condition } => {
                let source = self.eval(iterable, env)?;
                let mut values = Vec::new();
                for item in self.items(&source, iterable.span)? {
                    let scope = Scope::child(env);
                    scope.declare(variable, item, false);
                    let keep = match condition {
                        Some(condition) => self.condition(condition, &scope)?,
                        None => true,
                    };
                    if keep {
                        values.push(self.eval(element, &scope)?);
                    }
                }
                Ok(Value::list(values))
            }

            ExprKind::Unary { op, operand } => {
                let value = self.eval(operand, env)?;
                match (op, value) {
                    (UnaryOp::Neg, Value::Int(n)) => match n.checked_neg() {
                        Some(n) => Ok(Value::Int(n)),
                        None => raise("OverflowError", "the result is too big for an int", span),
                    },
                    (UnaryOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
                    (UnaryOp::Neg, Value::Quantity(x, unit)) => Ok(Value::Quantity(-x, unit)),
                    (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (UnaryOp::Neg, other) => {
                        fatal(format!("`-` does not work on {}", other.type_name()), span)
                    }
                    (UnaryOp::Not, other) => {
                        fatal(format!("`not` needs a bool, but this is {}", other.type_name()), span)
                    }
                }
            }

            // `and` and `or` only work out their right side when they have to.
            ExprKind::Binary { op: op @ (BinaryOp::And | BinaryOp::Or), left, right } => {
                let left = self.logic_operand(*op, left, env)?;
                let decided = match op {
                    BinaryOp::And => !left, // false and ... is false
                    _ => left,              // true or ... is true
                };
                if decided {
                    return Ok(Value::Bool(left));
                }
                Ok(Value::Bool(self.logic_operand(*op, right, env)?))
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.eval(left, env)?;
                let right = self.eval(right, env)?;
                self.binary(*op, left, right, span)
            }
            ExprKind::Compare { first, rest } => {
                // `a < b < c`: compare neighbours, stop at the first false,
                // and work out each operand once.
                let mut left = self.eval(first, env)?;
                for (op, operand) in rest {
                    let right = self.eval(operand, env)?;
                    match self.binary(*op, left, right.clone(), span)? {
                        Value::Bool(true) => left = right,
                        _ => return Ok(Value::Bool(false)),
                    }
                }
                Ok(Value::Bool(true))
            }

            // A function written as a value remembers the scope it was written in.
            ExprKind::Function(decl) => {
                let closure = Closure { decl: decl.clone(), env: env.clone() };
                Ok(Value::Function(Rc::new(closure)))
            }

            ExprKind::Call { callee, args } => {
                let callee = self.eval(callee, env)?;
                let mut positional = Vec::new();
                let mut named = Vec::new();
                for arg in args {
                    let value = self.eval(&arg.value, env)?;
                    match &arg.name {
                        Some(name) => named.push((name.clone(), value)),
                        None if named.is_empty() => positional.push(value),
                        None => {
                            return fatal("plain arguments must come before named ones", arg.value.span)
                        }
                    }
                }
                self.call(callee, positional, named, span)
            }

            ExprKind::Field { object, name } => {
                let object = self.eval(object, env)?;
                self.field(object, name, span)
            }
            ExprKind::Index { object, index } => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                match &object {
                    Value::List(items) => {
                        let items = items.borrow();
                        let i = self.index(&index, items.len(), "list", span)?;
                        Ok(items[i].clone())
                    }
                    Value::Str(text) => {
                        let chars: Vec<char> = text.chars().collect();
                        let i = self.index(&index, chars.len(), "str", span)?;
                        Ok(Value::str(chars[i].to_string()))
                    }
                    other => fatal(format!("cannot take an item from {}", other.type_name()), span),
                }
            }
        }
    }

    fn logic_operand(&mut self, op: BinaryOp, operand: &Expr, env: &Env) -> Result<bool, Control> {
        match self.eval(operand, env)? {
            Value::Bool(b) => Ok(b),
            other => fatal(
                format!("`{}` needs bools, but this is {}", op.text(), other.type_name()),
                operand.span,
            ),
        }
    }

    /// Check an index and turn it into a position. Positions start at 0 and
    /// negative ones are refused; see docs/decisions.md.
    fn index(&self, index: &Value, len: usize, what: &str, span: Span) -> Result<usize, Control> {
        let Value::Int(i) = index else {
            return fatal(format!("an index must be an int, but this is {}", index.type_name()), span);
        };
        match usize::try_from(*i) {
            Ok(i) if i < len => Ok(i),
            _ => raise("IndexError", format!("index {i} is out of range for a {what} of length {len}"), span),
        }
    }

    /// `object.name`
    fn field(&mut self, object: Value, name: &str, span: Span) -> Result<Value, Control> {
        match &object {
            Value::Instance(instance) => {
                if let Some(value) = instance.fields.borrow().get(name) {
                    return Ok(value.clone());
                }
                if let Some(method) = instance.class.methods.get(name) {
                    return Ok(Value::BoundMethod(instance.clone(), method.clone()));
                }
                fatal(format!("`{}` has no field or method `{name}`", instance.class.name), span)
            }
            Value::Case(case) => {
                let fields = &case.def.cases[case.index].fields;
                match fields.iter().position(|field| field.name == name) {
                    Some(i) => Ok(case.fields[i].clone()),
                    None => fatal(format!("`{}` has no field `{name}`", case.name()), span),
                }
            }
            Value::Enum(def) => match def.cases.iter().position(|case| case.name == name) {
                Some(index) => Ok(case_value(def, index)),
                None => fatal(format!("enum `{}` has no case `{name}`", def.name), span),
            },
            Value::Module(module) => match module.members.get(name) {
                Some(member) => Ok(member.clone()),
                None => fatal(format!("module `{}` has no `{name}`", module.name), span),
            },
            // `(3 s).ms`: the same quantity in another unit.
            Value::Quantity(x, unit) if units::is_unit(name) => match units::convert(unit, name) {
                Some(scale) => Ok(Value::Quantity(scale.apply(*x), Rc::from(name))),
                None => fatal(
                    format!("a quantity in {unit} cannot be turned into {name}. They measure different things"),
                    span,
                ),
            },
            Value::Error(error) if name == "message" => Ok(Value::str(&error.message)),
            Value::Error(error) if name == "kind" => Ok(Value::str(&error.kind)),
            _ if builtins::has_method(&object, name) => {
                Ok(Value::Method(Box::new(object), Rc::from(name)))
            }
            other => fatal(format!("{} has no field or method `{name}`", other.type_name()), span),
        }
    }

    // ----------------------------------------------------------------- Calls

    pub fn call(
        &mut self,
        callee: Value,
        mut positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control> {
        match callee {
            Value::Function(closure) => self.call_function(&closure, positional, named, span),
            Value::BoundMethod(instance, closure) => {
                // `p.distance_to(q)` passes `p` as the first argument, `self`.
                positional.insert(0, Value::Instance(instance));
                self.call_function(&closure, positional, named, span)
            }
            Value::Builtin(builtin) => {
                no_named(builtin.name, &named, span)?;
                (builtin.call)(self, positional, span)
            }
            Value::Method(receiver, name) => {
                no_named(&name, &named, span)?;
                builtins::call_method(self, &receiver, &name, positional, span)
            }
            Value::Class(class) => {
                let names: Vec<&str> = class.fields.iter().map(|f| f.name.as_str()).collect();
                let slots = bind(&class.name, &names, positional, named, span)?;
                let mut fields = HashMap::new();
                for (field, slot) in class.fields.iter().zip(slots) {
                    let value = match (slot, &field.default) {
                        (Some(value), _) => value,
                        (None, Some(default)) => self.eval(default, &class.env)?,
                        (None, None) => {
                            return fatal(
                                format!("`{}` needs a value for `{}`", class.name, field.name),
                                span,
                            )
                        }
                    };
                    fields.insert(field.name.clone(), value);
                }
                Ok(Value::Instance(Rc::new(Instance { class, fields: RefCell::new(fields) })))
            }
            Value::CaseConstructor(def, index) => {
                let case = &def.cases[index];
                let names: Vec<&str> = case.fields.iter().map(|f| f.name.as_str()).collect();
                let slots = bind(&case.name, &names, positional, named, span)?;
                let mut fields = Vec::new();
                for (field, slot) in case.fields.iter().zip(slots) {
                    match slot {
                        Some(value) => fields.push(value),
                        None => {
                            return fatal(
                                format!("`{}` needs a value for `{}`", case.name, field.name),
                                span,
                            )
                        }
                    }
                }
                Ok(Value::Case(Rc::new(CaseValue { def: def.clone(), index, fields })))
            }
            Value::ErrorType(kind) => {
                no_named(&kind, &named, span)?;
                let message = match positional.as_slice() {
                    [] => String::new(),
                    [message] => message.to_string(),
                    _ => return fatal(format!("`{kind}` takes one message"), span),
                };
                Ok(Value::Error(Rc::new(ErrorValue { kind: kind.to_string(), message })))
            }
            other => fatal(format!("{} cannot be called", other.type_name()), span),
        }
    }

    fn call_function(
        &mut self,
        closure: &Closure,
        positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control> {
        let decl = &closure.decl;
        let name = if decl.name.is_empty() { "function" } else { &decl.name };

        if self.depth >= MAX_DEPTH {
            return fatal(
                format!("more than {MAX_DEPTH} calls are nested. Is `{name}` calling itself forever?"),
                span,
            );
        }

        // The body runs in a new scope inside the scope where the function
        // was *defined*, not the one it was called from.
        let scope = Scope::child(&closure.env);
        let names: Vec<&str> = decl.params.iter().map(|p| p.name.as_str()).collect();
        let slots = bind(name, &names, positional, named, span)?;
        for (param, slot) in decl.params.iter().zip(slots) {
            let value = match (slot, &param.default) {
                (Some(value), _) => value,
                // Defaults are worked out fresh on every call.
                (None, Some(default)) => self.eval(default, &scope)?,
                (None, None) => {
                    return fatal(format!("`{name}` needs a value for `{}`", param.name), span)
                }
            };
            self.declare(&scope, &param.name, value, false, span)?;
        }

        self.depth += 1;
        let result = self.run_statements(&decl.body, &scope);
        self.depth -= 1;

        match result {
            Ok(()) => Ok(Value::None), // fell off the end without a `return`
            Err(Control::Return(value, _)) => Ok(value),
            Err(Control::Break(span)) => fatal("`break` outside a loop", span),
            Err(Control::Continue(span)) => fatal("`continue` outside a loop", span),
            Err(other) => Err(other),
        }
    }

    // ------------------------------------------------------------- Operators

    fn binary(&mut self, op: BinaryOp, left: Value, right: Value, span: Span) -> Result<Value, Control> {
        use BinaryOp::*;
        use Value::{Bool, Float, Int, List, Quantity, Str};

        let mismatch = || {
            fatal(
                format!(
                    "`{}` does not work between {} and {}",
                    op.text(),
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )
        };
        let overflow = || raise("OverflowError", "the result is too big for an int", span);
        let by_zero = || raise("ZeroDivisionError", "division by zero", span);
        // Both sides as floats, when both are numbers.
        let floats = match (left.number(), right.number()) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        };

        match op {
            Add => match (&left, &right) {
                (Int(a), Int(b)) => a.checked_add(*b).map(Int).map_or_else(overflow, Ok),
                (Str(a), Str(b)) => Ok(Value::str(format!("{a}{b}"))),
                (List(a), List(b)) => {
                    let mut items = a.borrow().clone();
                    items.extend(b.borrow().iter().cloned());
                    Ok(Value::list(items))
                }
                // `1 s + 500 ms` is `1500 ms`: both sides in the finer unit.
                (Quantity(a, unit), Quantity(b, other)) => match units::common(unit, other) {
                    Some((unit, left, right)) => Ok(Quantity(left.apply(*a) + right.apply(*b), Rc::from(unit))),
                    None => mismatch(),
                },
                _ => floats.map_or_else(mismatch, |(a, b)| Ok(Float(a + b))),
            },
            Sub => match (&left, &right) {
                (Int(a), Int(b)) => a.checked_sub(*b).map(Int).map_or_else(overflow, Ok),
                (Quantity(a, unit), Quantity(b, other)) => match units::common(unit, other) {
                    Some((unit, left, right)) => Ok(Quantity(left.apply(*a) - right.apply(*b), Rc::from(unit))),
                    None => mismatch(),
                },
                _ => floats.map_or_else(mismatch, |(a, b)| Ok(Float(a - b))),
            },
            Mul => match (&left, &right) {
                (Int(a), Int(b)) => a.checked_mul(*b).map(Int).map_or_else(overflow, Ok),
                // Scaling a quantity keeps its unit: 2 * 500 ms is 1000 ms.
                (Quantity(a, unit), other) | (other, Quantity(a, unit)) => match other.number() {
                    Some(b) => Ok(Quantity(a * b, unit.clone())),
                    None => mismatch(),
                },
                _ => floats.map_or_else(mismatch, |(a, b)| Ok(Float(a * b))),
            },
            // `/` always gives a float, even for two ints. `//` is the one that rounds down.
            Div => match (&left, &right) {
                (Quantity(_, _), Quantity(b, _)) | (Quantity(_, _), Float(b)) if *b == 0.0 => by_zero(),
                (Quantity(_, _), Int(0)) => by_zero(),
                (Quantity(a, unit), Quantity(b, other)) => match units::common(unit, other) {
                    Some((_, left, right)) => Ok(Float(left.apply(*a) / right.apply(*b))),
                    None => mismatch(),
                },
                (Quantity(a, unit), other) => match other.number() {
                    Some(b) => Ok(Quantity(a / b, unit.clone())),
                    None => mismatch(),
                },
                _ => match floats {
                    Some((_, b)) if b == 0.0 => by_zero(),
                    Some((a, b)) => Ok(Float(a / b)),
                    None => mismatch(),
                },
            },
            FloorDiv => match (&left, &right) {
                (Int(_), Int(0)) => by_zero(),
                (Int(a), Int(b)) => match a.checked_div(*b) {
                    // Rust rounds toward zero; step down once more when the
                    // signs differ and the division was not exact.
                    Some(q) if a % b != 0 && (*a < 0) != (*b < 0) => Ok(Int(q - 1)),
                    Some(q) => Ok(Int(q)),
                    None => overflow(),
                },
                _ => match floats {
                    Some((_, b)) if b == 0.0 => by_zero(),
                    Some((a, b)) => Ok(Float((a / b).floor())),
                    None => mismatch(),
                },
            },
            // The result takes the sign of the right side, to pair with `//`.
            Mod => match (&left, &right) {
                (Int(_), Int(0)) => by_zero(),
                (Int(a), Int(b)) => {
                    let r = a.checked_rem(*b).unwrap_or(0);
                    Ok(Int(if r != 0 && (r < 0) != (*b < 0) { r + b } else { r }))
                }
                _ => match floats {
                    Some((_, b)) if b == 0.0 => by_zero(),
                    Some((a, b)) => Ok(Float(a - b * (a / b).floor())),
                    None => mismatch(),
                },
            },
            Pow => match (&left, &right) {
                // int ** int is always an int, so a negative power, which
                // would need a fraction, is refused. See docs/decisions.md.
                (Int(_), Int(b)) if *b < 0 => raise(
                    "ValueError",
                    "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`",
                    span,
                ),
                (Int(a), Int(b)) => u32::try_from(*b)
                    .ok()
                    .and_then(|b| a.checked_pow(b))
                    .map(Int)
                    .map_or_else(overflow, Ok),
                _ => floats.map_or_else(mismatch, |(a, b)| Ok(Float(a.powf(b)))),
            },

            Eq | Is => Ok(Bool(equal(&left, &right))),
            NotEq | IsNot => Ok(Bool(!equal(&left, &right))),

            Lt | LtEq | Gt | GtEq => {
                let order = match (&left, &right) {
                    (Int(a), Int(b)) => Some(a.cmp(b)),
                    (Str(a), Str(b)) => Some(a.cmp(b)),
                    (Quantity(a, unit), Quantity(b, other)) => match units::common(unit, other) {
                        Some((_, left, right)) => left.apply(*a).partial_cmp(&right.apply(*b)),
                        None => return mismatch(),
                    },
                    _ => match floats {
                        Some((a, b)) => a.partial_cmp(&b),
                        None => return mismatch(),
                    },
                };
                // `order` is None only for NaN, which compares false with everything.
                Ok(Bool(match (op, order) {
                    (_, None) => false,
                    (Lt, Some(order)) => order == Ordering::Less,
                    (LtEq, Some(order)) => order != Ordering::Greater,
                    (Gt, Some(order)) => order == Ordering::Greater,
                    (_, Some(order)) => order != Ordering::Less,
                }))
            }

            In | NotIn => {
                let found = match (&left, &right) {
                    (item, List(items)) => items.borrow().iter().any(|other| equal(item, other)),
                    (Str(part), Str(text)) => text.contains(&**part),
                    _ => return mismatch(),
                };
                Ok(Bool(found == (op == In)))
            }

            // Handled in `eval`, because they must not always work out both sides.
            And | Or => mismatch(),
        }
    }
}

/// A case with no values is a finished value; one with values needs calling.
fn case_value(def: &Rc<EnumDef>, index: usize) -> Value {
    if def.cases[index].fields.is_empty() {
        Value::Case(Rc::new(CaseValue { def: def.clone(), index, fields: Vec::new() }))
    } else {
        Value::CaseConstructor(def.clone(), index)
    }
}

fn no_named(what: &str, named: &[(String, Value)], span: Span) -> Result<(), Control> {
    match named.first() {
        Some((name, _)) => fatal(format!("`{what}` does not take a named argument `{name}`"), span),
        None => Ok(()),
    }
}

/// Line the arguments of a call up with the names they are for. The result
/// has one slot per name, `None` where no argument was given.
fn bind(
    what: &str,
    names: &[&str],
    positional: Vec<Value>,
    named: Vec<(String, Value)>,
    span: Span,
) -> Result<Vec<Option<Value>>, Control> {
    if positional.len() > names.len() {
        return fatal(
            format!("`{what}` takes {} argument(s), but {} were given", names.len(), positional.len()),
            span,
        );
    }
    let mut slots: Vec<Option<Value>> = positional.into_iter().map(Some).collect();
    slots.resize(names.len(), None);

    for (name, value) in named {
        match names.iter().position(|n| *n == name) {
            None => return fatal(format!("`{what}` has no argument named `{name}`"), span),
            Some(i) if slots[i].is_some() => {
                return fatal(format!("`{what}` was given `{name}` twice"), span)
            }
            Some(i) => slots[i] = Some(value),
        }
    }
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use crate::{lexer, parser};

    /// Run a program and return what it printed and how it ended.
    fn run(source: &str) -> (String, Result<(), String>) {
        let program = parser::parse(lexer::lex(source).expect("lexing failed")).expect("parsing failed");
        let (output, result) = super::run(&program, true);
        (output, result.map_err(|error| error.message))
    }

    /// What a working program prints. A program that runs correctly must
    /// also get through the type checker, so that is tested here too.
    fn output(source: &str) -> String {
        let program = parser::parse(lexer::lex(source).unwrap()).unwrap();
        let complaints = crate::checker::check(&program, false);
        assert!(complaints.is_empty(), "the checker rejected a working program: {complaints:?}\n{source}");

        let (output, result) = run(source);
        if let Err(message) = result {
            panic!("program failed: {message}\noutput so far:\n{output}");
        }
        output
    }

    /// The message a failing program stops with.
    fn error(source: &str) -> String {
        run(source).1.expect_err("the program should have failed")
    }

    // ----- Values and operators -----

    #[test]
    fn arithmetic() {
        assert_eq!(output("print(1 + 2 * 3, 10 - 4, 2 ** 10, -3 + 1)"), "7 6 1024 -2\n");
        assert_eq!(output("print(1.5 + 1, 2 * 0.5, 2.0 ** -1, 2.0 ** 3)"), "2.5 1.0 0.5 8.0\n");
        assert_eq!(
            error("print(2 ** -1)"),
            "uncaught ValueError: a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`"
        );
    }

    #[test]
    fn division_always_gives_a_float_and_floor_division_rounds_down() {
        assert_eq!(output("print(7 / 2, 6 / 2, 7 // 2, -7 // 2, 7.5 // 2)"), "3.5 3.0 3 -4 3.0\n");
        assert_eq!(output("print(7 % 3, -7 % 3, 7 % -3, 7.5 % 2)"), "1 2 -2 1.5\n");
    }

    #[test]
    fn comparisons_and_equality() {
        assert_eq!(output("print(1 < 2, 2 <= 2, 3 > 4, \"a\" < \"b\", 1 == 1.0, 1 != 2)"), "true true false true true true\n");
        assert_eq!(output("print([1, 2] == [1, 2], [1] == [2], none == none)"), "true false true\n");
        // The checker refuses to compare an int with a str, but a value of
        // unknown type can still meet one when the program runs.
        assert_eq!(output("function same(a, b) { return a == b }\nprint(same(1, \"1\"), same(1, 1.0))"), "false true\n");
    }

    #[test]
    fn chained_comparisons() {
        assert_eq!(output("print(1 < 2 < 3, 1 < 5 < 3, 3 > 2 > 1 == 1)"), "true false true\n");
        // The middle operand runs once, and the chain stops at the first false.
        let source = "function mid() -> int {\n print(\"mid\")\n return 5\n}\nfunction last() -> int {\n print(\"last\")\n return 9\n}\nprint(1 < mid() < last())\nprint(9 < mid() < last())";
        assert_eq!(output(source), "mid\nlast\ntrue\nmid\nfalse\n");
    }

    #[test]
    fn logic_short_circuits() {
        let source = "function loud(x: bool) -> bool {\n print(\"called\")\n return x\n}\nprint(false and loud(true))\nprint(true or loud(true))\nprint(true and loud(false))\nprint(not true)";
        assert_eq!(output(source), "false\ntrue\ncalled\nfalse\nfalse\n");
    }

    #[test]
    fn strings() {
        assert_eq!(output("print(\"ab\" + \"cd\", len(\"héllo\"), \"hello\"[1], \"ell\" in \"hello\")"), "abcd 5 e true\n");
        assert_eq!(output("print(\" Hi \".strip().upper(), \"a,b\".split(\",\"), \"x\".replace(\"x\", \"y\"))"), "HI [\"a\", \"b\"] y\n");
        assert_eq!(output("print(\"zeno\".starts_with(\"ze\"), \"zeno\".ends_with(\"x\"))"), "true false\n");
    }

    #[test]
    fn format_strings() {
        assert_eq!(output("let n = 3\nprint(f\"{n} + {n} = {n + n}\")"), "3 + 3 = 6\n");
        assert_eq!(output("print(f\"{3.14159:.2f}|{42:5}|{'ab':5}|{'ab':>5}|{'a:}':^6}|\")"), "3.14|   42|ab   |   ab| a:}  |\n");
        assert_eq!(output("print(f\"{255:x} {255:X} {5:08b} {7:03} {-7:04} {2:.1f}\")"), "ff FF 00000101 007 -007 2.0\n");
        assert_eq!(output("print(f\"{{literal}} {1}\")"), "{literal} 1\n");
    }

    #[test]
    fn lists() {
        assert_eq!(output("let xs = [1, 2]\nxs.append(3)\nxs[0] = 9\nprint(xs, len(xs), xs.pop(), xs + [7], 2 in xs, 5 not in xs)"), "[9, 2] 3 3 [9, 2, 7] true true\n");
        assert_eq!(output("let a = [1]\nlet b = a\nb.append(2)\nprint(a)"), "[1, 2]\n");
        assert_eq!(output("print([n * n for n in range(6) if n % 2 == 0], range(2, 5), range(5, 0, -2))"), "[0, 4, 16] [2, 3, 4] [5, 3, 1]\n");
    }

    #[test]
    fn quantities() {
        assert_eq!(output("print(500 ms, 500 ms + 250 ms, 2 * 500 ms, 1.5 V / 3, 1 s / 250 s, 3 V > 2 V)"), "500 ms 750 ms 1000 ms 0.5 V 0.004 true\n");
        assert_eq!(error("500 ms + 3 V"), "`+` does not work between a quantity in ms and a quantity in V");
        assert_eq!(error("500 ms + 3"), "`+` does not work between a quantity in ms and an int");
        assert_eq!(
            error("print(500 mss)"),
            "`mss` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC"
        );
    }

    #[test]
    fn built_in_functions() {
        assert_eq!(output("print(int(\"42\"), int(3.9), float(\"1.5\"), str(12) + \"!\", abs(-3), min(3, 1, 2), max([1, 5.5]), sum([1, 2, 3]), sum([1, 0.5]))"), "42 3 1.5 12! 3 1 5.5 6 1.5\n");
    }

    #[test]
    fn modules() {
        assert_eq!(output("import math\nfrom math import sqrt, floor\nprint(sqrt(16), floor(2.7), math.ceil(2.1), math.pi > 3.14)"), "4.0 2 3 true\n");
        assert_eq!(error("import board"), "there is no module `board`. So far Zeno has `math` and `time`");
        assert_eq!(error("from math import nope"), "module `math` has no `nope`");
        assert_eq!(error("from time import sleep\nsleep(500)"), "`sleep` needs a time with a unit, such as `500 ms`, but this is an int");
    }

    // ----- Variables and scopes -----

    #[test]
    fn let_cannot_change_and_var_can() {
        assert_eq!(output("var x = 1\nx = 2\nx += 3\nprint(x)"), "5\n");
        assert_eq!(error("let x = 1\nx = 2"), "`x` cannot be changed. Declare it with `var` to allow that");
        assert_eq!(error("const N = 1\nN += 1"), "`N` cannot be changed. Declare it with `var` to allow that");
        assert_eq!(error("x = 2"), "`x` is not declared. Create it with `let` or `var` first");
        assert_eq!(error("print(y)"), "`y` is not defined");
    }

    #[test]
    fn blocks_have_their_own_scope() {
        assert_eq!(output("let x = 1\nif true {\n let x = 2\n print(x)\n}\nprint(x)"), "2\n1\n");
        assert_eq!(error("if true {\n let inner = 1\n}\nprint(inner)"), "`inner` is not defined");
        assert_eq!(error("let x = 1\nlet x = 2"), "`x` is already declared in this block");
        assert_eq!(output("var total = 0\nfor i in range(3) {\n let double = i * 2\n total += double\n}\nprint(total)"), "6\n");
    }

    // ----- Control flow -----

    #[test]
    fn if_else() {
        let source = "function size(n: int) -> str {\n if n < 10 { return \"small\" } else if n < 100 { return \"medium\" } else { return \"large\" }\n}\nprint(size(5), size(50), size(500))";
        assert_eq!(output(source), "small medium large\n");
        assert_eq!(error("if 1 { }"), "a condition must be true or false, but this is an int");
        assert_eq!(error("print(1 and true)"), "`and` needs bools, but this is an int");
    }

    #[test]
    fn loops_with_break_and_continue() {
        let source = "var i = 0\nwhile true {\n i += 1\n if i == 2 { continue }\n if i > 4 { break }\n print(i)\n}";
        assert_eq!(output(source), "1\n3\n4\n");
        assert_eq!(output("for c in \"ab\" {\n print(c)\n}\nfor x in [1, 2, 3] {\n if x == 2 { break }\n print(x)\n}"), "a\nb\n1\n");
        assert_eq!(error("break"), "`break` outside a loop");
        assert_eq!(error("function f() { continue }\nf()"), "`continue` outside a loop");
        assert_eq!(error("for x in 5 { }"), "cannot loop over an int");
    }

    // ----- Functions -----

    #[test]
    fn functions_and_arguments() {
        let source = "function greet(who: str, excited: bool = false) -> str {\n if excited { return f\"Hello, {who}!\" }\n return f\"Hello, {who}.\"\n}\nprint(greet(\"a\"), greet(\"b\", true), greet(who=\"c\", excited=true), greet(\"d\", excited=true))";
        assert_eq!(output(source), "Hello, a. Hello, b! Hello, c! Hello, d!\n");
        assert_eq!(output("function nothing() { }\nprint(nothing())"), "none\n");
    }

    #[test]
    fn argument_mistakes() {
        let f = "function f(a: int, b: int = 2) -> int { return a + b }\n";
        assert_eq!(error(&format!("{f}f()")), "`f` needs a value for `a`");
        assert_eq!(error(&format!("{f}f(1, 2, 3)")), "`f` takes 2 argument(s), but 3 were given");
        assert_eq!(error(&format!("{f}f(1, c=3)")), "`f` has no argument named `c`");
        assert_eq!(error(&format!("{f}f(1, a=3)")), "`f` was given `a` twice");
        assert_eq!(error(&format!("{f}f(a=1, 2)")), "plain arguments must come before named ones");
        assert_eq!(error("let x = 5\nx()"), "an int cannot be called");
        assert_eq!(error("print(len(1, 2))"), "`len` takes 1 argument(s), but 2 were given");
    }

    #[test]
    fn functions_can_be_used_before_they_are_written() {
        assert_eq!(output("print(double(4))\nfunction double(n: int) -> int { return n * 2 }"), "8\n");
        let source = "function even(n: int) -> bool {\n if n == 0 { return true }\n return odd(n - 1)\n}\nfunction odd(n: int) -> bool {\n if n == 0 { return false }\n return even(n - 1)\n}\nprint(even(10), odd(7))";
        assert_eq!(output(source), "true true\n");
    }

    #[test]
    fn recursion_and_its_limit() {
        assert_eq!(output("function fact(n: int) -> int {\n if n <= 1 { return 1 }\n return n * fact(n - 1)\n}\nprint(fact(20))"), "2432902008176640000\n");
        assert_eq!(output("function down(n: int) -> int {\n if n == 0 { return 0 }\n return down(n - 1)\n}\nprint(down(900))"), "0\n");
        assert_eq!(error("function forever() { forever() }\nforever()"), "more than 1000 calls are nested. Is `forever` calling itself forever?");
    }

    #[test]
    fn closures_remember_where_they_were_defined() {
        let source = "function counter() {\n var count = 0\n function next() -> int {\n  count += 1\n  return count\n }\n return next\n}\nlet a = counter()\nlet b = counter()\nprint(a(), a(), b())";
        assert_eq!(output(source), "1 2 1\n");
    }

    #[test]
    fn defaults_are_fresh_on_every_call() {
        let source = "function add(item: int, to: list[int] = []) -> list[int] {\n to.append(item)\n return to\n}\nprint(add(1), add(2))";
        assert_eq!(output(source), "[1] [2]\n");
    }

    #[test]
    fn parameters_cannot_be_reassigned() {
        assert_eq!(error("function f(n: int) { n = 2 }\nf(1)"), "`n` cannot be changed. Declare it with `var` to allow that");
    }

    #[test]
    fn functions_are_values() {
        let source = "function twice(f, x) { return f(f(x)) }\nfunction add_one(n) { return n + 1 }\nlet double = function(n) { return n * 2 }\nprint(twice(add_one, 5), twice(double, 5), function(a, b) { return a + b }(1, 2))";
        assert_eq!(output(source), "7 20 3\n");
        assert_eq!(output("function f() { }\nprint(f, function() { }, len)"), "<function f> <function> <function len>\n");
    }

    #[test]
    fn map_filter_reduce() {
        let source = "let xs = [1, 2, 3, 4, 5, 6]\nfunction even(n) { return n % 2 == 0 }\nprint(xs.filter(even).map(function(n) { return n * n }).reduce(function(total, n) { return total + n }, 0))\nprint(xs.map(str), [].map(even), [].reduce(function(a, b) { return a }, 7))";
        assert_eq!(output(source), "56\n[\"1\", \"2\", \"3\", \"4\", \"5\", \"6\"] [] 7\n");
        assert_eq!(
            error("[1].filter(function(n) { return n })"),
            "the function given to `filter` must return true or false, but it returned an int"
        );
        assert_eq!(error("[1].map(5)"), "an int cannot be called");
        assert_eq!(error("[1].map(function() { return 1 })"), "`function` takes 0 argument(s), but 1 were given");
    }

    #[test]
    fn a_function_written_inside_a_call_can_span_lines() {
        let source = "let total = [1, 2, 3].reduce(function(sum, n) {\n    let bigger = n * 10\n    return sum + bigger\n}, 0)\nprint(total)";
        assert_eq!(output(source), "60\n");
    }

    #[test]
    fn types_are_optional() {
        let typed = "class Point {\n x: float\n y: float\n}\nfunction length(p: Point) -> float {\n let squared: float = p.x ** 2 + p.y ** 2\n return squared ** 0.5\n}\nprint(length(Point(3.0, 4.0)))";
        let untyped = "class Point {\n x\n y\n}\nfunction length(p) {\n let squared = p.x ** 2 + p.y ** 2\n return squared ** 0.5\n}\nprint(length(Point(3.0, 4.0)))";
        assert_eq!(output(typed), "5.0\n");
        assert_eq!(output(untyped), "5.0\n");
    }

    // ----- Classes -----

    #[test]
    fn classes() {
        let source = "class Point {\n x: float\n y: float = 0.0\n function shifted(self, dx: float) -> Point {\n  return Point(self.x + dx, self.y)\n }\n}\nlet p = Point(1.0)\nlet q = Point(y=5.0, x=2.0)\np.x = 3.0\nprint(p, q, p.shifted(1.0).x, p == Point(3.0, 0.0), p == q)";
        assert_eq!(output(source), "Point(x=3.0, y=0.0) Point(x=2.0, y=5.0) 4.0 true false\n");
    }

    #[test]
    fn class_mistakes() {
        let point = "class Point {\n x: int\n}\n";
        assert_eq!(error(&format!("{point}Point()")), "`Point` needs a value for `x`");
        assert_eq!(error(&format!("{point}Point(1).z")), "`Point` has no field or method `z`");
        assert_eq!(error(&format!("{point}let p = Point(1)\np.z = 2")), "`Point` has no field `z`");
        assert_eq!(error("let s = \"x\"\ns.nope()"), "a str has no field or method `nope`");
    }

    #[test]
    fn objects_are_shared_not_copied() {
        assert_eq!(output("class Box {\n n: int\n}\nlet a = Box(1)\nlet b = a\nb.n = 2\nprint(a.n)"), "2\n");
    }

    // ----- Enums and match -----

    #[test]
    fn enums_and_match() {
        let source = "enum Shape {\n Dot\n Circle(radius: float)\n Rect(w: float, h: float)\n}\nfunction area(s: Shape) -> float {\n match s {\n  case Circle(r) { return 3.0 * r * r }\n  case Rect(w, h) { return w * h }\n  case Dot { return 0.0 }\n }\n}\nprint(area(Circle(2.0)), area(Rect(w=2.0, h=3.0)), area(Dot), area(Shape.Dot))\nprint(Circle(1.5), Dot, Circle(1.0).radius, Circle(1.0) == Circle(1.0), Dot == Circle(1.0))";
        assert_eq!(output(source), "12.0 6.0 0.0 0.0\nCircle(1.5) Dot 1.0 true false\n");
    }

    #[test]
    fn match_on_plain_values() {
        let source = "function name(n: int) -> str {\n match n {\n  case 0 { return \"zero\" }\n  case 1 { return \"one\" }\n  case other { return f\"many ({other})\" }\n }\n}\nprint(name(0), name(1), name(7))\nmatch \"x\" {\n case \"y\" { print(\"no\") }\n case _ { print(\"fallback\") }\n}";
        assert_eq!(output(source), "zero one many (7)\nfallback\n");
    }

    #[test]
    fn match_mistakes() {
        assert_eq!(error("match 3 {\n case 1 { }\n}"), "no `case` matches 3");
        assert_eq!(error("enum E {\n A(x: int)\n}\nmatch A(1) {\n case A(x, y) { }\n}"), "`A` carries 1 value(s), but this pattern has 2");
        assert_eq!(error("match 1 {\n case Nope(x) { }\n}"), "`Nope` is not an enum case that carries values");
    }

    // ----- Errors -----

    #[test]
    fn raise_and_except() {
        let source = "function parse_port(text: str) -> int raises ValueError {\n let port = int(text)\n if not 1 <= port <= 65535 { raise ValueError(\"port out of range\") }\n return port\n}\nfor text in [\"8080\", \"0\", \"abc\"] {\n try {\n  print(parse_port(text))\n } except ValueError as err {\n  print(f\"error: {err}\")\n }\n}";
        assert_eq!(output(source), "8080\nerror: port out of range\nerror: cannot turn \"abc\" into an int\n");
    }

    #[test]
    fn except_picks_the_matching_handler() {
        let source = "function risky(n: int) {\n if n == 0 { raise ValueError(\"v\") }\n if n == 1 { print(1 // 0) }\n if n == 2 { print([1][5]) }\n}\nfor n in range(3) {\n try {\n  risky(n)\n } except ValueError {\n  print(\"value\")\n } except ZeroDivisionError as e {\n  print(\"zero:\", e.message)\n } except Error as e {\n  print(e.kind)\n }\n}";
        assert_eq!(output(source), "value\nzero: division by zero\nIndexError\n");
    }

    #[test]
    fn a_class_can_be_raised() {
        let source = "class Timeout {\n after: int\n}\ntry {\n raise Timeout(30)\n} except Timeout as t {\n print(t.after)\n}";
        assert_eq!(output(source), "30\n");
    }

    #[test]
    fn uncaught_errors_stop_the_program() {
        assert_eq!(error("raise ValueError(\"bad\")"), "uncaught ValueError: bad");
        assert_eq!(error("try {\n raise ValueError(\"bad\")\n} except IndexError {\n}"), "uncaught ValueError: bad");
        assert_eq!(error("print(1 / 0)"), "uncaught ZeroDivisionError: division by zero");
        assert_eq!(error("print([1, 2][2])"), "uncaught IndexError: index 2 is out of range for a list of length 2");
        assert_eq!(error("print([1, 2][-1])"), "uncaught IndexError: index -1 is out of range for a list of length 2");
        assert_eq!(error("print(9223372036854775807 + 1)"), "uncaught OverflowError: the result is too big for an int");
        assert_eq!(error("raise 5"), "only an error can be raised, and this is an int");
    }

    #[test]
    fn bugs_cannot_be_caught() {
        assert_eq!(error("try {\n print(\"abc\" - 1)\n} except {\n print(\"caught\")\n}"), "`-` does not work between a str and an int");
        assert_eq!(error("try {\n print(missing)\n} except Error {\n}"), "`missing` is not defined");
    }

    #[test]
    fn output_before_an_error_is_kept() {
        let (output, result) = run("print(\"one\")\nprint(1 / 0)\nprint(\"two\")");
        assert_eq!(output, "one\n");
        assert!(result.is_err());
    }

    #[test]
    fn return_outside_a_function() {
        assert_eq!(error("return 1"), "`return` outside a function");
    }

    // ----- The examples -----

    #[test]
    fn hello_runs() {
        assert_eq!(output(include_str!("../examples/hello.zeno")), "Hello, Zeno!\n");
    }

    #[test]
    fn the_styles_example_gives_the_same_answer_four_ways() {
        assert_eq!(
            output(include_str!("../examples/styles.zeno")),
            "typed, with a class:   21.5\n\
             untyped, with a class: 21.5\n\
             functional:            21.5\n\
             comprehension:         21.5\n"
        );
    }

    #[test]
    fn units_of_one_kind_work_together() {
        assert_eq!(
            output("let t = 3 s\nlet d = 250 ms\nprint(t + d, t - d, t.ms, d.s, (90 min).h, (1 kWh).J)"),
            "3250 ms 2750 ms 3000 ms 0.25 s 1.5 h 3600000 J\n"
        );
        assert_eq!(output("print(1 s == 1000 ms, 3 s > 250 ms, 999 ms < 1 s, 1 h / 1 min)"), "true true true 60.0\n");
    }

    #[test]
    fn the_tour_runs() {
        assert_eq!(
            output(include_str!("../examples/tour.zeno")),
            "listening on 8080\n\
             area = 3.14\n\
             area = 6.00\n\
             Hello, Zeno! 4 [0, 4, 16, 36, 64] 5.0\n18 waiting 500 ms\n"
        );
    }
}
