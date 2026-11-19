# A list of numbers that may be `none`, with no collector

A number that may be `none` travels with a flag: `int or none` is a
number and a bit that says whether the number is there. A slot of a
list holds 64 bits, and the number fills it, so a list keeps such an
item in a small box of its own and holds the box's address, or `NULL`
for `none`. With no collector nothing freed those boxes, and until
0.8.0 `zeno build --no-collector` refused the list:

```
error: a list of numbers that may be `none` keeps each item in a small block of its own, and `zeno build --no-collector` leaves the collector out, so nothing would free them. ...
```

Now the list owns its boxes, as it owns its copies of text:

```zeno
function average(readings: list[float or none]) -> float or none {
    var sum = 0.0
    var count = 0
    for r in readings {
        if r is not none {
            sum += r
            count += 1
        }
    }
    if count == 0 {
        return none
    }
    return sum / count
}

var readings: list[float or none] = [21.5, none, 22.0]
readings.append(none)
readings.append(23.5)
readings[1] = 21.0
print(readings, average(readings))
let last = readings.pop()
print(last, len(readings))
```

```sh
cargo run -- build --no-collector --memory gaps.zeno
./gaps
```

```
[21.5, 21.0, 22.0, none, 23.5] 22.0
23.5 4
memory: 7 blocks made (128 bytes), at most 112 bytes in use at once, no collector, 0 blocks left at the end
pool: 248 bytes taken, in a pool of 32768
text: at most 61 bytes in use at once, in a room of 1024
```

`zeno run` prints the same two lines, and so does every other build
and every target. The reasons for each choice are in 56.14 of
[`decisions.md`](decisions.md). It is part 9.6 of
[`plan-9-14.md`](plan-9-14.md).

## Only with no collector

Nothing changes in any other build. With a collector the boxes are the
collector's, as they always were, and the C of such a program is what
0.7.0 wrote. Only a build that refused the list before builds it in a
new way.

## The rule

> With no collector, a list owns the box of each number that may be
> `none`, as it owns its copy of each text.

Each thing a list does follows from that:

| Written | What happens to the box |
|---|---|
| `[1, none]`, `append`, `insert` | a new box in the pool for each number; `none` needs none |
| `xs[i]` | the number is read out; the box stays |
| `xs[i] = v` | a new box for `v`, and the old box is freed |
| `pop()`, `take xs[i]` | the number is read out, and the box is freed |
| `clear()`, and the end of the list | every box is freed |
| `a + b`, `a * 3`, `slice`, `filter`, `map` | the new list gets boxes of its own |
| `for x in xs` | the loop walks a copy with boxes of its own, so the body may change `xs` |
| `find`, `in` | the number looked for sits in a box in the text room, given back when the statement is over |

The same is true of `float or none`, `bool or none` and the sized
numbers, such as `u8 or none`.

## What it costs

Each number that is there takes one block of the pool: 8 bytes for the
number, and the pool's own few bytes beside it. `none` takes nothing.
The list's own row of slots is a block of the pool too, as it is for
any list. In the pool of 32,768 bytes a build has by default, a
`list[int or none]` filled from empty holds 679 numbers; the 680th stops the
program with the message every full pool gives:

```
error: the memory pool is full (ZENO_POOL is 32768 bytes)
```

On a small board, where every block counts, two lists cost less:

- **A number that stands for "nothing"**, in a plain `list[int]`: `-1`
  for a reading that never came, when no reading is ever `-1`. The
  number sits in the slot itself, with no block.
- **An enum with a case for the missing value**, `Missing` and
  `Value(n: int)`. Its values are blocks too, but `Missing` takes none,
  and the type says what the missing value means.

## In the C

`runtime.c` is not changed. A program that has such a list gets the
functions it uses, written for it:

```c
static void *zeno_box_keep_float(zeno_opt_float value) {
    if (!value.present) return NULL;
    double *box = (double *)zeno_alloc_raw(sizeof(double));
    *box = value.value;
    return box;
}

static zeno_opt_float zeno_box_take_float(void *box) {
    zeno_opt_float value = zeno_unbox_float(box);
    zeno_free(box);
    return value;
}

static void zeno_list_drop_boxes(zeno_list *list) {
    if (list == NULL) return;
    for (int64_t i = 0; i < list->len; i++) zeno_free(list->items[i].p);
    zeno_list_drop(list);
}
```

and uses them where the list keeps, gives up or ends:

```c
    zeno_list_append(z_readings, (zeno_slot){.p = zeno_box_keep_float((zeno_opt_float){true, 23.5})});
    zeno_slot t2 = (zeno_slot){.p = zeno_box_keep_float((zeno_opt_float){true, t1})};
    zeno_slot *t3 = zeno_list_at(z_readings, 1, 19, 1);
    zeno_free(t3->p);
    *t3 = t2;
    ...
    z_last = zeno_box_take_float(zeno_list_pop(z_readings, 21, 12).p);
    ...
    zeno_list_drop_boxes(z_readings);
```

`zeno_list_keep_boxes_float` gives a list just made (by `+`, `*`, a
slice or a loop's copy) a box of its own for each item. The LLVM build
writes the same functions in IR, as `zeno.box.keep.float` and the rest.

## How it is tested

- **Both compilers**: `a_list_of_numbers_that_may_be_none_every_way`
  builds
  [`tools/targets/edge-none-list.zeno`](../tools/targets/edge-none-list.zeno)
  with no collector through C and LLVM, which give the same memory
  report with 0 blocks left. It is also built in the ordinary and
  firmware builds of both. The program has:
  - lists of ints, floats, u8s and bools that may be `none`, with
    `none` first and last;
  - `append`, `insert`, a store, `pop`, `pop(0)`, `take`, `find`,
    `reverse`, `clear`, `+`, `*` and `slice`;
  - `map` that gives a number that may be `none`, `filter` and
    `reduce`;
  - such a list in a field and in a list;
  - a loop whose body adds to the list it walks;
  - six hundred numbers stored and replaced.

  The same test reads the C of the helpers, checks that a build with a
  collector writes what it wrote before, and fills the pool with five
  thousand numbers, which stops with the message of a full pool
  through C and LLVM.
- **The targets** run the same program.

## Things to try

1. Build `gaps.zeno` with `--no-collector` and open `gaps.c`. Find
   `zeno_box_keep_float`. Why does `none` not call `zeno_alloc_raw`?
2. Build it again without `--no-collector` and search for
   `zeno_box_keep_float`. Why is it not there?
3. Add `readings.clear()` before the last `print`, and build with
   `--memory`. How many blocks are made, and how many are left?
4. Fill a `list[int or none]` with a thousand numbers in a loop and
   build with `--no-collector`. What does the pool say? Then make the
   same list with `none` in every other place. Why does it fit?
5. Write an enum `Reading` with `Missing` and `Value(n: int)`, make the
   same list of it, and compare the two memory reports.
6. Loop over `readings` and append to it inside the loop. Why does the
   loop not see the new items, and what does the copy it walks cost?
