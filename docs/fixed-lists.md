# A list of a fixed length

A buffer in firmware nearly always has a size that is known when the
program is written: a frame of 8 bytes, a block of 512, a ring of 64.
`list[u8, 64]` is a list that always has 64 items.

```zeno
var buffer: list[u8, 64] = [0] * 64
buffer[0] = 0x7E
buffer[63] = 0x7F
print(len(buffer))        # 64
print(buffer[0])          # 126
```

The reasons for each choice are in chapters 36.3 and 36.4 of
[`decisions.md`](decisions.md).

## Filling a list: `[0] * 64`

`*` with a list and a number makes a new list with the items so many
times over. It works for every list, not only one of a fixed length:

```zeno
print([0] * 4)            # [0, 0, 0, 0]
print([0x7E, 0] * 2)      # [126, 0, 126, 0]
let n = 3
print(["-"] * n)          # ["-", "-", "-"]
```

The list comes first: `3 * [0]` is refused, with a message that shows
`[0] * 3`. A count below zero is a `ValueError`. A list of lists, dicts
or objects cannot be repeated, because every row would be the same row:

```
error: `*` would put the same list[int] in the list again and again, and a change to one would be a change to all. Make each one: `[[0] * 4 for i in range(3)]`
```

## Giving one a value

A list of a fixed length gets a list whose length is known before the
program runs: one written out with that many items, `[0] * 64`, or
another list of the same type and length.

```zeno
let header: list[u8, 4] = [0x7E, command, 0, 0x7F]
let zeros: list[u8, 512] = [0] * 512
let copy: list[u8, 4] = header
```

The count is checked then:

```
error: this list has 2 items, and a list[u8, 8] always has 8
```

A list whose length is only known when the program runs cannot be one:

```
error: `frame` should be list[u8, 8], but this is list[u8]. A list[u8, 8] always has 8 items, and how many this one has is only known when the program runs. Write the 8 items, or `[0] * 8`
```

## What it does

Everything a list does that leaves its length as it is: an index,
`buffer[i] = v`, `len`, `for`, `in`, `==`, `find`, `sort`, `reverse`,
`slice`, `map`, `filter`, `sum`, `min`, `max`, printing. `+` makes a new,
ordinary list.

`append`, `pop`, `insert` and `clear` would change the length, and are
refused:

```
error: `append` would change how many items this list has, and a list of a fixed length always has 64. Give an item a new value instead, as in `list[0] = 7`
```

A position written as a number is checked before the program runs:

```
error: 64 is outside a list of 64 items, whose positions are 0 to 63
```

## Passing one to a function

A parameter, a field and what a function gives back can each be a list
of a fixed length:

```zeno
class Packet {
    kind: u8
    data: list[u8, 4]
}

function frame_of(command: u8) -> list[u8, 4] {
    return [0x7E, command, 0, 0x7F]
}

let packet = Packet(1, frame_of(9))
```

And a function that takes an ordinary `list[u8]` can be given one, as
long as it never changes how many items the list has:

```zeno
function checksum(data: list[u8]) -> u8 {
    var total = 0
    for b in data {
        total = (total + b) % 256
    }
    return u8(total)
}

function fill(data: list[u8], value: u8) {
    for i in range(len(data)) {
        data[i] = value
    }
}

fill(buffer, 0xFF)
print(checksum(buffer))
```

The checker reads the function to know. If something in it could
change the length, the message names the line:

```
error: this list always has 64 items, and `pad` may change how many its `data` has: it calls `data.append` (line 11). A function that only reads a list, or gives its items new values, can be given one of a fixed length
```

What counts as a change: `append`, `pop`, `insert` and `clear`, and
anything that keeps the list where it could be changed later: a
variable, another list, a dict, a field, a `return`, or a function the
checker cannot read, such as one held in a variable. Handing the list
on to another function of the program is fine when that function keeps
the length too.

A list of a fixed length is not kept where an ordinary list is, such as
a `let` that says `list[u8]`: whatever held it there could append to
it. Copy the items if a list that can grow is wanted:

```zeno
let longer: list[u8] = buffer.slice(0)
longer.append(1)
```

## In every build

A list of a fixed length is a list in every engine, every build and all
fifteen target languages. The length is made sure of before the program
runs, so nothing is left to check while it runs. A `list[u8, 64]` takes
64 bytes in a compiled program, as any `list[u8]` does (36.2).

In a build with no collector it is a block from the pool, as any list
is, and its type is written `owned list[u8, 64]` where a list needs an
owner. Keeping its items inside the object or on the stack, with no
block at all, is the next step (36.5).

## How it is built

- **The parser** (`parser.rs`) reads a number among the types in `[ ]`
  as a length: `ast::Type::Length`.
- **The checker** (`checker.rs`) has the type `Ty::Fixed(item, length)`.
  Whatever only looks at a value sees it as a list, so `len`, an index,
  a loop and the methods that read work as they did. The places that
  keep a value see the length. `src/lengths.rs` reads a function's body
  to say whether it changes the length of a parameter.
- **`[0] * 64`** is `ops.rs` for `zeno run` and the VM,
  `zeno_list_repeat` and `zeno_bytes_repeat` in `runtime.c` for C and
  LLVM, and the `mul` of each target language's runtime.

## How it is tested

- `cargo test`: `a_list_is_repeated_with_a_star`,
  `a_list_of_a_fixed_length_keeps_its_length` (every message) and
  `a_list_of_a_fixed_length_is_the_same_in_every_build`.
- `tools/targets.py`: `tools/targets/edge-repeat.zeno` and
  `tools/targets/edge-fixed.zeno` in all fifteen languages.

## Things to try

1. Make a `list[u8, 8]` and print `len` of it. Try to `append` to it,
   and read the message.
2. Write `function sum8(data: list[u8]) -> int` that adds the bytes.
   Give it a `list[u8, 8]`. Then add `data.append(0)` at the end of the
   function, and read what the checker says now.
3. Write a ring buffer: a `list[u8, 16]`, a `var head = 0`, and a
   function `put(byte: u8)` that writes at `head` and moves it on with
   `% 16`. Why does a ring buffer never need `append`?
4. Try `let grid = [[0] * 3] * 3`. Then write it the way the message
   says, set `grid[0][0] = 1`, and print `grid`.
5. Write a class `Frame` with a field `data: list[u8, 6]`, and a
   function that gives back a frame with the CRC-16 of its first four
   bytes in the last two.
