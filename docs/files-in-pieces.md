# A file in pieces

`files.read` reads a whole file into one piece of text, and `files.write`
writes one. A log of a gigabyte, or a file that grows while the program
runs, wants something else: a line or a few bytes at a time.

```zeno
import files

files.write("big.log", "boot\nerror: no card\nready\nerror: low battery\n")

let source = files.read_from("big.log")
let errors = files.write_to("errors.log")

for line in source {
    if line.starts_with("error") {
        errors.write(line + "\n")
    }
}
print(files.read("errors.log"))
```

```
error: no card
error: low battery

```

`errors.log` was read while `errors` was still open, and it held both
lines already: every `write` hands its text to the system before it
gives back. Both files are closed when `source` and `errors` end,
at the end of the program; there is no `close` to call, or to forget.

The same lines come out of `zeno run`, the VM, every build in C and
LLVM, and every target. The decisions are 59.6 and 59.7 in
[`decisions.md`](decisions.md).

## What was added

| Written | Gives | Can fail with |
|---|---|---|
| `files.read_from(path)` | a `files.Reader`, at the start of the file | `FileError` |
| `files.write_to(path)` | a `files.Writer` on a new, empty file | `FileError` |
| `files.append_to(path)` | a `files.Writer` after what the file holds, made if it is not there | `FileError` |
| `reader.read_line()` | the next line without its line end, or `none` after the last | `FileError` |
| `reader.read_bytes(count)` | up to `count` bytes as a `list[u8]`: fewer at the end, none after it | `FileError`, `ValueError` |
| `for line in reader` | each line in turn | `FileError` |
| `writer.write(text)` | writes the text, and adds no line end | `FileError` |
| `writer.write_bytes(bytes)` | writes the bytes | `FileError` |

A line ends at `"\n"`, and a `"\r"` just before it goes with it, as
`lines()` and `input()` read them: a file from Windows reads the same.
The last line need not end. A line that is not UTF-8, or that holds a
zero byte, is a `FileError`: ``` `big.log` does not hold text ```, as
`files.read` says of such a file.

## Closed when the owner ends

A Reader and a Writer are resources ([`resource.md`](resource.md)):
the file is closed when the `let` that holds it, or the parameter that
took it, ends. However the block is left, by its end, `return`,
`break`, or an error going past, the file is closed, and opening it
again on the next line works, on Windows too.

```zeno
import files

function first_line(path: str) -> str or none raises FileError {
    let source = files.read_from(path)
    return source.read_line()
}                                   # closed here, after the line was read
```

So they go where a resource goes: in a `let` or a parameter, and not in
a list or a field. A function may look at one, `function count(source:
files.Reader)`, or take it, `source: take files.Reader`, and close it
when it ends.

## `for line in reader`

```zeno
let source = files.read_from("big.log")
for line in source {
    print(line)
}
```

The loop holds one line at a time. `break` leaves the rest unread, and
the reader goes on from there: a `read_line` after the loop gives the
next line. The reader must have a name: `for line in
files.read_from(path)` is refused, because nothing would own the file
to close it.

## A file that grows

The end of a file is where it ends at that moment. A read that found
the end looks again the next time, so a reader follows a file that this
program, or another, is still writing:

```zeno
let out = files.write_to("grow.log")
let source = files.read_from("grow.log")
print(source.read_line())            # none: nothing is there yet
out.write("one\n")
print(source.read_line())            # one
```

What is there without a line end is given as a line, as the last line
of any file is. The reader does not wait for more: when nothing new is
there it gives `none`, and the program decides when to look again.

## A disk that is full

A `write` gives the text to the system before it returns, so a write
that cannot be done is a `FileError` at that `write`, where the program
can catch it:

```zeno
let log = files.write_to("/dev/full")
try {
    log.write("x")
} except FileError as e {
    print(e.message)                 # cannot write `/dev/full`
}
```

Nothing is kept back for the close, where an error would have nowhere
to go.

## What a program cannot do

- **Make one itself.** Only `read_from`, `write_to` and `append_to`
  make one; `files.Reader(...)` is refused.
- **Read from a Writer, or write to a Reader.** They are two types, and
  the checker says so before the program runs.
- **Close one by hand.** It closes when its owner ends; to close it
  sooner, give it to a function that takes it.

## With no collector

A line is built in the text room, as `input()` builds one, so a line
longer than the room stops the program with the room's own message:
``a statement built more text than there is room for``. Make the room
larger, or read such a file with `read_bytes`, whose list is taken from
the pool.

## When nothing catches the error

A `FileError` that nothing catches names the line of
`library/files.zeno` that asked the system, not the line of the program:

```
error: uncaught FileError: there is no file `nope.txt`
  --> library/files.zeno:86:19
```

An error names where it was raised (33.2), and that is in the library.
The message names the file; catch the error where you open it to say
more. It is listed in [`missing.md`](missing.md).

## On a board

These are the C library's `fopen`, `fgets` and `fwrite` on a board that
has them. A board with no file system answers `FileError`, as
`files.read` does there.

## What it costs a program that does not use it

Nothing. The classes are written in Zeno, in `library/files.zeno`, and
a program gets only the functions of the library it uses. The six
small built-ins under them are pieces of the runtime, written into a
program only if it reads or writes a file in pieces.

## How it is built

The two classes are ordinary Zeno, in `library/files.zeno`:

```zeno
owned class Reader is Resource {
    private file: int
    private path: str

    function read_line(self) -> str or none raises FileError {
        return files.handle_line(self.file, self.path)
    }
    ...
    function release(self) {
        files.handle_close(self.file)
    }
}
```

Under them are six built-ins, one for each thing done to an open file:
open it, read a line, read bytes, write text, write bytes, close it.
Only `library/files.zeno` can call them; to any other file they are not
there. Each engine has them beside `files.read`: `builtins.rs` for
`zeno run` and the VM, `runtime.c` for both compilers, and each target's
runtime. The file is opened as bytes on every system.

`for line in source` is turned by the checker into the loop it means,
a `while` that calls `read_line` and stops at `none`, so no engine has
a new kind of loop.

## What it does not do

- **Gather the pieces** before writing them. Each `write` goes to the
  system, which is safe and can be slow for a great many small writes;
  build the text first and write it once.
- **Go back**, or say where in the file it is.
- **Wait for a file to grow.** A read at the end gives `none` at once.
- **Read and write through one object.** A Reader and a Writer on the
  same file work side by side, as above.
- **Text in another encoding than UTF-8.**

## How it is tested

- `a_file_is_read_and_written_in_pieces`,
  `a_write_that_meets_a_full_disk_raises_there`,
  `a_reader_is_made_and_read_only_as_the_library_says`
  (`interpreter.rs`), each on the interpreter and the VM.
- `a_file_in_pieces_is_the_same_in_every_build`,
  `reading_a_file_in_pieces_does_not_grow_the_pool` and
  `a_file_in_pieces_is_only_in_a_program_that_uses_one` (`emit_c.rs`):
  the same thousand lines, shapes, bytes, a file that grows, and every
  error, built in C and LLVM with and without a collector.
- `a_test_that_uses_a_piece_of_the_runtime_is_built` (`test_build.rs`):
  `zeno test --build`, C and LLVM, with a file in pieces in a test.
- `tools/targets/edge-files.zeno`: a thousand lines with both line
  ends, files of every shape, bytes across the end, a file opened three
  times in a row and removed, a file that grows while it is read, and
  every error, run by `zeno run`, the VM and every target. (`zeno build`
  refuses it: its functions without names leave out `raises`, which a
  build checks and a target does not. The `emit_c.rs` tests above build
  the same cases.)

## Things to try

1. Count the lines of a file with `for` and with
   `len(files.read(path).lines())`. Which holds the whole file?

2. Read a file of bytes 100 at a time with `read_bytes`, and print how
   many came each time. What does the last call give?

3. Write a function `copy(original: str, copied: str)` that copies a file a line
   at a time. Is the copy the same as the original when the original's
   last line has no line end? Why not, and how would `read_bytes` fix
   it?

4. Open a file with `files.read_from` inside a loop that runs a thousand
   times. Build it with `--no-collector --memory`. How many blocks are
   left?

5. Try `for line in files.read_from("big.log")` and read the message.
   Why does the reader need a name?
