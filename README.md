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
cargo run -- run examples/driver.zeno           # memory by hand and registers: `zeno run` checks every use of a pointer
cargo run -- build --memory examples/driver.zeno  # the same, compiled: the report counts what `allocate` gave and what was never freed
cargo run -- check --no-collector examples/scheduler.zeno  # check as that build would, without building
cargo run -- build --llvm examples/shapes.zeno  # compile to LLVM IR (shapes.ll), then to ./shapes with clang
cargo run -- build --llvm --firmware examples/sensor.zeno  # the same through LLVM, with errors as values
cargo run -- build --llvm --no-collector --memory examples/buffer.zeno  # through LLVM with no collector: the same memory report as the C build
cargo run -- run examples/blink.zeno            # blink the LED of the simulated board (Ctrl-C stops it)
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
  [`docs/vm.md`](docs/vm.md),
  [`docs/llvm.md`](docs/llvm.md), [`docs/board.md`](docs/board.md),
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
15. Modules across files (next)
16. A package manager
17. Editor support: a formatter and a language server

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
