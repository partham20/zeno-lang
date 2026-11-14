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
`raise` jumps back to it from however deep. This backend uses that one,
because the runtime that raises (`zeno_raise`, and every
`IndexError` and `OverflowError` inside the runtime) already works that
way.

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

- **`--firmware`, `--no-collector`, `--memory`.** They do not go with
  `--llvm`. For firmware the C backend is the one to use.
- **Owned values are not freed early.** An `owned` list or object is
  held like a shared one and the collector frees it. The program
  prints the same, and uses memory for longer (decision 5.11n).
- **What the C backend also refuses**: a named function, class or enum
  declared inside a function, a module Zeno does not have, a default
  for a parameter that is not a literal, a method used as a value.
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
14. A big one: free an `owned` value at the end of its scope, as
    `zeno build` does ([`ownership.md`](ownership.md)). Start with a
    list that never leaves the function.
15. The biggest: `--llvm --firmware`. Errors become values there
    ([`firmware.md`](firmware.md)): a function that can raise returns
    its result and an error together. What IR type would that be? You
    have already met one that pairs a value with a flag.
