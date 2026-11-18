# The standard library, part 2: reading and writing

Until now a Zeno program could say things and could not be told
anything. Whatever it worked on was written in the program. This page
gives it the three ways in that a program on a computer has: files, the
keyboard, and the words it was started with.

```sh
cargo run -- run examples/logbook.zeno bench.txt
```

Type three lines and close the keyboard with Ctrl-D:

```
reading> temp 23.5
reading> hello
reading> humidity 40
reading>
3 lines typed
4 readings in bench.txt, mean 26.75, lines skipped: 1
still there: false
FileError: there is no file `bench.txt`
```

[`examples/logbook.zeno`](../examples/logbook.zeno) takes the name of a
file from its command line, starts the file with two readings, adds
every line that is typed, reads the file back, and removes it. The
decisions behind each part are 6.11 to 6.20 in
[`decisions.md`](decisions.md).

## What was added

| | Written | Gives | Can fail with |
|---|---|---|---|
| Read a file | `files.read(path)` | `str` | `FileError` |
| Write a file | `files.write(path, text)` | nothing | `FileError` |
| Add to a file | `files.append(path, text)` | nothing | `FileError` |
| Is there a file? | `files.exists(path)` | `bool` | never |
| Remove a file | `files.remove(path)` | nothing | `FileError` |
| The lines of a text | `text.lines()` | `list[str]` | never |
| A line from the keyboard | `input()`, `input("Name: ")` | `str or none` | never |
| The command line | `program.arguments()` | `list[str]` | never |
| The environment | `program.environment(name)` | `str or none` | never |
| A line on the error stream | `program.warn(text)` | nothing | never |
| End the program now | `program.exit(code)` | nothing | `ValueError` outside 0 to 255 |

`files` and `program` are modules: `import files`, `import program`.
`input` is a plain word, like `print`.

## The files

| File | What part 2 added |
|---|---|
| `src/value.rs` | `Outside`: the keyboard and the arguments, as both engines see them |
| `src/builtins.rs` | The functions, for `zeno run` and the VM |
| `src/checker.rs` | Their types, and that four of them raise `FileError` |
| `src/runtime.c` | The same functions in C, for compiled programs |
| `src/emit_c.rs`, `src/emit_llvm.rs` | A call of the C function, and a `main` that takes the command line |
| `src/main.rs` | The words after the file of `zeno run` go to the program |

## Files

```zeno
import files

files.write("notes.txt", "milk\neggs\n")     # makes it, or empties it
files.append("notes.txt", "bread\n")         # makes it, or adds to it
print(files.read("notes.txt"))               # all of it, as one str
print(files.exists("notes.txt"))             # true
files.remove("notes.txt")
```

A file is read whole and written whole. There is no `open` and no
`close`, so there is nothing to forget to close (6.12).

`write` and `append` put down the text they are given and nothing else.
A program that wants lines ends each with `"\n"` (6.13).

### What can go wrong

| What happened | The message of the `FileError` |
|---|---|
| Nothing is at the path | ``there is no file `notes.txt` `` |
| It is a folder, or the system will not let it be read | ``cannot read `notes.txt` `` |
| It is not text | `` `notes.txt` does not hold text `` |
| It cannot be made or written, such as a folder that is not there | ``cannot write `notes.txt` `` |
| It cannot be removed | ``cannot remove `notes.txt` `` |

A function that uses a file either catches the error or says it can
happen:

```zeno
function first_line(path: str) -> str raises FileError {
    return files.read(path).lines()[0]
}
```

Leave `raises FileError` out and `zeno build` (or `zeno check
--strict`) says:

```
error: `first_line` can raise FileError here but does not say so. Add `raises FileError` to it, or catch the error
```

This holds in a firmware build too. A bad index is a fault there and
stops the program, because the program could have checked the length.
No check rules out a missing file, so `FileError` travels as a value
and can always be caught (6.14).

### A file holds text

`files.read` gives a `str`, and a `str` is characters. So the file has
to be UTF-8 with no zero byte in it. Anything else is refused:

```
FileError: `logo.png` does not hold text
```

The interpreter asks Rust (`String::from_utf8`). A compiled program has
its own test, `zeno_is_text` in `runtime.c`, about forty lines. The
first byte of a character says how many follow:

```
0xxxxxxx                             one byte: the ASCII characters
110xxxxx 10xxxxxx                    two bytes
1110xxxx 10xxxxxx 10xxxxxx           three
11110xxx 10xxxxxx 10xxxxxx 10xxxxxx  four
```

and four first bytes hold the byte after them to a tighter range. That
rules out a character written in more bytes than it needs (`C0 80`),
the numbers Unicode keeps free (`ED A0 80`), and anything past the last
character (`F4 90 80 80`). The test
`what_is_text_is_the_same_in_every_build` gives both tests nine files
and requires the same answer about each.

Files are opened as bytes (`"rb"`, `"wb"`, `"ab"`), so nothing is
changed on the way in or out. A file from Windows arrives with its
`"\r\n"` line ends, and `lines()` deals with them.

## `lines()`

```zeno
for line in files.read("notes.txt").lines() {
    print(f"[{line}]")
}
```

| Text | `.lines()` |
|---|---|
| `""` | `[]` |
| `"one"` | `["one"]` |
| `"one\n"` | `["one"]` |
| `"one\ntwo"` | `["one", "two"]` |
| `"one\r\ntwo\r\n"` | `["one", "two"]` |
| `"\n"` | `[""]` |
| `"a\n\nb"` | `["a", "", "b"]` |

A line ends at `"\n"`. A `"\r"` just before that `"\n"` goes with it.
The last line need not end.

`split("\n")` is not the same: it gives `"a\nb\n"` a third, empty,
piece, and leaves the `"\r"` on. The C is a dozen lines:

```c
static zeno_list *zeno_lines(const char *text) {
    zeno_list *lines = zeno_list_new(0);
    const char *start = text;
    while (*start != '\0') {
        const char *end = strchr(start, '\n');
        size_t length = end != NULL ? (size_t)(end - start) : strlen(start);
        const char *next = start + length + (end != NULL ? 1 : 0);
        if (end != NULL && length > 0 && start[length - 1] == '\r') length--;
        zeno_list_append(lines, (zeno_slot){.s = zeno_piece(start, length)});
        start = next;
    }
    return lines;
}
```

In the interpreter it is one call, because Rust's own `str::lines` has
the same rules.

## `input()`

```zeno
let name = input("Name: ")
if name is none {
    print("nobody there")
} else {
    print(f"Hello, {name}!")
}
```

`input()` gives the next line without its line end. With a question,
it prints the question first and stays on that line. When there are no
more lines it gives `none`: Ctrl-D at a keyboard, or the end of a file
that was piped in (`zeno run count.zeno < notes.txt`).

Because the answer is `str or none`, the checker will not let it be
used as text until it has been tested:

```
error: this is str or none. Check it with `is not none` before using `.upper`
```

A loop that reads everything:

```zeno
var count = 0
while true {
    let line = input()
    if line is none {
        break
    }
    count += 1
}
print(count, "lines")
```

After `if line is none { break }` the checker knows `line` is a `str`
for the rest of the loop.

### How a line is read

`zeno_input` in `runtime.c` reads one byte at a time with `fgetc`:

```
'\n'        the line is finished
'\r'        held back: it is dropped if '\n' comes next, kept if not
'\0'        left out, because text ends at a zero byte
end         the line is finished, if there was anything in it
anything    added to the line
```

Nobody knows how long the line will be, so it grows as it is read, and
the two kinds of build grow it differently (`zeno_line_add`):

- **With a collector** the line is a block. When it is full it moves to
  a block twice the size, and the collector takes the old one.
- **With no collector** it grows in the text room, which is a stack:
  each byte goes on top of the one before. The line takes exactly its
  own length.

The interpreter's version is `Outside::line` in `value.rs`. It reads a
whole line with Rust's `read_until`, then takes off the line end and
any zero bytes, so that both come to the same text.

## `program.arguments()`

```zeno
import program

let words = program.arguments()
if len(words) != 2 {
    print("usage: copy <from> <to>")
}
```

```
zeno run copy.zeno a.txt b.txt        ["a.txt", "b.txt"]
./copy a.txt b.txt                    ["a.txt", "b.txt"]
```

The program's own name is not in the list, so `words[0]` is the first
thing the user wrote (6.19).

With `zeno run`, every word after the file is the program's, even one
that looks like an option of Zeno's. Zeno's options go before the file:

```
zeno run --vm copy.zeno a.txt --strict     the program gets ["a.txt", "--strict"]
```

In a compiled program the words come from C's `main`. A program that
never asks keeps the `main` it always had:

```c
int main(int argc, char **argv) {       /* was: int main(void) */
    ...
    zeno_program_start(argc, argv);     /* the runtime keeps both */
    zeno_program();
    return 0;
}
```

```llvm
define i32 @main(i32 %argc, ptr %argv) {
entry:
  ...
  call void @zeno_program_start(i32 signext %argc, ptr %argv)
  call void @zeno_program()
  ret i32 0
}
```

`zeno_program_arguments` then makes a new list from `argv[1]` onward
at each call.

The module has a second function since, `program.run(name, words)`,
which starts another program with words of its own. It has a page to
itself: [`running-programs.md`](running-programs.md).

Three more came later: `program.exit(code)`, `program.environment(name)`
and `program.warn(text)`, on a page of their own:
[`ending-a-program.md`](ending-a-program.md).

## One function, through the engines

Follow `files.read(path)`.

**1. `zeno run` and the VM** (`builtins.rs`). One Rust function, in the
table for `import files`:

```rust
fn files_read(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    let path = paths("read", &args, 1, span)?[0];
    match at(path) {
        At::Nothing => return file_error(format!("there is no file `{path}`"), span),
        At::Other => return file_error(format!("cannot read `{path}`"), span),
        At::File => {}
    }
    let Ok(bytes) = std::fs::read(path) else {
        return file_error(format!("cannot read `{path}`"), span);
    };
    match String::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => Ok(Value::str(text)),
        _ => file_error(format!("`{path}` does not hold text"), span),
    }
}
```

**2. The checker** (`checker.rs`, `builtin`). The argument is a `str`,
the answer is a `str`, and the function it is called in can now raise:

```rust
if short != "exists" {
    self.can_raise("FileError", span);
}
```

That one line is what makes a function say `raises FileError`.

**3. The runtime** (`runtime.c`). `zeno_file_read` does the work and
answers `NULL`, or the message of what went wrong. Around it are two
thin faces, one for each way errors travel:

```c
#ifndef ZENO_FIRMWARE
static const char *zeno_files_read(const char *path, int line, int col);                    /* jumps */
#else
static bool zeno_files_read(const char *path, int line, int col, const char **result);      /* says */
#endif
```

**4. The C backend.** In an ordinary build the call is an expression,
and an error jumps out of it:

```c
static const char *z_first(const char *z_path) {
    return (*zeno_list_at(zeno_lines(zeno_files_read(z_path, 5, 12)), 0, 5, 12)).s;
}
```

In a firmware build the same line is a test, and the function passes
the failure to its own caller:

```c
static bool z_first(const char *z_path, const char **zeno_result) {
    const char *t2;
    if (!zeno_files_read(z_path, 5, 12, &t2)) return false;
    *zeno_result = (*zeno_list_at(zeno_lines(t2), 0, 5, 12)).s;
    return true;
}
```

**5. The LLVM backend.** The firmware build again:

```llvm
define internal zeroext i1 @z_first(ptr %arg.path, ptr %result) {
entry:
  %answer.1 = alloca ptr
  %t1 = call zeroext i1 @zeno_files_read(ptr %arg.path, i32 5, i32 12, ptr %answer.1)
  br i1 %t1, label %worked2, label %failed2

failed2:
  ret i1 false

worked2:
  %t2 = load ptr, ptr %answer.1
  %t3 = call ptr @zeno_lines(ptr %t2)
  ...
```

Nothing new was needed in either backend for the firmware build: a
runtime function that says whether it worked is called the way
`int(text)` already was (`checked_call`).

## With no collector

The text of a file and a line from `input()` are built text, like the
answer of `upper()`: they are in the text room until the statement
ends, and text that is kept is copied to the pool
([`text.md`](text.md)). `lines()` and `program.arguments()` make their
items in the pool directly.

So the size of the text room is the size of the largest file that can
be read. It is 1024 bytes unless the C is compiled with another:

```
error: a statement built more text than there is room for (ZENO_TEXT_ROOM is 1024 bytes)
```

```sh
zeno build --no-collector copy.zeno
cc -O2 -DZENO_TEXT_ROOM=8192 -o copy copy.c -lm
```

`zeno build --no-collector --memory` prints how much of the room a run
used. The logbook example, with three lines typed:

```
memory: 35 blocks made (661 bytes), at most 214 bytes in use at once, no collector, 0 blocks left at the end
pool: 384 bytes taken, in a pool of 32768
text: at most 64 bytes in use at once, in a room of 1024
```

## On a board

These functions are the C library's: `fopen`, `fgetc`, `remove`. A
firmware C library usually ties standard input to a serial line, and
then `input()` reads a line from that. Where there is no file system,
every `files` call but `exists` raises a `FileError`, which the program
can catch. The simulated board (`board.md`) is not involved.

## What it does not do

- **Large files.** A file is read whole. Reading in pieces needs an
  open file with an owner, and is left until a program needs it (6.12).
- **Bytes.** A file that is not text cannot be read at all yet (6.16).
- **Folders.** No listing, making or removing of folders.
- **`program.exit(code)` and environment variables** (6.19).
- **Typed text that is not UTF-8.** `zeno run` replaces the bad bytes
  with `�`. A compiled program passes them on as they came.

## How it is tested

Every engine and every build has to do the same with the same keyboard
and the same command line, so the tests supply both. A test wraps its
programs in `with_outside` (`value.rs`):

```rust
with_outside("temp 23.5\nhello\n", &["bench.txt"], || {
    no_collector_as_interpreter("logbook", source);
});
```

Inside it, the interpreter and the VM read the typed text in place of
the keyboard, and each built program is started with the arguments and
has the text piped to its standard input. With no `with_outside`,
nothing is typed and there are no arguments, so no test can sit waiting
for a keyboard.

Each program that writes a file removes it again, so the eight runs of
one test (interpreter, VM, and C and LLVM in three builds) each start
from the same. Every test's file has its own name in the system's
temporary folder.

The tests:

- `files_are_the_same_in_every_build`: all five functions, the error
  messages, caught in a function and passed on by one.
- `an_uncaught_file_error_is_the_same_in_every_build`: the same message
  and the same line and column.
- `what_is_text_is_the_same_in_every_build`: nine files of bytes.
- `the_keyboard_and_the_arguments_are_the_same_in_every_build`: a line
  of 300 characters, `"\r"` in every position, an empty line, a zero
  byte, Tamil, and no line end on the last line.
- `the_logbook_example_is_the_same_in_every_build`.
- `main_takes_the_command_line_only_when_the_program_reads_it`.
- In the interpreter and the checker: the exact output and the exact
  messages.

There are 373 tests in all.

## Things to try

1. Run `zeno run examples/logbook.zeno` and type a few readings. Then
   put them in a file and run `zeno run examples/logbook.zeno <
   readings.txt`. Why does `reading> ` appear once more than there are
   lines?

2. In `logbook.zeno`, change the first `files.write` to `files.append`
   and put a `#` in front of `files.remove(path)`. Run it twice with
   nothing typed (`< /dev/null`). The second run reports 4 readings.
   Which decision says why `write` would have reported 2?

3. Take `raises FileError` off `summary` and run `zeno check --strict
   examples/logbook.zeno`. Read the message, then fix it the other way:
   keep the `raises` off and catch the error inside `summary`.

4. Run this, and say where the third item of the first list comes from:

   ```zeno
   print("a\nb\n".split("\n"))
   print("a\nb\n".lines())
   ```

5. Write `copy.zeno`: with two arguments it copies the first file to
   the second, with any other number it prints how to use it, and a
   `FileError` is caught and its message printed. Build it, run
   `./copy copy.zeno again.zeno`, and compare the two with `cmp`.

6. Build your `copy.zeno` with `--no-collector --memory` and copy a
   file of more than 1024 bytes. Read the message, then compile the C
   again with `-DZENO_TEXT_ROOM=8192` as shown above. What does the
   `text:` line say the copy needed?

7. In a shell, `printf '\xC0\x80' > odd.bin`, then read `odd.bin` with
   `files.read`. Find the line of `zeno_is_text` in `src/runtime.c`
   that refuses it. Which bytes would the same character be, written
   properly?

8. `input()` leaves out zero bytes. Find the two places that do it, one
   in `src/value.rs` and one in `src/runtime.c`. What would go wrong in
   a compiled program if only the Rust one did?
