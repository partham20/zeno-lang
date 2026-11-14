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

- **Types, where written, are checked before the program runs** (from the type checker stage on).
- **Errors are checked.** A function says what it `raises`, and the compiler makes sure someone handles it.
- **No surprise `none`.** A value can be `none` only if its type says `or none`, and you must check before using it.
- **Enums carry data**, and `match` must cover every case.
- **Compiled, not interpreted**, so it is fast and small enough for a microcontroller.

Samples, none of them final:

- [`examples/hello.zeno`](examples/hello.zeno): one line.
- [`examples/tour.zeno`](examples/tour.zeno): the main features.
- [`examples/styles.zeno`](examples/styles.zeno): one job written typed, untyped, object-oriented and functional.
- [`examples/blink.zeno`](examples/blink.zeno): the same language on a microcontroller.

## One language, every job

The goal is that anything you would reach for C, C++, Python, JavaScript, Java or Rust
to do, you can do in Zeno. That is a long road, and each use case arrives with a
piece of the compiler:

| Use case | What gets it there |
|---|---|
| Scripts, automation, a REPL | Interpreter (`zeno run`) |
| Microcontrollers and firmware | Compile to C, no heap required |
| Command-line tools, desktop, servers | Native code through C, later LLVM |
| Calling existing C libraries | C interop, in both directions |
| Web front ends | WebAssembly backend |
| Web back ends, networking | `async` / `await` and a standard library |
| Data and numerics | Fast arrays and operator overloading |
| Mobile apps, games | Native code plus platform bindings |

## Status

Zeno runs programs. The lexer, parser and interpreter work; types are parsed
but not checked yet, which is the next stage.

```sh
cargo run -- run examples/tour.zeno       # run a program
cargo run -- parse examples/tour.zeno     # the syntax tree the parser builds
cargo run -- tokens examples/hello.zeno   # the tokens the lexer finds
cargo test
```

`examples/blink.zeno` does not run yet, because there is no `board` module.

- **Why Zeno is the way it is:** [`docs/decisions.md`](docs/decisions.md) gives the reason for every design choice.
- **How each stage works:** [`docs/lexer.md`](docs/lexer.md), [`docs/parser.md`](docs/parser.md), [`docs/interpreter.md`](docs/interpreter.md).

## Roadmap

1. Lexer (done)
2. Parser and syntax tree (done)
3. Tree-walking interpreter, `zeno run` (done)
4. Type checker
5. Bytecode VM
6. Compile to C (`zeno build`)
7. Memory management
8. LLVM backend
9. Extras

## Layout

```
src/main.rs         the `zeno` command
src/token.rs        the kinds of token
src/lexer.rs        source text -> tokens
src/ast.rs          the shapes of the syntax tree
src/parser.rs       tokens -> syntax tree
src/printer.rs      syntax tree -> text, for `zeno parse`
src/value.rs        what a value is while a program runs
src/interpreter.rs  runs a syntax tree
src/builtins.rs     print, len, methods, modules, formatting
src/error.rs        the error type every stage shares
docs/               the design decisions, and how each stage works
examples/           sample .zeno programs
```
