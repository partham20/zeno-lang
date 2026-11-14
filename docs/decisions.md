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

They are not exceptions in the C++ sense. The plan is to compile a raising
function to one that returns "value or error", so there is no stack
unwinding and no hidden cost. *(Principle 5.)*

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
thing. Whether Zeno should also offer deeply unchangeable data is a memory
management question, to be settled at that stage.

### 3.10 Lists and objects are shared, not copied *(provisional)*

`let b = a` makes `b` the same list as `a`.

**Why.** It is what Python, Java and JavaScript programmers expect, and it is
the cheapest thing for the interpreter. Whether sharing, copying or Rust-style
ownership is the right default for the compiled language is the central
question of the memory management stage.

### 3.10a A `for` loop sees the list as it was when the loop began

Adding to a list, or removing from it, inside a loop over that list does
not change what the loop visits.

**Why.** The other choice, looping over the list as it changes, is the
source of two classic bugs: a loop that never ends because its body keeps
appending, and a loop that skips an item because its body removed one.
*(Principle 1.)* **The cost:** the loop takes a copy of the list first.
*(Provisional: the copy can be skipped when the compiler can see that the
body does not touch the list.)*

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

### 3.13 A function that promises a value must return on every path

**Why.** In C, falling off the end of a non-void function returns garbage.
In Python it returns `None`, which then fails somewhere else. Here it is an
error at the function. *(Principle 1.)*

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

### 5.5e The checked arithmetic uses GCC and Clang built-ins

**Why.** `__builtin_add_overflow` compiles to the processor's own overflow
flag: one instruction. Checking by hand in plain C is longer, slower and
easy to get wrong. GCC and Clang cover nearly every microcontroller
toolchain in use (ARM, RISC-V, AVR, ESP). *(Provisional: a plain-C fallback
can be added for other compilers.)*

### 5.5f Compiled strings are not freed yet

**Why.** Memory management is its own stage, and guessing at it now would
mean redoing it. Until then, memory for strings built at run time is taken
and never returned. Programs that use only numbers, bools and fixed strings
take no memory at all. *(Provisional, and not acceptable for firmware that
builds strings in a loop.)*

### 5.5g In C, objects, enum values and lists are pointers

**Why.** Zeno says objects and lists are shared, not copied (3.10): after
`let b = a`, changing `b` changes `a`. A C struct is copied on assignment; a
pointer to one is not. So the pointer is the faithful translation. The cost
is that each one needs memory from the heap. *(Provisional, with 3.10: a
value that never changes and is never shared could live on the stack, and
the memory stage is where that gets decided.)*

### 5.5h One list type in C, not one per kind of item

A compiled list holds `zeno_slot`s, a union the size of one machine word,
whatever its items are.

**Why.** The alternative is a full copy of the list code for every kind of
item (`list_int`, `list_float`, `list_Point`, ...), which is what C++ and
Rust do. That is faster by a little and bigger by a lot, and flash is the
scarce thing on a microcontroller. Every Zeno value already fits in one
slot, so one list implementation serves them all. *(Principle 5.)*

### 5.6 The checker and the interpreter are separate

**Why.** One stage, one job. The checker decides whether a program is sound;
the interpreter runs it and never looks at a type annotation. That is what
lets untyped programs run (0.2), and it means the checker can be made
stricter or smarter without touching how programs execute.

The interpreter still tests types as it goes, because dynamic values need it.
A program that passes the checker should never fail one of those tests on
typed code; the test suite checks this by running every working interpreter
test through the checker as well.
