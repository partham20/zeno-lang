# One step to a board: `zeno build --board`

```sh
zeno build --board boards/qemu-cortex-m3 examples/blink.zeno
```

```
Wrote blink.c
Built blink.elf for the board in `boards/qemu-cortex-m3`.
Run it with: qemu-system-arm -M lm3s6965evb -nographic -semihosting -kernel blink.elf
```

`blink.elf` is a program for an ARM Cortex-M3: the code for its flash,
with the table of vectors at the front. One command made it from a
`.zeno` file. This page says what that command does, what a board's
folder holds, how to try it with no hardware, and how to write the
folder for a board of your own. The reasons are in chapter 51 of
[`decisions.md`](decisions.md).

Until now `zeno build` wrote C and compiled it for the computer it ran
on. Building the same C for a chip was a second command, typed by
hand, with the chip's compiler, a dozen flags, a startup file and a
linker script on it ([`board.md`](board.md)). Those four things are
the same for every program on one board. So they are written down
once, in a folder, and `--board` names the folder.

## What the command does

1. It reads `zeno.board` in the folder, and says what is wrong with it
   before anything else happens.
2. It checks the program and writes its C, exactly as
   `zeno build --no-collector` does: the C for a chip, with a pool of
   a fixed size and no garbage collector
   ([`no-collector.md`](no-collector.md)). `--board` changes nothing
   in the C. It changes who compiles it.
3. It runs the board's compiler once, on the program's C and the
   board's own files, with the board's flags and linker script. Out
   comes `<name>.elf`.
4. It shows the command that runs the program, if the board gives one.

`--minimal`, `--release`, `--fast` and `--memory` go with it and mean
what they always mean. `--collector`, `--llvm`, `--web` and `--target`
are refused, each with its reason.

## A board is a folder

```
boards/qemu-cortex-m3/
    zeno.board     what the compiler is asked
    startup.c      what runs before `main`, and the clock
    system.c       what the C library asks of a system
    link.ld        where the chip's flash and RAM are
```

`zeno.board` is read as a `zeno.package` is: a `#` starts a comment,
and text is in quotes. This is the whole file of the emulated board,
without its comments:

```
about "An ARM Cortex-M3 that QEMU emulates (its machine lm3s6965evb)"

compiler "arm-none-eabi-gcc"
flags "-mcpu=cortex-m3" "-mthumb"
flags "-ffunction-sections" "-fdata-sections"
flags "-D_POSIX_TIMERS=1" "-D_POSIX_MONOTONIC_CLOCK=1"
sources "startup.c" "system.c"
script "link.ld"
link "-nostartfiles" "-Wl,--gc-sections"
run "qemu-system-arm" "-M" "lm3s6965evb" "-nographic" "-semihosting" "-kernel"
```

| Line | What it says | How often |
|---|---|---|
| `compiler "arm-none-eabi-gcc"` | The C compiler for the board's processor | once, and it must be there |
| `flags "..." "..."` | What the compiler is told every time: which processor, which floating point | any number of lines |
| `header "board.h"` | The header with the board's own functions ([`board.md`](board.md)). With none, the program gets the simulated board | once at most |
| `sources "startup.c"` | Files of C that are built with every program | any number of lines |
| `script "link.ld"` | The linker script | once at most |
| `link "..."` | What the compiler is told only when it links | any number of lines |
| `run "..." "..."` | The command that runs a built program, or loads it onto the board | once at most |
| `about "..."` | Which board this is, in one line | once at most |

Each word in quotes is one word of the command, so a path with a
space in it needs nothing special. A file (`header`, `sources`,
`script`) is named as it is in the board's folder, wherever the
command is typed. A file that is named and is not there is said before
anything is built.

From that file and `blink.zeno`, the compiler is asked this:

```sh
arm-none-eabi-gcc -O2 \
    -mcpu=cortex-m3 -mthumb -ffunction-sections -fdata-sections \
    -D_POSIX_TIMERS=1 -D_POSIX_MONOTONIC_CLOCK=1 \
    -Iboards/qemu-cortex-m3 \
    -o blink.elf blink.c \
    boards/qemu-cortex-m3/startup.c boards/qemu-cortex-m3/system.c \
    -T boards/qemu-cortex-m3/link.ld \
    -nostartfiles -Wl,--gc-sections \
    -I. -lm
```

The order gives the board the last word. Zeno's own flag comes first
(`-O2`, or `-Os` with `--minimal`), so a board that says `flags "-Os"`
gets `-Os`. The board's folder is on the path of headers (`-I`), so
its header and its sources can include what lies beside them. With a
`header` line, `-DZENO_BOARD="board.h"` is there too. What the program
itself brings comes last: the libraries of C it declares with
`import c` ([`c.md`](c.md)), and `-lm`.

The last word of the `run` line's command is the program. A tool that
wants it elsewhere says where with `{program}`:

```
run "openocd" "-f" "board/st_nucleo_f1.cfg" "-c" "program {program} verify reset exit"
```

`zeno build` shows that command and does not run it. Loading a
program onto a board changes the board, and that stays a step you
take.

## Try it with no hardware

QEMU emulates a whole board with a Cortex-M3 on it, the Stellaris
LM3S6965 evaluation board: 256 KB of flash, 64 KB of RAM, serial
ports, timers. Zeno's folder for it is
[`boards/qemu-cortex-m3`](../boards/qemu-cortex-m3). It needs the
compiler for ARM chips and QEMU. On Debian and Ubuntu:

```sh
sudo apt-get install gcc-arm-none-eabi libnewlib-arm-none-eabi qemu-system-arm
```

Then, in a copy of Zeno's repository, or in the folder an archive of a
release unpacks to, which has `boards/` since 0.7.0 (`install.sh` puts
it in `~/.zeno/boards`; the setup program for Windows and the packages
of apt, pacman, snap, flatpak and Homebrew do not carry it yet):

```sh
zeno build --board boards/qemu-cortex-m3 examples/signal.zeno
qemu-system-arm -M lm3s6965evb -nographic -semihosting -kernel signal.elf
```

```
pin 2 on
led on
pin 2 off
pin 4 on
led off
...
```

(Ctrl-A, then X, leaves QEMU when a program runs for ever.)

This board has no `header` line, so its pins are the simulated ones: a
line of text for each change, as under `zeno run`
([`board.md`](board.md)). The text comes out of the chip's first
serial port, which QEMU joins to the terminal. Everything else is
real: the code is ARM code, an `int` of the program is 64 bits on a
processor of 32, there is no floating point in the processor, the
clock is the chip's SysTick timer counting milliseconds from an
interrupt, and the memory is 64 KB.

What the three files of C and the script do:

- **`link.ld`** says where the flash and the RAM are, puts the table
  of vectors first in the flash, and gives names to the ends of the
  variables so that the other two files can find them.
- **`startup.c`** is what the processor runs first. It copies the
  first values of the variables from flash to RAM, clears the rest,
  starts the timer, and calls `main`. When `main` returns it calls
  `exit`.
- **`system.c`** has the few functions the C library (newlib) leaves
  to a system: write these bytes (to the serial port), read a byte
  (from it), give `malloc` more memory, and stop. On this board
  "stop" tells the debugger that holds the chip, which is QEMU, and
  QEMU ends with the program's own answer: 0, or 1 after an error
  nothing caught. The message of such an error goes to the debugger
  too, and QEMU writes it to the terminal's error stream.

How much of the chip a program takes, in bytes, as
`python3 tools/board.py --sizes` measures it:

| program | flash | flash, `--minimal` | RAM | RAM, `--minimal` |
|---|---|---|---|---|
| `hello.zeno` | 9,472 | 9,432 | 1,756 | 1,756 |
| `buffer.zeno` | 15,888 | 13,776 | 35,700 | 35,692 |
| `primes.zeno` | 16,172 | 14,876 | 35,684 | 2,800 |
| `thermostat.zeno` | 17,760 | 15,516 | 35,704 | 35,676 |
| `tour.zeno` | 22,000 | 19,648 | 35,724 | 35,716 |
| `blink.zeno` | 29,984 | 29,904 | 2,184 | 2,184 |
| `signal.zeno` | 31,016 | 30,800 | 2,240 | 2,240 |
| `doorbell.zeno` | 32,280 | 31,464 | 36,144 | 3,260 |
| `tachometer.zeno` | 33,488 | 32,144 | 36,896 | 4,012 |

A program that makes a list or an object, or builds text, has the pool
and the room for text in its RAM, about 34 KB together, whose sizes
are fixed when the program is built ([`pool.md`](pool.md)). Since
0.8.0 `--pool` and `--text-room` choose them, as in `zeno build --board
boards/qemu-cortex-m3 --pool 4096 examples/primes.zeno`
([`pool-size.md`](pool-size.md)). A program with neither, like `blink.zeno`, takes 2 KB. Most of the flash of a
small program on this board is the C library: its streams are 8 KB of
`hello.zeno`, and `printf` is 20 KB more for a program that drives a
simulated pin (`blink.zeno` and the three after it: a simulated pin
prints with `printf`). Zeno's own `print` does not call `printf`, in
the minimal build for a board either, and a number with a fraction is
made into text by the runtime itself (decision 53): `primes.zeno`,
which shows one, was 33,032 bytes when `snprintf` did that.

## The same chip, on its own registers

`boards/qemu-cortex-m3` leans on two things a real board may not
have: a whole C library under the program, and pins that are only
text. [`boards/lm3s6965evb`](../boards/lm3s6965evb) is the same chip
without either. It has a `header` line, and its `board.h` is the board
layer written for real ([`board.md`](board.md)):

```sh
zeno build --board boards/lm3s6965evb examples/hello.zeno
qemu-system-arm -M lm3s6965evb -nographic -semihosting -kernel hello.elf
```

- **A pin is a bit of one of the chip's seven ports.** Pins 0 to 7 are
  PA0 to PA7, and so on to 48 to 55 for port G. `led` is PF0, where
  the board's LED is. `pin(3).on()` gives port A its clock, makes PA3
  an output and writes its bit, through the registers the data sheet
  names.
- **The clock is the SysTick timer**, and `sleep` puts the processor
  to sleep between its ticks.
- **`serial(0, ...)` to `serial(2, ...)` are UART0 to UART2.**
- **`print` goes out of UART0, and the header does it**: the line
  `#define ZENO_BOARD_PRINTS` and four functions (decision 52). A
  warning and the message of an error go to the debugger, and so does
  the end of the program. There is no `printf`, no stream and no
  `exit` anywhere in the program, and `startup.c` has no `system.c`
  beside it.

QEMU emulates those registers, so the program runs with no hardware,
and that is the only place this folder has run. **It has never run on
the chip itself.** What QEMU does not emulate (a pull-up, the speed of
a UART, the clocks of the ports) is written from the data sheet and
has been tried nowhere.

What that buys, in bytes of flash and of RAM, for the build that
checks:

| program | flash on `qemu-cortex-m3` | flash on `lm3s6965evb` | RAM on `lm3s6965evb` |
|---|---|---|---|
| `hello.zeno` | 9,472 | 936 | 20 |
| `blink.zeno` | 29,984 | 3,996 | 20 |
| `signal.zeno` | 31,016 | 4,844 | 76 |
| `buffer.zeno` | 15,888 | 7,464 | 33,968 |
| `primes.zeno` | 16,172 | 7,884 | 33,952 |
| `modbus.zeno` | 14,452 | 9,720 | 35,656 |
| `words.zeno` | 18,704 | 10,408 | 33,980 |
| `tour.zeno` | 22,000 | 14,032 | 34,608 |

A program carries of the C library `memcpy`, `memset`, `strlen` and
their like, and the functions of `math` it calls. A number with a
fraction is made into text, and read out of text, by Zeno's own
runtime with whole numbers (decision 53): that is 1 to 4 KB of flash,
where the C library's `snprintf` and `strtod` were 28 KB. `primes.zeno`
shows one such number and was 25,360 bytes on `lm3s6965evb` before.
The sums on floats themselves are the compiler's, some 3 KB on a chip
with no floating point of its own.

## A folder for your own board

1. **Start from the vendor's project.** The compiler, the flags, the
   startup file and the linker script of a chip come with its vendor's
   package (STM32Cube, the nRF SDK, MCUXpresso), or from a project
   that already builds for the board. Copy the startup file and the
   linker script into a new folder, and copy the flags from the
   project's build line into `flags` and `link`.
2. **Say where `print` goes.** Either give the C library its few
   functions, as `system.c` of `boards/qemu-cortex-m3` does, unless
   the vendor's package has them (often a file called `syscalls.c`):
   `print` then goes wherever `_write` sends it. Or let the board
   print by itself, as `board.h` of `boards/lm3s6965evb` does: one
   line and four functions in the header, and no `printf` in the
   program ([`board.md`](board.md)). The second is far smaller.
3. **Write the header** with the board's functions, four for pins and
   a clock and more for what the program uses
   ([`board.md`](board.md), and the pages of each part:
   [`inputs.md`](inputs.md), [`serial.md`](serial.md),
   [`analog.md`](analog.md), [`pwm.md`](pwm.md),
   [`interrupts.md`](interrupts.md)). Name it with `header "board.h"`.
   The vendor's own C can be listed under `sources`.
4. **Say how a program is loaded**, in `run`.
5. **If the compiler is neither GCC nor Clang**, the C still builds:
   it is given its checks in plain C without being asked
   ([`plain-c.md`](plain-c.md)). `zeno build --board` types a command
   of the shape GCC takes, so for a compiler with another shape name a
   script of your own as the `compiler`. That has not been tried.

A `zeno.board` for an ST Nucleo-F103RB would look like this. **It is
a sketch: it has not been run on hardware**, as nothing in Zeno has
yet ([`missing.md`](missing.md)).

```
about "ST Nucleo-F103RB (STM32F103RB, Cortex-M3)"

compiler "arm-none-eabi-gcc"
flags "-mcpu=cortex-m3" "-mthumb" "-DSTM32F103xB" "-Os"
flags "-ffunction-sections" "-fdata-sections"
header "board.h"
sources "startup_stm32f103xb.s" "system_stm32f1xx.c" "syscalls.c"
script "STM32F103RBTx_FLASH.ld"
link "--specs=nano.specs" "-u" "_printf_float" "-Wl,--gc-sections"
run "openocd" "-f" "board/st_nucleo_f1.cfg" "-c" "program {program} verify reset exit"
```

A board need not be a chip. The smallest `zeno.board` is one line,
`compiler "cc"`, and builds the program for the computer itself, with
no collector, as `program.elf`.

## What a board does not have

A program is built for a board with no collector, so it is held to
the rules of [`no-collector.md`](no-collector.md): every list and
object has one owner. The garbage collector takes its memory from the
system 64 KB at a time, which is right for a computer and wrong for a
chip with 64 KB in all, so `--collector` is refused with `--board`.

The emulated board has no files, nobody types words after a program's
name, and a serial port has no end: a program that reads what is
typed until there is no more waits there for ever. Zeno does not
refuse such a program. `files.read` raises its `FileError`, as for a
file that is not there.

A board that prints by itself (`ZENO_BOARD_PRINTS`) has no `printf`
and no `exit` under the program, and no `snprintf` under a float.
`--memory` does not go with such a board yet: the report is written
as the C library ends a program ([`missing.md`](missing.md)). The
text of one statement has 1,024 bytes in a build with no collector,
unless `--text-room` says otherwise
([`pool-size.md`](pool-size.md)), and the largest float written
with decimals, `f"{x:.2f}"` of 1e308, is 312 of them.

## How it is checked

- `cargo test board_build` reads `zeno.board` files, right and wrong.
- `cargo test cortex_m3` builds one program with every part of a board
  in it for the emulated board, as `--board` does, runs it in QEMU and
  compares what it prints with `zeno run`, for the build that checks
  and the minimal one. It skips the run where the compiler for ARM or
  QEMU is not installed.
- `cargo test registers_of_a_cortex_m3` does the same on
  `boards/lm3s6965evb`: pins driven and read back through the chip's
  registers, the clock, a serial port, a line typed to `input`, a
  warning, and an error that stops the program.
- `python3 tools/board.py` builds every example, every sample of the
  book and the corner programs in `tools/targets/` for the computer
  and for both boards, and compares what they do. When this page was
  written, 228 of 228 programs did the same on `qemu-cortex-m3` and
  208 of 208 on `lm3s6965evb` in the build that checks, and 212 of
  212 and 193 of 193 in the minimal one. The rest are left out, and counted: a build with no
  collector refuses them (176), or they need files, words on their
  line, or an end to what is typed, or, for the minimal build, a check
  stops them, or, on the board with pins of its own, they use the
  `board` module (a simulated pin prints a line when it changes, and
  a real one does not). A number with a fraction may differ after its
  fifteenth digit, because `cosh` and its like come from another C
  library on the board.
- The workflow `board` runs all of it on a push that changes the C
  backend, the runtime or a board.

That check found a mistake the first time it ran, which was not about
boards at all: a class whose list field has a default was built wrong
with no collector (decision 51.8). A second processor is a second
opinion on the same C.

## Exercises

1. Build `examples/blink.zeno` for the emulated board with and without
   `--minimal`, and compare the two with `arm-none-eabi-size`. Which
   of text, data and bss changed, and why not the others?
2. Copy `boards/qemu-cortex-m3` and give the copy `flags "-Os"`. Is
   the program smaller than with `--minimal` alone? Now `-O0`: by how
   much does it grow?
3. `link.ld` gives the chip 64 KB of RAM. Change it to 16 KB in your
   copy and build `examples/signal.zeno`. Read what the linker says.
   Which two sizes would you make smaller to fit
   ([`pool.md`](pool.md))?
4. In your copy's `system.c`, make `_write` send the error stream to
   the serial port too. What does a program that stops with an error
   print now, and in what order?
5. Build `examples/primes.zeno` for `boards/lm3s6965evb` and look at
   the largest functions with `arm-none-eabi-nm --size-sort`. Which
   of them make a float into text, and which line of the program
   brings them in? Take that line out and build again.
6. Copy `boards/lm3s6965evb` and make `zeno_board_warn` in its
   `board.h` send its bytes out of UART0, as `zeno_board_print` does.
   That is a board with one wire and no debugger. What is lost?
5. Write a `board.h` for your copy with the four functions, where
   `zeno_board_write` prints all pins as one line of `0` and `1`. Add
   the `header` line and take the two `-D_POSIX` flags out. Why are
   they no longer needed?
6. QEMU's `lm3s6965evb` has real pins too: the LM3S6965's GPIO ports
   are emulated. Its status LED is on pin 0 of port F (data register
   at `0x400253FC`, direction at `0x40025400`, digital enable at
   `0x4002551C`). Make `led.on()` write that register. How can a
   program tell that it worked? (On the real chip the port's clock
   must be switched on first, in the register at `0x400FE108`.)
