//! The VM: runs bytecode.
//!
//! "VM" is short for virtual machine: a processor made of software, whose
//! machine code is the bytecode in bytecode.rs. Like a real processor it
//! does one thing for ever: fetch the next instruction, do what it says,
//! go on to the one after. That loop is `execute`, and it is most of this
//! file.
//!
//! It has three pieces of memory.
//!
//! - **The stack**, one long row of values. Instructions push values on
//!   the end and pop them off. Each function that is running owns a
//!   stretch of it: its parameters first, then its other variables, then
//!   whatever it is in the middle of working out.
//! - **The frames**, one for each function that is running: which
//!   function, how far through it, and where its stretch of the stack
//!   begins. Calling a function adds a frame. Returning removes it.
//! - **The handlers**, one for each `try` the program is inside: where to
//!   jump if an error is raised, and how to put the other two back.
//!
//! What each instruction *means* is not decided here. `Binary +` calls
//! the same `ops::binary` the interpreter calls, and `print` is the same
//! built-in. The VM is only a different way of getting from one step to
//! the next, so a program does the same thing under either.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{BinaryOp, Program};
use crate::board::Board;
use crate::builtins;
use crate::bytecode::{From, Op, Proto};
use crate::emit_bytecode;
use crate::error::{counted, Error};
use crate::ops;
use crate::token::Span;
use crate::value::*;

/// How deep function calls may nest before we assume runaway recursion.
/// The same limit as the interpreter's.
const MAX_DEPTH: usize = 1000;
/// A call made by the VM does not use the Rust stack: the frames are in a
/// list. A built-in that calls back into Zeno, as `map` does, is the
/// exception, so the VM runs on a thread with room for a deep pile of those.
const STACK_SIZE: usize = 256 * 1024 * 1024;

/// Compile a program to bytecode and run it. With `capture` the printed
/// text is returned instead of going to the screen, which is how the
/// tests check output.
/// `outside` is what is typed at the program, and the words after the
/// file on the command line.
pub fn run_with(program: &Program, capture: bool, outside: Outside) -> (String, Result<(), Error>) {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || {
                let main = emit_bytecode::compile(program);
                let mut vm = Vm {
                    output: capture.then(String::new),
                    board: Board::new(),
                    outside,
                    // Under every frame's stretch of the stack is the thing
                    // that was called. The program was called by nobody.
                    stack: vec![Value::None],
                    frames: Vec::new(),
                    handlers: Vec::new(),
                    depth: 0,
                    raised_at: Span { line: 1, col: 1 },
                };
                let nothing: Rc<[Cell]> = Rc::from(Vec::new());
                vm.frames.push(Frame::new(main, nothing, 1, Vec::new(), Span { line: 1, col: 1 }));
                let result = vm.run(0).map(|_| ()).map_err(ops::escaped);
                (vm.output.unwrap_or_default(), result)
            })
            .expect("could not start the VM thread")
            .join()
            .expect("the VM crashed")
    })
}

/// For the tests: run a program with both engines and insist that they
/// agree about everything, which is what was printed and, if the program
/// stopped with an error, its message and the place it points at. Every
/// test that runs a program goes through here, so every test program in
/// the project is also a test of the VM.
#[cfg(test)]
pub fn run_both(program: &Program, source: &str) -> (String, Result<(), Error>) {
    // What is typed at the program, and the words it is started with,
    // are the test's to say (`with_outside`). By default nothing is.
    let outside = crate::value::testing::outside();
    let interpreted = crate::interpreter::run_with(program, true, outside.clone());
    let on_vm = run_with(program, true, outside);
    assert_eq!(
        interpreted, on_vm,
        "the interpreter (left) and the VM (right) disagree about this program:\n{source}"
    );
    interpreted
}

struct Vm {
    /// `Some` collects printed text; `None` prints straight to the screen.
    output: Option<String>,
    /// The simulated board, for the `board` module.
    board: Board,
    /// The keyboard and the command line, for `input` and `program`.
    outside: Outside,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    handlers: Vec<Handler>,
    /// How many function calls are currently in progress.
    depth: usize,
    /// Where the error that was last caught had been raised, in case no
    /// `except` wants it and it has to go on its way (`Op::Reraise`).
    raised_at: Span,
}

/// One function that is running.
struct Frame {
    proto: Rc<Proto>,
    /// The cells it took with it from the functions around it.
    captured: Rc<[Cell]>,
    /// Its own cells: the variables that functions written inside it use.
    cells: Vec<Cell>,
    /// The instruction it will run next.
    ip: usize,
    /// Where its stretch of the stack begins: slot 0 is `stack[base]`.
    base: usize,
    /// Which parameters the caller left out. Empty if it gave them all.
    missing: Vec<bool>,
    /// Where it was called from, for an error about the call itself.
    called_at: Span,
    /// True once the call has been counted in `depth` (`Op::Enter`).
    counted: bool,
}

impl Frame {
    fn new(proto: Rc<Proto>, captured: Rc<[Cell]>, base: usize, missing: Vec<bool>, called_at: Span) -> Frame {
        // The cells are replaced by `NewCell` before they are used. These
        // only make the list the right length.
        let cells = vec![Rc::new(RefCell::new(None)); proto.cells.len()];
        Frame { proto, captured, cells, ip: 0, base, missing, called_at, counted: false }
    }
}

/// A `try` the program is inside.
struct Handler {
    /// Which frame the `try` is in. Every frame above it is abandoned
    /// when an error comes here.
    frame: usize,
    /// How tall the stack was at the `try`.
    height: usize,
    /// How many calls were in progress.
    depth: usize,
    /// The instruction the `except` blocks start at.
    to: usize,
}

/// How a call turned out.
enum Called {
    /// It has an answer already: a built-in ran, or an enum value was made.
    Done(Value),
    /// A frame was added for a function written in Zeno. Its answer comes
    /// when the loop has run it.
    Entered,
}

/// What the built-ins ask of the VM (see `Machine` in value.rs).
impl Machine for Vm {
    fn write_line(&mut self, text: &str) {
        match &mut self.output {
            Some(buffer) => {
                buffer.push_str(text);
                buffer.push('\n');
            }
            None => print_line(text),
        }
    }

    fn write(&mut self, text: &str) {
        match &mut self.output {
            Some(buffer) => buffer.push_str(text),
            None => print_text(text),
        }
    }

    fn outside(&mut self) -> &mut Outside {
        &mut self.outside
    }

    fn board(&mut self) -> &mut Board {
        &mut self.board
    }

    /// A built-in such as `map` calls a Zeno function. The instruction
    /// loop is already running further down the Rust stack, in the middle
    /// of the instruction that called the built-in. So a second loop is
    /// started here, which runs until the function called has returned.
    fn call(
        &mut self,
        callee: Value,
        positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Value, Control> {
        let frames = self.frames.len();
        match self.invoke(callee, positional, named, span)? {
            Called::Done(value) => Ok(value),
            Called::Entered => self.run(frames),
        }
    }
}

impl Vm {
    /// Run until only `stop` frames are left, which is when the function
    /// in frame number `stop` returns. The result is what it returned.
    fn run(&mut self, stop: usize) -> Result<Value, Control> {
        loop {
            match self.execute(stop) {
                // An error was raised. If a `try` is waiting for it, carry
                // on from its `except` blocks.
                Err(Control::Raise(error, span)) => self.catch(error, span, stop)?,
                finished => return finished,
            }
        }
    }

    /// Send a raised error to the nearest `try`.
    ///
    /// If that `try` belongs to a loop further down the Rust stack (see
    /// `call` above), or there is none, the error is passed on as it is.
    fn catch(&mut self, error: Value, span: Span, stop: usize) -> Result<(), Control> {
        if !self.handlers.last().is_some_and(|handler| handler.frame >= stop) {
            return Err(Control::Raise(error, span));
        }
        let handler = self.handlers.pop().expect("a handler");
        // Put everything back as it was at the `try`...
        self.frames.truncate(handler.frame + 1);
        self.stack.truncate(handler.height);
        self.depth = handler.depth;
        // ...and go to its `except` blocks with the error on top.
        self.frames[handler.frame].ip = handler.to;
        self.stack.push(error);
        self.raised_at = span;
        Ok(())
    }

    /// The loop: fetch an instruction, do it, repeat.
    fn execute(&mut self, stop: usize) -> Result<Value, Control> {
        /// Take the top value off the stack.
        macro_rules! pop {
            () => {
                self.stack.pop().expect("the compiler left a value on the stack")
            };
        }

        loop {
            let frame = self.frames.last_mut().expect("a function is running");
            let op = frame.proto.code[frame.ip];
            let span = frame.proto.spans[frame.ip];
            frame.ip += 1;
            let base = frame.base;

            match op {
                // --------------------------------------- Values on the stack
                Op::Const(n) => {
                    let value = frame.proto.constants[n as usize].clone();
                    self.stack.push(value);
                }
                Op::Pop(n) => {
                    let left = self.stack.len() - n as usize;
                    self.stack.truncate(left);
                }
                Op::PopTo(n) => self.stack.truncate(base + n as usize),

                // ------------------------------------------------- Variables
                Op::GetLocal(n) => {
                    let value = self.stack[base + n as usize].clone();
                    self.stack.push(value);
                }
                Op::SetLocal(n) => self.stack[base + n as usize] = pop!(),
                Op::NewCell(n) => frame.cells[n as usize] = Rc::new(RefCell::new(None)),
                Op::DefineCell(n) => *frame.cells[n as usize].borrow_mut() = Some(pop!()),
                Op::GetCell(n) => match &*frame.cells[n as usize].borrow() {
                    Some(value) => self.stack.push(value.clone()),
                    None => return not_defined(&frame.proto.cells[n as usize], span),
                },
                Op::SetCell(n) => match &mut *frame.cells[n as usize].borrow_mut() {
                    Some(value) => *value = pop!(),
                    None => return not_declared(&frame.proto.cells[n as usize], span),
                },
                Op::GetCaptured(n) => match &*frame.captured[n as usize].borrow() {
                    Some(value) => self.stack.push(value.clone()),
                    None => return not_defined(&frame.proto.captures[n as usize].name, span),
                },
                Op::SetCaptured(n) => match &mut *frame.captured[n as usize].borrow_mut() {
                    Some(value) => *value = pop!(),
                    None => return not_declared(&frame.proto.captures[n as usize].name, span),
                },

                // ------------------------------------------------- Operators
                Op::Unary(op) => {
                    let value = pop!();
                    self.stack.push(ops::unary(op, value, span)?);
                }
                Op::Binary(op) => {
                    let right = pop!();
                    let left = pop!();
                    self.stack.push(ops::binary(op, left, right, span)?);
                }
                Op::And(to) => {
                    // false and ... is false: the false stays as the answer.
                    if ops::logic_operand(BinaryOp::And, top(&self.stack).clone(), span)? {
                        self.stack.pop();
                    } else {
                        frame.ip = to as usize;
                    }
                }
                Op::Or(to) => {
                    // true or ... is true.
                    if ops::logic_operand(BinaryOp::Or, top(&self.stack).clone(), span)? {
                        frame.ip = to as usize;
                    } else {
                        self.stack.pop();
                    }
                }
                Op::CheckBool(op) => {
                    ops::logic_operand(op, top(&self.stack).clone(), span)?;
                }
                Op::Chain(op, to) => {
                    let right = pop!();
                    let left = pop!();
                    match ops::binary(op, left, right.clone(), span)? {
                        Value::Bool(true) => self.stack.push(right),
                        _ => frame.ip = to as usize,
                    }
                }

                // --------------------------------------------- Making values
                Op::List(n) => {
                    let first = self.stack.len() - n as usize;
                    let items = self.stack.split_off(first);
                    self.stack.push(Value::list(items));
                }
                Op::Dict(n) => {
                    let first = self.stack.len() - 2 * n as usize;
                    let mut dict = Dict::default();
                    let mut pairs = self.stack.split_off(first).into_iter();
                    while let (Some(key), Some(value)) = (pairs.next(), pairs.next()) {
                        dict.set(ops::key(&key, span)?, value);
                    }
                    self.stack.push(Value::dict(dict));
                }
                Op::Append(slot) => {
                    let value = pop!();
                    match &self.stack[base + slot as usize] {
                        Value::List(items) => items.borrow_mut().push(value),
                        _ => unreachable!("`Append` is only written for a list the compiler made"),
                    }
                }
                Op::Format => {
                    let value = pop!();
                    self.stack.push(Value::str(builtins::format(&value, None, span)?));
                }
                Op::FormatWith(n) => {
                    let value = pop!();
                    let spec = text(&frame.proto, n);
                    self.stack.push(Value::str(builtins::format(&value, Some(spec), span)?));
                }
                Op::Text(n) => {
                    let first = self.stack.len() - n as usize;
                    let mut joined = String::new();
                    for piece in self.stack.drain(first..) {
                        if let Value::Str(piece) = piece {
                            joined += &piece;
                        }
                    }
                    self.stack.push(Value::str(joined));
                }
                Op::Unit(n) => {
                    let value = pop!();
                    self.stack.push(ops::quantity(value, text(&frame.proto, n), span)?);
                }

                // ------------------------------------------ Fields and items
                Op::GetField(n) => {
                    let object = pop!();
                    self.stack.push(ops::field(object, text(&frame.proto, n), span)?);
                }
                Op::SetField(n) => {
                    let object = pop!();
                    let value = pop!();
                    ops::set_field(object, text(&frame.proto, n), value, span)?;
                }
                Op::GetIndex => {
                    let index = pop!();
                    let object = pop!();
                    self.stack.push(ops::item(&object, &index, span)?);
                }
                Op::SetIndex => {
                    let index = pop!();
                    let object = pop!();
                    let value = pop!();
                    ops::set_item(object, &index, value, span)?;
                }

                // ----------------------------------------------------- Jumps
                Op::Jump(to) => frame.ip = to as usize,
                Op::JumpIfFalse(to) => {
                    if !ops::condition(pop!(), span)? {
                        frame.ip = to as usize;
                    }
                }

                // ----------------------------------------------------- Loops
                Op::ForStart => {
                    let value = pop!();
                    self.stack.push(Value::list(ops::items(&value, span)?));
                }
                Op::ForNext(to) => {
                    let top = self.stack.len() - 1;
                    let (Value::List(items), Value::Int(position)) = (&self.stack[top - 1], &self.stack[top])
                    else {
                        unreachable!("`ForNext` runs on what `ForStart` left");
                    };
                    let position = *position;
                    let item = items.borrow().get(position as usize).cloned();
                    match item {
                        Some(item) => {
                            self.stack[top] = Value::Int(position + 1);
                            self.stack.push(item);
                        }
                        None => frame.ip = to as usize,
                    }
                }

                Op::ForStartPairs => {
                    let value = pop!();
                    let (keys, values): (Vec<Value>, Vec<Value>) = ops::pairs(&value, span)?.into_iter().unzip();
                    self.stack.push(Value::list(values));
                    self.stack.push(match value {
                        Value::Dict(_) => Value::list(keys),
                        _ => Value::None,
                    });
                }
                Op::ForNextPair(to) => {
                    let top = self.stack.len() - 1;
                    let (Value::List(seconds), Value::Int(position)) = (&self.stack[top - 2], &self.stack[top]) else {
                        unreachable!("`ForNextPair` runs on what `ForStartPairs` left");
                    };
                    let position = *position;
                    if position as usize >= seconds.borrow().len() {
                        frame.ip = to as usize;
                    } else {
                        let first = match &self.stack[top - 1] {
                            Value::List(keys) => keys.borrow()[position as usize].clone(),
                            _ => Value::Int(position),
                        };
                        self.stack[top] = Value::Int(position + 1);
                        self.stack.push(first);
                    }
                }
                Op::ForSecond(below) => {
                    let at = self.stack.len() - 1 - below as usize;
                    let (Value::List(seconds), Value::Int(position)) = (&self.stack[at - 2], &self.stack[at]) else {
                        unreachable!("`ForSecond` runs on what `ForStartPairs` left");
                    };
                    let second = seconds.borrow()[*position as usize - 1].clone();
                    self.stack.push(second);
                }

                // ----------------------------------------------------- match
                Op::IsCase(name, count) => {
                    let name = text(&frame.proto, name);
                    let Value::CaseConstructor(def, index) = pop!() else {
                        return fatal(format!("`{name}` is not an enum case that carries values"), span);
                    };
                    let fits = match top(&self.stack) {
                        Value::Case(case) if Rc::ptr_eq(&case.def, &def) && case.index == index => {
                            if case.fields.len() != count as usize {
                                return fatal(
                                    format!(
                                        "`{name}` carries {}, but this pattern has {count}",
                                        counted(case.fields.len(), "value")
                                    ),
                                    span,
                                );
                            }
                            true
                        }
                        _ => false,
                    };
                    self.stack.push(Value::Bool(fits));
                }
                Op::CaseField(n) => match pop!() {
                    Value::Case(case) => self.stack.push(case.fields[n as usize].clone()),
                    _ => unreachable!("`CaseField` follows an `IsCase` that said yes"),
                },
                Op::NoMatch => {
                    return fatal(format!("no `case` matches {}", top(&self.stack).repr()), span);
                }

                // --------------------------- Functions, classes and enums
                Op::Closure(n) => {
                    let closure = closure(frame, n);
                    self.stack.push(Value::Function(closure));
                }
                Op::Class(n) => {
                    let code = &frame.proto.classes[n as usize];
                    let methods = code
                        .methods
                        .iter()
                        .map(|method| {
                            let closure = closure(frame, *method);
                            (closure.decl.name.clone(), closure)
                        })
                        .collect();
                    let constructor = frame.proto.functions[code.constructor as usize].clone();
                    let captured = capture(frame, &constructor);
                    let class = ClassDef {
                        name: code.name.clone(),
                        fields: code.fields.clone(),
                        methods,
                        home: Home::Code(constructor, captured),
                    };
                    self.stack.push(Value::Class(Rc::new(class)));
                }
                Op::Enum(n) => {
                    let decl = &frame.proto.enums[n as usize];
                    let def = EnumDef { name: decl.name.clone(), cases: decl.variants.clone() };
                    self.stack.push(Value::Enum(Rc::new(def)));
                }
                Op::EnumCase(n) => match pop!() {
                    Value::Enum(def) => self.stack.push(ops::case_value(&def, n as usize)),
                    _ => unreachable!("`EnumCase` follows the enum it belongs to"),
                },

                Op::Call(count) => self.call(count as usize, &[], span)?,
                Op::CallNamed(count, names) => {
                    let names = frame.proto.names[names as usize].clone();
                    self.call(count as usize, &names, span)?;
                }
                Op::Default(slot, to) => {
                    if !frame.missing.get(slot as usize).copied().unwrap_or(false) {
                        frame.ip = to as usize;
                    }
                }
                Op::Required(slot, message) => {
                    if frame.missing.get(slot as usize).copied().unwrap_or(false) {
                        return fatal(text(&frame.proto, message), frame.called_at);
                    }
                }
                Op::Enter => {
                    frame.counted = true;
                    self.depth += 1;
                }
                Op::Object(n) => {
                    let first = self.stack.len() - n as usize;
                    let values = self.stack.split_off(first);
                    // The class that was called is just under this frame's
                    // stretch of the stack.
                    let Value::Class(class) = self.stack[base - 1].clone() else {
                        unreachable!("`Object` runs in the code that builds an object of a class");
                    };
                    let fields = class.fields.iter().map(|field| field.name.clone()).zip(values).collect();
                    self.stack.push(Value::Instance(Rc::new(Instance { class, fields: RefCell::new(fields) })));
                }
                Op::Return => {
                    let value = pop!();
                    let finished = self.frames.pop().expect("a function is running");
                    if finished.counted {
                        self.depth -= 1;
                    }
                    // A `return` from inside a `try` leaves the `try` too.
                    while self.handlers.last().is_some_and(|handler| handler.frame >= self.frames.len()) {
                        self.handlers.pop();
                    }
                    // Its stretch of the stack goes, and the thing that was
                    // called with it. The result takes their place.
                    self.stack.truncate(finished.base - 1);
                    if self.frames.len() == stop {
                        return Ok(value);
                    }
                    self.stack.push(value);
                }

                // ---------------------------------------------------- Errors
                Op::Try(to) => self.handlers.push(Handler {
                    frame: self.frames.len() - 1,
                    height: self.stack.len(),
                    depth: self.depth,
                    to: to as usize,
                }),
                Op::EndTry => {
                    self.handlers.pop();
                }
                Op::Except(kind, to) => {
                    let kind = text(&frame.proto, kind);
                    if !ops::catches(Some(kind), &ops::error_kind(top(&self.stack))) {
                        frame.ip = to as usize;
                    }
                }
                Op::Unwrap => {
                    let error = pop!();
                    self.stack.push(ops::raised_object(&error));
                }
                Op::Raise => return Err(ops::raised(pop!(), span)),
                Op::Reraise => return Err(Control::Raise(pop!(), self.raised_at)),
                Op::Fatal(message) => return fatal(text(&frame.proto, message), span),
            }
        }
    }

    // --------------------------------------------------------------- Calls

    /// `Op::Call`: the stack holds the thing to call and `count` arguments,
    /// the last of which have the names in `names`.
    fn call(&mut self, count: usize, names: &[String], span: Span) -> Result<(), Control> {
        let callee = self.stack.len() - count - 1;

        // The usual call: a Zeno function, given exactly its parameters
        // and none by name. The arguments are already where the function's
        // first slots will be, so nothing is moved. A new frame is put
        // over them and that is the whole call.
        if let (Value::Function(closure), true) = (&self.stack[callee], names.is_empty()) {
            if let Home::Code(proto, captured) = &closure.home {
                if proto.params.len() == count {
                    if self.depth >= MAX_DEPTH {
                        return too_deep(&proto.name, span);
                    }
                    let frame = Frame::new(proto.clone(), captured.clone(), callee + 1, Vec::new(), span);
                    self.frames.push(frame);
                    return Ok(());
                }
            }
        }

        // Every other call: take the arguments off the stack and sort
        // them out.
        let mut positional = self.stack.split_off(callee + 1);
        let named = positional.split_off(count - names.len());
        let named = names.iter().cloned().zip(named).collect();
        let callee = self.stack.pop().expect("the thing to call");
        if let Called::Done(value) = self.invoke(callee, positional, named, span)? {
            self.stack.push(value);
        }
        Ok(())
    }

    /// Call any value with any arguments. Nothing of the call is on the
    /// stack when this begins.
    fn invoke(
        &mut self,
        callee: Value,
        mut positional: Vec<Value>,
        named: Vec<(String, Value)>,
        span: Span,
    ) -> Result<Called, Control> {
        let (proto, captured) = match &callee {
            Value::Function(closure) => code(&closure.home),
            Value::BoundMethod(instance, closure) => {
                // `p.distance_to(q)` passes `p` as the first argument, `self`.
                positional.insert(0, Value::Instance(instance.clone()));
                code(&closure.home)
            }
            // A class is called like a function whose parameters are its
            // fields. The code is the class's constructor.
            Value::Class(class) => code(&class.home),

            Value::Builtin(builtin) => {
                ops::no_named(builtin.name, &named, span)?;
                return Ok(Called::Done((builtin.call)(self, positional, span)?));
            }
            Value::Method(receiver, name) => {
                ops::no_named(name, &named, span)?;
                return Ok(Called::Done(builtins::call_method(self, receiver, name, positional, span)?));
            }
            Value::CaseConstructor(def, index) => {
                return Ok(Called::Done(ops::new_case(def, *index, positional, named, span)?));
            }
            Value::ErrorType(kind) => return Ok(Called::Done(ops::new_error(kind, positional, named, span)?)),
            other => return fatal(format!("{} cannot be called", other.type_name()), span),
        };

        // Building an object is not counted as a call in progress, here
        // or in the interpreter.
        if proto.decl.is_some() && self.depth >= MAX_DEPTH {
            return too_deep(&proto.name, span);
        }
        let names: Vec<&str> = proto.params.iter().map(String::as_str).collect();
        let slots = ops::bind(&proto.name, &names, positional, named, span)?;

        // The frame's stretch of the stack: the thing called, then one
        // slot for each parameter, in order.
        self.stack.push(callee);
        let base = self.stack.len();
        let mut missing = Vec::new();
        for (slot, value) in slots.into_iter().enumerate() {
            match value {
                Some(value) => self.stack.push(value),
                // The function's first instructions fill this in, or
                // complain (`Op::Default`, `Op::Required`).
                None => {
                    missing.resize(proto.params.len(), false);
                    missing[slot] = true;
                    self.stack.push(Value::None);
                }
            }
        }
        self.frames.push(Frame::new(proto, captured, base, missing, span));
        Ok(Called::Entered)
    }
}

/// The value on top of the stack.
fn top(stack: &[Value]) -> &Value {
    stack.last().expect("the compiler left a value on the stack")
}

/// Constant number n of a function, which the compiler made a piece of text.
fn text(proto: &Proto, n: u32) -> &str {
    match &proto.constants[n as usize] {
        Value::Str(text) => text,
        _ => unreachable!("the compiler names things with text"),
    }
}

/// The compiled code of a function or class that the VM made.
fn code(home: &Home) -> (Rc<Proto>, Rc<[Cell]>) {
    match home {
        Home::Code(proto, captured) => (proto.clone(), captured.clone()),
        Home::Scope(_) => unreachable!("the VM met a function that the interpreter made"),
    }
}

/// Make function number n of the running function into a value: its code,
/// joined with the cells it shares with the code around it.
fn closure(frame: &Frame, n: u32) -> Rc<Closure> {
    let proto = frame.proto.functions[n as usize].clone();
    let captured = capture(frame, &proto);
    let decl = proto.decl.clone().expect("a function has a declaration");
    Rc::new(Closure { decl, home: Home::Code(proto, captured) })
}

/// The cells a function takes with it, found in the frame that is making it.
fn capture(frame: &Frame, proto: &Proto) -> Rc<[Cell]> {
    proto
        .captures
        .iter()
        .map(|capture| match capture.from {
            From::Cell(n) => frame.cells[n as usize].clone(),
            From::Captured(n) => frame.captured[n as usize].clone(),
        })
        .collect()
}

fn too_deep<T>(name: &str, span: Span) -> Result<T, Control> {
    fatal(format!("more than {MAX_DEPTH} calls are nested. Is `{name}` calling itself forever?"), span)
}

/// A cell was read before the line that declares its variable had run.
fn not_defined<T>(name: &str, span: Span) -> Result<T, Control> {
    fatal(format!("`{name}` is not defined"), span)
}

fn not_declared<T>(name: &str, span: Span) -> Result<T, Control> {
    fatal(format!("`{name}` is not declared. Create it with `let` or `var` first"), span)
}

#[cfg(test)]
mod tests {
    use crate::{lexer, parser};

    // Every program in the interpreter's tests and the C compiler's tests
    // already runs on the VM as well (see `run_both`). The tests here aim
    // at the places where a VM can go wrong and a tree-walker cannot: a
    // stack left taller or shorter than it was found, a variable shared
    // between functions, and an error that has to unwind several frames.

    /// Run a program on both engines, which must agree. Returns what it
    /// printed and, if it stopped with an error, the message.
    fn run(source: &str) -> (String, Option<String>) {
        let program = parser::parse(lexer::lex(source).expect("lexing failed")).expect("parsing failed");
        let (output, result) = super::run_both(&program, source);
        (output, result.err().map(|error| error.message))
    }

    /// What a working program prints.
    fn output(source: &str) -> String {
        let (output, error) = run(source);
        if let Some(message) = error {
            panic!("program failed: {message}\noutput so far:\n{output}");
        }
        output
    }

    /// The message a failing program stops with.
    fn error(source: &str) -> String {
        run(source).1.expect("the program should have failed")
    }

    #[test]
    fn leaving_loops_from_anywhere_keeps_the_stack_straight() {
        // `break` and `continue` from inside nested blocks, a `match` and
        // a `try`, two hundred times. A value left on the stack by any of
        // them would shift every variable after it.
        let source = r#"
var total = 0
for i in range(200) {
    let double = i * 2
    if i % 3 == 0 { continue }
    for j in [1, 2, 3] {
        let k = j + double
        if k % 7 == 0 { break }
        var n = 0
        while true {
            n += 1
            if n > 2 { break }
            if n == 1 { continue }
            total += k
        }
    }
    match i % 4 {
        case 0 { continue }
        case 1 { total += 1 }
        case _ {
            let extra = 2
            total += extra
        }
    }
    try {
        if i == 150 { break }
        if i % 5 == 0 { raise ValueError("five") }
        total += 1
    } except ValueError as e {
        if i % 10 == 0 { continue }
        total += len(e.message)
    }
}
print(total)
"#;
        assert_eq!(output(source), "57747\n");
    }

    #[test]
    fn functions_share_variables_through_cells() {
        // Each trip round a loop has variables of its own.
        let source = r#"
var makers = []
for i in range(3) {
    var count = i * 10
    makers.append(function() {
        count += 1
        return count + i
    })
}
print(makers[0](), makers[0](), makers[2](), makers[1]())
"#;
        assert_eq!(output(source), "1 2 23 12\n");

        // Two functions see one variable, and it outlives the call that made it.
        let source = r#"
function bank() {
    var balance = 0
    function deposit(n) { balance += n }
    function report() { return f"balance {balance}" }
    return [deposit, report]
}
let pair = bank()
pair[0](5)
pair[0](7)
print(pair[1]())

function outer(a) {
    return function(b) {
        return function(c) { return a + b + c }
    }
}
print(outer(1)(2)(3))
"#;
        assert_eq!(output(source), "balance 12\n6\n");
    }

    #[test]
    fn functions_can_be_used_above_where_they_are_written() {
        let source = r#"
print(even(10), odd(7))
function even(n) {
    if n == 0 { return true }
    return odd(n - 1)
}
function odd(n) {
    if n == 0 { return false }
    return even(n - 1)
}
"#;
        assert_eq!(output(source), "true true\n");

        // A function sees what is declared above the line it is written
        // on, even when it is called from further up...
        assert_eq!(output("let a = 1\nprint(f())\nfunction f() { return a }"), "1\n");
        // ...but not before that line has run.
        assert_eq!(error("print(f())\nlet a = 1\nfunction f() { return a }"), "`a` is not defined");
        // And what is declared below it is not the function's business.
        let source = "let x = 1\nfunction outer() {\n function inner() { return x }\n let x = 5\n return inner() + x\n}\nprint(outer())";
        assert_eq!(output(source), "6\n");
        let source = "function size(items) { return len(items) }\nlet len = 5\nprint(size([1, 2]), len)";
        assert_eq!(output(source), "2 5\n");
    }

    #[test]
    fn arguments_are_lined_up_with_parameters() {
        let source = r#"
var made = 0
function fresh() {
    made += 1
    return []
}
function add(item, to = fresh(), times = len(to) + 1) {
    for n in range(times) { to.append(item) }
    return to
}
print(add(1), add(2), made)
print(add(3, [0]), add(4, times=2), add(to=[9], item=5))
"#;
        assert_eq!(output(source), "[1] [2] 2\n[0, 3, 3] [4, 4] [9, 5, 5]\n");

        let f = "function f(a, b) { return a }\n";
        assert_eq!(error(&format!("{f}f(1)")), "`f` needs a value for `b`");
        assert_eq!(error(&format!("{f}f(1, 2, 3)")), "`f` takes 2 arguments, but 3 were given");
        assert_eq!(error(&format!("{f}f(1, c=2)")), "`f` has no argument named `c`");
        assert_eq!(error(&format!("{f}f(1, a=2)")), "`f` was given `a` twice");
        assert_eq!(error(&format!("{f}f(a=1, 2)")), "plain arguments must come before named ones");
        assert_eq!(error("print(x=1)"), "`print` does not take a named argument `x`");
        assert_eq!(error("let five = 5\nfive()"), "an int cannot be called");
    }

    #[test]
    fn classes_build_objects_and_bind_methods() {
        let source = r#"
let start = 10
class Counter {
    count: int = start
    step: int = 1

    function bump(self, times = 1) {
        self.count += self.step * times
        return self
    }
}
let c = Counter(step=5)
let bump = c.bump
bump()
bump(times=2)
print(c, Counter().bump().count, Counter(1, 2).bump(3).count)
"#;
        assert_eq!(output(source), "Counter(count=25, step=5) 11 7\n");
        assert_eq!(error("class P {\n x: int\n}\nP()"), "`P` needs a value for `x`");
        assert_eq!(error("class P {\n x: int\n}\nP(1, 2)"), "`P` takes 1 argument, but 2 were given");
    }

    #[test]
    fn runaway_recursion_is_stopped_at_the_same_depth() {
        assert_eq!(
            error("function f(n) { return f(n + 1) }\nf(0)"),
            "more than 1000 calls are nested. Is `f` calling itself forever?"
        );
        // 999 deep is allowed, and building objects on the way does not count.
        let source = "class Box {\n n: int\n}\nfunction f(n) {\n if n == 0 { return 0 }\n return Box(f(n - 1)).n + 1\n}\nprint(f(998))";
        assert_eq!(output(source), "998\n");
    }

    #[test]
    fn errors_find_the_right_except() {
        let source = r#"
function risky(n) {
    if n % 3 == 0 { raise ValueError(f"bad {n}") }
    if n % 5 == 0 { return [1][n] }
    return n
}
var log = []
for n in range(1, 12) {
    try {
        log.append(risky(n))
    } except ValueError as e {
        log.append(e.message)
    } except IndexError {
        log.append("index")
    }
}
print(log)
"#;
        assert_eq!(
            output(source),
            "[1, 2, \"bad 3\", 4, \"index\", \"bad 6\", 7, 8, \"bad 9\", \"index\", 11]\n"
        );

        // An error passes through the `try` blocks that do not want it,
        // and through the functions between.
        let source = r#"
class Timeout {
    after: int
}
function wait(n) {
    try {
        if n > 2 { raise Timeout(n) }
        raise IndexError("small")
    } except ValueError {
        print("never")
    }
}
function outer(n) {
    try {
        wait(n)
    } except Timeout as t {
        print("timeout after", t.after)
        return "late"
    }
    return "fine"
}
try {
    print(outer(5))
    print(outer(1))
} except Error as e {
    print("caught", e.kind, e.message)
}
"#;
        assert_eq!(output(source), "timeout after 5\nlate\ncaught IndexError small\n");
    }

    #[test]
    fn leaving_a_try_puts_everything_back() {
        let source = r#"
function first_bad(items) {
    for item in items {
        try {
            if item < 0 { raise ValueError("negative") }
            if item == 0 { continue }
            if item > 100 { return "huge" }
        } except {
            return f"bad {item}"
        }
    }
    return "none"
}
print(first_bad([1, 0, 2]), first_bad([1, 500]), first_bad([3, -4]))

# Nine hundred calls are abandoned each time the error is caught. If they
# were still counted, the second round would be "too deep".
function deep(n) {
    if n == 0 { raise ValueError("bottom") }
    return deep(n - 1)
}
var caught = 0
for round in range(50) {
    try {
        deep(900)
    } except {
        caught += 1
    }
}
print(caught)
"#;
        assert_eq!(output(source), "none huge bad -4\n50\n");
    }

    #[test]
    fn errors_cross_built_ins_that_call_back() {
        // `map` is Rust code that calls a Zeno function. An error raised
        // in that function has to get out through `map` to the `try`.
        let source = r#"
function check(n) {
    if n == 3 { raise ValueError("three") }
    return n * 2
}
try {
    print([1, 2].map(check))
    print([1, 2, 3, 4].map(check))
} except ValueError as e {
    print("stopped:", e.message)
}
let safe = [1, 3, 5].map(function(n) {
    try {
        return check(n)
    } except {
        return 0
    }
})
let evens = [1, 2, 3, 4].filter(function(n) { return n % 2 == 0 })
print(safe, evens, [1, 2, 3].reduce(function(a, b) { return a + b }, 10))
"#;
        assert_eq!(output(source), "[2, 4]\nstopped: three\n[2, 0, 10] [2, 4] 16\n");
        assert_eq!(
            error("function check(n) { raise ValueError(\"no\") }\nprint([1].map(check))"),
            "uncaught ValueError: no"
        );
    }

    #[test]
    fn a_pattern_that_fails_half_way_leaves_nothing_behind() {
        let source = r#"
enum Shape {
    Circle(radius: float)
    Rect(width: float, height: float)
    Pair(left: Shape, right: Shape)
    Empty
}
function describe(shape) {
    match shape {
        case Empty { return "empty" }
        case Pair(Circle(r), Empty) { return f"circle {r} alone" }
        case Pair(Rect(w, 2.0), Rect(_, h)) { return f"rects {w} {h}" }
        case Pair(left, _) { return f"pair starting {describe(left)}" }
        case Circle(1.0) { return "unit circle" }
        case Circle(r) { return f"circle {r}" }
        case other { return f"other {other}" }
    }
}
print(describe(Empty))
print(describe(Pair(Circle(2.0), Empty)))
print(describe(Pair(Rect(3.0, 2.0), Rect(4.0, 5.0))))
print(describe(Pair(Rect(3.0, 9.0), Rect(4.0, 5.0))))
print(describe(Circle(1.0)), describe(Circle(2.5)), describe(Rect(1.0, 2.0)))
match 3 {
    case 1 { print("one") }
    case n { print("number", n) }
}
"#;
        assert_eq!(
            output(source),
            "empty\ncircle 2.0 alone\nrects 3.0 5.0\npair starting other Rect(3.0, 9.0)\n\
             unit circle circle 2.5 other Rect(1.0, 2.0)\nnumber 3\n"
        );
        assert_eq!(error("match 5 {\n case 1 { print(1) }\n}"), "no `case` matches 5");
        assert_eq!(
            error("enum E {\n A(x: int)\n}\nmatch A(1) {\n case A(x, y) { print(x) }\n}"),
            "`A` carries 1 value, but this pattern has 2"
        );
        assert_eq!(
            error("match 1 {\n case Nope(x) { print(x) }\n}"),
            "`Nope` is not an enum case that carries values"
        );
    }

    #[test]
    fn comprehensions_build_their_list_out_of_sight() {
        let source = r#"
let grid = [[x * y for x in range(1, 4)] for y in range(1, 4) if y != 2]
print(grid)
let adders = [function(n) { return n + k } for k in [10, 20]]
print(adders[0](1), adders[1](1), [c.upper() for c in "abc"])
"#;
        assert_eq!(output(source), "[[1, 2, 3], [3, 6, 9]]\n11 21 [\"A\", \"B\", \"C\"]\n");
    }

    #[test]
    fn and_or_and_chains_stop_as_soon_as_they_know() {
        let source = r#"
function yes(tag) {
    print(tag)
    return true
}
function no(tag) {
    print(tag)
    return false
}
print(no("a") and yes("b"), yes("c") or no("d"), yes("e") and no("f"))
print(1 < 2 < 3, 1 < 2 < 2, 3 < 2 < yes("never"), 1 <= 1 == 1)
"#;
        assert_eq!(output(source), "a\nc\ne\nf\nfalse true false\ntrue false false true\n");
        assert_eq!(error("print(1 and true)"), "`and` needs bools, but this is an int");
        assert_eq!(error("print(false or 1)"), "`or` needs bools, but this is an int");
        assert_eq!(error("if 1 {\n print(1)\n}"), "a condition must be true or false, but this is an int");
    }

    #[test]
    fn a_mistake_stops_the_program_where_it_is_and_not_before() {
        // The compiler sees each of these mistakes before the program
        // starts. It must still run everything above them.
        for (source, message) in [
            ("print(\"before\")\nprint(missing)", "`missing` is not defined"),
            ("print(\"before\")\nlet x = 1\nx = 2", "`x` cannot be changed. Declare it with `var` to allow that"),
            ("print(\"before\")\ny = 2", "`y` is not declared. Create it with `let` or `var` first"),
            ("print(\"before\")\nlen = 2", "`len` cannot be changed. Declare it with `var` to allow that"),
            ("print(\"before\")\nbreak", "`break` outside a loop"),
            ("function f() { continue }\nprint(\"before\")\nf()", "`continue` outside a loop"),
            ("print(\"before\")\nlet x = 1\nlet x = 2", "`x` is already declared in this block"),
            ("print(\"before\")\nimport nothing", "there is no module `nothing`. So far Zeno has `math`, `time`, `board`, `files` and `program`"),
            ("print(\"before\")\nfrom math import nothing", "module `math` has no `nothing`"),
            ("print(\"before\")\nprint(5 parsecs)", "`parsecs` is not a unit Zeno knows"),
        ] {
            let (output, error) = run(source);
            assert_eq!(output, "before\n", "{source}");
            let error = error.expect("the program should have failed");
            assert!(error.starts_with(message), "{source}\ngave: {error}");
        }
        // A mistake on a line that never runs is not a mistake yet.
        assert_eq!(output("if false {\n print(missing)\n}\nprint(\"fine\")"), "fine\n");
    }

    #[test]
    fn every_example_runs_the_same() {
        for example in std::fs::read_dir("examples").expect("the examples folder") {
            let mut path = example.expect("an example").path();
            // A program in several files has a folder to itself, and
            // starts in the folder's `main.zeno`.
            if path.is_dir() {
                path.push("main.zeno");
            }
            let source = std::fs::read_to_string(&path).expect("the example can be read");
            // blink.zeno blinks for ever, as firmware does.
            if path.ends_with("blink.zeno") {
                continue;
            }
            let read = |path: &str| std::fs::read_to_string(path).ok();
            let loaded = crate::modules::load(&path.to_string_lossy(), source.clone(), &read);
            let program = loaded.program.unwrap_or_else(|errors| panic!("{}: {}", path.display(), errors[0].message));
            // `run_both` insists that the two engines agree, which is the test.
            let _ = super::run_both(&program, &source);
        }
    }
}
