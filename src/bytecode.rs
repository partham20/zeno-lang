//! Bytecode: a program as a flat list of small instructions.
//!
//! The interpreter runs the syntax tree as it is: to work out `a + b * 2`
//! it calls itself for `a`, calls itself for `b * 2`, and adds. That is
//! simple and slow, because finding out *what to do next* means walking a
//! tree of boxes, and finding a variable means looking its name up.
//!
//! Bytecode is the same program with those questions answered ahead of
//! time. `a + b * 2` becomes
//!
//! ```text
//! GetLocal 0      ; a
//! GetLocal 1      ; b
//! Const 0         ; 2
//! Binary *
//! Binary +
//! ```
//!
//! Each instruction (an `Op`) does one small thing to a **stack** of
//! values: push one, pop two and push their sum, and so on. There is no
//! tree left, and no names: `a` is "slot 0". The compiler
//! (emit_bytecode.rs) writes these lists. The VM (vm.rs) is the loop that
//! runs them.
//!
//! This file is the format they agree on, and a printer for it, which is
//! what `zeno bytecode <file>` shows.

use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::{self, BinaryOp, UnaryOp};
use crate::token::Span;
use crate::value::Value;

/// One instruction.
///
/// A number inside an instruction is called its operand. What it counts
/// depends on the instruction: a slot, a constant, a place in the code to
/// jump to. The comments say "top" for the value on top of the stack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    // ------------------------------------------------- Values on the stack
    /// Push constant number n of this function: `42`, `"hello"`, `print`.
    Const(u32),
    /// Drop the top n values.
    Pop(u32),
    /// Drop values until n are left in this function's part of the stack.
    /// Used where the code cannot know how many there are: leaving a loop
    /// from somewhere inside it, or a pattern that failed half way.
    PopTo(u32),

    // ----------------------------------------------------------- Variables
    // A variable that only its own function uses lives in a slot of the
    // stack. One that a function written inside uses as well lives in a
    // **cell**, a box of its own, so that it can outlive the stack.
    /// Push a copy of slot n.
    GetLocal(u32),
    /// Pop the top into slot n.
    SetLocal(u32),
    /// Make a new, empty cell and call it cell n of this function.
    NewCell(u32),
    /// Pop the top into cell n, as its first value.
    DefineCell(u32),
    /// Push what cell n holds.
    GetCell(u32),
    /// Pop the top into cell n, which must have a value already.
    SetCell(u32),
    /// Push what captured cell n holds: a variable of a function around this one.
    GetCaptured(u32),
    /// Pop the top into captured cell n.
    SetCaptured(u32),

    // ----------------------------------------------------------- Operators
    /// `-x`, `not x`: replace the top with the result.
    Unary(UnaryOp),
    /// `a + b`: pop b, pop a, push the result.
    Binary(BinaryOp),
    /// `a and b`, after a: if the top is false, jump to n and leave it
    /// there as the answer. Otherwise pop it and go on to work out b.
    And(u32),
    /// `a or b`, after a: if the top is true, jump to n. Otherwise pop it.
    Or(u32),
    /// After the b of `a and b`: the top must be a bool.
    CheckBool(BinaryOp),
    /// One link of `a < b < c`: pop b and a. If `a < b`, push b back for
    /// the next link. If not, the whole chain is false: jump to n.
    Chain(BinaryOp, u32),

    // ------------------------------------------------------ Making values
    /// Pop n values and push a list of them.
    List(u32),
    /// Pop n keys, each with its value on top of it, and push a dict.
    Dict(u32),
    /// Pop the top and add it to the end of the list in slot n.
    Append(u32),
    /// Replace the top with its text, as `{value}` in an f-string does.
    Format,
    /// The same with a format, constant n: `{value:.2f}`.
    FormatWith(u32),
    /// Pop n pieces of text and push them joined together.
    Text(u32),
    /// Replace the number on top with a quantity in the unit that constant n names.
    Unit(u32),

    // ---------------------------------------------------- Fields and items
    /// Replace the object on top with its field or method, named by constant n.
    GetField(u32),
    /// Pop an object and, under it, a value. Store the value in the field
    /// that constant n names.
    SetField(u32),
    /// Pop an index and an object, push the item.
    GetIndex,
    /// Pop an index, an object and, under them, a value. Store the value there.
    SetIndex,

    // --------------------------------------------------------------- Jumps
    /// Carry on from instruction n.
    Jump(u32),
    /// Pop a bool. If it is false, carry on from instruction n.
    JumpIfFalse(u32),

    // --------------------------------------------------------------- Loops
    /// Replace the top with what a loop over it steps through: a copy of a
    /// list, or the characters of a str.
    ForStart,
    /// The top two values are a list and a position in it. If there is an
    /// item left, push it and move the position on. If not, jump to n.
    ForNext(u32),
    /// The same for a loop with two names. Replace the top with two
    /// values: what the loop gives its second name each round (the values
    /// of a dict, or the items of a list or the characters of a str), and
    /// above it the keys of the dict, or `none` where the first name is
    /// given positions.
    ForStartPairs,
    /// The top three values are what `ForStartPairs` left and a position.
    /// If there is a round left, push what the first name is given (the
    /// key, or the position) and move the position on. If not, jump to n.
    ForNextPair(u32),
    /// Push what the second name is given this round. n says how far
    /// below the top the position is.
    ForSecond(u32),

    // --------------------------------------------------------------- match
    /// Pop the name a pattern like `Circle(r)` was written with, and look
    /// at the value under it. Push whether the value is that case. The
    /// operands are the name, for error messages, and how many fields the
    /// pattern has.
    IsCase(u32, u32),
    /// Replace the enum value on top with the nth value it carries.
    CaseField(u32),
    /// No `case` matched the value on top. Stop the program.
    NoMatch,

    // ------------------------------------- Functions, classes and enums
    /// Push function n of this function, together with the cells it shares.
    Closure(u32),
    /// Push class n of this function.
    Class(u32),
    /// Push enum n of this function.
    Enum(u32),
    /// Replace the enum on top with its nth case.
    EnumCase(u32),
    /// Call with n arguments. The stack holds the thing to call and then
    /// the arguments. All of them are replaced by the result.
    Call(u32),
    /// Call with n arguments, the last few of which are named. The names
    /// are list number m of this function.
    CallNamed(u32, u32),
    /// At the start of a function: if the caller gave parameter n a value,
    /// jump over the code that works out its default, to m.
    Default(u32, u32),
    /// At the start of a function: the caller must have given parameter n
    /// a value. Constant m is what to say if not.
    Required(u32, u32),
    /// At the start of a function, once its parameters are settled: count
    /// one more call in progress.
    Enter,
    /// In the code that builds an object: pop n field values and push an
    /// object of the class that was called.
    Object(u32),
    /// Pop the result, end this function, and give the result to the caller.
    Return,

    // -------------------------------------------------------------- Errors
    /// Start of a `try`: from here on, an error raised jumps to n.
    Try(u32),
    /// End of a `try` body: errors are no longer caught here.
    EndTry,
    /// `except Name`: if the error on top is not of the kind constant n
    /// names, jump to m, the next `except`.
    Except(u32, u32),
    /// `except Timeout as t`: if the error on top carries an object that
    /// was raised, replace it with that object.
    Unwrap,
    /// Pop a value and raise it.
    Raise,
    /// No `except` matched. Pop the error and raise it again, from the
    /// line that raised it first.
    Reraise,
    /// A mistake the compiler found, such as a name that does not exist.
    /// Stop with the message in constant n. It is an instruction, and not
    /// an error when compiling, so that the program runs up to the mistake
    /// exactly as the interpreter does.
    Fatal(u32),

    // ------------------------------------------------- Superinstructions
    // These are never written by the compiler and never printed. `fuse`
    // makes them, for the VM only, out of the few instructions that come
    // one after another most often. Each does the work of the instructions
    // it stands for and then skips over them, so the code keeps its
    // length and every jump still lands where it did (decisions 22.13).
    /// `GetLocal a`, `GetLocal b`, `Binary op`.
    LocalLocal(u16, u16, BinaryOp),
    /// `GetLocal a`, `Const c`, `Binary op`.
    LocalConst(u16, u16, BinaryOp),
    /// `GetLocal a`, `GetLocal b`, `Binary op`, `SetLocal d`.
    LocalLocalSet(u16, u16, BinaryOp, u16),
    /// `GetLocal a`, `Const c`, `Binary op`, `SetLocal d`: `i += 1`.
    LocalConstSet(u16, u16, BinaryOp, u16),
    /// `GetLocal a`, `GetLocal b`, `Binary op`, `JumpIfFalse to`.
    LocalLocalJump(u16, u16, BinaryOp, u32),
    /// `GetLocal a`, `Const c`, `Binary op`, `JumpIfFalse to`.
    LocalConstJump(u16, u16, BinaryOp, u32),
    /// `Binary op`, `JumpIfFalse to`.
    BinaryJump(BinaryOp, u32),
    /// `GetLocal a`, `GetField n`: `b.x`.
    LocalField(u16, u32),

    // ------------------------------------------------------------- Rare
    // Last, after the superinstructions, so that the instructions of
    // before keep their numbers and the VM's loop is laid out as it was
    // (decision 56.10).
    /// `take object.name`: replace the object on top with its field named
    /// by constant n, and leave `none` in the field.
    TakeField(u32),
    /// `take object[index]`: pop an index and an object, push the item,
    /// and leave `none` in its place.
    TakeIndex,
    /// `return a, b`: pop n values and push them as the several values a
    /// function gives (decision 57).
    Several(u32),
    /// `let a, b = ...`: pop several values, which must be n, and push
    /// each, the first deepest. Each becomes a slot of its own.
    Unpack(u32),
}

/// The code the VM runs: `code` with superinstructions put over the
/// first instruction of each run of instructions they stand for. The
/// instructions under them stay, so a jump into the middle of a run
/// still finds the instruction it jumped to.
pub fn fuse(code: &[Op]) -> Vec<Op> {
    use Op::*;
    let small = |n: u32| u16::try_from(n).ok();
    let at = |i: usize| code.get(i).copied();
    (0..code.len())
        .map(|i| match (at(i), at(i + 1), at(i + 2), at(i + 3)) {
            (Some(GetLocal(a)), Some(GetLocal(b)), Some(Binary(op)), next) => match (small(a), small(b)) {
                (Some(a), Some(b)) => match next {
                    Some(JumpIfFalse(to)) => LocalLocalJump(a, b, op, to),
                    Some(SetLocal(d)) if small(d).is_some() => LocalLocalSet(a, b, op, d as u16),
                    _ => LocalLocal(a, b, op),
                },
                _ => code[i],
            },
            (Some(GetLocal(a)), Some(Const(c)), Some(Binary(op)), next) => match (small(a), small(c)) {
                (Some(a), Some(c)) => match next {
                    Some(JumpIfFalse(to)) => LocalConstJump(a, c, op, to),
                    Some(SetLocal(d)) if small(d).is_some() => LocalConstSet(a, c, op, d as u16),
                    _ => LocalConst(a, c, op),
                },
                _ => code[i],
            },
            (Some(Binary(op)), Some(JumpIfFalse(to)), _, _) => BinaryJump(op, to),
            (Some(GetLocal(a)), Some(GetField(n)), _, _) if small(a).is_some() => LocalField(a as u16, n),
            _ => code[i],
        })
        .collect()
}

/// One compiled function: its instructions and everything they refer to
/// by number. (The name is short for "prototype": the function before it
/// is joined with the variables around it, which makes a closure.)
///
/// The whole program is one of these, called `main`, and the functions
/// written in it are inside, in `functions`.
pub struct Proto {
    /// The function's name for error messages. `function` if it has none.
    pub name: String,
    /// The function as written. `None` for the program itself and for the
    /// code that builds an object of a class.
    pub decl: Option<Arc<ast::Function>>,
    /// The names of the parameters, which the arguments of a call are
    /// lined up with. They are in slots 0, 1, 2...
    pub params: Vec<String>,

    pub code: Vec<Op>,
    /// `code` with superinstructions (`fuse`), which is what the VM runs.
    pub fast: Vec<Op>,
    /// Where in the source each instruction came from, for error messages.
    pub spans: Vec<Span>,
    /// A word about some instructions, such as the name of the variable in
    /// a slot. Only the printer reads it.
    pub notes: Vec<String>,

    pub constants: Vec<Value>,
    /// Lists of argument names, for `CallNamed`.
    pub names: Vec<Vec<String>>,
    /// The functions written inside this one.
    pub functions: Vec<Rc<Proto>>,
    pub classes: Vec<ClassCode>,
    pub enums: Vec<ast::Enum>,

    /// The names of this function's cells.
    pub cells: Vec<String>,
    /// The variables of the functions around it that this one uses, and
    /// where to find each when the closure is made.
    pub captures: Vec<Capture>,
}

/// A variable a function takes with it.
pub struct Capture {
    pub name: String,
    pub from: From,
}

/// Where the function that makes a closure finds a variable for it.
#[derive(Clone, Copy, PartialEq)]
pub enum From {
    /// Its own cell n.
    Cell(u32),
    /// A cell it captured itself, number n.
    Captured(u32),
}

/// A class, before the program reaches it and makes it a value.
pub struct ClassCode {
    pub name: String,
    pub fields: Vec<ast::Field>,
    /// Which of `functions` builds an object: it works out the defaults
    /// of the fields the caller left out.
    pub constructor: u32,
    /// Which of `functions` are the methods.
    pub methods: Vec<u32>,
}

// ------------------------------------------------------------------ Printing

/// The whole program as text, one instruction per line.
pub fn print(main: &Proto) -> String {
    let mut out = String::new();
    print_function(main, &mut out);
    out
}

fn print_function(proto: &Proto, out: &mut String) {
    let _ = writeln!(out, "== {}({}) ==", proto.name, proto.params.join(", "));
    if !proto.captures.is_empty() {
        let names: Vec<&str> = proto.captures.iter().map(|capture| capture.name.as_str()).collect();
        let _ = writeln!(out, "   captures: {}", names.join(", "));
    }
    if !proto.cells.is_empty() {
        let _ = writeln!(out, "   cells: {}", proto.cells.join(", "));
    }

    let mut last_line = 0;
    for (at, op) in proto.code.iter().enumerate() {
        // The source line is shown where it changes.
        let line = proto.spans[at].line_in_file();
        let shown = if line == last_line { "   |".to_string() } else { format!("{line:>4}") };
        last_line = line;

        let (name, operands) = describe(*op);
        let comment = comment(proto, at, *op);
        let text = format!("{at:04} {shown}  {name:<12} {operands}");
        match comment.is_empty() {
            true => {
                let _ = writeln!(out, "{}", text.trim_end());
            }
            false => {
                let _ = writeln!(out, "{text:<34} ; {comment}");
            }
        }
    }
    let _ = writeln!(out);

    for function in &proto.functions {
        print_function(function, out);
    }
}

/// An instruction's name and its operands.
fn describe(op: Op) -> (String, String) {
    // An operator is shown as it is written in a program.
    match op {
        Op::Unary(UnaryOp::Neg) => return ("Unary".to_string(), "-".to_string()),
        Op::Unary(UnaryOp::Not) => return ("Unary".to_string(), "not".to_string()),
        Op::Binary(op) => return ("Binary".to_string(), op.text().to_string()),
        Op::CheckBool(op) => return ("CheckBool".to_string(), op.text().to_string()),
        Op::Chain(op, to) => return ("Chain".to_string(), format!("{} {to}", op.text())),
        _ => {}
    }
    let text = format!("{op:?}");
    match text.split_once('(') {
        Some((name, rest)) => (name.to_string(), rest.trim_end_matches(')').replace(", ", " ")),
        None => (text, String::new()),
    }
}

/// What an instruction's operand stands for.
fn comment(proto: &Proto, at: usize, op: Op) -> String {
    let constant = |n: u32| proto.constants[n as usize].repr();
    match op {
        Op::Const(n) | Op::FormatWith(n) | Op::Unit(n) | Op::Fatal(n) => constant(n),
        Op::GetField(n) | Op::SetField(n) | Op::TakeField(n) => format!(".{}", proto.constants[n as usize]),
        Op::Required(_, n) | Op::IsCase(n, _) | Op::Except(n, _) => constant(n),
        Op::GetCell(n) | Op::SetCell(n) | Op::DefineCell(n) | Op::NewCell(n) => {
            proto.cells[n as usize].clone()
        }
        Op::GetCaptured(n) | Op::SetCaptured(n) => proto.captures[n as usize].name.clone(),
        Op::Closure(n) => proto.functions[n as usize].name.clone(),
        Op::Class(n) => format!("class {}", proto.classes[n as usize].name),
        Op::Enum(n) => format!("enum {}", proto.enums[n as usize].name),
        Op::CallNamed(_, n) => proto.names[n as usize].join("=, ") + "=",
        _ => proto.notes[at].clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{lexer, parser};

    #[test]
    fn bytecode_is_printed_one_instruction_per_line() {
        let source = "var total = 0\nfor n in [1, 2] {\n    total += n\n}\nprint(total)";
        let program = parser::parse(lexer::lex(source).unwrap()).unwrap();
        let printed = super::print(&crate::emit_bytecode::compile(&program));
        let expected = "\
== main() ==
0000    1  Const        0          ; 0
0001    2  Const        1          ; 1
0002    |  Const        2          ; 2
0003    |  List         2
0004    |  ForStart
0005    |  Const        0          ; 0
0006    |  ForNext      13
0007    3  GetLocal     0          ; total
0008    |  GetLocal     3          ; n
0009    |  Binary       +
0010    |  SetLocal     0          ; total
0011    2  Pop          1
0012    |  Jump         6
0013    |  Pop          2
0014    5  Const        3          ; <function print>
0015    |  GetLocal     0          ; total
0016    |  Call         1
0017    |  Pop          1
0018    |  Const        4          ; none
0019    |  Return

";
        assert_eq!(printed, expected);
    }
}
