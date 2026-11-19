# The plan for items 9 to 14

[`missing.md`](missing.md) lists what Zeno lacks, in the order to
build it. Items 1 to 8 are built, and went out in 0.7.0. This page is
the plan for the other six. It builds nothing. For each item it says
what is refused today, what the design is, which ways of writing it
were weighed and which one is chosen, which files change, what tests
it needs, and how it is shown to have cost nothing.

It was written on 2026-11-19, against `main` at `b4417df` (0.7.0).
Every "today" below was tried on that compiler; the list of what was
run is at the end.

A choice on this page is a recommendation with its reason. When a
part is built, the choice and the reason go into
[`decisions.md`](decisions.md), as every choice has since chapter 0.
If the code shows that a choice here is wrong, the choice changes,
and the reason for the change is written down with it, there and in
the last part of this page.

| | Item | Parts | Chapter of the decisions |
|---|---|---|---|
| 9 | What a build still refuses, and `raises` in a function type | 9.1 to 9.9 | 56 |
| 10 | Several values from one function | 10.1 to 10.4 | 57 |
| 11 | Generic enums, bounds, generic interfaces | 11.1 to 11.5 | 58 |
| 12 | Cleaning up when a block ends, and files in pieces | 12.1 to 12.3 | 59 |
| 13 | Tools | 13.1 to 13.6 | 60 |
| 14 | More than one thing at a time | 14.1, 14.2 | 61 |

The order is the order of `missing.md`, and it is kept because later
items stand on earlier ones: generic enums that own what they carry
need 9.4, the queue of item 14 needs the generics of item 11, and an
open file needs the cleaning up of 12.1.

## Rules for every part

### What may change, and what may not

1. **Nothing that is accepted today may come to mean something
   else.** Everything below is refused today with a message, or is a
   name that does not exist. This is the test of 11.6, and each part
   says how it passes.
2. **No word is taken.** Not one of the six items needs a new reserved
   word. `take` and `is` are reserved already, and the new names
   (`Resource`, `Ordered`, the module `tasks`) come with Zeno as
   `ValueError` and `math` do: a program that has its own declaration
   or its own file of that name keeps it, and means what it meant.
   Each part that brings a name has a test for exactly that.
3. **A program that does not use a feature pays nothing for it**: not
   an instruction, and not a byte of flash (decisions 34.5, 42.11).
4. **Every engine and every build says the same.** `zeno run`, `zeno
   run --vm`, `zeno build` through C and through LLVM, each of them
   ordinary, `--firmware` and `--no-collector`, and the fifteen
   targets print what `zeno run` prints (5.5d). Where a build cannot
   do a thing, it refuses with a message that says what to write
   instead. It never does something else in silence.
5. **The five questions of the audit** ([`audit.md`](audit.md)) are
   asked of every new rule: one obvious way to write it, no line that
   reads two ways, nothing happens that the line does not say, every
   word plain and with one meaning, and a refusal that says what to
   write.

### What "done" means for a part

A part is done when all of this is true, and not before.

- **Tests in the place the feature lives.** The parser's tests for
  the syntax, the checker's for each rule and each message, and for
  anything a build compiles a program that is built every way
  (ordinary, `--firmware`, `--no-collector`, each through C and
  through LLVM) and must print what `zeno run` prints. With no
  collector the program must end with 0 blocks left, and the C build
  and the LLVM build must write the same memory report (5.11t).
- **Every refusal has a test of its words.** A message is part of the
  language (4.2).
- **The awkward paths are tested, not only the plain one.** For
  anything that owns memory: every way out of a block (its end,
  `return`, `break`, `continue`, an error passing through, an error
  caught inside), a loop that runs it a thousand times in a pool that
  is not allowed to fill, and the same program run again with spare
  blocks overwritten (9.7 makes that a build option).
- **The targets.** A program in `tools/targets/` named
  `edge-<feature>.zeno` for anything the targets must do, and the
  `targets` workflow green on the commit. It takes about ninety
  minutes, so it is read after the push and not waited for at each
  commit; a part is not called done while it is red.
- **The other workflows that the change starts** are green: `board`,
  `minimal` and `plain` after any change to `emit_c.rs`, `minimal.rs`
  or `runtime.c`.
- **`cargo test` passes in a release build at every commit, and in a
  debug build at the end of each item.** A debug build stops on an
  overflow in the compiler's own sums that a release build lets
  through.
- **Windows and macOS** are tried with the `trial` workflow at the end
  of each item.
- **The speed checks below are run, and their numbers are in the
  commit message or the chapter.**
- **It is written down.** The chapter of the decisions, a page in
  `docs/` that walks through how it is built and ends in "Things to
  try", the rows of [`missing.md`](missing.md) and of the README
  struck or changed, the limits pages
  ([`no-collector.md`](no-collector.md), [`generics.md`](generics.md),
  [`interfaces.md`](interfaces.md), [`testing.md`](testing.md),
  [`lsp.md`](lsp.md)) brought up to date, a paragraph in
  `docs/releases/0.8.0.md`, and a line for each commit in
  `tools/history-notes.txt`.
- **What is left is listed**, under "Left for later" in the chapter,
  each with the message it is refused with.

### Showing that nothing got slower

Timings on a shared machine move by more than the changes they are
meant to show, so the proof comes in three steps, cheapest first.

1. **The same C means the same speed.** Before a part is pushed, every
   example, every program in `tools/targets/` and every benchmark is
   built by the `zeno` of 0.7.0 and by the new one, each way a build
   can be asked for (ordinary, `--firmware`, `--no-collector`,
   `--release`, `--fast`, `--minimal`, and `--llvm`), and the C and
   the IR are compared byte for byte. So are `zeno join` and `zeno
   bytecode` for the same programs. A program that does not use the
   new feature must come out the same. This is how 33.4 was shown,
   and it settles the speed and the size of every built program in a
   minute, with no clock. It is worth a script, `tools/same.py`, since
   every part runs it. Where a file differs, the difference is
   explained in the chapter or mended.
2. **The two engines are counted.** `zeno run` and `zeno run --vm` are
   the `zeno` program itself, and any change to the compiler can move
   them, also a change that no program reaches (24.29, 42.11). At the
   end of each item, and after any commit that touches `value.rs`,
   `ops.rs`, `vm.rs`, `bytecode.rs`, `emit_bytecode.rs` or
   `interpreter.rs`:

   ```sh
   python3 bench/run.py --count --engines vm,run --zeno path/to/0.7.0/zeno
   python3 bench/run.py --count --engines vm,run
   ```

   Each of the six programs must stay within 0.1% of 0.7.0 in both
   engines. More than that is found with callgrind and mended before
   anything else is built on it (42.11 shows how).
3. **And timed**, because a count does not see the same instructions
   laid out worse: `python3 bench/compare.py --old path/to/0.7.0/zeno`,
   with the two binaries under names of one length in one folder. No
   program may be more than 2% slower in the best of four rounds. The
   one loss already known is `zeno run` on `loop` (50.7).

After a change to `emit_c.rs`, `minimal.rs` or `runtime.c`, the sizes
are measured again (`tools/board.py --sizes`, `tools/minimal.py
--sizes`). They must not move for a program that does not use the
feature.

### How the work is laid out

- One part at a time, in the order of this page. Small commits, each
  of which passes `cargo test`, pushed to `main`.
- The first commit of all sets the version to 0.8.0 (`Cargo.toml`,
  `Cargo.lock`, `tools/install.sh`) and begins
  `docs/releases/0.8.0.md`.
- Each item ends in a state that could be released. A release itself
  is asked for separately, as always.
- Before a part is begun, its design is read against the code as it
  is, and what the code says differently from this page is settled
  first and written into "Changes to this plan" below.

---

## 9. What a build still refuses, and `raises` in a function type

The aim, in one sentence: **a build should take what `zeno run`
takes.** What is left after this item is refused by design, and says
so.

### What is refused today

Tried on 0.7.0. The first column is the part of this item that takes
it up.

| | Program | Refused by | The message begins |
|---|---|---|---|
| 9.1 | `["1", "2"].map(parse)`, where `parse` says `raises` | `--firmware`, `--no-collector` | "cannot compile a function that can raise, used as a value, yet" |
| 9.2 | `return function(n) { return f(g(n)) }` | `--no-collector` | "this function uses a value from outside it that has one owner" |
| 9.2 | `let m = counter.bump` | every build | "cannot compile a method used as a value" |
| 9.3 | `let t = slot.task`, with `task: Task or none` | `--no-collector` | "holds an owned value, which cannot be taken out of the object it is in" |
| 9.4 | `same([1, 2, 3])` with `function same[T](x: T)`; `Box([1, 2])` | `--no-collector` | "a value that has one owner cannot fill a type parameter yet" |
| 9.5 | `let d: dict[str, list[int]]` | `--no-collector` | "with no collector a value of a dict is an int, a float, a bool or a str for now" |
| 9.6 | `let xs: list[int or none]` | `--no-collector` | "a list of numbers that may be `none` keeps each item in a small block of its own" |
| 9.7 | `zeno build --pool 4096` | the command line | "cannot read `--pool`" |
| 9.8 | a function with a name inside a function | every build | "cannot compile a function inside a function or block yet" |
| 9.8 | `function add(x: int, y: int = base * 2)` | every build | "cannot compile a default value that is not a plain literal yet" |

Two rows of `missing.md` are no longer true and only need striking:
`raise e` to pass a caught error on builds with no collector since
5.17, and a method that takes out what its own object holds builds
since 5.19k.

Three stay refused, by design, and 9.9 says so in one place: a
function that changes a `var` of the function around it (nothing is
shared in such a build), `xs.append(xs.pop())` on one line (C does
not say which half runs first, 5.9q), and a function that reads a
`var` list at the top of a file (it could be replaced while the
function looks; a `let` works, audit part 5).

### 9.1 `raises` in a function type

```zeno
function parse(text: str) -> int raises ValueError { ... }

function each(texts: list[str], convert: function(str) -> int raises ValueError) -> int raises ValueError {
    var total = 0
    for text in texts { total += convert(text) }
    return total
}
```

**The rule.** A function type is written as the first line of a
function is, without the names: after what it returns it may say
`raises` and a kind of error. A call through a value of that type can
raise that kind, and is checked as a call of a function that says so:
its caller catches it or says `raises` itself. A function fits the
type if it raises that kind or nothing. `raises Error` takes a
function that raises anything.

**How it is written: three ways weighed.**

| | Written | Verdict |
|---|---|---|
| A | `function(str) -> int raises ValueError` | **Chosen.** It is the function's own first line. Nothing new to learn, and the printer and the messages already write a function that way |
| B | `raising function(str) -> int` | One more word, in a place no other word goes, and it cannot say which kind |
| C | `function(str) -> int or raises ValueError` | Reads as if the error were a value that comes back. It is not, anywhere but in the C of a firmware build |

**One kind, not a list.** A function's first line may say `raises
ValueError, IoError`. A function type stands inside lists of its own:

```zeno
function each(convert: function(str) -> int raises ValueError, IoError, text)
```

Is `text` a third kind of error, or a second parameter with no type?
The parser could be given a rule. A reader would have to know it,
which is question 2 of the audit. So in a type `raises` names one
kind, and a function that raises two kinds fits `raises Error`. A
second name after a comma gets a message that says so. A list could
be let in later, inside brackets of some kind, without changing what
any type written today means.

**What it changes for a program written before: nothing.** A type
with no `raises` is checked as it is today. In a firmware build and a
build with no collector, a function that can raise fits only a type
that says so; those builds refuse every such program today, so they
only come to accept more. In an ordinary build and in `zeno run` a
function that raises is still accepted where the type does not say
so, as it is today, and the call is then checked as raising nothing,
as it is today. That is a gap in "an error a function must declare",
and it is older than this item. Closing it means refusing programs
that build now, so it is not done here. It is measured instead: the
check is run over every example, every target program and every
sample of the book in an ordinary build, the number it would refuse
is written into the chapter, and whether to close the gap is put to
the owner of the language with that number.

**How it is built.**

- `ast.rs`: `Type::Function` gains `raises: Option<String>`.
  `parser.rs` (`type_`) reads it; `printer.rs`, `modules.rs` (the name
  is renamed as any name is) and `format.rs`'s tests follow.
- `checker.rs`: `CallableType` gains `raises`. `call` on a
  `Ty::Callable` calls `can_raise`, as it does for a function. `fits`
  and `same` compare it. `Display` writes it. A `var f = parse` gets
  the whole type (5.19l). `map`, `filter`, `reduce` and `sort` already
  pass on what the function given to them raises, and stay as they
  are.
- `emit_c.rs` and `emit_llvm.rs`, firmware and no collector: a value
  of a type that says `raises` is called the way a function that
  raises is called (the answer through a pointer, `true` or `false`
  back, 5.5l). A function that raises nothing, put where such a type
  is wanted, goes through a small function written for it, the first
  time it is needed (5.11k), which calls it and answers `true`. That
  covers a function of the program and a function written where a
  value goes. A value that is already of the plain type and is then
  put where the raising type is wanted needs a block of its own to
  carry it: it is made by the rule of 9.2, or refused with a message
  if 9.2 shows that to be more than it is worth.
- An ordinary build needs nothing in its C: both kinds of function are
  called alike there (5.5i).
- The interpreter, the VM and the targets do not look at types, and
  need nothing.

**Tests.** The parser: the type, the type inside a parameter list
followed by another parameter, the message for a second kind. The
checker: what fits what, in both directions; a call through the value
must be caught or declared; `raises Error`; a function of an
interface compared with a function type; the message for `raises
ZeroDivisionError` on a type in a firmware build (5.19m holds for a
type as for a function). Both compilers: `errors_as_values_match_in_methods_loops_and_function_values` grows a program that hands a
raising function to `map`, to a parameter, into a field and into a
list, and catches what comes out of each, built every way. `what_a_firmware_build_refuses_says_so` loses its first line.

**Speed.** No ordinary build changes: `tools/same.py` must show
every C file as it was. The engines are not touched.

### 9.2 A function that is kept takes what it uses

```zeno
function compose(f: take function(int) -> int, g: take function(int) -> int) -> function(int) -> int {
    return function(n: int) -> int { return f(g(n)) }
}
```

**The rule today** (5.17d): a function written where a value goes,
which uses a value that has one owner, only looks at it. So it lives
for the statement that writes it, and cannot be kept.

**The rule added.** *A function that is kept takes what it uses.* If
the function is returned, put in a variable or a field, or given to a
parameter that says `take`, each value with one owner that it uses is
given to it, as `let b = a` would give it. The function owns them
from then on and frees them when it is freed. A function that is only
passed along, to `map` or to a parameter that looks, looks as before.

What it takes must be the surrounding function's to give: its own
`let`, or a parameter that says `take`. A use of the value after that
line is the error every use after giving away is, and names the line.
A value that is only lent to the surrounding function cannot be
taken, and the message says to write `take` on the parameter.

**Why no word at the function**, as Rust's `move` is. The line already
says the function is kept: it is after `return`, or `=`. A function
that is kept and still only looked would be gone before it could be
called, so there is no second meaning for a word to tell apart. The
first line of a function says `take` for a parameter because a call
does not show what the function does; here the line shows it.

**A method as a value** is the same thing. `let m = counter.bump` is
the function `function(...) { return counter.bump(...) }`. Passed
along, it looks at `counter`. Kept, it takes `counter`, with one
owner; where values are shared it shares it, as every engine but the
compilers does now. So one rule answers both rows of the table.

**How it is built.** `checker.rs`: where a function literal with
`Capture`s is checked, the place it goes decides between
`Borrowed(Function)` and `Owned(Function)`; in the second case each
capture is given away (`owned` state, as for `let b = a`). The
decision is noted for the compilers in `Types`. `emit_c.rs` and
`emit_llvm.rs`: the block of such a function already has a `drop`
entry (5.17f); the function written for it frees what was taken. A
bound method is a block with the object in it and one small function
that calls the method. The interpreter, the VM and the targets
already do the right thing where values are shared, and need nothing.

**Tests.** `compose`, built every way and called a thousand times
with 0 blocks left. A counter whose method is kept, and one whose
method is passed to `map`. The value used after it was taken, the
value that was only lent, a `var` of the surrounding function (still
refused, by design), and a kept function inside a loop.

### 9.3 `take` in front of a place: moving out what may be `none`

```zeno
class Slot {
    task: Task or none
}

let task = take slot.task       # slot.task is none now
if task is not none { run(task) }
```

**The need.** With one owner, an object cannot give away what a field
holds, because nothing would be left in the field (5.8i). A list has
`pop()`. A field has nothing, so a state machine cannot hand on the
buffer it holds.

**The rule.** `take` in front of a field or an item gives what is
there and leaves `none`. The place must be one that may hold `none`,
so that what is left is something the type already allows. What comes
back may be `none` too, and is tested as any such value is.

**Ways weighed.**

| | Written | Verdict |
|---|---|---|
| A | `let task = take slot.task` | **Chosen.** `take` is the word for exactly this since 5.19b: the value goes over, and the place it was in holds nothing. It is reserved, so the line reads one way |
| B | `let task = slot.task` empties the field | Something happens that the line does not say: question 3 |
| C | `slot.task.take()` | A method on a value that has not been tested for `none`, which nothing else allows |
| D | `let task, slot.task = slot.task, none` | Needs item 10, and says in a riddle what A says in a word |

A variable is not a place for this word: `let b = a` already gives
`a` away. `take a` gets a message that says so. A field that cannot
be `none` gets one too, and it says to make the field `Task or none`.

It means the same in every build. Where values are shared it still
leaves `none`, so one program runs the same way under `zeno run` and
with no collector.

The three guards of 5.15e hold for it as for `pop()`: the place is
reached from an owner, no loop is walking that owner, and the line
has not already looked at it.

**How it is built.** One new expression, `ExprKind::Take(place)`.
`parser.rs` reads `take` at the start of an expression. `checker.rs`
gives it the type of the place and checks the guards (`takes_out`).
The interpreter and the VM read the place and store `none`. Both
compilers do the same, and with one owner pass the value on instead
of copying it. `lower.rs` makes it two statements for the targets,
so no target's runtime changes. `printer.rs`, `modules.rs`.

**Tests.** The parser and the messages; a scheduler that takes its
current task and gives it to another, built every way, 0 blocks left;
`take xs[i]` on a `list[Task or none]`; the three guards.

### 9.4 A value with one owner as what a type parameter stands for

```zeno
class Stack[T] {
    items: list[T]
}
let frames: Stack[Frame] = Stack([])       # refused today with no collector
```

**The rule.** A copy of a generic is checked as if it had been
written by hand with the type in the place of `T` (3.16d). That is
all the rule there is, and with no collector it has simply not been
followed: the checker stops at the call. The stop is taken away.
`Stack[Frame]` is then a class with a list of frames, which a build
with no collector has taken since 5.15c.

**`take T`** is allowed (it was refused by 5.19f, "because `T` may be
a number"). In a copy where `T` is copied, the word changes nothing,
which is what it already does where values are shared (5.19a). A
generic that keeps or returns what it is given says `x: take T`, and
is then right for an `int` and for a list. One that does not say it,
and is used with a list, is an error inside the copy, with the note
that names the copy and the call, as every error inside a copy has.

**How it is built.** `checker.rs`: the refusal goes, `infer` and the
making of a copy let `Owned` and one-owner classes through, and `take
T` is read. `emit_c.rs` and `emit_llvm.rs`: the free functions of a
copy are written by `Drops` as for any class; a name such as
`z_Stack_Frame` is made as names of copies are (5.14c).

**Tests.** `same`, `Box`, `Stack` and `Pair` with a list, an object
and an enum value, built every way, 0 blocks left. The error inside a
copy that returns what it only looked at, with its note.

### 9.5 A dict that owns its values

10.14 wrote the plan: with no collector a dict is the owner of the
lists and objects in it, as a list is of its items (5.15c).

| Written | With one owner |
|---|---|
| `d[key]` | lends the value: a look |
| `d[key] = value` | takes the new value, and frees the one that was there |
| `d.pop(key)` | hands the value to whoever receives it |
| `d.clear()`, and the end of the dict | free every value |
| `for key, value in d` | lends each value for the body |
| `d.get(key)` | lends, and may be `none` |
| `d.get(key, instead)` | refused: say `get(key)` and test for `none`. Who would own `instead` has no good answer |
| `d.values()` | refused: a second list of the same objects would be a second owner. The message says to loop with `for key, value in d` |

`pop`, `d[key] = value` and `clear` are held to the three guards of
5.15e. A dict is two lists and an index (6.40), so its values are
freed by the list free functions that exist.

**How it is built.** `checker.rs`: the rules above, beside those of
a list (`owns_items`, `takes_out`). `emit_c.rs`, `emit_llvm.rs`,
`runtime.c`: the dict's free function frees its values through
`Drops`; storing over a key frees the old value. Nothing changes for
a dict of plain values, and `tools/same.py` shows it.

**Tests.** A dict of lists and a dict of objects, each way in the
table, built every way, 0 blocks left, a thousand stores over one
key in a pool that does not fill. Each refusal and each guard.

### 9.6 `list[int or none]` with no collector

**Today** a number that may be `none`, inside a list, is a pointer to
a small block of its own, because a number and a flag do not fit in
one slot of a list (5.11i). With no collector those blocks have no
owner, and the list is refused.

**Two ways.**

| | How | For | Against |
|---|---|---|---|
| A | The list keeps a second, packed list of flags beside its values: one byte an item, as a `list[u8]` is packed (36.2) | No block for an item at all: 9 bytes an item. Nothing to free, nothing to copy deep | Each thing a list can do must keep the two in step |
| B | The blocks stay, and the list owns them, as it owns the text in it (5.9x) | The free and the deep copy exist for text | 24 bytes an item and a trip to the pool for each number, which is what firmware cannot afford |

**A is chosen**, as `missing.md` already says: "store the number and
a flag side by side". It is used in every C build and LLVM build and
not only with no collector, so that there is one way such a list is
built and a list like this gets cheaper everywhere. The interpreter,
the VM and the targets hold values that know what they are, and do
not change.

If the code shows that A touches more than the runtime's list
functions and the places both compilers read and write an item, B is
built for a build with no collector alone, the reason is recorded,
and A stays on the list.

**Tests.** Every list method on a `list[int or none]`, a `list[float
or none]` and a `list[u8 or none]`, built every way against `zeno
run`; `none` at the first and the last place; a list of them inside a
list; 0 blocks left. The C of a program with no such list must not
change.

### 9.7 The size of the pool on the command line, and spare blocks that are checked

- `zeno build --no-collector --pool 4096` and `--text-room 2048` hand
  `-DZENO_POOL=4096` and `-DZENO_TEXT_ROOM=2048` to the C compiler
  (5.9c, 5.9j). Both, because a person who finds one will look for
  the other, and `--memory` prints both numbers. They go with
  `--board`. Without `--no-collector` (or `--board`) they are refused
  with a message, and so is a number that is not a whole number above
  zero.
- With `--memory`, a block that is given back is filled with a
  pattern, and is checked to be untouched when it is handed out
  again. A write to a block after it was given back then stops the
  program with a message. It is the check 5.9k left as an exercise,
  and it stands in for the heap checkers that cannot see inside the
  pool. It is behind one `#ifdef`, so a build without `--memory` has
  not a byte of it.

**Tests.** A program that needs N bytes runs with `--pool N` and
stops with the pool's message at N - 1 (the test of 5.9j, driven from
the command line). A runtime test that writes to a block it gave back
and must be stopped. The options refused where they mean nothing.

### 9.8 A build takes what `zeno run` takes

| Today a build refuses | Built as |
|---|---|
| A function with a name inside a function | It is a function written where a value goes, kept in a `let` of its name at the top of its block (3.6d says so already, and `lower.rs` does it for the targets). It may call itself |
| A default that is not a plain literal | The default is worked out at each call that leaves the argument out (3.5), in the order of the parameters, by code written at the call |
| A method as a value | Done in 9.2 |

Before this part is built, every other refusal of the two compilers
is listed by searching `emit_c.rs` and `emit_llvm.rs` for their
refusing function, and each is either built here, or written into
[`compiler.md`](compiler.md) as left, with its reason. A named
argument to a function value, a built-in used as a value
(`xs.map(str)`) and `print([])` are known to be among them.

**Tests.** `the_examples_compile_and_match` already holds every
example to `zeno run`; the examples that were kept from it for these
refusals are let in. A function inside a function that calls itself,
that uses a `let` around it, and that is returned.

### 9.9 The sweep

Last, every refusal named in [`no-collector.md`](no-collector.md),
[`firmware.md`](firmware.md), [`functions-and-errors.md`](functions-and-errors.md),
[`one-owner.md`](one-owner.md), chapter 11 and part 7 of `missing.md`
is run again on the new compiler. Each page then lists what is
refused **by design**, with the reason, and nothing else. A test,
`what_a_build_with_no_collector_refuses_says_so`, holds the list: a
refusal that is not on it fails the test.

**Left for later after item 9** (each refused with a message): a
list of kinds after `raises` in a function type; `d.values()` and
`d.get(key, instead)` on a dict that owns its values; and whatever
9.8 writes into [`compiler.md`](compiler.md) as left.

---

## 10. Several values from one function

```zeno
function divide(a: int, b: int) -> (int, int) {
    return a // b, a % b
}

let quotient, remainder = divide(7, 2)

var low = 3
var high = 1
low, high = high, low
```

### 10.1 The rules

1. **A function may give two or more values.** Its first line says
   `-> (int, str)`, and `return` gives that many, with commas.
2. **They are received by that many names**: `let q, r = divide(7,
   2)`, `var q, r = ...`, or `q, r = ...` for places that exist.
   Every value is received. `_` in the place of a name receives one
   and drops it.
3. **Several values are not a value.** They cannot be put in one
   variable, in a list, in a field, passed as one argument or
   printed. Each of those is refused with one message: "`divide`
   gives 2 values. Receive each: `let a, b = divide(...)`. To keep
   them together, declare a class". A tuple that is kept is a class,
   as `missing.md` says, because a class gives each part a name.
4. **The right of `=` is one call that gives that many values, or
   that many values written out.** All of the right is worked out,
   left to right, before anything on the left is given a value; then
   the left is filled left to right. That is what makes `low, high =
   high, low` a swap.
5. `(int, str)` is written only after `->`, in a function's first
   line or in a function type. Nowhere else is it a type.

### 10.2 How it is written: the ways weighed

| | Written | Verdict |
|---|---|---|
| A | `-> (int, str)`, `return a, b`, `let a, b = f()` | **Chosen.** It is Go's shape: brackets in the first line, bare commas where values are given and received, and no tuple as a value. `for key, value in ages` (10.12) already receives two names with a bare comma |
| B | `-> int, str` | In a function type inside a list the comma reads two ways: `function(int) -> int, str` among parameters. The brackets are the fence |
| C | `return (a, b)`, `let (a, b) = f()` | `(a, b)` on the value side looks like a value that could be kept, and rule 3 says it cannot. And it is a second way to write the two names of a `for` |
| D | `-> int and str` | A plain word, but `return` and `let` would still need commas, so the type would say one thing and the line another; and `int or none and str` reads against the order of `or` and `and` everywhere else |
| E | Named results, `-> (quotient: int, remainder: int)` | That is a class. A class is how Zeno names the parts of a thing |

**Why not a tuple as a value**, as Python and Rust have. It would be
a second way to write a class with fields nobody named, and
`reading[1]` says less than `reading.status` (principle 2). What is
asked for, by a driver's `read` and by `divide`, is to give back two
things at once, and that needs no new kind of value.

**Why every value must be received.** A function that gives a value
and a status is written so that the status is looked at. `let value,
_ = port.read()` says in the line that it was dropped on purpose.

`let a, b = 1, 2` is accepted, by rule 4. Refusing it would be an
exception to the rule that `low, high = high, low` needs.

### 10.3 How it is built

- **The tree** (`ast.rs`): `Type::Several(Vec<Type>)`, only after
  `->`. `ExprKind::Several(Vec<Expr>)`, only as what `return` gives
  and as the right of the new statement. `StmtKind::Receive { declare:
  Option<bool>, places: Vec<...>, value: Expr }`, where `declare` is
  `let` or `var` or neither, and a place is a new name with an
  optional type, `_`, or a place that can be given a value. Three new
  kinds, and no change to an old one, so every stage that is not told
  of them fails to build until it is.
- **The parser**: `(` after `->` begins the list of types. A comma
  after the first name of a `let`, or after the first expression of a
  statement, begins a `Receive`. One result in brackets, `-> (int)`,
  is refused: "one value is written without brackets".
- **The checker**: a `Signature`'s `returns` may be several. A call
  that gives several is accepted only as the right of a `Receive`
  with as many places, or after `return` in a function that gives the
  same. Each place gets its type, and with one owner each value is
  given to its place. The record of uses (`Use`) notes each name.
- **The interpreter and the VM** run programs with no types written,
  so the values must travel as something that is not a list: `let a,
  b = [1, 2]` has to stay an error. One new variant of `Value`,
  `Several(Rc<[Value]>)`, made only by `return a, b` and taken apart
  only by a `Receive`. It is the last variant, and it is boxed, so no
  value grows. **This is the one place in the item that can cost a
  program that does not use it** (42.11 is the lesson), so the count
  of step 2 is run on the commit that adds it, before anything else
  is built on it. If the count moves and cannot be mended, the values
  travel beside the call instead, in a field of the interpreter and
  of the VM that `return a, b` fills and `Receive` empties, and no
  `Value` changes.
- **Both compilers**: the first value comes back as a function's
  answer does today. Each further one goes out through a pointer the
  caller passes, after the parameters. **Why pointers and not a
  struct.** It is what a firmware build already does with the answer
  of a function that can raise (5.5l), for the reasons given there: a
  struct for each shape of result is one more type for each, and C
  compilers for small chips return a struct through memory in any
  case. One rule, the same in every build. In a firmware build a
  function that raises and gives two values has two such pointers.
- **Text as a second value, with no collector**, needs care: a
  function's text answer travels in the text room (5.9n), and that
  road is built for one. The second text is kept by copy (5.9l) by
  the function and owned by whoever receives it. This is the part
  most likely to be got wrong, and it has a test of its own.
- **The targets**: `lower.rs` makes `return a, b` a call of the
  runtime that packs the values, and a `Receive` a temporary and one
  read for each place, which also says the error of rule 3 when a
  program with no types gets it wrong. Two small functions in each
  runtime, and two forms for each emitter to print.
- `printer.rs`, `modules.rs`, `format.rs`'s tests, and the language
  server's hover, which shows `-> (int, str)`.

### 10.4 Tests, and what is left

**Tests.** The parser: each form, and each refusal. The checker:
count and types on both sides, a call of several used as one value in
each of the places rule 3 names, `_`, a `Receive` into a field and an
item, a generic `function swap[A, B](a: A, b: B) -> (B, A)`, a
function of an interface that gives two, a function type that gives
two. Every engine and build: `divide`; a swap of two numbers, of two
lists and of two objects, with 0 blocks left with no collector; a
function that raises and gives two, caught and not caught; two texts
built inside the function; a function value that gives two.
`tools/targets/edge-several.zeno` for the targets, with the error of
rule 3 in a program that has no types.

**Speed.** `tools/same.py`: no C changes for any program that exists.
The count of step 2 on the commit that adds the `Value`.

**Left for later:** swapping two items that have one owner (`xs[i],
xs[j] = xs[j], xs[i]` reads each item as a look; a `swap(i, j)` on a
list is the way in); several values in a `for`; a default for one of
several results.

---

## 11. Generic enums, bounds, generic interfaces

### 11.1 Generic enums

```zeno
enum Maybe[T] {
    Nothing
    Just(value: T)
}

function first_even(numbers: list[int]) -> Maybe[int] {
    for n in numbers {
        if n % 2 == 0 { return Just(n) }
    }
    return Nothing
}
```

**The rules** are those of a generic class (3.16 to 3.16c), and
nothing new: square brackets after the name, a capital for the
parameter, and the types worked out at each use and never written
there. `Just(3)` is a `Maybe[int]` from its argument. `Nothing`
carries nothing that says what `T` is, so it takes it from the place
it goes: the type of the variable, of the parameter, or of what the
function returns (3.16b, rule 3). Where nothing says, it is an error
that shows how to write the type.

**How it is built.** `ast::Enum` gains `type_params`; the parser's
refusal goes. In `checker.rs` a generic's `Template` gains `Enum`,
and a copy is an `EnumInfo` with its `origin`, made as a copy of a
class is (5.14). A case of a generic enum that is not yet a copy is
worked out as a call of a generic function is. `match` needs no
change: it sees a copy. The interpreter and the VM run the generic
itself (5.14b). Both compilers write one enum for each copy,
`z_Maybe_int`, with its free function. With no collector a
`Maybe[Frame]` owns its frame (5.16), which is why 9.4 comes first.
The targets do not look at types.

**Tests.** `Maybe[T]` and `Result[T, E]` with numbers, text, a list
and an object; `Nothing` in each place that says its type and in one
that does not; `match` over a copy with a case left out; a list of
`Maybe[int]`; built every way, 0 blocks left.

### 11.2 Bounds: `function largest[T is Ordered]`

```zeno
function largest[T is Ordered](items: list[T]) -> T { ... }

function total[S is Shape](shapes: list[S]) -> float { ... }
```

**The rule.** A type parameter may say `is` and an interface. Each
call is then checked first against that: the type that `T` stands
for must be a class that says it is the interface. If it is not, the
error is at the call and names both: "`Task` is not `Ordered`, which
`largest` asks of `T`". Inside the generic, the functions of the
interface may be called on a `T`.

**How it is written.**

| | Written | Verdict |
|---|---|---|
| A | `[T is Ordered]` | **Chosen.** It is the sentence a class already says, `class Circle is Shape`, with the word that is already reserved |
| B | `[T: Ordered]` | A colon in Zeno says "this name has this type" (`port: int`). `T` does not have the type `Ordered` |
| C | `function largest[T](...) where T is Ordered` | A second place to look for what the first already has room for |

**The body is still checked for each copy** (3.16d). A bound is a
promise that is checked at the call. It is not the whole of what the
body may do: a generic with no bound that uses `>` is accepted today,
and stays accepted. Making the bound the whole truth would refuse
generics that build now, and it would need a bound for every operator
before the first one could be used; C++ took this road with concepts,
for the same reason. What a bound buys is what `missing.md` asked
for: the error moves from inside the generic to the call. And a
generic that nothing uses is checked with `T` as its interface, and
no longer as "anything" (5.14d).

**Why a bound and not simply a parameter of the interface's type.**
`shapes: list[Shape]` does not take a `list[Circle]` (10.7). `[S is
Shape](shapes: list[S])` does, and gives back an `S` as a `Circle`.

**`Ordered` comes with Zeno**, and is the first such interface: a
type is `Ordered` when `<` works on two values of it, which is every
kind of number, a quantity, and text. It is what `largest` and a
`sort` of one's own need. A class cannot say it is `Ordered` yet:
that is "operators for a class" in `missing.md`, which is built later
by the same road, and the message says so. A program's own `Ordered`
is the program's, as with every name that comes with Zeno.

**One interface for each parameter.** `[T is Ordered, U]` must read
one way, so a second interface cannot follow a comma. Until a program
needs two, a second is refused with a message.

**How it is built.** `ast.rs`: a type parameter becomes a name and an
optional bound, on a function and on a class. `parser.rs`.
`checker.rs`: `Generic` keeps the bounds; `infer` checks each against
`is_one` before a copy is made or looked up; inside a copy `T` is the
real type, so calls of the interface's functions are checked as on
any object. Nothing else changes: a bound makes no code. The
compilers and the targets need nothing.

**Tests.** The message at the call, for a function and for a class;
`largest` over ints, floats, text and volts; a class given where
`Ordered` is asked; a user's interface as a bound, with a `list[Circle]`;
a generic that nothing uses and that calls a function its bound does
not have; a program with its own `Ordered`.

### 11.3 Generic interfaces

```zeno
interface Source[T] {
    function next(self) -> T or none
}

class Countdown is Source[int] { ... }
class Lines[T] is Source[T] { ... }
```

An interface is a class to every stage after the parser (10.9), and
a generic class has copies. So `Source[int]` is a copy of the
interface, a class says which copy it is, and `fits` compares copies.
The parser's refusal goes, `ast::Class::is` becomes a list of types
where it was a list of names, and `stands_for` compares the functions
of the class with those of the copy. Both compilers write the
`switch` of 10.9 for each copy of the interface that the program
uses, over the classes that are that copy.

**Tests.** The two classes above, used through `Source[int]`, in a
list, as a bound (`[S is Source[int]]`); a class that says `Source[str]`
and has the functions of `Source[int]`; built every way.

### 11.4 What interfaces left out (10.10)

- **A default for a parameter of an interface's function.** The class
  writes each function as the interface does (10.6), so it writes the
  same default, and the checker compares the two as written. Only a
  default that is a plain value is allowed here, so that "the same"
  has one meaning. A call through the interface and a call on the
  object then cannot differ.
- **A function that takes out what its object owns, standing for one
  of an interface**, and **an `owned class` that is an interface**.
  They are one question (10.10): a call through the interface must be
  held to the guards of 5.15e when the function behind it takes
  something out. A function of an interface has no body to read, but
  the whole program is known (7.11): it takes out if the function of
  any class that is the interface does. The checker works that out
  the first time the function is called through the interface. Then
  both refusals go.

**Tests.** The program that 0.7.0 refuses ("`take_last` of `Stack`
takes out or replaces something its object owns"), called through
the interface inside a loop over the same list (refused, with the
loop's message) and outside it (built every way, 0 blocks left); an
`owned class` that is an interface in an ordinary build; a default
that differs from the interface's.

### 11.5 A method with a type parameter of its own

```zeno
class Box[T] {
    item: T
    function convert[U](self, change: function(T) -> U) -> Box[U] { ... }
}
```

For a class and not for an interface: a function of an interface is
reached through a `switch` over classes, and a copy for each `U` in
each class is more than any program has asked for. The method is a
generic whose copies are made for each `U`, inside each copy of the
class. It is the last part of the item and the least needed, and it
is built only if the generic of 11.1 to 11.3 makes it a small step.
If it does not, it stays refused, with that reason written down.

**Left for later after item 11:** types written at a call
(`first[int](xs)` reads as an index followed by a call, so 3.16b
stays: the place the result goes says the type); two interfaces for
one parameter; an interface that is another interface; a generic kept
as a value; a class that is `Ordered`.

---

## 12. Cleaning up when a block ends, and files in pieces

### 12.1 A `Resource`: something that is put back when its owner ends

```zeno
owned class Selected is Resource {
    line: Pin

    function release(self) {
        self.line.on()              # chip select is high again
    }
}

function read_id(bus: Bus, line: Pin) -> u8 {
    line.off()
    let chosen = Selected(line)
    return bus.transfer(0x9F)
}                                   # chosen.release() runs here, however the block is left
```

**The need.** An open file, a chip-select line and a lock all have a
second half that must happen on every way out of a block. Today the
second half is written before each `return`, and forgotten before one
of them.

**The ways weighed.**

| | How | Verdict |
|---|---|---|
| A | A class says it has something to put back, and the end of its owner does it | **Chosen.** It cannot be forgotten at a use, and it is the moment that already exists: an owned value is freed "at a line the programmer can point to" (5.8) |
| B | `defer file.close()`, as in Go | Written at each use, so it can be left out at each use, and nothing can check that. Principle 1 |
| C | `finally` | The same, and a second way to leave a `try`. `missing.md` leaves it out for this reason |
| D | `with files.open(path) as f { }`, as in Python | A new kind of block for one job (6.12 turned it down) |

**How the class says it.**

| | Written | Verdict |
|---|---|---|
| A | `owned class Selected is Resource`, with `function release(self)` | **Chosen.** A class already says on its first line what it is (10.4), so the reader of a class sees there that its end does something. No new word. It is the second interface that comes with Zeno, after `Ordered`, and operators for a class are to come the same way |
| B | A function with a set name, such as `close`, in any class | A name that changes what a function is for is a rule nobody can see (10.16 turned down `test_` for this) |
| C | A block in the class, `cleanup { ... }` | A new kind of member, with a `self` that is not written (3.8) |

The words: a *resource* is the common name for a thing that must be
given back, and *release* is what is done with a lock, a line and a
handle alike. `Closeable` has two spellings, and `drop` is a word
from one language.

**The rules.**

1. `Resource` comes with Zeno: `interface Resource { function
   release(self) }`. A program's own `Resource` is the program's.
2. Only an `owned class` can be one. A shared object has no owner
   whose end could be the moment. The message says to write `owned`.
3. `release` takes `self` alone, gives nothing and raises nothing: an
   error has nowhere to go from the end of a block. It cannot be
   called by hand: it runs once, and the message says how to end a
   resource sooner (give it to a function that takes it).
4. **It runs when the owner ends**: when the block of the `let` that
   holds it is left by reaching its end, by `return`, `break` or
   `continue`, or by an error passing through. Of two in one block,
   the one made last is released first. After `return x + y`, the
   value is worked out first and the release runs second.
5. A resource has one owner, like every owned value. `return` gives
   it to the caller, a parameter that says `take` takes it, `let b =
   a` moves it. Whoever has it at the end releases it; a resource
   that was given away is not released by the one who gave it.
6. **It does not run when the program is stopped**: by
   `program.exit`, by a fault in a firmware build (5.5l), or by a
   failed `assert`. Stopping ends no block.
7. **Where a resource can be, in this version: in a `let`, or a
   parameter.** Not in a `var`, a field, a list, a dict, a case of an
   enum, a `T or none`, a type parameter, or a function that is kept;
   and what a call gives must be received by a `let` and not used in
   passing. Each is refused with a message that says so. The reason
   is the next paragraph.

**Why so narrow.** The compilers free an owned value on every road it
can take, and for them a `release` in the class's free function would
be all there is to do. But the interpreter, the VM and the fifteen
targets free nothing: their values are shared, and collected by the
language they are written in. For them "the owner ends" has to be
made into something that runs, and for a `let` in a block that is
one thing: code at each way out of the block. For a field or an item
of a list it is the whole of chapters 5.15 to 5.17 again, in every
engine. A `let` covers the guard of a block, which is what the three
needs above are. The rest is refused, so letting it in later changes
no program (11.6).

**How it is built.**

- `checker.rs`: the rules, and where the type of a value is not
  known, a resource may not go (so that an engine never has to find
  one out while the program runs). Then `checked` rewrites the
  program, as it already does for sized numbers: the statements after
  `let chosen = ...`, to the end of its block, go inside a new
  statement, `StmtKind::Scoped { name, body }`, and each use that
  gives the resource away becomes `ExprKind::Give(name)`. Both are
  made only by the checker, and no program can write them.
- The meaning of `Scoped`, for every engine: run the body; however it
  is left, if the name still holds its value, call its `release`;
  then go on leaving in the same way. An error that passes through is
  passed on **as it was**: same kind, same message, same line. (`raise
  e` in a program gives the line of the `raise`, so it cannot be used
  for this.)
- The interpreter: one arm, which runs the body, looks at how it
  ended, and calls `release`. `Give` reads the variable and leaves
  it empty. The VM: the cleanup is written at each jump out of the
  body, and one handler passes an error on unchanged.
- `emit_c.rs` and `emit_llvm.rs`: with no collector and in a firmware
  build every way out of a block already frees what the block owns
  (5.8h), and the call of `release` goes in front of that. An
  ordinary build jumps over the blocks between a `raise` and its
  `except` (5.5i), so there a `Scoped` is a frame of the kind a `try`
  has, which releases and jumps on. It costs what a `try` costs, and
  only in a block that owns a resource.
- `lower.rs`: `Scoped` becomes a `try` that catches whatever passes,
  releases and passes it on unchanged, with the release also written
  at each other way out. One new lowered statement, "pass this error
  on as it was", for each emitter and each runtime.

**Tests.** One program that prints a line from each `release`, for
every way out: the end, `return` with a value that uses the resource,
`break`, `continue`, an error passing through two blocks that each
own one, an error caught inside, two in one block, one given away on
every path, one given away on one path of an `if`, one taken by a
parameter, one at the top of the file, one made in each turn of a
loop, and `program.exit`. It is built every way, with 0 blocks left,
and is `tools/targets/edge-resource.zeno`. An uncaught error that
passed through a `Scoped` must be reported exactly as it is without
one. Each refusal of rule 7. A program with its own `Resource`.

**Speed.** `tools/same.py`: a program with no resource has the C, the
IR and the bytecode it had. The interpreter gains one arm for a
statement old programs do not have; the count of step 2 confirms it.

### 12.2 A file read and written in pieces

```zeno
import files

let source = files.read_from("big.log")
let errors = files.write_to("errors.log")

for line in source {
    if line.find("error") is not none {
        errors.write(line + "\n")
    }
}
```

| Written | Gives |
|---|---|
| `files.read_from(path)` | a `files.Reader`, at the start of the file. `FileError` if it cannot be read |
| `files.write_to(path)` | a `files.Writer` on a new, empty file, as `files.write` begins one |
| `files.append_to(path)` | a `files.Writer` at the end of the file, as `files.append` |
| `reader.read_line()` | the next line without its line end, as `input()` gives one (6.18), or `none` at the end |
| `reader.read_bytes(count)` | up to that many bytes as a `list[u8]`; fewer at the end, none after it |
| `for line in reader` | each line in turn, one at a time |
| `writer.write(text)`, `writer.write_bytes(bytes)` | writes, and adds no line end (6.13) |

A `Reader` and a `Writer` are resources (12.1): the file is closed
when its owner ends. So there is still no `close`, which is what 6.12
wanted of an open file: nothing to forget.

**Why three functions and two types**, and not `open(path, "r")`.
6.12 turned the letters down: a wrong letter is found when the
program runs. The three names are the three that exist (`read`,
`write`, `append`) with the word that says "and go on from here". And
with two types, reading from a file opened for writing is a mistake
found before the program runs (principle 1).

**Why `for line in reader`.** A loop over a dict is given its keys
(6.38), and a loop over a reader is given its lines; it is what
everyone writes first. It holds one line at a time, where
`files.read(path).lines()` holds the file.

**With no collector** a line is built text, in the text room for its
statement and copied to the pool when kept (5.9c, 6.20). The open
file itself is one small block from the pool. **On a board** these
are the C library's, and a board with no file system answers
`FileError` (6.20).

**How it is built.** These ask the machine, so they are written for
each engine (33.10): `builtins.rs` for the interpreter and the VM,
`runtime.c` for both compilers, and each target's runtime, beside
`files.read` in each. `checker.rs` knows the two types and their
methods as it knows a `Serial`. The `for` is the one piece of
language: a loop over a `Reader` in the interpreter, the VM, both
compilers and `lower.rs`.

**Tests.** `tools/targets/edge-files.zeno` grows: a file of a
thousand lines read by `for` and by `read_line`, with both line ends
and with no line end at the end (6.17); bytes read in pieces across
the end; a file written in pieces and read back; a file opened twice
in a row, which fails on Windows if the first was not closed; a
missing file; a line longer than the text room, which stops with the
room's message. With no collector the pool must not grow over a
thousand lines.

### 12.3 Folders

`files.list(folder)` gives the names in a folder as a list of text,
sorted, so that every engine and every system gives the same list.
`files.make_folder(path)` makes one, and its parents.
`files.join(first, second)` joins two parts of a path with one `/`,
which every system Zeno runs on accepts; it is arithmetic on text, so
it is written in Zeno, in `library/files.zeno` (chapter 33). The
other two ask the machine, and are written for each engine.

This part is the last of the item and is cut loose from the rest: it
can go out a release later with no change to 12.1 or 12.2.

**Left for later after item 12:** a resource in a field, a list or a
`T or none`; moving to a place in a file; a file opened for both;
removing a file or a folder; the date and the time of day.

---

## 13. Tools

Six small parts. None of them changes the language but 13.1.

### 13.1 A message on `assert`

```zeno
assert reply == Ack, f"the board answered {reply} to the reset"
```

10.17 left it out and said it could be added without changing
anything. The text is worked out only if the `assert` fails, and is
printed on a line of its own above "this is false. The left of ...",
which stays. It must be text. A comma, as in Python: `assert` is a
statement, so a comma after its condition can be nothing else.

**Built in:** `ast.rs` (`Assert` gains an optional text), `parser.rs`,
`checker.rs`, `Interpreter::assert`, `printer.rs`.

### 13.2 Tests of a compiled program: `zeno test --build`

```
$ zeno test --build --no-collector examples/ring
```

Each file is built as a program that runs the file and then its
tests, and prints what `zeno test` prints, line for line. It takes
the options `zeno build` takes: `--firmware`, `--no-collector`,
`--llvm`, `--release`, and `--board`, where the program is written as
`<name>-tests.elf` and the line to run it is shown, as for any
program for a board (51.5).

**What it changes about a test.** `zeno build` leaves tests out
(10.20), and a test is not held to the rules of a build. Under
`--build` it is: a test that shares a list is refused with no
collector, by the message a program gets. That is the point of the
command: the program is tested as it is built.

**How.** A test is a C function. A failed `assert` prints its
message and leaves the test with a jump forward to the end of that
function, the one form of `goto` a firmware build already uses
(5.5l); `assert` stands only in a test (10.17), so the jump never
crosses a function. The line of source that a failure shows is
written into the program as text, in a test build only. `assert
raises` is a `try`. `main` runs the program, then each test, and ends
with 1 if any failed.

**Tests.** For every example with tests, and one written to fail
each way an `assert` can fail, `zeno test` and `zeno test --build`
must print the same, built every way. `--only` works with it (17.5).

### 13.3 Stepping through a `.zeno` file: `zeno build --debug`

The C that a build writes gets a `#line` mark in front of each
statement, naming the `.zeno` file and the line, and the C compiler
is asked for `-g -Og`. Then gdb, lldb and every probe for a board
step through the Zeno, set a breakpoint at `blink.zeno:12`, and show
a variable under the name it has in the C (`z_count`). The runtime
that follows the program is marked back to the C file, so a step
into it shows C.

It goes with every kind of C build and with `--board`. With `--llvm`
it is refused for now, with a message: debug information in IR is a
tree of its own.

**Tests.** A build without `--debug` writes the C it wrote before
(`tools/same.py`). With it, each mark names the right line, in a
program of several files too (7.12). Where gdb is installed, a test
drives it: a breakpoint at a line of a `.zeno` file is reached, and
`next` moves a line of Zeno at a time. A page, `docs/debugging.md`,
walks through a session on the computer and on the emulated board.

### 13.4 A REPL: `zeno repl`

```
$ zeno repl
> let rate = 9600 baud
> rate * 2
19200 baud
> function twice(n: int) -> int {
.     return n * 2
. }
> twice(21)
42
```

- An entry is checked, as `zeno run` checks, and then run by the
  interpreter, in one program that grows. An entry with a mistake
  shows it and leaves nothing behind.
- An entry that is a value shows it, as `print` would show it inside
  a list, so that text has its quotes; `none` shows nothing.
- A line that leaves a bracket open goes on to the next.
- A name can be declared again: the new declaration takes the place
  of the old one. In a file that is an error (3.2), and in a REPL it
  is what trying something twice means. This is the one rule that
  differs from a file, and the page says so in its first lines.
- It reads plain lines, with no library for editing them (5.2), and
  works the same when its input is a file, which is how it is tested.

**Built in:** `main.rs` and a new `repl.rs`, on the interpreter's
ability to keep going after a program ends, which `run_tests`
already uses.

### 13.5 Completion in the editor

9.11 left it out because the line being typed does not parse, and
the checker never sees such a file. The answer keeps the rule of
9.11, that the server answers from the checker and guesses nothing:

- The server takes the text of the editor and puts a name that no
  program can write where the cursor is. After a dot that is `port.`
  made into `port.<that name>`. It closes the brackets the line left
  open, so that the file parses in the common cases.
- The checker, asked to, notes two things when it meets that name:
  after a dot, the fields and methods of the type in front of the
  dot, or the names of the module; anywhere else, every name that
  can be seen from that place, with its type. Both come from the
  scopes the checker already walks.
- If the file still does not parse, the answer is empty. An empty
  answer is honest; a guess is shown as if it were known (9.11).

**Tests.** After a dot on an object, a list, text, a dict, a module
and a value that may be `none`; a name at the top of a file and
inside a function with a parameter that hides a variable; a line
inside an open bracket; a file that cannot be mended. `editors/talk.py`
asks for it.

### 13.6 Every use of a name, renaming, and where a field is declared

The checker's record (`Use`) says for each use which declaration it
means. So "every use of this name" is the uses with one declaration,
in every file of the program, and renaming is writing the new name
at each of them and at the declaration. `ast::Field` and a method
gain the place they are written, which 17.4 held back; then a field
and a method are found, listed and renamed too.

A rename is refused, with a reason the editor shows, when the new
name is not a name, is a word of the language, is declared already
in a scope where it would hide or be hidden, or when the declaration
is in a package and not in the program.

**Tests.** A variable hidden by a parameter of the same name (only
the right ones change); a function used in three files; a field; a
name of a module (`shapes.area`, where only the last part changes);
each refusal. After a rename the program must check and print what
it printed.

**Left for later after item 13:** lines run before each test; how
much of a program its tests ran; debug information through LLVM;
completion in a file that cannot be mended; the VS Code extension in
the Marketplace.

---

## 14. More than one thing at a time

`missing.md` says to choose one model, and the one a board can run:
tasks that take turns, with a queue between them. This is that
choice, with its reasons, and it comes in two parts of very different
size.

### The model

**One thing runs at a time, and it runs until it returns.** A task is
a function. Something says when it is due: a time, a queue with
something in it, an edge on a pin. A loop, `tasks.run()`, calls each
function that is due, one after the other.

| Model | Verdict |
|---|---|
| Threads that the system switches between (C, Java, Go, Rust) | Two pieces of code can then touch one value at once. Rust guards that with ownership through the whole language; in Zeno owning is one way to build a program. And a bare board has no system to switch them. 50.3 turned this down for interrupts, for the same reasons |
| A task with a stack of its own, switched by hand (Lua, early Go) | Switching stacks is not C: it is assembly for each processor, each stack is a size somebody guessed, and no tool can then say how deep a stack gets |
| `async` and `await` (JavaScript, Python, Rust) | Two kinds of function, where one kind cannot call the other in the plain way. It is the right machine underneath (see 14.2) and the wrong thing to write |
| **Functions that take turns, with queues between them** | **Chosen.** It is how most firmware is already written, as a loop that calls each job in turn. It is what a page already does in Zeno (`web.every`, `web.after`). It needs no stack, no switch and no lock, and the rule of 50.3 holds: the program's code never runs inside an interrupt |

What it gives up: two processors working at once. That is for a
server, and can come later as programs that send each other messages
through the same queues.

### 14.1 The module `tasks`

```zeno
import tasks
from board import analog_pin, input_pin, pull_up, falling

let probe = analog_pin(0)
let button = input_pin(2, pull_up).interrupt(falling)
let readings: tasks.Queue[int] = tasks.queue(8)

function sample() {
    readings.put(probe.count())
}

function show(reading: int) {
    print(reading)
}

function pressed(edges: int) {
    tasks.stop()
}

tasks.every(10 ms, sample)          # a time
tasks.when(readings, show)          # a queue that has something in it
tasks.when_edge(button, pressed)    # edges the board counted
tasks.run()                         # until tasks.stop(), or nothing is left to wait for
```

| Written | Does |
|---|---|
| `tasks.every(period, do)` | calls `do` that often. The time has a unit, and comes first, as in `web.every` |
| `tasks.after(delay, do)` | calls `do` once, that much later |
| `tasks.queue(size)` | a `tasks.Queue[T]` that holds at most `size` items. `put(item)` answers `false` when it is full, and nothing grows; `take()` gives the oldest, or `none` |
| `tasks.when(queue, do)` | calls `do` with each item, in the order they were put |
| `tasks.when_edge(interrupt, do)` | calls `do` with the number of edges, in the loop and not in the interrupt (50.3). It is the "function of the program run at each edge" that 50.6 left for this item |
| what each of those gives | a `tasks.Task`, with `stop()` |
| `tasks.run()` | the loop. It returns when `tasks.stop()` is called or no task is left |

- **Nothing is shared at the same moment**, so there is no lock, no
  `volatile`, and no rule of the checker for it. A function that is
  called reads and changes what any function may: a `let` at the top
  of the file (audit, part 5), with one owner too.
- An error that leaves a task ends the program with its message, as
  an error that leaves the program does.
- A task that never returns keeps every other task waiting. That is
  the price of the model, and the page says it in its second
  paragraph.

**It is written in Zeno**, in `library/tasks.zeno` (chapter 33), on
what exists: `time.now()`, `sleep`, the edges of 50.2, a generic
class for the queue, and a list of the things that are waited for,
each an object of a class that is one interface. So it needs no line
in any engine, and it runs in all four and in every target that has
a clock. That is why it is last: it leans on the generics of item 11
and on 9.2 (a function that is kept), and it is their first real
user.

Three things the library itself must learn, each small: a part
written in Zeno has held only functions so far, and must hold a
class and a `let`; a part may import another module that comes with
Zeno; and a program's own `tasks.zeno` must still win, with a test.
On a page (`--web`, `--target html`) a loop that sleeps would hold
the page still, so `import tasks` is refused there with a message
that names `web.every`.

**Tests.** `tools/targets/edge-tasks.zeno`: tasks that are due
together run in the order they were made; a queue that fills; a task
that stops itself and one that stops another; `after`; an error in a
task; `run()` returning when nothing is left. Time is driven by the
simulated board's clock, so the test waits for nothing. On the
emulated Cortex-M3 (`tools/board.py`) the same program, with
`when_edge`, with no collector, and with the pool not growing over
ten thousand turns of the loop.

**Speed.** The module is joined only into a program that names it
(33.3), and `tools/same.py` shows that every other program is the
program it was.

**Left for later:** waiting with the processor asleep until the next
interrupt or the next time that is due, which needs one more function
of the board; `tasks` on a page.

### 14.2 A task that waits in the middle: designed here, and not decided here

With 14.1 a task that does three things with a pause between them is
three functions, or one function and a variable that says how far it
got. That is how firmware is written, and it is also the part of
firmware that people dislike writing. The other way is a function
that can stop in the middle and go on later:

```zeno
task function blink(led: Pin) {
    while true {
        led.toggle()
        wait 500 ms          # the others run meanwhile
    }
}
```

It can be built with no stack of its own: the compiler makes such a
function into an object that keeps its variables as fields and a
number for where it is, with a `step` that runs to the next `wait`.
That is a change to the tree before any engine sees it, so it would
be written once, and it would run on 14.1 as it stands.

It is not planned here, for three reasons. It is the first thing on
this page that needs new words (`task`, `wait`), and a word is taken
from every program. It asks a question 14.1 does not: what a `wait`
inside a function that a task calls should mean. And whether it is
needed is best seen from programs written with 14.1. So when 14.1 is
done, this part is put to the owner of the language, with those
programs beside it.

---

## What was tried for this page

Each of these was run on the `zeno` of 0.7.0, and said what the page
says it says: a function that says `raises` handed to `map` and to a
parameter of a function type, under `zeno run`, `zeno check --strict`,
`zeno build --firmware` and `zeno build --no-collector`; `list[int or
none]` and a dict of lists with no collector; `raise e` in an `except`
with no collector (it builds); a method kept as a value; a function
with a name inside a function; a default that is not a literal; a
list as `T` of a generic function and of a generic class; `compose`;
a field that may be `none` moved out; `xs.append(xs.pop())`; a
function of an interface that takes out of its object; an `owned
class` that is an interface; a default in an interface; `--pool`;
`-> (int, int)`; `let q, r = f()`; `a, b = b, a`; `(1, "a")`; `enum
Maybe[T]`; `interface Ordered[T]`; `defer`; a block that stands
alone; `raise e` after a caught error, which names the line of the
`raise` and not the line the error came from.

## Changes to this plan

Nothing yet. When building a part shows that a choice above should be
different, the change and its reason are written here, with the date,
and in the chapter of the decisions.
