# Releasing Zeno: one `zeno` for each kind of machine

Every page so far ran Zeno with `cargo run --`, which builds the
compiler from its source first. That is right for somebody working on
Zeno, and wrong for somebody who only wants to use it: they should not
need Rust at all. A release is the answer: a `zeno` that is already
built, for the machine they have, to download and run.

This page is about how a release is made. The decisions are chapter 12
of [`decisions.md`](decisions.md).

## What a release is

A release is a version number, the source as it was at that moment, and
one file to download for each kind of machine:

| Machine | File |
|---|---|
| Linux, x86-64: most PCs and servers | `zeno-0.6.0-linux-x86_64.tar.gz` |
| Linux, ARM64: a Raspberry Pi 3, 4 or 5 with a 64-bit system, ARM servers | `zeno-0.6.0-linux-arm64.tar.gz` |
| Linux, 32-bit ARMv7: a Raspberry Pi 2 or later with a 32-bit system | `zeno-0.6.0-linux-armv7.tar.gz` |
| Linux, 32-bit ARMv6: a Raspberry Pi 1 or Zero | `zeno-0.6.0-linux-armv6.tar.gz` |
| Linux, 32-bit x86 | `zeno-0.6.0-linux-x86.tar.gz` |
| macOS, Apple silicon | `zeno-0.6.0-macos-arm64.tar.gz` |
| macOS, Intel | `zeno-0.6.0-macos-x86_64.tar.gz` |
| Windows, x86-64 | `zeno-0.6.0-windows-x86_64.zip` |
| Windows, 32-bit x86 | `zeno-0.6.0-windows-x86.zip` |
| Windows, ARM64 | `zeno-0.6.0-windows-arm64.zip` |

Each file unpacks to a folder with `zeno` in it (`zeno.exe` on
Windows), this README, the `docs`, the `examples` and the `editors`
folder. `zeno` is one file that needs nothing else installed: put it
anywhere on the `PATH` and it works.

From 0.2.0 on the folder also has `zeno.wasm`, which is
Zeno built for a browser. `zeno build --web` writes it into the page
it makes ([`web.md`](web.md)), and looks for it beside `zeno`, so the
two are moved together. Nothing else needs it.

Beside them is `SHA256SUMS.txt`, with one line for each file. It is
how to know that a download arrived whole:

```sh
sha256sum --check --ignore-missing SHA256SUMS.txt
```

## Making one

Two lines make a release:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The tag is the letter `v` and the version in `Cargo.toml`. Pushing it
starts [`.github/workflows/release.yml`](../.github/workflows/release.yml)
on GitHub's machines, which does the rest:

1. It checks that the tag and `Cargo.toml` name the same version, so a
   `zeno` that says `0.1.0` cannot be released as `v0.2.0`.
2. On Linux it runs the whole test suite (`cargo test`). If a test
   fails, nothing is released. On macOS and Windows it runs the suite
   too, to see what it says there, and puts the count on the summary
   page of the run.
3. It builds `zeno` ten times, once for each row of the table.
4. It runs [`tools/release-check.sh`](../tools/release-check.sh) on
   each build (see below).
5. It packs each build with the docs and the examples.
6. When all ten are done, it makes the release from
   `docs/releases/0.1.0.md`, which is the text people read on the
   release page, and puts the ten files and `SHA256SUMS.txt` on it.

If any of the ten fails, there is no release: a release with a hole in
the table is worse than one a day later.

Since 0.2 the same run also makes the two installers
([`install.md`](install.md)) and the packages
([`packages.md`](packages.md)), tries each, and puts them on the
release with the ten. When the repository has a key for it, it adds
the signed list that apt reads, and its last step starts the workflow
that sends the new version to the stores ([`stores.md`](stores.md)).

The same workflow can be started by hand, from the Actions page of the
repository ("Run workflow"). Then it does steps 2 to 5 and releases
nothing. That is the way to find out whether `main` could be released,
without releasing it.

There is a box to tick there, "publish". With it the workflow does
step 6 as well, and makes the tag itself, named after the version in
`Cargo.toml`. That is a release from a web page, with no `git` at
hand: tick the box, press the button, and wait about six minutes.

## Keeping its benchmark times

Each release keeps a record of how fast it is, so that the history can
be seen later (the benchmarks page on zeno-lang.org draws it). After
the tag is pushed, on the machine the earlier versions were timed on:

```sh
cargo build --release
python3 bench/record.py --version 0.6.0
git add bench/results/versions/0.6.0.json
```

[`bench/record.py`](../bench/record.py) times every program with every
engine the version has, beside C, Rust and Python, and writes
`bench/results/versions/<version>.json`. The page shows each time as a
number of times C's time, taken in the same run, so a record made on a
different machine still lines up with the others. An older version can
be recorded at any time by building it from its tag in a `git
worktree` and passing `--zeno` to that build.

It is not a step of the release workflow: GitHub's machines are shared
and their times move by a quarter from one run to the next, more than
most of the changes the record is there to show (decisions 22.12).

## Keeping the history

[`HISTORY.md`](../HISTORY.md) lists every commit under the release it
went into, with one plain line for each, and zeno-lang.org shows the
same file at `/history/`. [`tools/history.py`](../tools/history.py)
writes it: the commits, their order and the releases come from git and
its tags, and the words come from
[`tools/history-notes.txt`](../tools/history-notes.txt). After the tag
is pushed, in a clone that has the whole history:

```sh
git fetch --tags
python3 tools/history.py --missing
```

The second line prints each commit that has no line in the notes yet.
Give each one a line there, rename the block `Unreleased` to the
version, and write its paragraph on what the release brought. Then:

```sh
python3 tools/history.py
git add HISTORY.md tools/history-notes.txt
git commit -m "History: 0.6.0"
```

The two files go in a commit of their own, which the page leaves out:
a commit cannot hold its own name (decisions chapter 54). If the notes
are not touched at all, the script still writes a whole page: the new
tag makes its heading, and a commit with no line is listed with the
first line of its message. `python3 tools/history.py --check` fails
when `HISTORY.md` is behind.

## As fast as the release before

A release that adds code is compared with the one before, on one
machine, before it goes out. The one before is built from its tag in a
`git worktree`, and then:

```sh
python3 bench/run.py --count --engines zeno,zeno-release,zeno-owned,zeno-fast,vm --zeno path/to/old/zeno
python3 bench/run.py --count --engines zeno,zeno-release,zeno-owned,zeno-fast,vm
python3 bench/compare.py --old path/to/old/zeno
```

The first two count instructions, which shows a change of one in a
hundred in what the program does. The third times the two builds in
rounds, which shows what a count cannot: the same instructions placed
worse. 0.5.0 went out with the count alone and its `zeno run --vm` was
a few in a hundred slower than 0.4.0's; 0.5.1 mended it (decision
24.29).

## Where each one is built

GitHub lends three kinds of machine: Linux on x86-64, macOS on Apple
silicon, and Windows on x86-64. Ten builds come out of those three
because Rust can build for a processor it is not running on:

- The Linux machine builds all five Linux rows. They are linked against
  **musl**, a small C library that is put into the program itself. A
  Linux program usually borrows the C library of the system it runs on,
  and then it only runs on systems with one at least as new. With musl
  the one `zeno` runs on every Linux, old or new, whatever is installed.
- The macOS machine builds both macOS rows.
- The Windows machine builds all three Windows rows. Those carry their
  C library too, so `zeno.exe` asks for no `.dll` to be installed.

How each is linked is in [`.cargo/config.toml`](../.cargo/config.toml).
Building Zeno for the machine you are on uses none of it.

## Checking a build before it goes out

A build that has never been run is a guess. So each build is run before
it is packed, by `tools/release-check.sh`:

```sh
tools/release-check.sh target/release/zeno
```

```
zeno 0.1.0
27 examples run, and print the same through both engines
5 tests: 5 passed, 0 failed.
every example is laid out as `zeno format` lays it out
`zeno new` makes a program that runs and whose tests pass
ok
```

Since 0.5 it also has `zeno build --target` write `examples/hello.zeno`
in each of fourteen languages, which needs none of them installed.

A build for another processor cannot run on the machine that built it.
For the ARM builds for Linux the script is given an emulator, a program
that acts as the other processor:

```sh
tools/release-check.sh target/aarch64-unknown-linux-musl/release/zeno qemu-aarch64
```

The Intel build for macOS runs on the Apple silicon machine, because
macOS translates it. The 32-bit builds run as they are, because a
64-bit x86 processor runs 32-bit programs.

One build is not run: Windows for ARM64. GitHub's Windows machine
cannot run it and there is no emulator there to give it to. It is built
by the same compiler from the same source as the other two Windows
builds, which are run, and the release page says that it was not.

## What a released `zeno` needs

For `zeno run`, `check`, `test`, `format`, `new` and `lsp`: nothing.

`zeno build` writes C and hands it to the machine's C compiler, so it
needs one: `cc`, or whatever `CC` names. `zeno build --llvm` needs
clang. `zeno fetch` needs git. The release does not bring them,
because they are different on every machine and most machines that
build programs have them.

On Windows the C compiler is MinGW's gcc, which `zeno build` finds by itself (15.5),
and `zeno build --llvm` uses the clang that comes with Visual Studio.
The program is `hello.exe`, run with `.\hello`.

## Line ends

Windows ends a line with two characters and everything else with one.
Zeno reads a program with either. `zeno format` writes one, on every
machine, so a file has one layout wherever it was formatted. The
repository says the same thing to git in `.gitattributes`, so the
examples are the same bytes on Windows as on Linux.

## What the first release found

Until this release Zeno had only ever run on one kind of machine:
Linux on x86-64. Running the tests somewhere else for the first time
found three things, all about compiled programs.

**A last digit.** `examples/sensor.zeno` turns Celsius into Fahrenheit
with `c * 1.8 + 32`. For 23.5 `zeno run` prints `74.30000000000001`,
and so did the compiled program, on x86-64. On a Mac with Apple
silicon the compiled program printed `74.3`.

Neither is wrong as arithmetic. A float cannot hold 42.3 exactly, so
the multiplication is rounded, and then the addition is rounded again.
ARM64 has one instruction that multiplies and adds and rounds once,
at the end. It is a little faster and a little closer to the true
answer, and a C compiler is allowed to use it without being asked.
But then the same program gives two answers, and which one depends on
the processor. Zeno's promise is that a compiled program prints what
`zeno run` prints, so the C that Zeno writes now begins by telling the
compiler to keep the two steps apart (decision 12.5). The same lines
mend 32-bit x86, where gcc had the same effect for a different reason.

**A line end.** On Windows a compiled program printed `"\r\n"` where
`zeno run` printed `"\n"`, because that is what C does there unless
it is told not to. It is told not to now (decision 12.6).

Both were found by a test that had always passed, and both were
mended before the release. A test only tests the machines it is run
on.

## What Windows found

Of the tests of Zeno itself, 337 of 454 passed the first time they
were run on Windows. Now all of them do. What was in the way, in the
order it was found (decisions 12.8 and 12.9):

**Waiting.** `zeno build --llvm` did not work at all. The runtime
waited with `nanosleep` and read the clock with `clock_gettime`, and
Microsoft's C library has neither. On Windows it now uses `Sleep` and
the performance counter:

```c
#ifdef _WIN32
static void zeno_board_sleep(double seconds) {
    fflush(stdout);
    Sleep((unsigned long)(seconds * 1000.0 + 0.5));
}
#else
    ... nanosleep ...
#endif
```

Behind that were three smaller things of the same kind: `setjmp`
takes a second argument there, the linker does not know `-lm`, and a
program is called `hello.exe`.

**A path.** Windows writes `drivers\uart.zeno` and takes
`drivers/uart.zeno` too. Zeno now writes every path it makes with
`/`, so a message is the same on every system, and so that the file
an editor has open is the file an `import` finds.

**A word in Tamil.** `program.arguments()` in a compiled program
gave `?????` for `தமிழ்`, because Windows hands `main` its words in
an older encoding. The runtime now asks for them the way Windows
keeps them.

**Line ends, again.** `zeno fetch` got a package with `\r\n` in
every file, because that is how git is set on Windows. It is told to
leave them.

## A trial on one machine

Each of those was tried on a real Windows machine before it was kept,
with `.github/workflows/trial.yml`. On the Actions page choose
"trial", press "Run workflow" and pick the machine. It builds Zeno,
runs `tools/release-check.sh`, compiles three examples both ways and
runs every test, and writes what it found at the top of the run's
page. It releases nothing, and costs a quarter of what a run of the
release workflow does, or less.

## Things to try

1. Build a release `zeno` for your own machine with `cargo build
   --release` and run `tools/release-check.sh target/release/zeno`.
2. Change the `print` in `examples/hello.zeno` and run the check again.
   Which line stops it, and what does it say?
3. Run `zeno run` on a file saved with Windows line ends (most editors
   can save one). Then run `zeno format --check` on it, and `zeno
   format`, and look at the file again.
4. Add the Rust target for another processor (`rustup target add
   aarch64-unknown-linux-musl`), build for it with `cargo build
   --release --target aarch64-unknown-linux-musl`, and ask `file` what
   the result is. What happens if you try to run it?
5. Read `.github/workflows/release.yml` and find the line that stops a
   release when the tag and `Cargo.toml` disagree.
6. `print(23.5 * 1.8 + 32)` prints `74.30000000000001`. Work out by
   hand what the answer "should" be, and then explain to yourself why
   Zeno prints the longer number on purpose. Build the program with
   `zeno build` and look in the `.c` file for the lines that begin
   `#pragma`.
