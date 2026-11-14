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
  program: checked arithmetic, strings, `print`.

## One rule above all: build must agree with run

A program must print the same thing either way. The tests at the bottom of
`emit_c.rs` enforce it: each one runs a program through the interpreter,
compiles the same program, runs the result, and compares the output byte for
byte, including the error message when the program is meant to fail. If you
change one side, the tests tell you to change the other.

## What it compiles so far

Numbers, bools, strings, quantities, variables, `if` / `while` / `for ... in
range(...)`, functions, f-strings, `print`, and the `math` module.

Classes, lists, enums, `match`, `try` and functions used as values are not
compiled yet. `zeno build` says so and points at the line:

```
error: `zeno build` cannot compile lists yet. `zeno run` can still run this program
```

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

## Names

Every Zeno name gets `z_` in front: `count` becomes `z_count`. That way no
Zeno name can collide with a C keyword (`double`, `int`) or a C library
function (`printf`, `abs`). A name declared a second time, in another block
or function, becomes `z2_count`. Temporaries the compiler invents are `t1`,
`t2`, and everything in the runtime starts with `zeno_`.

## Top-level code

C has no code outside functions. The statements at the top of a Zeno file
become the body of C's `main`. Variables declared there become C globals, so
that functions can see them, and are given their values inside `main`, in
the order the file says.

## Memory

A string built while the program runs (`a + b`, an f-string) needs memory,
which the runtime takes from `malloc` and never gives back. For a program
that runs and exits, that is harmless. For firmware that runs for years it
is not acceptable, and fixing it is the memory management stage. Programs
that only use numbers, bools and string literals do not allocate at all.

## What differs from `zeno run`, on purpose

- **Very deep recursion.** The interpreter stops at 1000 calls with a clear
  error. The compiled program uses the real stack, so it goes much deeper,
  and if it runs out, the operating system stops it.
- **The error report** names the file, line and column but does not quote
  the source line, because the compiled program does not carry the source.

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
