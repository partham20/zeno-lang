//! The type checker: finds mistakes before the program runs.
//!
//! It walks the syntax tree the same way the interpreter does, but instead of
//! working out *values* it works out *types*. Where the interpreter's `eval`
//! turns `1 + 2` into `3`, the checker's `expr` turns it into `int`.
//!
//! Zeno's types are optional, so the checker is "gradual":
//!
//! 1. A type that is written down is checked.
//! 2. A type that is left out is worked out from the value, when possible.
//! 3. Otherwise the value is `Dynamic`: the checker lets it through, and any
//!    mistake is found when the program runs.
//!
//! In strict mode step 3 is an error. Everything must be known in advance.
//!
//! Unlike the parser, the checker does not stop at the first error. After
//! reporting one it treats the expression as `Invalid`, which is accepted
//! everywhere, so one mistake does not set off a string of others.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::ast::{self, *};
use crate::error::Error;
use crate::token::Span;

/// Check a whole program. Returns every error found, in source order.
pub fn check(program: &Program, strict: bool) -> Vec<Error> {
    let mut checker = Checker {
        strict,
        errors: Vec::new(),
        frames: vec![Frame { declared_return: None, returns: Vec::new(), loops: 0 }],
        raised: vec![Vec::new()],
    };
    let env = Scope::child(&globals());
    checker.statements(&program.statements, &env);

    let mut errors = checker.errors;
    errors.sort_by_key(|error| (error.span.line, error.span.col));
    errors.dedup();
    errors
}

// ---------------------------------------------------------------------- Types

/// The type of an expression, as far as the checker can tell.
#[derive(Clone)]
enum Ty {
    /// Not known until the program runs. Accepted everywhere.
    Dynamic,
    /// The type of something that already has an error reported against it.
    /// Accepted everywhere, so the same mistake is not reported twice.
    Invalid,

    Int,
    Float,
    Str,
    Bool,
    None,
    /// A number with a unit, such as `500 ms`.
    Quantity(Rc<str>),
    List(Box<Ty>),
    /// `T or none`
    Optional(Box<Ty>),
    /// An object of a class.
    Instance(Rc<ClassInfo>),
    /// A value of an enum.
    EnumValue(Rc<EnumInfo>),
    /// A built-in error value, such as a `ValueError`.
    Error(Rc<str>),

    // Things that can be called.
    Function(Rc<FunctionInfo>),
    /// A method read from an object: `self` is already supplied.
    BoundMethod(Rc<FunctionInfo>),
    Builtin(&'static str),
    /// A method of a str or a list, with the type it was read from.
    BuiltinMethod(Box<Ty>, Rc<str>),
    Class(Rc<ClassInfo>),
    Enum(Rc<EnumInfo>),
    /// An enum case that still needs its values: `Circle`
    CaseConstructor(Rc<EnumInfo>, usize),
    ErrorType(Rc<str>),

    Module(&'static str),
}

struct ClassInfo {
    name: String,
    fields: RefCell<Vec<Slot>>,
    methods: RefCell<HashMap<String, Rc<FunctionInfo>>>,
}

struct EnumInfo {
    name: String,
    cases: RefCell<Vec<CaseInfo>>,
}

struct CaseInfo {
    name: String,
    fields: Vec<Slot>,
}

/// A named place for an argument: a parameter, a class field, an enum case field.
#[derive(Clone)]
struct Slot {
    name: String,
    ty: Ty,
    has_default: bool,
}

/// What a caller needs to know about a function.
struct Signature {
    name: String,
    params: Vec<Slot>,
    returns: Ty,
    raises: Vec<String>,
}

/// A function and where it was defined. Its signature is worked out the
/// first time it is needed, because a return type that is not written down
/// has to be found by checking the body.
struct FunctionInfo {
    decl: ast::Function,
    env: Env,
    span: Span,
    /// For a method, the type of `self`.
    self_type: Option<Ty>,
    state: RefCell<State>,
}

#[derive(Clone)]
enum State {
    NotChecked,
    /// The body is being checked right now. A call that arrives in this
    /// state is recursion, and sees only what the signature declares.
    Checking(Rc<Signature>),
    Checked(Rc<Signature>),
}

impl Ty {
    fn list(item: Ty) -> Ty {
        Ty::List(Box::new(item))
    }

    /// True for the two types that are accepted everywhere.
    fn is_loose(&self) -> bool {
        matches!(self, Ty::Dynamic | Ty::Invalid)
    }

    fn is_number(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float)
    }

    /// Does this type have a `Dynamic` anywhere inside it?
    fn has_dynamic(&self) -> bool {
        match self {
            Ty::Dynamic => true,
            Ty::List(inner) | Ty::Optional(inner) => inner.has_dynamic(),
            _ => false,
        }
    }

    /// Can a value of this type be `none`?
    fn accepts_none(&self) -> bool {
        matches!(self, Ty::None | Ty::Optional(_)) || self.is_loose()
    }
}

/// How a type is written in an error message.
impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Ty::Dynamic | Ty::Invalid => write!(f, "any"),
            Ty::Int => write!(f, "int"),
            Ty::Float => write!(f, "float"),
            Ty::Str => write!(f, "str"),
            Ty::Bool => write!(f, "bool"),
            Ty::None => write!(f, "none"),
            Ty::Quantity(unit) => write!(f, "a quantity in {unit}"),
            Ty::List(item) => write!(f, "list[{item}]"),
            Ty::Optional(inner) => write!(f, "{inner} or none"),
            Ty::Instance(class) => write!(f, "{}", class.name),
            Ty::EnumValue(info) => write!(f, "{}", info.name),
            Ty::Error(kind) => write!(f, "{kind}"),
            Ty::Function(_) | Ty::BoundMethod(_) | Ty::Builtin(_) | Ty::BuiltinMethod(..) => {
                write!(f, "a function")
            }
            Ty::Class(class) => write!(f, "the class {}", class.name),
            Ty::Enum(info) => write!(f, "the enum {}", info.name),
            Ty::CaseConstructor(info, index) => {
                write!(f, "the enum case {}", info.cases.borrow()[*index].name)
            }
            Ty::ErrorType(kind) => write!(f, "the error type {kind}"),
            Ty::Module(name) => write!(f, "the module {name}"),
        }
    }
}

/// Are these the same type?
fn same(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Int, Ty::Int)
        | (Ty::Float, Ty::Float)
        | (Ty::Str, Ty::Str)
        | (Ty::Bool, Ty::Bool)
        | (Ty::None, Ty::None) => true,
        (Ty::Quantity(a), Ty::Quantity(b)) => a == b,
        (Ty::List(a), Ty::List(b)) | (Ty::Optional(a), Ty::Optional(b)) => same(a, b),
        (Ty::Instance(a), Ty::Instance(b)) => Rc::ptr_eq(a, b),
        (Ty::EnumValue(a), Ty::EnumValue(b)) => Rc::ptr_eq(a, b),
        (Ty::Error(a), Ty::Error(b)) => a == b,
        _ => false,
    }
}

/// May a value of type `from` go where a `to` is wanted?
fn fits(from: &Ty, to: &Ty) -> bool {
    if from.is_loose() || to.is_loose() || same(from, to) {
        return true;
    }
    match (from, to) {
        // `none` and a plain T both fit `T or none`.
        (Ty::None, Ty::Optional(_)) => true,
        (from, Ty::Optional(inner)) => fits(from, inner),
        (Ty::List(from), Ty::List(to)) => fits(from, to),
        // Any error fits the catch-all `Error`.
        (Ty::Error(_), Ty::Error(to)) => &**to == "Error",
        // Functions are not compared in detail yet.
        (
            Ty::Function(_) | Ty::BoundMethod(_) | Ty::Builtin(_) | Ty::BuiltinMethod(..),
            Ty::Function(_),
        ) => true,
        _ => false,
    }
}

/// The one type that covers both `a` and `b`: used for the items of a list
/// and the different `return`s of a function.
fn join(a: &Ty, b: &Ty) -> Ty {
    if same(a, b) {
        return a.clone();
    }
    match (a, b) {
        (Ty::Invalid, _) | (_, Ty::Invalid) => Ty::Invalid,
        (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
        (Ty::None, other) | (other, Ty::None) => match other {
            Ty::Optional(_) => other.clone(),
            _ => Ty::Optional(Box::new(other.clone())),
        },
        (Ty::Optional(inner), other) | (other, Ty::Optional(inner)) => {
            Ty::Optional(Box::new(join(inner, other)))
        }
        (Ty::List(a), Ty::List(b)) => Ty::list(join(a, b)),
        // Unrelated types: only known when the program runs.
        _ => Ty::Dynamic,
    }
}

fn join_all(types: &[Ty]) -> Option<Ty> {
    let (first, rest) = types.split_first()?;
    Some(rest.iter().fold(first.clone(), |joined, ty| join(&joined, ty)))
}

// --------------------------------------------------------------------- Scopes

/// The checker's scopes mirror the interpreter's, holding types in place of values.
struct Scope {
    variables: RefCell<HashMap<String, Variable>>,
    parent: Option<Env>,
}

type Env = Rc<Scope>;

#[derive(Clone)]
struct Variable {
    ty: Ty,
    mutable: bool,
    /// Set for a bare enum case such as `Idle`: which case it is.
    case: Option<usize>,
}

impl Scope {
    fn child(parent: &Env) -> Env {
        Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: Some(parent.clone()) })
    }

    fn declare(&self, name: &str, ty: Ty, mutable: bool) -> bool {
        let mut variables = self.variables.borrow_mut();
        if variables.contains_key(name) {
            return false;
        }
        variables.insert(name.to_string(), Variable { ty, mutable, case: None });
        true
    }

    fn get(&self, name: &str) -> Option<Variable> {
        if let Some(variable) = self.variables.borrow().get(name) {
            return Some(variable.clone());
        }
        self.parent.as_ref()?.get(name)
    }

    fn get_here(&self, name: &str) -> Option<Variable> {
        self.variables.borrow().get(name).cloned()
    }

    /// Record that, from here on in this scope, `name` is known to have a
    /// narrower type: for example `str` instead of `str or none` after a check.
    fn narrow(&self, name: &str, ty: Ty) {
        if let Some(mut variable) = self.get(name) {
            variable.ty = ty;
            self.variables.borrow_mut().insert(name.to_string(), variable);
        }
    }
}

/// The outermost scope: the built-in functions and error types.
fn globals() -> Env {
    let env = Rc::new(Scope { variables: RefCell::new(HashMap::new()), parent: None });
    for name in ["print", "len", "range", "int", "float", "str", "abs", "min", "max", "sum"] {
        env.declare(name, Ty::Builtin(name), false);
    }
    for kind in ["Error", "ValueError", "IndexError", "ZeroDivisionError", "OverflowError"] {
        env.declare(kind, Ty::ErrorType(Rc::from(kind)), false);
    }
    env
}

/// The type of `module.name`, for the built-in modules.
fn module_member(module: &str, name: &str) -> Option<Ty> {
    Some(match (module, name) {
        ("math", "pi") => Ty::Float,
        ("math", "sqrt") => Ty::Builtin("sqrt"),
        ("math", "floor") => Ty::Builtin("floor"),
        ("math", "ceil") => Ty::Builtin("ceil"),
        ("math", "sin") => Ty::Builtin("sin"),
        ("math", "cos") => Ty::Builtin("cos"),
        ("time", "sleep") => Ty::Builtin("sleep"),
        _ => return None,
    })
}

// -------------------------------------------------------------------- Checker

/// What the checker tracks for the function body it is currently inside.
/// The bottom frame stands for the top level of the file.
struct Frame {
    declared_return: Option<Ty>,
    /// The type of every `return` seen so far.
    returns: Vec<Ty>,
    /// How many loops we are inside, for `break` and `continue`.
    loops: usize,
}

/// One argument at a call, with where it is written.
struct Argument {
    name: Option<String>,
    ty: Ty,
    span: Span,
}

struct Checker {
    strict: bool,
    errors: Vec<Error>,
    frames: Vec<Frame>,
    /// The errors that can be raised by the code checked so far, innermost
    /// `try` body or function body last. Each entry is an error kind and
    /// where it comes from.
    raised: Vec<Vec<(String, Span)>>,
}

impl Checker {
    fn error(&mut self, message: impl Into<String>, span: Span) -> Ty {
        self.errors.push(Error::new(message, span));
        Ty::Invalid
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("there is always a top-level frame")
    }

    fn can_raise(&mut self, kind: &str, span: Span) {
        self.raised.last_mut().expect("there is always a top-level list").push((kind.to_string(), span));
    }

    fn declare(&mut self, env: &Env, name: &str, ty: Ty, mutable: bool, span: Span) {
        if !env.declare(name, ty, mutable) {
            self.error(format!("`{name}` is already declared in this block"), span);
        }
    }

    /// Report an error unless a `from` can go where a `to` is wanted.
    fn expect(&mut self, from: &Ty, to: &Ty, what: &str, span: Span) {
        if fits(from, to) {
            return;
        }
        let mut message = format!("{what} should be {to}, but this is {from}");
        match (from, to) {
            (Ty::Int, Ty::Float) => message += ". Write the number with a decimal point, as in `1.0`",
            (Ty::Optional(_), _) => message += ". Check it with `is not none` first",
            _ => {}
        }
        self.error(message, span);
    }

    // ------------------------------------------------------- Type annotations

    /// Turn a type written in the source into a `Ty`.
    fn resolve(&mut self, ty: &ast::Type, env: &Env, span: Span) -> Ty {
        match ty {
            ast::Type::Optional(inner) => Ty::Optional(Box::new(self.resolve(inner, env, span))),
            ast::Type::Named { name, args } => {
                let ty = match (name.as_str(), args.as_slice()) {
                    ("int", []) => Ty::Int,
                    ("float", []) => Ty::Float,
                    ("str", []) => Ty::Str,
                    ("bool", []) => Ty::Bool,
                    ("list", [item]) => Ty::list(self.resolve(item, env, span)),
                    ("list", _) => {
                        return self.error("`list` takes one type, as in `list[int]`", span)
                    }
                    // `any` asks for a dynamic value on purpose.
                    ("any", []) if self.strict => {
                        return self.error("`any` is not allowed in strict mode", span)
                    }
                    ("any", []) => Ty::Dynamic,
                    (_, []) => match env.get(name).map(|variable| variable.ty) {
                        Some(Ty::Class(class)) => Ty::Instance(class),
                        Some(Ty::Enum(info)) => Ty::EnumValue(info),
                        Some(Ty::ErrorType(kind)) => Ty::Error(kind),
                        _ => return self.error(format!("there is no type called `{name}`"), span),
                    },
                    _ => return self.error(format!("`{name}` does not take types in `[ ]`"), span),
                };
                ty
            }
        }
    }

    /// The type for something whose annotation may have been left out.
    fn resolve_or_dynamic(
        &mut self,
        ty: &Option<ast::Type>,
        what: &str,
        env: &Env,
        span: Span,
    ) -> Ty {
        match ty {
            Some(ty) => self.resolve(ty, env, span),
            None if self.strict => self.error(format!("strict mode: {what} needs a type"), span),
            None => Ty::Dynamic,
        }
    }

    // ------------------------------------------------------------ Statements

    /// Check a `{ }` block. Returns true if control can never reach the end
    /// of it, because every path returns, raises, breaks or continues.
    fn block(&mut self, statements: &[Stmt], env: &Env) -> bool {
        self.statements(statements, &Scope::child(env))
    }

    fn statements(&mut self, statements: &[Stmt], env: &Env) -> bool {
        self.hoist(statements, env);
        let mut exits = false;
        for statement in statements {
            exits |= self.stmt(statement, env);
        }
        exits
    }

    /// Declare the block's functions, classes and enums before checking any
    /// of it, exactly as the interpreter does before running it.
    fn hoist(&mut self, statements: &[Stmt], env: &Env) {
        // First the names of the new types, so that they can mention each other.
        let mut classes = Vec::new();
        let mut enums = Vec::new();
        for statement in statements {
            match &statement.kind {
                StmtKind::Class(decl) => {
                    let info = Rc::new(ClassInfo {
                        name: decl.name.clone(),
                        fields: RefCell::new(Vec::new()),
                        methods: RefCell::new(HashMap::new()),
                    });
                    self.declare(env, &decl.name, Ty::Class(info.clone()), false, statement.span);
                    classes.push((decl, info, statement.span));
                }
                StmtKind::Enum(decl) => {
                    let info = Rc::new(EnumInfo { name: decl.name.clone(), cases: RefCell::new(Vec::new()) });
                    self.declare(env, &decl.name, Ty::Enum(info.clone()), false, statement.span);
                    enums.push((decl, info, statement.span));
                }
                _ => {}
            }
        }

        // Then what is inside them.
        for (decl, info, span) in classes {
            let fields = decl.fields.iter().map(|field| self.slot(field, &decl.name, env, span)).collect();
            *info.fields.borrow_mut() = fields;
            for method in &decl.methods {
                let function = FunctionInfo {
                    decl: method.clone(),
                    env: env.clone(),
                    span,
                    self_type: Some(Ty::Instance(info.clone())),
                    state: RefCell::new(State::NotChecked),
                };
                info.methods.borrow_mut().insert(method.name.clone(), Rc::new(function));
            }
        }
        for (decl, info, span) in enums {
            let cases: Vec<CaseInfo> = decl
                .variants
                .iter()
                .map(|case| CaseInfo {
                    name: case.name.clone(),
                    fields: case.fields.iter().map(|f| self.slot(f, &case.name, env, span)).collect(),
                })
                .collect();
            for (index, case) in cases.iter().enumerate() {
                if case.fields.is_empty() {
                    // A bare case is a value of the enum.
                    self.declare(env, &case.name, Ty::EnumValue(info.clone()), false, span);
                    if let Some(variable) = env.variables.borrow_mut().get_mut(&case.name) {
                        variable.case = Some(index);
                    }
                } else {
                    self.declare(env, &case.name, Ty::CaseConstructor(info.clone(), index), false, span);
                }
            }
            *info.cases.borrow_mut() = cases;
        }

        for statement in statements {
            if let StmtKind::Function(decl) = &statement.kind {
                let function = FunctionInfo {
                    decl: decl.clone(),
                    env: env.clone(),
                    span: statement.span,
                    self_type: None,
                    state: RefCell::new(State::NotChecked),
                };
                self.declare(env, &decl.name, Ty::Function(Rc::new(function)), false, statement.span);
            }
        }
    }

    fn slot(&mut self, field: &ast::Field, owner: &str, env: &Env, span: Span) -> Slot {
        let what = format!("`{}` in `{owner}`", field.name);
        Slot {
            name: field.name.clone(),
            ty: self.resolve_or_dynamic(&field.ty, &what, env, span),
            has_default: field.default.is_some(),
        }
    }

    /// Check one statement. Returns true if it always leaves the block.
    fn stmt(&mut self, statement: &Stmt, env: &Env) -> bool {
        let span = statement.span;

        match &statement.kind {
            StmtKind::Let { name, ty, value, mutable } => {
                self.declaration(name, ty, value, *mutable, env, span);
                false
            }
            StmtKind::Const { name, ty, value } => {
                self.declaration(name, ty, value, false, env, span);
                false
            }
            StmtKind::Assign { target, op, value } => {
                let wanted = self.assignment_target(target, env);
                let mut given = self.expr(value, env);
                if let Some(op) = op {
                    given = self.binary(*op, &wanted, &given, span);
                }
                self.expect(&given, &wanted, "the new value", value.span);
                false
            }
            StmtKind::Expr(expr) => {
                self.expr(expr, env);
                false
            }

            // Declared by `hoist`. Their bodies are checked here, so that a
            // function nobody calls is still checked.
            StmtKind::Function(decl) => {
                if let Some(Variable { ty: Ty::Function(function), .. }) = env.get_here(&decl.name) {
                    self.signature(&function);
                }
                false
            }
            StmtKind::Class(decl) => {
                if let Some(Variable { ty: Ty::Class(class), .. }) = env.get_here(&decl.name) {
                    self.class_body(decl, &class, env);
                }
                false
            }
            StmtKind::Enum(_) => false,

            StmtKind::If { condition, then_block, else_block } => {
                self.condition(condition, env);
                let (when_true, when_false) = self.narrowings(condition, env);

                let then_scope = Scope::child(env);
                narrow_all(&then_scope, &when_true);
                let then_exits = self.statements(then_block, &then_scope);

                let else_scope = Scope::child(env);
                narrow_all(&else_scope, &when_false);
                let else_exits = match else_block {
                    Some(block) => self.statements(block, &else_scope),
                    None => false,
                };

                // If one branch always leaves, what follows the `if` only
                // runs when the other branch's condition held:
                //     if x is none { return }
                //     ... x is not none from here on ...
                if then_exits && !else_exits {
                    narrow_all(env, &when_false);
                } else if else_exits && !then_exits {
                    narrow_all(env, &when_true);
                }
                then_exits && else_exits
            }
            StmtKind::While { condition, body } => {
                self.condition(condition, env);
                self.frame().loops += 1;
                self.block(body, env);
                self.frame().loops -= 1;
                // `while true` with no `break` never finishes, so nothing follows it.
                condition.kind == ExprKind::Bool(true) && !has_break(body)
            }
            StmtKind::For { variable, iterable, body } => {
                let iterable_type = self.expr(iterable, env);
                let item = self.item_type(&iterable_type, iterable.span);
                let scope = Scope::child(env);
                scope.declare(variable, item, false);
                self.frame().loops += 1;
                self.statements(body, &scope);
                self.frame().loops -= 1;
                false
            }
            StmtKind::Match { subject, arms } => self.match_statement(subject, arms, env, span),
            StmtKind::Try { body, handlers } => self.try_statement(body, handlers, env, span),

            StmtKind::Return(value) => {
                let ty = match value {
                    Some(value) => self.expr(value, env),
                    None => Ty::None,
                };
                if self.frames.len() == 1 {
                    self.error("`return` outside a function", span);
                    return true;
                }
                if let Some(declared) = self.frame().declared_return.clone() {
                    let at = value.as_ref().map_or(span, |value| value.span);
                    self.expect(&ty, &declared, "the returned value", at);
                }
                self.frame().returns.push(ty);
                true
            }
            StmtKind::Raise(value) => {
                match self.expr(value, env) {
                    Ty::Error(kind) => self.can_raise(&kind, span),
                    Ty::Instance(class) => self.can_raise(&class.name, span),
                    ty if ty.is_loose() => self.can_raise("Error", span),
                    ty => {
                        self.error(format!("only an error can be raised, and this is {ty}"), span);
                    }
                }
                true
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.frame().loops == 0 {
                    let word = if statement.kind == StmtKind::Break { "break" } else { "continue" };
                    self.error(format!("`{word}` outside a loop"), span);
                }
                true
            }

            StmtKind::Import { module, names } => {
                let path = module.join(".");
                let module = match path.as_str() {
                    "math" => "math",
                    "time" => "time",
                    _ => {
                        self.error(
                            format!("there is no module `{path}`. So far Zeno has `math` and `time`"),
                            span,
                        );
                        // Declare the names anyway, so each use is not a second error.
                        for name in names.iter().chain(module.last().filter(|_| names.is_empty())) {
                            env.declare(name, Ty::Invalid, false);
                        }
                        return false;
                    }
                };
                if names.is_empty() {
                    self.declare(env, module, Ty::Module(module), false, span);
                }
                for name in names {
                    let ty = match module_member(module, name) {
                        Some(ty) => ty,
                        None => self.error(format!("module `{module}` has no `{name}`"), span),
                    };
                    self.declare(env, name, ty, false, span);
                }
                false
            }
        }
    }

    /// `let`, `var` and `const`.
    fn declaration(
        &mut self,
        name: &str,
        annotation: &Option<ast::Type>,
        value: &Expr,
        mutable: bool,
        env: &Env,
        span: Span,
    ) {
        let given = self.expr(value, env);
        let ty = match annotation {
            Some(annotation) => {
                let wanted = self.resolve(annotation, env, span);
                self.expect(&given, &wanted, &format!("`{name}`"), value.span);
                wanted
            }
            None if self.strict && given.has_dynamic() => self.error(
                format!("strict mode: the type of `{name}` cannot be worked out. Give it one, as in `{name}: list[int]`"),
                span,
            ),
            None => given,
        };
        self.declare(env, name, ty, mutable, span);
    }

    /// The type that may be assigned to `target`, after checking that it can
    /// be assigned to at all.
    fn assignment_target(&mut self, target: &Expr, env: &Env) -> Ty {
        let span = target.span;
        match &target.kind {
            ExprKind::Name(name) => match env.get(name) {
                None => self.error(
                    format!("`{name}` is not declared. Create it with `let` or `var` first"),
                    span,
                ),
                Some(variable) if !variable.mutable => self.error(
                    format!("`{name}` cannot be changed. Declare it with `var` to allow that"),
                    span,
                ),
                Some(variable) => variable.ty,
            },
            ExprKind::Field { object, name } => match self.expr(object, env) {
                Ty::Instance(class) => {
                    let field = class.fields.borrow().iter().find(|f| f.name == *name).cloned();
                    match field {
                        Some(field) => field.ty,
                        None => self.error(format!("`{}` has no field `{name}`", class.name), span),
                    }
                }
                ty if ty.is_loose() => ty,
                ty => self.error(format!("a field cannot be set on {ty}"), span),
            },
            ExprKind::Index { object, index } => {
                let object = self.expr(object, env);
                let index_type = self.expr(index, env);
                self.expect(&index_type, &Ty::Int, "an index", index.span);
                match object {
                    Ty::List(item) => *item,
                    ty if ty.is_loose() => ty,
                    ty => self.error(format!("an item cannot be set on {ty}"), span),
                }
            }
            _ => self.error("this cannot be assigned to", span),
        }
    }

    fn condition(&mut self, condition: &Expr, env: &Env) {
        let ty = self.expr(condition, env);
        if !fits(&ty, &Ty::Bool) {
            self.error(format!("a condition must be true or false, but this is {ty}"), condition.span);
        }
    }

    /// The type of each item when looping over a value of type `ty`.
    fn item_type(&mut self, ty: &Ty, span: Span) -> Ty {
        match ty {
            Ty::List(item) => (**item).clone(),
            Ty::Str => Ty::Str,
            ty if ty.is_loose() => ty.clone(),
            ty => self.error(format!("{ty} cannot be looped over"), span),
        }
    }

    /// What a condition tells us about the variables in it. Returns the
    /// narrower types that hold when it is true, and when it is false.
    ///
    /// `x is not none` means `x` is a plain `T` when true and `none` when false.
    fn narrowings(&self, condition: &Expr, env: &Env) -> (Vec<(String, Ty)>, Vec<(String, Ty)>) {
        match &condition.kind {
            ExprKind::Binary { op: op @ (BinaryOp::Is | BinaryOp::IsNot), left, .. } => {
                let ExprKind::Name(name) = &left.kind else {
                    return (Vec::new(), Vec::new());
                };
                let Some(Variable { ty: Ty::Optional(inner), .. }) = env.get(name) else {
                    return (Vec::new(), Vec::new());
                };
                let present = vec![(name.clone(), *inner)];
                let absent = vec![(name.clone(), Ty::None)];
                if *op == BinaryOp::Is {
                    (absent, present)
                } else {
                    (present, absent)
                }
            }
            ExprKind::Unary { op: UnaryOp::Not, operand } => {
                let (when_true, when_false) = self.narrowings(operand, env);
                (when_false, when_true)
            }
            // `a and b` being true means both were true.
            ExprKind::Binary { op: BinaryOp::And, left, right } => {
                let mut when_true = self.narrowings(left, env).0;
                when_true.extend(self.narrowings(right, env).0);
                (when_true, Vec::new())
            }
            // `a or b` being false means both were false.
            ExprKind::Binary { op: BinaryOp::Or, left, right } => {
                let mut when_false = self.narrowings(left, env).1;
                when_false.extend(self.narrowings(right, env).1);
                (Vec::new(), when_false)
            }
            _ => (Vec::new(), Vec::new()),
        }
    }

    fn class_body(&mut self, decl: &ast::Class, class: &Rc<ClassInfo>, env: &Env) {
        let fields = class.fields.borrow().clone();
        for (field, slot) in decl.fields.iter().zip(&fields) {
            if let Some(default) = &field.default {
                let given = self.expr(default, env);
                self.expect(&given, &slot.ty, &format!("`{}`", field.name), default.span);
            }
        }
        let methods: Vec<Rc<FunctionInfo>> = class.methods.borrow().values().cloned().collect();
        for method in methods {
            if method.decl.params.is_empty() {
                self.error(
                    format!("the method `{}` needs `self` as its first parameter", method.decl.name),
                    method.span,
                );
            }
            self.signature(&method);
        }
    }

    fn match_statement(&mut self, subject: &Expr, arms: &[Arm], env: &Env, span: Span) -> bool {
        let subject_type = self.expr(subject, env);
        let mut all_exit = true;
        // Does some case accept anything at all?
        let mut catch_all = false;
        // For an enum, the cases that are fully handled.
        let mut covered = Vec::new();

        for arm in arms {
            let scope = Scope::child(env);
            match self.pattern(&arm.pattern, &subject_type, &scope, span) {
                Covers::Everything => catch_all = true,
                Covers::Case(index) => covered.push(index),
                Covers::Some => {}
            }
            all_exit &= self.statements(&arm.body, &scope);
        }

        let complete = match &subject_type {
            _ if catch_all => true,
            Ty::EnumValue(info) => {
                let missing: Vec<String> = info
                    .cases
                    .borrow()
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !covered.contains(index))
                    .map(|(_, case)| case.name.clone())
                    .collect();
                if !missing.is_empty() {
                    self.error(
                        format!(
                            "this `match` does not handle {}. Add a `case` for it, or a final `case _`",
                            missing.join(", ")
                        ),
                        span,
                    );
                }
                missing.is_empty()
            }
            ty if ty.is_loose() => false,
            _ => {
                self.error("this `match` needs a final `case _` to handle every other value", span);
                false
            }
        };
        all_exit && complete
    }

    /// Check a pattern against the type being matched, declaring the names
    /// it introduces. Reports how much of the type the pattern accepts.
    fn pattern(&mut self, pattern: &Pattern, subject: &Ty, scope: &Env, span: Span) -> Covers {
        match pattern {
            Pattern::Wildcard => Covers::Everything,
            Pattern::Literal(literal) => {
                let ty = self.expr(literal, scope);
                if !fits(&ty, subject) && !fits(subject, &ty) {
                    self.error(format!("this case is {ty}, which can never match {subject}"), span);
                }
                Covers::Some
            }
            Pattern::Name(name) => {
                // A bare enum case is compared; any other name takes the value.
                if let Some(Variable { ty: Ty::EnumValue(info), case: Some(index), .. }) = scope.get(name) {
                    if !fits(&Ty::EnumValue(info.clone()), subject) {
                        self.error(
                            format!("`{name}` is a case of {}, which can never match {subject}", info.name),
                            span,
                        );
                    }
                    return Covers::Case(index);
                }
                scope.declare(name, subject.clone(), false);
                Covers::Everything
            }
            Pattern::Variant { name, fields } => {
                let Some(Variable { ty: Ty::CaseConstructor(info, index), .. }) = scope.get(name) else {
                    self.error(format!("`{name}` is not an enum case that carries values"), span);
                    return Covers::Some;
                };
                if !fits(&Ty::EnumValue(info.clone()), subject) {
                    self.error(
                        format!("`{name}` is a case of {}, which can never match {subject}", info.name),
                        span,
                    );
                }
                let slots = info.cases.borrow()[index].fields.clone();
                if slots.len() != fields.len() {
                    self.error(
                        format!(
                            "`{name}` carries {} value(s), but this pattern has {}",
                            slots.len(),
                            fields.len()
                        ),
                        span,
                    );
                    return Covers::Some;
                }
                // The case is fully handled only if every inner pattern accepts anything.
                let mut whole_case = true;
                for (pattern, slot) in fields.iter().zip(&slots) {
                    whole_case &= matches!(self.pattern(pattern, &slot.ty, scope, span), Covers::Everything);
                }
                if whole_case {
                    Covers::Case(index)
                } else {
                    Covers::Some
                }
            }
        }
    }

    fn try_statement(&mut self, body: &[Stmt], handlers: &[Handler], env: &Env, span: Span) -> bool {
        // Collect what the body can raise separately, so the handlers can
        // take out what they catch.
        self.raised.push(Vec::new());
        let mut all_exit = self.block(body, env);
        let raised = self.raised.pop().unwrap_or_default();

        let mut caught: Vec<&str> = Vec::new();
        for handler in handlers {
            let scope = Scope::child(env);
            let error_type = match &handler.error {
                None => Ty::Error(Rc::from("Error")),
                Some(name) => match env.get(name).map(|variable| variable.ty) {
                    Some(Ty::ErrorType(kind)) => Ty::Error(kind),
                    Some(Ty::Class(class)) => Ty::Instance(class),
                    _ => self.error(format!("`{name}` is not a kind of error"), span),
                },
            };
            caught.push(handler.error.as_deref().unwrap_or("Error"));
            if let Some(binding) = &handler.binding {
                scope.declare(binding, error_type, false);
            }
            all_exit &= self.statements(&handler.body, &scope);
        }

        // Whatever no handler catches carries on outward.
        for (kind, at) in raised {
            if !caught.iter().any(|name| *name == "Error" || *name == kind) {
                self.can_raise(&kind, at);
            }
        }
        all_exit
    }

    // -------------------------------------------------------------- Functions

    /// The signature of a function, checking its body first if that has not
    /// been done yet.
    fn signature(&mut self, function: &Rc<FunctionInfo>) -> Rc<Signature> {
        match function.state.borrow().clone() {
            State::Checked(signature) | State::Checking(signature) => return signature,
            State::NotChecked => {}
        }
        let decl = &function.decl;
        let span = function.span;
        let name = if decl.name.is_empty() { "function".to_string() } else { decl.name.clone() };

        let mut params = Vec::new();
        for (index, param) in decl.params.iter().enumerate() {
            let ty = match &function.self_type {
                // The first parameter of a method is the object itself.
                Some(self_type) if index == 0 && param.ty.is_none() => self_type.clone(),
                _ => {
                    let what = format!("the parameter `{}` of `{name}`", param.name);
                    self.resolve_or_dynamic(&param.ty, &what, &function.env, span)
                }
            };
            params.push(Slot { name: param.name.clone(), ty, has_default: param.default.is_some() });
        }
        let declared_return = decl.return_type.as_ref().map(|ty| self.resolve(ty, &function.env, span));
        for kind in &decl.raises {
            let known = function.env.get(kind).map(|variable| variable.ty);
            if !matches!(known, Some(Ty::ErrorType(_) | Ty::Class(_))) {
                self.error(format!("`{kind}` is not a kind of error"), span);
            }
        }

        // Until the body is checked, callers (that is, recursive calls) see
        // only what is written down.
        let declared = Rc::new(Signature {
            name: name.clone(),
            params: params.clone(),
            returns: declared_return.clone().unwrap_or(Ty::Dynamic),
            raises: decl.raises.clone(),
        });
        *function.state.borrow_mut() = State::Checking(declared);

        let scope = Scope::child(&function.env);
        for (param, slot) in decl.params.iter().zip(&params) {
            if let Some(default) = &param.default {
                let given = self.expr(default, &scope);
                self.expect(&given, &slot.ty, &format!("`{}`", param.name), default.span);
            }
            self.declare(&scope, &param.name, slot.ty.clone(), false, span);
        }

        self.frames.push(Frame { declared_return: declared_return.clone(), returns: Vec::new(), loops: 0 });
        self.raised.push(Vec::new());
        let exits = self.statements(&decl.body, &scope);
        let raised = self.raised.pop().unwrap_or_default();
        let frame = self.frames.pop().expect("the frame pushed above");

        let returns = match declared_return {
            Some(declared) => {
                if !exits && !declared.accepts_none() {
                    self.error(
                        format!("`{name}` can reach its end without returning the {declared} it promises"),
                        span,
                    );
                }
                declared
            }
            // Not written down: it is whatever the `return`s give, plus
            // `none` if the end of the body can be reached.
            None => {
                let mut types = frame.returns;
                if !exits {
                    types.push(Ty::None);
                }
                join_all(&types).unwrap_or(Ty::None)
            }
        };

        // In strict mode, or once a function says what it raises, the list
        // must be complete. Otherwise it is worked out, like a return type.
        let raises = if self.strict || !decl.raises.is_empty() {
            for (kind, at) in &raised {
                if !decl.raises.iter().any(|declared| declared == "Error" || declared == kind) {
                    self.error(
                        format!(
                            "`{name}` can raise {kind} here but does not say so. Add `raises {kind}` to it, or catch the error"
                        ),
                        *at,
                    );
                }
            }
            decl.raises.clone()
        } else {
            let mut kinds: Vec<String> = raised.into_iter().map(|(kind, _)| kind).collect();
            kinds.sort();
            kinds.dedup();
            kinds
        };

        let signature = Rc::new(Signature { name, params, returns, raises });
        *function.state.borrow_mut() = State::Checked(signature.clone());
        signature
    }

    // ----------------------------------------------------------- Expressions

    /// The type of an expression.
    fn expr(&mut self, expr: &Expr, env: &Env) -> Ty {
        let ty = self.expr_inner(expr, env);
        if self.strict && matches!(ty, Ty::Dynamic) {
            return self.error(
                "strict mode: the type of this cannot be worked out before the program runs",
                expr.span,
            );
        }
        ty
    }

    fn expr_inner(&mut self, expr: &Expr, env: &Env) -> Ty {
        let span = expr.span;

        match &expr.kind {
            ExprKind::Int(_) => Ty::Int,
            ExprKind::Float(_) => Ty::Float,
            ExprKind::Str(_) => Ty::Str,
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::None => Ty::None,

            ExprKind::FormatStr(parts) => {
                for part in parts {
                    if let FormatPart::Value { value, .. } = part {
                        self.expr(value, env);
                    }
                }
                Ty::Str
            }

            ExprKind::Name(name) => match env.get(name) {
                Some(variable) => variable.ty,
                None => self.error(format!("`{name}` is not defined"), span),
            },

            ExprKind::Unit { value, unit } => {
                self.expr(value, env);
                Ty::Quantity(Rc::from(unit.as_str()))
            }

            ExprKind::List(items) => {
                let types: Vec<Ty> = items.iter().map(|item| self.expr(item, env)).collect();
                // An empty list holds nothing yet, so its item type is open.
                Ty::list(join_all(&types).unwrap_or(Ty::Dynamic))
            }
            ExprKind::ListComp { element, variable, iterable, condition } => {
                let iterable_type = self.expr(iterable, env);
                let item = self.item_type(&iterable_type, iterable.span);
                let scope = Scope::child(env);
                scope.declare(variable, item, false);
                if let Some(condition) = condition {
                    self.condition(condition, &scope);
                    narrow_all(&scope, &self.narrowings(condition, &scope).0);
                }
                Ty::list(self.expr(element, &scope))
            }

            ExprKind::Unary { op, operand } => {
                let ty = self.expr(operand, env);
                match (op, &ty) {
                    (_, ty) if ty.is_loose() => ty.clone(),
                    (UnaryOp::Neg, Ty::Int | Ty::Float | Ty::Quantity(_)) => ty,
                    (UnaryOp::Not, Ty::Bool) => Ty::Bool,
                    (UnaryOp::Neg, _) => self.error(format!("`-` does not work on {ty}"), span),
                    (UnaryOp::Not, _) => self.error(format!("`not` needs a bool, but this is {ty}"), span),
                }
            }

            ExprKind::Binary { op: op @ (BinaryOp::And | BinaryOp::Or), left, right } => {
                let left_type = self.expr(left, env);
                // The right side only runs when the left side did not settle
                // it, so it can rely on what the left side established:
                //     x is not none and x.upper() == "A"
                let scope = Scope::child(env);
                let (when_true, when_false) = self.narrowings(left, env);
                narrow_all(&scope, if *op == BinaryOp::And { &when_true } else { &when_false });
                let right_type = self.expr(right, &scope);

                for (ty, operand) in [(left_type, left), (right_type, right)] {
                    if !fits(&ty, &Ty::Bool) {
                        self.error(format!("`{}` needs bools, but this is {ty}", op.text()), operand.span);
                    }
                }
                Ty::Bool
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.expr(left, env);
                let right = self.expr(right, env);
                self.binary(*op, &left, &right, span)
            }
            ExprKind::Compare { first, rest } => {
                let mut left = self.expr(first, env);
                for (op, operand) in rest {
                    let right = self.expr(operand, env);
                    self.binary(*op, &left, &right, span);
                    left = right;
                }
                Ty::Bool
            }

            ExprKind::Function(decl) => {
                let function = Rc::new(FunctionInfo {
                    decl: (**decl).clone(),
                    env: env.clone(),
                    span,
                    self_type: None,
                    state: RefCell::new(State::NotChecked),
                });
                self.signature(&function);
                Ty::Function(function)
            }

            ExprKind::Call { callee, args } => {
                let callee = self.expr(callee, env);
                let mut arguments = Vec::new();
                let mut named_seen = false;
                for arg in args {
                    if arg.name.is_some() {
                        named_seen = true;
                    } else if named_seen {
                        self.error("plain arguments must come before named ones", arg.value.span);
                    }
                    let ty = self.expr(&arg.value, env);
                    arguments.push(Argument { name: arg.name.clone(), ty, span: arg.value.span });
                }
                self.call(&callee, arguments, span)
            }

            ExprKind::Field { object, name } => {
                let object = self.expr(object, env);
                self.field(&object, name, span)
            }
            ExprKind::Index { object, index } => {
                let object = self.expr(object, env);
                let index_type = self.expr(index, env);
                self.expect(&index_type, &Ty::Int, "an index", index.span);
                match object {
                    Ty::List(item) => *item,
                    Ty::Str => Ty::Str,
                    ty if ty.is_loose() => ty,
                    Ty::Optional(_) => self.error(
                        format!("this is {object}. Check it with `is not none` before taking an item"),
                        span,
                    ),
                    ty => self.error(format!("an item cannot be taken from {ty}"), span),
                }
            }
        }
    }

    /// The type of `object.name`.
    fn field(&mut self, object: &Ty, name: &str, span: Span) -> Ty {
        match object {
            Ty::Instance(class) => {
                if let Some(field) = class.fields.borrow().iter().find(|f| f.name == name) {
                    return field.ty.clone();
                }
                if let Some(method) = class.methods.borrow().get(name) {
                    return Ty::BoundMethod(method.clone());
                }
                self.error(format!("`{}` has no field or method `{name}`", class.name), span)
            }
            // Which case the value is, is only known when the program runs.
            Ty::EnumValue(info) => {
                let known = info.cases.borrow().iter().any(|case| case.fields.iter().any(|f| f.name == name));
                if known {
                    Ty::Dynamic
                } else {
                    self.error(format!("no case of `{}` has a field `{name}`", info.name), span)
                }
            }
            Ty::Enum(info) => {
                let index = info.cases.borrow().iter().position(|case| case.name == name);
                match index {
                    Some(index) if info.cases.borrow()[index].fields.is_empty() => Ty::EnumValue(info.clone()),
                    Some(index) => Ty::CaseConstructor(info.clone(), index),
                    None => self.error(format!("enum `{}` has no case `{name}`", info.name), span),
                }
            }
            Ty::Module(module) => match module_member(module, name) {
                Some(ty) => ty,
                None => self.error(format!("module `{module}` has no `{name}`"), span),
            },
            Ty::Error(_) if name == "message" || name == "kind" => Ty::Str,
            Ty::Str if STR_METHODS.contains(&name) => Ty::BuiltinMethod(Box::new(Ty::Str), Rc::from(name)),
            Ty::List(_) if LIST_METHODS.contains(&name) => {
                Ty::BuiltinMethod(Box::new(object.clone()), Rc::from(name))
            }
            Ty::Optional(_) => self.error(
                format!("this is {object}. Check it with `is not none` before using `.{name}`"),
                span,
            ),
            ty if ty.is_loose() => ty.clone(),
            ty => self.error(format!("{ty} has no field or method `{name}`"), span),
        }
    }

    // ----------------------------------------------------------------- Calls

    fn call(&mut self, callee: &Ty, arguments: Vec<Argument>, span: Span) -> Ty {
        match callee {
            Ty::Function(function) => {
                let signature = self.signature(function);
                self.arguments(&signature.name, &signature.params, &arguments, span);
                for kind in &signature.raises {
                    self.can_raise(kind, span);
                }
                signature.returns.clone()
            }
            Ty::BoundMethod(function) => {
                let signature = self.signature(function);
                // `self` is already supplied by the object.
                let params = signature.params.get(1..).unwrap_or_default();
                self.arguments(&signature.name, params, &arguments, span);
                for kind in &signature.raises {
                    self.can_raise(kind, span);
                }
                signature.returns.clone()
            }
            Ty::Class(class) => {
                let fields = class.fields.borrow().clone();
                self.arguments(&class.name, &fields, &arguments, span);
                Ty::Instance(class.clone())
            }
            Ty::CaseConstructor(info, index) => {
                let case_name = info.cases.borrow()[*index].name.clone();
                let fields = info.cases.borrow()[*index].fields.clone();
                self.arguments(&case_name, &fields, &arguments, span);
                Ty::EnumValue(info.clone())
            }
            Ty::ErrorType(kind) => {
                if arguments.len() > 1 || arguments.iter().any(|a| a.name.is_some()) {
                    self.error(format!("`{kind}` takes one message"), span);
                }
                Ty::Error(kind.clone())
            }
            Ty::Builtin(name) => {
                self.no_named(name, &arguments, span);
                self.builtin(name, &arguments, span)
            }
            Ty::BuiltinMethod(receiver, name) => {
                self.no_named(name, &arguments, span);
                self.method(receiver, name, &arguments, span)
            }
            ty if ty.is_loose() => ty.clone(),
            ty => self.error(format!("{ty} cannot be called"), span),
        }
    }

    fn no_named(&mut self, what: &str, arguments: &[Argument], span: Span) {
        if let Some(name) = arguments.iter().find_map(|a| a.name.as_ref()) {
            self.error(format!("`{what}` does not take a named argument `{name}`"), span);
        }
    }

    /// Check the arguments of a call against the slots they are for. This
    /// follows the interpreter's `bind`, with types added.
    fn arguments(&mut self, what: &str, slots: &[Slot], arguments: &[Argument], span: Span) {
        let positional = arguments.iter().filter(|a| a.name.is_none()).count();
        if positional > slots.len() {
            self.error(
                format!("`{what}` takes {} argument(s), but {positional} were given", slots.len()),
                span,
            );
            return;
        }

        let mut filled = vec![false; slots.len()];
        let mut next = 0;
        for argument in arguments {
            let index = match &argument.name {
                None => {
                    next += 1;
                    next - 1
                }
                Some(name) => match slots.iter().position(|slot| slot.name == *name) {
                    Some(index) => index,
                    None => {
                        self.error(format!("`{what}` has no argument named `{name}`"), argument.span);
                        continue;
                    }
                },
            };
            if filled[index] {
                self.error(format!("`{what}` was given `{}` twice", slots[index].name), argument.span);
                continue;
            }
            filled[index] = true;
            let label = format!("`{}` of `{what}`", slots[index].name);
            self.expect(&argument.ty, &slots[index].ty, &label, argument.span);
        }

        for (slot, filled) in slots.iter().zip(filled) {
            if !filled && !slot.has_default {
                self.error(format!("`{what}` needs a value for `{}`", slot.name), span);
            }
        }
    }

    /// Check the number of arguments given to a built-in.
    fn arity(&mut self, name: &str, arguments: &[Argument], min: usize, max: usize, span: Span) -> bool {
        if (min..=max).contains(&arguments.len()) {
            return true;
        }
        let wanted = if min == max { min.to_string() } else { format!("{min} to {max}") };
        self.error(
            format!("`{name}` takes {wanted} argument(s), but {} were given", arguments.len()),
            span,
        );
        false
    }

    /// The result of calling a built-in function.
    fn builtin(&mut self, name: &str, arguments: &[Argument], span: Span) -> Ty {
        let first = arguments.first().map(|a| a.ty.clone()).unwrap_or(Ty::Invalid);
        let wrong = |checker: &mut Checker, wanted: &str| {
            checker.error(format!("`{name}` needs {wanted}, but this is {first}"), span)
        };

        match name {
            "print" => Ty::None,
            "str" => {
                self.arity(name, arguments, 1, 1, span);
                Ty::Str
            }
            "len" => {
                if self.arity(name, arguments, 1, 1, span) && !matches!(first, Ty::Str | Ty::List(_)) && !first.is_loose() {
                    wrong(self, "a str or a list");
                }
                Ty::Int
            }
            "range" => {
                self.arity(name, arguments, 1, 3, span);
                for argument in arguments {
                    self.expect(&argument.ty, &Ty::Int, "an argument of `range`", argument.span);
                }
                Ty::list(Ty::Int)
            }
            "int" | "float" => {
                if self.arity(name, arguments, 1, 1, span) {
                    match first {
                        Ty::Int | Ty::Float => {}
                        // Text from outside may not be a number at all.
                        Ty::Str | Ty::Dynamic => self.can_raise("ValueError", span),
                        Ty::Invalid => {}
                        _ => {
                            wrong(self, "a number or a str");
                        }
                    }
                }
                if name == "int" { Ty::Int } else { Ty::Float }
            }
            "abs" => {
                if !self.arity(name, arguments, 1, 1, span) {
                    return Ty::Invalid;
                }
                match first {
                    Ty::Int | Ty::Float | Ty::Quantity(_) => first,
                    ty if ty.is_loose() => ty,
                    _ => wrong(self, "a number"),
                }
            }
            "min" | "max" | "sum" => {
                // Either one list, or the numbers themselves.
                let types: Vec<Ty> = match arguments {
                    [Argument { ty: Ty::List(item), .. }] => vec![(**item).clone()],
                    _ => arguments.iter().map(|a| a.ty.clone()).collect(),
                };
                let joined = join_all(&types).unwrap_or(Ty::Int);
                if !joined.is_number() && !joined.is_loose() {
                    return self.error(format!("`{name}` needs numbers, but this is {joined}"), span);
                }
                joined
            }
            "sqrt" | "sin" | "cos" | "floor" | "ceil" => {
                if self.arity(name, arguments, 1, 1, span) && !first.is_number() && !first.is_loose() {
                    wrong(self, "a number");
                }
                if matches!(name, "floor" | "ceil") { Ty::Int } else { Ty::Float }
            }
            "sleep" => {
                if self.arity(name, arguments, 1, 1, span) && !matches!(first, Ty::Quantity(_)) && !first.is_loose() {
                    wrong(self, "a time with a unit, such as `500 ms`");
                }
                Ty::None
            }
            _ => Ty::Dynamic,
        }
    }

    /// The result of calling a built-in method on a str or a list.
    fn method(&mut self, receiver: &Ty, name: &str, arguments: &[Argument], span: Span) -> Ty {
        let strs = |checker: &mut Checker| {
            for argument in arguments {
                checker.expect(&argument.ty, &Ty::Str, &format!("an argument of `{name}`"), argument.span);
            }
        };
        // Call a function argument with the given argument types.
        let apply = |checker: &mut Checker, function: &Argument, types: Vec<Ty>| {
            let arguments = types
                .into_iter()
                .map(|ty| Argument { name: None, ty, span: function.span })
                .collect();
            checker.call(&function.ty, arguments, function.span)
        };

        match (receiver, name) {
            (Ty::Str, "upper" | "lower" | "strip") => {
                self.arity(name, arguments, 0, 0, span);
                Ty::Str
            }
            (Ty::Str, "split") => {
                self.arity(name, arguments, 0, 1, span);
                strs(self);
                Ty::list(Ty::Str)
            }
            (Ty::Str, "replace") => {
                self.arity(name, arguments, 2, 2, span);
                strs(self);
                Ty::Str
            }
            (Ty::Str, _) => {
                self.arity(name, arguments, 1, 1, span);
                strs(self);
                Ty::Bool
            }

            (Ty::List(item), "append") => {
                if self.arity(name, arguments, 1, 1, span) {
                    self.expect(&arguments[0].ty, item, "an item added to this list", arguments[0].span);
                }
                Ty::None
            }
            (Ty::List(item), "pop") => {
                self.arity(name, arguments, 0, 0, span);
                (**item).clone()
            }
            (Ty::List(item), "map") => {
                if !self.arity(name, arguments, 1, 1, span) {
                    return Ty::Invalid;
                }
                Ty::list(apply(self, &arguments[0], vec![(**item).clone()]))
            }
            (Ty::List(item), "filter") => {
                if self.arity(name, arguments, 1, 1, span) {
                    let result = apply(self, &arguments[0], vec![(**item).clone()]);
                    self.expect(&result, &Ty::Bool, "what the function given to `filter` returns", arguments[0].span);
                }
                receiver.clone()
            }
            (Ty::List(item), "reduce") => {
                if !self.arity(name, arguments, 2, 2, span) {
                    return Ty::Invalid;
                }
                let start = arguments[1].ty.clone();
                let result = apply(self, &arguments[0], vec![start.clone(), (**item).clone()]);
                join(&start, &result)
            }
            _ => Ty::Dynamic,
        }
    }

    // ------------------------------------------------------------- Operators

    /// The type of `left op right`. The rules are the interpreter's, applied
    /// to types.
    fn binary(&mut self, op: BinaryOp, left: &Ty, right: &Ty, span: Span) -> Ty {
        use BinaryOp::*;
        let comparison = matches!(op, Eq | NotEq | Lt | LtEq | Gt | GtEq | In | NotIn | Is | IsNot);

        // With a side that is only known at run time, so is the result,
        // except that a comparison is always a bool.
        if left.is_loose() || right.is_loose() {
            return match (left, right) {
                _ if comparison => Ty::Bool,
                (Ty::Invalid, _) | (_, Ty::Invalid) => Ty::Invalid,
                _ => Ty::Dynamic,
            };
        }

        let numbers = left.is_number() && right.is_number();
        let ints = matches!((left, right), (Ty::Int, Ty::Int));
        let same_unit = matches!((left, right), (Ty::Quantity(a), Ty::Quantity(b)) if a == b);

        let result = match op {
            Add | Sub if ints => Some(Ty::Int),
            Add | Sub if numbers => Some(Ty::Float),
            Add | Sub if same_unit => Some(left.clone()),
            Add => match (left, right) {
                (Ty::Str, Ty::Str) => Some(Ty::Str),
                (Ty::List(a), Ty::List(b)) => Some(Ty::list(join(a, b))),
                _ => None,
            },

            Mul if ints => Some(Ty::Int),
            Mul if numbers => Some(Ty::Float),
            Mul => match (left, right) {
                (Ty::Quantity(_), other) if other.is_number() => Some(left.clone()),
                (other, Ty::Quantity(_)) if other.is_number() => Some(right.clone()),
                _ => None,
            },

            Div if numbers || same_unit => Some(Ty::Float),
            Div => match (left, right) {
                (Ty::Quantity(_), other) if other.is_number() => Some(left.clone()),
                _ => None,
            },

            FloorDiv | Mod | Pow if ints => Some(Ty::Int),
            FloorDiv | Mod | Pow if numbers => Some(Ty::Float),

            Eq | NotEq => {
                // Two types that can never hold the same value: the answer
                // is already known, which means the comparison is a mistake.
                if !numbers && !fits(left, right) && !fits(right, left) {
                    let answer = if op == Eq { "false" } else { "true" };
                    self.error(
                        format!("`{}` between {left} and {right} is always {answer}", op.text()),
                        span,
                    );
                }
                Some(Ty::Bool)
            }
            Lt | LtEq | Gt | GtEq => {
                let both_strs = matches!((left, right), (Ty::Str, Ty::Str));
                (numbers || both_strs || same_unit).then_some(Ty::Bool)
            }
            In | NotIn => match (left, right) {
                (item, Ty::List(items)) if fits(item, items) || fits(items, item) => Some(Ty::Bool),
                (Ty::Str, Ty::Str) => Some(Ty::Bool),
                _ => None,
            },
            Is | IsNot => Some(Ty::Bool),
            _ => None,
        };

        match result {
            Some(ty) => ty,
            None => {
                let mut message = format!("`{}` does not work between {left} and {right}", op.text());
                if matches!(left, Ty::Optional(_)) || matches!(right, Ty::Optional(_)) {
                    message += ". Check for `none` first";
                }
                let ty = self.error(message, span);
                if comparison { Ty::Bool } else { ty }
            }
        }
    }
}

const STR_METHODS: &[&str] = &["upper", "lower", "strip", "split", "replace", "starts_with", "ends_with"];
const LIST_METHODS: &[&str] = &["append", "pop", "map", "filter", "reduce"];

/// How much of the matched type a pattern accepts.
enum Covers {
    /// Any value: `_`, or a new name.
    Everything,
    /// The whole of one enum case.
    Case(usize),
    /// Only some values.
    Some,
}

fn narrow_all(scope: &Env, narrowings: &[(String, Ty)]) {
    for (name, ty) in narrowings {
        scope.narrow(name, ty.clone());
    }
}

/// Does this loop body contain a `break` that belongs to it? Breaks inside
/// an inner loop belong to that loop instead.
fn has_break(statements: &[Stmt]) -> bool {
    statements.iter().any(|statement| match &statement.kind {
        StmtKind::Break => true,
        StmtKind::If { then_block, else_block, .. } => {
            has_break(then_block) || else_block.as_deref().is_some_and(has_break)
        }
        StmtKind::Match { arms, .. } => arms.iter().any(|arm| has_break(&arm.body)),
        StmtKind::Try { body, handlers } => {
            has_break(body) || handlers.iter().any(|handler| has_break(&handler.body))
        }
        _ => false,
    })
}
