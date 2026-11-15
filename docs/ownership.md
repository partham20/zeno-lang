# Owned values: memory freed at a line you can point to

Until this stage, every list and object in a compiled Zeno program was
**shared**. `let b = a` makes a second name for the same list, and a garbage
collector frees the list some time after the last name is gone
([`memory.md`](memory.md)).

That is easy to write, and for most code it is the right trade. It has two
costs. The collector pauses the program when it runs, and memory comes back
at no particular moment. A buffer filled a thousand times a second inside a
control loop wants neither.

So a value can now be marked **owned**. An owned value has exactly one
owner, and is freed the moment its owner's block ends:

```sh
cargo run -- run examples/buffer.zeno
cargo run -- build --memory examples/buffer.zeno && ./buffer
```

```
...
memory: 1806 blocks made (2646705 bytes), at most 15870 bytes in use at once, 0 collections, 1625 blocks freed without the collector
```

That program makes 2.6 megabytes of buffers, never holds more than 16
kilobytes, and the collector never runs.

Shared values are still the default. Nothing changes for a program that
does not say `owned`.

## The files

- [`src/checker.rs`](../src/checker.rs), the section headed "Owned
  values": `read`, `take`, `look` and `leave_loop`. This is where the
  rules live.
- [`src/emit_c.rs`](../src/emit_c.rs), the section headed "Owned values":
  `frees`, `free_owned` and `end_of_block`, and a few lines each in `let`,
  `return`, `break`, `raise` and assignment.
- [`src/runtime.c`](../src/runtime.c): `zeno_free` and `zeno_list_drop`,
  about 25 lines.
- [`examples/buffer.zeno`](../examples/buffer.zeno): the tour.

`zeno run` reads the new words and otherwise ignores them. The interpreter
is a Rust program and its values are freed for it. The checker runs first
either way, so the rules are enforced however the program is run.

## Three new words

```
owned class Frame {                 # every Frame has one owner
    channel: int
    samples: owned list[int]        # this list belongs to the frame
}

let window: owned list[int] = [0, 0, 0, 0]

function average(samples: owned list[int]) -> int { ... }    # looks
function send(frame: take Frame) -> int { ... }              # takes
```

- **`owned class`**: every object of the class has exactly one owner. That
  is decided once, where the class is declared, and not at each use.
- **`owned list[T]`**: a list with exactly one owner. Its items are
  ordinary values (numbers, text, shared objects).
- **`take`**, on a parameter: the function takes the value over. A
  parameter without the word only looks: the function may read the value
  and change what is inside it, and may not keep it.

`owned` and `take` are reserved words, like every word of the language.
A word is never also a name, so nobody has to work out which one a line
means. (The word for looking used to be `borrow`, and a plain parameter
took. The audit turned that round: [`audit.md`](audit.md).)

## The rules

**1. One owner.** A variable, a parameter, or a field of an owned object
holds the value. Nothing else does.

**2. Storing it somewhere gives it away.** Assigning it to another variable,
passing it to a parameter that says `take`, putting it in a field, or returning
it, moves it. The old name holds nothing afterwards, and using it is an
error:

```
let frame = capture(7, 4)
print(send(frame))        # `frame` moves into `send`
print(frame.channel)
```

```
error: `frame` was given away on line 2 and holds nothing now. Use it
before it goes, or give it only to what looks: a parameter without
`take`
```

**3. Looking is free.** Reading a field, calling a method, printing,
indexing, comparing, looping over it: none of these moves anything.

**4. A parameter looks, unless it says `take`.** The caller keeps the
value. What is only looked at cannot be stored, returned or passed on to
something that keeps it, so it can never outlive the call:

```
function keep(p: Packet) -> Packet { return p }
```

```
error: `p` is only looked at here: its owner still has it. So it cannot
be kept, returned or given away. If it is a parameter, write `take` in
front of its type to take the value over
```

A parameter written `owned list[int]` looks at any list, owned or
shared. Anyone may look at a shared one. A parameter written plain
`list[int]` is the shared list of every earlier page, which the function
may keep, so an owned list cannot go there:

```
error: `xs` of `total` is written `list[int]`, which is a shared list
that the function may keep, and this list has one owner. Write the
parameter as `owned list[int]`: then it only looks, and any list can be
passed to it
```

**5. It is freed where its owner's block ends.** That is the closing `}`,
or a `return`, `break` or `continue` that leaves the block early. A `var`
that is given a new value frees the old one at that line.

**6. Owned may hold shared. Shared may not hold owned.** A shared list can
be reached from many places. If it held an owned value, each of those
places would think the value was still there after its owner had freed it.
So a shared list and an ordinary class cannot hold an owned value. An
owned class can, and so can an owned list: it owns its items and frees
them with itself ([`one-owner.md`](one-owner.md)). An `or none` can too:
`var last: Packet or none` owns a packet or nothing
([`enums-and-none.md`](enums-and-none.md)).

That is the whole of it. There are no lifetimes to write and no `&`.

## How the checker knows

The checker already works out a type for every expression. An owned value
gets three ways of being used, and each is one function:

- **`take`** is called wherever a value is about to be kept: the right side
  of `let` and `=`, an argument for a parameter that takes it, a `return`.
  If the expression is the name of an owner, the checker notes that the
  owner is now empty, and where that happened.
- **`look`** is called wherever a value is only read. It changes nothing.
- **`expr`**, the ordinary function, is what everything else calls. For the
  name of an owner it answers with a special type, `Borrowed`, that fits
  nowhere.

The third one is the safety net. Suppose a later stage adds a new place
where values can go, and nobody remembers ownership. The value arrives
there as `Borrowed`, fits nothing, and the programmer gets an error
message. The alternative mistake, a second owner that nobody noticed, would
be a crash in the compiled program. The design makes the forgetful case the
safe one.

### Which owners are empty

The checker keeps one set: the owners that hold nothing at this point in
the code. Reading an owner that is in the set is the error above. The work
is in keeping the set right as the code branches:

- **`if` and `match`**: each branch starts from the same set. Afterwards the
  set is everything any branch gave away. A branch that ends in `return`
  is left out, because what follows the `if` never runs after it:

  ```
  if not ok {
      discard(frame)
      return
  }
  send(frame)            # fine: this line is only reached with the frame
  ```

- **Loops**: a value given away on one trip is not there on the next. So
  an owner declared outside a loop cannot be given away inside it, unless
  that path leaves the loop with `break`:

  ```
  error: `p` is given away inside a loop, and the loop may come round again
  with nothing in it. Give it away after the loop, or just before a `break`
  ```

- **`try`**: an error can leave the body after any line. So an `except`
  block starts with everything the body gives away already gone.

- **A `var` that is assigned** holds a value again, and leaves the set.

### Three smaller rules

- **One line cannot use an owner and give it away.**
  `print(look(p) + send(p))` is refused. It keeps the compiled code simple
  (nothing has to be saved before the move) and it is rarely what anyone
  meant. Put the look on a line of its own.
- **An owned value stays in the function whose block holds it.** A function
  cannot reach an owned variable declared somewhere else, and a function
  written inside another cannot carry one away. Pass it in as a
  parameter. This is what makes it safe to free the value when the block
  ends: nothing outside the block ever knew where it was. The one
  exception is a `let` at the top of the file. It is there until the
  program ends, so every function may look at it, and in return the top
  of the file may not give it away ([`audit.md`](audit.md)).
- **An owned value made in the middle of a statement, and only looked
  at, belongs to the statement.** In `print(look(make()))` nobody is
  given what `make()` returns, so the statement frees it when it is
  over. `let p = make()` gives it an owner that lasts longer. (An owned
  value made on a line by itself, and kept by nobody, is freed there in
  the same way.) [`temporaries.md`](temporaries.md) is the walkthrough.

## What the C looks like

An owned value is the same pointer in C that a shared one would be. All
that differs is who frees it. Take a small function:

```
function make(id: int, count: int) -> Packet {
    let data: owned list[int] = [n * id for n in range(count)]
    return Packet(id, data)
}
```

```c
static z_Packet *z_make(int64_t z_id, int64_t z_count) {
    zeno_list *t7 = zeno_list_new(0);
    ...the loop that fills t7...
    zeno_list *z_data = t7;
    zeno_list *t9 = z_data;
    z_data = NULL;                              // `data` gives its list away
    z_Packet *t10 = zeno_new_Packet(z_id, t9);
    zeno_list_drop(z_data);                     // end of block: nothing left
    return t10;
}
```

Two things to see.

**Giving away writes NULL.** The pointer is copied out and the variable is
set to NULL.

**The end of a block frees whatever is still there.** `zeno_list_drop`, and
the `zeno_drop_Packet` the compiler writes for each owned class, do nothing
when handed NULL. So the compiler does not need to know, at the end of a
block, which owners still hold something. It asks each one, and the pointer
itself is the answer. (Here the C compiler can see that `z_data` is NULL
and removes the call.)

A function that takes a value over (`p: take Packet`) is an owner like
any other:

```c
static int64_t z_send(z_Packet *z_p) {
    ...
    int64_t t13 = z_Packet_size(z_p);   // the answer is worked out first
    zeno_drop_Packet(z_p);              // then the packet is freed
    return t13;
}
```

A `continue` frees the blocks it leaves, and then the normal end of the
loop body frees the same variables for the trips that did not `continue`:

```c
for (int64_t z_i = 0; z_i < 3; z_i++) {
    z_Packet *z_p = z_make(zeno_add(z_i, 1, 51, 18), 3);
    if (z_i == 1) {
        z_Packet *t4 = z_p;
        z_p = NULL;
        zeno_print(1, zeno_str_int(z_send(t4)));
        zeno_drop_Packet(z_p);
        continue;
    }
    zeno_print(1, z_describe(z_p));
    zeno_drop_Packet(z_p);
}
```

And an object of an owned class frees what its fields own, then itself:

```c
static void zeno_drop_Packet(z_Packet *self) {
    if (self == NULL) return;
    zeno_list_drop(self->z_data);
    zeno_free(self);
}
```

## Freeing one block

The collector keeps a table of every block ([`memory.md`](memory.md)).
`zeno_free` has to take one block out of it:

```c
static void zeno_free(void *memory) {
    if (memory == NULL) return;
    zeno_block *block = (zeno_block *)memory - 1;
    zeno_block *last = zeno_blocks[--zeno_block_count];
    last->link.place = block->link.place;
    zeno_blocks[block->link.place] = last;
    zeno_in_use -= block->size;
    free(block);
}
```

Each block remembers its place in the table, so nothing is searched: the
last entry of the table moves into the gap. The cost is the same whether
the program has ten blocks or a million.

Where does a block keep its place? In the field the collector uses for its
waiting list. The two are never needed at once. A collection sorts the
table anyway, and writes every place afresh as it sweeps.

**An owned block is still in the collector's table while it lives.** That
matters twice:

- The collector looks inside it, as it must: an owned list may hold the
  only pointer to a shared string.
- If a free is ever skipped, nothing leaks. The block is still in the
  table, nothing points at it, and the next collection frees it. The
  collector is the net under the trapeze.

## Errors

When does a free get skipped? When an error jumps over it.

In an **ordinary build**, `raise` is a `longjmp` ([`compiler.md`](compiler.md),
"Errors"). The function that raises frees its own owned values first, since
it knows it is leaving. But the jump then passes straight through every
function between it and the `try`, and none of them gets a chance to free
anything. What they owned is left for the collector.

There is one more trap, and C sets it. Look at this:

```
let b = fresh("b", n)
try {
    let got = swallow(b, fail)     # takes `b`, frees it, then raises
    ...
} except Refused as r { ... }
```

`b` is set to NULL before the call. Then `swallow` raises and the `longjmp`
lands back in this function. C says that a local variable changed since the
`setjmp` may have **either** value after the jump, unless it is declared
`volatile`. If `b` came back as the old pointer, the end of the block would
free a buffer that `swallow` already freed. So in a function with a `try`,
every owner is declared `volatile`:

```c
z_Buf *volatile z_b = z_fresh(...);
```

In a **firmware build** ([`firmware.md`](firmware.md)) none of this arises.
An error is a return value, every caller tests it, and the test is one more
place where a block ends:

```c
if (!z_check(z_frame, &t1)) { zeno_drop_Frame(z_frame); return false; }
```

Every function on the error's way frees what it owns, so nothing is left
for the collector to find. That is what lets `zeno build --no-collector`
leave the collector out: see [`no-collector.md`](no-collector.md).

## Two frees that every program gets

While building this, two places turned up where a shared block is certain
to be garbage, and both now free it at once:

- **A list that grows.** `append` on a full list moves the items to a place
  twice the size. Only the list ever pointed at the old place.
- **The copy a `for` loop walks.** `for x in items` walks a copy of the
  list, so that the body can change `items` without confusing the loop
  (decision 3.10a). Nothing else ever sees that copy, so it is freed after
  the loop's `}`, and by a `return` or a failed call that leaves from
  inside the loop.

Neither needs the word `owned`. Run `examples/memory.zeno` with `--memory`
and the last number on the line counts them.

## The same, through LLVM

`zeno build --llvm` frees owned values at the same places. The pointer
lives in a slot (`alloca ptr`), giving it away is a `load` followed by
`store ptr null`, and each way out of a block loads the slot and calls
`@zeno_list_drop` or the class's `@zeno.drop.Packet`. [`llvm.md`](llvm.md),
"Owned values", shows the IR for a small program.

## What is refused for now

Each of these would need a rule the simple checker does not have. They are
errors today, with a message that says so, and can be allowed later without
breaking any program that works now.

- **An owned value inside an enum case in a build with a collector, or
  as what a type parameter stands for.** (An owned list can hold owned
  values, and so can an `or none`: see [`one-owner.md`](one-owner.md) and
  [`enums-and-none.md`](enums-and-none.md). With no collector an enum
  case can hold them as well, because there the enum value itself has
  one owner. A function type can mention one, and says what each
  parameter does with it, as in `function(take Packet) -> int`:
  [`functions-and-errors.md`](functions-and-errors.md).)
- **Moving a field out of an object** (`let d = packet.data`). Same reason.
- **Replacing an owned field through a look.** Somebody else may have
  been lent the old value at that moment. Only the owner of the object
  may write `packet.data = [...]`. A method may do it for its own object
  (`self.data = [...]`), and then only the owner of the object may call
  that method ([`audit.md`](audit.md)).
- **A shared list becoming an owned one.** Only a list that was just
  made can start life owned: one written out (`[1, 2]`), a
  comprehension, or what `split`, `map`, `filter` or `+` gives. Only
  those are certain to have no other name.
- **`match` on an owned value, raising one, or keeping a method of one as a
  value.**

## What this does not claim

- Text in an owned object or list is a shared value. The collector
  still frees it. Items that are objects of an owned class are the
  list's own, and are freed with it.
- An owned value is still taken from the heap with `malloc`. A build with
  no collector at all, for a program where everything is owned, is
  `zeno build --no-collector`: see [`no-collector.md`](no-collector.md).
  That build takes its blocks from a fixed pool ([`pool.md`](pool.md)).
- The checker for owners is small on purpose. It refuses some programs that
  would be safe, such as the one-line rule above. Rust accepts more, with
  far more machinery.

## How it is tested

- Some forty programs that break a rule, each checked for the exact message (`mod
  tests` in `checker.rs`, "Owned values").
- Four programs that use owned values in every way the compiler handles
  (early exits, errors caught and not caught, nested owned objects, function
  values, nested loops) are built both ways, ordinary and `--firmware`,
  with the collector forced to run before every allocation, and must print
  what `zeno run` prints.
- One test builds the same loop with shared lists and with owned lists, and
  checks that the owned one never needs the collector and stays under 8 KB.
- One test reads the generated C for the lines shown above, and one reads
  the LLVM IR. The LLVM build of the loop of owned lists must also free
  every block without the collector.
- While it was being built, the same programs were also run under GCC and
  Clang at several optimisation levels, and under AddressSanitizer, which
  reports any use of a block after it has been freed and any block freed
  twice.

## Things to try

1. Build `examples/buffer.zeno` with `--memory`, then change `owned class
   Frame` to `class Frame` and `owned list[int]` to `list[int]` and build
   again. Compare the two memory lines.
2. Take the `#` off the last line of `examples/buffer.zeno`. Then find the
   two lines in `read` (`checker.rs`) that produce the message.
3. Open `buffer.c` and find where `frame` is freed when the loop does
   `continue`, and where it is freed when it does not. Why are both needed?
4. Write a function `longest(a: owned list[int], b: owned list[int])` that
   returns the longer of the two. What does the checker say? Why must it?
   What could the function return instead?
5. In `leave_loop`, a value given away just before a `break` is allowed.
   Write a loop where that is exactly what you want.
6. Build a program with a `try` both ways and search the two C files for
   `volatile`. Then read the "Errors" section again.
7. `for x in items` copies the list. For an owned list that nobody else can
   change during the loop, is the copy needed? What would the checker have
   to know to leave it out?
8. An owned value can sit inside an `or none`
   ([`enums-and-none.md`](enums-and-none.md)). With
   `let p: Packet or none = none`, what does `let q = p` mean, and what
   does `p` hold afterwards? Try it, then read that page.
