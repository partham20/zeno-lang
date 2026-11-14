//! Everything a Zeno program can use without defining it: functions like
//! `print` and `len`, methods on strings and lists, the built-in error
//! types, the `math` and `time` modules, and f-string formatting.
//!
//! These are written in Rust because they cannot be written in Zeno yet.

use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::{Env, Interpreter, Scope};
use crate::token::Span;
use crate::value::*;

type Result<T = Value> = std::result::Result<T, Control>;

const ERROR_TYPES: &[&str] =
    &["Error", "ValueError", "IndexError", "ZeroDivisionError", "OverflowError"];

/// The outermost scope, holding every built-in name.
pub fn globals() -> Env {
    let functions: &[Builtin] = &[
        Builtin { name: "print", call: print },
        Builtin { name: "len", call: len },
        Builtin { name: "range", call: range },
        Builtin { name: "int", call: int },
        Builtin { name: "float", call: float },
        Builtin { name: "str", call: str },
        Builtin { name: "abs", call: abs },
        Builtin { name: "min", call: min },
        Builtin { name: "max", call: max },
        Builtin { name: "sum", call: sum },
    ];

    let env = Scope::root();
    for builtin in functions {
        env.declare(builtin.name, Value::Builtin(*builtin), false);
    }
    for kind in ERROR_TYPES {
        env.declare(kind, Value::ErrorType(Rc::from(*kind)), false);
    }
    env
}

/// Check how many arguments a built-in was given.
fn arity(name: &str, args: &[Value], min: usize, max: usize, span: Span) -> Result<()> {
    if (min..=max).contains(&args.len()) {
        return Ok(());
    }
    let wanted = if min == max { min.to_string() } else { format!("{min} to {max}") };
    fatal(format!("`{name}` takes {wanted} argument(s), but {} were given", args.len()), span)
}

fn wrong_type<T>(name: &str, wanted: &str, got: &Value, span: Span) -> Result<T> {
    fatal(format!("`{name}` needs {wanted}, but this is {}", got.type_name()), span)
}

// ------------------------------------------------------------------ Functions

fn print(interpreter: &mut Interpreter, args: Vec<Value>, _: Span) -> Result {
    let words: Vec<String> = args.iter().map(Value::to_string).collect();
    interpreter.write_line(&words.join(" "));
    Ok(Value::None)
}

fn len(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    arity("len", &args, 1, 1, span)?;
    match &args[0] {
        Value::Str(text) => Ok(Value::Int(text.chars().count() as i64)),
        Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
        other => wrong_type("len", "a str or a list", other, span),
    }
}

/// `range(stop)`, `range(start, stop)`, `range(start, stop, step)`
fn range(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    arity("range", &args, 1, 3, span)?;
    let mut ints = Vec::new();
    for arg in &args {
        match arg {
            Value::Int(n) => ints.push(*n),
            other => return wrong_type("range", "ints", other, span),
        }
    }
    let (start, stop, step) = match ints[..] {
        [stop] => (0, stop, 1),
        [start, stop] => (start, stop, 1),
        [start, stop, step] => (start, stop, step),
        _ => unreachable!(),
    };
    if step == 0 {
        return raise("ValueError", "the step of a range cannot be 0", span);
    }

    let mut items = Vec::new();
    let mut i = start;
    while (step > 0 && i < stop) || (step < 0 && i > stop) {
        items.push(Value::Int(i));
        i += step;
    }
    Ok(Value::list(items))
}

fn int(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    arity("int", &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(x) if x.is_finite() => Ok(Value::Int(x.trunc() as i64)),
        Value::Float(x) => raise("ValueError", format!("cannot turn {x} into an int"), span),
        Value::Str(text) => match text.trim().replace('_', "").parse() {
            Ok(n) => Ok(Value::Int(n)),
            Err(_) => raise("ValueError", format!("cannot turn {text:?} into an int"), span),
        },
        other => wrong_type("int", "a number or a str", other, span),
    }
}

fn float(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    arity("float", &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) => Ok(Value::Float(*n as f64)),
        Value::Float(x) => Ok(Value::Float(*x)),
        Value::Str(text) => match text.trim().parse() {
            Ok(x) => Ok(Value::Float(x)),
            Err(_) => raise("ValueError", format!("cannot turn {text:?} into a float"), span),
        },
        other => wrong_type("float", "a number or a str", other, span),
    }
}

fn str(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    arity("str", &args, 1, 1, span)?;
    Ok(Value::str(args[0].to_string()))
}

fn abs(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    arity("abs", &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) => match n.checked_abs() {
            Some(n) => Ok(Value::Int(n)),
            None => raise("OverflowError", "the result is too big for an int", span),
        },
        Value::Float(x) => Ok(Value::Float(x.abs())),
        Value::Quantity(x, unit) => Ok(Value::Quantity(x.abs(), unit.clone())),
        other => wrong_type("abs", "a number", other, span),
    }
}

/// The numbers `min`, `max` and `sum` work on: one list, or several arguments.
fn numbers(name: &str, args: Vec<Value>, span: Span) -> Result<Vec<Value>> {
    let items = match args.as_slice() {
        [Value::List(items)] => items.borrow().clone(),
        _ => args,
    };
    for item in &items {
        if item.number().is_none() {
            return wrong_type(name, "numbers", item, span);
        }
    }
    Ok(items)
}

/// Shared by `min` and `max`: keep the item that `better` prefers.
fn pick(name: &str, args: Vec<Value>, span: Span, better: fn(f64, f64) -> bool) -> Result {
    let mut items = numbers(name, args, span)?.into_iter();
    let Some(mut best) = items.next() else {
        return raise("ValueError", format!("`{name}` of nothing"), span);
    };
    for item in items {
        if better(item.number().unwrap(), best.number().unwrap()) {
            best = item;
        }
    }
    Ok(best)
}

fn min(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    pick("min", args, span, |a, b| a < b)
}

fn max(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    pick("max", args, span, |a, b| a > b)
}

fn sum(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    let items = numbers("sum", args, span)?;
    // Stay an int while every item is an int.
    let mut int_total: Option<i64> = Some(0);
    let mut float_total = 0.0;
    for item in &items {
        float_total += item.number().unwrap();
        int_total = match (int_total, item) {
            (Some(total), Value::Int(n)) => match total.checked_add(*n) {
                Some(total) => Some(total),
                None => return raise("OverflowError", "the result is too big for an int", span),
            },
            _ => None,
        };
    }
    Ok(int_total.map_or(Value::Float(float_total), Value::Int))
}

// -------------------------------------------------------------------- Methods

const STR_METHODS: &[&str] =
    &["upper", "lower", "strip", "split", "replace", "starts_with", "ends_with"];
const LIST_METHODS: &[&str] = &["append", "pop", "map", "filter", "reduce"];

pub fn has_method(receiver: &Value, name: &str) -> bool {
    match receiver {
        Value::Str(_) => STR_METHODS.contains(&name),
        Value::List(_) => LIST_METHODS.contains(&name),
        _ => false,
    }
}

/// The text inside a str argument of a string method.
fn text<'a>(method: &str, arg: &'a Value, span: Span) -> Result<&'a str> {
    match arg {
        Value::Str(text) => Ok(text),
        other => wrong_type(method, "a str", other, span),
    }
}

pub fn call_method(
    interpreter: &mut Interpreter,
    receiver: &Value,
    name: &str,
    mut args: Vec<Value>,
    span: Span,
) -> Result {
    match receiver {
        Value::Str(this) => match name {
            "upper" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::str(this.to_uppercase()))
            }
            "lower" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::str(this.to_lowercase()))
            }
            "strip" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::str(this.trim()))
            }
            "split" => {
                arity(name, &args, 0, 1, span)?;
                let parts: Vec<Value> = match args.first() {
                    // With no separator, split on any run of spaces.
                    None => this.split_whitespace().map(Value::str).collect(),
                    Some(separator) => match text(name, separator, span)? {
                        "" => return raise("ValueError", "cannot split on an empty str", span),
                        separator => this.split(separator).map(Value::str).collect(),
                    },
                };
                Ok(Value::list(parts))
            }
            "replace" => {
                arity(name, &args, 2, 2, span)?;
                let (old, new) = (text(name, &args[0], span)?, text(name, &args[1], span)?);
                Ok(Value::str(this.replace(old, new)))
            }
            "starts_with" => {
                arity(name, &args, 1, 1, span)?;
                Ok(Value::Bool(this.starts_with(text(name, &args[0], span)?)))
            }
            "ends_with" => {
                arity(name, &args, 1, 1, span)?;
                Ok(Value::Bool(this.ends_with(text(name, &args[0], span)?)))
            }
            _ => fatal(format!("a str has no method `{name}`"), span),
        },
        Value::List(items) => match name {
            "append" => {
                arity(name, &args, 1, 1, span)?;
                items.borrow_mut().push(args.remove(0));
                Ok(Value::None)
            }
            "pop" => {
                arity(name, &args, 0, 0, span)?;
                match items.borrow_mut().pop() {
                    Some(item) => Ok(item),
                    None => raise("IndexError", "cannot pop from an empty list", span),
                }
            }

            // The three below take a function and call it on the items.
            // They work on a copy, so the function may safely change the list.
            "map" => {
                arity(name, &args, 1, 1, span)?;
                let function = args.remove(0);
                let mut mapped = Vec::new();
                for item in items.borrow().clone() {
                    mapped.push(interpreter.call(function.clone(), vec![item], Vec::new(), span)?);
                }
                Ok(Value::list(mapped))
            }
            "filter" => {
                arity(name, &args, 1, 1, span)?;
                let function = args.remove(0);
                let mut kept = Vec::new();
                for item in items.borrow().clone() {
                    match interpreter.call(function.clone(), vec![item.clone()], Vec::new(), span)? {
                        Value::Bool(true) => kept.push(item),
                        Value::Bool(false) => {}
                        other => {
                            return fatal(
                                format!(
                                    "the function given to `filter` must return true or false, but it returned {}",
                                    other.type_name()
                                ),
                                span,
                            )
                        }
                    }
                }
                Ok(Value::list(kept))
            }
            // `items.reduce(function(total, item) { ... }, start)` folds the
            // list into one value, beginning from `start`.
            "reduce" => {
                arity(name, &args, 2, 2, span)?;
                let function = args.remove(0);
                let mut total = args.remove(0);
                for item in items.borrow().clone() {
                    total = interpreter.call(function.clone(), vec![total, item], Vec::new(), span)?;
                }
                Ok(total)
            }
            _ => fatal(format!("a list has no method `{name}`"), span),
        },
        other => fatal(format!("{} has no method `{name}`", other.type_name()), span),
    }
}

// -------------------------------------------------------------------- Modules

/// Find a built-in module by name, for `import`.
pub fn module(name: &str) -> Option<Rc<Module>> {
    let (name, members): (&'static str, Vec<(&'static str, Value)>) = match name {
        "math" => (
            "math",
            vec![
                ("pi", Value::Float(std::f64::consts::PI)),
                ("sqrt", Value::Builtin(Builtin { name: "sqrt", call: sqrt })),
                ("floor", Value::Builtin(Builtin { name: "floor", call: floor })),
                ("ceil", Value::Builtin(Builtin { name: "ceil", call: ceil })),
                ("sin", Value::Builtin(Builtin { name: "sin", call: sin })),
                ("cos", Value::Builtin(Builtin { name: "cos", call: cos })),
            ],
        ),
        "time" => ("time", vec![("sleep", Value::Builtin(Builtin { name: "sleep", call: sleep }))]),
        _ => return None,
    };
    Some(Rc::new(Module { name, members: HashMap::from_iter(members) }))
}

/// The one number a math function takes.
fn one_number(name: &str, args: &[Value], span: Span) -> Result<f64> {
    arity(name, args, 1, 1, span)?;
    match args[0].number() {
        Some(x) => Ok(x),
        None => wrong_type(name, "a number", &args[0], span),
    }
}

fn sqrt(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    let x = one_number("sqrt", &args, span)?;
    if x < 0.0 {
        return raise("ValueError", format!("cannot take the square root of {x}"), span);
    }
    Ok(Value::Float(x.sqrt()))
}

fn floor(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Int(one_number("floor", &args, span)?.floor() as i64))
}

fn ceil(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Int(one_number("ceil", &args, span)?.ceil() as i64))
}

fn sin(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(one_number("sin", &args, span)?.sin()))
}

fn cos(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(one_number("cos", &args, span)?.cos()))
}

/// `sleep(500 ms)`. A time without a unit is refused, because nobody can
/// tell whether `sleep(500)` means seconds or milliseconds.
fn sleep(_: &mut Interpreter, args: Vec<Value>, span: Span) -> Result {
    arity("sleep", &args, 1, 1, span)?;
    let seconds = match &args[0] {
        Value::Quantity(x, unit) => match &**unit {
            "s" => *x,
            "ms" => x / 1e3,
            "us" => x / 1e6,
            other => return fatal(format!("`sleep` needs a time in s, ms or us, not {other}"), span),
        },
        other => return wrong_type("sleep", "a time with a unit, such as `500 ms`", other, span),
    };
    if seconds.is_finite() && seconds > 0.0 {
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    }
    Ok(Value::None)
}

// ----------------------------------------------------------------- Formatting

/// Turn a value into text for an f-string. `spec` is what follows the `:` in
/// `{value:spec}`, and follows Python's layout:
///
/// ```text
/// [align][0][width][.precision][type]
///
/// align      <  left    >  right    ^  centre
/// 0          pad numbers with zeros instead of spaces
/// width      the least number of characters to fill
/// precision  digits after the decimal point
/// type       f  fixed decimals    d  int    x X  hex    b  binary
/// ```
///
/// So `{x:.2f}` is two decimals, `{n:5}` is width 5, `{reg:08b}` is a byte in binary.
pub fn format(value: &Value, spec: Option<&str>, span: Span) -> Result<String> {
    let Some(spec) = spec else {
        return Ok(value.to_string());
    };
    let bad = || fatal(format!("`{spec}` is not a format Zeno understands"), span);
    let mut rest = spec.chars().peekable();

    let align = rest.next_if(|c| matches!(c, '<' | '>' | '^'));
    let zero = rest.next_if_eq(&'0').is_some();
    let mut width = 0;
    while let Some(digit) = rest.peek().and_then(|c| c.to_digit(10)) {
        width = width * 10 + digit as usize;
        rest.next();
    }
    let mut precision = None;
    if rest.next_if_eq(&'.').is_some() {
        let mut digits = 0;
        while let Some(digit) = rest.peek().and_then(|c| c.to_digit(10)) {
            digits = digits * 10 + digit as usize;
            rest.next();
        }
        precision = Some(digits);
    }
    let kind = rest.next();
    if rest.next().is_some() {
        return bad();
    }

    // A float type or a precision asks for fixed decimals.
    let body = match (kind, precision, value) {
        (Some('f'), _, _) | (None, Some(_), Value::Float(_)) => match value.number() {
            Some(x) => format!("{x:.*}", precision.unwrap_or(6)),
            None => return wrong_type(spec, "a number", value, span),
        },
        (None, None, _) | (Some('d'), None, Value::Int(_)) => value.to_string(),
        (Some(kind @ ('x' | 'X' | 'b')), None, Value::Int(n)) => {
            let sign = if *n < 0 { "-" } else { "" };
            let digits = match kind {
                'x' => format!("{:x}", n.unsigned_abs()),
                'X' => format!("{:X}", n.unsigned_abs()),
                _ => format!("{:b}", n.unsigned_abs()),
            };
            format!("{sign}{digits}")
        }
        (Some('d' | 'x' | 'X' | 'b'), None, other) => {
            return wrong_type(spec, "an int", other, span)
        }
        _ => return bad(),
    };

    let padding = width.saturating_sub(body.chars().count());
    let is_number = value.number().is_some();
    Ok(if zero && is_number {
        // Zeros go between the sign and the digits: -0042.
        let (sign, digits) = match body.strip_prefix('-') {
            Some(digits) => ("-", digits),
            None => ("", body.as_str()),
        };
        format!("{sign}{}{digits}", "0".repeat(padding))
    } else {
        // Numbers line up on the right by default, everything else on the left.
        match align.unwrap_or(if is_number { '>' } else { '<' }) {
            '<' => format!("{body}{}", " ".repeat(padding)),
            '>' => format!("{}{body}", " ".repeat(padding)),
            _ => {
                let left = padding / 2;
                format!("{}{body}{}", " ".repeat(left), " ".repeat(padding - left))
            }
        }
    })
}
