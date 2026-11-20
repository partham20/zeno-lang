// The Zeno runtime for Rust. (Not a module of Zeno itself: emit_rust.rs
// puts this text at the top of every program built for Rust.)
//
// A program built with `zeno build --target rust` is one `.rs` file: this
// runtime, the module `z`, then the program, in `main`. It holds what
// every Zeno value is, and what each operator and built-in does with it:
// what ops.rs and builtins.rs do for `zeno run`, step for step and message
// for message, so that the program prints what `zeno run` prints. Zeno is
// written in Rust, so this one can print and read floats with the very
// same code `zeno run` uses.
//
// Every step that can fail gives a `z::R`: the value, or the error on its
// way to a `try` (`Exc::Raise`) or a mistake that stops the program
// (`Exc::Fatal`), and the program passes it on with `?`.

#![allow(warnings)]

mod z {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::rc::Rc;

    pub const LINES_A_FILE: usize = 1000000;

    // ---------------------------------------------------------- Values

    #[derive(Clone)]
    pub enum V {
        None,
        /// What a call gives a parameter it leaves out.
        Missing,
        Int(i64),
        Float(f64),
        /// An f32 (decision 41).
        F32(f32),
        /// A u64 (decision 42).
        U64(u64),
        Bool(bool),
        Str(Rc<str>),
        List(Rc<RefCell<Vec<V>>>),
        Dict(Rc<RefCell<Dict>>),
        Quantity(f64, Rc<str>),
        Func(Rc<Func>),
        Class(Rc<Class>),
        Instance(Rc<Instance>),
        Enum(Rc<Enum>),
        Case(Rc<Case>),
        Ctor(Rc<Enum>, usize),
        ErrType(Rc<str>),
        Error(Rc<ZError>),
        Module(Rc<str>),
        /// A method of a str, a list or a dict, read but not yet called.
        Method(Rc<(V, String)>),
        /// A method of a class, read from an object.
        Bound(Rc<(V, Rc<Func>)>),
        /// Several values from one function, as `return a, b` gives them, until
        /// `let a, b = ...` takes them apart (decision 57).
        Several(Rc<Vec<V>>),
    }

    pub type Cell = Rc<RefCell<V>>;

    pub fn cell(v: V) -> Cell {
        Rc::new(RefCell::new(v))
    }

    /// The value in a cell, without holding on to it.
    pub fn get(c: &Cell) -> V {
        c.borrow().clone()
    }

    pub fn set(c: &Cell, v: V) {
        *c.borrow_mut() = v;
    }

    pub fn s(text: &str) -> V {
        V::Str(Rc::from(text))
    }

    fn st(text: String) -> V {
        V::Str(Rc::from(text))
    }

    fn q(v: f64, unit: &str) -> V {
        V::Quantity(v, Rc::from(unit))
    }

    fn new_list(items: Vec<V>) -> V {
        V::List(Rc::new(RefCell::new(items)))
    }

    /// A key of a dict: an int or a str.
    #[derive(Clone, PartialEq, Eq, Hash)]
    pub enum Key {
        Int(i64),
        Str(Rc<str>),
    }

    /// A dict: its keys in the order they were put in, and their values.
    #[derive(Default)]
    pub struct Dict {
        pub keys: Vec<V>,
        pub map: HashMap<Key, V>,
    }

    pub type Body = Rc<dyn Fn(Vec<V>) -> R>;
    pub type BuiltinBody = Rc<dyn Fn(Vec<V>, usize) -> R>;

    pub struct Func {
        pub name: String,
        pub parameters: Vec<String>,
        pub defaults: Vec<bool>,
        pub body: Option<Body>,
        /// A function that comes with Zeno: its full name, and its body.
        pub builtin: String,
        pub bbody: Option<BuiltinBody>,
    }

    pub struct Class {
        pub name: String,
        pub fields: Vec<String>,
        pub defaults: Vec<V>,
        pub methods: Vec<(String, Rc<Func>)>,
    }

    impl Class {
        fn method(&self, name: &str) -> Option<Rc<Func>> {
            self.methods.iter().find(|(n, _)| n == name).map(|(_, f)| f.clone())
        }
    }

    pub struct Instance {
        pub cls: Rc<Class>,
        pub fields: RefCell<Vec<V>>,
    }

    pub struct Enum {
        pub name: String,
        pub cases: Vec<String>,
        pub fields: Vec<Vec<String>>,
    }

    pub struct Case {
        pub def: Rc<Enum>,
        pub index: usize,
        pub fields: Vec<V>,
    }

    // A long chain of objects (a linked list of a million nodes) is let
    // go link by link. Left to Rust, each link would let the next one go
    // from inside its own letting go, a million calls deep, which needs
    // more stack than a WebAssembly host gives. So an object or a case
    // that is let go hands what it held to a list, and one loop works
    // through the list.
    thread_local! {
        static LETTING_GO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static HELD: RefCell<Vec<V>> = const { RefCell::new(Vec::new()) };
    }

    fn let_go(values: Vec<V>) {
        if values.iter().all(|v| !matches!(v, V::List(_) | V::Dict(_) | V::Instance(_) | V::Case(_) | V::Error(_))) {
            return;
        }
        // At the end of the program the list may be gone already: then
        // Rust lets the values go its own way.
        let Ok(running) = HELD.try_with(|held| {
            held.borrow_mut().extend(values);
            LETTING_GO.with(|flag| flag.replace(true))
        }) else {
            return;
        };
        if running {
            return;
        }
        while let Some(value) = HELD.with(|held| held.borrow_mut().pop()) {
            match value {
                // A list or a dict nothing else holds: its items join the list.
                V::List(list) => {
                    if let Ok(items) = Rc::try_unwrap(list) {
                        HELD.with(|held| held.borrow_mut().extend(items.into_inner()));
                    }
                }
                V::Dict(dict) => {
                    if let Ok(dict) = Rc::try_unwrap(dict) {
                        let dict = dict.into_inner();
                        HELD.with(|held| held.borrow_mut().extend(dict.map.into_values()));
                    }
                }
                // An object or a case hands over what it held, below.
                other => drop(other),
            }
        }
        LETTING_GO.with(|flag| flag.set(false));
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            let_go(std::mem::take(self.fields.get_mut()));
        }
    }

    impl Drop for Case {
        fn drop(&mut self) {
            let_go(std::mem::take(&mut self.fields));
        }
    }

    pub struct ZError {
        pub kind: String,
        pub message: String,
        /// The object raised, if it was one of a class.
        pub obj: V,
    }

    /// An error the program may catch, on its way to a `try`.
    #[derive(Clone)]
    pub struct Raise {
        pub value: Rc<ZError>,
        pub line: usize,
    }

    pub enum Exc {
        Raise(Raise),
        /// A mistake in the program, which nothing catches.
        Fatal(String, usize),
    }

    pub type R = Result<V, Exc>;
    pub type RB = Result<bool, Exc>;

    /// How a `try` body ended: on to the next statement, or leaving by
    /// `break`, `continue` or `return`, which the code after it does.
    pub enum Ctl {
        Next,
        Break,
        Continue,
        Return(V),
    }

    pub fn fatal<T>(message: impl Into<String>, line: usize) -> Result<T, Exc> {
        Err(Exc::Fatal(message.into(), line))
    }

    fn raise_error<T>(kind: &str, message: impl Into<String>, line: usize) -> Result<T, Exc> {
        Err(Exc::Raise(Raise { value: Rc::new(ZError { kind: kind.to_string(), message: message.into(), obj: V::None }), line }))
    }

    fn overflow<T>(line: usize) -> Result<T, Exc> {
        raise_error("OverflowError", "the result is too big for an int", line)
    }

    fn by_zero<T>(line: usize) -> Result<T, Exc> {
        raise_error("ZeroDivisionError", "division by zero", line)
    }

    fn is_number(v: &V) -> bool {
        matches!(v, V::Int(_) | V::Float(_) | V::F32(_) | V::U64(_))
    }

    fn number(v: &V) -> f64 {
        match v {
            V::Int(n) => *n as f64,
            V::Float(x) => *x,
            V::F32(x) => *x as f64,
            V::U64(x) => *x as f64,
            _ => 0.0,
        }
    }

    /// An f32, or an int as a float and then the nearest f32 (41.2).
    fn single(v: &V) -> Option<f32> {
        match v {
            V::F32(x) => Some(*x),
            V::Int(n) => Some(*n as f64 as f32),
            _ => None,
        }
    }

    /// True when some value is an f32 and every one is an f32 or an int.
    fn all_single(values: &[V]) -> bool {
        values.iter().any(|v| matches!(v, V::F32(_))) && values.iter().all(|v| matches!(v, V::F32(_) | V::Int(_)))
    }

    /// The float with the fewest digits that reads back as this f32.
    fn f32_shown(x: f32) -> f64 {
        if x.is_nan() || x == 0.0 || x.is_infinite() {
            return x as f64;
        }
        for digits in 1..=9 {
            let d: f64 = format!("{:.*e}", digits - 1, x as f64).parse().unwrap_or(0.0);
            if d as f32 == x {
                return d;
            }
        }
        x as f64
    }

    // ---- u64 (decision 42): a Rust u64.

    const U64_TOO_BIG: &str = "the result is too big for a u64";

    fn is_u64(a: &V, b: &V) -> bool {
        matches!(a, V::U64(_)) || matches!(b, V::U64(_))
    }

    /// A u64, or an int of 0 or more, as a u64.
    fn unsigned(v: &V) -> Option<u64> {
        match v {
            V::U64(x) => Some(*x),
            V::Int(n) if *n >= 0 => Some(*n as u64),
            _ => None,
        }
    }

    fn u64_shift_count(n: &V, line: usize) -> Result<u32, Exc> {
        let (ok, text) = match n {
            V::U64(c) => (*c < 64, c.to_string()),
            V::Int(c) => ((0..64).contains(c), c.to_string()),
            _ => (true, String::new()),
        };
        if !ok {
            return raise_error("ValueError", format!("cannot shift by {text}. A u64 has 64 bits, so a shift is 0 to 63"), line);
        }
        Ok(match n {
            V::U64(c) => *c as u32,
            V::Int(c) => *c as u32,
            _ => 0,
        })
    }

    /// `a op b` where one side is a u64 (decision 42).
    fn u64_op(op: &str, a: &V, b: &V, line: usize) -> R {
        if let (V::U64(x), V::Int(_) | V::U64(_), "<<" | ">>") = (a, b, op) {
            let n = u64_shift_count(b, line)?;
            if op == ">>" {
                return Ok(V::U64(x >> n));
            }
            if (x << n) >> n != *x {
                return raise_error("OverflowError", U64_TOO_BIG, line);
            }
            return Ok(V::U64(x << n));
        }
        let (Some(x), Some(y)) = (unsigned(a), unsigned(b)) else {
            return fatal(format!("`{op}` between {} and {} would have to choose between a u64 and something else. Write `u64( )` or `int( )` around one of them", type_name(a), type_name(b)), line);
        };
        let too_big = || raise_error("OverflowError", U64_TOO_BIG, line);
        match op {
            "+" => x.checked_add(y).map(V::U64).map_or_else(too_big, Ok),
            "-" => match x.checked_sub(y) {
                Some(r) => Ok(V::U64(r)),
                None => raise_error("OverflowError", "the result is below 0, and a u64 never is", line),
            },
            "*" => x.checked_mul(y).map(V::U64).map_or_else(too_big, Ok),
            "/" | "//" | "%" if y == 0 => by_zero(line),
            "/" => Ok(V::Float(x as f64 / y as f64)),
            "//" => Ok(V::U64(x / y)),
            "%" => Ok(V::U64(x % y)),
            "**" => {
                if y == 0 {
                    return Ok(V::U64(1));
                }
                if x < 2 {
                    return Ok(V::U64(x));
                }
                let mut r: u64 = 1;
                for _ in 0..y {
                    r = match r.checked_mul(x) {
                        Some(r) => r,
                        None => return too_big(),
                    };
                }
                Ok(V::U64(r))
            }
            "&" => Ok(V::U64(x & y)),
            "|" => Ok(V::U64(x | y)),
            _ => Ok(V::U64(x ^ y)),
        }
    }

    pub fn b_u64(args: Vec<V>, line: usize) -> R {
        arity("u64", &args, 1, 1, line)?;
        match &args[0] {
            V::U64(x) => Ok(V::U64(*x)),
            V::Int(n) if *n < 0 => raise_error("OverflowError", format!("{n} does not fit in a u64 (0 to 18446744073709551615)"), line),
            V::Int(n) => Ok(V::U64(*n as u64)),
            other => wrong_type("u64", "a whole number", other, line),
        }
    }

    /// `u64.wrap(n)`: the 64 bits of an int read with no sign (42.6).
    pub fn b_u64__of(args: Vec<V>, line: usize) -> R {
        arity("u64.wrap", &args, 1, 1, line)?;
        match &args[0] {
            V::U64(x) => Ok(V::U64(*x)),
            V::Int(n) => Ok(V::U64(*n as u64)),
            other => wrong_type("u64.wrap", "a whole number", other, line),
        }
    }

    /// `u64.wrap(a + b)`: the answer cut to 64 bits (42.6).
    fn u64_wrapping(op: &str, args: Vec<V>, line: usize) -> R {
        arity("u64.wrap", &args, 2, 2, line)?;
        let x = unsigned(&args[0]);
        if let (Some(x), V::Int(_) | V::U64(_), "<<") = (x, &args[1], op) {
            return Ok(V::U64(x << u64_shift_count(&args[1], line)?));
        }
        let Some(x) = x else {
            return wrong_type("u64.wrap", "u64s", &args[0], line);
        };
        let Some(y) = unsigned(&args[1]) else {
            return wrong_type("u64.wrap", "u64s", &args[1], line);
        };
        Ok(V::U64(match op {
            "+" => x.wrapping_add(y),
            "-" => x.wrapping_sub(y),
            _ => x.wrapping_mul(y),
        }))
    }

    pub fn b_u64__add(args: Vec<V>, line: usize) -> R {
        u64_wrapping("+", args, line)
    }

    pub fn b_u64__sub(args: Vec<V>, line: usize) -> R {
        u64_wrapping("-", args, line)
    }

    pub fn b_u64__mul(args: Vec<V>, line: usize) -> R {
        u64_wrapping("*", args, line)
    }

    pub fn b_u64__shl(args: Vec<V>, line: usize) -> R {
        u64_wrapping("<<", args, line)
    }

    fn f32_op(op: &str, a: &V, b: &V, line: usize) -> R {
        let (Some(x), Some(y)) = (single(a), single(b)) else {
            if matches!(a, V::Float(_)) || matches!(b, V::Float(_)) {
                return fatal(format!("`{op}` between {} and {} would have to choose between 32 bits and 64. Write `f32( )` around the float to work in 32, or `float( )` around the f32 to work in 64", type_name(a), type_name(b)), line);
            }
            return mismatch(op, a, b, line);
        };
        let (wx, wy) = (V::Float(x as f64), V::Float(y as f64));
        let r = match op {
            "+" => x + y,
            "-" => x - y,
            "*" => x * y,
            _ => {
                let wide = match op {
                    "/" => div(wx, wy, line)?,
                    "//" => floordiv(wx, wy, line)?,
                    "%" => r#mod(wx, wy, line)?,
                    _ => pow(wx, wy, line)?,
                };
                number(&wide) as f32
            }
        };
        Ok(V::F32(r))
    }

    pub fn type_name(v: &V) -> String {
        match v {
            V::Int(_) => "an int".into(),
            V::Float(_) => "a float".into(),
            V::F32(_) => "an f32".into(),
            V::U64(_) => "a u64".into(),
            V::Str(_) => "a str".into(),
            V::Bool(_) => "a bool".into(),
            V::None => "none".into(),
            V::Missing => "a value".into(),
            V::Quantity(_, unit) => format!("a quantity in {unit}"),
            V::List(_) => "a list".into(),
            V::Dict(_) => "a dict".into(),
            V::Func(_) | V::Method(_) | V::Bound(_) => "a function".into(),
            V::Class(_) => "a class".into(),
            V::Instance(o) => format!("a {}", o.cls.name),
            V::Enum(_) => "an enum".into(),
            V::Ctor(..) => "an enum case".into(),
            V::Case(c) => format!("a {}", c.def.name),
            V::ErrType(_) => "an error type".into(),
            V::Error(e) => format!("a {}", e.kind),
            V::Module(_) => "a module".into(),
            V::Several(values) => format!("{} values", values.len()),
        }
    }

    /// `Point`, not `shapes.Point`; `Stack`, not `Stack[int]`.
    fn shown_name(name: &str) -> String {
        let name = name.split('[').next().unwrap_or(name);
        name.rsplit('.').next().unwrap_or(name).to_string()
    }

    // ------------------------------------------------------------ Text

    fn text_length(s: &str) -> usize {
        if s.is_ascii() { s.len() } else { s.chars().count() }
    }

    fn sub_text(s: &str, from: usize, to: usize) -> String {
        if s.is_ascii() { s[from..to].to_string() } else { s.chars().skip(from).take(to - from).collect() }
    }

    // --------------------------------------------------------- Showing

    /// A float as text: the fewest digits that name this float and no other,
    /// with `.0` on a whole one, and an exponent below 0.0001 and from 1e16.
///
    /// Now and then two texts of that length are equally near the float:
    /// 1e15 + 0.25 lies exactly between `...0.2` and `...0.3`. Then the one
    /// that ends in an even digit is taken, as C, JavaScript, Python, Java
    /// and C# take it. Rust alone takes the upper one, so
    /// that case is put right here. It needs 16 digits or more to happen.
    fn float_text(x: f64) -> String {
        let shown = format!("{x:?}");
        if shown.len() < 16 || !x.is_finite() {
            return shown;
        }
        let shortest = format!("{x:e}");
        let digits = shortest.bytes().take_while(|b| *b != b'e').filter(u8::is_ascii_digit).count();
        // The same number of digits, rounded from the exact value, to even.
        let even = format!("{:.*e}", digits - 1, x);
        if even == shortest || even.parse::<f64>() != Ok(x) {
            return shown;
        }
        // The two differ in the last digit alone: Rust's is one more, and odd.
        let last = even.bytes().take_while(|b| *b != b'e').filter(u8::is_ascii_digit).last().unwrap_or(b'0');
        let end = shown.find('e').unwrap_or(shown.len());
        match shown[..end].rfind(|c: char| c.is_ascii_digit()) {
            Some(at) if shown.as_bytes()[at] == last + 1 => format!("{}{}{}", &shown[..at], last as char, &shown[at + 1..]),
            _ => shown,
        }
    }

    /// A text in its quotes: `"`, `\\`, new line, carriage return and
    /// tab as a program writes them, any other control character as its
    /// code, and every other character as it is.
    fn quoted(s: &str) -> String {
        let mut b = String::with_capacity(s.len() + 2);
        b.push('"');
        for c in s.chars() {
            match c {
                '"' => b.push_str("\\\""),
                '\\' => b.push_str("\\\\"),
                '\n' => b.push_str("\\n"),
                '\r' => b.push_str("\\r"),
                '\t' => b.push_str("\\t"),
                '\0' => b.push_str("\\0"),
                c if (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)) => b.push_str(&format!("\\u{{{:x}}}", c as u32)),
                c => b.push(c),
            }
        }
        b.push('"');
        b
    }

    /// How `print`, `str()` and f-strings show a value.
    pub fn show(v: &V) -> String {
        match v {
            V::Str(s) => s.to_string(),
            V::Int(n) => n.to_string(),
            V::Float(x) => float_text(*x),
            V::F32(x) => float_text(f32_shown(*x)),
            V::U64(x) => x.to_string(),
            V::Bool(b) => b.to_string(),
            V::None => "none".into(),
            V::Missing => "?".into(),
            V::List(l) => {
                let parts: Vec<String> = l.borrow().iter().map(repr).collect();
                format!("[{}]", parts.join(", "))
            }
            V::Dict(d) => {
                let d = d.borrow();
                if d.keys.is_empty() {
                    return "[:]".into();
                }
                let parts: Vec<String> = d.keys.iter().map(|k| format!("{}: {}", repr(k), repr(&d.map[&key_of(k)]))).collect();
                format!("[{}]", parts.join(", "))
            }
            V::Quantity(x, unit) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{} {unit}", *x as i64),
            V::Quantity(x, unit) => format!("{} {unit}", float_text(*x)),
            V::Instance(o) => {
                let fields = o.fields.borrow();
                let parts: Vec<String> = o.cls.fields.iter().zip(fields.iter()).map(|(n, v)| format!("{n}={}", repr(v))).collect();
                format!("{}({})", shown_name(&o.cls.name), parts.join(", "))
            }
            V::Case(c) => {
                let name = shown_name(&c.def.cases[c.index]);
                if c.fields.is_empty() {
                    return name;
                }
                let parts: Vec<String> = c.fields.iter().map(repr).collect();
                format!("{name}({})", parts.join(", "))
            }
            V::Error(e) => e.message.clone(),
            V::Method(m) => format!("<function {}>", m.1),
            V::Bound(b) => format!("<function {}>", b.1.name),
            V::Class(c) => format!("<class {}>", c.name),
            V::Enum(e) => format!("<enum {}>", e.name),
            V::Ctor(e, i) => format!("<case {}>", e.cases[*i]),
            V::ErrType(k) => format!("<error {k}>"),
            V::Module(m) => format!("<module {m}>"),
            V::Several(values) => {
                let parts: Vec<String> = values.iter().map(show).collect();
                format!("<{} values: {}>", parts.len(), parts.join(", "))
            }
            V::Func(f) if f.name.is_empty() => "<function>".into(),
            V::Func(f) => format!("<function {}>", f.name),
        }
    }

    fn repr(v: &V) -> String {
        match v {
            V::Str(s) => quoted(s),
            _ => show(v),
        }
    }

    // ----------------------------------------------------------- Units

    fn unit_of(name: &str) -> Option<(&'static str, f64)> {
        Some(match name {
            "ns" => ("time", 1.0),
            "us" => ("time", 1e3),
            "ms" => ("time", 1e6),
            "s" => ("time", 1e9),
            "min" => ("time", 60e9),
            "h" => ("time", 3600e9),
            "Hz" => ("frequency", 1.0),
            "kHz" => ("frequency", 1e3),
            "MHz" => ("frequency", 1e6),
            "uV" => ("voltage", 1.0),
            "mV" => ("voltage", 1e3),
            "V" => ("voltage", 1e6),
            "kV" => ("voltage", 1e9),
            "uA" => ("current", 1.0),
            "mA" => ("current", 1e3),
            "A" => ("current", 1e6),
            "ohm" => ("resistance", 1.0),
            "kohm" => ("resistance", 1e3),
            "Mohm" => ("resistance", 1e6),
            "mW" => ("power", 1.0),
            "W" => ("power", 1e3),
            "kW" => ("power", 1e6),
            "J" => ("energy", 1.0),
            "Wh" => ("energy", 3600.0),
            "kWh" => ("energy", 3600e3),
            "degC" => ("temperature in Celsius", 1.0),
            "K" => ("temperature in kelvin", 1.0),
            "mm" => ("length", 1.0),
            "cm" => ("length", 10.0),
            "m" => ("length", 1e3),
            "km" => ("length", 1e6),
            "mg" => ("mass", 1.0),
            "g" => ("mass", 1e3),
            "kg" => ("mass", 1e6),
            "deg" => ("angle in degrees", 1.0),
            "rad" => ("angle in radians", 1.0),
            "rpm" => ("rotation", 1.0),
            _ => return None,
        })
    }

    /// How to turn a number in one unit into another: (times, over).
    fn convert(from: &str, to: &str) -> Option<(f64, f64)> {
        if from == "deg" && to == "rad" {
            return Some((std::f64::consts::PI / 180.0, 1.0));
        }
        if from == "rad" && to == "deg" {
            return Some((180.0 / std::f64::consts::PI, 1.0));
        }
        let (fk, fs) = unit_of(from)?;
        let (tk, ts) = unit_of(to)?;
        if fk != tk {
            return None;
        }
        Some(if fs >= ts { (fs / ts, 1.0) } else { (1.0, ts / fs) })
    }

    fn apply(scale: (f64, f64), x: f64) -> f64 {
        x * scale.0 / scale.1
    }

    /// The unit two quantities meet in, the finer one.
    fn common(a: &str, b: &str) -> Option<String> {
        let (ak, asz) = unit_of(a)?;
        let (bk, bsz) = unit_of(b)?;
        if ak != bk {
            return None;
        }
        Some(if asz <= bsz { a.to_string() } else { b.to_string() })
    }

    fn in_unit(x: f64, from: &str, to: &str) -> f64 {
        apply(convert(from, to).unwrap_or((1.0, 1.0)), x)
    }

    /// What `*` and `/` make of two quantities (decision 43): the
    /// operator, what each side measures ("" for a plain number), and what
    /// the answer measures ("" for a plain number). The answer is in the
    /// unit of its kind with no prefix.
    const MADE: &[(&str, &str, &str, &str)] = &[
        ("*", "voltage", "current", "power"),
        ("*", "current", "voltage", "power"),
        ("*", "current", "resistance", "voltage"),
        ("*", "resistance", "current", "voltage"),
        ("*", "power", "time", "energy"),
        ("*", "time", "power", "energy"),
        ("*", "time", "frequency", ""),
        ("*", "frequency", "time", ""),
        ("/", "power", "voltage", "current"),
        ("/", "power", "current", "voltage"),
        ("/", "voltage", "current", "resistance"),
        ("/", "voltage", "resistance", "current"),
        ("/", "energy", "time", "power"),
        ("/", "energy", "power", "time"),
        ("/", "", "time", "frequency"),
        ("/", "", "frequency", "time"),
    ];

    fn plain(kind: &str) -> &'static str {
        match kind {
            "time" => "s",
            "frequency" => "Hz",
            "voltage" => "V",
            "current" => "A",
            "resistance" => "ohm",
            "power" => "W",
            _ => "J",
        }
    }

    /// `a op b` for two quantities, or a number over a quantity.
    fn made(op: &str, a: &V, b: &V, line: usize) -> R {
        let V::Quantity(y, right_unit) = b else {
            return mismatch(op, a, b, line);
        };
        let left = match a {
            V::Quantity(_, unit) => unit_of(unit).map_or("", |(kind, _)| kind),
            _ => "",
        };
        let right = unit_of(right_unit).map_or("", |(kind, _)| kind);
        let Some((_, _, _, kind)) = MADE.iter().find(|(o, l, r, _)| *o == op && *l == left && *r == right) else {
            return mismatch(op, a, b, line);
        };
        let x = match a {
            V::Quantity(x, unit) => in_unit(*x, unit, plain(left)),
            _ if is_number(a) => number(a),
            _ => return mismatch(op, a, b, line),
        };
        if op == "/" && *y == 0.0 {
            return by_zero(line);
        }
        let y = in_unit(*y, right_unit, plain(right));
        let r = if op == "*" { x * y } else { x / y };
        Ok(if kind.is_empty() { V::Float(r) } else { q(r, plain(kind)) })
    }

    pub fn unit(v: V, name: &str, line: usize) -> R {
        if unit_of(name).is_none() {
            return fatal(format!("`{name}` is not a unit Zeno knows. Some that it does: s, ms, us, Hz, V, mV, A, mA, W, degC"), line);
        }
        if !is_number(&v) {
            return fatal("a unit can only follow a number", line);
        }
        Ok(q(number(&v), name))
    }

    // -------------------------------------------------------- Equality

    fn key_of(k: &V) -> Key {
        match k {
            V::Int(n) => Key::Int(*n),
            V::Str(s) => Key::Str(s.clone()),
            _ => Key::Int(0),
        }
    }

    fn equal(a: &V, b: &V) -> bool {
        match (a, b) {
            (V::Int(x), V::Int(y)) => x == y,
            (V::U64(_), _) | (_, V::U64(_)) => matches!((unsigned(a), unsigned(b)), (Some(x), Some(y)) if x == y),
            (V::F32(_), _) | (_, V::F32(_)) => match (single(a), single(b)) {
                (Some(x), Some(y)) => x == y,
                _ => is_number(a) && is_number(b) && number(a) == number(b),
            },
            (V::Int(_) | V::Float(_), V::Int(_) | V::Float(_)) => number(a) == number(b),
            (V::Str(x), V::Str(y)) => x == y,
            (V::Bool(x), V::Bool(y)) => x == y,
            (V::None, V::None) | (V::Missing, V::Missing) => true,
            // Several values are equal only to themselves (decision 57).
            (V::Several(x), V::Several(y)) => Rc::ptr_eq(x, y),
            (V::Quantity(x, u), V::Quantity(y, w)) => match common(u, w) {
                Some(c) => in_unit(*x, u, &c) == in_unit(*y, w, &c),
                None => false,
            },
            (V::List(x), V::List(y)) => {
                if Rc::ptr_eq(x, y) {
                    return true;
                }
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equal(p, q))
            }
            (V::Dict(x), V::Dict(y)) => {
                if Rc::ptr_eq(x, y) {
                    return true;
                }
                let (x, y) = (x.borrow(), y.borrow());
                x.keys.len() == y.keys.len()
                    && x.keys.iter().all(|k| match y.map.get(&key_of(k)) {
                        Some(other) => equal(&x.map[&key_of(k)], other),
                        None => false,
                    })
            }
            (V::Instance(x), V::Instance(y)) => {
                if Rc::ptr_eq(x, y) {
                    return true;
                }
                if !Rc::ptr_eq(&x.cls, &y.cls) {
                    return false;
                }
                let (p, q) = (x.fields.borrow(), y.fields.borrow());
                p.iter().zip(q.iter()).all(|(a, b)| equal(a, b))
            }
            (V::Case(x), V::Case(y)) => {
                Rc::ptr_eq(&x.def, &y.def)
                    && x.index == y.index
                    && x.fields.len() == y.fields.len()
                    && x.fields.iter().zip(y.fields.iter()).all(|(a, b)| equal(a, b))
            }
            (V::Error(x), V::Error(y)) => x.kind == y.kind && x.message == y.message,
            (V::Ctor(x, i), V::Ctor(y, j)) => Rc::ptr_eq(x, y) && i == j,
            (V::Func(x), V::Func(y)) => Rc::ptr_eq(x, y),
            (V::Class(x), V::Class(y)) => Rc::ptr_eq(x, y),
            (V::Enum(x), V::Enum(y)) => Rc::ptr_eq(x, y),
            (V::Module(x), V::Module(y)) => Rc::ptr_eq(x, y),
            (V::ErrType(x), V::ErrType(y)) => Rc::ptr_eq(x, y),
            (V::Method(x), V::Method(y)) => Rc::ptr_eq(x, y),
            (V::Bound(x), V::Bound(y)) => Rc::ptr_eq(x, y),
            _ => false,
        }
    }

    pub fn eq(a: V, b: V) -> RB {
        Ok(equal(&a, &b))
    }

    pub fn ne(a: V, b: V) -> RB {
        Ok(!equal(&a, &b))
    }

    // ------------------------------------------------------- Operators

    fn mismatch<T>(op: &str, a: &V, b: &V, line: usize) -> Result<T, Exc> {
        fatal(format!("`{op}` does not work between {} and {}", type_name(a), type_name(b)), line)
    }

    pub fn add(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("+", &a, &b, line);
        }
        if matches!(a, V::F32(_)) || matches!(b, V::F32(_)) {
            return f32_op("+", &a, &b, line);
        }
        match (&a, &b) {
            (V::Int(x), V::Int(y)) => x.checked_add(*y).map(V::Int).map_or_else(|| overflow(line), Ok),
            (V::Str(x), V::Str(y)) => Ok(st(format!("{x}{y}"))),
            (V::List(x), V::List(y)) => {
                let mut items = x.borrow().clone();
                items.extend(y.borrow().iter().cloned());
                Ok(new_list(items))
            }
            (V::Quantity(x, u), V::Quantity(y, w)) => match common(u, w) {
                Some(c) => Ok(q(in_unit(*x, u, &c) + in_unit(*y, w, &c), &c)),
                None => mismatch("+", &a, &b, line),
            },
            _ if is_number(&a) && is_number(&b) => Ok(V::Float(number(&a) + number(&b))),
            _ => mismatch("+", &a, &b, line),
        }
    }

    pub fn sub(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("-", &a, &b, line);
        }
        if matches!(a, V::F32(_)) || matches!(b, V::F32(_)) {
            return f32_op("-", &a, &b, line);
        }
        match (&a, &b) {
            (V::Int(x), V::Int(y)) => x.checked_sub(*y).map(V::Int).map_or_else(|| overflow(line), Ok),
            (V::Quantity(x, u), V::Quantity(y, w)) => match common(u, w) {
                Some(c) => Ok(q(in_unit(*x, u, &c) - in_unit(*y, w, &c), &c)),
                None => mismatch("-", &a, &b, line),
            },
            _ if is_number(&a) && is_number(&b) => Ok(V::Float(number(&a) - number(&b))),
            _ => mismatch("-", &a, &b, line),
        }
    }

    pub fn mul(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("*", &a, &b, line);
        }
        if matches!(a, V::F32(_)) || matches!(b, V::F32(_)) {
            return f32_op("*", &a, &b, line);
        }
        match (&a, &b) {
            (V::List(x), V::Int(times)) => {
                if *times < 0 {
                    return raise_error("ValueError", format!("`*` repeats a list 0 or more times, and this is {times}"), line);
                }
                let x = x.borrow();
                let mut items = Vec::new();
                for _ in 0..*times {
                    items.extend(x.iter().cloned());
                }
                Ok(new_list(items))
            }
            (V::Int(x), V::Int(y)) => x.checked_mul(*y).map(V::Int).map_or_else(|| overflow(line), Ok),
            (V::Quantity(..), V::Quantity(..)) => made("*", &a, &b, line),
            (V::Quantity(x, u), other) | (other, V::Quantity(x, u)) => {
                if !is_number(other) {
                    return mismatch("*", &a, &b, line);
                }
                Ok(V::Quantity(x * number(other), u.clone()))
            }
            _ if is_number(&a) && is_number(&b) => Ok(V::Float(number(&a) * number(&b))),
            _ => mismatch("*", &a, &b, line),
        }
    }

    pub fn div(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("/", &a, &b, line);
        }
        if matches!(a, V::F32(_)) || matches!(b, V::F32(_)) {
            return f32_op("/", &a, &b, line);
        }
        if let V::Quantity(x, u) = &a {
            match &b {
                V::Quantity(y, _) if *y == 0.0 => return by_zero(line),
                V::Float(y) if *y == 0.0 => return by_zero(line),
                V::Int(0) => return by_zero(line),
                _ => {}
            }
            if let V::Quantity(y, w) = &b {
                return match common(u, w) {
                    Some(c) => Ok(V::Float(in_unit(*x, u, &c) / in_unit(*y, w, &c))),
                    None => made("/", &a, &b, line),
                };
            }
            if !is_number(&b) {
                return mismatch("/", &a, &b, line);
            }
            return Ok(V::Quantity(x / number(&b), u.clone()));
        }
        if let V::Quantity(..) = &b {
            return made("/", &a, &b, line);
        }
        if !is_number(&a) || !is_number(&b) {
            return mismatch("/", &a, &b, line);
        }
        if number(&b) == 0.0 {
            return by_zero(line);
        }
        Ok(V::Float(number(&a) / number(&b)))
    }

    pub fn floordiv(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("//", &a, &b, line);
        }
        if matches!(a, V::F32(_)) || matches!(b, V::F32(_)) {
            return f32_op("//", &a, &b, line);
        }
        if let (V::Int(x), V::Int(y)) = (&a, &b) {
            let (x, y) = (*x, *y);
            if y == 0 {
                return by_zero(line);
            }
            if x == i64::MIN && y == -1 {
                return overflow(line);
            }
            let mut q = x / y;
            if x % y != 0 && (x < 0) != (y < 0) {
                q -= 1;
            }
            return Ok(V::Int(q));
        }
        if !is_number(&a) || !is_number(&b) {
            return mismatch("//", &a, &b, line);
        }
        if number(&b) == 0.0 {
            return by_zero(line);
        }
        Ok(V::Float((number(&a) / number(&b)).floor()))
    }

    pub fn r#mod(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("%", &a, &b, line);
        }
        if matches!(a, V::F32(_)) || matches!(b, V::F32(_)) {
            return f32_op("%", &a, &b, line);
        }
        if let (V::Int(x), V::Int(y)) = (&a, &b) {
            let (x, y) = (*x, *y);
            if y == 0 {
                return by_zero(line);
            }
            if y == -1 {
                return Ok(V::Int(0));
            }
            let mut r = x % y;
            if r != 0 && (r < 0) != (y < 0) {
                r += y;
            }
            return Ok(V::Int(r));
        }
        if !is_number(&a) || !is_number(&b) {
            return mismatch("%", &a, &b, line);
        }
        let (fx, fy) = (number(&a), number(&b));
        if fy == 0.0 {
            return by_zero(line);
        }
        let fq = fx / fy;
        if fq.is_nan() || fq.is_infinite() {
            return Ok(V::Float(fx - fy * fq));
        }
        Ok(V::Float(fx - fy * fq.floor()))
    }

    pub fn pow(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("**", &a, &b, line);
        }
        if matches!(a, V::F32(_)) || matches!(b, V::F32(_)) {
            return f32_op("**", &a, &b, line);
        }
        if let (V::Int(x), V::Int(n)) = (&a, &b) {
            let (x, n) = (*x, *n);
            if n < 0 {
                return raise_error("ValueError", "a whole number cannot be raised to a negative power. Use a float, as in `2.0 ** -1`", line);
            }
            if n > 4294967295 {
                return overflow(line);
            }
            if x == 0 || x == 1 {
                return Ok(V::Int(if n == 0 { 1 } else { x }));
            }
            if x == -1 {
                return Ok(V::Int(if n % 2 == 0 { 1 } else { -1 }));
            }
            if n >= 64 {
                return overflow(line);
            }
            let mut r: i64 = 1;
            for _ in 0..n {
                r = match r.checked_mul(x) {
                    Some(r) => r,
                    None => return overflow(line),
                };
            }
            return Ok(V::Int(r));
        }
        if !is_number(&a) || !is_number(&b) {
            return mismatch("**", &a, &b, line);
        }
        let (fx, fy) = (number(&a), number(&b));
        if fx == 1.0 || fy == 0.0 {
            return Ok(V::Float(1.0));
        }
        Ok(V::Float(fx.powf(fy)))
    }

    fn whole(op: &str, a: &V, b: &V, line: usize) -> Result<(i64, i64), Exc> {
        if let (V::Int(x), V::Int(y)) = (a, b) {
            return Ok((*x, *y));
        }
        let mut hint = "";
        if let (V::Bool(_), V::Bool(_)) = (a, b) {
            if op == "&" {
                hint = ". For two bools Zeno spells this `and`";
            }
            if op == "|" {
                hint = ". For two bools Zeno spells this `or`";
            }
        }
        fatal(format!("`{op}` works on the bits of whole numbers, and these are {} and {}{hint}", type_name(a), type_name(b)), line)
    }

    pub fn bitand(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("&", &a, &b, line);
        }
        let (x, y) = whole("&", &a, &b, line)?;
        Ok(V::Int(x & y))
    }

    pub fn bitor(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("|", &a, &b, line);
        }
        let (x, y) = whole("|", &a, &b, line)?;
        Ok(V::Int(x | y))
    }

    pub fn bitxor(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("^", &a, &b, line);
        }
        let (x, y) = whole("^", &a, &b, line)?;
        Ok(V::Int(x ^ y))
    }

    fn shift(n: i64, line: usize) -> Result<(), Exc> {
        if !(0..64).contains(&n) {
            return raise_error("ValueError", format!("cannot shift by {n}. An int has 64 bits, so a shift is 0 to 63"), line);
        }
        Ok(())
    }

    pub fn shl(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op("<<", &a, &b, line);
        }
        let (x, n) = whole("<<", &a, &b, line)?;
        shift(n, line)?;
        let s = ((x as u64) << n) as i64;
        if s >> n != x {
            return overflow(line);
        }
        Ok(V::Int(s))
    }

    pub fn shr(a: V, b: V, line: usize) -> R {
        if is_u64(&a, &b) {
            return u64_op(">>", &a, &b, line);
        }
        let (x, n) = whole(">>", &a, &b, line)?;
        shift(n, line)?;
        Ok(V::Int(x >> n))
    }

    pub fn neg(v: V, line: usize) -> R {
        match v {
            V::Int(i64::MIN) => overflow(line),
            V::Int(x) => Ok(V::Int(-x)),
            V::Float(x) => Ok(V::Float(-x)),
            V::F32(x) => Ok(V::F32(-x)),
            V::U64(_) => fatal("`-` does not work on a u64, which is never below 0. Make it an int first with `int( )`", line),
            V::Quantity(x, u) => Ok(V::Quantity(-x, u)),
            other => fatal(format!("`-` does not work on {}", type_name(&other)), line),
        }
    }

    pub fn bitnot(v: V, line: usize) -> R {
        match v {
            V::Int(x) => Ok(V::Int(!x)),
            V::U64(x) => Ok(V::U64(!x)),
            other => fatal(format!("`~` works on the bits of a whole number, and this is {}", type_name(&other)), line),
        }
    }

    /// -1, 0 or 1, or 2 when the two cannot be ordered (a NaN).
    fn order(op: &str, a: &V, b: &V, line: usize) -> Result<i32, Exc> {
        if is_u64(a, b) {
            return match (unsigned(a), unsigned(b)) {
                (Some(x), Some(y)) => Ok(x.cmp(&y) as i32),
                _ => {
                    u64_op(op, a, b, line)?;
                    Ok(2)
                }
            };
        }
        let (x, y) = match (a, b) {
            (V::F32(_), _) | (_, V::F32(_)) => match (single(a), single(b)) {
                (Some(x), Some(y)) => (x as f64, y as f64),
                _ => {
                    f32_op(op, a, b, line)?;
                    return Ok(2);
                }
            },
            (V::Int(x), V::Int(y)) => return Ok(x.cmp(y) as i32),
            (V::Str(x), V::Str(y)) => return Ok(x.cmp(y) as i32),
            (V::Quantity(x, u), V::Quantity(y, w)) => match common(u, w) {
                Some(c) => (in_unit(*x, u, &c), in_unit(*y, w, &c)),
                None => return mismatch(op, a, b, line),
            },
            _ if is_number(a) && is_number(b) => (number(a), number(b)),
            _ => return mismatch(op, a, b, line),
        };
        Ok(if x < y {
            -1
        } else if x > y {
            1
        } else if x == y {
            0
        } else {
            2
        })
    }

    pub fn lt(a: V, b: V, line: usize) -> RB {
        Ok(order("<", &a, &b, line)? == -1)
    }

    pub fn le(a: V, b: V, line: usize) -> RB {
        let o = order("<=", &a, &b, line)?;
        Ok(o == -1 || o == 0)
    }

    pub fn gt(a: V, b: V, line: usize) -> RB {
        Ok(order(">", &a, &b, line)? == 1)
    }

    pub fn ge(a: V, b: V, line: usize) -> RB {
        let o = order(">=", &a, &b, line)?;
        Ok(o == 1 || o == 0)
    }

    fn contains(op: &str, item: &V, container: &V, line: usize) -> RB {
        match (item, container) {
            (_, V::List(l)) => Ok(l.borrow().iter().any(|other| equal(item, other))),
            (V::Int(_) | V::Str(_), V::Dict(d)) => Ok(d.borrow().map.contains_key(&key_of(item))),
            (V::Str(s), V::Str(c)) => Ok(c.contains(&**s)),
            _ => mismatch(op, item, container, line),
        }
    }

    pub fn r#in(a: V, b: V, line: usize) -> RB {
        contains("in", &a, &b, line)
    }

    pub fn not_in(a: V, b: V, line: usize) -> RB {
        Ok(!contains("not in", &a, &b, line)?)
    }

    pub fn truth(v: V, line: usize) -> RB {
        match v {
            V::Bool(b) => Ok(b),
            other => fatal(format!("a condition must be true or false, but this is {}", type_name(&other)), line),
        }
    }

    // --------------------------------------------------- Lists and text

    fn key(v: V, line: usize) -> R {
        match v {
            V::Int(_) | V::Str(_) => Ok(v),
            other => fatal(format!("a key of a dict is a str or an int, but this is {}", type_name(&other)), line),
        }
    }

    fn dict_set(d: &mut Dict, k: V, v: V) {
        let kk = key_of(&k);
        if !d.map.contains_key(&kk) {
            d.keys.push(k);
        }
        d.map.insert(kk, v);
    }

    fn dict_remove(d: &mut Dict, k: &V) {
        let kk = key_of(k);
        d.map.remove(&kk);
        if let Some(at) = d.keys.iter().position(|x| key_of(x) == kk) {
            d.keys.remove(at);
        }
    }

    fn position(index: &V, length: usize, what: &str, line: usize) -> Result<usize, Exc> {
        let V::Int(i) = index else {
            return fatal(format!("an index must be an int, but this is {}", type_name(index)), line);
        };
        if *i < 0 || *i as u64 >= length as u64 {
            return raise_error("IndexError", format!("index {i} is out of range for a {what} of length {length}"), line);
        }
        Ok(*i as usize)
    }

    pub fn index(obj: V, i: V, line: usize) -> R {
        match &obj {
            V::List(l) => {
                let l = l.borrow();
                Ok(l[position(&i, l.len(), "list", line)?].clone())
            }
            V::Str(s) => {
                let at = position(&i, text_length(s), "str", line)?;
                Ok(st(sub_text(s, at, at + 1)))
            }
            V::Dict(d) => {
                let k = key(i.clone(), line)?;
                match d.borrow().map.get(&key_of(&k)) {
                    Some(v) => Ok(v.clone()),
                    None => raise_error("KeyError", format!("this dict has no key {}", repr(&i)), line),
                }
            }
            _ => fatal(format!("cannot take an item from {}", type_name(&obj)), line),
        }
    }

    pub fn set_index(obj: V, i: V, value: V, line: usize) -> R {
        match &obj {
            V::List(l) => {
                let at = position(&i, l.borrow().len(), "list", line)?;
                l.borrow_mut()[at] = value;
                Ok(V::None)
            }
            V::Dict(d) => {
                let k = key(i, line)?;
                dict_set(&mut d.borrow_mut(), k, value);
                Ok(V::None)
            }
            _ => fatal(format!("cannot set an item of {}", type_name(&obj)), line),
        }
    }

    pub fn items(v: V, line: usize) -> Result<Vec<V>, Exc> {
        match &v {
            V::List(l) => Ok(l.borrow().clone()),
            V::Dict(d) => Ok(d.borrow().keys.clone()),
            V::Str(s) => Ok(s.chars().map(|c| st(c.to_string())).collect()),
            _ => fatal(format!("cannot loop over {}", type_name(&v)), line),
        }
    }

    pub fn pairs(v: V, line: usize) -> Result<Vec<(V, V)>, Exc> {
        if let V::Dict(d) = &v {
            let d = d.borrow();
            return Ok(d.keys.iter().map(|k| (k.clone(), d.map[&key_of(k)].clone())).collect());
        }
        Ok(items(v, line)?.into_iter().enumerate().map(|(i, x)| (V::Int(i as i64), x)).collect())
    }

    pub fn list(values: Vec<V>) -> R {
        Ok(new_list(values))
    }

    pub fn dict(flat: Vec<V>, line: usize) -> R {
        let mut d = Dict::default();
        let mut it = flat.into_iter();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            let k = key(k, line)?;
            dict_set(&mut d, k, v);
        }
        Ok(V::Dict(Rc::new(RefCell::new(d))))
    }

    pub fn push(list: V, value: V) -> R {
        if let V::List(l) = &list {
            l.borrow_mut().push(value);
        }
        Ok(V::None)
    }

    pub fn concat(pieces: Vec<V>) -> R {
        let mut b = String::new();
        for p in &pieces {
            if let V::Str(s) = p {
                b.push_str(s);
            }
        }
        Ok(st(b))
    }

    // ----------------------------------------------- Objects and fields

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    pub fn func(name: &str, parameters: &[&str], defaults: &[bool], body: Body) -> V {
        V::Func(Rc::new(Func {
            name: name.to_string(),
            parameters: names(parameters),
            defaults: defaults.to_vec(),
            body: Some(body),
            builtin: String::new(),
            bbody: None,
        }))
    }

    pub fn class(name: &str, fields: &[&str], defaults: Vec<V>, methods: Vec<V>) -> V {
        let mut list = Vec::new();
        let mut it = methods.into_iter();
        while let (Some(V::Str(n)), Some(V::Func(f))) = (it.next(), it.next()) {
            list.push((n.to_string(), f));
        }
        V::Class(Rc::new(Class { name: name.to_string(), fields: names(fields), defaults, methods: list }))
    }

    pub fn enum_of(name: &str, cases: &[&str], fields: Vec<Vec<&str>>) -> V {
        V::Enum(Rc::new(Enum { name: name.to_string(), cases: names(cases), fields: fields.iter().map(|f| names(f)).collect() }))
    }

    pub fn case(def: V, i: usize) -> R {
        let V::Enum(e) = def else { return Ok(V::None) };
        if e.fields[i].is_empty() {
            Ok(V::Case(Rc::new(Case { def: e, index: i, fields: Vec::new() })))
        } else {
            Ok(V::Ctor(e, i))
        }
    }

    pub fn is_case(value: V, ctor: V, count: usize, line: usize) -> RB {
        let V::Ctor(def, index) = &ctor else {
            return fatal("this is not an enum case that carries values", line);
        };
        let V::Case(v) = &value else { return Ok(false) };
        if !Rc::ptr_eq(&v.def, def) || v.index != *index {
            return Ok(false);
        }
        if v.fields.len() != count {
            return fatal(format!("`{}` carries {} values, but this pattern has {count}", def.cases[*index], v.fields.len()), line);
        }
        Ok(true)
    }

    pub fn case_field(value: V, i: usize) -> R {
        match value {
            V::Case(c) => Ok(c.fields[i].clone()),
            _ => Ok(V::None),
        }
    }

    pub fn several(values: Vec<V>) -> R {
        Ok(V::Several(Rc::new(values)))
    }

    /// What place `at` of the `count` on the left of `=` is given.
    pub fn received(value: V, count: usize, at: usize, line: usize) -> R {
        let V::Several(values) = &value else {
            return fatal(format!("there are {count} places on the left of `=`, and the right gives one value, {}", type_name(&value)), line);
        };
        if values.len() != count {
            return fatal(format!("this gives {} values, and there are {count} places on the left of `=`", values.len()), line);
        }
        Ok(values[at].clone())
    }

    const WIDTHS: [(&str, i64, i64, u32, bool); 6] = [
        ("u8", 0, 255, 8, false),
        ("u16", 0, 65535, 16, false),
        ("u32", 0, 4294967295, 32, false),
        ("i8", -128, 127, 8, true),
        ("i16", -32768, 32767, 16, true),
        ("i32", -2147483648, 2147483647, 32, true),
    ];

    fn width(name: &str) -> Option<(&'static str, i64, i64, u32, bool)> {
        WIDTHS.iter().find(|w| w.0 == name).copied()
    }

    pub fn field(obj: V, name: &str, line: usize) -> R {
        match &obj {
            V::Instance(o) => {
                if let Some(at) = o.cls.fields.iter().position(|f| f == name) {
                    return Ok(o.fields.borrow()[at].clone());
                }
                if let Some(m) = o.cls.method(name) {
                    return Ok(V::Bound(Rc::new((obj.clone(), m))));
                }
                fatal(format!("`{}` has no field or method `{name}`", o.cls.name), line)
            }
            V::Case(c) => match c.def.fields[c.index].iter().position(|f| f == name) {
                Some(at) => Ok(c.fields[at].clone()),
                None => fatal(format!("`{}` has no field `{name}`", c.def.cases[c.index]), line),
            },
            V::Enum(e) => match e.cases.iter().position(|c| c == name) {
                Some(at) => case(obj.clone(), at),
                None => fatal(format!("enum `{}` has no case `{name}`", e.name), line),
            },
            V::Module(m) => builtin(&format!("{m}.{name}"), line),
            V::Quantity(x, u) if unit_of(name).is_some() => match convert(u, name) {
                Some(scale) => Ok(q(apply(scale, *x), name)),
                None => fatal(format!("a quantity in {u} cannot be turned into {name}. They measure different things"), line),
            },
            V::Error(e) if name == "message" => Ok(s(&e.message)),
            V::Error(e) if name == "kind" => Ok(s(&e.kind)),
            V::Func(f) if !f.builtin.is_empty() && width(&f.builtin).is_some() && name == "wrap" => {
                builtin(&format!("{}.wrap", f.builtin), line)
            }
            _ if has_method(&obj, name) => Ok(V::Method(Rc::new((obj.clone(), name.to_string())))),
            _ => fatal(format!("{} has no field or method `{name}`", type_name(&obj)), line),
        }
    }

    pub fn set_field(obj: V, name: &str, value: V, line: usize) -> R {
        match &obj {
            V::Instance(o) => match o.cls.fields.iter().position(|f| f == name) {
                Some(at) => {
                    o.fields.borrow_mut()[at] = value;
                    Ok(V::None)
                }
                None => fatal(format!("`{}` has no field `{name}`", o.cls.name), line),
            },
            _ => fatal(format!("cannot set a field on {}", type_name(&obj)), line),
        }
    }

    // ------------------------------------------------------------ Calls

    fn counted(n: usize, word: &str) -> String {
        if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
    }

    fn were(n: usize) -> String {
        if n == 1 { "1 was".into() } else { format!("{n} were") }
    }

    fn bind(what: &str, names: &[String], positional: Vec<V>, named_names: &[&str], named: Vec<V>, line: usize) -> Result<Vec<V>, Exc> {
        if positional.len() > names.len() {
            return fatal(format!("`{what}` takes {}, but {} given", counted(names.len(), "argument"), were(positional.len())), line);
        }
        let given = positional.len();
        let mut slots = positional;
        slots.resize(names.len(), V::Missing);
        for (n, v) in named_names.iter().zip(named) {
            let Some(at) = names.iter().position(|p| p == n) else {
                return fatal(format!("`{what}` has no argument named `{n}`"), line);
            };
            if !matches!(slots[at], V::Missing) {
                return fatal(format!("`{what}` was given `{n}` twice"), line);
            }
            slots[at] = v;
        }
        let _ = given;
        Ok(slots)
    }

    thread_local! {
        // How many calls are nested now. `zeno run` stops at 1000.
        static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn enter(f: &Func, line: usize) -> Result<(), Exc> {
        let depth = DEPTH.with(|d| d.get());
        if depth >= 1000 {
            let name = if f.name.is_empty() { "function" } else { &f.name };
            return fatal(format!("more than 1000 calls are nested. Is `{name}` calling itself forever?"), line);
        }
        DEPTH.with(|d| d.set(depth + 1));
        Ok(())
    }

    fn leave() {
        DEPTH.with(|d| d.set(d.get() - 1));
    }

    /// A call of a function of the program known where it is written,
    /// with its arguments lined up already.
    pub fn direct(f: V, line: usize, args: Vec<V>) -> R {
        let V::Func(f) = f else { return call(f, args, line) };
        enter(&f, line)?;
        let result = (f.body.as_ref().unwrap())(args);
        leave();
        result
    }

    fn call_function(f: &Func, positional: Vec<V>, named_names: &[&str], named: Vec<V>, line: usize) -> R {
        let name = if f.name.is_empty() { "function".to_string() } else { f.name.clone() };
        enter(f, line)?;
        let result = (|| {
            let slots = bind(&name, &f.parameters, positional, named_names, named, line)?;
            for (i, slot) in slots.iter().enumerate() {
                if matches!(slot, V::Missing) && !f.defaults[i] {
                    return fatal(format!("`{name}` needs a value for `{}`", f.parameters[i]), line);
                }
            }
            (f.body.as_ref().unwrap())(slots)
        })();
        leave();
        result
    }

    fn no_named(what: &str, named_names: &[&str], line: usize) -> Result<(), Exc> {
        if let Some(n) = named_names.first() {
            return fatal(format!("`{what}` does not take a named argument `{n}`"), line);
        }
        Ok(())
    }

    fn prepend(first: V, rest: Vec<V>) -> Vec<V> {
        let mut out = Vec::with_capacity(rest.len() + 1);
        out.push(first);
        out.extend(rest);
        out
    }

    pub fn call_named(callee: V, positional: Vec<V>, named_names: &[&str], named: Vec<V>, line: usize) -> R {
        match &callee {
            V::Bound(b) => call_function(&b.1, prepend(b.0.clone(), positional), named_names, named, line),
            V::Method(m) => {
                no_named(&m.1, named_names, line)?;
                call_method_of(m.0.clone(), &m.1, positional, line)
            }
            V::Class(c) => {
                let mut slots = bind(&c.name, &c.fields, positional, named_names, named, line)?;
                for i in 0..slots.len() {
                    if matches!(slots[i], V::Missing) {
                        match &c.defaults[i] {
                            V::Func(f) => slots[i] = (f.body.as_ref().unwrap())(Vec::new())?,
                            _ => return fatal(format!("`{}` needs a value for `{}`", c.name, c.fields[i]), line),
                        }
                    }
                }
                Ok(V::Instance(Rc::new(Instance { cls: c.clone(), fields: RefCell::new(slots) })))
            }
            V::Ctor(def, index) => {
                let name = &def.cases[*index];
                let fields = &def.fields[*index];
                let slots = bind(name, fields, positional, named_names, named, line)?;
                for (i, slot) in slots.iter().enumerate() {
                    if matches!(slot, V::Missing) {
                        return fatal(format!("`{name}` needs a value for `{}`", fields[i]), line);
                    }
                }
                Ok(V::Case(Rc::new(Case { def: def.clone(), index: *index, fields: slots })))
            }
            V::ErrType(kind) => {
                no_named(kind, named_names, line)?;
                new_error(kind, positional, line)
            }
            V::Func(f) => {
                if let Some(b) = &f.bbody {
                    no_named(&f.name, named_names, line)?;
                    return b(positional, line);
                }
                call_function(f, positional, named_names, named, line)
            }
            _ => fatal(format!("{} cannot be called", type_name(&callee)), line),
        }
    }

    pub fn call(callee: V, args: Vec<V>, line: usize) -> R {
        call_named(callee, args, &[], Vec::new(), line)
    }

    fn new_error(kind: &str, args: Vec<V>, line: usize) -> R {
        if args.len() > 1 {
            return fatal(format!("`{kind}` takes one message"), line);
        }
        let message = args.first().map(show).unwrap_or_default();
        Ok(V::Error(Rc::new(ZError { kind: kind.to_string(), message, obj: V::None })))
    }

    pub fn call_method_named(obj: V, name: &str, positional: Vec<V>, named_names: &[&str], named: Vec<V>, line: usize) -> R {
        if let V::Instance(o) = &obj {
            if !o.cls.fields.iter().any(|f| f == name) {
                if let Some(m) = o.cls.method(name) {
                    return call_function(&m, prepend(obj.clone(), positional), named_names, named, line);
                }
            }
        }
        if let V::Module(m) = &obj {
            no_named(name, named_names, line)?;
            return call_builtin(&format!("{m}.{name}"), positional, line);
        }
        if matches!(obj, V::Str(_) | V::List(_) | V::Dict(_)) {
            no_named(name, named_names, line)?;
            return call_method_of(obj, name, positional, line);
        }
        let f = field(obj, name, line)?;
        call_named(f, positional, named_names, named, line)
    }

    pub fn call_method(obj: V, name: &str, args: Vec<V>, line: usize) -> R {
        call_method_named(obj, name, args, &[], Vec::new(), line)
    }

    // ----------------------------------------------------------- Errors

    pub fn raise(value: V, line: usize) -> R {
        match &value {
            V::Error(e) => Err(Exc::Raise(Raise { value: e.clone(), line })),
            V::Instance(o) => Err(Exc::Raise(Raise {
                value: Rc::new(ZError { kind: o.cls.name.clone(), message: show(&value), obj: value.clone() }),
                line,
            })),
            _ => fatal(format!("only an error can be raised, and this is {}", type_name(&value)), line),
        }
    }

    pub fn catches(e: Raise, name: &str) -> RB {
        Ok(name == "Error" || name == e.value.kind)
    }

    pub fn caught(e: Raise, name: &str) -> R {
        if !name.is_empty() && name != "Error" && !matches!(e.value.obj, V::None) {
            return Ok(e.value.obj.clone());
        }
        Ok(V::Error(e.value))
    }

    pub fn no_match(v: V, line: usize) -> R {
        fatal(format!("no `case` matches {}", repr(&v)), line)
    }

    pub fn undefined(name: &str, line: usize) -> R {
        fatal(format!("`{name}` is not defined"), line)
    }

    // ----------------------------------------------- Built-in functions

    fn arity(name: &str, args: &[V], lo: usize, hi: usize, line: usize) -> Result<(), Exc> {
        if args.len() >= lo && args.len() <= hi {
            return Ok(());
        }
        let wanted = if lo == hi { counted(lo, "argument") } else { format!("{lo} to {hi} arguments") };
        fatal(format!("`{name}` takes {wanted}, but {} given", were(args.len())), line)
    }

    fn wrong_type<T>(name: &str, wanted: &str, got: &V, line: usize) -> Result<T, Exc> {
        fatal(format!("`{name}` needs {wanted}, but this is {}", type_name(got)), line)
    }

    fn text(name: &str, v: &V, line: usize) -> Result<Rc<str>, Exc> {
        match v {
            V::Str(s) => Ok(s.clone()),
            _ => wrong_type(name, "a str", v, line),
        }
    }

    fn whole_arg(name: &str, v: &V, line: usize) -> Result<i64, Exc> {
        match v {
            V::Int(n) => Ok(*n),
            _ => wrong_type(name, "a whole number", v, line),
        }
    }

    fn is_blank(c: char) -> bool {
        matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
    }

    fn strip(text: &str) -> &str {
        text.trim_matches(is_blank)
    }

    thread_local! {
        // What the program prints waits here, and goes out in big pieces.
        static OUT: RefCell<Vec<u8>> = RefCell::new(Vec::new());
        static ARGUMENTS: RefCell<Vec<String>> = RefCell::new(Vec::new());
        static STARTED: std::time::Instant = std::time::Instant::now();
    }

    fn flush() {
        OUT.with(|out| {
            let mut out = out.borrow_mut();
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(&out);
            let _ = stdout.flush();
            out.clear();
        });
    }

    fn write(text: &str) {
        let full = OUT.with(|out| {
            let mut out = out.borrow_mut();
            out.extend_from_slice(text.as_bytes());
            out.len() > 65536
        });
        if full {
            flush();
        }
    }

    fn amount(v: &V) -> f64 {
        match v {
            V::Quantity(x, _) => *x,
            _ => number(v),
        }
    }

    fn numbers(name: &str, args: Vec<V>, line: usize) -> Result<(Vec<V>, Option<Rc<str>>), Exc> {
        let values = match args.as_slice() {
            [V::List(l)] => l.borrow().clone(),
            _ => args,
        };
        let mut unit: Option<Rc<str>> = None;
        for (place, item) in values.iter().enumerate() {
            if let V::Quantity(_, u) = item {
                if let Some(have) = &unit {
                    if have != u {
                        return fatal(format!("`{name}` needs quantities in one unit, but these are in {have} and in {u}. Turn one into the other first, as in `x.{have}`"), line);
                    }
                }
                if unit.is_none() && place > 0 {
                    return fatal(format!("`{name}` needs all plain numbers or all quantities, but these are both"), line);
                }
                unit = Some(u.clone());
            } else if !is_number(item) {
                return wrong_type(name, "numbers", item, line);
            } else if unit.is_some() {
                return fatal(format!("`{name}` needs all plain numbers or all quantities, but these are both"), line);
            }
        }
        Ok((values, unit))
    }

    fn pick(name: &str, args: Vec<V>, line: usize, smaller: bool) -> R {
        let (mut values, _) = numbers(name, args, line)?;
        if all_single(&values) {
            values = values.iter().map(|v| V::F32(single(v).unwrap_or(0.0))).collect();
        }
        let Some(mut best) = values.first().cloned() else {
            return raise_error("ValueError", format!("`{name}` of nothing"), line);
        };
        for item in &values[1..] {
            if let (V::U64(x), V::U64(y)) = (item, &best) {
                if if smaller { x < y } else { x > y } {
                    best = item.clone();
                }
                continue;
            }
            if if smaller { amount(item) < amount(&best) } else { amount(item) > amount(&best) } {
                best = item.clone();
            }
        }
        Ok(best)
    }

    /// Heapsort, step for step as `zeno run` sorts, so that a function
    /// given to `sort` is called in the same order.
    fn heapsort(n: usize, before: &mut dyn FnMut(usize, usize) -> RB, swap: &mut dyn FnMut(usize, usize)) -> Result<(), Exc> {
        fn sift(mut root: usize, end: usize, before: &mut dyn FnMut(usize, usize) -> RB, swap: &mut dyn FnMut(usize, usize)) -> Result<(), Exc> {
            loop {
                let mut child = 2 * root + 1;
                if child >= end {
                    return Ok(());
                }
                if child + 1 < end && before(child, child + 1)? {
                    child += 1;
                }
                if !before(root, child)? {
                    return Ok(());
                }
                swap(root, child);
                root = child;
            }
        }
        for root in (0..n / 2).rev() {
            sift(root, n, before, swap)?;
        }
        for end in (1..n).rev() {
            swap(0, end);
            sift(0, end, before, swap)?;
        }
        Ok(())
    }

    fn sort_list(l: &RefCell<Vec<V>>, line: usize) -> Result<(), Exc> {
        let s = RefCell::new(l.borrow().clone());
        let n = s.borrow().len();
        heapsort(
            n,
            &mut |i, j| {
                let (a, b) = (s.borrow()[i].clone(), s.borrow()[j].clone());
                lt(a, b, line)
            },
            &mut |i, j| s.borrow_mut().swap(i, j),
        )?;
        *l.borrow_mut() = s.into_inner();
        Ok(())
    }

    fn sort_by(l: &RefCell<Vec<V>>, function: V, line: usize) -> Result<(), Exc> {
        let copy = l.borrow().clone();
        let mut keys = Vec::new();
        for item in copy {
            let k = call(function.clone(), vec![item], line)?;
            if !matches!(k, V::Int(_) | V::Float(_) | V::F32(_) | V::U64(_) | V::Str(_)) {
                return fatal(format!("the function given to `sort` must return a number or text to order by, but it returned {}", type_name(&k)), line);
            }
            keys.push(k);
        }
        let items = l.borrow().clone();
        if items.len() != keys.len() {
            return raise_error("ValueError", "the function given to `sort` changed how many items the list has", line);
        }
        let n = items.len();
        let state = RefCell::new((items, keys, (0..n).collect::<Vec<usize>>()));
        heapsort(
            n,
            &mut |a, b| {
                let (ka, kb, pa, pb) = {
                    let st = state.borrow();
                    (st.1[a].clone(), st.1[b].clone(), st.2[a], st.2[b])
                };
                if lt(ka.clone(), kb.clone(), line)? {
                    return Ok(true);
                }
                if lt(kb, ka, line)? {
                    return Ok(false);
                }
                Ok(pa < pb)
            },
            &mut |i, j| {
                let mut st = state.borrow_mut();
                st.0.swap(i, j);
                st.1.swap(i, j);
                st.2.swap(i, j);
            },
        )?;
        *l.borrow_mut() = state.into_inner().0;
        Ok(())
    }

    fn slice(args: &[V], length: usize, what: &str, line: usize) -> Result<(usize, usize), Exc> {
        let from = whole_arg("slice", &args[0], line)?;
        let to = if args.len() > 1 { whole_arg("slice", &args[1], line)? } else { length as i64 };
        if from < 0 || to < from || to > length as i64 {
            return raise_error("IndexError", format!("slice {from} to {to} is out of range for a {what} of length {length}"), line);
        }
        Ok((from as usize, to as usize))
    }

    const STR_METHODS: &[&str] = &["upper", "lower", "strip", "split", "lines", "replace", "starts_with", "ends_with", "find", "slice", "code"];
    const LIST_METHODS: &[&str] = &["append", "pop", "map", "filter", "reduce", "insert", "reverse", "sort", "clear", "find", "slice", "join"];
    const DICT_METHODS: &[&str] = &["get", "pop", "keys", "values", "clear"];

    fn has_method(v: &V, name: &str) -> bool {
        match v {
            V::Str(_) => STR_METHODS.contains(&name),
            V::List(_) => LIST_METHODS.contains(&name),
            V::Dict(_) => DICT_METHODS.contains(&name),
            _ => false,
        }
    }

    /// The lines of a text. A line ends at "\n", and a "\r" just before
    /// that "\n" goes with it. A "\r" anywhere else stays in its line. The
    /// last line need not end, and an end at the very end starts no line.
    fn lines(text: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            match rest.find('\n') {
                Some(at) => {
                    let line = &rest[..at];
                    out.push(line.strip_suffix('\r').unwrap_or(line));
                    rest = &rest[at + 1..];
                }
                None => {
                    out.push(rest);
                    rest = "";
                }
            }
        }
        out
    }

    fn strings<S: AsRef<str>>(parts: impl IntoIterator<Item = S>) -> V {
        new_list(parts.into_iter().map(|p| s(p.as_ref())).collect())
    }

    fn call_method_of(receiver: V, name: &str, args: Vec<V>, line: usize) -> R {
        match &receiver {
            V::Str(text_) => {
                let s_: &str = text_;
                match name {
                    "upper" => {
                        arity(name, &args, 0, 0, line)?;
                        Ok(st(s_.to_ascii_uppercase()))
                    }
                    "lower" => {
                        arity(name, &args, 0, 0, line)?;
                        Ok(st(s_.to_ascii_lowercase()))
                    }
                    "strip" => {
                        arity(name, &args, 0, 0, line)?;
                        Ok(s(strip(s_)))
                    }
                    "split" => {
                        arity(name, &args, 0, 1, line)?;
                        if args.is_empty() {
                            return Ok(strings(s_.split(is_blank).filter(|w| !w.is_empty())));
                        }
                        let sep = text(name, &args[0], line)?;
                        if sep.is_empty() {
                            return raise_error("ValueError", "cannot split on an empty str", line);
                        }
                        Ok(strings(s_.split(&*sep)))
                    }
                    "lines" => {
                        arity(name, &args, 0, 0, line)?;
                        Ok(strings(lines(s_)))
                    }
                    "replace" => {
                        arity(name, &args, 2, 2, line)?;
                        let (old, now) = (text(name, &args[0], line)?, text(name, &args[1], line)?);
                        Ok(st(s_.replace(&*old, &now)))
                    }
                    "starts_with" => {
                        arity(name, &args, 1, 1, line)?;
                        Ok(V::Bool(s_.starts_with(&*text(name, &args[0], line)?)))
                    }
                    "ends_with" => {
                        arity(name, &args, 1, 1, line)?;
                        Ok(V::Bool(s_.ends_with(&*text(name, &args[0], line)?)))
                    }
                    "find" => {
                        arity(name, &args, 1, 1, line)?;
                        Ok(match s_.find(&*text(name, &args[0], line)?) {
                            Some(at) => V::Int(s_[..at].chars().count() as i64),
                            None => V::None,
                        })
                    }
                    "slice" => {
                        arity(name, &args, 1, 2, line)?;
                        let (from, to) = slice(&args, text_length(s_), "str", line)?;
                        Ok(st(sub_text(s_, from, to)))
                    }
                    "code" => {
                        arity(name, &args, 0, 0, line)?;
                        let n = s_.chars().count();
                        if n != 1 {
                            return raise_error("ValueError", format!("`code` needs a str of exactly one character, and this one has {n}"), line);
                        }
                        Ok(V::Int(s_.chars().next().unwrap() as i64))
                    }
                    _ => fatal(format!("a str has no method `{name}`"), line),
                }
            }
            V::Dict(d) => match name {
                "get" => {
                    arity(name, &args, 1, 2, line)?;
                    let k = key(args[0].clone(), line)?;
                    if let Some(v) = d.borrow().map.get(&key_of(&k)) {
                        return Ok(v.clone());
                    }
                    Ok(args.get(1).cloned().unwrap_or(V::None))
                }
                "pop" => {
                    arity(name, &args, 1, 1, line)?;
                    let k = key(args[0].clone(), line)?;
                    let found = d.borrow().map.get(&key_of(&k)).cloned();
                    match found {
                        Some(v) => {
                            dict_remove(&mut d.borrow_mut(), &k);
                            Ok(v)
                        }
                        None => raise_error("KeyError", format!("this dict has no key {}", repr(&args[0])), line),
                    }
                }
                "keys" => {
                    arity(name, &args, 0, 0, line)?;
                    Ok(new_list(d.borrow().keys.clone()))
                }
                "values" => {
                    arity(name, &args, 0, 0, line)?;
                    let d = d.borrow();
                    Ok(new_list(d.keys.iter().map(|k| d.map[&key_of(k)].clone()).collect()))
                }
                "clear" => {
                    arity(name, &args, 0, 0, line)?;
                    let mut d = d.borrow_mut();
                    d.keys.clear();
                    d.map.clear();
                    Ok(V::None)
                }
                _ => fatal(format!("a dict has no method `{name}`"), line),
            },
            V::List(l) => match name {
                "append" => {
                    arity(name, &args, 1, 1, line)?;
                    l.borrow_mut().push(args[0].clone());
                    Ok(V::None)
                }
                "pop" => {
                    arity(name, &args, 0, 1, line)?;
                    if !args.is_empty() {
                        let at = position(&args[0], l.borrow().len(), "list", line)?;
                        return Ok(l.borrow_mut().remove(at));
                    }
                    let last = l.borrow_mut().pop();
                    match last {
                        Some(v) => Ok(v),
                        None => raise_error("IndexError", "cannot pop from an empty list", line),
                    }
                }
                "insert" => {
                    arity(name, &args, 2, 2, line)?;
                    let at = whole_arg(name, &args[0], line)?;
                    let len = l.borrow().len();
                    if at < 0 || at as usize > len {
                        return raise_error("IndexError", format!("cannot insert at {at} in a list of length {len}"), line);
                    }
                    l.borrow_mut().insert(at as usize, args[1].clone());
                    Ok(V::None)
                }
                "reverse" => {
                    arity(name, &args, 0, 0, line)?;
                    l.borrow_mut().reverse();
                    Ok(V::None)
                }
                "clear" => {
                    arity(name, &args, 0, 0, line)?;
                    l.borrow_mut().clear();
                    Ok(V::None)
                }
                "sort" => {
                    if args.len() == 1 {
                        sort_by(l, args[0].clone(), line)?;
                        return Ok(V::None);
                    }
                    arity(name, &args, 0, 1, line)?;
                    sort_list(l, line)?;
                    Ok(V::None)
                }
                "find" => {
                    arity(name, &args, 1, 1, line)?;
                    Ok(match l.borrow().iter().position(|x| equal(x, &args[0])) {
                        Some(at) => V::Int(at as i64),
                        None => V::None,
                    })
                }
                "slice" => {
                    arity(name, &args, 1, 2, line)?;
                    let (from, to) = slice(&args, l.borrow().len(), "list", line)?;
                    Ok(new_list(l.borrow()[from..to].to_vec()))
                }
                "join" => {
                    arity(name, &args, 1, 1, line)?;
                    let sep = text(name, &args[0], line)?;
                    let mut parts = Vec::new();
                    for item in l.borrow().iter() {
                        match item {
                            V::Str(p) => parts.push(p.to_string()),
                            _ => {
                                return fatal(format!("`join` puts text together, and this list holds {}. Make each item text first, with `str( )`", type_name(item)), line)
                            }
                        }
                    }
                    Ok(st(parts.join(&sep)))
                }
                "map" => {
                    arity(name, &args, 1, 1, line)?;
                    let copy = l.borrow().clone();
                    let mut out = Vec::new();
                    for item in copy {
                        out.push(call(args[0].clone(), vec![item], line)?);
                    }
                    Ok(new_list(out))
                }
                "filter" => {
                    arity(name, &args, 1, 1, line)?;
                    let copy = l.borrow().clone();
                    let mut kept = Vec::new();
                    for item in copy {
                        match call(args[0].clone(), vec![item.clone()], line)? {
                            V::Bool(true) => kept.push(item),
                            V::Bool(false) => {}
                            other => {
                                return fatal(format!("the function given to `filter` must return true or false, but it returned {}", type_name(&other)), line)
                            }
                        }
                    }
                    Ok(new_list(kept))
                }
                "reduce" => {
                    arity(name, &args, 2, 2, line)?;
                    let copy = l.borrow().clone();
                    let mut total = args[1].clone();
                    for item in copy {
                        total = call(args[0].clone(), vec![total, item], line)?;
                    }
                    Ok(total)
                }
                _ => fatal(format!("a list has no method `{name}`"), line),
            },
            _ => fatal(format!("{} has no method `{name}`", type_name(&receiver)), line),
        }
    }

    /// `{x:>8.2f}`: [<>^]? 0? width? (.precision)? kind?
    pub fn format(v: V, spec: &str, line: usize) -> R {
        if spec.is_empty() {
            return Ok(st(show(&v)));
        }
        let not_understood = || fatal(format!("`{spec}` is not a format Zeno understands"), line);
        let mut rest: &str = spec;
        let mut align = None;
        if let Some(c) = rest.chars().next().filter(|c| matches!(c, '<' | '>' | '^')) {
            align = Some(c);
            rest = &rest[1..];
        }
        let zero = rest.starts_with('0');
        if zero {
            rest = &rest[1..];
        }
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        let width: usize = rest[..digits].parse().unwrap_or(0);
        rest = &rest[digits..];
        let mut precision = None;
        if let Some(after) = rest.strip_prefix('.') {
            let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
            precision = Some(after[..digits].parse::<usize>().unwrap_or(0));
            rest = &after[digits..];
        }
        let mut chars = rest.chars();
        let kind = chars.next();
        if chars.next().is_some() || kind.is_some_and(|c| c.is_ascii_digit()) {
            return not_understood();
        }
        let places = precision.unwrap_or(6);
        let body = match (kind, precision, &v) {
            (Some('f') | None, _, V::Quantity(x, u)) if kind == Some('f') || precision.is_some() => format!("{x:.places$} {u}"),
            (Some('f'), _, _) | (None, Some(_), V::Float(_)) => {
                if !is_number(&v) {
                    return wrong_type(spec, "a number", &v, line);
                }
                format!("{:.places$}", number(&v))
            }
            (None, None, _) | (Some('d'), None, V::Int(_) | V::U64(_)) => show(&v),
            (Some('x'), None, V::U64(n)) => format!("{n:x}"),
            (Some('X'), None, V::U64(n)) => format!("{n:X}"),
            (Some('b'), None, V::U64(n)) => format!("{n:b}"),
            (Some('x'), None, V::Int(n)) => format!("{}{:x}", if *n < 0 { "-" } else { "" }, n.unsigned_abs()),
            (Some('X'), None, V::Int(n)) => format!("{}{:X}", if *n < 0 { "-" } else { "" }, n.unsigned_abs()),
            (Some('b'), None, V::Int(n)) => format!("{}{:b}", if *n < 0 { "-" } else { "" }, n.unsigned_abs()),
            (Some('d' | 'x' | 'X' | 'b'), None, _) => return wrong_type(spec, "an int", &v, line),
            _ => return not_understood(),
        };
        let padding = width.saturating_sub(body.chars().count());
        let is_num = is_number(&v) || matches!(v, V::Quantity(..));
        if zero && is_num {
            if let Some(digits) = body.strip_prefix('-') {
                return Ok(st(format!("-{}{digits}", "0".repeat(padding))));
            }
            return Ok(st(format!("{}{body}", "0".repeat(padding))));
        }
        let a = align.unwrap_or(if is_num { '>' } else { '<' });
        Ok(st(match a {
            '<' => format!("{body}{}", " ".repeat(padding)),
            '>' => format!("{}{body}", " ".repeat(padding)),
            _ => {
                let left = padding / 2;
                format!("{}{body}{}", " ".repeat(left), " ".repeat(padding - left))
            }
        }))
    }

    // ------------------------------------------------------------- JSON

    struct Json {
        cs: Vec<char>,
        at: usize,
    }

    impl Json {
        fn spaces(&mut self) {
            while self.at < self.cs.len() && matches!(self.cs[self.at], ' ' | '\t' | '\n' | '\r') {
                self.at += 1;
            }
        }

        fn word(&mut self, w: &str) -> bool {
            let w: Vec<char> = w.chars().collect();
            if self.at + w.len() > self.cs.len() || self.cs[self.at..self.at + w.len()] != w[..] {
                return false;
            }
            self.at += w.len();
            true
        }

        fn four_hex(&mut self) -> Option<u32> {
            if self.at + 4 > self.cs.len() {
                return None;
            }
            let mut n = 0;
            for i in 0..4 {
                n = n * 16 + self.cs[self.at + i].to_digit(16)?;
            }
            self.at += 4;
            Some(n)
        }

        fn text(&mut self) -> Option<String> {
            if self.at >= self.cs.len() || self.cs[self.at] != '"' {
                return None;
            }
            self.at += 1;
            let mut b = String::new();
            loop {
                let c = *self.cs.get(self.at)?;
                self.at += 1;
                if c == '"' {
                    return Some(b);
                }
                if c != '\\' {
                    b.push(c);
                    continue;
                }
                let e = *self.cs.get(self.at)?;
                self.at += 1;
                match e {
                    'n' => b.push('\n'),
                    't' => b.push('\t'),
                    'r' => b.push('\r'),
                    'b' => b.push('\x08'),
                    'f' => b.push('\x0C'),
                    'u' => {
                        let mut code = self.four_hex()?;
                        if (0xD800..0xDC00).contains(&code) && self.word("\\u") {
                            let low = self.four_hex()?;
                            if !(0xDC00..0xE000).contains(&low) {
                                return None;
                            }
                            code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        }
                        b.push(char::from_u32(code)?);
                    }
                    other => b.push(other),
                }
            }
        }

        fn value(&mut self) -> Option<V> {
            self.spaces();
            let c = *self.cs.get(self.at)?;
            if c == 'n' {
                return self.word("null").then_some(V::None);
            }
            if c == 't' {
                return self.word("true").then_some(V::Bool(true));
            }
            if c == 'f' {
                return self.word("false").then_some(V::Bool(false));
            }
            if c == '"' {
                return self.text().map(|t| st(t));
            }
            if c == '[' {
                self.at += 1;
                let mut out = Vec::new();
                loop {
                    self.spaces();
                    if self.word("]") {
                        return Some(new_list(out));
                    }
                    if !out.is_empty() && !self.word(",") {
                        return None;
                    }
                    out.push(self.value()?);
                }
            }
            if c == '{' {
                self.at += 1;
                let mut d = Dict::default();
                let mut count = 0;
                loop {
                    self.spaces();
                    if self.word("}") {
                        return Some(V::Dict(Rc::new(RefCell::new(d))));
                    }
                    if count > 0 && !self.word(",") {
                        return None;
                    }
                    self.spaces();
                    let name = self.text()?;
                    self.spaces();
                    if !self.word(":") {
                        return None;
                    }
                    let v = self.value()?;
                    count += 1;
                    if !matches!(v, V::None) {
                        dict_set(&mut d, st(name), v);
                    }
                }
            }
            let start = self.at;
            while self.at < self.cs.len() && "0123456789+-.eE".contains(self.cs[self.at]) {
                self.at += 1;
            }
            let chars: String = self.cs[start..self.at].iter().collect();
            let f: f64 = chars.parse().ok()?;
            if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 {
                return Some(V::Int(f as i64));
            }
            Some(V::Float(f))
        }
    }

    fn json_parse(s: &str) -> Option<V> {
        let mut j = Json { cs: s.chars().collect(), at: 0 };
        let v = j.value()?;
        j.spaces();
        (j.at == j.cs.len()).then_some(v)
    }

    fn json_text(s: &str) -> String {
        let mut b = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => b.push_str("\\\""),
                '\\' => b.push_str("\\\\"),
                '\n' => b.push_str("\\n"),
                '\r' => b.push_str("\\r"),
                '\t' => b.push_str("\\t"),
                c if (c as u32) < 0x20 => b.push_str(&format!("\\u{:04x}", c as u32)),
                c => b.push(c),
            }
        }
        b.push('"');
        b
    }

    fn json_write(v: &V, line: usize) -> Result<String, Exc> {
        Ok(match v {
            V::None => "null".into(),
            V::Bool(b) => b.to_string(),
            V::Int(n) => n.to_string(),
            V::Float(x) if !x.is_finite() => return raise_error("ValueError", format!("JSON has no way to write {}", float_text(*x)), line),
            V::Float(x) => float_text(*x),
            V::Str(s) => json_text(s),
            V::List(l) => {
                let parts: Result<Vec<String>, Exc> = l.borrow().iter().map(|item| json_write(item, line)).collect();
                format!("[{}]", parts?.join(","))
            }
            V::Dict(d) => {
                let d = d.borrow();
                let mut parts = Vec::new();
                for k in &d.keys {
                    parts.push(format!("{}:{}", json_text(&show(k)), json_write(&d.map[&key_of(k)], line)?));
                }
                format!("{{{}}}", parts.join(","))
            }
            V::Instance(o) => {
                let fields = o.fields.borrow();
                let mut parts = Vec::new();
                for (name, value) in o.cls.fields.iter().zip(fields.iter()) {
                    parts.push(format!("{}:{}", json_text(name), json_write(value, line)?));
                }
                format!("{{{}}}", parts.join(","))
            }
            V::Quantity(_, u) => {
                return raise_error("ValueError", format!("JSON has no units, and this is a quantity in {u}. Write its number, and put the unit in the name: `[\"{u}\": ...]`"), line)
            }
            other => return raise_error("ValueError", format!("JSON has objects, lists, text, numbers, bools and none, and this is {}", type_name(other)), line),
        })
    }

    // ------------------------------------------------------------ Files

    fn paths(name: &str, args: &[V], count: usize, line: usize) -> Result<Vec<String>, Exc> {
        arity(name, args, count, count, line)?;
        args.iter().map(|a| text(name, a, line).map(|t| t.to_string())).collect()
    }

    fn file_error<T>(message: String, line: usize) -> Result<T, Exc> {
        raise_error("FileError", message, line)
    }

    // ----------------------------------------------- The built-in table

    pub fn b_print(args: Vec<V>, line: usize) -> R {
        let parts: Vec<String> = args.iter().map(show).collect();
        write(&parts.join(" "));
        write("\n");
        Ok(V::None)
    }

    pub fn b_input(args: Vec<V>, line: usize) -> R {
        arity("input", &args, 0, 1, line)?;
        if let Some(prompt) = args.first() {
            write(&text("input", prompt, line)?);
        }
        flush();
        let mut bytes = Vec::new();
        let mut byte = [0u8];
        let mut stdin = std::io::stdin().lock();
        let (mut any, mut ended) = (false, false);
        while let Ok(1) = stdin.read(&mut byte) {
            any = true;
            if byte[0] == b'\n' {
                ended = true;
                break;
            }
            bytes.push(byte[0]);
        }
        if !any {
            return Ok(V::None);
        }
        if ended && bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        Ok(st(String::from_utf8_lossy(&bytes).replace('\0', "")))
    }

    pub fn b_len(args: Vec<V>, line: usize) -> R {
        arity("len", &args, 1, 1, line)?;
        match &args[0] {
            V::Str(s) => Ok(V::Int(text_length(s) as i64)),
            V::List(l) => Ok(V::Int(l.borrow().len() as i64)),
            V::Dict(d) => Ok(V::Int(d.borrow().keys.len() as i64)),
            other => wrong_type("len", "a str, a list or a dict", other, line),
        }
    }

    pub fn b_range(args: Vec<V>, line: usize) -> R {
        arity("range", &args, 1, 3, line)?;
        let mut n = Vec::new();
        for a in &args {
            match a {
                V::Int(x) => n.push(*x),
                other => return wrong_type("range", "ints", other, line),
            }
        }
        let (start, stop, step) = match n.as_slice() {
            [stop] => (0, *stop, 1),
            [start, stop] => (*start, *stop, 1),
            [start, stop, step, ..] => (*start, *stop, *step),
            _ => (0, 0, 1),
        };
        if step == 0 {
            return raise_error("ValueError", "the step of a range cannot be 0", line);
        }
        let mut out = Vec::new();
        let mut i = start;
        while if step > 0 { i < stop } else { i > stop } {
            out.push(V::Int(i));
            match i.checked_add(step) {
                Some(next) => i = next,
                None => break,
            }
        }
        Ok(new_list(out))
    }

    pub fn b_int(args: Vec<V>, line: usize) -> R {
        arity("int", &args, 1, 1, line)?;
        match &args[0] {
            V::Int(n) => Ok(V::Int(*n)),
            V::U64(x) if *x > i64::MAX as u64 => raise_error("OverflowError", format!("{x} does not fit in an int (-9223372036854775808 to 9223372036854775807)"), line),
            V::U64(x) => Ok(V::Int(*x as i64)),
            V::Float(x) if x.is_finite() => Ok(V::Int(x.trunc() as i64)),
            V::Float(x) => raise_error("ValueError", format!("cannot turn {x} into an int"), line),
            V::F32(x) if x.is_finite() => Ok(V::Int(x.trunc() as i64)),
            V::F32(x) => raise_error("ValueError", format!("cannot turn {x} into an int"), line),
            V::Str(t) => match strip(t).replace('_', "").parse() {
                Ok(n) => Ok(V::Int(n)),
                Err(_) => raise_error("ValueError", format!("cannot turn {} into an int", quoted(t)), line),
            },
            other => wrong_type("int", "a number or a str", other, line),
        }
    }

    pub fn b_float(args: Vec<V>, line: usize) -> R {
        arity("float", &args, 1, 1, line)?;
        match &args[0] {
            V::Int(n) => Ok(V::Float(*n as f64)),
            V::Float(x) => Ok(V::Float(*x)),
            V::F32(x) => Ok(V::Float(*x as f64)),
            V::U64(x) => Ok(V::Float(*x as f64)),
            V::Str(t) => match strip(t).parse() {
                Ok(x) => Ok(V::Float(x)),
                Err(_) => raise_error("ValueError", format!("cannot turn {} into a float", quoted(t)), line),
            },
            other => wrong_type("float", "a number or a str", other, line),
        }
    }

    pub fn b_f32(args: Vec<V>, line: usize) -> R {
        arity("f32", &args, 1, 1, line)?;
        match &args[0] {
            V::F32(x) => Ok(V::F32(*x)),
            V::Int(n) => Ok(V::F32(*n as f64 as f32)),
            V::Float(x) => {
                let r = *x as f32;
                if r.is_infinite() && x.is_finite() {
                    return raise_error("OverflowError", format!("{} does not fit in an f32, whose largest is about 3.4e38", float_text(*x)), line);
                }
                Ok(V::F32(r))
            }
            other => wrong_type("f32", "a number", other, line),
        }
    }

    pub fn b_str(args: Vec<V>, line: usize) -> R {
        arity("str", &args, 1, 1, line)?;
        Ok(st(show(&args[0])))
    }

    pub fn b_abs(args: Vec<V>, line: usize) -> R {
        arity("abs", &args, 1, 1, line)?;
        match &args[0] {
            V::Int(n) => n.checked_abs().map(V::Int).map_or_else(|| overflow(line), Ok),
            V::Float(x) => Ok(V::Float(x.abs())),
            V::F32(x) => Ok(V::F32(x.abs())),
            V::U64(x) => Ok(V::U64(*x)),
            V::Quantity(x, u) => Ok(V::Quantity(x.abs(), u.clone())),
            other => wrong_type("abs", "a number", other, line),
        }
    }

    pub fn b_min(args: Vec<V>, line: usize) -> R {
        pick("min", args, line, true)
    }

    pub fn b_max(args: Vec<V>, line: usize) -> R {
        pick("max", args, line, false)
    }

    pub fn b_sum(args: Vec<V>, line: usize) -> R {
        let (values, unit) = numbers("sum", args, line)?;
        if unit.is_some() {
            return fatal("`sum` works on plain numbers, and these have a unit. Add quantities with `+`, which keeps the unit: start from `var total = 0 V` and add each in a loop", line);
        }
        if values.iter().any(|v| matches!(v, V::U64(_))) {
            let mut total: u64 = 0;
            for item in &values {
                total = match total.checked_add(unsigned(item).unwrap_or(0)) {
                    Some(t) => t,
                    None => return raise_error("OverflowError", U64_TOO_BIG, line),
                };
            }
            return Ok(V::U64(total));
        }
        if all_single(&values) {
            let mut single32: f32 = 0.0;
            for item in &values {
                single32 += single(item).unwrap_or(0.0);
            }
            return Ok(V::F32(single32));
        }
        let mut total: i64 = 0;
        let mut all_ints = true;
        let mut float_total = 0.0;
        for item in &values {
            float_total += number(item);
            match item {
                V::Int(n) if all_ints => match total.checked_add(*n) {
                    Some(t) => total = t,
                    None => return overflow(line),
                },
                _ => all_ints = false,
            }
        }
        Ok(if all_ints { V::Int(total) } else { V::Float(float_total) })
    }

    pub fn b_char(args: Vec<V>, line: usize) -> R {
        arity("char", &args, 1, 1, line)?;
        let code = whole_arg("char", &args[0], line)?;
        match u32::try_from(code).ok().filter(|c| *c > 0).and_then(char::from_u32) {
            Some(c) => Ok(st(c.to_string())),
            None => raise_error("ValueError", format!("{code} is not the code of a character"), line),
        }
    }

    pub fn b_allocate(args: Vec<V>, line: usize) -> R {
        fatal("manual memory is only for a program compiled to machine code", line)
    }

    pub fn b_free(args: Vec<V>, line: usize) -> R {
        b_allocate(args, line)
    }

    fn one_number(name: &str, args: &[V], line: usize) -> Result<f64, Exc> {
        arity(name, args, 1, 1, line)?;
        if !is_number(&args[0]) {
            return wrong_type(name, "a number", &args[0], line);
        }
        Ok(number(&args[0]))
    }

    fn angle(name: &str, args: &[V], line: usize) -> Result<f64, Exc> {
        arity(name, args, 1, 1, line)?;
        match &args[0] {
            V::Quantity(x, u) => match convert(u, "rad") {
                Some(scale) => Ok(apply(scale, *x)),
                None => fatal(format!("`{name}` needs a number or an angle, such as `30 deg`, not a quantity in {u}"), line),
            },
            v if is_number(v) => Ok(number(v)),
            other => wrong_type(name, "a number or an angle, such as `30 deg`", other, line),
        }
    }

    fn ratio(name: &str, args: &[V], line: usize) -> Result<f64, Exc> {
        let x = one_number(name, args, line)?;
        if !(-1.0..=1.0).contains(&x) {
            return raise_error("ValueError", format!("`{name}` needs a number from -1 to 1, and this is {}", float_text(x)), line);
        }
        Ok(x)
    }

    fn above_zero(name: &str, args: &[V], line: usize) -> Result<f64, Exc> {
        let x = one_number(name, args, line)?;
        if x <= 0.0 {
            return raise_error("ValueError", format!("cannot take the logarithm of {}", float_text(x)), line);
        }
        Ok(x)
    }

    /// A whole float as an int; infinity and NaN stay as they are.
    /// What `floor` and `ceil` give is an int: one too big for an int is
    /// the biggest int there is, and what is not a number is 0.
    fn whole_of(x: f64) -> V {
        V::Int(x as i64)
    }

    pub fn b_math_sqrt(args: Vec<V>, line: usize) -> R {
        let x = one_number("sqrt", &args, line)?;
        if x < 0.0 {
            return raise_error("ValueError", format!("cannot take the square root of {}", float_text(x)), line);
        }
        Ok(V::Float(x.sqrt()))
    }

    pub fn b_math_floor(args: Vec<V>, line: usize) -> R {
        Ok(whole_of(one_number("floor", &args, line)?.floor()))
    }

    pub fn b_math_ceil(args: Vec<V>, line: usize) -> R {
        Ok(whole_of(one_number("ceil", &args, line)?.ceil()))
    }

    pub fn b_math_round(args: Vec<V>, line: usize) -> R {
        Ok(V::Int(one_number("round", &args, line)?.round() as i64))
    }

    pub fn b_math_sin(args: Vec<V>, line: usize) -> R {
        Ok(V::Float(angle("sin", &args, line)?.sin()))
    }

    pub fn b_math_cos(args: Vec<V>, line: usize) -> R {
        Ok(V::Float(angle("cos", &args, line)?.cos()))
    }

    pub fn b_math_tan(args: Vec<V>, line: usize) -> R {
        Ok(V::Float(angle("tan", &args, line)?.tan()))
    }

    pub fn b_math_asin(args: Vec<V>, line: usize) -> R {
        Ok(q(ratio("asin", &args, line)?.asin(), "rad"))
    }

    pub fn b_math_acos(args: Vec<V>, line: usize) -> R {
        Ok(q(ratio("acos", &args, line)?.acos(), "rad"))
    }

    pub fn b_math_atan(args: Vec<V>, line: usize) -> R {
        Ok(q(one_number("atan", &args, line)?.atan(), "rad"))
    }

    pub fn b_math_atan2(args: Vec<V>, line: usize) -> R {
        arity("atan2", &args, 2, 2, line)?;
        for a in &args {
            if !is_number(a) {
                return wrong_type("atan2", "two numbers", a, line);
            }
        }
        Ok(q(number(&args[0]).atan2(number(&args[1])), "rad"))
    }

    pub fn b_math_exp(args: Vec<V>, line: usize) -> R {
        Ok(V::Float(one_number("exp", &args, line)?.exp()))
    }

    pub fn b_math_log(args: Vec<V>, line: usize) -> R {
        Ok(V::Float(above_zero("log", &args, line)?.ln()))
    }

    pub fn b_math_log2(args: Vec<V>, line: usize) -> R {
        Ok(V::Float(above_zero("log2", &args, line)?.log2()))
    }

    pub fn b_math_log10(args: Vec<V>, line: usize) -> R {
        Ok(V::Float(above_zero("log10", &args, line)?.log10()))
    }

    pub fn b_time_sleep(args: Vec<V>, line: usize) -> R {
        arity("sleep", &args, 1, 1, line)?;
        let V::Quantity(x, u) = &args[0] else {
            return wrong_type("sleep", "a time with a unit, such as `500 ms`", &args[0], line);
        };
        let Some(scale) = convert(u, "s") else {
            return fatal(format!("`sleep` needs a time, such as `500 ms`, not a quantity in {u}"), line);
        };
        let seconds = apply(scale, *x);
        if seconds > 0.0 {
            flush();
            // Until the clock `now` reads says the time has passed: a host
            // may wake a program a little early (Node's WASI does, by a
            // fraction of a millisecond), and `now() - start` after
            // `sleep(20 ms)` must be at least 20 ms, as in `zeno run`.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds.min(1e7));
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                std::thread::sleep(deadline - now);
            }
        }
        Ok(V::None)
    }

    pub fn b_time_now(args: Vec<V>, line: usize) -> R {
        arity("now", &args, 0, 0, line)?;
        Ok(q(STARTED.with(|t| t.elapsed().as_secs_f64() * 1000.0), "ms"))
    }

    /// What is at a path: nothing, a file that can be read, or something
    /// else (a folder, a file that may not be read). It opens the path and
    /// reads one byte, since some systems open a folder and only fail
    /// when it is read.
    #[derive(PartialEq)]
    enum At {
        Nothing,
        File,
        Other,
    }

    fn at(path: &str) -> At {
        use std::io::Read;
        // A system may say something else of a path with nothing in it.
        if path.is_empty() {
            return At::Nothing;
        }
        match std::fs::File::open(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => At::Nothing,
            Err(_) => At::Other,
            Ok(mut file) => match file.read(&mut [0u8; 1]) {
                Ok(_) => At::File,
                Err(_) => At::Other,
            },
        }
    }

    pub fn b_files_read(args: Vec<V>, line: usize) -> R {
        let path = paths("read", &args, 1, line)?.remove(0);
        match at(&path) {
            At::Nothing => return file_error(format!("there is no file `{path}`"), line),
            At::Other => return file_error(format!("cannot read `{path}`"), line),
            At::File => {}
        }
        match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(t) if !t.contains('\0') => Ok(st(t)),
                _ => file_error(format!("`{path}` does not hold text"), line),
            },
            Err(_) => file_error(format!("cannot read `{path}`"), line),
        }
    }

    fn put(name: &str, args: &[V], append: bool, line: usize) -> R {
        let p = paths(name, args, 2, line)?;
        let file = std::fs::OpenOptions::new().write(true).create(true).append(append).truncate(!append).open(&p[0]);
        match file.and_then(|mut f| f.write_all(p[1].as_bytes())) {
            Ok(()) => Ok(V::None),
            Err(_) => file_error(format!("cannot write `{}`", p[0]), line),
        }
    }

    pub fn b_files_write(args: Vec<V>, line: usize) -> R {
        put("write", &args, false, line)
    }

    pub fn b_files_read_bytes(args: Vec<V>, line: usize) -> R {
        let path = paths("read_bytes", &args, 1, line)?.remove(0);
        match at(&path) {
            At::Nothing => return file_error(format!("there is no file `{path}`"), line),
            At::Other => return file_error(format!("cannot read `{path}`"), line),
            At::File => {}
        }
        match std::fs::read(&path) {
            Ok(bytes) => Ok(new_list(bytes.into_iter().map(|b| V::Int(b as i64)).collect())),
            Err(_) => file_error(format!("cannot read `{path}`"), line),
        }
    }

    pub fn b_files_write_bytes(args: Vec<V>, line: usize) -> R {
        arity("write_bytes", &args, 2, 2, line)?;
        let path = text("write_bytes", &args[0], line)?;
        let V::List(items) = &args[1] else {
            return wrong_type("write_bytes", "a list[u8]", &args[1], line);
        };
        let mut bytes = Vec::with_capacity(items.borrow().len());
        for item in items.borrow().iter() {
            match item {
                V::Int(n) if (0..=255).contains(n) => bytes.push(*n as u8),
                V::Int(n) => {
                    return raise_error("ValueError", format!("{n} is not a byte: a byte is a number from 0 to 255"), line)
                }
                other => return wrong_type("write_bytes", "a list[u8]", other, line),
            }
        }
        match std::fs::write(&*path, &bytes) {
            Ok(()) => Ok(V::None),
            Err(_) => file_error(format!("cannot write `{path}`"), line),
        }
    }

    pub fn b_files_append(args: Vec<V>, line: usize) -> R {
        put("append", &args, true, line)
    }

    pub fn b_files_exists(args: Vec<V>, line: usize) -> R {
        let path = paths("exists", &args, 1, line)?.remove(0);
        Ok(V::Bool(at(&path) == At::File))
    }

    pub fn b_files_remove(args: Vec<V>, line: usize) -> R {
        let path = paths("remove", &args, 1, line)?.remove(0);
        match at(&path) {
            At::Nothing => file_error(format!("there is no file `{path}`"), line),
            At::File if std::fs::remove_file(&path).is_ok() => Ok(V::None),
            _ => file_error(format!("cannot remove `{path}`"), line),
        }
    }

    /// `files.list(folder)`: the names in it, sorted by their bytes, which
    /// for UTF-8 is the order of the numbers of their characters.
    pub fn b_files_list(args: Vec<V>, line: usize) -> R {
        let folder = paths("list", &args, 1, line)?.remove(0);
        // A system may say something else of a path with nothing in it:
        // under WASI it is the folder the program runs in.
        if folder.is_empty() {
            return file_error(format!("there is no folder `{folder}`"), line);
        }
        let found = match std::fs::read_dir(&folder) {
            Ok(found) => found,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return file_error(format!("there is no folder `{folder}`"), line)
            }
            Err(_) => return file_error(format!("cannot list the folder `{folder}`"), line),
        };
        let mut names = Vec::new();
        for entry in found {
            let Ok(entry) = entry else {
                return file_error(format!("cannot list the folder `{folder}`"), line);
            };
            match entry.file_name().into_string() {
                Ok(name) => names.push(name),
                Err(_) => return file_error(format!("the folder `{folder}` holds a name that is not text"), line),
            }
        }
        names.sort();
        Ok(new_list(names.iter().map(|name| s(name)).collect()))
    }

    /// `files.make_folder(path)`: it worked if there is a folder there at the end.
    pub fn b_files_make_folder(args: Vec<V>, line: usize) -> R {
        let path = paths("make_folder", &args, 1, line)?.remove(0);
        if !path.is_empty() {
            let _ = std::fs::create_dir_all(&path);
        }
        if !path.is_empty() && std::path::Path::new(&path).is_dir() {
            Ok(V::None)
        } else {
            file_error(format!("cannot make the folder `{path}`"), line)
        }
    }

    /// What a `files.Reader` and a `files.Writer` are made of (decision
    /// 59.6). A file's number is its place in `OPEN_FILES`, plus one; a
    /// closed file leaves its place empty. Files are read as bytes.
    enum OpenFile {
        Reading(std::io::BufReader<std::fs::File>),
        // A `File` holds nothing back, so that a full disk is an error at
        // the write that met it.
        Writing(std::fs::File),
    }

    thread_local! {
        static OPEN_FILES: RefCell<Vec<Option<OpenFile>>> = const { RefCell::new(Vec::new()) };
    }

    /// The place of the open file numbered `args[0]`.
    fn handle(name: &str, args: &[V], line: usize) -> Result<usize, Exc> {
        let V::Int(number) = args[0] else {
            return wrong_type(name, "an int", &args[0], line);
        };
        let open = OPEN_FILES.with(|files| {
            let files = files.borrow();
            number >= 1 && number as u64 <= files.len() as u64 && files[(number - 1) as usize].is_some()
        });
        if !open {
            return fatal(format!("there is no open file numbered {number}"), line);
        }
        Ok((number - 1) as usize)
    }

    pub fn b_files_handle_open(args: Vec<V>, line: usize) -> R {
        arity("handle_open", &args, 2, 2, line)?;
        let path = text("handle_open", &args[0], line)?;
        let how = if let V::Int(n) = args[1] { n } else { 0 };
        let open = if how == 0 {
            match at(&path) {
                At::Nothing => return file_error(format!("there is no file `{path}`"), line),
                At::Other => return file_error(format!("cannot read `{path}`"), line),
                At::File => {}
            }
            match std::fs::File::open(&*path) {
                Ok(file) => OpenFile::Reading(std::io::BufReader::new(file)),
                Err(_) => return file_error(format!("cannot read `{path}`"), line),
            }
        } else {
            let file = std::fs::OpenOptions::new().write(true).create(true).append(how == 2).truncate(how != 2).open(&*path);
            match file {
                Ok(file) if !path.is_empty() => OpenFile::Writing(file),
                _ => return file_error(format!("cannot write `{path}`"), line),
            }
        };
        let number = OPEN_FILES.with(|files| {
            let mut files = files.borrow_mut();
            match files.iter().position(|f| f.is_none()) {
                Some(place) => {
                    files[place] = Some(open);
                    place + 1
                }
                None => {
                    files.push(Some(open));
                    files.len()
                }
            }
        });
        Ok(V::Int(number as i64))
    }

    /// The next line without its line end (a `\r` before the `\n` goes
    /// with it), or none at the end.
    pub fn b_files_handle_line(args: Vec<V>, line: usize) -> R {
        use std::io::BufRead;
        arity("handle_line", &args, 2, 2, line)?;
        let path = text("handle_line", &args[1], line)?;
        let place = handle("handle_line", &args, line)?;
        let mut data = Vec::new();
        let got = OPEN_FILES.with(|files| match &mut files.borrow_mut()[place] {
            Some(OpenFile::Reading(reader)) => Some(reader.read_until(b'\n', &mut data)),
            _ => None,
        });
        match got {
            Some(Ok(0)) => return Ok(V::None),
            Some(Ok(_)) => {}
            _ => return file_error(format!("cannot read `{path}`"), line),
        }
        if data.last() == Some(&b'\n') {
            data.pop();
            if data.last() == Some(&b'\r') {
                data.pop();
            }
        }
        match String::from_utf8(data) {
            Ok(t) if !t.contains('\0') => Ok(st(t)),
            _ => file_error(format!("`{path}` does not hold text"), line),
        }
    }

    /// Up to `count` bytes, fewer at the end, none after it.
    pub fn b_files_handle_bytes(args: Vec<V>, line: usize) -> R {
        arity("handle_bytes", &args, 3, 3, line)?;
        let path = text("handle_bytes", &args[1], line)?;
        let count = if let V::Int(n) = args[2] { n.max(0) as u64 } else { 0 };
        let place = handle("handle_bytes", &args, line)?;
        let mut data = Vec::new();
        let got = OPEN_FILES.with(|files| match &mut files.borrow_mut()[place] {
            Some(OpenFile::Reading(reader)) => Some(reader.by_ref().take(count).read_to_end(&mut data)),
            _ => None,
        });
        match got {
            Some(Ok(_)) => Ok(new_list(data.into_iter().map(|b| V::Int(b as i64)).collect())),
            _ => file_error(format!("cannot read `{path}`"), line),
        }
    }

    fn handle_put(name: &str, args: &[V], data: &[u8], path: &str, line: usize) -> R {
        let place = handle(name, args, line)?;
        let done = OPEN_FILES.with(|files| match &mut files.borrow_mut()[place] {
            Some(OpenFile::Writing(file)) => file.write_all(data).and_then(|()| file.flush()).is_ok(),
            _ => false,
        });
        if done {
            Ok(V::None)
        } else {
            file_error(format!("cannot write `{path}`"), line)
        }
    }

    pub fn b_files_handle_write(args: Vec<V>, line: usize) -> R {
        arity("handle_write", &args, 3, 3, line)?;
        let path = text("handle_write", &args[1], line)?;
        let t = text("handle_write", &args[2], line)?;
        handle_put("handle_write", &args, t.as_bytes(), &path, line)
    }

    pub fn b_files_handle_write_bytes(args: Vec<V>, line: usize) -> R {
        arity("handle_write_bytes", &args, 3, 3, line)?;
        let path = text("handle_write_bytes", &args[1], line)?;
        let V::List(items) = &args[2] else {
            return wrong_type("handle_write_bytes", "a list[u8]", &args[2], line);
        };
        let mut bytes = Vec::with_capacity(items.borrow().len());
        for item in items.borrow().iter() {
            match item {
                V::Int(n) if (0..=255).contains(n) => bytes.push(*n as u8),
                V::Int(n) => {
                    return raise_error("ValueError", format!("{n} is not a byte: a byte is a number from 0 to 255"), line)
                }
                other => return wrong_type("handle_write_bytes", "a list[u8]", other, line),
            }
        }
        handle_put("handle_write_bytes", &args, &bytes, &path, line)
    }

    /// The file is closed, and its number free.
    pub fn b_files_handle_close(args: Vec<V>, line: usize) -> R {
        arity("handle_close", &args, 1, 1, line)?;
        let place = handle("handle_close", &args, line)?;
        let open = OPEN_FILES.with(|files| files.borrow_mut()[place].take());
        drop(open);
        Ok(V::None)
    }

    /// `program.exit(code)`: what was printed goes out, and the program
    /// ends with `code`.
    pub fn b_program_exit(args: Vec<V>, line: usize) -> R {
        arity("exit", &args, 1, 1, line)?;
        let code = whole_arg("exit", &args[0], line)?;
        match u8::try_from(code) {
            Ok(status) => {
                flush();
                let _ = std::io::stderr().flush();
                std::process::exit(i32::from(status))
            }
            Err(_) => raise_error("ValueError", format!("a program ends with a number from 0 to 255, not {code}"), line),
        }
    }

    /// `program.environment(name)`: the text, or none. A name with `=` or
    /// a zero byte in it is in no environment.
    pub fn b_program_environment(args: Vec<V>, line: usize) -> R {
        arity("environment", &args, 1, 1, line)?;
        let name = text("environment", &args[0], line)?;
        if name.is_empty() || name.contains(['=', '\0']) {
            return Ok(V::None);
        }
        Ok(match std::env::var_os(&*name) {
            Some(value) => st(value.to_string_lossy().into_owned()),
            None => V::None,
        })
    }

    /// `program.warn(text)`: one line on the error stream, after what was
    /// printed.
    pub fn b_program_warn(args: Vec<V>, line: usize) -> R {
        arity("warn", &args, 1, 1, line)?;
        let said = text("warn", &args[0], line)?;
        flush();
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "{said}");
        let _ = stderr.flush();
        Ok(V::None)
    }

    pub fn b_program_arguments(args: Vec<V>, line: usize) -> R {
        arity("arguments", &args, 0, 0, line)?;
        Ok(ARGUMENTS.with(|a| strings(a.borrow().iter())))
    }

    pub fn b_json_parse(args: Vec<V>, line: usize) -> R {
        arity("parse", &args, 1, 1, line)?;
        let t = text("parse", &args[0], line)?;
        match json_parse(&t) {
            Some(v) => Ok(v),
            None => {
                let start = t.trim_start_matches([' ', '\t', '\n', '\r']);
                let shown = if start.chars().count() > 30 { format!("{}...", start.chars().take(30).collect::<String>()) } else { start.to_string() };
                raise_error("ValueError", format!("this text is not JSON: `{shown}`"), line)
            }
        }
    }

    pub fn b_json_write(args: Vec<V>, line: usize) -> R {
        arity("write", &args, 1, 1, line)?;
        Ok(st(json_write(&args[0], line)?))
    }

    fn sized(name: &'static str, wrap: bool) -> BuiltinBody {
        let (_, lo, hi, bits, signed) = width(name).unwrap();
        let article = if signed { "an" } else { "a" };
        if !wrap {
            return Rc::new(move |args: Vec<V>, line: usize| {
                arity(name, &args, 1, 1, line)?;
                let n = whole_arg(name, &args[0], line)?;
                if n < lo || n > hi {
                    return raise_error("OverflowError", format!("{n} does not fit in {article} {name} ({lo} to {hi})"), line);
                }
                Ok(V::Int(n))
            });
        }
        Rc::new(move |args: Vec<V>, line: usize| {
            let wrapped = format!("{name}.wrap");
            arity(&wrapped, &args, 1, 1, line)?;
            let n = whole_arg(&wrapped, &args[0], line)?;
            let low = n & ((1i64 << bits) - 1);
            Ok(V::Int(if signed && low > hi { low - (1i64 << bits) } else { low }))
        })
    }

    const ERROR_KINDS: [&str; 7] = ["Error", "ValueError", "IndexError", "KeyError", "ZeroDivisionError", "OverflowError", "FileError"];

    pub fn b_Error(args: Vec<V>, line: usize) -> R {
        new_error("Error", args, line)
    }
    pub fn b_ValueError(args: Vec<V>, line: usize) -> R {
        new_error("ValueError", args, line)
    }
    pub fn b_IndexError(args: Vec<V>, line: usize) -> R {
        new_error("IndexError", args, line)
    }
    pub fn b_KeyError(args: Vec<V>, line: usize) -> R {
        new_error("KeyError", args, line)
    }
    pub fn b_ZeroDivisionError(args: Vec<V>, line: usize) -> R {
        new_error("ZeroDivisionError", args, line)
    }
    pub fn b_OverflowError(args: Vec<V>, line: usize) -> R {
        new_error("OverflowError", args, line)
    }
    pub fn b_FileError(args: Vec<V>, line: usize) -> R {
        new_error("FileError", args, line)
    }

    fn builtin_body(name: &str) -> Option<BuiltinBody> {
        let f: fn(Vec<V>, usize) -> R = match name {
            "print" => b_print,
            "input" => b_input,
            "len" => b_len,
            "range" => b_range,
            "int" => b_int,
            "float" => b_float,
            "f32" => b_f32,
            "u64" => b_u64,
            "u64..of" => b_u64__of,
            "u64..add" => b_u64__add,
            "u64..sub" => b_u64__sub,
            "u64..mul" => b_u64__mul,
            "u64..shl" => b_u64__shl,
            "str" => b_str,
            "abs" => b_abs,
            "min" => b_min,
            "max" => b_max,
            "sum" => b_sum,
            "char" => b_char,
            "allocate" => b_allocate,
            "free" => b_free,
            "math.sqrt" => b_math_sqrt,
            "math.floor" => b_math_floor,
            "math.ceil" => b_math_ceil,
            "math.round" => b_math_round,
            "math.sin" => b_math_sin,
            "math.cos" => b_math_cos,
            "math.tan" => b_math_tan,
            "math.asin" => b_math_asin,
            "math.acos" => b_math_acos,
            "math.atan" => b_math_atan,
            "math.atan2" => b_math_atan2,
            "math.exp" => b_math_exp,
            "math.log" => b_math_log,
            "math.log2" => b_math_log2,
            "math.log10" => b_math_log10,
            "time.sleep" => b_time_sleep,
            "time.now" => b_time_now,
            "files.read" => b_files_read,
            "files.write" => b_files_write,
            "files.read_bytes" => b_files_read_bytes,
            "files.write_bytes" => b_files_write_bytes,
            "files.append" => b_files_append,
            "files.exists" => b_files_exists,
            "files.remove" => b_files_remove,
            "files.list" => b_files_list,
            "files.make_folder" => b_files_make_folder,
            "files.handle_open" => b_files_handle_open,
            "files.handle_line" => b_files_handle_line,
            "files.handle_bytes" => b_files_handle_bytes,
            "files.handle_write" => b_files_handle_write,
            "files.handle_write_bytes" => b_files_handle_write_bytes,
            "files.handle_close" => b_files_handle_close,
            "program.arguments" => b_program_arguments,
            "program.exit" => b_program_exit,
            "program.environment" => b_program_environment,
            "program.warn" => b_program_warn,
            "json.parse" => b_json_parse,
            "json.write" => b_json_write,
            "Error" => b_Error,
            "ValueError" => b_ValueError,
            "IndexError" => b_IndexError,
            "KeyError" => b_KeyError,
            "ZeroDivisionError" => b_ZeroDivisionError,
            "OverflowError" => b_OverflowError,
            "FileError" => b_FileError,
            _ => {
                let (base, wrap) = match name.strip_suffix(".wrap") {
                    Some(base) => (base, true),
                    None => (name, false),
                };
                let (w, ..) = width(base)?;
                return Some(sized(w, wrap));
            }
        };
        Some(Rc::new(f))
    }

    pub fn call_builtin(name: &str, args: Vec<V>, line: usize) -> R {
        match builtin_body(name) {
            Some(f) => f(args, line),
            None => fatal(format!("there is no `{name}` in a program built for Rust"), line),
        }
    }

    thread_local! {
        static BUILTIN_VALUES: RefCell<HashMap<String, V>> = RefCell::new(HashMap::new());
    }

    /// Something that comes with Zeno, as a value: `print`, `math.pi`.
    pub fn builtin(name: &str, line: usize) -> R {
        if name == "math.pi" {
            return Ok(V::Float(std::f64::consts::PI));
        }
        if name == "math.e" {
            return Ok(V::Float(std::f64::consts::E));
        }
        if ERROR_KINDS.contains(&name) {
            return Ok(V::ErrType(Rc::from(name)));
        }
        if let Some(v) = BUILTIN_VALUES.with(|b| b.borrow().get(name).cloned()) {
            return Ok(v);
        }
        let Some(body) = builtin_body(name) else {
            return match name.split_once('.') {
                Some((module, rest)) => fatal(format!("module `{module}` has no `{rest}`"), line),
                None => fatal(format!("`{name}` is not defined"), line),
            };
        };
        let short = name.split_once('.').map_or(name, |(_, rest)| rest);
        let v = V::Func(Rc::new(Func {
            name: short.to_string(),
            parameters: Vec::new(),
            defaults: Vec::new(),
            body: None,
            builtin: name.to_string(),
            bbody: Some(body),
        }));
        BUILTIN_VALUES.with(|b| b.borrow_mut().insert(name.to_string(), v.clone()));
        Ok(v)
    }

    pub fn module(name: &str) -> V {
        V::Module(Rc::from(name))
    }

    // ------------------------------------------------------ The program

    fn place(files: &[&str], line: usize) -> String {
        let index = line / LINES_A_FILE;
        let file = files.get(index).unwrap_or(&files[0]);
        format!(" --> {file}:{}", line % LINES_A_FILE)
    }

    /// Run the program, and say what stopped it if something did. It
    /// runs on a thread with a big stack, so that 1000 nested calls
    /// always fit.
    pub fn start(files: &'static [&'static str], main: fn() -> R) {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let run = move || {
            ARGUMENTS.with(|a| *a.borrow_mut() = arguments);
            STARTED.with(|_| {});
            let result = main();
            flush();
            match result {
                Ok(_) => 0,
                Err(Exc::Raise(e)) => {
                    eprintln!("error: uncaught {}: {}\n{}", shown_name(&e.value.kind), e.value.message, place(files, e.line));
                    1
                }
                Err(Exc::Fatal(message, line)) => {
                    eprintln!("error: {message}\n{}", place(files, line));
                    1
                }
            }
        };
        // WebAssembly has no threads: there the program runs on the one
        // there is, whose stack is made big when it is built.
        let status = if cfg!(target_family = "wasm") {
            run()
        } else {
            std::thread::Builder::new().stack_size(1 << 30).spawn(run).ok().and_then(|w| w.join().ok()).unwrap_or(1)
        };
        std::process::exit(status);
    }
}

use z::V;
