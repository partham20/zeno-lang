# Text that is kept

A build with no collector used to refuse this line:

```zeno
let line = f"channel {channel}"
```

Text built while the program runs lived for one statement and was gone.
It could be printed, and that was all. Now it can be kept: in a variable,
in a field, in a list. A function can return it and a function can be
given it.

```sh
cargo run -- build --no-collector --memory examples/labels.zeno
./labels
```

```
ch3 (channel 3, 0 notes)
probe ground (channel 3, 2 notes)
["was ch3", "was ground"]
started ch1 started ch1 renamed twice
renamed twice
probe ground at 95
100 probe ground at 99 probe ground at 99
an even total
memory: 22 blocks made (495 bytes), at most 346 bytes in use at once, no collector, 0 blocks left at the end
pool: 616 bytes taken, in a pool of 32768
text: at most 72 bytes in use at once, in a room of 1024
```

Every line of [`examples/labels.zeno`](../examples/labels.zeno) that
keeps text was refused before this stage. The report still ends with
**0 blocks left**.

This page follows on from [`no-collector.md`](no-collector.md) and
[`pool.md`](pool.md). It is the first of five steps that take the rules
of that build from "refuse what has no owner" to "whoever receives a
value owns it".

## The idea: text is never changed, so keep a copy

A list can be changed after it is made, so two names for one list can
tell that they share it: one appends, the other sees it. That is why an
owned list is *moved* when it is given away, and why the checker has
rules about it ([`ownership.md`](ownership.md)).

Text cannot be changed after it is made. `name.upper()` makes new text
and leaves `name` alone. So nobody can tell a copy of text from the
original, by any program you can write. That gives a rule with nothing
to learn:

> **Whatever keeps text has its own copy, and frees it when it is done.**

No moves, no borrows, no new errors. `let a = b` leaves both usable. A
program means exactly what it means under `zeno run`, and the checker
is not changed by one line. The price is the copying, and "What it
costs" below counts it.

This is only for builds with no collector. Where there is a collector,
text is shared as it always was, and none of the functions on this page
are in the program.

## The files

- [`src/runtime.c`](../src/runtime.c): the comment "Text that is kept"
  and the seven functions after it, then `zeno_list_drop_text` and
  `zeno_list_copy_text`, and `zeno_error_of`.
- [`src/emit_c.rs`](../src/emit_c.rs): the part headed "Kept text"
  (`keep` and `lend`), `free_code`, and the places that call them.
  Search for `is_text`.
- [`src/emit_llvm.rs`](../src/emit_llvm.rs): the same part under the
  same heading, and `arm` for `match`.

## Three places text can be

| Where | What is there | How long it lasts |
|---|---|---|
| Written in the program | `"ground"`, `"probe "` | for ever |
| The text room | text the running statement built, and a function's answer | until that statement is over |
| The pool | a kept copy | until whatever keeps it is done |

The runtime tells them apart **by address**. The room and the pool are
two arrays, so one subtraction says whether a pointer is inside one:

```c
static bool zeno_in_pool(const char *text) {
    return (uintptr_t)text - (uintptr_t)zeno_pool.bytes < (uintptr_t)ZENO_POOL;
}
```

(If `text` is below the array the subtraction wraps round to a huge
number, so one comparison checks both ends.) Text that is in neither
array was written in the program.

So a `str` needs no flag beside it saying who owns it. That keeps a
`str` one pointer, which is what C code that reads it expects.

## Keep, drop, lend, return

Four small functions do the work. Each starts by asking where the text
is.

**Keep** is called wherever text is stored: a `let` or `var`, an
assignment, a field, a list item, `append`.

```c
static const char *zeno_text_keep(const char *text) {
    if (text == NULL || !(zeno_in_pool(text) || zeno_in_room(text))) return text;
    size_t size = strlen(text) + 1;
    char *kept = zeno_alloc(size);
    memcpy(kept, text, size);
    return kept;
}
```

Text written in the program is kept as it is: it will always be there.
Anything else is copied into the pool. `NULL` is `none`, for a
`str or none`.

**Drop** is called when the keeper is done: its block ends, or it is
given other text.

```c
static void zeno_text_drop(const char *text) {
    if (text != NULL && zeno_in_pool(text)) zeno_free((void *)text);
}
```

Here are both, in `note` from the example:

```zeno
var last = "started " + label_for(1)

function note(what: str) -> str {
    let before = last
    last = what
    return before
}
```

```c
static const char *z_note(const char *z_what) {
    size_t m18 = zeno_text_top;
    const char *z_before = zeno_text_keep(z_last);
    zeno_text_top = m18;
    const char *t14 = zeno_text_keep(z_what);
    zeno_text_drop(z_last);
    z_last = t14;
    size_t m19 = zeno_text_top;
    const char *t15 = zeno_text_return(m19, z_before);
    zeno_text_drop(z_before);
    return t15;
}
```

`last = what` is three lines, and their order matters: the new copy is
made *first*, then the old text is freed, then the variable is set. With
the first two the other way round, `line = line + "*"` would free the
text it was about to read (exercise 1).

A parameter (`what`) is never dropped. It is the caller's text, lent
for the length of the call.

**Return.** `before` is about to be freed, and the caller wants its
text. So a function's answer travels in the text room:

```c
static const char *zeno_text_return(size_t mark, const char *text) {
    bool above = text != NULL && zeno_in_room(text) && (size_t)(text - zeno_text_room) >= mark;
    if (text == NULL || !(above || zeno_in_pool(text))) {
        zeno_text_top = mark;
        return text;
    }
    size_t size = strlen(text) + 1;
    zeno_text_top = mark;
    char *answer = zeno_alloc_text(size);
    memmove(answer, text, size);
    return answer;
}
```

`mark` is where the top of the room was when the `return` began. The
answer is moved down to `mark` and the top is left just above it:

```
before the return        |caller's text|
while it is worked out   |caller's text|scraps|scraps|the answer|
after zeno_text_return   |caller's text|the answer|
                                       ^mark      ^top
```

The caller's statement noted the top before it called, and puts it back
when it ends, which gives the answer back with everything else. So the
room is still a stack, and a function that returns text leaves exactly
its answer on it. `memmove` and not `memcpy`, because the answer may
overlap the place it is moved to.

Text written in the program is returned as it is. So is the caller's
own text, lower in the room (`function echo(t: str) -> str { return t }`).

**Lend** is the subtle one. Read this line from the example again:

```zeno
print(last, note("renamed twice"), last)
```

`last` is read, then `note` frees the text `last` held and gives it
other text, then `print` uses what was read. Without care, `print`
would be handed freed memory. So a read from a place that **a function
could change** lends the statement a copy, in the text room:

```c
const char *t2 = zeno_text_lend(z_last);
const char *t3 = z_note("renamed twice");
zeno_print(3, t2, t3, zeno_text_lend(z_last));
```

```
started ch1 started ch1 renamed twice
```

Which reads are lent:

| Read | Lent? | Why |
|---|---|---|
| A `var` at the top of the file | yes | any function can assign to it |
| A field, `channel.label` | yes | a method can assign to it |
| A list item, `names[0]` | yes | whoever has the list can replace it |
| A local variable, a parameter | no | no other function can reach it |
| A `let` at the top of the file | no | it never changes |

Lending text written in the program copies nothing.

**The short cut.** When text is read and stored at once, nothing can
happen in between, so it is copied straight from where it is kept:
`let before = last` above is one `zeno_text_keep(z_last)`, with no
`zeno_text_lend` inside it.

## Objects and lists

An object has its own copies of the text in its fields. The function
that makes one keeps them, and the function that ends one drops them:

```c
static z_Channel *zeno_new_Channel(int64_t z_id, const char *z_label, zeno_list *z_notes) {
    z_Channel *self = (z_Channel *)zeno_alloc(sizeof(z_Channel));
    self->z_id = z_id;
    self->z_label = zeno_text_keep(z_label);
    self->z_notes = z_notes;
    return self;
}

static void zeno_drop_Channel(z_Channel *self) {
    if (self == NULL) return;
    zeno_text_drop(self->z_label);
    zeno_list_drop_text(self->z_notes);
    zeno_free(self);
}
```

A list of text has its own copies of its items. `zeno_list_drop_text`
drops each item and then the list. Three things follow from that:

- `names[0] = text` drops what was there before it stores.
- `names.pop()` gives the statement the text and frees the list's copy
  (`zeno_text_take`: lend, then drop).
- A `for` loop walks a copy of the list (decision 3.10a). For a list of
  text the copy has **copies of the items** too
  (`zeno_list_copy_text`), so this loop prints the list as it was:

  ```zeno
  for name in names {
      names[1] = name + "?"
      print(name)
  }
  ```

## Errors that carry text

`raise TooHot(channel.label, reading)` makes an object with text in it,
and an error to carry it. The `except` that catches the error frees it
(`no-collector.md`, "Errors"), and now it must free the label as well.
The error did not know how: to the runtime an object is a `void *`. So
the error is told which function ends its object:

```c
z_TooHot *t13 = zeno_new_TooHot(zeno_text_lend(z2_channel->z_label), z2_reading);
zeno_note(zeno_error_of("TooHot", zeno_show_TooHot(t13), t13, zeno_drop_any_TooHot), 35, 23);
```

`zeno_error_drop` calls that function. Inside the `except`,
`hot.label` is a field like any other, so `hottest = f"{hot.label} at
{hot.reading}"` keeps a copy that outlives the error.

## `match` and `for`

Both were refused when their first line built text, because the text
would have had to last for the whole statement. Now each gives it back
at the earliest moment it is no longer needed.

A `for` has its numbers or its list in hand before the first trip, so
the text goes back there. A `match` needs it until a case is chosen:

```zeno
match label_for(total % 2) {
    case "ground" { print("an even total") }
    case other { print("an odd total, so", other) }
}
```

```c
size_t m13 = zeno_text_top;
const char *t9 = z_label_for(zeno_mod(z_total, 2, 70, 17));
if ((strcmp(t9, "ground") == 0)) {
    zeno_text_top = m13;
    zeno_print(1, "an even total");
} else {
    const char *z_other = zeno_text_keep(t9);
    zeno_text_top = m13;
    zeno_print(2, "an odd total, so", z_other);
    zeno_text_drop(z_other);
}
```

A name the pattern gives (`other`) keeps a copy, taken before the room
is given back, and drops it when its case ends.

## What it costs

Nothing here is free, and for firmware you should know the price.

- **A keep is a `strlen`, a block from the pool and a `memcpy`.** For
  text written in the program it is two comparisons and nothing else.
- **A kept text takes a pool block.** Blocks come in sizes of 16, 32,
  64 bytes and so on, with 8 bytes in front ([`pool.md`](pool.md)). So
  `"ch3"` kept costs 24 bytes of pool, and a 40-character line costs 72.
- **Text that comes back from a function and is kept is copied twice**:
  into the room by the `return`, into the pool by the `let`.
- **Reading a field or a list item of kept text copies it into the
  room**, unless it is stored at once. `len(channel.label)` copies the
  label to count it.
- **Nothing is shared.** Ten variables holding the same built line hold
  ten copies.

What you get for it: every one of those costs happens at a line you can
point to, takes a time that depends only on the length of that text,
and is given back at another line you can point to. The example makes
22 blocks, 495 bytes in all, and never holds more than 346.

If a loop keeps text on every trip, each trip gives its copy back, so
the pool does not grow: a test runs such a loop 50 times and 5000 times
and requires the same pool for both.

## Through LLVM

`zeno build --llvm --no-collector` does the same, with the same runtime.
A variable that keeps text is a slot, and the end of its block frees
what the slot holds:

```llvm
define internal ptr @z_note(ptr %arg.what) {
entry:
  %var.before = alloca ptr
  %mark.1 = load i64, ptr @zeno_text_top
  %t1 = load ptr, ptr @z_last
  %t3 = call ptr @zeno_text_keep(ptr %t1)
  store ptr %t3, ptr %var.before
  store i64 %mark.1, ptr @zeno_text_top
  %t4 = call ptr @zeno_text_keep(ptr %arg.what)
  %t5 = load ptr, ptr @z_last
  call void @zeno_text_drop(ptr %t5)
  store ptr %t4, ptr @z_last
  %mark.2 = load i64, ptr @zeno_text_top
  %t6 = load ptr, ptr %var.before
  %t7 = call ptr @zeno_text_return(i64 %mark.2, ptr %t6)
  %t8 = load ptr, ptr %var.before
  call void @zeno_text_drop(ptr %t8)
  ret ptr %t7
}
```

There is no `%t2`. It was the `zeno_text_lend` for reading `last`, and
when the very next thing was to keep the text, the compiler took the
line out again. That is the short cut from above, done the way IR has
to do it: an instruction is written when the value is read, so the
decision that it was not needed comes afterwards.

A function that ends an object is already `void (ptr)`, which is what
an error wants, so there is no `zeno_drop_any_TooHot` in the IR:

```llvm
%t7 = call ptr @zeno_error_of(ptr @.str.5, ptr %t6, ptr %t5, ptr @zeno.drop.TooHot)
```

The two builds print the same and write the same memory report for
every test program, as before.

## What this does not claim

- **A statement that shows a list and builds other text may fill the
  room to different heights in the C build and the LLVM build**, by a
  few bytes. C does not say in which order the arguments of a call are
  worked out, and showing a list uses the room for a moment while it
  works. What is printed is the same. The test programs keep the two
  apart. This is on the list for the audit of the language.
- **A lend copies even when nothing is called afterwards.**
  `print(channel.label)` copies the label into the room and prints the
  copy. The compiler could see that no function runs in between. It
  does not look yet.
- **The room must hold a function's answer.** A function that returns
  2000 characters needs a room of 2000, for the moment between the
  `return` and the `let`.
- **Lists made by `split`, `map`, `filter` and `+`, and `for ch in
  text`, are still refused.** They are the next step.

## How it is tested

- Four programs in `emit_c.rs` go through `no_collector_as_interpreter`
  (`no-collector.md`, "How it is tested"): the lines that used to be
  refused, every early way out with text in hand, text inside objects
  and lists, and the example. Each must print what `zeno run` prints,
  end with 0 blocks left, and write the same report through LLVM.
- `with_no_collector_kept_text_does_not_fill_the_pool_over_time` is the
  50 and 5000 trips.
- `the_three_places_text_can_be` drives the runtime's functions from C.
- `the_c_for_kept_text` and `the_ir_for_kept_text` compare what the two
  compilers write with what this page shows.

While this was built, each trial program was also run with freed pool
blocks and given-back room overwritten with `#` and `~`, so that any
text read too late would print as rubbish. None did. Exercise 6 of
`pool.md` builds the pool half of that check into the runtime.

## Things to try

1. Write a function that builds a line of stars in a loop, with
   `var line = f"{count}:"` and `line = line + "*"`, and print
   `stars(5)`. Build it with `--no-collector --memory` and find the
   three lines for the assignment in the C. Swap the first two by hand,
   compile the C yourself with `-DZENO_MEMORY_REPORT`, and run it. It
   prints `*`. Where did `5:****` go? (When a block is given back, the
   pool writes a pointer over its first 8 bytes.)
2. Build `examples/labels.zeno` the same way. In `labels.c`, take the
   `zeno_text_lend` off `t2` in the lines for
   `print(last, note("renamed twice"), last)`, so that it reads
   `const char *t2 = z_last;`. The first thing on that line of output
   is now empty. Explain it with the hint from exercise 1. Why would
   the program have printed the right thing, by luck, if `last` had
   started as `"nothing yet"`?
3. `"ch3"` kept costs 24 bytes of pool. Change `label_for` to return
   `f"channel number {id}"` and work out the new "pool: ... bytes
   taken" before you run it.
4. `print(channel.label)` lends the label although nothing is called
   after the read. In `emit_c.rs`, `sequence` already knows whether a
   later part of the statement calls a function (`calls` on `Val`).
   Use it to lend only when one does. Which of the tests' reports
   change, and do the two backends still agree?
5. A function's answer that is kept is copied twice. Write
   `zeno_text_keep_answer(mark, text)` that copies straight to the pool
   and puts the room's top back at `mark`. Where would the compiler
   call it? What must be true of the statement for that to be safe?
6. Text that is the same could be shared with a count of how many hold
   it, in the 8 bytes in front of the block. What would `keep`, `drop`
   and `lend` become? Which of them would no longer copy? What is the
   new cost, and what could now go wrong that cannot today?
7. Why is a parameter never dropped? Write a function that assigns to
   one of its own parameters and see what the checker says. What would
   the compiler have to do if that were allowed?
8. `zeno_in_pool` uses one comparison for both ends of the array.
   Convince yourself it is right for an address below the array, inside
   it, and above it. Then look up what the C standard says about
   subtracting pointers into different arrays, and say why the function
   casts to `uintptr_t` first.
9. `for name in names` copies every item of a list of text before the
   first trip. For a list of 100 lines that is 100 blocks. When is the
   copy needed? Could the checker tell the compiler that the body never
   changes the list?
