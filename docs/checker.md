# How the type checker works

The type checker is the fourth stage. It reads the syntax tree and finds
mistakes **without running the program**: a misspelt name, a str where an int
was promised, a `match` that forgets a case, a value that might be `none`
used as if it were not.

```sh
cargo run -- check examples/tour.zeno             # check only
cargo run -- run examples/tour.zeno               # check, then run
cargo run -- check --strict examples/tour.zeno    # every type must be known
```

Before this stage, a mistake on line 200 was found only when line 200 ran.
Now `zeno run` refuses to start until the checker is satisfied.

## The file

- [`src/checker.rs`](../src/checker.rs): everything, with its tests.

## The same walk, with types in place of values

The checker is the interpreter's twin. It has the same shape: scopes, a
function per statement kind, a function per expression kind. The difference
is what flows through it.

| | Interpreter | Checker |
|---|---|---|
| `1 + 2` gives | the value `3` | the type `int` |
| a scope maps names to | values | types |
| an `if` | runs one branch | checks both |
| a loop | runs the body many times | checks the body once |
| a function call | runs the body | compares arguments with the signature |

So the checker's `expr` returns a `Ty` where the interpreter's `eval` returns
a `Value`. `Ty` is an enum of every type Zeno has: `Int`, `Str`, `List` of
something, `Optional` of something, an object of a class, and so on.

## Gradual: three kinds of answer

Zeno's types are optional, so for any expression the checker ends up in one
of three places:

1. **The type was written down.** `let x: int = ...` is checked against it.
2. **The type was left out but can be worked out.** `let n = 3` is an `int`.
   `[n * 2 for n in range(3)]` is a `list[int]`. No annotation needed.
3. **The type cannot be known until the program runs.** A parameter that is
   called with an int in one place and a str in another could be either. Its
   type is `Dynamic`.

`Dynamic` fits everywhere and everything fits it, so untyped code passes
through without complaint. The function `fits(from, to)` is the centre of the
checker: it decides whether a value of one type may go where another is wanted.

**Strict mode** (`--strict`, or `strict` as the first line of the file) turns case 3 into an error. That is the mode for
firmware, where a value of unknown type would need extra memory at run time.

## Working out a return type

```
function double(n: int) { return n * 2 }
```

No return type is written, so the checker finds it by checking the body and
collecting the type of every `return`. If the end of the body can be reached
without a `return`, `none` is added. The types are combined with `join`:
`int` and `int` give `int`; `str` and `none` give `str or none`.

This is done the first time the function's signature is needed (`signature`),
which may be at a call that comes before the function in the file. A function
that is mid-check when it is called is being called recursively; that call
sees what the signature declares, or, if no return type is written, what the
previous pass worked out (see the next section).

### A use that comes too early

Checking a body at the first use has a second job. A function may use
the variables its block declares above it, and those must exist before
it runs:

```
let x = 1
function outer() {
    print(inner())      # too early: the x below has no value yet
    let x = 5
    function inner() { return x }
}
```

`hoist` gives each function the list of variables its block declares
above its line (`FunctionInfo::above`). When `signature` starts on the
body it notes which of those are not declared yet (`Early::missing`).
A name in the body that is on that list is reported there
(`too_early`), where it would otherwise be taken for the `x` on the
first line, and the program would stop when run (decision 3.6d). A
class keeps the same list for the defaults of its fields, which are
worked out where an object is built.

## Working out parameters and fields

```
function add(a, b) { return a + b }
print(add(1, 2))
```

Nothing says what `a` and `b` are, but the only call passes ints. The checker
uses that. Every untyped parameter and field is a *slot* with an identity (a
`SlotId`: where its function or class is declared, plus its name). Whenever a
value arrives at a slot (an argument at a call, a default value, a value
stored in a field, a function passed where a function type is wanted),
`sees` records its type against the slot.

One answer leads to another: if `a(x)` passes `x` on to `b`, then `b`'s
parameter is known only once `a`'s is. So `check` runs the whole checker
several times:

1. In the first pass every slot is *unsettled*. Its type is `Invalid`, the
   quiet type, so nothing built on it complains and nothing built on it
   counts as evidence.
2. At the end of the pass, each slot has seen either one consistent type
   (`known`), types that disagree or cannot be pinned down (`open`, which
   means dynamic for good), or nothing.
3. The next pass starts with those answers. This repeats until a pass changes
   nothing, at most `MAX_ROUNDS` times.
4. A final pass, with everything settled, is the only one whose errors are
   reported.

Types are carried between passes as a `Key`, a plain description such as
"list of int" or "the class declared at line 4", because each pass builds its
own `ClassInfo` objects and one pass's cannot be used in the next.

A function value can leave the checker's sight: stored in a `list[any]`, or
passed to something dynamic. From there it may be called with anything, so
`escapes` marks its slots open. `arrives` is where this is noticed: it is
called whenever a value has been accepted somewhere, and looks inside lists
and `or none` for functions.

The return type of a recursive function uses the same machinery, as a slot
named `->`. In the first pass the recursive call's type is unsettled and is
left out of the `join`, so `fact` is an `int` from `return 1`; the second
pass confirms it.

## Function types and units

`function(int) -> int` resolves to `Ty::Callable`. A real function is still a
`Ty::Function` that points at its declaration; when one arrives where a
`Callable` is wanted, `expect_function` compares them: the number of
parameters, each parameter, and what is returned.

A unit name used as a type, such as `d: ms`, resolves to the same
`Ty::Quantity` that the literal `500 ms` has. The list of units lives in
`src/units.rs`, shared with the interpreter, and a unit that is not on it is
an error.

## Knowing that something is not none

```
let user = find_user(2)         # str or none
print(user.upper())             # error: might be none
if user is not none {
    print(user.upper())         # fine: it is a str in here
}
```

When a condition is `x is not none`, the checker records that inside that
branch `x` has the plain type. This is called *narrowing*. `narrowings` looks
at a condition and returns two lists: what is known when it is true, and what
is known when it is false. It understands `is none`, `is not none`, `not`,
`and` and `or`.

The body of `while x is not none { ... }` knows it too.

It also follows early exits. After

```
if text is none { return "" }
```

the only way to reach the next line is for `text` not to be `none`, so the
rest of the block treats it as a plain `str`. To know that, every statement
check returns one bool: *does this always leave the block?* `return`, `raise`,
`break` and `continue` do; an `if` does when both its branches do.

That same bool is how the checker catches a function that can run off its
end without returning what it promised.

### A new value changes what is known

What is known about a variable lasts until the variable is given a new
value. Then the new value decides:

```
var node = head                 # Node or none
while node is not none {
    total += node.value         # a Node in here
    node = node.next            # may be none again...
}                               # ...which the `while` asks about next time

var name = given                # str or none
if name is none {
    name = "anon"               # a str from here on
}
return name                     # a str on both paths, so a str
```

Three small rules do this, all in the `Assign` and `If` arms of `stmt`:

- Assigning a value that cannot be `none` narrows the variable. Assigning
  one that might be takes the knowledge away again (`Scope::widen`), in
  every block between here and where the variable was declared.
- After an `if`, the variable is narrowed when both branches end with it
  narrowed. That is the `name` example above.
- A loop body is checked once but runs many times, so what was known
  before the loop may be untrue on the second pass. Before checking the
  body, `forget_assigned` drops what is known about every variable the
  body assigns.

The variable keeps its declared type the whole time (`Variable.declared`),
and that is what an assignment is checked against. A narrowed `int or none`
can still be given `none`.

## match must cover everything

For a `match` on an enum, the checker collects which cases the arms handle
and reports the ones left over:

```
error: this `match` does not handle Missing. Add a `case` for it, or a final `case _`
```

A pattern like `Circle(r)` handles the whole case. `Circle(1.0)` handles only
one value of it, so it does not count. `_` or a new name handles everything.

## raises

The checker keeps a list of the errors the code checked so far can raise. A
`raise` statement adds to it; so does calling a function whose signature has
a `raises`. A `try` takes out the ones its `except`s catch. At the end of a
function body, what is left is what the function can raise.

If the function wrote `raises ...`, that list must cover it. If it wrote
nothing, the list is worked out, exactly like a return type, and passed on to
its callers. In strict mode it must always be written.

## Many errors, but no avalanche

Unlike the parser, the checker reports every mistake it finds, because after
a type error it still knows exactly where it is in the program.

The danger is one mistake causing ten messages. To prevent that, an
expression that has had an error reported gets the type `Invalid`, which, like
`Dynamic`, fits everywhere. A misspelt name is reported once, and everything
built on it stays quiet.

## Handing the answers on

`check_with_types` returns the errors and a `Types`: the worked-out signature
of every function, in a plain form (`Kind`) with no links back into the
checker. `zeno build` uses it to give every C function its types. See
`docs/compiler.md`.

`Types` also lists every place where a variable is read after narrowing.
The compiler stores an `int or none` differently from an `int`, so it has
to know that `x` inside `if x is not none { ... }` is the plain number.
Only the checker knows that, so the checker writes it down.

Owned values are handed on the same way. `Types` lists every place where a
name gives its value away (`moves`), every `let` or `var` that becomes an
owner (`owner`), and every statement that makes an owned value and drops it
on the floor (`discarded`). The compiler turns each of those into a line of
C. The rules themselves, and how the checker follows a value through `if`,
loops and `try`, are in [`docs/ownership.md`](ownership.md).

## What it does not do yet

- **An untyped function is not generic by itself.** `function id(x) {
  return x }` called with an int and with a str leaves `x` dynamic.
  Written `function id[T](x: T) -> T`, the checker makes a copy for
  each type: see [`generics.md`](generics.md).
- **Derived units.** `1 s + 500 ms` works, but `V * A` is not `W`: units are
  grouped by what they measure, with no arithmetic between groups.
- **Fields of an enum value.** `shape.radius` is `Dynamic`, because which
  case the value is, is not known. Use `match`.
- **Exact positions inside signatures.** A bad parameter or field type is
  reported at the start of the function or class, not at the parameter.
- **Sized integers.** There is one `int`. `u8`, `i32` and friends come with
  the compile-to-C stage.

## Things to try

1. Make `"abc" * 3` type check (and add it to the interpreter).
2. Give parameters and fields their own spans in the tree, so that errors
   point at the exact parameter.
3. Add a unit of your own to `src/units.rs`, such as `kA` or `um`. What must its size be?
4. Warn about code that can never run: statements after a `return`.
5. Narrow on `==`: after `if x == none`. Why is `is none` the form Zeno prefers?
