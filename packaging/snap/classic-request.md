<!--
The request for `classic`, which the Snap Store gives only when it is
asked. Partha posts it, from his own account, as a new topic in
https://forum.snapcraft.io/c/store-requests/19 after the name `zeno`
is registered. The first line is the title of the topic and the rest
is its text. docs/stores.md has the steps.
-->

Classic confinement request for zeno

Hello. I would like to ask for classic confinement for the snap `zeno`.

**What it is.** Zeno is a programming language, and the snap holds its
compiler: one program, `zeno`, that checks, runs, tests, formats and
compiles Zeno programs. It is open source under the MIT licence.

- Website: https://zeno-lang.org
- Source: https://github.com/partham20/zeno-lang
- How the snap is made: `tools/package.sh` in that repository (the
  function `snap_of`), from the files of a release, with `snap pack`

**Which supported kind it is.** A compiler and a programming language,
both of which are in the list of what classic is given for.

**Why it cannot be strict.**

1. `zeno build` writes C and hands it to the C compiler that is
   installed on the machine (`cc`, `gcc` or `clang`, or the one named
   in `CC`, which for firmware is often a cross compiler such as
   `arm-none-eabi-gcc` kept somewhere of its own). That compiler, its
   headers and its libraries are the machine's, and are not in the
   snap. A strict snap cannot run a program of the host.
2. `zeno fetch` gets packages by running the machine's `git`, with the
   keys and settings of the person who runs it.
3. Programs are kept anywhere: in the home folder, but as often on
   another disk, under `/opt`, `/srv` or `/mnt`, or in a folder that
   begins with a dot. `zeno` has to read a program where it is and
   write what it builds beside it. The `home` and `removable-media`
   interfaces do not cover that.

The snap has no daemon and no hooks, and asks for nothing as root.

Thank you.
