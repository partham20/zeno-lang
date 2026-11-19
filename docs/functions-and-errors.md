# Functions and errors, with one owner

A build with no collector used to refuse two more everyday things: a
function written where a value goes (`function(n: int) -> int { ... }`),
and an error that is kept or raised again. Both build now, and with
them every value in the language has one owner in this build.

```sh
cargo run -- build --no-collector --memory examples/handlers.zeno
./handlers
```

```
35 -> 0
60 -> 40
  passing on: temp read 160
70 -> 60
  passing on: temp read 180
2 faults kept
   temp read 160
   temp read 180
0
995
[130, 150] [3, 4]
2
refused: no such door
memory: 40 blocks made (1208 bytes), at most 436 bytes in use at once, no collector, 0 blocks left at the end
pool: 960 bytes taken, in a pool of 32768
text: at most 49 bytes in use at once, in a room of 1024
```

[`examples/handlers.zeno`](../examples/handlers.zeno) sends raw sensor
readings through a chain of steps. Each step is a function value, kept
in a list. A reading that comes out too high raises an error, and the
error is kept in a log. It ends with **0 blocks left**, and `zeno run`
prints the same thirteen lines.

This page follows on from [`enums-and-none.md`](enums-and-none.md). It
is the last of five steps that take this build to "whoever receives a
value owns it".

## The files

- [`src/checker.rs`](../src/checker.rs): `an_error`, the `Raise` and
  `try_statement` arms, `CallableType.owned`, `Capture`, the
  `ExprKind::Function` arm, and `read`, which decides what a function
  may use from outside it.
- [`src/emit_c.rs`](../src/emit_c.rs): `try_with_values` (the `except`
  name owns the error) and `closure` (the block, the kept text and the
  function that frees them).
- [`src/emit_llvm.rs`](../src/emit_llvm.rs): the same in IR.
- [`src/runtime.c`](../src/runtime.c): `zeno_closure`,
  `zeno_closure_drop` and `zeno_closure_release`.

## An error is a value like any other

Until now an error lived by a rule of its own: the `except` that caught
it freed it, so it could be read there and nothing else. The rule now
is the one every other value follows.

- `raise fault` **takes** `fault`, as a parameter that says `take` would.
- The `except` that catches it receives it. The name after `as`
  **owns** it.
- When the block ends, the name frees the error, unless the error went
  somewhere else first.

So all of this is allowed:

```zeno
let faults: list[OutOfRange] = []
for raw in [35, 60, 120, 70, 130] {
    try {
        print(raw, "->", checked(temperature, raw))
    } except OutOfRange as fault {
        faults.append(fault)              # kept: the list owns the error now
    }
}
```

```zeno
function checked(channel: Channel, raw: int) -> int raises OutOfRange {
    try {
        return channel.process(raw)
    } except OutOfRange as fault {
        print("  passing on:", describe(fault))
        raise fault                       # the same error goes on up
    }
}
```

An error can be returned (`-> OutOfRange or none`), stored in a field,
lent (`fault: OutOfRange`), and made before it is needed:

```zeno
function open(door: int, problem: take ValueError) -> int raises ValueError {
    if door > 3 {
        raise problem
    }
    return door
}
```

`ValueError("not raised")` is made, handed to `open`, never raised, and
freed when `open` returns, because `problem` owned it: the parameter
says `take`, and `raise` needs an owner.

The rules that go with owning are the ones you know from lists:

```zeno
} except OutOfRange as fault {
    faults.append(fault)
    print(fault.value)
}
```

```
error: `fault` was given away on line 2 and holds nothing now. Use it before it goes, or give it only to what looks: a parameter without `take`
```

and a look cannot be raised, because `raise` takes:

```
error: `e` is only looked at here: its owner still has it. So it cannot be kept, returned or given away. If it is a parameter, write `take` in front of its type to take the value over
```

### In C

A raised error travels in a small wrapper (`zeno_error`) that holds the
kind, the message and, for an error that is an object of a class, the
object. The `except` takes the object out of the wrapper, so that the
object can go on living after the wrapper is freed (the comments are
added here):

```c
if (zeno_is(zeno_caught_1, "OutOfRange")) {
    z_OutOfRange *z_fault = (z_OutOfRange *)zeno_caught_1->object;
    zeno_caught_1->object = NULL;          /* the name owns it now */
    z_OutOfRange *t2 = z_fault;
    z_fault = NULL;                        /* given to the list */
    zeno_list_append(z_faults, (zeno_slot){.p = t2});
    zeno_drop_OutOfRange(z_fault);         /* frees nothing: it is NULL */
    zeno_error_drop(zeno_caught_1);        /* frees the wrapper only */
}
```

If the block had only printed the error, `z_fault` would still hold it
at the end, and `zeno_drop_OutOfRange` would free it. One line of C
does the right thing either way, because a name that gave its value
away holds NULL.

`raise fault` puts the object in a new wrapper and returns:

```c
z_OutOfRange *t5 = z2_fault;
z2_fault = NULL;
z_OutOfRange *t6 = t5;
zeno_note(zeno_error_of("OutOfRange", zeno_show_OutOfRange(t6), t6, zeno_drop_any_OutOfRange), 15, 9);
{ zeno_drop_OutOfRange(z2_fault); zeno_error_drop(zeno_caught_2); return false; }
```

An error of a kind Zeno knows (`ValueError`, `IndexError` and so on)
has no object. The wrapper is the error, and the name owns the wrapper.

## A function value has one owner

There are two kinds of function value, and they are freed differently.

**A function of the program**, used by its name:

```zeno
function not_below_zero(n: int) -> int {
    if n < 0 { return 0 }
    return n
}
temperature.add(not_below_zero)
```

It is not in a block. It is one fixed value that is there for as long
as the program runs, like an enum case that carries nothing. It can be
handed out any number of times.

**A function written where a value goes:**

```zeno
function scale_by(factor: int) -> function(int) -> int {
    return function(n: int) -> int { return n * factor }
}
```

This one has to remember `factor`, so it is a small block: the code,
and a copy of `factor`. A block needs an owner. Here the owner is
whoever calls `scale_by`, then the parameter `step` of `add`, then the
list `steps`:

```zeno
temperature.add(scale_by(2))
```

A variable whose type is a function type can hold either kind, and it
is an owner:

```zeno
var alarm: function(int) -> int = not_below_zero
print(alarm(-5))
alarm = offset_by(1000)       # frees the old value (which does nothing here)
print(alarm(-5))              # and the end of the program frees this one
```

```zeno
let add = make_adder(2)
let other = add
print(add(1))
```

```
error: `add` was given away on line 2 and holds nothing now. Use it before it goes, or give it only to what looks: a parameter without `take`
```

Lending is written as for a list, with the type alone:
`f: function(int) -> int`. A parameter that keeps the function says
`take`, as `add(self, step: take function(int) -> int)` does in
`handlers.zeno`.

## What a function may use from outside it

In `zeno run` a function written inside another can use anything it can
see. With no collector, what it may use depends on what the thing is:

| It uses | What happens |
|---|---|
| a `let` number, bool or plain enum | copied into the block |
| a `let` text | a copy of the text is kept in the block, and freed with it |
| a `var` of the function around it | refused |
| anything at the top of the file that is a number, bool, text or function | used where it is |
| a list or an object that a `let` at the top of the file holds | looked at where it is, for as long as the function likes ([`audit.md`](audit.md)) |
| any other list, object, error or function value | looked at, for one line (next section) |

The `var` is refused because the two functions would share it:

```zeno
function counter() -> function() -> int {
    var count = 0
    return function() -> int {
        count += 1
        return count
    }
}
```

```
error: `count` is a `var` of the function around this one, so the two functions would share it, and with no collector nothing is shared. If it does not change, declare it with `let`. If it does, keep it in an object, which a function written here can look at and change
```

In `zeno run`, `count` lives on after `counter` returns, because the
inner function still holds it. That takes a block with two holders.
Copying `count` instead would build, and then print something different
from `zeno run`. Zeno never does that, so it says no and tells you what
to write. A counter that must be kept is a class with a method.

## A function that looks lives for one line

```zeno
let over = [90, 130, 150].filter(function(n: int) -> bool { return n > temperature.limit })
print(over, [1, 2].map(function(n: int) -> int { return n + len(faults) }))
```

The first function uses `temperature`, an object. The second uses
`faults`, a list. Neither takes it: taking would empty the variable
without a word. They **look**, and a look has to be gone before the
owner is. The simplest way to be sure of that is this rule:

> A function that looks at a value from outside belongs to the
> statement that writes it.

It is made, used and freed on that one line. It can go straight into
`map`, `filter` and `reduce`, or into a parameter that looks.

**A function that is kept takes what it uses** (since 0.8.0). Put in a
variable or a field, returned, or given to a parameter that says
`take`, it takes each value with one owner that it uses, as `let b = a`
would, and frees them when it is freed:

```zeno
let xs = [1, 2]
let f = function(n: int) -> int { return n + len(xs) }
print(f(1))          # 3; `xs` holds nothing from the line above on
```

[`kept-functions.md`](kept-functions.md) walks through it.

It can change what it looks at (`counter.count += 1`), as any parameter
that looks can. It cannot take an item out of it or give it away, and
the line that writes the function cannot give the value away either.

## A function type says what each parameter does

Before this step a function type could not mention a list or an object
at all. Now each parameter of a function type is read as a parameter:

```zeno
function measure(with: function(list[int]) -> int, data: list[int]) -> int {
    return with(data)                 # lends `data` to whatever `with` is
}

function consume(with: function(take list[int]) -> int, n: int) -> int {
    return with([n, n + 1])           # gives the list away
}

function build(with: function(int) -> list[int], n: int) -> int {
    let made = with(n)                # `made` owns what comes back
    return len(made)
}
```

A function fits a type only if it does the same with each parameter:

```
error: `g` of `f` should be function(take list[int]) -> int, but the parameter `xs` of this function only looks at what it is given (`list[int]`)
```

Fields and variables can have such types too. `handlers.zeno` has
`steps: list[function(int) -> int]`.

## What the C looks like

The runtime's function value, with no collector:

```c
typedef struct zeno_closure {
    zeno_code code;
    const char *name; /* for `print`; empty when the function has no name */
    /* What frees the block this is in. NULL for a function of the program,
     * which is not in a block. */
    void (*drop)(struct zeno_closure *);
} zeno_closure;

static void zeno_closure_drop(zeno_closure *function) {
    if (function == NULL || function->drop == NULL) return;
    function->drop(function);
}
```

The `drop` entry is only there with no collector. Take this program:

```zeno
function make_tagger(tag: str, width: int) -> function(str) -> str {
    return function(text: str) -> str { return f"[{tag}:{width}] {text}" }
}
function double(n: int) -> int { return n * 2 }
var tagger = make_tagger("warn", 4)
tagger = make_tagger("note", 2)
var current: function(int) -> int = double
current = function(n: int) -> int { return n + 1 }
```

The literal in `make_tagger` uses a text and a number. Its block is the
function value followed by both, and it comes with a function that
frees the text and then the block:

```c
struct zeno_env_2 {
    zeno_closure base;
    const char *z_tag;
    int64_t z_width;
};

static void zeno_fn_drop_2(zeno_closure *zeno_self) {
    struct zeno_env_2 *env = (struct zeno_env_2 *)zeno_self;
    zeno_text_drop(env->z_tag);
    zeno_free(env);
}

static zeno_closure *z_make_tagger(const char *z_tag, int64_t z_width) {
    struct zeno_env_2 *t5 = (struct zeno_env_2 *)zeno_alloc(sizeof(struct zeno_env_2));
    t5->base.code = (zeno_code)zeno_fn_2;
    t5->base.name = "";
    t5->base.drop = zeno_fn_drop_2;
    t5->z_tag = zeno_text_keep(z_tag);
    t5->z_width = z_width;
    return (&t5->base);
}
```

`double` is a fixed value. Its `drop` is left out, which in C means
NULL:

```c
static zeno_closure zeno_value_double = {(zeno_code)zeno_wrap_double, "double"};
```

And the program. Every assignment frees the old value through the same
call, whichever kind it is (the two comments are added here):

```c
z_tagger = z_make_tagger("warn", 4);
zeno_closure *t1 = z_make_tagger("note", 2);
zeno_closure_drop(z_tagger);
z_tagger = t1;
z_current = (&zeno_value_double);
zeno_closure *t2 = (zeno_closure *)zeno_alloc(sizeof(zeno_closure));
t2->code = (zeno_code)zeno_fn_1;
t2->name = "";
t2->drop = zeno_closure_release;
zeno_closure *t3 = t2;
zeno_closure_drop(z_current);          /* `double`: nothing to free */
z_current = t3;
zeno_closure_drop(z_current);
zeno_closure_drop(z_tagger);
```

A literal that keeps no text uses `zeno_closure_release`, which only
frees the block. A literal that uses nothing from outside still gets a
block of its own, so that there is one rule for every literal.

## In every build

Function types whose parameters look at a value or `take` it work with
a collector too, for an `owned class` and an `owned list`:

```zeno
owned class Packet {
    id: int
    data: owned list[int]
}
function twice(with: function(Packet) -> int, p: Packet) -> int { return with(p) + with(p) }
function feed(with: function(take Packet) -> int, n: int) -> int { return with(make(n)) }
```

There the function values themselves are shared and the collector's,
as before, and each packet still has its one owner. In the test for
this, 3600 blocks are freed by their owners and the collector never has
to run.

Errors and function values that capture are unchanged in a build with a
collector: they are shared, and can be kept freely.

## The same, through LLVM

```sh
cargo run -- build --llvm --no-collector --memory examples/handlers.zeno
```

It prints the same and gives the same three lines of memory report. In
IR a function value is `{ ptr, ptr, ptr }` (code, name, drop) followed
by what it uses:

```llvm
@zeno.value.double = internal constant { ptr, ptr, ptr } { ptr @zeno.wrap.double, ptr @.str.3, ptr null }

define internal void @zeno.fn.drop.2(ptr %self) {
entry:
  %t1 = getelementptr { ptr, ptr, ptr, ptr, i64 }, ptr %self, i32 0, i32 3
  %t2 = load ptr, ptr %t1
  call void @zeno_text_drop(ptr %t2)
  call void @zeno_free(ptr %self)
  ret void
}
```

With a collector the value is `{ ptr, ptr }`, as it was.

## What is still refused

| Refused | Why | When |
|---|---|---|
| A function that uses a `var` of the function around it | the two would share it | by design. Use `let`, an object, or a class |
| Keeping a function that looks at a list or an object | it must be gone before the owner is | by design. Give it a parameter |
| ~~A function that uses another function value and is then returned (`compose`)~~ | builds since 0.8.0: a function that is kept takes what it uses ([`kept-functions.md`](kept-functions.md)) | done in 9.2 |
| A list or an object as what a type parameter stands for: `same[T](x: T)` | the copy for `T` would have to know it moves | noted |
| A list of numbers that may be `none` | see [`enums-and-none.md`](enums-and-none.md) | noted |
| ~~A function that can raise, used as a value~~ | builds since 0.8.0: a function type says what it raises ([`raises-in-types.md`](raises-in-types.md)) | done in 9.1 |
| A method used as a value: `let m = counter.bump` | it would be a look at `counter` that is kept | call it there |

When this step was built a plain parameter took and the word `borrow`
marked one that looked. The audit turned that round: a plain parameter
**looks**, and `take` marks the one that takes
([`audit.md`](audit.md), and 5.19 in [`decisions.md`](decisions.md)).
This page shows the language as it is now.

## How it is tested

- **The checker** (`checker.rs`):
  `with_one_owner_an_except_owns_the_error_it_catches` and
  `with_one_owner_a_function_value_has_one`: what each rule on this
  page accepts, and the message of what it refuses.
  `shared_things_cannot_hold_owned_ones` has the function types.
- **Both compilers** (`emit_c.rs`, `emit_llvm.rs`):
  `with_no_collector_an_error_and_a_function_value_have_one_owner` runs
  three programs and `handlers.zeno`. They keep errors in variables,
  fields and lists, raise them again, return them and make them early.
  They keep functions in variables, fields and lists, replace them, pop
  them, and leave loops and functions early while holding them. Each
  must print what `zeno run` prints, end with 0 blocks left, and give
  the same memory report through C and through LLVM.
- **In every build**: `a_function_type_says_who_owns_what_in_every_build`.
- **Over time**: a loop that makes and replaces functions and catches,
  raises again and keeps errors needs the same pool for 50 trips and
  for 5000
  (`with_no_collector_errors_and_function_values_do_not_fill_the_pool_over_time`).
- **The generated code**: `the_c_for_a_function_value_that_has_one_owner`
  and `the_ir_for_a_function_value_that_has_one_owner` compare the
  compiler's output with what this page shows.
- **What is refused**: `what_a_build_with_no_collector_refuses_says_so`
  has a row for each refusal, and checks that LLVM refuses in the same
  words.
- By hand: every program was also run with each freed block
  overwritten, so that a look at freed memory would print rubbish. None
  did, in either backend.

## Things to try

1. In `handlers.zeno`, add a fourth step that halves the value, written
   as a literal: `temperature.add(function(n: int) -> int { return n // 2 })`.
   Build with `--memory`. The program makes one more function, yet
   fewer blocks than before (33, down from 40). Which blocks are no
   longer made?
2. Change `faults.append(fault)` to `print(describe(fault))`. Build
   again and open `handlers.c`. Which line frees the error now? Which
   line freed it before?
3. Write `function last_fault() -> OutOfRange or none` that tries one
   reading and returns the error it catches. Who owns the error after
   the call? Check your answer with `--memory`.
4. Write the `counter` from this page with a class `Counter` and a
   method `next`. Then pass
   `function(n: int) -> int { return n + counter.next() }` to `map`.
   Why is this one allowed when the `var` version was not?
5. `var alarm: function(int) -> int = not_below_zero` can be given
   another function later. Take the type away (`var alarm =
   not_below_zero`), assign `offset_by(1)` to it and read the message.
   Should Zeno work the type out here? Write down what could go wrong
   if it did.
6. Open `handlers.c` and find `zeno_fn_drop_`. There is none, yet
   `scale_by` makes a block. Why does it not need one? Change `scale_by`
   to also use a text and look again.
7. `compose(f, g)` returns a function that calls both. Since 0.8.0 it
   builds: a function that is kept takes what it uses. Rust would write
   `move |n| g(f(n))`. Why does Zeno need no word there? Build it with
   `--no-collector` and find, in the C, the line that frees `f`.
