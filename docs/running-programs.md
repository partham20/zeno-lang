# Starting another program

A computer is full of programs that already do one thing well: `git`
keeps versions, `ffmpeg` turns one kind of video into another, a C
compiler compiles. A program that needs one of those things done does
not do it again. It starts the program that can, and waits.

```zeno
import program

let ended = program.run("git", ["status", "--short"])
print(f"git ended with {ended}")
```

```
 M docs/decisions.md
?? examples/download.zeno
git ended with 0
```

That is the whole of it: one function, in the module that already had
`program.arguments()`.

| Written | Gives | Can fail with |
|---|---|---|
| `program.run(name, words)` | the `int` the program ended with | `FileError` |

The decisions behind it are 25.1 to 25.7 in
[`decisions.md`](decisions.md).

## What `run` does

1. It looks for a program called `name` where the system looks for
   programs: the folders of `PATH`. A name with a `/` in it is a path,
   and is used as it is. On Windows `yt-dlp` finds `yt-dlp.exe`.
2. It starts the program with `words` as the words after its name.
3. It waits until the program ends. While it runs, the program uses
   this program's screen and keyboard, so what it prints appears among
   this program's lines, in the order things happened.
4. It gives the number the program ended with. 0 means that all went
   well; what any other number means is for the other program to say.

## The words are a list

```zeno
program.run("yt-dlp", ["--paths", "my videos", address])
```

Each item is one word for the other program, whatever is in it. `my
videos` has a blank in it and is still one word, because it is one
item. Nothing reads the words on the way: no shell is started, so a
`*`, a `$`, a `;` or a quote in an item means nothing but itself.

This matters most when a word comes from outside the program, as an
address does. In languages where the command is one line of text
handed to a shell, an address that ends in `; rm -rf ~` is two
commands. In Zeno it is one odd address, which the other program will
refuse (25.3).

A program that wants a shell's pipes and stars starts a shell, and the
line shows it:

```zeno
program.run("sh", ["-c", "ls *.zeno | wc -l"])
```

A Zeno program that is started this way gets the list back, item for
item, from its own `program.arguments()`.

## When it cannot be started

```zeno
try {
    program.run("yt-dlp", ["--version"])
} except FileError as error {
    print(error.message)        # there is no program `yt-dlp` to run
}
```

That a program is installed cannot be known before it is started, so
`run` raises a `FileError`, the error the `files` module raises for a
file that is not there. A function that calls `run` catches it or says
`raises FileError`, and `zeno check --strict` holds it to that.

A program that started and then ended with 3 is not an error. It ran,
and 3 is its answer.

## The example

[`examples/download.zeno`](../examples/download.zeno) saves a video
from YouTube in a folder. It is for videos you have the right to keep,
and its first lines say so.

```
zeno run examples/download.zeno https://youtu.be/VIDEO videos
```

Zeno does not fetch the video. The program checks that the address is
one of YouTube's, puts together the words for
[yt-dlp](https://github.com/yt-dlp/yt-dlp), starts it, and says in
plain words how it went:

```zeno
try {
    let ended = program.run("yt-dlp", words_for(address, folder, audio_only))
    if ended == 0 and folder == "." {
        print("Saved in this folder.")
    } else if ended == 0 {
        print(f"Saved in `{folder}`.")
    } else {
        print(f"yt-dlp could not save it, and ended with {ended}. What went wrong is in its lines above.")
    }
} except FileError {
    print("yt-dlp is not installed, or is not where programs are looked for.")
    print("Get it with `pip install yt-dlp`, or from https://github.com/yt-dlp/yt-dlp")
}
```

Three things can happen, and each has its line: the program is not
there, it ran and failed, it ran and worked.

## Where it works

| | |
|---|---|
| `zeno run`, `zeno run --vm`, `zeno test` | yes |
| `zeno build`, `zeno build --llvm` | not yet: the build says so and names the line |
| `zeno build --target ...` | not yet, the same way |
| `zeno build --firmware` | never: a board has no other programs |
| In a page | a `FileError`: a browser starts no programs |

## How it is built

One function in `builtins.rs`, `program_run`, which both engines call.
It hands the name and the words to Rust's `std::process::Command`,
which starts the program without a shell on every system, and on
Windows joins the words into the one line Windows wants, quoted by
Windows' rules.

Just before, it pushes out whatever this program has printed, so that
the other program's lines come after it and not before.

The checker's part is `"program.run"` in `builtin`: a `str`, a
`list[str]`, an `int` back, and `can_raise("FileError")`.

`emit_c.rs` and `emit_llvm.rs` each have one arm that refuses, and
`lower.rs` has `no_target_has`, which refuses for every target at
once.

The tests are in `interpreter.rs`
(`another_program_is_run_with_the_words_as_they_are`, which has `sh`
write each word it was given into a file, among them `two words`,
`a"b`, `$HOME`, `*` and an empty one; and
`a_program_that_is_not_there_is_a_file_error`), in `checker.rs`, and
one in each of `emit_c.rs`, `emit_llvm.rs` and `lower.rs` for the
refusals. Every one that runs a program runs it in both engines.

## Things to try

1. Run `zeno run examples/download.zeno` with no words, then with an
   address that is not YouTube's. Which lines of the program decide
   each answer?

2. Write `today.zeno`, which runs `date` (or `cmd` with `["/C",
   "date", "/T"]` on Windows) and prints the number it ended with.
   Then misspell the name of the program and read the message.

3. Write a program that prints its `program.arguments()`, and a second
   that starts the first with `program.run("zeno", ["run",
   "first.zeno", "two words", "*"])`. How many arguments does the
   first one see?

4. `program.run("ls", ["*.zeno"])` says there is no such file, and
   `program.run("sh", ["-c", "ls *.zeno"])` lists them. Who turns `*`
   into names?

5. In `download.zeno`, add a third word, `--small`, that asks yt-dlp
   for a picture no taller than 480 lines (`--format-sort` and then
   `res:480`). Where do the words go in `words_for`, and why must
   they be two items and not one?
