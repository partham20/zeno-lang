# A build with no collector

`zeno build --no-collector` compiles a Zeno program to C that has no
garbage collector in it. Not a collector that is switched off: the code is
not in the file.

```sh
cargo run -- build --no-collector --memory examples/buffer.zeno
./buffer
```

```
sent 44400 samples, skipped 13 quiet frames
refused 33 frames, the first on channel 84
[3, 3, 3, 3]
4
memory: 1691 blocks made (2646299 bytes), at most 13336 bytes in use at once, no collector, 0 blocks left at the end
pool: 25384 bytes taken, in a pool of 32768
text: at most 53 bytes in use at once, in a room of 1024
```

The program made 1691 blocks and gave every one of them back, at a line
you can find in `buffer.c`. Open that file and search for `zeno_collect`,
`zeno_roots` or `setjmp`. None is there. Nor is `malloc`: the blocks come
from a pool of a fixed size that is part of the program
([`pool.md`](pool.md)).

It is a firmware build ([`firmware.md`](firmware.md)) with one more
promise, so `--no-collector` turns `--firmware` on by itself.

## Why

The collector ([`memory.md`](memory.md)) is the right default. It lets you
write `let names = text.split(",")` and never think about it again. It
also has three habits that firmware cannot always live with:

- **It pauses the program**, for a time that depends on how much memory is
  in use. A control loop with a deadline cannot plan for that.
- **It reads the C stack** to find out what is still in use. That works on
  every mainstream compiler, and no C standard describes it.
- **Nobody can say when a block is freed**, so nobody can say how much
  memory the program needs in the worst case.

Owned values ([`ownership.md`](ownership.md)) answer all three for the
values that are owned. This build is what you get when *every* value is.

## The files

- [`src/emit_c.rs`](../src/emit_c.rs), the part headed "With no collector
  to help": `hold`, `text_begin`, `text_end`, `stays`, `list_has_owner`.
  The refusals are where each kind of value is made: search for
  `without_collector`.
- [`src/runtime.c`](../src/runtime.c): the parts between
  `#ifdef ZENO_NO_COLLECTOR` and `#endif`. As with `ZENO_FIRMWARE`, the
  compiler keeps one side when it pastes the runtime in (`runtime` in
  `emit_c.rs`).
- [`src/checker.rs`](../src/checker.rs): one small addition, `fresh`. The
  checker tells the compiler which lists start life owned.

## Where memory comes from now

With no collector, memory that is taken must be given back by a line of
the program. There are exactly three ways to get memory in this build, and
each has such a line.

| What | Where it lives | When it is given back |
|---|---|---|
| An owned list or object | the pool, a fixed array | when its owner's block ends |
| Text built while the program runs | the text room, a fixed array | when the statement that built it is over |
| An error that was raised | the pool | when the `except` block that caught it ends |

Numbers, bools, quantities, enum cases without data and text written in
the program (`"idle"`) need no memory at all, here as in every build.

Anything else that would take memory is **refused when you build**, with
the line that does it:

```
error: this list has no owner. Give it one, as in `let items: owned list[int] = [1, 2, 3]`. `zeno build --no-collector` leaves the collector out, so nothing would free it. docs/no-collector.md lists what such a build accepts
  --> buffer.zeno:58:26
   |
58 | var refused: list[int] = []
   |                          ^
```

That is the whole idea of the build. It does not find leaks while the
program runs. It refuses to build a program that could have one.

## Owned lists and objects

Nothing new happens here, and that is the point. Stage 2 already wrote a
free at the end of every owner's block and on every early way out. What
changes is what the free *is*. With a collector, `zeno_free` takes the
block out of the collector's table and hands it to the C library's
`free`. Here there is no table and no C library heap. `zeno_alloc` takes
a block from the **pool**, one array whose size is fixed when the C is
compiled, and `zeno_free` puts it on a list of spare blocks of its size:

```c
block->head.asked = 0;
block->next = zeno_pool_spare[block->head.step];
zeno_pool_spare[block->head.step] = block;
```

[`pool.md`](pool.md) is the walkthrough of the pool: how it is laid out,
why it cannot break up into unusable pieces, what it costs, and how to
choose its size.

## Text: one statement, one fixed room

Firmware prints. `print(f"sent {sent} samples")` has to work, and the text
it builds has to live somewhere for a moment.

It lives in the **text room**: one array, `zeno_text_room`, whose size is
fixed when the C is compiled. `zeno_text_top` says how much of it is in
use. Building text writes at the top and moves the top up. The compiler
notes the top before a statement that builds text, and puts it back after:

```c
size_t m2 = zeno_text_top;
zeno_print(1, zeno_join(5, "sent ", zeno_str_int(z_sent), " samples, skipped ", zeno_str_int(z_quiet), " quiet frames"));
zeno_text_top = m2;
```

Giving text back is one assignment. There is nothing to search and nothing
to fragment.

A function called in the middle of a statement does the same for its own
statements, above the caller's half-built text. So the room is used like a
stack, and its size is the most text that can be in the making at one
moment. The report's second line tells you what that was. Compile the C by
hand to choose the size:

```sh
cc -O2 -DZENO_TEXT_ROOM=256 -o buffer buffer.c -lm
```

If a statement needs more than the room has, the program stops and says
so. That is a fault, like an index past the end of a list.

**The rule that makes this safe:** built text cannot be kept. It can be
printed, compared, measured and passed to `int()`. It cannot be put in a
variable, a field or a list, returned, or given to one of your own
functions:

```zeno
print(f"channel {channel}")              # fine
if command.strip().upper() == "ON" { }   # fine
let line = f"channel {channel}"          # refused: `line` would outlive it
```

So every `str` that is stored anywhere is text that was written in the
program. The compiler tracks this with one flag on each compiled value
(`fixed` on `Val`): a literal, a variable, a field, a list item and the
result of one of your functions are fixed, and everything else is built.
`stays` is asked wherever a value is about to be kept.

An `if` or `while` whose condition builds text takes its answer first,
gives the text back, and then runs its body. A `for` or `match` whose
first line builds text is refused. Work the value out on the line before.

## Errors

`raise Overload(channel, peak)` makes an object, and an error to carry it.
Neither has an owner in the usual sense: the error leaves the function
that made it. So this build gives it one rule of its own. **The `except`
block that catches an error frees it when the block ends**, however the
block is left:

```c
if (zeno_is(zeno_caught_1, "Overload")) {
    z_Overload *z_overload = (z_Overload *)zeno_caught_1->object;
    zeno_list_append(z_refused, (zeno_slot){.i = z_overload->z_channel});
    zeno_error_drop(zeno_caught_1);
}
```

Inside the block the error can be read, printed and asked for its fields.
It cannot be kept, handed to a function or raised again. Copy out what you
need. The other half of the rule: an error can only be made by `raise`, as
in `raise ValueError("too hot")`. `let e = ValueError("too hot")` is
refused, because no `except` would ever free it.

The error's message is often built text (`raise ValueError(f"{n} is too
big")`), which would be gone before the `except` runs. So in this build
the error keeps its own copy of the message, in the same block as itself.

An error that nobody catches stops the program. There is nothing to free
after that.

## A value between two owners

Three times, a value belongs to nobody for a moment. With a collector that
does not matter. Here each needs an answer, and all three get the same
one: the compiler **holds** the value (`hold` in `emit_c.rs`), which means
that any early way out of the block frees it, just as it frees what the
block's owners hold.

1. **A list being filled.** `[check(n) for n in range(size)]`: if `check`
   fails on the fourth item, the first three are in a list that has no
   name yet.

   ```c
   zeno_list *t24 = zeno_list_new(0);
   for (int64_t z2_n = 0; z2_n < t25; z2_n++) {
       int64_t t26;
       if (!z_check(z2_n, &t26)) { zeno_list_drop(t24); return false; }
       zeno_list_append(t24, (zeno_slot){.i = t26});
   }
   ```

2. **An owned value on its way into a call.** In `join(a, b, check(n))`,
   `a` and `b` have left their variables by the time `check` runs. If it
   fails, the failure frees them.

3. **The copy a `for` loop walks** (decision 3.10a). A `return` or a
   failed call inside the loop frees it on the way out.

The first and third were small leaks to the collector in stages 1 and 2.
They are now fixed in every build.

## What is refused

Each of these makes memory that no line gives back. Each error names the
line and says what to write instead where there is something.

| Refused | Why |
|---|---|
| A list or object that is not `owned` | nobody frees it |
| Built text that is kept | it lasts one statement |
| An error that is kept, passed on, or raised again | its `except` frees it |
| An error made anywhere but in `raise` | no `except` would free it |
| An enum case that carries values | it sits in a block of its own |
| A function written where a value goes | so does it, with the variables it uses |
| `split`, `map`, `filter`, `+` on lists | each makes a new list with no owner |
| `for ch in text` | it makes a list of the characters. Use `range(len(text))` and `text[i]` |
| A list of `int or none` | each item sits in a block of its own |
| A list written where it is only looked at: `x in [1, 2]`, `look([1, 2])` | nobody owns it |

Two things you might expect on this list are not. A named function used as
a value (`items.reduce(add, 0)`) is fine: it takes no memory. A list
written as what a `for` loop walks (`for pin in [2, 3, 5]`) is fine: the
loop frees it.

Most of these are provisional. An enum value or a function value could be
owned, and a literal list could be lent for the length of a call. None is
needed yet.

## What this does not claim

- **Memory is still taken and given back while the program runs.** It
  comes from a fixed pool and not from the heap, so MISRA C:2012 Rule
  21.3 (no `malloc`, `calloc`, `realloc` or `free`) is met. Directive
  4.12 is wider: it is against dynamic memory of any kind, and a pool is
  still that. [`pool.md`](pool.md), "What this does not claim", says what
  the pool does and does not answer.
- **It does not bound memory for you.** A loop that appends to an owned
  list for ever fills the pool, and the program stops there. `--memory`
  tells you what a run needed.
- **The text room is a fixed size that you choose.** Too small, and the
  program stops at the statement that overflowed it.
- The rest of the list in [`firmware.md`](firmware.md) stands: `printf`
  and `exit`, variable argument lists in the runtime, the GCC builtins for
  checked arithmetic, and the forward `goto`.

## The same, through LLVM

```sh
cargo run -- build --llvm --no-collector --memory examples/buffer.zeno
```

`zeno build --llvm --no-collector` is this design written in LLVM IR.
An owner is a slot that is set to null when it gives its value away
and dropped where its block ends. The text room's top is a global the
IR loads before a statement that builds text and stores back after it.
An `except` drops the error it caught. The runtime file beside the IR
is the one from this page, and what is refused is refused in the same
words. [`llvm.md`](llvm.md), "Owned values" and "With no collector",
walks through the IR.

The two builds do not only print the same. They make the same number
of blocks, hold the same number of bytes at most, and fill the text
room to the same height, and the tests require it.

## How it is tested

Every test program for this build (`no_collector_as_interpreter` in
`emit_c.rs`) is checked four ways:

1. It prints exactly what `zeno run` prints.
2. Its memory report ends with **0 blocks left at the end**. With no
   collector, a block that no line frees is never freed, so this number
   is the test.
3. The C file does not contain `zeno_collect`, `zeno_roots` or `setjmp`,
   nor `malloc`, `calloc`, `realloc` or the library's `free`.
4. The same program still agrees with the interpreter in the two builds
   that do have a collector.
5. Built through LLVM with no collector, it prints the same, and its
   memory report is the same as the C build's, line for line.

The programs aim at the hard places: an error raised inside a loop over a
list, inside another `except`, half way through filling a list, and while
owned values are on their way into a call; `return`, `break` and
`continue` out of `except` blocks; and two thousand trips round a loop
that builds text, which must leave the text room as it found it. A table
of 29 small programs checks that each refusal says what it should, and
that each of them still builds where there is a collector.

They were also run by hand with gcc and clang at several optimisation
levels, and, while memory still came from the heap, under
AddressSanitizer with its leak check on. [`pool.md`](pool.md) lists the
tests of the pool itself.

## Things to try

1. Build `examples/buffer.zeno` with `--memory`, then with
   `--no-collector --memory`. The second makes fewer blocks. Which ones
   are gone? (Look at where the text went.)
2. In `buffer.zeno`, change `refused` back to a plain `list[int]` and
   build with `--no-collector`. Read the error. Then try keeping an error:
   `var last: Overload or none = none` and `last = overload` in the
   `except`.
3. Compile `buffer.c` by hand with `-DZENO_TEXT_ROOM=32`. Which statement
   stops the program? What is the smallest room it runs in, and does the
   report agree?
4. `let line = f"plain text"` is refused, though nothing in it is built.
   Find where `format_string` makes its value and mark it `fixed` when
   every part is a literal.
5. This exercise used to be "replace `calloc` and `free` with a pool".
   That is done ([`pool.md`](pool.md)), so here is what is left of it:
   print, from `zeno_memory_report`, how many blocks of each size were
   ever cut from the pool. Which sizes does `buffer.zeno` ask for, and
   which size holds most of the pool?
6. `print(42)` builds the text `"42"` and then prints it. Give the runtime
   a `zeno_put_int` that prints a number with no text room, and make the
   compiler use it when `print` has one argument.
7. `look([1, 2, 3])` is refused, where `look` takes a `borrow list[int]`.
   The list could be held for the length of the call and freed after it.
   `hold` and `let_go` are most of what that needs.
8. `raise e` inside `except ... as e` is refused. It could be a move: the
   error goes back to `zeno_raised` and the block no longer frees it. What
   would the checker have to know?
