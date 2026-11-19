# The least C: `zeno build --minimal`

`zeno build` writes C that keeps every promise Zeno makes. An index is
checked, a sum is checked for overflow, an error says the line it
happened on, and the whole of Zeno's runtime is written in front of the
program in case it is needed. For a program of one line that is about
4,500 lines of C.

`zeno build --minimal` writes the other kind of C: the least that runs
the program, close to what a person would write by hand.

```sh
zeno build --minimal primes.zeno      # writes primes.c, and builds ./primes from it
```

Nothing about `zeno build` without the option has changed. The C it
writes is byte for byte what it was.

## What it looks like

`examples/primes.zeno` counts the primes below 10,000:

```zeno
function is_prime(n) {
    if n < 2 {
        return false
    }
    var d = 2
    while d * d <= n {
        if n % d == 0 {
            return false
        }
        d += 1
    }
    return true
}
```

`zeno build` writes this for it, after 4,500 lines of runtime:

```c
static bool z_is_prime(int64_t z_n) {
    if (z_n < 2) {
        return false;
    }
    int64_t z_d = 2;
    while (zeno_mul(z_d, z_d, 15, 11) <= z_n) {
        if (zeno_mod(z_n, z_d, 16, 12) == 0) {
            return false;
        }
        z_d = zeno_add(z_d, 1, 19, 9);
    }
    return true;
}
```

and `zeno build --minimal` writes this, in a file of 100 lines:

```c
static bool z_is_prime(int64_t z_n) {
    if (z_n < 2) {
        return false;
    }
    int64_t z_d = 2;
    while ((z_d * z_d) <= z_n) {
        if (zeno_mod(z_n, z_d) == 0) {
            return false;
        }
        z_d += 1;
    }
    return true;
}
```

`zeno_mod` is still there, eight lines above it in the same file. That
one is not a check. `-7 % 3` is 2 in Zeno and -1 in C, and a correct
program may rely on it.

The last lines of the program,

```zeno
print(f"primes below {limit}: {count_primes(limit)}")
print(f"one in every {average_gap(limit):.2f} numbers")
```

are

```c
printf("primes below %lld: %lld\n", (long long)t1, (long long)z_count_primes(z_limit));
printf("one in every %.2f numbers\n", z_average_gap(z_limit));
```

## What is left out

Six things are done to the C. Decisions chapter 30 has the reason for
each.

1. **No check.** The table below.
2. **No place in the source.** No function is told the line and column
   it was called from.
3. **Only what the program uses.** Every function, type, variable,
   `#define` and `#include` of the runtime that the program does not
   reach is left out, and so is a function of the program that nothing
   calls.
4. **`print` is `printf`**, or `puts` for words alone. (Not in a
   build for a board, `--board`: on a chip `printf` is copied into the
   program, some 20 KB of it, so `print` stays with the runtime there.
   Decision 52.4.)
5. **Statements the short way.** `x += 1`, not `x = (x + 1)`, and the
   top of the program is the body of `main`.
6. **`-Os`.** The C compiler is asked for a small program.

## What you give up

A mistake in the program does what it would do in C. The checked build
stops and says what went wrong and where; this one does not.

| | `zeno build` | `zeno build --minimal` |
|---|---|---|
| `xs[i]` with `i` out of range | `IndexError` | reads or writes whatever memory is there |
| `ages["Eve"]` with no such key | `KeyError` | reads an item that is not there |
| overflow of `+ - *`, of `-n`, `abs(n)`, `**`, `sum` | `OverflowError` | no defined result |
| `n // 0`, `n % 0` | `ZeroDivisionError` | what the processor does: on a computer the system stops the program |
| `x / 0.0` | `ZeroDivisionError` | infinity, or NaN |
| `math.sqrt(-1.0)`, `math.log(0.0)`, `math.asin(2.0)` | `ValueError` | NaN or infinity |
| `u8(300)` | `ValueError` | 300 goes on as it is |
| `n << 70`, `n << -1` | `ValueError` | no defined result |
| `xs.pop()` or `min(xs)` of an empty list, a slice from past the end | an error | no defined result |
| `int(x)` of infinity or NaN | `ValueError` | the biggest int, the smallest, or 0 |
| a variable that became `none` again after it was checked | an error | used as if it held a value |
| `free(p)` twice | stops, and says so | no defined result |

"No defined result" means what it means in C: a wrong number, a crash
far from the line that caused it, or nothing you notice until later.
The C compiler is also allowed to build on the mistake not happening.

And an error that nothing catches no longer says where it was raised:

```
error: uncaught ValueError: cannot turn "ten" into an int
```

## What you keep

- **What the program prints.** A correct program prints the same in a
  minimal build as in any other, to the last digit of a float.
- **The errors a correct program can meet.** `int("ten")` still raises
  `ValueError`, a file that is not there still raises `FileError`,
  `raise`, `try` and `except` work as they did, and running out of
  memory still stops the program and says so. These come from outside
  the program, and the program was written to deal with them.
- **What `%`, `//` and `**` mean in Zeno**, for numbers below zero too.
- **How memory is freed.** The collector, `--owned` and `--no-collector`
  each work as they did.

So: write the program and test it with `zeno run` and `zeno build`,
which check everything. Build with `--minimal` when it is correct and
you want the C.

## With the other options

`--minimal` goes with every build that writes C:

```sh
zeno build --minimal main.zeno                  # with the collector, if the program needs one
zeno build --minimal --owned main.zeno          # one owner for each list and object, no collector
zeno build --minimal --no-collector main.zeno   # the same, and memory from a pool of a fixed size
zeno build --minimal --firmware main.zeno       # errors as values, no setjmp
zeno build --minimal --memory main.zeno         # the program reports its memory as it ends
zeno build --minimal --fast main.zeno           # -O3 in place of -Os
```

A program that never makes a list, an object or a piece of text has no
memory management in it under any of these. One that does has what its
kind of build needs: with the collector that is about 500 lines of C,
and with `--owned` or `--no-collector` 100 to 200. For the smallest C,
use one of those two (docs/memory-modes.md says what they ask of the
program).

`--llvm`, `--web` and `--target` do not take it. `--release` adds
nothing to it.

## How much smaller

`python3 tools/minimal.py --sizes` builds every example both ways and
prints this table. Each cell is the checked build, then the minimal
one. "Code" is the bytes of machine code and constants (`text`, as the
`size` command counts it), the checked build compiled with `-O2` and
the minimal one with `-Os`, which is how `zeno build` compiles each.
"Held" is the memory the program has from start to end (`.data` +
`.bss`). "Asked for" is the most bytes of lists, objects and text it
had at one time (what `--memory` reports).

| program | build | lines of C | code | held | asked for |
|---|---|---|---|---|---|
| hello | collector | 4,494 → 23 | 1,942 → 1,401 | 648 → 624 | 0 → 0 |
| blink | `--no-collector` | 4,128 → 95 | 1,838 → 1,765 | 728 → 728 | runs for ever |
| signal | `--no-collector` | 4,213 → 180 | 3,350 → 2,746 | 840 → 816 | 0 → 0 |
| primes | collector | 4,539 → 100 | 13,453 → 2,086 | 1,928 → 648 | 192 → 0 |
| primes | `--no-collector` | 4,176 → 100 | 7,680 → 2,086 | 34,832 → 648 | 46 → 0 |
| thermostat | collector | 4,615 → 828 | 15,575 → 8,762 | 2,016 → 1,912 | 1,360 → 240 |
| thermostat | `--no-collector` | 4,260 → 350 | 10,671 → 4,241 | 34,896 → 33,796 | 289 → 224 |
| ring | collector | 4,624 → 911 | 13,304 → 9,376 | 1,936 → 1,912 | 352 → 176 |
| ring | `--owned` | 4,415 → 519 | 8,985 → 5,891 | 2,936 → 2,920 | 170 → 147 |
| ring | `--no-collector` | 4,283 → 447 | 8,317 → 5,105 | 34,848 → 34,800 | 170 → 147 |
| scheduler | `--no-collector` | 4,372 → 575 | 11,227 → 7,720 | 34,840 → 34,840 | 527 → 527 |
| shapes | collector | 4,627 → 1,093 | 19,894 → 13,176 | 1,984 → 1,960 | 848 → 560 |
| words | collector | 4,696 → 1,151 | 17,175 → 13,325 | 2,000 → 2,008 | 2,432 → 2,400 |
| words | `--no-collector` | 4,378 → 789 | 13,784 → 9,781 | 34,952 → 34,960 | 529 → 529 |

(Linux, x86-64, gcc 13, 2026-11-19. The 34 KB a `--no-collector` build
holds is its pool, 32 KB unless the program is built with another
size: docs/pool.md.)

Three things to read from it:

- **The C is where the difference is.** 23 lines for hello, about 100
  for a program of numbers, 350 to 1,150 for a program with lists,
  objects and text.
- **The program shrinks less: for most examples a quarter to a third of
  its code.** The C compiler never put an unused function into the
  program of any build, so what a minimal build saves there is the
  checks themselves, their messages, and what `-Os` finds. The file on
  disk moves in steps of 4 KB and often does not move.
- **Memory changes where `print` made text**, and where a program
  turns out not to need the pool at all (`primes`).

One number goes the other way. On Linux, a program whose `print` is
now `printf` has about 250 KB more of the C library in memory while it
runs, because `printf` is a large function that nothing ran before.
That is the library's memory and not the program's, and a board does
not page, but it is there in what the system reports.

## How it is checked

`python3 tools/minimal.py` builds 363 programs (every example, the
corner programs in `tools/targets/`, every sample of the book) both
ways, for the collector, `--owned`, `--no-collector` and `--firmware`,
and holds the minimal build to the checked one: the same text printed,
the same end, the same message when an error stops it. A program that
one of the checks stops is left out, since it has no defined result
here. The C of each minimal build is also read with the C compiler's
warnings on, and anything left in it that the program does not use is
a failure.

## The files

- `src/minimal.rs`: cuts C into its items, finds what the program
  reaches, and puts the file together (`assemble`).
- `src/emit_c.rs`: `Options { minimal }`; `Emitter::at`, `slot_at`,
  `divided` and `print_direct` write the unchecked forms; `printf_of`
  says what `printf` can show by itself; `tidy` shortens a statement.
- `src/runtime.c`: what a minimal build leaves out is between `#ifndef
  ZENO_MINIMAL` and `#endif /* ZENO_MINIMAL */`. The note at the top of
  the file says how the marks are written.
- `src/main.rs`: the option, what it does not go with, and `-Os`.
- `tools/minimal.py`: the check, and `--sizes`.

## Things to try

1. Build `examples/hello.zeno` with and without `--minimal` and count
   the lines of the two `hello.c` files.
2. Write `let xs = [1, 2, 3]` and `print(xs[5])`. Run it with `zeno
   run`, then build it both ways and run each. Do it twice.
3. Build `examples/ring/main.zeno` with `--minimal`, then with
   `--minimal --no-collector`. Which functions are in the first file
   and not in the second?
4. Find `zeno_float_text` in a minimal build of a program that prints
   a float. Why does `print(0.1 + 0.2)` not use `%g`?
5. Write a function that nothing calls and build with `--minimal`. Is
   it in the C? Now call it.
6. Run `python3 tools/minimal.py --sizes --only examples/primes` and
   read the row beside the table above.
