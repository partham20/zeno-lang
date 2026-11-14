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
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::builtins;
use crate::error::Error;
use crate::ops;
use crate::token::Span;
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
                    .map_err(ops::escaped);
                (interpreter.output.unwrap_or_default(), result)
            })
            .expect("could not start the interpreter thread")
            .join()
            .expect("the interpreter crashed")
    })
}

// --------------------------------------------------------------------- Scopes

/// A scope is one set of variables. Scopes nest: each `{ }` block gets a new
/// one whose `parent` is the scope around it. Looking up a name walks outward
/// until it is found.
pub struct Scope {
    variables: RefCell<HashMap<String, Rc<Variable>>>,
    parent: Option<Env>,
    /// Set for a scope that is a *view* of another (see `seen`). A view
    /// has no variables of its own.
    view: Option<View>,
}

/// What a function can see of the block it is written in.
struct View {
    /// The block's scope.
    of: Env,
    /// How many of that scope's variables can be seen: the ones declared
    /// first.
    count: usize,
    /// The names of the variables among those that the block declares
    /// with `let`, `var`, `const` or `import`. When the function is called
    /// from above the line that declares one of them, the variable does
    /// not exist yet. The name still means that variable, and not one of
    /// the same name further out.
    lets: Rc<[String]>,
}

/// Scopes are shared (`Rc`) because a function keeps hold of the scope it
/// was defined in for as long as the function exists.
pub type Env = Rc<Scope>;

/// One variable. It is shared (`Rc`) because a function written inside
/// another keeps hold of the variables it uses: both must see the same
/// variable, so that a change made by one is seen by the other.
struct Variable {
    value: RefCell<Value>,
    /// True for `var`, false for `let`, `const`, parameters and loop variables.
    mutable: bool,
    /// How many variables its scope had before this one: 0 for the first
    /// to be declared there, 1 for the second.
    number: usize,
}

enum AssignError {
    NotDeclared,
    Immutable,
}

impl Scope {
    pub fn root() -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: None, view: None })
    }

    pub fn child(parent: &Env) -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: Some(parent.clone()), view: None })
    }

    /// Create a variable in this scope. Returns false if the name is taken here.
    pub fn declare(&self, name: &str, value: Value, mutable: bool) -> bool {
        let mut variables = self.variables.borrow_mut();
        if variables.contains_key(name) {
            return false;
        }
        let number = variables.len();
        variables.insert(name.to_string(), Rc::new(Variable { value: RefCell::new(value), mutable, number }));
        true
    }

    /// How many variables have been declared in this scope so far.
    fn count(&self) -> usize {
        self.variables.borrow().len()
    }

    /// The scope a function keeps: what can be seen from where it is
    /// written, and nothing that is declared later.
    ///
    /// Keeping `env` itself would be wrong. A variable declared further
    /// down the block would then be visible inside the function, and could
    /// hide an outer variable of the same name that the function was
    /// written to use.
    ///
    /// So the function gets a **view** of `env`: a scope with no variables
    /// of its own, which looks into `env` and sees only the first `count`
    /// variables declared there. They are the same variables, not copies,
    /// so changes still pass both ways. The scopes around `env` are treated
    /// the same way, each seen as it is at this moment.
    fn seen(env: &Env, count: usize, lets: &[String]) -> Env {
        // The built-ins never gain a variable, and a view never changes.
        let Some(parent) = &env.parent else {
            return env.clone();
        };
        if env.view.is_some() {
            return env.clone();
        }
        let parent = Scope::seen(parent, parent.count(), &[]);
        Rc::new(Scope {
            variables: RefCell::new(HashMap::new()),
            parent: Some(parent),
            view: Some(View { of: env.clone(), count, lets: Rc::from(lets) }),
        })
    }

    /// Find a variable by name, here or in a scope around this one.
    fn find(&self, name: &str) -> Option<Rc<Variable>> {
        let here = match &self.view {
            Some(view) => {
                let seen =
                    view.of.variables.borrow().get(name).filter(|variable| variable.number < view.count).cloned();
                if seen.is_none() && view.lets.iter().any(|declared| declared == name) {
                    // The block declares it, on a line that has not run yet.
                    return None;
                }
                seen
            }
            None => self.variables.borrow().get(name).cloned(),
        };
        here.or_else(|| self.parent.as_ref()?.find(name))
    }

    fn get(&self, name: &str) -> Option<Value> {
        Some(self.find(name)?.value.borrow().clone())
    }

    fn assign(&self, name: &str, value: Value) -> Result<(), AssignError> {
        let variable = self.find(name).ok_or(AssignError::NotDeclared)?;
        if !variable.mutable {
            return Err(AssignError::Immutable);
        }
        *variable.value.borrow_mut() = value;
        Ok(())
    }
}

// ---------------------------------------------------------------- Interpreter

pub struct Interpreter {
    /// `Some` collects printed text; `None` prints straight to the screen.
    output: Option<String>,
    /// How many function calls are currently in progress.
    depth: usize,
}

/// What the built-ins ask of the interpreter (see `Machine` in value.rs).
impl Machine for Interpreter {
    fn write_line(&mut self, text: &str) {
        match &mut self.output {
            Some(buffer) => {
                buffer.push_str(text);
                buffer.push('\n');
            }
            None => println!("{text}"),
        }
    }

    fn call(
        &mut self,
        callee: Value,
        positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control> {
        Interpreter::call(self, callee, positional, named, span)
    }
}

/// The scope a function or class made by the interpreter was defined in.
fn scope_of(home: &Home) -> Env {
    match home {
        Home::Scope(scope) => scope.clone(),
        Home::Code(..) => unreachable!("the interpreter met a function that the VM made"),
    }
}

impl Interpreter {

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
    ///
    /// Each sees the block as it will be when the program reaches the line
    /// it is written on: everything declared here, and the variables
    /// declared above that line. Those variables do not exist yet, so what
    /// is worked out is how many there will be (`above`) and what they are
    /// called (`lets`).
    fn hoist(&mut self, statements: &[Stmt], env: &Env) -> Result<(), Control> {
        let hoisted: usize = statements
            .iter()
            .map(|statement| match &statement.kind {
                StmtKind::Function(_) | StmtKind::Class(_) => 1,
                StmtKind::Enum(decl) => 1 + decl.variants.len(),
                _ => 0,
            })
            .sum();
        let mut above = env.count() + hoisted;
        let mut lets: Vec<String> = Vec::new();

        for statement in statements {
            let span = statement.span;
            match &statement.kind {
                StmtKind::Function(decl) => {
                    let home = Scope::seen(env, above, &lets);
                    let closure = Closure { decl: decl.clone(), home: Home::Scope(home) };
                    self.declare(env, &decl.name, Value::Function(Rc::new(closure)), false, span)?;
                }
                StmtKind::Class(class) => {
                    let home = Scope::seen(env, above, &lets);
                    let methods = class
                        .methods
                        .iter()
                        .map(|decl| {
                            let closure = Closure { decl: decl.clone(), home: Home::Scope(home.clone()) };
                            (decl.name.clone(), Rc::new(closure))
                        })
                        .collect();
                    let def = ClassDef {
                        name: class.name.clone(),
                        fields: class.fields.clone(),
                        methods,
                        home: Home::Scope(home),
                    };
                    self.declare(env, &class.name, Value::Class(Rc::new(def)), false, span)?;
                }
                StmtKind::Enum(decl) => {
                    let def = Rc::new(EnumDef { name: decl.name.clone(), cases: decl.variants.clone() });
                    self.declare(env, &decl.name, Value::Enum(def.clone()), false, span)?;
                    // Each case is also a name of its own, so you can write
                    // `Circle(1.0)` as well as `Shape.Circle(1.0)`.
                    for (index, case) in decl.variants.iter().enumerate() {
                        self.declare(env, &case.name, ops::case_value(&def, index), false, span)?;
                    }
                }
                // Each of these adds to what the functions below it can see.
                StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
                    above += 1;
                    lets.push(name.clone());
                }
                StmtKind::Import { module, names } => {
                    // `import a.b` declares one name, the module's.
                    above += names.len().max(1);
                    lets.extend(names.iter().cloned());
                    if names.is_empty() {
                        lets.extend(module.last().cloned());
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
                    new = ops::binary(*op, old, new, span)?;
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
                for item in ops::items(&iterable, iterable_span)? {
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
                let kind = ops::error_kind(&error);
                for handler in handlers {
                    if ops::catches(handler.error.as_deref(), &kind) {
                        let scope = Scope::child(env);
                        if let Some(binding) = &handler.binding {
                            scope.declare(binding, ops::caught(handler.error.as_deref(), &error), false);
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
                Err(ops::raised(error, span))
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
        let value = self.eval(condition, env)?;
        ops::condition(value, condition.span)
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
            ExprKind::Field { object, name } => {
                let object = self.eval(object, env)?;
                ops::set_field(object, name, value, span)
            }
            ExprKind::Index { object, index } => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                ops::set_item(object, &index, value, span)
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

            ExprKind::Unit { value, unit } => {
                let value = self.eval(value, env)?;
                ops::quantity(value, unit, span)
            }

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
                for item in ops::items(&source, iterable.span)? {
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
                ops::unary(*op, value, span)
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
                ops::binary(*op, left, right, span)
            }
            ExprKind::Compare { first, rest } => {
                // `a < b < c`: compare neighbours, stop at the first false,
                // and work out each operand once.
                let mut left = self.eval(first, env)?;
                for (op, operand) in rest {
                    let right = self.eval(operand, env)?;
                    match ops::binary(*op, left, right.clone(), span)? {
                        Value::Bool(true) => left = right,
                        _ => return Ok(Value::Bool(false)),
                    }
                }
                Ok(Value::Bool(true))
            }

            // A function written as a value remembers the scope it was written in.
            ExprKind::Function(decl) => {
                let closure =
                    Closure { decl: decl.clone(), home: Home::Scope(Scope::seen(env, env.count(), &[])) };
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
                ops::field(object, name, span)
            }
            ExprKind::Index { object, index } => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                ops::item(&object, &index, span)
            }
        }
    }

    fn logic_operand(&mut self, op: BinaryOp, operand: &Expr, env: &Env) -> Result<bool, Control> {
        let value = self.eval(operand, env)?;
        ops::logic_operand(op, value, operand.span)
    }


    // ----------------------------------------------------------------- Calls

    fn call(
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
                ops::no_named(builtin.name, &named, span)?;
                (builtin.call)(self, positional, span)
            }
            Value::Method(receiver, name) => {
                ops::no_named(&name, &named, span)?;
                builtins::call_method(self, &receiver, &name, positional, span)
            }
            Value::Class(class) => {
                let names: Vec<&str> = class.fields.iter().map(|f| f.name.as_str()).collect();
                let slots = ops::bind(&class.name, &names, positional, named, span)?;
                let scope = scope_of(&class.home);
                let mut fields = HashMap::new();
                for (field, slot) in class.fields.iter().zip(slots) {
                    let value = match (slot, &field.default) {
                        (Some(value), _) => value,
                        (None, Some(default)) => self.eval(default, &scope)?,
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
            Value::CaseConstructor(def, index) => ops::new_case(&def, index, positional, named, span),
            Value::ErrorType(kind) => ops::new_error(&kind, positional, named, span),
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
        let scope = Scope::child(&scope_of(&closure.home));
        let names: Vec<&str> = decl.params.iter().map(|p| p.name.as_str()).collect();
        let slots = ops::bind(name, &names, positional, named, span)?;
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
    fn a_raised_object_is_an_error_to_those_who_ask_for_any_error() {
        // `except Timeout as t` gets the object. `except Error as e` gets
        // an error named after the class, and raising that again passes
        // the object on.
        let source = "class Timeout {\n after: int\n}\n\
                      try {\n try {\n  raise Timeout(30)\n } except Error as e {\n  print(e.kind, e.message, e)\n  raise e\n }\n\
                      } except Timeout as t {\n print(t.after)\n}";
        assert_eq!(output(source), "Timeout Timeout(after=30) Timeout(after=30)\n30\n");
    }

    #[test]
    fn text_methods_know_only_plain_letters_and_blanks() {
        // The compiled program has to do the same on a microcontroller,
        // with no table of every alphabet to look letters up in.
        assert_eq!(output("print(\"Stra\u{df}e \u{e9}\".upper(), \"\u{c9}A\".lower())"), "STRA\u{df}E \u{e9} \u{c9}a\n");
        assert_eq!(output("print(\"\\t a b \\n\".strip(), \" a  b\\tc \".split(), int(\" 1_000\\n\"))"), "a b [\"a\", \"b\", \"c\"] 1000\n");
    }

    #[test]
    fn a_function_value_keeps_the_variables_it_was_written_with() {
        // The `y` declared after `f` must not get into `f`, which was
        // written when `y` meant the one at the top.
        let source = "let y = 1\nfunction g() -> int {\n let f = function() { return y }\n let y = 2\n return f() + y\n}\nprint(g())";
        assert_eq!(output(source), "3\n");
        // But it does see changes to the variables it kept.
        let source = "function g() -> int {\n var n = 1\n let f = function() { return n }\n n = 5\n return f()\n}\nprint(g())";
        assert_eq!(output(source), "5\n");
    }

    #[test]
    fn a_named_function_sees_what_is_declared_above_it_and_no_more() {
        // The same rule for a function with a name. It is made at the top
        // of its block, so that it can be called from above, but the `x`
        // declared below it is still not the one it was written to use.
        let source = "let x = 1\nfunction outer() -> int {\n function inner() -> int { return x }\n let x = 5\n return inner() + x\n}\nprint(outer())";
        assert_eq!(output(source), "6\n");
        // That holds at the top level of a file too, where the name
        // hidden is a built-in.
        let source = "function size(items: list[int]) -> int { return len(items) }\nlet len = 5\nprint(size([1, 2]), len)";
        assert_eq!(output(source), "2 5\n");
        // What is declared above it is seen, wherever it is called from...
        assert_eq!(output("let a = 1\nprint(f())\nfunction f() -> int { return a }"), "1\n");
        // ...once the line that declares it has run.
        assert_eq!(error("print(f())\nlet a = 1\nfunction f() { return a }"), "`a` is not defined");
        // A name means one variable, whenever the function is called. Here
        // `inner` uses the `x` of its own block, which has no value at
        // the first call. It does not fall back on the `x` outside.
        let source = "let x = 1\nfunction outer() {\n print(inner())\n let x = 5\n function inner() { return x }\n}\nouter()";
        assert_eq!(error(source), "`x` is not defined");
        // A method follows the same rule.
        let source = "let k = 2\nfunction make() -> int {\n class Scale {\n  by: int = k\n  function apply(self, n: int) -> int { return n * self.by * k }\n }\n let k = 10\n return Scale().apply(k)\n}\nprint(make())";
        assert_eq!(output(source), "40\n");
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
