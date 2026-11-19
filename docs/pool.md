# The memory pool

A program built with `zeno build --no-collector` does not use the heap.
Its lists, objects and errors, and the text it keeps
([`text.md`](text.md)), live in **one array of a fixed size**, the
pool, which is part of the program from the start.

```sh
cargo run -- build --no-collector --memory examples/buffer.zeno
./buffer
grep -c -e malloc -e calloc -e realloc -e '[^_]free(' buffer.c    # 0
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

The middle line of the report is new. The program asked for memory 1691
times, 2.6 million bytes in all, and 25,384 bytes of pool were enough
for the whole run.

This page follows on from [`no-collector.md`](no-collector.md). That
page is about the compiler: it writes a free at the end of every
owner's block. This page is about what the free gives the block back
*to*.

## Why

Until now `zeno_alloc` was `calloc` and `zeno_free` was `free`. That
was honest and it worked, and for firmware it has four things wrong
with it.

- **MISRA C:2012 Rule 21.3** says the memory functions of `<stdlib.h>`
  shall not be used. That is `malloc`, `calloc`, `realloc` and `free`.
- **A small board may have no heap at all**, or one you would rather
  not link in.
- **Nobody can say how long `malloc` takes.** It searches. How long
  depends on what was freed before, in what order.
- **A heap breaks up.** After enough taking and giving back in mixed
  sizes there can be plenty of free memory and no piece of it big
  enough. The program then fails after three weeks, not on the bench.

A pool with fixed block sizes answers all four. How much memory the
program can ever have is decided when it is built. Taking and giving
are a handful of instructions whatever came before. And it cannot
break up, for a reason that is one sentence long and comes below.

## The files

- [`src/runtime.c`](../src/runtime.c): search for "The pool". It is
  about sixty lines, between `#else /* ZENO_NO_COLLECTOR */` and the
  text room. `zeno_alloc`, `zeno_free`, and three things they share.

That is the whole list. Nothing in the compiler changed, in either
backend. The compiler writes calls to `zeno_alloc` and `zeno_free`, and
does not care what is behind them (decision 5.9b said it would be so).

## The idea: a few sizes, and a list of spares for each

Blocks come in fixed sizes, each twice the one before:

| Step | Bytes | What fits |
|---|---|---|
| 0 | 16 | an object with two fields |
| 1 | 32 | a list (its length, its room, where its items are), or the items of a list of 4 |
| 2 | 64 | the items of a list of 8 |
| 3 | 128 | the items of a list of 16 |
| ... | ... | ... |

A request gets the smallest size it fits in. 24 bytes gets a 32-byte
block. 100 bytes gets 128.

The pool keeps, for each size, a **list of spare blocks**: blocks of
that size that were given back and are waiting to be used again. At
the start every list is empty and the whole array is unused.

```c
static union {
    char bytes[ZENO_POOL];
    uint64_t lined_up;
} zeno_pool;
static size_t zeno_pool_top = 0;
static zeno_spare *zeno_pool_spare[ZENO_POOL_STEPS];
```

`zeno_pool_top` says how much of the array has ever been cut into
blocks. It only goes up. (The union is there for its second member,
which is never used: it makes the C compiler line the array up so that
an `int` or a `float` can sit at the start of a block.)

**Taking a block:**

```c
zeno_spare *block = zeno_pool_spare[step];
if (block != NULL) {
    zeno_pool_spare[step] = block->next;          // a spare one: take it off its list
} else {
    size_t whole = sizeof(zeno_head) + room;
    if (whole > (size_t)ZENO_POOL - zeno_pool_top) zeno_pool_full();
    block = (zeno_spare *)(zeno_pool.bytes + zeno_pool_top);
    zeno_pool_top += whole;                       // none spare: cut a new one
    block->head.step = step;
}
```

**Giving it back:**

```c
block->head.asked = 0;
block->next = zeno_pool_spare[block->head.step];
zeno_pool_spare[block->head.step] = block;        // on the list for its size
```

There is no loop in either, apart from the short one that finds the
size (at most 24 turns, usually one or two). Nothing is searched.

## What a block looks like

```
        8 bytes              16, 32, 64 ... bytes
   +---------------+------------------------------------+
   | step | asked  |  what the program uses             |
   +---------------+------------------------------------+
                   ^
                   what zeno_alloc returns
```

In front of each block are eight bytes of the pool's own: which size
the block is, and how many bytes of it were asked for. `zeno_free` is
handed only an address, so it steps back eight bytes to find out which
list the block goes on.

A spare block needs to say where the next spare one is. It says so in
its own first bytes, which nobody is using:

```c
typedef struct zeno_spare {
    zeno_head head;
    struct zeno_spare *next;
} zeno_spare;
```

So the lists of spares cost no memory beyond one pointer for each
size. This is why the smallest block is 16 bytes and not 8: a pointer
has to fit.

## Follow three trips round a loop

```
owned class Frame {
    id: int
    bytes: owned list[int]
}

for id in range(3) {
    let bytes: owned list[int] = [id, id + 1, id + 2]
    let frame = Frame(id, bytes)
    print(len(frame.bytes))
}
```

```
memory: 9 blocks made (192 bytes), at most 64 bytes in use at once, no collector, 0 blocks left at the end
pool: 104 bytes taken, in a pool of 32768
```

**First trip.** Three requests, and every list of spares is empty, so
each one is cut from the array:

| Request | Size it gets | With its 8 bytes | `zeno_pool_top` after |
|---|---|---|---|
| the list, 24 bytes | 32 | 40 | 40 |
| its three items, 24 bytes | 32 | 40 | 80 |
| the `Frame`, 16 bytes | 16 | 24 | 104 |

At the `}` the compiler's `zeno_drop_Frame` gives all three back. Two
blocks go on the 32-byte list and one on the 16-byte list.

**Second trip.** The same three requests. This time each finds a spare
block of its size, and `zeno_pool_top` does not move.

**Third trip.** The same again.

Nine blocks were made and the pool grew three times. Make it three
million trips and the second line of the report does not change. The
test `with_no_collector_the_pool_does_not_fill_up_over_time` checks
exactly that.

## Why it cannot break up

A block is never cut in two, and two blocks are never joined. So a
block that was 32 bytes once is 32 bytes for ever, and when it is
spare it is there for the next 32-byte request, whole.

A heap gets into trouble because it does the opposite: it cuts big
free pieces into small ones to be thrifty, and then has to hope the
small ones come back next to each other so that it can join them. A
pool of fixed sizes gives up that thrift, and gets in return a promise
that can be stated plainly:

> If the program got through a piece of work once, it will get through
> the same piece of work again, any number of times, in the same pool.

For a control loop that runs for months, that is the promise that
matters.

## What it costs

Nothing is free, and the costs are easy to measure with `--memory`.

**Rounding up.** A request for 40 bytes takes a 64-byte block. On
average about a quarter of each block is unused.

**Eight bytes in front of every block.** For a 16-byte object that is
half as much again.

**Memory cut for one size is never used for another.** This is the
price of the promise above, and a growing list shows it best:

```
var items: owned list[int] = []
for n in range(100) {
    items.append(n)
}
```

```
memory: 7 blocks made (2040 bytes), at most 1560 bytes in use at once, no collector, 0 blocks left at the end
pool: 2104 bytes taken, in a pool of 32768
```

A list that is full moves to a place twice the size. This one moved
through room for 4, 8, 16, 32, 64 and 128 items. Each old place went
back on the list of spares for its size, and no later request was that
size, so they sit there: 32 + 64 + 128 + 256 + 512 bytes that are
spare and of no use to this program. The list ends up holding 1024
bytes of items and the pool needed 2104.

So a list that grows costs about twice its final size. `buffer.zeno`
is nearly all growing lists, which is why it needs 17,184 bytes of
pool for 12,592 bytes in use at once. Exercises 3 and 4 are two ways
to get that back.

## Choosing the size

The pool is `ZENO_POOL` bytes, and 32768 unless you say otherwise.
That default is only there so that the examples run. On a board you
choose it, and the report tells you what to choose. `--pool` says it
to `zeno build`, which hands it to the C compiler as `-DZENO_POOL`
(since 0.8.0, [`pool-size.md`](pool-size.md)):

```sh
cargo run -- build --no-collector --memory examples/buffer.zeno
./buffer 2>&1 | grep pool
cargo run -- build --no-collector --pool 17184 examples/buffer.zeno
```

By hand it is the same flag: `cc -O2 -DZENO_POOL=17184 -o buffer
buffer.c -lm`.

The number in the report is how much of the array was ever cut into
blocks, and it is exact. The program runs in a pool of that many bytes
and not in one a byte smaller (the test
`with_no_collector_the_pool_has_an_end` tries both). It is exact for
*that run*, of course: a program whose lists depend on its input needs
the report from its worst input, and some room on top.

Through LLVM it is the same option, and the same flag on the runtime
file:

```sh
cargo run -- build --llvm --no-collector --pool 17184 examples/buffer.zeno
clang -O2 -Wno-override-module -DZENO_POOL=17184 -o buffer buffer.ll buffer_runtime.c -lm
```

The pool lives in the runtime, which both backends share, so nothing
in the IR changed. The two builds must still write the same report,
and now that includes the line about the pool.

## When the pool is full

```
error: the memory pool is full (ZENO_POOL is 1024 bytes)
```

The program stops. That is a fault, like an index past the end of a
list or a statement that overflows the text room: a firmware build
does not try to carry on from it.

It can happen at only a few kinds of line: one that makes a list or an
object, appends to a list, walks a list with `for` (the loop walks a
copy), or raises an error. Every other line of the program is certain
not to need memory.

## A block given back twice

```
error: a block of memory was given back twice
```

The compiler writes exactly one free for each block, so this should
never be seen. It is checked all the same, because of what would
happen otherwise: the block would be on its list of spares twice, the
next two requests of that size would both be handed it, and two owners
would write over each other. That is the worst kind of bug to find,
and the check is one comparison: `asked` is 0 while a block is spare.

Since memory can be asked for and freed by hand ([`unsafe.md`](unsafe.md)),
a program can reach this message itself: with `free(p)` written twice.

## What this does not claim

- **MISRA Directive 4.12 is wider than Rule 21.3.** The rule bans the
  library's functions, and this build no longer calls them. The
  directive says dynamic memory allocation shall not be used at all,
  and a pool you wrote yourself is still that. What the pool does is
  take away the reasons the directive gives: memory that leaks (the
  compiler writes every free), a heap that breaks up (it cannot), time
  that cannot be predicted (it can), and running out with no warning
  (the report gives the number). A project under MISRA would still
  write a deviation for it. It is a far easier one to argue.
- **`printf` is still there**, and a C library's `printf` may use the
  heap for its own purposes. So are `exit`, variable argument lists
  and the GCC builtins: the list in [`firmware.md`](firmware.md).
- **The pool is not safe to use from an interrupt.** Zeno has no
  interrupts yet. When it does, taking and giving will need to be
  guarded.
- **It does not bound memory for you.** A loop that appends for ever
  fills the pool. It does so at a line that says so, which a heap does
  not promise.
- **Reading a block after giving it back is not caught.** The compiler
  is what prevents that (a value that was given away is null), and the
  tests used to run under a tool that watches the heap. Such a tool
  cannot see inside one big array. Exercise 6 puts some of that back.

## How it is tested

- Every test program for a build with no collector now runs in the
  pool, through C and through LLVM, and must still print what
  `zeno run` prints with 0 blocks left at the end.
- The C file, the IR and the runtime written beside the IR are
  searched for `malloc`, `calloc`, `realloc` and `free(`. None may be
  there, comments included. (The other two kinds of build are searched
  too, and must have them, so the search is known to work.)
- `with_no_collector_the_pool_has_an_end`: a pool that is too small
  stops the program with the message above, and the number in the
  report is exactly enough.
- `with_no_collector_the_pool_does_not_fill_up_over_time`: 4000 trips
  round a loop that makes lists of forty different lengths need the
  same pool as 80 trips.
- `the_pool_gives_a_block_back_to_its_own_size` drives the pool from a
  few lines of C: a block is lined up for an `int`, a block given back
  is handed out again for the next request of its size and is all
  zeros, another size does not get it, and giving back twice is caught.

By hand, `nm buffer` lists what the finished program takes from the C
library. Nothing in the list allocates.

## Things to try

1. Build the `Frame` program above with `--no-collector --memory` and
   change `range(3)` to `range(3000)`. Which numbers in the report
   change? Now make the list `[id, id + 1, id + 2, id + 3, id + 4]`.
   Work out the new pool size before you run it.
2. Build `buffer.zeno` with `--no-collector --memory --pool 16000`.
   It stops before it prints a line, having made 10 blocks and taken
   8,304 bytes. What asked for the block that did not fit? (A frame's
   samples are in a block of 8192 bytes. Which line of `buffer.zeno`
   needs a second one while the first is still in use?)
3. A comprehension over a range knows how long its list will be before
   it starts: `[n * n for n in range(size)]`. Find where `emit_c.rs`
   writes `zeno_list_new(0)` for a comprehension and pass the length
   when the loop is over a range with no `if`. What does `buffer.zeno`
   need now? Do the same in `emit_llvm.rs`: the two reports must agree
   again.
4. Give lists a method `reserve(n)` that makes room for `n` items in
   one step. Where does it go in the checker, the interpreter, the VM
   and the two compilers? What should the interpreter do with it?
5. Add `-DZENO_POOL_STEPS=8` to a build by hand. What is the largest
   list the program can now make? Each size costs one pointer of RAM
   for its list of spares. How much RAM did that save, and when would
   it be worth it?
6. Put back a little of what the heap checker gave. (Built in 0.8.0,
   behind `--memory`: [`pool-size.md`](pool-size.md). Read it after
   you try.) In `zeno_free`, fill the block with the byte `0xDD`, and
   in `zeno_alloc`, when the block came from a list of spares, check
   that every byte after the first pointer is still `0xDD` before
   wiping it. What kind of bug does that catch? Wrap it in `#ifdef
   ZENO_POOL_CHECK` and run the tests with it.
7. `zeno_pool_full` does not say which size was asked for or how much
   of the pool is spare. Add both to the message. Then explain this
   output: a 32 KB pool that is "full" with 20 KB of it spare.
8. The other classic design is one pool for each *type*: so many
   `Frame`s, so many lists of 64 items, each count chosen by the
   programmer. What would Zeno's syntax for that look like? What could
   the compiler then promise that this pool cannot?
9. Give `zeno build` an option `--pool 4096` that passes
   `-DZENO_POOL=4096` to the C compiler, and one for the text room.
   Look at how `--memory` does it in `src/main.rs`. (Built in 0.8.0 as
   `--pool` and `--text-room`. Compare yours with `size_option`.)
