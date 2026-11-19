# A compiler that is neither GCC nor Clang

`zeno build` writes C, and a C compiler makes the program of it. On a
computer that compiler is GCC or Clang. For a chip it is often
`arm-none-eabi-gcc`, which is GCC, and sometimes it is not: IAR's
compiler, Keil's older one, the compiler a chip maker ships.

The C that Zeno writes is C as the standard of 1999 has it, and every
one of them reads that. A few things in it are not in that standard,
and are asked of GCC and Clang by their own names:

| what | asked of GCC and Clang as | in plain C |
|---|---|---|
| did `a + b`, `a - b`, `a * b` overflow | `__builtin_add_overflow` and its like: one instruction | worked out from the signs, and from halves of 32 bits |
| a function never comes back | `__attribute__((noreturn))` | `_Noreturn` in the C of 2011, nothing before it |
| keep a slow function out of a quick one | `__attribute__((noinline))` | nothing: it is a hint |
| the collector: the lowest bit of a word | `__builtin_ctzll` | the word halved six times |
| the collector: the registers | `__builtin_unwind_init` | `setjmp` |

**Each is written twice, and the compiler gets the one it can read.**
There is nothing to ask for: the C looks at which compiler is reading
it, in one place at the top of the runtime, and a compiler that is
neither GCC (from version 5) nor Clang gets the plain C.

A program does the same either way. An overflow is an `OverflowError`
at the same line and column, and every result that fits is the same
number.

## Trying it without such a compiler

Building with `-DZENO_PLAIN_C` asks GCC or Clang for the plain C too:

```sh
zeno build primes.zeno                      # writes primes.c, and builds ./primes
cc -DZENO_PLAIN_C -O2 -o primes primes.c -lm
```

and tcc, the Tiny C Compiler, is neither GCC nor Clang and takes it
unasked:

```sh
tcc -o primes primes.c -lm
```

A board says it in its `zeno.board`, if its compiler is a GCC that is
to be given the plain C all the same:

```
flags "-DZENO_PLAIN_C"
```

## What it costs

On a computer, with gcc 13 and Clang 18 at `-O2`, a loop of sums and
products is about a fifth slower in plain C (0.23 s for a hundred
million rounds, where the built-ins take 0.19 s). A processor of 64
bits knows in one instruction that a sum overflowed, and the plain C
has to work it out.

On a 32-bit chip there is nothing to lose: a number of 64 bits is two
words there, the processor has no such instruction for it, and what
GCC writes for its own built-in is about what the plain C says.
For `boards/lm3s6965evb`, an ARM Cortex-M3, in bytes of flash:

| program | with the built-ins | in plain C |
|---|---|---|
| `hello.zeno` | 936 | 932 |
| `scheduler.zeno` | 7,480 | 7,188 |
| `primes.zeno` | 7,884 | 7,780 |
| `modbus.zeno` | 9,720 | 9,596 |
| `tour.zeno` | 14,032 | 14,208 |

## What it has been tried with, and what not

- **tcc 0.9.27** builds every example, every sample of the book and
  the corner programs, with the collector, with `--owned` and with
  `--no-collector`, and each does what it does built by gcc.
- **gcc and Clang, told to take the plain C** and to read it as the C
  of 1999 with nothing added (`-std=c99 -pedantic-errors`), do too.
- **On the emulated Cortex-M3**, built as plain C by
  `arm-none-eabi-gcc`, a program prints the sums that only just fit
  and stops at the first that does not, with the line and column.
- **No program has been built by IAR's, Keil's or a chip maker's
  compiler.** None of them can be run where Zeno is made. If you have
  one, this is the page to hold it to, and
  [`missing.md`](missing.md) says what else has never met hardware.

`python3 tools/plain.py` is the check, and the `plain` workflow runs
it on every change to the C.

## What such a compiler still has to bring

- **The C of 1999.** Keil's older compiler reads it with `--c99`.
- **A few functions of the C library**: `memcpy`, `memset`, `strlen`
  and their like, and the functions of `math` a program calls. A
  board that prints by itself needs no `printf` and no `exit`
  ([`board.md`](board.md)), and a float is made into text with no
  `snprintf` (decision 53).
- **The board's own files.** A start-up file and a linker's file are
  written for one compiler. The two board folders that come with Zeno
  are for `arm-none-eabi-gcc`; a folder for another compiler has that
  compiler's files in it ([`board-build.md`](board-build.md)).
- **A command `zeno build --board` can type.** It types a command of
  the shape GCC takes (`-O2`, `-I`, `-D`, `-o`, the C files, `-T` and
  the linker's file). For a compiler that takes another shape, name a
  script of your own as the board's `compiler`: it is handed those
  words and can call what it likes. Or take the `.c` file that the
  build wrote and add it to the vendor's project. Neither has been
  tried.

## Two limits

- **A firmware build with a collector** (`zeno build --firmware` on a
  computer) cannot be built in plain C. The collector reads the
  processor's registers, plain C can only do that with `setjmp`, and
  a firmware build has no `setjmp` in it at all
  ([`firmware.md`](firmware.md)). The C says so in one line and stops
  the compiler. `--no-collector` and `--owned` have no collector, and
  a build for a board is always one of those.
- **Windows.** One function there runs before `main` by an attribute
  of GCC's, and the compilers tried on Windows are gcc and Clang.
  Microsoft's compiler is not one Zeno builds with yet.

## The files

- `src/runtime.c`: "The C compiler", at the top, decides
  `ZENO_PLAIN_C` and the words `ZENO_NORETURN` and `ZENO_NOINLINE`.
  "Whole numbers" has the four functions that find an overflow, each
  in both forms. The collector has `zeno_lowest_bit` and
  `zeno_mark_running`.
- `src/emit_c.rs` and `src/minimal.rs`: in a build with a collector
  the top of the program must stay a function of its own. GCC and
  Clang are told so with `ZENO_NOINLINE`, and in plain C `main` calls
  it by its address, so that no compiler can put it inside `main`.
- `tools/plain.py`: the check.
- Decisions chapter 55 has the reason for each choice.

## Things to try

1. Build `examples/primes.zeno`, then compile `primes.c` again with
   `-DZENO_PLAIN_C`. Do the two programs print the same?
2. Write `print(9223372036854775807 + int(input()))` and type 1.
   Build it both ways. What does each say, and where?
3. Find `zeno_mul_overflows` in the C. Work `3037000500 * 3037000500`
   through the plain C by hand: which line says it is too big?
4. Why is the result of a sum worked out twice in the plain C, once
   with no sign and once as `a + b`?
5. Install tcc and build `examples/words.zeno` with it. Then try
   `zeno build --firmware` and tcc. What does the C say?
