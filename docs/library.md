# The standard library, part 1: text and lists

A language is its grammar and its library. Until now Zeno's library was
thin: text could be split and replaced but not searched, and a list
could grow at the end and shrink at the end and nothing else. This page
adds the methods that nearly every program reaches for, and shows how
one method is added to a compiler that has five engines.

Part 2, [`reading-and-writing.md`](reading-and-writing.md), adds files,
the keyboard and the command line.

```sh
cargo run -- run examples/sentence.zeno
```

```
$TEMP,21.5,19.0,22.25*07
   TEMP: median 21.5 of 21.5 19.0 22.25
$VOLT,3.3,3.1,3.4,3.2,3.3*07
   VOLT: median 3.3 of 3.3 3.1 3.4 3.2 3.3
$VOLT,3.3,3.1*00
   bad checksum: 03, not 00
$VOLT,3.3,high*21
   VOLT: `high` is not a number
$FLOW*12
   FLOW: no readings
hello
   not a sentence
VOLT > TEMP
Zeno
```

[`examples/sentence.zeno`](../examples/sentence.zeno) reads lines in the
style of a GPS receiver: a `$`, a name, readings with commas between, a
`*` and a checksum. `zeno run`, the VM and all six kinds of compiled
build print these fourteen lines, and the build with no collector ends
with 0 blocks left.

## What was added

Text:

| Written | Gives | |
|---|---|---|
| `"set speed".find(" ")` | `3` | where the piece begins, or `none` |
| `"set speed".slice(4)` | `"speed"` | from position 4 to the end |
| `"set speed".slice(0, 3)` | `"set"` | positions 0, 1 and 2 |
| `"A".code()` | `65` | the number of one character |
| `char(65)` | `"A"` | and back |

Lists:

| Written | Does |
|---|---|
| `xs.insert(0, x)` | puts `x` at position 0, and moves the rest up |
| `xs.pop(2)` | takes out the item at position 2, and gives it |
| `xs.reverse()` | turns the list round, where it is |
| `xs.sort()` | smallest first, where it is. Numbers or text |
| `xs.clear()` | empties the list |
| `xs.find(x)` | the position of the first item equal to `x`, or `none` |
| `xs.slice(1, 3)` | a new list with the items at 1 and 2 |
| `words.join(", ")` | one text, with the separator between the items |

That is all of it. What was left out, and why, is decision 6.1: if Zeno
can already say a thing in one short expression (`x in xs`), it does not
get a method as well.

## The files

| File | What it has for this stage |
|---|---|
| `src/builtins.rs` | each method, for `zeno run` and the VM; `sort`, `ends`, `char_of` |
| `src/checker.rs` | the type of each method, in `method`; who may `clear` and `pop(at)` |
| `src/parser.rs` | the message for `xs[1:3]` |
| `src/runtime.c` | each method again, in C: `zeno_list_sort`, `zeno_slice`, `zeno_char` |
| `src/emit_c.rs` | the call of each C function, in `method` |
| `src/emit_llvm.rs` | the same call in IR, and `found` |

## `find` gives a position, or `none`

```zeno
function first_word(line: str) -> str {
    let space = line.find(" ")
    if space is none { return line }
    return line.slice(0, space)
}
```

Python's `find` gives -1 when the piece is not there, and -1 is a
number, so nothing stops a program from using it as a position. Zeno's
gives `int or none`. The checker will not let `space` be used as a
number until the program has asked:

```
error: `+` does not work between int or none and int. Check for `none` first
```

After `if space is none { return line }` the lines below know it is an
`int`. A `match` does the same (decision 5.19h):

```zeno
match line.find(" ") {
    case none { return "" }
    case at { return line.slice(at + 1) }
}
```

`xs.find(item)` on a list works the same way, and compares with `==`.

## `slice` takes a part

`slice(from, to)` takes the positions from `from` up to `to`, and not
`to` itself, as `range(from, to)` counts. `slice(from)` goes to the end.

```zeno
let text = "hello, wörld"
print(text.slice(7))        # wörld
print(text.slice(0, 5))     # hello
print(text.slice(12) == "") # true: an empty slice at the very end is fine
```

Anything outside is an error, and not a shorter slice:

```
error: uncaught IndexError: slice 2 to 9 is out of range for a str of length 3
```

Positions count characters. `ö` is two bytes in UTF-8, and `slice`
never cuts it in half. In `runtime.c`, `zeno_slice` walks to each end
one character at a time with `zeno_char_size`, which reads how many
bytes a character has from its first byte.

If you write Python's `xs[1:3]`, the parser says what Zeno's way is:

```
error: a part of a list or a str is taken with `.slice(from, to)`, as in `xs.slice(1, 3)`
```

## `insert`, `pop(at)`, `clear`, `reverse`

```zeno
let xs = [3, 1, 2]
xs.insert(1, 9)         # [3, 9, 1, 2]
print(xs.pop(0))        # 3, and xs is [9, 1, 2]
xs.reverse()            # [2, 1, 9]
xs.clear()              # []
```

`insert(len(xs), x)` is allowed and is `append`. One past that is an
`IndexError`.

In C a list is a count, a room and a row of slots (`zeno_list` in
`runtime.c`). `insert` makes room by appending the item at the end,
then moves every slot from `at` on up by one, and writes the item into
the gap. `pop(at)` is the other way round: read the slot, move the rest
down, and wipe the slot that is now spare, so that a collector does not
go on thinking the list holds it.

## `sort`

```zeno
let numbers = [5, 3, 9, 1]
numbers.sort()          # [1, 3, 5, 9]
```

`sort` works on ints, floats, sized numbers and text: the things `<`
compares. A list of anything else is refused, with the reason.

The way it sorts is **heapsort**, and it is worth reading, because it
is short and it is the kind of algorithm firmware likes: it needs no
second list, it never calls itself, and it is never slow.

See the list as a tree. The items at positions `2i + 1` and `2i + 2`
are the two "below" the item at `i`:

```
position:  0  1  2  3            5
value:     5  3  9  1          /   \
                              3     9
                             /
                            1
```

A **heap** is such a tree in which no item is smaller than one below
it. So in a heap, the biggest item is at the front.

Sorting has two halves. `sift` is the one tool both use: it moves one
item down the tree, swapping it with the bigger of the two below, until
neither is bigger.

1. **Make the list a heap.** Sift each item that has something below
   it, last one first.

   ```
   5 3 9 1     sift position 1 (the 3): only 1 is below. Stay.
   5 3 9 1     sift position 0 (the 5): 9 is the bigger below. Swap.
   9 3 5 1     a heap
   ```

2. **Take the biggest off the front, again and again.** Swap the front
   with the last item of the heap. The biggest is now in its final
   place, and the heap is one shorter. The item that came to the front
   is probably too small to be there, so sift it down.

   ```
   9 3 5 1     swap front and last     1 3 5 | 9     sift:  5 3 1 | 9
   5 3 1 | 9   swap front and last     1 3 | 5 9     sift:  3 1 | 5 9
   3 1 | 5 9   swap front and last     1 | 3 5 9     done
   ```

The whole of it in C is `zeno_before`, `zeno_sift` and `zeno_list_sort`
in `runtime.c`, about thirty lines.

There is a second copy, in Rust: `sort` in `builtins.rs`. `zeno run`
could have called Rust's own `sort`, which is faster. It does not,
because a compiled program must print what `zeno run` prints (decision
5.5d), and heapsort can swap two items that compare equal. For most
values nobody can tell. For `0.0` and `-0.0` they can: the two are
equal, and print differently. With the same steps on both sides, every
engine gives `[-1.0, -0.0, 0.0, 0.5, 2.5]` for the list in the test.

## `join`, `code` and `char`

```zeno
print(["pear", "fig"].join(", "))    # pear, fig
print("A".code(), char(66))          # 65 B
print("€".code())                    # 8364
```

`join` is asked of the list. It adds up the length of every item first
(`zeno_list_join`), so the text is made in one piece and not one item
at a time.

`code` gives the number Unicode has for a character, and `char` makes
the character for a number. UTF-8 spreads the bits of that number over
one to four bytes. The first byte says how many there are, by how many
1s it starts with, and each byte after it starts with `10` and carries
six bits:

```
0xxxxxxx                              up to 127
110xxxxx 10xxxxxx                     up to 2047
1110xxxx 10xxxxxx 10xxxxxx            up to 65535
11110xxx 10xxxxxx 10xxxxxx 10xxxxxx   the rest
```

`€` is 8364, which is `0010 000010 101100` in bits, so its bytes are
`1110 0010`, `10 000010`, `10 101100`. `zeno_char_code` and `zeno_char`
in `runtime.c` are those four cases, one way and the other.

`code()` wants exactly one character, and `char` refuses a number that
is no character, 0 among them (decision 6.8).

## One method, through five engines

Follow `text.find(part)` from the source to the machine.

**1. `zeno run` and the VM** share `call_method` in `builtins.rs`:

```rust
"find" => {
    arity(name, &args, 1, 1, span)?;
    Ok(match this.find(text(name, &args[0], span)?) {
        Some(byte) => Value::Int(this[..byte].chars().count() as i64),
        None => Value::None,
    })
}
```

Rust's `find` answers in bytes, so the bytes before the place are
counted as characters. The name also goes into `STR_METHODS`, which is
how `text.find` is known to be a method at all.

**2. The checker** gives it a type, in `method`:

```rust
(Ty::Str, "find") => {
    self.arity(name, arguments, 1, 1, span);
    strs(self);
    Ty::Optional(Box::new(Ty::Int))
}
```

One argument, it must be text, and the answer is `int or none`. That
last line is the whole reason a program has to test the answer.

**3. The runtime** has the C that does the work:

```c
static int64_t zeno_find(const char *text, const char *part) {
    const char *found = strstr(text, part);
    if (found == NULL) return -1;
    int64_t position = 0;
    for (const char *p = text; p < found; p++) {
        if (((unsigned char)*p & 0xC0) != 0x80) position++;
    }
    return position;
}
```

Inside the runtime -1 is fine: no Zeno program ever sees it.

**4. The C backend** writes the call, and wraps it in `zeno_found`,
which turns -1 into the `none` of an `int or none`:

```c
z_space = zeno_found(zeno_find(z_line, " "));
if (z_space.present) {
    zeno_print(1, zeno_slice(z_line, 0, z_space.value, false, 17, 11));
}
```

An `int or none` in C is a small struct, a flag and a number. The
`17, 11` is the line and column, for the message if the slice is out
of range.

**5. The LLVM backend** writes the same call in IR, and does the work
of `zeno_found` itself, in `found`:

```llvm
%t35 = call i64 @zeno_find(ptr %t34, ptr @.str.7)
%t36 = icmp sge i64 %t35, 0                         ; found?
%t37 = select i1 %t36, i64 %t35, i64 0
%t38 = insertvalue { i1, i64 } undef, i1 %t36, 0    ; the flag
%t39 = insertvalue { i1, i64 } %t38, i64 %t37, 1    ; the position
```

Five places for one method is a lot, and it is why the library is kept
small. Decision 6.10 says what will change that: once Zeno can bring in
modules written in Zeno, what can be written in Zeno will be.

## With no collector

In a build with `--no-collector` every list and object has one owner,
and each new method has to say what it does with what it is handed.

| Method | What happens to the item |
|---|---|
| `insert(at, item)` | the list keeps it, as `append` does. Text is kept as a copy |
| `pop(at)` | it leaves the list, and whoever receives it owns it |
| `clear()` | every item is freed, then the list is emptied |
| `find(item)` | only looked at. An item made in the brackets is freed by the statement |
| `slice(from, to)` | numbers are copied, text is copied, objects are refused |
| `reverse()`, `sort()` | nothing is made and nothing freed |
| `join(sep)` | the text is built in the text room, like any built text |

`pop(at)` and `clear()` take items out, so the checker holds them to
the rules `pop()` has always had (decision 5.15e): only the owner, not
inside a loop over the same list, and on a line of its own.

```
error: the `for` on line 5 is walking a list that `tasks` owns, and is
still looking at its items. Nothing can take an item out of this list
until the loop is over
```

This is what a list of tasks compiles to:

```zeno
tasks.insert(0, Task("c"))
let first = tasks.pop(1)
tasks.clear()
```

```c
zeno_list_insert(z_tasks, 0, (zeno_slot){.p = zeno_new_Task("c")}, 5, 1);
z_first = ((z_Task *)zeno_list_pop_at(z_tasks, 1, 6, 13).p);
zeno_list *t2 = z_tasks;
for (int64_t i = 0; i < t2->len; i++) zeno_drop_Task(((z_Task *)t2->items[i].p));
zeno_list_clear(t2);
...
zeno_drop_Task(z_first);        /* at the end of the block: `first` owns it now */
```

The loop before `zeno_list_clear` is written by the compiler, because
only the compiler knows what kind of item the list holds and so which
function frees one.

## In a firmware build

A slice out of range, an `insert` or `pop` at a position that is not
there, `code()` of two characters and `char(0)` are **faults**, like a
bad index. `zeno run` and an ordinary build raise an error that can be
caught. A firmware build stops, with the same message at the same
place (see [`firmware.md`](firmware.md)). The test runs each of them in
every build and compares.

## What it does not do

- `sort` takes no function, so a list of objects cannot be sorted by a
  field yet (decision 6.6, provisional).
- `slice` of a list of objects is refused where each has one owner.
- Text methods still know only plain letters for `upper` and `lower`
  (decision 3.14).
- `char(0)` is refused (decision 6.8).
- A built-in function cannot be handed to `map` in a compiled program:
  `xs.map(str)` runs but does not build. Write
  `xs.map(function(n: int) -> str { return str(n) })`.

## How it is tested

- **The interpreter and the VM** (`interpreter.rs`):
  `text_and_list_methods_of_the_library` runs each method and each
  error through both.
- **The checker** (`checker.rs`): `text_and_list_methods_are_checked`
  for the types and the messages,
  `with_one_owner_a_list_method_that_takes_out_is_for_the_owner` for
  who may `clear` and `pop(at)`, and
  `a_firmware_build_cannot_raise_a_fault_to_a_caller` for
  `raises ValueError`.
- **The parser** (`parser.rs`): `a_part_is_taken_with_slice`.
- **Both compilers** (`emit_c.rs`):
  `text_and_list_methods_are_the_same_in_every_build` builds one
  program and `examples/sentence.zeno` six ways (ordinary, `--firmware`
  and `--no-collector`, each through C and through LLVM). Each must
  print what `zeno run` prints, and with no collector must end with 0
  blocks left and the same memory report from both backends.
  `a_position_outside_the_text_or_the_list_is_an_error_in_every_build`
  does the same for the errors, caught and not caught.

361 tests in all.

## Things to try

1. Write `function last_word(line: str) -> str`. There is no `rfind`.
   What is the shortest way with what there is? (One answer uses
   `split`, another a loop over `find` and `slice`.)
2. Sort `[3, 1, 2]` by hand with the two halves of heapsort, writing the
   list after every swap. Then put a `printf` in `zeno_sift` in a built
   `.c` file and compare.
3. Sort `[0.0, -0.0]`, print it, sort it again and print it again. The
   two zeros change places each time. Follow the two halves of heapsort
   to see why, and say why neither answer is wrong.
4. Build `examples/sentence.zeno` with `--no-collector --memory`. Then
   change `median` so that it sorts `values` itself, without `slice(0)`,
   and print `readings` after the call in `report`. The program still
   builds. What happened to the caller's list, and why is that a worse
   function?
5. In a program with a `list[Task]`, call `tasks.clear()` inside
   `for task in tasks`. Read the message, then find the line in
   `checker.rs` that makes `clear` follow the rule `pop` follows.
6. Add a method `text.count(part)` that says how many times `part` is in
   `text`. Go through the five places in "One method, through five
   engines". Which of the five could you leave out and still have
   `zeno run` work?
7. `char(0)` is refused. Take the test out of `zeno_char` and out of
   `char_of`, and print `"a" + char(0) + "b"` with `zeno run` and with a
   built program. What does each print?
