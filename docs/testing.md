# Tests: `test`, `assert` and `zeno test`

A program that can check itself gets changed with less fear. This
page adds the tests of a Zeno program, written in Zeno, and the
command that runs them.

```sh
cargo run -- test examples/ring
```

```
examples/ring/ring.zeno
  ok      a new ring is empty
  ok      bytes come out in the order they went in
  ok      a full ring keeps what it has
  ok      the ring goes round
  ok      a ring of no size is refused

5 tests: 5 passed, 0 failed.
```

[`examples/ring/ring.zeno`](../examples/ring/ring.zeno) is the queue
of bytes from [`private.md`](private.md), with its tests at the end
of the file. The decisions are 10.16 to 10.21 in
[`decisions.md`](decisions.md).

## What a test looks like

```zeno
test "a full ring keeps what it has" {
    let queue = ring(2)
    assert queue.put(1)
    assert queue.put(2)
    assert not queue.put(3)
    assert queue.slots == [1, 2]
}
```

A test is a block with a name. The name is text, so it can say what
is checked in plain words. Inside, `assert` says what must be so.

| Written | Means |
|---|---|
| `test "name" { ... }` | a test, at the top of a file |
| `assert total == 6` | this must be true, or the test fails here |
| `assert total == 6, "the total of the three"` | the same, and a failure prints that text first |
| `assert raises ValueError { ... }` | the block must raise that kind of error |

Three more things are worth knowing.

**A test sees what is private to its file.** `slots` is a private
field of `Ring`. No other file can read it, and the test can, because
it stands in `ring.zeno`. That is why tests go in the file they test.

**An error that leaves a test fails it.** A test needs no `try`
around what should work.

**A failed `assert` ends its test.** The lines after it do not run,
and no `except` catches it: it is not an error of the program.

## What a failure says

Change `assert queue.get() == 20` in the example to `== 30`:

```
examples/ring/ring.zeno
  ok      a new ring is empty
  FAILED  bytes come out in the order they went in
  ok      a full ring keeps what it has
  ok      the ring goes round
  ok      a ring of no size is refused

FAILED: bytes come out in the order they went in
error: this is false. The left of `==` is 20, and the right is 30
  --> examples/ring/ring.zeno:83:12
   |
83 |     assert queue.get() == 30
   |            ^

5 tests: 4 passed, 1 failed.
```

The line, and what was on each side of the comparison. Other
languages have `assert_eq`, `assert_ne`, `assertIn` and more, each so
that a failure can show the values. Zeno has the one `assert`,
because the interpreter can see that the condition is a comparison
and keep the two sides (10.17).

When the values alone do not say what went wrong, an `assert` can
say it, after a comma:

```zeno
assert reply == Reply.Ack, f"the board answered {reply} to the reset"
```

```
FAILED: the board answers a reset
error: the board answered Nak to the reset
this is false. The left of `==` is Nak, and the right is Ack
```

The text is printed on a line of its own above what was false, which
stays. It must be text, and it is worked out only when the `assert`
fails, so it can use what a passing test would never need. If working
it out raises an error, the test fails with that error (60.1).

When an error was expected and did not come:

```
error: this block was expected to raise ValueError, and it raised nothing
```

And when another one came:

```
error: this block was expected to raise KeyError, and it raised ValueError: a ring has room for one byte at least
  = note: that error was raised on line 57
```

## What `zeno test` runs

```sh
zeno test ring.zeno        # the tests in this file
zeno test examples/ring    # the tests in every file of this folder, and below
zeno test                  # the same, for the folder you are in
zeno test --only "empty"   # only the tests with `empty` in their name
```

For each file that has a test, `zeno test` runs the file as `zeno
run` would, and then each test in it, in the order written (10.19).
So a test can use everything the file declares.

- **A file with no test is not run.** `examples/ring/main.zeno` is
  the program, and it is left alone.
- **The tests of an imported file are not run.** Testing `main.zeno`
  would not run the tests of `ring.zeno`. They are that file's own.
- **The command fails if a test did**, so a script can ask.
- **`--only` chooses by what is in the name.** `zeno test --only
  "empty box"` runs the tests whose name has that text in it, as
  written, capitals and all. The file itself still runs whole first,
  since its tests may use what it made (17.5).

`zeno run` and `zeno build` go past the tests. `zeno run` and `zeno
check` still check them, so a test with a mistake in it is found
when the program is next checked, not when someone remembers to run
the tests. A build takes them out before it checks, so a test is not
held to the rules of a board (10.20):

```zeno
test "double" {
    let shared = [1, 2]
    let again = shared          # two names for one list
    assert double(len(shared) + len(again)) == 8
}
```

With `--no-collector` the program itself could not share that list.
The test may, because `zeno test` never runs it on the board. To hold
the tests to the build, run them in it: that is the next section.

## Tests of a compiled program: `zeno test --build`

```sh
zeno test --build ring.zeno                       # in the program C makes of it
zeno test --build --firmware --no-collector ring.zeno
zeno test --build --llvm examples/ring
zeno test --build --board boards/qemu-cortex-m3 ring.zeno
```

`--build` compiles each file with its tests in it, as `zeno build`
would compile it, runs the program, and prints what `zeno test`
prints, line for line: the same names, the same reports with the line
and the caret, and the same totals (decision 60.2). It takes
`--firmware`, `--no-collector`, `--llvm`, `--release`, `--pool`,
`--text-room`, `--memory` and `--board`. So the test of `double` above
is refused with `--no-collector`, as the program would be, and a test
that passes in `zeno test` and fails here says where the interpreter
and the build part.

- **Each test becomes a function** at the end of its file, with no
  parameters, that may raise any error. It sees what the file
  declares, and is checked with the rules of the build.
- **A failed `assert` leaves its test as `return` would**, freeing
  what the test owns. No `except` catches it, as in `zeno test`.
- **The reports wait until the last test**, in a room of 8 KB that
  only a test build has. One that does not fit is left out whole, and
  a line says how many were and that `zeno test` shows them all.
- **On a board** the program is written as `ring-tests.c` and built
  as `ring-tests.elf` beside the file, and the command says how to run
  it. The board prints the lines and the totals itself, and the
  program ends with 1 if a test failed.

One thing it refuses that `zeno test` runs: an `assert` in a function
written inside a test.

```
error: under `zeno test --build` an `assert` stands in the test itself, and not in a function written inside it. Let the function give back what it found, and `assert` that in the test
```

A failed `assert` leaves the test as `return` would, and from inside
another function that `return` would leave the wrong one. A firmware
build has no other way out of a function that raises nothing.

Where it differs from `zeno test`, on purpose:

| | `zeno test` | `zeno test --build` |
|---|---|---|
| A fault of a firmware build: an index out of range, an overflow | the test fails, and the next runs | the program stops, as any firmware program does; the command names the test it was in and how many were not run |
| `program.exit` in a test | the test fails | the program ends; the command says so as for a fault |
| An error before the tests | the snippet of the line | the compiled program's own message for an error nothing caught |
| A `try` that nothing in it can raise into, in a firmware build | runs | refused, as in any firmware program |

## How it is built

### The lexer and the parser

`assert` is a new reserved word (`src/token.rs`). `test` is not. In
`statement` in `src/parser.rs`:

```rust
TokenKind::Ident(word) if word == "test" && matches!(self.peek_at(1), TokenKind::Str { .. }) => self.test()?,
```

A name followed by text means nothing else in Zeno, so the parser
needs to look one token ahead and no more, and `test` stays free to
be a name (`examples/generics.zeno` has a parameter called `test`).

`test()` reads the name and the block. It refuses a test inside a
block, a name with `f` in front, and a name that the file has used
already. While it reads the block it sets `in_test`, which is how
`assert()` knows where it is:

```
error: `assert` belongs in a test: `test "name" { assert total == 6 }`. In
the program itself, check with `if`, and `raise` an error
```

The tree has three new statements (`src/ast.rs`): `Test { name,
body }`, `Assert(condition)` and `AssertRaises { error, body }`.

### The checker

A test is checked as a block at the top of the file. `assert` takes
a condition, checked by the function that checks the condition of an
`if`. `assert raises` is checked as a `try` with one `except` that
has nothing in it, so "is that a kind of error?" and "what does the
block raise?" are answered by code that was there:

```rust
StmtKind::AssertRaises { error, body } => {
    let handler = Handler { error: Some(error.clone()), binding: None, body: Vec::new() };
    self.try_statement(body, std::slice::from_ref(&handler), env, span);
    false
}
```

### Running them

`run_tests` in `src/interpreter.rs` is `zeno test`'s engine. It runs
the program, and then walks its statements once more:

```rust
for statement in &program.statements {
    let StmtKind::Test { name, body } = &statement.kind else { continue };
    if statement.span.file() != 0 { continue }
    let failure = interpreter.exec_block(body, &env).map_err(ops::escaped).err();
    ...
}
```

`span.file()` is which file of the joined program a statement came
from (docs/modules.md), and 0 is the first one: the file that is
being tested.

`assert` is in `Interpreter::assert`. For a comparison it works out
the two sides, keeps what each looks like, and compares them with the
function every comparison uses. A failure is a `Control::Fatal`, the
kind of stop that `except` does not catch.

Everywhere else a test is nothing. In `exec`, the line for `zeno
run` is:

```rust
StmtKind::Test { .. } => Ok(()),
```

### The compilers

`Program::without_tests` in `src/ast.rs` takes the tests out, and
`zeno build` calls it before the checker. The VM, the C backend and
the LLVM backend each have one line that says a test is nothing, for
the programs that reach them another way.

Under `zeno test --build`, `with_tests` in `src/test_build.rs` makes
each test of the file a function instead, and the C and LLVM
backends compile `assert` and `assert raises` in such a function: the
condition, and if it is false a report handed to the runtime of the
test build and a `return`. `src/test_build.rs` also holds that
runtime: the room for the reports, and the marks the command counts.

### The command

`test` in `src/main.rs` parses the file alone first, to see whether
it has a test. Then it loads the program the file belongs to.
`program_folder` finds where that is: the nearest folder upward with
a `zeno.package` or a `main.zeno` in it. The language server finds
it the same way, and it is why a file in `drivers/` can be tested
with the imports it has in the real program.

## What is left out

- `assert` in a function outside a test.
- Lines to run before and after each test.
- Running tests side by side.
- An `assert` in a function written in a test, under `--build`.
- Reading the totals back from a board: the board prints them.

## Things to try

1. Run `zeno test examples/ring`. Then break `put` in `ring.zeno`:
   leave out `self.count += 1`. Which tests fail, and what does each
   failure tell you about the mistake?

2. Write a test for `room()`: after two bytes go into a ring of
   four, there is room for two. Run it. Then make it wrong on
   purpose and read the message.

3. Make a file `ring_test.zeno` beside `ring.zeno` with `import
   ring` and a test that reads `queue.slots`. What does `zeno check`
   say, and why can the test inside `ring.zeno` do it?

4. Put `assert queue.size() == 0` at the top of `main.zeno`, outside
   any test. Read the message. How would the program itself say that
   something is wrong, in a way a caller could catch?

5. Write `try { assert 1 == 2 } except Error { print("caught") }`
   inside a test and run it. Is "caught" printed? Find the line in
   `Interpreter::assert` that makes it so, and `Control::Fatal` in
   `src/value.rs`.

6. Add a test to `examples/ring/main.zeno` and run `zeno test
   examples/ring` again. What is printed before the tests of that
   file, and why (10.19)?

7. Write a test that uses a `dict[str, list[int]]`. Run it with
   `zeno test`. Then build the file with `--no-collector`. Why does
   the build not complain (10.20)? Now use such a dict outside the
   test and build again.

8. Run `zeno parse` on a file with a test and find the three new
   lines in the tree. Then find where `printer.rs` writes each.

9. Two tests of one file change the same `var`. Does the order they
   are written in matter? Write them so that it does, and then so
   that it does not.

10. Give an `assert` a message: `assert total == 6, f"the total of
    {xs}"`. Make it fail, and read where the text goes. Then write
    `xs[99]` inside the text of an `assert` that passes. Why does
    nothing go wrong, and what happens when it fails? Find the lines
    in `Interpreter::assert` that decide it.

11. Run `zeno test --build examples/ring`, then with `--firmware
    --no-collector`. Is anything printed differently? Then add the
    test of `double` from this page to a file and run both again.
    Which refuses it, and why does `zeno test` not?

12. Write a test with `xs[9]` in it, for a list of two, and run it
    with `zeno test` and with `zeno test --build --firmware`. What
    does each print, and what does the second say about the tests
    after it? Find where `test_built` in `src/main.rs` reads the
    marks.

13. Write sixty tests that each fail, with a few lines of a script.
    How many reports does `zeno test --build` print, and what does
    the last line say? Find `ZENO_TEST_ROOM` in `src/test_build.rs`.
