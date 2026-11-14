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
| `( )` for calls, `[ ]` for lists, `,` between items | Everyone already reads these |
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
- **Whole numbers have a size where it matters.** A `u8` field is one byte, a number that does not fit is an error, and wrapping is something you ask for.
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
- [`examples/blink.zeno`](examples/blink.zeno): the same language on a microcontroller. It blinks the LED of a simulated board.
- [`examples/signal.zeno`](examples/signal.zeno): a traffic light on three pins, as a state machine. It builds for firmware with no collector.
- [`examples/packet.zeno`](examples/packet.zeno): sensor readings packed into six-byte frames and unpacked again, with `u8`, `u16` and the bit operators.

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
| Web front ends | WebAssembly backend |
| Web back ends, networking | `async` / `await` and a standard library |
| Data and numerics | Fast arrays and operator overloading |
| Mobile apps, games | Native code plus platform bindings |

## Status

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
`zeno build --no-collector` goes one step further for a program where
everything is owned, and leaves the collector out of the C altogether
([`docs/no-collector.md`](docs/no-collector.md)).

`zeno run` has two engines. The interpreter walks the syntax tree. With
`--vm` the program is compiled to bytecode and run by a small virtual
machine, which is two to three times as fast and prints exactly the same
([`docs/vm.md`](docs/vm.md)). Every test program is run on both.

`zeno build --llvm` compiles a second way: to LLVM IR, the input of the
compiler library behind Clang and Rust, which clang turns into a
program ([`docs/llvm.md`](docs/llvm.md)). It compiles what the C backend
compiles, and the tests build every program both ways.

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
cargo run -- build --no-collector --memory examples/buffer.zeno  # no garbage collector in the C at all
cargo run -- build --llvm examples/shapes.zeno  # compile to LLVM IR (shapes.ll), then to ./shapes with clang
cargo run -- run examples/blink.zeno            # blink the LED of the simulated board (Ctrl-C stops it)
cargo run -- build --firmware --no-collector examples/signal.zeno  # pins and a clock, built for firmware
cargo run -- build examples/packet.zeno         # u8 and u16 fields, stored in as many bytes in the C struct
cargo run -- parse examples/tour.zeno           # the syntax tree the parser builds
cargo run -- tokens examples/hello.zeno         # the tokens the lexer finds
cargo test
```

`from board import led` and `from time import sleep` give a program pins and a
clock. On a computer the board is simulated and prints each pin as it changes.
A compiled program meets the board through four C functions, so a real one
can be put underneath ([`docs/board.md`](docs/board.md)).

`u8`, `u16`, `u32`, `i8`, `i16` and `i32` are whole numbers with a size, and
`&`, `|`, `^`, `~`, `<<` and `>>` work on their bits. Arithmetic always gives
a full `int`, so nothing wraps by accident: `u8(n)` checks that `n` fits, and
`u8.wrap(n)` keeps the low eight bits on purpose
([`docs/sized-ints.md`](docs/sized-ints.md)).

- **Why Zeno is the way it is:** [`docs/decisions.md`](docs/decisions.md) gives the reason for every design choice.
- **How each stage works:** [`docs/lexer.md`](docs/lexer.md), [`docs/parser.md`](docs/parser.md),
  [`docs/checker.md`](docs/checker.md), [`docs/interpreter.md`](docs/interpreter.md),
  [`docs/compiler.md`](docs/compiler.md), [`docs/memory.md`](docs/memory.md),
  [`docs/firmware.md`](docs/firmware.md), [`docs/ownership.md`](docs/ownership.md),
  [`docs/no-collector.md`](docs/no-collector.md), [`docs/vm.md`](docs/vm.md),
  [`docs/llvm.md`](docs/llvm.md), [`docs/board.md`](docs/board.md),
  [`docs/sized-ints.md`](docs/sized-ints.md).

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
9. Owned values and borrows, beside the collector (done: `owned` values have
   one owner and are freed where its block ends, `borrow` lends one for a call)
10. A firmware build with no collector (done: `zeno build --no-collector`
    accepts a program where every list and object is owned, and leaves the
    collector out of the C)
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
    - Generics
    - `--llvm` together with `--firmware`

## Layout

```
src/main.rs         the `zeno` command
src/token.rs        the kinds of token
src/lexer.rs        source text -> tokens
src/ast.rs          the shapes of the syntax tree
src/parser.rs       tokens -> syntax tree
src/printer.rs      syntax tree -> text, for `zeno parse`
src/checker.rs      finds type mistakes before the program runs
src/value.rs        what a value is while a program runs
src/ops.rs          what each operator and step does, for both engines
src/interpreter.rs  runs a syntax tree
src/bytecode.rs     the instructions of the VM, and a printer for them
src/emit_bytecode.rs  syntax tree -> bytecode
src/vm.rs           runs bytecode, for `zeno run --vm`
src/builtins.rs     print, len, methods, modules, formatting
src/board.rs        the simulated board that `zeno run` uses
src/ints.rs         the sized whole numbers: what a `u8` holds
src/units.rs        the units a number can carry
src/emit_c.rs       syntax tree -> C source, for `zeno build`
src/emit_llvm.rs    syntax tree -> LLVM IR, for `zeno build --llvm`
src/runtime.c       the C library every compiled program carries
src/error.rs        the error type every stage shares
docs/               the design decisions, and how each stage works
examples/           sample .zeno programs
```
