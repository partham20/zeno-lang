# Enum values and `none`, with one owner

A build with no collector used to refuse two everyday things: an enum
case that carries values (`Wait(ms: int)`), and a list or an object
that may be `none` (`var last: Task or none`). Both build now.

```sh
cargo run -- build --no-collector --memory examples/commands.zeno
./commands
```

```
not a command: jump 3
  ( cannot turn "soon" into an int )
not a command: wait soon
6 commands, busy for 575
[300] Blink([2, 3, 5])
[315] says: say hello there
[316] Set(13, true)
[566] Wait(250)
Halted with 1 left
last: pin 4 on: false
Machine(mode=Halted, clock=566, waiting=[Wait(9)], last=Set(4, false))
memory: 68 blocks made (1285 bytes), at most 481 bytes in use at once, no collector, 0 blocks left at the end
pool: 880 bytes taken, in a pool of 32768
text: at most 101 bytes in use at once, in a room of 1024
```

[`examples/commands.zeno`](../examples/commands.zeno) reads lines of
text as commands, queues them in a machine and runs them. A command
may carry a number, a text or a list. A line may be no command at all.
It ends with **0 blocks left**, and `zeno run` prints the same eleven
lines.

This page follows on from [`one-owner.md`](one-owner.md). It is the
fourth of five steps that take this build to "whoever receives a value
owns it".

## The files

- [`src/checker.rs`](../src/checker.rs): `EnumInfo.owned`, `lent`,
  `borrowable`, `Looking`, and `match_statement` with `pattern`.
- [`src/emit_c.rs`](../src/emit_c.rs): `register_enum`, which writes the
  function that makes each case and the one that frees a value, and
  `pattern`.
- [`src/emit_llvm.rs`](../src/emit_llvm.rs): `drop_enum` and
  `must_be_there`, the same in IR.

## An enum value that carries something is a small object

```zeno
enum Command {
    Stop
    Wait(ms: int)
    Say(text: str)
    Blink(pins: list[int])
}
```

`Wait(250)` has to keep its 250 somewhere, and `Blink([2, 3, 5])` has
to keep a list. So such a value is kept in a block of its own, as an
object is, and in this build it follows the one rule of
[`one-owner.md`](one-owner.md): **whoever received it owns it, and
frees it.**

```zeno
let a = Wait(3)
let b = a           # the command moves to `b`
print(a)
```

```
error: `a` was given away on line 2 and holds nothing now. Use it
before it goes, or give it only to what looks: a parameter without
`take`
```

Everything on the earlier page holds for a `Command` as it does for an
object: a list of commands owns each one, `pop()` takes one out, a
plain parameter looks and a `take` parameter takes.

Two details make it cheaper than an object.

**A case that carries nothing is never made.** There is one `Stop` in
the whole program, fixed in place before it starts. Writing `Stop`
costs nothing, and freeing a command that turns out to be `Stop` does
nothing. A variable that holds it is still a `Command`, and still has
one owner: the checker goes by the type, not by which case it happens
to be at the time.

**An enum where no case carries anything is not owned at all.**

```zeno
enum Mode {
    Idle
    Busy
    Halted
}
```

A `Mode` is one of three fixed values. It is copied like a number, in
every build, and `let a = Idle` followed by `let b = a` leaves both
usable.

| The enum | A value of it | In a build with no collector |
|---|---|---|
| no case carries anything (`Mode`) | one of a few fixed values | copied freely |
| some case carries something (`Command`) | a block, or a fixed value for a bare case | one owner, which frees it |

A case owns what it carries. `Say(line)` keeps its own copy of the
text. `Blink(pins)` is given the list, and frees it when the command
is freed.

## What may be `none` has one owner too

`parse` gives back a command, or nothing:

```zeno
function parse(line: str) -> Command or none { ... }

let command = parse(line)
if command is none {
    print("not a command:", line)
} else {
    machine.queue(command)    # `command` holds nothing after this line
}
```

`command` owns whatever came back. If that is `none` there is nothing
to free, and freeing it does nothing. If it is a command, the variable
frees it at the end of its block, unless it has been given away first,
as it is here.

The value cannot be used as a `Command` until the program has asked:

```zeno
let found = find(2)
send(found)
```

```
error: `command` of `send` should be Command, but this is Command or
none. Check it with `is not none` first
```

That rule is as old as `or none` itself. What is new is that the thing
inside may be a list, an object or an enum value in this build:

| Written | Who owns it |
|---|---|
| `let found = find(2)` | `found`, until it gives it away |
| `last: Command or none` in a class | the object |
| `machine.last = parse("set 4 off")` | the object. The old value is freed on the spot |
| `function describe(p: Packet or none)` | the caller: `p` is a look, at a packet or at nothing |
| `list[Command or none]` | the list |

A value that sits in a field stays there. Taking it out would leave a
hole that the type does not admit to:

```zeno
let last = machine.last
```

```
error: `last` holds an owned value, which cannot be taken out of the
object it is in. Use it where it is, or pass it to a parameter that
looks
```

## `match` looks

`match` is how a program gets at what a case carries. It never takes
anything out. The value stays with its owner, and each name in a
pattern is either a copy or a look:

```zeno
function cost(command: Command) -> int {
    match command {
        case Stop { return 0 }
        case Wait(ms) { return ms }                    # a number: copied
        case Set(_, _) { return 1 }
        case Say(text) { return len(text) }            # a text: copied
        case Blink(pins) { return 100 * len(pins) }    # a list: a look
    }
}
```

| The name stands for | It is |
|---|---|
| a number, a `bool`, a text, a plain enum value | a copy, yours to keep |
| a list, an object, an enum value that has one owner | a look: read it, change it, lend it. It cannot be kept |
| the whole value (`case other`) | a look at it |

```zeno
match command {
    case Blink(pins) { let mine = pins }
    case _ { }
}
```

```
error: `pins` is a look at the value that the `match` is looking at,
or at a part of it. Its owner still has it, so it cannot be kept,
returned or given away. Use it here, or pass it to a parameter that
looks
```

### While a `case` is looking, the owner stays as it is

A look is only good while the thing it looks at is alive. So for as
long as a `case` holds a look, the owner of the matched value cannot
give it away or be given a new value:

```zeno
var command = Blink([1, 2])
match command {
    case Blink(pins) {
        send(command)
        print(pins)
    }
    case _ { }
}
```

```
error: the `match` on line 2 gave a name to a part of what `command`
holds, and this `case` is still looking at it. `command` cannot be
given away until the `case` is over. If the name is not needed, write
`_` in its place
```

The rule is no wider than it needs to be. A `case` with no look in it
may do what it likes with the owner, and that is exactly the shape of
a state machine:

```zeno
var state = Wait(1)
match state {
    case Wait(ms) { state = Say("done") }    # `ms` is a copy, not a look
    case _ { state = Stop }
}
```

That builds. The old `Wait(1)` is freed when the new value takes its
place.

## What the C looks like

A `Command` is a tag and then room for whichever case it is:

```c
struct z_Command {
    int tag;
    union {
        struct { int64_t z_ms; } c1;
        struct { int64_t z_pin; bool z_on; } c2;
        struct { const char *z_text; } c3;
        struct { zeno_list *z_pins; } c4;
    } as;
};
static z_Command zeno_case_Command_Stop = {.tag = 0};
```

`Stop` is that last line: one value, not in the pool.

**Each case asks the pool for its own size.** A `Wait` needs a tag and
one number. It should not pay for the room a `Set` would need:

```c
static z_Command *zeno_new_Command_Wait(int64_t z_ms) {
    z_Command *self = (z_Command *)zeno_alloc(offsetof(z_Command, as) + sizeof(((z_Command *)0)->as.c1));
    self->tag = 1;
    self->as.c1.z_ms = z_ms;
    return self;
}

static z_Command *zeno_new_Command_Say(const char *z_text) {
    z_Command *self = (z_Command *)zeno_alloc(offsetof(z_Command, as) + sizeof(((z_Command *)0)->as.c3));
    self->tag = 3;
    self->as.c3.z_text = zeno_text_keep(z_text);
    return self;
}
```

`zeno_text_keep` is the copy of the text that the command owns
([`text.md`](text.md)).

**Freeing goes by the tag.** One function for the enum, written by the
compiler:

```c
static void zeno_drop_Command(z_Command *self) {
    if (self == NULL) return;
    switch (self->tag) {
    case 1: zeno_free(self); break;
    case 2: zeno_free(self); break;
    case 3: zeno_text_drop(self->as.c3.z_text); zeno_free(self); break;
    case 4: zeno_list_drop(self->as.c4.z_pins); zeno_free(self); break;
    default: break;
    }
}
```

The first line is what makes `none` free to own: it is `NULL`, and
there is nothing to do. `default` is `Stop`, the fixed value, which is
left alone.

**A new value frees the old one.** `machine.last = parse("set 4 off")`:

```c
z_Command *t9 = z_parse("set 4 off");
z_Command *t10 = t9;
zeno_drop_Command(z_machine->z_last);
z_machine->z_last = t10;
```

**`match` reads in place.** `cost`:

```c
z_Command *t28 = z2_command;
if (t28->tag == 0) {
    return 0;
} else if (t28->tag == 1) {
    int64_t z_ms = t28->as.c1.z_ms;
    return z_ms;
...
} else if (t28->tag == 3) {
    const char *z_text = zeno_text_keep(t28->as.c3.z_text);
    int64_t t29 = zeno_len(z_text);
    zeno_text_drop(z_text);
    return t29;
} else {
    zeno_list *z_pins = t28->as.c4.z_pins;
    return zeno_mul(100, z_pins->len, 64, 35);
}
```

`z_ms` is a copy of a number. `z_text` is a copy of the text, freed
when the case is over. `z_pins` is the command's own list, looked at
and not freed.

**A parameter that was not given on is freed on every way out.** `run`
takes its command:

```c
if (t35->tag == 0) {
    z_Mode *t36 = (&zeno_case_Mode_Halted);
    zeno_drop_Command(z2_command);
    return t36;
}
...
z_Mode *t37 = (&zeno_case_Mode_Busy);
zeno_drop_Command(z2_command);
return t37;
```

## A bug this found: `case Stop` on a value that may be `none`

```zeno
match machine.last {
    case none { print("nothing ran last") }
    case Set(pin, on) { print("last: pin", pin, "on:", on) }
    case _ { print("last: something else") }
}
```

`zeno run` has always handled this. The two compilers did not: on a
subject that may be `none`, `case Set(pin, on)` was refused and a bare
`case Stop` was compiled as a new name that matches anything. That was
wrong in every compiled build, with or without a collector. Both now
ask "is it there?" first and then test the tag:

```c
} else if (t11 != NULL && t11->tag == 2) {
```

The test `a_case_of_an_enum_matches_a_value_that_may_be_none` holds it
in place.

## In every build

Nothing above needs `--no-collector`. A build with a collector accepts
the same shapes when the words are written, as on the earlier pages:

```zeno
owned class Packet {
    id: int
    data: owned list[int]
}

owned class Slot {
    held: Packet or none
    spare: owned list[int] or none
}

function describe(p: Packet or none) -> str { ... }
```

And `take` can be written on any enum that carries values, in any
build. Where a collector runs it changes nothing about memory, but the
same file then builds both ways.

## The same, through LLVM

```sh
cargo run -- build --llvm --no-collector --memory examples/commands.zeno
```

The checker does all the deciding, so the LLVM backend needed the
frees and nothing else. `drop_enum` writes this for an enum with a
bare case, a case with a number and a case with a text and a list
(from the test `the_ir_for_an_enum_value_that_has_one_owner`):

```llvm
there1:
  %t2 = load i64, ptr %self
  switch i64 %t2, label %bare [ i64 1, label %case1 i64 2, label %case2 ]

case1:
  call void @zeno_free(ptr %self)
  ret void

case2:
  %t3 = getelementptr %z_Shape.Named, ptr %self, i32 0, i32 1
  %t4 = load ptr, ptr %t3
  call void @zeno_text_drop(ptr %t4)
  %t5 = getelementptr %z_Shape.Named, ptr %self, i32 0, i32 2
  %t6 = load ptr, ptr %t5
  call void @zeno_list_drop(ptr %t6)
  call void @zeno_free(ptr %self)
  ret void

bare:
  ret void
```

Both builds of `commands.zeno` print the same and give the same memory
report, line for line. That is why the C build asks for each case's
own size: the LLVM build always did, and the two reports must agree.

## What is still refused

| Refused | Why | When |
|---|---|---|
| A list of numbers that may be `none`: `list[int or none]` | each number sits in a block of its own, which the list would have to copy and free everywhere | noted. Use an enum with a `Missing` case |
| ~~A function written where a value goes, a function type that mentions a list or an object, an error that is kept or raised again~~ | all three build now: [`functions-and-errors.md`](functions-and-errors.md) | done in step 5 |
| A value with one owner as what a type parameter stands for | the copy for `T` would have to know it moves | noted |
| ~~A function that uses a list or object declared at the top of the file~~ | it builds now, for a `let`: [`audit.md`](audit.md) | done in the audit |
| A method that pops an item or replaces a field that has one owner | `self` only looks | for the audit |

The first row has an answer today, and the compiler's message gives
it:

```zeno
enum Reading {
    Missing
    Value(n: int)
}

var readings: list[Reading] = []
```

`Missing` costs nothing, and the list owns each `Value`.

One thing here was clumsier than it should have been, and the audit
fixed it. After `case none`, a later `case packet` used to have the
type `Packet or none`, so the body had to ask `if packet is not none`
again. Now the cases after `case none` know that the value is there,
and an enum that may be `none` is complete once `none` and each of its
cases has a `case` ([`audit.md`](audit.md)).

## How it is tested

- **The checker** (`checker.rs`): `what_may_be_none_can_have_one_owner`,
  `a_match_looks_at_a_value_that_has_one_owner` and
  `with_one_owner_an_enum_value_that_carries_values_has_one`: what each
  rule on this page accepts, and the message of what it refuses.
- **Both compilers** (`emit_c.rs`, `emit_llvm.rs`): four programs and
  `commands.zeno`. They cover every case of an enum, errors raised half
  way through building a value, every early way out of a `match`, an
  enum that holds itself (a tree), and `or none` in variables, fields,
  parameters, returns and lists. Each must print what `zeno run`
  prints, end with 0 blocks left, and give the same memory report
  through C and through LLVM.
- **In every build**: `what_may_be_none_can_have_one_owner_in_every_build`
  runs the `owned class` program above with a collector as well.
- **Over time**: a state that changes on every trip of a loop, 50 trips
  and 5000 trips, needs the same pool
  (`with_no_collector_enum_values_do_not_fill_the_pool_over_time`).
- **The generated code**: `the_c_for_an_enum_value_that_has_one_owner`
  and `the_ir_for_an_enum_value_that_has_one_owner` compare the frees
  with what this page shows.
- By hand: every program was also run with each freed block
  overwritten, so that a look at freed memory would print rubbish. None
  did, in either backend.

## Things to try

1. In `commands.zeno`, write `take` in front of the type of `cost`'s
   parameter and run `zeno check --no-collector`. Two places complain. Why is the loop's
   `command` in `busy_for` not the loop's to give?
2. Add a case `Repeat(times: int, what: Command)` to `Command`: a
   command that holds a command. Build it, open `commands.c`, and read
   what `zeno_drop_Command` does for the new case. How deep can it go?
3. In `run`, change `case other` to print and then `return Halted`
   after giving the command away with a function that takes it. Read
   the error. Now do the same in a `case _`. Why is one refused and the
   other not?
4. Write the state machine from this page with a case
   `Collecting(seen: list[int])`, and try to move from it to `Stop`
   inside `case Collecting(seen)`. Fix it two ways: with `_`, and by
   deciding the next state inside the `match` and assigning after it.
5. `Wait(250)` takes 16 bytes of the pool and `Stop` takes none. Use
   `--memory` to find out what a `Set(13, true)` takes, then explain
   the number from `struct z_Command` above.
6. `list[int or none]` is refused. Write the `Reading` enum from this
   page, fill a list with five readings, and compare its memory report
   with the same program using `list[int]` and -1 for "missing". What
   does the safety cost in bytes?
7. `let last = machine.last` is refused because it would leave a hole.
   Rust has `take()`, which leaves a `none` behind and hands you the
   value. What would that look like in Zeno, as one plain word? What
   should happen when the field's type is not `or none`?
