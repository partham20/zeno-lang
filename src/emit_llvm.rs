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
//! written, then functions, classes and enums, statements, what a build
//! with no collector needs, owned values, loops, `match` and `try`,
//! expressions (with lists and values that may be `none`), calls, objects
//! and enum values, functions as values, and at the end how a value is
//! shown as text and compared with another.
//!
//! The compiled program still needs the runtime (`runtime.c`) for text,
//! printing, lists, errors and the collector. `zeno build --llvm` writes
//! it out as a second file and hands both to clang. See `runtime`.
//!
//! It builds the three ways the C backend does (`Build` in `emit_c`):
//! the ordinary way, `--firmware` (errors as values, nothing jumps) and
//! `--no-collector` (a firmware build with no collector in it).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::ast::*;
use crate::checker::{FunctionKind, Kind, Pointee, Types};
use crate::emit_c::{self, is_text, is_written, Build};
use crate::error::Error;
use crate::ints::{self, Width};
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
pub fn runtime(build: Build) -> String {
    let (firmware, collector) = (build != Build::Ordinary, build != Build::NoCollector);
    // A function that never returns has that said in front of `static`.
    const NEVER: &str = "__attribute__((noreturn)) ";
    let mut text = String::new();
    for line in emit_c::runtime(firmware, collector).lines() {
        match line.strip_prefix(NEVER) {
            Some(rest) => {
                text += NEVER;
                text += rest.strip_prefix("static ").unwrap_or(rest);
            }
            None => text += line.strip_prefix("static ").unwrap_or(line),
        }
        text.push('\n');
    }
    // A firmware build has no `setjmp`, so it has no `zeno_try` either.
    if !firmware {
        text += FOR_TRY;
    }
    text
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
    // A register is its address, and the `i32` is how many bits it has.
    ("zeno_register_read", "declare i64 @zeno_register_read(i64, i32 signext)"),
    ("zeno_register_write", "declare void @zeno_register_write(i64, i32 signext, i64)"),
    ("zeno_str_register", "declare ptr @zeno_str_register(i64)"),
    // `allocate` and `free`: how many items, and the bytes in one.
    ("zeno_allocate", "declare ptr @zeno_allocate(i64, i64, i32 signext, i32 signext)"),
    ("zeno_release", "declare void @zeno_release(ptr)"),
    ("zeno_sleep", "declare void @zeno_sleep(double)"),
    ("zeno_now", "declare double @zeno_now()"),
    ("zeno_abs", "declare i64 @zeno_abs(i64, i32 signext, i32 signext)"),
    ("zeno_shl", "declare i64 @zeno_shl(i64, i64, i32 signext, i32 signext)"),
    ("zeno_fit", "declare i64 @zeno_fit(i64, i64, i64, ptr, i32 signext, i32 signext)"),
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
    ("tan", "declare double @tan(double)"),
    ("atan", "declare double @atan(double)"),
    ("atan2", "declare double @atan2(double, double)"),
    ("exp", "declare double @exp(double)"),
    ("round", "declare double @round(double)"),
    ("zeno_asin", "declare double @zeno_asin(double, i32 signext, i32 signext)"),
    ("zeno_acos", "declare double @zeno_acos(double, i32 signext, i32 signext)"),
    ("zeno_log", "declare double @zeno_log(double, i32 signext, i32 signext, i32 signext)"),
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
    ("zeno_lines", "declare ptr @zeno_lines(ptr)"),
    ("zeno_input", "declare ptr @zeno_input(ptr)"),
    ("zeno_program_start", "declare void @zeno_program_start(i32 signext, ptr)"),
    ("zeno_program_arguments", "declare ptr @zeno_program_arguments()"),
    ("zeno_chars", "declare ptr @zeno_chars(ptr)"),
    ("zeno_list_new", "declare ptr @zeno_list_new(i64)"),
    ("zeno_list_of", "declare ptr @zeno_list_of(i32 signext, ...)"),
    ("zeno_list_append", "declare void @zeno_list_append(ptr, i64)"),
    ("zeno_list_copy", "declare ptr @zeno_list_copy(ptr)"),
    ("zeno_list_drop", "declare void @zeno_list_drop(ptr)"),
    ("zeno_free", "declare void @zeno_free(ptr)"),
    ("zeno_error_drop", "declare void @zeno_error_drop(ptr)"),
    ("zeno_closure_drop", "declare void @zeno_closure_drop(ptr)"),
    ("zeno_closure_release", "declare void @zeno_closure_release(ptr)"),
    // Text that is kept, in a build with no collector (docs/text.md).
    ("zeno_text_keep", "declare ptr @zeno_text_keep(ptr)"),
    ("zeno_text_drop", "declare void @zeno_text_drop(ptr)"),
    ("zeno_text_lend", "declare ptr @zeno_text_lend(ptr)"),
    ("zeno_text_take", "declare ptr @zeno_text_take(ptr)"),
    ("zeno_text_return", "declare ptr @zeno_text_return(i64, ptr)"),
    ("zeno_list_drop_text", "declare void @zeno_list_drop_text(ptr)"),
    ("zeno_list_copy_text", "declare ptr @zeno_list_copy_text(ptr)"),
    ("zeno_list_keep_text", "declare ptr @zeno_list_keep_text(ptr)"),
    ("zeno_error_of", "declare ptr @zeno_error_of(ptr, ptr, ptr, ptr)"),
    ("zeno_list_concat", "declare ptr @zeno_list_concat(ptr, ptr)"),
    ("zeno_list_at", "declare ptr @zeno_list_at(ptr, i64, i32 signext, i32 signext)"),
    ("zeno_list_pop", "declare i64 @zeno_list_pop(ptr, i32 signext, i32 signext)"),
    ("zeno_list_str", "declare ptr @zeno_list_str(ptr, ptr)"),
    ("zeno_list_eq", "declare zeroext i1 @zeno_list_eq(ptr, ptr, ptr)"),
    ("zeno_list_contains", "declare zeroext i1 @zeno_list_contains(ptr, i64, ptr)"),
    ("zeno_list_find", "declare i64 @zeno_list_find(ptr, i64, ptr)"),
    ("zeno_list_insert", "declare void @zeno_list_insert(ptr, i64, i64, i32 signext, i32 signext)"),
    ("zeno_list_pop_at", "declare i64 @zeno_list_pop_at(ptr, i64, i32 signext, i32 signext)"),
    ("zeno_list_reverse", "declare void @zeno_list_reverse(ptr)"),
    ("zeno_list_clear", "declare void @zeno_list_clear(ptr)"),
    ("zeno_list_sort", "declare void @zeno_list_sort(ptr, i32 signext)"),
    ("zeno_list_slice", "declare ptr @zeno_list_slice(ptr, i64, i64, i1 zeroext, i32 signext, i32 signext)"),
    ("zeno_list_join", "declare ptr @zeno_list_join(ptr, ptr)"),
    // Dicts. A key or a value travels as a slot of 64 bits, as an item
    // of a list does.
    ("zeno_dict_new", "declare ptr @zeno_dict_new(i32 signext, i32 signext)"),
    ("zeno_dict_kinds", "declare ptr @zeno_dict_kinds(ptr, i32 signext, i32 signext)"),
    ("zeno_dict_set", "declare void @zeno_dict_set(ptr, i64, i64)"),
    ("zeno_dict_at", "declare i64 @zeno_dict_at(ptr, i64, i32 signext, i32 signext)"),
    ("zeno_dict_find", "declare i64 @zeno_dict_find(ptr, i64)"),
    ("zeno_dict_value", "declare i64 @zeno_dict_value(ptr, i64)"),
    ("zeno_dict_get", "declare i64 @zeno_dict_get(ptr, i64, i64)"),
    ("zeno_dict_pop", "declare i64 @zeno_dict_pop(ptr, i64, i32 signext, i32 signext)"),
    ("zeno_dict_keys", "declare ptr @zeno_dict_keys(ptr)"),
    ("zeno_dict_values", "declare ptr @zeno_dict_values(ptr)"),
    ("zeno_dict_clear", "declare void @zeno_dict_clear(ptr)"),
    ("zeno_dict_drop", "declare void @zeno_dict_drop(ptr)"),
    ("zeno_dict_str", "declare ptr @zeno_dict_str(ptr)"),
    ("zeno_dict_eq", "declare zeroext i1 @zeno_dict_eq(ptr, ptr)"),
    ("zeno_find", "declare i64 @zeno_find(ptr, ptr)"),
    ("zeno_slice", "declare ptr @zeno_slice(ptr, i64, i64, i1 zeroext, i32 signext, i32 signext)"),
    // The `files` module. In a firmware build the four that can fail
    // are declared another way: see `DECLARED_FOR_FIRMWARE`.
    ("zeno_files_exists", "declare zeroext i1 @zeno_files_exists(ptr)"),
    ("zeno_files_read", "declare ptr @zeno_files_read(ptr, i32 signext, i32 signext)"),
    ("zeno_files_write", "declare void @zeno_files_write(ptr, ptr, i32 signext, i32 signext)"),
    ("zeno_files_append", "declare void @zeno_files_append(ptr, ptr, i32 signext, i32 signext)"),
    ("zeno_files_remove", "declare void @zeno_files_remove(ptr, i32 signext, i32 signext)"),
    ("zeno_char_code", "declare i64 @zeno_char_code(ptr, i32 signext, i32 signext)"),
    ("zeno_char", "declare ptr @zeno_char(i64, i32 signext, i32 signext)"),
    ("zeno_list_sum_int", "declare i64 @zeno_list_sum_int(ptr, i32 signext, i32 signext)"),
    ("zeno_list_sum_float", "declare double @zeno_list_sum_float(ptr)"),
    ("zeno_list_best_int", "declare i64 @zeno_list_best_int(ptr, i1 zeroext, ptr, i32 signext, i32 signext)"),
    ("zeno_list_best_float", "declare double @zeno_list_best_float(ptr, i1 zeroext, ptr, i32 signext, i32 signext)"),
    ("zeno_error_same", "declare zeroext i1 @zeno_error_same(ptr, ptr)"),
    ("zeno_error_new", "declare ptr @zeno_error_new(ptr, ptr, ptr)"),
    ("zeno_raise", "declare void @zeno_raise(ptr, i32 signext, i32 signext) noreturn"),
    // A firmware build's three: note an error, ask its kind, and stop.
    ("zeno_note", "declare void @zeno_note(ptr, i32 signext, i32 signext)"),
    ("zeno_is", "declare zeroext i1 @zeno_is(ptr, ptr)"),
    ("zeno_uncaught", "declare void @zeno_uncaught() noreturn"),
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
    /// For text: it was there before this statement began (a literal, or
    /// what an unchanging variable holds), so it will still be there
    /// afterwards. Text that is not `fixed` is in the text room: this
    /// statement built it, or a function gave it back, or it was lent
    /// (see `lend`). Only a build with no collector cares: there, such
    /// text is gone when the statement is over, and is copied to be kept.
    fixed: bool,
    /// For text that was lent: where it was read from. See `keep`.
    lent: Option<String>,
    /// For a list: this very expression made it, and nobody has it yet.
    /// `[1, 2]` does, and a comprehension, `split`, `map`, `filter` and
    /// `+` on two lists.
    made: bool,
    /// The statement being written owns it and frees it when the
    /// statement is over. So it can be looked at, and it cannot be kept.
    /// See `belongs`.
    passing: bool,
}

/// A value that belongs to the statement being written. See
/// `Emitter::belongs`.
struct Passing {
    /// What it is held under, so that leaving early frees it. See `hold`.
    key: String,
    /// The slot that holds its address.
    slot: String,
    kind: Kind,
}

impl Val {
    fn new(code: impl Into<String>, kind: Kind) -> Val {
        Val { code: code.into(), kind, fixed: false, lent: None, made: false, passing: false }
    }

    /// A list that was made right here, and is nobody's yet.
    fn made(mut self) -> Val {
        self.made = true;
        self
    }

    fn fixed(mut self) -> Val {
        self.fixed = true;
        self
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
    /// It owns the list or object it holds, and frees it when its block
    /// ends. An owner that is a variable always has a slot, and the slot
    /// holds null once the value has been given away. See "Owned values".
    owner: bool,
}

/// One of the program's functions or methods, as a call needs to know it.
#[derive(Clone)]
struct Entry {
    decl: Arc<Function>,
    kind: FunctionKind,
    /// Its name in the IR, such as `@z_area`.
    name: String,
    /// For each parameter: does the function take over an owned value?
    owned: Vec<bool>,
    /// For each parameter: is it without `take`? Then the function only
    /// looks, and may be lent a value that is gone when the statement is.
    lends: Vec<bool>,
}

/// What a call can be given: the name, kind and default of each place.
type Slots = Vec<(String, Kind, Option<Expr>)>;

/// A class: an object is a block of memory holding its fields in order.
struct ClassEntry {
    fields: Slots,
    /// For each field, its size if it is a sized whole number such as
    /// `level: u8`. The struct holds it in that many bits (`i8`), and it
    /// is an `i64` everywhere else: see `load_field` and `store_field`.
    sized: Vec<Option<Width>>,
    methods: HashMap<String, Entry>,
    /// The name of its struct type in the IR, such as `%z_Point`.
    ty: String,
    /// For each field: does the object own what the field holds? Only an
    /// object of an `owned class` owns anything.
    owned_fields: Vec<bool>,
}

/// An enum: each case with the names and kinds of the values it carries.
struct EnumEntry {
    cases: Vec<(String, Vec<(String, Kind)>)>,
    /// For each case, which of the values it carries have one owner,
    /// which is the enum value from then on.
    takes: Vec<Vec<bool>>,
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
    /// How many blocks were open when the loop's body began. `break` and
    /// `continue` end the ones opened since, and free what they own.
    scopes: usize,
}

/// A counting loop that has been opened: its counter, and its labels.
struct Count {
    counter: String,
    test: String,
    body: String,
    next: String,
    done: String,
    /// For a loop over a list: the list that is walked, and its kind. It
    /// is the loop's own, and is freed when the loop is over. See
    /// `close_loop`.
    walked: Option<(String, Kind)>,
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
    /// of each one's `zeno_try`. In a firmware build there is no
    /// `zeno_try`, and this holds the label of each one's `except` blocks.
    tries: Vec<String>,
    /// For each of those, in a firmware build: the kinds of error that
    /// can arrive at its `except` blocks.
    arriving: Vec<BTreeSet<String>>,
    /// In a firmware build: this function can raise, so it gives back
    /// `true` or `false` and puts its answer where `%result` points.
    fallible: bool,
    /// For each `try` that is open: how many blocks were open when its
    /// body began. An error raised here ends the ones opened since.
    try_scopes: Vec<usize>,
    /// How many values have been held between two owners. See `hold`.
    holds: usize,
    /// In a build with no collector: the statement being written has
    /// built text, which it must give back when it is over.
    built: bool,
    /// How many times the top of the text room has been noted.
    marks: usize,
    /// The values that belong to the statement being written, which
    /// frees them when it is over. See `belongs`.
    passing: Vec<Passing>,
    /// How many such values there have been, to name their slots.
    owns: usize,
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
            arriving: Vec::new(),
            fallible: false,
            try_scopes: Vec::new(),
            holds: 0,
            built: false,
            marks: 0,
            passing: Vec::new(),
            owns: 0,
            on_stack: HashSet::new(),
            careful: false,
            shared: HashSet::new(),
        }
    }

    /// The same, for a function with these statements in it.
    /// `jumps` says whether errors jump (an ordinary build) or travel as
    /// values (a firmware build). Only a jump needs the care.
    fn of(body: &[Stmt], returns: Kind, in_function: bool, jumps: bool) -> Code {
        let mut code = Code::new(returns, in_function);
        code.careful = jumps && emit_c::has_try(body);
        for statement in body {
            emit_c::names_in_functions(statement, false, &mut code.shared);
        }
        code
    }
}

struct Emitter<'a> {
    types: &'a Types,
    /// Which values have one owner, and so what freeing a list frees.
    /// The C backend's, so that both agree.
    drops: emit_c::Drops,
    /// `zeno build --llvm --firmware`: errors travel as values that each
    /// caller tests, and nothing jumps. See "Errors as values".
    firmware: bool,
    /// There is a garbage collector in the program. `zeno build --llvm
    /// --no-collector` leaves it out: see "With no collector to help".
    collector: bool,
    /// `zeno build --memory`: report the memory used as the program ends.
    memory: bool,
    /// What a `for` loop is about to walk. If that is a list made right
    /// there, as in `for pin in [2, 3, 5]`, the loop frees it.
    walked: Option<usize>,
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

/// The runtime functions that are called differently in a firmware
/// build (`zeno build --llvm --firmware`). There they say whether they
/// worked, and give their answer through one more parameter.
const DECLARED_FOR_FIRMWARE: &[(&str, &str)] = &[
    ("zeno_parse_int", "declare zeroext i1 @zeno_parse_int(ptr, i32 signext, i32 signext, ptr)"),
    ("zeno_parse_float", "declare zeroext i1 @zeno_parse_float(ptr, i32 signext, i32 signext, ptr)"),
    ("zeno_files_read", "declare zeroext i1 @zeno_files_read(ptr, i32 signext, i32 signext, ptr)"),
    ("zeno_files_write", "declare zeroext i1 @zeno_files_write(ptr, ptr, i32 signext, i32 signext)"),
    ("zeno_files_append", "declare zeroext i1 @zeno_files_append(ptr, ptr, i32 signext, i32 signext)"),
    ("zeno_files_remove", "declare zeroext i1 @zeno_files_remove(ptr, i32 signext, i32 signext)"),
];

/// The error for a list that would outlive the statement that owns it:
/// one that is put somewhere that is not an owner.
fn no_owner(span: Span) -> Error {
    emit_c::without_collector("this list has no owner. Give it one, as in `let items: owned list[int] = [1, 2, 3]`", span)
}

/// The error for something a firmware build has to refuse. The words
/// are the C backend's, so that one message means one thing.
fn firmware_cannot(what: impl AsRef<str>, span: Span) -> Error {
    Error::new(format!("{}, or build without `--firmware`", what.as_ref()), span)
}

/// A firmware build calls a function that can raise in a different way
/// from one that cannot, and a function value does not say which it is.
fn raises_as_value(span: Span) -> Error {
    firmware_cannot(
        "`zeno build --firmware` cannot compile a function that can raise, used as a value, yet. Call it by its name",
        span,
    )
}

/// The error for a part of Zeno this stage does not handle yet.
fn cannot(what: impl AsRef<str>, span: Span) -> Error {
    Error::new(
        format!("`zeno build --llvm` cannot compile {} yet. `zeno run` can still run this program", what.as_ref()),
        span,
    )
}

/// The type written as the first argument of `allocate` or `register`,
/// and the argument after it.
fn written_type<'a>(what: &str, args: &'a [Arg], span: Span) -> Result<(Pointee, &'a Expr), Error> {
    match args {
        [Arg { name: None, value: Expr { kind: ExprKind::Name(item), .. } }, Arg { name: None, value }] => {
            match Pointee::named(item) {
                Some(pointee) => Ok((pointee, value)),
                None => Err(cannot(format!("`{what}` of `{item}`"), span)),
            }
        }
        _ => Err(cannot(format!("`{what}` called this way"), span)),
    }
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
        Kind::Pointer(_) => "a pointer".to_string(),
        Kind::Register(_) => "a register".to_string(),
        Kind::None => "a value that is nothing".to_string(),
        Kind::Quantity(unit) => format!("a number with a unit ({unit})"),
        Kind::List(_) => "a list".to_string(),
        Kind::Dict(..) => "a dict".to_string(),
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
    matches!(
        kind,
        Kind::Str
            | Kind::List(_)
            | Kind::Dict(..)
            | Kind::Class(_)
            | Kind::Enum(_)
            | Kind::Error
            | Kind::Function(_)
            | Kind::Pointer(_)
    )
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

/// A kind without the word `owned` on it. An owned value is held exactly
/// as a shared one is. What differs is who frees it, and that is kept
/// beside the kind: see `Variable::owner`.
fn shared(kind: &Kind) -> Kind {
    emit_c::disowned(kind.clone())
}

/// The kind of `[]` before the place it arrives at has said what it holds.
fn is_empty_list(kind: &Kind) -> bool {
    matches!(kind, Kind::List(item) if matches!(**item, Kind::Other(_)))
}

/// The kind of `[:]` before the place it arrives at has said what it holds.
fn is_empty_dict(kind: &Kind) -> bool {
    matches!(kind, Kind::Dict(key, _) if matches!(**key, Kind::Other(_)))
}

/// What a dict is told about its keys and its values when it is made, as
/// the two numbers the runtime takes: the letters 's', 'i', 'f' and 'b'
/// (see `emit_c::dict_kinds`).
fn dict_kinds(key: &Kind, value: &Kind, span: Span) -> Result<String, Error> {
    let (key, value) = emit_c::dict_kinds(key, value, span)?;
    Ok(format!("i32 {}, i32 {}", key as u32, value as u32))
}

/// The one kind that values of kinds `a` and `b` can both be stored as, if
/// there is one: `int` and `none` meet in `int or none`.
fn joined(a: &Kind, b: &Kind) -> Option<Kind> {
    match (a, b) {
        _ if a == b => Some(a.clone()),
        // `[[], [1]]`: the empty list goes along with the other one.
        (empty, Kind::List(_)) if is_empty_list(empty) => Some(b.clone()),
        (Kind::List(_), empty) if is_empty_list(empty) => Some(a.clone()),
        // `[:]` beside a dict that says what it holds.
        (empty, Kind::Dict(..)) if is_empty_dict(empty) => Some(b.clone()),
        (Kind::Dict(..), empty) if is_empty_dict(empty) => Some(a.clone()),
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
        Kind::Pointer(pointee) => format!("pointer_{}", pointee.name()),
        Kind::Register(width) => format!("register_{}", width.name),
        Kind::Str => "str".to_string(),
        Kind::None => "none".to_string(),
        Kind::Quantity(unit) => format!("in_{unit}"),
        Kind::List(item) => format!("list_{}", key(item)),
        Kind::Dict(keys, values) => format!("dict_{}_{}", key(keys), key(values)),
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
        // A pin of the board is its number, and a register its address.
        Kind::Int | Kind::Pin | Kind::Register(_) => "i64",
        // A raw pointer is an address, and nothing more.
        Kind::Pointer(_) => "ptr",
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
        Kind::Str | Kind::List(_) | Kind::Dict(..) | Kind::Class(_) | Kind::Enum(_) | Kind::Error | Kind::Function(_) => {
            "ptr"
        }
        // An address can already say "nothing": it is null. A number or
        // a bool cannot, so it travels as a pair with a flag in front.
        Kind::Optional(inner) => match &**inner {
            inner if is_pointer(inner) => "ptr",
            Kind::Int | Kind::Pin | Kind::Register(_) => "{ i1, i64 }",
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
        Kind::Int | Kind::Pin | Kind::Register(_) => Ok("int"),
        Kind::Float | Kind::Quantity(_) => Ok("float"),
        Kind::Bool => Ok("bool"),
        _ => Err(cannot("this kind of value that may be `none`", span)),
    }
}

/// `line, col` as the last two arguments of a runtime call that can fail.
fn at(span: Span) -> String {
    format!("i32 {}, i32 {}", span.line, span.col)
}

/// The functions to compile for a function written in the program: the
/// function itself, or, if it is generic, each of its copies.
fn copies_of_function<'a>(types: &'a Types, decl: &'a Arc<Function>) -> Vec<&'a Arc<Function>> {
    match decl.type_params.is_empty() {
        true => vec![decl],
        false => types.function_copies(decl).iter().collect(),
    }
}

/// The same for a class.
fn copies_of_class<'a>(types: &'a Types, decl: &'a Class) -> Vec<&'a Class> {
    match decl.type_params.is_empty() {
        true => vec![decl],
        false => types.class_copies(decl).iter().map(|copy| &**copy).collect(),
    }
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
/// error messages. `build` says which of the three kinds of build this
/// is, as for the C backend. `memory` is `zeno build --memory`: the
/// program reports how much memory it used as it ends.
pub fn emit(program: &Program, types: &Types, file: &str, build: Build, memory: bool) -> Result<String, Error> {
    let (firmware, collector) = (build != Build::Ordinary, build != Build::NoCollector);
    let mut emitter = Emitter {
        types,
        drops: emit_c::Drops::of(program, types, collector),
        firmware,
        collector,
        memory,
        walked: None,
        functions: HashMap::new(),
        classes: HashMap::new(),
        enums: HashMap::new(),
        cases: HashMap::new(),
        imported: HashMap::new(),
        waiting: VecDeque::new(),
        scopes: vec![HashMap::new()],
        code: Code::of(&program.statements, Kind::None, false, !firmware),
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
            // A generic function is not compiled. Its copies are, one
            // for each set of types the program calls it with. The
            // checker made them, and each is an ordinary function with a
            // name of its own: `@"z_first[int]"`, `@"z_first[str]"`.
            StmtKind::Function(decl) => {
                for decl in copies_of_function(types, decl) {
                    // `z_` keeps the program's names apart from the
                    // runtime's and from the C library's: a Zeno function
                    // may well be called `main`.
                    let entry = emitter.entry(decl, named('@', "z_", &decl.name), statement.span)?;
                    emitter.functions.insert(decl.name.clone(), entry);
                }
            }
            // The same for a generic class: `%"z_Stack[int]"`.
            StmtKind::Class(decl) => {
                for decl in copies_of_class(types, decl) {
                    emitter.register_class(decl, statement.span)?;
                }
            }
            StmtKind::Enum(decl) => emitter.register_enum(decl, statement.span)?,
            _ => {}
        }
    }

    // The top of the file becomes one function, `zeno_program`. Each
    // function it passes on the way is put aside, and written afterwards.
    for statement in &program.statements {
        emitter.statement(statement)?;
    }
    // The end of the file is the end of the outermost block.
    if !emitter.code.ended {
        emitter.free_owned(0)?;
    }
    // `noinline` keeps it a function of its own. The collector looks for
    // addresses on the stack below `main`, and this is where they are.
    emitter.program = emitter.finish("define internal void @zeno_program() noinline".to_string());
    while let Some(waiting) = emitter.waiting.pop_front() {
        emitter.function(waiting)?;
    }

    Ok(emitter.file(file, &program.modules))
}

impl Emitter<'_> {
    /// Put the finished parts together.
    fn file(mut self, file: &str, modules: &[ModuleFile]) -> String {
        let name = self.string(file);
        // The files the program imports, for an error that stops the
        // program in one of them.
        let modules: Vec<String> = modules.iter().map(|module| format!("ptr {}", self.string(&module.path))).collect();
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
        if !modules.is_empty() {
            out += "@zeno_modules = external global ptr\n";
            out += &format!("@zeno.modules = private constant [{} x ptr] [{}]\n", modules.len(), modules.join(", "));
        }
        out += "@zeno_raised = external global ptr\n";
        if self.collector {
            out += "@zeno_stack_base = external global ptr\n";
            self.used.insert("zeno_mark_words");
        }
        if !self.collector {
            // How much of the text room is in use. See `text_end`.
            out += "@zeno_text_top = external global i64\n";
        }
        if self.memory {
            // `atexit` is C's "call this when the program ends".
            out += "declare i32 @atexit(ptr)\n";
            out += "declare void @zeno_memory_report()\n";
        }
        for (name, declaration) in DECLARED {
            if self.used.contains(name) {
                let for_firmware = DECLARED_FOR_FIRMWARE.iter().find(|(other, _)| self.firmware && other == name);
                out += for_firmware.map_or(declaration, |(_, declaration)| declaration);
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
        if self.collector {
            out += "define void @zeno_roots() {\nentry:\n";
            out += "  call void @zeno_mark_words(ptr @zeno_raised, i64 8)\n";
            for root in &self.roots {
                out += &format!("  call void @zeno_mark_words(ptr {root}, i64 8)\n");
            }
            out += "  ret void\n}\n\n";
        }

        // Where the program starts. `%base` is a variable of `main`, so
        // its address marks where the stack begins, for the collector.
        // `main` takes the command line only in a program that reads it.
        let arguments = self.used.contains("zeno_program_start");
        out += if arguments { "define i32 @main(i32 %argc, ptr %argv) {\nentry:\n" } else { "define i32 @main() {\nentry:\n" };

        if self.collector {
            out += "  %base = alloca ptr\n";
            out += "  store ptr %base, ptr @zeno_stack_base\n";
        }
        if self.memory {
            out += "  %asked = call i32 @atexit(ptr @zeno_memory_report)\n";
        }
        out += &format!("  store ptr {name}, ptr @zeno_file\n");
        if !modules.is_empty() {
            out += "  store ptr @zeno.modules, ptr @zeno_modules\n";
        }
        if arguments {
            out += "  call void @zeno_program_start(i32 signext %argc, ptr %argv)\n";
        }
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

    /// Read field `index` of the object of `class` at `object`.
    ///
    /// A sized field is read at its own width and then made an `i64`,
    /// which is what a whole number is everywhere outside an object.
    /// `zext` ("zero extend") fills the new bits with zeros, right for a
    /// `u8`. `sext` ("sign extend") fills them with copies of the top
    /// bit, so that an `i8` holding -1 (eight ones) becomes sixty-four
    /// ones, which is -1 again.
    fn load_field(&mut self, class: &str, object: &str, index: usize, span: Span) -> Result<String, Error> {
        let (ty, kind) = (self.classes[class].ty.clone(), self.classes[class].fields[index].1.clone());
        let at = self.field_at(&ty, object, index);
        Ok(match self.classes[class].sized[index] {
            Some(width) => {
                let small = self.set(format!("load {}, ptr {at}", width.ir_type()));
                let extend = if width.signed { "sext" } else { "zext" };
                self.set(format!("{extend} {} {small} to i64", width.ir_type()))
            }
            None => self.set(format!("load {}, ptr {at}", ir_type(&kind, span)?)),
        })
    }

    /// Put `value` in field `index` of the object of `class` at `object`.
    /// For a sized field, `trunc` ("truncate") keeps the low bits. The
    /// checker made sure the number is in range, so none are lost.
    fn store_field(&mut self, class: &str, object: &str, index: usize, value: &str, span: Span) -> Result<(), Error> {
        let (ty, kind) = (self.classes[class].ty.clone(), self.classes[class].fields[index].1.clone());
        let at = self.field_at(&ty, object, index);
        match self.classes[class].sized[index] {
            Some(width) => {
                let small = self.set(format!("trunc i64 {value} to {}", width.ir_type()));
                self.ins(format!("store {} {small}, ptr {at}", width.ir_type()));
            }
            None => self.ins(format!("store {} {value}, ptr {at}", ir_type(&kind, span)?)),
        }
        Ok(())
    }

    /// Finish the function being written and return its text.
    fn finish(&mut self, first_line: String) -> String {
        if !self.code.ended && self.code.fallible {
            // The end of a function that can raise, reached with no
            // `return`: it worked.
            match (self.code.returns.clone(), ir_type(&self.code.returns, Span { line: 0, col: 0 })) {
                (Kind::None, _) => self.end("ret i1 true"),
                (Kind::Optional(_), Ok(ty)) => {
                    self.ins(format!("store {ty} {}, ptr %result", zero(ty)));
                    self.end("ret i1 true");
                }
                _ => self.end("unreachable"),
            }
        }
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
        let owned = kind.params.iter().map(|param| matches!(param, Kind::Owned(_))).collect();
        let kind = FunctionKind { params: kind.params.iter().map(shared).collect(), returns: shared(&kind.returns) };
        for param in &kind.params {
            ir_type(param, span)?;
        }
        return_type(&kind.returns, span)?;
        let lends = self.types.looks(decl);
        Ok(Entry { decl: decl.clone(), kind, name, owned, lends })
    }

    /// A class becomes a struct type: its fields, in the order they are
    /// listed. `%z_Point = type { double, double }`.
    fn register_class(&mut self, decl: &Class, span: Span) -> Result<(), Error> {
        let Some(kinds) = self.types.class(decl) else {
            return Err(cannot(format!("the class `{}`", decl.name), span));
        };
        let owned_fields = kinds.iter().map(|kind| self.types.owned_class(decl) && matches!(kind, Kind::Owned(_))).collect();
        let kinds: Vec<Kind> = kinds.iter().map(shared).collect();
        let ty = named('%', "z_", &decl.name);
        let mut sized = self.types.widths(decl).cloned().unwrap_or_default();
        sized.resize(kinds.len(), None);
        let mut inside = Vec::new();
        for (kind, width) in kinds.iter().zip(&sized) {
            // `%z_Packet = type { i8, i16, ptr }`: a u8 and a u16 take
            // three bytes between them and not sixteen.
            inside.push(match width {
                Some(width) => width.ir_type(),
                None => ir_type(kind, span)?,
            });
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
        self.classes.insert(decl.name.clone(), ClassEntry { fields, sized, methods, ty, owned_fields });
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
        let mut takes = Vec::new();
        for (index, (case, case_kinds)) in decl.variants.iter().zip(&kinds).enumerate() {
            takes.push(case_kinds.iter().map(|kind| matches!(kind, Kind::Owned(_))).collect());
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
        self.enums.insert(decl.name.clone(), EnumEntry { cases, takes });
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
            params.insert(param.name.clone(), Variable { place: Place::Value(arrives), kind: kind.clone(), open: false, owner: false });
        }
        Ok((params, list))
    }

    /// Write out one function or method.
    fn function(&mut self, waiting: Waiting) -> Result<(), Error> {
        let Waiting { entry, span, sees } = waiting;
        self.code = Code::of(&entry.decl.body, entry.kind.returns.clone(), true, !self.firmware);
        // In a firmware build a function that can raise gives back `true`
        // or `false`, and its answer goes out through `%result`.
        self.code.fallible = self.firmware && !entry.decl.raises.is_empty();
        let fallible = self.code.fallible;
        let (mut params, mut list) = self.parameters(&entry.decl, &entry.kind, span)?;
        // A parameter that takes over an owned value is an owner like any
        // other: the function frees the value when it ends, unless it has
        // passed it on by then. So it is moved into a slot, where giving
        // it away can leave null behind.
        for (index, param) in entry.decl.params.iter().enumerate() {
            if entry.owned.get(index) == Some(&true) {
                let kind = entry.kind.params[index].clone();
                let ty = ir_type(&kind, span)?;
                let slot = self.stack_slot(&param.name, ty);
                self.store(ty, &named('%', "arg.", &param.name), &slot);
                params.insert(param.name.clone(), Variable { place: Place::Slot(slot), kind, open: false, owner: true });
            }
        }
        let outer = std::mem::replace(&mut self.scopes, vec![sees, params]);
        let mut done = self.block(&entry.decl.body);
        if done.is_ok() && !self.code.ended {
            // The end of the body: free what the parameters still own.
            done = self.free_owned(1);
        }
        self.scopes = outer;
        done?;

        // `internal` is what `static` is in C: nothing outside this file
        // needs the function, so LLVM may inline it and drop it.
        let mut returns = return_type(&entry.kind.returns, span)?;
        if fallible {
            if entry.kind.returns != Kind::None {
                list.push("ptr %result".to_string());
            }
            returns = "zeroext i1";
        }
        let text = self.finish(format!("define internal {returns} {}({})", entry.name, list.join(", ")));
        self.definitions.push(text);
        Ok(())
    }

    /// The Zeno type written in the source, as a kind.
    fn kind_of_type(&self, ty: &Type) -> Option<Kind> {
        // Inside a copy of a generic, `T` is whatever the copy was made
        // for, and `Stack[int]` is a copy itself. Only the checker knows
        // those, so its answer is used where it has one.
        if let Some(kind) = self.types.annotation(ty) {
            return Some(emit_c::disowned(kind.clone()));
        }
        let (name, args) = match ty {
            Type::Named { name, args } => (name, args),
            // Who owns a value does not change how it is held.
            Type::Owned(inner) | Type::Take(inner) => return self.kind_of_type(inner),
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
            ("dict", [key, value]) => {
                Kind::Dict(Box::new(self.kind_of_type(key)?), Box::new(self.kind_of_type(value)?))
            }
            (name, []) if self.classes.contains_key(name) => Kind::Class(name.to_string()),
            (name, []) if self.enums.contains_key(name) => Kind::Enum(name.to_string()),
            (name, []) if units::is_unit(name) => Kind::Quantity(name.to_string()),
            ("Pin", []) => Kind::Pin,
            ("pointer", [Type::Named { name, args }]) if args.is_empty() => Kind::Pointer(Pointee::named(name)?),
            ("Register", [Type::Named { name, args }]) if args.is_empty() => Kind::Register(ints::width(name)?),
            // `u8` and the rest: an int that is known to be small.
            (name, []) if ints::width(name).is_some() => Kind::Int,
            _ => return None,
        })
    }

    // ----------------------------------------------------------- Statements

    fn block(&mut self, statements: &[Stmt]) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        let mut done = statements.iter().try_for_each(|statement| self.statement(statement));
        // The block has reached its end. Unless its last statement left
        // it (and freed on its way out), free what its owners hold.
        if done.is_ok() && !self.code.ended {
            done = self.free_owned(self.scopes.len() - 1);
        }
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
        // The statements that work out one value and are done. In a build
        // with no collector, text they build is given back as they end.
        // (The others give it back at a point of their own choosing.)
        let simple =
            matches!(statement.kind, StmtKind::Let { .. } | StmtKind::Const { .. } | StmtKind::Assign { .. } | StmtKind::Expr(_));
        let begin = (!self.collector && simple).then(|| self.text_begin());
        // What the statement makes and gives to nobody is freed as it
        // ends, in every build (see `belongs`). A `for` or a `match` keeps
        // such a value until its last block is over.
        let own = self.own_begin();
        let mut result = self.one_statement(statement);
        if result.is_ok() {
            result = self.own_end(own, !matches!(statement.kind, StmtKind::Return(_) | StmtKind::Raise(_)));
        }
        if let Some(mark) = begin.and_then(|begin| self.text_end(begin)) {
            self.text_back(&mark);
        }
        result
    }

    fn one_statement(&mut self, statement: &Stmt) -> Result<(), Error> {
        let span = statement.span;
        self.open();
        match &statement.kind {
            StmtKind::Let { name, ty, value, .. } | StmtKind::Const { name, ty, value } => {
                let mutable = matches!(statement.kind, StmtKind::Let { mutable: true, .. });
                // A type written beside the name settles what `none` and
                // `[]` mean here.
                let expected = ty.as_ref().and_then(|ty| self.kind_of_type(ty));
                let mut val = self.expr_as(value, expected.as_ref())?;
                self.stays(&val, value.span)?;
                val = self.keep(val, is_written(value));
                if let Some(expected) = &expected {
                    val = self.coerce(val, expected, value.span)?;
                }
                if is_empty_list(&val.kind) {
                    return Err(cannot(
                        "an empty list with no type beside it. Write `let items: list[int] = []`",
                        value.span,
                    ));
                }
                if is_empty_dict(&val.kind) {
                    return Err(cannot(
                        "an empty dict with no type beside it. Write `let ages: dict[str, int] = [:]`",
                        value.span,
                    ));
                }
                // The checker knows whether this variable owns its value.
                // With no collector a variable has its own copy of its
                // text, and frees it. Text written in the program is not a
                // copy, but a `var` may be given one later.
                let owner = self.types.owner(statement)
                    || (!self.collector && is_text(&val.kind) && (mutable || !is_written(value)));
                self.declare(name, val, mutable, owner, value.span)
            }
            StmtKind::Assign { target, op, value } => self.assign(target, *op, value, span),
            StmtKind::Expr(expr) => {
                let val = self.expr(expr)?;
                if self.types.discarded(expr) {
                    // An owned value that nobody keeps is freed on the spot.
                    self.free(&val.kind, &val.code, span)?;
                }
                Ok(())
            }

            StmtKind::Function(decl) => {
                if !self.at_the_top() {
                    return Err(cannot("a function with a name inside a function or a block", span));
                }
                // It is written out later. It may use the variables
                // declared so far and no others, so a copy of them goes
                // with it. A generic is written out once for each copy.
                for decl in copies_of_function(self.types, decl) {
                    let entry = self.functions[&decl.name].clone();
                    self.waiting.push_back(Waiting { entry, span, sees: self.scopes[0].clone() });
                }
                Ok(())
            }
            StmtKind::Class(decl) => {
                if !self.at_the_top() {
                    return Err(cannot("a class inside a function or a block", span));
                }
                // Its methods are functions, and wait like functions.
                for decl in copies_of_class(self.types, decl) {
                    for method in &decl.methods {
                        let entry = self.classes[&decl.name].methods[&method.name].clone();
                        self.waiting.push_back(Waiting { entry, span, sees: self.scopes[0].clone() });
                    }
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

            // The checker has seen to what is written inside. In IR it
            // is a block like any other.
            StmtKind::Unsafe(body) => {
                let n = self.number();
                let (inside, after) = (format!("unsafe{n}"), format!("safe{n}"));
                self.place(&inside);
                self.block(body)?;
                self.place(&after);
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
                self.code.loops.push(Loop {
                    next: test.clone(),
                    done: done.clone(),
                    tries: self.code.tries.len(),
                    scopes: self.scopes.len(),
                });
                let result = self.block(body);
                self.code.loops.pop();
                result?;
                self.jump(&test);
                self.place(&done);
                Ok(())
            }

            StmtKind::For { variable, iterable, body } => {
                // Text built to find what to loop over is given back
                // before the first trip: see `open_loop`.
                let begin = self.text_begin();
                let count = self.open_loop(variable, iterable, Some(begin))?;
                self.code.loops.push(Loop {
                    next: count.next.clone(),
                    done: count.done.clone(),
                    tries: self.code.tries.len(),
                    scopes: self.scopes.len(),
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
                        let begin = self.text_begin();
                        let val = self.expr_as(value, Some(&returns))?;
                        self.stays(&val, value.span)?;
                        let mut val = self.fit(val, &returns, span)?;
                        // With no collector, text that a function gives
                        // back travels in the text room (docs/text.md):
                        // the runtime moves it down to where the room's
                        // top was when this line began.
                        let text = !self.collector && is_text(&returns) && !is_written(value);
                        self.code.built |= text;
                        match self.text_end(begin) {
                            Some(mark) if text => {
                                val.code = self.call("ptr", "zeno_text_return", format!("i64 {mark}, ptr {}", val.code));
                            }
                            // The answer has been taken. The text it was
                            // worked out from can be given back.
                            Some(mark) => self.text_back(&mark),
                            None => {}
                        }
                        // The value is worked out while any `try` around
                        // this line is still watching, and before anything
                        // is freed: it may have been read out of what is
                        // freed next. Every block of the function ends
                        // here. Then the `try` is left, and then the
                        // function.
                        self.free_owned(self.base())?;
                        self.leave_tries(0);
                        let ty = ir_type(&returns, span)?;
                        if self.code.fallible {
                            // The answer goes out through the pointer,
                            // and `true` says that it is there.
                            self.ins(format!("store {ty} {}, ptr %result", val.code));
                            self.end("ret i1 true");
                        } else {
                            self.end(format!("ret {ty} {}", val.code));
                        }
                    }
                    Some(value) if !matches!(value.kind, ExprKind::None) => {
                        return Err(cannot("returning a value from a function that returns nothing", span))
                    }
                    _ => {
                        self.free_owned(self.base())?;
                        self.leave_tries(0);
                        self.end(if self.code.fallible { "ret i1 true" } else { "ret void" });
                    }
                }
                Ok(())
            }
            StmtKind::Break | StmtKind::Continue => {
                let Some(Loop { next, done, tries, scopes }) = self.code.loops.last() else {
                    return Err(cannot("`break` or `continue` outside a loop", span));
                };
                let to = if statement.kind == StmtKind::Break { done.clone() } else { next.clone() };
                let (tries, scopes) = (*tries, *scopes);
                // The blocks opened since the loop's body began end here.
                self.free_owned(scopes)?;
                self.leave_tries(tries);
                self.jump(&to);
                Ok(())
            }

            StmtKind::Import { module, names } => {
                let module = module.join(".");
                if !matches!(module.as_str(), "math" | "time" | "board" | "files" | "program") {
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
    fn declare(&mut self, name: &str, val: Val, mutable: bool, owner: bool, span: Span) -> Result<(), Error> {
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
        } else if mutable && self.code.shared.contains(name) && self.collector {
            // A function written inside this one uses the variable too,
            // and either may change it, perhaps long after this function
            // has returned. So it lives in a box of its own that the
            // collector looks after, and both hold the box's address.
            let boxed = self.call("ptr", "zeno_alloc", format!("i64 {}", size_of(ty)));
            self.ins(format!("store {ty} {}, ptr {boxed}", val.code));
            (Place::Slot(boxed), true)
        } else if mutable || owner {
            // A `var` gets a slot on the stack. So does an owner, even one
            // declared with `let`: giving its value away writes null into
            // the slot, and the end of its block frees whatever the slot
            // holds by then.
            let slot = self.stack_slot(name, ty);
            self.store(ty, &val.code, &slot);
            (Place::Slot(slot), false)
        } else {
            // A `let` never changes: the value itself is the variable.
            (Place::Value(val.code), false)
        };
        let variable = Variable { place, kind: val.kind, open, owner };
        self.scopes.last_mut().expect("a scope").insert(name.to_string(), variable);
        Ok(())
    }

    /// A slot on the stack for the variable `name`. Two variables of one
    /// name in one function get two slots: `%var.total`, `%var.total.2`.
    fn stack_slot(&mut self, name: &str, ty: &str) -> String {
        let count = self.code.named.entry(name.to_string()).or_insert(0);
        *count += 1;
        let slot = match *count {
            1 => named('%', "var.", name),
            count => named('%', "var.", &format!("{name}.{count}")),
        };
        self.slot(slot, ty)
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
        self.stays(&new, value.span)?;
        let new = self.keep(new, op.is_none() && is_written(value));
        // With no collector, a list that was just made (`xs += [4]` makes
        // one) has to arrive at an owner, which frees the list it had.
        let needs_owner = new.made && !self.collector;
        match &target.kind {
            ExprKind::Name(name) => {
                let Some(Variable { place: Place::Slot(slot), kind, owner, .. }) = self.lookup(name).cloned() else {
                    return Err(cannot(format!("changing `{name}`"), target.span));
                };
                if needs_owner && !owner {
                    return Err(no_owner(value.span));
                }
                let new = self.fit(new, &kind, span)?;
                let ty = ir_type(&kind, span)?;
                if owner {
                    // The new value was made first, since making it may
                    // read the old one. Now the old one is freed.
                    let old = self.load(ty, &slot);
                    self.free(&kind, &old, span)?;
                }
                self.store(ty, &new.code, &slot);
            }
            ExprKind::Field { object, name } => {
                let object = self.expr(object)?;
                let field = match &object.kind {
                    Kind::Class(class) => self.field_of(class, name).map(|field| (class.clone(), field)),
                    _ => None,
                };
                let Some((class, (_, index, kind))) = field else {
                    return Err(cannot("an assignment to this", target.span));
                };
                let new = self.fit(new, &kind, span)?;
                let owned = self.classes[&class].owned_fields[index] || (!self.collector && is_text(&kind));
                if needs_owner && !owned {
                    return Err(no_owner(value.span));
                }
                if owned {
                    // As for an owner variable: the new value is made,
                    // and then the one the field held is freed.
                    let old = self.load_field(&class, &object.code, index, span)?;
                    self.free(&kind, &old, span)?;
                }
                self.store_field(&class, &object.code, index, &new.code, span)?;
            }
            ExprKind::Index { object, index } => {
                let object = self.expr(object)?;
                let index = self.expr(index)?;
                // Through a pointer: the item is stored at its own size,
                // and nothing is checked. That is what `unsafe` is for.
                if let Kind::Pointer(pointee) = object.kind {
                    let new = self.fit(new, &pointee.kind(), span)?;
                    let item = pointee.ir_type();
                    let place = self.set(format!("getelementptr {item}, ptr {}, i64 {}", object.code, index.code));
                    let value = match pointee {
                        Pointee::Sized(_) => self.set(format!("trunc i64 {} to {item}", new.code)),
                        _ => new.code.clone(),
                    };
                    self.ins(format!("store {item} {value}, ptr {place}"));
                    return Ok(());
                }
                // `ages["Ada"] = 36`. With no collector the value is
                // already a copy that the dict can call its own (`keep`,
                // above), and the dict frees the one it had.
                if let Kind::Dict(_, item) = &object.kind {
                    let new = self.fit(new, item, span)?;
                    let key = self.to_slot(&index, span)?;
                    let slot = self.to_slot(&new, span)?;
                    self.run("zeno_dict_set", format!("ptr {}, i64 {key}, i64 {slot}", object.code));
                    return Ok(());
                }
                let Kind::List(item) = object.kind.clone() else {
                    return Err(cannot("an assignment to this", target.span));
                };
                // The list owns an item that has one owner. Any other
                // list has no use for one that was just made.
                let owns = self.drops.owns(&item);
                if needs_owner && !owns {
                    return Err(no_owner(value.span));
                }
                let new = self.fit(new, &item, span)?;
                self.fits_a_slot(&new.kind, span)?;
                let slot = self.to_slot(&new, span)?;
                // The runtime checks the index and gives back the address
                // of that item's slot.
                let args = format!("ptr {}, i64 {}, {}", object.code, index.code, at(span));
                let place = self.call("ptr", "zeno_list_at", args);
                if owns {
                    // The item that was there is freed, as a field's old
                    // value is, and the new one takes its place.
                    let old = self.set(format!("load ptr, ptr {place}"));
                    self.free(&item, &old, span)?;
                } else if !self.collector && is_text(&new.kind) {
                    // The list is done with the text it held there.
                    let old = self.set(format!("load ptr, ptr {place}"));
                    self.run("zeno_text_drop", format!("ptr {old}"));
                }
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

    // ------------------------------------------- With no collector to help
    //
    // `zeno build --llvm --no-collector` (docs/no-collector.md). Three
    // things need care that a collector would otherwise take: a value
    // that belongs to nobody for a moment (`hold`, further down), text
    // built while the program runs, and values that must not be kept.
    // What such a build refuses is refused where each kind of value is
    // made, in the C backend's words: search for `without_collector`.

    /// A statement is about to be written. In a build with no collector,
    /// text that the statement builds is written into the text room (see
    /// runtime.c) and stays there until the statement is over. Call
    /// `text_end` with what this returns when it is.
    fn text_begin(&mut self) -> (usize, bool) {
        self.open();
        (self.code.lines.len(), std::mem::replace(&mut self.code.built, false))
    }

    /// The statement begun at `text_begin` has been written. If it built
    /// text, note where the text room's top was before it:
    ///
    /// ```text
    ///   %mark.1 = load i64, ptr @zeno_text_top
    /// ```
    ///
    /// and return that name, for `text_back` to put the top back there
    /// once nothing needs the text any more. The line is written where
    /// the statement began, which is known only now: most statements
    /// build no text, and get no such line.
    fn text_end(&mut self, (start, outer): (usize, bool)) -> Option<String> {
        if !std::mem::replace(&mut self.code.built, outer) {
            return None;
        }
        self.code.marks += 1;
        let mark = format!("%mark.{}", self.code.marks);
        self.code.lines.insert(start, format!("  {mark} = load i64, ptr @zeno_text_top"));
        Some(mark)
    }

    /// Give back the text built since `mark` was noted. That is one
    /// `store`: there is nothing to search and nothing to fragment.
    fn text_back(&mut self, mark: &str) {
        self.ins(format!("store i64 {mark}, ptr @zeno_text_top"));
    }

    // ------------------------------------------------------------ Kept text
    //
    // In a build with no collector, text is kept by copy (docs/text.md).
    // Whatever stores text (a variable, a field, a list) has a copy of
    // its own in the pool and frees it when it is done. Text is never
    // changed once made, so a copy cannot be told from the original, and
    // the program means what it means in every other build.

    /// `val` is about to be stored. If it is text, store a copy that the
    /// place can call its own. `written` says the text is written in the
    /// program, which needs no copy.
    ///
    /// ```text
    ///   %t4 = call ptr @zeno_text_keep(ptr %t3)
    /// ```
    fn keep(&mut self, mut val: Val, written: bool) -> Val {
        if self.collector || !is_text(&val.kind) || written {
            return val;
        }
        let mut from = val.code.clone();
        if let Some(place) = val.lent.take() {
            // Text that was lent a moment ago, with nothing done since,
            // is copied straight from where it is kept: the statement's
            // own copy is not needed, and is not made.
            let lend = format!("  {} = call ptr @zeno_text_lend(ptr {place})", val.code);
            if self.code.lines.last() == Some(&lend) {
                self.code.lines.pop();
                from = place;
            }
        }
        val.code = self.call("ptr", "zeno_text_keep", format!("ptr {from}"));
        val
    }

    /// `val` was read from a place that a function could change while
    /// this statement is still using what it read: a variable at the top
    /// of the file, a field, an item of a list. If it is text, the
    /// statement gets a copy of its own, in the text room.
    fn lend(&mut self, val: Val) -> Val {
        if self.collector || !is_text(&val.kind) {
            return val.fixed();
        }
        let code = self.call("ptr", "zeno_text_lend", format!("ptr {}", val.code));
        Val { code, kind: val.kind, fixed: false, lent: Some(val.code), made: false, passing: false }
    }

    /// `val` is about to be kept: put in a variable, a field or a list,
    /// given to a function, or returned. In a build with no collector a
    /// value that the statement owns cannot be (see `belongs`). (Text
    /// can: see `keep`. An error can: whoever keeps it owns it.)
    fn stays(&self, val: &Val, span: Span) -> Result<(), Error> {
        if !self.collector && val.passing {
            // It is freed when this statement is over.
            return Err(no_owner(span));
        }
        Ok(())
    }

    // ------------------------------------------------ What a statement owns
    //
    // A value that is made in the middle of a statement and given to
    // nobody belongs to the statement (docs/temporaries.md):
    //
    //     print(len(text.split()))      the list of words
    //     print(size(make()))           the owned value `make` gives
    //
    // The statement keeps its address in a slot and frees it when it is
    // over:
    //
    //   store ptr null, ptr %own.1               ; where the statement begins
    //   %t1 = call ptr @zeno_split_blanks(ptr %t0)
    //   store ptr %t1, ptr %own.1
    //   ...
    //   %t5 = load ptr, ptr %own.1               ; where it ends
    //   call void @zeno_list_drop_text(ptr %t5)
    //
    // A slot, and not simply `%t1`, because the value may be made in a
    // block that `and` or `or` skips, and a name from such a block may
    // not be used after it. The slot holds null until the value is made,
    // and freeing null does nothing.

    /// A statement begins, or a part of one that is over before the
    /// statement is: a condition, one item of a comprehension.
    fn own_begin(&mut self) -> (usize, usize) {
        self.open();
        (self.code.lines.len(), self.code.passing.len())
    }

    /// It is over. Write the lines that empty the slots, where it began.
    /// If `free`, free the values here. A statement that leaves
    /// (`return`, `raise`) has freed them on its way out, as it frees
    /// what the owners hold, and passes `false`.
    fn own_end(&mut self, (start, from): (usize, usize), free: bool) -> Result<(), Error> {
        let ended: Vec<Passing> = self.code.passing.drain(from..).collect();
        for (position, passing) in ended.iter().enumerate() {
            let how = self.how(&passing.slot);
            self.code.lines.insert(start + position, format!("  store{how} ptr null, ptr {}", passing.slot));
        }
        // The last one made may be looking at an earlier one.
        for passing in ended.iter().rev() {
            if free {
                let value = self.load("ptr", &passing.slot);
                self.free(&passing.kind, &value, Span { line: 0, col: 0 })?;
            }
            self.let_go(&passing.key);
        }
        Ok(())
    }

    /// `val` is what `expr` gave. If it was made right here and nobody is
    /// getting it, the statement owns it from now on.
    ///
    /// Two kinds of value are in that position. An owned value that is
    /// only looked at, which the checker has noted (`Types::lent`), in
    /// every build. And, in a build with no collector, a list that was
    /// just made and does not start life owned: every other build leaves
    /// such a list to the collector.
    fn belongs(&mut self, mut val: Val, expr: &Expr) -> Val {
        let lent = self.types.lent(expr);
        if self.walked == Some(expr as *const Expr as usize) {
            // The `for` loop that walks it is the one that frees it.
            val.made |= lent;
            return val;
        }
        let nobodys = val.made && !self.collector && !self.types.fresh(expr);
        if !lent && !nobodys {
            return val;
        }
        self.code.owns += 1;
        let slot = self.slot(format!("%own.{}", self.code.owns), "ptr");
        self.store("ptr", &val.code, &slot);
        // Held in its slot, as an owner is: leaving early frees it.
        self.code.holds += 1;
        let key = format!("held {}", self.code.holds);
        let variable = Variable { place: Place::Slot(slot.clone()), kind: val.kind.clone(), open: false, owner: true };
        self.scopes.last_mut().expect("a scope").insert(key.clone(), variable);
        self.code.passing.push(Passing { key, slot, kind: val.kind.clone() });
        val.made = false;
        val.passing = true;
        val
    }

    /// Each item of a list sits in one slot. A number that may be `none`
    /// does not fit, and is kept in a small block of its own, which
    /// nothing would free.
    fn fits_a_slot(&self, item: &Kind, span: Span) -> Result<(), Error> {
        match item {
            Kind::Optional(inner) if !self.collector && !is_pointer(inner) => Err(emit_c::none_in_a_slot(span)),
            _ => Ok(()),
        }
    }

    // --------------------------------------------------------- Owned values
    //
    // An owned value is freed by its owner, at the end of the owner's
    // block (docs/ownership.md). In the IR an owner is a slot that holds
    // an address, with one extra rule: when its value is given away, null
    // is stored in the slot. So "free whatever the owners still hold" is
    // one load and one call for each owner, and the call does nothing for
    // null. The checker has already made sure a variable is never read
    // after it gave its value away.

    /// The first block that belongs to the function being written. The
    /// one before it, in a function, is the top of the file.
    fn base(&self) -> usize {
        self.code.in_function as usize
    }

    /// Free the owned value at `value`, a list or an object, if there is
    /// one there.
    fn free(&mut self, kind: &Kind, value: &str, span: Span) -> Result<(), Error> {
        match kind {
            Kind::Class(class) => {
                let drop = self.drop_class(class, span)?;
                self.ins(format!("call void {drop}(ptr {value})"));
            }
            Kind::Enum(owner) => {
                let drop = self.drop_enum(owner, span)?;
                self.ins(format!("call void {drop}(ptr {value})"));
            }
            // What may be `none` is a pointer that may be null, and every
            // function that frees does nothing with null.
            Kind::Optional(inner) if is_pointer(inner) => return self.free(inner, value, span),
            // An error that an `except` caught, in a build with no collector.
            Kind::Error => self.run("zeno_error_drop", format!("ptr {value}")),
            // A function that was written where a value goes frees the
            // block it is kept in. A function of the program has none,
            // and is left alone.
            Kind::Function(_) => self.run("zeno_closure_drop", format!("ptr {value}")),
            // Text that is kept by copy, and a list of such text, which
            // frees its items as well as itself.
            kind if is_text(kind) => self.run("zeno_text_drop", format!("ptr {value}")),
            // A dict frees its two lists, its index and, with no
            // collector, its copies of any text.
            Kind::Dict(..) => self.run("zeno_dict_drop", format!("ptr {value}")),
            Kind::List(item) if !self.collector && is_text(item) => {
                self.run("zeno_list_drop_text", format!("ptr {value}"))
            }
            // A list of items that have one owner each, the list: its
            // end is theirs.
            Kind::List(item) if self.drops.owns(item) => {
                let drop = self.drop_list(item, span)?;
                self.ins(format!("call void {drop}(ptr {value})"));
            }
            _ => self.run("zeno_list_drop", format!("ptr {value}")),
        }
        Ok(())
    }

    /// The function that ends a list together with its items, for a
    /// list whose items have one owner each. Null means the list was
    /// given away, and there is nothing to do.
    ///
    /// ```text
    /// define internal void @zeno_list_drop_1_Point(ptr %list) {
    /// entry:
    ///   %t1 = icmp eq ptr %list, null
    ///   br i1 %t1, label %gone1, label %there1
    /// gone1:
    ///   ret void
    /// there1:
    ///   ...for each item:
    ///   call void @zeno.drop.Point(ptr %t9)
    ///   ...then the list itself:
    ///   call void @zeno_list_drop(ptr %list)
    ///   ret void
    /// }
    /// ```
    fn drop_list(&mut self, item: &Kind, span: Span) -> Result<String, Error> {
        // The C backend's name for it. No class can be given a name
        // that makes the same one.
        let name = format!("@{}", self.drops.name(&Kind::List(Box::new(item.clone()))));
        // Noted as written before it is, as for a class.
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let item = item.clone();
        self.side(format!("define internal void {name}(ptr %list)"), |emitter| {
            let gone = emitter.set("icmp eq ptr %list, null");
            let n = emitter.number();
            let (nothing, there) = (format!("gone{n}"), format!("there{n}"));
            emitter.jump_if(&gone, &nothing, &there);
            emitter.place(&nothing);
            emitter.end("ret void");
            emitter.place(&there);
            let (count, slot) = emitter.walk_open("item", "%list");
            let each = emitter.from_slot(&item, &slot, span)?;
            emitter.free(&item, &each.code, span)?;
            emitter.count_close(&count);
            emitter.run("zeno_list_drop", "ptr %list");
            emitter.end("ret void");
            Ok(())
        })?;
        Ok(name)
    }

    /// Free what the owners in `scopes[from..]` still hold, innermost
    /// block first: a block is ending, or being left early.
    ///
    /// ```text
    ///   %t9 = load ptr, ptr %var.frame
    ///   call void @zeno.drop.Frame(ptr %t9)
    /// ```
    fn free_owned(&mut self, from: usize) -> Result<(), Error> {
        let mut owners = Vec::new();
        for scope in self.scopes.iter().skip(from).rev() {
            let mut here: Vec<(String, bool, Kind)> = scope
                .values()
                .filter(|variable| variable.owner)
                .map(|variable| match &variable.place {
                    Place::Slot(slot) => (slot.clone(), true, variable.kind.clone()),
                    Place::Value(code) => (code.clone(), false, variable.kind.clone()),
                })
                .collect();
            // In the same order on every run of the compiler.
            here.sort_by(|a, b| a.0.cmp(&b.0));
            owners.extend(here);
        }
        for (place, is_slot, kind) in owners {
            let value = if is_slot { self.load("ptr", &place) } else { place };
            self.free(&kind, &value, Span { line: 0, col: 0 })?;
        }
        Ok(())
    }

    /// `value` is an owned value that has no owner just now: it is on its
    /// way into a call, or it is a list still being filled, or the list a
    /// `for` loop walks. Until `let_go`, whatever leaves the block early
    /// (a `return`, a failed call) frees it, as it frees what the block's
    /// owners hold.
    fn hold(&mut self, kind: Kind, value: &str) -> String {
        self.code.holds += 1;
        // No Zeno name has a space in it, so this cannot hide a variable.
        let key = format!("held {}", self.code.holds);
        let variable = Variable { place: Place::Value(value.to_string()), kind, open: false, owner: true };
        self.scopes.last_mut().expect("a scope").insert(key.clone(), variable);
        key
    }

    /// The value has arrived where it was going, and has an owner again.
    fn let_go(&mut self, key: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.remove(key).is_some() {
                return;
            }
        }
    }

    /// The function that ends an object of an owned class: free what its
    /// fields own, then the object. Null means the object was given
    /// away, and there is nothing to do.
    ///
    /// ```text
    /// define internal void @zeno.drop.Frame(ptr %self) {
    /// entry:
    ///   %t1 = icmp eq ptr %self, null
    ///   br i1 %t1, label %gone1, label %there1
    /// gone1:
    ///   ret void
    /// there1:
    ///   %t2 = getelementptr %z_Frame, ptr %self, i32 0, i32 1
    ///   %t3 = load ptr, ptr %t2
    ///   call void @zeno_list_drop(ptr %t3)
    ///   call void @zeno_free(ptr %self)
    ///   ret void
    /// }
    /// ```
    fn drop_class(&mut self, class: &str, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.drop.", class);
        // Noted as written before it is: an object may own another of its
        // own class, and freeing that one calls this same function.
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        // With no collector an object has its own copies of the text in
        // its fields, and those end with it.
        let texts = !self.collector;
        let entry = &self.classes[class];
        let owned: Vec<(usize, Kind)> = entry
            .fields
            .iter()
            .enumerate()
            .filter(|(index, (_, kind, _))| entry.owned_fields[*index] || (texts && is_text(kind)))
            .map(|(index, (_, kind, _))| (index, kind.clone()))
            .collect();
        self.side(format!("define internal void {name}(ptr %self)"), |emitter| {
            let gone = emitter.set("icmp eq ptr %self, null");
            let n = emitter.number();
            let (nothing, there) = (format!("gone{n}"), format!("there{n}"));
            emitter.jump_if(&gone, &nothing, &there);
            emitter.place(&nothing);
            emitter.end("ret void");
            emitter.place(&there);
            for (index, kind) in &owned {
                let held = emitter.load_field(class, "%self", *index, span)?;
                emitter.free(kind, &held, span)?;
            }
            emitter.run("zeno_free", "ptr %self");
            emitter.end("ret void");
            Ok(())
        })?;
        Ok(name)
    }

    /// The function that ends a value of an enum whose values have one
    /// owner each. A case that carries nothing is one fixed value and is
    /// never freed. A case that carries something is a block: free what
    /// it carries that has an owner, and the text it keeps, then the
    /// block.
    fn drop_enum(&mut self, owner: &str, span: Span) -> Result<String, Error> {
        let name = named('@', "zeno.drop.", owner);
        // Noted as written before it is: a value may carry another of
        // its own enum.
        if !self.written.insert(name.clone()) {
            return Ok(name);
        }
        let texts = !self.collector;
        let cases = self.enums[owner].cases.clone();
        let takes = self.enums[owner].takes.clone();
        self.side(format!("define internal void {name}(ptr %self)"), |emitter| {
            let gone = emitter.set("icmp eq ptr %self, null");
            let n = emitter.number();
            let (nothing, there) = (format!("gone{n}"), format!("there{n}"));
            emitter.jump_if(&gone, &nothing, &there);
            emitter.place(&nothing);
            emitter.end("ret void");
            emitter.place(&there);
            let tag = emitter.set("load i64, ptr %self");
            let ways: Vec<String> = cases
                .iter()
                .enumerate()
                .filter(|(_, (_, fields))| !fields.is_empty())
                .map(|(index, _)| format!("i64 {index}, label %case{index}"))
                .collect();
            emitter.end(format!("switch i64 {tag}, label %bare [ {} ]", ways.join(" ")));
            for (index, (case, fields)) in cases.iter().enumerate() {
                if fields.is_empty() {
                    continue;
                }
                emitter.place(&format!("case{index}"));
                let ty = case_type(owner, case);
                for (position, (_, kind)) in fields.iter().enumerate() {
                    if takes[index][position] || (texts && is_text(kind)) {
                        let at = emitter.field_at(&ty, "%self", position + 1);
                        let held = emitter.set(format!("load {}, ptr {at}", ir_type(kind, span)?));
                        emitter.free(kind, &held, span)?;
                    }
                }
                emitter.run("zeno_free", "ptr %self");
                emitter.end("ret void");
            }
            emitter.place("bare");
            emitter.end("ret void");
            Ok(())
        })?;
        Ok(name)
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
            walked: None,
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
    ///
    /// `begin` is for a `for` statement in a build with no collector:
    /// text built to find what to loop over is given back once the loop
    /// has its list or its numbers in hand, before the first trip.
    fn open_loop(&mut self, variable: &str, iterable: &Expr, begin: Option<(usize, bool)>) -> Result<Count, Error> {
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
        let (mut count, value, walked) = match range {
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
                self.text_given_back(begin);
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
                (count, Val::new(number, Kind::Int), None)
            }
            // `for x in items`. The loop walks the list as it was when
            // the loop began, so a body that adds to the list or removes
            // from it cannot confuse the loop. That is what `zeno run`
            // does too.
            None => {
                self.walked = Some(iterable as *const Expr as usize);
                let list = self.expr(iterable);
                self.walked = None;
                let list = list?;
                // A loop over a dict walks a list of its keys, made for
                // the loop. A dict that was made right here (`for key in
                // ["a": 1]`) is nobody's, and has done its work once the
                // keys are out of it.
                let list = match &list.kind {
                    Kind::Dict(key, _) => {
                        let keys = self.call("ptr", "zeno_dict_keys", format!("ptr {}", list.code));
                        if list.made {
                            self.run("zeno_dict_drop", format!("ptr {}", list.code));
                        }
                        Val::new(keys, Kind::List(key.clone())).made()
                    }
                    _ => list,
                };
                // A list made right here is nobody else's, so it needs no
                // copy: one written out, or what `split`, `map`, `filter`
                // or `+` gave, or an owned list that a function gave back.
                let fresh = list.made;
                // Nor does a list that the statement owns: nothing the
                // body does can reach it. The statement frees it, after
                // the loop.
                let own = list.passing && list.kind != Kind::Str;
                let (items, item) = match &list.kind {
                    Kind::List(item) if fresh || own => (list.code.clone(), (**item).clone()),
                    // With no collector a list of text has its own copies
                    // of its items, and so does the copy the loop walks:
                    // the body may give the list new items while the loop
                    // looks at the old.
                    Kind::List(item) if !self.collector && is_text(item) => {
                        (self.call("ptr", "zeno_list_copy_text", format!("ptr {}", list.code)), (**item).clone())
                    }
                    Kind::List(item) => {
                        (self.call("ptr", "zeno_list_copy", format!("ptr {}", list.code)), (**item).clone())
                    }
                    // A str is looped over one character at a time: the
                    // loop walks a list of them, which is its own.
                    Kind::Str => (self.call("ptr", "zeno_chars", format!("ptr {}", list.code)), Kind::Str),
                    _ => return Err(cannot("a `for` loop over anything but a list, a str or `range(...)`", span)),
                };
                self.text_given_back(begin);
                let (count, slot) = self.walk_open(variable, &items);
                // What frees the list: its items too, if they are text
                // it has copies of, or if the list is the one that was
                // made and its items have no other owner. The items in
                // a copy still belong to the list that was copied.
                let whole = (!self.collector && is_text(&item)) || (fresh && self.drops.owns(&item));
                let kind = Kind::List(Box::new(if whole { item.clone() } else { Kind::None }));
                (count, self.from_slot(&item, &slot, span)?.fixed(), (!own).then_some((items, kind)))
            }
        };
        // What the body sees is a value, which it cannot change.
        self.scopes.push(HashMap::from([(
            variable.to_string(),
            Variable { place: Place::Value(value.code), kind: value.kind, open: false, owner: false },
        )]));
        // The list being walked is the loop's own: a copy, or a list
        // written out for it. A `return` or a failed call inside the loop
        // frees it on the way out; `close_loop` does when the loop ends
        // by itself.
        if let Some((items, kind)) = &walked {
            self.hold(kind.clone(), items);
        }
        count.walked = walked;
        Ok(count)
    }

    /// The first line of a `for` has been worked out. Give back the text
    /// it built, if it built any.
    fn text_given_back(&mut self, begin: Option<(usize, bool)>) {
        if let Some(mark) = begin.and_then(|begin| self.text_end(begin)) {
            self.text_back(&mark);
        }
    }

    /// The end of a `for` loop. Nothing but the loop ever saw the list
    /// it walked, so that is freed here and now, and not left for the
    /// collector.
    fn close_loop(&mut self, count: &Count) {
        self.scopes.pop();
        self.count_close(count);
        if let Some((items, kind)) = &count.walked {
            self.free(kind, items, Span { line: 0, col: 0 }).expect("a list can be freed");
        }
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
        // With no collector, text built to find the value is needed until
        // a case has been chosen, and is given back as that case begins.
        let begin = self.text_begin();
        let subject = self.expr(subject)?;
        let mark = self.text_end(begin);
        let n = self.number();
        let end = format!("end{n}");
        let mut chosen = false;
        for (index, arm) in arms.iter().enumerate() {
            let next = format!("case{n}.{}", index + 1);
            // The names the pattern gives are variables of this case.
            self.scopes.push(HashMap::new());
            let result = match self.pattern(&arm.pattern, &subject, &next, span) {
                Ok(tested) => self.arm(&arm.body, mark.as_deref()).map(|()| tested),
                Err(error) => Err(error),
            };
            self.scopes.pop();
            let tested = result?;
            self.jump(&end);
            if !tested {
                chosen = true;
                break; // nothing after a pattern that takes everything can run
            }
            self.place(&next);
        }
        // No case was chosen: the text is given back all the same.
        if let (Some(mark), false) = (&mark, chosen) {
            self.text_back(mark);
        }
        self.place(&end);
        Ok(())
    }

    /// The body of one `case`, once its pattern has matched. The names
    /// the pattern gave are in the innermost scope.
    fn arm(&mut self, body: &[Stmt], mark: Option<&str>) -> Result<(), Error> {
        if !self.collector {
            // A name that was given text outlives the text room's copy,
            // and whatever the text was read from: it gets its own.
            let scope = self.scopes.last().expect("a scope");
            let mut names: Vec<String> =
                scope.iter().filter(|(_, variable)| is_text(&variable.kind)).map(|(name, _)| name.clone()).collect();
            names.sort();
            for name in names {
                let variable = self.scopes.last().expect("a scope")[&name].clone();
                let Place::Value(code) = &variable.place else { continue };
                let kept = self.call("ptr", "zeno_text_keep", format!("ptr {code}"));
                let variable = Variable { place: Place::Value(kept), owner: true, ..variable };
                self.scopes.last_mut().expect("a scope").insert(name, variable);
            }
        }
        if let Some(mark) = mark {
            self.text_back(mark);
        }
        self.block(body)?;
        // Unless the body left by itself, free what the names hold.
        if !self.code.ended {
            self.free_owned(self.scopes.len() - 1)?;
        }
        Ok(())
    }

    /// Write the tests of one pattern against `val`, each jumping to
    /// `fail` if it does not hold, and declare the names the pattern
    /// gives. Returns whether there was anything to test.
    fn pattern(&mut self, pattern: &Pattern, val: &Val, fail: &str, span: Span) -> Result<bool, Error> {
        // A value that may be `none` is a pointer that may be null, and a
        // case of an enum matches only a value that is there.
        let there = match &val.kind {
            Kind::Optional(inner) => (**inner).clone(),
            kind => kind.clone(),
        };
        let may_be_none = there != val.kind;
        match pattern {
            Pattern::Wildcard => Ok(false),
            Pattern::Name(name) => match (self.cases.get(name), &there) {
                // A bare enum case, such as `Idle`, is compared.
                (Some((owner, index)), Kind::Enum(subject)) if owner == subject && self.lookup(name).is_none() => {
                    let index = *index;
                    if may_be_none {
                        self.must_be_there(&val.code, fail);
                    }
                    self.tag_is(&val.code, index, fail);
                    Ok(true)
                }
                // Any other name is a new variable holding the value.
                _ => {
                    let variable = Variable { place: Place::Value(val.code.clone()), kind: val.kind.clone(), open: false, owner: false };
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
                let (owner, index) = match (self.cases.get(name), &there) {
                    (Some((owner, index)), Kind::Enum(subject)) if owner == subject => (owner.clone(), *index),
                    _ => return Err(cannot(format!("the pattern `{name}(...)` here"), span)),
                };
                let carried = self.enums[&owner].cases[index].1.clone();
                if carried.len() != fields.len() {
                    return Err(cannot(format!("the pattern `{name}(...)` with this many parts"), span));
                }
                if may_be_none {
                    self.must_be_there(&val.code, fail);
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

    /// Go on only if `value`, which may be `none`, is not.
    fn must_be_there(&mut self, value: &str, fail: &str) {
        let there = self.set(format!("icmp ne ptr {value}, null"));
        self.must(&there, fail);
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
        if self.firmware {
            return self.try_with_values(body, handlers, span);
        }
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
        self.code.try_scopes.push(self.scopes.len());
        let result = self.block(body);
        self.code.try_scopes.pop();
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
                let variable = Variable { place: Place::Value(value), kind: holds, open: false, owner: false };
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
        // In a firmware build a `try` leaves nothing behind to undo.
        if self.firmware {
            return;
        }
        if let Some(outermost) = self.code.tries.get(keep).cloned() {
            self.run("zeno_try_close", format!("ptr {outermost}"));
        }
    }

    /// `raise`. The runtime jumps to the nearest `try` that is running,
    /// or stops the program if there is none. Either way it does not
    /// come back here, which is what `unreachable` says.
    fn raise(&mut self, value: &Expr, span: Span) -> Result<(), Error> {
        let begin = self.text_begin();
        // With no collector `raise` takes the error: whoever held it
        // holds nothing now, and the `except` that catches it is its
        // next owner.
        let val = self.expr(value)?;
        self.stays(&val, value.span)?;
        let error = match &val.kind {
            Kind::Error => val.code,
            // An object of a class can be raised too. It travels inside
            // an error value whose kind is the class's name.
            Kind::Class(name) => {
                let kind = self.string(name);
                let shown = self.text(&val, span)?;
                if self.collector {
                    self.call("ptr", "zeno_error_new", format!("ptr {kind}, ptr {shown}, ptr {}", val.code))
                } else {
                    // The `except` that catches it frees the object, and
                    // the text the object keeps, through this function.
                    let drop = self.drop_class(name, span)?;
                    self.call("ptr", "zeno_error_of", format!("ptr {kind}, ptr {shown}, ptr {}, ptr {drop}", val.code))
                }
            }
            _ => return Err(cannot("`raise` with anything but an error or an object", span)),
        };
        if self.firmware {
            // Note the error, then go where a failed call would go.
            let kind = match (&val.kind, &value.kind) {
                (Kind::Class(name), _) => name.clone(),
                // `raise ValueError("...")` says its kind. An error held
                // in a variable could be any kind.
                (_, ExprKind::Call { callee, .. }) => match &callee.kind {
                    ExprKind::Name(name) if emit_c::ERROR_KINDS.contains(&name.as_str()) => name.clone(),
                    _ => "Error".to_string(),
                },
                _ => "Error".to_string(),
            };
            self.run("zeno_note", format!("ptr {error}, {}", at(span)));
            // The error has its own copy of its message by now.
            if let Some(mark) = self.text_end(begin) {
                self.text_back(&mark);
            }
            return self.failed(&[kind], span);
        }
        // The jump leaves at least the blocks opened since the nearest
        // `try` in this function, or all of the function's if there is
        // none. Free what their owners hold first. (The functions the
        // jump passes through on its way are not asked: what they own is
        // left to the collector.)
        let inside = self.code.try_scopes.last().copied().unwrap_or(self.base());
        self.free_owned(inside)?;
        self.run("zeno_raise", format!("ptr {error}, {}", at(span)));
        self.end("unreachable");
        Ok(())
    }

    // ------------------------------------------- Errors as values (firmware)
    //
    // `zeno build --llvm --firmware`. Nothing jumps out of a function. A
    // function that can raise gives back an `i1`: 1 when it worked, 0
    // when it raised. Its answer goes out through one more parameter, a
    // pointer. `raise` notes the error in the runtime's `zeno_raised` and
    // gives back 0, and every call to such a function is followed by a
    // test. This is `docs/firmware.md`, written in IR in place of C.

    /// Something here has failed: a `raise`, or a call that has just
    /// given back 0. The error is already noted in `zeno_raised`. Leave
    /// the block for where the error goes next:
    ///
    /// - inside a `try`, its `except` blocks, further down this function;
    /// - in a function that can raise, back to the caller, with 0;
    /// - at the top of the file, nobody is left to catch it.
    ///
    /// `kinds` is what may have been raised, so that the `try` around
    /// this spot knows what can reach it.
    fn failed(&mut self, kinds: &[String], span: Span) -> Result<(), Error> {
        if let Some(arriving) = self.code.arriving.last_mut() {
            arriving.extend(kinds.iter().cloned());
        }
        // Leaving this way ends blocks early, as `break` and `return` do:
        // what their owners hold is freed on the way out.
        if let Some(label) = self.code.tries.last().cloned() {
            let inside = self.code.try_scopes.last().copied().unwrap_or(self.scopes.len());
            self.free_owned(inside)?;
            self.jump(&label);
        } else if self.code.fallible {
            self.free_owned(self.base())?;
            self.end("ret i1 false");
        } else if !self.code.in_function {
            self.run("zeno_uncaught", "");
            self.end("unreachable");
        } else {
            // The checker makes a function say what it raises, so this
            // is a function value, which has no way to say it.
            return Err(raises_as_value(span));
        }
        Ok(())
    }

    /// `worked` is the `i1` that a call which can fail gave back. If it
    /// is 0 the error goes on its way; if 1, carry on.
    ///
    /// ```text
    ///   %t4 = call zeroext i1 @z_parse(ptr %t3, ptr %answer.2)
    ///   br i1 %t4, label %worked2, label %failed2
    /// failed2:
    ///   ret i1 false                    ; or: br label %except1
    /// worked2:
    ///   %t5 = load double, ptr %answer.2
    /// ```
    fn tested(&mut self, worked: &str, kinds: &[String], span: Span) -> Result<(), Error> {
        let n = self.number();
        let (fine, bad) = (format!("worked{n}"), format!("failed{n}"));
        self.jump_if(worked, &fine, &bad);
        self.place(&bad);
        self.failed(kinds, span)?;
        self.place(&fine);
        Ok(())
    }

    /// A call, in a firmware build, to something that gives back an `i1`
    /// and puts its answer where its last argument points. `call` is the
    /// call up to that last argument, with no closing bracket.
    fn checked_call(&mut self, call: String, returns: &Kind, kinds: &[String], span: Span) -> Result<Val, Error> {
        if *returns == Kind::None {
            let worked = self.set(format!("{call})"));
            self.tested(&worked, kinds, span)?;
            return Ok(Val::nothing());
        }
        // A place on this function's stack for the answer.
        let ty = ir_type(returns, span)?;
        let n = self.number();
        let answer = self.slot(format!("%answer.{n}"), ty);
        let comma = if call.ends_with('(') { "" } else { ", " };
        let worked = self.set(format!("{call}{comma}ptr {answer})"));
        self.tested(&worked, kinds, span)?;
        let val = Val::new(self.set(format!("load {ty}, ptr {answer}")), returns.clone());
        Ok(self.came_back(val))
    }

    /// `try` / `except` in a firmware build. Each thing in the body that
    /// can fail is followed by a test, and a failure goes forward to the
    /// `except` blocks. So does the end of the body, and there the first
    /// question is whether anything failed at all:
    ///
    /// ```text
    ///   ...the body, with `br label %except1` after each failure...
    ///   br label %except1
    /// except1:
    ///   %t6 = load ptr, ptr @zeno_raised
    ///   %t7 = icmp ne ptr %t6, null             ; did the body fail?
    ///   br i1 %t7, label %caught1, label %end1
    /// caught1:
    ///   store ptr null, ptr @zeno_raised        ; it is being dealt with
    ///   %t8 = call zeroext i1 @zeno_is(ptr %t6, ptr @.str.4)
    ///   br i1 %t8, label %except1.1, label %caught1.1
    /// except1.1:
    ///   ...the except block...
    ///   br label %end1
    /// caught1.1:
    ///   store ptr %t6, ptr @zeno_raised         ; not ours: pass it on
    ///   ret i1 false
    /// end1:
    /// ```
    fn try_with_values(&mut self, body: &[Stmt], handlers: &[Handler], span: Span) -> Result<(), Error> {
        let n = self.number();
        let (except, caught, end) = (format!("except{n}"), format!("caught{n}"), format!("end{n}"));
        self.code.tries.push(except.clone());
        self.code.arriving.push(BTreeSet::new());
        self.code.try_scopes.push(self.scopes.len());
        // With no collector: an error can leave a statement half way
        // through, with its text still in the text room. Whatever the
        // body leaves there is given back where the error is caught.
        let text_top = (!self.collector).then(|| self.set("load i64, ptr @zeno_text_top"));
        let result = self.block(body);
        self.code.try_scopes.pop();
        self.code.tries.pop();
        let arrived = self.code.arriving.pop().unwrap_or_default();
        result?;

        // Only an error that travels as a value can reach an `except`. A
        // division by zero or a bad index stops a firmware build instead,
        // and an `except` waiting for one would wait for ever.
        const WHY: &str = "In a firmware build, an error the program raises itself (and `int` or `float` of text) travels as a value and can be caught. A division by zero, a bad index, a key that is not in a dict or an overflow stops the program. Test for it before it happens";
        if arrived.is_empty() {
            return Err(firmware_cannot(format!("nothing in this `try` can be caught. {WHY}"), span));
        }
        let anything = arrived.contains("Error");

        self.place(&except);
        if let Some(mark) = &text_top {
            self.text_back(mark);
        }
        let raised = self.set("load ptr, ptr @zeno_raised");
        let failed = self.set(format!("icmp ne ptr {raised}, null"));
        self.jump_if(&failed, &caught, &end);
        self.place(&caught);
        self.ins("store ptr null, ptr @zeno_raised");

        let mut handled: Vec<&str> = Vec::new();
        let mut catches_everything = false;
        for (index, handler) in handlers.iter().enumerate() {
            let (wanted, holds) = match handler.error.as_deref() {
                None | Some("Error") => (None, Kind::Error),
                Some(class) if self.classes.contains_key(class) => (Some(class), Kind::Class(class.to_string())),
                Some(kind) if emit_c::ERROR_KINDS.contains(&kind) => (Some(kind), Kind::Error),
                Some(other) => return Err(cannot(format!("`except {other}`"), span)),
            };
            let next = format!("caught{n}.{}", index + 1);
            if let Some(kind) = wanted {
                // An `except` for a class that nothing here raises is idle
                // in every kind of build. One for a built-in kind is most
                // likely waiting for the runtime to raise it.
                if !anything && !arrived.contains(kind) && !self.classes.contains_key(kind) {
                    return Err(firmware_cannot(format!("nothing in this `try` can reach `except {kind}`. {WHY}"), span));
                }
                handled.push(kind);
                let kind = self.string(kind);
                let ours = self.call("i1", "zeno_is", format!("ptr {raised}, ptr {kind}"));
                let handle = format!("except{n}.{}", index + 1);
                self.jump_if(&ours, &handle, &next);
                self.place(&handle);
            }
            self.scopes.push(HashMap::new());
            // With no collector this block has received the error and
            // owns it. However the block is left, the error is freed on
            // the way out, unless the block gave it on: kept it,
            // returned it or raised it again.
            let named = handler.binding.is_some();
            if !self.collector && !(named && holds == Kind::Error) {
                self.hold(Kind::Error, &raised);
            }
            if let Some(name) = &handler.binding {
                let value = match holds {
                    // An object that was raised travels inside the error.
                    Kind::Class(_) => {
                        let at = self.field_at(ERROR, &raised, 2);
                        let object = self.set(format!("load ptr, ptr {at}"));
                        if !self.collector {
                            // The name owns the object from here, so it
                            // is taken out of the error.
                            self.store("ptr", "null", &at);
                        }
                        object
                    }
                    _ => raised.clone(),
                };
                let variable = match self.collector {
                    true => Variable { place: Place::Value(value), kind: holds, open: false, owner: false },
                    // The name is an owner, so it is a slot: giving the
                    // error on leaves null behind.
                    false => {
                        let slot = self.slot(format!("%caught.{n}.{}", index + 1), "ptr");
                        self.store("ptr", &value, &slot);
                        Variable { place: Place::Slot(slot), kind: holds, open: false, owner: true }
                    }
                };
                self.scopes.last_mut().expect("a scope").insert(name.clone(), variable);
            }
            let mut result = self.block(&handler.body);
            if result.is_ok() && !self.code.ended {
                result = self.free_owned(self.scopes.len() - 1);
            }
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
            // What no `except` here takes carries on outward.
            self.ins(format!("store ptr {raised}, ptr @zeno_raised"));
            let passed_on: Vec<String> =
                arrived.iter().filter(|kind| !handled.contains(&kind.as_str())).cloned().collect();
            if passed_on.is_empty() {
                // Every kind the body can raise has an `except` above, so
                // this cannot happen. LLVM cannot see that, and a block
                // has to end somehow.
                self.run("zeno_uncaught", "");
                self.end("unreachable");
            } else {
                self.failed(&passed_on, span)?;
            }
        }
        self.place(&end);
        Ok(())
    }

    // ---------------------------------------------------------- Expressions

    /// An expression that must be a bool, for `if` and `while`.
    fn condition(&mut self, expr: &Expr) -> Result<String, Error> {
        // With no collector: if working it out builds text, the answer
        // is one bit by the time it is known, and the text is given back
        // before anything else runs.
        let begin = self.text_begin();
        // The same goes for a value that the condition alone owns.
        let own = self.own_begin();
        let val = self.expr(expr)?;
        self.own_end(own, true)?;
        if let Some(mark) = self.text_end(begin) {
            self.text_back(&mark);
        }
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
                let val = Val::new(self.call("ptr", "zeno_list_new", "i64 0"), kind.clone()).made();
                Ok(self.belongs(val, expr))
            }
            (ExprKind::Dict(entries), Some(kind @ Kind::Dict(key, value))) if entries.is_empty() => {
                let kinds = dict_kinds(key, value, expr.span)?;
                let val = Val::new(self.call("ptr", "zeno_dict_new", kinds), kind.clone()).made();
                Ok(self.belongs(val, expr))
            }
            _ => self.expr(expr),
        }
    }

    /// Write the instructions that work an expression out, and return
    /// the value they leave.
    fn expr(&mut self, expr: &Expr) -> Result<Val, Error> {
        let val = self.value(expr)?;
        let val = self.belongs(val, expr);
        // For a build with no collector: text that was not already there
        // has just been built, and the statement will have to give it
        // back.
        let text = match &val.kind {
            Kind::Optional(inner) => **inner == Kind::Str,
            kind => *kind == Kind::Str,
        };
        if text && !val.fixed && !self.collector {
            self.code.built = true;
        }
        Ok(val)
    }

    fn value(&mut self, expr: &Expr) -> Result<Val, Error> {
        let span = expr.span;
        Ok(match &expr.kind {
            // The lexer lets one number through that is one too big for an
            // int, and `zeno run` reports it when it gets there.
            ExprKind::Int(n) if *n > i64::MAX as u64 => return Err(cannot("a whole number this big", span)),
            ExprKind::Int(n) => Val::new(n.to_string(), Kind::Int),
            ExprKind::Float(x) => Val::new(float_code(*x), Kind::Float),
            ExprKind::Bool(b) => Val::new(b.to_string(), Kind::Bool),
            ExprKind::Str(text) => Val::new(self.string(text), Kind::Str).fixed(),
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

            // A call of a generic goes to one of its copies, and the
            // checker noted which: `first([1, 2])` calls `first[int]`.
            ExprKind::Call { args, .. } if self.types.copy_called(expr).is_some() => {
                let copy = self.types.copy_called(expr).unwrap_or_default().to_string();
                if let Some(entry) = self.functions.get(&copy).cloned() {
                    self.call_named(&entry, args, span)?
                } else if self.classes.contains_key(&copy) {
                    self.construct(&copy, args, span)?
                } else {
                    return Err(cannot(format!("`{copy}`, a generic that is not at the top of the file,"), span));
                }
            }
            ExprKind::Call { callee, args } => self.call_expr(callee, args, span)?,
            ExprKind::Field { object, name } => self.field(object, name, span)?,
            ExprKind::Index { object, index } => {
                let list = self.expr(object)?;
                let position = self.expr(index)?;
                // `ages["Ada"]`: the runtime finds the key and gives its
                // value, or fails with a KeyError.
                if let Kind::Dict(_, value) = &list.kind {
                    let key = self.to_slot(&position, span)?;
                    let found = self.call("i64", "zeno_dict_at", format!("ptr {}, i64 {key}, {}", list.code, at(span)));
                    let val = self.from_slot(value, &found, span)?;
                    return Ok(self.lend(val));
                }
                let args = format!("ptr {}, i64 {}, {}", list.code, position.code, at(span));
                match &list.kind {
                    _ if position.kind != Kind::Int => return Err(cannot("`[ ]` with anything but an int inside", span)),
                    Kind::Str => Val::new(self.call("ptr", "zeno_char_at", args), Kind::Str),
                    // The runtime checks the index and gives back the
                    // address of that item's slot.
                    Kind::List(item) => {
                        let place = self.call("ptr", "zeno_list_at", args);
                        let slot = self.set(format!("load i64, ptr {place}"));
                        let val = self.from_slot(item, &slot, span)?;
                        self.lend(val)
                    }
                    // Through a pointer: the address of that item, and
                    // a load, with no check. A sized item is made wide
                    // again, like a field.
                    Kind::Pointer(pointee) => {
                        let item = pointee.ir_type();
                        let place = self.set(format!("getelementptr {item}, ptr {}, i64 {}", list.code, position.code));
                        let read = self.set(format!("load {item}, ptr {place}"));
                        let code = match pointee {
                            Pointee::Sized(width) => {
                                let widen = if width.signed { "sext" } else { "zext" };
                                self.set(format!("{widen} {item} {read} to i64"))
                            }
                            _ => read,
                        };
                        Val::new(code, pointee.kind())
                    }
                    _ => return Err(cannot(format!("`[ ]` on {}", described(&list.kind)), span)),
                }
            }
            ExprKind::List(items) => self.list(items, span)?.made(),
            ExprKind::Dict(entries) => self.dict(entries, span)?.made(),
            ExprKind::ListComp { element, variable, iterable, condition } => {
                self.comprehension(element, variable, iterable, condition.as_deref(), span)?.made()
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
            if let (Some(number), true) =
                (emit_c::math_constant(name), self.imported.get(name).is_some_and(|module| module == "math"))
            {
                return Ok(Val::new(float_code(number), Kind::Float));
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
            // The owner gives its value away here. Take the address out
            // and leave null behind, so that the end of the owner's block
            // finds nothing to free.
            Place::Slot(slot) if self.types.moves(expr) => {
                let ty = ir_type(&variable.kind, span)?;
                let taken = self.load(ty, slot);
                self.store(ty, "null", slot);
                return Ok(Val::new(taken, variable.kind.clone()));
            }
            Place::Slot(slot) => {
                let ty = ir_type(&variable.kind, span)?;
                Val::new(self.load(ty, slot), variable.kind.clone())
            }
        };
        // Whatever a variable holds was there before this statement
        // began. If a function could give the variable new text, what it
        // holds now is lent.
        let val = self.present(val, expr, variable.open)?;
        Ok(if variable.open { self.lend(val) } else { val.fixed() })
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
            // `p + 4`: the address four items further on.
            (Add | Sub, Kind::Pointer(pointee), Int) => {
                let by = if op == Sub { self.set(format!("sub i64 0, {r}")) } else { r.clone() };
                let code = self.set(format!("getelementptr {}, ptr {l}, i64 {by}", pointee.ir_type()));
                Val::new(code, left.kind.clone())
            }
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
                let mut joined = self.call("ptr", "zeno_list_concat", format!("ptr {l}, ptr {r}"));
                if !self.collector && is_text(a) {
                    // With no collector a list of text has its own copy
                    // of each item, and so has the list that `+` makes.
                    joined = self.call("ptr", "zeno_list_keep_text", format!("ptr {joined}"));
                }
                Val::new(joined, left.kind.clone()).made()
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
            // `"Ada" in ages`: is there such a key? The runtime gives
            // where it is, or -1.
            (item, Kind::Dict(key, _)) if item == &**key => {
                let slot = self.to_slot(&left, span)?;
                let at = self.call("i64", "zeno_dict_find", format!("ptr {}, i64 {slot}", right.code));
                self.set(format!("icmp sge i64 {at}, 0"))
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
                if let ("math", Some(number)) = (owner.as_str(), emit_c::math_constant(name)) {
                    return Ok(Val::new(float_code(number), Kind::Float));
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
                Some((_, index, kind)) => {
                    // A list or an object inside an object that the
                    // statement owns is gone when the statement is.
                    let passing = val.passing && matches!(kind, Kind::List(_) | Kind::Dict(..) | Kind::Class(_));
                    let val = Val::new(self.load_field(class, &val.code, index, span)?, kind);
                    Ok(Val { passing, ..self.lend(val) })
                }
                None => Err(cannot(format!("a method used as a value: `{name}`"), span)),
            },
            Kind::Error if matches!(name, "message" | "kind") => {
                let at = self.field_at(ERROR, &val.code, if name == "kind" { 0 } else { 1 });
                Ok(Val::new(self.set(format!("load ptr, ptr {at}")), Kind::Str))
            }
            // `led.number`: a pin is its number already.
            Kind::Pin if name == "number" => Ok(Val { kind: Kind::Int, ..val }),
            Kind::Register(_) if name == "address" => Ok(Val { kind: Kind::Int, ..val }),
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
            Kind::Int | Kind::Pin | Kind::Register(_) => val.code.clone(),
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
            Kind::Int | Kind::Pin | Kind::Register(_) => slot.to_string(),
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
        let mut held = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let val = self.expr(item)?;
            self.stays(&val, item.span)?;
            // An item that has an owner of its own has left that owner
            // and not yet reached the list. In a firmware build a later
            // item may fail, and the failure leaves this function at
            // once. Hold the item until the list is made, so that such
            // a failure frees it.
            if self.firmware && self.drops.owns(&val.kind) && index + 1 < items.len() {
                held.push(self.hold(val.kind.clone(), &val.code));
            }
            vals.push(val);
        }
        for key in &held {
            self.let_go(key);
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
        self.fits_a_slot(&item, span)?;
        // `zeno_list_of` takes a count and then that many slots.
        let mut slots = format!("i32 {}", vals.len());
        for (mut val, written) in vals.into_iter().zip(items) {
            // Every item has been worked out by now, so each is copied
            // from what the statement holds.
            val.lent = None;
            let val = self.keep(val, is_written(written));
            let val = self.coerce(val, &item, span)?;
            slots += &format!(", i64 {}", self.to_slot(&val, span)?);
        }
        self.uses("zeno_list_of");
        let list = self.set(format!("call ptr (i32, ...) @zeno_list_of({slots})"));
        Ok(Val::new(list, Kind::List(Box::new(item))))
    }

    /// `["Ada": 36, "Alan": 41]`: a new dict, and then one call for each
    /// entry, left to right.
    ///
    /// ```text
    ///   %t1 = call ptr @zeno_dict_new(i32 115, i32 105)       ; 's' and 'i'
    ///   %t2 = ptrtoint ptr @.str.1 to i64
    ///   call void @zeno_dict_set(ptr %t1, i64 %t2, i64 36)
    /// ```
    fn dict(&mut self, entries: &[(Expr, Expr)], span: Span) -> Result<Val, Error> {
        let mut vals = Vec::new();
        for (key, value) in entries {
            let key = self.expr(key)?;
            let value = self.expr(value)?;
            vals.push((key, value));
        }
        let Some((first_key, first_value)) = vals.first() else {
            // `[:]` alone does not say what it will hold. The place it
            // arrives at does (`coerce`).
            let nothing_yet = || Box::new(Kind::Other("nothing yet".to_string()));
            let dict = self.call("ptr", "zeno_dict_new", "i32 0, i32 0");
            return Ok(Val::new(dict, Kind::Dict(nothing_yet(), nothing_yet())));
        };
        let (key, value) = (first_key.kind.clone(), first_value.kind.clone());
        let kinds = dict_kinds(&key, &value, span)?;
        if vals.iter().any(|(a_key, a_value)| a_key.kind != key || a_value.kind != value) {
            return Err(cannot("a dict whose keys, or whose values, are not all one type", span));
        }
        let dict = self.call("ptr", "zeno_dict_new", kinds);
        for ((a_key, mut a_value), (_, written)) in vals.into_iter().zip(entries) {
            // The dict keeps a copy of its own of each value that is
            // text, as a list does of each item. Every value has been
            // worked out by now, so each is copied from what the
            // statement holds.
            a_value.lent = None;
            let a_value = self.keep(a_value, is_written(written));
            let key_slot = self.to_slot(&a_key, span)?;
            let value_slot = self.to_slot(&a_value, span)?;
            self.run("zeno_dict_set", format!("ptr {dict}, i64 {key_slot}, i64 {value_slot}"));
        }
        Ok(Val::new(dict, Kind::Dict(Box::new(key), Box::new(value))))
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
        // Until it is full, the list is nobody's. If working out an item
        // fails, the failure frees what there is of it.
        let filling = self.hold(Kind::List(Box::new(Kind::None)), &list);
        let filled_from = self.code.lines.len();
        let count = self.open_loop(variable, iterable, None)?;
        // Text built for one item is given back before the next.
        let begin = self.text_begin();
        if let Some(condition) = condition {
            // An item that fails the test is skipped: on to the next trip.
            let wanted = self.condition(condition)?;
            let keep = format!("keep{}", self.number());
            self.jump_if(&wanted, &keep, &count.next);
            self.place(&keep);
        }
        // So is what working out one item makes and gives to nobody.
        let own = self.own_begin();
        let item = self.expr(element)?;
        self.stays(&item, element.span)?;
        self.fits_a_slot(&item.kind, span)?;
        let item = self.keep(item, is_written(element));
        let owns = self.drops.owns(&item.kind);
        if (!self.collector && is_text(&item.kind)) || owns {
            // The list has copies of its items, or is the owner of them,
            // which was not known when the lines above were written: a
            // failure half way has to free those as well.
            let whole = match owns {
                true => self.drop_list(&item.kind, span)?,
                false => {
                    self.uses("zeno_list_drop_text");
                    "@zeno_list_drop_text".to_string()
                }
            };
            let (plain, whole) = (format!("call void @zeno_list_drop(ptr {list})"), format!("call void {whole}(ptr {list})"));
            for line in &mut self.code.lines[filled_from..] {
                if line.trim_start() == plain {
                    *line = line.replace(&plain, &whole);
                }
            }
        }
        let slot = self.to_slot(&item, span)?;
        self.run("zeno_list_append", format!("ptr {list}, i64 {slot}"));
        self.own_end(own, true)?;
        if let Some(mark) = self.text_end(begin) {
            self.text_back(&mark);
        }
        self.close_loop(&count);
        self.let_go(&filling);
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
        // `[:]` becomes a dict of whatever this place holds, and is told so.
        if is_empty_dict(&val.kind) {
            let wanted = match to {
                Kind::Optional(inner) => &**inner,
                to => to,
            };
            if let Kind::Dict(key, value) = wanted {
                let kinds = dict_kinds(key, value, span)?;
                self.call("ptr", "zeno_dict_kinds", format!("ptr {}, {kinds}", val.code));
                val.kind = wanted.clone();
                if val.kind == *to {
                    return Ok(val);
                }
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
        Ok(Val { code, kind: to.clone(), lent: None, ..val })
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
        Ok(Val { code, kind: (**inner).clone(), lent: None, ..val.clone() })
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

    /// A call of a function of the program by its name.
    fn call_named(&mut self, entry: &Entry, args: &[Arg], span: Span) -> Result<Val, Error> {
        let slots = entry
            .decl
            .params
            .iter()
            .zip(&entry.kind.params)
            .map(|(param, kind)| (param.name.clone(), kind.clone(), param.default.clone()))
            .collect();
        let list = self.bind(&slots, &entry.owned, &entry.lends, args, span)?;
        self.call_entry(entry, list, span)
    }

    fn call_expr(&mut self, callee: &Expr, args: &[Arg], span: Span) -> Result<Val, Error> {
        match &callee.kind {
            ExprKind::Name(name) if self.lookup(name).is_none() => {
                if let Some(entry) = self.functions.get(name).cloned() {
                    return self.call_named(&entry, args, span);
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
                        if matches!(owner.as_str(), "time" | "board" | "files" | "program") {
                            return self.device(owner, name, args, span);
                        }
                        // `u8.wrap(n)`: cut `n` down to eight bits and
                        // make it an i64 again. Two instructions, no call.
                        if let (Some(width), "wrap", [arg]) = (self.sized_name(owner), name.as_str(), args) {
                            let n = self.expr(&arg.value)?;
                            if n.kind != Kind::Int || arg.name.is_some() {
                                return Err(cannot(format!("`{owner}.wrap` called this way"), span));
                            }
                            let small = self.set(format!("trunc i64 {} to {}", n.code, width.ir_type()));
                            let extend = if width.signed { "sext" } else { "zext" };
                            return Ok(Val::new(self.set(format!("{extend} {} {small} to i64", width.ir_type())), Kind::Int));
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
    ///
    /// `owned` says which places take over an owned value, and `lends`
    /// which only look (no `take`).
    fn bind(&mut self, slots: &Slots, owned: &[bool], lends: &[bool], args: &[Arg], span: Span) -> Result<Vec<String>, Error> {
        let mut given: Vec<Option<Val>> = vec![None; slots.len()];
        let mut held = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let place = match &arg.name {
                None => index,
                Some(name) => match slots.iter().position(|(slot, _, _)| slot == name) {
                    Some(place) => place,
                    None => return Err(cannot(format!("the argument `{name}`"), span)),
                },
            };
            let val = self.expr(&arg.value)?;
            // A parameter without `take` only looks, so it may be lent what
            // the statement owns. Anything else may keep what it is given.
            if !(val.passing && lends.get(place) == Some(&true)) {
                self.stays(&val, arg.value.span)?;
            }
            // An owned value on its way into the call has left its owner
            // and not yet reached the function. In a firmware build a
            // later argument may fail, and the failure leaves this
            // function at once. Hold the value until the call, so that
            // such a failure frees it.
            if self.firmware && owned.get(place) == Some(&true) && index + 1 < args.len() {
                held.push(self.hold(val.kind.clone(), &val.code));
            }
            match given.get_mut(place) {
                Some(slot) => *slot = Some(val),
                None => return Err(cannot("this many arguments", span)),
            }
        }
        // From here the function owns what it was given.
        for key in &held {
            self.let_go(key);
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
        if self.firmware && !entry.decl.raises.is_empty() {
            // It says whether it worked, and that is tested at once.
            let call = format!("call zeroext i1 {}({}", entry.name, list.join(", "));
            return self.checked_call(call, &entry.kind.returns, &entry.decl.raises, span);
        }
        let returns = return_type(&entry.kind.returns, span)?;
        let call = format!("call {returns} {}({})", entry.name, list.join(", "));
        Ok(match &entry.kind.returns {
            // Nothing comes back, so there is nothing to name.
            Kind::None => {
                self.ins(call);
                Val::nothing()
            }
            kind => {
                let val = Val::new(self.set(call), kind.clone());
                self.came_back(val)
            }
        })
    }

    /// What a function gave back. With a collector, text that comes back
    /// will last. With none it is in the text room, like text this
    /// statement built itself.
    fn came_back(&self, val: Val) -> Val {
        if !self.collector && is_text(&val.kind) { val } else { val.fixed() }
    }

    /// `sqrt`, `floor` and the rest of `import math`.
    /// A function of the `time`, the `board` or the `files` module. Each
    /// is one call into the runtime, which passes it on to the board, or
    /// to the C library where the files are.
    fn device(&mut self, module: &str, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        // `register(u32, 0x4002_0014)`: a register is its address.
        if (module, name) == ("board", "register") {
            let (pointee, address) = written_type("register", args, span)?;
            let address = self.expr(address)?;
            return match (pointee, &address.kind) {
                (Pointee::Sized(width), Kind::Int) => Ok(Val { kind: Kind::Register(width), ..address }),
                _ => Err(cannot("`register` called this way", span)),
            };
        }
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
            // `files.exists(path)` cannot fail: it is a question.
            ("files", "exists", [path]) if path.kind == Kind::Str => {
                Ok(Val::new(self.call("i1", "zeno_files_exists", format!("ptr {}", path.code)), Kind::Bool))
            }
            // The other four can: the file may not be there, or may not
            // be text. Each is one call into the runtime, which raises a
            // `FileError`. In a firmware build the call says whether it
            // worked, as `int(text)` does.
            ("files", "read" | "write" | "append" | "remove", texts)
                if texts.iter().all(|text| text.kind == Kind::Str)
                    && texts.len() == if matches!(name, "write" | "append") { 2 } else { 1 } =>
            {
                let runtime = match name {
                    "read" => "zeno_files_read",
                    "write" => "zeno_files_write",
                    "append" => "zeno_files_append",
                    _ => "zeno_files_remove",
                };
                let returns = if name == "read" { Kind::Str } else { Kind::None };
                let mut list = String::new();
                for text in texts {
                    list += &format!("ptr {}, ", text.code);
                }
                list += &at(span);
                // The runtime builds text on the way: the file's own, or
                // the message of the error.
                self.code.built = !self.collector;
                if self.firmware {
                    self.uses(runtime);
                    let call = format!("call zeroext i1 @{runtime}({list}");
                    return self.checked_call(call, &returns, &["FileError".to_string()], span);
                }
                if name == "read" {
                    let text = Val::new(self.call("ptr", runtime, list), Kind::Str);
                    return Ok(self.came_back(text));
                }
                self.run(runtime, list);
                Ok(Val::nothing())
            }
            // `program.arguments()`: a new list, which is nobody's yet.
            // `main` hands the runtime the command line: see `file`.
            ("program", "arguments", []) => {
                self.uses("zeno_program_start");
                let words = self.call("ptr", "zeno_program_arguments", "");
                Ok(Val::new(words, Kind::List(Box::new(Kind::Str))).made())
            }
            _ => Err(cannot(format!("`{module}.{name}` called this way"), span)),
        }
    }

    fn math(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let wrong = || cannot(format!("`{name}` called this way"), span);
        if args.iter().any(|arg| arg.name.is_some()) {
            return Err(wrong());
        }
        // An angle that a function works out is in radians, and says so.
        let radians = Kind::Quantity("rad".to_string());
        // `atan2(y, x)` is the one function of two numbers.
        if let ("atan2", [y, x]) = (name, args) {
            let (y, x) = (self.expr(&y.value)?, self.expr(&x.value)?);
            if !matches!(y.kind, Kind::Int | Kind::Float) || !matches!(x.kind, Kind::Int | Kind::Float) {
                return Err(wrong());
            }
            let (y, x) = (self.double(&y), self.double(&x));
            return Ok(Val::new(self.call("double", "atan2", format!("double {y}, double {x}")), radians));
        }
        let [arg] = args else {
            return Err(wrong());
        };
        let val = self.expr(&arg.value)?;
        let x = match &val.kind {
            // `sin(30 deg)`: the C library counts in radians.
            Kind::Quantity(unit) if matches!(name, "sin" | "cos" | "tan") => match units::convert(unit, "rad") {
                Some(scale) => self.scaled(&val.code, scale),
                None => return Err(cannot(format!("`{name}` of a quantity in {unit}"), span)),
            },
            Kind::Int | Kind::Float => self.double(&val),
            _ => return Err(wrong()),
        };
        Ok(match name {
            "sqrt" => Val::new(self.call("double", "zeno_sqrt", format!("double {x}, {}", at(span))), Kind::Float),
            // The C library rounds, and the runtime turns the float that
            // comes back into an int.
            "floor" | "ceil" => {
                let rounded = self.call("double", if name == "floor" { "floor" } else { "ceil" }, format!("double {x}"));
                Val::new(self.call("i64", "zeno_whole", format!("double {rounded}")), Kind::Int)
            }
            // C's `round` sends a half away from zero, as Zeno's does.
            "round" => {
                let rounded = self.call("double", "round", format!("double {x}"));
                Val::new(self.call("i64", "zeno_whole", format!("double {rounded}")), Kind::Int)
            }
            "sin" => Val::new(self.call("double", "sin", format!("double {x}")), Kind::Float),
            "cos" => Val::new(self.call("double", "cos", format!("double {x}")), Kind::Float),
            "tan" => Val::new(self.call("double", "tan", format!("double {x}")), Kind::Float),
            "exp" => Val::new(self.call("double", "exp", format!("double {x}")), Kind::Float),
            "atan" => Val::new(self.call("double", "atan", format!("double {x}")), radians),
            // These two refuse a number that is the sine of no angle.
            "asin" => Val::new(self.call("double", "zeno_asin", format!("double {x}, {}", at(span))), radians),
            "acos" => Val::new(self.call("double", "zeno_acos", format!("double {x}, {}", at(span))), radians),
            // And these a number that has no logarithm.
            "log" | "log2" | "log10" => {
                let base = match name {
                    "log2" => 2,
                    "log10" => 10,
                    _ => 0,
                };
                let args = format!("double {x}, i32 signext {base}, {}", at(span));
                Val::new(self.call("double", "zeno_log", args), Kind::Float)
            }
            _ => return Err(cannot(format!("`math.{name}`"), span)),
        })
    }

    /// A call to one of the functions Zeno comes with.
    fn builtin(&mut self, name: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        if args.iter().any(|arg| arg.name.is_some()) {
            return Err(cannot(format!("`{name}` with a named argument"), span));
        }
        // `allocate(u8, 16)`: sixteen bytes, all zero. The runtime takes
        // them from the heap, or from the pool where there is no
        // collector. It is told how many items, and the bytes in one.
        if name == "allocate" {
            let (pointee, count) = written_type("allocate", args, span)?;
            let count = self.expr(count)?;
            if count.kind != Kind::Int {
                return Err(cannot("`allocate` of a number of items that is not an int", span));
            }
            let args = format!("i64 {}, i64 {}, {}", count.code, pointee.bytes(), at(span));
            return Ok(Val::new(self.call("ptr", "zeno_allocate", args), Kind::Pointer(pointee)));
        }
        let mut vals = Vec::new();
        for arg in args {
            vals.push(self.expr(&arg.value)?);
        }
        let wrong = || cannot(format!("`{name}` used this way"), span);
        Ok(match (name, vals.as_slice()) {
            // `free(p)`: back to the heap, or to the pool.
            ("free", [pointer]) if matches!(pointer.kind, Kind::Pointer(_)) => {
                self.run("zeno_release", format!("ptr {}", pointer.code));
                Val::nothing()
            }
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
            // `u8(n)`: `n` itself, once the runtime has seen that it fits.
            // The size goes along in words, for the message.
            (sized, [n]) if ints::width(sized).is_some() && n.kind == Kind::Int => {
                let width = ints::width(sized).unwrap_or(ints::WIDTHS[0]);
                let said = self.string(&format!("{} {sized}", if width.signed { "an" } else { "a" }));
                let args = format!("i64 {}, i64 {}, i64 {}, ptr {said}, {}", n.code, width.min(), width.max(), at(span));
                Val::new(self.call("i64", "zeno_fit", args), Kind::Int)
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
            // A dict starts with its list of keys, and has as many
            // entries as that list has items.
            ("len", [val]) if matches!(val.kind, Kind::Dict(..)) => {
                let keys = self.set(format!("load ptr, ptr {}", val.code));
                Val::new(self.set(format!("load i64, ptr {keys}")), Kind::Int)
            }
            // `char(65)`: the runtime writes the bytes of that character.
            // `input()`, `input("Name: ")`: the runtime reads a line, and
            // gives null, which is `none`, when there are no more.
            ("input", []) | ("input", [Val { kind: Kind::Str, .. }]) => {
                let question = vals.first().map_or("null", |val| val.code.as_str());
                self.code.built = !self.collector;
                let line = Val::new(self.call("ptr", "zeno_input", format!("ptr {question}")), Kind::Optional(Box::new(Kind::Str)));
                self.came_back(line)
            }
            ("char", [val]) if val.kind == Kind::Int => {
                Val::new(self.call("ptr", "zeno_char", format!("i64 {}, {}", val.code, at(span))), Kind::Str)
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
                // Text may not be a number at all, so this one can raise.
                Kind::Str if self.firmware => {
                    // The runtime builds text of its own on the way.
                    self.code.built = !self.collector;
                    self.uses("zeno_parse_int");
                    let call = format!("call zeroext i1 @zeno_parse_int(ptr {}, {}", val.code, at(span));
                    self.checked_call(call, &Kind::Int, &["ValueError".to_string()], span)?
                }
                Kind::Str => {
                    Val::new(self.call("i64", "zeno_parse_int", format!("ptr {}, {}", val.code, at(span))), Kind::Int)
                }
                _ => return Err(wrong()),
            },
            ("float", [val]) => match val.kind {
                Kind::Int | Kind::Float => Val::new(self.double(val), Kind::Float),
                Kind::Str if self.firmware => {
                    self.code.built = !self.collector;
                    self.uses("zeno_parse_float");
                    let call = format!("call zeroext i1 @zeno_parse_float(ptr {}, {}", val.code, at(span));
                    self.checked_call(call, &Kind::Float, &["ValueError".to_string()], span)?
                }
                Kind::Str => Val::new(
                    self.call("double", "zeno_parse_float", format!("ptr {}, {}", val.code, at(span))),
                    Kind::Float,
                ),
                _ => return Err(wrong()),
            },
            // `sum`, `min` and `max` of one list of numbers.
            ("sum" | "min" | "max", [Val { kind: Kind::List(item), code, .. }]) => {
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

    /// The size called `name`, if `name` is still the built-in one and not
    /// something of the program's own.
    fn sized_name(&self, name: &str) -> Option<Width> {
        let taken = self.functions.contains_key(name) || self.classes.contains_key(name) || self.enums.contains_key(name);
        ints::width(name).filter(|_| !taken)
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
                list.extend(self.bind(&slots, &entry.owned[1..], &entry.lends[1..], args, span)?);
                self.call_entry(&entry, list, span)
            }
            // A register of the board: one call into the runtime, with
            // the address and how many bits wide the register is.
            Kind::Register(width) => {
                let register = format!("i64 {}, i32 signext {}", receiver.code, width.bits);
                match (name, args) {
                    ("read", []) => Ok(Val::new(self.call("i64", "zeno_register_read", register), Kind::Int)),
                    ("write", [Arg { name: None, value }]) => {
                        let value = self.expr(value)?;
                        if value.kind != Kind::Int {
                            return Err(cannot("`write` on a register with anything but a whole number", span));
                        }
                        self.run("zeno_register_write", format!("{register}, i64 {}", value.code));
                        Ok(Val::nothing())
                    }
                    _ => Err(cannot(format!("`{name}` on a register called this way"), span)),
                }
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
                // Two that do not take text: `slice` takes positions,
                // and `code` takes nothing.
                match (name, args) {
                    ("slice", [_]) | ("slice", [_, _]) => {
                        let ends = self.slice_ends(args, span)?;
                        let slice = format!("ptr {}, {ends}, {}", receiver.code, at(span));
                        return Ok(Val::new(self.call("ptr", "zeno_slice", slice), Kind::Str));
                    }
                    ("code", []) => {
                        let text = format!("ptr {}, {}", receiver.code, at(span));
                        return Ok(Val::new(self.call("i64", "zeno_char_code", text), Kind::Int));
                    }
                    _ => {}
                }
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
                    // The runtime answers -1 for "not there".
                    ("find", 1) => {
                        let at = self.call("i64", "zeno_find", list);
                        return Ok(self.found(&at));
                    }
                    ("split", 0) => ("zeno_split_blanks", "ptr", words),
                    ("lines", 0) => ("zeno_lines", "ptr", words),
                    ("split", 1) => {
                        list += &format!(", {}", at(span));
                        ("zeno_split", "ptr", words)
                    }
                    _ => return Err(wrong()),
                };
                // The list of pieces is new, and nobody's yet.
                let val = Val::new(self.call(returns, function, list), kind);
                Ok(if name == "split" || name == "lines" { val.made() } else { val })
            }
            Kind::List(item) => match (name, args) {
                ("map" | "filter" | "reduce", _) => self.over_list(name, &receiver, &item, args, span),
                ("append", [arg]) if arg.name.is_none() => {
                    let value = self.expr(&arg.value)?;
                    self.stays(&value, span)?;
                    self.fits_a_slot(&item, span)?;
                    let value = self.keep(value, is_written(&arg.value));
                    let value = self.fit(value, &item, span)?;
                    let slot = self.to_slot(&value, span)?;
                    self.run("zeno_list_append", format!("ptr {}, i64 {slot}", receiver.code));
                    Ok(Val::nothing())
                }
                ("pop", []) | ("pop", [Arg { name: None, .. }]) => {
                    let slot = match args.first() {
                        // `pop(2)`: the item at that position.
                        Some(position) => {
                            let position = self.expr(&position.value)?;
                            if position.kind != Kind::Int {
                                return Err(cannot("`pop` with a position that is not an int", span));
                            }
                            let pop = format!("ptr {}, i64 {}, {}", receiver.code, position.code, at(span));
                            self.call("i64", "zeno_list_pop_at", pop)
                        }
                        None => self.call("i64", "zeno_list_pop", format!("ptr {}, {}", receiver.code, at(span))),
                    };
                    let val = self.from_slot(&item, &slot, span)?;
                    if self.collector || !is_text(&item) {
                        return Ok(val);
                    }
                    // The list is done with it: the statement gets it,
                    // and the list's copy is freed.
                    Ok(Val::new(self.call("ptr", "zeno_text_take", format!("ptr {}", val.code)), val.kind))
                }
                // The list keeps what it is given, as `append` does.
                ("insert", [Arg { name: None, value: position }, arg]) if arg.name.is_none() => {
                    let position = self.expr(position)?;
                    if position.kind != Kind::Int {
                        return Err(cannot("`insert` with a position that is not an int", span));
                    }
                    let value = self.expr(&arg.value)?;
                    self.stays(&value, span)?;
                    self.fits_a_slot(&item, span)?;
                    let value = self.keep(value, is_written(&arg.value));
                    let value = self.fit(value, &item, span)?;
                    let slot = self.to_slot(&value, span)?;
                    let insert = format!("ptr {}, i64 {}, i64 {slot}, {}", receiver.code, position.code, at(span));
                    self.run("zeno_list_insert", insert);
                    Ok(Val::nothing())
                }
                ("reverse", []) => {
                    self.run("zeno_list_reverse", format!("ptr {}", receiver.code));
                    Ok(Val::nothing())
                }
                // The runtime compares two slots, and has to be told how
                // to read one: 0 a whole number, 1 a float, 2 text.
                ("sort", []) => {
                    let kind = match &*item {
                        Kind::Int => 0,
                        Kind::Float => 1,
                        Kind::Str => 2,
                        _ => return Err(cannot("`sort` on a list of anything but numbers or text", span)),
                    };
                    self.run("zeno_list_sort", format!("ptr {}, i32 {kind}", receiver.code));
                    Ok(Val::nothing())
                }
                // Items that have one owner (and, with no collector, text,
                // which the list keeps copies of) end with the list.
                // `clear` is the end of each of them: a loop over the
                // items that frees each, and then the call.
                ("clear", []) => {
                    if self.drops.owns(&item) || (!self.collector && is_text(&item)) {
                        let (count, slot) = self.walk_open("item", &receiver.code);
                        let each = self.from_slot(&item, &slot, span)?;
                        self.free(&item, &each.code, span)?;
                        self.count_close(&count);
                    }
                    self.run("zeno_list_clear", format!("ptr {}", receiver.code));
                    Ok(Val::nothing())
                }
                ("find", [arg]) if arg.name.is_none() => {
                    let same = self.eq_adapter(&item, span)?;
                    let value = self.expr(&arg.value)?;
                    let value = self.coerce(value, &item, span)?;
                    let slot = self.to_slot(&value, span)?;
                    let at = self.call("i64", "zeno_list_find", format!("ptr {}, i64 {slot}, ptr {same}", receiver.code));
                    Ok(self.found(&at))
                }
                ("slice", [_]) | ("slice", [_, _]) => {
                    let ends = self.slice_ends(args, span)?;
                    let mut slice = self.call("ptr", "zeno_list_slice", format!("ptr {}, {ends}, {}", receiver.code, at(span)));
                    if !self.collector && is_text(&item) {
                        // With no collector a list of text has its own
                        // copy of each item, and so has the slice.
                        slice = self.call("ptr", "zeno_list_keep_text", format!("ptr {slice}"));
                    }
                    // The new list is nobody's yet.
                    Ok(Val::new(slice, Kind::List(item.clone())).made())
                }
                ("join", [arg]) if arg.name.is_none() && *item == Kind::Str => {
                    let separator = self.expr(&arg.value)?;
                    if separator.kind != Kind::Str {
                        return Err(cannot("`join` with a separator that is not a str", span));
                    }
                    let join = format!("ptr {}, ptr {}", receiver.code, separator.code);
                    Ok(Val::new(self.call("ptr", "zeno_list_join", join), Kind::Str))
                }
                _ => Err(cannot(format!("the list method `{name}` called this way"), span)),
            },
            Kind::Dict(key, value) => match (name, args) {
                // `get(key)`: the value, or `none`. The runtime says
                // where the key is, or -1, and then gives the value
                // there (zeros for -1). Text that is not there is null
                // already. A number or a bool gets its flag here:
                //
                //   %t3 = call i64 @zeno_dict_find(ptr %t1, i64 %t2)
                //   %t4 = call i64 @zeno_dict_value(ptr %t1, i64 %t3)
                //   %t5 = icmp sge i64 %t3, 0                        ; found?
                //   %t6 = insertvalue { i1, i64 } undef, i1 %t5, 0
                //   %t7 = insertvalue { i1, i64 } %t6, i64 %t4, 1
                ("get", [wanted]) if wanted.name.is_none() => {
                    let wanted = self.expr(&wanted.value)?;
                    let slot = self.to_slot(&wanted, span)?;
                    let at = self.call("i64", "zeno_dict_find", format!("ptr {}, i64 {slot}", receiver.code));
                    let found = self.call("i64", "zeno_dict_value", format!("ptr {}, i64 {at}", receiver.code));
                    let inside = self.from_slot(&value, &found, span)?;
                    let kind = Kind::Optional(value.clone());
                    if is_pointer(&value) {
                        return Ok(self.lend(Val::new(inside.code, kind)));
                    }
                    let present = self.set(format!("icmp sge i64 {at}, 0"));
                    let (pair, ty) = (ir_type(&kind, span)?, ir_type(&value, span)?);
                    let flagged = self.set(format!("insertvalue {pair} undef, i1 {present}, 0"));
                    let whole = self.set(format!("insertvalue {pair} {flagged}, {ty} {}, 1", inside.code));
                    Ok(Val::new(whole, kind))
                }
                // `get(key, instead)`.
                ("get", [wanted, instead]) if wanted.name.is_none() && instead.name.is_none() => {
                    let wanted = self.expr(&wanted.value)?;
                    let instead = self.expr(&instead.value)?;
                    let instead = self.fit(instead, &value, span)?;
                    let slot = self.to_slot(&wanted, span)?;
                    let other = self.to_slot(&instead, span)?;
                    let get = format!("ptr {}, i64 {slot}, i64 {other}", receiver.code);
                    let found = self.call("i64", "zeno_dict_get", get);
                    let val = self.from_slot(&value, &found, span)?;
                    Ok(self.lend(val))
                }
                ("pop", [wanted]) if wanted.name.is_none() => {
                    let wanted = self.expr(&wanted.value)?;
                    let slot = self.to_slot(&wanted, span)?;
                    let pop = format!("ptr {}, i64 {slot}, {}", receiver.code, at(span));
                    let found = self.call("i64", "zeno_dict_pop", pop);
                    let val = self.from_slot(&value, &found, span)?;
                    if self.collector || !is_text(&value) {
                        return Ok(val);
                    }
                    // The dict is done with it: the statement gets it,
                    // and the dict's copy is freed.
                    Ok(Val::new(self.call("ptr", "zeno_text_take", format!("ptr {}", val.code)), val.kind))
                }
                // A new list each time, which is nobody's yet.
                ("keys", []) => {
                    let keys = self.call("ptr", "zeno_dict_keys", format!("ptr {}", receiver.code));
                    Ok(Val::new(keys, Kind::List(key.clone())).made())
                }
                ("values", []) => {
                    let values = self.call("ptr", "zeno_dict_values", format!("ptr {}", receiver.code));
                    Ok(Val::new(values, Kind::List(value.clone())).made())
                }
                ("clear", []) => {
                    self.run("zeno_dict_clear", format!("ptr {}", receiver.code));
                    Ok(Val::nothing())
                }
                _ => Err(cannot(format!("the dict method `{name}` called this way"), span)),
            },
            kind => Err(cannot(format!("the method `{name}` of {}", described(&kind)), span)),
        }
    }

    /// The arguments of `slice(from)` or `slice(from, to)`, as the runtime
    /// takes them: where it begins, where it ends, and whether no end
    /// was written (and it goes on to the last item).
    fn slice_ends(&mut self, args: &[Arg], span: Span) -> Result<String, Error> {
        let mut positions = Vec::new();
        for arg in args {
            let position = self.expr(&arg.value)?;
            if position.kind != Kind::Int || arg.name.is_some() {
                return Err(cannot("`slice` with a position that is not an int", span));
            }
            positions.push(position.code);
        }
        Ok(match positions.as_slice() {
            [from] => format!("i64 {from}, i64 0, i1 true"),
            [from, to] => format!("i64 {from}, i64 {to}, i1 false"),
            _ => return Err(cannot("`slice` called this way", span)),
        })
    }

    /// What `find` found, as an `int or none`. The runtime answers with
    /// a position, or -1 when the thing is not there:
    ///
    /// ```text
    ///   %t2 = icmp sge i64 %t1, 0                           ; found?
    ///   %t3 = select i1 %t2, i64 %t1, i64 0
    ///   %t4 = insertvalue { i1, i64 } undef, i1 %t2, 0      ; the flag
    ///   %t5 = insertvalue { i1, i64 } %t4, i64 %t3, 1       ; the position
    /// ```
    fn found(&mut self, at: &str) -> Val {
        let present = self.set(format!("icmp sge i64 {at}, 0"));
        let position = self.set(format!("select i1 {present}, i64 {at}, i64 0"));
        let flagged = self.set(format!("insertvalue {{ i1, i64 }} undef, i1 {present}, 0"));
        let pair = self.set(format!("insertvalue {{ i1, i64 }} {flagged}, i64 {position}, 1"));
        Val::new(pair, Kind::Optional(Box::new(Kind::Int)))
    }

    // ---------------------------------------------------- Objects and enums

    /// `Point(3.0, 4.0)`: a new object.
    fn construct(&mut self, class: &str, args: &[Arg], span: Span) -> Result<Val, Error> {
        let (slots, owned) = (self.classes[class].fields.clone(), self.classes[class].owned_fields.clone());
        let list = self.bind(&slots, &owned, &[], args, span)?;
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
                let mut value = format!("%f{index}");
                if !emitter.collector && is_text(kind) {
                    // With no collector the object has a copy of its own.
                    value = emitter.call("ptr", "zeno_text_keep", format!("ptr {value}"));
                }
                emitter.store_field(class, &object, index, &value, span)?;
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
        let fields = self.classes[class].fields.clone();
        // A `Stack[int]` prints as `Stack(...)`, as `zeno run` prints it.
        let shown_as = crate::ast::shown_name(class);
        self.side(format!("define internal ptr {name}(ptr %self)"), |emitter| {
            let mut pieces = Vec::new();
            for (index, (field, kind, _)) in fields.iter().enumerate() {
                let label = if index == 0 { format!("{shown_as}({field}=") } else { format!(", {field}=") };
                pieces.push(emitter.string(&label));
                let value = emitter.load_field(class, "%self", index, span)?;
                pieces.push(emitter.repr(&Val::new(value, kind.clone()), span)?);
            }
            if fields.is_empty() {
                pieces.push(emitter.string(&format!("{shown_as}(")));
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
        let fields = self.classes[class].fields.clone();
        self.side(format!("define internal i1 {name}(ptr %a, ptr %b)"), |emitter| {
            for (index, (_, kind, _)) in fields.iter().enumerate() {
                let a = emitter.load_field(class, "%a", index, span)?;
                let b = emitter.load_field(class, "%b", index, span)?;
                emitter.same_or_differ(kind, &a, &b, &format!("more{index}"), span)?;
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
        self.same_or_differ(kind, &a, &b, more, span)
    }

    /// Leave for `differ` unless `a` and `b`, two values of `kind`, are
    /// equal. Otherwise carry on at `more`.
    fn same_or_differ(&mut self, kind: &Kind, a: &str, b: &str, more: &str, span: Span) -> Result<(), Error> {
        let same = self.equal(kind, a, b, span)?;
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
        // Which of the values it carries have one owner: the new value
        // takes those over.
        let takes = self.enums[owner].takes[index].clone();
        let slots: Slots = fields.iter().map(|(name, kind)| (name.clone(), kind.clone(), None)).collect();
        let list = self.bind(&slots, &takes, &[], args, span)?;
        // With no collector the value has its own copy of each text.
        let texts = !self.collector;

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
                    let carried = match texts && is_text(kind) {
                        true => emitter.call("ptr", "zeno_text_keep", format!("ptr %f{position}")),
                        false => format!("%f{position}"),
                    };
                    emitter.ins(format!("store {} {carried}, ptr {at}", ir_type(kind, span)?));
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
                // A case of an imported file prints by the name its
                // file gave it: `Dot`, not `shapes.Dot`.
                let shown_as = shown_name(case);
                if fields.is_empty() {
                    let text = emitter.string(shown_as);
                    emitter.end(format!("ret ptr {text}"));
                    continue;
                }
                let ty = case_type(owner, case);
                let mut pieces = Vec::new();
                for (position, (_, kind)) in fields.iter().enumerate() {
                    pieces.push(emitter.string(&if position == 0 { format!("{shown_as}(") } else { ", ".to_string() }));
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
        if self.firmware && !entry.decl.raises.is_empty() {
            return Err(raises_as_value(span));
        }
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
            self.globals.push(match self.collector {
                true => format!("{value} = internal constant {{ ptr, ptr }} {{ ptr {wrap}, ptr {text} }}"),
                // With no collector a function value says what frees
                // it. Nothing frees this one: it is not in a block.
                false => {
                    format!("{value} = internal constant {{ ptr, ptr, ptr }} {{ ptr {wrap}, ptr {text}, ptr null }}")
                }
            });
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
        let owned: Vec<bool> = kind.params.iter().map(|param| matches!(param, Kind::Owned(_))).collect();
        let kind = FunctionKind { params: kind.params.iter().map(shared).collect(), returns: shared(&kind.returns) };
        if self.firmware && !decl.raises.is_empty() {
            return Err(raises_as_value(span));
        }
        self.closures += 1;
        let code_name = format!("@zeno.fn.{}", self.closures);
        let drop_name = format!("@zeno.fn.drop.{}", self.closures);

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
                let mut variable = variable.clone();
                if !self.collector {
                    // With no collector nothing is shared. The function
                    // takes what the variable holds at this moment: a
                    // number, or the address of a list or an object that
                    // it will only look at.
                    if let Place::Slot(slot) = &variable.place {
                        let ty = ir_type(&variable.kind, span)?;
                        variable.place = Place::Value(self.load(ty, slot));
                    }
                    variable.owner = false;
                }
                taken.push((name, variable));
            }
        }
        // Every function value begins with its code and its name. With
        // no collector, what frees it comes third.
        let mut inside = vec!["ptr", "ptr"];
        if !self.collector {
            inside.push("ptr");
        }
        let first = inside.len();
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
        // With no collector the function has its own copy of any text
        // it uses, which goes when the block goes.
        let texts: Vec<usize> = taken
            .iter()
            .enumerate()
            .filter(|(_, (_, variable))| !self.collector && is_text(&variable.kind))
            .map(|(index, _)| index + first)
            .collect();
        for (index, (_, variable)) in taken.iter().enumerate() {
            let at = self.field_at(&block, &made, index + first);
            match &variable.place {
                Place::Value(code) if texts.contains(&(index + first)) => {
                    let kept = self.call("ptr", "zeno_text_keep", format!("ptr {code}"));
                    self.ins(format!("store ptr {kept}, ptr {at}"));
                }
                Place::Value(code) => self.ins(format!("store {} {code}, ptr {at}", inside[index + first])),
                Place::Slot(boxed) => self.ins(format!("store ptr {boxed}, ptr {at}")),
            }
        }
        if !self.collector {
            let drop_at = self.field_at(&block, &made, 2);
            if texts.is_empty() {
                self.uses("zeno_closure_release");
                self.ins(format!("store ptr @zeno_closure_release, ptr {drop_at}"));
            } else {
                let block = block.clone();
                self.side(format!("define internal void {drop_name}(ptr %self)"), |emitter| {
                    for index in &texts {
                        let at = emitter.field_at(&block, "%self", *index);
                        let text = emitter.set(format!("load ptr, ptr {at}"));
                        emitter.run("zeno_text_drop", format!("ptr {text}"));
                    }
                    emitter.run("zeno_free", "ptr %self");
                    emitter.end("ret void");
                    Ok(())
                })?;
                self.ins(format!("store ptr {drop_name}, ptr {drop_at}"));
            }
        }

        // Set aside the function being written, and write the new one.
        let outer = std::mem::replace(&mut self.code, Code::of(&decl.body, kind.returns.clone(), true, !self.firmware));
        let mut outside = std::mem::take(&mut self.scopes);
        let globals = outside.remove(0);
        // It begins by taking its variables out of the block.
        let mut brought = HashMap::new();
        for (index, (name, variable)) in taken.iter().enumerate() {
            let at = self.field_at(&block, "%self", index + first);
            let place = match &variable.place {
                Place::Value(_) => Place::Value(self.set(format!("load {}, ptr {at}", inside[index + first]))),
                Place::Slot(_) => Place::Slot(self.set(format!("load ptr, ptr {at}"))),
            };
            brought.insert(name.clone(), Variable { place, kind: variable.kind.clone(), open: variable.open, owner: false });
        }
        let (mut params, list) = self.parameters(decl, &kind, span)?;
        // A parameter that takes over a value is its owner, as in a
        // function of the program: see `function`.
        for (index, param) in decl.params.iter().enumerate() {
            if owned.get(index) == Some(&true) {
                let kind = kind.params[index].clone();
                let ty = ir_type(&kind, span)?;
                let slot = self.stack_slot(&param.name, ty);
                self.store(ty, &named('%', "arg.", &param.name), &slot);
                params.insert(param.name.clone(), Variable { place: Place::Slot(slot), kind, open: false, owner: true });
            }
        }
        self.scopes = vec![globals, brought, params];
        let mut done = self.block(&decl.body);
        if done.is_ok() && !self.code.ended {
            // The end of the body: free what the parameters still own.
            done = self.free_owned(1);
        }

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
        // What may be given to each parameter, and what it does with
        // it, is the checker's to say: the function's type tells it.
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
            returns => {
                let val = Val::new(self.set(call), returns.clone());
                self.came_back(val)
            }
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
        // With no collector a list of text is walked as a `for` loop
        // walks one: a copy with copies of the items, which the function
        // that is called cannot pull away.
        let texts = !self.collector && is_text(item);
        let copy = if texts { "zeno_list_copy_text" } else { "zeno_list_copy" };
        // A list that the statement owns is out of every function's
        // reach, so it is walked as it is.
        let own = list.passing;
        let items = if own { list.code.clone() } else { self.call("ptr", copy, format!("ptr {}", list.code)) };
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

        // With no collector the text each call of `map` gives back is in
        // the text room. The new list takes a copy of its own, and the
        // room is given back before the next call.
        let mark = match name == "map" && !self.collector && is_text(&kind.returns) {
            true => {
                self.code.marks += 1;
                let mark = format!("%mark.{}", self.code.marks);
                self.ins(format!("{mark} = load i64, ptr @zeno_text_top"));
                Some(mark)
            }
            false => None,
        };
        let (count, slot) = self.walk_open(name, &items);
        let each = self.from_slot(item, &slot, span)?;
        let result = match (name, &total) {
            ("map", _) => {
                let mut made = self.invoke(&function, &kind, vec![each], span)?;
                if mark.is_some() {
                    made.code = self.call("ptr", "zeno_text_keep", format!("ptr {}", made.code));
                }
                let slot = self.to_slot(&made, span)?;
                self.run("zeno_list_append", format!("ptr {out}, i64 {slot}"));
                if let Some(mark) = &mark {
                    self.text_back(mark);
                }
                Val::new(out, Kind::List(Box::new(kind.returns.clone()))).made()
            }
            ("filter", _) => {
                let wanted = self.invoke(&function, &kind, vec![each.clone()], span)?;
                let keep = format!("keep{}", self.number());
                self.jump_if(&wanted.code, &keep, &count.next);
                self.place(&keep);
                // With no collector the new list has its own copy of each
                // text it takes.
                let taken = match texts {
                    true => {
                        let kept = self.call("ptr", "zeno_text_keep", format!("ptr {}", each.code));
                        self.to_slot(&Val::new(kept, each.kind.clone()), span)?
                    }
                    false => slot,
                };
                self.run("zeno_list_append", format!("ptr {out}, i64 {taken}"));
                Val::new(out, Kind::List(Box::new(item.clone()))).made()
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
        if !own {
            // The copy that was walked was nobody else's.
            // Its items, if they have one owner each, are still the
            // first list's.
            self.run(if texts { "zeno_list_drop_text" } else { "zeno_list_drop" }, format!("ptr {items}"));
        }
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
        // Anything but text has to be turned into text, which builds some.
        if val.kind != Kind::Str && !self.collector {
            self.code.built = true;
        }
        Ok(match &val.kind {
            Kind::Str => val.code.clone(),
            Kind::Int => self.call("ptr", "zeno_str_int", format!("i64 {}", val.code)),
            Kind::Float => self.call("ptr", "zeno_str_float", format!("double {}", val.code)),
            Kind::Bool => self.call("ptr", "zeno_str_bool", format!("i1 {}", val.code)),
            Kind::Pin => self.call("ptr", "zeno_str_pin", format!("i64 {}", val.code)),
            // Where a pointer points is different on every run, so it
            // is not shown.
            Kind::Pointer(pointee) => self.string(&format!("<pointer[{}]>", pointee.name())),
            Kind::Register(_) => self.call("ptr", "zeno_str_register", format!("i64 {}", val.code)),
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
            // A dict knows what it holds, and shows itself.
            Kind::Dict(..) => self.call("ptr", "zeno_dict_str", format!("ptr {}", val.code)),
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
        if !self.collector {
            self.code.built = true;
        }
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
            Kind::Int | Kind::Pin | Kind::Register(_) => self.set(format!("icmp eq i64 {a}, {b}")),
            // Two pointers are equal when they point at the same item.
            Kind::Pointer(_) => self.set(format!("icmp eq ptr {a}, {b}")),
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
            Kind::Dict(..) => self.call("i1", "zeno_dict_eq", format!("ptr {a}, ptr {b}")),
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
        // The values are worked out left to right, as they are written.
        // A number, a bool or a piece of text shows the same whenever it
        // is shown, so turning it into text waits until every value is
        // known. One of the later values may be a call, and text that is
        // built before the call takes room all the while it runs: with no
        // collector, room in the text room. (A list or an object is shown
        // at once: a later value may be a call that changes it.)
        enum Piece<'a> {
            Text(String),
            Waiting(Val, &'a Option<String>),
        }
        let mut waiting = Vec::new();
        for part in parts {
            waiting.push(match part {
                FormatPart::Text(literal) => Piece::Text(self.string(literal)),
                FormatPart::Value { value, format } => {
                    let val = self.expr(value)?;
                    match val.kind {
                        Kind::Int | Kind::Float | Kind::Bool | Kind::Pin | Kind::Quantity(_) | Kind::Str => {
                            Piece::Waiting(val, format)
                        }
                        _ => Piece::Text(self.shown(&val, format, span)?),
                    }
                }
            });
        }
        let mut pieces = Vec::new();
        for piece in waiting {
            pieces.push(match piece {
                Piece::Text(text) => text,
                Piece::Waiting(val, format) => self.shown(&val, format, span)?,
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

    /// One value of an f-string as text, with its format if it has one.
    fn shown(&mut self, val: &Val, format: &Option<String>, span: Span) -> Result<String, Error> {
        match format {
            Some(spec) => self.formatted(val, spec, span),
            None => self.text(val, span),
        }
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
        let program = crate::modules::testing::program(source);
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        emit(&program, &types, "test.zeno", Build::Ordinary, false).map_err(|error| error.message)
    }

    /// The same, as `zeno build --llvm --firmware` compiles it.
    fn compile_firmware(source: &str) -> Result<String, String> {
        let program = crate::modules::testing::program(source);
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        emit(&program, &types, "test.zeno", Build::Firmware, false).map_err(|error| error.message)
    }

    /// Build the IR and the runtime with clang, run the program, and
    /// return what it printed, what it wrote as errors, and whether it
    /// finished cleanly. `None` if there is no clang here to try it with.
    fn build_and_run(name: &str, ir: &str, firmware: bool) -> Option<(String, String, bool)> {
        let build = if firmware { Build::Firmware } else { Build::Ordinary };
        // ZENO_COLLECT_ALWAYS runs the garbage collector before every
        // single allocation, which is the setting most likely to show a
        // value the collector cannot find.
        build_and_run_as(name, ir, build, &["-DZENO_COLLECT_ALWAYS"])
    }

    /// The same, for any of the three kinds of build, with these flags
    /// for clang beside the usual ones.
    fn build_and_run_as(name: &str, ir: &str, build: Build, flags: &[&str]) -> Option<(String, String, bool)> {
        let dir = std::env::temp_dir().join(format!("zeno-test-{}-llvm-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        let (ir_file, runtime_file, binary) = (dir.join("test.ll"), dir.join("test_runtime.c"), dir.join("test"));
        std::fs::write(&ir_file, ir).expect("the IR file is written");
        std::fs::write(&runtime_file, runtime(build)).expect("the runtime is written");
        let built = Command::new("clang")
            .args(["-O2", "-Wno-override-module"])
            .args(flags)
            .arg("-o")
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
        // Set `ZENO_VALGRIND` when running the tests, and each program
        // built with no collector is run under valgrind, which stops a
        // program that frees a block twice, uses one it has freed, or
        // ends with one it never freed. (Not the other builds: a
        // collector reads the whole stack, which valgrind objects to.)
        let ran = match build == Build::NoCollector && std::env::var_os("ZENO_VALGRIND").is_some() {
            true => crate::value::testing::run(
                Command::new("valgrind").args(["-q", "--error-exitcode=99", "--leak-check=full"]).arg(&binary),
            ),
            false => crate::value::testing::run(&mut Command::new(&binary)),
        }
        .expect("the program runs");
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
        let program = crate::modules::testing::program(source);
        let (interpreted, result) = crate::vm::run_both(&program, source);

        // Both kinds of build must agree with the interpreter: the
        // ordinary one, where errors jump, and `--firmware`, where they
        // travel as values.
        for firmware in [false, true] {
            let ir = if firmware {
                match compile_firmware(source) {
                    Ok(ir) => ir,
                    // A firmware build refuses a few things on purpose,
                    // and says so. Any other refusal is a failure.
                    Err(message) => {
                        assert!(message.contains("--firmware"), "the firmware build failed: {message}\n{source}");
                        continue;
                    }
                }
            } else {
                compile(source).expect("the test program compiles")
            };
            let name = if firmware { format!("{name}-firmware") } else { name.to_string() };
            let Some((compiled, errors, finished)) = build_and_run(&name, &ir, firmware) else {
                return;
            };
            assert_eq!(compiled, interpreted, "\nfirmware: {firmware}\n{source}");
            match &result {
                Ok(()) => assert!(finished, "the compiled program failed (firmware: {firmware}): {errors}"),
                Err(error) => {
                    assert!(!finished, "the compiled program should have failed");
                    let place = crate::modules::testing::place(&program, error.span);
                    let expected = format!("error: {}\n --> {place}\n", error.message);
                    assert_eq!(errors, expected, "\nfirmware: {firmware}\n{source}");
                }
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
        same_as_interpreter("packet", include_str!("../examples/packet.zeno"));
        same_as_interpreter("generics", include_str!("../examples/generics.zeno"));
        same_as_interpreter("labels", include_str!("../examples/labels.zeno"));
        same_as_interpreter("words", include_str!("../examples/words.zeno"));
        same_as_interpreter("scheduler", include_str!("../examples/scheduler.zeno"));
        same_as_interpreter("commands", include_str!("../examples/commands.zeno"));
        same_as_interpreter("handlers", include_str!("../examples/handlers.zeno"));
        same_as_interpreter("driver", include_str!("../examples/driver.zeno"));
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
        let Some((printed, errors, finished)) = build_and_run("gone", &ir, false) else {
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

    // ---------------------------------------------- Errors as values (firmware)

    /// `same_as_interpreter`, for a program that a firmware build must
    /// accept, so that the firmware half of the comparison really runs.
    pub(crate) fn firmware_as_interpreter(name: &str, source: &str) {
        if let Err(message) = compile_firmware(source) {
            panic!("the firmware build through LLVM refused this: {message}");
        }
        same_as_interpreter(name, source);
    }

    #[test]
    fn a_firmware_build_has_nothing_to_jump_with() {
        let source = include_str!("../examples/sensor.zeno");
        let ir = compile_firmware(source).expect("it compiles for firmware");
        for word in ["setjmp", "zeno_try", "zeno_raise(", "zeno_raise_again", "zeno_caught", "volatile"] {
            assert!(!ir.contains(word), "`{word}` is in the IR of a firmware build");
        }
        // A function that can raise says whether it worked, and gives its
        // answer through a pointer. `none` is an answer like any other.
        assert!(ir.contains("define internal zeroext i1 @z_parse_line(ptr %arg.line, ptr %result) {"), "{ir}");
        assert!(ir.contains("  store { i1, double } zeroinitializer, ptr %result\n  ret i1 true\n"), "{ir}");
        // `float(text)` can fail, so what it gives back is tested, and a
        // failure is passed straight on to the caller.
        assert!(
            ir.contains(
                "  %t4 = call zeroext i1 @zeno_parse_float(ptr %t1, i32 19, i32 17, ptr %answer.2)\n  \
                 br i1 %t4, label %worked3, label %failed3\n\nfailed3:\n  ret i1 false\n\nworked3:\n  \
                 %t5 = load double, ptr %answer.2\n"
            ),
            "{ir}"
        );
        // `raise` notes the error and gives back 0.
        assert!(ir.contains("  call void @zeno_note(ptr %t13, i32 21, i32 9)\n  ret i1 false\n"), "{ir}");
        assert!(ir.contains("declare zeroext i1 @zeno_parse_float(ptr, i32 signext, i32 signext, ptr)"), "{ir}");
        // A function that cannot raise is compiled exactly as before.
        let ordinary = compile(source).expect("it compiles");
        let in_range = |ir: &str| {
            let from = ir.find("define internal ptr @z_in_range(").expect("in_range is there");
            ir[from..].split("\n}\n").next().expect("its end").to_string()
        };
        assert_eq!(in_range(&ir), in_range(&ordinary));
    }

    #[test]
    fn the_ir_for_a_try_in_a_firmware_build() {
        let ir = compile_firmware(
            "function parse(text: str) -> int raises ValueError {\n    return int(text)\n}\n\
             function safe(text: str) -> int {\n    try {\n        return parse(text)\n    } except ValueError as e {\n        return -1\n    }\n}\n\
             print(safe(\"4\"), safe(\"x\"))\nprint(parse(\"7\"))\n",
        )
        .expect("it compiles for firmware");
        // Inside the `try`, a failed call goes to the `except` blocks...
        assert!(ir.contains("failed3:\n  br label %except1\n"), "{ir}");
        // ...where the first question is whether anything failed at all,
        // and the second is whether this `except` wants it.
        assert!(
            ir.contains(
                "except1:\n  %t3 = load ptr, ptr @zeno_raised\n  %t4 = icmp ne ptr %t3, null\n  \
                 br i1 %t4, label %caught1, label %end1\n\ncaught1:\n  store ptr null, ptr @zeno_raised\n  \
                 %t5 = call i1 @zeno_is(ptr %t3, ptr @.str."
            ),
            "{ir}"
        );
        // At the top of the file nobody is left to catch it.
        assert!(ir.contains("  call void @zeno_uncaught()\n  unreachable\n"), "{ir}");
    }

    #[test]
    fn what_a_firmware_build_refuses_says_so() {
        let refused = |source: &str| compile_firmware(source).err().unwrap_or_else(|| "it compiled".to_string());
        // The words are the C backend's: one message for one thing.
        let message = refused("let zero = 0\ntry { print(1 // zero) } except ZeroDivisionError { print(\"no\") }");
        assert!(message.starts_with("nothing in this `try` can be caught. In a firmware build"), "{message}");
        assert!(message.ends_with("Test for it before it happens, or build without `--firmware`"), "{message}");
        let parse = "function parse(text: str) -> int raises ValueError { return int(text) }\n";
        let message = refused(&format!("{parse}try {{ print(parse(\"1\")) }} except IndexError {{ print(\"no\") }}"));
        assert!(message.starts_with("nothing in this `try` can reach `except IndexError`."), "{message}");
        let as_value = "`zeno build --firmware` cannot compile a function that can raise, used as a value, yet. Call it by its name, or build without `--firmware`";
        assert_eq!(refused(&format!("{parse}print([\"1\"].map(parse))")), as_value);
        assert_eq!(
            refused("let f = function(text: str) -> int raises ValueError { return int(text) }\nprint(1)"),
            as_value
        );
        // All of them are fine in an ordinary build.
        assert!(compile(&format!("{parse}print([\"1\"].map(parse))")).is_ok());
    }

    #[test]
    fn a_fault_stops_a_firmware_build_where_an_ordinary_one_catches_it() {
        // The one place the two kinds of build differ, on purpose, and
        // they differ here exactly as the two C builds do.
        let source = "function parse(text: str) -> int raises ValueError { return int(text) }\nlet zero = 0\ntry {\n print(parse(\"1\"))\n print(1 // zero)\n} except {\n print(\"caught\")\n}\ntry { print(parse(\"x\")) } except { print(\"caught too\") }";
        let Some((printed, _, finished)) = build_and_run("fault", &compile(source).unwrap(), false) else {
            return;
        };
        assert_eq!(printed, "1\ncaught\ncaught too\n");
        assert!(finished);
        let (printed, errors, finished) =
            build_and_run("fault-firmware", &compile_firmware(source).unwrap(), true).unwrap();
        assert_eq!(printed, "1\n");
        assert!(!finished);
        assert_eq!(errors, "error: uncaught ZeroDivisionError: division by zero\n --> test.zeno:5:8\n");
    }

    // ------------------------------------------------------- Owned values

    /// A small program with an owned class in it, an owned list, a value
    /// given away and a function that takes one over.
    const FRAME: &str = "owned class Frame {
    id: int
    bytes: owned list[int]
}

function make(id: int) -> Frame {
    let bytes: owned list[int] = [id, id + 1]
    return Frame(id, bytes)
}

function size(frame: take Frame) -> int {
    return len(frame.bytes)
}

let frame = make(7)
print(size(frame))
";

    #[test]
    fn the_ir_for_an_owned_value() {
        let ir = compile(FRAME).expect("it compiles");
        // An object of an owned class frees what its fields own, then
        // itself. Null means it was given away.
        let drop = "define internal void @zeno.drop.Frame(ptr %self) {
entry:
  %t1 = icmp eq ptr %self, null
  br i1 %t1, label %gone1, label %there1

gone1:
  ret void

there1:
  %t2 = getelementptr %z_Frame, ptr %self, i32 0, i32 1
  %t3 = load ptr, ptr %t2
  call void @zeno_list_drop(ptr %t3)
  call void @zeno_free(ptr %self)
  ret void
}";
        assert!(ir.contains(drop), "{ir}");
        // An owner has a slot, even one declared with `let`. Giving the
        // value away leaves null behind, and the end of the block frees
        // what is still there, which by then is nothing.
        let gives = "  store ptr %t4, ptr %var.bytes
  %t5 = load ptr, ptr %var.bytes
  store ptr null, ptr %var.bytes
  %t6 = call ptr @zeno.new.Frame(i64 %arg.id, ptr %t5)
  %t7 = load ptr, ptr %var.bytes
  call void @zeno_list_drop(ptr %t7)
  ret ptr %t6";
        assert!(ir.contains(gives), "{ir}");
        // A function that takes a value over frees it, after the answer
        // has been worked out.
        let takes = "define internal i64 @z_size(ptr %arg.frame) {
entry:
  %var.frame = alloca ptr
  store ptr %arg.frame, ptr %var.frame
  %t1 = load ptr, ptr %var.frame
  %t2 = getelementptr %z_Frame, ptr %t1, i32 0, i32 1
  %t3 = load ptr, ptr %t2
  %t4 = load i64, ptr %t3
  %t5 = load ptr, ptr %var.frame
  call void @zeno.drop.Frame(ptr %t5)
  ret i64 %t4
}";
        assert!(ir.contains(takes), "{ir}");
    }

    /// What a program built with `--memory` says as it ends: the numbers
    /// of its report, in order.
    fn memory_of(name: &str, source: &str) -> Option<(String, Vec<usize>)> {
        let program = crate::modules::testing::program(source);
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        let ir = emit(&program, &types, "test.zeno", Build::Ordinary, true).expect("it compiles");
        let flags = ["-DZENO_MEMORY_REPORT", "-DZENO_FIRST_COLLECTION=16384"];
        let (printed, errors, finished) = build_and_run_as(name, &ir, Build::Ordinary, &flags)?;
        assert!(finished, "the compiled program failed: {errors}");
        let report = errors.lines().find(|line| line.starts_with("memory:")).expect("a memory report");
        let numbers = report
            .split(|c: char| !c.is_ascii_digit())
            .filter(|word| !word.is_empty())
            .map(|word| word.parse().expect("a number"))
            .collect();
        Some((printed, numbers))
    }

    #[test]
    fn owned_values_are_freed_without_the_collector() {
        // Two thousand lists, one after another. Shared, they are left
        // for the collector, which has to run to get the memory back.
        // Owned, each is freed where its block ends, and the collector
        // never has anything to do.
        let work = |list: &str| {
            format!(
                "function work(rounds: int) -> int {{
    var total = 0
    for round in range(rounds) {{
        let scratch: {list} = [round, round + 1, round + 2]
        total += len(scratch)
    }}
    return total
}}
print(work(2000))
"
            )
        };
        let Some((shared_printed, shared)) = memory_of("shared-work", &work("list[int]")) else { return };
        let Some((owned_printed, owned)) = memory_of("owned-work", &work("owned list[int]")) else { return };
        assert_eq!(shared_printed, "6000\n");
        assert_eq!(owned_printed, "6000\n");
        // blocks made, bytes, most in use at once, collections, and for
        // the owned ones, how many were freed without the collector: a
        // list is two blocks.
        assert!(shared[3] > 0 && shared.len() == 4, "{shared:?}");
        assert_eq!((owned[3], owned[4]), (0, 4000), "{owned:?}");
        assert!(owned[2] < shared[2], "owned: {owned:?}, shared: {shared:?}");
    }

    // ---------------------------------------------- With no collector to help

    /// The same, as `zeno build --llvm --no-collector --memory` compiles it.
    fn compile_no_collector(source: &str) -> Result<String, String> {
        let program = crate::modules::testing::program(source);
        let (errors, types) = crate::checker::check_by(&program, Build::NoCollector.rules());
        if let Some(error) = errors.first() {
            return Err(error.message.clone());
        }
        emit(&program, &types, "test.zeno", Build::NoCollector, true).map_err(|error| error.message)
    }

    /// A program that the C build with no collector accepted and ran.
    /// `printed` is what it printed and `report` what it wrote as it
    /// ended: how many blocks it made, how many bytes, how much text was
    /// in the text room at once, and that no block was left.
    ///
    /// Built through LLVM the program must print the same and write the
    /// same report, line for line. So the two backends take and give back
    /// memory at the same places, and neither leaves a block behind.
    pub(crate) fn no_collector_as_c(name: &str, source: &str, printed: &str, report: &str) {
        let ir = match compile_no_collector(source) {
            Ok(ir) => ir,
            Err(message) => panic!("the build with no collector through LLVM refused this: {message}"),
        };
        // The point of the build: none of this is in the IR.
        for gone in ["zeno_collect", "zeno_roots", "zeno_stack_base", "zeno_mark_words", "_setjmp", "volatile"] {
            assert!(!ir.contains(gone), "`{gone}` is still in a build with no collector");
        }
        assert_eq!(asks_the_heap(&ir), None, "the IR takes memory from the heap");
        let name = format!("{name}-llvm");
        let Some((compiled, errors, finished)) = build_and_run_as(&name, &ir, Build::NoCollector, &["-DZENO_MEMORY_REPORT"])
        else {
            return;
        };
        assert!(finished, "the compiled program failed: {errors}");
        assert_eq!(compiled, printed, "\n{source}");
        assert_eq!(errors, report, "the two backends do not use memory alike\n{source}");
    }

    /// Does this C, or this IR, take memory from the C library's heap?
    /// The answer is the first of its four names that is found. A build
    /// with no collector must not: its memory is the pool in runtime.c
    /// (docs/pool.md). The names are looked for everywhere, comments
    /// included, because a reviewer's search would look there too. The
    /// runtime's own `zeno_free` is not the library's `free`.
    pub(crate) fn asks_the_heap(code: &str) -> Option<&'static str> {
        let free = code.match_indices("free(").any(|(at, _)| !code[..at].ends_with("zeno_"));
        ["malloc", "calloc", "realloc"].into_iter().find(|name| code.contains(name)).or(free.then_some("free"))
    }

    /// A program that the C build with no collector refused, with these
    /// words. The LLVM build must refuse it in the same words.
    pub(crate) fn refuses_as_c(source: &str, message: &str) {
        let ours = compile_no_collector(source).err().unwrap_or_else(|| "it compiled".to_string());
        assert_eq!(ours, message, "\n{source}");
    }

    #[test]
    fn the_ir_for_a_build_with_no_collector() {
        let ir = compile_no_collector(FRAME).expect("it compiles");
        // No list of roots, and no note of where the stack begins.
        assert!(!ir.contains("zeno_roots") && !ir.contains("zeno_stack_base") && !ir.contains("%base"), "{ir}");
        // `print` builds text, to show the number. The top of the text
        // room is noted before the statement and put back after it, and
        // then the end of the file frees what `frame` still holds.
        let top = "define internal void @zeno_program() noinline {
entry:
  %t1 = call ptr @z_make(i64 7)
  store ptr %t1, ptr @z_frame
  %mark.1 = load i64, ptr @zeno_text_top
  %t2 = load ptr, ptr @z_frame
  store ptr null, ptr @z_frame
  %t3 = call i64 @z_size(ptr %t2)
  %t4 = call ptr @zeno_str_int(i64 %t3)
  call void (i32, ...) @zeno_print(i32 1, ptr %t4)
  store i64 %mark.1, ptr @zeno_text_top
  %t5 = load ptr, ptr @z_frame
  call void @zeno.drop.Frame(ptr %t5)
  ret void
}";
        assert!(ir.contains(top), "{ir}");
        // A statement that builds no text has no such lines.
        let make = ir.split("define internal ptr @z_make(").nth(1).expect("make").split("\n}\n").next().expect("its body");
        assert!(!make.contains("zeno_text_top"), "{make}");
    }

    #[test]
    fn the_ir_for_what_a_statement_owns() {
        // docs/temporaries.md, in IR.
        let ir = compile_no_collector(
            "function longest(words: list[str]) -> int {
    var most = 0
    for word in words {
        if len(word) > most { most = len(word) }
    }
    return most
}

let line = \"set led on\"
print(longest(line.split()))
let ok = len(line) > 4 and len(line.split()) == 3
print(ok)
",
        )
        .unwrap();
        let program = ir.split("define internal void @zeno_program() noinline {\n").nth(1).expect("the program");
        let program = program.split("\n}\n").next().unwrap();
        // The list of words is kept in a slot of the statement's own:
        // null where the statement begins, freed where it ends. The right
        // side of `and` is over before the statement is, so the list it
        // made is freed in its own block.
        assert_eq!(
            program,
            "entry:
  %own.1 = alloca ptr
  %own.2 = alloca ptr
  store ptr @.str.1, ptr @z_line
  %mark.1 = load i64, ptr @zeno_text_top
  store ptr null, ptr %own.1
  %t1 = load ptr, ptr @z_line
  %t2 = call ptr @zeno_split_blanks(ptr %t1)
  store ptr %t2, ptr %own.1
  %t3 = call i64 @z_longest(ptr %t2)
  %t4 = call ptr @zeno_str_int(i64 %t3)
  call void (i32, ...) @zeno_print(i32 1, ptr %t4)
  %t5 = load ptr, ptr %own.1
  call void @zeno_list_drop_text(ptr %t5)
  store i64 %mark.1, ptr @zeno_text_top
  %t6 = load ptr, ptr @z_line
  %t7 = call i64 @zeno_len(ptr %t6)
  %t8 = icmp sgt i64 %t7, 4
  br i1 %t8, label %and1, label %end1

and1:
  store ptr null, ptr %own.2
  %t9 = load ptr, ptr @z_line
  %t10 = call ptr @zeno_split_blanks(ptr %t9)
  store ptr %t10, ptr %own.2
  %t11 = load i64, ptr %t10
  %t12 = icmp eq i64 %t11, 3
  %t13 = load ptr, ptr %own.2
  call void @zeno_list_drop_text(ptr %t13)
  br label %end1

end1:
  %t14 = phi i1 [ false, %entry ], [ %t12, %and1 ]
  store i1 %t14, ptr @z_ok
  %mark.2 = load i64, ptr @zeno_text_top
  %t15 = load i1, ptr @z_ok
  %t16 = call ptr @zeno_str_bool(i1 %t15)
  call void (i32, ...) @zeno_print(i32 1, ptr %t16)
  store i64 %mark.2, ptr @zeno_text_top
  ret void"
        );
    }

    #[test]
    fn the_ir_for_a_list_that_owns_its_items() {
        // docs/one-owner.md, in IR.
        let ir = compile_no_collector(
            "class Point {\n    x: int\n}\n\
             let points = [Point(1), Point(2)]\n\
             points[0] = Point(3)\n\
             for p in points {\n    print(p.x)\n}\n\
             let grid = [[points.pop()]]\n",
        )
        .unwrap();
        // One function for each kind of list that frees its items: it
        // walks the list, frees each item, then frees the list alone.
        let drop = ir.split("define internal void @zeno_list_drop_1_Point(ptr %list) {\n").nth(1).expect("the function");
        let drop = drop.split("\n}\n").next().unwrap();
        assert!(drop.contains("  call void @zeno.drop.Point(ptr %t9)\n"), "{drop}");
        assert!(drop.ends_with("done2:\n  call void @zeno_list_drop(ptr %list)\n  ret void"), "{drop}");
        // A list of lists of Points frees each item with the one above.
        let outer = ir.split("define internal void @zeno_list_drop_2_Point(ptr %list) {\n").nth(1).expect("the function");
        assert!(outer.split("\n}\n").next().unwrap().contains("  call void @zeno_list_drop_1_Point(ptr %t9)\n"), "{ir}");

        let program = ir.split("define internal void @zeno_program() noinline {\n").nth(1).expect("the program");
        let program = program.split("\n}\n").next().unwrap();
        // A new item frees the one whose place it takes.
        assert!(
            program.contains(
                "  %t9 = call ptr @zeno_list_at(ptr %t7, i64 0, i32 5, i32 1)
  %t10 = load ptr, ptr %t9
  call void @zeno.drop.Point(ptr %t10)
  store i64 %t8, ptr %t9
"
            ),
            "{program}"
        );
        // The copy a loop walks is freed alone: its items are the list's.
        assert!(program.contains("done1:\n  call void @zeno_list_drop(ptr %t12)\n"), "{program}");
        // The end of the program frees each list with its items.
        assert!(
            program.ends_with(
                "  %t33 = load ptr, ptr @z_grid
  call void @zeno_list_drop_2_Point(ptr %t33)
  %t34 = load ptr, ptr @z_points
  call void @zeno_list_drop_1_Point(ptr %t34)
  ret void"
            ),
            "{program}"
        );
    }

    #[test]
    fn the_ir_for_an_enum_value_that_has_one_owner() {
        // docs/enums-and-none.md, in IR.
        let ir = compile_no_collector(crate::emit_c::tests::ENUM_C).unwrap();
        // The function that makes a case keeps a copy of its text.
        let make = ir.split("define internal ptr @zeno.new.Shape.Named(ptr %f0, ptr %f1) {\n").nth(1).expect("the function");
        assert!(make.split("\n}\n").next().unwrap().contains("  %t3 = call ptr @zeno_text_keep(ptr %f0)\n  store ptr %t3, ptr %t2\n"), "{ir}");
        // Freeing a value goes by its case. One that carries nothing is
        // a fixed value and is left alone.
        let drop = ir.split("define internal void @zeno.drop.Shape(ptr %self) {\n").nth(1).expect("the function");
        let drop = drop.split("\n}\n").next().unwrap();
        assert_eq!(
            drop,
            "entry:
  %t1 = icmp eq ptr %self, null
  br i1 %t1, label %gone1, label %there1

gone1:
  ret void

there1:
  %t2 = load i64, ptr %self
  switch i64 %t2, label %bare [ i64 1, label %case1 i64 2, label %case2 ]

case1:
  call void @zeno_free(ptr %self)
  ret void

case2:
  %t3 = getelementptr %z_Shape.Named, ptr %self, i32 0, i32 1
  %t4 = load ptr, ptr %t3
  call void @zeno_text_drop(ptr %t4)
  %t5 = getelementptr %z_Shape.Named, ptr %self, i32 0, i32 2
  %t6 = load ptr, ptr %t5
  call void @zeno_list_drop(ptr %t6)
  call void @zeno_free(ptr %self)
  ret void

bare:
  ret void"
        );
        let program = ir.split("define internal void @zeno_program() noinline {\n").nth(1).expect("the program");
        let program = program.split("\n}\n").next().unwrap();
        // A new value frees the old one, and `none` is null.
        assert!(
            program.ends_with(
                "end1:
  %t13 = load ptr, ptr @z_shape
  call void @zeno.drop.Shape(ptr %t13)
  store ptr @zeno.case.Shape.Dot, ptr @z_shape
  store ptr null, ptr @z_last
  %t14 = call ptr @zeno.new.Shape.Circle(double 1.0)
  %t15 = load ptr, ptr @z_last
  call void @zeno.drop.Shape(ptr %t15)
  store ptr %t14, ptr @z_last
  %t16 = load ptr, ptr @z_last
  call void @zeno.drop.Shape(ptr %t16)
  %t17 = load ptr, ptr @z_shape
  call void @zeno.drop.Shape(ptr %t17)
  ret void"
            ),
            "{program}"
        );
    }

    #[test]
    fn the_ir_for_a_function_value_that_has_one_owner() {
        // docs/functions-and-errors.md, in IR.
        let ir = compile_no_collector(crate::emit_c::tests::FUNCTION_C).unwrap();
        // A function of the program is a fixed value with no `drop`.
        assert!(
            ir.contains("@zeno.value.double = internal constant { ptr, ptr, ptr } { ptr @zeno.wrap.double, ptr @.str.3, ptr null }\n"),
            "{ir}"
        );
        // One written where a value goes is a block: code, name, what
        // frees it, and then what it uses. The text is a copy.
        let make = ir.split("define internal ptr @z_make_tagger(ptr %arg.tag, i64 %arg.width) {\n").nth(1).expect("the function");
        assert_eq!(
            make.split("\n}\n").next().unwrap(),
            "entry:
  %t1 = call ptr @zeno_alloc(i64 ptrtoint (ptr getelementptr ({ ptr, ptr, ptr, ptr, i64 }, ptr null, i32 1) to i64))
  store ptr @zeno.fn.2, ptr %t1
  %t2 = getelementptr { ptr, ptr, ptr, ptr, i64 }, ptr %t1, i32 0, i32 1
  store ptr @.str.4, ptr %t2
  %t3 = getelementptr { ptr, ptr, ptr, ptr, i64 }, ptr %t1, i32 0, i32 3
  %t4 = call ptr @zeno_text_keep(ptr %arg.tag)
  store ptr %t4, ptr %t3
  %t5 = getelementptr { ptr, ptr, ptr, ptr, i64 }, ptr %t1, i32 0, i32 4
  store i64 %arg.width, ptr %t5
  %t6 = getelementptr { ptr, ptr, ptr, ptr, i64 }, ptr %t1, i32 0, i32 2
  store ptr @zeno.fn.drop.2, ptr %t6
  ret ptr %t1"
        );
        let drop = ir.split("define internal void @zeno.fn.drop.2(ptr %self) {\n").nth(1).expect("the function");
        assert_eq!(
            drop.split("\n}\n").next().unwrap(),
            "entry:
  %t1 = getelementptr { ptr, ptr, ptr, ptr, i64 }, ptr %self, i32 0, i32 3
  %t2 = load ptr, ptr %t1
  call void @zeno_text_drop(ptr %t2)
  call void @zeno_free(ptr %self)
  ret void"
        );
        // A new value frees the old one, whichever kind each is.
        let program = ir.split("define internal void @zeno_program() noinline {\n").nth(1).expect("the program");
        assert_eq!(
            program.split("\n}\n").next().unwrap(),
            "entry:
  %t1 = call ptr @z_make_tagger(ptr @.str.1, i64 4)
  store ptr %t1, ptr @z_tagger
  %t2 = call ptr @z_make_tagger(ptr @.str.2, i64 2)
  %t3 = load ptr, ptr @z_tagger
  call void @zeno_closure_drop(ptr %t3)
  store ptr %t2, ptr @z_tagger
  store ptr @zeno.value.double, ptr @z_current
  %t4 = call ptr @zeno_alloc(i64 ptrtoint (ptr getelementptr ({ ptr, ptr, ptr }, ptr null, i32 1) to i64))
  store ptr @zeno.fn.1, ptr %t4
  %t5 = getelementptr { ptr, ptr, ptr }, ptr %t4, i32 0, i32 1
  store ptr @.str.4, ptr %t5
  %t6 = getelementptr { ptr, ptr, ptr }, ptr %t4, i32 0, i32 2
  store ptr @zeno_closure_release, ptr %t6
  %t7 = load ptr, ptr @z_current
  call void @zeno_closure_drop(ptr %t7)
  store ptr %t4, ptr @z_current
  %t8 = load ptr, ptr @z_current
  call void @zeno_closure_drop(ptr %t8)
  %t9 = load ptr, ptr @z_tagger
  call void @zeno_closure_drop(ptr %t9)
  ret void"
        );
        // Where there is a collector, a function value has no third word.
        let shared = compile(crate::emit_c::tests::FUNCTION_C).unwrap();
        assert!(shared.contains("@zeno.value.double = internal constant { ptr, ptr } { ptr @zeno.wrap.double,"), "{shared}");
    }

    #[test]
    fn the_ir_for_kept_text() {
        // docs/text.md, in IR.
        let ir = compile_no_collector(
            "function label(channel: int) -> str {
    return f\"ch{channel}\"
}

var best = \"none yet\"
for channel in range(3) {
    let name = label(channel)
    if channel == 1 {
        best = name
    }
}
print(best)
",
        )
        .expect("it compiles");
        // A function's answer travels in the text room: the runtime moves
        // it down to where the room's top was when the `return` began.
        let label = "define internal ptr @z_label(i64 %arg.channel) {
entry:
  %mark.1 = load i64, ptr @zeno_text_top
  %t1 = call ptr @zeno_str_int(i64 %arg.channel)
  %t2 = call ptr (i32, ...) @zeno_join(i32 2, ptr @.str.2, ptr %t1)
  %t3 = call ptr @zeno_text_return(i64 %mark.1, ptr %t2)
  ret ptr %t3
}";
        assert!(ir.contains(label), "{ir}");
        // `name` keeps a copy of the answer, in a slot, and the end of
        // each trip frees what the slot holds.
        let trip = "do1:
  %mark.1 = load i64, ptr @zeno_text_top
  %t3 = call ptr @z_label(i64 %t1)
  %t4 = call ptr @zeno_text_keep(ptr %t3)
  store ptr %t4, ptr %var.name
  store i64 %mark.1, ptr @zeno_text_top
  %t5 = icmp eq i64 %t1, 1
  br i1 %t5, label %then2, label %end2

then2:
  %t6 = load ptr, ptr %var.name
  %t7 = call ptr @zeno_text_keep(ptr %t6)
  %t8 = load ptr, ptr @z_best
  call void @zeno_text_drop(ptr %t8)
  store ptr %t7, ptr @z_best
  br label %end2

end2:
  %t9 = load ptr, ptr %var.name
  call void @zeno_text_drop(ptr %t9)
  br label %next1
";
        assert!(ir.contains(trip), "{ir}");
        // `best` can be changed by a function, so reading it lends the
        // statement a copy. The end of the file frees what it holds.
        let end = "done1:
  %mark.2 = load i64, ptr @zeno_text_top
  %t12 = load ptr, ptr @z_best
  %t13 = call ptr @zeno_text_lend(ptr %t12)
  call void (i32, ...) @zeno_print(i32 1, ptr %t13)
  store i64 %mark.2, ptr @zeno_text_top
  %t14 = load ptr, ptr @z_best
  call void @zeno_text_drop(ptr %t14)
  ret void
}";
        assert!(ir.contains(end), "{ir}");
        // Text read from a field and stored at once is copied straight
        // from the field: nothing is lent on the way.
        let direct = compile_no_collector(
            "owned class Tag {\n    name: str\n}\nlet tag = Tag(\"a\" + \"b\")\nlet copy = tag.name\nprint(copy)\n",
        )
        .expect("it compiles");
        assert!(direct.contains("call ptr @zeno_text_keep(ptr %t"), "{direct}");
        assert!(!direct.contains("call ptr @zeno_text_lend("), "{direct}");
        // With a collector none of it is there: text is shared, as ever.
        let shared = compile("var best = \"a\"\nbest = best + \"b\"\nprint(best)").expect("it compiles");
        for gone in ["zeno_text_keep", "zeno_text_drop", "zeno_text_lend", "zeno_text_return"] {
            assert!(!shared.contains(gone), "{gone}");
        }
    }

    #[test]
    fn with_no_collector_an_error_is_freed_by_the_except_that_catches_it() {
        let source = "function check(n: int) -> int raises ValueError {
    if n > 2 {
        raise ValueError(f\"{n} is too big\")
    }
    return n
}
for n in [1, 5] {
    try {
        print(check(n))
    } except ValueError as e {
        if n > 4 {
            print(e)
            continue
        }
        print(\"small\")
    }
    print(\"fine\")
}
";
        let ir = compile_no_collector(source).expect("it compiles");
        // Once where the `except` block ends, and once where `continue`
        // leaves it early.
        assert_eq!(ir.matches("call void @zeno_error_drop(ptr %t").count(), 2, "{ir}");
        // The list the loop walks is the loop's own, and is freed when
        // the loop is over.
        assert_eq!(ir.matches("call void @zeno_list_drop(ptr %t").count(), 1, "{ir}");
        let Some((printed, errors, finished)) = build_and_run_as("error-drop", &ir, Build::NoCollector, &["-DZENO_MEMORY_REPORT"])
        else {
            return;
        };
        assert!(finished, "{errors}");
        assert_eq!(printed, "1\nfine\n5 is too big\n");
        assert!(errors.contains("no collector, 0 blocks left at the end\n"), "{errors}");
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
        let (ordinary, firmware, bare) = (runtime(Build::Ordinary), runtime(Build::Firmware), runtime(Build::NoCollector));
        let has = |runtime: &str, name: &str| runtime.contains(&format!(" {name}(")) || runtime.contains(&format!("*{name}("));
        // What only some of the three kinds of build call.
        let jumps = ["zeno_raise", "zeno_raise_again", "zeno_caught", "zeno_try_size", "zeno_try_open", "zeno_try_close"];
        let values = ["zeno_note", "zeno_is"];
        let with_collector = ["zeno_mark_words"];
        let without_collector = [
            "zeno_error_drop",
            "zeno_text_keep",
            "zeno_text_drop",
            "zeno_text_lend",
            "zeno_text_take",
            "zeno_text_return",
            "zeno_list_drop_text",
            "zeno_list_copy_text",
            "zeno_list_keep_text",
            "zeno_error_of",
            "zeno_closure_drop",
            "zeno_closure_release",
        ];
        for runtime in [&ordinary, &firmware, &bare] {
            assert!(!runtime.lines().any(|line| line.starts_with("static ")));
        }
        // Everything the IR may call is there to be found.
        for (name, _) in DECLARED {
            if name.starts_with("zeno_") {
                assert_eq!(has(&ordinary, name), !values.contains(name) && !without_collector.contains(name), "{name}");
                assert_eq!(has(&firmware, name), !jumps.contains(name) && !without_collector.contains(name), "{name}");
                assert_eq!(has(&bare, name), !jumps.contains(name) && !with_collector.contains(name), "{name}");
            }
        }
        // A firmware build has nothing to jump with, and a build with no
        // collector has no collector.
        for runtime in [&firmware, &bare] {
            assert!(!runtime.contains("setjmp") && !runtime.contains("longjmp"));
        }
        for gone in ["zeno_collect", "zeno_roots", "zeno_stack_base", "zeno_block_count"] {
            assert!(ordinary.contains(gone) && firmware.contains(gone) && !bare.contains(gone), "{gone}");
        }
        assert!(bare.contains("char zeno_text_room[") && bare.contains("size_t zeno_text_top"));
        // And it takes its memory from the pool, where the other two take
        // it from the heap.
        assert_eq!(asks_the_heap(&bare), None);
        assert!(bare.contains("char bytes[ZENO_POOL];"));
        assert_eq!(asks_the_heap(&ordinary), Some("malloc"));
        assert_eq!(asks_the_heap(&firmware), Some("malloc"));
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
    fn the_ir_for_a_sized_field() {
        let source = "class Packet {\n    kind: u8\n    length: u16\n    offset: i8\n}\n\
                      let p = Packet(3, 1024, -1)\nprint(p.kind + p.offset, u8.wrap(p.length), u8(p.length))\n";
        let ir = compile(source).expect("it compiles");
        // Four bytes of fields, where three plain ints would take 24.
        assert!(ir.contains("%z_Packet = type { i8, i16, i8 }"), "{ir}");
        // Going in, the i64 is cut down to the size of the field.
        assert!(ir.contains("  %t3 = trunc i64 %f0 to i8\n  store i8 %t3, ptr %t2\n"), "{ir}");
        // Coming out, it is made an i64 again: with zeros for a u8, and
        // with copies of the sign bit for an i8.
        assert!(ir.contains("  %t4 = load i8, ptr %t3\n  %t5 = zext i8 %t4 to i64\n"), "{ir}");
        assert!(ir.contains("  %t8 = load i8, ptr %t7\n  %t9 = sext i8 %t8 to i64\n"), "{ir}");
        // `u8.wrap` is the same two instructions, and `u8( )` asks the runtime.
        assert!(ir.contains(" = trunc i64 %t"), "{ir}");
        assert!(ir.contains("call i64 @zeno_fit(i64 %t"), "{ir}");
        assert!(ir.contains(", i64 0, i64 255, ptr @.str"), "{ir}");
    }

    #[test]
    fn the_ir_for_a_generic() {
        let source = "function first[T](items: list[T]) -> T {\n    return items[0]\n}\n\
                      class Cell[T] {\n    value: T\n    function get(self) -> T {\n        return self.value\n    }\n}\n\
                      function unused[T](x: T) -> T {\n    return x\n}\n\
                      let small: u8 = 7\n\
                      print(first([1, 2]), first([\"a\"]), first([3]))\n\
                      print(Cell(1.5).get(), Cell(small))\n";
        let ir = compile(source).expect("it compiles");
        // One function for each type `first` is called with. IR allows
        // any name in quotes, so the copies keep the names Zeno gives them.
        assert!(ir.contains("define internal i64 @\"z_first[int]\"(ptr %arg.items) {"), "{ir}");
        assert!(ir.contains("define internal ptr @\"z_first[str]\"(ptr %arg.items) {"), "{ir}");
        assert_eq!(ir.matches("define internal i64 @\"z_first[int]\"").count(), 1, "{ir}");
        // One struct type for each `Cell`, as big as what it holds.
        assert!(ir.contains("%\"z_Cell[float]\" = type { double }"), "{ir}");
        assert!(ir.contains("%\"z_Cell[u8]\" = type { i8 }"), "{ir}");
        assert!(ir.contains("define internal double @\"z_Cell[float].get\"(ptr %arg.self) {"), "{ir}");
        // Nothing for the generic itself, or for one that nothing calls.
        assert!(!ir.contains("@z_first(") && !ir.contains("%z_Cell ") && !ir.contains("z_unused"), "{ir}");
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

    // ------------------------------------------- Memory by hand (docs/unsafe.md)

    #[test]
    fn the_ir_for_memory_by_hand() {
        let ir = compile(crate::emit_c::tests::BY_HAND_C).unwrap();
        // A pointer is `ptr`, and an item is found with `getelementptr`
        // at the size of the item. Reading makes it 64 bits wide again.
        let sum = ir.split("define internal i64 @z_sum(ptr %arg.bytes, i64 %arg.count) {\n").nth(1).expect("the function");
        assert!(
            sum.contains(
                "do2:
  %t3 = getelementptr i8, ptr %arg.bytes, i64 %t1
  %t4 = load i8, ptr %t3
  %t5 = zext i8 %t4 to i64
"
            ),
            "{sum}"
        );
        // The block is a label going in and a label coming out.
        assert!(sum.contains("  br label %unsafe1\n\nunsafe1:\n") && sum.contains("  br label %safe1\n\nsafe1:\n"), "{sum}");
        // A register is its address, and its size goes with each call.
        let set = ir.split("define internal void @z_set_bit(i64 %arg.port, i64 %arg.bit) {\n").nth(1).expect("the function");
        assert_eq!(
            set.split("\n}\n").next().unwrap(),
            "entry:
  br label %unsafe1

unsafe1:
  %t1 = call i64 @zeno_register_read(i64 %arg.port, i32 signext 32)
  %t2 = call i64 @zeno_shl(i64 1, i64 %arg.bit, i32 15, i32 38)
  %t3 = or i64 %t1, %t2
  %t4 = call i64 @zeno_fit(i64 %t3, i64 0, i64 4294967295, ptr @.str.2, i32 15, i32 20)
  call void @zeno_register_write(i64 %arg.port, i32 signext 32, i64 %t4)
  br label %safe1

safe1:
  ret void"
        );
        let program = ir.split("define internal void @zeno_program() noinline {\n").nth(1).expect("the program");
        // `allocate`: how many, and the size of one.
        assert!(program.contains("  %t2 = call ptr @zeno_allocate(i64 2, i64 2, i32 20, i32 13)\n"), "{program}");
        // A write is cut down to the size of the item. A signed item is
        // read back with its sign.
        assert!(
            program.contains(
                "  %t17 = load ptr, ptr @z_words
  %t18 = getelementptr i16, ptr %t17, i64 1
  %t19 = trunc i64 -300 to i16
  store i16 %t19, ptr %t18
"
            ),
            "{program}"
        );
        assert!(program.contains("  %t25 = load i16, ptr %t24\n  %t26 = sext i16 %t25 to i64\n"), "{program}");
        // `bytes + 1` is an address one item further on.
        assert!(
            program.contains("  %t21 = getelementptr i8, ptr %t20, i64 1\n  %t22 = call i64 @z_sum(ptr %t21, i64 3)\n"),
            "{program}"
        );
        assert!(program.contains("  call void @zeno_release(ptr %t29)\n"), "{program}");
    }

    #[test]
    fn the_memory_report_counts_what_was_made_by_hand() {
        let source = "let a = allocate(u8, 16)\nlet b = allocate(int, 4)\nunsafe { free(a) }\nprint(b)";
        let program = crate::parser::parse(crate::lexer::lex(source).unwrap()).unwrap();
        let (errors, types) = crate::checker::check_with_types(&program, true);
        assert!(errors.is_empty(), "the test program has mistakes: {:?}", errors[0].message);
        // `true`: this build is asked for the report, as `--memory` does.
        let ir = emit(&program, &types, "test.zeno", Build::Ordinary, true).expect("it compiles");
        let Some((printed, errors, finished)) = build_and_run_as("by-hand-report", &ir, Build::Ordinary, &["-DZENO_MEMORY_REPORT"])
        else {
            return;
        };
        assert!(finished && printed == "<pointer[int]>\n", "{printed}{errors}");
        assert!(errors.ends_with("by hand: 2 blocks from `allocate`, 1 never freed\n"), "{errors}");
    }
}
