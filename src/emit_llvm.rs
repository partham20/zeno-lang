//! Stage 12: the LLVM backend. It turns a checked program into LLVM IR.
//!
//! ```text
//! tokens -> parser -> tree -> checker -> [emit_llvm] -> LLVM IR -> clang -> program
//! ```
//!
//! LLVM is the compiler library behind Clang, Rust and Swift. Its input is
//! a small typed language called IR ("intermediate representation"). Hand
//! LLVM a program in IR and it does the optimising and writes the machine
//! code for whichever processor is asked for. So this stage does what
//! `emit_c` does, one level further down: there is no C compiler between
//! Zeno and LLVM, and nothing of C's rules to work around.
//!
//! IR reads like assembly for a machine that does not exist:
//!
//! ```text
//! define internal double @z_area(double %arg.w, double %arg.h) {
//! entry:
//!   %t1 = fmul double %arg.w, %arg.h
//!   ret double %t1
//! }
//! ```
//!
//! Four things about it shape everything in this file:
//!
//! 1. **Every value has a type, written at every use.** `i64` is a whole
//!    number of 64 bits, `double` a float, `i1` one bit (a bool), `ptr` an
//!    address. The checker's `Types` say which is which, as for the C
//!    backend.
//! 2. **There are no nested expressions.** `a + b * c` is two
//!    instructions, each giving a new name: `%t1 = fmul ...` then
//!    `%t2 = fadd ...`. The order things happen in is the order they are
//!    written, so Zeno's left-to-right rule costs nothing here. (In
//!    `emit_c` it took `sequence` and `settle`.)
//! 3. **A name is given a value once, and never again.** This is called
//!    SSA, "static single assignment". A Zeno `let` is exactly that, so a
//!    `let` *is* an IR value. A `var` changes, so it gets a place in
//!    memory (`alloca`), and is read with `load` and written with
//!    `store`. LLVM's first pass turns those back into plain values where
//!    it can. See `declare`.
//! 4. **There is no `if` and no `while`.** A function is a list of
//!    blocks. Each block has a label, runs straight through, and ends
//!    with exactly one jump: `br` to another block, or `ret`. Loops and
//!    branches are spelled out with those. See `Code`.
//!
//! The file is in this order: the runtime the program links to, the
//! pieces the emitter keeps, the file it writes, how one instruction is
//! written, then functions, classes and enums, statements (with loops,
//! `match` and `try`), expressions (with lists and values that may be
//! `none`), calls, objects and enum values, functions as values, and at
//! the end how a value is shown as text and compared with another.
//!
//! The compiled program still needs the runtime (`runtime.c`) for text,
//! printing, lists, errors and the collector. `zeno build --llvm` writes
//! it out as a second file and hands both to clang. See `runtime`.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::ast::*;
use crate::checker::{FunctionKind, Kind, Types};
use crate::emit_c;
use crate::error::Error;
use crate::token::Span;
use crate::units;

// ---------------------------------------------------------------- The runtime

/// The runtime as an LLVM build needs it: the ordinary one, with its
/// functions made visible to another file.
///
/// A C build pastes the runtime in front of the program, and every
/// function in it is `static`, which in C means "only this file may use
/// it". That lets the C compiler drop the ones a program never calls.
/// Here the program is in a second file, of IR, so the word has to go.
/// Only a `static` at the start of a line is removed, which is one that
/// stands outside any function.
pub fn runtime() -> String {
    // A function that never returns has that said in front of `static`.
    const NEVER: &str = "__attribute__((noreturn)) ";
    let mut text = String::new();
    for line in emit_c::runtime(false, true).lines() {
        match line.strip_prefix(NEVER) {
            Some(rest) => {
                text += NEVER;
                text += rest.strip_prefix("static ").unwrap_or(rest);
            }
            None => text += line.strip_prefix("static ").unwrap_or(line),
        }
        text.push('\n');
    }
    text + FOR_TRY
}

/// Three small functions added to the runtime for `try`.
///
/// A `try` needs a `zeno_try` on the stack, which holds a C `jmp_buf`:
/// the processor's registers, saved by `setjmp`. How big that is differs
/// from one processor and C library to the next, and only C knows. So the
/// IR asks for the size while the program runs, and leaves the fields to
/// these.
const FOR_TRY: &str = "
/* ------------------------------------------- For a build through LLVM IR */

size_t zeno_try_size(void) {
    return sizeof(zeno_try);
}

/* A `try` begins: from here on, errors come to `place`. */
void zeno_try_open(zeno_try *place) {
    place->outer = zeno_trying;
    zeno_trying = place;
}

/* It is over, with no error. */
void zeno_try_close(zeno_try *place) {
    zeno_trying = place->outer;
}
";

/// Everything in the runtime and the C library that the IR may call, as
/// LLVM wants it declared. Only the ones a program uses are written into
/// its file.
///
/// `signext` and `zeroext` say how a value narrower than a register is
/// widened when it is handed over. C compilers do this without being
/// asked. IR has to say it, or a processor such as RISC-V may read
/// leftover bits.
///
/// A list keeps each item in a slot, which in C is a union the size of
/// one register (`zeno_slot`). A union that small is handed over as a
/// plain number, so here a slot is an `i64`. See `to_slot`.
const DECLARED: &[(&str, &str)] = &[
    // The processor's own overflow flag, as LLVM exposes it: the result
    // and one bit that says whether it fitted.
    ("llvm.sadd.with.overflow.i64", "declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)"),
    ("llvm.ssub.with.overflow.i64", "declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)"),
    ("llvm.smul.with.overflow.i64", "declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)"),
    ("zeno_overflow", "declare void @zeno_overflow(i32 signext, i32 signext)"),
    ("zeno_by_zero", "declare void @zeno_by_zero(i32 signext, i32 signext)"),
    ("zeno_gone", "declare void @zeno_gone(i32 signext, i32 signext)"),
    ("zeno_pow", "declare i64 @zeno_pow(i64, i64, i32 signext, i32 signext)"),
    // The board. A pin is its number.
    ("zeno_pin", "declare i64 @zeno_pin(i64, i32 signext, i32 signext)"),
    ("zeno_pin_on", "declare void @zeno_pin_on(i64)"),
    ("zeno_pin_off", "declare void @zeno_pin_off(i64)"),
    ("zeno_pin_toggle", "declare void @zeno_pin_toggle(i64)"),
    ("zeno_pin_is_on", "declare zeroext i1 @zeno_pin_is_on(i64)"),
    ("zeno_str_pin", "declare ptr @zeno_str_pin(i64)"),
    ("zeno_sleep", "declare void @zeno_sleep(double)"),
    ("zeno_now", "declare double @zeno_now()"),
    ("zeno_abs", "declare i64 @zeno_abs(i64, i32 signext, i32 signext)"),
    ("zeno_shl", "declare i64 @zeno_shl(i64, i64, i32 signext, i32 signext)"),
    ("zeno_shr", "declare i64 @zeno_shr(i64, i64, i32 signext, i32 signext)"),
    ("zeno_range_count", "declare i64 @zeno_range_count(i64, i64, i64, i32 signext, i32 signext)"),
    ("zeno_floordiv_float", "declare double @zeno_floordiv_float(double, double, i32 signext, i32 signext)"),
    ("zeno_mod_float", "declare double @zeno_mod_float(double, double, i32 signext, i32 signext)"),
    ("zeno_int", "declare i64 @zeno_int(double, i32 signext, i32 signext)"),
    ("zeno_whole", "declare i64 @zeno_whole(double)"),
    ("zeno_sqrt", "declare double @zeno_sqrt(double, i32 signext, i32 signext)"),
    ("pow", "declare double @pow(double, double)"),
    ("fabs", "declare double @fabs(double)"),
    ("floor", "declare double @floor(double)"),
    ("ceil", "declare double @ceil(double)"),
    ("sin", "declare double @sin(double)"),
    ("cos", "declare double @cos(double)"),
    ("strcmp", "declare i32 @strcmp(ptr, ptr)"),
    ("strstr", "declare ptr @strstr(ptr, ptr)"),
    ("zeno_alloc", "declare ptr @zeno_alloc(i64)"),
    ("zeno_alloc_text", "declare ptr @zeno_alloc_text(i64)"),
    ("zeno_join", "declare ptr @zeno_join(i32 signext, ...)"),
    ("zeno_print", "declare void @zeno_print(i32 signext, ...)"),
    ("zeno_len", "declare i64 @zeno_len(ptr)"),
    ("zeno_char_at", "declare ptr @zeno_char_at(ptr, i64, i32 signext, i32 signext)"),
    ("zeno_str_int", "declare ptr @zeno_str_int(i64)"),
    ("zeno_str_float", "declare ptr @zeno_str_float(double)"),
    ("zeno_str_bool", "declare ptr @zeno_str_bool(i1 zeroext)"),
    ("zeno_str_quantity", "declare ptr @zeno_str_quantity(double, ptr)"),
    ("zeno_repr_str", "declare ptr @zeno_repr_str(ptr)"),
    ("zeno_fixed", "declare ptr @zeno_fixed(double, i32 signext)"),
    ("zeno_radix", "declare ptr @zeno_radix(i64, i8 signext)"),
    ("zeno_pad", "declare ptr @zeno_pad(ptr, i32 signext, i8 signext, i1 zeroext)"),
    ("zeno_parse_int", "declare i64 @zeno_parse_int(ptr, i32 signext, i32 signext)"),
    ("zeno_parse_float", "declare double @zeno_parse_float(ptr, i32 signext, i32 signext)"),
    ("zeno_upper", "declare ptr @zeno_upper(ptr)"),
    ("zeno_lower", "declare ptr @zeno_lower(ptr)"),
    ("zeno_strip", "declare ptr @zeno_strip(ptr)"),
    ("zeno_replace", "declare ptr @zeno_replace(ptr, ptr, ptr)"),
    ("zeno_starts_with", "declare zeroext i1 @zeno_starts_with(ptr, ptr)"),
    ("zeno_ends_with", "declare zeroext i1 @zeno_ends_with(ptr, ptr)"),
    ("zeno_split_blanks", "declare ptr @zeno_split_blanks(ptr)"),
    ("zeno_split", "declare ptr @zeno_split(ptr, ptr, i32 signext, i32 signext)"),
    ("zeno_chars", "declare ptr @zeno_chars(ptr)"),
    ("zeno_list_new", "declare ptr @zeno_list_new(i64)"),
    ("zeno_list_of", "declare ptr @zeno_list_of(i32 signext, ...)"),
    ("zeno_list_append", "declare void @zeno_list_append(ptr, i64)"),
    ("zeno_list_copy", "declare ptr @zeno_list_copy(ptr)"),
    ("zeno_list_concat", "declare ptr @zeno_list_concat(ptr, ptr)"),
    ("zeno_list_at", "declare ptr @zeno_list_at(ptr, i64, i32 signext, i32 signext)"),
    ("zeno_list_pop", "declare i64 @zeno_list_pop(ptr, i32 signext, i32 signext)"),
    ("zeno_list_str", "declare ptr @zeno_list_str(ptr, ptr)"),
    ("zeno_list_eq", "declare zeroext i1 @zeno_list_eq(ptr, ptr, ptr)"),
    ("zeno_list_contains", "declare zeroext i1 @zeno_list_contains(ptr, i64, ptr)"),
    ("zeno_list_sum_int", "declare i64 @zeno_list_sum_int(ptr, i32 signext, i32 signext)"),
    ("zeno_list_sum_float", "declare double @zeno_list_sum_float(ptr)"),
    ("zeno_list_best_int", "declare i64 @zeno_list_best_int(ptr, i1 zeroext, ptr, i32 signext, i32 signext)"),
    ("zeno_list_best_float", "declare double @zeno_list_best_float(ptr, i1 zeroext, ptr, i32 signext, i32 signext)"),
    ("zeno_error_same", "declare zeroext i1 @zeno_error_same(ptr, ptr)"),
    ("zeno_error_new", "declare ptr @zeno_error_new(ptr, ptr, ptr)"),
    ("zeno_raise", "declare void @zeno_raise(ptr, i32 signext, i32 signext) noreturn"),
    ("zeno_raise_again", "declare void @zeno_raise_again() noreturn"),
    ("zeno_caught", "declare zeroext i1 @zeno_caught(ptr)"),
    ("zeno_try_size", "declare i64 @zeno_try_size()"),
    ("zeno_try_open", "declare void @zeno_try_open(ptr)"),
    ("zeno_try_close", "declare void @zeno_try_close(ptr)"),
    // `returns_twice` warns LLVM that this call can come back a second
    // time, much later, when an error jumps to it.
    ("_setjmp", "declare i32 @_setjmp(ptr) returns_twice"),
    ("zeno_show_function", "declare ptr @zeno_show_function(ptr)"),
    ("zeno_mark_words", "declare void @zeno_mark_words(ptr, i64)"),
];

/// A few functions of the runtime, written again in IR.
///
/// A call into `runtime.c` cannot be inlined, because the C is in
/// another file that LLVM does not see while it compiles this one. For
/// text that does not matter. For `%`, `//` and `/` it does: a loop of
/// them ran at a third of the speed of the C build. So these are
/// written here, in the same file as the program, where LLVM inlines
/// them. Each is copied into a program only if the program uses it.
///
/// They follow `zeno_mod`, `zeno_floordiv` and `zeno_div` in `runtime.c`
/// line for line. The dot in the names keeps them apart from anything C
/// can name.
///
/// The last six put a number that may be `none` into a list. The
/// runtime's own (`zeno_box_int`) take a C struct, and how a struct is
/// handed to a C function differs from processor to processor. Written
/// in IR, they never leave IR.
const WRITTEN: &[(&str, &str)] = &[
    (
        "zeno.floordiv",
        "; `//` on two ints. The processor rounds toward zero and Zeno rounds
; down, so step down once more when the signs differ and the division
; was not exact.
define internal i64 @zeno.floordiv(i64 %a, i64 %b, i32 %line, i32 %col) {
entry:
  %zero = icmp eq i64 %b, 0
  br i1 %zero, label %by_zero, label %check

by_zero:
  call void @zeno_by_zero(i32 %line, i32 %col)
  unreachable

check:
  ; The one division that does not fit: the smallest int by -1.
  %smallest = icmp eq i64 %a, -9223372036854775808
  %minus_one = icmp eq i64 %b, -1
  %too_big = and i1 %smallest, %minus_one
  br i1 %too_big, label %overflow, label %divide

overflow:
  call void @zeno_overflow(i32 %line, i32 %col)
  unreachable

divide:
  %quotient = sdiv i64 %a, %b
  %left_over = srem i64 %a, %b
  %inexact = icmp ne i64 %left_over, 0
  ; The sign bit of `a xor b` is set when the two signs differ.
  %mixed = xor i64 %a, %b
  %differ = icmp slt i64 %mixed, 0
  %step_down = and i1 %inexact, %differ
  %lower = sub i64 %quotient, 1
  %result = select i1 %step_down, i64 %lower, i64 %quotient
  ret i64 %result
}
",
    ),
    (
        "zeno.mod",
        "; `%` on two ints. The result takes the sign of the right side, to pair
; with `//`.
define internal i64 @zeno.mod(i64 %a, i64 %b, i32 %line, i32 %col) {
entry:
  %zero = icmp eq i64 %b, 0
  br i1 %zero, label %by_zero, label %divide

by_zero:
  call void @zeno_by_zero(i32 %line, i32 %col)
  unreachable

divide:
  ; Anything `% -1` is 0. Dividing by 1 instead gives that, and avoids
  ; the smallest int `% -1`, which the processor refuses.
  %minus_one = icmp eq i64 %b, -1
  %by = select i1 %minus_one, i64 1, i64 %b
  %left_over = srem i64 %a, %by
  %inexact = icmp ne i64 %left_over, 0
  %mixed = xor i64 %left_over, %b
  %differ = icmp slt i64 %mixed, 0
  %move = and i1 %inexact, %differ
  %moved = add i64 %left_over, %b
  %result = select i1 %move, i64 %moved, i64 %left_over
  ret i64 %result
}
",
    ),
    (
        "zeno.div",
        "; `/`, which reports a zero on the right.
define internal double @zeno.div(double %a, double %b, i32 %line, i32 %col) {
entry:
  %zero = fcmp oeq double %b, 0.0
  br i1 %zero, label %by_zero, label %divide

by_zero:
  call void @zeno_by_zero(i32 %line, i32 %col)
  unreachable

divide:
  %result = fdiv double %a, %b
  ret double %result
}
",
    ),
    (
        "zeno.box.int",
        "; An `int or none` on its way into a list. A slot has no room for the
; flag, so the number goes into a small block of its own and the slot
; holds the address: null for `none`.
define internal ptr @zeno.box.int({ i1, i64 } %value) {
entry:
  %present = extractvalue { i1, i64 } %value, 0
  br i1 %present, label %some, label %none

some:
  %box = call ptr @zeno_alloc_text(i64 8)
  %inside = extractvalue { i1, i64 } %value, 1
  store i64 %inside, ptr %box
  ret ptr %box

none:
  ret ptr null
}
",
    ),
    (
        "zeno.unbox.int",
        "; And out of the list again.
define internal { i1, i64 } @zeno.unbox.int(ptr %box) {
entry:
  %none = icmp eq ptr %box, null
  br i1 %none, label %nothing, label %some

some:
  %inside = load i64, ptr %box
  %value = insertvalue { i1, i64 } { i1 true, i64 undef }, i64 %inside, 1
  ret { i1, i64 } %value

nothing:
  ret { i1, i64 } zeroinitializer
}
",
    ),
    (
        "zeno.box.float",
        "; A `float or none` on its way into a list (see `zeno.box.int`).
define internal ptr @zeno.box.float({ i1, double } %value) {
entry:
  %present = extractvalue { i1, double } %value, 0
  br i1 %present, label %some, label %none

some:
  %box = call ptr @zeno_alloc_text(i64 8)
  %inside = extractvalue { i1, double } %value, 1
  store double %inside, ptr %box
  ret ptr %box

none:
  ret ptr null
}
",
    ),
    (
        "zeno.unbox.float",
        "define internal { i1, double } @zeno.unbox.float(ptr %box) {
entry:
  %none = icmp eq ptr %box, null
  br i1 %none, label %nothing, label %some

some:
  %inside = load double, ptr %box
  %value = insertvalue { i1, double } { i1 true, double undef }, double %inside, 1
  ret { i1, double } %value

nothing:
  ret { i1, double } zeroinitializer
}
",
    ),
    (
        "zeno.box.bool",
        "; A `bool or none` on its way into a list (see `zeno.box.int`).
define internal ptr @zeno.box.bool({ i1, i1 } %value) {
entry:
  %present = extractvalue { i1, i1 } %value, 0
  br i1 %present, label %some, label %none

some:
  %box = call ptr @zeno_alloc_text(i64 1)
  %inside = extractvalue { i1, i1 } %value, 1
  %byte = zext i1 %inside to i8
  store i8 %byte, ptr %box
  ret ptr %box

none:
  ret ptr null
}
",
    ),
    (
        "zeno.unbox.bool",
        "define internal { i1, i1 } @zeno.unbox.bool(ptr %box) {
entry:
  %none = icmp eq ptr %box, null
  br i1 %none, label %nothing, label %some

some:
  %byte = load i8, ptr %box
  %inside = trunc i8 %byte to i1
  %value = insertvalue { i1, i1 } { i1 true, i1 undef }, i1 %inside, 1
  ret { i1, i1 } %value

nothing:
  ret { i1, i1 } zeroinitializer
}
",
    ),
];

// ------------------------------------------------------------------- Pieces

/// A value that has been worked out: how the IR refers to it, and what
/// kind of Zeno value it is. `code` is a name (`%t3`), a constant (`42`,
/// `true`, `1.5`) or the address of a piece of text (`@.str.2`).
#[derive(Clone)]
struct Val {
    code: String,
    kind: Kind,
}

impl Val {
    fn new(code: impl Into<String>, kind: Kind) -> Val {
        Val { code: code.into(), kind }
    }

    /// What a call that gives nothing back leaves behind.
    fn nothing() -> Val {
        Val::new("", Kind::None)
    }
}

/// Where a variable's value is.
#[derive(Clone)]
enum Place {
    /// It never changes, so the variable is simply another name for this
    /// value. Every `let` inside a function, and every parameter.
    Value(String),
    /// It lives in memory at this address, and is read with `load` and
    /// written with `store`. Every `var`, and everything declared at the
    /// top of the file, where functions must be able to find it.
    Slot(String),
}

#[derive(Clone)]
struct Variable {
    place: Place,
    kind: Kind,
    /// Other functions can reach it and change it: a `var` at the top of
    /// the file. What was known about such a variable before a call may
    /// not be true after it. See `present`.
    open: bool,
}

/// One of the program's functions or methods, as a call needs to know it.
#[derive(Clone)]
struct Entry {
    decl: Arc<Function>,
    kind: FunctionKind,
    /// Its name in the IR, such as `@z_area`.
    name: String,
}

/// What a call can be given: the name, kind and default of each place.
type Slots = Vec<(String, Kind, Option<Expr>)>;

/// A class: an object is a block of memory holding its fields in order.
struct ClassEntry {
    fields: Slots,
    methods: HashMap<String, Entry>,
    /// The name of its struct type in the IR, such as `%z_Point`.
    ty: String,
}

/// An enum: each case with the names and kinds of the values it carries.
struct EnumEntry {
    cases: Vec<(String, Vec<(String, Kind)>)>,
}

/// A function that has been reached but not yet written out. `sees` is
/// the top of the file as it stood at the function's own line, because
/// those are the variables it can use (decision 3.6d).
struct Waiting {
    entry: Entry,
    span: Span,
    sees: HashMap<String, Variable>,
}

/// A loop that is being written: where `continue` and `break` jump to.
struct Loop {
    next: String,
    done: String,
    /// How many `try` blocks were open when the loop began. Leaving the
    /// loop from inside a newer one has to close it first.
    tries: usize,
}

/// A counting loop that has been opened: its counter, and its labels.
struct Count {
    counter: String,
    test: String,
    body: String,
    next: String,
    done: String,
}

/// The function being written.
///
/// A function in IR is a list of blocks. A block starts with a label and
/// must end with exactly one instruction that leaves it (`br`, `ret` or
/// `unreachable`), with none after it. `ended` keeps track of that: it is
/// true from the moment the block has been left until a new label opens
/// the next one.
struct Code {
    lines: Vec<String>,
    /// The `alloca`s, which all go at the top of the first block. One
    /// inside a loop would take new space on the stack on every trip.
    slots: Vec<String>,
    /// The label of the block being filled.
    block: String,
    ended: bool,
    temps: usize,
    labels: usize,
    /// How often each variable name has been given a slot, to tell two
    /// variables of one name apart.
    named: HashMap<String, usize>,
    loops: Vec<Loop>,
    /// What the function gives back. `None` for the top of the file.
    returns: Kind,
    in_function: bool,
    /// The `try` blocks that are open here, innermost last: the address
    /// of each one's `zeno_try`.
    tries: Vec<String>,
    /// The slots this function has on its own stack.
    on_stack: HashSet<String>,
    /// There is a `try` somewhere in this function. See `load`.
    careful: bool,
    /// The names that functions written inside this one mention. A `var`
    /// with one of these names is shared with such a function, so it
    /// cannot live on this function's stack. See `declare`.
    shared: HashSet<String>,
}

impl Code {
    fn new(returns: Kind, in_function: bool) -> Code {
        Code {
            lines: Vec::new(),
            slots: Vec::new(),
            block: "entry".to_string(),
            ended: false,
            temps: 0,
            labels: 0,
            named: HashMap::new(),
            loops: Vec::new(),
            returns,
            in_function,
            tries: Vec::new(),
            on_stack: HashSet::new(),
            careful: false,
            shared: HashSet::new(),
        }
    }

    /// The same, for a function with these statements in it.
    fn of(body: &[Stmt], returns: Kind, in_function: bool) -> Code {
        let mut code = Code::new(returns, in_function);
        code.careful = emit_c::has_try(body);
        for statement in body {
            emit_c::names_in_functions(statement, false, &mut code.shared);
        }
        code
    }
}

struct Emitter<'a> {
    types: &'a Types,
    functions: HashMap<String, Entry>,
    classes: HashMap<String, ClassEntry>,
    enums: HashMap<String, EnumEntry>,
    /// Each enum case by name: the enum it belongs to, and its number.
    cases: HashMap<String, (String, usize)>,
    /// The names brought in by `import`, and the module of each.
    imported: HashMap<String, String>,
    waiting: VecDeque<Waiting>,
    /// The variables in view, innermost block last. The first entry is
    /// the top of the file.
    scopes: Vec<HashMap<String, Variable>>,
    code: Code,
    /// How many functions written as values there have been, to give
    /// each a name.
    closures: usize,

    // The finished parts of the file.
    /// The struct types: one for each class and each enum case.
    structs: Vec<String>,
    /// Each piece of text the program mentions, and its name in the IR.
    strings: Vec<(String, String)>,
    globals: Vec<String>,
    /// The globals that may hold an address, which the collector must be
    /// shown.
    roots: Vec<String>,
    /// The functions the compiler writes for itself: to make an object,
    /// show one, compare two. `written` has their names, so that each is
    /// written once.
    helpers: Vec<String>,
    written: HashSet<String>,
    definitions: Vec<String>,
    /// The top of the file, as the function `zeno_program`.
    program: String,
    /// The names from `DECLARED` and `WRITTEN` that are called.
    used: BTreeSet<&'static str>,
}

/// The error for a part of Zeno this stage does not handle yet.
fn cannot(what: impl AsRef<str>, span: Span) -> Error {
    Error::new(
        format!("`zeno build --llvm` cannot compile {} yet. `zeno run` can still run this program", what.as_ref()),
        span,
    )
}

/// The pin the LED is on. The same number as `ZENO_LED` in the runtime
/// and `board::LED` in the interpreter.
const LED: i64 = crate::board::LED;

/// A kind in words, for a message.
fn described(kind: &Kind) -> String {
    match kind {
        Kind::Int => "an int".to_string(),
        Kind::Float => "a float".to_string(),
        Kind::Str => "a str".to_string(),
        Kind::Bool => "a bool".to_string(),
        Kind::Pin => "a pin".to_string(),
        Kind::None => "a value that is nothing".to_string(),
        Kind::Quantity(unit) => format!("a number with a unit ({unit})"),
        Kind::List(_) => "a list".to_string(),
        Kind::Class(name) => format!("an object of the class `{name}`"),
        Kind::Enum(name) => format!("a value of the enum `{name}`"),
        Kind::Optional(_) => "a value that may be `none`".to_string(),
        Kind::Error => "an error value".to_string(),
        Kind::Function(_) => "a function used as a value".to_string(),
        Kind::Owned(_) => "an owned value".to_string(),
        Kind::Other(name) => format!("a value of type {name}"),
    }
}

/// Is this a kind that is held through an address?
fn is_pointer(kind: &Kind) -> bool {
    matches!(kind, Kind::Str | Kind::List(_) | Kind::Class(_) | Kind::Enum(_) | Kind::Error | Kind::Function(_))
}

/// Can a value of this kind be, or contain, the address of memory that
/// the collector looks after? Numbers and bools cannot.
fn holds_pointer(kind: &Kind) -> bool {
    match kind {
        Kind::Int | Kind::Float | Kind::Bool | Kind::Pin | Kind::Quantity(_) | Kind::None => false,
        Kind::Optional(inner) => holds_pointer(inner),
        _ => true,
    }
}

/// A kind without the word `owned` on it. Here an owned value is held
/// exactly as a shared one is, and the collector frees it.
fn shared(kind: &Kind) -> Kind {
    match kind {
        Kind::Owned(inner) => shared(inner),
        other => other.clone(),
    }
}

/// The kind of `[]` before the place it arrives at has said what it holds.
fn is_empty_list(kind: &Kind) -> bool {
    matches!(kind, Kind::List(item) if matches!(**item, Kind::Other(_)))
}

/// The one kind that values of kinds `a` and `b` can both be stored as, if
/// there is one: `int` and `none` meet in `int or none`.
fn joined(a: &Kind, b: &Kind) -> Option<Kind> {
    match (a, b) {
        _ if a == b => Some(a.clone()),
        // `[[], [1]]`: the empty list goes along with the other one.
        (empty, Kind::List(_)) if is_empty_list(empty) => Some(b.clone()),
        (Kind::List(_), empty) if is_empty_list(empty) => Some(a.clone()),
        (Kind::None, Kind::Optional(_)) => Some(b.clone()),
        (Kind::Optional(_), Kind::None) => Some(a.clone()),
        (Kind::None, other) | (other, Kind::None) => Some(Kind::Optional(Box::new(other.clone()))),
        (Kind::Optional(inner), other) | (other, Kind::Optional(inner)) if **inner == *other => {
            Some(Kind::Optional(inner.clone()))
        }
        // Two functions that take and give back the same things.
        (Kind::Function(x), Kind::Function(y)) => (signature(x).ok()? == signature(y).ok()?).then(|| a.clone()),
        _ => None,
    }
}

/// A short name for a kind, to build the names of helper functions from.
fn key(kind: &Kind) -> String {
    match kind {
        Kind::Int => "int".to_string(),
        Kind::Float => "float".to_string(),
        Kind::Bool => "bool".to_string(),
        Kind::Pin => "pin".to_string(),
        Kind::Str => "str".to_string(),
        Kind::None => "none".to_string(),
        Kind::Quantity(unit) => format!("in_{unit}"),
        Kind::List(item) => format!("list_{}", key(item)),
        Kind::Class(name) | Kind::Enum(name) => name.clone(),
        Kind::Optional(inner) => format!("opt_{}", key(inner)),
        Kind::Error => "error".to_string(),
        Kind::Function(_) => "function".to_string(),
        Kind::Other(_) => "other".to_string(),
        Kind::Owned(inner) => key(inner),
    }
}

/// The IR type that holds a Zeno value of this kind.
fn ir_type(kind: &Kind, span: Span) -> Result<&'static str, Error> {
    Ok(match kind {
        // A pin of the board is its number.
        Kind::Int | Kind::Pin => "i64",
        // A quantity is a float. Its unit has done its work in the
        // checker and costs nothing in the compiled program.
        Kind::Float | Kind::Quantity(_) => "double",
        // One bit.
        Kind::Bool => "i1",
        // Text is the address of its first byte, as in the C backend.
        // Lists, objects, enum values, errors and functions live in
        // memory the collector looks after, and are held by address
        // too, because in Zeno they are shared: `let b = a` makes a
        // second name for the same object, not a copy.
        Kind::Str | Kind::List(_) | Kind::Class(_) | Kind::Enum(_) | Kind::Error | Kind::Function(_) => "ptr",
        // An address can already say "nothing": it is null. A number or
        // a bool cannot, so it travels as a pair with a flag in front.
        Kind::Optional(inner) => match &**inner {
            inner if is_pointer(inner) => "ptr",
            Kind::Int | Kind::Pin => "{ i1, i64 }",
            Kind::Float | Kind::Quantity(_) => "{ i1, double }",
            Kind::Bool => "{ i1, i1 }",
            _ => return Err(cannot("this kind of value that may be `none`", span)),
        },
        Kind::Owned(inner) => ir_type(inner, span)?,
        Kind::None => {
            return Err(cannot("a variable that only ever holds `none`. Give it a type, as in `x: int or none`", span))
        }
        Kind::Other(_) => return Err(cannot(described(kind), span)),
    })
}

/// As `ir_type`, for what a function gives back, which may be nothing.
fn return_type(kind: &Kind, span: Span) -> Result<&'static str, Error> {
    match kind {
        Kind::None => Ok("void"),
        _ => ir_type(kind, span),
    }
}

/// The IR type of the code of a function, for telling whether two
/// function kinds are called the same way.
fn signature(kind: &FunctionKind) -> Result<String, Error> {
    let nowhere = Span { line: 0, col: 0 };
    let mut params = vec!["ptr"];
    for param in &kind.params {
        params.push(ir_type(param, nowhere)?);
    }
    Ok(format!("{} ({})", return_type(&kind.returns, nowhere)?, params.join(", ")))
}

/// What a value of an IR type is before it has been given: all zeros.
fn zero(ty: &str) -> &'static str {
    match ty {
        "i64" => "0",
        "double" => "0.0",
        "i1" => "false",
        "ptr" => "null",
        _ => "zeroinitializer",
    }
}

/// Which of the three flagged kinds holds an optional `inner`.
fn scalar(inner: &Kind, span: Span) -> Result<&'static str, Error> {
    match inner {
        Kind::Int | Kind::Pin => Ok("int"),
        Kind::Float | Kind::Quantity(_) => Ok("float"),
        Kind::Bool => Ok("bool"),
        _ => Err(cannot("this kind of value that may be `none`", span)),
    }
}

/// `line, col` as the last two arguments of a runtime call that can fail.
fn at(span: Span) -> String {
    format!("i32 {}, i32 {}", span.line, span.col)
}

/// A name in the IR: `sigil` is `%` for something inside a function and
/// `@` for something global. Names with anything but plain letters and
/// digits in them are written in quotes, which IR allows.
fn named(sigil: char, prefix: &str, name: &str) -> String {
    if name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.') {
        return format!("{sigil}{prefix}{name}");
    }
    let mut quoted = String::new();
    for byte in name.bytes() {
        match byte {
            b'"' | b'\\' | 0..=0x1f | 0x7f.. => quoted += &format!("\\{byte:02X}"),
            _ => quoted.push(byte as char),
        }
    }
    format!("{sigil}\"{prefix}{quoted}\"")
}

/// A float as IR writes one. IR wants a decimal point in it, and has no
/// spelling for infinity, so anything unusual is written as the 64 bits
/// that make up the float.
fn float_code(x: f64) -> String {
    if !x.is_finite() {
        return format!("0x{:016X}", x.to_bits());
    }
    let text = format!("{x:?}");
    match text.split_once('e') {
        Some((digits, power)) if !digits.contains('.') => format!("{digits}.0e{power}"),
        _ => text,
    }
}

/// The size in bytes of an IR type, as IR can say it without knowing the
/// processor: the address of the second item in an array that starts at
/// address zero.
fn size_of(ty: &str) -> String {
    format!("ptrtoint (ptr getelementptr ({ty}, ptr null, i32 1) to i64)")
}

/// What "nothing" is for a value that may be `none`.
fn nothing(inner: &Kind, span: Span) -> Result<&'static str, Error> {
    if is_pointer(inner) {
        return Ok("null");
    }
    // A flag that says "not here", and whatever is beside it.
    scalar(inner, span).map(|_| "zeroinitializer")
}

/// The struct type of an enum case that carries values, such as
/// `%z_Shape.Circle`: its number first, then what it carries.
fn case_type(owner: &str, case: &str) -> String {
    named('%', "z_", &format!("{owner}.{case}"))
}

/// The one fixed value that stands for an enum case that carries nothing.
fn bare_case(owner: &str, case: &str) -> String {
    named('@', "zeno.case.", &format!("{owner}.{case}"))
}

/// How the IR sees the runtime's `zeno_error`: its kind, its message, and
/// the object that was raised, if one was.
const ERROR: &str = "{ ptr, ptr, ptr }";

// ----------------------------------------------------------------- The file

/// Turn a checked program into the text of one LLVM IR file. `file` is
/// the name of the Zeno source, which the compiled program uses in its
/// error messages.
pub fn emit(program: &Program, types: &Types, file: &str) -> Result<String, Error> {
    let mut emitter = Emitter {
        types,
        functions: HashMap::new(),
        classes: HashMap::new(),
        enums: HashMap::new(),
        cases: HashMap::new(),
        imported: HashMap::new(),
        waiting: VecDeque::new(),
        scopes: vec![HashMap::new()],
        code: Code::of(&program.statements, Kind::None, false),
        closures: 0,
        structs: Vec::new(),
        strings: Vec::new(),
        globals: Vec::new(),
        roots: Vec::new(),
        helpers: Vec::new(),
        written: HashSet::new(),
        definitions: Vec::new(),
        program: String::new(),
        used: BTreeSet::new(),
    };

    // A function can be called above the line it is written on, and a
    // class used above its own, so all of them are made known before
    // anything is compiled.
    for statement in &program.statements {
        match &statement.kind {
            StmtKind::Function(decl) => {
                // `z_` keeps the program's names apart from the runtime's
                // and from the C library's: a Zeno function may well be
                // called `main`.
                let entry = emitter.entry(decl, named('@', "z_", &decl.name), statement.span)?;
                emitter.functions.insert(decl.name.clone(), entry);
            }
            StmtKind::Class(decl) => emitter.register_class(decl, statement.span)?,
            StmtKind::Enum(decl) => emitter.register_enum(decl, statement.span)?,
            _ => {}
        }
    }

    // The top of the file becomes one function, `zeno_program`. Each
    // function it passes on the way is put aside, and written afterwards.
    for statement in &program.statements {
        emitter.statement(statement)?;
    }
    // `noinline` keeps it a function of its own. The collector looks for
    // addresses on the stack below `main`, and this is where they are.
    emitter.program = emitter.finish("define internal void @zeno_program() noinline".to_string());
    while let Some(waiting) = emitter.waiting.pop_front() {
        emitter.function(waiting)?;
    }

    Ok(emitter.file(file))
}

impl Emitter<'_> {
    /// Put the finished parts together.
    fn file(mut self, file: &str) -> String {
        let name = self.string(file);
        let mut out = format!("; {file}, compiled by `zeno build --llvm`.\n");
        out += "; The runtime it calls is in the C file written beside this one.\n\n";

        // The shapes of the program's data: what is in an object of each
        // class, and in a value of each enum case. A list is the
        // runtime's: how many items, room for how many, and where they are.
        out += "%zeno_list = type { i64, i64, ptr }\n";
        for shape in &self.structs {
            out += shape;
            out.push('\n');
        }
        out.push('\n');

        // The text the program mentions. `[6 x i8]` is six bytes: the
        // characters and the zero byte that ends a C string.
        for (text, name) in &self.strings {
            let mut bytes = String::new();
            for byte in text.bytes() {
                match byte {
                    b'"' | b'\\' | 0..=0x1f | 0x7f.. => bytes += &format!("\\{byte:02X}"),
                    _ => bytes.push(byte as char),
                }
            }
            out += &format!("{name} = private unnamed_addr constant [{} x i8] c\"{bytes}\\00\"\n", text.len() + 1);
        }
        out.push('\n');

        // The variables at the top of the file, and the values that are
        // made once and never change.
        for global in &self.globals {
            out += global;
            out.push('\n');
        }
        if !self.globals.is_empty() {
            out.push('\n');
        }

        // What lives in the runtime.
        out += "@zeno_file = external global ptr\n";
        out += "@zeno_stack_base = external global ptr\n";
        out += "@zeno_raised = external global ptr\n";
        self.used.insert("zeno_mark_words");
        for (name, declaration) in DECLARED {
            if self.used.contains(name) {
                out += declaration;
                out.push('\n');
            }
        }
        out.push('\n');
        for (name, written) in WRITTEN {
            if self.used.contains(name) {
                out += written;
                out.push('\n');
            }
        }

        // The functions the compiler wrote for itself, then the program's
        // own as they come in the program, then the top of the file.
        for definition in self.helpers.iter().chain(&self.definitions) {
            out += definition;
            out.push('\n');
        }
        out += &self.program;
        out.push('\n');

        // The collector calls this to be shown every global that may hold
        // the address of something it looks after. Each is one word (8
        // bytes) wide.
        out += "define void @zeno_roots() {\nentry:\n";
        out += "  call void @zeno_mark_words(ptr @zeno_raised, i64 8)\n";
        for root in &self.roots {
            out += &format!("  call void @zeno_mark_words(ptr {root}, i64 8)\n");
        }
        out += "  ret void\n}\n\n";

        // Where the program starts. `%base` is a variable of `main`, so
        // its address marks where the stack begins, for the collector.
        out += "define i32 @main() {\nentry:\n";
        out += "  %base = alloca ptr\n";
        out += "  store ptr %base, ptr @zeno_stack_base\n";
        out += &format!("  store ptr {name}, ptr @zeno_file\n");
        out += "  call void @zeno_program()\n";
        out += "  ret i32 0\n}\n";
        out
    }

    // ------------------------------------------------- Writing instructions

    /// Write one instruction into the block being filled.
    fn ins(&mut self, text: impl AsRef<str>) {
        self.open();
        self.code.lines.push(format!("  {}", text.as_ref()));
    }

    /// Write an instruction that gives a value, and return the new name
    /// for that value.
    fn set(&mut self, text: impl AsRef<str>) -> String {
        self.code.temps += 1;
        let temp = format!("%t{}", self.code.temps);
        self.ins(format!("{temp} = {}", text.as_ref()));
        temp
    }

    /// A number for a new group of labels, so that `then4`, `else4` and
    /// `end4` can be seen to belong together.
    fn number(&mut self) -> usize {
        self.code.labels += 1;
        self.code.labels
    }

    /// Write the instruction that ends the block.
    fn end(&mut self, text: impl AsRef<str>) {
        self.ins(text);
        self.code.ended = true;
    }

    /// Leave the block for another one, unless it has been left already
    /// (by a `return`, say).
    fn jump(&mut self, to: &str) {
        if !self.code.ended {
            self.end(format!("br label %{to}"));
        }
    }

    /// Leave the block for one of two others.
    fn jump_if(&mut self, test: &str, yes: &str, no: &str) {
        self.end(format!("br i1 {test}, label %{yes}, label %{no}"));
    }

    /// Start the block with this label. A block cannot simply run on into
    /// the next one, so if the one before is still open it jumps here.
    fn place(&mut self, label: &str) {
        self.jump(label);
        self.code.lines.push(String::new());
        self.code.lines.push(format!("{label}:"));
        self.code.block = label.to_string();
        self.code.ended = false;
    }

    /// Make sure there is a block to write into. After a `return` there
    /// is none, and what follows can never run. It still has to be valid
    /// IR, so it gets a block of its own that nothing jumps to. LLVM
    /// throws such a block away.
    fn open(&mut self) {
        if self.code.ended {
            let label = format!("dead{}", self.number());
            self.place(&label);
        }
    }

    /// Call something from `DECLARED` or `WRITTEN` and return its result.
    fn call(&mut self, returns: &str, name: &'static str, args: impl AsRef<str>) -> String {
        self.uses(name);
        self.set(format!("call {returns} @{name}({})", args.as_ref()))
    }

    /// The same, for one that gives nothing back.
    fn run(&mut self, name: &'static str, args: impl AsRef<str>) {
        self.uses(name);
        self.ins(format!("call void @{name}({})", args.as_ref()));
    }

    fn uses(&mut self, name: &'static str) {
        let known = DECLARED.iter().chain(WRITTEN).any(|(declared, _)| *declared == name);
        debug_assert!(known, "{name} is not declared");
        self.used.insert(name);
        // The ones written in IR lean on the runtime in their turn.
        if name.starts_with("zeno.box") {
            self.used.insert("zeno_alloc_text");
        } else if name.starts_with("zeno.") && !name.starts_with("zeno.unbox") {
            self.used.insert("zeno_by_zero");
            self.used.insert("zeno_overflow");
        }
    }

    /// The name of a constant holding `text`.
    fn string(&mut self, text: &str) -> String {
        if let Some((_, name)) = self.strings.iter().find(|(known, _)| known == text) {
            return name.clone();
        }
        let name = format!("@.str.{}", self.strings.len() + 1);
        self.strings.push((text.to_string(), name.clone()));
        name
    }

    /// A new slot on the stack of the function being written.
    fn slot(&mut self, name: String, ty: &str) -> String {
        self.code.slots.push(format!("  {name} = alloca {ty}"));
        self.code.on_stack.insert(name.clone());
        name
    }

    /// Read what a slot holds.
    ///
    /// In a function with a `try` in it, the function's own slots are
    /// read and written with `volatile`, which tells LLVM to leave them
    /// in memory and touch them exactly when the program says. Otherwise
    /// LLVM would keep a `var` in a processor register, and the jump that
    /// brings an error back to its `try` puts the registers back as they
    /// were when the `try` began: the `except` block would see the `var`
    /// as it was then. C has the same rule and the same word for it; see
    /// `changed_in_try` in `emit_c`.
    fn load(&mut self, ty: &str, slot: &str) -> String {
        let how = self.how(slot);
        self.set(format!("load{how} {ty}, ptr {slot}"))
    }

    /// Write a value into a slot.
    fn store(&mut self, ty: &str, value: &str, slot: &str) {
        let how = self.how(slot);
        self.ins(format!("store{how} {ty} {value}, ptr {slot}"));
    }

    fn how(&self, slot: &str) -> &'static str {
        if self.code.careful && self.code.on_stack.contains(slot) {
            " volatile"
        } else {
            ""
        }
    }

    /// The address of one field of a struct. `getelementptr` ("get
    /// element pointer", GEP for short) does no reading: it is the sum
    /// "where the struct starts, plus how far in this field is", which
    /// LLVM works out from the type. The first `i32 0` says "the struct
    /// at this address itself", the second number picks the field.
    fn field_at(&mut self, ty: &str, object: &str, index: usize) -> String {
        self.set(format!("getelementptr {ty}, ptr {object}, i32 0, i32 {index}"))
    }

    /// Finish the function being written and return its text.
    fn finish(&mut self, first_line: String) -> String {
        if !self.code.ended {
            match (self.code.returns.clone(), ir_type(&self.code.returns, Span { line: 0, col: 0 })) {
                (Kind::None, _) => self.end("ret void"),
                // A function that may give back `none` does so if it
                // reaches its end without a `return`.
                (Kind::Optional(_), Ok(ty)) => self.end(format!("ret {ty} {}", zero(ty))),
                // The checker made sure every other path gives a value
                // back, so the end of the body is never reached.
                _ => self.end("unreachable"),
            }
        }
        let code = std::mem::replace(&mut self.code, Code::new(Kind::None, false));
        let mut text = format!("{first_line} {{\nentry:\n");
        for line in code.slots.iter().chain(&code.lines) {
            text += line;
            text.push('\n');
        }
        text += "}\n";
        text
    }

    /// Write a function of the compiler's own (one that shows an object,
    /// say), putting aside the function that was being written and taking
    /// it up again afterwards. `write` must end the function itself.
    fn side(&mut self, first_line: String, write: impl FnOnce(&mut Self) -> Result<(), Error>) -> Result<(), Error> {
        let code = std::mem::replace(&mut self.code, Code::new(Kind::None, true));
        let scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
        let done = write(self);
        let text = self.finish(first_line);
        self.code = code;
        self.scopes = scopes;
        done?;
        self.helpers.push(text);
        Ok(())
    }

    // ------------------------------------------- Functions, classes, enums

    /// What a call to this function or method needs to know.
    fn entry(&self, decl: &Arc<Function>, name: String, span: Span) -> Result<Entry, Error> {
        let Some(kind) = self.types.function(decl) else {
            return Err(cannot(format!("`{}`, whose types could not be worked out", decl.name), span));
        };
        let kind = FunctionKind { params: kind.params.iter().map(shared).collect(), returns: shared(&kind.returns) };
        for param in &kind.params {
            ir_type(param, span)?;
        }
        return_type(&kind.returns, span)?;
        Ok(Entry { decl: decl.clone(), kind, name })
    }

    /// A class becomes a struct type: its fields, in the order they are
    /// listed. `%z_Point = type { double, double }`.
    fn register_class(&mut self, decl: &Class, span: Span) -> Result<(), Error> {
        let Some(kinds) = self.types.class(decl) else {
            return Err(cannot(format!("the class `{}`", decl.name), span));
        };
        let kinds: Vec<Kind> = kinds.iter().map(shared).collect();
        let ty = named('%', "z_", &decl.name);
        let mut inside = Vec::new();
        for kind in &kinds {
            inside.push(ir_type(kind, span)?);
        }
        if inside.is_empty() {
            // An object with no fields still needs an address of its own.
            inside.push("i8");
        }
        self.structs.push(format!("{ty} = type {{ {} }}", inside.join(", ")));

        // A method is a function whose first parameter is the object.
        let mut methods = HashMap::new();
        for method in &decl.methods {
            let name = named('@', "z_", &format!("{}.{}", decl.name, method.name));
            methods.insert(method.name.clone(), self.entry(method, name, span)?);
        }
        let fields =
            decl.fields.iter().zip(kinds).map(|(field, kind)| (field.name.clone(), kind, field.default.clone())).collect();
        self.classes.insert(decl.name.clone(), ClassEntry { fields, methods, ty });
        Ok(())
    }

    /// An enum value is the address of a small block that starts with a
    /// number saying which case it is (its "tag"), followed by whatever
    /// that case carries. Each case that carries something has a struct
    /// type of its own: `%z_Shape.Circle = type { i64, double }`.
    ///
    /// In C the cases share one `union`, as big as the biggest. IR has no
    /// unions and needs none: every block starts with the tag, and once
    /// the tag has been read the right struct type is known.
    fn register_enum(&mut self, decl: &Enum, span: Span) -> Result<(), Error> {
        let Some(kinds) = self.types.enumeration(decl).cloned() else {
            return Err(cannot(format!("the enum `{}`", decl.name), span));
        };
        let mut cases = Vec::new();
        for (index, (case, case_kinds)) in decl.variants.iter().zip(&kinds).enumerate() {
            self.cases.insert(case.name.clone(), (decl.name.clone(), index));
            let fields: Vec<(String, Kind)> =
                case.fields.iter().map(|field| field.name.clone()).zip(case_kinds.iter().map(shared)).collect();
            if fields.is_empty() {
                // A case that carries nothing needs no memory of its own:
                // one fixed value stands for every use of it.
                let value = bare_case(&decl.name, &case.name);
                self.globals.push(format!("{value} = internal constant {{ i64 }} {{ i64 {index} }}"));
            } else {
                let mut inside = vec!["i64"];
                for (_, kind) in &fields {
                    inside.push(ir_type(kind, span)?);
                }
                self.structs.push(format!("{} = type {{ {} }}", case_type(&decl.name, &case.name), inside.join(", ")));
            }
            cases.push((case.name.clone(), fields));
        }
        self.enums.insert(decl.name.clone(), EnumEntry { cases });
        Ok(())
    }

    /// The parameters of a function: as variables for its body, and as
    /// they are written on its first line. A parameter cannot be changed,
    /// so the value that arrives is the variable.
    fn parameters(
        &self,
        decl: &Function,
        kind: &FunctionKind,
        span: Span,
    ) -> Result<(HashMap<String, Variable>, Vec<String>), Error> {
        let mut params = HashMap::new();
        let mut list = Vec::new();
        for (param, kind) in decl.params.iter().zip(&kind.params) {
            let arrives = named('%', "arg.", &param.name);
            list.push(format!("{} {arrives}", ir_type(kind, span)?));
            params.insert(param.name.clone(), Variable { place: Place::Value(arrives), kind: kind.clone(), open: false });
        }
        Ok((params, list))
    }

    /// Write out one function or method.
    fn function(&mut self, waiting: Waiting) -> Result<(), Error> {
        let Waiting { entry, span, sees } = waiting;
        self.code = Code::of(&entry.decl.body, entry.kind.returns.clone(), true);
        let (params, list) = self.parameters(&entry.decl, &entry.kind, span)?;
        let outer = std::mem::replace(&mut self.scopes, vec![sees, params]);
        let done = self.block(&entry.decl.body);
        self.scopes = outer;
        done?;

        // `internal` is what `static` is in C: nothing outside this file
        // needs the function, so LLVM may inline it and drop it.
        let returns = return_type(&entry.kind.returns, span)?;
        let text = self.finish(format!("define internal {returns} {}({})", entry.name, list.join(", ")));
        self.definitions.push(text);
        Ok(())
    }

    /// The Zeno type written in the source, as a kind.
    fn kind_of_type(&self, ty: &Type) -> Option<Kind> {
        let (name, args) = match ty {
            Type::Named { name, args } => (name, args),
            // Who owns a value does not change how it is held.
            Type::Owned(inner) | Type::Borrow(inner) => return self.kind_of_type(inner),
            Type::Optional(inner) => return Some(Kind::Optional(Box::new(self.kind_of_type(inner)?))),
            Type::Function { params, returns } => {
                let params = params.iter().map(|param| self.kind_of_type(param)).collect::<Option<Vec<Kind>>>()?;
                let returns = match returns {
                    Some(returns) => self.kind_of_type(returns)?,
                    None => Kind::None,
                };
                return Some(Kind::Function(Box::new(FunctionKind { params, returns })));
            }
        };
        Some(match (name.as_str(), args.as_slice()) {
            ("int", []) => Kind::Int,
            ("float", []) => Kind::Float,
            ("str", []) => Kind::Str,
            ("bool", []) => Kind::Bool,
            ("list", [item]) => Kind::List(Box::new(self.kind_of_type(item)?)),
            (name, []) if self.classes.contains_key(name) => Kind::Class(name.to_string()),
            (name, []) if self.enums.contains_key(name) => Kind::Enum(name.to_string()),
            (name, []) if units::is_unit(name) => Kind::Quantity(name.to_string()),
            ("Pin", []) => Kind::Pin,
            _ => return None,
        })
    }

    // ----------------------------------------------------------- Statements

    fn block(&mut self, statements: &[Stmt]) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        let done = statements.iter().try_for_each(|statement| self.statement(statement));
        self.scopes.pop();
        done
    }

    /// Is this the top of the file, outside every function and block?
    fn at_the_top(&self) -> bool {
        !self.code.in_function && self.scopes.len() == 1
    }

    fn lookup(&self, name: &str) -> Option<&Variable> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), Error> {
        let span = statement.span;
        self.open();
        match &statement.kind {
            StmtKind::Let { name, ty, value, .. } | StmtKind::Const { name, ty, value } => {
                let mutable = matches!(statement.kind, StmtKind::Let { mutable: true, .. });
                // A type written beside the name settles what `none` and
                // `[]` mean here.
                let expected = ty.as_ref().and_then(|ty| self.kind_of_type(ty));
                let mut val = self.expr_as(value, expected.as_ref())?;
                if let Some(expected) = &expected {
                    val = self.coerce(val, expected, value.span)?;
                }
                if is_empty_list(&val.kind) {
                    return Err(cannot(
                        "an empty list with no type beside it. Write `let items: list[int] = []`",
                        value.span,
                    ));
                }
                self.declare(name, val, mutable, value.span)
            }
            StmtKind::Assign { target, op, value } => self.assign(target, *op, value, span),
            StmtKind::Expr(expr) => self.expr(expr).map(|_| ()),

            StmtKind::Function(decl) => {
                if !self.at_the_top() {
                    return Err(cannot("a function with a name inside a function or a block", span));
                }
                // It is written out later. It may use the variables
                // declared so far and no others, so a copy of them goes
                // with it.
                let entry = self.functions[&decl.name].clone();
                self.waiting.push_back(Waiting { entry, span, sees: self.scopes[0].clone() });
                Ok(())
            }
            StmtKind::Class(decl) => {
                if !self.at_the_top() {
                    return Err(cannot("a class inside a function or a block", span));
                }
                // Its methods are functions, and wait like functions.
                for method in &decl.methods {
                    let entry = self.classes[&decl.name].methods[&method.name].clone();
                    self.waiting.push_back(Waiting { entry, span, sees: self.scopes[0].clone() });
                }
                Ok(())
            }
            StmtKind::Enum(_) if self.at_the_top() => Ok(()),
            StmtKind::Enum(_) => Err(cannot("an enum inside a function or a block", span)),

            StmtKind::If { condition, then_block, else_block } => {
                let test = self.condition(condition)?;
                let n = self.number();
                let (then, otherwise, end) = (format!("then{n}"), format!("else{n}"), format!("end{n}"));
                self.jump_if(&test, &then, if else_block.is_some() { &otherwise } else { &end });
                self.place(&then);
                self.block(then_block)?;
                if let Some(else_block) = else_block {
                    // `place` adds the jump from the end of `then` to `end`.
                    self.jump(&end);
                    self.place(&otherwise);
                    self.block(else_block)?;
                }
                self.place(&end);
                Ok(())
            }

            StmtKind::While { condition, body } => {
                let n = self.number();
                let (test, body_label, done) = (format!("while{n}"), format!("do{n}"), format!("done{n}"));
                // The test is a block of its own, because it is jumped
                // back to after every trip.
                self.place(&test);
                let holds = self.condition(condition)?;
                self.jump_if(&holds, &body_label, &done);
                self.place(&body_label);
                self.code.loops.push(Loop { next: test.clone(), done: done.clone(), tries: self.code.tries.len() });
                let result = self.block(body);
                self.code.loops.pop();
                result?;
                self.jump(&test);
                self.place(&done);
                Ok(())
            }

            StmtKind::For { variable, iterable, body } => {
                let count = self.open_loop(variable, iterable)?;
                self.code.loops.push(Loop {
                    next: count.next.clone(),
                    done: count.done.clone(),
                    tries: self.code.tries.len(),
                });
                let result = self.block(body);
                self.code.loops.pop();
                self.close_loop(&count);
                result
            }

            StmtKind::Match { subject, arms } => self.match_statement(subject, arms, span),
            StmtKind::Try { body, handlers } => self.try_statement(body, handlers, span),
            StmtKind::Raise(value) => self.raise(value, span),

            StmtKind::Return(value) => {
                if !self.code.in_function {
                    return Err(cannot("`return` outside a function", span));
                }
                let returns = self.code.returns.clone();
                match value {
                    Some(value) if returns != Kind::None => {
                        let val = self.expr_as(value, Some(&returns))?;
                        let val = self.fit(val, &returns, span)?;
                        // The value is worked out while any `try` around
                        // this line is still watching. Then the `try` is
                        // left, and then the function.
                        self.leave_tries(0);
                        self.end(format!("ret {} {}", ir_type(&returns, span)?, val.code));
                    }
                    Some(value) if !matches!(value.kind, ExprKind::None) => {
                        return Err(cannot("returning a value from a function that returns nothing", span))
                    }
                    _ => {
                        self.leave_tries(0);
                        self.end("ret void");
                    }
                }
                Ok(())
            }
            StmtKind::Break | StmtKind::Continue => {
                let Some(Loop { next, done, tries }) = self.code.loops.last() else {
                    return Err(cannot("`break` or `continue` outside a loop", span));
                };
                let to = if statement.kind == StmtKind::Break { done.clone() } else { next.clone() };
                let tries = *tries;
                self.leave_tries(tries);
                self.jump(&to);
                Ok(())
            }

            StmtKind::Import { module, names } => {
                let module = module.join(".");
                if !matches!(module.as_str(), "math" | "time" | "board") {
                    return Err(cannot(format!("the module `{module}`"), span));
                }
                for name in names {
                    self.imported.insert(name.clone(), module.clone());
                }
                Ok(())
            }
        }
    }

    /// `let name = val` or `var name = val`.
    fn declare(&mut self, name: &str, val: Val, mutable: bool, span: Span) -> Result<(), Error> {
        let ty = ir_type(&val.kind, span)?;
        let (place, open) = if self.at_the_top() {
            // Functions use the variables at the top of the file, so
            // those live in memory that is there for the whole run.
            // `internal global` is a `static` variable in C.
            let global = named('@', "z_", name);
            self.globals.push(format!("{global} = internal global {ty} {}", zero(ty)));
            if holds_pointer(&val.kind) {
                self.roots.push(global.clone());
            }
            self.ins(format!("store {ty} {}, ptr {global}", val.code));
            (Place::Slot(global), mutable)
        } else if mutable && self.code.shared.contains(name) {
            // A function written inside this one uses the variable too,
            // and either may change it, perhaps long after this function
            // has returned. So it lives in a box of its own that the
            // collector looks after, and both hold the box's address.
            let boxed = self.call("ptr", "zeno_alloc", format!("i64 {}", size_of(ty)));
            self.ins(format!("store {ty} {}, ptr {boxed}", val.code));
            (Place::Slot(boxed), true)
        } else if mutable {
            // A `var` gets a slot on the stack.
            let count = self.code.named.entry(name.to_string()).or_insert(0);
            *count += 1;
            let slot = match *count {
                1 => named('%', "var.", name),
                count => named('%', "var.", &format!("{name}.{count}")),
            };
            let slot = self.slot(slot, ty);
            self.store(ty, &val.code, &slot);
            (Place::Slot(slot), false)
        } else {
            // A `let` never changes: the value itself is the variable.
            (Place::Value(val.code), false)
        };
        self.scopes.last_mut().expect("a scope").insert(name.to_string(), Variable { place, kind: val.kind, open });
        Ok(())
    }

    /// `x = v`, `point.x = v`, `items[i] = v`, and the same with `+=`.
    fn assign(&mut self, target: &Expr, op: Option<BinaryOp>, value: &Expr, span: Span) -> Result<(), Error> {
        // The new value first, then the old one, then the place it goes
        // to, as `zeno run` does.
        let mut new = self.expr(value)?;
        if let Some(op) = op {
            let old = self.expr(target)?;
            new = self.arithmetic(op, old, new, span)?;
        }
        match &target.kind {
            ExprKind::Name(name) => {
                let Some(Variable { place: Place::Slot(slot), kind, .. }) = self.lookup(name).cloned() else {
                    return Err(cannot(format!("changing `{name}`"), target.span));
                };
                let new = self.fit(new, &kind, span)?;
                self.store(ir_type(&kind, span)?, &new.code, &slot);
            }
            ExprKind::Field { object, name } => {
                let object = self.expr(object)?;
                let field = match &object.kind {
                    Kind::Class(class) => self.field_of(class, name),
                    _ => None,
                };
                let Some((ty, index, kind)) = field else {
                    return Err(cannot("an assignment to this", target.span));
                };
                let new = self.fit(new, &kind, span)?;
                let at = self.field_at(&ty, &object.code, index);
                self.ins(format!("store {} {}, ptr {at}", ir_type(&kind, span)?, new.code));
            }
            ExprKind::Index { object, index } => {
                let object = self.expr(object)?;
                let index = self.expr(index)?;
                let Kind::List(item) = object.kind.clone() else {
                    return Err(cannot("an assignment to this", target.span));
                };
                let new = self.fit(new, &item, span)?;
                let slot = self.to_slot(&new, span)?;
                // The runtime checks the index and gives back the address
                // of that item's slot.
                let args = format!("ptr {}, i64 {}, {}", object.code, index.code, at(span));
                let place = self.call("ptr", "zeno_list_at", args);
                self.ins(format!("store i64 {slot}, ptr {place}"));
            }
            _ => return Err(cannot("an assignment to this", target.span)),
        }
        Ok(())
    }

    /// Where a field of a class is: the class's struct type, the field's
    /// number, and its kind.
    fn field_of(&self, class: &str, name: &str) -> Option<(String, usize, Kind)> {
        let entry = self.classes.get(class)?;
        let index = entry.fields.iter().position(|(field, _, _)| field == name)?;
        Some((entry.ty.clone(), index, entry.fields[index].1.clone()))
    }

    // ---------------------------------------------------------------- Loops

    /// Begin a loop that counts up by one from `start`. The counter
    /// changes, so it has a slot, like a `var`. Returns the loop, and the
    /// counter as it stands at the top of this trip. The caller decides
    /// whether there is another trip, and says so with `count_body`.
    fn count_open(&mut self, variable: &str, start: &str) -> (Count, String) {
        let n = self.number();
        let counter = self.slot(named('%', "for.", &format!("{variable}.{n}")), "i64");
        self.store("i64", start, &counter);
        let count = Count {
            counter,
            test: format!("for{n}"),
            body: format!("do{n}"),
            next: format!("next{n}"),
            done: format!("done{n}"),
        };
        self.place(&count.test);
        let trip = self.load("i64", &count.counter);
        (count, trip)
    }

    /// The test is written: go round again if `more` holds.
    fn count_body(&mut self, count: &Count, more: &str) {
        self.jump_if(more, &count.body, &count.done);
        self.place(&count.body);
    }

    /// The end of the loop's body. `continue` comes here: add one and
    /// test again. The counter is below its limit, so adding one cannot
    /// overflow.
    fn count_close(&mut self, count: &Count) {
        self.place(&count.next);
        let trip = self.load("i64", &count.counter);
        let up = self.set(format!("add i64 {trip}, 1"));
        self.store("i64", &up, &count.counter);
        self.jump(&count.test);
        self.place(&count.done);
    }

    /// Begin a loop over the items of the list at `items`. Returns the
    /// loop, and the slot of this trip's item.
    fn walk_open(&mut self, variable: &str, items: &str) -> (Count, String) {
        let (count, trip) = self.count_open(variable, "0");
        // A list starts with how many items it has.
        let len = self.set(format!("load i64, ptr {items}"));
        let more = self.set(format!("icmp slt i64 {trip}, {len}"));
        self.count_body(&count, &more);
        // Its third field is where the items are, one slot after another.
        // This `getelementptr` has no `i32 0` in front: it steps along an
        // array of slots, where the others stepped into a struct.
        let array_at = self.field_at("%zeno_list", items, 2);
        let array = self.set(format!("load ptr, ptr {array_at}"));
        let slot_at = self.set(format!("getelementptr i64, ptr {array}, i64 {trip}"));
        let slot = self.set(format!("load i64, ptr {slot_at}"));
        (count, slot)
    }

    /// The first part of a `for` loop: `for i in range(...)` or `for x in
    /// items`. It declares the loop variable in a scope of its own. The
    /// caller writes the body and then calls `close_loop`.
    fn open_loop(&mut self, variable: &str, iterable: &Expr) -> Result<Count, Error> {
        let span = iterable.span;
        let range = match &iterable.kind {
            ExprKind::Call { callee, args }
                if matches!(&callee.kind, ExprKind::Name(name) if name == "range" && self.lookup(name).is_none())
                    && !self.functions.contains_key("range")
                    && args.iter().all(|arg| arg.name.is_none()) =>
            {
                Some(args)
            }
            _ => None,
        };
        let (count, value) = match range {
            // A range becomes a plain counting loop: no list of numbers
            // is built.
            Some(bounds) => {
                // The ends are worked out once, before the loop.
                let mut ends = Vec::new();
                for bound in bounds {
                    let val = self.expr(&bound.value)?;
                    if val.kind != Kind::Int {
                        return Err(cannot("a `range` of anything but ints", span));
                    }
                    ends.push(val.code);
                }
                // How many trips, and how to get the loop's number from
                // the trip.
                let (start, stop, step) = match ends.as_slice() {
                    [stop] => ("0".to_string(), stop.clone(), None),
                    [start, stop] => (start.clone(), stop.clone(), None),
                    // With a step, count the trips first. Stepping up to
                    // `stop` directly could run past the end of what an
                    // int holds.
                    [start, _, step] => {
                        let args = format!("i64 {}, {}", ends.join(", i64 "), at(span));
                        let trips = self.call("i64", "zeno_range_count", args);
                        ("0".to_string(), trips, Some((start.clone(), step.clone())))
                    }
                    _ => return Err(cannot("this `range`", span)),
                };
                let (count, trip) = self.count_open(variable, &start);
                let more = self.set(format!("icmp slt i64 {trip}, {stop}"));
                self.count_body(&count, &more);
                let number = match step {
                    None => trip,
                    Some((start, step)) => {
                        let along = self.set(format!("mul i64 {trip}, {step}"));
                        self.set(format!("add i64 {start}, {along}"))
                    }
                };
                (count, Val::new(number, Kind::Int))
            }
            // `for x in items`. The loop walks the list as it was when
            // the loop began, so a body that adds to the list or removes
            // from it cannot confuse the loop. That is what `zeno run`
            // does too.
            None => {
                // A list written out right here is nobody else's, so it
                // needs no copy.
                let fresh = matches!(iterable.kind, ExprKind::List(_) | ExprKind::ListComp { .. });
                let list = self.expr(iterable)?;
                let (items, item) = match &list.kind {
                    Kind::List(item) if fresh => (list.code.clone(), (**item).clone()),
                    Kind::List(item) => {
                        (self.call("ptr", "zeno_list_copy", format!("ptr {}", list.code)), (**item).clone())
                    }
                    // A str is looped over one character at a time.
                    Kind::Str => (self.call("ptr", "zeno_chars", format!("ptr {}", list.code)), Kind::Str),
                    _ => return Err(cannot("a `for` loop over anything but a list, a str or `range(...)`", span)),
                };
                let (count, slot) = self.walk_open(variable, &items);
                (count, self.from_slot(&item, &slot, span)?)
            }
        };
        // What the body sees is a value, which it cannot change.
        self.scopes.push(HashMap::from([(
            variable.to_string(),
            Variable { place: Place::Value(value.code), kind: value.kind, open: false },
        )]));
        Ok(count)
    }

    fn close_loop(&mut self, count: &Count) {
        self.scopes.pop();
        self.count_close(count);
    }

    // ---------------------------------------------------------------- Match

    /// `match`: the cases are tried in order. Each pattern is a row of
    /// tests, and a test that fails jumps to the next case.
    ///
    /// ```text
    ///   %tag = load i64, ptr %shape          ; case Circle(r)
    ///   %t1 = icmp eq i64 %tag, 0
    ///   br i1 %t1, label %is2, label %case1.1
    /// is2:
    ///   ...read r out of the block, run the body...
    ///   br label %end1
    /// case1.1:                               ; the next case
    /// ```
    fn match_statement(&mut self, subject: &Expr, arms: &[Arm], span: Span) -> Result<(), Error> {
        let subject = self.expr(subject)?;
        let n = self.number();
        let end = format!("end{n}");
        for (index, arm) in arms.iter().enumerate() {
            let next = format!("case{n}.{}", index + 1);
            // The names the pattern gives are variables of this case.
            self.scopes.push(HashMap::new());
            let result = match self.pattern(&arm.pattern, &subject, &next, span) {
                Ok(tested) => self.block(&arm.body).map(|()| tested),
                Err(error) => Err(error),
            };
            self.scopes.pop();
            let tested = result?;
            self.jump(&end);
            if !tested {
                break; // nothing after a pattern that takes everything can run
            }
            self.place(&next);
        }
        self.place(&end);
        Ok(())
    }

    /// Write the tests of one pattern against `val`, each jumping to
    /// `fail` if it does not hold, and declare the names the pattern
    /// gives. Returns whether there was anything to test.
    fn pattern(&mut self, pattern: &Pattern, val: &Val, fail: &str, span: Span) -> Result<bool, Error> {
        match pattern {
            Pattern::Wildcard => Ok(false),
            Pattern::Name(name) => match (self.cases.get(name), &val.kind) {
                // A bare enum case, such as `Idle`, is compared.
                (Some((owner, index)), Kind::Enum(subject)) if owner == subject && self.lookup(name).is_none() => {
                    let index = *index;
                    self.tag_is(&val.code, index, fail);
                    Ok(true)
                }
                // Any other name is a new variable holding the value.
                _ => {
                    let variable = Variable { place: Place::Value(val.code.clone()), kind: val.kind.clone(), open: false };
                    self.scopes.last_mut().expect("a scope").insert(name.clone(), variable);
                    Ok(false)
                }
            },
            Pattern::Literal(literal) => {
                let literal = self.expr(literal)?;
                let Some(common) = joined(&val.kind, &literal.kind) else {
                    return Err(cannot("a pattern of a different type from the value", span));
                };
                let subject = self.coerce(val.clone(), &common, span)?;
                let literal = self.coerce(literal, &common, span)?;
                let same = self.equal(&common, &subject.code, &literal.code, span)?;
                self.must(&same, fail);
                Ok(true)
            }
            Pattern::Variant { name, fields } => {
                let (owner, index) = match (self.cases.get(name), &val.kind) {
                    (Some((owner, index)), Kind::Enum(subject)) if owner == subject => (owner.clone(), *index),
                    _ => return Err(cannot(format!("the pattern `{name}(...)` here"), span)),
                };
                let carried = self.enums[&owner].cases[index].1.clone();
                if carried.len() != fields.len() {
                    return Err(cannot(format!("the pattern `{name}(...)` with this many parts"), span));
                }
                self.tag_is(&val.code, index, fail);
                // Now it is known which case this is, and so which struct
                // type describes the block. What it carries comes after
                // the tag.
                let ty = case_type(&owner, name);
                for (position, (field, (_, kind))) in fields.iter().zip(&carried).enumerate() {
                    if *field == Pattern::Wildcard {
                        continue;
                    }
                    let at = self.field_at(&ty, &val.code, position + 1);
                    let inside = self.set(format!("load {}, ptr {at}", ir_type(kind, span)?));
                    self.pattern(field, &Val::new(inside, kind.clone()), fail, span)?;
                }
                Ok(true)
            }
        }
    }

    /// Go on only if `test` holds. Otherwise jump to `fail`.
    fn must(&mut self, test: &str, fail: &str) {
        let held = format!("is{}", self.number());
        self.jump_if(test, &held, fail);
        self.place(&held);
    }

    /// Go on only if the enum value at `value` is case number `index`.
    fn tag_is(&mut self, value: &str, index: usize, fail: &str) {
        let tag = self.set(format!("load i64, ptr {value}"));
        let same = self.set(format!("icmp eq i64 {tag}, {index}"));
        self.must(&same, fail);
    }

    // --------------------------------------------------------------- Errors

    /// `try { ... } except Kind as name { ... }`
    ///
    /// This is built on C's `setjmp` and `longjmp`, as in the C backend.
    /// `setjmp` remembers a place in the program and gives 0. If anything
    /// inside the body raises, however deep in function calls, the
    /// runtime's `longjmp` comes back to that same `setjmp`, which this
    /// time gives 1.
    ///
    /// ```text
    ///   call void @zeno_try_open(ptr %try.1)     ; errors come here now
    ///   %t1 = call i32 @_setjmp(ptr %try.1) returns_twice
    ///   %t2 = icmp eq i32 %t1, 0
    ///   br i1 %t2, label %try1, label %caught1
    /// try1:
    ///   ...the body...
    ///   call void @zeno_try_close(ptr %try.1)    ; finished with no error
    ///   br label %end1
    /// caught1:                                   ; an error jumped back
    ///   %t3 = call zeroext i1 @zeno_caught(ptr @.str.4)
    ///   br i1 %t3, label %except1.1, label %caught1.1
    /// except1.1:
    ///   ...the except block...
    ///   br label %end1
    /// caught1.1:
    ///   call void @zeno_raise_again()            ; not ours: pass it on
    ///   unreachable
    /// ```
    fn try_statement(&mut self, body: &[Stmt], handlers: &[Handler], span: Span) -> Result<(), Error> {
        let n = self.number();
        // The place to come back to lives on this function's stack. Its
        // size is asked of the runtime (see `FOR_TRY`).
        let place = format!("%try.{n}");
        self.uses("zeno_try_size");
        self.code.slots.push(format!("  %try.size.{n} = call i64 @zeno_try_size()"));
        self.code.slots.push(format!("  {place} = alloca i8, i64 %try.size.{n}, align 16"));

        self.run("zeno_try_open", format!("ptr {place}"));
        self.uses("_setjmp");
        let came = self.set(format!("call i32 @_setjmp(ptr {place}) returns_twice"));
        let first = self.set(format!("icmp eq i32 {came}, 0"));
        let (inside, caught, end) = (format!("try{n}"), format!("caught{n}"), format!("end{n}"));
        self.jump_if(&first, &inside, &caught);

        self.place(&inside);
        self.code.tries.push(place.clone());
        let result = self.block(body);
        self.code.tries.pop();
        result?;
        // Reaching the end of the body means nothing was raised.
        if !self.code.ended {
            self.run("zeno_try_close", format!("ptr {place}"));
            self.jump(&end);
        }

        // The runtime has already taken this `try` off its list by the
        // time an error arrives here.
        self.place(&caught);
        let mut catches_everything = false;
        for (index, handler) in handlers.iter().enumerate() {
            // What the `except` asks for, and what `as name` then holds.
            let (wanted, holds) = match handler.error.as_deref() {
                None | Some("Error") => (None, Kind::Error),
                Some(class) if self.classes.contains_key(class) => (Some(class), Kind::Class(class.to_string())),
                Some(kind) if emit_c::ERROR_KINDS.contains(&kind) => (Some(kind), Kind::Error),
                Some(other) => return Err(cannot(format!("`except {other}`"), span)),
            };
            let next = format!("caught{n}.{}", index + 1);
            if let Some(kind) = wanted {
                let kind = self.string(kind);
                let ours = self.call("i1", "zeno_caught", format!("ptr {kind}"));
                let handle = format!("except{n}.{}", index + 1);
                self.jump_if(&ours, &handle, &next);
                self.place(&handle);
            }
            self.scopes.push(HashMap::new());
            if let Some(name) = &handler.binding {
                let error = self.set("load ptr, ptr @zeno_raised");
                let value = match holds {
                    // An object that was raised travels inside the error.
                    Kind::Class(_) => {
                        let at = self.field_at(ERROR, &error, 2);
                        self.set(format!("load ptr, ptr {at}"))
                    }
                    _ => error,
                };
                let variable = Variable { place: Place::Value(value), kind: holds, open: false };
                self.scopes.last_mut().expect("a scope").insert(name.clone(), variable);
            }
            let result = self.block(&handler.body);
            self.scopes.pop();
            result?;
            self.jump(&end);
            if wanted.is_none() {
                catches_everything = true;
                break; // nothing gets past an `except` that takes every error
            }
            self.place(&next);
        }
        if !catches_everything {
            self.run("zeno_raise_again", "");
            self.end("unreachable");
        }
        self.place(&end);
        Ok(())
    }

    /// Before a `return`, `break` or `continue` jumps out of `try`
    /// blocks, tell the runtime that those blocks are over. `keep` is how
    /// many of the open ones stay open.
    fn leave_tries(&mut self, keep: usize) {
        if let Some(outermost) = self.code.tries.get(keep).cloned() {
            self.run("zeno_try_close", format!("ptr {outermost}"));
        }
    }

    /// `raise`. The runtime jumps to the nearest `try` that is running,
    /// or stops the program if there is none. Either way it does not
    /// come back here, which is what `unreachable` says.
    fn raise(&mut self, value: &Expr, span: Span) -> Result<(), Error> {
        let val = self.expr(value)?;
        let error = match &val.kind {
            Kind::Error => val.code,
            // An object of a class can be raised too. It travels inside
            // an error value whose kind is the class's name.
            Kind::Class(name) => {
                let kind = self.string(name);
                let shown = self.text(&val, span)?;
                self.call("ptr", "zeno_error_new", format!("ptr {kind}, ptr {shown}, ptr {}", val.code))
            }
            _ => return Err(cannot("`raise` with anything but an error or an object", span)),
        };
        self.run("zeno_raise", format!("ptr {error}, {}", at(span)));
        self.end("unreachable");
        Ok(())
    }

    // ---------------------------------------------------------- Expressions

    /// An expression that must be a bool, for `if` and `while`.
    fn condition(&mut self, expr: &Expr) -> Result<String, Error> {
        let val = self.expr(expr)?;
        match val.kind {
            Kind::Bool => Ok(val.code),
            _ => Err(cannot(format!("{} used as a condition", described(&val.kind)), expr.span)),
        }
    }

    /// `expr`, for a place that knows what kind it wants. Only an empty
    /// list needs the help: `[]` alone does not say what it will hold.
    fn expr_as(&mut self, expr: &Expr, expected: Option<&Kind>) -> Result<Val, Error> {
        match (&expr.kind, expected) {
            (ExprKind::List(items), Some(kind @ Kind::List(_))) if items.is_empty() => {
                Ok(Val::new(self.call("ptr", "zeno_list_new", "i64 0"), kind.clone()))
            }
            _ => self.expr(expr),
        }
    }

    /// Write the instructions that work an expression out, and return
    /// the value they leave.
    fn expr(&mut self, expr: &Expr) -> Result<Val, Error> {
        let span = expr.span;
        Ok(match &expr.kind {
            // The lexer lets one number through that is one too big for an
            // int, and `zeno run` reports it when it gets there.
            ExprKind::Int(n) if *n > i64::MAX as u64 => return Err(cannot("a whole number this big", span)),
            ExprKind::Int(n) => Val::new(n.to_string(), Kind::Int),
            ExprKind::Float(x) => Val::new(float_code(*x), Kind::Float),
            ExprKind::Bool(b) => Val::new(b.to_string(), Kind::Bool),
            ExprKind::Str(text) => Val::new(self.string(text), Kind::Str),
            ExprKind::FormatStr(parts) => self.format_string(parts, span)?,
            // What `none` is in IR depends on where it is going, so it is
            // settled there. See `coerce`.
            ExprKind::None => Val::new("null", Kind::None),
            ExprKind::Name(name) => self.name(name, expr)?,

            // A quantity is a float. Its unit has done its work in the
            // checker and costs nothing in the compiled program.
            ExprKind::Unit { value, unit } => {
                let val = self.expr(value)?;
                if !matches!(val.kind, Kind::Int | Kind::Float) {
                    return Err(cannot(format!("{} with a unit", described(&val.kind)), span));
                }
                Val::new(self.double(&val), Kind::Quantity(unit.clone()))
            }

            ExprKind::Unary { op: UnaryOp::Neg, operand } => match &operand.kind {
                // `-5` is a number, not a sum to be worked out.
                ExprKind::Int(n) if *n <= i64::MAX as u64 => Val::new(format!("-{n}"), Kind::Int),
                _ => {
                    let val = self.expr(operand)?;
                    match val.kind {
                        Kind::Int => Val::new(self.checked("ssub", "0", &val.code, span), Kind::Int),
                        Kind::Float | Kind::Quantity(_) => {
                            Val::new(self.set(format!("fneg double {}", val.code)), val.kind)
                        }
                        _ => return Err(cannot(format!("`-` in front of {}", described(&val.kind)), span)),
                    }
                }
            },
            // Every bit turned over: exclusive-or with a number that is
            // all ones, which is -1.
            ExprKind::Unary { op: UnaryOp::BitNot, operand } => {
                let val = self.expr(operand)?;
                if val.kind != Kind::Int {
                    return Err(cannot(format!("`~` in front of {}", described(&val.kind)), span));
                }
                Val::new(self.set(format!("xor i64 {}, -1", val.code)), Kind::Int)
            }
            ExprKind::Unary { op: UnaryOp::Not, operand } => {
                let test = self.condition(operand)?;
                // There is no "not" instruction. Exclusive-or with 1 flips the bit.
                Val::new(self.set(format!("xor i1 {test}, true")), Kind::Bool)
            }

            ExprKind::Binary { op: op @ (BinaryOp::And | BinaryOp::Or), left, right } => {
                self.logic(*op, left, right)?
            }
            ExprKind::Binary { op: op @ (BinaryOp::Is | BinaryOp::IsNot), left, .. } => self.is_none(*op, left)?,
            ExprKind::Binary { op, left, right } => {
                let left = self.expr(left)?;
                let right = self.expr(right)?;
                match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::FloorDiv
                    | BinaryOp::Mod
                    | BinaryOp::Pow
                    | BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Shl
                    | BinaryOp::Shr => self.arithmetic(*op, left, right, span)?,
                    BinaryOp::In | BinaryOp::NotIn => self.contains(*op, left, right, span)?,
                    _ => Val::new(self.compare(*op, &left, &right, span)?, Kind::Bool),
                }
            }
            ExprKind::Compare { first, rest } => self.chain(first, rest)?,

            ExprKind::Call { callee, args } => self.call_expr(callee, args, span)?,
            ExprKind::Field { object, name } => self.field(object, name, span)?,
            ExprKind::Index { object, index } => {
                let list = self.expr(object)?;
                let position = self.expr(index)?;
                let args = format!("ptr {}, i64 {}, {}", list.code, position.code, at(span));
                match &list.kind {
                    _ if position.kind != Kind::Int => return Err(cannot("`[ ]` with anything but an int inside", span)),
                    Kind::Str => Val::new(self.call("ptr", "zeno_char_at", args), Kind::Str),
                    // The runtime checks the index and gives back the
                    // address of that item's slot.
                    Kind::List(item) => {
                        let place = self.call("ptr", "zeno_list_at", args);
                        let slot = self.set(format!("load i64, ptr {place}"));
                        self.from_slot(item, &slot, span)?
                    }
                    _ => return Err(cannot(format!("`[ ]` on {}", described(&list.kind)), span)),
                }
            }
            ExprKind::List(items) => self.list(items, span)?,
            ExprKind::ListComp { element, variable, iterable, condition } => {
                self.comprehension(element, variable, iterable, condition.as_deref(), span)?
            }
            ExprKind::Function(decl) => self.closure(decl, span)?,
        })
    }

    /// A name used as a value: a variable, an enum case that carries
    /// nothing, `pi`, or one of the program's functions.
    fn name(&mut self, name: &str, expr: &Expr) -> Result<Val, Error> {
        let span = expr.span;
        let Some(variable) = self.lookup(name).cloned() else {
            if let Some((owner, index)) = self.cases.get(name).cloned() {
                if !self.enums[&owner].cases[index].1.is_empty() {
                    return Err(cannot("an enum case that carries values, used without them", span));
                }
                return Ok(Val::new(bare_case(&owner, name), Kind::Enum(owner)));
            }
            if name == "pi" && self.imported.get(name).is_some_and(|module| module == "math") {
                return Ok(Val::new(float_code(std::f64::consts::PI), Kind::Float));
            }
            // `from board import led`: a pin is its number.
            if name == "led" && self.imported.get(name).is_some_and(|module| module == "board") {
                return Ok(Val::new(LED.to_string(), Kind::Pin));
            }
            if self.functions.contains_key(name) {
                return self.function_value(name, span);
            }
            return Err(cannot(format!("`{name}` used this way"), span));
        };
        let val = match &variable.place {
            Place::Value(code) => Val::new(code.clone(), variable.kind.clone()),
            Place::Slot(slot) => {
                let ty = ir_type(&variable.kind, span)?;
                Val::new(self.load(ty, slot), variable.kind.clone())
            }
        };
        self.present(val, expr, variable.open)
    }

    /// An int as a float, for arithmetic that mixes the two.
    fn double(&mut self, val: &Val) -> String {
        match val.kind {
            // "signed int to floating point".
            Kind::Int => self.set(format!("sitofp i64 {} to double", val.code)),
            _ => val.code.clone(),
        }
    }

    /// `+`, `-` or `*` on two ints, with the check Zeno promises.
    ///
    /// A plain `add` would wrap round on overflow. This asks for the
    /// result together with the processor's overflow flag, and leaves for
    /// a block that reports the error if the flag is set. The runtime
    /// function does not come back, which is what `unreachable` says.
    fn checked(&mut self, op: &'static str, left: &str, right: &str, span: Span) -> String {
        let name = match op {
            "sadd" => "llvm.sadd.with.overflow.i64",
            "ssub" => "llvm.ssub.with.overflow.i64",
            _ => "llvm.smul.with.overflow.i64",
        };
        let both = self.call("{ i64, i1 }", name, format!("i64 {left}, i64 {right}"));
        let result = self.set(format!("extractvalue {{ i64, i1 }} {both}, 0"));
        let overflowed = self.set(format!("extractvalue {{ i64, i1 }} {both}, 1"));
        let n = self.number();
        let (overflow, fits) = (format!("overflow{n}"), format!("fits{n}"));
        self.jump_if(&overflowed, &overflow, &fits);
        self.place(&overflow);
        self.run("zeno_overflow", at(span));
        self.end("unreachable");
        self.place(&fits);
        result
    }

    /// A quantity turned into another unit: multiplied or divided by a
    /// number that is known now, while compiling.
    fn scaled(&mut self, code: &str, scale: units::Scale) -> String {
        if scale.times != 1.0 {
            self.set(format!("fmul double {code}, {}", float_code(scale.times)))
        } else if scale.over != 1.0 {
            self.set(format!("fdiv double {code}, {}", float_code(scale.over)))
        } else {
            code.to_string()
        }
    }

    /// Two quantities, both scaled to the unit they share: the finer of
    /// the two, so that `1 s + 500 ms` is `1500 ms`.
    fn in_common(&mut self, left: &Val, a: &str, right: &Val, b: &str, span: Span) -> Result<(String, String, String), Error> {
        match units::common(a, b) {
            Some((unit, l, r)) => Ok((unit.to_string(), self.scaled(&left.code, l), self.scaled(&right.code, r))),
            None => Err(cannot(format!("a quantity in {a} with a quantity in {b}"), span)),
        }
    }

    fn arithmetic(&mut self, op: BinaryOp, left: Val, right: Val, span: Span) -> Result<Val, Error> {
        use BinaryOp::*;
        use Kind::{Float, Int, Quantity, Str};
        let (l, r) = (left.code.clone(), right.code.clone());
        let numbers = matches!(left.kind, Int | Float) && matches!(right.kind, Int | Float);

        Ok(match (op, &left.kind, &right.kind) {
            (Add, Int, Int) => Val::new(self.checked("sadd", &l, &r, span), Int),
            (Sub, Int, Int) => Val::new(self.checked("ssub", &l, &r, span), Int),
            (Mul, Int, Int) => Val::new(self.checked("smul", &l, &r, span), Int),
            // These have more to them than one instruction (`//` rounds
            // down, `%` follows the right side's sign). `//` and `%` are
            // functions written in IR (see `WRITTEN`), `**` is in the
            // runtime.
            (FloorDiv | Mod | Pow, Int, Int) => {
                let name = match op {
                    FloorDiv => "zeno.floordiv",
                    Mod => "zeno.mod",
                    _ => "zeno_pow",
                };
                Val::new(self.call("i64", name, format!("i64 {l}, i64 {r}, {}", at(span))), Int)
            }

            // The bits of whole numbers: one instruction each.
            (BitAnd | BitOr | BitXor, Int, Int) => {
                let instruction = match op {
                    BitAnd => "and",
                    BitOr => "or",
                    _ => "xor",
                };
                Val::new(self.set(format!("{instruction} i64 {l}, {r}")), Int)
            }
            // The two shifts go through the runtime, which checks how far.
            (Shl | Shr, Int, Int) => {
                let name = if op == Shl { "zeno_shl" } else { "zeno_shr" };
                Val::new(self.call("i64", name, format!("i64 {l}, i64 {r}, {}", at(span))), Int)
            }

            (Add, Str, Str) => Val::new(self.join(&[l, r]), Str),
            // A new list with the items of both.
            (Add, Kind::List(a), Kind::List(b)) if a == b => {
                Val::new(self.call("ptr", "zeno_list_concat", format!("ptr {l}, ptr {r}")), left.kind.clone())
            }

            // Quantities. The checker has made sure the two measure the
            // same thing. If the units differ, both sides are scaled to
            // the finer one.
            (Add | Sub, Quantity(a), Quantity(b)) => {
                let (unit, l, r) = self.in_common(&left, a, &right, b, span)?;
                let instruction = if op == Add { "fadd" } else { "fsub" };
                Val::new(self.set(format!("{instruction} double {l}, {r}")), Quantity(unit))
            }
            (Mul, Quantity(unit), Int | Float) | (Mul, Int | Float, Quantity(unit)) => {
                let (l, r) = (self.double(&left), self.double(&right));
                Val::new(self.set(format!("fmul double {l}, {r}")), Quantity(unit.clone()))
            }
            // Ten seconds over two seconds is five, with no unit left.
            (Div, Quantity(a), Quantity(b)) => {
                let (_, l, r) = self.in_common(&left, a, &right, b, span)?;
                Val::new(self.call("double", "zeno.div", format!("double {l}, double {r}, {}", at(span))), Float)
            }
            (Div, Quantity(unit), Int | Float) => {
                let r = self.double(&right);
                let args = format!("double {l}, double {r}, {}", at(span));
                Val::new(self.call("double", "zeno.div", args), Quantity(unit.clone()))
            }

            // What is left is floats, or an int with a float. The float
            // instructions already do what Zeno wants, except for
            // dividing by zero.
            (Add | Sub | Mul, _, _) if numbers => {
                let (l, r) = (self.double(&left), self.double(&right));
                let instruction = match op {
                    Add => "fadd",
                    Sub => "fsub",
                    _ => "fmul",
                };
                Val::new(self.set(format!("{instruction} double {l}, {r}")), Float)
            }
            (Div | FloorDiv | Mod, _, _) if numbers => {
                let (l, r) = (self.double(&left), self.double(&right));
                let name = match op {
                    Div => "zeno.div",
                    FloorDiv => "zeno_floordiv_float",
                    _ => "zeno_mod_float",
                };
                Val::new(self.call("double", name, format!("double {l}, double {r}, {}", at(span))), Float)
            }
            (Pow, _, _) if numbers => {
                let (l, r) = (self.double(&left), self.double(&right));
                Val::new(self.call("double", "pow", format!("double {l}, double {r}")), Float)
            }

            _ => {
                return Err(cannot(
                    format!("`{}` between {} and {}", op.text(), described(&left.kind), described(&right.kind)),
                    span,
                ))
            }
        })
    }

    /// `a and b`, `a or b`. The right side must not run unless it is needed.
    ///
    /// ```text
    ///   br i1 %a, label %and1, label %end1     ; a is false: skip b
    /// and1:
    ///   ...work out b...
    ///   br label %end1
    /// end1:
    ///   %t = phi i1 [ false, %entry ], [ %b, %and1 ]
    /// ```
    ///
    /// `phi` is how a value that depends on the way a block was reached
    /// is written when names can be given a value only once: "false if we
    /// came from `entry`, `%b` if we came from `and1`".
    fn logic(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Result<Val, Error> {
        let is_and = op == BinaryOp::And;
        let first = self.condition(left)?;
        let from = self.code.block.clone();
        let n = self.number();
        let (more, end) = (format!("{}{n}", if is_and { "and" } else { "or" }), format!("end{n}"));
        match is_and {
            true => self.jump_if(&first, &more, &end),
            false => self.jump_if(&first, &end, &more),
        }
        self.place(&more);
        let second = self.condition(right)?;
        // Working out the right side may have ended in another block.
        let after = self.code.block.clone();
        self.place(&end);
        let settled = if is_and { "false" } else { "true" };
        let result = self.set(format!("phi i1 [ {settled}, %{from} ], [ {second}, %{after} ]"));
        Ok(Val::new(result, Kind::Bool))
    }

    /// `a < b`, and `a < b < c`: each piece is worked out once, and only
    /// if the comparisons before it held.
    fn chain(&mut self, first: &Expr, rest: &[(BinaryOp, Expr)]) -> Result<Val, Error> {
        let mut previous = self.expr(first)?;
        let n = if rest.len() > 1 { self.number() } else { 0 };
        let end = format!("end{n}");
        // The blocks that leave early because a comparison failed.
        let mut failed = Vec::new();
        let mut held = String::new();
        for (index, (op, expr)) in rest.iter().enumerate() {
            let next = self.expr(expr)?;
            held = self.compare(*op, &previous, &next, expr.span)?;
            if index + 1 < rest.len() {
                failed.push(self.code.block.clone());
                let more = format!("then{n}.{}", index + 1);
                self.jump_if(&held, &more, &end);
                self.place(&more);
            }
            previous = next;
        }
        // One comparison needs no blocks at all.
        if failed.is_empty() {
            return Ok(Val::new(held, Kind::Bool));
        }
        let last = self.code.block.clone();
        self.place(&end);
        let mut ways: Vec<String> = failed.iter().map(|block| format!("[ false, %{block} ]")).collect();
        ways.push(format!("[ {held}, %{last} ]"));
        Ok(Val::new(self.set(format!("phi i1 {}", ways.join(", "))), Kind::Bool))
    }

    /// `==` and the rest, between two values.
    fn compare(&mut self, op: BinaryOp, left: &Val, right: &Val, span: Span) -> Result<String, Error> {
        let plain = |kind: &Kind| matches!(kind, Kind::Int | Kind::Float | Kind::Bool | Kind::Str | Kind::Quantity(_));
        if plain(&left.kind) && plain(&right.kind) || !matches!(op, BinaryOp::Eq | BinaryOp::NotEq) {
            return self.ordered(op, left, right, span);
        }
        // Everything else is equal when its contents are. `x == none` and
        // `x == 5`, where `x` may be `none`, first bring both sides to
        // the same kind.
        let Some(kind) = joined(&left.kind, &right.kind) else {
            return Err(cannot(
                format!("`{}` between {} and {}", op.text(), described(&left.kind), described(&right.kind)),
                span,
            ));
        };
        let left = self.coerce(left.clone(), &kind, span)?;
        let right = self.coerce(right.clone(), &kind, span)?;
        let same = self.equal(&kind, &left.code, &right.code, span)?;
        Ok(if op == BinaryOp::Eq { same } else { self.set(format!("xor i1 {same}, true")) })
    }

    /// One comparison between two numbers, bools, pieces of text or
    /// quantities.
    fn ordered(&mut self, op: BinaryOp, left: &Val, right: &Val, span: Span) -> Result<String, Error> {
        use BinaryOp::*;
        use Kind::{Bool, Float, Int, Quantity, Str};
        // `icmp` compares whole numbers, and wants to be told that they
        // are signed (`slt` is "signed less than"). `fcmp` compares
        // floats. Its `o` is for "ordered": false if either side is not
        // a number at all. `!=` is the one that is true then.
        let (whole, float) = match op {
            Eq => ("eq", "oeq"),
            NotEq => ("ne", "une"),
            Lt => ("slt", "olt"),
            LtEq => ("sle", "ole"),
            Gt => ("sgt", "ogt"),
            GtEq => ("sge", "oge"),
            _ => return Err(cannot(format!("`{}`", op.text()), span)),
        };
        Ok(match (&left.kind, &right.kind) {
            (Int, Int) => self.set(format!("icmp {whole} i64 {}, {}", left.code, right.code)),
            (Bool, Bool) if matches!(op, Eq | NotEq) => {
                self.set(format!("icmp {whole} i1 {}, {}", left.code, right.code))
            }
            (Int | Float, Int | Float) => {
                let (l, r) = (self.double(left), self.double(right));
                self.set(format!("fcmp {float} double {l}, {r}"))
            }
            (Quantity(a), Quantity(b)) => {
                let (_, l, r) = self.in_common(left, a, right, b, span)?;
                self.set(format!("fcmp {float} double {l}, {r}"))
            }
            // Two pieces of text are compared by what they say, which
            // `strcmp` answers with a number below, at or above zero.
            (Str, Str) => {
                let order = self.call("i32", "strcmp", format!("ptr {}, ptr {}", left.code, right.code));
                self.set(format!("icmp {whole} i32 {order}, 0"))
            }
            _ => {
                return Err(cannot(
                    format!("`{}` between {} and {}", op.text(), described(&left.kind), described(&right.kind)),
                    span,
                ))
            }
        })
    }

    /// `x in items`, `x not in items`, and the same for a piece of a str.
    fn contains(&mut self, op: BinaryOp, left: Val, right: Val, span: Span) -> Result<Val, Error> {
        let found = match (&left.kind, &right.kind) {
            // `none in items` and `2 in items`, where the items may be
            // `none`, first become what the list holds.
            (item, Kind::List(items)) if joined(item, items).as_ref() == Some(&**items) => {
                let same = self.eq_adapter(items, span)?;
                let left = self.coerce(left.clone(), items, span)?;
                let slot = self.to_slot(&left, span)?;
                self.call("i1", "zeno_list_contains", format!("ptr {}, i64 {slot}, ptr {same}", right.code))
            }
            // `strstr` gives the address where the piece was found, or
            // null.
            (Kind::Str, Kind::Str) => {
                let found = self.call("ptr", "strstr", format!("ptr {}, ptr {}", right.code, left.code));
                self.set(format!("icmp ne ptr {found}, null"))
            }
            _ => {
                return Err(cannot(
                    format!("`in` between {} and {}", described(&left.kind), described(&right.kind)),
                    span,
                ))
            }
        };
        let code = if op == BinaryOp::NotIn { self.set(format!("xor i1 {found}, true")) } else { found };
        Ok(Val::new(code, Kind::Bool))
    }

    /// `object.name`: a field of an object, or a quantity in another unit.
    fn field(&mut self, object: &Expr, name: &str, span: Span) -> Result<Val, Error> {
        if let ExprKind::Name(owner) = &object.kind {
            if self.lookup(owner).is_none() {
                if owner == "math" && name == "pi" {
                    return Ok(Val::new(float_code(std::f64::consts::PI), Kind::Float));
                }
                if owner == "board" && name == "led" {
                    return Ok(Val::new(LED.to_string(), Kind::Pin));
                }
                // `Shape.Idle`
                if self.enums.contains_key(owner) && self.cases.get(name).is_some_and(|(of, _)| of == owner) {
                    let index = self.cases[name].1;
                    if self.enums[owner].cases[index].1.is_empty() {
                        return Ok(Val::new(bare_case(owner, name), Kind::Enum(owner.clone())));
                    }
                }
            }
        }
        let val = self.expr(object)?;
        match &val.kind {
            // `(3 s).ms`: the same quantity in another unit.
            Kind::Quantity(unit) if units::is_unit(name) => match units::convert(unit, name) {
                Some(scale) => Ok(Val::new(self.scaled(&val.code, scale), Kind::Quantity(name.to_string()))),
                None => Err(cannot(format!("`.{name}` on this"), span)),
            },
            // Where the field is, then what is there.
            Kind::Class(class) => match self.field_of(class, name) {
                Some((ty, index, kind)) => {
                    let at = self.field_at(&ty, &val.code, index);
                    Ok(Val::new(self.set(format!("load {}, ptr {at}", ir_type(&kind, span)?)), kind))
                }
                None => Err(cannot(format!("a method used as a value: `{name}`"), span)),
            },
            Kind::Error if matches!(name, "message" | "kind") => {
                let at = self.field_at(ERROR, &val.code, if name == "kind" { 0 } else { 1 });
                Ok(Val::new(self.set(format!("load ptr, ptr {at}")), Kind::Str))
            }
            // `led.number`: a pin is its number already.
            Kind::Pin if name == "number" => Ok(Val { kind: Kind::Int, ..val }),
            _ => Err(cannot(format!("`.{name}` on {}", described(&val.kind)), span)),
        }
    }

    // ---------------------------------------------------------------- Lists

    /// A value on its way into a list. Every item of a list is kept in a
    /// slot of 64 bits, whatever it is, so that one runtime serves lists
    /// of everything. A whole number goes in as it is; anything else is
    /// carried over bit for bit: `bitcast` reads the 64 bits of a float
    /// as a whole number, `ptrtoint` does the same for an address.
    fn to_slot(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        Ok(match &val.kind {
            Kind::Int | Kind::Pin => val.code.clone(),
            Kind::Float | Kind::Quantity(_) => self.set(format!("bitcast double {} to i64", val.code)),
            // "Zero extend": the one bit, with 63 zeros in front.
            Kind::Bool => self.set(format!("zext i1 {} to i64", val.code)),
            kind if is_pointer(kind) => self.set(format!("ptrtoint ptr {} to i64", val.code)),
            Kind::Optional(inner) if is_pointer(inner) => self.set(format!("ptrtoint ptr {} to i64", val.code)),
            // A number and its flag do not fit in a slot; see `zeno.box.int`.
            Kind::Optional(inner) => {
                let name = match scalar(inner, span)? {
                    "int" => "zeno.box.int",
                    "float" => "zeno.box.float",
                    _ => "zeno.box.bool",
                };
                let boxed = self.call("ptr", name, format!("{} {}", ir_type(&val.kind, span)?, val.code));
                self.set(format!("ptrtoint ptr {boxed} to i64"))
            }
            _ => return Err(cannot("a list of these", span)),
        })
    }

    /// And out of a list again.
    fn from_slot(&mut self, kind: &Kind, slot: &str, span: Span) -> Result<Val, Error> {
        let code = match kind {
            Kind::Int | Kind::Pin => slot.to_string(),
            Kind::Float | Kind::Quantity(_) => self.set(format!("bitcast i64 {slot} to double")),
            // "Truncate": keep the lowest bit.
            Kind::Bool => self.set(format!("trunc i64 {slot} to i1")),
            kind if is_pointer(kind) => self.set(format!("inttoptr i64 {slot} to ptr")),
            Kind::Optional(inner) if is_pointer(inner) => self.set(format!("inttoptr i64 {slot} to ptr")),
            Kind::Optional(inner) => {
                let name = match scalar(inner, span)? {
                    "int" => "zeno.unbox.int",
                    "float" => "zeno.unbox.float",
                    _ => "zeno.unbox.bool",
                };
                let boxed = self.set(format!("inttoptr i64 {slot} to ptr"));
                self.call(ir_type(kind, span)?, name, format!("ptr {boxed}"))
            }
            _ => return Err(cannot("a list of these", span)),
        };
        Ok(Val::new(code, kind.clone()))
    }

    /// `[1, 2, 3]`
    fn list(&mut self, items: &[Expr], span: Span) -> Result<Val, Error> {
        let mut vals = Vec::new();
        for item in items {
            vals.push(self.expr(item)?);
        }
        let Some(first) = vals.first() else {
            // `[]` alone does not say what it will hold. The place it
            // arrives at does (`coerce`), and a `let` with no type
            // beside it says it cannot tell.
            let kind = Kind::List(Box::new(Kind::Other("nothing yet".to_string())));
            return Ok(Val::new(self.call("ptr", "zeno_list_new", "i64 0"), kind));
        };
        // `[1, none]` is a list of `int or none`: find the one kind that
        // every item can be stored as.
        let mut item = Some(first.kind.clone());
        for val in &vals {
            item = item.and_then(|item| joined(&item, &val.kind));
        }
        let Some(item) = item else {
            return Err(cannot("a list whose items are not all one type", span));
        };
        // `zeno_list_of` takes a count and then that many slots.
        let mut slots = format!("i32 {}", vals.len());
        for val in vals {
            let val = self.coerce(val, &item, span)?;
            slots += &format!(", i64 {}", self.to_slot(&val, span)?);
        }
        self.uses("zeno_list_of");
        let list = self.set(format!("call ptr (i32, ...) @zeno_list_of({slots})"));
        Ok(Val::new(list, Kind::List(Box::new(item))))
    }

    /// `[n * n for n in items if n > 0]` is a loop that fills a list.
    fn comprehension(
        &mut self,
        element: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
        span: Span,
    ) -> Result<Val, Error> {
        let list = self.call("ptr", "zeno_list_new", "i64 0");
        let count = self.open_loop(variable, iterable)?;
        if let Some(condition) = condition {
            // An item that fails the test is skipped: on to the next trip.
            let wanted = self.condition(condition)?;
            let keep = format!("keep{}", self.number());
            self.jump_if(&wanted, &keep, &count.next);
            self.place(&keep);
        }
        let item = self.expr(element)?;
        let slot = self.to_slot(&item, span)?;
        self.run("zeno_list_append", format!("ptr {list}, i64 {slot}"));
        self.close_loop(&count);
        Ok(Val::new(list, Kind::List(Box::new(item.kind))))
    }

    // ------------------------------------------------ Values that may be none

    /// Make `val` fit a place that holds values of kind `to`.
    ///
    /// Nearly always there is nothing to do. The work is at the border of
    /// `T or none`: a plain number going into an `int or none` has a flag
    /// put beside it, and `none` becomes whatever "nothing" is for that
    /// kind.
    fn coerce(&mut self, val: Val, to: &Kind, span: Span) -> Result<Val, Error> {
        if val.kind == *to {
            return Ok(val);
        }
        // An empty list becomes a list of whatever this place holds.
        let mut val = val;
        if is_empty_list(&val.kind) {
            match to {
                Kind::List(_) => return Ok(Val { kind: to.clone(), ..val }),
                Kind::Optional(inner) if matches!(**inner, Kind::List(_)) => val.kind = (**inner).clone(),
                _ => {}
            }
        }
        let code = match (&val.kind, to) {
            (Kind::None, Kind::Optional(inner)) => nothing(inner, span)?.to_string(),
            // An address already has a way to say "nothing".
            (from, Kind::Optional(inner)) if from == &**inner && is_pointer(from) => val.code.clone(),
            // `insertvalue` makes a copy of a pair with one part
            // replaced. The pair it starts from has the flag set and
            // nothing yet (`undef`) in the place of the number.
            (from, Kind::Optional(inner)) if from == &**inner => {
                let (pair, inside) = (ir_type(to, span)?, ir_type(from, span)?);
                self.set(format!("insertvalue {pair} {{ i1 true, {inside} undef }}, {inside} {}, 1", val.code))
            }
            (Kind::Function(_), Kind::Function(_)) => match joined(&val.kind, to) {
                Some(_) => val.code.clone(),
                None => {
                    return Err(cannot("a function used where a function of a different shape is wanted", span))
                }
            },
            // Anything else the checker has already agreed to as it is.
            _ => return Ok(val),
        };
        Ok(Val { code, kind: to.clone() })
    }

    /// `coerce`, and then make sure the value can be held where it is
    /// going. The checker has agreed to the types. This is the last look
    /// before IR is written that LLVM would refuse.
    fn fit(&mut self, val: Val, to: &Kind, span: Span) -> Result<Val, Error> {
        let val = self.coerce(val, to, span)?;
        if ir_type(&val.kind, span)? != ir_type(to, span)? {
            return Err(cannot(format!("{} where {} is wanted", described(&val.kind), described(to)), span));
        }
        Ok(val)
    }

    /// Is a value that may be `none` there? One bit.
    fn is_there(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        match &val.kind {
            Kind::Optional(inner) if is_pointer(inner) => Ok(self.set(format!("icmp ne ptr {}, null", val.code))),
            // `extractvalue` reads one part of a pair: part 0 is the flag.
            kind => Ok(self.set(format!("extractvalue {} {}, 0", ir_type(kind, span)?, val.code))),
        }
    }

    /// The value inside one that is known to be there.
    fn inside(&mut self, val: &Val, span: Span) -> Result<Val, Error> {
        let Kind::Optional(inner) = &val.kind else {
            return Ok(val.clone());
        };
        let code = match is_pointer(inner) {
            true => val.code.clone(),
            false => self.set(format!("extractvalue {} {}, 1", ir_type(&val.kind, span)?, val.code)),
        };
        Ok(Val::new(code, (**inner).clone()))
    }

    /// A variable has just been read. If the checker has ruled `none` out
    /// at this spot (it is inside `if x is not none { ... }`, say), give
    /// the value inside instead of the `T or none` around it.
    ///
    /// `changeable` is for a variable that other functions can reach and
    /// change. One of them may have set it back to `none` since the
    /// check, so it is looked at once more before it is trusted.
    fn present(&mut self, val: Val, expr: &Expr, changeable: bool) -> Result<Val, Error> {
        if !matches!(val.kind, Kind::Optional(_)) || matches!(self.types.narrowed(expr), None | Some(Kind::None)) {
            return Ok(val);
        }
        if changeable {
            let there = self.is_there(&val, expr.span)?;
            let n = self.number();
            let (here, gone) = (format!("here{n}"), format!("gone{n}"));
            self.jump_if(&there, &here, &gone);
            self.place(&gone);
            self.run("zeno_gone", at(expr.span));
            self.end("unreachable");
            self.place(&here);
        }
        self.inside(&val, expr.span)
    }

    /// `x is none`, `x is not none`
    fn is_none(&mut self, op: BinaryOp, left: &Expr) -> Result<Val, Error> {
        let val = self.expr(left)?;
        let is = op == BinaryOp::Is;
        let code = match &val.kind {
            Kind::Optional(_) => {
                let there = self.is_there(&val, left.span)?;
                if is {
                    self.set(format!("xor i1 {there}, true"))
                } else {
                    there
                }
            }
            // Anything else is either always `none` or never.
            other => ((*other == Kind::None) == is).to_string(),
        };
        Ok(Val::new(code, Kind::Bool))
    }

    // ---------------------------------------------------------------- Calls

    fn call_expr(&mut self, callee: &Expr, args: &[Arg], span: Span) -> Result<Val, Error> {
        match &callee.kind {
            ExprKind::Name(name) if self.lookup(name).is_none() => {
                if let Some(entry) = self.functions.get(name).cloned() {
                    let slots = entry
                        .decl
                        .params
                        .iter()
                        .zip(&entry.kind.params)
                        .map(|(param, kind)| (param.name.clone(), kind.clone(), param.default.clone()))
                        .collect();
                    let list = self.bind(&slots, args, span)?;
                    return self.call_entry(&entry, list, span);
                }
                if self.classes.contains_key(name) {
                    return self.construct(name, args, span);
                }
                if let Some((owner, index)) = self.cases.get(name).cloned() {
                    return self.construct_case(&owner, index, args, span);
                }
                if self.imported.get(name).is_some_and(|module| module == "math") {
                    return self.math(name, args, span);
                }
                if let Some(module) = self.imported.get(name).cloned() {
                    return self.device(&module, name, args, span);
                }
                self.builtin(name, args, span)
            }
            ExprKind::Field { object, name } => {
                if let ExprKind::Name(owner) = &object.kind {
                    if self.lookup(owner).is_none() {
                        // `math.sqrt(x)`
                        if owner == "math" {
                            return self.math(name, args, span);
                        }
                        // `time.sleep(1 s)`, `board.pin(7)`
                        if owner == "time" || owner == "board" {
                            return self.device(owner, name, args, span);
                        }
                        // `Shape.Circle(1.0)`
                        let case =
                            self.enums.get(owner).and_then(|entry| entry.cases.iter().position(|(case, _)| case == name));
                        if let Some(index) = case {
                            return self.construct_case(owner, index, args, span);
                        }
                    }
                }
                self.method(object, name, args, span)
            }
            // Anything else that is called must be a function value.
            _ => {
                let function = self.expr(callee)?;
                self.call_value(function, args, span)
            }
        }
    }

    /// The arguments of a call, each put in its place and written with
    /// its type, ready to go between the brackets. `slots` are the names,
    /// kinds and default values of what is being called.
    ///
    /// The arguments are worked out left to right as they are written,
    /// even when names put them elsewhere.
    fn bind(&mut self, slots: &Slots, args: &[Arg], span: Span) -> Result<Vec<String>, Error> {
        let mut given: Vec<Option<Val>> = vec![None; slots.len()];
        for (index, arg) in args.iter().enumerate() {
            let place = match &arg.name {
                None => index,
                Some(name) => match slots.iter().position(|(slot, _, _)| slot == name) {
                    Some(place) => place,
                    None => return Err(cannot(format!("the argument `{name}`"), span)),
                },
            };
            let val = self.expr(&arg.value)?;
            match given.get_mut(place) {
                Some(slot) => *slot = Some(val),
                None => return Err(cannot("this many arguments", span)),
            }
        }
        let mut list = Vec::new();
        for (val, (_, kind, default)) in given.into_iter().zip(slots) {
            // An argument that was left out takes the default.
            let val = match (val, default) {
                (Some(val), _) => val,
                (None, Some(default)) if emit_c::is_literal(default) => self.expr(default)?,
                (None, _) => return Err(cannot("a default value that is not a plain literal", span)),
            };
            let val = self.fit(val, kind, span)?;
            list.push(format!("{} {}", ir_type(kind, span)?, val.code));
        }
        Ok(list)
    }

    /// Write the call to one of the program's functions or methods.
    fn call_entry(&mut self, entry: &Entry, list: Vec<String>, span: Span) -> Result<Val, Error> {
        let returns = return_type(&entry.kind.returns, span)?;
        let call = format!("call {returns} {}({})", entry.name, list.join(", "));
        Ok(match &entry.kind.returns {
            // Nothing comes back, so there is nothing to name.
            Kind::None => {
                self.ins(call);
                Val::nothing()
            }
            kind => Val::new(self.set(call), kind.clone()),
        })
    }

    /// `sqrt`, `floor` and the rest of `import math`.
    /// A function of the `time` or the `board` module. Each is one call
    /// into the runtime, which passes it on to the board.
    fn device(&mut self, module: &str, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let mut vals = Vec::new();
        for arg in args {
            if arg.name.is_some() {
                return Err(cannot(format!("`{module}.{name}` called this way"), span));
            }
            vals.push(self.expr(&arg.value)?);
        }
        match (module, name, vals.as_slice()) {
            // The board counts in seconds, whatever unit the program wrote.
            ("time", "sleep", [time]) => {
                let scale = match &time.kind {
                    Kind::Quantity(unit) => units::convert(unit, "s"),
                    _ => None,
                };
                let Some(scale) = scale else {
                    return Err(cannot("`sleep` of something that is not a time", span));
                };
                let seconds = self.scaled(&time.code, scale);
                self.run("zeno_sleep", format!("double {seconds}"));
                Ok(Val::nothing())
            }
            ("time", "now", []) => Ok(Val::new(self.call("double", "zeno_now", ""), Kind::Quantity("ms".to_string()))),
            ("board", "pin", [number]) if number.kind == Kind::Int => {
                Ok(Val::new(self.call("i64", "zeno_pin", format!("i64 {}, {}", number.code, at(span))), Kind::Pin))
            }
            _ => Err(cannot(format!("`{module}.{name}` called this way"), span)),
        }
    }

    fn math(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let [arg] = args else {
            return Err(cannot(format!("`{name}` called this way"), span));
        };
        let val = self.expr(&arg.value)?;
        if !matches!(val.kind, Kind::Int | Kind::Float) || arg.name.is_some() {
            return Err(cannot(format!("`{name}` called this way"), span));
        }
        let x = self.double(&val);
        Ok(match name {
            "sqrt" => Val::new(self.call("double", "zeno_sqrt", format!("double {x}, {}", at(span))), Kind::Float),
            // The C library rounds, and the runtime turns the float that
            // comes back into an int.
            "floor" | "ceil" => {
                let rounded = self.call("double", if name == "floor" { "floor" } else { "ceil" }, format!("double {x}"));
                Val::new(self.call("i64", "zeno_whole", format!("double {rounded}")), Kind::Int)
            }
            "sin" => Val::new(self.call("double", "sin", format!("double {x}")), Kind::Float),
            "cos" => Val::new(self.call("double", "cos", format!("double {x}")), Kind::Float),
            _ => return Err(cannot(format!("`math.{name}`"), span)),
        })
    }

    /// A call to one of the functions Zeno comes with.
    fn builtin(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        if args.iter().any(|arg| arg.name.is_some()) {
            return Err(cannot(format!("`{name}` with a named argument"), span));
        }
        let mut vals = Vec::new();
        for arg in args {
            vals.push(self.expr(&arg.value)?);
        }
        let wrong = || cannot(format!("`{name}` used this way"), span);
        Ok(match (name, vals.as_slice()) {
            ("print", _) => {
                // `zeno_print` takes a count and then that many pieces of
                // text. `(i32, ...)` says the call has a varying number
                // of arguments, which IR must be told at each call.
                let mut list = format!("i32 {}", vals.len());
                for val in &vals {
                    list += &format!(", ptr {}", self.text(val, span)?);
                }
                self.uses("zeno_print");
                self.ins(format!("call void (i32, ...) @zeno_print({list})"));
                Val::nothing()
            }
            // `ValueError("bad")` makes an error value: its kind, its
            // message, and no object.
            (kind, _) if emit_c::ERROR_KINDS.contains(&kind) => {
                let message = match vals.as_slice() {
                    [] => self.string(""),
                    [message] => self.text(message, span)?,
                    _ => return Err(cannot(format!("`{name}` with more than one message"), span)),
                };
                let kind = self.string(kind);
                let error = self.call("ptr", "zeno_error_new", format!("ptr {kind}, ptr {message}, ptr null"));
                Val::new(error, Kind::Error)
            }
            ("str", [val]) => Val::new(self.text(val, span)?, Kind::Str),
            ("len", [val]) if val.kind == Kind::Str => {
                Val::new(self.call("i64", "zeno_len", format!("ptr {}", val.code)), Kind::Int)
            }
            // A list starts with how many items it has.
            ("len", [val]) if matches!(val.kind, Kind::List(_)) => {
                Val::new(self.set(format!("load i64, ptr {}", val.code)), Kind::Int)
            }
            ("abs", [val]) => match val.kind {
                Kind::Int => Val::new(self.call("i64", "zeno_abs", format!("i64 {}, {}", val.code, at(span))), Kind::Int),
                Kind::Float | Kind::Quantity(_) => {
                    Val::new(self.call("double", "fabs", format!("double {}", val.code)), val.kind.clone())
                }
                _ => return Err(wrong()),
            },
            ("int", [val]) => match val.kind {
                Kind::Int => val.clone(),
                Kind::Float => {
                    Val::new(self.call("i64", "zeno_int", format!("double {}, {}", val.code, at(span))), Kind::Int)
                }
                Kind::Str => {
                    Val::new(self.call("i64", "zeno_parse_int", format!("ptr {}, {}", val.code, at(span))), Kind::Int)
                }
                _ => return Err(wrong()),
            },
            ("float", [val]) => match val.kind {
                Kind::Int | Kind::Float => Val::new(self.double(val), Kind::Float),
                Kind::Str => Val::new(
                    self.call("double", "zeno_parse_float", format!("ptr {}, {}", val.code, at(span))),
                    Kind::Float,
                ),
                _ => return Err(wrong()),
            },
            // `sum`, `min` and `max` of one list of numbers.
            ("sum" | "min" | "max", [Val { kind: Kind::List(item), code }]) => {
                let (returns, function, args) = match (name, &**item) {
                    ("sum", Kind::Int) => ("i64", "zeno_list_sum_int", format!("ptr {code}, {}", at(span))),
                    ("sum", Kind::Float) => ("double", "zeno_list_sum_float", format!("ptr {code}")),
                    (_, Kind::Int | Kind::Float) => {
                        // The runtime is told which of the two is wanted,
                        // and the name, for its message about an empty list.
                        let said = self.string(name);
                        let args = format!("ptr {code}, i1 {}, ptr {said}, {}", name == "min", at(span));
                        match **item {
                            Kind::Int => ("i64", "zeno_list_best_int", args),
                            _ => ("double", "zeno_list_best_float", args),
                        }
                    }
                    _ => return Err(cannot(format!("`{name}` of a list of anything but ints or floats"), span)),
                };
                Val::new(self.call(returns, function, args), (**item).clone())
            }
            ("min" | "max", [first, rest @ ..])
                if !rest.is_empty()
                    && matches!(first.kind, Kind::Int | Kind::Float)
                    && rest.iter().all(|val| val.kind == first.kind) =>
            {
                // `select` picks one of two values by a bool, with no
                // jump: `b < a ? b : a`. The earlier value is kept when
                // the two are equal.
                let (compare, ty) = match (&first.kind, name) {
                    (Kind::Int, "min") => ("icmp slt", "i64"),
                    (Kind::Int, _) => ("icmp sgt", "i64"),
                    (_, "min") => ("fcmp olt", "double"),
                    _ => ("fcmp ogt", "double"),
                };
                let mut best = first.code.clone();
                for val in rest {
                    let better = self.set(format!("{compare} {ty} {}, {best}", val.code));
                    best = self.set(format!("select i1 {better}, {ty} {}, {ty} {best}", val.code));
                }
                Val::new(best, first.kind.clone())
            }
            _ => return Err(wrong()),
        })
    }

    /// `object.name(args)`: a method of a class, of a str, or of a list.
    fn method(&mut self, object: &Expr, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let receiver = self.expr(object)?;
        match receiver.kind.clone() {
            Kind::Class(class) => {
                let Some(entry) = self.classes[&class].methods.get(name).cloned() else {
                    // Not a method. It may be a field that holds a function.
                    let Some((ty, index, kind @ Kind::Function(_))) = self.field_of(&class, name) else {
                        return Err(cannot(format!("the method `{name}`"), span));
                    };
                    let at = self.field_at(&ty, &receiver.code, index);
                    let function = Val::new(self.set(format!("load ptr, ptr {at}")), kind);
                    return self.call_value(function, args, span);
                };
                // The first parameter is `self`, which the receiver fills.
                let slots = entry
                    .decl
                    .params
                    .iter()
                    .zip(&entry.kind.params)
                    .skip(1)
                    .map(|(param, kind)| (param.name.clone(), kind.clone(), param.default.clone()))
                    .collect();
                let mut list = vec![format!("ptr {}", receiver.code)];
                list.extend(self.bind(&slots, args, span)?);
                self.call_entry(&entry, list, span)
            }
            // A pin of the board: `led.on()`. Each is one call into the
            // runtime, with the pin's number.
            Kind::Pin => {
                if !args.is_empty() {
                    return Err(cannot(format!("`{name}` on a pin with arguments"), span));
                }
                let pin = format!("i64 {}", receiver.code);
                match name {
                    "on" => self.run("zeno_pin_on", pin),
                    "off" => self.run("zeno_pin_off", pin),
                    "toggle" => self.run("zeno_pin_toggle", pin),
                    "is_on" => return Ok(Val::new(self.call("i1", "zeno_pin_is_on", pin), Kind::Bool)),
                    _ => return Err(cannot(format!("the method `{name}`"), span)),
                }
                Ok(Val::nothing())
            }
            Kind::Str => {
                let wrong = || cannot(format!("the str method `{name}` called this way"), span);
                let mut list = format!("ptr {}", receiver.code);
                for arg in args {
                    let val = self.expr(&arg.value)?;
                    if val.kind != Kind::Str || arg.name.is_some() {
                        return Err(wrong());
                    }
                    list += &format!(", ptr {}", val.code);
                }
                let words = Kind::List(Box::new(Kind::Str));
                let (function, returns, kind) = match (name, args.len()) {
                    ("upper", 0) => ("zeno_upper", "ptr", Kind::Str),
                    ("lower", 0) => ("zeno_lower", "ptr", Kind::Str),
                    ("strip", 0) => ("zeno_strip", "ptr", Kind::Str),
                    ("replace", 2) => ("zeno_replace", "ptr", Kind::Str),
                    ("starts_with", 1) => ("zeno_starts_with", "i1", Kind::Bool),
                    ("ends_with", 1) => ("zeno_ends_with", "i1", Kind::Bool),
                    ("split", 0) => ("zeno_split_blanks", "ptr", words),
                    ("split", 1) => {
                        list += &format!(", {}", at(span));
                        ("zeno_split", "ptr", words)
                    }
                    _ => return Err(wrong()),
                };
                Ok(Val::new(self.call(returns, function, list), kind))
            }
            Kind::List(item) => match (name, args) {
                ("map" | "filter" | "reduce", _) => self.over_list(name, &receiver, &item, args, span),
                ("append", [arg]) if arg.name.is_none() => {
                    let value = self.expr(&arg.value)?;
                    let value = self.fit(value, &item, span)?;
                    let slot = self.to_slot(&value, span)?;
                    self.run("zeno_list_append", format!("ptr {}, i64 {slot}", receiver.code));
                    Ok(Val::nothing())
                }
                ("pop", []) => {
                    let slot = self.call("i64", "zeno_list_pop", format!("ptr {}, {}", receiver.code, at(span)));
                    self.from_slot(&item, &slot, span)
                }
                _ => Err(cannot(format!("the list method `{name}` called this way"), span)),
            },
            kind => Err(cannot(format!("the method `{name}` of {}", described(&kind)), span)),
        }
    }

    // ---------------------------------------------------- Objects and enums

    /// `Point(3.0, 4.0)`: a new object.
    fn construct(&mut self, class: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let slots = self.classes[class].fields.clone();
        let list = self.bind(&slots, args, span)?;
        let make = self.new_class(class, span)?;
        let object = self.set(format!("call ptr {make}({})", list.join(", ")));
        Ok(Val::new(object, Kind::Class(class.to_string())))
    }

    /// The function that makes an object of a class: ask the collector
    /// for a block the size of the struct, and fill in each field.
    ///
    /// ```text
    /// define internal ptr @zeno.new.Point(double %f0, double %f1) {
    /// entry:
    ///   %t1 = call ptr @zeno_alloc(i64 ptrtoint (ptr getelementptr (%z_Point, ptr null, i32 1) to i64))
    ///   %t2 = getelementptr %z_Point, ptr %t1, i32 0, i32 0
    ///   store double %f0, ptr %t2
    ///   ...
    ///   ret ptr %t1
    /// }
    /// ```
    ///
    /// Like the other functions the compiler writes for itself, it is
    /// written the first time it is needed, and `written` remembers that
    /// it has been.
    fn new_class(&mut self, class: &str, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.new.", class);
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let (ty, fields) = (self.classes[class].ty.clone(), self.classes[class].fields.clone());
        let mut params = Vec::new();
        for (index, (_, kind, _)) in fields.iter().enumerate() {
            params.push(format!("{} %f{index}", ir_type(kind, span)?));
        }
        self.side(format!("define internal ptr {name}({})", params.join(", ")), |emitter| {
            let object = emitter.call("ptr", "zeno_alloc", format!("i64 {}", size_of(&ty)));
            for (index, (_, kind, _)) in fields.iter().enumerate() {
                let at = emitter.field_at(&ty, &object, index);
                emitter.ins(format!("store {} %f{index}, ptr {at}", ir_type(kind, span)?));
            }
            emitter.end(format!("ret ptr {object}"));
            Ok(())
        })?;
        Ok(name)
    }

    /// The function that shows an object as text: `Point(x=3.0, y=4.0)`.
    fn show_class(&mut self, class: &str, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.show.", class);
        // Noted as written before it is: an object may hold another of
        // its own class, and showing that one calls this same function.
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let (ty, fields) = (self.classes[class].ty.clone(), self.classes[class].fields.clone());
        self.side(format!("define internal ptr {name}(ptr %self)"), |emitter| {
            let mut pieces = Vec::new();
            for (index, (field, kind, _)) in fields.iter().enumerate() {
                let label = if index == 0 { format!("{class}({field}=") } else { format!(", {field}=") };
                pieces.push(emitter.string(&label));
                let at = emitter.field_at(&ty, "%self", index);
                let value = emitter.set(format!("load {}, ptr {at}", ir_type(kind, span)?));
                pieces.push(emitter.repr(&Val::new(value, kind.clone()), span)?);
            }
            if fields.is_empty() {
                pieces.push(emitter.string(&format!("{class}(")));
            }
            pieces.push(emitter.string(")"));
            let shown = emitter.join(&pieces);
            emitter.end(format!("ret ptr {shown}"));
            Ok(())
        })?;
        Ok(name)
    }

    /// The function that says whether two objects of a class are equal:
    /// they are when every field is. It stops at the first field that
    /// differs.
    fn same_class(&mut self, class: &str, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.same.", class);
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let (ty, fields) = (self.classes[class].ty.clone(), self.classes[class].fields.clone());
        self.side(format!("define internal i1 {name}(ptr %a, ptr %b)"), |emitter| {
            for (index, (_, kind, _)) in fields.iter().enumerate() {
                emitter.same_field(&ty, index, kind, &format!("more{index}"), span)?;
            }
            emitter.end("ret i1 true");
            emitter.place("differ");
            emitter.end("ret i1 false");
            Ok(())
        })?;
        Ok(name)
    }

    /// Inside a function that compares `%a` and `%b`: read field `index`
    /// of both, and leave for `differ` unless the two are equal.
    fn same_field(&mut self, ty: &str, index: usize, kind: &Kind, more: &str, span: Span) -> Result<(), Error> {
        let field_type = ir_type(kind, span)?;
        let (a_at, b_at) = (self.field_at(ty, "%a", index), self.field_at(ty, "%b", index));
        let a = self.set(format!("load {field_type}, ptr {a_at}"));
        let b = self.set(format!("load {field_type}, ptr {b_at}"));
        let same = self.equal(kind, &a, &b, span)?;
        self.jump_if(&same, more, "differ");
        self.place(more);
        Ok(())
    }

    /// `Circle(1.0)`: a new value of an enum.
    fn construct_case(&mut self, owner: &str, index: usize, args: &[Arg], span: Span) -> Result<Val, Error> {
        let (case, fields) = self.enums[owner].cases[index].clone();
        if fields.is_empty() {
            return Err(cannot(format!("`{case}` called like a function: it carries no values"), span));
        }
        let slots: Slots = fields.iter().map(|(name, kind)| (name.clone(), kind.clone(), None)).collect();
        let list = self.bind(&slots, args, span)?;

        let make = named('@', "zeno.new.", &format!("{owner}.{case}"));
        if self.written.insert(make.clone()) {
            let ty = case_type(owner, &case);
            let mut params = Vec::new();
            for (position, (_, kind)) in fields.iter().enumerate() {
                params.push(format!("{} %f{position}", ir_type(kind, span)?));
            }
            self.side(format!("define internal ptr {make}({})", params.join(", ")), |emitter| {
                let value = emitter.call("ptr", "zeno_alloc", format!("i64 {}", size_of(&ty)));
                // The tag is the first thing in the block.
                emitter.ins(format!("store i64 {index}, ptr {value}"));
                for (position, (_, kind)) in fields.iter().enumerate() {
                    let at = emitter.field_at(&ty, &value, position + 1);
                    emitter.ins(format!("store {} %f{position}, ptr {at}", ir_type(kind, span)?));
                }
                emitter.end(format!("ret ptr {value}"));
                Ok(())
            })?;
        }
        let value = self.set(format!("call ptr {make}({})", list.join(", ")));
        Ok(Val::new(value, Kind::Enum(owner.to_string())))
    }

    /// The function that shows an enum value as text: `Idle`, or
    /// `Circle(1.5)`. It reads the tag and goes to the block for that
    /// case. `switch` is a jump with many ways out: one for each number
    /// listed, and one for anything else.
    fn show_enum(&mut self, owner: &str, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.show.", owner);
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let cases = self.enums[owner].cases.clone();
        self.side(format!("define internal ptr {name}(ptr %self)"), |emitter| {
            let tag = emitter.set("load i64, ptr %self");
            let ways: Vec<String> = (0..cases.len()).map(|index| format!("i64 {index}, label %case{index}")).collect();
            emitter.end(format!("switch i64 {tag}, label %other [ {} ]", ways.join(" ")));
            for (index, (case, fields)) in cases.iter().enumerate() {
                emitter.place(&format!("case{index}"));
                if fields.is_empty() {
                    let text = emitter.string(case);
                    emitter.end(format!("ret ptr {text}"));
                    continue;
                }
                let ty = case_type(owner, case);
                let mut pieces = Vec::new();
                for (position, (_, kind)) in fields.iter().enumerate() {
                    pieces.push(emitter.string(&if position == 0 { format!("{case}(") } else { ", ".to_string() }));
                    let at = emitter.field_at(&ty, "%self", position + 1);
                    let value = emitter.set(format!("load {}, ptr {at}", ir_type(kind, span)?));
                    pieces.push(emitter.repr(&Val::new(value, kind.clone()), span)?);
                }
                pieces.push(emitter.string(")"));
                let shown = emitter.join(&pieces);
                emitter.end(format!("ret ptr {shown}"));
            }
            emitter.place("other");
            let empty = emitter.string("");
            emitter.end(format!("ret ptr {empty}"));
            Ok(())
        })?;
        Ok(name)
    }

    /// The function that says whether two values of an enum are equal:
    /// the same case, carrying equal values.
    fn same_enum(&mut self, owner: &str, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.same.", owner);
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let cases = self.enums[owner].cases.clone();
        self.side(format!("define internal i1 {name}(ptr %a, ptr %b)"), |emitter| {
            let a_tag = emitter.set("load i64, ptr %a");
            let b_tag = emitter.set("load i64, ptr %b");
            let same = emitter.set(format!("icmp eq i64 {a_tag}, {b_tag}"));
            emitter.jump_if(&same, "cases", "differ");
            emitter.place("cases");
            // Only a case that carries something has more to compare.
            let ways: Vec<String> = cases
                .iter()
                .enumerate()
                .filter(|(_, (_, fields))| !fields.is_empty())
                .map(|(index, _)| format!("i64 {index}, label %case{index}"))
                .collect();
            emitter.end(format!("switch i64 {a_tag}, label %alike [ {} ]", ways.join(" ")));
            for (index, (case, fields)) in cases.iter().enumerate() {
                if fields.is_empty() {
                    continue;
                }
                emitter.place(&format!("case{index}"));
                let ty = case_type(owner, case);
                for (position, (_, kind)) in fields.iter().enumerate() {
                    emitter.same_field(&ty, position + 1, kind, &format!("more{index}.{position}"), span)?;
                }
                emitter.end("ret i1 true");
            }
            emitter.place("alike");
            emitter.end("ret i1 true");
            emitter.place("differ");
            emitter.end("ret i1 false");
            Ok(())
        })?;
        Ok(name)
    }

    // --------------------------------------------------- Functions as values

    /// A function of the program used as a value: `apply(double, 3)`.
    ///
    /// A function value is the address of a small block: the address of
    /// its code, its name, and then whatever variables it took with it
    /// (see `closure`). Every function value is called the same way, with
    /// that block as a hidden first argument. An ordinary function does
    /// not take that argument, so it gets a small wrapper that does, and
    /// one fixed block that points at the wrapper.
    fn function_value(&mut self, name: &str, span: Span) -> Result<Val, Error> {
        let entry = self.functions[name].clone();
        let value = named('@', "zeno.value.", name);
        if self.written.insert(value.clone()) {
            let wrap = named('@', "zeno.wrap.", name);
            let mut params = vec!["ptr %self".to_string()];
            let mut args = Vec::new();
            for (index, param) in entry.kind.params.iter().enumerate() {
                let ty = ir_type(param, span)?;
                params.push(format!("{ty} %a{index}"));
                args.push(format!("{ty} %a{index}"));
            }
            let returns = return_type(&entry.kind.returns, span)?;
            self.side(format!("define internal {returns} {wrap}({})", params.join(", ")), |emitter| {
                let result = emitter.call_entry(&entry, args, span)?;
                match result.kind {
                    Kind::None => emitter.end("ret void"),
                    _ => emitter.end(format!("ret {returns} {}", result.code)),
                }
                Ok(())
            })?;
            let text = self.string(name);
            self.globals.push(format!("{value} = internal constant {{ ptr, ptr }} {{ ptr {wrap}, ptr {text} }}"));
        }
        Ok(Val::new(value, Kind::Function(Box::new(entry.kind))))
    }

    /// A function written where a value goes: `function(n) { return n * k }`.
    ///
    /// It becomes a function of its own in the IR. The variables it uses
    /// from the function around it are packed into the block that the
    /// function value is, right after the two fields every function value
    /// has. That pair, some code and the variables it took with it, is
    /// called a closure.
    ///
    /// ```text
    ///   %t1 = call ptr @zeno_alloc(i64 ...)          ; { code, name, k }
    ///   store ptr @zeno.fn.1, ptr %t1
    ///   ...
    /// define internal i64 @zeno.fn.1(ptr %self, i64 %arg.n) {
    /// entry:
    ///   %t1 = getelementptr { ptr, ptr, i64 }, ptr %self, i32 0, i32 2
    ///   %t2 = load i64, ptr %t1                       ; k
    /// ```
    ///
    /// A `let` is copied into the block: it never changes, so a copy is
    /// as good as the original. A `var` that such a function uses already
    /// lives in a box of its own (see `declare`), and the block holds the
    /// box's address, so both functions see each other's changes.
    fn closure(&mut self, decl: &Arc<Function>, span: Span) -> Result<Val, Error> {
        let Some(kind) = self.types.function(decl) else {
            return Err(cannot("this function", span));
        };
        let kind = FunctionKind { params: kind.params.iter().map(shared).collect(), returns: shared(&kind.returns) };
        self.closures += 1;
        let code_name = format!("@zeno.fn.{}", self.closures);

        // The variables of the functions around this spot that the new
        // function mentions. The top of the file is not among them: its
        // variables are globals, which every function can reach.
        let mut mentioned = HashSet::new();
        for statement in &decl.body {
            emit_c::names_in_functions(statement, true, &mut mentioned);
        }
        let mut mentioned: Vec<String> = mentioned.into_iter().collect();
        mentioned.sort();
        let mut taken = Vec::new();
        for name in mentioned {
            if decl.params.iter().any(|param| param.name == name) {
                continue;
            }
            if let Some(variable) = self.scopes[1..].iter().rev().find_map(|scope| scope.get(&name)) {
                taken.push((name, variable.clone()));
            }
        }
        let mut inside = vec!["ptr", "ptr"];
        for (name, variable) in &taken {
            inside.push(match &variable.place {
                Place::Value(_) => ir_type(&variable.kind, span)?,
                Place::Slot(slot) if self.code.on_stack.contains(slot) => {
                    return Err(cannot(format!("a function that uses `{name}` from outside it"), span))
                }
                Place::Slot(_) => "ptr",
            });
        }
        let block = format!("{{ {} }}", inside.join(", "));

        // The value itself is made here, each time this spot is reached:
        // two function values are equal only if they are the very same
        // one, as in `zeno run`.
        let made = self.call("ptr", "zeno_alloc", format!("i64 {}", size_of(&block)));
        self.ins(format!("store ptr {code_name}, ptr {made}"));
        let name_at = self.field_at(&block, &made, 1);
        let name = self.string(&decl.name);
        self.ins(format!("store ptr {name}, ptr {name_at}"));
        for (index, (_, variable)) in taken.iter().enumerate() {
            let at = self.field_at(&block, &made, index + 2);
            match &variable.place {
                Place::Value(code) => self.ins(format!("store {} {code}, ptr {at}", inside[index + 2])),
                Place::Slot(boxed) => self.ins(format!("store ptr {boxed}, ptr {at}")),
            }
        }

        // Set aside the function being written, and write the new one.
        let outer = std::mem::replace(&mut self.code, Code::of(&decl.body, kind.returns.clone(), true));
        let mut outside = std::mem::take(&mut self.scopes);
        let globals = outside.remove(0);
        // It begins by taking its variables out of the block.
        let mut brought = HashMap::new();
        for (index, (name, variable)) in taken.iter().enumerate() {
            let at = self.field_at(&block, "%self", index + 2);
            let place = match &variable.place {
                Place::Value(_) => Place::Value(self.set(format!("load {}, ptr {at}", inside[index + 2]))),
                Place::Slot(_) => Place::Slot(self.set(format!("load ptr, ptr {at}"))),
            };
            brought.insert(name.clone(), Variable { place, kind: variable.kind.clone(), open: variable.open });
        }
        let (params, list) = self.parameters(decl, &kind, span)?;
        self.scopes = vec![globals, brought, params];
        let done = self.block(&decl.body);

        // Put everything back as it was.
        outside.insert(0, self.scopes.remove(0));
        self.scopes = outside;
        done?;
        let mut all = vec!["ptr %self".to_string()];
        all.extend(list);
        let returns = return_type(&kind.returns, span)?;
        let text = self.finish(format!("define internal {returns} {code_name}({})", all.join(", ")));
        self.code = outer;
        self.definitions.push(text);
        Ok(Val::new(made, Kind::Function(Box::new(kind))))
    }

    /// A call to a function held in a variable, or to anything else that
    /// is a function value: `f(3)`, `make_adder(2)(3)`, `handlers[0]()`.
    fn call_value(&mut self, function: Val, args: &[Arg], span: Span) -> Result<Val, Error> {
        let Kind::Function(kind) = function.kind.clone() else {
            return Err(cannot(format!("a call to {}", described(&function.kind)), span));
        };
        if args.iter().any(|arg| arg.name.is_some()) {
            return Err(cannot("named arguments to a function held in a variable", span));
        }
        if args.len() != kind.params.len() {
            return Err(cannot("a call with this many arguments", span));
        }
        let mut vals = Vec::new();
        for arg in args {
            vals.push(self.expr(&arg.value)?);
        }
        self.invoke(&function.code, &kind, vals, span)
    }

    /// Call the function value at `function` with these values. The
    /// address of its code is the first thing in its block, and the block
    /// itself goes along as the first argument.
    fn invoke(&mut self, function: &str, kind: &FunctionKind, vals: Vec<Val>, span: Span) -> Result<Val, Error> {
        let mut list = vec![format!("ptr {function}")];
        for (val, param) in vals.into_iter().zip(&kind.params) {
            let val = self.fit(val, param, span)?;
            list.push(format!("{} {}", ir_type(param, span)?, val.code));
        }
        let code = self.set(format!("load ptr, ptr {function}"));
        let call = format!("call {} {code}({})", return_type(&kind.returns, span)?, list.join(", "));
        Ok(match &kind.returns {
            Kind::None => {
                self.ins(call);
                Val::nothing()
            }
            returns => Val::new(self.set(call), returns.clone()),
        })
    }

    /// `items.map(f)`, `items.filter(f)`, `items.reduce(f, start)`: each
    /// is a loop that calls `f` on every item.
    ///
    /// They walk the list as it was when they began, as `zeno run` does,
    /// so `f` may change the list without confusing the loop.
    fn over_list(&mut self, name: &str, list: &Val, item: &Kind, args: &[Arg], span: Span) -> Result<Val, Error> {
        let wrong = || cannot(format!("the list method `{name}` called this way"), span);
        if args.iter().any(|arg| arg.name.is_some()) {
            return Err(wrong());
        }
        let mut vals = Vec::new();
        for arg in args {
            vals.push(self.expr(&arg.value)?);
        }
        let takes = if name == "reduce" { 2 } else { 1 };
        let kind = match vals.first().map(|function| &function.kind) {
            Some(Kind::Function(kind)) if kind.params.len() == takes && vals.len() == takes => (**kind).clone(),
            _ => return Err(wrong()),
        };
        let function = vals[0].code.clone();

        // `reduce` starts from its second argument. The running total
        // changes on every trip, so it has a slot.
        let total = match vals.get(1) {
            Some(start) => {
                let start = self.fit(start.clone(), &kind.returns, span)?;
                let ty = ir_type(&kind.returns, span)?;
                let n = self.number();
                let total = self.slot(format!("%total.{n}"), ty);
                self.store(ty, &start.code, &total);
                Some((total, ty))
            }
            None => None,
        };
        let items = self.call("ptr", "zeno_list_copy", format!("ptr {}", list.code));
        let out = match name {
            "map" => {
                let len = self.set(format!("load i64, ptr {items}"));
                self.call("ptr", "zeno_list_new", format!("i64 {len}"))
            }
            "filter" if kind.returns != Kind::Bool => {
                return Err(cannot("`filter` with a function that does not return a bool", span))
            }
            "filter" => self.call("ptr", "zeno_list_new", "i64 0"),
            _ => String::new(),
        };

        let (count, slot) = self.walk_open(name, &items);
        let each = self.from_slot(item, &slot, span)?;
        let result = match (name, &total) {
            ("map", _) => {
                let made = self.invoke(&function, &kind, vec![each], span)?;
                let slot = self.to_slot(&made, span)?;
                self.run("zeno_list_append", format!("ptr {out}, i64 {slot}"));
                Val::new(out, Kind::List(Box::new(kind.returns.clone())))
            }
            ("filter", _) => {
                let wanted = self.invoke(&function, &kind, vec![each], span)?;
                let keep = format!("keep{}", self.number());
                self.jump_if(&wanted.code, &keep, &count.next);
                self.place(&keep);
                self.run("zeno_list_append", format!("ptr {out}, i64 {slot}"));
                Val::new(out, Kind::List(Box::new(item.clone())))
            }
            (_, Some((total, ty))) => {
                let so_far = Val::new(self.load(ty, total), kind.returns.clone());
                let next = self.invoke(&function, &kind, vec![so_far, each], span)?;
                self.store(ty, &next.code, total);
                Val::nothing()
            }
            _ => return Err(wrong()),
        };
        self.count_close(&count);
        Ok(match total {
            Some((total, ty)) => Val::new(self.load(ty, &total), kind.returns.clone()),
            None => result,
        })
    }

    // ------------------------------------------------- Showing and comparing

    /// Several pieces of text made into one.
    fn join(&mut self, pieces: &[String]) -> String {
        self.uses("zeno_join");
        let list: Vec<String> = pieces.iter().map(|piece| format!("ptr {piece}")).collect();
        self.set(format!("call ptr (i32, ...) @zeno_join(i32 {}, {})", pieces.len(), list.join(", ")))
    }

    /// The value as text, for `print`, `str` and f-strings.
    fn text(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        Ok(match &val.kind {
            Kind::Str => val.code.clone(),
            Kind::Int => self.call("ptr", "zeno_str_int", format!("i64 {}", val.code)),
            Kind::Float => self.call("ptr", "zeno_str_float", format!("double {}", val.code)),
            Kind::Bool => self.call("ptr", "zeno_str_bool", format!("i1 {}", val.code)),
            Kind::Pin => self.call("ptr", "zeno_str_pin", format!("i64 {}", val.code)),
            Kind::Quantity(unit) => {
                let unit = self.string(unit);
                self.call("ptr", "zeno_str_quantity", format!("double {}, ptr {unit}", val.code))
            }
            Kind::Class(name) => {
                let show = self.show_class(name, span)?;
                self.set(format!("call ptr {show}(ptr {})", val.code))
            }
            Kind::Enum(name) => {
                let show = self.show_enum(name, span)?;
                self.set(format!("call ptr {show}(ptr {})", val.code))
            }
            // The runtime knows how to lay a list out, and is handed a
            // function that shows one item.
            Kind::List(item) => {
                let show = self.repr_adapter(item, span)?;
                self.call("ptr", "zeno_list_str", format!("ptr {}, ptr {show}", val.code))
            }
            Kind::Error => {
                let at = self.field_at(ERROR, &val.code, 1);
                self.set(format!("load ptr, ptr {at}"))
            }
            Kind::Function(_) => self.call("ptr", "zeno_show_function", format!("ptr {}", val.code)),
            Kind::Optional(inner) => {
                let show = self.optional_text(inner, false, span)?;
                self.set(format!("call ptr {show}({} {})", ir_type(&val.kind, span)?, val.code))
            }
            // `none` itself, or a call that gave nothing back.
            Kind::None => self.string("none"),
            kind => return Err(cannot(format!("printing {}", described(kind)), span)),
        })
    }

    /// The text for a value inside a list or an object. Only a str
    /// differs: there it is shown in quotes.
    fn repr(&mut self, val: &Val, span: Span) -> Result<String, Error> {
        match &val.kind {
            Kind::Str => Ok(self.call("ptr", "zeno_repr_str", format!("ptr {}", val.code))),
            Kind::Optional(inner) => {
                let show = self.optional_text(inner, true, span)?;
                Ok(self.set(format!("call ptr {show}({} {})", ir_type(&val.kind, span)?, val.code)))
            }
            _ => self.text(val, span),
        }
    }

    /// One bit that is set when two values of this kind are equal.
    /// Objects, enum values and lists are equal when their contents are.
    fn equal(&mut self, kind: &Kind, a: &str, b: &str, span: Span) -> Result<String, Error> {
        Ok(match kind {
            Kind::Int | Kind::Pin => self.set(format!("icmp eq i64 {a}, {b}")),
            Kind::Float | Kind::Quantity(_) => self.set(format!("fcmp oeq double {a}, {b}")),
            Kind::Bool => self.set(format!("icmp eq i1 {a}, {b}")),
            Kind::Str => {
                let order = self.call("i32", "strcmp", format!("ptr {a}, ptr {b}"));
                self.set(format!("icmp eq i32 {order}, 0"))
            }
            Kind::Class(name) => {
                let same = self.same_class(name, span)?;
                self.set(format!("call i1 {same}(ptr {a}, ptr {b})"))
            }
            Kind::Enum(name) => {
                let same = self.same_enum(name, span)?;
                self.set(format!("call i1 {same}(ptr {a}, ptr {b})"))
            }
            Kind::List(item) => {
                let same = self.eq_adapter(item, span)?;
                self.call("i1", "zeno_list_eq", format!("ptr {a}, ptr {b}, ptr {same}"))
            }
            Kind::Error => self.call("i1", "zeno_error_same", format!("ptr {a}, ptr {b}")),
            Kind::Optional(inner) => {
                let same = self.optional_same(inner, span)?;
                let ty = ir_type(kind, span)?;
                self.set(format!("call i1 {same}({ty} {a}, {ty} {b})"))
            }
            Kind::None => "true".to_string(),
            // Two function values are equal only if they are the same one.
            Kind::Function(_) => self.set(format!("icmp eq ptr {a}, {b}")),
            Kind::Other(_) | Kind::Owned(_) => return Err(cannot("comparing these", span)),
        })
    }

    /// The function that shows a `T or none` as text: the value inside,
    /// or the word `none`. One is written for each kind, the first time
    /// it is needed. `quoted` is for a str inside a list or an object.
    fn optional_text(&mut self, inner: &Kind, quoted: bool, span: Span) -> Result<String, Error> {
        let quoted = quoted && *inner == Kind::Str;
        let kind = Kind::Optional(Box::new(inner.clone()));
        let name = named('@', if quoted { "zeno.repr." } else { "zeno.text." }, &key(&kind));
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let ty = ir_type(&kind, span)?;
        self.side(format!("define internal ptr {name}({ty} %x)"), |emitter| {
            let x = Val::new("%x", kind.clone());
            let there = emitter.is_there(&x, span)?;
            emitter.jump_if(&there, "some", "none");
            emitter.place("some");
            let value = emitter.inside(&x, span)?;
            let shown = if quoted { emitter.repr(&value, span)? } else { emitter.text(&value, span)? };
            emitter.end(format!("ret ptr {shown}"));
            emitter.place("none");
            let none = emitter.string("none");
            emitter.end(format!("ret ptr {none}"));
            Ok(())
        })?;
        Ok(name)
    }

    /// The same, for comparing two of them: equal when both are `none`,
    /// or when neither is and the values inside are equal.
    fn optional_same(&mut self, inner: &Kind, span: Span) -> Result<String, Error> {
        let kind = Kind::Optional(Box::new(inner.clone()));
        let name = named('@', "zeno.same.", &key(&kind));
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let ty = ir_type(&kind, span)?;
        self.side(format!("define internal i1 {name}({ty} %a, {ty} %b)"), |emitter| {
            let (a, b) = (Val::new("%a", kind.clone()), Val::new("%b", kind.clone()));
            let a_there = emitter.is_there(&a, span)?;
            let b_there = emitter.is_there(&b, span)?;
            let both = emitter.set(format!("and i1 {a_there}, {b_there}"));
            emitter.jump_if(&both, "both", "missing");
            emitter.place("both");
            let (a, b) = (emitter.inside(&a, span)?, emitter.inside(&b, span)?);
            let same = emitter.equal(&a.kind, &a.code, &b.code, span)?;
            emitter.end(format!("ret i1 {same}"));
            // At least one is `none`: equal only if the other is too.
            emitter.place("missing");
            let alike = emitter.set(format!("icmp eq i1 {a_there}, {b_there}"));
            emitter.end(format!("ret i1 {alike}"));
            Ok(())
        })?;
        Ok(name)
    }

    /// The runtime's list functions do not know what a list holds. To
    /// show a list they are handed a small function that shows one item,
    /// given its slot. This writes that function for a kind, the first
    /// time it is needed, and returns its name.
    fn repr_adapter(&mut self, kind: &Kind, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.item.repr.", &key(kind));
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        self.side(format!("define internal ptr {name}(i64 %item)"), |emitter| {
            let item = emitter.from_slot(kind, "%item", span)?;
            let shown = emitter.repr(&item, span)?;
            emitter.end(format!("ret ptr {shown}"));
            Ok(())
        })?;
        Ok(name)
    }

    /// The same, for comparing two items. C calls this one and reads a
    /// whole byte for the answer, which is what `zeroext` arranges.
    fn eq_adapter(&mut self, kind: &Kind, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.item.eq.", &key(kind));
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        self.side(format!("define internal zeroext i1 {name}(i64 %a, i64 %b)"), |emitter| {
            let (a, b) = (emitter.from_slot(kind, "%a", span)?, emitter.from_slot(kind, "%b", span)?);
            let same = emitter.equal(kind, &a.code, &b.code, span)?;
            emitter.end(format!("ret i1 {same}"));
            Ok(())
        })?;
        Ok(name)
    }

    // ------------------------------------------------------------ f-strings

    /// `f"{name} is {age:>3}"`: each piece as text, then all of them joined.
    fn format_string(&mut self, parts: &[FormatPart], span: Span) -> Result<Val, Error> {
        let mut pieces = Vec::new();
        for part in parts {
            pieces.push(match part {
                FormatPart::Text(literal) => self.string(literal),
                FormatPart::Value { value, format } => {
                    let val = self.expr(value)?;
                    match format {
                        Some(spec) => self.formatted(&val, spec, span)?,
                        None => self.text(&val, span)?,
                    }
                }
            });
        }
        Ok(Val::new(
            match pieces.as_slice() {
                [] => self.string(""),
                [only] => only.clone(),
                _ => self.join(&pieces),
            },
            Kind::Str,
        ))
    }

    /// A value with a format after the colon, as in `{x:.2f}` or `{n:08b}`.
    /// The format is read now, by the same function the C backend uses.
    fn formatted(&mut self, val: &Val, spec: &str, span: Span) -> Result<String, Error> {
        let bad = || Error::new(format!("`{spec}` is not a format Zeno understands"), span);
        let emit_c::Format { align, zero, width, precision, kind } = emit_c::read_format(spec).ok_or_else(bad)?;

        let is_number = matches!(val.kind, Kind::Int | Kind::Float);
        let body = match (kind, precision, &val.kind) {
            (Some('f'), _, _) | (None, Some(_), Kind::Float) if is_number => {
                let x = self.double(val);
                self.call("ptr", "zeno_fixed", format!("double {x}, i32 {}", precision.unwrap_or(6)))
            }
            (None, None, _) | (Some('d'), None, Kind::Int) => self.text(val, span)?,
            // A character is handed over as its number: `x` is 120.
            (Some(kind @ ('x' | 'X' | 'b')), None, Kind::Int) => {
                self.call("ptr", "zeno_radix", format!("i64 {}, i8 {}", val.code, kind as u32))
            }
            _ => return Err(bad()),
        };
        if width == 0 {
            return Ok(body);
        }
        // Numbers line up on the right by default, everything else on the left.
        let align = align.unwrap_or(if is_number { '>' } else { '<' });
        let args = format!("ptr {body}, i32 {width}, i8 {}, i1 {}", align as u32, zero && is_number);
        Ok(self.call("ptr", "zeno_pad", args))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::process::Command;

    fn compile(source: &str) -> Result<String, String> {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        emit(&program, &types, "test.zeno").map_err(|error| error.message)
    }

    /// Build the IR and the runtime with clang, run the program, and
    /// return what it printed, what it wrote as errors, and whether it
    /// finished cleanly. `None` if there is no clang here to try it with.
    fn build_and_run(name: &str, ir: &str) -> Option<(String, String, bool)> {
        let dir = std::env::temp_dir().join(format!("zeno-test-{}-llvm-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let (ir_file, runtime_file, binary) = (dir.join("test.ll"), dir.join("test_runtime.c"), dir.join("test"));
        std::fs::write(&ir_file, ir).expect("the IR file is written");
        std::fs::write(&runtime_file, runtime()).expect("the runtime is written");
        // ZENO_COLLECT_ALWAYS runs the garbage collector before every
        // single allocation, which is the setting most likely to show a
        // value the collector cannot find.
        let built = Command::new("clang")
            .args(["-O2", "-DZENO_COLLECT_ALWAYS", "-Wno-override-module", "-o"])
            .arg(&binary)
            .arg(&ir_file)
            .arg(&runtime_file)
            .arg("-lm")
            .output();
        let Ok(built) = built else {
            eprintln!("no clang found; skipping the compiled half of this test");
            return None;
        };
        let complaints = String::from_utf8_lossy(&built.stderr);
        let complaints: String = complaints.chars().take(1500).collect();
        assert!(built.status.success(), "the IR does not compile:\n{complaints}\n{ir}");
        let ran = Command::new(&binary).output().expect("the program runs");
        let _ = std::fs::remove_dir_all(&dir);
        Some((
            String::from_utf8_lossy(&ran.stdout).into_owned(),
            String::from_utf8_lossy(&ran.stderr).into_owned(),
            ran.status.success(),
        ))
    }

    /// The heart of these tests, as for the C backend: the compiled
    /// program must print exactly what the interpreter and the VM print,
    /// and stop with the same error at the same place.
    pub(crate) fn same_as_interpreter(name: &str, source: &str) {
        let tokens = crate::lexer::lex(source).expect("the test program lexes");
        let program = crate::parser::parse(tokens).expect("the test program parses");
        let (interpreted, result) = crate::vm::run_both(&program, source);

        let ir = compile(source).expect("the test program compiles");
        let Some((compiled, errors, finished)) = build_and_run(name, &ir) else {
            return;
        };
        assert_eq!(compiled, interpreted, "\n{source}");
        match &result {
            Ok(()) => assert!(finished, "the compiled program failed: {errors}"),
            Err(error) => {
                assert!(!finished, "the compiled program should have failed");
                let expected =
                    format!("error: {}\n --> test.zeno:{}:{}\n", error.message, error.span.line, error.span.col);
                assert_eq!(errors, expected, "\n{source}");
            }
        }
    }

    #[test]
    fn numbers_match() {
        same_as_interpreter(
            "numbers",
            "print(7 + 2, 7 - 9, 7 * 6, 7 / 2, 7 // 2, -7 // 2, 7 % 3, -7 % 3, 7 % -3, 2 ** 10, 2 ** 0)\n\
             print(1.5 + 2, 3 - 0.5, 2 * 2.5, 7.0 / 2, 7.5 // 2, 7.5 % 2, 2 ** 0.5, 2.0 ** 3)\n\
             print(-(3 + 4), -2.5, - -3, 1e21, 1e-7, 0.1 + 0.2, 100.0, 3.0 * 1e10)\n\
             print(9223372036854775807, -9223372036854775807 - 1)\n\
             print(abs(-5), abs(-2.5), min(3, 1, 2), max(3, 1, 2), min(2.5, 1.5), max(2.5, 1.5))\n\
             print(int(3.9), int(-3.9), int(\"42\"), float(3), float(\"2.5\"), int(7), int(1e30))\n",
        );
    }

    #[test]
    fn division_written_in_ir_matches() {
        // `//`, `%` and `/` are not the runtime's here (see `WRITTEN`),
        // so every mix of signs is tried, and the ends of what an int holds.
        same_as_interpreter(
            "division",
            "let small = -9223372036854775807 - 1\n\
             let big = 9223372036854775807\n\
             var minus = 0\n\
             minus -= 1\n\
             print(big // minus, small // 2, small // big, 0 // 5)\n\
             print(small % minus, big % minus, small % 7, small % -7, big % small, 0 % 5)\n\
             print(7 / 2, -7 / 2, 1 / 3, 0 / 5, 7.5 / -2.5)\n\
             for a in range(-7, 8) {\n\
             \x20   for b in range(-3, 4) {\n\
             \x20       if b != 0 { print(a, b, a // b, a % b) }\n\
             \x20   }\n\
             }\n",
        );
        same_as_interpreter(
            "division-overflow",
            "let small = -9223372036854775807 - 1\nvar minus = 0\nminus -= 1\nprint(small // minus)\n",
        );
    }

    #[test]
    fn comparisons_and_logic_match() {
        same_as_interpreter(
            "logic",
            "print(1 < 2, 2 <= 2, 3 > 4, 4 >= 5, 1 == 1, 1 != 1, 1 == 1.0, 1 < 1.5, 2.5 >= 2)\n\
             print(1 < 2 < 3, 1 < 2 < 2, 3 > 2 > 1 > 0, 1 < 3 > 2, 5 < 4 < 3)\n\
             print(\"apple\" < \"banana\", \"a\" == \"a\", \"a\" != \"b\", \"b\" >= \"a\", true == true, true != false)\n\
             print(true and false, true or false, not true, not (1 > 2), 1 < 2 and 2 < 3 or false)\n\
             function loud(name: str, answer: bool) -> bool {\n\
             \x20   print(f\"asked {name}\")\n\
             \x20   return answer\n\
             }\n\
             function number(name: str, n: int) -> int {\n\
             \x20   print(f\"asked {name}\")\n\
             \x20   return n\n\
             }\n\
             print(loud(\"a\", false) and loud(\"b\", true))\n\
             print(loud(\"c\", true) or loud(\"d\", true))\n\
             print(loud(\"e\", true) and loud(\"f\", false) or loud(\"g\", true))\n\
             print(number(\"h\", 3) < number(\"i\", 2) < number(\"j\", 1))\n\
             print(number(\"k\", 1) < number(\"l\", 2) < number(\"m\", 3))\n",
        );
    }

    #[test]
    fn text_matches() {
        same_as_interpreter(
            "text",
            "let name = \"Zeno\"\n\
             var greeting = \"hello, \" + name\n\
             greeting += \"!\"\n\
             print(greeting, len(greeting), greeting[0], greeting[7], str(12) + str(1.5) + str(true))\n\
             print(name.upper(), name.lower(), \"  padded  \".strip(), name.replace(\"Z\", \"X\"))\n\
             print(name.starts_with(\"Ze\"), name.ends_with(\"x\"))\n\
             print(f\"{name}: {3.14159:.2f} | {42:>6} | {42:<6}| {255:x} {255:X} {5:b} | {7:03}\")\n\
             print(f\"{name:>8} | {name:^8} | {2.5:8.3f} | {10:d} | {true} | {1 + 2}\")\n\
             print(f\"no values\", f\"{1}{2}{3}\", f\"{name}\", \"\")\n\
             print(\"quote \\\" backslash \\\\ tab\\tend\", \"na\u{ef}ve caf\u{e9} \u{2615}\")\n\
             print()\n",
        );
    }

    #[test]
    fn control_flow_matches() {
        same_as_interpreter(
            "control",
            "function classify(n: int) -> str {\n\
             \x20   if n < 0 {\n\
             \x20       return \"negative\"\n\
             \x20   } else if n == 0 {\n\
             \x20       return \"zero\"\n\
             \x20   } else if n < 10 {\n\
             \x20       return \"small\"\n\
             \x20   } else {\n\
             \x20       return \"big\"\n\
             \x20   }\n\
             }\n\
             print(classify(-5), classify(0), classify(7), classify(70))\n\
             function collatz(start: int) -> int {\n\
             \x20   var n = start\n\
             \x20   var steps = 0\n\
             \x20   while n != 1 {\n\
             \x20       if n % 2 == 0 {\n\
             \x20           n = n // 2\n\
             \x20       } else {\n\
             \x20           n = 3 * n + 1\n\
             \x20       }\n\
             \x20       steps += 1\n\
             \x20   }\n\
             \x20   return steps\n\
             }\n\
             print(collatz(27))\n\
             var total = 0\n\
             for i in range(10) {\n\
             \x20   if i == 3 { continue }\n\
             \x20   if i == 8 { break }\n\
             \x20   total += i\n\
             }\n\
             print(total)\n\
             for i in range(2, 5) { print(i) }\n\
             for i in range(10, 0, -3) { print(i) }\n\
             for i in range(0, 10, 4) { print(i) }\n\
             for i in range(5, 5) { print(\"never\") }\n\
             var sum = 0\n\
             for a in range(4) {\n\
             \x20   for b in range(a) {\n\
             \x20       if b == 2 { break }\n\
             \x20       sum += a * b\n\
             \x20   }\n\
             }\n\
             var n = 0\n\
             while true {\n\
             \x20   n += 1\n\
             \x20   if n < 5 { continue }\n\
             \x20   break\n\
             }\n\
             print(sum, n)\n",
        );
    }

    #[test]
    fn functions_match() {
        same_as_interpreter(
            "functions",
            "print(fib(20), power(2), power(2, 5), power(exponent=3, base=4))\n\
             function fib(n: int) -> int {\n\
             \x20   if n < 2 { return n }\n\
             \x20   return fib(n - 1) + fib(n - 2)\n\
             }\n\
             function power(base: int, exponent: int = 2) -> int {\n\
             \x20   var result = 1\n\
             \x20   for i in range(exponent) { result *= base }\n\
             \x20   return result\n\
             }\n\
             function shout(what: str) {\n\
             \x20   print(what.upper() + \"!\")\n\
             \x20   return\n\
             \x20   print(\"never\")\n\
             }\n\
             shout(\"done\")\n\
             print(shout(\"again\"))\n\
             function shadow(n: int) -> int {\n\
             \x20   let x = n\n\
             \x20   var y = 1\n\
             \x20   if n > 0 {\n\
             \x20       let x = n * 2\n\
             \x20       var y = 10\n\
             \x20       y += x\n\
             \x20       return y\n\
             \x20   }\n\
             \x20   return x + y\n\
             }\n\
             print(shadow(4), shadow(0))\n\
             function average(a: float, b: float) -> float { return (a + b) / 2 }\n\
             print(average(1.0, 2.0), average(b=4.0, a=1.5))\n\
             function untyped(a, b) { return a * b + 1 }\n\
             print(untyped(3, 4))\n",
        );
    }

    #[test]
    fn variables_at_the_top_are_shared_with_functions() {
        same_as_interpreter(
            "globals",
            "var counter = 0\n\
             let step = 5\n\
             var log = \"\"\n\
             function bump() -> int {\n\
             \x20   counter += step\n\
             \x20   log += f\"{counter} \"\n\
             \x20   return counter\n\
             }\n\
             bump()\n\
             bump()\n\
             print(counter, bump(), log)\n\
             if counter > 10 {\n\
             \x20   let inside = counter * 2\n\
             \x20   var changing = inside\n\
             \x20   changing += 1\n\
             \x20   print(inside, changing)\n\
             }\n",
        );
        // A function means the built-in `len` when the `len` of the
        // program is declared below it (decision 3.6d).
        same_as_interpreter("below", "function size(text: str) -> int { return len(text) }\nlet len = 5\nprint(size(\"abc\"), len)\n");
    }

    #[test]
    fn text_survives_the_collector() {
        // Text that is held only by a value in the IR, or by a global,
        // while a great deal more is made and thrown away.
        same_as_interpreter(
            "collector",
            "var kept = \"start\"\n\
             function grow(times: int) -> str {\n\
             \x20   var text = \"\"\n\
             \x20   for i in range(times) {\n\
             \x20       let piece = f\"{i},\"\n\
             \x20       text += piece\n\
             \x20       if i % 50 == 0 { kept = text.upper() + \"!\" }\n\
             \x20   }\n\
             \x20   return text\n\
             }\n\
             let made = grow(120)\n\
             var again = \"\"\n\
             for i in range(300) {\n\
             \x20   again = str(i) + made[i % 10]\n\
             }\n\
             print(len(made), len(kept), made[0], kept[5], again)\n",
        );
    }

    #[test]
    fn errors_stop_the_program_the_same_way() {
        let programs = [
            "print(\"before\")\nlet big = 9223372036854775807\nprint(big + 1)\n",
            "let big = 9223372036854775807\nprint(big * 2)\n",
            "var low = -9223372036854775807\nlow -= 2\nprint(low)\n",
            "let low = -9223372036854775807 - 1\nprint(-low)\n",
            "let low = -9223372036854775807 - 1\nprint(abs(low))\n",
            "let zero = 0\nprint(1 // zero)\n",
            "let zero = 0\nprint(1 % zero)\n",
            "let zero = 0\nprint(1 / zero)\n",
            "let zero = 0.0\nprint(1.5 // zero)\n",
            "print(2 ** -1)\n",
            "print(int(\"abc\"))\n",
            "print(float(\"x1\"))\n",
            "let text = \"abc\"\nprint(text[5])\n",
            "for i in range(0, 10, 0) { print(i) }\n",
            "function deep(n: int) -> int {\n    if n == 0 { return 1 // n }\n    return deep(n - 1)\n}\nprint(\"start\")\nprint(deep(3))\n",
        ];
        for (index, program) in programs.iter().enumerate() {
            same_as_interpreter(&format!("error-{index}"), program);
        }
    }

    #[test]
    fn the_examples_it_can_compile_match() {
        same_as_interpreter("hello", include_str!("../examples/hello.zeno"));
        same_as_interpreter("primes", include_str!("../examples/primes.zeno"));
        same_as_interpreter("tour", include_str!("../examples/tour.zeno"));
        same_as_interpreter("styles", include_str!("../examples/styles.zeno"));
        same_as_interpreter("shapes", include_str!("../examples/shapes.zeno"));
        same_as_interpreter("sensor", include_str!("../examples/sensor.zeno"));
        same_as_interpreter("memory", include_str!("../examples/memory.zeno"));
        same_as_interpreter("buffer", include_str!("../examples/buffer.zeno"));
        same_as_interpreter("signal", include_str!("../examples/signal.zeno"));
    }

    #[test]
    fn classes_match() {
        same_as_interpreter(
            "classes",
            "class Point {\n    x: float = 0.0\n    y: float = 0.0\n    name: str = \"p\"\n\
             \x20   function moved(self, dx: float = 1.0, dy: float = 0.0) -> Point {\n\
             \x20       return Point(self.x + dx, self.y + dy, self.name)\n    }\n\
             \x20   function shout(self) { print(self.name.upper(), self.x, self.y) }\n}\n\
             class Empty { }\n\
             class Node {\n    value: int\n    next: Node or none\n}\n\
             let p = Point(y=2.0)\n\
             p.moved(dy=5.0).shout()\n\
             p.x = 7.5\np.x += 1\np.shout()\n\
             print(p, p == Point(8.5, 2.0), Point() == Point(), p == Point())\n\
             print(Empty(), Empty() == Empty(), [Empty()])\n\
             var chain: Node or none = none\n\
             for i in range(4) { chain = Node(i, chain) }\n\
             print(chain, chain == Node(3, Node(2, Node(1, Node(0, none)))), chain == Node(3, none))\n",
        );
    }

    #[test]
    fn lists_match() {
        same_as_interpreter(
            "lists",
            "let grid: list[list[int]] = [[1, 2], [3, 4], []]\n\
             grid[2].append(9)\ngrid[0][1] = 20\ngrid[1][0] += 5\n\
             print(grid, len(grid), grid[1], grid == [[1, 20], [8, 4], [9]], [9] in grid, [7] not in grid)\n\
             print([n * n for n in range(6) if n % 2 == 1], [c.upper() for c in \"abc\"], [len(row) for row in grid])\n\
             print(sum([1, 2, 3]), sum([0.5, 0.25]), max([1.5, 0.5]), min([4, 2, 8]), [1] + [2, 3])\n\
             print(\"a b  c\".split(), \"a,b\".split(\",\"), \"b\" in \"abc\", \"z\" not in \"abc\")\n\
             var words: list[str] = []\n\
             for word in \"the quick brown\".split() { words.append(word.upper()) }\n\
             print(words, words.pop(), words, [true, false], [1.5, 2.0], [1 s, 2 s])\n\
             var seen = 0\n\
             for word in words {\n    words.append(\"again\")\n    seen += 1\n}\n\
             print(seen, len(words))\n\
             for letter in \"hey\" { print(letter) }\n\
             print(words[9])\n",
        );
    }

    #[test]
    fn enums_and_match_match() {
        same_as_interpreter(
            "enums",
            "enum Shape {\n    Circle(r: float)\n    Rect(w: float, h: float)\n    Dot\n    Label(text: str, of: Shape or none)\n}\n\
             function area(s: Shape) -> float {\n    match s {\n\
             \x20       case Circle(r) { return 3.0 * r * r }\n\
             \x20       case Rect(w, h) { return w * h }\n\
             \x20       case Dot { return 0.0 }\n\
             \x20       case Label(_, inner) {\n            if inner is not none { return area(inner) }\n            return -1.0\n        }\n\
             \x20   }\n}\n\
             let shapes = [Circle(1.0), Rect(2.0, 3.0), Dot, Label(\"hi\", Circle(2.0)), Label(\"no\", none)]\n\
             for s in shapes { print(s, area(s)) }\n\
             print(shapes, Dot == Dot, Circle(1.0) == Circle(1.0), Circle(1.0) == Circle(2.0), Circle(1.0) == Dot)\n\
             print(Rect(2.0, 3.0) in shapes, Shape.Dot, Shape.Circle(0.5), Label(\"a\", Dot) == Label(\"a\", Dot))\n\
             function describe(n: int) -> str {\n    match n {\n\
             \x20       case 0 { return \"zero\" }\n        case 1 { return \"one\" }\n        case other { return f\"many ({other})\" }\n\
             \x20   }\n}\n\
             print(describe(0), describe(1), describe(7))\n\
             match \"x\" + \"y\" {\n    case \"xy\" { print(\"joined\") }\n    case _ { print(\"no\") }\n}\n\
             match Rect(1.0, 2.0) {\n    case Rect(1.0, h) { print(\"one wide\", h) }\n    case _ { print(\"no\") }\n}\n",
        );
    }

    #[test]
    fn values_that_may_be_none_match() {
        same_as_interpreter(
            "none",
            "function find(items: list[str], want: str) -> int or none {\n\
             \x20   for i in range(len(items)) { if items[i] == want { return i } }\n}\n\
             let at = find([\"a\", \"b\"], \"b\")\n\
             print(at, find([\"a\"], \"z\"), at == 1, at != none, at is none, find([], \"a\") is none)\n\
             if at is not none { print(at + 1) }\n\
             match at {\n    case none { print(\"nothing\") }\n    case 1 { print(\"one\") }\n    case n { print(\"other\", n) }\n}\n\
             let maybe: list[int or none] = [1, none, 3]\n\
             print(maybe, maybe[1], maybe[2], maybe == [1, none, 3], none in maybe, 2 in maybe, 3 in maybe)\n\
             var flags: list[bool or none] = [true, none, false]\n\
             flags.append(none)\nflags[0] = false\n\
             print(flags, flags.pop(), len(flags))\n\
             let halves: list[float or none] = [1.5, none]\n\
             print(halves, halves[0] == 1.5, halves[1] is none)\n\
             var name: str or none = none\n\
             print(name, name is none)\n\
             name = \"zeno\"\n\
             if name is not none { print(name.upper(), [name]) }\n\
             var level: float or none = 2.5\n\
             function raise_it() { level = 4.0 }\n\
             if level is not none {\n    print(level * 2)\n    raise_it()\n    print(level * 2)\n}\n",
        );
    }

    #[test]
    fn a_shared_variable_is_looked_at_again_after_a_check() {
        // `clear()` sets `n` back to none after the check. The compiled
        // program must stop, not carry on with a number that is not there.
        let source = "var n: int or none = 5\nfunction clear() { n = none }\nif n is not none {\n clear()\n print(n + 1)\n}";
        let ir = compile(source).expect("it compiles");
        assert!(ir.contains("call void @zeno_gone(i32 5, i32 8)"), "{ir}");
        let Some((printed, errors, finished)) = build_and_run("gone", &ir) else {
            return;
        };
        assert_eq!(printed, "");
        assert!(!finished);
        assert_eq!(
            errors,
            "error: this was checked with `is not none`, but it has been set to `none` since\n --> test.zeno:5:8\n"
        );
        // A variable nothing else can reach needs no second look.
        let ir = compile("function f(n: int or none) -> int {\n if n is not none { return n + 1 }\n return 0\n}\nprint(f(1))")
            .expect("it compiles");
        assert!(!ir.contains("zeno_gone"), "{ir}");
    }

    #[test]
    fn errors_are_raised_and_caught_the_same_way() {
        // `total` and `seen` change inside the `try` and are read after
        // an error has jumped out of it: the case `volatile` is for.
        same_as_interpreter(
            "errors",
            "function risky(n: int) -> int raises ValueError {\n\
             \x20   if n > 2 { raise ValueError(f\"too big: {n}\") }\n    return n * 10\n}\n\
             function count_up() -> int {\n    var total = 0\n    var seen = 0\n\
             \x20   for i in range(6) {\n        try {\n            seen = seen + 1\n            total = total + risky(i)\n            seen = seen + 100\n\
             \x20       } except ValueError as err {\n            total = total + 1\n            print(\"caught\", err.message, err.kind, seen, total)\n        }\n    }\n\
             \x20   return total + seen\n}\n\
             print(count_up())\n\
             function early(n: int) -> int {\n    var steps = 0\n    while true {\n        steps = steps + 1\n\
             \x20       try {\n            if steps == n { return steps * 2 }\n            if steps % 2 == 0 { continue }\n            if steps > 50 { break }\n\
             \x20           let x = 10 // (steps - 3)\n            steps = steps + x - x\n\
             \x20       } except ZeroDivisionError {\n            print(\"zero at\", steps)\n        }\n    }\n    return -1\n}\n\
             print(early(7), early(100))\n\
             class Fault {\n    code: int\n    place: str\n}\n\
             function deep(n: int) raises Fault {\n    if n == 0 { raise Fault(42, \"deep\") }\n    deep(n - 1)\n}\n\
             try {\n    try { deep(5) } except ValueError { print(\"no\") }\n} except Fault as fault {\n    print(\"fault\", fault.code, fault.place, fault)\n}\n\
             try { deep(2) } except Error as error { print(\"any\", error, error.kind) }\n\
             function nested() -> str {\n    var log = \"\"\n    try {\n        log = log + \"a\"\n\
             \x20       try {\n            log = log + \"b\"\n            raise ValueError(\"inner\")\n\
             \x20       } except ValueError as error {\n            log = log + \"c\"\n            raise Error(\"outer \" + error.message)\n        }\n\
             \x20   } except Error as error {\n        log = log + \"d:\" + error.message\n    }\n    return log\n}\n\
             print(nested())\n\
             print(int(\"12x\"))\n",
        );
    }

    #[test]
    fn a_var_changed_in_a_try_is_kept_in_memory() {
        let ir = compile(
            "function f() -> int {\n    var n = 0\n    try { n = 1 } except Error { n = 2 }\n    return n\n}\nprint(f())\n",
        )
        .expect("it compiles");
        assert!(ir.contains("store volatile i64 1, ptr %var.n"), "{ir}");
        assert!(ir.contains("call i32 @_setjmp(ptr %try.1) returns_twice"), "{ir}");
        // A function with no `try` in it has no need of that.
        let ir = compile("function f() -> int {\n    var n = 0\n    n = 1\n    return n\n}\nprint(f())\n").expect("it compiles");
        assert!(!ir.contains("volatile"), "{ir}");
    }

    #[test]
    fn functions_as_values_match() {
        same_as_interpreter(
            "closures",
            "function make_counter(start: int) -> function() -> int {\n    var count = start\n\
             \x20   return function() -> int {\n        count = count + 1\n        return count\n    }\n}\n\
             let a = make_counter(10)\nlet b = make_counter(100)\n\
             print(a(), a(), b(), a(), b())\n\
             function twice(f: function(int) -> int, x: int) -> int { return f(f(x)) }\n\
             function inc(n: int) -> int { return n + 1 }\n\
             function adder(k: int) -> function(int) -> int {\n    return function(n: int) -> int { return n + k }\n}\n\
             let add5 = adder(5)\n\
             print(twice(inc, 5), twice(function(n: int) -> int { return n * n }, 3), add5(1), adder(2)(3), inc)\n\
             let nums = [1, 2, 3, 4, 5, 6]\n\
             print(nums.map(function(n: int) -> int { return n * 2 }))\n\
             print(nums.filter(function(n: int) -> bool { return n % 2 == 0 }))\n\
             print(nums.reduce(function(sum: int, n: int) -> int { return sum + n }, 0), nums.map(inc).map(add5))\n\
             function outer() -> int {\n    var total = 0\n    let bump = function(by: int) { total = total + by }\n\
             \x20   for i in range(5) { bump(i) }\n\
             \x20   let sizes = [\"x\", \"yy\"].map(function(s: str) -> int {\n        total = total + 1\n        return len(s)\n    })\n\
             \x20   print(sizes, total)\n    return total\n}\n\
             print(outer())\n\
             class Button {\n    label: str\n    on_click: function(str) -> str\n}\n\
             let button = Button(\"ok\", function(who: str) -> str { return \"clicked by \" + who })\n\
             print(button.on_click(\"me\"), button.label)\n\
             let all: list[function(int) -> int] = [inc, add5, function(n: int) -> int { return -n }]\n\
             for f in all { print(f(10)) }\n\
             print(all[2](7))\n\
             if true {\n    var local = 3\n    let peek = function() -> int {\n        local = local + 1\n        return local\n    }\n\
             \x20   print(peek(), peek(), local)\n}\n",
        );
    }

    #[test]
    fn units_and_imports_match() {
        same_as_interpreter(
            "units",
            "import math\nfrom math import floor, pi\n\
             print(500 ms + 1 s, (2 s).ms, 3 V * 2, 10 s / 4 s, 6 V / 2, 1 s > 900 ms, abs(-3 ms), -(2 s))\n\
             let wait: ms = 250 ms\n\
             print(wait, wait * 4 == 1 s, wait.s, f\"{wait}\", [wait, 1 ms])\n\
             print(math.sqrt(16.0), floor(2.7), math.ceil(2.1), math.sin(0.0), math.cos(0.0), pi > 3.14, math.pi < 3.15)\n\
             print(math.sqrt(-1.0))\n",
        );
    }

    #[test]
    fn the_ir_for_a_class() {
        let ir = compile(
            "class Point {\n    x: float\n    y: float\n    function width(self) -> float { return self.x }\n}\n\
             let p = Point(1.0, 2.0)\np.y = 3.0\nprint(p.width())\n",
        )
        .expect("it compiles");
        // A class is a struct type, and a field is found with
        // `getelementptr`.
        assert!(ir.contains("%z_Point = type { double, double }"), "{ir}");
        let method = "define internal double @z_Point.width(ptr %arg.self) {\n\
                      entry:\n\
                      \x20 %t1 = getelementptr %z_Point, ptr %arg.self, i32 0, i32 0\n\
                      \x20 %t2 = load double, ptr %t1\n\
                      \x20 ret double %t2\n\
                      }\n";
        assert!(ir.contains(method), "{ir}");
        assert!(ir.contains("getelementptr %z_Point, ptr %t2, i32 0, i32 1\n  store double 3.0, ptr %t3"), "{ir}");
    }

    #[test]
    fn the_runtime_can_be_linked_to() {
        let runtime = runtime();
        assert!(!runtime.lines().any(|line| line.starts_with("static ")));
        // Everything the IR may call is there to be found.
        for (name, _) in DECLARED {
            if name.starts_with("zeno_") {
                assert!(runtime.contains(&format!(" {name}(")) || runtime.contains(&format!("*{name}(")), "{name}");
            }
        }
    }

    #[test]
    fn the_ir_for_a_small_function() {
        // A `let` and a parameter are values. A `var` has a slot.
        let ir = compile(
            "function scale(n: int, by: float) -> float {\n\
             \x20   let twice = by * 2.0\n\
             \x20   var result = twice\n\
             \x20   if n > 0 { result = result * 0.5 }\n\
             \x20   return result\n\
             }\n\
             print(scale(1, 1.5))\n",
        )
        .expect("it compiles");
        let function = "define internal double @z_scale(i64 %arg.n, double %arg.by) {\n\
                        entry:\n\
                        \x20 %var.result = alloca double\n\
                        \x20 %t1 = fmul double %arg.by, 2.0\n\
                        \x20 store double %t1, ptr %var.result\n\
                        \x20 %t2 = icmp sgt i64 %arg.n, 0\n\
                        \x20 br i1 %t2, label %then1, label %end1\n\
                        \n\
                        then1:\n\
                        \x20 %t3 = load double, ptr %var.result\n\
                        \x20 %t4 = fmul double %t3, 0.5\n\
                        \x20 store double %t4, ptr %var.result\n\
                        \x20 br label %end1\n\
                        \n\
                        end1:\n\
                        \x20 %t5 = load double, ptr %var.result\n\
                        \x20 ret double %t5\n\
                        }\n";
        assert!(ir.contains(function), "{ir}");
    }

    #[test]
    fn the_ir_for_blink() {
        let ir = compile(include_str!("../examples/blink.zeno")).expect("it compiles");
        // A pin is its number, and each thing done to it is one call.
        // 500 ms reaches the board as seconds.
        let body = "do1:\n\
                    \x20 call void @zeno_pin_toggle(i64 13)\n\
                    \x20 %t1 = sitofp i64 500 to double\n\
                    \x20 %t2 = fdiv double %t1, 1000.0\n\
                    \x20 call void @zeno_sleep(double %t2)\n\
                    \x20 br label %while1\n";
        assert!(ir.contains(body), "{ir}");
    }

    #[test]
    fn what_is_not_compiled_yet_says_so() {
        let refused = [
            (
                "function outer() -> int {\n    function inner() -> int { return 1 }\n    return inner()\n}\nprint(outer())\n",
                "a function with a name inside a function or a block",
            ),
            ("class Box {\n    function size(self) -> int { return 1 }\n}\nlet f = Box().size\n", "a method used as a value: `size`"),
            (
                "function f(n: int = 1 + 1) -> int { return n }\nprint(f())\n",
                "a default value that is not a plain literal",
            ),
        ];
        for (source, what) in refused {
            let message = compile(source).expect_err("it should be refused");
            assert_eq!(
                message,
                format!("`zeno build --llvm` cannot compile {what} yet. `zeno run` can still run this program"),
                "\n{source}"
            );
        }
    }
}
