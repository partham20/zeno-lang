# What Zeno does not have yet, and what to do about each

This page builds nothing. It is the whole list of what Zeno is still
missing, in the order it should be built, with a recommendation for
each item. It was written on 2026-11-18, against `main` after 0.5.1.

The list comes from three places:

1. **The limits Zeno wrote down itself.** Chapter 11 of
   [`decisions.md`](decisions.md) and appendix G of the book, "What
   Zeno Does Not Have Yet". Each one was tried again on today's
   compiler for this page.
2. **What the README promises and has not delivered.** The table "One
   language, every job" names C in both directions, `async` and
   `await`, fast arrays, operator overloading, and mobile.
3. **What C, C++, Python, JavaScript, Java, Rust and Go have** that
   neither of the first two mentions. Each was tried as a small
   program, to see what Zeno says today.

Zeno is for firmware first, so what a board needs is weighed above
what a server needs.

## Will adding these change what works today?

No. Every item below is refused today with a message, or is a name
that does not exist yet. No program that is accepted now uses any of
them, so allowing one later changes the meaning of no program. That
was the test for putting a limit on the list in the first place
(11.6).

What each one costs is work, and the work is wide. A new built-in
function is written for the interpreter, the VM, the C runtime, the
LLVM build and fifteen targets. That is why the first item below is
the one that makes the others cheap. And nothing may make a program
that does not use it slower or larger: speed is compared before every
release ([`release.md`](release.md)).

## The order to build them in

| | What | Size | Why here |
|---|---|---|---|
| 1 | A library written in Zeno. **Done** ([`library-in-zeno.md`](library-in-zeno.md)) | medium | Every later library function is then written once, and not once for each engine and each target |
| 2 | The small library gaps. **Done:** `hypot`, `sinh`, `cosh`, `tanh`, `gcd`, `clamp`, random numbers, and `program.exit`, `program.environment` and `program.warn` ([`ending-a-program.md`](ending-a-program.md)) | small | First use of 1. Every one is asked for by ordinary programs |
| 3 | An enum case with a number: `Ack = 6`. **Done** ([`enum-numbers.md`](enum-numbers.md)); a loop over the cases is left | medium | Every protocol and every register field is a number with a name |
| 4 | Bytes: bytes from and to a file (**done**, [`bytes.md`](bytes.md)), a packed `list[u8]` (**done**, decision 36.2), a list of a fixed length (**done**, [`fixed-lists.md`](fixed-lists.md), decision 36.4; kept off the pool is left) | large | "The first thing a real driver will ask for" (5.13a) |
| 5 | `u64` and `f32`. **Done** ([`f32.md`](f32.md), chapter 41; [`u64.md`](u64.md), chapter 42) | medium | Counters and timestamps; a board whose float unit is 32 bits |
| 6 | Units made of units: `V * A` is `W`. **Done** ([`units-made.md`](units-made.md), chapter 43) | medium | Units are what Zeno has that C does not. Power, resistance and frequency are the firmware's daily sums |
| 7 | Inputs on a board, a serial port, an ADC, PWM, interrupts | large, wants a board | The one thing that keeps Zeno off real hardware |
| 8 | One step from a `.zeno` file to a board: a runtime with no `printf` and no `exit`, the cross-compiler named on the line, checks without GCC built-ins | large | The same |
| 9 | What a build with no collector still refuses, and `raises` in a function type | medium | Firmware builds should take every program `zeno run` takes |
| 10 | Several values from one function | medium | Go, Python and Rust all have it, and a driver's `read` wants a value and a status |
| 11 | Generic enums, generic interfaces, and bounds | large | A `Maybe[T]` and a `sort` that asks for an order |
| 12 | Cleaning up when a block ends, and files read in pieces | medium | An open file, a lock and an interrupt mask all need "and put it back" |
| 13 | Tools: step through a `.zeno` file in gdb, tests of a compiled program, a message on `assert`, completion and rename in the editor, a REPL | medium, in pieces | Each is small by itself |
| 14 | More than one thing at a time: tasks, threads, `async` | very large | Last, because it touches memory, errors and every engine |

Calling C from Zeno belongs between 7 and 8. It is being built
elsewhere, as the third version of the bridges to other languages, so
it is not on this list. What matters for firmware is that it works in
`zeno build --firmware`, and not only in `zeno run`: a vendor's HAL is
C.

## 1. On a board

| Missing | What the others have | Recommendation |
|---|---|---|
| **Inputs**: reading a pin, saying a pin is an input, a pull-up (3.15b) | C through the vendor's header. Rust: `embedded-hal` | Build it. Outputs were designed on a simulated board and a header put underneath; inputs can be done the same way, with a test that says what the pin reads. Then try it on one real board before it is called done |
| **A serial port, an ADC, PWM, a hardware timer** | The same | Build after inputs, in that order. The serial port first: it is how every board talks |
| **Interrupts**: a function that runs when a pin or a timer says so, and a safe way to share a variable with it | C: a vector table and `volatile`. Rust: `#[interrupt]` and a critical section | Design it with item 14 in mind. A variable shared with an interrupt needs a rule, and the rule should be one sentence |
| **A runtime for a board**: the firmware C still calls `printf` and `exit` ([`board.md`](board.md)) | C and Rust build with no library at all | Build it. `print` goes to a function the board supplies, as the pins do |
| **Another processor in one step**: a build writes C, and compiling it for a board is a second command (5.12c) | Rust: `--target thumbv7em-none-eabihf`. Go: `GOARCH` | `zeno build --cc arm-none-eabi-gcc`, with the flags and the linker script named in `zeno.package` |
| **Checks without GCC and Clang built-ins** (5.5e) | | Write the checks out in `runtime.c` behind one `#if`. IAR, Keil and the older TI compilers have no such built-ins |
| **MISRA**: variable argument lists in the runtime, `printf`, a forward `goto` ([`firmware.md`](firmware.md)) | | Take them one rule at a time after the board runtime, which removes two of them |
| **`time.now()` is a float of milliseconds** | C: a `uint32_t` of ticks | Give a whole number of ticks beside it, after `u64` |
| **Never run on hardware.** The STM32 header in [`board.md`](board.md) was written and not tried | | This needs a board on a desk. It is the only item on this page that needs hands |

## 2. Numbers and memory

| Missing | What the others have | Recommendation |
|---|---|---|
| ~~**A packed `list[u8]`**: an item takes eight bytes (5.13a)~~ | C: `uint8_t buf[64]`. Rust: `Vec<u8>`, `[u8; 64]`. Go: `[]byte` | **Done** (36.2): a `list[u8]` takes one byte an item. Lists of the other sized numbers still take eight |
| ~~**A list of a fixed length**~~, whose size is known when the program is built | C, C++, Rust and Go arrays | **Done** (36.4): `list[u8, 64]`, with `[0] * 64` to fill one (36.3). Left: keeping its items in the object or on the stack, with no block from the pool (36.5) |
| ~~Bytes from and to a file~~ (6.16) | All seven | **Done** (chapter 36): `files.read_bytes` gives a `list[u8]`, `files.write_bytes` takes one |
| ~~**`u64`**~~ (2.8d), and `i64` as a name | All but JavaScript and Python, which have big numbers | **Done** (chapter 42): an operator between two `u64`s gives a `u64`, checked, and it meets an `int` only when the number is written. `i64` is not added: it would be a second name for `int` (42.1) |
| ~~**`f32`**~~ | C, C++, Java, Rust, Go | **Done** (chapter 41): sums in 32 bits, an f32 goes where a float is wanted, and an f32 beside a float is refused. Left: `math` in 32 bits (41.9) |
| ~~**`u8.max` and `u8.min`**~~ | Rust: `u8::MAX` | **Done** (42.7): `.max` and `.min` on every whole type, from `u8` to `u64`. Left: `float.max` |
| **Small variables and parameters**: only a field of a class is stored at its size | C | Leave. The C compiler keeps them in registers either way |
| **The layout of an object**: no promise of the order or the gaps between fields, and no fields of a few bits | C: structs and bit fields. Rust: `repr(C)` | Build after bytes, as a word on the class. A frame or a register block is then a class |
| **Pointers to more than numbers**, the address of a variable, `p - q`, a 64-bit register ([`unsafe.md`](unsafe.md)) | C, C++, Rust | Build as drivers ask. A pointer to an object first, since that is what a C library hands back |
| ~~**Units made of units**~~, and a unit a program names itself (1.15) | None of the seven have units at all. F# does | **Done** (chapter 43): `V * A` is `W`, `V / A` is `ohm`, `W * h` is `J`, `1 / s` is `Hz`, from a closed list. Left: charge (`mAh`), speed, and a program's own units |
| **Numbers of a fixed number of decimals**, for a processor with no float unit | None as part of the language | Leave for a package |
| **Code that is built only for one board** | C: `#if`. Rust: `cfg`. Go: build tags | A `const` and an `if` that the compiler drops when it can never be true. No second language inside the language |

## 3. The language

| Missing | What the others have | Recommendation |
|---|---|---|
| ~~An enum case with a number, an enum from a number~~, a loop over the cases | C, C++, Java, Rust, Go (`iota`) | **Done** (chapter 35): `Ack = 6`, `int(Ack)`, `Reply(6)` gives a `Reply or none`. The loop over the cases is next |
| **Several values from one function**, and `a, b = b, a` | Python, Go, Rust, C++ | Build it as two or more results and not as a new kind of value: `-> (int, str)`, taken apart where it is received. A tuple that is kept in a list is a class |
| **Generic enums** (3.16e) | Rust, Java, C++ | Build it. `enum Maybe[T]` is the commonest generic there is |
| **Generic interfaces and bounds** (3.16d, 10.10) | Rust traits, Java, Go, C++ concepts | Build after generic enums. `function largest[T is Ordered]` moves the error from inside the generic to the call |
| **A method with a type parameter of its own**, type arguments written at a call | Rust, Java, C++, Go | With bounds |
| **Something that runs when a block ends** | C++ destructors, Rust `Drop`, Go `defer`, Python `with`, Java `try` with resources | Build it for owned values: a function of the class that the owner's end calls. It is what an open file needs (6.12) |
| **`finally`** | Python, Java, JavaScript | Leave. The item above does its work, and one way is the rule |
| **A value chosen by a condition** in one line (`x if c else y`) | All but Go | Decide once. There is one way to write an `if` today, and it is a statement |
| **Operators for a class** (`a + b` for two vectors) | C++, Python, Rust | Build it late, as an interface the class says it is: `class Vector is Addable`. The README promises it for numerics |
| **Unions** other than `or none` (1.13) | Python, TypeScript | Leave. An enum that carries data is the one way |
| **Text over several lines, text that repeats (`"-" * 20`), an item counted from the end (`xs[-1]`)** | Python, mostly | Small. Text over several lines is worth having. The other two have a way already (`slice`, `len`) |
| **A set** | Python, Java, Rust, C++, JavaScript | A dict of `true` does the work. Write it in Zeno once item 1 is done |
| **A key of a dict that is not a str or an int**: an enum case, a float | All | Enum cases, once they can be numbers. Floats: leave |
| **A second name for a type** (`type Volts = float`) | C, C++, Rust, Go | Leave. Units do this job better |
| **A named function inside a function, in a build**; a method as a value; a default that is not written out | | Build. `zeno run` takes all three, and a build should take what `zeno run` takes |
| **More than one thing at a time**: threads, tasks, `async` and `await`, channels | All seven, each its own way | Last and largest. Choose one model, and choose the one a board can run: tasks that take turns, with a queue between them. The README's `async` row waits for this |
| **Something that gives its values one at a time** (`yield`) | Python, JavaScript, Rust, Go | Leave until a program asks |
| **Code that writes code**: macros, reflection, running at build time | C, C++, Rust | Leave. It is the hardest thing to keep unambiguous |
| **Inheritance, a second way to write a function, `import x as y`** | | Decided, and not limits ([`version-0.1.md`](version-0.1.md)) |

## 4. The library

| Missing | Recommendation |
|---|---|
| ~~A library written in Zeno~~ (6.10) | **Done** (chapter 33). A module that comes with Zeno may have a part that is a `.zeno` file kept inside the `zeno` program, joined in only when a program names something from it |
| ~~`hypot`, `sinh`, `cosh`, `tanh`, `gcd`, `clamp`~~ (6.21) | **Done**, as the first functions written in Zeno |
| ~~Random numbers~~ (6.21) | **Done**: the module `random`. What is left is a seed from the machine, which waits for the time of day |
| **The methods of text and lists in Zeno** (6.10) | Leave. They are called as `xs.insert(...)` and take items out of a list they were lent, which a function written in Zeno may not |
| ~~`program.exit(code)` and the environment~~ (6.19) | **Done** (chapter 34): `program.exit` and `program.environment`, in every engine and target |
| ~~Writing to the error stream~~ | **Done**: `program.warn(text)` (34.3) |
| **A file read in pieces** (6.12, 6.20) | After cleaning up at the end of a block |
| **Folders**: what is in one, making one, joining the parts of a path | Build with files in pieces |
| **The date and the time of day** | Build: `time` has only a stopwatch today |
| **The network** outside a page: a socket, an HTTP request | After tasks, for a server. `web.fetch` in a page is there |
| **Patterns in text** (regular expressions) | Leave for a package, or use `import python` |
| **Capital letters outside plain English** (3.14) | Settled for the language. A package for programs with the room |
| **JSON in a compiled program, and straight into a class** (18.5) | Build the first once the library can be written in Zeno. The second waits for generic bounds |
| **`program.run` in a build** (25.6) | Build as programs ask |
| **A queue, a heap, a search in a sorted list** | Write in Zeno once item 1 is done |

## 5. Tools

| Missing | What the others have | Recommendation |
|---|---|---|
| **A debugger.** The C that a build writes has no `#line`, so gdb steps through the C and not the `.zeno` file | All seven | Build the `#line` marks, in a `--debug` build only. It is small, and gdb, lldb and every probe for a board then work as they are |
| **A REPL.** The README names one | Python, JavaScript, Java | Small with the interpreter. Build it |
| **Tests of a compiled program**, a message on `assert`, something run before each test (10.21) | Rust, Go | Build the first two. A firmware program should be tested as it is built |
| **How much of a program its tests ran, and timing a function** | Go, Rust | Later |
| **In the editor: completion, rename, every use of a name, going to a field** (9.11, 17.4) | All | Completion first. It is what people notice is missing |
| **The VS Code extension is a folder to copy**, and not in the Marketplace | | Needs an account, like the stores in [`stores.md`](stores.md) |
| **Pages made from the comments of a program** | Rust, Go, Java, Python | Later, when packages exist that need them |
| **Compiling one file at a time** (7.11) | C, C++, Rust, Go, Java | Leave until a program is large enough to wait for |
| **The registry is empty**, and no package has been published through its form | | Publish the first one. `examples/filters` is ready-made |

## 6. Where a program can run

| Missing | Recommendation |
|---|---|
| **A real board.** See part 1 | The first platform to add |
| **A page that is compiled and has `import web`.** `--web` carries the interpreter (14.1). `--target html` is compiled and small, and has no `web` module | Give the JavaScript target the `web` module. The book's line on this is half out of date: the compiled page exists since 0.5 |
| **Canvas images and curves, more than one finger, the address bar** (16.13) | As pages ask |
| **The playground**: one file, no colours, no Stop button (16.12) | Small. Build with the next change to the playground |
| **A window on a desktop, a phone** | Far off. The Kotlin and Dart targets are the way to a phone, and neither has a screen module |

## 7. Firmware builds and builds with no collector

Each of these runs under `zeno run` and is refused by one kind of
build. They are item 9 in the order.

| Refused | Recommendation |
|---|---|
| A function that says `raises`, used as a value in a firmware build (5.5l) | `raises` in a function type: `function(str) -> int raises ValueError` |
| `list[int or none]` with no collector (5.16h) | Store the number and a flag side by side, with no block of its own |
| A dict of lists or objects with no collector (10.14) | The dict owns its values, as a list owns its items |
| A few moves of an owned value (5.8i, 5.15e, 5.17d), and `raise e` (5.9d) | One rule each, written in the sections named |
| The pool's size on the compiler's line (5.9j); spare blocks not filled with a pattern (5.9k) | `zeno build --pool 4096`. Both are exercises in [`pool.md`](pool.md) already |
| A function of an interface with a default or of its own type; an owned class as an interface (10.5, 10.10) | With generic interfaces |

## How Zeno compares, feature by feature

"Yes" means the language has it as part of itself or of the library
that comes with it.

| | C | C++ | Python | JS | Java | Rust | Go | Zeno |
|---|---|---|---|---|---|---|---|---|
| Sized whole numbers to 64 bits | yes | yes | no | no | yes | yes | yes | to 32 |
| A 32-bit float | yes | yes | no | no | yes | yes | yes | no |
| A list of a fixed length | yes | yes | no | no | yes | yes | yes | yes |
| Bytes packed one to a byte | yes | yes | yes | yes | yes | yes | yes | yes |
| Registers and raw pointers | yes | yes | no | no | no | yes | some | yes |
| Interrupts | yes | yes | no | no | no | yes | no | no |
| Builds with no library under it | yes | yes | no | no | no | yes | no | nearly |
| Numbers with units | no | no | no | no | no | no | no | yes |
| Overflow checked | no | no | no need | no | no | in debug | no | yes |
| `none` checked before use | no | no | no | no | no | yes | no | yes |
| Errors a function must declare | no | no | no | no | yes | yes | no | yes |
| Enums that carry data | no | some | no | no | some | yes | no | yes |
| An enum case with a number | yes | yes | yes | no | yes | yes | yes | yes |
| Generics | no | yes | no | no | yes | yes | yes | yes, no bounds |
| Several results from a function | no | yes | yes | yes | no | yes | yes | no |
| Cleaning up at the end of a block | no | yes | yes | yes | yes | yes | yes | freeing only |
| Operators for a class | no | yes | yes | no | no | yes | no | no |
| Threads or tasks | yes | yes | yes | yes | yes | yes | yes | no |
| Modules and packages | no | some | yes | yes | yes | yes | yes | yes |
| A formatter with the language | no | no | no | no | no | yes | yes | yes |
| A test runner with the language | no | no | yes | yes | no | yes | yes | yes |
| A language server | yes | yes | yes | yes | yes | yes | yes | yes, no completion |
| A debugger that shows the source | yes | yes | yes | yes | yes | yes | yes | no |
| A REPL | no | no | yes | yes | yes | no | no | no |
| Calls C | is C | yes | yes | yes | yes | yes | yes | being built |

## What was tried for this page

These were run on the compiler built from `main` on 2026-11-18, and
each was refused as the list says: `u64`, `f32`, `int or str`, `3 V *
2 A`, `program.exit`, the environment, `math.hypot`, `math.sinh`,
`math.clamp`, `import random`, `assert` outside a test,
`files.read_bytes`, `enum Maybe[T]`, `interface Ordered[T]`, a named
function inside a function in a build, `import json` in a build,
`list[int or none]` and a dict of lists with `--no-collector`, a
tuple, `-> (int, str)`, `a, b = b, a`, an enum case with `= 1`,
`defer`, `finally`, `async`, a set, `u8.max`, the address of a
variable, an enum case as a key, `"ab" * 3`, `xs[-1]`, text between
three quotes, `x if c else y`.

Since this page was written, `math.hypot`, `math.sinh`, `math.clamp`
and `import random` are no longer refused: see part 4.

Two lines of the book's appendix are no longer the whole story.
`sort` by a function came in 0.3. And a compiled page exists since
0.5 as `--target html`, without the `web` module.

The rest of the appendix was taken from the documents and not run
again: the moves of an owned value, `raise e` with no collector, the
functions of an interface, and the web items.
