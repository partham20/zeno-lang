# Zeno

> Every journey is halfway done. Forever.

Zeno is a small programming language, built from scratch as a learning project.
It is named after Zeno of Elea, whose paradoxes say Achilles can never catch the
tortoise, and after the quantum Zeno effect, where a system watched closely
enough never changes. A fitting name for a compiler that will always be almost finished.

## The idea

**Python first.** If Python has a way to say something, Zeno says it the same way:
`def`, `class`, `if / elif / else`, `for x in`, `match / case`, `try / except`, f-strings,
list comprehensions, and no `main()` needed for a script. Syntax from Rust or Java
appears only where Python has no answer.

What changes is underneath:

- **Type hints are real.** Types are checked before the program runs, and inferred so you rarely write them.
- **Errors are checked.** A function says what it `raises`, and the compiler makes sure someone handles it.
- **No surprise `None`.** A value can be `None` only if its type says `| None`, and you must check before using it.
- **Enums carry data**, and `match` must cover every case.
- **Compiled, not interpreted**, so it is fast and small enough for a microcontroller.

The borrowed pieces so far: `const` (C/Java), `raises` (Java), enums with data (Rust),
and units on numbers such as `500 ms` (new).

Samples, none of them final:

- [`examples/hello.zeno`](examples/hello.zeno): one line.
- [`examples/tour.zeno`](examples/tour.zeno): the main features.
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

Nothing works yet. The `zeno` command only prints its version and usage.

```sh
cargo run -- --help
```

## Roadmap

1. Lexer
2. Parser and AST
3. Tree-walking interpreter (`zeno run`)
4. Type checker
5. Bytecode VM
6. Compile to C (`zeno build`)
7. Memory management
8. LLVM backend
9. Extras

## Layout

```
src/main.rs        the `zeno` CLI (compiler stages will live under src/)
examples/          sample .zeno programs
```
