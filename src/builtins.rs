//! Everything a Zeno program can use without defining it: functions like
//! `print` and `len`, methods on strings and lists, the built-in error
//! types, the modules (`math`, `time`, `board`, `files`, `program`), and
//! f-string formatting.
//!
//! These are written in Rust because they cannot be written in Zeno yet.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::BinaryOp;
use crate::board;
use crate::checker::Pointee;
use crate::error::{counted, were};
use crate::interpreter::{Env, Scope};
use crate::ints::{self, Width};
use crate::ops;
use crate::token::Span;
use crate::value::*;

type Result<T = Value> = std::result::Result<T, Control>;

const ERROR_TYPES: &[&str] =
    &["Error", "ValueError", "IndexError", "ZeroDivisionError", "OverflowError", "FileError"];

/// The end of the message for an `import` of a module there is not.
pub const MODULES: &str = "So far Zeno has `math`, `time`, `board`, `files` and `program`";

/// Every built-in name and what it stands for.
pub fn all() -> Vec<(&'static str, Value)> {
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
        Builtin { name: "allocate", call: allocate },
        Builtin { name: "free", call: free },
        Builtin { name: "char", call: char_of },
        Builtin { name: "input", call: input },
    ];

    let mut all: Vec<(&'static str, Value)> =
        functions.iter().map(|builtin| (builtin.name, Value::Builtin(*builtin))).collect();
    // `u8(n)` and the other sized whole numbers.
    for (index, width) in ints::WIDTHS.iter().enumerate() {
        all.push((width.name, Value::Builtin(Builtin { name: width.name, call: FITS[index] })));
    }
    all.extend(ERROR_TYPES.iter().map(|kind| (*kind, Value::ErrorType(Rc::from(*kind)))));
    all
}

/// The outermost scope of the interpreter, holding every built-in name.
pub fn globals() -> Env {
    let env = Scope::root();
    for (name, value) in all() {
        env.declare(name, value, false);
    }
    env
}

/// Check how many arguments a built-in was given.
fn arity(name: &str, args: &[Value], min: usize, max: usize, span: Span) -> Result<()> {
    if (min..=max).contains(&args.len()) {
        return Ok(());
    }
    let wanted = if min == max { counted(min, "argument") } else { format!("{min} to {max} arguments") };
    fatal(format!("`{name}` takes {wanted}, but {} given", were(args.len())), span)
}

fn wrong_type<T>(name: &str, wanted: &str, got: &Value, span: Span) -> Result<T> {
    fatal(format!("`{name}` needs {wanted}, but this is {}", got.type_name()), span)
}

// ------------------------------------------------------------------ Functions

fn print(machine: &mut dyn Machine, args: Vec<Value>, _: Span) -> Result {
    let words: Vec<String> = args.iter().map(Value::to_string).collect();
    machine.write_line(&words.join(" "));
    Ok(Value::None)
}

/// `input()`, `input("Name: ")`: one line from the keyboard, without
/// its line end. `none` when there are no more lines, which is how a
/// program that reads to the end knows it is there.
fn input(machine: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("input", &args, 0, 1, span)?;
    if let Some(question) = args.first() {
        machine.write(text("input", question, span)?);
    }
    Ok(match machine.outside().line() {
        Some(line) => Value::str(line),
        None => Value::None,
    })
}

/// `program.arguments()`: the words the program was started with, after
/// its own name. A new list at each call.
fn program_arguments(machine: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("arguments", &args, 0, 0, span)?;
    Ok(Value::list(machine.outside().arguments.iter().map(Value::str).collect()))
}

fn len(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("len", &args, 1, 1, span)?;
    match &args[0] {
        Value::Str(text) => Ok(Value::Int(text.chars().count() as i64)),
        Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
        other => wrong_type("len", "a str or a list", other, span),
    }
}

/// `range(stop)`, `range(start, stop)`, `range(start, stop, step)`
fn range(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
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

fn int(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("int", &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(x) if x.is_finite() => Ok(Value::Int(x.trunc() as i64)),
        Value::Float(x) => raise("ValueError", format!("cannot turn {x} into an int"), span),
        Value::Str(text) => match strip(text).replace('_', "").parse() {
            Ok(n) => Ok(Value::Int(n)),
            Err(_) => raise("ValueError", format!("cannot turn {text:?} into an int"), span),
        },
        other => wrong_type("int", "a number or a str", other, span),
    }
}

fn float(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("float", &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) => Ok(Value::Float(*n as f64)),
        Value::Float(x) => Ok(Value::Float(*x)),
        Value::Str(text) => match strip(text).parse() {
            Ok(x) => Ok(Value::Float(x)),
            Err(_) => raise("ValueError", format!("cannot turn {text:?} into a float"), span),
        },
        other => wrong_type("float", "a number or a str", other, span),
    }
}

fn str(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("str", &args, 1, 1, span)?;
    Ok(Value::str(args[0].to_string()))
}

fn abs(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
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

fn min(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    pick("min", args, span, |a, b| a < b)
}

fn max(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    pick("max", args, span, |a, b| a > b)
}

fn sum(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
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

// ----------------------------------------------------------------- Sized ints

// ------------------------------------------------------------ Manual memory
//
// `allocate` and `free`, for the lines inside `unsafe { }`. See
// docs/unsafe.md. A compiled program gets real memory. Here a block is a
// list of values with a note of whether it has been freed, so that a
// mistake stops the program with its reason instead of damaging something.

/// The most items one block may have here. A compiled program has no
/// such limit. This one keeps a slip such as `allocate(u8, 1 << 40)`
/// from taking the whole computer along.
const MOST_ITEMS: i64 = 1 << 24;

/// The type a built-in such as `u8` or `float` names, where one is
/// written as the first argument of `allocate` or `register`.
fn named_type(name: &str, value: &Value, span: Span) -> Result<Pointee> {
    match value {
        Value::Builtin(builtin) if Pointee::named(builtin.name).is_some() => {
            Ok(Pointee::named(builtin.name).unwrap_or(Pointee::Int))
        }
        other => wrong_type(name, "a type first, such as u8", other, span),
    }
}

/// `allocate(u8, 16)`: a block of sixteen bytes, all zero.
fn allocate(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("allocate", &args, 2, 2, span)?;
    let pointee = named_type("allocate", &args[0], span)?;
    let count = match &args[1] {
        Value::Int(count) if (1..=MOST_ITEMS).contains(count) => *count as usize,
        Value::Int(count) if *count < 1 => {
            return fatal(format!("`allocate` needs at least one item, and this asks for {count}"), span)
        }
        Value::Int(count) => {
            return fatal(format!("`zeno run` allocates at most {MOST_ITEMS} items at once, and this asks for {count}"), span)
        }
        other => return wrong_type("allocate", "how many items, as an int", other, span),
    };
    let zero = if pointee == Pointee::Float { Value::Float(0.0) } else { Value::Int(0) };
    let memory = Memory { pointee, items: RefCell::new(vec![zero; count]), freed: std::cell::Cell::new(false) };
    Ok(Value::Pointer(Rc::new(memory), 0))
}

/// `free(p)`: give the block back. Every pointer into it is dead from
/// here on, and `zeno run` remembers that.
fn free(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("free", &args, 1, 1, span)?;
    match &args[0] {
        Value::Pointer(memory, _) if memory.freed.get() => fatal(
            "this memory was freed already. `zeno run` checks every `free`. \
             A compiled program does not: freeing twice damages the memory it hands out next",
            span,
        ),
        Value::Pointer(_, at) if *at != 0 => fatal(
            format!(
                "`free` needs the pointer that `allocate` gave, and this one points {} further on. \
                 `zeno run` checks every `free`. A compiled program does not",
                counted(*at as usize, "item")
            ),
            span,
        ),
        Value::Pointer(memory, _) => {
            memory.freed.set(true);
            memory.items.borrow_mut().clear();
            Ok(Value::None)
        }
        other => wrong_type("free", "a pointer that `allocate` gave", other, span),
    }
}

/// `register(u32, 0x4002_0014)`: the register of the board at that address.
fn register(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("register", &args, 2, 2, span)?;
    let width = match named_type("register", &args[0], span)? {
        Pointee::Sized(width) if !width.signed => width,
        _ => return fatal("a register is a u8, a u16 or a u32", span),
    };
    match &args[1] {
        Value::Int(address) => Ok(Value::Register(*address, width)),
        other => wrong_type("register", "an address, as an int", other, span),
    }
}

// A sized whole number is an ordinary `Value::Int` that is known to be in
// range: nothing about it is different once it exists. So all the work is
// in the two ways of making one.

/// `u8(n)`: `n` itself if a u8 can hold it, and an OverflowError if not.
fn fit_in(width: Width, args: Vec<Value>, span: Span) -> Result {
    arity(width.name, &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) if width.holds(*n) => Ok(Value::Int(*n)),
        Value::Int(n) => raise("OverflowError", width.does_not_fit(n), span),
        other => wrong_type(width.name, "a whole number", other, span),
    }
}

/// `u8.wrap(n)`: the low eight bits of `n`. Never an error.
fn wrap_to(width: Width, args: Vec<Value>, span: Span) -> Result {
    arity(width.wrap, &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(width.wrap(*n))),
        other => wrong_type(width.wrap, "a whole number", other, span),
    }
}

// A built-in is a plain function pointer, which cannot remember which size
// it is for. So there is one function for each size, stamped out from
// these two by the number in `::<N>`: `fit::<0>` is `u8( )`, because `u8`
// is first in `ints::WIDTHS`.

fn fit<const N: usize>(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    fit_in(ints::WIDTHS[N], args, span)
}

fn wrap<const N: usize>(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    wrap_to(ints::WIDTHS[N], args, span)
}

type Call = fn(&mut dyn Machine, Vec<Value>, Span) -> Result;
const FITS: [Call; 6] = [fit::<0>, fit::<1>, fit::<2>, fit::<3>, fit::<4>, fit::<5>];
const WRAPS: [Call; 6] = [wrap::<0>, wrap::<1>, wrap::<2>, wrap::<3>, wrap::<4>, wrap::<5>];

/// `u8.wrap`, given the built-in `u8`. `None` for any other built-in.
pub fn wrapping(sized: &Builtin) -> Option<Value> {
    let index = ints::WIDTHS.iter().position(|width| width.name == sized.name)?;
    // `u8` could also be a function of the program's own. Only the real
    // one has a `.wrap`.
    (sized.call as usize == FITS[index] as usize)
        .then(|| Value::Builtin(Builtin { name: ints::WIDTHS[index].wrap, call: WRAPS[index] }))
}

// -------------------------------------------------------------------- Methods

/// The blank characters that `strip`, `split`, `int` and `float` skip:
/// space, tab, the two line endings and the form feed. Only these, so that
/// the compiled program needs no table of the world's other blanks.
fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

/// `text` without the blanks at either end.
fn strip(text: &str) -> &str {
    text.trim_matches(is_blank)
}

const STR_METHODS: &[&str] =
    &["upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code"];
const LIST_METHODS: &[&str] = &[
    "append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join",
];
const PIN_METHODS: &[&str] = &["on", "off", "toggle", "is_on"];
const REGISTER_METHODS: &[&str] = &["read", "write"];

pub fn has_method(receiver: &Value, name: &str) -> bool {
    match receiver {
        Value::Str(_) => STR_METHODS.contains(&name),
        Value::List(_) => LIST_METHODS.contains(&name),
        Value::Pin(_) => PIN_METHODS.contains(&name),
        Value::Register(..) => REGISTER_METHODS.contains(&name),
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

/// A whole number given to a method.
fn whole(method: &str, arg: &Value, span: Span) -> Result<i64> {
    match arg {
        Value::Int(n) => Ok(*n),
        other => wrong_type(method, "a whole number", other, span),
    }
}

/// Where `slice(from)` or `slice(from, to)` begins and ends, in something
/// `length` long. The end is not part of the slice, as in `range`.
fn ends(args: &[Value], length: usize, what: &str, span: Span) -> Result<(usize, usize)> {
    let from = whole("slice", &args[0], span)?;
    let to = match args.get(1) {
        Some(to) => whole("slice", to, span)?,
        None => length as i64,
    };
    if from < 0 || to < from || to > length as i64 {
        return raise(
            "IndexError",
            format!("slice {from} to {to} is out of range for a {what} of length {length}"),
            span,
        );
    }
    Ok((from as usize, to as usize))
}

/// Put `items` in order, smallest first, with `<`.
///
/// This is heapsort, and it is the same heapsort as `zeno_list_sort` in
/// runtime.c, step for step. Any way of sorting gives the same answer
/// for numbers that can all be told apart. Writing the same steps twice
/// means the two also agree on the ones that cannot (`0.0` and `-0.0`),
/// so a compiled program prints what `zeno run` prints.
///
/// Heapsort needs no second list and never calls itself, which is what
/// a firmware build wants (see docs/library.md).
fn sort(items: &mut [Value], span: Span) -> Result<()> {
    let before = |a: &Value, b: &Value| -> Result<bool> {
        match ops::binary(BinaryOp::Lt, a.clone(), b.clone(), span)? {
            Value::Bool(before) => Ok(before),
            _ => Ok(false),
        }
    };
    // Move the item at `root` down until both items below it are smaller.
    let sift = |items: &mut [Value], mut root: usize, end: usize| -> Result<()> {
        loop {
            let mut child = 2 * root + 1;
            if child >= end {
                return Ok(());
            }
            if child + 1 < end && before(&items[child], &items[child + 1])? {
                child += 1;
            }
            if !before(&items[root], &items[child])? {
                return Ok(());
            }
            items.swap(root, child);
            root = child;
        }
    };
    let len = items.len();
    for root in (0..len / 2).rev() {
        sift(items, root, len)?;
    }
    for end in (1..len).rev() {
        items.swap(0, end);
        sift(items, 0, end)?;
    }
    Ok(())
}

pub fn call_method(
    machine: &mut dyn Machine,
    receiver: &Value,
    name: &str,
    mut args: Vec<Value>,
    span: Span,
) -> Result {
    match receiver {
        Value::Str(this) => match name {
            "upper" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::str(this.to_ascii_uppercase()))
            }
            "lower" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::str(this.to_ascii_lowercase()))
            }
            "strip" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::str(strip(this)))
            }
            "split" => {
                arity(name, &args, 0, 1, span)?;
                let parts: Vec<Value> = match args.first() {
                    // With no separator, split on any run of spaces.
                    None => this.split(is_blank).filter(|word| !word.is_empty()).map(Value::str).collect(),
                    Some(separator) => match text(name, separator, span)? {
                        "" => return raise("ValueError", "cannot split on an empty str", span),
                        separator => this.split(separator).map(Value::str).collect(),
                    },
                };
                Ok(Value::list(parts))
            }
            // The lines of a text: what a file holds, read one at a time.
            // A line ends at "\n", and a "\r" just before it (a file
            // from Windows) goes with it. The last line need not end.
            "lines" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::list(this.lines().map(Value::str).collect()))
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
            // Where `part` first begins, counted in characters, or
            // `none` if it is not there.
            "find" => {
                arity(name, &args, 1, 1, span)?;
                Ok(match this.find(text(name, &args[0], span)?) {
                    Some(byte) => Value::Int(this[..byte].chars().count() as i64),
                    None => Value::None,
                })
            }
            "slice" => {
                arity(name, &args, 1, 2, span)?;
                let (from, to) = ends(&args, this.chars().count(), "str", span)?;
                Ok(Value::str(this.chars().skip(from).take(to - from).collect::<String>()))
            }
            // The number Unicode gives to a character: 65 for "A".
            "code" => {
                arity(name, &args, 0, 0, span)?;
                let mut chars = this.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Ok(Value::Int(c as i64)),
                    _ => raise(
                        "ValueError",
                        format!(
                            "`code` needs a str of exactly one character, and this one has {}",
                            this.chars().count()
                        ),
                        span,
                    ),
                }
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
                arity(name, &args, 0, 1, span)?;
                // `pop(2)` takes out the item at position 2.
                if let Some(at) = args.first() {
                    let len = items.borrow().len();
                    let at = ops::position(at, len, "list", span)?;
                    return Ok(items.borrow_mut().remove(at));
                }
                match items.borrow_mut().pop() {
                    Some(item) => Ok(item),
                    None => raise("IndexError", "cannot pop from an empty list", span),
                }
            }
            // `insert(0, x)` puts `x` in front. The position may be one
            // past the last item, which is what `append` does.
            "insert" => {
                arity(name, &args, 2, 2, span)?;
                let (at, len) = (whole(name, &args[0], span)?, items.borrow().len());
                if at < 0 || at > len as i64 {
                    return raise(
                        "IndexError",
                        format!("cannot insert at {at} in a list of length {len}"),
                        span,
                    );
                }
                items.borrow_mut().insert(at as usize, args.remove(1));
                Ok(Value::None)
            }
            "reverse" => {
                arity(name, &args, 0, 0, span)?;
                items.borrow_mut().reverse();
                Ok(Value::None)
            }
            "clear" => {
                arity(name, &args, 0, 0, span)?;
                items.borrow_mut().clear();
                Ok(Value::None)
            }
            "sort" => {
                arity(name, &args, 0, 0, span)?;
                // Sorted on the side, so that a `<` which fails leaves
                // the list as it was.
                let mut sorted = items.borrow().clone();
                sort(&mut sorted, span)?;
                *items.borrow_mut() = sorted;
                Ok(Value::None)
            }
            // The position of the first item equal to the one given, or
            // `none`.
            "find" => {
                arity(name, &args, 1, 1, span)?;
                Ok(match items.borrow().iter().position(|item| equal(item, &args[0])) {
                    Some(at) => Value::Int(at as i64),
                    None => Value::None,
                })
            }
            "slice" => {
                arity(name, &args, 1, 2, span)?;
                let (from, to) = ends(&args, items.borrow().len(), "list", span)?;
                Ok(Value::list(items.borrow()[from..to].to_vec()))
            }
            // `words.join(", ")`: the items, with the separator between.
            "join" => {
                arity(name, &args, 1, 1, span)?;
                let separator = text(name, &args[0], span)?;
                let mut pieces = Vec::new();
                for item in items.borrow().iter() {
                    match item {
                        Value::Str(piece) => pieces.push(piece.to_string()),
                        other => {
                            return fatal(
                                format!(
                                    "`join` puts text together, and this list holds {}. \
                                     Make each item text first, with `str( )`",
                                    other.type_name()
                                ),
                                span,
                            )
                        }
                    }
                }
                Ok(Value::str(pieces.join(separator)))
            }

            // The three below take a function and call it on the items.
            // They work on a copy, so the function may safely change the list.
            "map" => {
                arity(name, &args, 1, 1, span)?;
                let function = args.remove(0);
                let mut mapped = Vec::new();
                for item in items.borrow().clone() {
                    mapped.push(machine.call(function.clone(), vec![item], Vec::new(), span)?);
                }
                Ok(Value::list(mapped))
            }
            "filter" => {
                arity(name, &args, 1, 1, span)?;
                let function = args.remove(0);
                let mut kept = Vec::new();
                for item in items.borrow().clone() {
                    match machine.call(function.clone(), vec![item.clone()], Vec::new(), span)? {
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
                    total = machine.call(function.clone(), vec![total, item], Vec::new(), span)?;
                }
                Ok(total)
            }
            _ => fatal(format!("a list has no method `{name}`"), span),
        },
        // A pin of the board. Driving it goes through `drive`, so that
        // the line the simulated board prints comes out with the rest of
        // the program's output.
        Value::Pin(pin) => {
            arity(name, &args, 0, 0, span)?;
            match name {
                "on" => drive(machine, *pin, true),
                "off" => drive(machine, *pin, false),
                "toggle" => {
                    let level = !machine.board().read(*pin);
                    drive(machine, *pin, level)
                }
                "is_on" => Ok(Value::Bool(machine.board().read(*pin))),
                _ => fatal(format!("a pin has no method `{name}`"), span),
            }
        }
        // A register of the simulated board holds what was last written
        // to it, and the board prints a line when that changes.
        Value::Register(address, width) => match name {
            "read" => {
                arity(name, &args, 0, 0, span)?;
                Ok(Value::Int(machine.board().register_read(*address)))
            }
            "write" => {
                arity(name, &args, 1, 1, span)?;
                let value = match &args[0] {
                    Value::Int(value) if width.holds(*value) => *value,
                    Value::Int(value) => return raise("OverflowError", width.does_not_fit(value), span),
                    other => return wrong_type("write", "a whole number", other, span),
                };
                let line = machine.board().register_write(*address, value, (width.bits / 4) as usize);
                machine.write_line(&line);
                Ok(Value::None)
            }
            _ => fatal(format!("a register has no method `{name}`"), span),
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
                ("e", Value::Float(std::f64::consts::E)),
                ("sqrt", Value::Builtin(Builtin { name: "sqrt", call: sqrt })),
                ("floor", Value::Builtin(Builtin { name: "floor", call: floor })),
                ("ceil", Value::Builtin(Builtin { name: "ceil", call: ceil })),
                ("round", Value::Builtin(Builtin { name: "round", call: round })),
                ("sin", Value::Builtin(Builtin { name: "sin", call: sin })),
                ("cos", Value::Builtin(Builtin { name: "cos", call: cos })),
                ("tan", Value::Builtin(Builtin { name: "tan", call: tan })),
                ("asin", Value::Builtin(Builtin { name: "asin", call: asin })),
                ("acos", Value::Builtin(Builtin { name: "acos", call: acos })),
                ("atan", Value::Builtin(Builtin { name: "atan", call: atan })),
                ("atan2", Value::Builtin(Builtin { name: "atan2", call: atan2 })),
                ("exp", Value::Builtin(Builtin { name: "exp", call: exp })),
                ("log", Value::Builtin(Builtin { name: "log", call: log })),
                ("log2", Value::Builtin(Builtin { name: "log2", call: log2 })),
                ("log10", Value::Builtin(Builtin { name: "log10", call: log10 })),
            ],
        ),
        "time" => (
            "time",
            vec![
                ("sleep", Value::Builtin(Builtin { name: "sleep", call: sleep })),
                ("now", Value::Builtin(Builtin { name: "now", call: now })),
            ],
        ),
        "files" => (
            "files",
            vec![
                ("read", Value::Builtin(Builtin { name: "read", call: files_read })),
                ("write", Value::Builtin(Builtin { name: "write", call: files_write })),
                ("append", Value::Builtin(Builtin { name: "append", call: files_append })),
                ("exists", Value::Builtin(Builtin { name: "exists", call: files_exists })),
                ("remove", Value::Builtin(Builtin { name: "remove", call: files_remove })),
            ],
        ),
        "program" => (
            "program",
            vec![("arguments", Value::Builtin(Builtin { name: "arguments", call: program_arguments }))],
        ),
        "board" => (
            "board",
            vec![
                ("led", Value::Pin(board::LED)),
                ("pin", Value::Builtin(Builtin { name: "pin", call: pin })),
                ("register", Value::Builtin(Builtin { name: "register", call: register })),
            ],
        ),
        _ => return None,
    };
    Some(Rc::new(Module { name, members: HashMap::from_iter(members) }))
}

/// `char(65)`: the character Unicode gives that number to, as a str.
/// The opposite of `"A".code()`.
fn char_of(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("char", &args, 1, 1, span)?;
    let code = whole("char", &args[0], span)?;
    // 0 is left out: it could not be told from the end of the text in
    // a compiled program, where text ends at the first zero byte.
    match u32::try_from(code).ok().filter(|code| *code != 0).and_then(char::from_u32) {
        Some(c) => Ok(Value::str(c.to_string())),
        None => raise("ValueError", format!("{code} is not the code of a character"), span),
    }
}

// ---------------------------------------------------------------- Files
//
// The `files` module: a whole file read as text, text written to a file,
// and two questions about a path. Each function here has a twin in
// runtime.c (`zeno_files_read` and the rest), and the two are written
// to give the same answer and the same message in every case, so that
// a compiled program prints what `zeno run` prints.

/// What is at a path.
#[derive(PartialEq)]
enum At {
    /// Nothing.
    Nothing,
    /// A file that can be read.
    File,
    /// Something else: a folder, or a file this program may not read.
    Other,
}

/// Open the path and try to read one byte. A folder can be opened on
/// some systems, and only fails when it is read.
fn at(path: &str) -> At {
    use std::io::Read;
    match std::fs::File::open(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => At::Nothing,
        Err(_) => At::Other,
        Ok(mut file) => match file.read(&mut [0u8; 1]) {
            Ok(_) => At::File,
            Err(_) => At::Other,
        },
    }
}

/// The text arguments of a `files` function.
fn paths<'a>(name: &str, args: &'a [Value], count: usize, span: Span) -> Result<Vec<&'a str>> {
    arity(name, args, count, count, span)?;
    args.iter().map(|arg| text(name, arg, span)).collect()
}

fn file_error<T>(message: String, span: Span) -> Result<T> {
    raise("FileError", message, span)
}

/// `files.read(path)`: everything in the file, as one str.
fn files_read(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    let path = paths("read", &args, 1, span)?[0];
    match at(path) {
        At::Nothing => return file_error(format!("there is no file `{path}`"), span),
        At::Other => return file_error(format!("cannot read `{path}`"), span),
        At::File => {}
    }
    let Ok(bytes) = std::fs::read(path) else {
        return file_error(format!("cannot read `{path}`"), span);
    };
    // Text is UTF-8 with no zero byte in it: a compiled program ends
    // text at the first zero.
    match String::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => Ok(Value::str(text)),
        _ => file_error(format!("`{path}` does not hold text"), span),
    }
}

/// `files.write(path, text)` makes the file, or replaces what was in
/// it. `files.append(path, text)` adds to the end.
fn put(name: &str, args: Vec<Value>, append: bool, span: Span) -> Result {
    use std::io::Write;
    let texts = paths(name, &args, 2, span)?;
    let (path, text) = (texts[0], texts[1]);
    let mut options = std::fs::OpenOptions::new();
    if append { options.append(true) } else { options.write(true).truncate(true) };
    match options.create(true).open(path).and_then(|mut file| file.write_all(text.as_bytes())) {
        Ok(()) => Ok(Value::None),
        Err(_) => file_error(format!("cannot write `{path}`"), span),
    }
}

fn files_write(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    put("write", args, false, span)
}

fn files_append(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    put("append", args, true, span)
}

/// `files.exists(path)`: is there a file there that can be read?
fn files_exists(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    let path = paths("exists", &args, 1, span)?[0];
    Ok(Value::Bool(at(path) == At::File))
}

/// `files.remove(path)`: the file is gone. A folder is left alone.
fn files_remove(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    let path = paths("remove", &args, 1, span)?[0];
    match at(path) {
        At::Nothing => file_error(format!("there is no file `{path}`"), span),
        At::File if std::fs::remove_file(path).is_ok() => Ok(Value::None),
        _ => file_error(format!("cannot remove `{path}`"), span),
    }
}

/// The one number a math function takes.
fn one_number(name: &str, args: &[Value], span: Span) -> Result<f64> {
    arity(name, args, 1, 1, span)?;
    match args[0].number() {
        Some(x) => Ok(x),
        None => wrong_type(name, "a number", &args[0], span),
    }
}

fn sqrt(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    let x = one_number("sqrt", &args, span)?;
    if x < 0.0 {
        return raise("ValueError", format!("cannot take the square root of {x:?}"), span);
    }
    Ok(Value::Float(x.sqrt()))
}

fn floor(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Int(one_number("floor", &args, span)?.floor() as i64))
}

fn ceil(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Int(one_number("ceil", &args, span)?.ceil() as i64))
}

/// `round(2.5)` is 3 and `round(-2.5)` is -3: a half goes away from
/// zero, as it is taught at school.
fn round(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Int(one_number("round", &args, span)?.round() as i64))
}

/// The angle given to `sin`, `cos` or `tan`, in radians. A plain number
/// is radians already, as in mathematics. An angle with a unit says
/// which it is: `sin(30 deg)`.
fn angle(name: &str, args: &[Value], span: Span) -> Result<f64> {
    arity(name, args, 1, 1, span)?;
    match &args[0] {
        Value::Quantity(x, unit) => match crate::units::convert(unit, "rad") {
            Some(scale) => Ok(scale.apply(*x)),
            None => fatal(
                format!("`{name}` needs a number or an angle, such as `30 deg`, not a quantity in {unit}"),
                span,
            ),
        },
        other => match other.number() {
            Some(x) => Ok(x),
            None => wrong_type(name, "a number or an angle, such as `30 deg`", other, span),
        },
    }
}

/// An angle that a function worked out. It is in radians, and says so.
fn radians(x: f64) -> Result {
    Ok(Value::Quantity(x, Rc::from("rad")))
}

fn sin(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(angle("sin", &args, span)?.sin()))
}

fn cos(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(angle("cos", &args, span)?.cos()))
}

fn tan(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(angle("tan", &args, span)?.tan()))
}

/// The number given to `asin` or `acos`, which is the sine or cosine
/// of some angle, and so lies from -1 to 1.
fn ratio(name: &str, args: &[Value], span: Span) -> Result<f64> {
    let x = one_number(name, args, span)?;
    if x < -1.0 || x > 1.0 {
        return raise("ValueError", format!("`{name}` needs a number from -1 to 1, and this is {x:?}"), span);
    }
    Ok(x)
}

fn asin(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    radians(ratio("asin", &args, span)?.asin())
}

fn acos(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    radians(ratio("acos", &args, span)?.acos())
}

fn atan(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    radians(one_number("atan", &args, span)?.atan())
}

/// `atan2(y, x)`: the angle of the point (x, y) seen from (0, 0), all
/// the way round, which `atan(y / x)` cannot tell.
fn atan2(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("atan2", &args, 2, 2, span)?;
    match (args[0].number(), args[1].number()) {
        (Some(y), Some(x)) => radians(y.atan2(x)),
        (None, _) => wrong_type("atan2", "two numbers", &args[0], span),
        (_, None) => wrong_type("atan2", "two numbers", &args[1], span),
    }
}

fn exp(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(one_number("exp", &args, span)?.exp()))
}

/// The number given to a logarithm, which has to be above zero.
fn above_zero(name: &str, args: &[Value], span: Span) -> Result<f64> {
    let x = one_number(name, args, span)?;
    if x <= 0.0 {
        return raise("ValueError", format!("cannot take the logarithm of {x:?}"), span);
    }
    Ok(x)
}

fn log(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(above_zero("log", &args, span)?.ln()))
}

fn log2(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(above_zero("log2", &args, span)?.log2()))
}

fn log10(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    Ok(Value::Float(above_zero("log10", &args, span)?.log10()))
}

/// `sleep(500 ms)`. A time without a unit is refused, because nobody can
/// tell whether `sleep(500)` means seconds or milliseconds.
fn sleep(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("sleep", &args, 1, 1, span)?;
    let seconds = match &args[0] {
        Value::Quantity(x, unit) => match crate::units::convert(unit, "s") {
            Some(scale) => scale.apply(*x),
            None => return fatal(format!("`sleep` needs a time, such as `500 ms`, not a quantity in {unit}"), span),
        },
        other => return wrong_type("sleep", "a time with a unit, such as `500 ms`", other, span),
    };
    // Zero, a negative time and "not a number" wait for nothing. A time
    // too long to count is cut to about thirty years.
    if seconds > 0.0 {
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds.min(1e9)));
    }
    Ok(Value::None)
}

/// `now()`: the reading of a clock that only goes forward, in
/// milliseconds. Take two readings and subtract.
fn now(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("now", &args, 0, 0, span)?;
    Ok(Value::Quantity(board::now(), Rc::from("ms")))
}

/// `pin(7)`: the pin with that number.
fn pin(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    arity("pin", &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(number) if (0..board::PINS).contains(number) => Ok(Value::Pin(*number)),
        Value::Int(number) => raise(
            "ValueError",
            format!("this board has no pin {number}. Its pins are 0 to {}", board::PINS - 1),
            span,
        ),
        other => wrong_type("pin", "an int", other, span),
    }
}

/// Set a pin high or low, and print what the simulated board shows.
fn drive(machine: &mut dyn Machine, pin: i64, level: bool) -> Result {
    if let Some(line) = machine.board().write(pin, level) {
        machine.write_line(&line);
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
