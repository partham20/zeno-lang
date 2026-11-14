//! The values a running Zeno program works with.
//!
//! The parser's tree describes the program. A `Value` is what an expression
//! in that tree turns into when it runs: `1 + 2` becomes `Value::Int(3)`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::ast;
use crate::error::Error;
use crate::interpreter::{Env, Interpreter};
use crate::token::Span;

/// `Rc` lets several variables share one value without copying it, which is
/// how lists and objects behave: `let b = a` makes `b` the same list as `a`.
#[derive(Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Bool(bool),
    None,
    /// A number with a unit: `500 ms`
    Quantity(f64, Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),

    /// A function written in Zeno.
    Function(Rc<Closure>),
    /// A function written in Rust and built into the language, like `print`.
    Builtin(Builtin),
    /// A built-in method waiting to be called, like `"abc".upper`.
    Method(Box<Value>, Rc<str>),
    /// A method of a class, already attached to the object it was read from.
    BoundMethod(Rc<Instance>, Rc<Closure>),

    Class(Rc<ClassDef>),
    Instance(Rc<Instance>),

    Enum(Rc<EnumDef>),
    /// An enum case that carries values and still needs them: `Circle`
    CaseConstructor(Rc<EnumDef>, usize),
    /// A finished enum value: `Circle(1.0)`, or a bare case like `Idle`
    Case(Rc<CaseValue>),

    /// The name of a kind of error, callable to make one: `ValueError`
    ErrorType(Rc<str>),
    /// An error that can be raised: `ValueError("port out of range")`
    Error(Rc<ErrorValue>),

    Module(Rc<Module>),
}

/// A function together with the scope it was defined in, so that it can
/// still see the variables around it when it is called later.
pub struct Closure {
    pub decl: std::sync::Arc<ast::Function>,
    pub env: Env,
}

pub struct ClassDef {
    pub name: String,
    pub fields: Vec<ast::Field>,
    pub methods: HashMap<String, Rc<Closure>>,
    /// Where the class was defined, for working out field defaults.
    pub env: Env,
}

pub struct Instance {
    pub class: Rc<ClassDef>,
    pub fields: RefCell<HashMap<String, Value>>,
}

pub struct EnumDef {
    pub name: String,
    pub cases: Vec<ast::Variant>,
}

pub struct CaseValue {
    pub def: Rc<EnumDef>,
    /// Which case of the enum this is.
    pub index: usize,
    pub fields: Vec<Value>,
}

impl CaseValue {
    pub fn name(&self) -> &str {
        &self.def.cases[self.index].name
    }
}

pub struct ErrorValue {
    pub kind: String,
    pub message: String,
}

pub struct Module {
    pub name: &'static str,
    pub members: HashMap<&'static str, Value>,
}

#[derive(Clone, Copy)]
pub struct Builtin {
    pub name: &'static str,
    pub call: fn(&mut Interpreter, Vec<Value>, Span) -> Result<Value, Control>,
}

/// Every way a statement or expression can finish other than normally.
///
/// Rust's `?` passes these up through the interpreter until something
/// handles them: a loop catches `Break`, a call catches `Return`, a `try`
/// catches `Raise`.
pub enum Control {
    Break(Span),
    Continue(Span),
    Return(Value, Span),
    /// An error the program can catch with `try / except`.
    Raise(Value, Span),
    /// A bug in the program, such as a misspelt name or `"abc" - 1`.
    /// It cannot be caught; the type checker will later find these before the program runs.
    Fatal(Error),
}

/// Stop with a bug report. Returns `Err` so it can be used with `return` or `?`.
pub fn fatal<T>(message: impl Into<String>, span: Span) -> Result<T, Control> {
    Err(Control::Fatal(Error::new(message, span)))
}

/// Raise a built-in error that the program may catch.
pub fn raise<T>(kind: &str, message: impl Into<String>, span: Span) -> Result<T, Control> {
    let error = ErrorValue { kind: kind.to_string(), message: message.into() };
    Err(Control::Raise(Value::Error(Rc::new(error)), span))
}

impl Value {
    pub fn str(text: impl AsRef<str>) -> Value {
        Value::Str(Rc::from(text.as_ref()))
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(items)))
    }

    /// The number inside an int or float, as a float.
    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Int(n) => Some(*n as f64),
            Value::Float(x) => Some(*x),
            _ => None,
        }
    }

    /// What to call this value's type in an error message.
    pub fn type_name(&self) -> String {
        match self {
            Value::Int(_) => "an int".to_string(),
            Value::Float(_) => "a float".to_string(),
            Value::Str(_) => "a str".to_string(),
            Value::Bool(_) => "a bool".to_string(),
            Value::None => "none".to_string(),
            Value::Quantity(_, unit) => format!("a quantity in {unit}"),
            Value::List(_) => "a list".to_string(),
            Value::Function(_) | Value::Builtin(_) | Value::Method(..) | Value::BoundMethod(..) => {
                "a function".to_string()
            }
            Value::Class(_) => "a class".to_string(),
            Value::Instance(instance) => format!("a {}", instance.class.name),
            Value::Enum(_) => "an enum".to_string(),
            Value::CaseConstructor(..) => "an enum case".to_string(),
            Value::Case(case) => format!("a {}", case.def.name),
            Value::ErrorType(_) => "an error type".to_string(),
            Value::Error(error) => format!("a {}", error.kind),
            Value::Module(_) => "a module".to_string(),
        }
    }

    /// How the value looks inside a list or an object: strings get their quotes.
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => format!("{s:?}"),
            other => other.to_string(),
        }
    }
}

/// `==` in Zeno. Values are equal when their contents are equal; an int and
/// a float are equal when they are the same number.
pub fn equal(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Int(a), Int(b)) => a == b,
        (Int(_) | Float(_), Int(_) | Float(_)) => a.number() == b.number(),
        (Str(a), Str(b)) => a == b,
        (Bool(a), Bool(b)) => a == b,
        (None, None) => true,
        // `1 s == 1000 ms`: compared in the finer unit of the two.
        (Quantity(a, unit_a), Quantity(b, unit_b)) => match crate::units::common(unit_a, unit_b) {
            Some((_, left, right)) => left.apply(*a) == right.apply(*b),
            Option::None => false,
        },
        (List(a), List(b)) => {
            let (a, b) = (a.borrow(), b.borrow());
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| equal(a, b))
        }
        (Instance(a), Instance(b)) => {
            if !Rc::ptr_eq(&a.class, &b.class) {
                return false;
            }
            let (a, b) = (a.fields.borrow(), b.fields.borrow());
            a.iter().all(|(name, value)| b.get(name).is_some_and(|other| equal(value, other)))
        }
        (Case(a), Case(b)) => {
            Rc::ptr_eq(&a.def, &b.def)
                && a.index == b.index
                && a.fields.iter().zip(&b.fields).all(|(a, b)| equal(a, b))
        }
        (Error(a), Error(b)) => a.kind == b.kind && a.message == b.message,
        (Function(a), Function(b)) => Rc::ptr_eq(a, b),
        (Class(a), Class(b)) => Rc::ptr_eq(a, b),
        (Enum(a), Enum(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

/// How `print`, `str()` and f-strings show a value.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            // {:?} keeps the ".0" on whole floats, so 3.0 does not print as 3.
            Value::Float(x) => write!(f, "{x:?}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::None => write!(f, "none"),
            Value::Quantity(x, unit) if x.fract() == 0.0 && x.abs() < 1e15 => {
                write!(f, "{} {unit}", *x as i64)
            }
            Value::Quantity(x, unit) => write!(f, "{x:?} {unit}"),
            Value::List(items) => {
                let items: Vec<String> = items.borrow().iter().map(Value::repr).collect();
                write!(f, "[{}]", items.join(", "))
            }
            Value::Function(closure) if closure.decl.name.is_empty() => write!(f, "<function>"),
            Value::Function(closure) => write!(f, "<function {}>", closure.decl.name),
            Value::Builtin(builtin) => write!(f, "<function {}>", builtin.name),
            Value::Method(_, name) => write!(f, "<function {name}>"),
            Value::BoundMethod(_, closure) => write!(f, "<function {}>", closure.decl.name),
            Value::Class(class) => write!(f, "<class {}>", class.name),
            Value::Instance(instance) => {
                // Fields in the order the class lists them.
                let fields = instance.fields.borrow();
                let shown: Vec<String> = instance
                    .class
                    .fields
                    .iter()
                    .map(|field| format!("{}={}", field.name, fields[&field.name].repr()))
                    .collect();
                write!(f, "{}({})", instance.class.name, shown.join(", "))
            }
            Value::Enum(def) => write!(f, "<enum {}>", def.name),
            Value::CaseConstructor(def, index) => write!(f, "<case {}>", def.cases[*index].name),
            Value::Case(case) if case.fields.is_empty() => write!(f, "{}", case.name()),
            Value::Case(case) => {
                let fields: Vec<String> = case.fields.iter().map(Value::repr).collect();
                write!(f, "{}({})", case.name(), fields.join(", "))
            }
            Value::ErrorType(kind) => write!(f, "<error {kind}>"),
            Value::Error(error) => write!(f, "{}", error.message),
            Value::Module(module) => write!(f, "<module {}>", module.name),
        }
    }
}
