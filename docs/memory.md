# How memory is managed

Memory management is the stage after compiling to C. Before it, a compiled Zeno program
took memory from the machine and never gave any back. Now it carries a
**garbage collector**: a small piece of the runtime that finds the memory
the program can no longer use and frees it.

```sh
cargo run -- build --memory examples/memory.zeno
./memory
```

```
...
memory: 780027 blocks made (13222784 bytes), at most 1048574 bytes in use at once, 10 collections, 60002 blocks freed without the collector
```

That program asked for thirteen megabytes, a few bytes at a time, and never
held more than one. `--memory` is what adds the last line; without it the
collector works just the same and says nothing.

The last figure counts blocks that never waited for the collector. A few
frees are safe at a line the compiler can point to: the old storage of a
list that has just grown, the copy a `for` loop walks, and every value
marked `owned`. Those are the subject of
[`docs/ownership.md`](ownership.md). Everything else on this page is about
the blocks nobody owns.

`zeno run` is not affected. The interpreter is a Rust program, and Rust
frees its values for it.

## The files

- [`src/runtime.c`](../src/runtime.c), the section headed "Memory": the
  whole collector, about 200 lines of C.
- [`src/emit_c.rs`](../src/emit_c.rs): three small pieces. The top-level
  code moved out of `main`, `main` notes where the stack starts, and one
  generated function, `zeno_roots`, lists the top-level variables.
- [`examples/memory.zeno`](../examples/memory.zeno): a program that makes
  far more than it keeps.

## What needs memory

Numbers and bools live in C variables and need nothing. These do take
memory from the machine while the program runs:

- text that is built (`a + b`, an f-string, `text.upper()`),
- every list, and the items inside it,
- every object and every enum value that carries data,
- a function written inside another, and the box of a variable they share.

Each of these is a **block**: a piece of memory from C's `malloc`. The
question of this stage is when to call `free`.

## The question nobody can answer in advance

```
function label(reading: Reading) -> str {
    let text = f"tick {reading.tick}"
    return text
}
```

Is `text` finished with when `label` returns? No: the caller has it now.
And the caller may put it in a list, which goes into an object, which a
function value remembers. In C the programmer keeps track of all that by
hand, and the mistakes are famous: freeing too early (the program uses
memory that is now something else), freeing twice, or never freeing.

A collector answers a simpler question, and answers it while the program
runs: **can the program still reach this block?** If no variable leads to
it, directly or through other blocks, nothing can ever use it again, and it
is safe to free.

## Mark and sweep

The collector here is the oldest and simplest kind. It was invented for
Lisp in 1960 and is still what MicroPython runs on microcontrollers.

1. **Every block is written in a table** as it is handed out
   (`zeno_new_block`).
2. **Mark.** Start from the variables, which are called the *roots*.
   Whatever a root points at is in use: mark it. Whatever a marked block
   points at is in use too: mark that. Carry on until nothing new turns up.
3. **Sweep.** Go through the table. A block with no mark cannot be reached:
   free it. Take the marks off the rest, ready for next time.

```
roots            blocks
-----            ------
recent  ------>  [list] ---> [Reading 49999] ---> "sample 49999"
hottest ------>  [Reading 102] ---> "sample 102"

                 [Reading 7] ---> "sample 7"       nothing points here:
                 [list] ---> [Reading 3]           all four are freed
```

Two objects that point at each other, and at nothing else that is in use,
are freed like anything else. Nothing reaches them from a root, so neither
gets a mark. (This is the case that defeats the other popular method,
counting references. More on that below.)

## Where the roots are

A variable of a compiled Zeno program is in one of two places.

**Top-level variables** are C globals. The compiler knows them all, so it
writes a function that hands each one to the collector. For
`examples/memory.zeno` it is:

```c
static void zeno_roots(void) {
    zeno_mark_words(&zeno_raised, sizeof zeno_raised);
    zeno_mark_words(&z_recent, sizeof z_recent);
    zeno_mark_words(&z_hottest, sizeof z_hottest);
}
```

(`zeno_raised` is the runtime's own variable: an error that has been raised
and is on its way to an `except`.)

**Everything else is on the C stack**: local variables, arguments, and the
temporaries the compiler invents. Here the collector does something
blunt. It does not ask which stack slots are pointers. It reads the whole
stack, from the function running now back to `main`, one word at a time,
and asks of each word: *is this the address of one of my blocks?*

```c
static void zeno_mark_stack(void) {
    void *here = NULL;
    uintptr_t low = (uintptr_t)&here, high = (uintptr_t)zeno_stack_base;
    ...
    zeno_mark_words((const void *)low, high - low + sizeof(void *));
}
```

`&here` is the address of a variable in the newest function, so it is one
end of the stack. `zeno_stack_base` is the address of a variable in `main`,
the other end. That is why the top-level code no longer lives in `main`:

```c
int main(void) {
    void *base = NULL;
    zeno_stack_base = &base; /* the stack starts here */
    zeno_file = "memory.zeno";
    zeno_program();
    return 0;
}
```

Everything the program does happens inside `zeno_program`, so every
variable it has lies between those two addresses.

One more hiding place: a pointer may be in a processor register and nowhere
in memory. `__builtin_unwind_init()` at the top of `zeno_collect` makes the
C compiler copy the registers onto the stack first.

## "Is this word a pointer?"

`zeno_mark` answers it. The table is sorted by address at the start of each
collection, so the answer is a binary search: find the last block that
starts at or before the word, then see whether the word falls inside it.
Most words are turned away sooner, by two comparisons against the lowest
and highest addresses any block has.

The same question is asked of the words *inside* a marked block, which is
how the collector gets from a list to its items and from an object to its
fields. It still does not know which fields are pointers. It does not need
to.

This is called a **conservative** collector: when unsure, it keeps. It can
be wrong in one direction only. An integer that happens to equal the
address of a block keeps that block alive for nothing. It can never free
something in use, as long as every pointer is somewhere it looks.

Blocks of text are marked as *plain*: they cannot hold a pointer, so the
collector does not read them. That is quicker, and a piece of text is the
likeliest thing to contain bytes that look like an address by accident.

## Following pointers without recursion

The obvious way to mark is a function that marks a block and calls itself
for everything inside. On a list of 300,000 nodes, each pointing to the
next, that is 300,000 calls deep, and the collector crashes the program it
is serving.

Instead, a block that has been marked but not yet looked into is put on a
waiting list, and one loop takes blocks off it until it is empty. The
waiting list is threaded through the blocks themselves (`next_waiting`, a
field in front of every block), so the collector needs no memory of its own
at the one moment when there may be none to have.

Between collections that field has nothing to do, so it holds something
else: the block's position in the table (`place`). The two share one word,
`link`. Knowing the position is what lets `zeno_free` take a single block
out of the table without searching for it.

## When it runs

Collecting takes time, in proportion to what is in use. So it is not done
on every allocation. The runtime counts the bytes in use, and collects when
they pass a limit. After each collection the limit becomes twice what
survived. A program that keeps little is collected often and cheaply. A
program that keeps a great deal is left alone for longer.

The first limit is one megabyte (`ZENO_FIRST_COLLECTION`). There is a
second trigger: if `malloc` itself says there is no more, the collector
runs and the request is tried again. On a board with 64 KB of memory that
second trigger is the one that matters.

## What it costs

- **Pauses.** While the collector runs, the program does not. The pause
  grows with the amount of memory in use, not with the amount of garbage.
  A control loop that must answer within microseconds should not make
  blocks inside the loop. Numbers, bools and fixed text never do.
- **A few words per block**: the header in front of it and its line in the
  table.
- **It leans on the C compiler.** Reading the stack like this is not
  something the C standard describes. It works on every mainstream compiler
  and processor, and the Boehm collector, Ruby and MicroPython have relied
  on it for decades, but it is a habit, not a guarantee.
- **Now and then it keeps too much.** A stale value in a stack slot that no
  longer means anything can hold one block, and all that block leads to.

[`docs/decisions.md`](decisions.md), 5.7, says why this was chosen over
counting references and over Rust-style ownership, and what would make
the choice change.

## How it is tested

A collector has one unforgivable bug: freeing a block that is still in use.
The program then reads memory that belongs to something else, and fails
much later, somewhere unrelated.

So every test in `emit_c.rs` that compares a compiled program with the
interpreter is now built with `-DZENO_COLLECT_ALWAYS`, which runs the
collector before **every single allocation**. A block that is wrongly freed
is freed at the first chance, and the program prints something different
from `zeno run`. Three more tests aim at the collector itself: one checks
that a loop of garbage stays within a small amount of memory, one that a
long chain, lists, objects and a function value come through many
collections intact, and one that an error in flight is not lost.

## Things to try

1. Build `examples/memory.zeno` with `--memory`. Change `range(50000)` to
   ten times as many. Which of the numbers in the report grow, and which
   do not?
2. Keep every reading (`all.append(reading)`, never emptied). Watch the
   number of collections. Why does it rise so slowly?
3. Compile `memory.c` by hand with a lower first limit:
   `cc -O2 -DZENO_MEMORY_REPORT -DZENO_FIRST_COLLECTION=4096 -o memory memory.c -lm`.
   What happens to the most-in-use figure, and to the running time?
4. Make the report say how long the longest collection took. (`clock()`
   from `<time.h>` is enough.)
5. Break it on purpose: in `memory.c`, delete the line for `z_hottest`
   from `zeno_roots`, compile by hand with `-DZENO_COLLECT_ALWAYS`, and
   run. What happens when the program gets to the hottest reading? This
   is what a missed root looks like.
6. The table is sorted again at every collection. Blocks that survived are
   still in order; only the new ones are not. Sort just those and merge.
7. `malloc` is a general-purpose allocator. Most Zeno blocks are 16 to 48
   bytes. Give the collector its own pool for small blocks and compare the
   speed.
