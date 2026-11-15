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
| Linux, x86-64: most PCs and servers | `zeno-0.1.0-linux-x86_64.tar.gz` |
| Linux, ARM64: a Raspberry Pi 3, 4 or 5 with a 64-bit system, ARM servers | `zeno-0.1.0-linux-arm64.tar.gz` |
| Linux, 32-bit ARMv7: a Raspberry Pi 2 or later with a 32-bit system | `zeno-0.1.0-linux-armv7.tar.gz` |
| Linux, 32-bit ARMv6: a Raspberry Pi 1 or Zero | `zeno-0.1.0-linux-armv6.tar.gz` |
| Linux, 32-bit x86 | `zeno-0.1.0-linux-x86.tar.gz` |
| macOS, Apple silicon | `zeno-0.1.0-macos-arm64.tar.gz` |
| macOS, Intel | `zeno-0.1.0-macos-x86_64.tar.gz` |
| Windows, x86-64 | `zeno-0.1.0-windows-x86_64.zip` |
| Windows, 32-bit x86 | `zeno-0.1.0-windows-x86.zip` |
| Windows, ARM64 | `zeno-0.1.0-windows-arm64.zip` |

Each file unpacks to a folder with `zeno` in it (`zeno.exe` on
Windows), this README, the `docs`, the `examples` and the `editors`
folder. `zeno` is one file that needs nothing else installed: put it
anywhere on the `PATH` and it works.

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

The same workflow can be started by hand, from the Actions page of the
repository. Then it does steps 2 to 5 and releases nothing. That is the
way to find out whether `main` could be released, without releasing it.

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

On Windows the C compiler is MinGW's gcc, named with `set CC=gcc`,
and `zeno build --llvm` does not work yet (decision 12.7 says why).

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
mended before the release. A third thing was found and left for the
next one: `zeno build --llvm` on Windows (decision 12.7). A test only tests
the machines it is run on.

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
