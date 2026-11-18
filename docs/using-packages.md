# Using packages: modules from another folder

[`modules.md`](modules.md) put a program into several files. This page
lets a program use files that are not its own: a **package** is a
folder of modules that lives somewhere else, beside the program or in
somebody else's git repository.

A package can also be found by its name alone, in the registry. That
came later and has a page of its own, [`registry.md`](registry.md);
this page is what the registry is built on. (Zeno as a package for
apt, Homebrew and the rest is another matter:
[`packages.md`](packages.md).)

```sh
cargo run -- run examples/thermostat/main.zeno
```

```
read  18.2  smoothed  18.2  heater On
read  18.9  smoothed  18.5  heater On
...
read  23.1  smoothed  22.2  heater Off
mean 20.44, heater on for 7 of 10 minutes
```

Two folders in `examples/` make this program:

```
examples/thermostat/         the program
    zeno.package             its name, and what it needs
    main.zeno                the file the program starts in
    control.zeno             the module `control`, a file of the program
examples/filters/            a package
    zeno.package             its name
    filters.zeno             the module `filters`
    lowpass.zeno             the module `filters.lowpass`
    main.zeno                a small program that tries the package
```

The decisions are 8.1 to 8.11 in [`decisions.md`](decisions.md).

## The project file

A folder is a package when it has a file called `zeno.package` in it.
This is the thermostat's:

```
# What this program is called, and the packages it needs.
package thermostat

needs filters in "../filters"
```

These are the lines it can have, and that is all the file can say:

| Line | Means |
|---|---|
| `package thermostat` | the name of this package. The first line that is not a comment |
| `needs filters in "../filters"` | it needs the package in that folder. The folder is counted from this file's folder. Nothing is copied |
| `needs units from "address" at "v1.2.0"` | it needs the package in that git repository, at that tag, branch or commit. `zeno fetch` gets it |
| `needs units at "v1.2.0"` | it needs that version of the package the registry calls `units` ([`registry.md`](registry.md)) |
| `about "Filters for noisy readings"` | what the package is for, in one line. A package needs it to be published |
| `registry "address"` | names are looked up in that registry, and not in Zeno's own |
| `memory owned` | how the program frees memory ([`memory-modes.md`](memory-modes.md)) |

A program with no `zeno.package` is still a program: `examples/station`
has none. The file is there from the day the program needs a package.

## Importing from a package

```zeno
import filters              # filters.zeno in the package's folder
import filters.lowpass      # lowpass.zeno in the package's folder
import control              # control.zeno, a file of the thermostat

let smooth = filters.lowpass.start(0.4, readings[0])
print(filters.mean(readings))
```

The rule has one step more than the rule for modules:

1. **If the first word of the import is a package this file's
   `zeno.package` needs**, the rest is a file in that package's
   folder. `import filters.lowpass` is `lowpass.zeno` there. `import
   filters` with nothing after it is the file with the package's own
   name, `filters.zeno`.
2. **Otherwise** it is a file of the package the importing file is
   in. For `main.zeno` that is the thermostat's own folder.

Everything else is as it was. What is written at the import is
written at the use. `from filters.lowpass import Lowpass` brings in
one name. A type is `filters.lowpass.Lowpass`.

**Inside the package the same rule holds, from the package's own
side.** `lowpass.zeno` wants `clamp` from its neighbour, and writes:

```zeno
from filters import clamp       # filters.zeno, in this package's folder
```

It does not matter which program is using the package. A package sees
its own files and the packages its own `zeno.package` needs, and
nothing of the program that needs it. So a package can also be run on
its own:

```sh
cargo run -- run examples/filters/main.zeno
```

## Other people's packages: `zeno fetch`

A package that somebody else keeps is named by where its git
repository is, and by a version of it:

```
package logger

needs units from "https://github.com/someone/zeno-units" at "v1.2.0"
```

`at` is a tag, a branch or a commit. It is one exact thing: there are
no ranges such as `^1.2` (decision 8.4). Then, in the program's
folder:

```
$ zeno run main.zeno
error: the package `units` has not been fetched: there is no file
`packages/units/zeno.package`. Run `zeno fetch` in the folder of the program
 --> zeno.package:3:1

$ zeno fetch
Fetched units v1.2.0 (1c4e326) into packages/units
Fetched prefix main (d4461d9) into packages/prefix
Wrote zeno.lock

$ zeno run main.zeno
42.0
```

Three things to see:

- **Running never fetches.** `zeno run`, `check` and `build` read
  files and nothing else. Only `zeno fetch` uses the network, and it
  does so with the `git` that is on the machine (8.9).
- **`units` needed `prefix`, and that came too.** A fetched package
  has its own `zeno.package`, and what it needs is fetched as well.
  All of them go into one folder, `packages/`, beside the program's
  `zeno.package`. There is one package of each name in a program
  (8.7).
- **`zeno.lock` was written.** It holds the commit each package was
  at:

  ```
  prefix from "https://.../zeno-prefix" at "main" commit "d4461d9976d7..."
  units from "https://.../zeno-units" at "v1.2.0" commit "1c4e3266dcc4..."
  ```

  `prefix` follows the branch `main`, and a branch moves. The next
  `zeno fetch`, here or on another machine, still gets `d4461d9`,
  because the lock says so. `zeno fetch --update` looks every `at` up
  again and writes the lock afresh (8.10). Keep `zeno.lock` with the
  program. The folder `packages/` can be made again from it at any
  time.

## What can go wrong, and what Zeno says

```
error: this does not say which version of `units` is needed. Add `at` and a tag,
a branch or a commit of that repository, as in `needs units from "..." at "v1.0.0"`

error: the package in `lib` is called `filters`, not `filter`. Write `needs filters ...`

error: two packages called `b` are needed: the one in `a/b` and the one in `b`.
A program has one package of each name. Make the `needs` lines agree

error: the package `filters` has no module `lowpas`. `import filters.lowpas` reads
the file `../filters/lowpas.zeno`, and there is no such file

error: `import filters` has two readings here: the file `filters.zeno`, and the file
`../filters/filters.zeno` of the package `filters` that `zeno.package` needs.
Give the first another name
```

A mistake in a `zeno.package` is shown as a mistake in any file is:
the file, the line, and the line itself with a mark under it.

## How it works

Two files do the work. `src/package.rs` reads `zeno.package` and
`zeno.lock`, and is `zeno fetch`. `src/modules.rs`, the loader from
the last stage, learned which file an import names when there are
packages.

### Reading `zeno.package`

The file is split into words by Zeno's own lexer, the one that reads
programs. That is why a comment starts with `#` and a string is
written as in a program, and it is why this reader is short:

```rust
for line in tokens.split(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Eof)) {
    ...
    match words.as_slice() {
        [_, TokenKind::Ident(package)] if word(0, "package") => { ... }
        [_, TokenKind::Ident(need), rest @ ..] if word(0, "needs") => {
            let source = match rest {
                [TokenKind::In, TokenKind::Str { value, .. }] => Source::Folder(value.clone()),
                [TokenKind::From, TokenKind::Str { value: url, .. }, _, TokenKind::Str { value: at, .. }]
                    if word(4, "at") => Source::Git { url: url.clone(), at: at.clone() },
                ...
```

A line is a list of tokens, and each kind of line is one pattern over
that list. `in` and `from` are already keywords of Zeno, so they
arrive as `TokenKind::In` and `TokenKind::From`. `package`, `needs`
and `at` are ordinary words, matched by their text. What comes out is
a `Manifest`: a name, and a list of `Need`.

### Finding the packages

`Loader` in `src/modules.rs` got a list of packages. The first is the
program itself:

```rust
struct Package {
    name: String,                    // empty for a program with no zeno.package
    folder: PathBuf,
    needs: HashMap<String, usize>,   // name -> which package in the list
}
```

Before the first import is followed, `packages_needed` reads the
program's `zeno.package`, and `need` follows each `needs` line to the
`zeno.package` it points at, and so on down. A `needs ... in` folder
is counted from the folder of the file that says it. A `needs ...
from` package is always in `packages/<name>` of the *program*,
whichever package asked for it. If a name is met a second time it
must be the same folder, or the program would have two packages of
one name.

Each `zeno.package` that is read becomes one of the program's files,
with a number like any other (see "One number says the file and the
line" in `modules.md`). That is all it takes for a mistake in one to
be shown with its file and line.

### Which file an import names

`find` is the two-step rule from above, in code:

```rust
let own = file(&self.packages[from].folder, &parts);
let Some(&needed) = self.packages[from].needs.get(parts[0]) else {
    return Some(Found { module: self.full(from, written), package: from, path: own });
};
...
let inside = if parts.len() == 1 { &parts[..] } else { &parts[1..] };
let path = file(&self.packages[needed].folder, inside);
```

`from` is the package the importing file is in. If the first word is
not one of its needs, the import is a file of that same package.
Otherwise the words after the first are a file of the needed package.

### One name in the whole program

Inside `filters`, the file writes `import lowpass`. In the thermostat,
the same file is `import filters.lowpass`. The joined program needs
one name for it, and `full` gives it: a module of the program keeps
its name, and a module of another package gets the package's name in
front.

```
lowpass.start           written in examples/filters/main.zeno ...
filters.lowpass.start   ... and in examples/thermostat/main.zeno
filters.lowpass.start   is what both are in the thermostat's joined program
```

So the renamer of the last stage now keeps two names for each
imported module: the one the file wrote, which is what it looks for
in the source, and the one the program uses, which is what it writes
into the tree. See it with `join`:

```sh
cargo run -- join examples/thermostat/main.zeno
```

```
function filters.clamp(value: float, low: float, high: float) -> float
...
class filters.lowpass.Lowpass
...
function filters.lowpass.start(weight: float, first: float) -> filters.lowpass.Lowpass
    return filters.lowpass.Lowpass(filters.clamp(weight, 0.0, 1.0), first)
```

In `lowpass.zeno` that last line is `Lowpass(clamp(weight, 0.0, 1.0),
first)`. After this point nothing knows there were packages: the type
checker, the interpreter, the VM and both compilers got one ordinary
program, as they did with modules.

### `zeno fetch`

`fetch` in `src/package.rs` walks the same `needs` lines, and for each
one with `from` runs git:

```
git clone <address> packages/<name>         if it is not there yet
git fetch --tags origin                     if it is
git rev-parse refs/tags/<at>                is `at` a tag? else a branch? else a commit?
git checkout --detach <commit>
```

A line with a name and no address (`needs units at "v1.2.0"`) first
asks the registry for the address and the commit, and then goes the
same way ([`registry.md`](registry.md)).

If `zeno.lock` has a line for the same package, address and `at`, the
commit in that line is used and `at` is not looked up. Then the
fetched package's own `zeno.package` is read, its needs join the
queue, and at the end the lock is written, sorted by name so that it
is the same file whichever order things were fetched in.

The test for all this (`fetch_gets_what_is_needed_and_pins_it`) makes
two small git repositories in a temporary folder and fetches from
them by `file://` address, so it needs no network.

## What is left out

- *(A registry and short names were left out when this page was
  written, and have come since: [`registry.md`](registry.md).)*
- **No version ranges.** `at` names one tag, branch or commit, and two
  packages that need the same package must name the same one (8.4,
  8.7).
- **No `zeno add`.** The project file is a few kinds of line, written
  by hand (8.9). `zeno new` writes the first one, and shows the
  others in a comment ([`new.md`](new.md)).
- **A package is a whole repository.** One that sits in a subfolder
  of a repository cannot be fetched yet.
- **`packages/` is not tidied.** A package that is no longer needed
  stays in the folder until you remove it. It is not read.
- **An edit inside `packages/` is not noticed.** `zeno fetch` compares
  commits, not files. To change a fetched package, make it a folder
  package (`needs ... in`) while you work on it.
- **Everything is still compiled together**, packages included.

## Things to try

1. Run `cargo run -- join examples/thermostat/main.zeno`. Find a name
   that starts with `filters.lowpass.`, then open
   `examples/filters/lowpass.zeno` and see how the file itself
   wrote it.
2. Run `cargo run -- run examples/filters/main.zeno`, then `join` it.
   The same `start` has another name now. Which, and why? (Read
   `Loader::full`.)
3. In `examples/thermostat/main.zeno`, change `import filters.lowpass`
   to `import lowpass`. Read the message: which file did Zeno look
   for, and in which folder?
4. In `examples/thermostat/zeno.package`, change the folder to
   `"../filter"`. Which file and line does the message point at? Put
   it back.
5. In the same file, call the package `filter` in the `needs` line
   (and leave the folder right). Read the message.
6. Add the line `import control` to the top of
   `examples/filters/lowpass.zeno` and run the thermostat. The
   thermostat has a `control.zeno`. Why is it not found? (Decision
   8.6.) Take the line out again.
7. Add a function `lowpass()` to `examples/filters/filters.zeno` and
   run the thermostat. Read the message, then read decision 8.11.
   Take it out again.
8. Make a package of your own. In an empty folder make `zeno.package`
   with `package greet` and a file `greet.zeno` with a function
   `hello(name: str) -> str`. In a second folder beside it write a
   program that needs it and prints `greet.hello("Zeno")`.
9. Now fetch it. In the `greet` folder run `git init`, `git add .`,
   `git commit -m first` and `git tag v1`. In the program's
   `zeno.package` write
   `needs greet from "file:///full/path/to/greet" at "v1"`, run
   `zeno fetch`, and look at `packages/` and `zeno.lock`.
10. Change `hello` in the `greet` folder, commit, and run `zeno fetch`
    again. Nothing moves: why? Tag the new commit `v2`, change `at` to
    `"v2"`, and fetch once more. What changed in `zeno.lock`?
11. Build the thermostat with no collector:
    `cargo run -- build --no-collector --memory examples/thermostat/main.zeno`
    and run `./main`. How many blocks are left at the end?
