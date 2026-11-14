# How compiling to C works

`zeno run` reads your program and does what it says, one node of the syntax
tree at a time. `zeno build` reads the same tree and writes a C program that
does what it says. Then the machine's C compiler turns that into a real
program: no Zeno needed to run it, and many times faster.

```
zeno build examples/primes.zeno     # writes primes.c, then builds ./primes
./primes
```

```
source -> lexer -> parser -> checker -> emit_c -> primes.c -> cc -> ./primes
```

Open `primes.c` after building. Reading the C next to the Zeno is the best
way to follow this document.

## The files

- `src/emit_c.rs` walks the tree and writes C. It is the compiler proper.
- `src/runtime.c` is a small C library that is pasted in front of every
  program: checked arithmetic, strings, lists, errors, `print`.

## One rule above all: build must agree with run

A program must print the same thing either way. The tests at the bottom of
`emit_c.rs` enforce it: each one runs a program through the interpreter,
compiles the same program, runs the result, and compares the output byte for
byte, including the error message when the program is meant to fail. If you
change one side, the tests tell you to change the other.

Since the bytecode VM ([`vm.md`](vm.md)) the same tests run each program a
third way, on the VM, which must agree with the interpreter as well.

## What it compiles so far

Nearly all of the language: numbers, bools, strings and their methods,
quantities, variables, `if` / `while` / `for`, functions, f-strings,
`print`, the `math`, `time` and `board` modules, classes with methods, lists (literals,
indexing, `append`, `pop`, `+`, `in`, comprehensions, `map`, `filter`,
`reduce`), enums and `match`, values that may be `none`, errors (`try`,
`except`, `raise`), functions used as values, and owned values
([`docs/ownership.md`](ownership.md)).

Every program in `examples/` builds. `blink.zeno` and `signal.zeno` run on
a simulated board ([`docs/board.md`](board.md)).

A few things are left. `zeno build` names each one and points at the line:

```
error: `zeno build` cannot compile a function inside a function or block yet. `zeno run` can still run this program
```

- A **named** function, class or enum declared inside a function. (Write
  `let helper = function(n) { ... }` instead: that one compiles.)
- A default value that is not a plain literal or `[]`:
  `function f(at = now())`.
- A method taken off an object without calling it: `let f = point.length`.

## Where the types come from

C wants a type on everything. Zeno lets you write none:

```
function average_gap(limit) {
    return limit / count_primes(limit)
}
```
```c
static double z_average_gap(int64_t z3_limit) {
    return zeno_div((double)z3_limit, (double)z_count_primes(z3_limit), 34, 12);
}
```

The checker has already worked out that `limit` is an int (from the call) and
that the function returns a float (from the `/`). It passes every function's
signature to the compiler in `Types`. From those, the compiler works out the
rest as it goes: a literal knows its own type, a variable has the type of its
first value, `a + b` has the type its two sides give.

Each compiled expression is a `Val`: a piece of C text (`code`) and its
`kind`. `arithmetic` looks at the two kinds to choose what to write. Two ints
become `zeno_add(a, b, ...)`; an int and a float become `((double)a + b)`.

This is why `zeno build` always checks in strict mode: a value whose type is
not known cannot be given a C type.

## The runtime: making C keep Zeno's promises

C is looser than Zeno. Three examples:

| | Zeno says | C says |
|---|---|---|
| `a + b` overflows | OverflowError | undefined; anything may happen |
| `a // 0` | ZeroDivisionError | undefined |
| `-7 // 2` | `-4`, rounds down | `-3`, rounds toward zero |

So whole-number arithmetic goes through functions in `runtime.c`. `zeno_add`
asks the processor whether the addition overflowed, and stops the program
with Zeno's error if it did. The last two arguments of each call are the line
and column in the Zeno source, so the message can say where:

```
error: uncaught ZeroDivisionError: division by zero
 --> primes.zeno:15:12
```

Floats need less: C's float arithmetic already matches, apart from dividing
by zero. But printing them does not match. `zeno_str_float` is the longest
function in the runtime, because "the shortest digits that read back as the
same number, and always with a `.0`" is not something C's `printf` can do in
one step.

## Order: the part C leaves open

```
print(next() - next())
```

Zeno promises left to right. C promises nothing about which operand of `-`,
or which argument of a call, is worked out first. If `next()` prints or
changes a variable, a C compiler may give a different answer from `zeno run`,
and a different answer on a different machine.

So every `Val` carries three flags: does it *read* a top-level variable, can
it *fail*, does it *call* a function. `sequence` compiles a list of
expressions that Zeno orders left to right, and whenever a later one could
disturb an earlier one, it moves the earlier one into a temporary:

```c
int64_t t1 = z_next();
zeno_print(1, zeno_str_int(zeno_sub(t1, z_next(), 9, 7)));
```

When nothing can be disturbed, as in `a + b * 2`, no temporary is made and
the C reads like the Zeno.

`and`, `or` and chained comparisons (`a < f() < g()`) have a second concern:
the right side must not run at all unless it is needed. C's `&&` does that
when the right side is a single expression. When the right side needed
statements of its own, `logic` wraps them in an `if`.

## Classes

```
class Point {
    x: float
    y: float
    function distance_to(self, other: Point) -> float { ... }
}
```
```c
struct z_Point { double z_x; double z_y; };
static z_Point *zeno_new_Point(double z_x, double z_y);       /* Point(3.0, 4.0) */
static const char *zeno_show_Point(z_Point *self);            /* for print */
static bool zeno_same_Point(z_Point *a, z_Point *b);          /* for == */
static double z_Point_distance_to(z_Point *z_self, z_Point *z_other);
```

A class is a C struct. The compiler writes three functions for it that you
never wrote: one makes an object, one shows it as text, one compares two.
A method is an ordinary C function whose first argument is the object.

An object is always handled through a pointer. That is what makes
`let b = a` a second name for the same object, as Zeno says it is: copying
a pointer does not copy what it points to.

A field with a size, such as `level: u8`, is a `uint8_t` in the struct and
an `int64_t` everywhere else. [`sized-ints.md`](sized-ints.md) follows one
through.

## Enums and match

```
enum Shape {
    Circle(radius: float)
    Rect(width: float, height: float)
    Nothing
}
```
```c
struct z_Shape {
    int tag;                                   /* which case: 0, 1 or 2 */
    union {
        struct { double z_radius; } c0;
        struct { double z_width; double z_height; } c1;
    } as;                                      /* room for the biggest case */
};
```

A value of an enum is one case at a time, so the cases can share the same
memory: that is what a C `union` is. The `tag` says which case is in there
now. This pair, a tag and a union, is how every language with enums like
these stores them.

`match` then becomes a chain of `if`s on the tag. A pattern such as
`Rect(w, h)` gives the compiler two things: a test (`tag == 1`) and new
variables to declare (`w` and `h`, read out of the union). `pattern`
collects both, and works the same way for a pattern inside a pattern.

Because the checker has already made sure every case is covered, the last
`case` needs no test and becomes a plain `else`.

## Lists

C has no list that grows, so the runtime provides one: a length, how much
room there is, and a pointer to the items. `append` moves the items to a
place twice the size when the room runs out.

A list must hold ints, or floats, or strings, or objects. C would want a
separate list type for each. Instead every item sits in a `zeno_slot`, a
union big enough for any single Zeno value:

```c
typedef union { int64_t i; double f; bool b; const char *s; void *p; } zeno_slot;
```

The compiler knows what each list holds, so it knows which member to read:
`xs[0]` on a `list[int]` becomes `(*zeno_list_at(z_xs, 0, 3, 7)).i`.
`zeno_list_at` is where the position is checked, so `xs[99]` is an
IndexError, not a read of some other memory.

Showing a list and comparing two lists need to know how to show or compare
one item. The runtime cannot know that, so the compiler writes a small
function per kind of item (`zeno_item_repr_float`) and passes it in.

A `for` loop over a list walks a copy of it. That matches `zeno run`, where
the loop sees the list as it was when the loop began, even if the body adds
to it. Only the loop ever points at the copy, so it is freed as soon as the
loop ends.

## Values that may be none

`str or none` has to be stored somehow. There are two cases.

A str, a list, an object, an enum value, an error and a function are all
pointers in C already, and a pointer has a spare value: `NULL`. So for
those, `none` is `NULL`, and `T or none` costs nothing more than `T`.

A number or a bool has no spare value: every pattern of bits is some
number. So `int or none` is a number with a flag beside it:

```c
typedef struct { bool present; int64_t value; } zeno_opt_int;
```

```
function half(n: int) -> int or none {
    if n % 2 == 0 { return n // 2 }
    return none
}
let h = half(8)
if h is not none { print(h + 1) }
```
```c
static zeno_opt_int z_half(int64_t z_n) {
    if (zeno_mod(z_n, 2, 2, 8) == 0) {
        return (zeno_opt_int){true, zeno_floordiv(z_n, 2, 2, 28)};
    }
    return (zeno_opt_int){false, 0};
}
...
if (z_h.present) {
    zeno_print(1, zeno_str_int(zeno_add(z_h.value, 1, 6, 26)));
}
```

Three things to see there:

- `return n // 2` puts the flag on, because the function returns
  `int or none`. Going from `T` into `T or none`, and writing `none`
  itself, is the one place the two differ, and `coerce` in `emit_c.rs`
  does it wherever a value arrives somewhere: a `return`, an argument, an
  assignment, an item of a list.
- `h is not none` reads the flag.
- Inside the `if`, `h` is read as `z_h.value`, the plain number. The
  compiler cannot see for itself that the `if` has ruled `none` out. The
  checker can (that is narrowing, in `docs/checker.md`), and it passes on
  a note of every place where a variable is read after such a check.

One more case. A top-level `var`, or a `var` shared with an inner function,
can be changed by a function that is called between the check and the use:

```
var limit: int or none = 5
function clear() { limit = none }
if limit is not none {
    clear()
    print(limit + 1)        # limit is none again by now
}
```

For those variables only, the value is looked at once more where it is
used (`zeno_still_int`), and the program stops with an error if it has
gone. A local variable that nothing else can reach needs no second look.

A list gives every item one slot, which has no room for a flag. In a list,
a number that may be `none` is kept as a pointer to a copy of the number.

## Errors: try, except and raise

```
try {
    let port = parse_port(text)
    print(port)
} except ValueError as err {
    print(f"error: {err}")
}
```
```c
zeno_try t1;                        // a place to come back to
t1.outer = zeno_trying;
zeno_trying = &t1;                  // from here on, errors come to t1
if (setjmp(t1.place) == 0) {
    z_port = z_parse_port(z_text);
    zeno_print(1, zeno_str_int(z_port));
    zeno_trying = t1.outer;         // finished with no error
} else if (zeno_caught("ValueError")) {
    zeno_error *z_err = zeno_raised;
    zeno_print(1, zeno_join(2, "error: ", z_err->message));
} else {
    zeno_raise_again();             // not ours: pass it outward
}
```

`setjmp` is a C library function that remembers "here": where the program
is, and what is in the processor's registers. It returns 0. Later,
`longjmp` puts all of that back, and the program finds itself returning
from that same `setjmp` a second time, now with 1.

So a `try` remembers a place, and tells the runtime about it by putting it
at the front of a chain (`zeno_trying`). `raise`, or any error the runtime
finds itself (dividing by zero, a bad index), goes through one function,
`zeno_raise`. If the chain is empty, nobody is trying: it prints the error
and stops the program, exactly as before. Otherwise it takes the innermost
place off the chain and jumps to it, out of however many function calls
are in the way.

That means code outside a `try` pays nothing, and a `try` in which nothing
goes wrong pays for one `setjmp`.

Details that the compiler looks after:

- **Leaving early.** A `return`, `break` or `continue` inside a `try` body
  jumps past the line that takes the place off the chain. So the compiler
  writes that line before each of them.
- **Variables changed inside a `try`.** C has a rule here: after the jump
  back, a local variable that was changed since the `setjmp` may hold its
  old value or its new one, unless it is declared `volatile`. (The C
  compiler may have kept the new value in a register, and the jump puts
  the registers back.) The compiler finds the variables that a `try` body
  assigns to, and declares those `volatile`. See `changed_in_try`.
- **Raising an object.** `raise Timeout(30 ms)` wraps the object in an
  error whose kind is the class's name. `except Timeout as t` gets the
  object back out. `except Error as e` gets the error it travelled in.

### In a firmware build

`zeno build --firmware` compiles all of this differently, with no
`setjmp`: a function that can raise returns `true` or `false`, and each
caller tests what came back. [`docs/firmware.md`](firmware.md) is the
walkthrough.

## Functions as values

A function held in a variable is a pointer to a small struct:

```c
typedef struct zeno_closure { zeno_code code; const char *name; } zeno_closure;
```

`code` is the C function to run. Calling `f(3)` becomes "call `f`'s code,
passing `f` itself first":

```c
((int64_t (*)(zeno_closure *, int64_t))z_f->code)(z_f, 3)
```

The long bracket in front turns the stored pointer back into the type it
really is, which the compiler knows from the type of `f`.

Why pass the function to itself? Because of this:

```
function make_scaler(k: int) {
    return function(n: int) { return n * k }
}
```

The inner function uses `k`, which belongs to a call of `make_scaler` that
has finished by the time the inner function runs. So when the inner
function is made, the variables it uses are packed up with it:

```c
struct zeno_env_1 { zeno_closure base; int64_t z_k; };

static int64_t zeno_fn_1(zeno_closure *zeno_self, int64_t z_n) {
    struct zeno_env_1 *env = (struct zeno_env_1 *)zeno_self;
    return zeno_mul(z_n, env->z_k, 2, 38);
}

static zeno_closure *z_make_scaler(int64_t z_k) {
    struct zeno_env_1 *t1 = (struct zeno_env_1 *)zeno_alloc(sizeof(struct zeno_env_1));
    t1->base.code = (zeno_code)zeno_fn_1;
    t1->base.name = "";
    t1->z_k = z_k;
    return (&t1->base);
}
```

A function together with the variables it took with it is called a
**closure**. The header comes first in the struct, so a pointer to the
whole thing is also a valid pointer to the header, and every function
value can be handled the same way whatever it carries.

How does the compiler know which variables to pack? While it compiles the
inner function, every variable of the outer one is on offer as
`env->name`, and each notes whether it was looked up. The ones that were
are the ones to pack.

**Sharing a variable.** `k` above is copied, which is fine because it never
changes. This is different:

```
function counter() {
    var count = 0
    return function() {
        count += 1
        return count
    }
}
```

`count` changes, and the change has to last. So a `var` that an inner
function uses is not kept in a C variable at all. It is kept in a box on
the heap, and the outer function and the inner one both hold a pointer to
the box:

```c
int64_t *z_count = (int64_t *)zeno_alloc(sizeof(int64_t));
*z_count = 0;
...
t2->z_count = z_count;              // the inner function gets the box
```

**Ordinary functions as values.** `xs.map(double)` passes a top-level
function. It does not take the hidden first argument, so the compiler
writes a small wrapper that does (`zeno_wrap_double`) and one fixed
`zeno_closure` that points at it.

**`map`, `filter` and `reduce`** are loops written out where they are
used, each calling the function on every item.

## Names

Every Zeno name gets `z_` in front: `count` becomes `z_count`. That way no
Zeno name can collide with a C keyword (`double`, `int`) or a C library
function (`printf`, `abs`). A name declared a second time, in another block
or function, becomes `z2_count`. Temporaries the compiler invents are `t1`,
`t2`, and everything in the runtime starts with `zeno_`.

## Top-level code

C has no code outside functions. The statements at the top of a Zeno file
become the body of one C function, `zeno_program`, which `main` calls.
(`main` itself only notes where the stack begins, for the garbage
collector.) Variables declared at the top become C globals, so that
functions can see them, and are given their values inside `zeno_program`,
in the order the file says.

## Memory

A string built while the program runs (`a + b`, an f-string) needs memory,
and so does every object, list and enum value that carries data, every
function made inside another one, and the box of a variable they share.
The runtime takes it from `malloc`, and a garbage collector in the runtime
frees it once the program can no longer reach it.
[`docs/memory.md`](memory.md) is the walkthrough.

A value marked `owned` does not wait for the collector. The compiler writes
a call that frees it at the end of its owner's block, and on every other way
out of that block: `return`, `break`, `continue`, and an error.
[`docs/ownership.md`](ownership.md) is the walkthrough for that. When
every list and object is owned, `zeno build --no-collector` leaves the
collector out of the C file: [`docs/no-collector.md`](no-collector.md).

Programs that only use numbers, bools, string literals and enum cases
without data do not allocate at all, and the collector never runs.

## What differs from `zeno run`, on purpose

- **Very deep recursion.** The interpreter stops at 1000 calls with a clear
  error. The compiled program uses the real stack, so it goes much deeper,
  and if it runs out, the operating system stops it.
- **The error report** names the file, line and column but does not quote
  the source line, because the compiled program does not carry the source.
- **A variable that became `none` again after it was checked** (the
  `limit` example above). Both stop with an error at that line, but the
  wording differs: the interpreter complains about the operation that was
  tried, the compiled program about the variable.
- **In a firmware build, an error the runtime finds stops the program**
  even inside a `try`. An `except` that could only be waiting for one is
  refused when the program is built. See [`firmware.md`](firmware.md).
- **`sum` of an empty list of floats** prints `0.0` compiled and `0`
  interpreted. The compiled answer is the right one: the interpreter does
  not know what an empty list was meant to hold.

## Things to try

1. Build `examples/primes.zeno` and find, in `primes.c`, the line each Zeno
   line turned into.
2. Write a program where the order of two calls matters, build it, and find
   the temporaries. Then make the calls not matter and watch them vanish.
3. Add `math.tan`. You need one line in `emit_c.rs` (and it should already
   work in the interpreter and checker; if not, add it there first).
4. Add `time.sleep(500 ms)`. What should it compile to on a PC? On a
   microcontroller?
5. Compile `examples/primes.zeno` with `-O0` and `-O2` and time both against
   `zeno run`. Where does the time go in each?
6. Build `examples/sensor.zeno` and find the `setjmp` in `sensor.c`. Which
   variable in `read_all` became `volatile`, and why that one?
7. In the same file, find the struct that `in_range` packs `low` and `high`
   into. Then find the box that `counter` keeps `count` in.
8. Make a named function inside a function compile. Start with the easy
   half: one that is only used below the line it is written on.
