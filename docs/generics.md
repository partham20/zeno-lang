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

## Generic enums

An enum can have type parameters too (decision 58.1):

```zeno
enum Maybe[T] {
    Nothing
    Just(value: T)
}

enum Result[T, E] {
    Ok(value: T)
    Failed(error: E)
}

function first_even(numbers: list[int]) -> Maybe[int] {
    for n in numbers {
        if n % 2 == 0 { return Just(n) }
    }
    return Nothing
}

match first_even([1, 3, 4]) {
    case Just(n) { print(f"found {n}") }
    case Nothing { print("none of them") }
}
```

The rules are those of a generic class, and nothing new. `Maybe[int]`
is a type, a copy of the enum with `T` standing for `int`. Its cases
are written by their names alone, as the cases of any enum are, and
the types are worked out where a case is used:

- **A case that carries values says `T` by what it carries.**
  `Just(3)` is a `Maybe[int]`, `Just("a")` a `Maybe[str]`.
- **A case that carries nothing says nothing**, so `Nothing` takes its
  type from the place it goes: the type of the variable, the
  parameter, the field, the list or dict it goes into, what the
  function returns, or the other side of `==`. An argument of a
  generic waits for the others: in `or_else(Nothing, 5)` the `5` says
  that `T` is `int`, and so `Nothing` is a `Maybe[int]`.
- **Where nothing says, it is an error** that shows how to write it:

```
error: `Nothing` is a case of the generic enum `Maybe`, and nothing here says which `Maybe` it is. Put it where its type is written, as in `let x: Maybe[int] = Nothing`
```

`Result[T, E]` has two parameters, and `Ok(3)` only says one of them,
so `let r: Result[int, str] = Ok(3)` is how it is written where the
result has no type of its own yet. A `match` on a `Maybe[int]` must
handle `Just` and `Nothing`, and says which it left out, as for any
enum. An enum may hold itself: `enum Chain[T] { End; Link(item: T,
rest: Chain[T]) }`.

A case name belongs to one enum of a file, generic or not, since it is
written alone. A generic enum cannot give its cases numbers (`Ack =
6`), since a case with a number carries nothing, and a generic enum is
for carrying a `T`.

### How it is built

- **The parser** reads `[T]` after the name of an enum, as after the
  name of a class, into `Enum::type_params`.
- **The checker** keeps a generic enum as a `Generic` whose template is
  the enum, and each case name stands for `Ty::GenericCase`: the
  generic and the place of the case. A call `Just(3)` is worked out as
  a call of a generic function is, with the values that case carries
  as its parameters and `Maybe[T]` as what it returns (`case_outline`,
  then the same `infer` as every generic). The copy, `Maybe[int]`, is
  an ordinary enum made by `copy_enum`, with its `origin` kept, as a
  copy of a class is. A bare `Nothing` looks at the place it goes
  (`expecting` notes it, `generic_bare_case` reads it). Each case
  where it is used is noted in `copy_called` with the copy it means:
  the call for `Just(3)`, the name for `Nothing`. A copy of a generic
  function has a syntax tree of its own, so `return Just(x)` inside
  `wrap[T]` is noted once for each copy of `wrap`. In a `match`, a
  case finds itself in the copy that is matched (`copy_matched`).
- **The interpreter and the VM** run the generic as it is written:
  they never needed the types.
- **The compilers** write one enum for each copy (`z_Maybe_int`, with
  its own cases, `zeno_new_Maybe_int_Just`), and none for the generic.
  A case of a copy is found through what the checker noted, and a
  pattern finds its case in the enum of what is matched (`case_in`).
  An enum that is not generic goes the way it always went, so what is
  written for it did not change, to the byte.
- **The fifteen targets** do not look at types, and run it as it is.

## Bounds

A type parameter may say what it asks of its type, with `is` and an
interface, as a class says what it is (decision 58.3):

```zeno
function largest[T is Ordered](items: list[T]) -> T {
    var best = items[0]
    for item in items {
        if item > best { best = item }
    }
    return best
}

function total[S is Shape](shapes: list[S]) -> float {
    var sum = 0.0
    for s in shapes { sum += s.area() }
    return sum
}

print(largest([3, 9, 2]), largest(["pear", "apple"]), largest([3 V, 12 V]))
print(total([Circle(1.0), Circle(2.0)]))
```

Each use is checked against the bound first, and a type that lacks it
is refused at the call, which names both:

```
error: `Task` is not `Shape`, which `total` asks of `S`. It is one if it says so: `class Task is Shape`, with what `Shape` lists
```

- **`Ordered` comes with Zeno.** A type is `Ordered` when `<` works on
  two of it: every kind of number, a quantity, and text. A class cannot
  be `Ordered` yet; that comes with operators for a class. A program
  that declares its own `Ordered` has its own.
- **A bound of the program's own** takes a class that says it is the
  interface, or the interface itself. `total` takes a `list[Circle]`
  as one, which `shapes: list[Shape]` would not, and a `list[Shape]`
  too.
- **A class and an enum take bounds too**: `class Range[T is
  Ordered]`, `enum Span[T is Ordered]`.
- **One interface for each type parameter.** A comma begins the next
  parameter, so `[T is Ordered, Shape]` would make `Shape` a second
  type parameter, and is refused with a message that says so.
- **The body is still checked for each copy.** A bound is a promise
  checked at the call, not all that the body may do: a generic with no
  bound that uses `>` is still accepted.
- **A generic that nothing uses** is checked with each bounded
  parameter standing for its interface, so a call of a function the
  interface does not list is found (58.5).
- **A bound makes no code.** The checker answers it, and the C of a
  program with bounds is the C of the same program without them.

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

- **A bound is one interface.** `[T is Ordered]` or `[S is Shape]`,
  not two at once, and a class cannot be `Ordered` until a class can
  say what `<` means for its objects.
- **A case of a generic enum cannot be written after the enum's
  name**, as `Maybe.Just(3)`, and cannot be kept as a value: `let f =
  Just` is refused, as `let f = first` is.
- **A method cannot have type parameters of its own.** The class can.
- **A generic cannot be kept as a value**: `let f = first` is refused,
  because `f` would have to be every copy at once. Wrap it:
  `function(xs: list[int]) -> int { return first(xs) }`.
- **The types cannot be written at the call.** There is no
  `first[int](xs)`. If nothing says what `T` is, put the result where
  its type is written.
- **A list of objects of an `owned class` cannot fill a type
  parameter with a collector**, as such a list cannot be inside any
  other type. With no collector `Stack[list[Frame]]` builds, and any
  list or object can fill `T` since 0.8.0
  ([`generic-owned.md`](generic-owned.md)).
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
- `checker::tests::a_generic_enum_takes_its_types_from_where_it_is_used`
  and `what_a_generic_enum_says_when_it_cannot`: every place `Nothing`
  takes its type from, and the words of each refusal.
- `emit_c::tests::a_generic_enum_every_way`:
  `tools/targets/edge-generic-enum.zeno`, with `Maybe`, `Result` and
  `Chain` over numbers, text, lists and objects, copies made inside
  generics, a thousand rounds and every way out of a `match`, built
  every way (with no collector, nothing is left), and one struct for
  each copy.
- `checker::tests::a_type_parameter_may_ask_for_an_interface`,
  `a_type_that_lacks_what_is_asked_is_refused_at_the_call` and
  `what_a_bound_says_when_it_is_written_wrong`: bounds over numbers,
  quantities, text and a program's own interface, and the words of
  each refusal.
- `emit_c::tests::a_bound_every_way`: `tools/targets/edge-bounds.zeno`,
  bounds on functions, a class and an enum, built every way, and the
  same C as without the bounds.

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
11. Write `enum Tree[T] { Leaf; Node(left: Tree[T], value: T, right:
    Tree[T]) }` and a function that counts the values in a
    `Tree[int]`. Build it with `--no-collector --memory`. How many
    blocks does a tree of three values take, and how many are left
    at the end?
12. Write `let x = Nothing` and read the message. Then give `x` a type.
    Now try `print(Nothing)`. Why can `print` not say what `T` is,
    when `let x: Maybe[int] = Nothing` can?
13. Build `tools/targets/edge-generic-enum.zeno` and search the C for
    `struct z_Maybe_`. How many copies of `Maybe` are there, and which
    line of the program asked for each?
