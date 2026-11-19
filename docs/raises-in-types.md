# `raises` in a function type

A function type can say what a function of that type raises:

```zeno
function parse(text: str) -> int raises ValueError {
    if text == "x" {
        raise ValueError("not a number: x")
    }
    return int(text)
}

function double(n: int) -> int {
    return n * 2
}

function each(texts: list[str], convert: function(str) -> int raises ValueError) -> int raises ValueError {
    var total = 0
    for text in texts {
        total += convert(text)
    }
    return total
}

function twice(n: int, f: function(int) -> int raises ValueError) -> int raises ValueError {
    return f(f(n))
}

print(each(["1", "2", "3"], parse))
print(twice(5, double))
try {
    print(each(["4", "x"], parse))
} except ValueError as e {
    print("caught:", e)
}
print(["7", "8"].map(parse))
```

```sh
cargo run -- build --no-collector --memory types.zeno
./types
```

```
6
20
caught: not a number: x
[7, 8]
memory: 13 blocks made (304 bytes), at most 128 bytes in use at once, no collector, 0 blocks left at the end
pool: 280 bytes taken, in a pool of 32768
text: at most 13 bytes in use at once, in a room of 1024
```

`zeno run` prints the same four lines, and so does every build. Until
0.8.0 a firmware build, and a build with no collector, refused the
first line with "cannot compile a function that can raise, used as a
value, yet". The reasons for each choice on this page are in chapter
56 of [`decisions.md`](decisions.md). It is part 9.1 of
[`plan-9-14.md`](plan-9-14.md).

## The files

- [`src/parser.rs`](../src/parser.rs): `type_in` reads `raises` and
  the brackets, `result_type` reads a type after `->` (where `raises`
  belongs to the function), and `one_kind` gives the message for a
  second kind.
- [`src/checker.rs`](../src/checker.rs): `CallableType.raises`, the
  call through a value (`can_raise`), `raises_fit` (what fits a type,
  and what a firmware build refuses), `caught_by`, and
  `Types.made_raising`.
- [`src/emit_c.rs`](../src/emit_c.rs) and
  [`src/emit_llvm.rs`](../src/emit_llvm.rs): `raising`, `through`,
  `invoke`, and the two kinds of small function in `function_value`.
- [`tools/targets/edge-raises-in-types.zeno`](../tools/targets/edge-raises-in-types.zeno):
  the program every build and every target runs.

## How it is written

After what the function returns, `raises` and one kind of error, as
in a function's first line:

```zeno
let keep: function(str) -> int raises ValueError = parse
```

Three things follow from where the words stand.

**`raises` after a result belongs to the outermost function.** In
`function make() -> function(int) -> int raises Missing`, it is
`make` that raises `Missing`, as it always was. A function type that
is the result and raises stands in brackets:

```zeno
function make(n: int) -> (function(int) -> int raises ValueError) raises Missing {
    if n < 0 {
        raise Missing("n")
    }
    return function(x: int) -> int raises ValueError {
        if x == 0 {
            raise ValueError("zero")
        }
        return x + n
    }
}
```

Brackets go around a function type and nothing else. They also say
what could not be said before: `(function(int) -> int) or none` is a
function that may be missing, and `function(int) -> int or none` is a
function whose answer may be. After `raises` they are needed:
`function() raises ValueError or none` could be read as raising
`ValueError` or nothing, so it is refused, and the message writes
`(function() raises ValueError) or none`.

**One kind.** A function that raises two kinds fits `raises Error`. A
type ends at the comma, so in `convert: function() -> int raises
ValueError, text` the word `text` is a second parameter. A second
kind of error there is named:

```
error: a function type names one kind of error after `raises`. A function that raises more than one kind fits `raises Error`, which catches every kind
```

**A call through the value raises that kind**, and is checked like a
call of `parse` by its name: catch it, or say `raises`.

## What fits

A function fits `raises K` if `except K` would catch everything it
raises: it raises `K`, or nothing, or the type says `raises Error`.

```
error: `convert` of `each` should be function(str) -> int raises ValueError, but this function can raise Missing, and the type says it raises only ValueError. Write `raises Error` on the type to take a function that raises any kind
```

Where the type says nothing, an ordinary build and `zeno run` do as
they always did: a function that raises is let in, and the call is
checked as raising nothing (decision 56.4 counts how many programs
that touches: none that a strict check accepts). A firmware build,
and a build with no collector, hold it:

```
error: `f` of `apply` should be function(str) -> int, which raises nothing, but this function can raise ValueError. In a firmware build a call through a function type raises only what the type says: write `raises ValueError` after what it returns, or build without `--firmware`
```

## In a firmware build

There an error travels as a value. A function that can raise answers
`true` or `false`, and gives its answer through a pointer (see
[`firmware.md`](firmware.md)). A call through a value of a raising
type is that call. In `each`, built with no collector:

```c
static bool z_each(zeno_list *z_texts, zeno_closure *z_convert, int64_t *zeno_result) {
    int64_t z_total = 0;
    zeno_list *t14 = zeno_list_copy_text(z_texts);
    for (int64_t t15 = 0; t15 < t14->len; t15++) {
        const char *z_text = t14->items[t15].s;
        int64_t t16;
        if (!((bool (*)(zeno_closure *, const char *, int64_t *))z_convert->code)(z_convert, z_text, &t16)) { zeno_list_drop_text(t14); return false; }
        z_total = zeno_add(z_total, t16, 15, 9);
    }
    zeno_list_drop_text(t14);
    *zeno_result = z_total;
    return true;
}
```

The copy of the list that the `for` walks is let go on the way out, so
nothing is left behind when `convert` fails.

`double` raises nothing, and `twice` wants a function that can. The
compiler writes a small function for it, once, that calls `double`
and answers `true`:

```c
static bool zeno_raising_wrap_double(zeno_closure *zeno_self, int64_t a0, int64_t *zeno_result) {
    (void)zeno_self;
    *zeno_result = z_double(a0);
    return true;
}

static zeno_closure zeno_raising_value_double = {(zeno_code)zeno_raising_wrap_double, "double"};
```

Through LLVM it is the same function:

```llvm
define internal zeroext i1 @zeno.raising.wrap.double(ptr %self, i64 %a0, ptr %result) {
entry:
  %t1 = call i64 @z_double(i64 %a0)
  store i64 %t1, ptr %result
  ret i1 true
}
```

A function written where a value goes, handed straight to a raising
type, is compiled the raising way from the start.

One thing cannot be done. A value that already has a plain type,
such as `let g: function(int) -> int = double`, cannot be handed to
`twice`: it is called the other way, and changing that would take a
block of memory at every such place.

```
error: `f` of `twice` should be function(int) -> int raises ValueError, and this is a function value that raises nothing. In a firmware build a function that can raise is called in another way from one that cannot, so one kept as the first cannot become the second. Give it the type `function(int) -> int raises ValueError` where it is made, or build without `--firmware`
```

An ordinary build writes no C of its own for any of this: both kinds
of function are called alike there.

## How it is tested

- **The parser**: `a_function_type_can_say_what_it_raises`,
  `raises_after_a_result_belongs_to_the_function_it_ends`,
  `brackets_go_round_a_function_type_and_nothing_else`.
- **The checker**: eight tests, from
  `a_call_through_a_function_type_that_raises_is_checked_as_raising`
  to `a_function_of_an_interface_says_what_its_function_types_raise`. Each
  message on this page has a test of its words.
- **Both compilers**: `raising_function_values_match_every_way` builds
  `edge-raises-in-types.zeno` as firmware and with no collector,
  through C and through LLVM, and holds each to what `zeno run`
  prints, with 0 blocks left. It also holds the C of the small
  functions above to this page.
- **The targets** run the same program.
- **Nothing else moved**: `tools/same.py` shows that every other
  program is built into the same C, IR and bytecode as by 0.7.0.

## Things to try

1. Build the program at the top with `--firmware` and open `types.c`.
   Find `zeno_wrap_parse` and `zeno_raising_wrap_double`. Why does the
   one for `parse` not need to say `return true`?
2. Change `twice` to take `function(int) -> int` with no `raises`, and
   build with `--firmware`. Which line is refused now, and what does
   the message tell you to write?
3. Write `function find(key: str) -> int raises KeyError` and pass it
   to `each`. Read the message. Then change `each` to say `raises
   Error` in both places. What must the caller's `except` say now?
4. Write `let maybe: (function(int) -> int) or none = none` and call
   `maybe` only when it is not `none`. Then take the brackets away.
   What does the type mean without them?
5. Put `[parse, double]` in a `let` with the type `list[function(str)
   -> int raises ValueError]`. Why is `double` refused, and what is the
   smallest change to it that makes the list build?
6. Build `make` from this page with `--no-collector --memory`. How
   many blocks does it take, and which of them is the function that
   `make` returns?
