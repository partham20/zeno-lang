# The size of the pool on the command line, and spare blocks that are checked

A build with no collector keeps its lists, objects and kept text in a
pool, and the text a statement builds in a text room. Both are arrays
of a size fixed when the program is built: 32,768 bytes for the pool
and 1,024 for the text room, unless the C compiler is told otherwise
([`pool.md`](pool.md), [`no-collector.md`](no-collector.md)). Until
0.8.0 the only way to tell it was by hand, as `-DZENO_POOL=` on a
command line of the C compiler. Now `zeno build` says it:

```zeno
var readings: list[int] = []
for n in range(100) {
    readings.append(n * 3)
}
print(len(readings), readings[99])
```

```sh
cargo run -- build --no-collector --memory readings.zeno
./readings
```

```
100 297
memory: 7 blocks made (2040 bytes), at most 1560 bytes in use at once, no collector, 0 blocks left at the end
pool: 2104 bytes taken, in a pool of 32768
text: at most 8 bytes in use at once, in a room of 1024
```

The report says the program needs 2,104 bytes of pool, so that is
what it is given:

```sh
cargo run -- build --no-collector --pool 2104 readings.zeno     # prints 100 297
cargo run -- build --no-collector --pool 2103 readings.zeno
./readings
```

```
error: the memory pool is full (ZENO_POOL is 2103 bytes)
```

`--text-room 64` does the same for the text room. The reasons for each
choice are in 56.15 of [`decisions.md`](decisions.md). It is part 9.7
of [`plan-9-14.md`](plan-9-14.md).

## The two options

| Written | What it does |
|---|---|
| `--pool 4096`, `--pool=4096` | hands `-DZENO_POOL=4096` to the C compiler |
| `--text-room 2048`, `--text-room=2048` | hands `-DZENO_TEXT_ROOM=2048` to the C compiler |

They go with `--no-collector` and with `--board`, which builds with no
collector too, and with `--llvm`, where the flag goes on the runtime
file beside the IR. Nothing in the C or the IR changes: the C file a
build writes is the one it wrote without them, and the size is in the
command that compiles it, as it was by hand.

Each is refused where it means nothing, with a message that says why:

- **With a collector**: the memory comes from the system as the
  program needs it, so there is no pool to size. The message says to
  add `--no-collector`, or to build for a board.
- **With `memory owned` on a computer** (`--owned`, or the line in
  `zeno.package`): the pool and the text room grow, so there is no
  size to set. `--no-collector` asks for the fixed ones.
- **With `--web` or `--target`**, and with any command but `build`.
- **A size that is not a whole number above zero**, or more than
  2,147,483,647 bytes, which the runtime's messages could not print.
  The message gives an example.
- **The same option twice.**

## Spare blocks that are checked

A block that a program gives back goes on a list of spare blocks of
its size, and the next request of that size gets it. A line that kept
the block's address and writes to it afterwards writes into whatever
the block holds next. In Zeno only `unsafe` can do that, with
`allocate` and `free` ([`unsafe.md`](unsafe.md)), or a library of C:

```zeno
let samples = allocate(u8, 16)
unsafe {
    samples[0] = 7
    free(samples)
    samples[12] = 9     # the block was given back
}
let next = allocate(u8, 16)
print("done")
```

`zeno run` checks every use of a pointer and stops at line 5. A
compiled program does not, and prints `done`. Built with `--memory`
and no collector, it stops where the block is handed out again:

```sh
cargo run -- build --no-collector --memory late.zeno
./late
```

```
error: a block of memory was written to after it was given back
memory: 1 block made (16 bytes), at most 16 bytes in use at once, no collector, 0 blocks left at the end
pool: 24 bytes taken, in a pool of 32768
text: at most 0 bytes in use at once, in a room of 1024
```

The heap checkers of C (valgrind, the address sanitizer) cannot see
inside the pool, because to them it is one array. This stands in for
them. It is two things:

1. **A block that is given back is filled** with the byte `0xA5`,
   after the link to the next spare block.
2. **When it is handed out again**, every one of those bytes must
   still be `0xA5`, and the link must be the end of the list or a
   spare block of the same size in the pool. Anything else stops the
   program. The link is looked at before it is followed, so a write
   over it is caught before it can hand out a block that is not there.

It is only with `--memory`, which is how a program is tried and not
how it is shipped, and only with no collector, where the pool is.
With `--memory` the C has the two functions and the two calls behind
`#ifdef ZENO_MEMORY_REPORT`, so the same file built without that flag
has none of it. Without `--memory` the C is what it was before 0.8.0.
A minimal build leaves it out, as it leaves out every check.

## What it costs

Nothing without `--memory`. With it, a block that is given back is
written once more, and read once more when it is handed out again:
work in proportion to the size of the block, on top of the counting
that `--memory` does already. No block is bigger, and the report is
the same.

## In the C

`runtime.c` is not changed, since it is written whole into the C of
every program. A build with `--memory` and no collector gets the
runtime with these in it:

```c
static void zeno_spare_fill(zeno_spare *block) {
    unsigned char *end = (unsigned char *)block + sizeof(zeno_head) + ((size_t)16 << block->head.step);
    for (unsigned char *byte = (unsigned char *)(block + 1); byte < end; byte++) *byte = ZENO_SPARE_BYTE;
}
```

called in `zeno_free` as the block goes on its list, and
`zeno_spare_check` called in `zeno_alloc_raw` as one comes off. The
LLVM build writes the same runtime beside its IR.

## How it is tested

- **The options**: `the_pool_and_the_text_room_are_sized_on_the_command_line`
  takes both ways of writing each out of the command line, checks the
  flags they make, and each refusal of a size.
- **The size, exactly**: `with_no_collector_the_pool_has_an_end` runs
  a program in a pool of the size its report gives and not in one a
  byte smaller, with the flags `--pool` makes.
- **The check**: `with_memory_a_write_to_a_block_given_back_stops_the_program`
  writes into a block and into its link after `free`, through C and
  LLVM. Both stop with the message. The same C built without
  `--memory` runs to the end. The C differs from the plain one only by
  the check, and a program that gives back hundreds of blocks and
  never writes to them runs as `zeno run` runs it, with 0 blocks left.

## Things to try

1. Build `examples/buffer.zeno` with `--no-collector --memory`, read
   the pool line, and build it again with `--pool` set to that
   number. Then one less.
2. Build `readings.zeno` with `--text-room 4`. Which statement stops,
   and why does it need more than 4 bytes?
3. Change `samples[12]` in `late.zeno` to `samples[0]`. Which of the
   two tests catches it now? Then free a second block before the
   first and try again.
4. Why is the byte `0xA5` and not `0`? What would a fill of zeros
   miss?
5. The check runs when a block is handed out again. A block that is
   never handed out again is never checked. Add a pass over every
   spare list to `zeno_memory_report`, at the end of the program, so
   that those are checked too.
6. Build `late.zeno` with `--owned --memory`. The pool grows there.
   What does the check of the link look for when the pool is many
   pieces?
