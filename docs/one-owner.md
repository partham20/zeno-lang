# One owner

Until now a build with no collector made you write the word `owned` on
every list and every class, and it refused a list of objects however
you wrote it. Both are gone.

```sh
cargo run -- build --no-collector --memory examples/scheduler.zeno
./scheduler
```

```
runs: 10
blink ran 5 times: [2, 4, 6, 8, 10]
sample ran 3 times: [3, 6, 9]
log ran 2 times: [5, 10]
busiest: blink
blink now runs every 4
left: 2
retiring log
it ran 2 times
2 ran at tick 12
[0, 0, 0]
[0, 0, 7]
[1, 2, 3]
memory: 53 blocks made (1328 bytes), at most 504 bytes in use at once, no collector, 0 blocks left at the end
pool: 800 bytes taken, in a pool of 32768
text: at most 55 bytes in use at once, in a room of 1024
```

[`examples/scheduler.zeno`](../examples/scheduler.zeno) has two classes,
a list of objects, a list inside each object and a list of lists. The
word `owned` is not in it. It still ends with **0 blocks left**, and
`zeno run` prints the same thirteen lines.

This page follows on from [`temporaries.md`](temporaries.md). It is the
third of five steps that take the rules of that build from "refuse what
has no owner" to "whoever receives a value owns it".

## The files

- [`src/checker.rs`](../src/checker.rs): `Rules` and `check_by` at the
  top. Then, under "Owned values": `takes_out`, `walk`, `being_walked`,
  `item`, and the end of `take`.
- [`src/emit_c.rs`](../src/emit_c.rs): `Drops`, which names the function
  that frees each kind of value, and `write_drops`, which writes the
  ones a program needs.
- [`src/emit_llvm.rs`](../src/emit_llvm.rs): `drop_list`, the same in IR.
- [`src/main.rs`](../src/main.rs): `zeno check --no-collector`.

## The idea: a build with no collector has no shared values

Zeno has two kinds of lists and objects ([`ownership.md`](ownership.md)).
A **shared** one can be reached from many places and the collector frees
it when the last place lets go. An **owned** one has one owner, which
frees it where its block ends.

A build with no collector has nothing to free a shared value with. So
the old rule was "write `owned` everywhere", and the build refused the
program until you had. That is a rule a compiler can follow by itself:

> **With no collector, every list and every object has one owner: whoever
> received it.**

| It is received by | Example | Who frees it |
|---|---|---|
| a variable | `let tasks = [1, 2, 3]` | the variable's block, at its `}` |
| a field | `Task("blink", 2, [])` | the object, when it is freed |
| a list | `tasks.append(Task(...))` | the list, when it is freed |
| a parameter | `retire(last)` | the function, when it returns |
| a `borrow` parameter | `report(task)` | not the function: the caller still owns it |
| nobody | `print(len(line.split()))` | the statement ([`temporaries.md`](temporaries.md)) |

So in this build `list[int]` *means* `owned list[int]` and `class Task`
*means* `owned class Task`. The rules of [`ownership.md`](ownership.md)
apply to all of them, and the two that matter day to day are:

```zeno
let a = [1, 2, 3]
let b = a           # the list moves to `b`
print(a)
```

```
error: `a` was given away on line 2 and holds nothing now. Lend it
instead (a `borrow` parameter), or use it before it goes
```

and a function that only looks says so:

```zeno
function report(task: borrow Task) -> str { ... }    # looks
function retire(task: Task) -> int { ... }           # takes, and frees
```

The word `owned` still works in this build and changes nothing there.
In a build *with* a collector it means what it always did.

### Fewer programs, never different ones

The same file must mean the same thing under `zeno run`, `zeno build`
and `zeno build --no-collector`. It does. This build adds refusals and
nothing else: a program it accepts never gives a value two owners, and
for such a program "one owner" and "shared" cannot be told apart from
the output. The tests build every program of this page both ways and
compare.

`zeno check --no-collector file.zeno` checks as this build would,
without building.

## A list owns its items

`scheduler.tasks` is a list of tasks. The scheduler owns the list. Who
owns each task? The list does, and that is new in **every** build: an
owned list may now hold values that have one owner. (In a build with a
collector that needs the words: `owned list[Packet]` of an
`owned class Packet`.)

Four things can be done with an item, and each has one meaning:

| Written | What happens | The item belongs to |
|---|---|---|
| `tasks[0]`, `for task in tasks` | a look | the list, as before |
| `tasks.append(task)` | the list is given the item | the list, from here on |
| `tasks.pop()` | the item is taken out | whoever receives it |
| `tasks[1] = Task(...)` | a new item takes the place of an old one | the list. The old one is freed on the spot |

A look can do everything except keep the item. It can read it, change
it (`tasks[0].every = 4`), call its methods and lend it to a `borrow`
parameter. Keeping it is the one thing refused:

```zeno
let first = tasks[0]
```

```
error: this item is an owned Task, which has one owner: the list. `[ ]`
only looks at it. Use it where it is, lend it to a `borrow` parameter,
or take it out with `pop()`
```

Three things that would put an item in two lists are refused the same
way, each with what to write instead: `a + b`, `a += b` and `filter`.
`map` and `reduce` are fine: they lend each item to the function and
build something new from the answers.

## Only the owner takes an item out

`pop()` and `tasks[1] = ...` both end an item's time in the list. If
anyone were still looking at that item, they would be looking at freed
memory. Three small rules make sure nobody is. Each can be checked by
reading one function, with no lifetimes to write.

**1. Only the owner.** A `borrow` parameter cannot pop, and nor can
`self` in a method, which is a borrow too.

```zeno
function last(tasks: borrow list[Task]) -> Task { return tasks.pop() }
```

```
error: only the owner can take an item out of this list: somebody else
may have been lent it. Here `tasks` is only borrowed
```

**2. Not while a loop is walking the list.** The loop is looking at an
item on every trip.

```zeno
for task in tasks {
    if task.name == "a" { print(retire(tasks.pop())) }
}
```

```
error: the `for` on line 8 is walking a list that `tasks` owns, and is
still looking at its items. Nothing can take an item out of this list
until the loop is over
```

**3. Not on a line that has already looked.** In
`print(report(tasks[0]), retire(tasks.pop()))` the first look may still
be in use when the pop happens.

```
error: this line has already looked at `tasks`, and the look might
still be in use. Do the other thing on a line of its own first
```

Appending is not on this list. A `for` loop walks a copy of the list
(decision 3.10a), so `tasks.append(...)` inside the loop, or through a
borrow, moves nothing the loop can see.

These rules refuse some programs that would be safe.
`tasks.append(tasks.pop())` is one: write it on two lines. Rust accepts
more, with far more machinery. Rule 3 in particular could be finer, and
that is noted in [`decisions.md`](decisions.md) 5.15e.

## What the C looks like

Freeing a list of tasks means freeing each task and then the list. The
compiler writes one small function for each kind of list that owns its
items, the first time a program needs it:

```c
static void zeno_list_drop_1_Task(zeno_list *list) {
    if (list == NULL) return;
    for (int64_t i = 0; i < list->len; i++) zeno_drop_Task(((z_Task *)list->items[i].p));
    zeno_list_drop(list);
}
```

The `1` is how deep the lists go. A `list[list[Task]]` gets
`zeno_list_drop_2_Task`, which calls the one above for each row. A task
frees what it holds in the same way, and so on down:

```c
static void zeno_drop_Task(z_Task *self) {
    if (self == NULL) return;
    zeno_text_drop(self->z_name);
    zeno_list_drop(self->z_runs);
    zeno_free(self);
}

static void zeno_drop_Scheduler(z_Scheduler *self) {
    if (self == NULL) return;
    zeno_list_drop_1_Task(self->z_tasks);
    zeno_free(self);
}
```

So the one line `zeno_drop_Scheduler(z_scheduler);` at the end of the
program gives back the scheduler, its list, every task, every task's
name and every task's list of runs.

**A new item frees the old one.** `scheduler.tasks[1] = Task("sample
fast", 1, [])`:

```c
z_Task *t9 = zeno_new_Task("sample fast", 1, zeno_list_new(0));
zeno_slot *t10 = zeno_list_at(z_scheduler->z_tasks, 1, 91, 1);
zeno_drop_Task(((z_Task *)(*t10).p));
*t10 = (zeno_slot){.p = t9};
```

The new task is made first. If making it failed, the old one would
still be in its place.

**The copy a loop walks is freed alone.** `for task in scheduler.tasks`:

```c
zeno_list *t3 = zeno_list_copy(z_scheduler->z_tasks);
for (int64_t t4 = 0; t4 < t3->len; t4++) {
    z_Task *z_task = ((z_Task *)t3->items[t4].p);
    ...
}
zeno_list_drop(t3);
```

`zeno_list_drop`, not `zeno_list_drop_1_Task`: the copy holds the same
tasks as the real list, and they are the real list's to free.

**Giving away empties the owner.** `scheduler.add(slow)`:

```c
z_Task *t1 = z_slow;
z_slow = NULL;
z_Scheduler_add(z_scheduler, t1);
```

and the `zeno_drop_Task(z_slow);` at the end of the program finds
nothing to free. That part is unchanged from [`ownership.md`](ownership.md).

## An assignment can reach through a list

`points[1].x = 30` and `grid[1][0] = 5` used to be refused by
`zeno build` (not by `zeno run`). Lists of objects make them everyday
lines, so the C backend compiles them now, in every build. The value is
worked out first, then the list or object, then the index, which is
the order `zeno run` uses. `an_assignment_can_reach_through_a_list` in
`emit_c.rs` checks it with functions that print when they are called.

## Errors

The `except` block that catches an error owns it, and frees it when the
block ends ([`no-collector.md`](no-collector.md), "Errors"). That used
to be said by the compiler in its own words. It is the checker's now,
because it is the same rule as a loop's item: `e` is a look.

```zeno
try { print(risky(5)) } except Bad as e { let again = e }
```

```
error: `e` is the error that this `except` caught. It is freed when the
block ends, so it cannot be kept, returned or given away. Copy out what
is needed from it
```

One thing got wider: `e` can be lent to a `borrow` parameter now.

## The same, through LLVM

```sh
cargo run -- build --llvm --no-collector --memory examples/scheduler.zeno
```

The checker does all the deciding, so the LLVM backend needed only the
frees. `drop_list` writes the IR for `zeno_list_drop_1_Task` the first
time it is needed: a loop over the items that calls `@zeno.drop.Task`
for each, then `@zeno_list_drop` for the list. Replacing an item reads
as it does in C (this is `points[0] = Point(3)`, from the test
`the_ir_for_a_list_that_owns_its_items`):

```llvm
%t9 = call ptr @zeno_list_at(ptr %t7, i64 0, i32 5, i32 1)
%t10 = load ptr, ptr %t9
call void @zeno.drop.Point(ptr %t10)
store i64 %t8, ptr %t9
```

Both builds of the scheduler print the same and give the same memory
report, line for line.

## What is still refused

| Refused | Why | When |
|---|---|---|
| A list or object inside an `or none`: `var last: Task or none` | taking it out would have to leave a `none` behind | step 4 |
| An enum case that carries values, a list of `int or none` | each sits in a block of its own | step 4 |
| A function written where a value goes | so does it, with the variables it uses | step 5 |
| A function type that mentions a list or an object | the type cannot say `borrow` yet | step 5 |
| An error that is kept or raised again | its `except` frees it | step 5 |
| A function that uses a list or object declared at the top of the file | the owner is the top of the file, which could give it away first | noted, see below |
| A method that pops an item or replaces a field that has one owner | `self` is a borrow (rule 1) | noted |
| A list or object as the `T` of a generic | the copy for `T` would have to know it moves | noted |

The sixth row is the one a firmware program will meet first: a table of
numbers at the top of the file that several functions read. For now,
lend it through a `borrow` parameter. A top-level `let` that is never
given away could be lent to every function without being asked, and
that is on the list for the audit that follows these five steps.

## How it is tested

- **The checker** (`checker.rs`): one test for each rule on this page,
  for what it accepts and for the message of what it refuses. They are
  the tests whose names begin `with_one_owner_`, and
  `an_owned_list_can_hold_values_that_have_one_owner`,
  `an_item_that_has_one_owner_stays_in_its_list` and
  `only_the_owner_takes_out_what_has_one_owner`.
- **Both compilers**: three programs with nested lists and objects, with
  errors raised half way through a list, and with every early way out of
  a loop, plus `scheduler.zeno`. Each must print what `zeno run` prints,
  end with 0 blocks left, and give the same memory report through C and
  through LLVM (`with_no_collector_a_list_and_an_object_need_no_word`,
  `a_list_owns_items_that_have_one_owner`).
- **Over time**: a loop that fills and empties a list of objects 50 times
  and 5000 times needs the same pool
  (`with_no_collector_lists_of_objects_do_not_fill_the_pool_over_time`).
- **The table of refusals** (`what_a_build_with_no_collector_refuses_says_so`)
  lost the rows that build now, and its program of accepted lines grew
  by as many lines.
- By hand: every program was also run with each freed block overwritten,
  so that a look at freed memory would print rubbish. None did.

## Things to try

1. In `scheduler.zeno`, take the word `borrow` off `report` and run
   `zeno check --no-collector`. Read the error. Why is the loop's `task`
   not the loop's to give?
2. Add `let first = scheduler.tasks[0]` and read the error. Fix it two
   ways: once so that the task stays in the list, once so that it
   leaves.
3. Make a second list, `var done: list[Task] = []`, and move the last
   task across with `done.append(scheduler.tasks.pop())`. That builds.
   Now try `scheduler.tasks.append(scheduler.tasks.pop())`. Which rule
   refuses it, and is the program it refuses really unsafe?
4. Build the scheduler and open `scheduler.c`. Before you search for it,
   write down what `zeno_list_drop_2_Task` must look like. Then add
   `let rows = [[Task("x", 1, [])]]` to the program and compare.
5. Give `Scheduler` a method `remove_last(self) -> Task` that pops.
   Rule 1 refuses it. Rust would have you write `&mut self` to promise
   that nobody else is looking. What would Zeno need: a new word, or a
   rule about who may be lent a scheduler while a method runs? Write the
   error message you would want to read first.
6. `for task in scheduler.tasks` copies the list before walking it, so
   that an `append` in the loop cannot disturb the walk. When the body
   never appends, the copy is wasted. Find `open_list_loop` in
   `emit_c.rs` and decide what the checker would have to tell it.
7. Build `examples/tour.zeno` with `--no-collector` and read what it is
   refused for. That is step 4.
