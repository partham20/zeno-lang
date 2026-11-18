# Starting a program: `zeno new`

Every page so far began with a file that was already there. This one
is about the first minute of a program that is not: one command makes
a folder with a small program in it that runs, and has a test that
passes.

```sh
zeno new blinker
```

```
Made the program `blinker` in blinker/
  zeno.package   the name of the program, and the packages it needs
  main.zeno      the program
  greeting.zeno  a module, with a test
Run it with `zeno run blinker/main.zeno`, and its tests with `zeno test blinker`.
```

```sh
zeno run blinker/main.zeno      # Hello, world, from blinker.
zeno test blinker               # 1 test: 1 passed, 0 failed.
```

(In this repository `zeno` is `cargo run --`, as on every other
page.) The decisions are 10.22 to 10.24 in
[`decisions.md`](decisions.md).

## What it writes

Three files, and each one is one of the three things a program of
more than one file is made of.

`main.zeno` is where the program starts ([`modules.md`](modules.md)):

```zeno
# The program starts here. In this folder:
#
#     zeno run main.zeno
#     zeno test
import greeting

print(greeting.hello("world"))
```

`greeting.zeno` is a module, a file of the program that another file
imports, and it has a test at its end ([`testing.md`](testing.md)):

```zeno
# A module: one file of the program. `main.zeno` imports it, and calls
# `greeting.hello`.

function hello(who: str) -> str {
    return f"Hello, {who}, from blinker."
}

# A test stands in the file it tests. `zeno test` runs it, `zeno run`
# goes past it, and a build leaves it out.
test "hello says who it greets" {
    assert hello("Ada") == "Hello, Ada, from blinker."
}
```

`zeno.package` is the project file ([`packages.md`](packages.md)). It
has one line that counts, and comments that show the three lines you
may add to it one day ([`registry.md`](registry.md)):

```
# What this program is called, and the packages it needs.
#
# A package in another folder is one line:
#
#     needs filters in "../filters"
#
# One from the registry is named by its name and a version of it.
# `zeno search` lists what there is, and `zeno fetch` gets it:
#
#     needs units at "v1.2.0"
#
# One that is in no registry is named by its git repository:
#
#     needs units from "https://github.com/someone/zeno-units" at "v1.2.0"
package blinker
```

Nothing else is written. There is no hidden folder, no settings file
and nothing to delete before you start. The program is what you would
have typed, and you go on by changing it: rename `greeting.zeno`, or
throw it away and write in `main.zeno` alone.

## A package: `--package`

A package is a folder of modules for other programs to use. It has no
`main.zeno`, and the module that matters is the one with the
package's own name, because that is the file `import filters` reads.

```sh
zeno new --package filters
```

```
Made the package `filters` in filters/
  zeno.package   the name of the package, and the packages it needs
  filters.zeno   the module `import` gets, with a test
Run its tests with `zeno test filters`.
```

`filters.zeno` has the same `hello` and the same test. The comment at
the top of its `zeno.package` says what a program has to write to use
it:

```
# This file makes the folder a package: a folder of modules that a
# program in another folder can use. A program that needs it says so
# in its own `zeno.package`:
#
#     needs filters in "../filters"
#
# and then writes `import filters`.
package filters
```

So with both folders side by side, `blinker` can use `filters` after
two edits: the `needs` line in `blinker/zeno.package`, and `import
filters` in `blinker/main.zeno`.

## What it refuses, and what it says

| You write | Zeno says |
|---|---|
| `zeno new my-app` | `my-app` cannot be the name of a program. A name is letters, digits and `_`, and does not start with a digit, as every name in a Zeno program is. Try `my_app` |
| `zeno new while` | `while` is a word of the language, so a program cannot have that name |
| `zeno new --package math` | `math` is a module that comes with Zeno, so a package cannot have that name |
| `zeno new blinker`, a second time | there is a `blinker` here already. `zeno new` makes a new folder, and writes nothing into one that is there |
| `zeno new --lib filters` | `zeno new` has no option `--lib`. It has one, `--package`, as in `zeno new --package filters` |
| `zeno new` | `new` needs a name for the program, e.g. `zeno new blinker` |

The name is the last part of what you write, so `zeno new
work/bench/logger` makes the program `logger` in that folder, and the
folders above it if they are not there.

## How it works

`new` in [`src/package.rs`](../src/package.rs) is the whole of it,
about a hundred lines, in four steps.

1. **The name.** The last part of the path. It is checked the way the
   lexer would check it: letters, digits and `_`, not a digit first.
   Then two tables that the compiler already has are asked:
   `token::keyword` (is it a word of the language?) and
   `builtins::comes_with_zeno` (is it `math`, `time`, ...?). When the
   name will not do, a name that will is worked out by putting `_`
   for each character that cannot be in one.
2. **The folder.** If anything is at the path, a file or a folder,
   `new` stops. This is checked before anything is written.
3. **The files.** Each is a `format!` with the name put in. A package
   differs from a program in a list of two files in place of three.
4. **What it says.** The lines it gives back are what `main.rs`
   prints. `new` prints nothing itself, which is what lets a test
   call it and look at what it said.

The test (`zeno_new_makes_a_program_that_runs_and_a_package_it_can_need`
in `src/main.rs`) does what you would do by hand. It makes a program
and a package in a temporary folder, runs the program and reads what
it prints, runs the tests of both, and checks that each file is
already laid out the way `zeno format` would lay it out. Then it
makes the program need the package and runs it again. What `zeno new`
writes is therefore checked by the checker, the interpreter and the
formatter every time the compiler's tests run: if the language
changes under it, that test fails.

## What is left out

- **No templates.** There is one program and one package. A program
  for a board is the same program with other lines in `main.zeno`.
- **No questions.** The command asks nothing and reads nothing from
  the keyboard.
- **Nothing into a folder that is there.** There is no `zeno init`.
- **No git.** No repository is made and no `.gitignore` is written.
  `packages/` is the one folder worth ignoring, and it is only there
  after `zeno fetch`.
- **No `zeno add`.** A `needs` line is written by hand.

## Things to try

1. Run `cargo build`, then in an empty folder outside the repository
   run `zeno new blinker` (with the full path to `target/debug/zeno`).
   Run the program and its tests.
2. In `blinker/greeting.zeno`, change the text `hello` gives back and
   run `zeno test blinker`. Read what the failure shows on each side.
   Which of the two do you change to make it pass, and why does it
   depend on what you meant?
3. Run `zeno format --check blinker`. It says nothing. Find the test
   in `src/main.rs` that promises this, and the three files it names.
4. Run `zeno new --package filters` beside `blinker`. Make `blinker`
   use it: one line in `blinker/zeno.package`, and `import filters`
   in `main.zeno`. Both modules have a `hello`. Why is that no clash?
   (Decision 8.8.)
5. Run `zeno build --no-collector blinker/main.zeno` and run what it
   built. The module has a test in it. Where did the test go?
   (Decision 10.20.)
6. Try `zeno new 9lives`, `zeno new for` and `zeno new time`. Each is
   refused for a different reason. Find the three checks in `new`.
7. Run `zeno new blinker` again, then `ls blinker`. Nothing changed.
   In `new`, move the `path.exists()` check below
   `fs::create_dir_all`, run the tests, and read which line of the
   test catches it. Put it back.
8. Add a file to what a new program gets: a `README.md` that says how
   to run it. You need one more entry in the list of files, and the
   test's list of file names will tell you it noticed.
