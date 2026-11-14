# Zeno

> Every journey is halfway done. Forever.

Zeno is a small programming language, built from scratch as a learning project.
It is named after Zeno of Elea, whose paradoxes say Achilles can never catch the
tortoise, and after the quantum Zeno effect, where a system watched closely
enough never changes. A fitting name for a compiler that will always be almost finished.

## The idea

Zeno borrows what works from languages that came before it:

- **Readable like Python**: indentation-based blocks, little punctuation.
- **Static types like Rust and Java**, with inference so you rarely write them.
- **No null, no exceptions**: `Option` and `Result`, with `?` for early returns.
- **Pattern matching** over enums and values.
- **Compiles to C**, so the output runs anywhere a C compiler does, including microcontrollers.
- Possible extras for embedded work: units in types (`500 ms`, `3.3 V`), timing budgets, state machines.

See [`examples/hello.zeno`](examples/hello.zeno) for a rough sketch. None of the syntax is final.

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
