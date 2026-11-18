# Bytes from and to a file

A firmware image, a capture from a logic analyser, a record a board
wrote to its flash: none of them is text, and `files.read` refuses
them (``FileError: `logo.png` does not hold text``). Two functions
read and write a file as its bytes, each byte a `u8` in a `list[u8]`.

```zeno
import files

let frame: list[u8] = [0x7E, 0x01, 0x00, 0xFF]
files.write_bytes("frame.bin", frame)

let back = files.read_bytes("frame.bin")
print(len(back))      # 4
print(back)           # [126, 1, 0, 255]
print(back[0])        # 126
```

| Call | Gives | Raises |
|---|---|---|
| `files.read_bytes(path)` | `list[u8]`: every byte of the file, as it is | `FileError` |
| `files.write_bytes(path, bytes)` | nothing: the file is made, or what it held is replaced | `FileError` |

The reasons for each choice are in chapter 36 of
[`decisions.md`](decisions.md). The example
[`examples/hexdump.zeno`](../examples/hexdump.zeno) shows any file
sixteen bytes to a line, as `xxd` does, with a CRC-16 of the whole.

## A byte is a `u8`

There is no new type to learn. A byte is a `u8`, a number from 0 to
255, and a file of bytes is a list of them, so everything a list does
works: `len`, `for`, an index, a slice, `map`, `==`.

```zeno
var sum = 0
for byte in files.read_bytes("frame.bin") {
    sum = (sum + byte) % 256
}
print(sum)            # 126
```

`files.read_bytes` changes nothing and refuses nothing. A zero, a byte
that is no UTF-8, a `"\r\n"` from Windows: each arrives as its number.
The only errors are the ones `files.read` has too: no file, or one that
cannot be read.

```
FileError: there is no file `frame.bin`
```

## What goes into a file

`files.write_bytes` takes a `list[u8]`, so every item already fits in
a byte. A number written out that does not fit is found before the
program runs:

```
error: 256 does not fit in a u8 (0 to 255)
 --> frame.zeno:2:32
  |
2 | files.write_bytes("frame.bin", [1, 256])
  |                                ^
```

A list of `int`s is refused too, because it could hold anything:

```
error: the bytes given to `files.write_bytes` should be list[u8], but this is list[int]
```

Say where the check is, and it is made there:

```zeno
let numbers = [72, 105, 10]
files.write_bytes("hi.txt", [u8(n) for n in numbers])   # OverflowError if one does not fit
print(files.read("hi.txt"))                              # Hi
```

A list that came from `json.parse` says what it holds only when the
program runs. Its items are looked at one by one, and one that is no
byte is a `ValueError`: `300 is not a byte: a byte is a number from 0
to 255`.

## Text and bytes

Text written with `files.write` is UTF-8, so reading it as bytes shows
how each character is stored:

```zeno
files.write("name.txt", "é")
print(files.read_bytes("name.txt"))     # [195, 169]
```

And bytes that are UTF-8 can be read back as text with `files.read`.

## In a build with no collector

The list is new at each call, so an owner can keep it:

```zeno
let image: owned list[u8] = files.read_bytes("image.bin")
```

## How it is built

**The checker** (`checker.rs`) knows `files.read_bytes` takes a `str`
and gives a `list[u8]`, and that `files.write_bytes` takes a `str` and
a `list[u8]`. A list written out in the call, such as `[1, 2]`, is
taken as a `list[u8]` when its numbers fit, as it is for any parameter
of that type. Both can raise `FileError`, in every kind of build.

**`zeno run` and the VM** (`builtins.rs`) read with `std::fs::read`
and make each byte an `Int`.

**C and LLVM** (`runtime.c`) call one pair of functions:

```c
static const char *zeno_file_read_bytes(const char *path, zeno_list **result) {
    ...
    zeno_list *bytes = zeno_list_new((int64_t)size);
    unsigned char piece[512];
    ...
        for (size_t i = 0; i < got; i++) bytes->items[bytes->len++].i = piece[i];
```

The list is made with room for the whole file, and filled 512 bytes at
a time. In a firmware build the call says whether it worked, and the
list comes back through a pointer, as `files.read` does there.

**The target languages** (`runtime.*`) each use their own: Python
`list(f.read())`, JavaScript `Array.from(fs.readFileSync(path))`, Ruby
`File.binread(path).bytes`, Go `os.ReadFile`, Java
`Files.readAllBytes`, and so on, with each byte made a number of that
language.

**What it costs today.** A `list[u8]` in a compiled program still
keeps each item in eight bytes, because the runtime has one kind of
list (5.13a). A file of a megabyte takes eight megabytes as a list.
The next stage stores a `list[u8]` one byte to an item.

## How it is tested

- `cargo test`: `a_file_is_read_and_written_as_bytes` in both engines,
  with the messages; `bytes_from_and_to_a_file_are_the_same_in_every_build`
  holds the C build, the firmware build, the build with no collector and
  LLVM to the interpreter.
- `tools/targets.py`: `tools/targets/edge-bytes.zeno` (every byte from
  0 to 255, the files `files.read` refuses, a folder, a missing folder,
  items that are no byte) and `examples/hexdump.zeno`, in all fifteen
  languages.

## Things to try

1. Run `examples/hexdump.zeno` on the `zeno` program itself
   (`target/release/zeno`). What are its first four bytes? Look up what
   an ELF file begins with.
2. Write `copy.zeno`, which copies any file with `read_bytes` and
   `write_bytes`. Copy a picture, and check with `cmp` that nothing
   changed.
3. Write a function that says whether a file is a PNG: its first eight
   bytes are `137, 80, 78, 71, 13, 10, 26, 10`.
4. Write a file with `files.write("t.txt", "a\r\nb")` and read it with
   `read_bytes`. Which number is the `"\r"`? Why does `lines()` give
   two lines and not three?
5. Try `files.write_bytes("x.bin", [1, 2, 300])`. Then put the list in
   a `let` with no type first, and pass that. Why are the two messages
   different?
6. Write a frame of the serial protocol in `examples/protocol.zeno` to
   a file: a start byte, the command as `u8(command)`, and an end byte.
   Read it back, and turn the command byte into a case with
   `Command(byte)`.
7. Change `crc16` in `examples/hexdump.zeno` to the CRC-16 that Modbus
   uses (start from `0xFFFF`, take the bits from the bottom, polynomial
   `0xA001`). The CRC of the nine bytes `123456789` must be `4b37`.
