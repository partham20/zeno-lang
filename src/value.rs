//! The values a running Zeno program works with.
//!
//! The parser's tree describes the program. A `Value` is what an expression
//! in that tree turns into when it runs: `1 + 2` becomes `Value::Int(3)`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::ast;
use crate::bytecode::Proto;
use crate::checker::Pointee;
use crate::error::Error;
use crate::ints::Width;
use crate::interpreter::Env;
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
    /// `["a": 1]`: keys with their values.
    Dict(Rc<RefCell<Dict>>),
    /// A pin of the board, by its number: `led`, `pin(7)`
    Pin(i64),
    /// A raw pointer: the block that `allocate` made, and how many items
    /// into it this pointer points.
    Pointer(Rc<Memory>, i64),
    /// A register of the board: its address and its size.
    Register(i64, Width),
    /// An element of the page, for the `web` module: the number the page
    /// knows it by, and how the program got it (`#count`, or `<li>` for
    /// one it made), which is how `print` shows it.
    Element(u32, Rc<str>),
    /// What happened on the page, handed to a function given to `on`
    /// that asks for it: `function(event: Event) { ... }`.
    Event(Rc<Event>),
    /// What `web.after` and `web.every` give: the call that is waiting,
    /// by the number the page knows it by. `timer.stop()` calls it off.
    Timer(u32),

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

/// An event of the page: what kind it was, the element it happened to,
/// where the mouse was and which key was pressed. Its fields can be
/// read and never written (decision 16.2).
pub struct Event {
    /// `"click"`, `"input"`, `"keydown"`: the name `on` was given.
    pub kind: Rc<str>,
    /// The element it happened to. For a click on an `<li>` inside the
    /// `<ul>` that `on` was given to, this is the `<li>`.
    pub target: Value,
    /// Where the mouse was, in pixels from the top left corner of the
    /// element that `on` was given to. 0 for an event of no mouse.
    pub x: f64,
    pub y: f64,
    /// The key that was pressed, for `keydown` and `keyup`. `""` for any
    /// other event.
    pub key: Rc<str>,
}

/// The fields of an `Event`, in the order the checker and `print` show them.
pub const EVENT_FIELDS: &[&str] = &["kind", "target", "x", "y", "key"];

impl Event {
    /// The field of this name, if an event has one.
    pub fn field(&self, name: &str) -> Option<Value> {
        Some(match name {
            "kind" => Value::Str(self.kind.clone()),
            "target" => self.target.clone(),
            "x" => Value::Float(self.x),
            "y" => Value::Float(self.y),
            "key" => Value::Str(self.key.clone()),
            _ => return None,
        })
    }
}

/// What a dict holds: each key with its value, in the order the keys
/// were first put in. That order is what a loop over the dict follows
/// and what `print` shows, so it has to be the same in every engine,
/// and "the order they were put in" is the one everybody can predict.
#[derive(Default)]
pub struct Dict {
    entries: Vec<(Value, Value)>,
    /// Where each key is in `entries`, so that finding one does not
    /// mean looking at them all.
    places: HashMap<Key, usize>,
}

/// What can be a key: text or a whole number.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i64),
    Str(Rc<str>),
}

impl Key {
    /// The key a value is, or `None` for a value that cannot be one.
    pub fn of(value: &Value) -> Option<Key> {
        match value {
            Value::Int(n) => Some(Key::Int(*n)),
            Value::Str(text) => Some(Key::Str(text.clone())),
            _ => None,
        }
    }
}

impl Dict {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every key with its value, in the order the keys were put in.
    pub fn entries(&self) -> &[(Value, Value)] {
        &self.entries
    }

    pub fn get(&self, key: &Key) -> Option<&Value> {
        self.places.get(key).map(|place| &self.entries[*place].1)
    }

    /// Put `value` under `key`. A key that is there already keeps its
    /// place and gets the new value.
    pub fn set(&mut self, key: Key, value: Value) {
        match self.places.get(&key) {
            Some(place) => self.entries[*place].1 = value,
            None => {
                let shown = match &key {
                    Key::Int(n) => Value::Int(*n),
                    Key::Str(text) => Value::Str(text.clone()),
                };
                self.places.insert(key, self.entries.len());
                self.entries.push((shown, value));
            }
        }
    }

    /// Take a key out, with its value. The ones after it move up.
    pub fn remove(&mut self, key: &Key) -> Option<Value> {
        let place = self.places.remove(key)?;
        let (_, value) = self.entries.remove(place);
        for other in self.places.values_mut() {
            if *other > place {
                *other -= 1;
            }
        }
        Some(value)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.places.clear();
    }
}

/// A block of memory made by `allocate`.
///
/// A compiled program has only the bytes. `zeno run` keeps a little more
/// beside them, so that it can say what went wrong: how long the block
/// is, and whether it has been freed.
pub struct Memory {
    pub pointee: Pointee,
    pub items: RefCell<Vec<Value>>,
    pub freed: std::cell::Cell<bool>,
}

/// A function together with the variables around it, so that it can still
/// see them when it is called later.
pub struct Closure {
    pub decl: std::sync::Arc<ast::Function>,
    pub home: Home,
}

/// Where a function keeps the variables around it. Each engine has its own
/// answer, and a program only ever meets the one that is running it.
pub enum Home {
    /// The interpreter: the scope the function was defined in.
    Scope(Env),
    /// The VM: the function's bytecode, and the variables it shares with
    /// the code around it, each in a cell.
    Code(Rc<Proto>, Rc<[Cell]>),
}

/// A variable that more than one function can see. `None` until the line
/// that declares it has run.
pub type Cell = Rc<RefCell<Option<Value>>>;

pub struct ClassDef {
    pub name: String,
    pub fields: Vec<ast::Field>,
    pub methods: HashMap<String, Rc<Closure>>,
    /// What works out the defaults of the fields: for the interpreter the
    /// scope the class was defined in, for the VM the code that builds an
    /// object.
    pub home: Home,
}

pub struct Instance {
    pub class: Rc<ClassDef>,
    /// The values of the fields, in the order the class lists them, so a
    /// field is found by its place and not by hashing its name.
    pub fields: RefCell<Vec<Value>>,
}

impl Instance {
    /// The value of the field called `name`, if the class has one.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.class.slot(name).map(|i| self.fields.borrow()[i].clone())
    }
}

impl ClassDef {
    /// Where the field called `name` is kept in each object of the class.
    #[inline]
    pub fn slot(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|field| field.name == name)
    }
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
    /// When an object of a class is raised, it travels inside an error
    /// whose kind is the class's name and whose message is the object as
    /// text. This is the object.
    pub object: Option<Value>,
}

pub struct Module {
    pub name: &'static str,
    pub members: HashMap<&'static str, Value>,
}

#[derive(Clone, Copy)]
pub struct Builtin {
    pub name: &'static str,
    pub call: fn(&mut dyn Machine, Vec<Value>, Span) -> Result<Value, Control>,
}

/// What a built-in may ask of the engine that is running the program.
///
/// Built-ins are written once and used by both engines, so they cannot
/// know which one called them. This is the little they need from either.
pub trait Machine {
    /// Print one line.
    fn write_line(&mut self, text: &str);

    /// Print text with no line end after it: the question `input` asks.
    fn write(&mut self, text: &str);

    /// The keyboard, and the words the program was started with.
    fn outside(&mut self) -> &mut Outside;

    /// The board the program runs on, for `led.on()` and the like.
    fn board(&mut self) -> &mut crate::board::Board;

    /// The page the program is in, for the `web` module.
    fn page(&mut self) -> &mut crate::page::Page;

    /// Call a Zeno value, as `map` calls the function it was given.
    fn call(
        &mut self,
        callee: Value,
        positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control>;
}

/// What is outside a running program: the keyboard it reads from, and
/// the words it was started with (`zeno run copy.zeno a.txt b.txt`).
/// Both engines carry one, and the built-ins `input` and
/// `program.arguments` ask for it.
#[derive(Default, Clone)]
pub struct Outside {
    /// Text to read in place of the keyboard, which is how the tests
    /// type. `None` reads the keyboard itself.
    pub typed: Option<String>,
    /// How many bytes of `typed` have been read so far.
    read: usize,
    /// The words after the file on the command line.
    pub arguments: Vec<String>,
    /// The page that was written for the program: the text of
    /// `todo.html` for `todo.zeno`, if there is one. The simulated page
    /// then has the elements it has (`page.rs`).
    pub page: Option<String>,
}

impl Outside {
    /// A program at the real keyboard, started with these words.
    pub fn started_with(arguments: Vec<String>) -> Outside {
        Outside { typed: None, read: 0, arguments, page: None }
    }

    /// The same, with the page that was written for the program.
    pub fn with_page(mut self, page: Option<String>) -> Outside {
        self.page = page;
        self
    }

    /// A program that is given `typed` in place of a keyboard.
    #[cfg(test)]
    pub fn typing(typed: &str, arguments: &[&str]) -> Outside {
        Outside {
            typed: Some(typed.to_string()),
            read: 0,
            arguments: arguments.iter().map(|word| word.to_string()).collect(),
            page: None,
        }
    }

    /// The next line that was typed, without its line end. `None` when
    /// there is no more: the keyboard was closed (Ctrl-D), or the file
    /// that was piped in has been read to its end.
    pub fn line(&mut self) -> Option<String> {
        let mut line = match &self.typed {
            Some(typed) => {
                let rest = &typed[self.read..];
                if rest.is_empty() {
                    return None;
                }
                let end = rest.find('\n').map_or(rest.len(), |at| at + 1);
                self.read += end;
                rest[..end].to_string()
            }
            None => {
                use std::io::BufRead;
                let mut bytes = Vec::new();
                match std::io::stdin().lock().read_until(b'\n', &mut bytes) {
                    Ok(0) | Err(_) => return None,
                    Ok(_) => String::from_utf8_lossy(&bytes).into_owned(),
                }
            }
        };
        // The line end is not part of the line: "\n", or the "\r\n" of
        // a file from Windows. Neither is a zero byte, which text in a
        // compiled program cannot hold.
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        line.retain(|c| c != '\0');
        Some(line)
    }
}

/// For the tests: what is typed at the programs a test runs, and the
/// words they are started with. A test sets it once (`with_outside`),
/// and every engine and every built program in that test is given the
/// same, so that they can be held to the same output.
#[cfg(test)]
pub mod testing {
    use super::Outside;
    use std::cell::RefCell;
    use std::io::Write;
    use std::process::{Command, Output, Stdio};

    thread_local! {
        // Nothing typed and no arguments, unless a test says otherwise.
        static OUTSIDE: RefCell<Outside> = RefCell::new(Outside::typing("", &[]));
    }

    /// Run `test` with `typed` at the keyboard of every program it runs,
    /// each started with `arguments`.
    pub fn with_outside(typed: &str, arguments: &[&str], test: impl FnOnce()) {
        OUTSIDE.with(|outside| *outside.borrow_mut() = Outside::typing(typed, arguments));
        test();
        OUTSIDE.with(|outside| *outside.borrow_mut() = Outside::typing("", &[]));
    }

    /// Run `test` with this page written for every program it runs.
    pub fn on_page(html: &str, test: impl FnOnce()) {
        OUTSIDE.with(|outside| outside.borrow_mut().page = Some(html.to_string()));
        test();
        OUTSIDE.with(|outside| *outside.borrow_mut() = Outside::typing("", &[]));
    }

    /// What the interpreter and the VM are given in this test.
    pub fn outside() -> Outside {
        OUTSIDE.with(|outside| outside.borrow().clone())
    }

    /// Run a built program with the same: the arguments on its command
    /// line, and the typed text on its standard input.
    pub fn run(command: &mut Command) -> std::io::Result<Output> {
        let outside = outside();
        let mut child = command
            .args(&outside.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // A program that never reads may be gone before this is written.
        let _ = child.stdin.take().expect("a pipe").write_all(outside.typed.unwrap_or_default().as_bytes());
        child.wait_with_output()
    }
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

/// Print one line of a program's output. If nobody is reading any more,
/// as in `zeno run program.zeno | head -3`, stop quietly, the way other
/// command-line tools do. (`println!` would crash with a long report.)
pub fn print_line(text: &str) {
    // In a browser there is no screen of this kind. The page is told.
    #[cfg(target_arch = "wasm32")]
    crate::web_host::print(&format!("{text}\n"));
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        if writeln!(std::io::stdout(), "{text}").is_err() {
            std::process::exit(0);
        }
    }
}

/// Print text and stay on the line, for the question `input` asks. The
/// text is pushed out at once, so that it is on the screen before the
/// program waits for the answer.
pub fn print_text(text: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::web_host::print(text);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        let mut out = std::io::stdout();
        if write!(out, "{text}").and_then(|_| out.flush()).is_err() {
            std::process::exit(0);
        }
    }
}

/// Stop with a bug report. Returns `Err` so it can be used with `return` or `?`.
pub fn fatal<T>(message: impl Into<String>, span: Span) -> Result<T, Control> {
    Err(Control::Fatal(Error::new(message, span)))
}

/// Raise a built-in error that the program may catch.
pub fn raise<T>(kind: &str, message: impl Into<String>, span: Span) -> Result<T, Control> {
    let error = ErrorValue { kind: kind.to_string(), message: message.into(), object: None };
    Err(Control::Raise(Value::Error(Rc::new(error)), span))
}

impl Value {
    pub fn str(text: impl AsRef<str>) -> Value {
        Value::Str(Rc::from(text.as_ref()))
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(items)))
    }

    pub fn dict(dict: Dict) -> Value {
        Value::Dict(Rc::new(RefCell::new(dict)))
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
            Value::Dict(_) => "a dict".to_string(),
            Value::Pin(_) => "a pin".to_string(),
            Value::Element(..) => "an element".to_string(),
            Value::Event(_) => "an Event".to_string(),
            Value::Timer(_) => "a Timer".to_string(),
            Value::Pointer(..) => "a pointer".to_string(),
            Value::Register(..) => "a register".to_string(),
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
        (Pin(a), Pin(b)) => a == b,
        // Two elements are equal when they are one element of the page.
        (Element(a, _), Element(b, _)) => a == b,
        // An event is equal only to itself.
        (Event(a), Event(b)) => Rc::ptr_eq(a, b),
        // Two timers are equal when they are one call that waits.
        (Timer(a), Timer(b)) => a == b,
        // Two pointers are equal when they point at the same item.
        (Pointer(a, at_a), Pointer(b, at_b)) => Rc::ptr_eq(a, b) && at_a == at_b,
        (Register(a, size_a), Register(b, size_b)) => a == b && size_a == size_b,
        // `1 s == 1000 ms`: compared in the finer unit of the two.
        (Quantity(a, unit_a), Quantity(b, unit_b)) => match crate::units::common(unit_a, unit_b) {
            Some((_, left, right)) => left.apply(*a) == right.apply(*b),
            Option::None => false,
        },
        (List(a), List(b)) => {
            let (a, b) = (a.borrow(), b.borrow());
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| equal(a, b))
        }
        // Two dicts are equal when they hold the same keys with the
        // same values. The order they were put in is not part of it.
        (Dict(a), Dict(b)) => {
            let (a, b) = (a.borrow(), b.borrow());
            a.len() == b.len()
                && a.entries().iter().all(|(key, value)| {
                    Key::of(key).and_then(|key| b.get(&key)).is_some_and(|other| equal(value, other))
                })
        }
        (Instance(a), Instance(b)) => {
            if !Rc::ptr_eq(&a.class, &b.class) {
                return false;
            }
            let (a, b) = (a.fields.borrow(), b.fields.borrow());
            a.iter().zip(b.iter()).all(|(value, other)| equal(value, other))
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
            // A dict is shown the way it is written.
            Value::Dict(dict) if dict.borrow().len() == 0 => write!(f, "[:]"),
            Value::Dict(dict) => {
                let entries: Vec<String> = dict
                    .borrow()
                    .entries()
                    .iter()
                    .map(|(key, value)| format!("{}: {}", key.repr(), value.repr()))
                    .collect();
                write!(f, "[{}]", entries.join(", "))
            }
            Value::Pin(number) => write!(f, "Pin({number})"),
            Value::Element(_, name) => write!(f, "Element({name})"),
            Value::Event(event) => write!(f, "Event({})", event.kind),
            Value::Timer(_) => write!(f, "Timer"),
            // Where a pointer points is different on every run, so it is
            // not shown.
            Value::Pointer(memory, _) => write!(f, "<pointer[{}]>", memory.pointee.name()),
            Value::Register(address, _) => write!(f, "Register(0x{address:08x})"),
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
                    .zip(fields.iter())
                    .map(|(field, value)| format!("{}={}", field.name, value.repr()))
                    .collect();
                // A class of an imported file prints by the name its
                // file gave it: `Point`, not `shapes.Point`.
                write!(f, "{}({})", crate::ast::shown_name(&instance.class.name), shown.join(", "))
            }
            Value::Enum(def) => write!(f, "<enum {}>", def.name),
            Value::CaseConstructor(def, index) => write!(f, "<case {}>", def.cases[*index].name),
            Value::Case(case) if case.fields.is_empty() => write!(f, "{}", crate::ast::shown_name(case.name())),
            Value::Case(case) => {
                let fields: Vec<String> = case.fields.iter().map(Value::repr).collect();
                write!(f, "{}({})", crate::ast::shown_name(case.name()), fields.join(", "))
            }
            Value::ErrorType(kind) => write!(f, "<error {kind}>"),
            Value::Error(error) => write!(f, "{}", error.message),
            Value::Module(module) => write!(f, "<module {}>", module.name),
        }
    }
}
