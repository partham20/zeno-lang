# Several values from one function

A function can give back two things at once: a value and a status, a
quotient and a remainder. The first line says how many, with their
types in brackets after `->`; `return` gives them with commas; and
the line that calls it receives each with a name of its own.

```zeno
function divide(a: int, b: int) -> (int, int) {
    return a // b, a % b
}

function reading(raw: int) -> (float, bool) {
    let volts = float(raw) * 3.3 / 4095.0
    return volts, volts < 3.0
}

let quotient, remainder = divide(7, 2)
print(quotient, remainder)

let volts, low = reading(3100)
if low {
    print("battery low:", volts)
}

var first = "left"
var second = "right"
first, second = second, first
print(first, second)
```

```
3 1
battery low: 2.498168498168498
right left
```

`zeno run`, `zeno run --vm`, every build (`--firmware`,
`--no-collector`, `--llvm`) and every target print the same. The
reasons for each choice are in chapter 57 of
[`decisions.md`](decisions.md). It is item 10 of
[`missing.md`](missing.md), and is new in 0.8.0. A program that does
not write it is unchanged.

## The rules

1. **A function may give two or more values.** Its first line says
   `-> (int, str)`, and each `return` gives that many, with commas.
   One value is written without brackets: `-> int`.
2. **They are received by that many names**: `let q, r = divide(7,
   2)`, `var q, r = ...`, or `q, r = ...` for places that already
   exist, which may be variables, fields or items:
   `box.low, items[0] = divide(20, 6)`. Each name may have a type:
   `let q: int, r = divide(7, 2)`.
3. **Every value is received.** `_` receives one and drops it: `let _,
   r = divide(9, 4)`.
4. **Several values are not one value.** They cannot be kept in one
   variable, put in a list or a field, passed as one argument, used
   with an operator, or printed. To keep them together, declare a
   class: a class gives each part a name.
5. **Everything on the right is worked out before anything on the
   left is given a value.** That is why `first, second = second,
   first` swaps them. Values written out, `let a, b = 1, "two"`, work
   the same way.
6. `(int, str)` is a type only after `->`, in a function's first line
   or in a function type: `function(int, int) -> (int, int)`.

A function that gives several values can be passed on with `return`
(`return divide(a, 3)` in a function that gives `(int, int)`), held in
a variable (`let f = divide`), written in an interface, made generic
(`function swap[A, B](a: A, b: B) -> (B, A)`), and can raise:

```zeno
function checked(a: int, b: int) -> (int, str) raises ValueError {
    if b == 0 {
        raise ValueError("no zero")
    }
    return a // b, "ok"
}
```

## What is refused, and what it says

```zeno
let both = divide(7, 2)
```

```
error: `divide` gives 2 values. Receive each: `let a, b = divide(...)`. To keep them together, declare a class
```

The same message is given for `print(divide(7, 2))`, `[divide(7,
2)]`, `divide(7, 2) + 1`, and a call written alone on a line. Wrong
counts:

```
error: `divide` gives 2 values, and there are 3 places on the left of `=`. Receive each, and write `_` for one that is not wanted
error: there are 2 places on the left of `=` and 3 values on the right. Each place receives one
error: this function gives 2 values, and this is one. Give each, as in `return a, b`
```

A function that gives several values cannot be given to `map`,
`filter`, `reduce` or `sort`, which need one value from each call.

## In a program with no types

The checker refuses what it can see even with no types written:
`let x = pair(3)` names `pair`, and `pair`'s `return n, m` says it
gives two. When the values pass through a function that only says
`return make(n)`, nothing tells the checker how many there are, and
they are checked where they are received:

```zeno
function pair(n) {
    return n, n * 2
}
function twice(make, n) {
    return make(n)
}
let shown = twice(pair, 3)
print(shown)
let a, b, c = twice(pair, 3)
```

```
<2 values: 3, 6>
error: this gives 2 values, and there are 3 places on the left of `=`
```

Shown, they say what they are, as a function shows as `<function
pair>`. An operator on them gets the message any wrong kind of value
gets, with the kind named "2 values". Section 57.3 of the decisions
says why they are not stopped at every use.

## How a build gives them

The first value comes back as a function's answer always does, and
each further one goes out through a pointer the caller passes after
the arguments. The C of `divide` begins

```c
static int64_t z_divide(int64_t z_a, int64_t z_b, int64_t *zeno_gives_2);
```

A firmware build does the same, and a function that can also raise
has its answer pointer after those. With no collector, a text given
as a further value is a copy that whoever receives it owns, and `_`
gives it back at once: a loop of a thousand calls leaves nothing.

## Left for later

Swapping two items of a list that owns them with no collector
(`xs[i], xs[j] = xs[j], xs[i]`), several names in a `for`, and a
default for one of several results.
