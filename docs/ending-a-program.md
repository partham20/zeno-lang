# Ending a program, its environment, and the error stream

A program that other programs call has to talk to them in three ways
besides what it prints. It ends with a number that says how it went.
It reads settings from the environment it was started in. And it
writes complaints where they do not get mixed into its real output.
The module `program` has had `arguments` and `run`; now it has these
three as well.

```zeno
import files
import program

let words = program.arguments()
if len(words) != 1 {
    program.warn("usage: count <file>")
    program.exit(2)
}

let verbose = program.environment("COUNT_VERBOSE") == "yes"
if verbose {
    program.warn(f"reading {words[0]}")
}
print(len(files.read(words[0]).lines()))
```

| Call | Gives | Raises |
|---|---|---|
| `program.exit(code)` | nothing: the program ends here, with `code` | `ValueError` if `code` is not from 0 to 255 |
| `program.environment(name)` | `str or none` | never |
| `program.warn(text)` | nothing: one line on the error stream | never |

The reasons for each choice are in chapter 34 of
[`decisions.md`](decisions.md).

## `program.exit(code)`

```
$ zeno run count.zeno
usage: count <file>
$ echo $?
2
```

0 says that all went well. Any other number, up to 255, says that
something did not, and what each one means is for the program to
decide and say. A script can then act on it:

```sh
zeno run count.zeno notes.txt || echo "count failed"
```

Three things to know.

**Nothing stops it.** Not a `try`, and not an `except` with no kind
after it. Ending is not an error, so there is nothing to catch:

```zeno
try {
    program.exit(3)
} except {
    print("never printed")
}
```

**What was printed is not lost.** Every engine sends the program's
output on its way before it ends, even when it was waiting to be
sent in one large piece (which every target language does, for
speed).

**A number from 0 to 255.** That is what every system hands on, and on
Linux `exit(256)` would arrive as 0: a failure would look like a
success. Outside that, it is a `ValueError` where the call is. A
number written down outside it is found before the program runs:

```
error: a program ends with a number from 0 to 255, not 300
 --> tool.zeno:3:14
  |
3 | program.exit(300)
  |              ^
```

A function that promises an `int` still returns one on every path. The
checker does not treat `program.exit` as the end of a path, as Java and
Go do not, so a reader never has to know which calls end the program
to see that a function returns.

## `program.environment(name)`

Every program is started with an environment: names and their text,
such as `HOME`, `PATH`, `LANG`, set by the shell or by the program
that started it. Tools read settings from it that nobody wants to
type each time.

```zeno
let home = program.environment("HOME")
if home is none {
    print("no home folder")
} else {
    print(f"home is {home}")
}
```

It gives `str or none`, and the checker makes the program look at the
`none` before it uses the text, as it does for `input()`. A variable
that is set but empty gives `""`, which is not `none`.

```
$ COUNT_VERBOSE=yes zeno run count.zeno notes.txt
```

It only reads. A firmware build refuses it: a board has no environment,
and a value the build needs is written in the program as a `const`.

## `program.warn(text)`

A program has two streams to write on: the output, where `print`
writes, and the error stream. A shell keeps them apart, so that

```
$ zeno run count.zeno notes.txt > count.txt
reading notes.txt
```

puts the count in `count.txt` and still shows the warning on the
screen. A program whose output is read by another program should
write everything that is not that output with `warn`.

It takes one `str` and adds a line end. An f-string makes anything
into one: `program.warn(f"{missing} files were not there")`.

What was printed before it is sent first, so that on a screen that
shows both, the lines come in the order the program wrote them.

## Where it works

| | `exit` | `environment` | `warn` |
|---|---|---|---|
| `zeno run`, `zeno run --vm` | yes | yes | yes |
| `zeno build`, `--llvm`, `--release`, `--fast` | yes | yes | yes |
| `zeno build --minimal` | yes, with no check of the number | yes | yes |
| `zeno build --firmware`, `--no-collector` | yes | refused | yes |
| `zeno build --target` (all fifteen) | yes | yes | yes |
| a page (`--web`, `--target html`) | ends the turn; the page stays | always `none` | shown with the rest |
| `zeno test` | the test fails | yes | yes |

## How it is built

Follow `program.exit(3)` through the engines.

**1. The checker** (`checker.rs`). `program.exit` takes an `int` and
gives nothing. If the argument is a number written down, it must be
from 0 to 255.

**2. `zeno run` and the VM** (`builtins.rs`, `value.rs`). The built-in
checks the number and returns `Err(Control::Exit(3, span))`. `Control`
is how both engines carry `return`, `break` and a raised error up
through the Rust functions that run the program, so `Exit` goes up
the same way. A `try` looks only for `Control::Raise`, so it lets
`Exit` pass. At the top, `Ran::ended` turns it into a run that went
well, with status 3, and `main.rs` hands 3 to the shell.

```rust
pub enum Control {
    Break(Span),
    Continue(Span),
    Return(Value, Span),
    Raise(Value, Span),
    Fatal(Error),
    Exit(u8, Span),
}
```

**3. C and LLVM** (`runtime.c`). One function, which both backends call:

```c
static void zeno_program_exit(int64_t code, int line, int col) {
#ifndef ZENO_MINIMAL
    if (code < 0 || code > 255) {
        zeno_fail("ValueError", ..., line, col);
    }
#endif
    exit((int)code);
}
```

C's `exit` sends what `printf` was holding and runs the functions the
program asked to run at its end, such as the memory report of
`--memory`.

**4. The target languages** (`runtime.*`). One function each, under
the name `program.exit` in the table of built-ins. Each sends the
waiting output, then ends in a way that language's own `except` or
`catch` cannot stop: `os._exit` in Python, `exit!` in Ruby,
`process.exit` in Node, `System.exit` on the JVM, `os.Exit` in Go,
`std::process::exit` in Rust, `std::_Exit` in C++. In a page, where
there is nothing to end, JavaScript throws an `Exit` that only the
runtime's own `start` catches.

## How it is tested

- `cargo test`: `a_program_can_end_with_a_number_of_its_choosing`,
  `a_warning_goes_on_the_error_stream`,
  `the_environment_of_the_program` and
  `a_test_that_ends_the_program_fails` run each program in both
  engines, which must agree on everything (`run_both_ending`).
  `a_program_ends_warns_and_reads_its_environment_alike_in_every_build`
  builds one program in C and through LLVM and holds the output, the
  warnings and the number to the engines', and tries a firmware build.
- `tools/targets.py`: `tools/targets/edge-ending.zeno` and
  `edge-ending-zero.zeno` in all fifteen languages. The script now
  compares the number each program ended with, and the lines it wrote
  on the error stream, as well as what it printed.

## Things to try

1. Write `grep.zeno`, which prints the lines of a file that contain a
   word, and ends with 0 if it printed any and 1 if not, as `grep`
   does. Try it in a shell with `&&` and `||`.
2. Give the program an `exit` of its own: `function exit() { ... }`.
   Does it clash with `program.exit`? Why not?
3. Run a program with `NAME=` in front, and with `NAME` not set at
   all. Which one gives `none`, and which gives `""`?
4. Redirect a program's output to a file with `>` and its error stream
   with `2>`. Which file did each `warn` go to?
5. Write a function that promises an `int` and ends with
   `program.exit(1)` on its last line. What does the checker say, and
   what is the shortest way to satisfy it?
6. Build the program from item 1 with `--target py` and with
   `--target go`, and check with `echo $?` that each ends with the
   same number.
7. Use `program.environment` in a program, and build it with
   `--firmware`. Read the message. What would you write instead?
