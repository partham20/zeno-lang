# How the interpreter works

The interpreter is the third stage, and the first one that makes a program
*do* something. It takes the syntax tree from the parser and runs it directly.

```sh
cargo run -- run examples/tour.zeno
```

## The files

- [`src/value.rs`](../src/value.rs): what a value is while the program runs.
- [`src/interpreter.rs`](../src/interpreter.rs): scopes, statements, expressions, calls.
- [`src/ops.rs`](../src/ops.rs): what each operator does, reading fields
  and items, and lining arguments up with parameters. It is a file of its
  own because the bytecode VM ([`vm.md`](vm.md)) calls the same code.
- [`src/builtins.rs`](../src/builtins.rs): `print`, `len`, string and list
  methods, the `math` and `time` modules, f-string formatting.

## Walking the tree

The parser produced two kinds of node, and the interpreter has one function
for each:

- `exec(statement)` runs a statement. It returns nothing; it has an effect.
- `eval(expression)` works out an expression and returns a `Value`.

Each is one big `match` on the kind of node, and each calls itself for the
nodes nested inside. Here is all of `while`:

```rust
StmtKind::While { condition, body } => {
    while self.condition(condition, env)? {
        match self.exec_block(body, env) {
            Ok(()) | Err(Control::Continue(_)) => {}
            Err(Control::Break(_)) => break,
            Err(other) => return Err(other),
        }
    }
    Ok(())
}
```

A Zeno `while` loop is run by a Rust `while` loop. That is the whole idea of
a tree-walking interpreter: the host language does the work, one node at a time.

For `1 + 2 * 3`, `eval` on the `+` node calls `eval` on its left side (giving
`1`) and on its right side, the `*` node, which in turn evaluates `2` and `3`
and returns `6`. Then the `+` adds them. The order of operations was decided
by the shape of the tree; the interpreter just follows it.

## Values

`Value` (in `value.rs`) is an enum with a case for everything a variable can
hold: `Int`, `Float`, `Str`, `Bool`, `None`, `List`, a function, a class, an
object, an enum case, an error.

Simple values are copied. Lists and objects are wrapped in `Rc`, a shared
pointer, so `let b = a` makes two names for one list.

## Scopes

A `Scope` is a table from names to values, plus a link to the scope around
it. Each `{ }` block gets a new one.

```
let x = 1            scope A: { x }
if true {
    let y = 2        scope B: { y }, parent A
    print(x + y)     looks for x in B, then in A
}
```

Looking a name up walks outward through the parents until it is found. When
the block ends, its scope is dropped, and `y` is gone. The outermost scope
holds the built-ins, so `print` is found the same way as any variable.

Each variable remembers whether it was a `let` or a `var`, which is how
assigning to a `let` gets caught.

## Functions and closures

A function value is a `Closure`: the function's code plus the scope it was
*defined* in. Calling it:

1. makes a new scope whose parent is that defining scope,
2. declares each parameter in it, from the arguments or the defaults,
3. runs the body there.

Because the parent is where the function was defined, not where it was
called from, a function can go on using the variables around it even after
the outer function has returned:

```
function counter() {
    var count = 0
    function next() -> int {
        count += 1          # still reaches count
        return count
    }
    return next
}
```

A function written as a value, `function(n) { return n * 2 }`, becomes a
closure at the moment that expression is evaluated. Since a closure is just
a `Value`, it can be stored in a variable, passed as an argument, or handed
to `map`, `filter` and `reduce`, which call it once per item.

A function keeps the variables in sight where it is written, and not ones
declared further down the same block. So it does not keep the block's
scope itself, which goes on growing. It keeps a **view** of it
(`Scope::seen`): a scope with no variables of its own that looks into the
block's and sees only the first so many declared there. Each variable is
held through an `Rc`, so the function and the block share the very same
variable, and a change made by one is seen by the other. Why it matters
is in `docs/decisions.md` 3.6c and 3.6d.

Before running a block, `hoist` declares all the functions, classes and
enums in it, so they can be used above the line where they are written.
None of the block's variables exist yet at that moment. So `hoist` counts
how many will have been declared by the time the program reaches each
function's line, and that count is where the function's view ends.

## Leaving early: break, return, raise

`return` can be deep inside loops and ifs, and has to abandon all of them.
The interpreter uses Rust's own error mechanism for this. `exec` and `eval`
return `Result<_, Control>`, where `Control` is one of:

- `Break`, `Continue`: caught by the nearest loop.
- `Return(value)`: caught by the function call.
- `Raise(error)`: caught by a `try` with a matching `except`. When an
  object of a class is raised, it is wrapped in an error named after the
  class, so every `except` deals with the same kind of thing.
- `Fatal(error)`: a bug in the program; caught by nothing.

Every `?` in the interpreter means "if something is leaving early, pass it
up". Each construct catches only what belongs to it and passes the rest on,
as the `while` code above shows.

## Classes, enums and match

A class is a list of fields and a table of methods. Calling it builds an
`Instance` with a value for each field. Reading `p.distance_to` finds the
method and returns it already bound to `p`; calling that passes `p` as `self`.

An enum declares its cases as names. A case with no data is a finished value
(`Idle`); a case with data is a constructor (`Circle(1.0)`).

`match` tries each `case` in order. `matches` compares a pattern with a
value, and when a pattern contains a name, declares it in the arm's scope so
the body can use it.

## What it does not do yet

- **Look at types.** The interpreter ignores annotations. Checking them is
  the type checker's job, and `zeno run` calls it first; see
  [`checker.md`](checker.md).
- **Free cyclic memory.** A function and its scope can point at each other
  and are never released. That belongs to the memory management stage.
- **Run fast.** Names are looked up in hash tables on every use. The
  bytecode VM does not have to: see [`vm.md`](vm.md).

## Things to try

1. Add a `round(x)` built-in. (One function in `builtins.rs`, one line in `globals`.)
2. Add a `loop { }` statement, if you added it to the parser earlier.
3. Add `insert(index, item)` and `clear()` to lists.
4. Add a `board` module whose `led.toggle()` prints `led on` / `led off`, so
   that `examples/blink.zeno` runs on a PC.
5. Make `print(a, b, sep=", ")` work. Why does it not already?
