//! What the operators and the other small steps of the language do.
//!
//! Zeno has two engines that run a program: the interpreter, which walks
//! the syntax tree, and the VM, which runs bytecode. They differ in *how*
//! they get from one step to the next. What a step does must not differ:
//! `7 // 2` is 3 in both, and `"a" - 1` is the same mistake with the same
//! message. So each step is written once, here, and both engines call it.
//!
//! Nothing in this file knows which engine is asking.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::ast::{BinaryOp, UnaryOp};
use crate::builtins;
use crate::checker::Pointee;
use crate::error::{counted, were, Error};
use crate::token::Span;
use crate::units;
use crate::value::*;

type Result<T = Value> = std::result::Result<T, Control>;

// -------------------------------------------------------------------- Errors

/// The name `except` matches against: the error's kind, or an object's class.
pub fn error_kind(value: &Value) -> String {
    match value {
        Value::Error(error) => error.kind.clone(),
        Value::Instance(instance) => instance.class.name.clone(),
        _ => "Error".to_string(),
    }
}

/// What to report when something escapes the whole program.
pub fn escaped(control: Control) -> Error {
    match control {
        Control::Fatal(error) => error,
        Control::Raise(value, span) => {
            // An object of a class from an imported file is named as
            // that file names it: `BadShape`, not `shapes.BadShape`.
            Error::new(format!("uncaught {}: {value}", crate::ast::shown_name(&error_kind(&value))), span)
        }
        Control::Break(span) => Error::new("`break` outside a loop", span),
        Control::Continue(span) => Error::new("`continue` outside a loop", span),
        Control::Return(_, span) => Error::new("`return` outside a function", span),
    }
}

/// `raise value`: what travels up to the nearest `try`.
pub fn raised(value: Value, span: Span) -> Control {
    match value {
        Value::Error(_) => Control::Raise(value, span),
        // An object is wrapped in an error named after its class, so that
        // every `except` receives the same kind of thing whatever was raised.
        Value::Instance(ref instance) => {
            let wrapped = ErrorValue {
                kind: instance.class.name.clone(),
                message: value.to_string(),
                object: Some(value.clone()),
            };
            Control::Raise(Value::Error(Rc::new(wrapped)), span)
        }
        other => Control::Fatal(Error::new(
            format!("only an error can be raised, and this is {}", other.type_name()),
            span,
        )),
    }
}

/// Does `except name` catch an error of this kind? A bare `except` and
/// `except Error` both catch everything.
pub fn catches(name: Option<&str>, kind: &str) -> bool {
    match name {
        None => true,
        Some(name) => name == "Error" || name == kind,
    }
}

/// What `except name as e` calls `e`.
///
/// `except Timeout as t` asked for the object that was raised, so `t` is
/// that object. `except Error as e` asked for any error, so `e` is the
/// error the object travelled in.
pub fn caught(name: Option<&str>, error: &Value) -> Value {
    match name {
        Some(name) if name != "Error" => raised_object(error),
        _ => error.clone(),
    }
}

/// The object an error is carrying, if an object was raised. If not, the
/// error itself.
pub fn raised_object(error: &Value) -> Value {
    match error {
        Value::Error(inner) => inner.object.clone().unwrap_or_else(|| error.clone()),
        _ => error.clone(),
    }
}

// ----------------------------------------------------------------- Operators

/// A condition has to be a real bool; see docs/decisions.md.
pub fn condition(value: Value, span: Span) -> Result<bool> {
    match value {
        Value::Bool(b) => Ok(b),
        other => fatal(
            format!("a condition must be true or false, but this is {}", other.type_name()),
            span,
        ),
    }
}

/// One side of `and` or `or`.
pub fn logic_operand(op: BinaryOp, value: Value, span: Span) -> Result<bool> {
    match value {
        Value::Bool(b) => Ok(b),
        other => fatal(format!("`{}` needs bools, but this is {}", op.text(), other.type_name()), span),
    }
}

/// `-x` and `not x`.
pub fn unary(op: UnaryOp, value: Value, span: Span) -> Result {
    match (op, value) {
        (UnaryOp::Neg, Value::Int(n)) => match n.checked_neg() {
            Some(n) => Ok(Value::Int(n)),
            None => raise("OverflowError", "the result is too big for an int", span),
        },
        (UnaryOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
        (UnaryOp::Neg, Value::F32(x)) => Ok(Value::F32(-x)),
        (UnaryOp::Neg, Value::Quantity(x, unit)) => Ok(Value::Quantity(-x, unit)),
        (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        // Every bit turned over. For a number held in two's complement
        // that is `-n - 1`, which always fits.
        (UnaryOp::BitNot, Value::Int(n)) => Ok(Value::Int(!n)),
        (UnaryOp::BitNot, Value::U64(n)) => Ok(Value::U64(!n)),
        (UnaryOp::Neg, Value::U64(_)) => fatal("`-` does not work on a u64, which is never below 0", span),
        (UnaryOp::BitNot, other) => {
            fatal(format!("`~` works on the bits of a whole number, and this is {}", other.type_name()), span)
        }
        (UnaryOp::Neg, other) => fatal(format!("`-` does not work on {}", other.type_name()), span),
        (UnaryOp::Not, other) => {
            fatal(format!("`not` needs a bool, but this is {}", other.type_name()), span)
        }
    }
}

/// `&` and its kin, given something that is not a whole number. Two bools
/// are the likely mistake, coming from C.
fn bits_mismatch(op: BinaryOp, left: &Value, right: &Value, span: Span) -> Result {
    let hint = match (op, left, right) {
        (BinaryOp::BitAnd, Value::Bool(_), Value::Bool(_)) => ". For two bools Zeno spells this `and`",
        (BinaryOp::BitOr, Value::Bool(_), Value::Bool(_)) => ". For two bools Zeno spells this `or`",
        _ => "",
    };
    fatal(
        format!(
            "`{}` works on the bits of whole numbers, and these are {} and {}{hint}",
            op.text(),
            left.type_name(),
            right.type_name()
        ),
        span,
    )
}

/// A number with a unit after it: `500 ms`.
pub fn quantity(value: Value, unit: &str, span: Span) -> Result {
    match value.number() {
        _ if !units::is_unit(unit) => fatal(units::unknown(unit), span),
        Some(number) => Ok(Value::Quantity(number, Rc::from(unit))),
        None => fatal("a unit can only follow a number", span),
    }
}

/// What `*` or `/` makes of two quantities, or of a plain number over a
/// quantity (decision 43). Each side is first put in the unit of its kind
/// with no prefix, and the answer is in that unit of its own kind.
fn made(op: &str, left: (f64, Option<&str>), right: (f64, &str)) -> Option<Value> {
    let ((x, left_unit), (y, right_unit)) = (left, right);
    let made = units::made(op, left_unit, right_unit)?;
    let x = match left_unit {
        Some(unit) => units::to_plain(unit)?.apply(x),
        None => x,
    };
    let y = units::to_plain(right_unit)?.apply(y);
    let answer = if op == "*" { x * y } else { x / y };
    Some(match made {
        units::Made::Unit(unit) => Value::Quantity(answer, Rc::from(unit)),
        units::Made::Number => Value::Float(answer),
    })
}

/// An operator with a value on each side: `a + b`, `a < b`, `a in b`.
/// Two ints and two floats, which most operators are between, are
/// answered here; the rest is in `binary_rest` (decision 42.11).
#[inline]
pub fn binary(op: BinaryOp, left: Value, right: Value, span: Span) -> Result {
    match fast_binary(op, &left, &right) {
        Some(value) => Ok(value),
        None => binary_rest(op, left, right, span),
    }
}

/// The answer of `binary_rest` for two ints or two floats, when it is
/// an answer and not an error, worked out without calling it. `None`
/// sends everything else, and every error, the long way.
#[inline(always)]
pub fn fast_binary(op: BinaryOp, left: &Value, right: &Value) -> Option<Value> {
    use BinaryOp::*;
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Some(match op {
            Add => Value::Int(a.checked_add(*b)?),
            Sub => Value::Int(a.checked_sub(*b)?),
            Mul => Value::Int(a.checked_mul(*b)?),
            // The result takes the sign of the right side (see below).
            Mod if *b > 0 => {
                let r = a % b;
                Value::Int(if r < 0 { r + b } else { r })
            }
            Lt => Value::Bool(a < b),
            LtEq => Value::Bool(a <= b),
            Gt => Value::Bool(a > b),
            GtEq => Value::Bool(a >= b),
            Eq | Is => Value::Bool(a == b),
            NotEq | IsNot => Value::Bool(a != b),
            _ => return None,
        }),
        (Value::Float(a), Value::Float(b)) => Some(match op {
            Add => Value::Float(a + b),
            Sub => Value::Float(a - b),
            Mul => Value::Float(a * b),
            Div if *b != 0.0 => Value::Float(a / b),
            // NaN compares false with everything, as below.
            Lt => Value::Bool(a < b),
            LtEq => Value::Bool(a <= b),
            Gt => Value::Bool(a > b),
            GtEq => Value::Bool(a >= b),
            _ => return None,
        }),
        _ => None,
    }
}

/// Every operator, the long way: what `binary` is, with the errors.
pub fn binary_rest(op: BinaryOp, left: Value, right: Value, span: Span) -> Result {
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

    // `p + 4`, `p - 1`: the same block, so many items further on or
    // back. Nothing is read, so nothing is checked until it is.
    if let (Add | Sub, Value::Pointer(memory, at), Int(n)) = (op, &left, &right) {
        let n = if op == Add { *n } else { n.wrapping_neg() };
        return Ok(Value::Pointer(memory.clone(), at.wrapping_add(n)));
    }

    // An f32 or a u64 on either side: worked out in 32 bits (decision
    // 41.2) or as a u64 (decision 42.2). One test of both sides, so the
    // ints, floats and strings of every other program pay for no more
    // (decision 42.11).
    if matches!(left, Value::F32(_) | Value::U64(_)) || matches!(right, Value::F32(_) | Value::U64(_)) {
        if let Some(answer) = narrow_binary(op, &left, &right, span) {
            return answer;
        }
    }

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
            // `[0] * 64`: the items, so many times over (decision 36.3).
            (List(_), Int(times)) if *times < 0 => {
                raise("ValueError", format!("`*` repeats a list 0 or more times, and this is {times}"), span)
            }
            (List(items), Int(times)) => {
                let items = items.borrow();
                let mut repeated = Vec::with_capacity(items.len().saturating_mul(*times as usize));
                for _ in 0..*times {
                    repeated.extend(items.iter().cloned());
                }
                Ok(Value::list(repeated))
            }
            // `3.3 V * 20 mA` is a power (decision 43).
            (Quantity(a, unit), Quantity(b, other)) => match made("*", (*a, Some(unit)), (*b, other)) {
                Some(value) => Ok(value),
                None => mismatch(),
            },
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
                None => match made("/", (*a, Some(unit)), (*b, other)) {
                    Some(value) => Ok(value),
                    None => mismatch(),
                },
            },
            (Quantity(a, unit), other) => match other.number() {
                Some(b) => Ok(Quantity(a / b, unit.clone())),
                None => mismatch(),
            },
            // `1 / (2 ms)` is a frequency (decision 43).
            (Int(_) | Float(_), Quantity(b, other)) => match left.number().and_then(|a| made("/", (a, None), (*b, other))) {
                _ if *b == 0.0 => by_zero(),
                Some(value) => Ok(value),
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

        // The bits of two whole numbers. A negative number is taken in
        // two's complement, as every processor holds it.
        BitAnd | BitOr | BitXor => match (&left, &right) {
            (Int(a), Int(b)) => Ok(Int(match op {
                BitAnd => a & b,
                BitOr => a | b,
                _ => a ^ b,
            })),
            _ => bits_mismatch(op, &left, &right, span),
        },
        // `a << n` is `a` times two, `n` times, and like `*` it must fit.
        // `a >> n` is `a` halved `n` times, rounding down like `//`.
        Shl | Shr => match (&left, &right) {
            (Int(_), Int(n)) if !(0..64).contains(n) => raise(
                "ValueError",
                format!("cannot shift by {n}. An int has 64 bits, so a shift is 0 to 63"),
                span,
            ),
            (Int(a), Int(n)) if op == Shr => Ok(Int(a >> n)),
            (Int(a), Int(n)) => {
                let shifted = a << n;
                // Shifting back gives the number again unless bits fell off the top.
                if shifted >> n == *a { Ok(Int(shifted)) } else { overflow() }
            }
            _ => bits_mismatch(op, &left, &right, span),
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
                // `"ann" in ages`: is it one of the keys?
                (item, Value::Dict(dict)) => match Key::of(item) {
                    Some(key) => dict.borrow().get(&key).is_some(),
                    None => return mismatch(),
                },
                (Str(part), Str(text)) => text.contains(&**part),
                (item, Value::Foreign(foreign)) => crate::bridge::has(foreign, item, span)?,
                _ => return mismatch(),
            };
            Ok(Bool(found == (op == In)))
        }

        // Handled by each engine, because they must not always work out both sides.
        And | Or => mismatch(),
    }
}

/// What is said when a u64 sum does not fit in 64 bits.
pub const TOO_BIG_FOR_U64: &str = "the result is too big for a u64";
/// And when it would go below 0.
pub const BELOW_0_FOR_U64: &str = "the result is below 0, and a u64 never is";

/// `a op b` where one side is a u64, or `None` for an operator that works
/// no differently on one (`in`). Sums are checked as an int's are: one
/// past 64 bits, or below 0, is an `OverflowError` (decision 42.2).
/// An operator with an f32 or a u64 on one side, kept out of `binary`.
#[inline(never)]
fn narrow_binary(op: BinaryOp, left: &Value, right: &Value, span: Span) -> Option<Result> {
    if matches!(left, Value::F32(_)) || matches!(right, Value::F32(_)) {
        if let Some(answer) = f32_binary(op, left, right, span) {
            return Some(answer);
        }
    }
    if matches!(left, Value::U64(_)) || matches!(right, Value::U64(_)) {
        return u64_binary(op, left, right, span);
    }
    None
}

fn u64_binary(op: BinaryOp, left: &Value, right: &Value, span: Span) -> Option<Result> {
    use BinaryOp::*;
    let too_big = || raise("OverflowError", TOO_BIG_FOR_U64, span);
    let by_zero = || raise("ZeroDivisionError", "division by zero", span);
    // A shift is by a count, which is an int or a u64.
    if let (Shl | Shr, Value::U64(a)) = (op, left) {
        let n = match right {
            Value::Int(n) => *n as i128,
            Value::U64(n) => *n as i128,
            _ => return Some(bits_mismatch(op, left, right, span)),
        };
        if !(0..64).contains(&n) {
            return Some(raise("ValueError", format!("cannot shift by {n}. A u64 has 64 bits, so a shift is 0 to 63"), span));
        }
        let shifted = a << n;
        return Some(match op {
            Shr => Ok(Value::U64(a >> n)),
            _ if shifted >> n == *a => Ok(Value::U64(shifted)),
            _ => too_big(),
        });
    }
    let (Some(a), Some(b)) = (left.unsigned(), right.unsigned()) else {
        let sum = matches!(op, Add | Sub | Mul | Div | FloorDiv | Mod | Pow | BitAnd | BitOr | BitXor | Shl | Shr);
        let order = matches!(op, Lt | LtEq | Gt | GtEq);
        if !(sum || order) {
            return None;
        }
        return Some(fatal(
            format!(
                "`{}` between {} and {} would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them",
                op.text(),
                left.type_name(),
                right.type_name()
            ),
            span,
        ));
    };
    Some(match op {
        Add => a.checked_add(b).map(Value::U64).map_or_else(too_big, Ok),
        Sub => a.checked_sub(b).map(Value::U64).map_or_else(|| raise("OverflowError", BELOW_0_FOR_U64, span), Ok),
        Mul => a.checked_mul(b).map(Value::U64).map_or_else(too_big, Ok),
        Div if b == 0 => by_zero(),
        Div => Ok(Value::Float(a as f64 / b as f64)),
        FloorDiv | Mod if b == 0 => by_zero(),
        FloorDiv => Ok(Value::U64(a / b)),
        Mod => Ok(Value::U64(a % b)),
        Pow => match (a, u32::try_from(b)) {
            (0 | 1, _) if b > 0 => Ok(Value::U64(a)),
            (_, _) if b == 0 => Ok(Value::U64(1)),
            (_, Ok(b)) => a.checked_pow(b).map(Value::U64).map_or_else(too_big, Ok),
            _ => too_big(),
        },
        BitAnd => Ok(Value::U64(a & b)),
        BitOr => Ok(Value::U64(a | b)),
        BitXor => Ok(Value::U64(a ^ b)),
        Eq | Is => Ok(Value::Bool(a == b)),
        NotEq | IsNot => Ok(Value::Bool(a != b)),
        Lt => Ok(Value::Bool(a < b)),
        LtEq => Ok(Value::Bool(a <= b)),
        Gt => Ok(Value::Bool(a > b)),
        GtEq => Ok(Value::Bool(a >= b)),
        _ => return None,
    })
}

/// `a op b` where one side is an f32, or `None` for an operator that
/// works no differently on one (`in`). The other side is an f32 or an
/// int, which becomes a float and then the nearest f32 first. `+`, `-`,
/// `*` and `/` are done in 32 bits; `//`, `%` and `**` in 64, rounded
/// once at the end, so that every engine gives the same f32 (41.2).
fn f32_binary(op: BinaryOp, left: &Value, right: &Value, span: Span) -> Option<Result> {
    use BinaryOp::*;
    let side = |value: &Value| match value {
        Value::F32(x) => Some(*x),
        Value::Int(n) => Some(*n as f64 as f32),
        _ => None,
    };
    let (Some(a), Some(b)) = (side(left), side(right)) else {
        if matches!(op, Add | Sub | Mul | Div | FloorDiv | Mod | Pow | Lt | LtEq | Gt | GtEq)
            && (matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)))
        {
            return Some(fatal(
                format!(
                    "`{}` between {} and {} would have to choose between 32 bits and 64. \
                     Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64",
                    op.text(),
                    left.type_name(),
                    right.type_name()
                ),
                span,
            ));
        }
        return None;
    };
    let by_zero = || raise("ZeroDivisionError", "division by zero", span);
    let (wide_a, wide_b) = (a as f64, b as f64);
    Some(match op {
        Add => Ok(Value::F32(a + b)),
        Sub => Ok(Value::F32(a - b)),
        Mul => Ok(Value::F32(a * b)),
        Div | FloorDiv | Mod if b == 0.0 => by_zero(),
        Div => Ok(Value::F32(a / b)),
        FloorDiv => Ok(Value::F32((wide_a / wide_b).floor() as f32)),
        Mod => Ok(Value::F32((wide_a - wide_b * (wide_a / wide_b).floor()) as f32)),
        Pow => Ok(Value::F32(wide_a.powf(wide_b) as f32)),
        Eq => Ok(Value::Bool(a == b)),
        NotEq => Ok(Value::Bool(a != b)),
        Lt => Ok(Value::Bool(a < b)),
        LtEq => Ok(Value::Bool(a <= b)),
        Gt => Ok(Value::Bool(a > b)),
        GtEq => Ok(Value::Bool(a >= b)),
        _ => return None,
    })
}

// ----------------------------------------------------------- Fields and items

/// A value as the key of a dict.
pub fn key(value: &Value, span: Span) -> Result<Key> {
    match Key::of(value) {
        Some(key) => Ok(key),
        None => fatal(format!("a key of a dict is a str or an int, but this is {}", value.type_name()), span),
    }
}

/// The error for a key that is not in the dict.
pub fn no_key<T>(key: &Value, span: Span) -> Result<T> {
    raise("KeyError", format!("this dict has no key {}", key.repr()), span)
}

/// The things a `for` loop or comprehension steps through.
pub fn items(value: &Value, span: Span) -> Result<Vec<Value>> {
    match value {
        // A copy, so changing the list inside the loop cannot upset the loop.
        Value::List(items) => Ok(items.borrow().clone()),
        // A dict is gone through by its keys, in the order they were
        // put in. A copy again: the loop goes through the keys that
        // were there when it started.
        Value::Dict(dict) => Ok(dict.borrow().entries().iter().map(|(key, _)| key.clone()).collect()),
        Value::Str(text) => Ok(text.chars().map(|c| Value::str(c.to_string())).collect()),
        // Whatever Python can loop over, taken now, all of it.
        Value::Foreign(foreign) => crate::bridge::items(foreign, span),
        other => fatal(format!("cannot loop over {}", other.type_name()), span),
    }
}

/// The same for a loop with two names, `for key, value in ages`: each
/// key of a dict with what is under it, or each position of a list or a
/// str with what is there. They are taken as they were when the loop
/// began, like the items of a loop with one name.
pub fn pairs(value: &Value, span: Span) -> Result<Vec<(Value, Value)>> {
    match value {
        Value::Dict(dict) => Ok(dict.borrow().entries().to_vec()),
        other => Ok(items(other, span)?.into_iter().enumerate().map(|(at, item)| (Value::Int(at as i64), item)).collect()),
    }
}

/// Check an index and turn it into a position. Positions start at 0 and
/// negative ones are refused; see docs/decisions.md.
pub fn position(index: &Value, len: usize, what: &str, span: Span) -> Result<usize> {
    let Value::Int(i) = index else {
        return fatal(format!("an index must be an int, but this is {}", index.type_name()), span);
    };
    match usize::try_from(*i) {
        Ok(i) if i < len => Ok(i),
        _ => raise("IndexError", format!("index {i} is out of range for a {what} of length {len}"), span),
    }
}

/// `object[index]`
pub fn item(object: &Value, index: &Value, span: Span) -> Result {
    match object {
        Value::Foreign(foreign) => crate::bridge::item(foreign, index, span),
        Value::List(items) => {
            let items = items.borrow();
            let i = position(index, items.len(), "list", span)?;
            Ok(items[i].clone())
        }
        Value::Str(text) => {
            let chars: Vec<char> = text.chars().collect();
            let i = position(index, chars.len(), "str", span)?;
            Ok(Value::str(chars[i].to_string()))
        }
        Value::Dict(dict) => match dict.borrow().get(&key(index, span)?) {
            Some(value) => Ok(value.clone()),
            None => no_key(index, span),
        },
        Value::Pointer(memory, at) => {
            let i = pointed_at(memory, *at, index, "reads", span)?;
            Ok(memory.items.borrow()[i].clone())
        }
        other => fatal(format!("cannot take an item from {}", other.type_name()), span),
    }
}

/// Which item of its block `pointer[index]` means.
///
/// This is the difference between `zeno run` and a compiled program. A
/// compiled program has an address and nothing else, so it reads or
/// writes whatever is there. `zeno run` knows the block, and stops with
/// the reason.
fn pointed_at(memory: &Memory, at: i64, index: &Value, does: &str, span: Span) -> Result<usize> {
    let Value::Int(index) = index else {
        return fatal(format!("an index must be an int, but this is {}", index.type_name()), span);
    };
    if memory.freed.get() {
        return fatal(
            format!(
                "this {does} memory that was freed. `zeno run` checks every use of a pointer. \
                 A compiled program does not: it {does} whatever is there now, which may be somebody else's"
            ),
            span,
        );
    }
    let len = memory.items.borrow().len() as i64;
    let item = at.wrapping_add(*index);
    if !(0..len).contains(&item) {
        return fatal(
            format!(
                "this {does} item {item} of a block that has {len} (0 to {}). `zeno run` checks every use of \
                 a pointer. A compiled program does not: it {does} whatever is beside the block",
                len - 1
            ),
            span,
        );
    }
    Ok(item as usize)
}

/// `object[index] = value`
pub fn set_item(object: Value, index: &Value, value: Value, span: Span) -> Result<()> {
    match object {
        Value::Foreign(foreign) => crate::bridge::set_item(&foreign, index, &value, span),
        Value::List(items) => {
            let len = items.borrow().len();
            let i = position(index, len, "list", span)?;
            items.borrow_mut()[i] = value;
            Ok(())
        }
        // `ages["ann"] = 31` puts the key in, or gives it a new value.
        Value::Dict(dict) => {
            let key = key(index, span)?;
            dict.borrow_mut().set(key, value);
            Ok(())
        }
        Value::Pointer(memory, at) => {
            let i = pointed_at(&memory, at, index, "writes over", span)?;
            // The checker lets only the right kind of number through.
            // Where it could not know, the value is checked here.
            let value = match (memory.pointee, value) {
                (Pointee::Float, Value::Int(n)) => Value::Float(n as f64),
                (Pointee::Float, value @ Value::Float(_)) | (Pointee::Int, value @ Value::Int(_)) => value,
                (Pointee::Sized(width), Value::Int(n)) if width.holds(n) => Value::Int(n),
                (Pointee::Sized(width), Value::Int(n)) => return raise("OverflowError", width.does_not_fit(n), span),
                (pointee, other) => {
                    return fatal(
                        format!("this pointer points at {} items, and this is {}", pointee.name(), other.type_name()),
                        span,
                    )
                }
            };
            memory.items.borrow_mut()[i] = value;
            Ok(())
        }
        other => fatal(format!("cannot set an item of {}", other.type_name()), span),
    }
}

/// `object.name`
pub fn field(object: Value, name: &str, span: Span) -> Result {
    match &object {
        Value::Foreign(foreign) => crate::bridge::field(foreign, name, span),
        Value::Instance(instance) => {
            if let Some(value) = instance.get(name) {
                return Ok(value);
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
        Value::Event(event) => match event.field(name) {
            Some(value) => Ok(value),
            None => fatal(format!("an Event has no field `{name}`. It has `{}`", crate::value::EVENT_FIELDS.join("`, `")), span),
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
        // `beat.period`: from one tick to the next, in milliseconds.
        Value::Ticker(_, period) if name == "period" => Ok(Value::Quantity(*period, Rc::from("ms"))),
        // `led.number`: which pin this is.
        Value::Pin(number)
        | Value::InputPin(number)
        | Value::Serial(number)
        | Value::AnalogPin(number)
        | Value::PwmPin(number)
            if name == "number" =>
        {
            Ok(Value::Int(*number))
        }
        Value::AnalogPin(_) if name == "max_count" => Ok(Value::Int(crate::board::analog_max())),
        Value::Register(address, _) if name == "address" => Ok(Value::Int(*address)),
        // `u8.wrap`: the conversion that keeps the low bits.
        Value::Builtin(sized) if name == "wrap" && builtins::wrapping(sized).is_some() => {
            Ok(builtins::wrapping(sized).unwrap_or(Value::None))
        }
        Value::Error(error) if name == "message" => Ok(Value::str(&error.message)),
        Value::Error(error) if name == "kind" => Ok(Value::str(&error.kind)),
        _ if builtins::has_method(&object, name) => {
            Ok(Value::Method(Box::new(object), Rc::from(name)))
        }
        other => fatal(format!("{} has no field or method `{name}`", other.type_name()), span),
    }
}

/// `object.name = value`
pub fn set_field(object: Value, name: &str, value: Value, span: Span) -> Result<()> {
    match object {
        Value::Instance(instance) => {
            match instance.class.slot(name) {
                Some(i) => instance.fields.borrow_mut()[i] = value,
                None => return fatal(format!("`{}` has no field `{name}`", instance.class.name), span),
            }
            Ok(())
        }
        Value::Foreign(foreign) => crate::bridge::set_field(&foreign, name, &value, span),
        other => fatal(format!("cannot set a field on {}", other.type_name()), span),
    }
}

// --------------------------------------------------------------------- Calls

/// A case with no values is a finished value; one with values needs calling.
pub fn case_value(def: &Rc<EnumDef>, index: usize) -> Value {
    if def.cases[index].fields.is_empty() {
        Value::Case(Rc::new(CaseValue { def: def.clone(), index, fields: Vec::new() }))
    } else {
        Value::CaseConstructor(def.clone(), index)
    }
}

pub fn no_named(what: &str, named: &[(String, Value)], span: Span) -> Result<()> {
    match named.first() {
        Some((name, _)) => fatal(format!("`{what}` does not take a named argument `{name}`"), span),
        None => Ok(()),
    }
}

/// Line the arguments of a call up with the names they are for. The result
/// has one slot per name, `None` where no argument was given.
pub fn bind(
    what: &str,
    names: &[&str],
    positional: Vec<Value>,
    named: Vec<(String, Value)>,
    span: Span,
) -> Result<Vec<Option<Value>>> {
    if positional.len() > names.len() {
        return fatal(
            format!("`{what}` takes {}, but {} given", counted(names.len(), "argument"), were(positional.len())),
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

/// Call an enum case that carries values: `Circle(1.0)`.
pub fn new_case(
    def: &Rc<EnumDef>,
    index: usize,
    positional: Vec<Value>,
    named: Vec<(String, Value)>,
    span: Span,
) -> Result {
    let case = &def.cases[index];
    let names: Vec<&str> = case.fields.iter().map(|f| f.name.as_str()).collect();
    let slots = bind(&case.name, &names, positional, named, span)?;
    let mut fields = Vec::new();
    for (field, slot) in case.fields.iter().zip(slots) {
        match slot {
            Some(value) => fields.push(value),
            None => return fatal(format!("`{}` needs a value for `{}`", case.name, field.name), span),
        }
    }
    Ok(Value::Case(Rc::new(CaseValue { def: def.clone(), index, fields })))
}

/// Call the name of a kind of error: `ValueError("too hot")`.
pub fn new_error(kind: &str, positional: Vec<Value>, named: Vec<(String, Value)>, span: Span) -> Result {
    no_named(kind, &named, span)?;
    let message = match positional.as_slice() {
        [] => String::new(),
        [message] => message.to_string(),
        _ => return fatal(format!("`{kind}` takes one message"), span),
    };
    Ok(Value::Error(Rc::new(ErrorValue { kind: kind.to_string(), message, object: None })))
}
