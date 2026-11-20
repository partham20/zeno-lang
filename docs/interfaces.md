# Interfaces

A function that adds up areas should work for circles, for squares,
and for the shape somebody writes next year. An `interface` says
what all of them can do, and each class says that it is one. This
page says what the two words do, and walks through how each stage of
the compiler handles them, which turns out to be very little.

```sh
cargo run -- run examples/ports.zeno
cargo run -- build examples/ports.zeno && ./ports
cargo run -- build --no-collector --memory examples/ports.zeno && ./ports
```

The decisions are 10.4 to 10.10 in [`decisions.md`](decisions.md),
and 58.20 to 58.23 for defaults, functions that take out, and
`owned interface`.

## What it looks like

```zeno
interface Shape {
    function area(self) -> float
    function scale(self, by: float)
}

class Circle is Shape {
    radius: float

    function area(self) -> float {
        return 3.0 * self.radius * self.radius
    }

    function scale(self, by: float) {
        self.radius = self.radius * by
    }
}

class Square is Shape {
    side: float

    function area(self) -> float {
        return self.side * self.side
    }

    function scale(self, by: float) {
        self.side = self.side * by
    }
}

function total(shapes: list[Shape]) -> float {
    var sum = 0.0
    for shape in shapes {
        sum += shape.area()
    }
    return sum
}

let shapes: list[Shape] = [Circle(1.0), Square(2.0)]
print(total(shapes))      # 7.0
print(shapes)             # [Circle(radius=1.0), Square(side=2.0)]
```

`total` was written once. `shape.area()` runs the `area` of whatever
class the object is, and that is decided when the program runs.

[`examples/ports.zeno`](../examples/ports.zeno) is the same idea on a
board: a UART and a log in memory are both a `Port`, a board has a
main port and perhaps a spare, and `report` sends through any of
them.

## The rules, all of them

1. **An interface lists functions.** No fields and no bodies. Each
   function has `self` first and a type on every other parameter
   (10.5).
2. **A class says which interfaces it is**, after `is` on its first
   line: `class Box is Named, Sized`. It is not one unless it says
   so (10.4).
3. **The class has every function of the interface, written the
   same way**: the same parameter names, the same types, the same
   result. It may have more of its own (10.6).
4. **A class may raise less than the interface says, never more.**
5. **An object of the class goes where the interface is wanted**: a
   variable, a parameter, a result, a field, and each of those with
   `or none`.
6. **A list of the class does not** go where a list of the interface
   is wanted. A list that is written out, `[Circle(1.0),
   Square(2.0)]`, is a list of the interface wherever the place says
   so (10.7).
7. **Through an interface, only its functions are there.** No
   fields, no functions the class has besides, and no way back to
   the class (10.8).
8. **A class has no parent class.** What two classes share is an
   interface that both are.
9. **A parameter of an interface's function may have a default**,
   if it is a plain value: a number, text, `true`, `false`, `none`,
   a quantity such as `5 ms`, or a case of an enum that carries
   nothing. Each class writes the same value. Where the interface
   writes none, a class may have its own, and a call through the
   interface gives every argument (58.20).
10. **A class's function may take out what its object owns** (5.19k)
    and still stand for one of the interface. Then a call through the
    interface is for the owner only, as a call on the object is
    (58.21).
11. **`owned interface Shape`**: each object that is one has one
    owner, with a collector too. Only an `owned class` can be one,
    and an `owned class` can only be one (58.22).

An object seen through an interface prints as the class it is. Two
of them are equal when they are the same class and that class says
they are equal.

## The messages

Most of the feature is the checker saying no, so here is what it
says. Each one is at the class, or at the line that tried.

```
`Dot` says it is a `Shape`, and a `Shape` has
`function scale(self, by: float)`. Give `Dot` that function

`scale` of `Dot` is `function scale(self, factor: float)`, and `Shape`
asks for `function scale(self, by: float)`. Write the two the same

`send` of `Uart` can raise ValueError, and `send` of `Port` does not
say so. Whoever calls it through a `Port` would not know. Add
`raises ValueError` to the function in `Port`, or catch the error in
`Uart`

`shapes` should be list[Shape], but this is list[Circle]. Whoever
holds it as a list of Shape could append another class to it. Say
what the list holds where it is made, as in
`let items: list[Shape] = [...]`

`radius` is not a function of the interface `Shape`, which has `area`
and `scale`. What else this object has depends on its class, and
that is only known when the program runs. Give `Shape` a function
for it

`A` is a class, not an interface, so `B` cannot be one. A class has
no parent class in Zeno. Declare what the two share as an
`interface`, and let both be it

`spin` of `Fan` gives `speed` the default `2000`, on line 11, and
`Motor` gives it `1000`, on line 7. Write the same default in both,
so that a call through a `Motor` and a call on a `Fan` mean the same

only the owner can call `take_last`, which takes out or replaces
something the object owns (`Stack` is a `Pile`, and its `take_last`
does): somebody else may have been lent it. Here `pile` only looks
at it

`Dot` is an owned class, and `Shape` is not an `owned interface`: an
object with one owner cannot be where a shared one is wanted. Write
`owned interface Shape`, or take `owned` off `Dot`
```

## How it works

The idea that keeps this small: **to every stage after the parser,
an interface is a class.** It has no fields, and its functions have
no bodies. Everything that already knew what to do with a class
(names in several files, `private`, the type `Shape or none`, a list
of them, printing, `==`, freeing) goes on knowing.

### The parser

One new reserved word, `interface`. `is` was already one. The tree
gets two fields on a class it already had:

```rust
pub struct Class {
    pub name: String,
    ...
    /// `class Circle is Shape, Named`: the interfaces it is.
    pub is: Vec<String>,
    /// `interface Shape { ... }`: a list of functions with no bodies.
    pub interface: bool,
}
```

`Parser::interface` reads the body. It is strict on purpose, since a
caller will have nothing but this to go by: a field, a body, a
missing `self`, a parameter with no type are each refused with the
reason. `zeno parse` shows the result:

```
interface Port
    function send(self, text: str) -> int raises Full
    function name(self) -> str
class Uart is Port
    field speed: int
    ...
```

The loader ([`modules.md`](modules.md)) renames the names after `is`
as it renames any name, so `class Uart is ports.Port` in one file is
the `Port` of `ports.zeno`, and `private interface` works with no
code written for it.

### The checker

Three things.

**The functions of an interface.** A function is normally checked by
walking its body, and its result type comes out of that. Here there
is no body, so `signature` takes the first line at its word:

```rust
if of_interface {
    let signature = Signature {
        params,
        returns: declared_return.unwrap_or(Ty::None),
        raises: decl.raises.clone(),
        takes_out: false,
    };
    ...
}
```

From then on `shape.area()` is checked as a call of any method: it
gives a float, and in strict mode a call of `port.send` can raise
what the interface says.

**The promise.** When the body of a class is checked, `stands_for`
goes through each interface it names and each function in it, and
compares first lines:

```rust
let agree = got.params.len() == want.params.len()
    && got.params.iter().zip(&want.params).skip(1)
        .all(|(got, want)| got.name == want.name && same(&got.ty, &want.ty))
    && same(&got.returns, &want.returns);
```

`skip(1)` is `self`, which is a `Circle` on one side and a `Shape`
on the other, as it should be. After that it checks that the class
raises nothing the interface does not list.

**What fits where.** `fits(from, to)` is the one function that
answers "can this value go there". It gained one line and one guard:

```rust
(Ty::Instance(from), Ty::Instance(to)) => from.is.borrow().iter().any(|interface| Rc::ptr_eq(interface, to)),
...
(Ty::List(from), Ty::List(to)) => !sized_into_int(from, to) && !seen_as_interface(from, to) && fits(from, to),
```

The first lets a `Circle` in where a `Shape` is wanted. The second
keeps a `list[Circle]` out of a `list[Shape]`, beside the guard that
already kept a `list[u8]` out of a `list[int]`, for the same reason.

**A list that is written out.** `[Circle(1.0), Square(2.0)]` on its
own is a list of two different things, which is an error. Where the
place says `list[Shape]`, the checker leaves a note for the list
before it looks at it (`expecting`), and the list reads the note:
each item is checked against `Shape`, and the list is a list of
`Shape`. The compilers are told too (`Types::items`), because they
have to build it as one.

This is also where an older fault was found. A `[1, 2]` given to a
parameter that takes a `list[int or none]` was accepted, built as a
list of ints, and read as a list of `int or none`, and the compiled
program crashed. The same note now tells that list what it is.

### The interpreter and the VM

Nothing changed. Both look a method up by its name on the object in
front of them, and always have. An interface never runs: it has no
bodies.

### The C build

Three small things: a number, a cast and a `switch`.

**A number.** Each class that is some interface gets a number, and
carries it as its first field. The interface's own struct is that
field and nothing else:

```c
struct z_Shape {
    int32_t zeno_class;
};
struct z_Circle {
    int32_t zeno_class;
    double z_radius;
};

static z_Circle *zeno_new_Circle(double z_radius) {
    z_Circle *self = (z_Circle *)zeno_alloc(sizeof(z_Circle));
    self->zeno_class = 1;
    self->z_radius = z_radius;
    return self;
}
```

A class that is no interface is exactly what it was.

**A cast.** Because every class that is a `Shape` begins with the
same field, a pointer to a `Circle` can be read as a pointer to a
`Shape`. Where a `Circle` goes into a place that wants a `Shape`,
`coerce` writes `((z_Shape *)circle)` and that is all. Nothing is
copied or wrapped, so a `Shape` in a list takes one slot like any
object.

**A `switch`.** For each function of the interface, `pass_on` writes
one C function that reads the number and calls the function of that
class:

```c
static double z_Shape_area(z_Shape *a0) {
    switch (a0->zeno_class) {
    case 1: return z_Circle_area((z_Circle *)a0);
    default: return z_Square_area((z_Square *)a0);
    }
}
```

The last class is the `default`, so that C sees every path return.
`shape.area()` compiles to `z_Shape_area(shape)`, as a method call
of any class would. The same `switch` is written three more times
for the things every class already had: showing an object as text,
comparing two, and, with no collector, freeing one:

```c
static void zeno_drop_Port(z_Port *self) {
    if (self == NULL) return;
    switch (self->zeno_class) {
    case 1: zeno_drop_Uart((z_Uart *)self); return;
    default: zeno_drop_Log((z_Log *)self); return;
    }
}
```

That is why `examples/ports.zeno` ends with `0 blocks left at the
end` in a build with no collector: a list of ports frees each port
as the class it is.

In a firmware build a function that can raise gives back `true` or
`false` and hands its answer out through a pointer
([`functions-and-errors.md`](functions-and-errors.md)). The
interface's `send` says `raises Full`, the UART's can, the log's
cannot, and the `switch` makes the two look alike to the caller:

```c
static bool z_Port_send(z_Port *a0, const char *a1, int64_t *zeno_result) {
    switch (a0->zeno_class) {
    case 1: return z_Uart_send((z_Uart *)a0, a1, zeno_result);
    default: *zeno_result = z_Log_send((z_Log *)a0, a1); return true;
    }
}
```

There is no table of pointers to functions anywhere, and decision
10.9 says why: the whole program is compiled at once, so every class
that is a `Port` is known, and a `switch` over a known set is
something a C compiler can see through.

### The LLVM build

The same three things in LLVM's words. The number is an `i32` at the
front of the struct type, which moves every other field up by one
(`ClassEntry::at`). A pointer is a pointer in LLVM, so the cast is
nothing at all. And the `switch` is LLVM's own:

```llvm
%z_Shape = type { i32 }
%z_Circle = type { i32, double }

define internal double @z_Shape.area(ptr %a0) {
entry:
  %t1 = load i32, ptr %a0
  switch i32 %t1, label %is1.2 [ i32 1, label %is1.1 ]

is1.1:
  %t2 = call double @z_Circle.area(ptr %a0)
  ret double %t2

is1.2:
  %t3 = call double @z_Square.area(ptr %a0)
  ret double %t3
}
```

The tests build every interface program both ways, in all three
kinds of build, and compare each with the interpreter. With no
collector the two builds must also report the same use of memory,
line for line.

## What is left out

Each is refused with a message. Decisions 10.10 and 58.23 have the
reasons.

- A default body for a function of an interface, and a default for a
  parameter that is not a plain value (58.20).
- Type parameters: `interface Stack[T]`.
- An interface that is another interface.
- A way back from an interface to the class (10.8).

A default for a parameter, a function that takes out what its object
owns, and an `owned class` as an interface were on this list, and
are in the language since 0.8.0 (58.20 to 58.22). A dict whose values
are of an interface is in it since 10.11.

## Things to try

1. Run `examples/ports.zeno`. Then write a third port, `class Count
   is Port`, that only counts the characters it is sent. Add it to
   the list. Nothing else in the file needs to change.
2. In `Log`, rename the parameter of `send` from `text` to `line`.
   Read the message. Why do the names have to agree?
   (`port.send(text="x")`.)
3. Take `raises Full` off `send` in the interface. Which class is
   now wrong, and what are the two ways the message offers to put it
   right?
4. Add `function sent(self) -> int` to `Port`. The checker now names
   every class that is missing it. Give each one the function.
5. Write `let uarts = [Uart(9600), Uart(1200)]` and pass `uarts` to
   a function that takes a `list[Port]`. Read the message, then make
   the list a `list[Port]` where it is made.
6. `cargo run -- build examples/ports.zeno`, and find `z_Port_name`
   in `ports.c`. Add your third port, build again, and look at the
   `switch`.
7. Build it with `--firmware` and find `z_Port_send`. Which line
   turns a function that cannot fail into one that says it worked?
8. Build it with `--llvm` and find `@z_Port.name` in `ports.ll`.
   Compare it with the C.
9. In `src/checker.rs`, find `stands_for`. Write two interfaces
   that both have a function `name`, one returning `str` and one
   returning `int`, and a class that is both. What does the checker
   say, and is it right?
10. Write an interface `Sensor` with `function read(self) -> float
    raises Fault`, two sensors, and a function that reads a list of
    sensors and returns the highest reading, skipping the ones that
    raise. Build it with `--firmware`.
