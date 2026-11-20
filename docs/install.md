# Installing Zeno: a setup program, and one line

A release of Zeno is an archive for each kind of machine
([`release.md`](release.md)). To use one, somebody has to know which
of ten files is theirs, unpack it, find a folder for `zeno`, put that
folder on the PATH, and then find out, from an error, that `zeno
build` wanted a C compiler. Each step is small, and each is a place to
give up.

So there are two installers that do all of it.

**On Windows**, a setup program, as Python has:

```
zeno-0.8.0-windows-setup.exe
```

Run it and press Next. It is the same file for every Windows machine.

**On macOS and Linux**, one line in a terminal, as Rust has:

```sh
curl -fsSL https://github.com/zeno-src/zeno-lang/releases/latest/download/install.sh | sh
```

Both do the same five things:

1. Work out which `zeno` this machine needs.
2. Put it, `zeno.wasm`, the docs, the examples and the editor setup in
   a folder that belongs to the user. Nothing asks for an
   administrator's password.
3. Add that folder to the PATH, so that `zeno` works in any terminal
   opened afterwards.
4. Look for the programs that parts of Zeno hand work to, and offer
   to install the ones that are missing.
5. Leave a way to take all of it off again.

The archives are still there, for anyone who would rather do it by
hand. The decisions are chapter 15 of [`decisions.md`](decisions.md).

## What Zeno needs, and what it does not

`zeno` is one file that needs nothing else to check, run, test and
format a program. Three commands hand part of their work to another
program:

| Command | Hands work to |
|---|---|
| `zeno build` | a C compiler: `cc`, `gcc` or `clang` |
| `zeno build --llvm` | `clang` |
| `zeno fetch` | `git` |
| `zeno build --web` | nothing, but `zeno.wasm` must be beside `zeno` |

That is the whole of "whatever is needed", and the installers offer
it: a C compiler and git. They ask first. A compiler is a few hundred
megabytes that somebody who only wants `zeno run` does not need, and
it is installed by the system's own package manager, which on Linux
wants a password.

`zeno build` used to want `set CC=gcc` on Windows, which has no `cc`.
It now tries `cc`, then `gcc`, then `clang`, and uses the first that
is installed. `CC` still names another.

## The setup program for Windows

It installs into `%LOCALAPPDATA%\Programs\Zeno`, which is where
Windows puts programs that are installed for one user. Its pages:

1. **Where to install.** The folder above, unless another is chosen.
2. **What to do besides.**
   - *Add Zeno to the PATH.* Ticked. Python's installer leaves its
     own box unticked, and "python is not recognized" is the first
     thing many people see of Python.
   - *Install a C compiler.* Shown only if this machine has none.
   - *Install Git.* Shown only if this machine has none.
3. **Install.**

The two tools are installed by **winget**, the package manager that
comes with Windows 10 and 11: the compiler is WinLibs (gcc, and clang
too, so `zeno build --llvm` works as well), and Git is Git for
Windows. On a machine without winget the two boxes are not shown.

In the Start menu it leaves a folder, Zeno, with four things: **Zeno
Prompt** (a terminal in which `zeno` works even if the PATH was left
alone), the examples, the docs, and **Uninstall Zeno**. Zeno is also
in the list of installed apps in Windows' settings, where it can be
removed like any other. Uninstalling removes the folder and takes it
out of the PATH. The compiler and Git are programs of their own and
stay.

A newer version is installed over an older one: the setup program
finds the folder by itself.

### One file for three processors

The setup program holds three `zeno.exe`: for x86-64, for 32-bit x86
and for ARM64. It asks Windows which processor it is on and installs
that one. Zeno is small enough (a few megabytes each) that three
in one file is easier than three files and a question.

### Windows will warn about it

The first time it is run, Windows says "Windows protected your PC",
with a "More info" link that leads to "Run anyway". Windows shows
this for every program whose maker has not bought a certificate to
sign it with, until enough people have run it. Nothing is wrong with
the file. A certificate costs money every year, and is not worth
buying for a language that is being learnt on (15.6).

### How it is made

[`tools/installer/zeno.iss`](../tools/installer/zeno.iss) is a script
for **Inno Setup**, a free program that turns such a script into a
setup `.exe`. Most of it is lists: which files go where, which boxes
to show, which shortcuts to make. The part that is code is at the end,
in Pascal, and does three things the lists cannot: choose the
`zeno.exe` for this processor, look for `gcc` and `git` on the PATH,
and add a folder to the PATH and take it out again.

The PATH of a user is one piece of text in the registry, under
`HKEY_CURRENT_USER\Environment`, with `;` between the folders.
`AddToPath` reads it, adds the folder at the end unless it is there,
and writes it back. `RemoveFromPath` puts a `;` at each end of the
text, so that every folder in it reads `;folder;`, and cuts out the
one that matches. Setup then tells the programs that are running that
the environment has changed, which is why a terminal opened after it
finds `zeno` and one that was already open does not.

```
tools\installer\build.cmd 0.8.0 files       makes zeno-0.8.0-windows-setup.exe
pwsh tools\installer\try.ps1 zeno-0.8.0-windows-setup.exe
```

`try.ps1` installs without showing a window, checks that `zeno` is
there, runs and is on the PATH, that it builds a page, that installing
twice leaves the PATH with one entry, and that uninstalling leaves
nothing. The release workflow runs it on the setup program before
releasing it.

## The line for macOS and Linux

```sh
curl -fsSL https://github.com/zeno-src/zeno-lang/releases/latest/download/install.sh | sh
```

`curl` downloads [`tools/install.sh`](../tools/install.sh), which is a
file of every release, and `sh` runs it. It is a shell script of about
three hundred lines, half of them comments, written for the plainest
shell there is, and it is meant to be read before it is trusted.

```
Zeno 0.8.0 for macos on arm64
  from https://github.com/zeno-src/zeno-lang/releases/download/v0.8.0
  the download is whole
  installed zeno 0.8.0 in /Users/partha/.zeno
  added it to the PATH, in /Users/partha/.zshrc

Zeno is installed. Open a new terminal, so that it is on the PATH, and try it:

    zeno run /Users/partha/.zeno/examples/hello.zeno
    zeno new first
```

What it does, in order:

1. **Which machine.** `uname -s` says Linux or Darwin (macOS), and
   `uname -m` the processor. A Raspberry Pi with the 32-bit system on
   a 64-bit processor says `aarch64` and gets the 32-bit Zeno, because
   that is what its system runs.
2. **The download**, with `curl` or `wget`, of the archive and of
   `SHA256SUMS.txt`.
3. **The check.** The archive's checksum must be the one the release
   lists. If it is not, nothing is unpacked.
4. **The install**, into `~/.zeno`: `bin/zeno`, `bin/zeno.wasm`, and
   the docs, examples and editors beside `bin`. An older Zeno there is
   replaced. A folder that holds something else is refused, so that a
   wrong `ZENO_HOME` cannot empty it.
5. **The PATH.** One line at the end of the startup file of the shell
   in use: `.zshrc`, `.bashrc` (`.bash_profile` on macOS),
   `config.fish`, or `.profile`. The line ends with a comment, `# added
   by the Zeno installer`, which is how the script knows not to add it
   twice and how `--uninstall` finds it.
6. **The tools.** On Linux, if there is no `cc`, `gcc` or `clang`, it
   says so and asks whether to install one with the system's package
   manager (`apt-get`, `dnf`, `yum`, `pacman`, `zypper` or `apk`), and
   the same for `git`. On macOS both come with Apple's command line
   tools, and it offers `xcode-select --install`. The question is
   asked of the terminal, not of the script's input, because with
   `curl ... | sh` the input is the script.

On macOS this also avoids a step the archives need: a file that a
browser downloads is held back by macOS until `xattr -d
com.apple.quarantine` is run on it, and a file that `curl` downloads
is not.

```sh
sh install.sh --yes          # install the missing tools without asking
sh install.sh --no-tools     # do not look for them
sh install.sh --uninstall    # remove ~/.zeno and the PATH line
curl -fsSL .../install.sh | sh -s -- --yes     # options, when piped
```

`ZENO_VERSION` installs another version, `ZENO_HOME` into another
folder, and `ZENO_FROM` from another place, which can be a folder.
That last one is how it is tried:
[`tools/try-install.sh`](../tools/try-install.sh) packs the `zeno`
that was just built as a release would, installs it from that folder
into a home folder of its own, and checks the same things `try.ps1`
does, and that a download with one byte added is refused.

## In the release

[`release.yml`](../.github/workflows/release.yml) has a job after the
ten builds, `windows-setup`, that takes the three Windows builds,
makes the setup program, and tries it. The Linux and macOS builds that
run on their own processor run `try-install.sh`. If either fails,
there is no release.

A release then has the ten archives, `zeno-0.8.0-windows-setup.exe`,
`install.sh` and their checksums, and with them the ten packages of
[`packages.md`](packages.md).

`.github/workflows/installer.yml` tries the installers on one machine
without a release, and keeps the setup program it made for a week, to
download and run by hand.

## For a version that was released before

Zeno 0.1.0 was released the day before there were installers. It has
them too, added afterwards by
[`add-installers.yml`](../.github/workflows/add-installers.yml), which
is started by hand with a version:

1. It downloads the three Windows archives of that release, checks
   them against the release's `SHA256SUMS.txt`, and makes the setup
   program from the three `zeno.exe` in them. Nothing is built again:
   what the setup program installs is what was released.
2. It takes today's `install.sh`, writes that version into its
   `version=` line, and has it install the released Linux archive into
   a home folder of its own.
3. Only if both passed, and "add" was ticked, it adds the two files to
   the release, and two lines to its `SHA256SUMS.txt`. The lines of
   the archives stay as they were.

Two things differ for 0.1.0, and the installers allow for both.
It has no `zeno.wasm`, so the setup program leaves that out and
`try.ps1` does not build a page. And its `zeno build` runs `cc` and
nothing else, so on Windows it still wants `set CC=gcc` first: the
box that offers the compiler says so.

So each release has its own `install.sh`, at
`releases/download/v0.1.0/install.sh`, which installs that version.
`releases/latest/download/install.sh` is the one of the newest.

## What it does not do yet

- **The setup program is not signed**, and neither is `zeno` on
  macOS. Windows warns once (above).
- **The winget step has not been run by a test.** GitHub's Windows
  machines have no winget, and already have a compiler and Git, so
  there the two boxes are not shown. Everything else in the setup
  program is run before each release.
- **Packages for package managers** (apt, pacman, snap, flatpak,
  Homebrew, winget) came next, and are in [`packages.md`](packages.md).
- **No `.pkg` for macOS.** An unsigned one is refused by macOS more
  firmly than the script is.
- **A machine that cannot reach GitHub** cannot use the line. Put
  `install.sh`, `SHA256SUMS.txt` and the archive for the machine in
  one folder, by whatever way there is, and run `ZENO_FROM=. sh
  install.sh` there.
- **`zeno` does not update itself.** Running the installer again
  installs the newest version over the old one.

## Things to try

1. Read `tools/install.sh` from top to bottom before running it.
   Which lines change something outside `~/.zeno`? Is that all of
   them?
2. Run `tools/try-install.sh target/release/zeno`. Then break
   something in `install.sh` (take out the checksum comparison) and
   run it again. Which check catches it?
3. `ZENO_HOME=$HOME sh install.sh` is refused. Find the line. What
   would have happened without it? What does the same line allow that
   it perhaps should not?
4. The PATH line is added to one startup file. Somebody uses zsh on
   Monday and bash on Tuesday. What happens? What would rustup do
   (it writes a file of its own and has each startup file read it)?
   Is that better?
5. In `zeno.iss`, `RemoveFromPath` puts a `;` at each end of the text
   first. Work through it for a PATH that is exactly the Zeno folder,
   for one where the folder is first, and for one where it is last.
   Then find the case it gets wrong: a PATH in which somebody wrote
   the folder with a `\` at its end.
6. Add a box to the setup program that opens `docs\web.md` when it
   finishes. It is one line in `[Run]`.
7. A big one: write a Homebrew formula for Zeno. It is a Ruby file
   that names the two macOS archives and their checksums. Where does
   it have to live for `brew install zeno` to find it, and what would
   the release workflow have to do at each release?
