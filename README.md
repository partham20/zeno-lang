# Zeno

> Every journey is halfway done. Forever.

Zeno is a small programming language, built from scratch as a learning project.
It is named after Zeno of Elea, whose paradoxes say Achilles can never catch the
tortoise, and after the quantum Zeno effect, where a system watched closely
enough never changes. A fitting name for a compiler that will always be almost finished.

## The idea

Zeno should read like plain English and still be checked like Rust. Two rules shape the syntax:

1. **Words over symbols.** Keywords are whole words you can read aloud: `function`,
   `and`, `or`, `not`, `match`, `case`, `raise`. No `fn`, `&&`, `=>` or `?`.
2. **Punctuation has to earn its place.**

| Kept | Why |
|---|---|
| `{ }` around blocks | You can always see where a block ends, and a stray space can't change what the code does |
| `( )` for calls, `[ ]` for lists and dicts, `,` between items | Everyone already reads these |
| `:` before a type, `->` before a return type | Separates a name from its type at a glance |
| `#` for comments | One character, hard to miss |
| `&`, `|`, `^`, `~`, `<<`, `>>` on whole numbers | How every datasheet writes a register |

| Dropped | Why |
|---|---|
| `;` at the end of a line | The line break already says it |
| `:` after `if`, `for`, `function` | The brace already says it |
| `( )` around conditions | `if x > 0 {` needs no help |
| `&&`, `||`, `!` | `and`, `or`, `not` read better |

Variables say once whether they can change: `let` for a value that is set once,
`var` for one that changes, and plain `x = 2` to update. A misspelt name is an
error, not a new variable.

Zeno does not make you pick a style:

- **Object-oriented and functional.** Classes with methods, and also functions
  as values, closures, `map` / `filter` / `reduce` and comprehensions.
- **Types are optional.** Write `function area(w: float, h: float) -> float`
  or just `function area(w, h)`. Start loose, add types as the program grows.

Underneath:

- **Types, where written, are checked before the program runs.** Where they are left out, they are worked out when possible.
- **Errors are checked.** A function says what it `raises`, and the compiler makes sure someone handles it.
- **No surprise `none`.** A value can be `none` only if its type says `or none`, and you must check before using it.
- **Enums carry data**, and `match` must cover every case.
- **Generics.** `class Ring[T]` is written once and used for bytes, voltages and text. Each use is checked, and compiled as if written by hand for that type.
- **Whole numbers have a size where it matters.** A `u8` field is one byte, a number that does not fit is an error, and wrapping is something you ask for.
- **Memory by hand, where you ask for it.** `allocate`, raw pointers, `free` and registers at fixed addresses, as in C. The lines Zeno cannot check go inside `unsafe { }`, and `zeno run` checks them anyway.
- **Compiled, not interpreted**, so it is fast and small enough for a microcontroller.

Samples, none of them final:

- [`examples/hello.zeno`](examples/hello.zeno): one line.
- [`examples/tour.zeno`](examples/tour.zeno): the main features.
- [`examples/styles.zeno`](examples/styles.zeno): one job written typed, untyped, object-oriented and functional.
- [`examples/primes.zeno`](examples/primes.zeno): no types written, and it compiles to C.
- [`examples/shapes.zeno`](examples/shapes.zeno): classes, enums, lists and `match`, compiled to C.
- [`examples/sensor.zeno`](examples/sensor.zeno): values that may be `none`, errors, and functions as values.
- [`examples/memory.zeno`](examples/memory.zeno): a program that makes far more memory than it keeps, and the collector that gives it back.
- [`examples/buffer.zeno`](examples/buffer.zeno): owned values, which are freed at a line you can point to. It also builds with no collector at all.
- [`examples/labels.zeno`](examples/labels.zeno): text that is built and kept, in a build with no collector.
- [`examples/words.zeno`](examples/words.zeno): a line split into words, with lists that nobody keeps and the statement frees.
- [`examples/scheduler.zeno`](examples/scheduler.zeno): tasks in a list, with no collector and not one `owned`: each list frees its items.
- [`examples/commands.zeno`](examples/commands.zeno): commands read from text, as an enum whose cases carry values, with no collector. A line that is no command is `none`.
- [`examples/handlers.zeno`](examples/handlers.zeno): steps kept as function values and faults kept as errors, with no collector. Each has one owner, which frees it.
- [`examples/driver.zeno`](examples/driver.zeno): a driver for a serial port, with a queue of bytes that is allocated and freed by hand and three registers, inside `unsafe { }`.
- [`examples/sentence.zeno`](examples/sentence.zeno): lines from a sensor, checked and taken apart with `find`, `slice`, `sort`, `join` and `code`, the methods of text and lists.
- [`examples/logbook.zeno`](examples/logbook.zeno): readings typed at the keyboard, kept in a file named on the command line, and read back: `input`, `files` and `program.arguments`.
- [`examples/instruments.zeno`](examples/instruments.zeno): a compass, a thermistor, a level meter, a charging capacitor and a sine table, with the `math` module and angles that carry their unit.
- [`examples/faults.zeno`](examples/faults.zeno): fault codes counted and looked up in a dict: `["mode": "auto"]`, `get`, `in`, `pop` and a loop over the keys.
- [`examples/channels.zeno`](examples/channels.zeno): a dict that holds lists and objects. The readings of each channel are a list under its name, and `for channel, values in readings` is given each key and what is under it.
- [`examples/station`](examples/station/main.zeno): one program in four files. A weather station whose sensors, report and serial port are each a module that `main.zeno` imports.
- [`examples/thermostat`](examples/thermostat/main.zeno): a program that needs a package. Its `zeno.package` says `needs filters in "../filters"`, and [`examples/filters`](examples/filters/main.zeno) is that package: a folder of modules with a `zeno.package` of its own.
- [`examples/ring`](examples/ring/main.zeno): a module that keeps its insides to itself. A queue of a fixed size whose slots and count are `private`, so that only `ring.zeno` can change them. The module has its tests at the end of its file: `zeno test examples/ring` runs them.
- [`examples/ports.zeno`](examples/ports.zeno): an `interface`. A UART and a log in memory are both a `Port`, and the code that sends a report takes either.
- [`examples/todo.zeno`](examples/todo.zeno): a page in a browser. A list of things to do, with [`todo.html`](examples/todo.html) as the page and `import web` to reach it. `zeno build --web` makes the two into one file that a browser opens.
- [`examples/sketch.zeno`](examples/sketch.zeno): a sheet to draw on with the mouse, with [`sketch.html`](examples/sketch.html) as the page. A function asks for the `Event` to learn where the mouse is, draws on a `<canvas>`, sets the style of one element, and keeps the chosen colour for the next visit. Its tests press the mouse with `fire`.
- [`examples/blink.zeno`](examples/blink.zeno): the same language on a microcontroller. It blinks the LED of a simulated board.
- [`examples/signal.zeno`](examples/signal.zeno): a traffic light on three pins, as a state machine. It builds for firmware with no collector.
- [`examples/packet.zeno`](examples/packet.zeno): sensor readings packed into six-byte frames and unpacked again, with `u8`, `u16` and the bit operators.
- [`examples/generics.zeno`](examples/generics.zeno): a ring buffer written once with `class Ring[T]`, and used for bytes, voltages and text.

## One language, every job

The goal is that anything you would reach for C, C++, Python, JavaScript, Java or Rust
to do, you can do in Zeno. That is a long road, and each use case arrives with a
piece of the compiler:

| Use case | What gets it there |
|---|---|
| Scripts, automation, a REPL | Interpreter and bytecode VM (`zeno run`) |
| Microcontrollers and firmware | Compile to C, no heap required |
| Command-line tools, desktop, servers | Native code through C or LLVM |
| Calling existing C libraries | C interop, in both directions |
| Web front ends | Zeno itself built for WebAssembly, and the `web` module (`zeno build --web`) |
| Web back ends, networking | `async` / `await` and a standard library |
| Data and numerics | Fast arrays and operator overloading |
| Mobile apps, games | Native code plus platform bindings |

## Install

**Windows.** Download `zeno-0.4.0-windows-setup.exe` from the
[releases page](https://github.com/partham20/zeno-lang/releases) and
run it. It installs Zeno for you, puts it on the PATH, and offers to
install a C compiler and Git if the machine has none. Windows warns
about it the first time, because it is not signed: "More info", then
"Run anyway".

**macOS and Linux.** One line in a terminal:

```sh
curl -fsSL https://github.com/partham20/zeno-lang/releases/latest/download/install.sh | sh
```

It finds the `zeno` for your machine, checks the download, installs it
in `~/.zeno`, puts it on the PATH, and offers to install a C compiler
and git if there is none. [`docs/install.md`](docs/install.md) says
what both do, line by line, and how to take Zeno off again.

**With a package manager.** Each release has a package for apt,
pacman, snap and flatpak, installed from the file:

```sh
sudo apt install ./zeno_0.4.0_amd64.deb                         # Debian, Ubuntu, Raspberry Pi OS
sudo pacman -U zeno-lang-bin-0.4.0-1-x86_64.pkg.tar.zst         # Arch
sudo snap install --classic --dangerous zeno_0.4.0_amd64.snap
flatpak install --user zeno-0.4.0-x86_64.flatpak
```

The `.deb` is also there for `arm64`, `armhf` (every Raspberry Pi with
a 32-bit system) and `i386`, the snap for `arm64` and `armhf`, and the
flatpak for `aarch64`. apt installs a C compiler and git along with
Zeno if the machine has none.

Homebrew, on macOS and Linux:

```sh
brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
brew install partham20/zeno-lang/zeno
```

For winget there are three files in
[packaging/winget](packaging/winget),
which are what Microsoft's collection of packages takes.
[docs/packages.md](docs/packages.md)
says what each package is, and
[docs/stores.md](docs/stores.md)
what is still to be done before `apt install zeno` and the rest work
with no file: for apt a signed list in the release itself, for pacman
the AUR, for snap the Snap Store, and for winget a pull request.

**By hand.** Each release also has a `zeno` that is already built, as
an archive, for ten kinds of machine:

| Machine | File |
|---|---|
| Linux, x86-64: most PCs and servers | `zeno-0.4.0-linux-x86_64.tar.gz` |
| Linux, ARM64: a Raspberry Pi 3, 4 or 5 with a 64-bit system, ARM servers | `zeno-0.4.0-linux-arm64.tar.gz` |
| Linux, 32-bit ARMv7: a Raspberry Pi 2 or later with a 32-bit system | `zeno-0.4.0-linux-armv7.tar.gz` |
| Linux, 32-bit ARMv6: a Raspberry Pi 1 or Zero | `zeno-0.4.0-linux-armv6.tar.gz` |
| Linux, 32-bit x86 | `zeno-0.4.0-linux-x86.tar.gz` |
| macOS, Apple silicon | `zeno-0.4.0-macos-arm64.tar.gz` |
| macOS, Intel | `zeno-0.4.0-macos-x86_64.tar.gz` |
| Windows, x86-64 | `zeno-0.4.0-windows-x86_64.zip` |
| Windows, 32-bit x86 | `zeno-0.4.0-windows-x86.zip` |
| Windows, ARM64 | `zeno-0.4.0-windows-arm64.zip` |

Unpack it and put `zeno` (`zeno.exe` on Windows) somewhere on your
`PATH`. It is one file and needs nothing else installed. To build web
pages, put `zeno.wasm` from the same folder beside it.

```sh
tar -xzf zeno-0.4.0-linux-x86_64.tar.gz
cd zeno-0.4.0-linux-x86_64
./zeno --version                 # zeno 0.4.0
./zeno run examples/hello.zeno   # Hello, Zeno!
```

On macOS, a program downloaded with a browser is held back because
Apple has not signed it: `xattr -d com.apple.quarantine zeno` lets it
run.

`zeno build` hands the C it writes to a C compiler, so it needs one:
`cc`, `gcc` or `clang`, whichever it finds first, or the one `CC`
names. `zeno build --llvm` needs clang. `zeno fetch` needs git.
Everything else needs nothing. The installers offer to install these.

Or build it from this source, which needs only
[Rust](https://rustup.rs): `cargo build --release` puts `zeno` in
`target/release/`. For web pages, `cargo build --release --target
wasm32-unknown-unknown` builds `zeno.wasm` as well
([`docs/web.md`](docs/web.md)). How a release is made, and what is checked before
one goes out, is in [`docs/release.md`](docs/release.md).

## Status

**Zeno is at version 0.2.** The language is complete as it was set out
for 0.1: everything on the roadmap below is done, and every decision
that was once marked provisional has been read again and settled
([`docs/version-0.1.md`](docs/version-0.1.md), which also lists what
0.1 leaves out). Version 0.2 adds a program in a web page
([`docs/web.md`](docs/web.md)) and installers
([`docs/install.md`](docs/install.md)), and changes nothing that ran
before.

Zeno checks and runs programs, and compiles nearly all of the language to C:
numbers, strings, functions, control flow, classes, lists, enums and `match`,
values that may be `none`, errors (`try` / `except` / `raise`), and functions
used as values. A compiled program prints what `zeno run` prints;
[`docs/compiler.md`](docs/compiler.md) lists the two small exceptions.
It also frees the memory it no longer uses, with a small garbage collector
that is compiled in ([`docs/memory.md`](docs/memory.md)). A value marked
`owned` has one owner and is freed the moment its owner's block ends, without
waiting for the collector ([`docs/ownership.md`](docs/ownership.md)).
`zeno build --firmware` writes C with no `setjmp`, where errors are return
values that every caller tests ([`docs/firmware.md`](docs/firmware.md)).
`zeno build --no-collector` goes one step further and leaves the
collector out of the C altogether
([`docs/no-collector.md`](docs/no-collector.md)). In that build every
list and object has one owner, whoever received it, with no word to
write ([`docs/one-owner.md`](docs/one-owner.md)), and so has an enum
value that carries something and a value that may be `none`
([`docs/enums-and-none.md`](docs/enums-and-none.md)), and so has a
function value and an error
([`docs/functions-and-errors.md`](docs/functions-and-errors.md)). Such a program does
not use the heap either: its lists and objects live in a pool of a
fixed size that is part of the program ([`docs/pool.md`](docs/pool.md)).
Text that such a program keeps is a copy that its keeper frees
([`docs/text.md`](docs/text.md)), and a list that is made and given to
nobody is freed by the statement that made it
([`docs/temporaries.md`](docs/temporaries.md)).

Beside both, a program can look after memory by hand, as C does:
`allocate`, a raw `pointer[u8]`, `free`, and a `Register[u32]` at a
fixed address. The lines that Zeno cannot check go inside `unsafe { }`.
`zeno run` checks every one of them anyway, and a compiled program
does what C does ([`docs/unsafe.md`](docs/unsafe.md)).

`zeno run` has two engines. The interpreter walks the syntax tree. With
`--vm` the program is compiled to bytecode and run by a small virtual
machine, which is two to three times as fast and prints exactly the same
([`docs/vm.md`](docs/vm.md)). Every test program is run on both.

`zeno build --llvm` compiles a second way: to LLVM IR, the input of the
compiler library behind Clang and Rust, which clang turns into a
program ([`docs/llvm.md`](docs/llvm.md)). It compiles what the C backend
compiles, and the tests build every program both ways. It takes the
same flags: `--firmware` (errors as values, no `setjmp` in the IR),
`--no-collector` (owned values freed where their owner ends, no
collector in the program) and `--memory`.

```sh
cargo run -- run examples/tour.zeno             # check a program, then run it
cargo run -- run --vm examples/tour.zeno        # the same, compiled to bytecode and run by the VM
cargo run -- bytecode examples/hello.zeno       # the bytecode, one instruction per line
cargo run -- check examples/tour.zeno           # only check it for mistakes
cargo run -- check --strict examples/tour.zeno  # every type must be known, as firmware needs
cargo run -- check --strict examples/styles.zeno  # passes with no types written: they are worked out
cargo run -- build examples/primes.zeno         # compile to primes.c, then to ./primes
cargo run -- build examples/shapes.zeno         # classes, enums, lists and match, compiled
cargo run -- build examples/sensor.zeno         # none, errors and functions as values, compiled
cargo run -- build --memory examples/memory.zeno  # the program reports its memory use as it ends
cargo run -- build --memory examples/buffer.zeno  # owned values: most blocks are freed without the collector
cargo run -- build --firmware examples/sensor.zeno  # errors as values, no setjmp, for firmware
cargo run -- build --no-collector --memory examples/buffer.zeno  # no garbage collector and no malloc: memory is a fixed pool
cargo run -- build --no-collector --memory examples/labels.zeno  # text that is kept, with no collector: every copy is freed
cargo run -- build --no-collector --memory examples/words.zeno   # split, map, filter and + with no collector: each statement frees what it made
cargo run -- build --no-collector --memory examples/scheduler.zeno  # lists of objects with no collector and no `owned`: each list frees its items
cargo run -- build --no-collector --memory examples/commands.zeno   # enum cases that carry values, and values that may be none, with no collector
cargo run -- build --no-collector --memory examples/handlers.zeno   # function values and kept errors with no collector: each has one owner
cargo run -- run examples/sentence.zeno         # the methods of text and lists: find, slice, sort, join, code
cargo run -- run examples/logbook.zeno bench.txt  # the keyboard, a file and the command line: type readings, then Ctrl-D
cargo run -- run examples/instruments.zeno      # math: atan2, log, exp, round, and `sin(30 deg)`
cargo run -- run examples/faults.zeno           # a dict: fault codes counted and looked up
cargo run -- run examples/station/main.zeno     # a program in four files: `import sensors`, `import drivers.uart`
cargo run -- join examples/station/main.zeno    # the same four files as the one program the compiler sees
cargo run -- run examples/thermostat/main.zeno  # a program that needs a package: `import filters.lowpass`
cargo run -- run examples/ring/main.zeno        # a module with `private` fields and functions
cargo run -- run examples/ports.zeno            # an interface: `class Uart is Port`, and a list of ports
cargo run -- run examples/channels.zeno         # a dict of lists and of objects, and `for key, value in ...`
cargo run -- fetch examples/thermostat          # get the packages a program needs from git (this one needs none fetched)
cargo run -- test examples/ring                 # run the tests written in the files of a folder: `test "name" { assert ... }`
cargo run -- new /tmp/blinker                   # make a new program: a `main.zeno`, a module with a test and a `zeno.package`
cargo run -- new --package /tmp/filters         # make a new package: a folder of modules for other programs to need
cargo run -- format --check examples            # is every example laid out the way `zeno format` lays programs out?
python3 editors/talk.py target/debug/zeno examples/shapes.zeno  # play the editor's part to `zeno lsp`, and print both sides
cargo run -- run examples/driver.zeno           # memory by hand and registers: `zeno run` checks every use of a pointer
cargo run -- build --memory examples/driver.zeno  # the same, compiled: the report counts what `allocate` gave and what was never freed
cargo run -- check --no-collector examples/scheduler.zeno  # check as that build would, without building
cargo run -- build --llvm examples/shapes.zeno  # compile to LLVM IR (shapes.ll), then to ./shapes with clang
cargo run -- build --llvm --firmware examples/sensor.zeno  # the same through LLVM, with errors as values
cargo run -- build --llvm --no-collector --memory examples/buffer.zeno  # through LLVM with no collector: the same memory report as the C build
cargo run -- run examples/blink.zeno            # blink the LED of the simulated board (Ctrl-C stops it)
cargo run -- run examples/todo.zeno             # a program for a web page, on a simulated page
cargo build --release --target wasm32-unknown-unknown   # Zeno itself, built for a browser (once)
cargo run -- build --web examples/todo.zeno     # todo.web.html: the page and the program in one file. Open it in a browser
cargo run -- test examples/sketch.zeno          # a page that draws: its tests press the mouse on the simulated page
cargo run -- playground                         # playground.html: write Zeno programs and run them in a browser, with nothing installed
cargo run -- build --firmware --no-collector examples/signal.zeno  # pins and a clock, built for firmware
cargo run -- build examples/packet.zeno         # u8 and u16 fields, stored in as many bytes in the C struct
cargo run -- build examples/generics.zeno       # a generic class: one C struct for each type it is used with
cargo run -- parse examples/tour.zeno           # the syntax tree the parser builds
cargo run -- tokens examples/hello.zeno         # the tokens the lexer finds
cargo test
```

`from board import led` and `from time import sleep` give a program pins and a
clock. On a computer the board is simulated and prints each pin as it changes.
A compiled program meets the board through four C functions, so a real one
can be put underneath ([`docs/board.md`](docs/board.md)).

`import web` gives a program a web page: `web.element("#count").text = "3"`,
`button.on("click", pressed)`. `zeno build --web` writes one file that a
browser opens, and `zeno run` runs the same program on a simulated page that
prints each change ([`docs/web.md`](docs/web.md)).

`u8`, `u16`, `u32`, `i8`, `i16` and `i32` are whole numbers with a size, and
`&`, `|`, `^`, `~`, `<<` and `>>` work on their bits. Arithmetic always gives
a full `int`, so nothing wraps by accident: `u8(n)` checks that `n` fits, and
`u8.wrap(n)` keeps the low eight bits on purpose
([`docs/sized-ints.md`](docs/sized-ints.md)).

`function first[T](items: list[T]) -> T` and `class Stack[T]` are generic:
written once, with `T` standing for a type. The call does not say what `T`
is. The checker works it out, and makes a copy of the function or class for
each type it is used with, so a `Stack[u8]` is as small and as fast as one
written for bytes by hand ([`docs/generics.md`](docs/generics.md)).

- **Why Zeno is the way it is:** [`docs/decisions.md`](docs/decisions.md) gives the reason for every design choice.
- **How each stage works:** [`docs/lexer.md`](docs/lexer.md), [`docs/parser.md`](docs/parser.md),
  [`docs/checker.md`](docs/checker.md), [`docs/interpreter.md`](docs/interpreter.md),
  [`docs/compiler.md`](docs/compiler.md), [`docs/memory.md`](docs/memory.md),
  [`docs/firmware.md`](docs/firmware.md), [`docs/ownership.md`](docs/ownership.md),
  [`docs/no-collector.md`](docs/no-collector.md), [`docs/pool.md`](docs/pool.md),
  [`docs/text.md`](docs/text.md), [`docs/temporaries.md`](docs/temporaries.md),
  [`docs/one-owner.md`](docs/one-owner.md), [`docs/enums-and-none.md`](docs/enums-and-none.md),
  [`docs/functions-and-errors.md`](docs/functions-and-errors.md),
  [`docs/unsafe.md`](docs/unsafe.md), [`docs/audit.md`](docs/audit.md),
  [`docs/library.md`](docs/library.md),
  [`docs/reading-and-writing.md`](docs/reading-and-writing.md),
  [`docs/math.md`](docs/math.md), [`docs/dict.md`](docs/dict.md),
  [`docs/modules.md`](docs/modules.md), [`docs/packages.md`](docs/packages.md),
  [`docs/format.md`](docs/format.md), [`docs/lsp.md`](docs/lsp.md),
  [`docs/private.md`](docs/private.md), [`docs/interfaces.md`](docs/interfaces.md),
  [`docs/testing.md`](docs/testing.md), [`docs/new.md`](docs/new.md),
  [`docs/gaps.md`](docs/gaps.md), [`docs/version-0.1.md`](docs/version-0.1.md),
  [`docs/release.md`](docs/release.md), [`docs/found-by-the-book.md`](docs/found-by-the-book.md),
  [`docs/vm.md`](docs/vm.md),
  [`docs/llvm.md`](docs/llvm.md), [`docs/board.md`](docs/board.md),
  [`docs/web.md`](docs/web.md), [`docs/playground.md`](docs/playground.md),
  [`docs/json.md`](docs/json.md), [`docs/install.md`](docs/install.md),
  [`docs/sized-ints.md`](docs/sized-ints.md), [`docs/generics.md`](docs/generics.md).

## Roadmap

1. Lexer (done)
2. Parser and syntax tree (done)
3. Tree-walking interpreter, `zeno run` (done)
4. Type checker, `zeno check` (done)
5. Compile to C, `zeno build` (done: numbers, strings, functions, control flow, classes, lists, enums, `match`)
6. Compile the rest (done: errors with `try` / `raise`, `none`, functions as values)
7. Memory management (done: a mark-and-sweep garbage collector in the compiled program)
8. Errors as values, for firmware and MISRA builds (done: `zeno build --firmware`
   compiles `try` / `raise` to status returns and `if`, with no `setjmp`)
9. Owned values and looks, beside the collector (done: `owned` values have
   one owner and are freed where its block ends, and a parameter only
   looks at one unless it says `take`)
10. A firmware build with no collector (done: `zeno build --no-collector`
    gives every list and object one owner, and leaves the collector out
    of the C)
11. Bytecode VM (done: `zeno run --vm` compiles to bytecode and runs it on a
    stack machine; `zeno bytecode` shows the instructions)
12. LLVM backend (done: `zeno build --llvm` writes LLVM IR for the whole
    language the C backend compiles, and builds it with clang)
13. Extras:
    - The `board` and `time` modules (done: pins, `sleep` and `now`, on a
      simulated board, with a real one put underneath by a header)
    - Whole numbers with a size, and bit operators (done: `u8` to `i32`,
      checked and wrapping conversions, `& | ^ ~ << >>`, and small fields
      in compiled objects)
    - Generics (done: `function first[T]` and `class Stack[T]`, with the
      types worked out at each call, and one compiled copy for each set of
      types)
    - `--llvm` together with `--firmware` (done: errors as values in the
      IR, with no `setjmp`, and the same refusals as the C firmware build)
    - `--llvm` together with `--no-collector` and `--memory` (done: owned
      values are freed where their owner ends, text is given back at the
      end of its statement, and the memory report matches the C build's
      line for line)
    - A fixed memory pool for `--no-collector` builds (done: no `malloc`,
      `calloc` or `free` in the program; blocks in sizes that double,
      taken and given back in constant time, from one array whose size
      is chosen when the C is compiled)
    - Values owned by whoever receives them, in `--no-collector` builds
      (done. Text that is built can be kept, returned and
      stored in objects and lists, as a copy its keeper frees.
      A list made by `split`, `map`, `filter` or `+`, or written out,
      and an owned value that is only looked at, belong to the
      statement that made them, which frees them when it is over.
      Plain lists and classes have one owner without the word
      `owned`, and a list owns its items, so lists of objects and lists
      of lists work. An enum value that carries something has
      one owner, a list, object or enum value that may be `none` has
      one owner, and `match` looks at it without taking it. A function
      value has one owner, and so has an error, which can be kept and
      raised again)
    - Memory by hand in `unsafe { }` blocks (done: `pointer[T]`,
      `allocate` and `free`, and `Register[T]` at a fixed address, in
      every build and through both backends. `zeno run` checks every
      use of a pointer, and a compiled program checks none, as in C)
    - A pass over the whole language for "unambiguous and intuitive"
      (done: a plain parameter looks and `take` takes, the words of the
      language are never names, the cases after `case none` know the
      value is there, every function can read a `let` at the top of the
      file, a method can take out what its object holds, and a firmware
      build refuses what it cannot do instead of doing something else.
      See [`docs/audit.md`](docs/audit.md))
14. A standard library (in progress)
    - Text and lists (done: `find`, `slice`, `code` and `char` for text;
      `insert`, `pop(at)`, `reverse`, `sort`, `clear`, `find`, `slice`
      and `join` for lists, in every engine and every kind of build.
      See [`docs/library.md`](docs/library.md))
    - Reading and writing (done: `files.read`, `write`, `append`,
      `exists` and `remove`, with `FileError` as a value in every build;
      `text.lines()`; `input()`, which gives a line or `none`; and
      `program.arguments()`.
      See [`docs/reading-and-writing.md`](docs/reading-and-writing.md))
    - More of `math` (done: `tan`, `asin`, `acos`, `atan`, `atan2`,
      `exp`, `log`, `log2`, `log10`, `round` and `e`. `sin(30 deg)`
      takes an angle with its unit, `atan2` gives one in `rad`, and
      `.deg` and `.rad` change between them.
      See [`docs/math.md`](docs/math.md))
    - A dictionary (done: `["Ada": 36]`, `[:]` and the type
      `dict[str, int]`; reading, storing, `in`, `len`, a loop over the
      keys in the order they were put in, `get`, `pop`, `keys`,
      `values`, `clear` and `==`. A key that is not there is a
      `KeyError`. Compiled programs keep it in a hash table, and a
      build with no collector frees it with its owner.
      See [`docs/dict.md`](docs/dict.md))
15. Programs in several files (done: `import shapes` reads
    `shapes.zeno` from the program's folder and `import drivers.uart`
    reads `drivers/uart.zeno`; `shapes.area`, or `from shapes import
    area`. Each file runs once, before the file that imports it, and
    files that import each other are refused. An error names the file
    it is in. The files are joined into one program before it is
    checked, so every engine and every kind of build runs them the
    same. See [`docs/modules.md`](docs/modules.md))
16. A package manager (done: a folder with a `zeno.package` is a
    package. A program's own `zeno.package` says what it needs, in
    plain words: `needs filters in "../filters"` for a folder, or
    `needs units from "git address" at "v1.2.0"` for a repository.
    `import filters.lowpass` then reads `lowpass.zeno` of that
    package. `zeno fetch` gets the repositories with git into
    `packages/` and pins each commit in `zeno.lock`; running a
    program never uses the network.
    See [`docs/packages.md`](docs/packages.md))
17. Editor support
    - A formatter (done: `zeno format file.zeno` lays a file out the
      one way Zeno lays programs out, with no options. It changes
      white space and nothing else: every token and every line break
      stays, so a program cannot come to mean something else, and the
      formatter checks that on every run. `--check` only looks.
      See [`docs/format.md`](docs/format.md))
    - A language server (done: `zeno lsp` speaks the Language Server
      Protocol, so VS Code, Neovim, Helix and others show the mistakes
      in a file as it is typed, from the text in the editor, and lay
      it out on save. They also jump to where a name was declared
      and show its type, from what the checker noted about each name.
      It is the `zeno` program itself, running the checker
      `zeno check` runs. Colours and setup for each editor
      are in [`editors/`](editors/README.md).
      See [`docs/lsp.md`](docs/lsp.md))
18. What real programs turn out to need (done). This was the finish
    line for the language: with this list done, Zeno is version 0.1.
    - `private` (done: `private` in front of a declaration at the top
      of a file, or of a field or a function of a class, keeps it to
      the file it is written in.
      See [`docs/private.md`](docs/private.md))
    - Interfaces (done: `interface Shape { ... }` lists functions, and
      `class Circle is Shape` says that the class has them, which is
      checked at the class. A `Circle` then goes wherever a `Shape`
      is wanted, so one function serves every class that is one. In
      a compiled program it is a small number in the object and a
      `switch`, with no pointer to a function.
      See [`docs/interfaces.md`](docs/interfaces.md))
    - A dict that holds any type of value, and a loop over its pairs
      (done: a value is anything a list can hold, such as a list, an
      object, a value of an interface or another dict, and never
      `none`, which is what `get` says for a key that is not there.
      `for key, value in ages` is given each key and what is under
      it, and over a list the first name is the position. With no
      collector a dict still holds plain values only.
      See [`docs/dict.md`](docs/dict.md))
    - `zeno test`: the tests of a Zeno program, written in Zeno
      (done: `test "what it checks" { ... }` at the top of a file,
      with `assert total == 6` and `assert raises ValueError { ... }`
      inside. A failed `assert` shows what was on each side. `zeno
      test` runs the tests of a file or of every file in a folder,
      `zeno run` goes past them, and a build leaves them out.
      See [`docs/testing.md`](docs/testing.md))
    - `zeno new`: a new program or package in one command (done:
      `zeno new blinker` makes a folder with a `main.zeno`, a module
      with a test and a `zeno.package`. It runs, its test passes,
      and it is laid out as `zeno format` lays programs out. `zeno
      new --package filters` makes a package. Nothing is written
      into a folder that is there.
      See [`docs/new.md`](docs/new.md))
    - The gaps programs have run into (done: `count += 1` on a `u8`
      is `count = u8(count + 1)`, the sum checked as it is stored.
      `f"{angle:.1f}"` is `30.5 deg`: a quantity takes the formats
      of a float and keeps its unit. `//=`, `%=` and `**=` are
      there, so every operator has an `=` form. Two mistakes were
      found and mended on the way: `x += f()` now reads `x` first
      in every build, and a call in the place of a `+=`, which ran
      twice, is refused.
      See [`docs/gaps.md`](docs/gaps.md))
    - A last pass over every decision marked provisional (done: each
      of them, about eighty, is now settled, a stated limit of 0.1,
      or changed, with the reason. They are all in chapter 11 of
      [`docs/decisions.md`](docs/decisions.md).
      See [`docs/version-0.1.md`](docs/version-0.1.md))
19. The Zeno Book (done: 36 chapters and seven appendices, from the
    first program to the inside of the compiler, with every sample run
    by the real compiler as the book is built. A printable PDF in
    [`book/dist/zeno-book.pdf`](book/dist/zeno-book.pdf) and pages for
    the web in `book/dist/html/`. See [`book/README.md`](book/README.md))
20. The first release (done: `zeno` 0.1.0, already built, for ten kinds
    of machine: Linux on x86-64, ARM64, 32-bit x86, ARMv7 and ARMv6,
    macOS on Apple silicon and Intel, Windows on x86-64, 32-bit x86
    and ARM64. A tag starts the build, and each build is run before
    it is released. Making it found that a compiled program rounded
    `c * 1.8 + 32` differently from `zeno run` on ARM64 and on 32-bit
    x86, which is mended.
    See [`docs/release.md`](docs/release.md))
21. What writing the book found (done: `zeno check --no-collector`
    says what the build would say. `none` is tested for with `is`
    only, and only where a value can be `none`. A field of an enum
    case is taken out with `match`. A `const` is written from what
    the compiler can work out. A parameter with a default comes last.
    `None`, `null`, `True` and `elif` each get a message that says
    what Zeno writes.
    See [`docs/found-by-the-book.md`](docs/found-by-the-book.md))
22. Windows as a whole (done: `zeno build --llvm` works with the clang
    that comes with Visual Studio. A compiled program waits, reads
    the clock and takes its arguments the Windows way. Every path
    Zeno makes is written with `/`. Every test of Zeno passes on
    Windows, tried there by a small workflow for one machine.
    See [`docs/release.md`](docs/release.md))
23. A program in a web page (done, and what version 0.2 is:
    `import web` gives a program the elements of a page, their text
    and classes, clicks, keys and timers. `zeno build --web` writes
    one `.html` file that a browser opens, with the program and Zeno
    itself in it, built for WebAssembly. Under `zeno run` and `zeno
    test` the page is a simulated one, so what a page does is tested
    on a desk.
    See [`docs/web.md`](docs/web.md))
24. Installers (done, in 0.2: a setup program for Windows and one
    line for macOS and Linux. Each picks the right `zeno`, puts it on
    the PATH, offers the C compiler and git that `zeno build` and
    `zeno fetch` hand work to, and can take it all off again.
    See [`docs/install.md`](docs/install.md))
25. A page that does more (done, in 0.2: a function given to `on` may
    ask for the `Event`, which says where the mouse was and what was
    clicked. The style of one element, drawing on a canvas, a timer
    that repeats, text kept between visits, and `fetch` and `post`
    for the network. `fire` presses a button in a test. `zeno run`
    reads the page that was written, so an id spelt wrong is found on
    a desk. A built page is 0.4 MB where it was 1.5 MB. And `zeno
    playground` writes a page in which programs are written and run
    with nothing installed.
    See [`docs/web.md`](docs/web.md) and
    [`docs/playground.md`](docs/playground.md))
26. What 0.1 left, taken up (done, in 0.2: `min` and `max` take
    quantities in one unit. A mistake in `let x: str = first([1])`
    is said at `x`. An editor says the type of a field. `zeno test
    --only "text"` chooses tests by name.
    See chapter 17 of [`docs/decisions.md`](docs/decisions.md))
27. JSON (done, in 0.2: `import json` has `json.parse(text)`, which
    gives the value the text describes, and `json.write(value)`,
    which gives the text of a value. A page reads what a server
    answers with it, and a program its settings.
    See [`docs/json.md`](docs/json.md))
28. Packages (done, in 0.2: each release has a `.deb` for apt, a
    package for pacman, a snap and a flatpak, and recipes for Homebrew
    and winget, all made from the release's own files and each
    installed, run and removed before a release. What is left is the
    place each manager looks in. Homebrew works, and for apt, the AUR,
    winget and the Snap Store everything is ready up to the account
    each needs.
    See [`docs/packages.md`](docs/packages.md) and
    [`docs/stores.md`](docs/stores.md))
29. A timer that can be stopped (done, in 0.2: `web.after` and
    `web.every` give a `Timer`, and `timer.stop()` calls off what
    it was waiting to do.
    See chapter 19 of [`docs/decisions.md`](docs/decisions.md))
30. `sort` with a function (done, in 0.3: `people.sort(age_of)` puts
    a list of anything in the order of what the function gives for
    each item, and items with the same stay as they were. In every
    engine.
    See [`docs/library.md`](docs/library.md) and chapter 21 of
    [`docs/decisions.md`](docs/decisions.md))

## Layout

```
src/main.rs         the `zeno` command
src/token.rs        the kinds of token
src/lexer.rs        source text -> tokens
src/ast.rs          the shapes of the syntax tree
src/parser.rs       tokens -> syntax tree
src/printer.rs      syntax tree -> text, for `zeno parse`
src/modules.rs      the files of a program -> one syntax tree
src/package.rs      reads `zeno.package` and `zeno.lock`, and is `zeno fetch` and `zeno new`
src/format.rs       `zeno format`: one layout for every file
src/json.rs         reads and writes JSON, for the language server and the `json` module
src/lsp.rs          `zeno lsp`: the language server an editor starts
editors/            colours and setup for VS Code, Vim, Neovim and Helix
src/checker.rs      finds type mistakes before the program runs
src/value.rs        what a value is while a program runs
src/ops.rs          what each operator and step does, for both engines
src/interpreter.rs  runs a syntax tree
src/bytecode.rs     the instructions of the VM, and a printer for them
src/emit_bytecode.rs  syntax tree -> bytecode
src/vm.rs           runs bytecode, for `zeno run --vm`
src/builtins.rs     print, len, methods, modules, formatting
src/board.rs        the simulated board that `zeno run` uses
src/page.rs         the page of the `web` module: the browser's, or a simulated one
src/web_host.rs     where `main` would be, when Zeno is built for a browser
src/web.js          starts a program in a page, and does what it asks of the page
src/html.rs         reads a page of HTML, so that the simulated page has its elements
src/deflate.rs      packs `zeno.wasm` into a built page, the way a `.zip` is packed
src/playground.html the playground, before `zeno playground` writes Zeno into it
src/ints.rs         the sized whole numbers: what a `u8` holds
src/units.rs        the units a number can carry
src/emit_c.rs       syntax tree -> C source, for `zeno build`
src/emit_llvm.rs    syntax tree -> LLVM IR, for `zeno build --llvm`
src/runtime.c       the C library every compiled program carries
src/error.rs        the error type every stage shares
docs/               the design decisions, and how each stage works
examples/           sample .zeno programs
tools/release-check.sh        runs a built `zeno` before it is released
tools/install.sh              installs Zeno on macOS and Linux
tools/try-install.sh          tries install.sh, with no release to download
tools/installer/              the setup program for Windows (Inno Setup)
tools/package.sh              makes the packages: .deb, snap, flatpak, pacman, Homebrew, winget
tools/try-packages.sh         installs each package, runs it, and removes it
tools/keep-recipes.sh         puts the recipes of a release into the repository
Formula/                      the Homebrew formula, written at each release
packaging/                    the PKGBUILD and the winget files, written at each release
.github/workflows/release.yml builds and releases `zeno` when a tag is pushed
.github/workflows/trial.yml   tries Zeno on one kind of machine, and releases nothing
.github/workflows/installer.yml  tries the installers on one kind of machine
.github/workflows/add-installers.yml  makes the installers for a version already released
.github/workflows/packages.yml  makes and tries the packages of a version already released
.cargo/config.toml  how `zeno` is linked for each kind of machine
```

## Licence

Zeno is under the MIT licence: anyone may use it, change it and pass it
on, as long as the notice in [`LICENSE`](LICENSE) goes with it. Every
archive, installer and package of a release carries that file, from
0.2.0 on.
