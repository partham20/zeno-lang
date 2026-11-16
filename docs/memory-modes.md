# Choosing how memory is freed

A Zeno program that makes lists, objects and text needs memory for
them, and has to give it back. There are three ways to do that in
Zeno, and a program chooses:

| | How memory is freed | Like | Where you choose it |
|---|---|---|---|
| **collector** | a garbage collector frees whatever nothing can reach | Python, Java, Go | nothing to say: it is what a program gets |
| **owned** | every list and object has one owner, and is freed on the line its owner ends. No collector | Rust | `memory owned` in `zeno.package`, or `zeno build --owned` |
| **by hand** | you ask for memory with `allocate` and give it back with `free` | C | an `unsafe { }` block, for the lines that need it |

They mix, at the sizes where mixing has a clear meaning:

- **The program**, in `zeno.package`: `memory owned` or `memory
  collector`. This is the one most programs set.
- **One build**, on the command line: `--owned` or `--collector`, over
  what the file says.
- **One file**, with the line `memory owned` at its top: every class the
  file declares is owned, as if written `owned class`, and its objects
  are freed where their owner ends even in a program with a collector.
  When every file of a program says it, the program is owned.
- **One value**, in a collector program: `owned class Packet` or `owned
  list[int]` (docs/ownership.md). That value has one owner and is freed
  where its owner ends, and everything else is the collector's.
- **A few lines**, anywhere: `unsafe { }` (docs/unsafe.md).

Why not one line at a time: a value is made on one line and used on
many, and how it is freed has to go with the value. Decision 22.1 has
the whole reasoning.

## A program that says `memory owned`

```sh
zeno new server
```

and add a line to `server/zeno.package`:

```text
package server
memory owned
```

Now, in that folder:

- `zeno build main.zeno` builds with no collector. Each list and object
  is freed on the line its owner ends, as in Rust. The program asks the
  system for memory as it needs it, in pieces that double.
- `zeno check main.zeno` and `zeno run main.zeno` check the program by
  the same rules, so a mistake about who owns what is found before the
  build. The words for it are in docs/one-owner.md: a parameter looks
  unless it says `take`, `pop()` takes an item out of a list, and so on.
- `zeno build --collector main.zeno` builds it with the collector after
  all, for one build.

## One file that says `memory owned`

A program can keep the collector for most of its code and free the
objects of one file exactly. Put the line at the top of that file:

```zeno
memory owned

class Node {
    left: Node or none
    right: Node or none
}

function make(depth: int) -> Node {
    if depth == 0 {
        return Node(none, none)
    }
    return Node(make(depth - 1), make(depth - 1))
}
```

and import it from a file that says nothing:

```zeno
import shapes

let tree = shapes.make(10)
print(tree.left is none)
```

`zeno build --memory main.zeno` shows the objects of `Node` freed
"without the collector", and nothing else changes. Where the two ways
meet, the rules are those of owned values in a collector program
(docs/ownership.md): a function that only looks can be given any
object, and an object with one owner cannot be kept in a shared list.
A list that should have one owner says so where it is made: `owned
list[int]`. Decision 22.9 has the reasons, and why it is the file and
not the function.

## For a chip

`--no-collector` is the same rules with a pool and a text room of a
fixed size, set when the program is built (docs/pool.md), so the
program can never fail to get memory from a system that is not there.
`memory owned` with `--firmware` means the same.

| Command | Collector | Memory |
|---|---|---|
| `zeno build` | yes | from the system, freed by the collector |
| `zeno build --owned` | no | from the system, in pieces that grow |
| `zeno build --no-collector` | no | one array of a fixed size |
| `zeno build --owned --firmware` | no | one array of a fixed size |

## `--release`

```sh
zeno build --release --owned main.zeno
```

`--release` drops the overflow checks: `+`, `-` and `*` of whole numbers
wrap round, as the processor does, instead of stopping the program with
an `OverflowError`. Everything else is still checked (an index past the
end, `//` by zero, `u8(300)`), and `zeno run` checks everything as it
always did. It is what Rust's release builds do, for the same reason:
the check costs a little on every step of a tight loop. Decision 22.3.

## `--fast`

```sh
zeno build --fast --owned main.zeno
```

`--fast` leaves out every check that a correct program never fails, as
C leaves them out:

| | other builds | `--fast` |
|---|---|---|
| `xs[i]` with `i` out of range | `IndexError` | reads (or writes) whatever memory is there |
| `x / 0.0` | `ZeroDivisionError` | infinity, or NaN for `0.0 / 0.0` |
| `math.sqrt(-1.0)` | `ValueError` | NaN |
| `n // 0`, `n % 0` of whole numbers | `ZeroDivisionError` | `ZeroDivisionError` still |
| overflow of `+ - *` | `OverflowError` (`--release`: wraps round) | no defined result: the C compiler is told it does not happen |

The C compiler is asked for `-O3` and told that no line reads `errno`
after a function of maths (`-fno-math-errno`), so `sqrt` is one
instruction. The C says `#define ZENO_UNCHECKED 1` on its first line,
so the file builds the same way by hand. It goes with `--llvm`, where
an item is read with a `getelementptr` and `/` is `fdiv`, and with any
way of freeing memory. `zeno run` checks everything, as always.

The last row is where `--fast` and `--release` part. With `--release`
a sum that overflows wraps round: a wrong number, but a known one.
With `--fast` the `+`, `-` and `*` are C's own (`add nsw` in LLVM IR),
and the C compiler builds on their not overflowing, as it does for a
program written in C: a count that starts at 0 and goes up is never
below zero, so `count % 7` needs no care for a number below zero. That
knowledge is what made the loop benchmark the same instructions as C's
(decision 23.6). A program that does overflow may then print a wrong
number or never leave a loop. An overflow is an error in Zeno, so a
program that runs clean under `zeno run` and `zeno build` has none.

Build it the ordinary way while a program is being written and tested,
and with `--fast` when its tests pass and the last bit of speed
matters. A wrong index in a `--fast` build is a wrong answer or a crash
far from the line that caused it, exactly as in C. Decisions 23.1 and
23.6.

## What it buys

On the benchmarks in `bench/` (`python3 bench/run.py --size big`; the
numbers and how they were taken are in bench/README.md), a program
that makes many small objects runs about as fast as the same program in
C when it is owned, because the pool hands out and takes back a block
in a few instructions. With the collector the same program is about two
times slower than C.

## The files

- `src/package.rs`: the `memory` line of `zeno.package` (`Memory`).
- `src/main.rs`: `how_built`, which turns the options and the file into
  a kind of build, and `package_memory`.
- `src/runtime.c`: the pool that grows and the chain of text rooms,
  marked `ZENO_HEAP`; the collector, under "Where blocks live".
- `src/emit_c.rs`, `src/emit_llvm.rs`: `Options { release, grows, fast }`.
  The checks `--fast` leaves out are behind `#ifndef ZENO_UNCHECKED` in
  runtime.c; `optimise` in main.rs picks the C compiler's flags.
- `src/ast.rs`: `walks_in_place`, which lets a loop skip its copy.

## Things to try

1. Build `examples/scheduler.zeno` with `--owned --memory` and with
   `--no-collector --memory`. Compare the last two lines of the reports.
2. Write `var n = 9223372036854775807` and `n += 1`, and build it with
   and without `--release`. Then run it with `zeno run`.
3. Put `memory owned` in a `zeno.package` beside a program that shares
   a list between two variables. What does `zeno run` say now?
4. Time `bench/trees.zeno` built the three ways in the table above.
5. Build `let xs = [1, 2, 3]` / `print(xs[5])` with `--release` and with
   `--fast`, and run both. Then look for `ZENO_UNCHECKED` in the two `.c`
   files.
6. Write a loop that adds `(i * i) % 7` for a thousand million `i`,
   build it with `--release` and with `--fast`, and time the two. Then
   find the loop in each `.c` file: which one could a C compiler know
   more about?
7. Move `Node`, `make` and `check` of `bench/trees.zeno` into a file
   of their own that says `memory owned`, import it, and build with
   `--memory`. How many collections are left?
