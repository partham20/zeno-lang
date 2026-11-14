# How generics work

A ring buffer of bytes and a ring buffer of voltages are the same code.
So are "the biggest int in a list" and "the biggest float in a list".
Without generics there are two ways to write them: once for each type,
by hand, or once with the types left out, and then the checker can no
longer say what comes back.

A **generic** is written once, with a name standing for the type:

```zeno
function first[T](items: list[T]) -> T {
    return items[0]
}

class Stack[T] {
    items: list[T]

    function push(self, item: T) {
        self.items.append(item)
    }
}
```

`T` is a **type parameter**. `first([1, 2])` gives an `int`,
`first(["a"])` gives a `str`, and the checker knows which. A
`Stack[int]` takes only ints, and what it pops is an int.

`examples/generics.zeno` is a ring buffer written this way, used for
bytes from a serial port, for voltages and for text:

```sh
cargo run -- run examples/generics.zeno
cargo run -- build examples/generics.zeno          # generics.c, and ./generics
cargo run -- build --llvm examples/generics.zeno   # generics.ll, and ./generics
```

```
bytes: [108, 108, 111, 33] newest 33 dropped 2
above 0x60: 3
as hex: ["6c", "6c", "6f", "21"]
no reading yet: none
volts: [3.3 V, 2.9 V, 3.2 V] highest 3.3 V
low: 1
log: ["sensor ready", "first reading"] last in the alphabet: sensor ready
lengths: [12, 13]
Ring(slots=["first reading", "sensor ready"], next=1, count=2, dropped=1)
```

The reasons for every choice are in [`decisions.md`](decisions.md):
3.16 for the language and 5.14 for how it is built.

## The rules

- **Type parameters go in square brackets after the name**:
  `function first[T](...)`, `class Pair[A, B] { ... }`. Each starts
  with a capital letter.
- **Inside the generic, the name is a type** like `int`:
  `var out: list[T] = []`, `-> T or none`, `function(T) -> U`.
- **A generic class with its types filled in is a type**: `Stack[int]`,
  `Pair[str, list[float]]`, `list[Stack[int]]`.
- **The call does not say the types. Zeno works them out**, from three
  places, in this order:
  1. the arguments: `first([1, 2])` has a `list[int]` where a
     `list[T]` goes, so `T` is `int`;
  2. an argument that is a function, once the others have said what it
     will be called with: in `convert(bytes, function(b) { ... })` the
     first argument gives `T`, and what the function returns gives `U`;
  3. the place the result goes, for whatever is still not known:
     `let s: Stack[int] = Stack(items=[])`.
- **A number that is written out waits for the others.**
  `ring(4, 0)` is a `Ring[int]`, but `let bytes: Ring[u8] = ring(4, 0)`
  is a `Ring[u8]`: the `0` can be either, and the type beside `bytes`
  says which. This is the rule for written numbers from
  [`sized-ints.md`](sized-ints.md), carried into generics.
- **The body is checked for each type it is used with.** `largest`
  uses `>`, so it works for ints, floats, text and quantities, and
  `largest([Point(1, 2)])` is an error that says so.

## One generic, many copies

This is the idea the whole stage rests on.

`first[T]` is not a function. It is a pattern for functions. When the
program calls it with a `list[int]`, the checker makes a **copy** of
the function with `T` standing for `int`, and names the copy
`first[int]`. Called with a `list[str]`, it makes `first[str]`. A
second call with a `list[int]` finds the copy already made.

A copy is an ordinary function. It is checked like one and compiled
like one. That is why most of the compiler did not have to learn what
a type parameter is: by the time the C backend sees the program, there
are no generics in it, only `first[int]` and `first[str]`.

```
function first[T](items: list[T]) -> T        the pattern, written once
        |
        |   first([1, 2])        first(["a"])        first([3])
        v
first[int](items: list[int]) -> int           made at the first call,
first[str](items: list[str]) -> str           found again at the third
```

C++ and Rust do the same thing, and call it *templates* and
*monomorphization*. Java does the opposite: it keeps one function and
forgets `T` when the program runs. Decision 5.14 says why Zeno copies.

## Through the stages

### The parser

Two small changes in `src/parser.rs`. After the name of a function or
a class, there may be names in square brackets:

```rust
fn type_params(&mut self) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    if self.eat(TokenKind::LBracket) {
        loop {
            names.push(self.name("a name for a type, such as `T`")?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RBracket)?;
    }
    Ok(names)
}
```

They are kept in a new field, `type_params`, on `Function` and `Class`
in `src/ast.rs`. `Stack[int]` as a type needed nothing new: a type
could already carry types in brackets, for `list[int]`.

There is no confusion with an index. `first[0]` in an expression is
still "item 0 of first". Type parameters only appear straight after
`function name` or `class Name`, where an index cannot.

### The interpreter and the VM

Nothing changed in either. When a program runs, every value carries
its own type: a `1` knows it is an int. `first` takes whatever it is
given and returns item 0. The one function does for every type, so
both engines run the pattern itself and never see a copy.

### The checker

All the real work is here, at the end of `src/checker.rs` under
"Generic functions and classes". It has three steps.

**1. Work out the types** (`infer`). The checker lays the written
type of each parameter over the type of the argument given for it, and
notes what each type parameter must be. `bind` does the laying over.
Its heart is short:

```rust
(Ty::List(pattern), Ty::List(actual))
| (Ty::Optional(pattern), Ty::Optional(actual))
| (Ty::Owned(pattern), Ty::Owned(actual))
| (Ty::Borrowed(pattern), Ty::Borrowed(actual)) => self.bind(pattern, actual, found),
```

A list over a list: look inside both. When the pattern is down to a
bare `T`, whatever is opposite it is what `T` is:

```
pattern   list[ T ]            Pair[ A , list[B] ]
actual    list[int]            Pair[str, list[float]]
found       T = int              A = str, B = float
```

Two arguments can speak for one `T`. A `u8` and an `int` make it
`int`, the wider one. `none` and an `int` make it `int or none`. Two
types with nothing in common leave the first, and the ordinary check
of the arguments then reports the second in plain words: "`b` of
`same[int]` should be int, but this is str".

**2. Make the copy** (`copy_function`, `copy_class`). The copy is a
copy of the syntax tree itself:

```rust
let mut decl = template.fresh();
decl.type_params.clear();
if !alone {
    decl.name = name;
}
```

`fresh` is in `src/ast.rs`. A plain `clone` would not do. The stages
after the parser keep their notes about a function by *where its nodes
are in memory*, and `first[int]` and `first[str]` need separate notes:
the `items[0]` in one is an int and in the other a str. So each copy
gets nodes of its own.

The copy is checked in a scope where `T` is declared as a name for the
type:

```rust
env.declare(param, Ty::Alias(Box::new(ty.clone())), false);
```

After that the checker does not know the function was ever generic.

**3. Carry on as an ordinary call.** `call_generic` ends by calling
the same `call` every other function goes through, with the copy.

Four tables come out of this for the compilers, in `Types`:
`function_copies` and `class_copies` (the copies of each generic),
`copy_called` (which copy each call reaches) and `annotation` (what
each written type means, since `T` means something different in each
copy).

**A mistake inside a generic** is reported in the copy it happens in,
with a note saying which:

```
error: `*` does not work between str and int
 --> box.zeno:4:16
  |
4 |         return self.value * 2
  |                ^
  = note: this is inside `Box[str]`: `Box` with T = str, which line 9 asks for
```

**A generic nothing uses** has no copy, so nothing would check it. So
that a misspelt name in it is still found, `finish_generics` checks it
once with every type parameter standing for "anything".

### The C backend

`src/emit_c.rs` changed in three places. Where it learns the
program's classes and functions, a generic is skipped and its copies
are registered:

```rust
StmtKind::Class(decl) if !decl.type_params.is_empty() => {
    for copy in types.class_copies(decl) {
        emitter.register_class(copy, statement.span)?;
    }
}
```

At a call, the checker's note says which copy to call. And a name
like `Stack[int]` cannot be written in C, so `ident` turns it into
`Stack_int`.

From this small program:

```zeno
class Cell[T] {
    value: T

    function get(self) -> T {
        return self.value
    }
}

let small: u8 = 7
print(Cell(1.5).get(), Cell(small))
```

comes this C:

```c
struct z_Cell_float {
    double z_value;
};
struct z_Cell_u8 {
    uint8_t z_value;
};

static double z_Cell_float_get(z_Cell_float *z_self) {
    return z_self->z_value;
}
static int64_t z_Cell_u8_get(z_Cell_u8 *z_self) {
    return ((int64_t)z_self->z_value);
}
```

Two structs, two functions, and a `Cell[u8]` is one byte. This is what
copying buys: each copy is exactly as big and as fast as if it had
been written by hand for that type. Nothing is looked up when the
program runs.

`--firmware` and `--no-collector` needed nothing more. An
`owned class Box[T]` works too: each copy gets its own `zeno_drop_`
function, and the tests check that a build with no collector frees
every block.

### The LLVM backend

The same three changes in `src/emit_llvm.rs`. LLVM allows any name if
it is in quotes, so the copies keep the names Zeno gave them:

```llvm
%"z_Cell[float]" = type { double }
%"z_Cell[u8]" = type { i8 }

define internal double @"z_Cell[float].get"(ptr %arg.self) {
```

## What it does not do yet

- **No bounds.** There is no way to write "`T` must be something `>`
  works on". The body is checked for each type it is used with, so a
  wrong use is still found before the program runs. But it is reported
  inside the generic, with a note, and not at the call.
- **An enum cannot be generic.** `enum Maybe[T]` is refused by the
  parser. `T or none` covers the commonest case.
- **A method cannot have type parameters of its own.** The class can.
- **A generic cannot be kept as a value**: `let f = first` is refused,
  because `f` would have to be every copy at once. Wrap it:
  `function(xs: list[int]) -> int { return first(xs) }`.
- **The types cannot be written at the call.** There is no
  `first[int](xs)`. If nothing says what `T` is, put the result where
  its type is written.
- **An owned or borrowed value cannot fill a type parameter.**
  `Stack[owned list[int]]` would need the stack to free what it holds.
- **An object of a generic class cannot be raised**, because `raises`
  and `except` take a plain name.
- **A generic inside a function is not compiled.** `zeno run` runs it.
  The compilers refuse every function with a name inside another, and
  this is the same refusal.
- **In C, `Stack[int]` and a class called `Stack_int` cannot be in one
  program.** Both would be `z_Stack_int`. The build says so. `--llvm`
  has no such limit.
- **More copies is more code.** A generic used with ten types is ten
  functions in the program. On a small chip that matters. It is the
  price of the speed.

## The tests

Every program in the tests runs on the interpreter and the VM, then
compiled through C, C for firmware, and LLVM, and all five must print
the same thing. The new ones:

- `parser::tests::generics`: what is parsed, and what is refused.
- `checker::tests::generics_are_checked`: programs that must pass,
  with every type written out so that a wrong answer would be caught.
- `the_types_of_a_generic_come_from_the_call` and
  `a_written_number_in_a_generic_call_takes_the_size_that_is_wanted`:
  the three places a type is worked out from.
- `a_generic_is_checked_once_for_each_set_of_types`: the note under an
  error, a generic nothing uses, and a class that would need copies
  without end.
- `what_a_generic_cannot_do_yet`: the exact words of each refusal.
- `interpreter::tests::generics` and `the_generics_example`.
- `emit_c::tests::generics_match`: one long program with generics over
  ints, floats, text, a `u8`, an enum, lists and objects, with errors,
  defaults and functions as arguments. Then an owned generic class,
  built with no collector, which must free everything.
- `emit_c::tests::a_generic_is_compiled_once_for_each_set_of_types`
  and `emit_llvm::tests::the_ir_for_a_generic`: three calls make two
  functions, each `Cell` is its own struct, and a generic nothing
  calls leaves nothing behind.

## Things to try

1. In `generics.zeno`, change `ring(4, u8(0))` to `ring(4, 0)` and run
   it. Two lines are refused. What do the messages say `T` is now?
   Then write `let bytes: Ring[u8] = ring(4, 0)`. Why does that work
   when the first change did not?
2. Add `log.push(3.3 V)` at the end. What does the message say `item`
   should be? `push` was written once, with `item: T`. Where did the
   `str` come from?
3. Add `print(largest([bytes, bytes]))`, a list of two rings. The
   message is about `>`. Which line does it point at, and what does
   the note under it tell you?
4. Build the example and count the `struct z_Ring_` lines in
   `generics.c`. Then add `let flags = ring(2, false)` and count
   again. How many new functions came with it? Search for
   `z_Ring_bool_`.
5. Write `function last[T](items: list[T]) -> T`. Then
   `function swap[A, B](p: Pair[A, B]) -> Pair[B, A]` for a class
   `Pair[A, B]` of your own. Print what `swap` returns.
6. Write `class Queue[T]` with `put` and `take`, where `take` returns
   `T or none`. What must the caller do with what `take` returns
   before using it? (Decision 3.12.)
7. Find `bind` in `checker.rs`. There is no arm for a function type
   laid over a function type that takes a different number of
   arguments. What happens then? Write a test program and see which
   message you get, and from where.
8. In `copy_function`, change `template.fresh()` to
   `(**template).clone()` and run the tests. One fails: `doubled`, at
   the end of the long program in `generics_match`, has a nameless
   function in its body. Read what the C compiler says about it. Which
   copy's types did the inner function end up with, and why does
   `fresh` fix it?
9. `ident` in `emit_c.rs` turns `Stack[int]` into `Stack_int`. Find
   two different copy names that it turns into the same C name. (One
   class name with an underscore in it is enough.) Then make the C
   backend refuse that program with a clear message, the way it
   refuses a class called `Stack_int` today.
10. A big one: bounds. Let a type parameter say what it needs, as in
    `function largest[T: Ordered](items: list[T]) -> T`. Start with
    one built-in bound, `Ordered`, meaning "`>` works". The checker
    must refuse `largest([Point(1, 2)])` at the call, before it makes
    the copy. Where in `infer` does that check go? What should the
    message say?
