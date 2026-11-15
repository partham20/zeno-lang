# How the LLVM backend works

`zeno build` has a second way to make a program. With `--llvm` it does
not write C. It writes **LLVM IR**, the input language of the compiler
library behind Clang, Rust and Swift, and hands that to clang.

```sh
cargo run -- build --llvm examples/shapes.zeno   # writes shapes.ll and shapes_runtime.c, builds ./shapes
./shapes                                         # same output as `zeno run`
```

```
tokens -> parser -> tree -> checker -> [emit_llvm] -> LLVM IR -> clang -> program
```

The program prints exactly what `zeno run` prints. As with every stage
since the C backend, that is the rule, and the tests hold it.

It compiles what the C backend compiles: numbers, text, functions,
classes, enums and `match`, lists, `none`, `try` and `raise`, functions
as values, units and `import math`. The first half of this page is the
ground floor (values, slots, blocks, overflow). The second half is one
section for each of those, and each is mostly one new idea in IR.

It also takes the same flags: `--firmware`, `--no-collector` and
`--memory`. The last sections show what each one changes in the IR.

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

`src/emit_llvm.rs`, about 3,800 lines without its tests, a good part of
them comments. It has no dependencies: IR is text, and the file is
built up with `format!`. It follows the order of this page.

| Piece | What it is |
|---|---|
| `Val` | A value that has been worked out: how the IR names it (`%t3`, `42`, `@.str.2`) and its Zeno kind |
| `Place`, `Variable` | Where a variable is: a `Value`, or a `Slot` in memory |
| `Code` | The function being written: its lines, its `alloca`s, the block being filled, the `try`s it is inside |
| `ins`, `set` | Write one instruction. `set` also makes the new name |
| `place`, `jump`, `jump_if`, `end` | Start a block, leave one |
| `slot`, `load`, `store` | Memory for a `var`, and reading and writing it |
| `statement`, `expr` | The walk over the tree, as in every other stage |
| `checked`, `arithmetic`, `compare`, `logic`, `chain` | Operators |
| `ir_type` | The IR type for a Zeno kind (the table below) |
| `field_at`, `construct`, `match_statement` | Objects and enums |
| `to_slot`, `from_slot`, `walk_open` | Lists |
| `coerce`, `is_there`, `inside`, `present` | Values that may be `none` |
| `try_statement`, `raise`, `leave_tries` | Errors |
| `closure`, `function_value`, `invoke` | Functions as values |
| `side` | Write a helper function while in the middle of another |
| `DECLARED` | What the IR may call in the runtime, as LLVM wants it declared |
| `WRITTEN` | Small functions written in IR and put into every file that uses them |

Every Zeno kind becomes one IR type:

| Zeno | IR | |
|---|---|---|
| `int` | `i64` | |
| `float`, a number with a unit | `double` | the unit is gone by now; the checker dealt with it |
| `bool` | `i1` | |
| `str` | `ptr` | to its first byte |
| a list, an object, an enum value, an error, a function | `ptr` | to memory from `zeno_alloc` |
| `T or none`, where `T` is one of the `ptr` kinds | `ptr` | `null` is `none` |
| `int or none`, `float or none`, `bool or none` | `{ i1, T }` | a pair: is it there, and the value |

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

Two kinds of `var` are kept out of an `alloca`, and both come up later:
one that a function written inside this one uses
([Functions as values](#functions-as-values)), and any `var` in a
function that has a `try`
([Why the slots are `volatile`](#why-the-slots-are-volatile)).

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

`//` and `%` have more to them. The processor rounds toward zero and
Zeno rounds down, the result of `%` takes the sign of the right side,
and dividing by zero is an error. The runtime has C functions for both,
and the first version of this backend called them. That made a loop
full of `%` two and a half times slower than the C build, because LLVM
cannot see into a function in another file, so it could not inline it.

So they are written in IR instead, once, in the table `WRITTEN`:

```llvm
define internal i64 @zeno.mod(i64 %a, i64 %b, i32 %line, i32 %col) {
entry:
  %zero = icmp eq i64 %b, 0
  br i1 %zero, label %by_zero, label %divide

by_zero:
  call void @zeno_by_zero(i32 %line, i32 %col)
  unreachable

divide:
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
```

`srem` is the processor's remainder. `select` is "this value or that
one, depending on an `i1`": an `if` with no jump. The program calls
`@zeno.mod`, and because it is in the same file LLVM inlines it. In
`n % 2` it then sees that `%b` is 2 and throws nearly all of it away.
A name with a dot in it (`zeno.mod`) cannot be a Zeno name or a C name,
so it can meet neither (decision 5.11c).

`**` and the float forms of `//` and `%` still call the runtime. Floats
use `fadd`, `fsub` and `fmul` directly.

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
is where it stands outside a function), adds three small functions that
only an LLVM build needs (see [Errors](#errors-try-and-raise)), and
`zeno build --llvm` writes the result as `shapes_runtime.c`. Then:

```sh
clang -O2 -Wno-override-module -o shapes shapes.ll shapes_runtime.c -lm
```

clang compiles the C, compiles the IR, and links the two. The IR does
not say which processor it is for, so the same file builds anywhere.
clang fills in its own and would warn that it did, which is what the
`-W` flag turns off.

## One program for the rest

The sections from here on all read from one small program. Save it as
`demo.zeno`, build it with `--llvm`, and keep `demo.ll` open.

```
class Point {
    x: float
    y: float
}

enum Shape {
    Circle(centre: Point, radius: float)
    Square(side: float)
    Nothing
}

function area(shape: Shape) -> float {
    match shape {
        case Circle(_, r) { return 3.14159 * r * r }
        case Square(side) { return side * side }
        case Nothing { return 0.0 }
    }
}

function largest(shapes: list[Shape]) -> float or none {
    var best: float or none = none
    for shape in shapes {
        let size = area(shape)
        if best is none or size > best { best = size }
    }
    return best
}

function halve(n: int) -> int raises ValueError {
    if n % 2 == 1 { raise ValueError(f"{n} is odd") }
    return n // 2
}

function make_counter() -> function() -> int {
    var count = 0
    return function() -> int {
        count = count + 1
        return count
    }
}

let shapes = [Circle(Point(0.0, 0.0), 1.0), Square(2.0), Nothing]
print(shapes[0], largest(shapes), largest([]))
var done = 0
try {
    done = halve(8)
    done = halve(done + 1)
} except ValueError as error {
    print("stopped:", error.message, done)
}
let next = make_counter()
print(next(), next(), next())
```

```
Circle(Point(x=0.0, y=0.0), 1.0) 4.0 none
stopped: 5 is odd 4
1 2 3
```

## Objects and `getelementptr`

A class becomes a **struct type**, declared once at the top of the
file. It is a list of field types and nothing else. The names are gone.

```llvm
%z_Point = type { double, double }
```

(A field with a size, such as `level: u8`, is an `i8` in the struct
type and an `i64` everywhere else. [`sized-ints.md`](sized-ints.md)
has that. A generic class gets one struct type for each type it is
used with, such as `%"z_Stack[int]"`: see [`generics.md`](generics.md).)

An object is a `ptr` to that much memory from the collector. Making one
is a small function, written the first time the program makes a
`Point`:

```llvm
define internal ptr @zeno.new.Point(double %f0, double %f1) {
entry:
  %t1 = call ptr @zeno_alloc(i64 ptrtoint (ptr getelementptr (%z_Point, ptr null, i32 1) to i64))
  %t2 = getelementptr %z_Point, ptr %t1, i32 0, i32 0
  store double %f0, ptr %t2
  %t3 = getelementptr %z_Point, ptr %t1, i32 0, i32 1
  store double %f1, ptr %t3
  ret ptr %t1
}
```

The new instruction is **`getelementptr`**, "GEP" for short. It is the
one everybody finds odd at first, and it does one thing: **address
arithmetic**. It never reads memory. Given a type, an address and some
indices, it works out another address.

```llvm
%t3 = getelementptr %z_Point, ptr %t1, i32 0, i32 1
```

Read it as C's `&t1[0].y`. The first index steps over whole `Point`s,
as if `%t1` were an array of them. 0 means "this one". The second picks
field 1 inside it. The result is the address of `y`, and `store` or
`load` does the rest. `field_at` in the emitter writes this line.

The long argument to `zeno_alloc` is the same instruction used as a
trick. IR has no `sizeof`. But "the address of `Point` number 1 in an
array that starts at address 0" is exactly the size of one `Point`, and
`ptrtoint` turns that address into a number. LLVM folds the whole thing
to `16` at compile time. Writing 16 ourselves would mean knowing every
processor's rules for padding, and LLVM already knows them.

A **method** is a function with the object first. Give `Point` a
method, `function width(self) -> float { return self.x }`, and it is:

```llvm
define internal double @z_Point.width(ptr %arg.self) {
entry:
  %t1 = getelementptr %z_Point, ptr %arg.self, i32 0, i32 0
  %t2 = load double, ptr %t1
  ret double %t2
}
```

`p.width()` is `call double @z_Point.width(ptr %p)`. The checker knows
the class of `p`, so there is nothing to look up while the program runs
(decision 5.11h).

## Enums and `match`

An enum value is a `ptr` too. What it points to starts with an `i64`,
the **tag**: which case this is, counting from 0. The case's own fields
follow. Each case with fields has its own struct type:

```llvm
%z_Shape.Circle = type { i64, ptr, double }
%z_Shape.Square = type { i64, double }

@zeno.case.Shape.Nothing = internal constant { i64 } { i64 2 }
```

A case with no fields needs no memory at run time. There is only ever
one `Nothing`, so it is a constant in the program, and the value is the
address of that constant.

The C backend uses one struct with a `union` inside, as big as the
biggest case. IR has no unions. It does not need them: a `ptr` does not
say what it points to, so the code reads the tag through it, and then
uses the struct type of the case it found.

`match` is a row of tests, in the order the cases are written:

```llvm
define internal double @z_area(ptr %arg.shape) {
entry:
  %t1 = load i64, ptr %arg.shape
  %t2 = icmp eq i64 %t1, 0
  br i1 %t2, label %is2, label %case1.1

is2:
  %t3 = getelementptr %z_Shape.Circle, ptr %arg.shape, i32 0, i32 2
  %t4 = load double, ptr %t3
  %t5 = fmul double 3.14159, %t4
  %t6 = fmul double %t5, %t4
  ret double %t6

case1.1:
  %t7 = load i64, ptr %arg.shape
  %t8 = icmp eq i64 %t7, 1
  br i1 %t8, label %is3, label %case1.2
  ...

case1.3:
  br label %end1

end1:
  unreachable
}
```

`load i64, ptr %arg.shape` reads the tag: it is the first thing there,
so no GEP is needed. A case that fails jumps to the next one. The `_`
in `Circle(_, r)` loads nothing at all.

The checker has already proved that the cases cover every `Shape`, so
falling off the end cannot happen, and the function ends in
`unreachable`. That is a promise to LLVM, and it uses it: at `-O2` the
three tests become one `switch` on the tag, with no fourth way out.
A nested pattern such as `case Circle(Point(0.0, _), r)` is the same
row, with more tests before the body.

## Lists

A list is the runtime's own `zeno_list`: a length, a capacity, and the
address of the items. The IR declares the same layout so that it can
read the length and the items itself:

```llvm
%zeno_list = type { i64, i64, ptr }
```

Everything that changes a list (`append`, `pop`, growing) is a call to
the runtime. The C is already there and already tested.

In C every item is a `zeno_slot`, a union of an int, a float, a bool
and a pointer: eight bytes that mean whatever the list's kind says. In
IR **an item is an `i64`**, eight bytes, and each kind is turned into
one on the way in and back on the way out (`to_slot`, `from_slot`):

| Kind | Into a slot | Back |
|---|---|---|
| `int` | as it is | as it is |
| `float` | `bitcast double to i64` (the same bits) | `bitcast` |
| `bool` | `zext i1 to i64` | `trunc` |
| anything that is a `ptr` | `ptrtoint` | `inttoptr` |
| `int or none` and the like | boxed: the pair goes into memory, the slot is its address | unboxed |

The list at the top of the demo:

```llvm
  %t4 = ptrtoint ptr %t2 to i64
  %t5 = ptrtoint ptr %t3 to i64
  %t6 = ptrtoint ptr @zeno.case.Shape.Nothing to i64
  %t7 = call ptr (i32, ...) @zeno_list_of(i32 3, i64 %t4, i64 %t5, i64 %t6)
```

`shapes[0]` asks the runtime for the address of item 0, because the
runtime checks the index and reports an `IndexError`:

```llvm
  %t9 = call ptr @zeno_list_at(ptr %t8, i64 0, i32 43, i32 7)
  %t10 = load i64, ptr %t9
  %t11 = inttoptr i64 %t10 to ptr
```

A `for` over a list does not need that check. It walks the items
directly (`walk_open`), from `largest`:

```llvm
  %t1 = call ptr @zeno_list_copy(ptr %arg.shapes)
  ...
for1:
  %t2 = load i64, ptr %for.shape.1          ; the counter
  %t3 = load i64, ptr %t1                   ; the length: the list's first field
  %t4 = icmp slt i64 %t2, %t3
  br i1 %t4, label %do1, label %done1

do1:
  %t5 = getelementptr %zeno_list, ptr %t1, i32 0, i32 2
  %t6 = load ptr, ptr %t5                   ; where the items are
  %t7 = getelementptr i64, ptr %t6, i64 %t2 ; the address of item number %t2
  %t8 = load i64, ptr %t7
  %t9 = inttoptr i64 %t8 to ptr
```

Here GEP is used the other way: the type is `i64` and there is one
index, so it steps over whole `i64`s. That is C's `&items[i]`.

The copy at the top is Zeno's rule that a loop walks the list as it was
when the loop began, so a body that appends to it does not loop for
ever. It is left out when nobody else can hold the list: a list literal
or a comprehension written in the `for` itself.

Nothing else ever sees that copy, so it is freed as soon as the loop is
over. `done1` begins with `call void @zeno_list_drop(ptr %t1)`, and a
`return` written inside the body gets the same call just before its
`ret`. The C backend does the same ([`ownership.md`](ownership.md),
"Two frees that every program gets").

The `i64` slot is provisional (decision 5.11i). It holds on a 64-bit
processor that hands a union to a function the way it hands an `i64`,
which is true of x86-64 and ARM64.

## Values that may be `none`

There are two shapes, by what `T` is.

If `T` is already a `ptr` (text, a list, an object), `none` is the
address 0, `null`. No real value is ever there. Checking is one compare
and nothing is stored that was not stored before.

A number has no value to spare: every `i64` is a valid `int`. So
`int or none`, `float or none` and `bool or none` are a **pair**, an
`i1` that says whether there is a value, then the value. `largest` in
the demo returns one:

```llvm
define internal { i1, double } @z_largest(ptr %arg.shapes) {
entry:
  %var.best = alloca { i1, double }
  store { i1, double } zeroinitializer, ptr %var.best
```

`zeroinitializer` is "all zero bits", so the `i1` is false: `none`.

```llvm
  %t11 = load { i1, double }, ptr %var.best
  %t12 = extractvalue { i1, double } %t11, 0     ; is there a value?
  %t13 = xor i1 %t12, true                       ; `best is none`
  br i1 %t13, label %end2, label %or2

or2:
  %t14 = load { i1, double }, ptr %var.best
  %t15 = extractvalue { i1, double } %t14, 1     ; the float inside
  %t16 = fcmp ogt double %t10, %t15
  br label %end2
  ...
then3:
  %t18 = insertvalue { i1, double } { i1 true, double undef }, double %t10, 1
  store { i1, double } %t18, ptr %var.best
```

`extractvalue` was met at the overflow check. `insertvalue` is its
partner: it takes a pair and gives back a new one with one half
replaced. `undef` is "any bits will do", for the half about to be
filled in. `xor` with `true` is how IR writes `not`.

After `best is none or`, the checker knows `best` is a float on the
right side, and it left a note saying so (`types.narrowed`). The
emitter reads the note and takes half 1 out without testing again
(`present`). There is one case where it does test again: a `var` that
some other function could have set back to `none` in between. The C
backend does the same, and the message is the same.

Four functions do all of it. `coerce` makes a `T` into a `T or none`
where one is wanted. `is_there` gives the `i1`. `inside` takes the
value out. `is_none` compiles `x is none` (decision 5.11j).

## Helpers written when they are first needed

`print(shapes[0])` has to turn a `Shape` into text. No instruction does
that, so the emitter writes a function for it, the first time one is
needed, and only then:

```llvm
define internal ptr @zeno.show.Point(ptr %self) {
entry:
  %t1 = getelementptr %z_Point, ptr %self, i32 0, i32 0
  %t2 = load double, ptr %t1
  %t3 = call ptr @zeno_str_float(double %t2)
  %t4 = getelementptr %z_Point, ptr %self, i32 0, i32 1
  %t5 = load double, ptr %t4
  %t6 = call ptr @zeno_str_float(double %t5)
  %t7 = call ptr (i32, ...) @zeno_join(i32 5, ptr @.str.2, ptr %t3, ptr @.str.3, ptr %t6, ptr @.str.4)
  ret ptr %t7
}
```

`@zeno.show.Shape` reads the tag and uses `switch`, the one instruction
that jumps more than two ways, then calls `@zeno.show.Point` for the
centre of a circle. `==` on two objects gets `@zeno.same.Point` in the
same way, and `float or none` gets `@zeno.text.opt_float`.

The emitter is in the middle of some other function when it finds it
needs one of these. `side` handles that: it puts the function being
written to one side, writes the helper from an empty start, and puts
the first one back. The helper's name goes into a set before its body
is written, so a class with a field of its own kind (a tree, a linked
list) does not go round for ever (decision 5.11k).

The runtime's list functions need the same thing from the other
direction. `zeno_list_str` joins the items of a list, and it is C: it
cannot know how to show a `Shape`. So it takes a function to call for
each item, and the emitter writes a small **adapter** that takes the
`i64` slot, turns it back into the real kind, and calls the helper
(`@zeno.item.repr.…`, `@zeno.item.eq.…` for `in` and `==`).

## Errors: `try` and `raise`

The C backend has two ways to do errors ([`firmware.md`](firmware.md)).
The ordinary one is `setjmp` and `longjmp`: `try` marks a place, and
`raise` jumps back to it from however deep. `zeno build --llvm` uses
that one, and this section shows it. `zeno build --llvm --firmware`
uses the other, where an error is a value that each caller tests. That
is the section after this one.

`raise` is short:

```llvm
then1:
  %t3 = call ptr @zeno_str_int(i64 %arg.n)
  %t4 = call ptr (i32, ...) @zeno_join(i32 2, ptr %t3, ptr @.str.12)
  %t5 = call ptr @zeno_error_new(ptr @.str.10, ptr %t4, ptr null)
  call void @zeno_raise(ptr %t5, i32 30, i32 21)
  unreachable
```

An error value is three addresses: its kind (`"ValueError"`), its
message, and the object when a class of the program's own was raised.

`try` is the larger half:

```llvm
entry:
  %try.size.1 = call i64 @zeno_try_size()
  %try.1 = alloca i8, i64 %try.size.1, align 16
  ...
  call void @zeno_try_open(ptr %try.1)
  %t19 = call i32 @_setjmp(ptr %try.1) returns_twice
  %t20 = icmp eq i32 %t19, 0
  br i1 %t20, label %try1, label %caught1

try1:
  ...                                    ; the body
  call void @zeno_try_close(ptr %try.1)
  br label %end1

caught1:
  %t27 = call i1 @zeno_caught(ptr @.str.10)      ; is it a ValueError?
  br i1 %t27, label %except1.1, label %caught1.1

except1.1:
  %t28 = load ptr, ptr @zeno_raised              ; `as error`
  ...
  br label %end1

caught1.1:
  call void @zeno_raise_again()                  ; not ours: pass it on
  unreachable
```

- **`_setjmp` returns twice.** Once now, with 0, and the body runs.
  Once more if something raises, with 1, as if the call had only just
  come back. The words `returns_twice` tell LLVM so. Without them it
  would optimise as if that could not happen.
- **The buffer.** `setjmp` saves the processor's registers in a
  `jmp_buf`, and how big that is differs from one C library to the
  next. The IR must not guess. So the runtime is asked
  (`zeno_try_size`), and the `alloca` takes that many bytes. This is
  one of the three functions added to the runtime for LLVM. The other
  two, `zeno_try_open` and `zeno_try_close`, put the buffer on the
  runtime's list of open `try`s and take it off.
- **Leaving early.** A `return`, `break` or `continue` inside a `try`
  jumps out without reaching `zeno_try_close`. `leave_tries` writes the
  close before the jump, once for every `try` being left.

### Why the slots are `volatile`

There is a trap in `setjmp`, and it is the reason for one more word in
the IR. Take this:

```
function steps(n: int) -> int {
    var count = 0
    var left = n
    try {
        while left > 1 {
            left = halve(left)
            count = count + 1
        }
    } except ValueError as error {
        print("stopped:", error.message)
    }
    return count
}

print(steps(64), steps(40))
```

`steps(40)` halves three times (40, 20, 10, 5), then 5 is odd, and the
answer should be 3. `setjmp` saved the registers at the top of the
`try`. If the optimiser keeps `count` in a register through the loop,
the jump back puts the old register back, and `count` is 0 again.

C has the same trap and the same answer: a variable changed inside a
`try` and read after it must be `volatile`, which means "really read
and really write memory, every time". So in a function that has a
`try`, every slot is read and written that way:

```llvm
  %t7 = load volatile i64, ptr %var.count
  ...
  store volatile i64 %t9, ptr %var.count
```

and `mem2reg` leaves those slots alone. Functions with no `try` are
not touched, so nothing else pays for it. Exercise 6 takes the word out
by hand, and the program prints `6 0` (decision 5.11l).

## Errors as values: `--llvm --firmware`

```sh
cargo run -- build --llvm --firmware examples/sensor.zeno
grep -c setjmp sensor.ll sensor_runtime.c     # 0 and 0
```

Coding standards for firmware do not allow `setjmp`
([`firmware.md`](firmware.md) says why). So a firmware build carries
an error the slow, plain way: as a return value. The C backend has
done this since its firmware stage. This is the same design, written
in IR, and the two builds must print the same thing.

Take the `steps` program from just above, and build it this way.

**A function that can raise** gives back an `i1`: 1 for "it worked", 0
for "it raised". Its real answer goes out through one more parameter,
a pointer the caller supplies:

```llvm
define internal zeroext i1 @z_halve(i64 %arg.n, ptr %result) {
entry:
  %t1 = call i64 @zeno.mod(i64 %arg.n, i64 2, i32 2, i32 8)
  %t2 = icmp ne i64 %t1, 0
  br i1 %t2, label %then1, label %end1

then1:
  %t3 = call ptr @zeno_str_int(i64 %arg.n)
  %t4 = call ptr (i32, ...) @zeno_join(i32 2, ptr %t3, ptr @.str.1)
  %t5 = call ptr @zeno_error_new(ptr @.str.2, ptr %t4, ptr null)
  call void @zeno_note(ptr %t5, i32 3, i32 9)      ; raise: note the error...
  ret i1 false                                     ; ...and say "it failed"

end1:
  %t6 = call i64 @zeno.floordiv(i64 %arg.n, i64 2, i32 5, i32 12)
  store i64 %t6, ptr %result                       ; return: the answer...
  ret i1 true                                      ; ...and "it worked"
}
```

`zeno_note` puts the error in the runtime's `zeno_raised`, with its
line and column. Nothing jumps. `raise` is a `ret`.

**A call to such a function** is followed at once by a test:

```llvm
do2:
  %t3 = load i64, ptr %var.left
  %t4 = call zeroext i1 @z_halve(i64 %t3, ptr %answer.3)
  br i1 %t4, label %worked4, label %failed4

failed4:
  br label %except1               ; inside a `try`: go to its `except`s

worked4:
  %t5 = load i64, ptr %answer.3   ; the answer `halve` stored
```

`%answer.3` is an `alloca` at the top of `steps`, one for each such
call. Where `failed4` goes depends on where the call is, and it is the
table from `firmware.md` again (`failed` in `emit_llvm.rs`):

| Where | What a failure does |
|---|---|
| Inside a `try` in this function | `br label %except1` |
| In a function that says `raises` | `ret i1 false` |
| At the top of the file | `call void @zeno_uncaught()`, which prints the error and stops |

**`try`** has no `setjmp`, no buffer, no `zeno_try_open`. The body is
just the body. Its end, and every failure inside it, arrive at one
label:

```llvm
done2:                                    ; the loop ended: the body is over
  br label %except1

except1:
  %t10 = load ptr, ptr @zeno_raised
  %t11 = icmp ne ptr %t10, null           ; did anything fail?
  br i1 %t11, label %caught1, label %end1

caught1:
  store ptr null, ptr @zeno_raised        ; it is being dealt with
  %t12 = call i1 @zeno_is(ptr %t10, ptr @.str.2)       ; a ValueError?
  br i1 %t12, label %except1.1, label %caught1.1

except1.1:
  %t13 = getelementptr { ptr, ptr, ptr }, ptr %t10, i32 0, i32 1
  %t14 = load ptr, ptr %t13                            ; error.message
  call void (i32, ...) @zeno_print(i32 2, ptr @.str.3, ptr %t14)
  br label %end1

caught1.1:
  store ptr %t10, ptr @zeno_raised        ; not ours: put it back
  call void @zeno_uncaught()
  unreachable
```

`caught1.1` cannot be reached here: the body can only raise a
`ValueError`, and there is an `except` for it. The block is written
all the same, because a `br` needs two labels.

Now look for the word `volatile`. It is gone:

```llvm
  %t6 = load i64, ptr %var.count
  ...
  store i64 %t8, ptr %var.count
```

`volatile` was there because a jump could put old registers back.
Nothing jumps now, so `count` can live in a register through the whole
loop, and `mem2reg` is free to put it there. `return`, `break` and
`continue` inside a `try` need no `zeno_try_close` either, because
there is nothing to close. The plain way of carrying errors is also
the one the optimiser understands best.

What it costs is a test after every call to a function that can
raise, whether or not it raised. Exercise 16 measures both.

**What the runtime finds for itself**, such as a division by zero or
an index past the end, stops a firmware build, as in C. The runtime
file written beside the IR is the firmware one, where `zeno_fail`
prints and stops. `int(text)` and `float(text)` are the two runtime
functions a program can catch, so they follow the same pattern as
`halve`: `call zeroext i1 @zeno_parse_int(ptr %t1, i32 2, i32 12, ptr
%answer.1)`.

**What it refuses** is what the C firmware build refuses, in the same
words: a `try` in which nothing can be caught, and a function that can
raise used as a value.

## Functions as values

A function held in a variable is a `ptr` to a small **block**:

```
{ ptr code, ptr name, the variables it uses from outside ... }
```

`make_counter` in the demo builds one:

```llvm
define internal ptr @z_make_counter() {
entry:
  %t1 = call ptr @zeno_alloc(i64 ptrtoint (ptr getelementptr (i64, ptr null, i32 1) to i64))
  store i64 0, ptr %t1                          ; `count`, in a box of its own
  %t2 = call ptr @zeno_alloc(i64 ptrtoint (ptr getelementptr ({ ptr, ptr, ptr }, ptr null, i32 1) to i64))
  store ptr @zeno.fn.1, ptr %t2                 ; the code
  %t3 = getelementptr { ptr, ptr, ptr }, ptr %t2, i32 0, i32 1
  store ptr @.str.8, ptr %t3                    ; the name: none, it was written in place
  %t4 = getelementptr { ptr, ptr, ptr }, ptr %t2, i32 0, i32 2
  store ptr %t1, ptr %t4                        ; where `count` is
  ret ptr %t2
}
```

The function written in place becomes an ordinary function with one
more parameter in front, the block it was called through:

```llvm
define internal i64 @zeno.fn.1(ptr %self) {
entry:
  %t1 = getelementptr { ptr, ptr, ptr }, ptr %self, i32 0, i32 2
  %t2 = load ptr, ptr %t1          ; the box
  %t3 = load i64, ptr %t2          ; count
  ...
  store i64 %t5, ptr %t2           ; count = count + 1
```

And a call loads the code out of the block and calls it, with the block
as the first argument:

```llvm
  %t34 = load ptr, ptr @z_next
  %t35 = load ptr, ptr %t34
  %t36 = call i64 %t35(ptr %t34)
```

Two rules decide what goes into the block (decision 5.11m):

- A **`let`** cannot change, so its value is copied in.
- A **`var`** can, and the function and the place it was made must see
  the same one. Here `count` even outlives the call to `make_counter`
  that made it. So a `var` that an inner function uses does not get an
  `alloca`. It gets a **box**, eight bytes from the collector, and the
  block holds the address of the box. The emitter looks for these
  before it compiles a function's body (`Code.shared`).

A named function used as a value (`let f = area`) has no block yet.
The emitter writes a constant one, and a small wrapper that takes the
unused `self` and calls the real function, so that every function value
is called the same way.

`map`, `filter` and `reduce` are loops written in IR that call the
function value for each item (`over_list`).

## Units and `import math`

A number with a unit is a `double`. The checker has made sure the units
agree, and what is left for the emitter is the scale:

```
function total(wait: s, extra: ms) -> ms {
    return wait + extra
}
```

```llvm
define internal double @z_total(double %arg.wait, double %arg.extra) {
entry:
  %t1 = fmul double %arg.wait, 1000.0
  %t2 = fadd double %t1, %arg.extra
  ret double %t2
}
```

The seconds are brought to milliseconds and added. Printing calls
`zeno_str_quantity` with the unit's name, which is all that is left of
the unit in the program.

`import math` is known to the emitter, as it is to the C backend:
`math.sqrt(x)` is `call double @zeno_sqrt(double %x, i32 line, i32 col)`,
where the runtime checks for a negative number, and `math.pi` is the
constant written out. Another module is refused.

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

## Owned values: a slot, a null and a drop

An `owned` list or object has one owner, and it is freed where that
owner ends ([`ownership.md`](ownership.md)). The checker has already
done the hard part: it knows which variables are owners, and which
expressions give a value away. The emitter asks it two questions
(`types.owner`, `types.moves`) and writes three small things.

```
owned class Frame {
    id: int
    bytes: owned list[int]
}

function make(id: int) -> Frame {
    let bytes: owned list[int] = [id, id * 2]
    return Frame(id, bytes)
}

function size(frame: take Frame) -> int {
    return len(frame.bytes)
}

function first_big(limit: int) -> int {
    for id in range(100) {
        let frame = make(id)
        if frame.bytes[1] > limit { return id }
    }
    return -1
}

let frame = make(7)
print(size(frame))
print(first_big(20))
```

**An owner is always a slot.** Earlier on this page a `let` was a
value and only a `var` was a slot. An owner is the exception. `bytes`
is a `let`, and it still gets an `alloca`:

```llvm
define internal ptr @z_make(i64 %arg.id) {
entry:
  %var.bytes = alloca ptr
  ...
  %t4 = call ptr (i32, ...) @zeno_list_of(i32 2, i64 %arg.id, i64 %t2)
  store ptr %t4, ptr %var.bytes
  %t5 = load ptr, ptr %var.bytes            ; `bytes` gives its list away:
  store ptr null, ptr %var.bytes            ; copy it out, write null
  %t6 = call ptr @zeno.new.Frame(i64 %arg.id, ptr %t5)
  %t7 = load ptr, ptr %var.bytes            ; end of the function:
  call void @zeno_list_drop(ptr %t7)        ; free whatever is still there
  ret ptr %t6
}
```

The reason is the second line of the pair. **Giving away writes
null**, and a value in IR cannot be written to twice. Only memory can.

**The end of a block asks each owner.** `zeno_list_drop` does nothing
when it is handed null. So the emitter does not work out which owners
still hold something at the end of a block. It loads each one and
calls the drop, and the pointer is the answer (`free_owned`).

Here `%t7` is always null. `opt -O2` works that out: the slot goes and
the call becomes `zeno_list_drop(ptr null)`. It cannot take the call
away, though. `zeno_list_drop` is in the other file, so LLVM does not
know that it does nothing for null. The C build can, because there the
runtime is in the same file. Exercise 14 closes the gap.

A parameter that takes a value over (it says `take`) is an owner too, so it is moved
into a slot at the top of the function and dropped at the end:

```llvm
define internal i64 @z_size(ptr %arg.frame) {
entry:
  %var.frame = alloca ptr
  store ptr %arg.frame, ptr %var.frame
  ...
  %t4 = load i64, ptr %t3                   ; the answer is worked out first
  %t5 = load ptr, ptr %var.frame
  call void @zeno.drop.Frame(ptr %t5)       ; then the frame is freed
  ret i64 %t4
}
```

**Each owned class gets a drop function**, written the first time it
is needed (`drop_class`). It frees what the fields own, then the
object:

```llvm
define internal void @zeno.drop.Frame(ptr %self) {
entry:
  %t1 = icmp eq ptr %self, null
  br i1 %t1, label %gone1, label %there1

gone1:
  ret void

there1:
  %t2 = getelementptr %z_Frame, ptr %self, i32 0, i32 1
  %t3 = load ptr, ptr %t2
  call void @zeno_list_drop(ptr %t3)        ; the list the frame owns
  call void @zeno_free(ptr %self)           ; then the frame
  ret void
}
```

**Every way out of a block is an end of that block.** In `first_big`
the `return` inside the loop leaves the body early, so it gets the
same drop the closing `}` gets:

```llvm
then2:
  %t10 = load ptr, ptr %var.frame
  call void @zeno.drop.Frame(ptr %t10)      ; `return id` leaves the body
  ret i64 %t1

end2:
  %t11 = load ptr, ptr %var.frame
  call void @zeno.drop.Frame(ptr %t11)      ; the `}` of the body
  br label %next1
```

`break`, `continue`, `raise` and (in a firmware build) the `failed`
block after a call all do the same. Each one frees the scopes it is
leaving and no more: a `continue` frees the body's owners, a `return`
frees the function's.

An assignment to an owner frees the old value, after the new one has
been made. An owned value that nobody takes, such as `make(3)` written
as a statement, is freed on the spot.

Some values are owned by nobody for a moment: a list while a
comprehension fills it, or an owned argument that has been worked out
while the next argument is still to come. If something fails in that
moment, nobody would free it. `hold` notes such a value and `let_go`
forgets it, and whatever is noted is freed with the owners when a
block is left early ([`no-collector.md`](no-collector.md), "A value
between two owners").

In an ordinary build an error is a jump, and the jump passes over the
functions between the `raise` and the `try`. What they owned is left
for the collector, as in the C build. The one trap is in the function
that has the `try`: a slot written since `_setjmp` might come back
with its old value. That is the `volatile` rule from the section on
errors, and it already covers every slot of such a function, owners
included.

Build the program with `--llvm --memory` and the last number on the
line is the blocks freed this way:

```
memory: 41 blocks made (733 bytes), at most 58 bytes in use at once, 0 collections, 39 blocks freed without the collector
```

The two that are left are the texts `print` made for its numbers.

## With no collector: `--llvm --no-collector`

```sh
cargo run -- build --llvm --no-collector --memory examples/buffer.zeno
./buffer
grep -c zeno_collect buffer.ll buffer_runtime.c     # 0 and 0
```

[`no-collector.md`](no-collector.md) is the design: owned values are
freed by their owners, text lives in a fixed room until the end of the
statement that built it, text that is kept is a copy
([`text.md`](text.md)), and the compiler refuses what would need
anything more. This is that design again in IR. The runtime file
written beside the IR is the one the C build uses, with the collector
left out of it.

Three things are missing from the top of the file: `zeno_roots`,
`@zeno_stack_base` and the `%base` slot in `main`. They were all for
the collector. One declaration is new:

```llvm
@zeno_text_top = external global i64
```

That is how full the text room is. It is the whole of memory
management for text:

```
function halve(n: int) -> int raises ValueError {
    if n % 2 == 1 { raise ValueError(f"{n} is odd") }
    return n // 2
}

var total = 0
for n in range(1, 5) {
    print(f"reading {n} of 4")
    try {
        total = total + halve(n)
    } except ValueError as error {
        print("skipped:", error.message)
    }
}
print(f"total {total}")
```

```llvm
do1:
  %mark.1 = load i64, ptr @zeno_text_top    ; how full is the room now?
  %t3 = call ptr @zeno_str_int(i64 %t1)
  %t4 = call ptr (i32, ...) @zeno_join(i32 3, ptr @.str.1, ptr %t3, ptr @.str.2)
  call void (i32, ...) @zeno_print(i32 1, ptr %t4)
  store i64 %mark.1, ptr @zeno_text_top     ; the statement is over: back to there
```

**A statement that builds text notes the mark first and puts it back
after.** Everything it built is gone in one `store`. A statement that
builds no text gets neither line.

The emitter does not know that a statement builds text until it has
compiled it, and by then the `load` belongs several lines up. So
`text_begin` only remembers the line it is at, and `text_end` goes
back and inserts the `load` there if text was built. An `if`, a
`while`, a `return` and a `raise` do the same round their own
expression, and a comprehension does it for each item.

**A `try` puts the mark back where failures arrive.** A failure half
way through a statement skips that statement's own `store`:

```llvm
  %t5 = load i64, ptr @zeno_text_top        ; before the body of the `try`
  ...
except2:
  store i64 %t5, ptr @zeno_text_top         ; however the body ended
  %t12 = load ptr, ptr @zeno_raised
```

**An error is owned by the `except` that catches it.** The error is
one block, made by `zeno_error_new`, with a copy of its message in the
same block. (The message is copied out of the text room, which is why
`raise` can put the mark back.) The name after `as` is an owner slot,
freed like any other when the handler is over:

```llvm
except2.1:
  store ptr %t12, ptr %caught.2.1           ; `error` owns it
  ...
  call void (i32, ...) @zeno_print(i32 2, ptr @.str.4, ptr %t17)
  ...
  %t18 = load ptr, ptr %caught.2.1
  call void @zeno_error_drop(ptr %t18)      ; the handler is over
  br label %end2
```

A `return` or a `continue` inside the handler gets the same call. If
the handler kept the error or raised it again, the slot holds null by
then and the call frees nothing
([`functions-and-errors.md`](functions-and-errors.md)).

**Text that is kept is copied.** The room is about to be emptied, so
wherever text is stored (a `let`, an assignment, a field, a list
item) the IR calls `zeno_text_keep`, which copies it into the pool
unless it is written in the program. The slot that holds the copy is
an owner, and the end of its block calls `zeno_text_drop`. A `return`
of text calls `zeno_text_return`, which leaves the answer in the room
for the caller. [`text.md`](text.md), "Through LLVM", shows the IR.
Each value the emitter makes carries one bit for this (`fixed` in
`Val`): text that is not fixed is in the room.

**What a statement makes and gives to nobody is the statement's.** In
`print(count(line.split()))` the list of words has no owner. Its
address goes into a slot of the statement's own, and the statement
frees it when it is over:

```llvm
  store ptr null, ptr %own.1                 ; where the statement begins
  %t2 = call ptr @zeno_split_blanks(ptr %t1)
  store ptr %t2, ptr %own.1
  ...
  %t5 = load ptr, ptr %own.1                 ; where it ends
  call void @zeno_list_drop_text(ptr %t5)
```

The slot is an owner like any other while the statement runs, so a
`ret i1 false` in the middle of the statement frees the list on its
way out. It is a slot and not simply `%t2` because the list may be
made in a block that is skipped (the right side of `and`), and a name
from such a block cannot be used after it. The `store ptr null` line
is written where the statement began once the statement is finished,
as the `%mark` line is. [`temporaries.md`](temporaries.md) is the
walkthrough; `own_begin`, `own_end` and `belongs` are the code.

**What it refuses** is what the C build refuses, in the same words,
because both call the same function for the message
(`emit_c::without_collector`).

### Why an f-string waits

```
print(f"{a} then {next()}")
```

The values are worked out left to right, as always. But a number is
not turned into text until all the values are known:

```llvm
  %t1 = load i64, ptr @z_a
  %t2 = call i64 @z_next()
  %t7 = call ptr @zeno_str_int(i64 %t1)     ; only now
  %t8 = call ptr @zeno_str_int(i64 %t2)
```

Text in the room while `next()` runs is room `next()` cannot use. If
`next()` is the function this line is in, every level of the recursion
would hold one text more. The C build never did that, and its numbers
for "at most N bytes of text in use" were lower until this was done
the same way.

It is safe for a number, a bool, a quantity and text, because a later
call cannot change the value that was read. A list or an object is
turned into text at once: a later call could append to it, and the
f-string has to show it as it was.

### `--memory`

`zeno build --memory` makes the program print what it used when it
ends. Through LLVM that is one line in `main` and one flag:

```llvm
  %asked = call i32 @atexit(ptr @zeno_memory_report)
```

and `-DZENO_MEMORY_REPORT` for the runtime file, so that it counts.
With no collector the report has a line for the pool that blocks come
from ([`pool.md`](pool.md)) and one for the text room:

```
memory: 2 blocks made (66 bytes), at most 33 bytes in use at once, no collector, 0 blocks left at the end
pool: 72 bytes taken, in a pool of 32768
text: at most 17 bytes in use at once, in a room of 1024
```

Those are the two errors with their messages, both freed, the one
64-byte block of the pool that held each in turn, and the longest line
the program ever built. The C build of the same program prints the
same three lines, and the tests require that.

The pool is in the runtime file, so the IR knows nothing of it. To
choose its size, build by hand with `-DZENO_POOL=`, as exercise 19
does for the text room.

## The tests

`same_as_interpreter` in `emit_llvm.rs` runs a program on the
interpreter and the VM (`vm::run_both`), builds it with clang, and
requires the same output and, when it stops, the same error at the same
line and column. The runtime is built with `ZENO_COLLECT_ALWAYS`, which
runs the collector before every allocation.

The C backend's own tests end by calling it too. So each of the 52
programs written to test the C backend is also built through LLVM, and
a difference between the two backends shows up as a failed test
(decision 5.11g).

Two tests compare IR with what it should be, character for character:
one for a small function and one for a class. A change to the shape of
the IR is noticed. One checks that everything in `DECLARED` is in the
runtime.

Every one of those programs is built a second time for firmware
(`--llvm --firmware`) and must print the same again. A refusal has to
say `--firmware`, as in the C backend's tests. On top of that:
`a_firmware_build_has_nothing_to_jump_with` (no `setjmp`, `zeno_try`
or `volatile` in the IR, and a function that cannot raise is the same
in both kinds of build), `the_ir_for_a_try_in_a_firmware_build`,
`what_a_firmware_build_refuses_says_so`, and
`a_fault_stops_a_firmware_build_where_an_ordinary_one_catches_it`.
The two long programs the C backend uses for errors as values must be
accepted here too (`firmware_as_interpreter`).

Owned values: `the_ir_for_an_owned_value` reads the IR above, and
`owned_values_are_freed_without_the_collector` builds a program that
makes four thousand owned blocks with `--memory` and requires that all
of them were freed by their owners and none by the collector.

With no collector the test is stricter than "prints the same". Each
of the C backend's no-collector programs is built both ways with
`--memory`, and the two **memory reports must be the same line for
line** (`no_collector_as_c`): blocks made, bytes, the most in use at
once, nothing left at the end, how much of the pool was taken, and
the most text in the room. A free
that is missing, or one that comes a statement late, changes a number.
Each refusal is tried through LLVM too and must use the C build's
words (`refuses_as_c`).

Set `ZENO_VALGRIND=1` and the no-collector tests run each program
under valgrind. (It was most use while blocks came from the heap,
where it stops at a block freed twice or read after it was freed. It
cannot see inside the pool. The pool catches a block given back twice
by itself.)

```sh
ZENO_VALGRIND=1 cargo test no_collector
```

If clang is not installed the compiled half of each test is skipped.

## How fast

On the machine it was written on, with `-O2`, best of several runs:

| Program | C, gcc | C, clang | LLVM |
|---|---|---|---|
| `fib(38)` | 0.22 s | 0.19 s | 0.19 s |
| 300 million times `total += i % 7` | 0.29 s | 0.28 s | 0.28 s |
| Objects, an enum with `match`, lists | 0.16 s | 0.12 s | 0.14 s |

The IR written here ends up as much the same machine code clang makes
from Zeno's C. That is the point of the stage: LLVM does not need C to
sit in between.

The second row was 0.76 s when `%` was a call into the runtime. Writing
it in IR is what brought it level.

The third row is a little behind clang on the C. That program spends
its time in `zeno_alloc` and the list functions, which are in the other
file. In a C build they are in the same file and can be inlined.

## What it does not do yet

- **What `--no-collector` refuses in C it refuses here**, in the same
  words ([`no-collector.md`](no-collector.md), "What is refused"). A
  function written where a value goes is a block with a `drop` entry,
  freed by its owner ([`functions-and-errors.md`](functions-and-errors.md)). Lists of
  objects and lists of lists are freed with their items, by functions
  written into the IR when a program needs them (`drop_list`;
  [`one-owner.md`](one-owner.md)). An enum value that carries something
  is freed by `drop_enum`, which goes by its tag
  ([`enums-and-none.md`](enums-and-none.md)).
- **What the C backend also refuses**: a named function, class or enum
  declared inside a function, a default for a parameter that is not
  a literal, a method used as a value.
  The message says so, and `zeno run` runs all of them.
- **It needs clang**, version 15 or later (the first to write every
  pointer as `ptr`). gcc cannot read IR.
- **The runtime is not inlined**, as above, and all of it is linked in:
  a small program is about 38 KB where the C build is 18 to 22 KB.
- **64-bit processors only**, and the slot of a list assumes the bytes
  of a number are stored lowest first, which is true of every processor
  it is likely to meet. `_setjmp` is the name on Linux and macOS.
- **No debug information.** A debugger shows the program as assembly.

One thing it does that the C backend does not: an assignment through
any expression, such as `grid[0][1] = 5`.

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
4. Build the demo and run `opt -O2 -S demo.ll`. `z_area` is gone:
   find where it went, and find the `switch` the three tests turned
   into. It lists two cases, not three. Where does the third go, and
   what told LLVM that there is no fourth?
5. Give `Point` a third field of kind `bool` and look at the size
   `zeno_alloc` is asked for (`opt -O2 -S` folds it to a number). It is
   not 17. Put the `bool` first instead. Does the size change? Does the
   number in the last `getelementptr` index change?
6. Save the `steps` program above as `tries.zeno` and build it. Then
   take the word out and build again by hand:
   `sed 's/ volatile//' tries.ll > loose.ll`, then
   `clang -O2 -Wno-override-module -o loose loose.ll tries_runtime.c -lm`.
   Run both. Now build `loose` with `-O0`. Why is it right again?
7. Run `opt -passes=mem2reg -S tries.ll` and look at `z_steps`, then at
   `z_halve`. Which slots are still there? `left` is never read after
   the `try`. Could it be left out of the rule? What would the emitter
   have to know?
8. `math.sqrt` calls into the runtime. Write it in IR, as an entry in
   `WRITTEN` next to `zeno.mod`: compare with `fcmp olt`, raise through
   a runtime function when it is negative, otherwise call
   `llvm.sqrt.f64`. The test `units_and_imports_match` will tell you
   when it is right.
9. Do the same for `zeno_list_at`: the check on the index in IR, and a
   call to the runtime only for the error. Time a loop that reads
   `items[i]` a hundred million times before and after.
10. `bool or none` is `{ i1, i1 }`, two bytes for three possible
    values. Make it one `i8`: 0 for `none`, 1 for `false`, 2 for
    `true`. Which functions have to change? (`coerce`, `is_there`,
    `inside`, and what puts it in a list.)
11. In `make_counter`, `count` has to be in a box. Write a function
    where a captured `var` is never assigned after the inner function
    is made. A copy would do there. How could the emitter tell? What
    does the checker already know that would help?
12. Add `-flto` to the clang line in `src/main.rs`. That lets LLVM
    optimise across the two files. Does the third row of the table
    change? What does it need installed?
13. Give the IR another processor: `clang --target=riscv64 -O2 -S primes.ll`
    writes `primes.s`. Find an overflow check in it (look for
    `zeno_overflow`). Nothing for RISC-V had to be installed first. Why
    not, and what would be needed to make a program that runs?
14. Build the `Frame` program above with `--llvm` and run
    `opt -O2 -S frame.ll`. Find `zeno_list_drop(ptr null)`: a call
    that does nothing. The drop of a `Frame` that is null has gone,
    because `zeno.drop.Frame` is in the IR and LLVM can read its null
    test. Give lists the same: an entry in `WRITTEN` called
    `zeno.drop.list` that tests for null and only then calls the
    runtime, and use it in `free`. Run `opt` again.
15. Build `examples/sensor.zeno` with `--llvm` and again with
    `--llvm --firmware` (keep the first `sensor.ll` under another
    name). Compare `z_read_all` in the two files. Which has more
    blocks? Which has `volatile`? Then run `opt -passes=mem2reg -S` on
    both and count the `alloca`s that are left in each.
16. Time it. Write a loop that calls `halve` twenty million times with
    an even number, and build it both ways. Which is faster when
    nothing is ever raised? Now make every tenth call raise, with the
    `try` inside the loop. Which is faster now, and why does the
    ordinary build suffer more?
17. A firmware build gives back an `i1` and writes the answer through
    a pointer. The other way is to give back both together, as
    `{ i1, i64 }`: you have already met that type, for `int or none`.
    Change `function`, the `return` statement and `checked_call` to do
    that, and read `opt -O2 -S` of both. Is the machine code
    different? What would a `list[int]` answer look like?
18. In `first_big`, write `break` in place of `return id`. Before you
    build it, write down the IR you expect for the `then` block. Then
    put a second owner in the function, above the loop. Does `break`
    free it? Does `return`?
19. Build the `halve` program above with `--llvm --no-collector
    --memory`, then again with `-DZENO_TEXT_ROOM=16` added by hand:
    `clang -O2 -Wno-override-module -DZENO_MEMORY_REPORT
    -DZENO_TEXT_ROOM=16 -o small room.ll room_runtime.c -lm`. What
    happens, and at which line of the program? What is the smallest
    room it runs in?
20. In `room.ll`, delete the `store i64 %mark.1, ptr @zeno_text_top`
    that follows the first `zeno_print` in `zeno_program`, and build
    it by hand as in 19 (without the small room). The output is the
    same. Which number in the report changes, and by how much for each
    trip round the loop? What would a thousand trips do? This is the
    bug the test `no_collector_as_c` is there to catch.
21. `text_begin` goes back and inserts the `load` only when text was
    built. The simpler way is to write the `load` and the `store` for
    every statement. Do that, and look at `opt -O2 -S` of a function
    that builds no text. Are they still there? Why can LLVM not remove
    a `load` and a `store` of a global round a call?
22. A `for` whose first line builds text, such as
    `for i in range(len(str(x))) { }`, gives the text back before the
    first trip. Find `text_given_back` in `emit_llvm.rs` and the two
    places that call it. Why is it called after the list is copied
    and not before? (Think of `for name in [str(x), str(y)]`.)
