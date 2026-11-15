# Memory by hand: `unsafe { }`, pointers and registers

Zeno already looks after memory in two ways. An ordinary build has a
collector, and a build with `--no-collector` gives every value one
owner that frees it. This page adds the third way, the one C has: ask
for memory, use it through a pointer, and give it back yourself. It
also adds what firmware needs most of all, a register at a fixed
address.

All three live in one program. A list is still looked after for you.
Only what you asked for with `allocate` is yours to free.

```sh
cargo run -- run examples/driver.zeno
```

```
register 0x40004408 = 0x0068
register 0x40004400 = 0x00000009
waiting: 4 refused: 2
checksum of all four: 156
checksum of the last three: 66
register 0x4000440c = 0x5a
register 0x4000440c = 0x65
register 0x4000440c = 0x6e
register 0x4000440c = 0x6f
sent: 4
register 0x4000440c = 0x21
register 0x4000440c = 0x0a
sent: 2
register 0x40004400 = 0x00000000
Register(0x40004400) Register(0x4000440c) <pointer[u8]>
```

[`examples/driver.zeno`](../examples/driver.zeno) is a driver for a
made-up serial port. Bytes wait in a queue whose memory the program
asks for and gives back by hand, and they go out through three
registers. `zeno run`, the VM, and all six kinds of compiled build
print these fifteen lines.

## The files

- [`src/token.rs`](../src/token.rs) and
  [`src/parser.rs`](../src/parser.rs): the word `unsafe` and the
  statement `StmtKind::Unsafe`.
- [`src/checker.rs`](../src/checker.rs): `Pointee`, `Ty::Pointer`,
  `Ty::Register`, `needs_unsafe`, and the built-ins `allocate`, `free`
  and `register`.
- [`src/value.rs`](../src/value.rs): `Memory`, the block that
  `zeno run` keeps behind a pointer.
- [`src/ops.rs`](../src/ops.rs): `pointed_at`, which checks every use.
- [`src/builtins.rs`](../src/builtins.rs) and
  [`src/board.rs`](../src/board.rs): `allocate`, `free`, and the
  registers of the simulated board.
- [`src/emit_c.rs`](../src/emit_c.rs) and
  [`src/emit_llvm.rs`](../src/emit_llvm.rs): a pointer as C's pointer
  and as IR's `ptr`.
- [`src/runtime.c`](../src/runtime.c): `zeno_allocate`, `zeno_release`,
  `zeno_register_read` and `zeno_register_write`.

## The block

```zeno
let bytes = allocate(u8, 16)

unsafe {
    bytes[0] = 0x41
    print(bytes[0])
    free(bytes)
}
```

`unsafe { }` is a block like the body of an `if`. Inside it a program
may do five things that it may not do anywhere else:

| Inside `unsafe { }` only | Why Zeno cannot check it |
|---|---|
| read through a pointer, `p[i]` | a pointer does not know how long its memory is, or whether it was freed |
| write through a pointer, `p[i] = v` | the same, and a wrong write damages something else |
| `free(p)` | nothing says whether `p` is still used somewhere |
| read a register, `r.read()` | the address is taken on trust, and reading can change the hardware |
| write a register, `r.write(v)` | the same |

Outside the block the checker says so, and says why:

```
error: reading through a pointer goes inside `unsafe { }`. Zeno cannot check that it is right, so the block marks the lines where a mistake can damage memory
 --> test.zeno:2:7
  |
2 | print(bytes[0])
  |       ^
```

The block does not turn anything off. Types are still checked inside
it, numbers still report overflow, and a list is still looked after.
It only allows those five things. So when a program damages memory,
the lines to read are the ones inside the blocks, and
`grep -n unsafe` finds them all.

Everything else about a pointer is safe, and needs no block:

```zeno
let bytes = allocate(u8, 16)          # asking for memory
let second = bytes + 1                # working out an address
let same = second - 1 == bytes        # comparing two
let all = [bytes, second]             # keeping one in a list or a field
print(bytes)                          # <pointer[u8]>
```

None of these can damage anything. An address that points nowhere is
harmless until somebody reads or writes through it.

Three more rules, each chosen so that the block means one thing:

- `unsafe` is a reserved word. It is never a name.
- The block is a statement. It gives no value, so
  `let x = unsafe { p[0] }` is refused, with what to write instead.
- A function written inside the block starts safe again. It is a new
  function that may be called from anywhere, so its own body needs its
  own block.

## Pointers

A pointer type says what one item is: `pointer[u8]` points at bytes.
The item is a number of one size:

| Item | Size of one |
|---|---|
| `u8`, `i8` | 1 byte |
| `u16`, `i16` | 2 bytes |
| `u32`, `i32` | 4 bytes |
| `int`, `float` | 8 bytes |

`allocate(u8, 16)` takes the item first and then how many. It gives a
`pointer[u8]` to sixteen bytes that are all zero. Asking for fewer than
one item stops the program, in `zeno run` and in a compiled program
alike.

`p[i]` is item `i`, counted from 0. `p + 1` is the address of the next
**item**, not of the next byte, as in C: on a `pointer[u32]` it moves
four bytes on.

What is written through a pointer follows the rules of the sized
numbers ([`sized-ints.md`](sized-ints.md)):

```zeno
unsafe {
    bytes[0] = 200                  # a number written out is checked as you compile
    bytes[1] = u8(bytes[0] + 1)     # arithmetic gives an int, so say `u8(...)`, which checks that it fits
}
```

`free(p)` gives the memory back. It wants the pointer that `allocate`
gave, not one worked out from it.

## What `zeno run` checks, and a compiled program does not

A compiled program holds an address and nothing more, as a C program
does. `zeno run` holds more: behind each pointer it keeps the block,
how long it is and whether it was freed (`Memory` in `value.rs`). So it
can catch the four classic mistakes at the line that makes them.

```zeno
let a = allocate(u8, 16)
let b = allocate(u8, 16)
unsafe {
    a[24] = 7          # a has items 0 to 15
    print(b[0])
}
```

```
error: this writes over item 24 of a block that has 16 (0 to 15). `zeno run` checks every use of a pointer. A compiled program does not: it writes over whatever is beside the block
 --> past.zeno:4:5
  |
4 |     a[24] = 7          # a has items 0 to 15
  |     ^
```

Built with `--no-collector`, the same program runs to its end and
prints `7`. In the pool `b` lies right after `a`, with eight bytes of
the pool's own notes between them, so item 24 of `a` is item 0 of `b`.
Nothing went wrong that the machine could see. That is the whole
danger, and the reason for running a program under `zeno run` first.

The other three:

| The mistake | What `zeno run` says |
|---|---|
| using memory after `free` | `this reads memory that was freed. ...` |
| `free` twice | `this memory was freed already. ...` |
| `free(p + 1)` | `` `free` needs the pointer that `allocate` gave, and this one points 1 item further on. ... `` |

Each message ends by saying what a compiled program would have done
instead.

## Registers

On a microcontroller the hardware is controlled through registers:
places in memory at addresses the data sheet gives. `register` comes
from the `board` module, like `pin`:

```zeno
from board import register

let control = register(u32, 0x4000_4400)

unsafe {
    control.write(u32(control.read() | 1 << 3))
}
```

`register(u32, 0x4000_4400)` takes the size first and then the
address, in the same order as `allocate`. The size is `u8`, `u16` or
`u32`, and the type is `Register[u32]`. Making a register and passing
it around is safe. `read()` and `write(v)` go inside the block.
`control.address` gives the address back, and `print(control)` shows
`Register(0x40004400)`.

On a computer there is no hardware at that address, so the simulated
board remembers what was last written to each register and prints
every write:

```
register 0x40004400 = 0x00000009
```

A register nobody has written to reads as 0. Every write prints, also
one that changes nothing, because on real hardware a write is an
event: the same byte written twice to a serial port is two bytes sent.

On a real board (built with `-DZENO_BOARD`, see
[`board.md`](board.md)) a write is one line of C:

```c
static void zeno_register_write(int64_t address, int bits, int64_t value) {
    switch (bits) {
    case 8: *(volatile uint8_t *)(uintptr_t)address = (uint8_t)value; break;
    case 16: *(volatile uint16_t *)(uintptr_t)address = (uint16_t)value; break;
    default: *(volatile uint32_t *)(uintptr_t)address = (uint32_t)value; break;
    }
}
```

`volatile` tells the C compiler that somebody else is watching this
memory: it may not skip the access, repeat it or move it. `bits` is
always a number written out in the call, so the compiler keeps one
case and throws the `switch` away. A board that reaches its registers
some other way defines `ZENO_BOARD_REGISTERS` in its header and brings
its own two functions.

## The driver, piece by piece

The queue keeps its bytes behind a pointer, with the count beside it,
because a pointer does not know how many items it points at:

```zeno
class Queue {
    bytes: pointer[u8]    # `size` bytes, asked for with `allocate`
    size: int
    first: int            # where the oldest byte is
    count: int            # how many are waiting

    function put(self, byte: u8) -> bool {
        if self.count == self.size {
            return false
        }
        let at = (self.first + self.count) % self.size
        unsafe { self.bytes[at] = byte }
        self.count += 1
        return true
    }
```

The `unsafe` block is one line long, and the line before it is what
makes it right: `at` is always less than `size`. This is how to use
the block. Keep it small, and keep the reason next to it. A caller of
`put` cannot get it wrong, because the caller never sees the pointer.

`get` gives `u8 or none`, so an empty queue is an answer and not an
accident. `close` frees the memory:

```zeno
    function close(self) {
        unsafe { free(self.bytes) }
        self.size = 0
        self.count = 0
    }
```

Setting `size` to 0 afterwards makes every later `put` answer `false`
before it reaches the freed memory. Nothing forces that line to be
there. It is the kind of care that memory by hand asks for.

`checksum` shows a pointer as a parameter, and an address worked out
for it:

```zeno
print("checksum of all four:", checksum(queue.bytes, 4))
print("checksum of the last three:", checksum(queue.bytes + 1, 3))
```

## What it compiles to

A pointer is C's own pointer, `[ ]` is C's own `[ ]`, and the block is
a pair of braces:

```c
struct z_Queue {
    uint8_t *z_bytes;
    int64_t z_size;
    int64_t z_first;
    int64_t z_count;
};
```

```c
static bool z_Queue_put(z_Queue *z_self, int64_t z2_byte) {
    if (z_self->z_count == z_self->z_size) {
        return false;
    }
    int64_t z_at = zeno_mod(zeno_add(z_self->z_first, z_self->z_count, 37, 19), z_self->z_size, 37, 19);
    {
        int64_t t3 = z2_byte;
        z_self->z_bytes[z_at] = (uint8_t)t3;
    }
    z_self->z_count = zeno_add(z_self->z_count, 1, 39, 9);
    return true;
}
```

There is no check on `z_at` in the C. The arithmetic around it is
still checked (`zeno_add`), because the block only allows the five
things and switches nothing off.

`allocate` says how many items and the size of one, and where in the
program it is, for the one message it can give:

```c
static z_Queue *z_open_queue(int64_t z_size) {
    return zeno_new_Queue(((uint8_t *)zeno_allocate(z_size, sizeof(uint8_t), 63, 18)), z_size, 0, 0);
}
```

A register is its address, an `int64_t`, and its size goes with each
call:

```c
static void z_start(int64_t z_ticks) {
    {
        zeno_register_write(z_divider, 16, z_ticks);
        int64_t t5 = z_control;
        int64_t t4 = zeno_register_read(z_control, 32);
        zeno_register_write(t5, 32, zeno_fit(((t4 | z_PORT_ON) | z_SENDER_ON), INT64_C(0), INT64_C(4294967295), "a u32", 70, 23));
    }
}
```

### Through LLVM

In IR a pointer is `ptr`. An item is found with `getelementptr` at the
size of the item, and is made 64 bits wide again as it is read (`zext`
for an unsigned item, `sext` for a signed one). This is the loop of a
function that adds up bytes:

```llvm
do2:
  %t3 = load i64, ptr %var.total
  %t4 = getelementptr i8, ptr %arg.bytes, i64 %t1
  %t5 = load i8, ptr %t4
  %t6 = zext i8 %t5 to i64
```

(The first line is the `total` of `total += bytes[i]`, which is read
before the byte is.)

The block is a label going in and a label coming out (`unsafe1:` and
`safe1:`), which changes nothing in the machine code and makes the
blocks easy to find in the `.ll` file.

## Where the memory comes from

```sh
cargo run -- build --memory examples/driver.zeno && ./driver
```

```
memory: 11 blocks made (161 bytes), at most 104 bytes in use at once, 0 collections, 2 blocks freed without the collector
by hand: 1 block from `allocate`, 0 never freed
```

In an ordinary build and in `--firmware`, `allocate` takes memory from
the heap with C's `calloc`. The collector never looks at it and never
frees it. The report has a line of its own for it, and the second
number is the one to watch: memory that was asked for and never given
back.

```sh
cargo run -- build --no-collector --memory examples/driver.zeno && ./driver
```

```
memory: 4 blocks made (108 bytes), at most 108 bytes in use at once, no collector, 0 blocks left at the end
pool: 176 bytes taken, in a pool of 32768
text: at most 42 bytes in use at once, in a room of 1024
```

With no collector there is no heap either ([`pool.md`](pool.md)), so
`allocate` takes from the pool like everything else, and a block that
was never freed shows up as a block left at the end. The pool also
notices a second `free` of the same block and stops the program. That
is the pool's own check, and the only one a compiled program has.

A queue object has one owner in this build, which frees the object.
It does **not** free what the object's pointer points at. That memory
was asked for by hand, so it is given back by hand: `queue.close()`.

## What it does not do

- A pointer points at numbers. There is no pointer to a bool, an
  object, a list, or another pointer, and no way to take the address
  of a variable.
- Two pointers can be compared with `==` only. There is no `<`, and no
  `p - q`.
- A pointer of one type cannot be read as another: a `pointer[u8]` is
  not a `pointer[u32]`.
- A register is 8, 16 or 32 bits wide.
- A compiled program checks nothing but the count given to `allocate`.
  That is the point of the block, and the reason `zeno run` checks
  everything.

## How it is tested

- `checker.rs`: `what_zeno_cannot_check_goes_inside_unsafe` (each of
  the five, a parameter with no type written, a function inside the
  block) and `pointers_and_registers_have_types` (twenty-six
  mistakes and their messages).
- `parser.rs`: `unsafe_is_a_block_and_a_word_of_its_own`.
- `interpreter.rs`, which runs each program through the VM as well:
  `memory_by_hand`, `zeno_run_checks_every_use_of_a_pointer`,
  `registers_on_the_simulated_board`, `the_driver_example_runs`.
- `emit_c.rs`: `memory_by_hand_is_the_same_in_every_build` runs one
  program with every kind of item, a pointer in a field, in a list and
  where `none` may be, through C and LLVM in all three kinds of build,
  with the collector run before every allocation of its own, and
  expects the pool to end with no blocks left.
  `leaving_from_inside_an_unsafe_block_frees_what_has_an_owner`,
  `the_memory_report_counts_what_was_made_by_hand` and
  `the_c_for_memory_by_hand` cover the rest.
- `emit_llvm.rs`: `the_ir_for_memory_by_hand` and the same report test.

## Things to try

1. In `driver.zeno`, change `open_queue(4)` to `open_queue(6)`. How many
   bytes are refused now, and what does the first checksum become?
2. Delete the line `queue.close()`, build with `--memory` and then with
   `--no-collector --memory`. Each report says the same thing in its
   own words. Which words?
3. Add `queue.put(1)` after `queue.close()`. Nothing bad happens. Now
   delete `self.size = 0` from `close` and run it again with
   `zeno run`. Which line is reported, and why that one and not the
   `put` you added?
4. Take the `unsafe { }` away from around `data.write(byte)` in
   `flush`. Read the message. Then put the block around the whole
   `while` loop instead. It compiles, so why is the small block the
   better one?
5. Write `function fill(bytes: pointer[u8], count: int, value: u8)` and
   use it to set a block of 32 bytes to `0xFF`. Then write
   `function copy(from: pointer[u8], to: pointer[u8], count: int)`.
6. Write the two programs from "What `zeno run` checks" and run them
   both ways. Then change `a[24]` to `a[20]`. `zeno run` still stops.
   What does the build with no collector do now, and what did that
   write land on?
7. Allocate a `pointer[u32]` of two items and print `p + 1 == p`.
   Open the `.c` file and find the `+ 1`. How many bytes does C move?
8. Open `driver.c` and find `zeno_register_write`. It is the simulated
   one. Read the one above it, for a real board, and say what
   `volatile` stops the compiler from doing to two writes of the same
   value in a row.
9. A bigger one: write a `class Arena` that asks for 256 bytes once and
   hands out pieces of it with `claim(count) -> pointer[u8] or none`,
   keeping a `used` count. Nothing is freed until `close()`. This is
   how much firmware gets by with no `free` at all.
