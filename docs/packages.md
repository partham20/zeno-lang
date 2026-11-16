# Packages: Zeno for apt, pacman, snap, flatpak, Homebrew and winget

The installers of [`install.md`](install.md) are Zeno's own: a setup
program and a script. But most systems already have a program whose
job is to install programs, and people who use one would rather use
it for everything. It knows what is installed, it updates it, and it
takes it off cleanly.

There are many of them. This stage makes Zeno something that six of
them can install:

| Its program | On | What is made for it |
|---|---|---|
| **apt** | Debian, Ubuntu, Raspberry Pi OS | `zeno_0.2.0_amd64.deb`, and `arm64`, `armhf`, `i386` |
| **pacman** | Arch, Manjaro | `zeno-bin-0.2.0-1-x86_64.pkg.tar.zst`, and a `PKGBUILD` |
| **snap** | Ubuntu, and others | `zeno_0.2.0_amd64.snap`, and `arm64`, `armhf` |
| **flatpak** | most Linux desktops | `zeno-0.2.0-x86_64.flatpak`, and `aarch64` |
| **Homebrew** | macOS, and Linux | a formula, `Formula/zeno.rb` |
| **winget** | Windows 10 and 11 | three small files, in `packaging/winget` |

None of them holds a `zeno` of its own. Each is the `zeno` of a
release's archive, laid out the way that system wants it. The
decisions are 15.12 to 15.21 of [`decisions.md`](decisions.md).

## Two halves, and which one this is

Installing with a package manager has two halves.

The first is **a package**: a file in that manager's own form. All six
are made here, by one script, and five of them are tried on a real
system before each release.

The second is **a place the manager looks in**: when somebody types
`apt install zeno`, apt asks the places it knows about, and Zeno is in
none of them. Each manager has such a place, each place has a door,
and each door wants an account or a person's say-so. That half is at
the end of this page, as a list.

Until then, each package installs from its file:

```sh
sudo apt install ./zeno_0.2.0_amd64.deb
sudo pacman -U zeno-bin-0.2.0-1-x86_64.pkg.tar.zst
sudo snap install --classic --dangerous zeno_0.2.0_amd64.snap
flatpak install --user zeno-0.2.0-x86_64.flatpak
```

and Homebrew, which reads formulas from a repository, reads this one:

```sh
brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
brew install partham20/zeno-lang/zeno
```

## apt: a .deb

A `.deb` is an archive of files with the paths they are to have, and a
folder, `DEBIAN`, with one file that says what the package is:

```
Package: zeno
Version: 0.2.0
Architecture: amd64
Recommends: gcc | c-compiler, libc6-dev | libc-dev, git
Suggests: clang
```

`dpkg-deb --build` turns the folder into the file. What is in it:

```
/usr/lib/zeno/zeno          the program
/usr/lib/zeno/zeno.wasm     Zeno for a browser, beside it
/usr/bin/zeno               a link to the program, where the PATH looks
/usr/share/zeno/examples    and /editors
/usr/share/doc/zeno         the README and the docs
```

**`Recommends` is how apt installs whatever is needed.** apt installs
what a package recommends along with it, unless it is told
`--no-install-recommends`. So `apt install ./zeno_0.2.0_amd64.deb` on
a machine with no compiler brings gcc and git too, which is what the
setup program's two boxes do on Windows. `gcc | c-compiler` means gcc,
or anything that calls itself a C compiler: a machine that has clang
is left alone.

**The one for `armhf` holds the build for ARMv6.** Debian's `armhf` is
ARMv7, and Raspberry Pi OS calls a Pi Zero, which is ARMv6, `armhf`
too. The ARMv6 build runs on both, and the ARMv7 build would not start
on the Pi Zero.

## pacman: a PKGBUILD, and the package it makes

Arch does not pass packages around as much as recipes for them. A
`PKGBUILD` is a shell script that says where the files come from, what
their checksums are, and, in a function called `package`, where each
goes. `makepkg` runs it and makes the package, and `makepkg -si`
installs it too.

The package is called `zeno-bin`: on Arch, `-bin` says "this packs a
program that was already built", and plain `zeno` is left for a
package that would build it from source.

pacman has nothing like `Recommends`. The compiler and git are named
as `optdepends`, which pacman shows after installing and does not
install. Nearly every Arch machine that builds anything has both.

`makepkg` exists only on Arch, so `package.sh` runs Arch in a
container to make the package for x86-64.

## snap

A snap is one file that holds a folder (it is a squashfs, a read-only
disk in a file), with `meta/snap.yaml` in it to say what it is. `snap
pack` makes the file from the folder.

```yaml
name: zeno
confinement: classic
apps:
  zeno:
    command: bin/zeno
```

**`classic`** is the line that matters. A snap is normally shut in: it
sees its own files and little of the machine. A compiler cannot work
like that. It has to read a program wherever somebody keeps it, and
run the C compiler that is installed. `classic` says the snap is not
shut in at all, and installing one says so too: `--classic`.

`--dangerous` only means "this file did not come from the store, so
nobody has signed it".

## flatpak

A flatpak is shut in as well, and has no `classic`. It runs in a box,
on top of a **runtime**: a set of files that stands in for the system,
the same on every Linux. So the question is what to do about the
compiler, which is outside the box.

The answer here is to pick a runtime that has one. There are two for
each version: the Platform, which programs run on, and the Sdk, which
they are built on. The Sdk has gcc and git. Zeno names the **Sdk** as
its runtime, so `zeno build` and `zeno fetch` work inside the box with
nothing else installed. The price is the download: the Sdk is several
hundred megabytes, once.

The box is given two things from outside: the home folder, where
programs are kept, and the network, for `zeno fetch`.

Its name is `org.zeno_lang.Zeno`. A flatpak is named after a web
address its maker owns, turned round, and `zeno-lang.org` turned round
is `org.zeno-lang`, with `_` because `-` is not allowed. It is run as

```sh
flatpak run org.zeno_lang.Zeno run hello.zeno
```

which is long, so: `alias zeno='flatpak run org.zeno_lang.Zeno'`.

Of the six this is the poorest fit. Flatpak was made for programs with
a window. It is here because it was asked for and because it works.

## Homebrew: a formula

A formula is a small Ruby file. For each kind of machine it says where
the archive is and what its checksum must be, and then what to do with
what is in it:

```ruby
def install
  bin.install "zeno"
  bin.install "zeno.wasm" if File.exist?("zeno.wasm")
  doc.install "README.md", "docs"
  pkgshare.install "examples", "editors"
end
```

Homebrew reads formulas from a git repository, which it calls a
**tap**. The formula is kept in this repository, in `Formula`, so this
repository is the tap. That is why the first of the two lines above
names it in full: a tap that is called `homebrew-something` can be
named in short, and this one is not.

On macOS the compiler and git come with Apple's command line tools,
which Homebrew itself needs, so there is nothing to recommend.

## winget

winget does not have packages of its own. It runs a program's setup
program without showing it, and what it needs to know is in three
small files: which version this is, where the setup program is and its
checksum, and what to show somebody who searches. The setup program is
the one from [`install.md`](install.md). It is one file for three
processors, so the files name it three times.

One line in them is there for a reason:

```yaml
InstallerSwitches:
  Custom: /MERGETASKS="!compiler,!git"
```

The setup program's two boxes install the compiler and Git **with
winget**. Run by winget, without being shown, it would start a second
winget inside the first, and nobody would have been asked. The line
unticks the two boxes. Somebody who uses winget installs those two the
same way: `winget install BrechtSanders.WinLibs.POSIX.UCRT.LLVM
Git.Git`.

## How they are made

```
tools/package.sh 0.2.0 files out
```

`files` is a folder with the files of a release. The script unpacks
the archive for each processor, lays the files out, and calls the tool
that makes that kind of package: `dpkg-deb`, `snap pack`, `flatpak
build-export` and `build-bundle`, and `makepkg` in a container. The
three recipes are text, and are written with `cat`. A kind whose tool
is missing is left out, with a line that says so.

Nothing is compiled. The script works on any release, including one
that was made before the script was: that is how it was tried, on
0.1.0.

The recipes go into `out/recipes`, laid out as this repository is:

```
out/recipes/Formula/zeno.rb
out/recipes/packaging/arch/PKGBUILD
out/recipes/packaging/winget/ZenoLang.Zeno.yaml   and two more
```

They hold the checksums of a release's files, so they can only be
written when those files exist. `tools/keep-recipes.sh` copies them
into the repository and pushes, as the last step of a release.

## How they are tried

```
tools/try-packages.sh deb 0.2.0 out
```

and the same with `snap`, `flatpak`, `pacman` and `brew`. Each
installs the package the way somebody would, checks that `zeno
--version` is the version, runs the first example, builds a page (from
0.2.0), removes the package and checks that nothing is left. The
flatpak is also made to compile a program, since its compiler is the
one thing about it that is unusual.

Homebrew downloads what its formula names, and before a release there
is nothing there to download. So the formula is tried with the address
of a folder in place of the address of the release, and nothing else
changed.

## In the release

[`release.yml`](../.github/workflows/release.yml) has a job,
`packages`, after the ten builds and the setup program. It makes the
packages from what was just built and tries them, and a second job
tries the formula on macOS. If either fails, there is no release.

A release then has its archives, the two installers, and nine
packages: four `.deb`, three `.snap`, two `.flatpak` and the pacman
package. Then the recipes are pushed to `main`.

[`packages.yml`](../.github/workflows/packages.yml) does the same for
a version that is already released, started by hand.

## The other half: where each manager looks

For `apt install zeno` and the rest to work with one word, Zeno has to
be in the place that manager looks in. Every one of them first needs
**the repository to be public**: they all download from the release.
After that:

| For | What has to happen | Then it is |
|---|---|---|
| Homebrew | Nothing more. Better: a repository named `homebrew-zeno` to hold the formula | `brew install partham20/zeno/zeno` |
| winget | The three files are sent to Microsoft's `winget-pkgs` repository as a pull request, which people there check | `winget install ZenoLang.Zeno` |
| pacman | An account on the AUR, and the `PKGBUILD` pushed there | `yay -S zeno-bin` |
| snap | An account on the Snap Store, the name `zeno` registered, and a request for `classic`, which people there decide | `sudo snap install zeno --classic` |
| flatpak | A request to Flathub, which people there check | `flatpak install flathub org.zeno_lang.Zeno` |
| apt | An apt repository: the `.deb` files and a signed list of them, kept somewhere such as zeno-lang.org | one line to add it, then `sudo apt install zeno` |

Being in Debian's or Arch's own collection, or in Homebrew's main one,
is a different matter: each takes programs that are already widely
used, and Zeno is days old.

winget, the Snap Store and Flathub each ask what licence a program has.
It is MIT (15.19), and from 0.2.0 each package says so and carries the
file. Debian keeps it at `/usr/share/doc/zeno/copyright` and Arch at
`/usr/share/licenses/zeno-bin/LICENSE`.

## What it does not do yet

- **None of the six works with one word** (above).
- **The winget files have not been run.** GitHub's Windows machines
  have no winget. The other five are installed and removed before
  each release.
- **The packages for other processors are made and not run**: the
  `.deb` for arm64, armhf and i386, the snap for arm64 and armhf, and
  the flatpak for aarch64. The `zeno` inside each was run, by the
  build that made it.
- **No apt repository**, so apt does not update Zeno by itself.
- **No package for pacman on ARM**, only the `PKGBUILD`, which makes
  one there.
- **No `.rpm`** for Fedora and openSUSE, no Nix, no Scoop or
  Chocolatey. `install.sh` works on all the Linux ones.

## Things to try

1. `dpkg-deb --info zeno_0.2.0_amd64.deb` and `dpkg-deb --contents`
   of the same. Find each line of the first in `tools/package.sh`.
2. Install the `.deb` with `--no-install-recommends` in a container
   that has no compiler (`docker run -it debian`). What does `zeno
   build` say? Now without that option.
3. A snap is a squashfs: `unsquashfs -l zeno_0.2.0_amd64.snap` lists
   what is in it. Find `meta/snap.yaml`.
4. In `package.sh`, make the flatpak's runtime the Platform and not
   the Sdk, and run `try-packages.sh flatpak`. Which check fails, and
   what does it say? How much smaller is the download?
5. Read the `PKGBUILD`. `package()` begins with `cd
   "$srcdir"/zeno-$pkgver-linux-*/`. Why a `*`? Take the `/` off the
   end and run `makepkg`: it says `cd: too many arguments`, which is
   what it said the first time this was tried. What are the two
   things the `*` matched?
6. The formula has `on_macos` and `on_linux`, each with `on_arm` and
   `on_intel`. Homebrew on Linux does not run on 32-bit ARM. What
   does somebody on a Raspberry Pi with a 32-bit system see, and what
   should they use?
7. A big one: an apt repository is a folder with the `.deb` files, a
   file `Packages` that lists them (`dpkg-scanpackages` writes it) and
   a file `Release` with the checksum of that list, signed. Make one
   in a folder, serve it with `python3 -m http.server`, and add it to
   a container's `/etc/apt/sources.list.d`. What goes wrong without
   the signature, and what is the option that gets past it? Why is
   that option a bad idea on a real machine?
