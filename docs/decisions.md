# Design decisions

Every choice in Zeno, and the reason for it. If a choice has no good reason,
it should not be here; and if a better reason turns up for the opposite
choice, the choice should change.

Each entry says what was decided, why, and what was turned down. Entries
marked *(provisional)* are expected to be revisited at the stage named.

## The principles

Most decisions below fall out of five principles. When two choices are
otherwise equal, the principle higher up the list wins.

1. **Catch the mistake as early as possible.** Before the program runs is
   better than while it runs; while it runs is better than a wrong answer.
   Zeno should never quietly do something the programmer did not mean.
2. **Read like plain words.** Code is read far more often than it is written.
   A whole word beats a symbol or an abbreviation.
3. **Punctuation has to earn its place.** A mark stays only if removing it
   would make code harder to read or easier to get wrong.
4. **One way to say a thing, with one meaning.** An operator or keyword
   should not change meaning depending on the values it is given.
5. **Nothing that a microcontroller cannot afford.** Every feature must be
   able to compile to small, predictable C with no hidden runtime.

Where Python already has a spelling that passes these tests, Zeno uses it,
because more people can read Python than any other language.

---

## 0. What kind of language Zeno is

### 0.1 Zeno is both object-oriented and functional

**What that means.** The two are not opposites; they answer different
questions. Object-oriented style is about *where code lives*: data and the
functions that work on it are bundled into a class. Functional style is
about *how data flows*: functions are values that can be passed around, and
results are built from new values instead of by changing old ones.

Zeno has the pieces for both:

| Object-oriented | Functional |
|---|---|
| `class` with fields and methods | functions are values: store them, pass them, return them |
| objects that hold state | `function(n) { ... }` written in place, with no name |
| `self` | closures: a function remembers the variables around it |
| | `let` is the default, so values do not change unless you say `var` |
| | enums with data, taken apart by `match` |
| | `map`, `filter`, `reduce`, and comprehensions |

**Why.** Different problems have different natural shapes. A device driver
is an object: it has state (a register, a mode) and operations on it. A
signal-processing chain is functional: readings go in, get filtered and
scaled, and a result comes out. A language meant to cover every use must not
force one shape onto both. Python, Rust, Kotlin and Swift all made the same
choice. [`examples/styles.zeno`](../examples/styles.zeno) shows one job done
each way.

**The limits, stated plainly.**
- Zeno is not a *pure* functional language like Haskell, where functions are
  forbidden from having effects. Writing to a hardware register is an effect,
  and firmware is made of them.
- Classes have no inheritance yet *(provisional, see 3.7)*.

### 0.2 Types are optional

Every type annotation can be left out: on variables, parameters, return
values, class fields and enum cases. The same program runs with or without
them.

```
function area(w: float, h: float) -> float { return w * h }    # typed
function area(w, h) { return w * h }                           # untyped
```

**Why.** Types pay for themselves in large programs and in firmware, where a
mistake found on the bench costs hours. They get in the way of a ten-line
script or a quick experiment. With optional types, one language covers both,
and a program can start loose and be tightened a line at a time as it grows,
instead of being rewritten in another language. *(This is called gradual
typing; TypeScript and Python's type hints took the same road.)*

**How it works.**
1. Where a type is written, it is checked before the program runs.
2. Where it is left out, the checker works it out from the value when it
   can. `let n = 3` is an `int` without being told.
3. Where it cannot be worked out, the value is *dynamic*: it carries its type
   with it and is checked while the program runs.

**The cost, stated plainly.** A dynamic value needs a tag saying what it is,
and usually memory from the heap. A small microcontroller cannot afford
either. *(Principle 5.)* So there is a **strict mode** (`--strict`), in which
step 3 is an error: everything must be known before the program runs. Strict
mode will be the default when compiling for a microcontroller, and optional
elsewhere.

**What gets worked out.** Everything that can be: variables, return types
(of recursive functions too), and also the parameters and class fields that
have no type written. A parameter is worked out from the calls: if every call
to `add(a, b)` passes two ints, `a` and `b` are ints, and the body is checked
with that knowledge. A field is worked out from every value stored in it.

```
function add(a, b) { return a + b }
let total: str = add(1, 2)        # error: `total` should be str, but this is int
```

**Why.** The first version of the checker left untyped parameters dynamic.
That made strict mode demand a type on every parameter, so the "same program,
typed or untyped" promise held for scripts and broke for firmware. Working
them out keeps the promise: `examples/styles.zeno`, written with no types at
all, now passes strict mode and can be compiled without a heap.

**The rules, and the reason for each.**
- *Only what is certain is used.* If two calls disagree (`id(1)` and
  `id("s")`), the parameter stays dynamic; the checker never picks a winner.
  An int and `none` do combine, to `int or none`, because that is one real type.
- *A written type always wins.* It is a promise to the reader; the calls are
  checked against it and never change it.
- *A function that goes where its type is not tracked* (stored in a `list[any]`,
  passed to an untyped parameter that is itself dynamic) may be called from
  there with anything, so its parameters are not worked out from the calls
  that happen to be visible.
- *A function that is never called* has nothing to go on, and stays dynamic.

**The cost, stated plainly.** There are two.
1. When a parameter's type was worked out, a wrong call is reported *inside
   the function*, where the value is misused, not at the call. To keep the
   real mistake findable, the error carries a note naming the line the type
   came from:

   ```
   error: int has no field or method `upper`
    --> main.zeno:2:12
     |
   2 |     return a.upper()
     |            ^
     = note: `a` was worked out to be int from line 5. Give `a` a type to
             have each call checked against it instead
   ```

   The checker cannot know which of the two lines is wrong, so it shows
   both. Writing the type settles it: then the call is the error. This is
   the reason annotations are still worth writing on anything other people
   call.
2. Adding a call can change what a parameter is worked out to be: a second
   call with a different type turns it dynamic again, which strict mode then
   reports at the function. The message says the type could not be worked
   out from how it is used.

**Alternatives rejected.** *Leave parameters dynamic* (the first version):
simple, but strict mode then forces annotations everywhere. *Make an untyped
function generic*, checked afresh for each call as C++ templates are: more
precise (`id(1)` and `id("s")` would both be known), but one function then
becomes many in the compiled program, which costs flash on a microcontroller
without the programmer having asked for it. That belongs to a later, explicit
generics feature. *(Provisional: generics are not designed yet.)*

**How it is done.** The checker makes several passes over the program. Each
pass notes what arrives at every untyped parameter and field, and the next
pass starts from those answers. It stops when a pass learns nothing new. If
that does not happen within eight passes, everything worked out is thrown
away and treated as dynamic: unknown is always safe, a wrong guess is not.

**Strict mode is chosen by the file or by the command.** A file whose first
line is the word `strict` is checked in strict mode, however it is run.
`--strict` on the command line does the same from outside.

**Why a line in the file.** Whether a file may hold dynamic values is a fact
about that file: firmware must not, a script may. A flag lives in a build
script, where it can be forgotten; a first line travels with the code and is
seen by every reader. It is a bare word, with no `#` or `@`, because it is
part of the language and not a comment or a decoration. *(Principle 3.)* It
must be the first line so that nothing above it is checked under different
rules. There is no way to switch strict mode *off* from inside a file that is
run with `--strict`: the stricter of the two always wins.

---

## 1. Syntax

### 1.1 Blocks use `{ }`, not indentation

**Why.** With braces, where a block ends is written down rather than implied.
A stray space or a tab-versus-spaces mix can never change what a program
does, code survives being pasted into a chat or an email, and editors can
re-indent it automatically. *(Principle 1.)*

**Turned down.** Python-style indentation is cleaner to look at, but it makes
whitespace meaningful, which is exactly the kind of invisible mistake
principle 1 is against. It also makes the lexer harder.

### 1.2 No semicolons; a line break ends a statement

**Why.** The line break is already there and already means "next statement"
to the reader. A semicolon on every line repeats that information.
*(Principle 3.)* Go, Swift and Kotlin work the same way.

**The cost, and how it is handled.** A long statement must be able to
continue on the next line. It does in two cases that are impossible to
misread: inside `( )` or `[ ]`, and after a token that cannot end a statement
(`+`, `,`, `=`, `and`, ...). The rule is "an unfinished line continues".

### 1.3 No `:` after `if` / `for` / `function`, no `( )` around conditions

**Why.** The `{` already marks where the condition ends, so both are
decoration. *(Principle 3.)*

### 1.4 Keywords are whole words: `function`, not `fn` or `def`

**Why.** `function` can be read aloud by someone who has never seen the
language. `fn` and `def` have to be learnt. The extra characters cost nothing
with editor completion. *(Principle 2.)*

### 1.5 `and`, `or`, `not` instead of `&&`, `||`, `!`

**Why.** They are words *(principle 2)*, and `!` in particular is easy to
miss: `if !ready` and `if ready` differ by one thin character. It also frees
`&` and `|` for bit operations later, which firmware code needs, without the
`&` versus `&&` confusion C has.

### 1.6 `let` and `var` to declare; plain `=` only updates

`let x = 1` creates a value that never changes, `var x = 1` one that can, and
`x = 2` changes an existing `var`.

**Why.** In Python, `x = 1` both creates and updates, so a misspelt name
(`coutn = 5`) silently creates a new variable. Separating the two turns that
into an error. *(Principle 1.)* Making the unchanging form the short, default
one nudges code toward values that do not change, which are easier to reason
about and let the compiler put data in flash instead of RAM. *(Principle 5.)*

**Turned down.** `let` / `let mut` (Rust): `mut` is an abbreviation.
`const` / `let` (JavaScript): `const` is kept for true compile-time constants.

### 1.7 `const` for values fixed when the program is built

**Why.** A `const` must be computable by the compiler, so it can live in
read-only memory and be used for things like array sizes. `let` is "set once
while running", which is a different promise. *(Principle 5.)*

### 1.8 `else if`, no `elif`

**Why.** With braces, `} else if x {` already reads naturally and needs no
keyword of its own. *(Principle 4.)* Typing `elif` gives a hint pointing to it.

### 1.9 `match` uses `case Pattern { ... }`, with no `=>`

**Why.** Every other block in the language is `keyword something { }`. An
arrow would be a second way to introduce a block. *(Principles 3 and 4.)*

### 1.10 `#` for comments

**Why.** One character, and it leaves `//` free for floor division (4.2).

### 1.11 Types go after the name: `port: int`, `-> int`

**Why.** The name is what you scan for, so it comes first; and because types
can usually be inferred, the optional part should be the trailing part. The
`:` and `->` earn their place by separating a name from its type at a glance.

**Optional.** A type can always be left out; see 0.2.

### 1.11a The type of a function is written `function(int) -> int`

```
function twice(f: function(int) -> int, x: int) -> int { return f(f(x)) }
```

**Why.** It is the heading of a function with the names and body taken out,
so there is nothing new to learn: the same keyword, the same `( )`, the same
`->`. *(Principle 2.)* Leaving `->` off means the function returns nothing.
A function fits the type if it can be called with those arguments and what it
returns fits; an untyped anonymous function passed there has its parameters
worked out from the type.

**Alternative rejected.** `(int) -> int`, as in TypeScript and Swift: shorter,
but it gives `( )` a second meaning in types and reads as punctuation where
Zeno prefers a word. *(Principle 3.)*

### 1.12 `true`, `false`, `none` in lower case

**Why.** They are keywords like any other, and every other keyword is lower
case. Python's capitals are a historical accident. *(Principle 4.)*

### 1.13 Absence is written `T or none`, and tested with `is none`

**Why.** There is no null that can turn up anywhere. A value can be `none`
only if its type says so, and then it must be checked before use.
*(Principle 1.)* `str or none` reads as it means. *(Principle 2.)*

`is` and `is not` work **only** with `none`. In Python, `is` compares
identity, which gives surprising answers on numbers and strings and is a
well-known trap. Restricting it leaves one way to compare values (`==`) and
one way to test for absence. *(Principle 4.)*

**Provisional.** General unions (`int or str`) are not accepted. They overlap
with enums, and they need a tag stored beside the value in the compiled
code. To be decided at the type checker.

### 1.14 Formatted strings need the `f` prefix

**Why.** Without the prefix, any string containing `{` would change meaning,
which is common in firmware (JSON, format templates). The single `f` marks
the strings that contain code. *(Principle 1.)*

### 1.15 Numbers can carry units: `500 ms`, `3.3 V`

**Why.** Mixing up units is a real source of firmware bugs: milliseconds
passed where microseconds were expected, or volts added to millivolts. If
the unit is part of the value, that is an error instead of a wrong reading.
*(Principle 1.)* `sleep(500 ms)` also documents itself. *(Principle 2.)*

**A unit is also a type.** `function wait(d: ms)` takes a quantity in
milliseconds; passing `3 s` or a bare `3` is an error before the program
runs. The unit's own name is the type, so there is no second vocabulary such
as `Duration` to learn.

**Units come from a fixed list** (`s ms us ns min h`, `Hz kHz MHz`, `V mV uV
kV`, `A mA uA`, `ohm kohm Mohm`, `W mW kW`, `J Wh kWh`, `degC K`, `m mm cm
km`, `g mg kg`, `deg rad`, `rpm`). Anything else, such as `5 sec`, is an
error. **Why.** If any word after a number were a unit, a typing mistake
would silently create a new one, which is the very bug units are there to
prevent. *(Principle 1.)*

**Units that measure the same thing work together.** `1 s + 500 ms` is
`1500 ms`, `1 s == 1000 ms` is true, and `1 h / 1 min` is `60.0`. The answer
is always in the *finer* of the two units.

**Why the finer unit.** Going from seconds to milliseconds multiplies by a
whole number, which loses nothing. Going the other way divides, and `1 ms`
is not exactly representable as a number of seconds. So the direction that
is always exact is the one chosen without being asked.

**Changing unit is written `.ms`.** `(3 s).ms` is `3000 ms`; `delay.s`
gives seconds. It reads as "delay, in seconds", needs no new keyword, and
cannot clash with a variable called `s` or `m`. Passing `3 s` where `ms` is
wanted is still an error, and the message says to write `.ms`.

**Why not convert automatically at a call.** It was tried on paper and
rejected: the interpreter does not look at type annotations (5.6), so it
would print `3 s` where the compiled program, which knows the parameter is
`ms`, would print `3000 ms`. One program, two outputs. An explicit `.ms`
means the same thing everywhere. *(Principle 1, and 5.5d.)*

**What does not convert.** `degC` and `K` differ by an offset, not a scale,
so adding them means nothing. `deg` and `rad` have no exact factor between
them. Both pairs are kept apart on purpose.

**Provisional.** No derived units yet (`V * A` is not `W`), and no
user-defined units.

### 1.16 Comparisons chain: `0 <= i < len`

**Why.** It is how the condition is written in mathematics, and it means what
it looks like: `0 <= i and i < len`, with `i` worked out once. *(Principle 2.)*

**Turned down.** C's meaning, `(0 <= i) < len`, compares a true/false with a
number and is a classic bug. Refusing chains altogether (Rust) is safe but
loses the readable form. An earlier version of Zeno did refuse them, while
this was being decided.

### 1.17 Errors use `try` / `except` / `raise`, and are declared with `raises`

**Why.** The words are Python's and are widely known. *(Principle 2.)* The
addition is `raises ValueError` in a function's signature, which lets the
compiler check that every error is handled or passed on deliberately.
*(Principle 1.)*

They are not exceptions in the C++ sense: there are no unwinding tables and
no cost on code that does not use `try`. How they compile is in 5.5i.
*(Principle 5.)*

**An object of any class can be raised**, as well as the built-in kinds:

```
class Timeout { after: ms }
raise Timeout(30 ms)
```

**Why.** An error often needs to carry more than a message: how long was
waited, which pin, what code the device returned. A class already says
"these named values, together", so no second way to declare an error is
needed. *(Principle 4.)*

`except Timeout as t` gives the object, with its fields. `except Error as e`
asks for any error at all, so it always gives an error value, whatever was
raised: for an object, `e.kind` is the class's name and `e.message` is the
object as text. Raising `e` again passes the object on untouched.

**How `raises` is checked.** It follows the same rule as every other
annotation (0.2). If a function writes `raises`, the list must be complete,
and the checker reports anything missing. If it writes nothing, the list is
worked out from the body and passed on to callers. In strict mode it must
always be written, so that every signature is a complete contract.

**What counts.** Only errors that are *raised on purpose* are tracked: a
`raise` statement, a call to a function that raises, and turning text into a
number with `int()` or `float()`, which fails on bad input. Errors that an
operator can produce, such as dividing by zero, indexing out of range or
overflow, can still be caught, but do not have to be declared. Those can
happen on almost any line, so declaring them would put `raises` on every
function, and a label that is on everything says nothing. Java's checked
exceptions are widely disliked for exactly this.

---

## 2. Numbers and operators

### 2.1 `/` always gives a float; `//` divides and rounds down

**Why.** In C, `7 / 2` is `3` and `7.0 / 2` is `3.5`: the same operator does
two different things depending on its operands, and silently dropping the
remainder is a classic bug. In Zeno the operator decides the meaning, not
the values. *(Principle 4.)* When you want whole-number division you say so
with `//`.

`//` rounds toward minus infinity and `%` takes the sign of the right-hand
side, so that `(a // b) * b + a % b == a` always holds and `i % n` is a valid
index for any `i`, negative or not.

### 2.2 Whole-number arithmetic is checked for overflow

**Why.** In C, overflow either wraps silently or is undefined behaviour. A
wrong number that looks plausible is the worst kind of bug. Zeno raises
`OverflowError` instead. *(Principle 1.)*

**Provisional.** The interpreter has one 64-bit `int`. Sized types (`u8`,
`i32`, ...) and explicit wrapping arithmetic, which firmware genuinely needs
for counters and checksums, come with the type checker.

### 2.3 An int and a float can be mixed; nothing else converts by itself

`1 + 0.5` is `1.5`. But `"3" + 4` is an error, and so is `if count { }`.

**Why.** Widening an int to a float matches what people expect from
arithmetic. Every other automatic conversion guesses at what the programmer
meant, and JavaScript shows where that leads (`"3" + 4` is `"34"`).
*(Principle 1.)*

**But not at a boundary.** `let x: float = 1` is an error; write `1.0`. Mixing
inside arithmetic produces a new value, so nothing is disguised. Storing an
int in a float variable would leave a value whose written type and real type
differ, and in compiled code it would hide a conversion instruction.
*(Principles 1 and 5.)* Kotlin, Swift and Rust draw the same line.

### 2.3b Outside arithmetic, an int is never quietly made a float

`1 + 2.5` mixes the two (2.3). But `[1, 2.5]` is not a `list[float]`, and a
function that returns `1` on one path and `2.5` on another does not return
`float`. Both are "a mix", which strict mode asks you to settle by writing
`1.0` or `float(n)`.

**Why.** The first checker did treat these as float. Compiling to C showed
what that costs: the compiled function really would return `1.0`, while the
interpreter, which never looks at types, returns the int `1`, and prints
`1`. The same program would print different things under `zeno run` and
`zeno build`. Converting only when asked keeps one answer everywhere, and it
matches the rule that was already there for `let x: float = 1`. *(Principle 1.)*

### 2.3a A whole number to a negative power is an error

`2 ** 8` is an int. `2 ** -1` would have to be `0.5`, a float. Then the type
of `a ** b` would depend on the *value* of `b`, which the checker cannot see.
Zeno refuses the negative power and says to write `2.0 ** -1`.
*(Principle 4: the operator decides the type, not the values.)*

### 2.4 Conditions must be real bools

`if items { }` is an error; write `if len(items) > 0 { }`.

**Why.** "Truthiness" means remembering which values count as false: is it
`0`, `""`, `[]`, `none`, `0.0`? The rules differ between languages and cause
bugs such as treating a valid reading of `0` as "no reading". Saying what you
are testing is clearer. *(Principles 1 and 2.)*

### 2.5 `==` compares contents

Two lists with the same items are equal, and so are two objects of the same
class with equal fields.

**Why.** That is what "equal" means to a reader. Comparing by identity
(Java's `==` on objects) is a famous source of confusion. *(Principle 2.)*

### 2.5a Comparing things that can never be equal is an error

`1 == "1"` is always false, so writing it is a mistake, most often a number
read as text and never converted. The checker reports it instead of quietly
answering false. *(Principle 1.)* An int and a float can be compared, because
they can hold the same number.

### 2.6 Positions start at 0, and negative positions are an error

**Why for 0.** It matches C, Python, and the memory layout of every target.

**Why no negatives.** In Python, `items[-1]` is the last item. That is handy,
but it means an index that goes negative *by mistake* quietly reads from the
wrong end instead of failing. For code that runs on hardware, a clear
`IndexError` is the safer behaviour. *(Principle 1.)*

---

## 3. Variables, functions and data

### 3.1 Each `{ }` block is its own scope

**Why.** A variable lives exactly as long as the block it is declared in, so
its lifetime is visible in the layout of the code. In Python a variable made
inside an `if` leaks into the rest of the function.

### 3.2 A name cannot be declared twice in the same block

**Why.** It is almost always a mistake or a copy-paste slip. *(Principle 1.)*
An inner block may reuse an outer name, since the braces make the boundary
clear.

### 3.3 Parameters and loop variables cannot be reassigned

**Why.** They behave like `let`. A parameter that still holds what the caller
passed, all the way down the function, is easier to follow.

### 3.4 Functions, classes and enums can be used above where they are written

**Why.** The alternative is C's forward declarations, or ordering a file to
suit the compiler rather than the reader. The usual way to read a program is
the main logic first, helpers below.

### 3.5 Default arguments are worked out on every call

**Why.** In Python, `def add(item, to=[])` creates the list once, and every
call shares it. It is one of the most-reported surprises in the language.
*(Principle 1.)*

### 3.6 Named arguments: `Point(x=3.0, y=4.0)`

**Why.** `set_timer(500, true, false)` cannot be read without looking up the
function. Names at the call site fix that. *(Principle 2.)*

### 3.6a A function with no name is written `function(n) { ... }`

```
let double = function(n) { return n * 2 }
readings.map(function(r) { return r * 1.8 + 32 })
```

**Why.** It is exactly a function declaration with the name left out, so
there is nothing new to learn and only one way to write a function.
*(Principle 4.)*

**Turned down.** Python's `lambda n: n * 2` is a second, more limited syntax
for the same thing (it cannot hold statements). JavaScript's `n => n * 2`
needs the `=>` that 1.9 removed.

**Provisional.** This is wordy for one-line functions. A shorter form may be
added once there is enough real Zeno code to see how often it is needed;
comprehensions already cover the most common cases.

### 3.6b `map`, `filter` and `reduce` are methods on a list

```
readings.filter(valid).map(to_celsius).reduce(add, 0.0)
```

**Why.** As methods they read left to right, in the order things happen:
take the readings, keep the valid ones, convert them, add them up. As plain
functions (Python's `reduce(add, map(to_celsius, filter(valid, readings)))`)
the same chain has to be read inside out. *(Principle 2.)*

### 3.6c A function written inside another keeps the variables it can see

```
function counter() {
    var count = 0
    return function() {
        count += 1
        return count
    }
}
```

The inner function uses `count`, and goes on using it after `counter` has
returned. It keeps the variable itself, not a copy: a change made by
either function is seen by the other.

**Why.** This is what the code says when read: there is one `count`. A copy
would make the example above return 1 every time, silently. *(Principle 2.)*

It keeps the variables that were in sight **where it is written**. A
variable declared further down the same block is not one of them, even if
it has the same name as an outer variable the function uses:

```
let y = 1
function g() {
    let f = function() { return y }    # the y above
    let y = 2
    return f()                         # 1, not 2
}
```

**Why.** The meaning of a name should be settled by where it is written,
not by what happens to be declared later. This is the rule in nearly every
modern language (it is called lexical scope), and it is what lets the
compiler decide, once, which variables a function needs to take with it.

An earlier version of this decision made an exception for the top level
of a file, where a function could use a variable declared below it. The
checker never allowed that, so no program that `zeno run` accepted used
it, and it is gone (3.6d).

### 3.6d A function with a name follows the same rule

A named function can be *called* from above the line it is written on
(3.4). What it can *see* is still decided by that line: every function,
class and enum of the blocks around it, and the variables declared above
it. Never a variable declared below it.

```
let x = 1
function outer() {
    function inner() { return x }      # the x above
    let x = 5
    return inner()                     # 1, not 5
}
```

**Why.** The reasons in 3.6c do not depend on whether the function has a
name. And the checker already read programs this way, so anything else
means `zeno check` and `zeno run` disagree about what a program does.

They did disagree, until the bytecode VM was written (5.10). The
interpreter made a named function at the top of its block and gave it
the block's scope, which goes on growing, so the function above returned
5. At the top of a file a `let len = 5` below a function that calls `len`
broke the function. The VM decides what a name means when it compiles
the function, could not reproduce either, and the comparison between the
two engines failed. The interpreter was wrong, and so was `zeno build`
for the top-level case. Both are fixed.

**A name means one variable.** If a block declares `x` above a function,
`x` inside that function is that variable, from the start. A call that
comes before the line declaring `x` has run stops with "`x` is not
defined". It does not quietly use an `x` from further out, which would
make the same name mean two variables in two calls of one function.
JavaScript has the same rule for `let`.

**The checker refuses it first.** `zeno check` reports a function used
above a variable it needs: "`x` is declared further down than the first
use of `inner`, which needs it". It can do this because it checks a
function's body at the first place the function is used, so it knows
which of the variables above the function exist at that moment. The same
goes for building an object above a variable that a field's default
uses. At first the checker missed the case where there *is*
an `x` further out: it took the name for that one and passed a program
that both engines then stopped.

The check is cautious in one way. A function written as a value,
`function() { return inner() }`, is checked where it is written, so if
that is above `x` it is refused even when the value is only called
later. Telling the two apart needs to know when a function value runs,
which the checker does not track. Naming a function above the variable
(`let f = inner`) and calling it below is accepted.

### 3.7 A class lists its fields, and gets a constructor for free

**Why.** The fields are the shape of the data, and the shape should be
written down in one place, not discovered by reading assignments inside an
initialiser. A fixed list of fields is also what lets a class compile to a
plain C struct. *(Principle 5.)* Setting a field that was not declared is an
error.

**Provisional.** There is no inheritance and no custom constructor yet. Both
are to be decided when classes are type checked.

### 3.8 Methods name `self` explicitly

**Why.** It shows that a method is an ordinary function that receives its
object as the first argument, with no hidden variable. *(Principle 4.)*

### 3.9 `let` fixes the name, not the contents *(provisional)*

`let xs = [1]` followed by `xs.append(2)` is allowed; `xs = [3]` is not.

**Why.** It is the simplest rule to state: the name always refers to the same
thing. Deeply unchangeable data was left open until memory was managed.
The collector (5.7) does not need it, so it is still open, and now purely
a question of what helps the programmer.

### 3.10 Lists and objects are shared, not copied

`let b = a` makes `b` the same list as `a`.

**Why.** It is what Python, Java and JavaScript programmers expect, and it is
the cheapest thing for the interpreter. This was provisional until the
memory management stage, which had to choose between sharing, copying and
Rust-style ownership for compiled programs. It chose to keep sharing, with
a garbage collector to make it safe. 5.7 has the reasons.

### 3.10a A `for` loop sees the list as it was when the loop began

Adding to a list, or removing from it, inside a loop over that list does
not change what the loop visits.

**Why.** The other choice, looping over the list as it changes, is the
source of two classic bugs: a loop that never ends because its body keeps
appending, and a loop that skips an item because its body removed one.
*(Principle 1.)* **The cost:** the loop takes a copy of the list first.
*(Provisional: the copy can be skipped when the compiler can see that the
body does not touch the list.)* Since 5.8 the copy is given back the moment
the loop ends, because only the loop ever pointed at it (5.8j).

### 3.11 Enums carry data, and `match` takes them apart

**Why.** "A reading is either a value or a fault code" is one of the most
common shapes in real programs, and most languages make it awkward. An enum
whose cases hold their own data says it directly, and `match` guarantees
every case is considered.

The checker reports a `match` on an enum that leaves a case out, naming the
missing cases. This is the main reason to use an enum instead of a set of
integer constants: when a new case is added, every `match` that needs
updating becomes an error, instead of silently falling through. A `match` on
anything else (numbers, strings) needs a final `case _`, since those cannot
be listed in full.

### 3.12 A value that may be `none` must be tested first

```
let user = find_user(2)          # str or none
print(user.upper())              # error
if user is not none {
    print(user.upper())          # fine
}
```

**Why.** Using a null value is the most common crash in C, Java and
JavaScript; its inventor called it his "billion-dollar mistake". Zeno makes
it impossible to forget the test. *(Principle 1.)*

The checker follows the test itself rather than asking for special syntax:
after `x is not none`, or after `if x is none { return }`, it knows `x` is
present. The programmer writes the `if` they would have written anyway.

**Turned down.** Rust's `match` / `if let Some(x)` and Swift's `if let x = x`
need a new construct and a second name for the same thing.

**A new value changes what is known.** `x = 5` makes an `int or none`
variable an `int` from that line on, and `x = maybe()` makes it uncertain
again. After `if name is none { name = "anon" }` the name is a `str`, since
it is one on both paths. A loop body forgets, before it starts, what was
known about the variables it assigns, since the body runs more than once.

**Why.** Without this, the two most ordinary uses of `none` do not check:
walking a linked list (`while node is not none { ...; node = node.next }`)
and filling in a default. The programmer would have to copy the value into
a second variable only to satisfy the checker. TypeScript and Kotlin follow
assignments the same way. *(Principles 1 and 3.)*

**Turned down.** Following the value through every path exactly (a full
data-flow analysis, joining at every loop until nothing changes). The three
rules above are a page of code, cover the patterns people write, and when
they are unsure they forget, which is the safe direction: the worst case is
an extra `if`.

**One gap, closed at run time.** The check is trusted for as long as nothing
changes the variable. For a local variable the checker can see that. For a
top-level `var`, or a `var` shared with an inner function (3.6c), a function
that is called between the check and the use could set it back to `none`.
The checker does not follow that, so the running program looks at such a
variable once more where it is used, and stops with an error if the value
has gone. Variables that nothing else can reach, which is most of them, are
not looked at twice. *(Provisional: Kotlin refuses to narrow such variables
at all, which is stricter and needs no check at run time. That may turn out
to be the better rule.)*

### 3.13 A function that promises a value must return on every path

**Why.** In C, falling off the end of a non-void function returns garbage.
In Python it returns `None`, which then fails somewhere else. Here it is an
error at the function. *(Principle 1.)*

A function whose type says it may return `none` (`-> int or none`) is the
exception: reaching its end returns `none`, since that is one of the
answers it promised.

### 3.14 `upper`, `lower`, `strip` and `split` know plain letters and blanks only *(provisional)*

`"straße".upper()` gives `"STRAßE"`: the letters a to z change, and nothing
else does. `strip`, `split()` with no separator, `int()` and `float()` skip
the space, the tab, the two line endings and the form feed, and no other
blank.

**Why.** Doing this for every alphabet needs tables of several kilobytes,
and a compiled Zeno program has to fit on a microcontroller, where that
can be most of the flash. The same program must also give the same answer
under `zeno run` and when built (5.5d), so the interpreter follows the
same rule even though a PC has room for the tables. *(Principle 5.)*

Text in any language still passes through untouched: `len`, indexing and
loops count whole characters, not bytes.

**Provisional.** A separate module can offer the full rules to programs
that have the room. Rust makes the same split, with `to_ascii_uppercase`
beside `to_uppercase`.

---

## 4. Errors

### 4.1 Bugs and errors are different things

An **error** is something that can go wrong in a correct program: bad input,
a division by a value that turned out to be zero, an index from outside.
These can be caught with `try / except`.

A **bug** is a mistake in the program itself: a misspelt name, `"abc" - 1`,
calling something that is not a function. These stop the program and cannot
be caught.

**Why.** Catching a bug hides it. And the type checker reports almost every
bug in the second group before the program runs, so in checked code there is
nothing to catch. *(Principle 1.)*

A firmware build draws the line in a different place: errors that the
runtime finds, such as a division by zero, stop the program there too.
5.5l says why.

### 4.2 Error messages say what to do

`x` declared with `let` and then assigned gives "`x` cannot be changed.
Declare it with `var` to allow that". Habits from other languages (`;`, `&&`,
`def`, `elif`, `if x:`) each get a message naming the Zeno spelling.

**Why.** An error message is the language teaching its user. The moment
something goes wrong is when the explanation is most useful. *(Principle 2.)*

### 4.3 The lexer and the checker report every error; the parser stops at the first

**Why.** The rule is: report an error only if it is certainly real. After a
bad character the lexer can skip it and carry on, so all its errors are real.
After a type error the checker still knows exactly where it is, and it marks
the faulty expression so that nothing built on it is reported again. After a
syntax error the parser no longer knows where it is, and what follows is
usually nonsense caused by the first. One accurate error beats ten misleading
ones.

### 4.4 `zeno run` checks first, and does not run a program with errors

**Why.** A mistake found before the program starts costs seconds. The same
mistake found halfway through a run, with a motor already spinning, costs
more. Untyped code is not held back by this, because the checker only
reports what it is certain of. *(Principle 1.)*

---

## 5. How it is built

### 5.1 The compiler is written in Rust

**Why.** A compiler is mostly "look at a node, decide by its kind". Rust's
enums and `match` fit that exactly, and the compiler forces every kind to be
handled. It also produces one fast binary with no runtime to install.

### 5.2 No libraries: the lexer and parser are written by hand

**Why.** The project exists to learn how languages work. A parser generator
would hide exactly the part worth understanding. Hand-written code also gives
full control over error messages (4.2), and the build has zero dependencies.

### 5.3 The parser is recursive descent

**Why.** It is the technique where the code looks like the grammar: one
function per rule. It is the easiest to read, debug and extend, and it is
what most production compilers (GCC, Clang, Rust, Go) use.

### 5.4 An interpreter first, then a compiler

**Why.** An interpreter is the fastest way to find out whether the language
design works, because programs can be run from the first week. Design
mistakes are far cheaper to fix before a code generator depends on them. It
stays useful afterwards as `zeno run` and a future interactive prompt.

### 5.5 The compiler outputs C before anything else

**Why.** Every microcontroller vendor ships a C compiler, so C output runs
on every target from day one, works with existing debuggers, and can be read
to see what the compiler did. Writing an optimiser can wait.

### 5.5a Compiling to C comes before the bytecode VM

The first roadmap had a bytecode VM next. C was moved ahead of it.

**Why.** A VM makes `zeno run` faster on a PC. C output is what puts Zeno on
a microcontroller, which is the reason the language exists. C also forces
every question about types to be answered now, while the checker is fresh:
it has already changed one rule (2.3b).

### 5.5b `zeno build` keeps the C file and calls the machine's C compiler

`zeno build hello.zeno` writes `hello.c`, then runs `cc` (or whatever `CC`
names) to make `hello`.

**Why.** The C is the compiler's work, shown: reading it is how you learn
what the compiler did, and how you debug it when it is wrong. It is one
file, with the runtime pasted in, so it can be copied into any vendor's
project or IDE with nothing else to carry along.

### 5.5c `zeno build` always checks in strict mode

**Why.** A C variable must have one type. A value whose type is only known
when the program runs would need a tag and usually heap memory (0.2), which
the compiler does not have yet. Because parameters and fields are now worked
out, most programs with no types written still pass. *(Provisional: a
dynamic value type can be added to the runtime later for PC targets.)*

### 5.5d Compiled code must behave exactly like interpreted code

Same output, same errors, same line numbers. The tests compare the two
byte for byte.

**Why.** Two implementations that disagree are two languages. The
interpreter is the definition of what a program means; the compiler is only
allowed to be faster. This is what makes it safe to develop with `zeno run`
and ship with `zeno build`. It has three direct consequences:
- Whole-number arithmetic is checked in compiled code too, at the cost of a
  test after each operation. *(Principle 1. An unchecked mode for hot loops
  can be offered later, asked for by name.)*
- The compiler pins down left-to-right order with temporaries wherever C
  would leave it open. See `docs/compiler.md`.
- Where they cannot agree (recursion depth), the difference is written down.

When the two disagreed while the compiler was being written, and the
compiled answer was the one that followed the rules of the language, it was
the interpreter that was corrected. 3.6c and 3.14 came about that way.

### 5.5e The checked arithmetic uses GCC and Clang built-ins

**Why.** `__builtin_add_overflow` compiles to the processor's own overflow
flag: one instruction. Checking by hand in plain C is longer, slower and
easy to get wrong. GCC and Clang cover nearly every microcontroller
toolchain in use (ARM, RISC-V, AVR, ESP). *(Provisional: a plain-C fallback
can be added for other compilers.)*

### 5.5f Compiled strings were not freed at first

Until the memory management stage, memory for text built at run time was
taken and never returned. **Replaced by 5.7.**

### 5.5g In C, objects, enum values and lists are pointers

**Why.** Zeno says objects and lists are shared, not copied (3.10): after
`let b = a`, changing `b` changes `a`. A C struct is copied on assignment; a
pointer to one is not. So the pointer is the faithful translation. The cost
is that each one needs memory from the heap, which the collector (5.7)
gives back. *(Provisional in one respect: an object that never leaves the
function that made it could live on the stack and cost the collector
nothing. Finding those is an optimisation for later.)*

### 5.5h One list type in C, not one per kind of item

A compiled list holds `zeno_slot`s, a union the size of one machine word,
whatever its items are.

**Why.** The alternative is a full copy of the list code for every kind of
item (`list_int`, `list_float`, `list_Point`, ...), which is what C++ and
Rust do. That is faster by a little and bigger by a lot, and flash is the
scarce thing on a microcontroller. Every Zeno value already fits in one
slot, so one list implementation serves them all. *(Principle 5.)*

### 5.5i `try` compiles to C's `setjmp` and `longjmp`

A `try` remembers a place with `setjmp`. `raise`, and every error the
runtime finds itself, jumps back to the innermost such place with
`longjmp`. `docs/compiler.md` walks through the C.

**This replaces the earlier plan**, which was to compile a function that
can raise into one that returns "a value or an error", with a test after
each call.

**Why the plan changed.** That plan assumed errors come only from `raise`.
But 4.1 lets a program catch a division by zero, a bad index or an
overflow, and 1.17 says those need not be declared, because they can
happen on almost any line. With errors as return values, "almost any line"
means a test after almost every operation, in every function, in case some
caller far above is inside a `try`. That is the hidden cost the plan was
meant to avoid. With `setjmp`, code that is not inside a `try` pays
nothing at all, a `try` in which nothing goes wrong pays for one `setjmp`
(saving a dozen or so registers), and the compiled code for ordinary
arithmetic stays as simple as it was. *(Principle 5.)*

It is not C++ exception handling: there are no tables describing every
function, and nothing is added to code that does not use `try`.

**What it costs.**

- Objects made between the `try` and the `raise` are not tidied up by the
  jump. This turned out to cost nothing: the collector (5.7) finds them
  unreachable and frees them, and no `try` has to remember anything. It
  was one of the reasons for choosing a collector.
- A local variable changed inside a `try` must be declared `volatile` in C,
  or its value after the jump is not guaranteed. The compiler finds those
  variables and does it. They are a little slower to use.
- Some coding standards for safety-critical C, MISRA C among them, forbid
  `setjmp` and `longjmp` outright. A project held to such a standard could
  not use the generated C as it is.

For that last reason there is a second build mode that uses checked
returns, for projects that need it: 5.5l.

### 5.5j `T or none` in C: NULL for pointers, a flag for numbers

A str, list, object, enum value, error or function that may be `none` is
the same C pointer it always was, with `NULL` meaning `none`. A number or
bool that may be `none` is a small struct: the value and a `present` flag.

**Why.** For pointers this is free: no extra memory and no extra code,
because C already has a spare value. Numbers have none to spare (every bit
pattern of an `int64_t` is a real number), so the flag is the least that
will do. Picking a "magic" number such as -1 to mean `none` is exactly the
C habit this type exists to end. *(Principles 1 and 5.)*

The checker has already proved that the value is tested before it is used
(3.12), so the compiled code reads it directly, with no test of its own.

Inside a list, where every item gets one machine word (5.5h), a number that
may be `none` is stored as a pointer to a copy of the number. That costs an
allocation per item, which is acceptable for a rare case and keeps every
other list at one word per item.

### 5.5k A function value is a pointer to its code and its variables

Every function held as a value is a pointer to a struct that starts with
the same two fields (the C function to run, and a name for `print`),
followed by whatever outside variables that function uses. It is always
called the same way: the struct itself is passed as a hidden first
argument.

**Why one shape for all.** `xs.map(f)` cannot know whether `f` is a
top-level function, or one made inside another with three variables in
tow. With one shape and one way of calling, it does not need to.
*(Principle 4.)* The price is an indirect call, and a small wrapper for
each top-level function that is used as a value.

**Variables that never change are copied in; a `var` that is shared lives
in a box on the heap** that both functions point to. Copying is cheaper
and is safe when nothing changes. Only a variable that can change needs
the box, and only when an inner function mentions it, so ordinary local
variables are unaffected. (3.6c is the rule this implements.)

**Each function value is made fresh on the heap**, even one that uses no
outside variables and could have been a single fixed value. Two function
values are equal only when they are the very same one, and with fixed
values a function written in a loop would be equal to itself from the last
trip round, which is not what `zeno run` says. *(5.5d. Provisional: once
memory is managed, the fixed value can come back for functions that are
never compared.)*

**Turned down.** Compiling a separate copy of `map` for every function
passed to it, as C++ and Rust do, makes the fastest code and the most of
it. Flash is the scarce thing here (see 5.5h).

**Not done yet.** A *named* function declared inside another function is
visible from the top of its block, before the variables it uses exist.
That needs the box treatment for every variable of the block, so for now
`zeno build` refuses it and asks for `let name = function(...) { ... }`.

### 5.5l A firmware build carries errors as values, with no `setjmp`

`zeno build --firmware` compiles errors without `setjmp` and `longjmp`.
`docs/firmware.md` walks through the C.

- A function that says `raises` returns `true` or `false`, and gives its
  answer through a pointer the caller passes in. The caller tests what
  came back, and passes a failure on.
- `raise` notes the error in one variable of the runtime and returns
  `false`.
- `try` / `except` becomes an `if`: a failure inside the body goes forward
  to it.
- A function that cannot raise keeps a plain return and no tests. The
  checker already knows which is which, from `raises` (1.17).
- The default build keeps `setjmp` (5.5i). The same source compiles both
  ways, and every compiler test runs both against `zeno run`.

**Why.** MISRA C:2012 Rule 21.4 bans `setjmp` and `longjmp`: the jump skips
whatever the functions in between meant to do on the way out, it can leave
local variables that are not `volatile` with no defined value, and tools
that prove how deep the stack gets, or that every path is covered, cannot
follow it. Firmware that must be certified cannot ship the C that 5.5i
produces. Zeno is meant for that firmware *(principle 5)*, so it needs a
mode that can.

**Why not make it the only way.** 5.5i still holds: on a PC, and on
firmware with no such standard to meet, `setjmp` costs nothing where no
`try` is running, and the generated C is shorter.

**Why one switch called `--firmware`,** and not a setting for errors
alone. The planned build with no collector (5.9) answers to the same
standards and the same people. One switch that means "C a firmware review
will accept" is easier to remember than several that must be combined.

**Why `true` / `false` plus one noted error,** and not the error itself as
the return value. The error has to be somewhere the collector can see it
while it travels (5.7), and it needs the line it was raised on for the
message when nobody catches it. One variable in the runtime does both, and
the return value stays the simplest thing a reviewer can check: a `bool`
that every caller tests.

**Why the answer goes out through a pointer,** and not in a struct beside
the status. A struct per return type is one more type for every function
shape, and C compilers return small structs less well than they pass a
pointer.

**Why a `goto`.** Inside a `try`, a failure has to leave the body from
wherever it is, including from inside loops. The alternative with no
`goto` is a flag tested around every statement that follows, which is
longer, slower and harder to read. The `goto` used only goes forward, to a
label in a block around it: MISRA's required Rules 15.2 and 15.3 allow
that form. Its advisory Rule 15.1 prefers none at all, and a project may
have to record a deviation for it.

**Errors the runtime finds are faults in this mode.** A division by zero,
a bad index, an overflow or `pop` on an empty list stops the program and
no `except` sees it. This was the open question when the stage was
planned, and it is settled this way because the other answer makes every
function one that can fail, with a test after nearly every line: the cost
5.5i was written to avoid, paid everywhere. Rust (`panic` beside `Result`)
and Swift (a trap beside `throws`) draw the same line. It changes 4.1 for
this mode only. So that it never comes as a surprise *(principle 1)*, a
`try` whose `except` could only ever be waiting for a fault is refused
when the program is built, with a message that says to test first.

**One difference that is not refused.** `except` with no kind, around a
body that raises something as a value and can also fault: an ordinary
build catches both, a firmware build catches the first and stops at the
second. Refusing every `except` with no kind would be too strict, so this
is documented and pinned by a test instead.

**Provisional: a function that can raise cannot be used as a value** in a
firmware build. The two sorts of function are called differently, and the
type `function(str) -> int` does not say which sort it is. The fix is to
let function types say `raises`, which the checker needs anyway: today it
does not follow errors through function values in any build.

**What this does not make true.** The C of a firmware build is not thereby
MISRA-compliant. Still to do, each its own piece of work: the heap (Rule
21.3; plan in 5.9), variable argument lists in the runtime (17.1),
`printf` and `exit` (21.6, 21.8), the GCC and Clang builtins (1.2), and the
forward `goto` above (15.1, advisory).

Memory needs nothing special in either mode. The collector frees what an
error leaves behind whichever way the error travels (5.7).

### 5.6 The checker and the interpreter are separate

**Why.** One stage, one job. The checker decides whether a program is sound;
the interpreter runs it and never looks at a type annotation. That is what
lets untyped programs run (0.2), and it means the checker can be made
stricter or smarter without touching how programs execute.

The interpreter still tests types as it goes, because dynamic values need it.
A program that passes the checker should never fail one of those tests on
typed code; the test suite checks this by running every working interpreter
test through the checker as well.

### 5.7 Memory: a mark-and-sweep garbage collector, in the runtime

A compiled program frees memory by itself. Now and then the runtime stops
the program, marks every block of memory that can still be reached from a
variable, and frees the rest. `docs/memory.md` walks through the C.

**Why a collector at all.** Zeno says lists and objects are shared (3.10),
function values remember variables (3.6c), and errors jump out of
functions (5.5i). With those three in the language, "when is this last
used?" has no answer the compiler can work out in general. Something has
to find out while the program runs.

**Why this kind.** There were three real candidates.

- *Ownership, as in Rust.* No cost at run time and no pauses. But it is a
  different language: `let b = a` would move or copy, not share, and the
  checker would need a borrow checker, which is the largest and hardest
  part of Rust. It would undo 3.10 and make the simplest programs harder
  to write. Turned down.
- *Counting references, as in Swift and CPython.* Each block counts how
  many places point at it and is freed when the count reaches zero. Memory
  comes back at once and there are no pauses. But the compiler must add
  code at every place a pointer is copied or dropped, including every early
  `return`, `break` and temporary; two objects that point at each other
  are never freed without extra help from the programmer; and a `longjmp`
  (5.5i) skips the code that lowers the counts, so every `try` would need
  a list of what to release. That is a great deal of generated code and
  many places to get wrong. Turned down for now.
- *Mark and sweep.* The compiler adds almost nothing: one generated
  function that lists the top-level variables. Cycles are freed like
  anything else. An error needs no special care. The whole collector is
  about 200 lines in one place, small enough to read in one sitting, which
  matters in a compiler that is built to be learned from. Chosen.

**Why it reads the stack instead of being told.** A collector that knows
exactly which variables are pointers needs the compiler to keep a second,
shadow stack of them, or a table for every point in every function. Both
are far more machinery than the collector itself. Reading every word of
the stack and asking "is this one of my blocks?" needs none, and is what
MicroPython does on the same class of microcontroller.

**What it costs**, stated plainly.

- *It bends principle 5.* A collector is exactly the "hidden runtime" that
  principle warns about: work the program did not write, done at a moment
  it did not choose. What keeps it within bounds is that a program which
  makes no blocks never runs it, and the first provisional point below.
- *Pauses*, in proportion to the memory in use. Not suitable inside a loop
  with a hard deadline. Code that uses only numbers, bools and fixed text
  never allocates and so never pauses.
- *It depends on how C compilers behave*, not on what the C standard
  promises. Reading the stack this way is outside the standard. It is the
  same bet Boehm's collector, Ruby and MicroPython make.
- *It can keep too much.* A leftover value in a stack slot, or a number
  that looks like an address, holds a block that is really garbage. This
  wastes memory and breaks nothing. (Seen in testing: built without
  optimisation, one stale slot kept a 300,000-node list alive. With `-O2`,
  which is what `zeno build` uses, it was freed.)
- *MISRA* bans `malloc` and `free` as well (Rule 21.3), collector or not.
  See the first point below.

**Provisional, in three respects.**

1. Certified firmware usually allows no heap at all. The answer for it is
   not a different collector but a build that proves a program makes no
   blocks, and says which line does if it cannot. The checker already
   knows where blocks are made. This is planned as 5.9.
2. The blocks come from the C library's `malloc`. On a real board the
   collector should own one fixed region and hand out pieces of it itself.
3. If pauses turn out to matter in practice, counting references can be
   added underneath without changing the language, since nothing in the
   source depends on which method is used.

### 5.8 Owned values and borrows, beside the collector

Values stay shared and collected by default (3.10, 5.7). A value can
instead be **owned**. `docs/ownership.md` is the walkthrough.

- It has exactly one owner. Storing it somewhere else *moves* it: the old
  name can no longer be used.
- It is freed at the end of its owner's block, at a line the programmer
  can point to, without a collection.
- It can be *borrowed* for the length of a call: the function may use it
  and must not keep it. A borrow cannot outlive the call it was passed to.
- **An owned value may hold shared values. A shared value may not hold an
  owned one.** Otherwise a shared value could still point at an owned one
  after its owner had freed it.

**Why.** 5.7 lists what the collector costs: pauses, and memory that comes
back at no particular moment. For most code that is a good trade. For a
buffer in an interrupt handler or a control loop it is not. This lets the
programmer pay for precision only where it matters, and write everything
else the easy way. *(Principles 1 and 5.)*

**Why opt-in, and not ownership everywhere.** 5.7 turned that down: it is
a different, harder language. Opt-in keeps the first program a beginner
writes free of it.

**Precedent.** D collects by default and has `@nogc` and `scope`. Nim
collects by default and has moves (`sink`) and borrows (`lent`). Swift
counts references by default and has added `consuming`, `borrowing` and
values that cannot be copied. Mojo has `owned` and `borrowed` arguments.
All four arrived at "automatic by default, exact where asked for".

What was settled when it was built, each with its reason:

#### 5.8a The words are `owned` and `borrow`, and only three places take them

`owned class Name`, `owned list[T]`, and `borrow` in front of a parameter's
type. They are whole words *(principle 3)*, and they are not reserved: they
mean something only in those positions, so no existing program breaks.

*Turned down:* `&x` and `&mut x` at each use, as in Rust. Punctuation, and
it puts a mark at every call where one in the signature says the same.
*Turned down:* a mark at the call (`send(move p)`). It would make each
move visible where it happens, which is worth something, but the checker
already reports any use after a move by line, and the calls stay plain.
Provisional: if moves prove easy to miss when reading, this is the first
thing to revisit.

#### 5.8b A class is owned as a whole, where it is declared

Not `let p: owned Packet` at one use and a shared `Packet` at another.

**Why.** If the same class could be owned here and shared there, a method
could not know whether `self` may be kept, and an owned object could be
handed to code that shares it. One word on the class settles it for every
use, and it is the first thing a reader of the class sees.

Lists are the exception, because there is only one list type: there the
word goes on the type, `owned list[int]`.

#### 5.8c A parameter takes the value unless it says `borrow`

`function send(p: Packet)` takes `p` over. `function look(p: borrow Packet)`
only looks.

**Why.** A parameter is a variable. `p: Packet` on a `let` means "this
variable holds a Packet", and a variable that holds an owned value owns it.
The parameter means the same. The exception is the one that gets a word.
A method's `self` is always a borrow, since calling a method on an object
that then vanished would surprise everyone.

*Turned down:* borrow by default, with a word for taking (Mojo, Swift). It
makes the common case shorter. But then `p: Packet` would mean two
different things on a `let` and on a parameter. Provisional.

#### 5.8d The borrow checker is four rules, and stops there

1. A borrow exists only as a parameter. It cannot be stored, returned, or
   passed to something that keeps it.
2. One statement cannot both use an owner and give it away.
3. An owner declared outside a loop cannot be given away inside it, except
   on a path that leaves the loop.
4. An owned variable can be used only by the function whose block holds
   it. Not by another function, not even when it is at the top level of
   the file, and not by a function written inside this one.

**Why so few.** Rule 1 is what makes lifetimes unnecessary: a borrow that
only lives for a call needs no name for how long it lives. Rule 4 is what
makes freeing at the end of the block safe: nothing outside ever knew where
the value was. Rules 2 and 3 close the two ways a value could be used after
the same piece of code gave it away. Together they are about 200 lines of
the checker.

**What it costs.** Programs that would be safe are refused: a function
cannot return one of two borrowed lists, and `f(p.size, p)` must be two
lines. Rust accepts these and needs lifetimes, two kinds of borrow and
flow analysis over a second form of the program to do it. That is the part
of Rust most often named as hard, and Zeno keeps the collector for the
cases that need the freedom. *("Start with the minimal borrow checker and
grow it only if needed.")*

#### 5.8e A place that forgets about ownership gets an error, not a second owner

Reading an owner gives a type, `Borrowed`, that fits nowhere. Only the
places that are known to merely look take it off, and only the places that
are known to keep a value mark the owner empty.

**Why.** The checker has dozens of places where a value goes somewhere, and
more will be added. The two possible mistakes are not equally bad. A place
wrongly treated as keeping gives the programmer a confusing error. A place
wrongly treated as looking gives the compiled program two owners, and it
frees the value twice. So the default is the first.

#### 5.8f In C, giving a value away sets the variable to NULL

And every free does nothing when handed NULL. The end of a block calls the
free for each owner declared in it, moved or not.

**Why.** The compiler then needs no knowledge, at the end of a block, of
which owners still hold something: the pointer is the answer. `if` branches
that move on one side and not the other need nothing special.

*Turned down:* working out at build time exactly which owners are live at
each exit, and emitting only those frees. The checker has that knowledge
for errors, but passing it to the compiler exactly, for every exit of every
block, is far more machinery, and the C compiler removes a free of a
variable it can see is NULL.

#### 5.8g An owned value is still one of the collector's blocks

`zeno_free` takes the block out of the collector's table at once. Until
then the block is in the table like any other.

**Why.** Two reasons. The collector must look inside an owned value while
it lives, since it may hold the only pointer to a shared one, and a block
in the table is looked into with no extra code. And if a free is skipped,
the collector frees the block later, so a skipped free costs time and
never correctness.

*Turned down:* taking owned memory straight from `malloc`, outside the
table. The collector would then need a list of every live owned value to
look inside, kept up to date at every move.

A block finds its own line in the table (it keeps its index in the field
the collector's waiting list uses, since the two are never needed at
once), so freeing costs the same however many blocks there are.

#### 5.8h Errors: an ordinary build frees what it can, a firmware build frees everything

In an ordinary build an error is a `longjmp` (5.5i). The function that
raises frees its owners first. The functions the jump passes through free
nothing, and what they owned waits for the collector (5.8g). An owner in a
function with a `try` is declared `volatile`, because C does not promise
that a local changed since `setjmp` keeps its new value after the jump,
and an owner that forgot it had given its value away would free it twice.

In a firmware build an error is a return value (5.5l), and each failed
call frees the block's owners before it passes the failure on.

**Why not make the ordinary build exact too.** It would need what 5.7
turned down for counting references: every `try` keeping a list of what to
release. The collector is already there for it.

#### 5.8i What is refused for now *(provisional)*

- An owned value inside a list, an `or none`, an enum case or a function
  type, and moving a field out of an object. Each needs a rule for what is
  left behind in the hole.
- Replacing an owned field through a borrow, including `self`. Somebody
  else may have been lent the old value at that moment. Telling "may
  change" borrows from "may only read" borrows would lift this, and is the
  first growth to consider (5.8d).
- A shared list becoming an owned one. Only a list written out or a
  comprehension can start life owned: those are the only lists certain to
  have no other name.
- `match` on an owned value, raising one, and a method of one kept as a
  value.

Each is an error with a message that says what to do instead. Allowing any
of them later breaks no program that works now.

#### 5.8j No escape analysis; two frees that are always safe instead

The plan mentioned freeing shared values that never leave the function
that made them. That was not built: working out "never leaves" for a
shared value means following it through every call it is passed to.

Two cases need no analysis at all, and were built: a list that grows frees
its old storage, and the copy a `for` loop walks (3.10a) is freed after
the loop. Only the list ever pointed at either. Every program gets these,
with or without the word `owned`.

### 5.9 A firmware build with no collector

`zeno build --no-collector` writes C with no garbage collector in it.
Every list and object in the program must be owned (5.8). Whatever else
would take memory that no line gives back is refused when the program is
built, with the line that does it. `docs/no-collector.md` is the
walkthrough.

**Why.** A collector pauses the program for a time nobody can promise,
finds its roots by reading the C stack (which no C standard describes),
and frees a block at a moment nobody can name. Firmware with a deadline
or a memory budget cannot always accept those. With 5.5l (no `setjmp`)
this is the build such a project can start from. *(Principle 5: this is
where "no hidden runtime" comes closest to being true without an
exception.)*

**Why after 5.8.** Without owned values, "no collector" would mean "no
lists and no objects", which is too little to write real firmware in.

**What it does not do yet.** It still takes owned values from the heap,
and MISRA C bans `malloc` and `free` (Rule 21.3). The earlier plan for
this decision said the build would answer that rule. It does not, and the
walkthrough says so. What it gives is a `free` for every allocation at a
line that can be pointed to. See 5.9b.

### 5.9a `--no-collector` is a firmware build

The flag turns `--firmware` on.

**Why.** In an ordinary build an error jumps (5.5i), and the jump skips
the lines that free owned values in the functions it passes through. The
collector picks those up (5.8h). With no collector they would leak, and
the only fix is the one a firmware build already has: an error that
travels as a return value passes every one of those lines.

**Turned down:** a separate, fourth kind of build with jumps and no
collector. It could not be made sound without unwinding tables, which is
C++ exceptions by another name.

### 5.9b Owned values use `calloc` and `free`, for now *(provisional)*

**Why.** It is the least that works, and it is honest about what it is.
The compiler's part of the job, a `free` on every path, is the same
whatever sits underneath.

**Turned down for now:** a fixed pool in the runtime. That is the right
answer for a board with no C library heap, and for Rule 21.3. It is a
change to about twenty lines of runtime.c that touches nothing in the
compiler, so it can come when a real board asks for it. It is also a good
exercise, and the walkthrough sets it as one.

### 5.9c Built text lives for one statement, in a room of fixed size

Text built while the program runs (`f"..."`, `str(n)`, `a + b`,
`.upper()`) is written into one fixed array and given back when the
statement that built it is over. It can be printed, compared and
measured. It cannot be kept: not in a variable, a field or a list, not
returned, and not passed to one of the program's own functions.

**Why text needs an answer at all.** Firmware prints. A build that
refused `print(f"temperature {t}")` would be of no use.

**Why one statement.** It is the shortest life that still lets text be
built and used, and it has an end the compiler can see without following
the text anywhere. Giving it back is setting one number, so there is
nothing to search and nothing to fragment. Calls nest inside statements,
so the room behaves as a stack, and its size is the answer to "how much
text can be in the making at once", which the report measures.

**Why it cannot be passed to a function.** The function could keep it.
Knowing that it does not is the borrow checker's question, asked of
text. That can be added later (`borrow str`) if the rule proves too
tight.

**The cost.** A program cannot build a line and keep it for later. It
keeps the numbers and builds the line again. Every `str` that is stored
is text written in the program, which is also what makes the rule cheap
to check: one flag on a compiled value.

**Turned down:** owned text, moved and freed like an owned list. It is
the complete answer and a much bigger one, since text is copied about far
more freely than lists are (`let a = b` would have to move). **Also
turned down:** printing piece by piece with no text at all. It only
helps `print`, and `if name.upper() == "ON"` still needs somewhere to put
`"ON"`.

### 5.9d The `except` that catches an error frees it

An error is made only by `raise`. The `except` block that catches it
frees it when the block ends, however the block is left. Inside the
block the error can be read. It cannot be kept, passed to a function or
raised again.

**Why.** An error has to outlive the function that raised it, so no
block's end in that function can own it. The one place every caught
error is certain to reach is its `except`, and after that block nothing
can refer to it, as long as it was not kept. So the block's end is a line
that can free it. An error nobody catches stops the program, and needs
no freeing.

**Why the message is copied into the error.** The message is often built
text, which lasts one statement (5.9c). The error keeps its own copy, in
the same block, so one `free` gives back both.

**The cost.** `raise e` to pass a caught error on is refused. Raise a new
one. *(Provisional: passing it on could be a move.)*

### 5.9e The compiler refuses, not the checker

What needs a collector is found by `emit_c.rs`, at the spot where each
kind of value is made.

**Why.** Whether a program needs a collector depends on the build, and
the checker checks the language, which is the same in every build. The
same program is right for `zeno run` and wrong for `--no-collector`. It
also keeps the list honest: a refusal sits beside the line that would
have written the allocation, so a new way to allocate cannot be added
without meeting it. The checker helps in one place, where it already
knew the answer: which lists start life owned (`Types::fresh`).

### 5.9f A value between two owners is held

A list being filled by a comprehension, an owned value on its way into a
call, and the copy a `for` loop walks (3.10a) each belong to nobody for a
moment. If the block is left early in that moment, the value is freed
with what the block's owners hold.

**Why.** 5.8 left these three to the collector. Here there is none, so
each needed a line. One mechanism covers all three: the compiler lists
the value among the block's owners until it arrives. The first and third
apply to every build, so every build is a little tidier for it.

### 5.9g What a build with no collector refuses *(provisional)*

Shared lists and objects; enum cases that carry values; functions written
as values; `split`, `map`, `filter` and `+` on lists; `for` over the
characters of a str; lists of numbers that may be `none`; and a list
written where it is only looked at.

**Why refuse and not support.** Each could be given an owner or held for
a moment, and each would be more machinery. None was needed to build the
example or the tests, so none was built. *(Principle: keep what works.)*
The error for each names the line, and where there is one, the thing to
write instead.

### 5.10 A bytecode VM, beside the interpreter

`zeno run --vm` compiles a program to bytecode and runs that. The
walkthrough is `docs/vm.md`.

**Why build one.** Three reasons, in order of weight.

1. It is the next thing to understand. Python, Java, C#, Lua, Ruby and
   every JavaScript engine run programs this way. Stacks, frames,
   closures over cells and handler tables are the working parts of all
   of them, and they are easier to see in fifty instructions of your own
   than in someone else's five hundred.
2. It is a second, independent reading of what a Zeno program means. Two
   engines that must agree find mistakes that one engine and its tests
   do not. It found one the day it first ran (3.6d).
3. It makes `zeno run` faster: about three times on calls, twice on
   loops. That matters least. Speed on a PC was never the point of Zeno,
   and `zeno build` is far faster than either.

**Why now, after the C backend and not before.** Decision 5.5a.

### 5.10a A stack machine

Instructions take their inputs from a stack and leave their result on
it. `a + b` is "push a, push b, add".

**Why.** It is the simplest thing to compile to. The code for an
expression is the code for its parts followed by one instruction, with
no decision to make about where anything goes. The JVM, CPython and
WebAssembly are stack machines for the same reason.

**Turned down:** a register machine, where instructions name the slots
they read and write (`add r2, r0, r1`), as in Lua 5 and Dalvik. It needs
fewer instructions for the same work and is faster. It also needs the
compiler to choose a register for every temporary, which is a problem of
its own. The LLVM stage will meet that problem properly.

### 5.10b One meaning, written once

The VM uses the interpreter's `Value`, its built-ins, and the same
functions for every operator, field access, index, call binding and
error rule. Those functions moved out of the interpreter into
`src/ops.rs`, and the built-ins now take a `Machine` (the two things
they need from an engine) and not the interpreter.

**Why.** "Compiled code must behave exactly like interpreted code"
(5.5d). For the C backend that had to be earned test by test, because C
cannot call Rust. Here it can be true by construction: if both engines
call `ops::binary`, they cannot disagree about `7 // -2`. What is left
for the VM to get wrong is control flow and where variables live, and
that is what its tests aim at.

**Turned down:** a compact value of the VM's own (8 bytes, with small
numbers and pointers packed into one word, as fast VMs do). It is where
most of the missing speed is. It would also mean two of everything, and
two places for each to go wrong.

### 5.10c An instruction is a Rust enum, not bytes *(provisional)*

`Op::GetLocal(3)` is a value of an enum, 12 bytes wide, in a `Vec<Op>`.

**Why.** The Rust compiler checks every `match` on it for a missing
case, an operand cannot be read with the wrong width, and the printer is
nearly free. Real bytes would make the loop a little faster and the code
a good deal harder to read, for a file format nobody needs yet.

**When it changes:** when compiled programs are saved to disk, or a VM
has to fit on a microcontroller. Both want the code to be bytes. It is
the last exercise in the walkthrough.

### 5.10d Variables are stack slots; shared ones are cells

A variable used only by its own function is a slot of the stack, known
when the program is compiled. A variable that a function written inside
also uses lives in a cell on the heap, which both hold. Which is which
is decided by name: if any inner function mentions `count`, every
`count` in the enclosing function gets a cell.

**Why slots.** A read is one index into an array. No name exists while
the program runs.

**Why cells.** 3.6c: the inner function can outlive the call that made
the variable, and both must see one variable, so it cannot stay on the
stack. The C backend made the same choice for the same reason (5.5k).

**Why by name.** It is one walk over the function before compiling it,
and it can only err toward a cell that was not needed, which is slower
and never wrong.

**Turned down:** Lua's upvalues, where a shared variable stays on the
stack until its function returns and is moved to the heap only then.
Faster, and a good deal more machinery. **Also turned down for now:**
working out exactly which variable each inner function uses. It is an
exercise.

There are no global variables. The program is a function called `main`,
and a top-level variable that a function uses is a variable of `main` in
a cell. One mechanism, not two.

### 5.10e The bytecode compiler never refuses a program

A mistake it can see while compiling (a name that does not exist, an
assignment to a `let`, `break` outside a loop) becomes a `Fatal`
instruction at the place where the interpreter would have found it.

**Why.** The interpreter runs a program up to its mistake and then
stops. If the VM refused the whole program, the two would differ for
every program with a mistake in it, and the tests that compare them
could not cover those programs. Refusing programs is the checker's job
(4.4), and `zeno run --vm` runs the checker first.

### 5.10f Every test program runs on both engines

`vm::run_both` runs a program on the interpreter and on the VM and
requires the same output and, when it stops, the same message at the
same line and column. The interpreter's tests and the C compiler's
tests go through it.

**Why.** It is the cheapest strong test there is: about two hundred
programs that were already written, each now checking the VM with no
expected output to write. And it works in both directions, as 3.6d
showed.

### 5.10g The interpreter stays the default *(provisional)*

`zeno run` uses the interpreter unless given `--vm`.

**Why.** The interpreter is the plainest statement of what Zeno means:
a `while` loop is run by a `while` loop. It is what the VM and the C
backend are compared against, so it should be the one people meet first
and the one that is hardest to break. The VM is new.

**When it changes:** when the VM has run everything for a while without
a difference turning up, the default can flip, and the interpreter stays
as the reference behind a flag.

### 5.10h A Zeno call does not use the Rust stack, with one exception

A call adds a frame to a list. The loop does not call itself.

**Why.** It is what makes an error cheap to unwind (drop frames) and
what would let a later stage pause a function and come back to it, for
generators or `async`. The interpreter cannot do either without help
from Rust's own stack.

**The exception** is a built-in that calls a Zeno function, such as
`map`. It is Rust code, so the loop is started a second time underneath
it. **Turned down:** making `map`, `filter` and `reduce` instructions of
their own so that they never leave the loop. It would be right for
`async`, and it is not needed until then.

### 5.11 An LLVM backend, beside the C backend

`zeno build --llvm` writes the program as LLVM IR and hands it to clang
(`docs/llvm.md`). `zeno build` without it still writes C.

**Why.** IR is what clang turns C into, so writing it directly takes
one language out of the path. The order of evaluation, overflow and
temporaries, which `emit_c.rs` works hard to get right in C, are plain
in IR. LLVM then gives the optimiser and code for every processor it
knows, WebAssembly included, which the roadmap wants later.

**Turned down:** replacing the C backend. Firmware is built with the
vendor's C compiler, a reviewer reads C, and MISRA is a standard for C.
The two backends are for two jobs.

### 5.11a The IR is written as text, with no LLVM library

`emit_llvm.rs` builds a `.ll` file with `format!`. Zeno still has no
dependencies.

**Why.** The text is the lesson: every instruction the backend emits can
be read in the file it writes, and tried by hand with `opt` and `clang`.
**Turned down:** a binding to LLVM's own library (`inkwell`, `llvm-sys`).
It needs the LLVM development files of one exact version installed
before Zeno itself will build, and it hides the IR behind method calls.
**Cost:** clang must be installed to get a program, mistakes in the IR
are found only when clang reads it, and the text form changes between
LLVM versions. This one needs LLVM 15 or later.

### 5.11b A `let` is a value and a `var` is a slot; LLVM builds the SSA

A name in IR is given a value once. A `let` and a parameter are that
already, so they are IR values with nothing more to do. A `var` gets an
`alloca`, read with `load` and written with `store`, and LLVM's
`mem2reg` pass turns it into values and `phi`.

**Why.** Placing `phi` for variables that change in loops and branches
is the hard part of building SSA, and LLVM has it. Every front end on
LLVM does this. Zeno gets something extra from its own rule that `let`
cannot change: most variables need no slot at all, and the IR reads
close to the program.

`phi` is written by hand in two places, `and`/`or` and `a < b < c`,
where the shape is always the same.

### 5.11c Overflow is checked in the IR; the rest of arithmetic calls the runtime *(provisional)*

`+`, `-` and `*` on ints use `llvm.sadd.with.overflow` and its
relations. `//`, `%`, `**` and `/` call `zeno_floordiv`, `zeno_mod`,
`zeno_pow` and `zeno_div` in `runtime.c`.

**Why.** The three intrinsics are one instruction each on the processor
and are the commonest arithmetic there is. The others have rules of
their own (which way `//` rounds, the sign of `%`) that the runtime
already has right and tested, and a second copy in IR is a second place
to get them wrong.

**Cost, measured:** the runtime is in another file, so those calls are
not inlined. A loop of 300 million `%` takes 0.76 s where the C build
takes 0.29 s. A recursive `fib(38)` takes the same 0.20 s either way.
**When it changes:** when a real program is slow because of it. Then
`%` and `//` are written out in IR, or the two files are optimised
together (`-flto`).

### 5.11d The runtime is the C backend's, with `static` taken off *(provisional)*

An LLVM build links to the same `runtime.c`. `emit_llvm::runtime()`
removes `static` from the start of each line, so that the functions can
be called from the IR file, and `zeno build --llvm` writes the result
beside the IR as `name_runtime.c`.

**Why.** One runtime means text, printing, float formatting and the
collector behave the same in both backends with nothing to keep in
step. Writing the file out, instead of piping it to clang, lets the
build be repeated by hand, which the exercises need.

**Turned down:** a macro in `runtime.c` in place of every `static`
(a change to 150 lines for the same result), and a runtime rewritten in
IR. **Cost:** the whole runtime is linked in, used or not, because
without `static` the compiler cannot tell. A build is about 20 KB
bigger.

### 5.11e The first version compiles what the C backend began with *(provisional)*

Numbers, bools, text, functions, `if`, `while`, `for` over a range,
f-strings, and the built-ins and str methods that need only those.
Everything else is refused where it is met: "`zeno build --llvm` cannot
compile a class yet. `zeno build` without `--llvm` can".

**Why.** It is the smallest part that teaches all of IR's ideas
(types, blocks, SSA, `phi`, calls into C) and makes a real program:
`examples/primes.zeno` builds. The C backend grew the same way, in
stages 5 and 6. Classes need `getelementptr`, errors need a choice
between the two ways `emit_c` has, and each is a stage of its own size.

### 5.11f The IR names no processor, and assumes 64 bits

The file has no `target triple` and no `datalayout`. clang supplies the
machine it is running on.

**Why.** One `.ll` then builds on Linux and macOS, x86 and ARM, with
nothing to detect. **Cost:** the IR does take a pointer and `size_t` to
be 64 bits wide, so it is not for a 32-bit microcontroller. The C
backend is.

### 5.11g An LLVM build must print what the interpreter prints

The tests in `emit_llvm.rs` run each program through `vm::run_both`,
build it with clang, and require the same output and the same error at
the same place, with the collector running before every allocation.

**Why.** The reason is the one in 5.10f: the programs are already
written, and a third engine that disagrees with the other two is wrong
somewhere.
