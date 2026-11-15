# What writing the book found

The Zeno Book prints hundreds of small programs, and each one is run
by the real compiler as the book is built, so that what the book says
a program prints is what it prints. Writing it was the first time the
whole language was used by somebody trying to explain it. That finds
different things from building it: the places where a program was
accepted by `zeno check` and then failed, or where a message said less
than the checker knew.

This page goes through what was found and what was done. The decisions
are chapter 13 of [`decisions.md`](decisions.md).

## `check --no-collector` runs the compiler too

```zeno
var readings: list[int or none] = [1, none, 3]
```

```sh
zeno check --no-collector gaps.zeno     # used to say: No errors found.
zeno build --no-collector gaps.zeno     # ... and this refused it
```

A build with no collector keeps each value somewhere that is known
when the program is built. Most of the rules that follow from that are
the checker's. A few are met only by the compiler, as it decides where
each value goes: a list of numbers that may be `none` is one, because
each item would need a small block of its own.

`check --no-collector` now does what the build does, up to the point
of writing the C, and throws the C away (`check_no_collector` in
`src/main.rs`). Both commands give the same message now:

```
error: a list of numbers that may be `none` keeps each item in a small
block of its own, and `zeno build --no-collector` leaves the collector
out, so nothing would free them. Make it a list of an enum ...
```

## `none` has one test

```zeno
let found: int or none = find(names, "Ada")

if found is none { ... }      # the way to ask
if found == none { ... }      # refused
```

```
error: `none` is tested for with `is`: write `is none`, not `== none`
```

`==` compares two values, and `none` is the lack of one. Both forms
used to work and gave the same answer, which is two ways to say one
thing. Two values that may each be `none` are still compared with
`==`.

The other half is a test that cannot be true:

```zeno
let count = 3
if count is none { ... }
```

```
error: `is none` is always false: this is int, which is never `none`.
Only a value whose type says `or none` can be
```

Before, this was `false` every time and nothing was said. It is the
same rule as `1 == "1"` (decision 2.5a): a comparison whose answer is
known before the program runs is a mistake.

One case is left alone on purpose. After `if found is not none {`, the
checker knows `found` is there, so a second `found is none` inside the
block is also always false. It is not refused, because the name was
declared `int or none`, and asking twice is caution.

## A field of an enum case is taken out with `match`

```zeno
enum Shape {
    Circle(radius: float)
    Square(side: float)
}

let shape = Circle(2.0)
print(shape.radius)
```

Outside strict mode this ran, and printed `2.0`. With `Square(2.0)` on
the fifth line it was accepted just the same, and stopped when it ran.
Now it is refused in both, and the message writes out what to do:

```
error: `radius` belongs to the case `Circle`, and which case a `Shape`
is, is only known inside a `match`. Take it out there:
`match ... { case Circle(radius) { ... } }`
```

For a case with several fields the message puts `_` in the places that
are not the one asked for: `case Rect(_, h)`.

## A `const` is a constant

```zeno
import math

const LIMIT = 100
const DOUBLE = LIMIT * 2            # another const, and an operator
const TABLE = [1, 2, 4, 8]          # a list of constants
const START: u8 = 0xA5
const WAIT = 500 ms
const TAU = 2 * math.pi

var reading = 3
const NOW = reading                 # refused
const SIZE = len("abc")             # refused
```

```
error: this is only known when the program runs, and a `const` is
fixed when the program is built. A `const` is written from numbers,
text, other `const`s and operators. For a value that is worked out once
while the program runs, use `let`
```

`const` was checked exactly as `let` was, so the last two lines were
accepted. The rule is in `Checker::not_constant`, which walks the
value and gives back the first part the compiler cannot work out by
itself. The message points at that part.

## A default comes last

```zeno
function scaled(by = 2, value) { ... }
```

```
error: `value` has no default, and comes after `by`, which has one. A
call gives its arguments in order, so `by` could only be left out by
naming every argument after it. Put the parameters that have a default
last
```

And a call with one mistake in it now gets one message. `f(b=5, 1)`
used to get three: the order, "`b` was given twice" and "needs a value
for `a`". The last two were made by counting the misplaced argument.

## Messages

| Written | Before | Now |
|---|---|---|
| `let a = None` | `None` is not defined | `None` is not defined. Zeno spells it `none` |
| `let a = True` | `True` is not defined | `True` is not defined. Zeno spells it `true` |
| `elif` at the start of a line | expected the end of the line, found the name `x` | Zeno spells `elif` as `else if` |
| `let y: float = x`, `x` an int | Write the number with a decimal point | Write a number with a decimal point, as in `1.0`, and convert a value with `float( )` |
| `min(3 V, 200 mV)` | `min` needs numbers, but this is a quantity in V | `min` works on plain numbers, and these have a unit. Compare quantities with `<` and `>`, which know the units |

The same hint is given for `null`, `nil`, `NULL`, `nullptr`,
`undefined` and `False`. A program can still use any of them as a name
of its own: the hint is only for a name that means nothing.

## What was left

Decision 13.8 lists three things that were found and not changed,
with the reason for each. The one most likely to be met: for `let x:
str = first([1])` the error is true and points at the argument
instead of at `x`.

## Three more messages

These were on that list for a few hours (decision 13.9).

```zeno
class User {
    name: str or none
}

let user = User("Ada")
if user.name is not none {
    print(user.name + "!")          # refused, and it always was
}
```

A field is never known to be there. It belongs to an object, and any
call between the test and the use could reach that object and put
`none` in the field. The message used to say "Check for `none`
first", which the program above has just done. Now it says:

```
error: `+` does not work between str or none and str. `name` is a
field, and a field can be given a new value between a test and its
use, so a test of it is not remembered. Copy it into a variable with
`let`, and test that for `none`
```

```zeno
let name = user.name
if name is not none {
    print(name + "!")               # fine: nobody else can change `name`
}
```

Two enums that both have a `Dot` are told which enum has it already,
and why that matters: a case is written by its name alone. And a
method written without `self` is shown on its own line, where it used
to be shown on the line of its class.

## The tests

In `src/checker.rs`: `a_field_that_may_be_none_says_to_copy_it`,
`a_case_has_one_enum`,
`a_mistake_in_the_first_line_of_a_method_is_shown_there`,
`none_is_tested_for_with_is`,
`a_const_is_written_from_what_the_compiler_can_work_out`,
`a_field_of_a_case_is_taken_out_in_a_match`,
`one_mistake_in_a_call_gets_one_message`,
`a_mistake_from_another_language_says_what_zeno_writes`. In
`src/parser.rs`: `a_parameter_with_a_default_comes_after_those_without`,
`elif_at_the_start_of_a_line_gets_the_hint_too`. In `src/main.rs`:
`check_with_no_collector_says_what_that_build_would_say`.

## Things to try

1. Write a `const` from a `let`. Read the message, then make the
   `let` a `const` too. What else has to be true of it?
2. `const TABLE = [1, 2]` and then `TABLE.append(3)`. Is that
   refused? Read decisions 3.9 and 13.4 and say what `const` promises
   and what it does not.
3. In `Checker::not_constant`, let `len` through as `u8` and `float`
   are. Does `const SIZE = len("abc")` now check? Does it build with
   `zeno build`? Put it back.
4. Write a function that takes `x: int or none`, tests `x is not
   none` and inside that block tests `x is none`. Why is the second
   test allowed when `let n = 3` followed by `n is none` is not?
5. `shape.radius` is refused even for an enum with one case, where it
   could never be wrong. Would you allow it there? What happens to a
   program that relied on it on the day a second case is added?
6. Give `min` and `max` quantities: what should `min(3 V, 200 mV)`
   print, and in which unit? Look at what `3 V + 200 mV` prints first.
