# How the LLVM backend works

`zeno build` has a second way to make a program. With `--llvm` it does
not write C. It writes **LLVM IR**, the input language of the compiler
library behind Clang, Rust and Swift, and hands that to clang.

```sh
cargo run -- build --llvm examples/primes.zeno   # writes primes.ll and primes_runtime.c, builds ./primes
./primes                                         # same output as `zeno run`
```

```
tokens -> parser -> tree -> checker -> [emit_llvm] -> LLVM IR -> clang -> program
```

The program prints exactly what `zeno run` prints. As with every stage
since the C backend, that is the rule, and the tests hold it.

This first version compiles numbers, bools, text, functions and control
flow: what the C backend began with. The rest is refused with a message
(see [What it does not do yet](#what-it-does-not-do-yet)).

## Why

The C backend works, and for firmware it stays the right one: every
microcontroller has a C compiler, and a reviewer can read C. But it
leans on a second language, and a good part of `emit_c.rs` is there to
get round C's rules: the order arguments are worked out in, overflow
being undefined, temporaries so that one thing happens before another.

LLVM takes that layer away. Its IR is lower than C and stricter, and it
is the same thing clang turns C into before it does anything clever. So
everything LLVM can do for C it can do for Zeno: the optimiser, and
machine code for x86, ARM, RISC-V or WebAssembly from one input.

It is also how most new languages are built today (Rust, Swift, Zig,
Julia), which makes it worth seeing from the inside.

## IR in five minutes

```
function area(w: float, h: float) -> float {
    return w * h
}
```

becomes

```llvm
define internal double @z_area(double %arg.w, double %arg.h) {
entry:
  %t1 = fmul double %arg.w, %arg.h
  ret double %t1
}
```

Read it as assembly for a machine that has as many registers as you
like:

- **`@name` is global, `%name` is local** to one function.
- **Every value has a type, written at every use.** `i64` is a whole
  number of 64 bits, `double` a float, `i1` one bit (a bool), `ptr` an
  address. Text is a `ptr` to its first byte, as in the C backend.
- **One instruction does one thing** and gives its result a new name.
  `fmul` multiplies floats. `a + b * c` is two instructions. Nothing is
  nested, so the order things happen in is the order they are written.
  Zeno's left-to-right rule costs nothing here.
- **A name is given a value once.** `%t1` is never assigned again. This
  is called SSA, "static single assignment", and it is what makes IR
  easy for an optimiser to reason about: to know what `%t1` is, find
  the one line that makes it.
- **A function is a list of blocks.** A block starts with a label
  (`entry:`), runs straight down, and ends with exactly one instruction
  that leaves it: `br` (jump), `ret` (return) or `unreachable`.
  There is no `if` and no `while`.

`internal` means what `static` means in C: nothing outside this file
needs the function, so LLVM may inline it everywhere and drop it.
The `z_` in front keeps a Zeno function called `main` or `printf` from
meeting the real one.

## The file

`src/emit_llvm.rs`, about 1,400 lines without its tests, a good part of
them comments. It has no
dependencies: IR is text, and the file is built up with `format!`.

| Piece | What it is |
|---|---|
| `Val` | A value that has been worked out: how the IR names it (`%t3`, `42`, `@.str.2`) and its Zeno kind |
| `Place` | Where a variable is: a `Value`, or a `Slot` in memory |
| `Code` | The function being written: its lines, its `alloca`s, the block being filled |
| `ins`, `set` | Write one instruction. `set` also makes the new name |
| `place`, `jump`, `jump_if`, `end` | Start a block, leave one |
| `statement`, `expr` | The walk over the tree, as in every other stage |
| `checked`, `arithmetic`, `compare`, `logic`, `chain` | Operators |
| `DECLARED` | What the IR may call in the runtime, as LLVM wants it declared |

## A `let` is a value, a `var` is a slot

A Zeno `let` is given a value once. So is an IR name. So a `let` needs
nothing at all: the variable *is* the value. The same goes for a
parameter, which Zeno does not let you change.

A `var` changes, and a name in IR cannot. The way out is memory:
`alloca` makes room for one value on the stack and gives its address.
`store` writes there and `load` reads.

```
function sum_to(limit: int) -> int {
    var total = 0
    for n in range(limit) {
        total += n
    }
    return total
}
```

```llvm
define internal i64 @z_sum_to(i64 %arg.limit) {
entry:
  %var.total = alloca i64
  %for.n.1 = alloca i64
  store i64 0, ptr %var.total
  store i64 0, ptr %for.n.1
  br label %for1

for1:
  %t1 = load i64, ptr %for.n.1
  %t2 = icmp slt i64 %t1, %arg.limit
  br i1 %t2, label %do1, label %done1

do1:
  %t3 = load i64, ptr %var.total
  ...
```

`limit` is used as it arrived (`%arg.limit`). `total` has a slot. The
loop's counter has one too, because it changes, but the `n` the body
sees is `%t1`, a value.

This looks wasteful, and it is on purpose. LLVM's first pass, called
`mem2reg`, finds every slot whose address never leaves the function and
turns it back into plain values. Ask for only that pass:

```sh
opt -passes=mem2reg -S demo.ll
```

```llvm
for1:
  %for.n.1.0 = phi i64 [ 0, %entry ], [ %t8, %next1 ]
  %var.total.0 = phi i64 [ 0, %entry ], [ %t5, %next1 ]
  %t2 = icmp slt i64 %for.n.1.0, %arg.limit
```

The slots are gone. In their place is `phi`, the one odd instruction
SSA needs: "`total` is 0 if we came here from `entry`, and `%t5` if we
came round from `next1`". Working out where the `phi`s go is the hard
part of building SSA, and LLVM does it for us. Every front end built on
LLVM, clang included, uses this trick (decision 5.11b).

The `alloca`s all go at the top of the first block, wherever the `var`
is written. One inside a loop would take new stack on every trip.

Variables at the top of the file are different: functions use them, so
they are globals (`@z_limit = internal global i64 0`), read and written
with the same `load` and `store`.

## Blocks and jumps

`if n < 0 { return "minus" }` then `return "plus"`:

```llvm
define internal ptr @z_sign(i64 %arg.n) {
entry:
  %t1 = icmp slt i64 %arg.n, 0
  br i1 %t1, label %then1, label %end1

then1:
  ret ptr @.str.4

end1:
  ret ptr @.str.5
}
```

`icmp slt` is "compare whole numbers, signed less than", and gives an
`i1`. `br i1` jumps one of two ways on it. The number on the labels
groups the ones that belong to one statement.

`Code` keeps one flag for all of this: `ended`, true once the block has
been left. `place(label)` starts a new block, and first adds a `br` to
it if the block before is still open, because a block may not simply
run on into the next. After `return` the flag is set, so no `br` is
added after `ret`.

A `while` is three blocks: the test, the body, and what follows. The
test is a block of its own because the body jumps back to it. `break`
is a jump to the block after the loop, `continue` a jump to the test.
A `for` over a `range` is the same with a counter. No list is built.

Code after a `return` can never run, but it still has to be valid IR.
It gets a block that nothing jumps to (`dead3:`), which LLVM deletes.

## `and`, `or` and `phi`

`a and b` must not work `b` out when `a` is false. That is a jump:

```llvm
define internal i1 @z_in_range(i64 %arg.n, i64 %arg.low, i64 %arg.high) {
entry:
  %t1 = icmp sge i64 %arg.n, %arg.low
  br i1 %t1, label %and1, label %end1

and1:
  %t2 = icmp sle i64 %arg.n, %arg.high
  br label %end1

end1:
  %t3 = phi i1 [ false, %entry ], [ %t2, %and1 ]
  ret i1 %t3
}
```

Here the `phi` is written by hand, because the shape never varies:
false if the left side sent us straight to the end, otherwise whatever
the right side gave. `a < b < c` is built the same way, one block for
each comparison after the first.

## Whole numbers that report overflow

A plain `add i64` wraps round on overflow. Zeno says that is an
`OverflowError`. LLVM has an instruction for exactly this, which gives
back the result and the processor's overflow flag together:

```llvm
  %t4 = call { i64, i1 } @llvm.sadd.with.overflow.i64(i64 %t3, i64 %t1)
  %t5 = extractvalue { i64, i1 } %t4, 0
  %t6 = extractvalue { i64, i1 } %t4, 1
  br i1 %t6, label %overflow2, label %fits2

overflow2:
  call void @zeno_overflow(i32 8, i32 9)
  unreachable

fits2:
  store i64 %t5, ptr %var.total
```

`{ i64, i1 }` is a pair, and `extractvalue` takes one half out. The
`8, 9` is the line and column of the `+`, for the error message.
`unreachable` tells LLVM that `zeno_overflow` does not come back.

It reads as a lot for one `+`. On the processor it is an add and a
"jump if overflow" that is almost never taken. This is the same thing
`__builtin_add_overflow` in `runtime.c` turns into.

`//`, `%` and `**` have more to them (which way to round, the sign of
the result), so they call the functions the runtime already has.
Floats use `fadd`, `fsub` and `fmul` directly. `/` calls the runtime,
which checks for zero.

## Text, printing and the runtime

The compiled program still needs `runtime.c`: joining text, turning
numbers into text, printing, the collector. The IR declares what it
uses and calls it:

```llvm
declare ptr @zeno_str_int(i64)
declare ptr @zeno_join(i32 signext, ...)
declare void @zeno_print(i32 signext, ...)

  %t5 = call ptr @zeno_str_int(i64 %t4)
  %t9 = call ptr (i32, ...) @zeno_join(i32 3, ptr %t1, ptr @.str.2, ptr %t5)
  call void (i32, ...) @zeno_print(i32 1, ptr %t9)
```

- `...` is a function that takes any number of arguments, like C's
  `printf`. The call has to repeat `(i32, ...)`.
- `signext` and `zeroext` say how a value narrower than a register is
  widened when it is handed to a C function. A C compiler does this
  without being asked. In IR it has to be said, or on some processors
  (RISC-V, PowerPC) the C side may read leftover bits.
- A piece of text in the program is a constant, with the zero byte C
  expects written out: `@.str.1 = private unnamed_addr constant [5 x i8] c"Zeno\00"`.
  The same text is stored once however often it is written.

The table `DECLARED` holds every declaration. Only the ones a program
uses are written into its file.

### Linking to the runtime

A C build pastes the runtime in front of the program, and every
function in it is `static`. Here the program is in another file, and
`static` would hide the runtime from it. So `runtime()` in
`emit_llvm.rs` takes the word off (only where it starts a line, which
is where it stands outside a function), and `zeno build --llvm` writes
the result as `primes_runtime.c`. Then:

```sh
clang -O2 -Wno-override-module -o primes primes.ll primes_runtime.c -lm
```

clang compiles the C, compiles the IR, and links the two. The IR does
not say which processor it is for, so the same file builds anywhere.
clang fills in its own and would warn that it did, which is what the
`-W` flag turns off.

## The top of the file

```llvm
define internal void @zeno_program() noinline { ... }

define void @zeno_roots() {
entry:
  call void @zeno_mark_words(ptr @zeno_raised, i64 8)
  call void @zeno_mark_words(ptr @z_name, i64 8)
  ret void
}

define i32 @main() {
entry:
  %base = alloca ptr
  store ptr %base, ptr @zeno_stack_base
  store ptr @.str.6, ptr @zeno_file
  call void @zeno_program()
  ret i32 0
}
```

The same three pieces the C backend writes, for the same reasons
([`memory.md`](memory.md)). The statements at the top of the file
become `zeno_program`. `main` notes where the stack begins and the
file's name, and calls it. `zeno_roots` shows the collector each global
that holds text. The collector finds text held by a value inside a
function by looking through the stack, exactly as it does for C.

Functions are compiled in a second step. When the walk over the top of
the file reaches a function, it puts it aside together with a copy of
the variables declared so far. Those are the only ones the function
may use (decision 3.6d), and the copy is what its body is compiled
against.

## The tests

`same_as_interpreter` in `emit_llvm.rs` runs a program on the
interpreter and the VM (`vm::run_both`), builds it with clang, and
requires the same output and, when it stops, the same error at the same
line and column. The runtime is built with `ZENO_COLLECT_ALWAYS`, which
runs the collector before every allocation.

One test compares the IR of a small function with what it should be,
character for character, so that a change to the shape of the IR is
noticed. One checks that everything in `DECLARED` is in the runtime.

If clang is not installed the compiled half of each test is skipped.

## How fast

On the machine it was written on, with `-O2`:

| Program | C, gcc | C, clang | LLVM |
|---|---|---|---|
| `fib(38)` | 0.26 s | 0.20 s | 0.20 s |
| 300 million times `total += i % 7` | 0.30 s | 0.29 s | 0.76 s |

The first row is the point of the stage: the IR written here ends up
as the same machine code clang makes from Zeno's C.

The second row is the cost of the two files. `%` is a call to
`zeno_mod`. In a C build that function is in the same file and is
inlined into the loop. Here it is in the runtime, which LLVM cannot see
into while it compiles the IR. Exercises 5 and 6 are two ways to get
the time back.

## What it does not do yet

- **Most of the language.** Classes, lists, enums and `match`, `none`,
  `try` and `raise`, functions as values, functions inside functions,
  numbers with units, and `import` are refused: "`zeno build --llvm`
  cannot compile a class yet. `zeno build` without `--llvm` can".
- **`--firmware`, `--no-collector`, `--memory`.** They do not go with
  `--llvm` yet.
- **It needs clang**, version 15 or later (the first to write every
  pointer as `ptr`). gcc cannot read IR.
- **The runtime is not inlined**, as above, and all of it is linked in:
  `primes` is about 38 KB where the C build is 18 KB.
- **64-bit processors only.** The IR says `i64` where C says `size_t`,
  and takes a pointer to be 8 bytes.
- **No debug information.** A debugger shows the program as assembly.

## Things to try

1. Build `examples/primes.zeno` with `--llvm` and read `primes.ll`
   beside `primes.zeno`. Find the `phi` for `while not is_prime(largest)`.
   There is none. Why not? Now run `opt -passes=mem2reg -S primes.ll`
   and look at `z_is_prime`.
2. Run `opt -O2 -S primes.ll` and list the functions that are left.
   Where did `z_is_prime` and `z_count_primes` go? What made that
   allowed? (Look at the word after `define`.)
3. `2 * 3` is compiled as an overflow check on two constants. LLVM
   folds it to `6`. Do it in `arithmetic` instead, when both sides are
   literals. What should happen for `9223372036854775807 + 1`?
4. Add `sqrt`. The C backend has it in `math`, behind `import`. Start
   smaller: make `x ** 0.5` call `llvm.sqrt.f64` when the right side is
   the literal `0.5`. Is that always the same answer as `pow`?
5. Make `%` on two ints faster. Write `zeno_mod` out as IR in
   `arithmetic`: `srem`, then the fix for the sign, with `select`. The
   test `numbers_match` will tell you when you have it right. Time the
   loop in the table again.
6. Or leave the IR alone and add `-flto` to the clang line in
   `src/main.rs`. That lets LLVM optimise across the two files. What
   does the loop take now? What does it need installed?
7. Give the IR another processor: `clang --target=riscv64 -O2 -S primes.ll`
   writes `primes.s`. Find an overflow check in it (look for
   `zeno_overflow`). Nothing for RISC-V had to be installed first. Why not, and what would be needed to make a
   program that runs?
8. Compile a `class` with only `int` fields. An object is a `ptr` from
   `zeno_alloc`. A field is reached with `getelementptr`, the one
   instruction this stage did not need. Start by reading what clang
   makes of a C struct: `clang -S -emit-llvm -O0 shapes.c`.
9. The big one: `try` and `raise`. The C backend has two ways
   ([`firmware.md`](firmware.md)). Which one is easier to write in IR?
