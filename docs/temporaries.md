# What a statement owns

A build with no collector used to refuse each of these lines:

```zeno
print(count(line.split()))
print("led" in line.split())
for word in words(line).map(loud) {
    print(word)
}
```

Each makes a list that is given to nobody. `line.split()` is counted and
never named. With a collector that is fine: the list is left behind and
collected later. With none, "nothing would free it" was the whole reason
for the refusal.

Now something does free it: **the statement it is in**.

```sh
cargo run -- build --no-collector --memory examples/words.zeno
./words
```

```
5
5
true
SET
LED
3
ON
250
["3", "250"]
["set", "led", "3", "on", "250", "end"]
set/2
refused: an empty line
stop/0
memory: 219 blocks made (3314 bytes), at most 505 bytes in use at once, no collector, 0 blocks left at the end
pool: 1160 bytes taken, in a pool of 32768
text: at most 79 bytes in use at once, in a room of 1024
```

[`examples/words.zeno`](../examples/words.zeno) splits a line from a
serial console into words nine times, and gives a name to only a few of
the lists it makes on the way. The report still ends with **0 blocks
left**.

This page follows on from [`text.md`](text.md). It is the second of five
steps that take the rules of that build from "refuse what has no owner"
to "whoever receives a value owns it".

## The idea: what is given to nobody belongs to the statement

A list that was just made is nobody's yet. One of four things happens to
it next, and each has an answer to "who frees it?":

| The list goes to | Example | Who frees it |
|---|---|---|
| an owner | `let parts: owned list[str] = line.split()` | the owner, when its block ends |
| nobody | `print(count(line.split()))` | the statement, when it is over |
| a `for` loop | `for word in line.split() { ... }` | the loop, after its last trip |
| a place that is not an owner | `let parts = line.split()` | nobody could, so it was refused. Since [`one-owner.md`](one-owner.md) every such place is an owner |

The second row is the new one:

> **A value that is made in the middle of a statement and given to nobody
> belongs to that statement, which frees it when it is over.**

Rust and C++ have the same rule and call such a value a *temporary*.
It is the rule you would guess. In `print(count(line.split()))` the list
is needed until `count` has answered, and there is no way to mention it
after the statement, so the end of the statement is the first moment
that is certainly late enough and the last that is certainly reachable.

Nothing new is written in the program. There is no keyword for it and
no choice to make, which is the test "unambiguous and intuitive" asks
for: a line that reads as "split, count, print" does exactly that, and
the build that has no collector agrees with the one that has.

## The files

- [`src/checker.rs`](../src/checker.rs): `made` (which expressions make
  a new list), `fresh_list` (a made list can start life owned) and
  `lent` (an owned value that is only looked at).
- [`src/emit_c.rs`](../src/emit_c.rs), the part headed "What a statement
  owns": `own_begin`, `own_end`, `belongs`. The C that walks and fills
  lists is in `over_list` and `open_list_loop`.
- [`src/emit_llvm.rs`](../src/emit_llvm.rs): the same three names.
- [`src/runtime.c`](../src/runtime.c): `zeno_piece` and
  `zeno_list_keep_text`.

## What makes a list

Six things make a new list:

```zeno
[1, 2, 3]                      # a list written out
[n * n for n in items]         # a comprehension
line.split()                   # also line.split(",")
items.map(double)
items.filter(big)
left + right                   # two lists joined
```

The compiler marks the value each of these gives as *made*
(`Val::made`). Then `belongs`, which every expression passes through,
asks one question: is somebody getting it? The checker has the answer.
It notes each made list that arrives where an **owned list is wanted**:
a `let` with an owned type, an owned parameter, an owned field, a
`return` from a function that gives back an owned list. Those start
life owned (`Types::fresh`), and nothing more is done here. Any other
made list becomes the statement's.

That is why this works, with no `owned` written on the right:

```zeno
function words(line: str) -> owned list[str] {
    return line.split(" ").filter(not_empty)
}
```

`split` makes a list that goes to nobody: `filter` only reads it. It is
the `return` statement's, which frees it. `filter` makes a second list,
and that one is wanted as an owned list, so it is given to the caller.

## The C: declare, make, free

```zeno
let line = "set led on"
print(longest(line.split()))
```

```c
zeno_list *t1 = NULL;                        // where the statement begins
t1 = zeno_split_blanks(z_line);
zeno_print(1, zeno_str_int(z_longest(t1)));
zeno_list_drop_text(t1);                     // where it ends
```

The statement puts the list in a C variable of its own, and one line at
its end frees it. (The two lines about the text room that surround
these are left out. They are from [`text.md`](text.md).)

Why declare the variable first and fill it on the next line? Because of
where a value may be made. A comparison such as `a < f(x) < g([1, 2])`
becomes nested `if`s in C, and a list made inside an `if` cannot be
freed by a line after it unless the variable is declared outside. So
every such variable is declared where the statement begins, holding
`NULL`, and freeing `NULL` does nothing. `own_end` inserts those
declarations once the statement has been compiled, which is the first
moment it knows how many there are.

### Leaving early

A statement can fail half way. In a firmware build that is a `return
false` in the middle of it ([`firmware.md`](firmware.md)):

```zeno
function pick(text: str) -> str raises ValueError {
    return str(count(text.split())) + first(text)
}
```

```c
zeno_list *t4 = NULL;
size_t m7 = zeno_text_top;
t4 = zeno_split_blanks(z_text);
const char *t6 = zeno_str_int(z_count(t4));
const char *t5;
if (!z_first(z_text, &t5)) { zeno_list_drop_text(t4); return false; }
const char *t7 = zeno_text_return(m7, zeno_join(2, t6, t5));
zeno_list_drop_text(t4);
*zeno_result = t7;
return true;
```

The list is freed on both ways out. Nothing new was built for this: the
value is **held** from the line that makes it (`hold`, from
[`no-collector.md`](no-collector.md), "A value between two owners"), so
every early way out of the block frees it along with what the block's
owners hold. `return`, `break`, `continue` and a failed call all go
through that one path.

## Where a statement ends

For `let`, an assignment and a call on a line of its own, the end is
the end of the line. The others each have a rule, and it is always
"as soon as nothing can still be looking":

- **A condition** is over when it has its answer. `if`, `else if` and
  `while` free what their condition made before the block runs, and a
  `while` does so on every trip.
- **Each side of `and` and `or`, and what `not` is in front of**, is a
  condition too. The right side of `and` may never run, so what it
  makes is freed inside the `if` that guards it:

  ```zeno
  let ok = len(line) > 4 and len(line.split()) == 3
  ```

  ```c
  bool t4 = (zeno_len(z_line) > 4);
  if (t4) {
      zeno_list *t2 = NULL;
      t2 = zeno_split_blanks(z_line);
      bool t3 = (t2->len == 3);
      zeno_list_drop_text(t2);
      t4 = t3;
  }
  z_ok = t4;
  ```

- **One item of a comprehension**: `[count(w.split(",")) for w in ws]`
  frees each little list before it works out the next item.
- **`for` and `match`** keep what their first line made until their last
  block is over. `for x in make().data` walks a list inside an object
  that the statement owns, so the object has to last as long as the
  loop does. Rust does the same.
- **`return` and `raise`** leave the function, so they free what the
  statement owns on the way out, with everything else.

## Only looked at, so it cannot be kept

A value the statement owns is gone when the statement is. So it may be
looked at, and nothing may keep it. The compiler marks it
(`Val::passing`) and `stays`, which already guards every place a value
is kept, refuses it:

```
error: this list has no owner. Give it one, as in `let items: owned list[int] = [1, 2, 3]`. `zeno build --no-collector` leaves the collector out, so nothing would free it. docs/no-collector.md lists what such a build accepts
 --> words.zeno:2:13
  |
2 | let parts = line.split()
  |             ^
```

Looking is: a built-in (`len`, `print`, `in`, `sum`), an index, a loop,
a method of the list (`map`, `filter`, `reduce`), and a parameter.
Every parameter looks unless it says `take`, and looking is a promise
the checker already enforces: the function cannot keep, return or give
away what it was lent ([`ownership.md`](ownership.md)). So
`count(line.split())` is safe by a rule that was there before this
stage.

A parameter that says `take` is handed the list instead
(`total(words: take list[str])`). Then nothing is left for the
statement to free: `total` owns the list and frees it when it returns.

A field of an object the statement owns is marked the same way:
`make().data` may be walked and counted and not kept.

## Lists of text

With no collector a list has **its own copy** of each text in it
([`text.md`](text.md), "Objects and lists"). The lists made here follow
that rule, each in the cheapest way there is:

- **`split`** and the characters of `for ch in text` are cut straight
  into the pool (`zeno_piece` in `runtime.c`). They never pass through
  the text room.
- **`map`** with a function that gives text: each answer arrives in the
  text room, the new list takes a copy, and the room is given back
  before the next call. So a `map` over a thousand items needs room for
  one answer and not for a thousand:

  ```c
  zeno_list *t7 = zeno_list_new(t5->len);
  size_t m4 = zeno_text_top;
  for (int64_t t6 = 0; t6 < t5->len; t6++) {
      zeno_list_append(t7, (zeno_slot){.s = zeno_text_keep(...loud(t5->items[t6].s)...)});
      zeno_text_top = m4;
  }
  ```

- **`filter`** copies each item it takes.
- **`+`** joins the two lists and then gives the new one copies of its
  own (`zeno_list_keep_text`).

Every one of them is freed by `zeno_list_drop_text`, which frees the
items and then the list.

## Loops

A `for` loop walks the list *as it was when the loop began*, so it
walks a copy (decision 3.10a). Two kinds of list need no copy, because
nothing the body does can reach them:

- **A list made right there**: `for word in line.split()`. The loop
  walks the list itself and frees it after the last trip. That was
  already so for `for pin in [2, 3, 5]`. Now it is so for all six ways
  of making a list, and for an owned list that a function gives back
  (`for word in words(line)`).
- **A list the statement owns**: `for x in make().data`. The loop walks
  it where it is, and the statement frees the object after the loop.

`map`, `filter` and `reduce` skip their copy for the same reason when
the list they are called on is the statement's:

```zeno
for word in words(line).map(loud) {
    print(word)
}
```

```c
zeno_list *t4 = NULL;
t4 = z_words(z_line);              // the statement's
zeno_list *t5 = t4;                // `map` walks it as it is
...the loop that fills t7...
zeno_list *t8 = t7;                // the `for` loop walks what `map` made
for (int64_t t9 = 0; t9 < t8->len; t9++) {
    const char *z_word = t8->items[t9].s;
    zeno_print(1, z_word);
}
zeno_list_drop_text(t8);           // the loop frees its list
zeno_list_drop_text(t4);           // the statement frees its own
```

**`for ch in text`** was refused, because it makes a list of the
characters. That list is now the loop's own, like any other made list.

## `+=` on an owned list

`xs += ys` means `xs = xs + ys` everywhere else in Zeno, and the checker
used to refuse it for an owned list. It is accepted now, in every build:
`+` makes a list, and the owner takes it and frees the one it had.

```zeno
var history: owned list[str] = []
history += words(line)
```

```c
zeno_list *t14 = NULL;
zeno_list *t15 = z_history;
t14 = z_words(z_line);
zeno_list *t16 = zeno_list_keep_text(zeno_list_concat(t15, t14));
zeno_list_drop_text(z_history);    // the old list
z_history = t16;
zeno_list_drop_text(t14);          // the statement's
```

Where the variable or field on the left was not an owner, a build with
no collector refused the line. Every variable and field that holds a
list is an owner there now ([`one-owner.md`](one-owner.md)).

## The same rule, in every build

The checker had a refusal of its own, in every build:

```
error: this Packet has no owner yet. Give it a name with `let` first, and use the name here
```

for `print(size(make()))`, where `make` gives back an owned value. That
was the same question, "who frees it?", and it now has the same answer.
The checker notes the expression (`Types::lent`) and both compilers
give the value to the statement, with a collector or without:

```zeno
total += last(make(round)) + make(round).id % 2
```

```c
z_Packet *t3 = NULL;
z_Packet *t4 = NULL;
t3 = z_make(z_round);
int64_t t5 = z_last(t3);
t4 = z_make(z_round);
z_total = zeno_add(z_total, zeno_add(t5, zeno_mod(t4->z_id, 2, 10, 38), 10, 18), 10, 9);
zeno_drop_Packet(t4);
zeno_drop_Packet(t3);
```

Four hundred trips of that make six megabytes and the collector never
runs. A made list that is *not* owned (a plain `list[int]`) is left to
the collector where there is one, as before.

## Through LLVM

```sh
cargo run -- build --llvm --no-collector --memory examples/words.zeno
```

The IR keeps the value's address in a **slot**, as it does for an owner
([`llvm.md`](llvm.md), "Owned values"):

```llvm
  store ptr null, ptr %own.1                 ; where the statement begins
  %t1 = load ptr, ptr @z_line
  %t2 = call ptr @zeno_split_blanks(ptr %t1)
  store ptr %t2, ptr %own.1
  %t3 = call i64 @z_longest(ptr %t2)
  %t4 = call ptr @zeno_str_int(i64 %t3)
  call void (i32, ...) @zeno_print(i32 1, ptr %t4)
  %t5 = load ptr, ptr %own.1                 ; where it ends
  call void @zeno_list_drop_text(ptr %t5)
```

A slot, and not simply `%t2`, for the reason C declares its variable
first: in IR a name made in a block that may be skipped cannot be used
after it. The `store ptr null` line is inserted where the statement
began, as the line that notes the text room's top is.

The two builds make the same blocks in the same order, and the tests
require the same memory report from both, line for line.

## What it costs

Nothing, compared with naming the list:

```zeno
print(len(line.split()))                       # 5 blocks, 67 bytes

let words: owned list[str] = line.split()      # 5 blocks, 67 bytes
print(len(words))
```

The list is made once and freed once either way. What differs is how
long it is held: to the end of the statement, or to the end of the
block.

Three costs that are worth knowing:

- **`for ch in text` takes a block for every character** while the loop
  runs: 24 bytes of the pool each. For a line from a console that is
  nothing. For long text, `for i in range(len(text))` with `text[i]`
  takes no block at all.
- **`filter` on a list of text copies** each item it takes, and `+`
  copies every item of both lists.
- **A `for` or a `match` holds what its first line made until it is
  over.** `for x in make().data` keeps the whole object for the length
  of the loop.

## What this does not claim

- When this page was written, a plain `list[T]` parameter, variable or
  field could not keep a made list in a build with no collector, and
  nested lists (`[[1, 2], [3]]`) were refused. Both work since the next
  step: [`one-owner.md`](one-owner.md).
- **The statement's end is not the last use.** `print(len(a.split()),
  slow())` holds the list while `slow()` runs. Rust makes the same
  choice. A name and a block of your own (`let`) is how to say
  otherwise.
- **The text room's high-water mark can differ between the C build and
  the LLVM build** when one statement both shows a list and builds other
  text (decision 5.9q, "Known gap"). The blocks and the bytes do not.

## How it is tested

- `with_no_collector_a_statement_owns_the_lists_it_makes`,
  `..._made_lists_of_text_keep_their_own_copies` and
  `..._what_a_statement_owns_is_freed_on_every_way_out` in `emit_c.rs`:
  three programs that between them use every kind of made list in a
  call, a condition, a loop, a comprehension, a `match`, a `return`, and
  a statement that fails half way. Each goes through
  `no_collector_as_interpreter`: the same output as `zeno run`, 0 blocks
  left, the same in the two builds with a collector, and the same
  memory report through LLVM.
- `with_no_collector_made_lists_do_not_fill_the_pool_over_time`: 5000
  trips need exactly the pool that 50 trips need.
- `the_c_for_what_a_statement_owns` and
  `the_ir_for_what_a_statement_owns` pin down the code on this page.
- `an_owned_value_that_is_only_looked_at_is_freed_in_every_build`: with
  a collector, the collector has nothing to do.
- The table in `what_one_owner_turns_down_says_so` lost six
  rows and gained seven: every way a made list can be kept by something
  that is not an owner.
- By hand: each trial program was also built with freed pool blocks
  overwritten with `#` and given-back text room with `~`, in both
  backends, and still printed what `zeno run` prints.

## Things to try

1. Build `examples/words.zeno` with `--no-collector --memory`. Then give
   the first list a name (`let parts: owned list[str] = line.split()`
   and `print(count(parts))`). The report says the same number of
   blocks. Which two numbers change, and why?
2. In `words.zeno`, write `take` in front of the type of `count`'s
   parameter and build with `--no-collector --memory`. The report is
   the same. Open `words.c`: which line frees the list now, and which
   line no longer does?
3. Write `let both = line.split() + line.split()` and read the error.
   Fix it two ways: with a type on the `let`, and by not naming it.
4. Open `words.c` and find `z_words`. Two lists are made there. Follow
   each to the line that frees it. One of those lines is not in this
   function: where is it?
5. `if "x" in line.split() or "y" in line.split()` makes two lists.
   Look at the C. When is the second one made, and where is each freed?
6. `for ch in text` makes a list of every character. Change
   `open_list_loop` so that a loop over a str walks the text itself,
   one character at a time, with `zeno_char_size`. What happens to
   text built in the loop's first line, which is given back before the
   first trip today?
7. `filter` on a list the statement owns copies each text it takes and
   then frees the original. It could *move* the text into the new list
   and leave `NULL` behind. `zeno_list_drop_text` already skips `NULL`.
   Try it, and compare the reports.
8. `print(len(a.split()), slow())` holds the list while `slow()` runs.
   Could the compiler free it after `len`? What would it have to know
   about `len`, and what about a function of your own in its place?
