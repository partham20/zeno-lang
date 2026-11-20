# Folders

A program can now see what is in a folder, make one, and put a path
together from its parts:

```zeno
import files

files.make_folder("logs/2026/november")
files.write(files.join("logs/2026/november", "20.txt"), "temp 23.5\n")
files.write(files.join("logs/2026", "notes.txt"), "calibrated\n")

for name in files.list("logs/2026") {
    print(name)
}
```

```
notes.txt
november
```

The same list comes out of `zeno run`, the VM, every build in C and
LLVM, and every target, on Linux, macOS and Windows. The decisions are
59.20 to 59.23 in [`decisions.md`](decisions.md).

## What was added

| | Written | Gives | Can fail with |
|---|---|---|---|
| The names in a folder | `files.list(folder)` | `list[str]`, sorted | `FileError` |
| Make a folder, and those above it | `files.make_folder(path)` | nothing | `FileError` |
| Two parts of a path | `files.join(first, second)` | `str` | never |

## `files.list(folder)`

The names of everything in the folder, files and folders alike, with
no `.` and no `..`. A name is the last part of the path only:
`files.list("logs")` gives `"notes.txt"`, not `"logs/notes.txt"`, and
`files.join` puts them back together.

**The list is sorted by the numbers of the characters**, which is the
order `<` gives two texts in Zeno. Digits come before capitals, and
capitals before small letters, and a name with a character from
beyond the first 65,536 comes after one without:

```
["10", "9", "C.txt", "_", "a", "a b", "a.txt", "b.txt", "z", "~", "é", "ｚ", "😀"]
```

The system gives the names in an order of its own, which is not the
same from Linux to Windows, nor from one disk to another. Sorting them
means a program that lists a folder prints the same thing everywhere,
and a test of it can be written once. Every list is a new one: a
program can sort it again, or change it, and the next call still gives
the folder as it is.

### What can go wrong

| What happened | The message of the `FileError` |
|---|---|
| Nothing is at the path, or the path is `""` | ``there is no folder `logs` `` |
| It is a file, or the system will not let it be read | ``cannot list the folder `logs` `` |
| A name in it is not UTF-8 | ``the folder `logs` holds a name that is not text`` |

An empty path is no folder. Several systems take `""` for the folder
the program was started in, and some do not; `"."` says that in every
one.

## `files.make_folder(path)`

Makes the folder and every folder above it that is not there yet, as
`mkdir -p` does. **It worked if there is a folder at the path at the
end**, so making one that is already there is no error, and a program
can call it every time it starts. Anything that is not a folder in
the way is: ``cannot make the folder `notes.txt/old` ``, and so is an
empty path.

## `files.join(first, second)`

```zeno
files.join("logs", "today.txt")    # "logs/today.txt"
files.join("logs/", "today.txt")   # "logs/today.txt"
files.join("logs", "/today.txt")   # "logs/today.txt"
files.join("logs/", "/today.txt")  # "logs/today.txt"
files.join("", "today.txt")        # "today.txt"
files.join("logs", "")             # "logs/"
```

What it does is small. Where neither part has a `/` at the end that
meets the other, it puts one between them; where one has, it puts in
none; where both have, it drops the one at the start of the second.
That is all: `files.join("a//", "b")` is `"a//b"`, since the first
part's own `//` is left as it is, and `files.join("logs", "")` is
`"logs/"`, a folder with nothing named in it yet. A first part that is
empty is no folder at all, and the second comes back as it is. Every system Zeno runs
on takes `/` in a path, Windows too, so the path is the same text
everywhere. It does no more than that: it does not look at the disk,
and does not take out `..` or a `/` that is doubled inside a part.

`join` is written in Zeno, in [`library/files.zeno`](../library/files.zeno),
as `math.hypot` is (chapter 33). A program that calls it gets it, and
one that does not gets nothing.

## On a board

A board has no folders. `files.list` and `files.make_folder` raise a
`FileError` there, which the program can catch, as `files.read` does
for a file. `files.join` works, since it is arithmetic on text.

## What it costs a program that does not use it

Nothing. The C for listing and making a folder is a piece of the
runtime (decision 56.19), written into a program only if it calls
`files.list` or `files.make_folder`, through C and LLVM alike. A
program that calls neither is built into the same C, IR and bytecode
as before, to the byte.

## The files

| File | What it holds |
|---|---|
| `src/builtins.rs` | `list` and `make_folder` for `zeno run` and the VM |
| `src/checker.rs` | Their types, and that both raise `FileError` |
| `library/files.zeno` | `join` |
| `src/runtime.c` | The two pieces `ZENO_FILES_LIST` and `ZENO_FILES_MAKE_FOLDER`, with POSIX and Windows inside each |
| `src/emit_c.rs`, `src/emit_llvm.rs` | A call of each, in every kind of build |
| `src/runtime.*` | Both for each target |
| `tools/targets/edge-folders.zeno` | A folder of names that sort differently in every language's own order, and every error |

## What it does not do

- **Remove a folder.** Left for later ([`plan-9-14.md`](plan-9-14.md),
  after item 12).
- **Say what a name is.** `files.list` does not say which names are
  folders. `files.exists(path)` is true for a file only, so it tells
  the two apart.
- **A name that is not UTF-8, on the JVM, .NET and Dart.** Those
  languages turn such a name into text of their own before a program
  sees it, so the Java, Kotlin, C# and Dart targets give a changed name
  where every other engine raises a `FileError`.
- **A name that is not ASCII, in Java where the system's language
  setting is not UTF-8.** With no `LANG` set, or `LC_ALL=C`, the JVM
  reads `é` in a name as `?`, so the Java and Kotlin targets list a
  name that `files.read` cannot open. Set `LC_ALL=C.UTF-8`, or any
  UTF-8 setting, and they list it as every other engine does.
- **Tell apart two names that differ only in case, on a disk that does
  not.** Zeno lists what the disk holds. On Windows and macOS, as they
  are set up out of the box, `files.write("b.txt", ...)` and
  `files.write("B.txt", ...)` write one file, and the folder holds one
  name.
- **A name that is not ASCII, in a C or LLVM build on Windows, made by
  another program.** A build reads and writes a file through the C
  library's `fopen`, which takes a name through the machine's code
  page, and `files.list` and `files.make_folder` take names the same
  way, so what the program writes it lists and reads back by the same
  name. A name the code page cannot hold, made by `zeno run` or another
  program, is listed with question marks, and does not open.
- **Lua without LuaFileSystem on Windows.** Lua itself cannot see into
  a folder; the Lua target uses LuaFileSystem where it is installed,
  and `find` and `mkdir -p` where there is a POSIX shell.

## How it is tested

- `folders_are_the_same_in_every_build` (`emit_c.rs`): one program
  through the interpreter, the VM, and C and LLVM with a collector,
  `--firmware` and `--no-collector`, with 0 blocks left in the last.
- `folders_are_only_in_a_program_that_uses_them`.
- `a_folder_is_made_and_listed` (`interpreter.rs`): the exact output,
  each message, a name that is not UTF-8, and what the checker says.
- `tools/targets/edge-folders.zeno`, run by every target.

## Things to try

1. Run the program at the top twice. Why is the second run no error,
   when `files.make_folder` is asked for a folder that is there?

2. Make a folder with the files `a`, `B`, `é` and `10` in it, list it,
   and say why `"10"` comes first and `"é"` last. Then sort the list
   with `.sort()` and see that nothing moves.

3. Write `tree.zeno`: a function that prints a folder's names, and the
   names inside each folder in it, indented. Tell a folder from a file
   with `files.exists`, which is false for a folder. It needs
   `raises FileError`: take that off and read what `zeno check
   --strict` says.

4. Try `files.list("")` and `files.list(".")`. Which decision says why
   the first is an error?

5. Build `tree.zeno` with `--no-collector --memory` and give each list
   an owner (`let names: owned list[str] = files.list(folder)`). Is
   anything left at the end?

6. Print `files.join("a/", "/b")` and `files.join("a//", "b")`. Why
   does `join` mend the first and leave the second?
