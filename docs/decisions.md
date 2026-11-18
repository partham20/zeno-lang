# Design decisions

Every choice in Zeno, and the reason for it. If a choice has no good reason,
it should not be here; and if a better reason turns up for the opposite
choice, the choice should change.

Each entry says what was decided, why, and what was turned down. An
entry marked *was provisional* was decided on less than it deserved
at the time. Each of those was read again before version 0.1, and
chapter 11 says what became of it.

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
- Classes have no inheritance yet *(was provisional (11.1), see 3.7)*.

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
without the programmer having asked for it. That is what generics are for,
and there the programmer does ask: `function id[T](x: T) -> T` (3.16).

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

**Was provisional (11.1).** General unions (`int or str`) are not accepted. They overlap
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
them. Both pairs are kept apart on purpose. *(Changed in 6.22: `.rad` and
`.deg` now change an angle when the program asks. The two still never
meet by themselves.)*

**Was provisional (11.1).** No derived units yet (`V * A` is not `W`), and no
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

There is one `int`, of 64 bits. The sized types (`u8`, `i32`, ...) and
wrapping on purpose, which firmware needs for counters and checksums, are
in 2.8.

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

### 2.7 Bit operators are symbols: `&`, `|`, `^`, `~`, `<<`, `>>`

They work on whole numbers only, with `&=`, `|=`, `^=`, `<<=` and `>>=`
beside them. An int is 64 bits in two's complement, so `~x` is `-x - 1`
and `-1 & 0xFF` is `255`.

**Why symbols, when 1.5 chose words.** Principle 2 says a word beats a
symbol, and principle 3 lets a mark stay when it earns its place. These
do. `status & MASK` is how every datasheet, every C header and every
Python script writes it, and a firmware engineer reads it faster than
any word. The words tried were `bit_and(status, MASK)`, which buries a
register expression in brackets, and infix `band` / `bor` / `shl`,
which nobody has seen before. 1.5 kept `&` and `|` free for exactly
this.

**The C slip is caught.** `&&` and `||` are still lexer errors with a
hint (`and`, `or`). `true & false` is a checker error that says the
same: the operators take whole numbers, never bools. So `&` never means
"and", which is the confusion 1.5 was about.

### 2.7a Their precedence is Python's, not C's

From loosest: `or`, `and`, `not`, comparisons, `|`, `^`, `&`, shifts,
`+ -`, `* / // %`, unary `-` and `~`, `**`.

**Why.** In C, `flags & MASK == 0` means `flags & (MASK == 0)`, which is
almost never what was meant and compiles without a word. C's own
authors called it a mistake that was too late to fix. Python put the
bit operators above the comparisons, so the line means what it looks
like. Zeno copies Python, as it does wherever Python's spelling passes
the principles. *(Principle 1.)*

**Cost.** Someone who knows C's table by heart has one row to
unlearn. The row they unlearn is the one that bites.

### 2.7b A shift is checked

`a << n` is `a` times two, `n` times, and like `*` (2.2) it raises
`OverflowError` when bits would be lost. `a >> n` rounds down, so
`-5 >> 1` is `-3`, the same as `-5 // 2`. A count outside 0 to 63 is a
`ValueError`.

**Why.** In C a shift by 64 or by a negative count is undefined, and
`1 << 40` on a 32-bit int is a wrong number. Each of the three is a
quiet wrong answer. *(Principle 1.)* Checking costs a comparison or two.

**Turned down.** Python's shift, which never overflows because its
ints grow: Zeno's do not (2.2). Java's, which uses only the low six
bits of the count, so `1 << 64` is `1`: that is the quiet wrong answer
again. To throw the top bits away on purpose, say so: `u32.wrap(x <<
4)` (2.8b).

### 2.8 Sized whole numbers: `u8`, `u16`, `u32`, `i8`, `i16`, `i32`

`let level: u8 = 200`. Six types that say a whole number fits in
fewer bits than an `int`. They are for where the size is real: a field
of an object, a byte in a frame, a register.

**Why.** A protocol says "one byte", and a struct on a microcontroller
with 2 KB of memory cannot spend eight bytes on a flag. With the size
in the type, the checker knows that a `u8` field is never handed 300,
and the compiler can store it in one byte (5.13a). *(Principles 1 and
5.)*

**Why these names.** They are Rust's, and close to C's `uint8_t` with
the noise taken off. `byte` and `short` say less: how long is a
`short`?

### 2.8a Arithmetic on sized numbers gives an `int`

`a + b` on two `u8`s is an `int`. So are `a << 8`, `~a` and `-a`. To
put the answer back in a `u8`, say how: `u8(a + b)` or `u8.wrap(a + b)`.

**Why.** There are three other choices, and each one hides something.

- *Wrap at the size*, as C does after promotion and Rust does in a
  release build: `200 + 100` is `44`, quietly. That is the bug 2.2
  exists to stop.
- *Check at the size*, as Rust does in a debug build: `(a + b) // 2`
  fails for two `u8`s whose average fits perfectly well. The size of
  the storage has leaked into the middle of a sum.
- *Refuse to mix sizes*, as Rust does: `length + offset` needs a
  conversion on every line, and people stop reading them.

With the sum done on ints, the middle of an expression cannot
overflow at 255, and the one place where a number may not fit is the
place it is stored, where the program has to say what it wants. This
is C's integer promotion, kept, without the silent cut at the end.
*(Principles 1 and 4.)*

**Cost, at first.** `count += 1` on a `u8` did not compile: the sum
is an int, and an int does not go into a `u8` by itself. Programs ran
into it at once, and 10.27 took the cost away without changing this
rule: `count += 1` now means `count = u8(count + 1)`. The sum is still
an int, and it is checked where it is stored.

### 2.8b `u8(n)` checks, `u8.wrap(n)` cuts

`u8(n)` is `n` if it fits and an `OverflowError` if not. `u8.wrap(n)`
keeps the low eight bits and never fails: `u8.wrap(300)` is `44`, and
`i8.wrap(200)` is `-56`.

**Why two.** Both are needed, and they must not look alike. A length
that does not fit in its field is a bug. A checksum is *meant* to lose
its top bits. In C both are written `(uint8_t)x`, and the reader has
to guess which was intended. Here the short spelling is the safe one,
and wrapping is a word in the program that a reviewer can search for.
*(Principles 1 and 2.)*

**Turned down.** Wrapping operators such as `+%` (Zig) or methods such
as `a.wrapping_add(b)` (Rust): one more of each for every operator.
One `wrap` after ordinary arithmetic does the same job, because the
low bits of a sum do not depend on when the top ones are dropped.

In a firmware build, a `u8( )` that does not fit stops the program,
like any overflow (5.5l).

### 2.8c A written number adapts; a sized number is an `int` for free

- `let b: u8 = 7`, `f(7)`, `[1, 2, 255]` where a `list[u8]` is wanted:
  a number written out takes the size of where it goes. The checker
  sees whether it fits, and `let b: u8 = 300` is an error before the
  program runs.
- A `u8` goes wherever an `int` is wanted, with nothing written. Every
  `u8` is an `int`.
- An `int` does not go where a `u8` is wanted, and a `u8` does not go
  where a `u16` is wanted. Each needs `u8( )` or `u16( )`.

**Why.** The first is the only bearable way to write a constant, and
it is checked at no cost when the program runs. The second cannot
lose anything, so asking for it would be noise. The third can lose
something, so it is written. *(Principles 1 and 3.)* It is the same
line 2.3 draws between an int and a float, drawn for the same reason.

**A `u8` to a `u16` could be allowed**, since it always fits. It is
not, yet: the rule "sized to `int` is free, everything else is
written" has no table to remember. *(Was provisional: 11.1.)*

**A `list[u8]` is not a `list[int]`.** Lists are shared (3.10), so
whoever held it as a `list[int]` could append 300, and the first
holder would find a `u8` that is not one. Copy it: `[int(n) for n in
bytes]`.

### 2.8d No `u64` *(was provisional: 11.1)*

**Why.** An `int` is 64 bits with a sign, so half of a `u64`'s values
do not fit in one, and 2.8a ("the sum is an `int`") would stop being
true. Hashes and 64-bit registers will want it. It waits for one of
them, and for an answer to what `u64 + u64` is.

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

**Was provisional (11.1).** This is wordy for one-line functions. A shorter form may be
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

**Was provisional (11.1).** There is no inheritance and no custom constructor yet. Both
are to be decided when classes are type checked.

### 3.8 Methods name `self` explicitly

**Why.** It shows that a method is an ordinary function that receives its
object as the first argument, with no hidden variable. *(Principle 4.)*

### 3.9 `let` fixes the name, not the contents *(was provisional: 11.1)*

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
*(Was provisional (11.1): the copy can be skipped when the compiler can see that the
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
not looked at twice. *(Was provisional (11.1): Kotlin refuses to narrow such variables
at all, which is stricter and needs no check at run time. That may turn out
to be the better rule.)*

### 3.13 A function that promises a value must return on every path

**Why.** In C, falling off the end of a non-void function returns garbage.
In Python it returns `None`, which then fails somewhere else. Here it is an
error at the function. *(Principle 1.)*

A function whose type says it may return `none` (`-> int or none`) is the
exception: reaching its end returns `none`, since that is one of the
answers it promised.

### 3.14 `upper`, `lower`, `strip` and `split` know plain letters and blanks only *(was provisional: 11.1)*

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

**Was provisional (11.1).** A separate module can offer the full rules to programs
that have the room. Rust makes the same split, with `to_ascii_uppercase`
beside `to_uppercase`.

### 3.15 A pin is a kind of value of its own, and a number underneath

`from board import led` gives a `Pin`. It has `on`, `off`, `toggle`,
`is_on` and `.number`. It is not an `int`: `led + 1` and `led == 13` do
not compile. `pin(7)` makes one from a number and raises a `ValueError`
if the board has no such pin.

**Why.** Hardware is where a mixed-up number does real damage, so pins
get the treatment units got (1.15): the checker keeps them apart from
plain numbers, and after the checker they cost nothing. A pin is its
number in every compiled program, so `blink` needs no memory from a
collector, and builds with `--no-collector`. *(Principles 1 and 5.)*

**Turned down.** A class `Pin` written in Zeno, with the level in a
field: every stage already has objects, but an object needs the
collector, and switching an LED must not. A plain `int` with functions
`on(13)`: free, and `on(count)` would compile.

`Pin` is a type name the language knows, as it knows `int`. A class of
the program's own called `Pin` is found first, so no existing program
changes meaning.

### 3.15a `sleep` takes a time with a unit, and `now` gives one

`sleep(500 ms)`, `sleep(2 s)`. `sleep(500)` and `sleep(3 V)` are
refused by the checker. `now()` is a quantity in `ms`.

**Why.** Whether a bare 500 is seconds or milliseconds is the question
that every delay function in every C library answers differently. The
unit answers it in the program text. *(Principle 1.)* Refusing `3 V` at
check time and not at run time is new in this stage: before it, the
interpreter found it when it got there.

`now()` counts from a fixed moment that the program cannot know (on a
microcontroller, power-on). **Why not the time of day:** a
microcontroller does not know it, and code that measures how long
something took must not jump when a clock is set. Only the difference
of two readings is promised to mean anything.

### 3.15b On a computer the board is simulated, and prints a pin when it changes

`zeno run examples/blink.zeno` prints `led on`, `led off`, and so on.
A pin that is written with the level it already has prints nothing.

**Why.** A program for a board should be runnable, and testable,
without the board. Printing only changes is what an instrument on the
pin would show, and it keeps the output of a program that sets a pin
in a loop readable. **Cost:** the simulated board's output is part of
the program's output, so a program that also prints mixes the two.
That is the point for a test, and a real board prints nothing.

The LED is pin 13 on every board, as far as a Zeno program can tell. A
real board wires that number to wherever its LED is. **Why 13:** it is
the Arduino Uno's, the number most people have already met.
*(Was provisional (11.1): one LED and numbered pins is the least a board can be.
Named pins, inputs and the rest wait for a real board to ask.)*

### 3.16 Generics: `function first[T](...)`, `class Stack[T]`

A function or a class can leave a type open and name it:

```zeno
function first[T](items: list[T]) -> T {
    return items[0]
}

class Stack[T] {
    items: list[T]
}
```

`Stack[int]` is then a type, and `first([1, 2])` is known to be an int.

**Why have them.** A ring buffer of bytes and one of voltages are the
same code. Without generics it is written twice, or written once with
the types left out, and then nothing after it is checked. A language
that checks types before the program runs needs a way to say "the same
type that went in". *(Principle 1.)*

**Why square brackets.** Zeno already writes "a type applied to a
type" as `list[int]`. `Stack[int]` is the same sentence, so there is
one rule and no new punctuation. The angle brackets of C++, Java and
Rust would make `a < b > c` a puzzle for the parser and the reader.
Go chose square brackets for that reason, and Python writes
`list[int]` too. *(Principle 3: punctuation has to earn its place.)*

**Why after the name.** `function first[T](...)` reads "first, for any
T". A keyword in front, as in C++'s `template<typename T>`, is one
more line for the same information.

### 3.16a A type parameter starts with a capital letter, and its name wins inside

`function first[t]` is an error. Inside `class Pair[A, B]`, `A` is the
type parameter and not amperes.

**Why the capital.** `int`, `str`, `list`, `ms` and `u8` are all lower
case or units. A capital says "this is a name for a type" without the
reader looking back at the header.

**Why it wins.** Zeno has dozens of units, and several are the single
capital letters people reach for first: `A`, `V`, `K`, `W`, `J`. The
first version refused a type parameter with a unit's name, and
`Pair[A, B]` and `Map[K, V]` would not compile. A name declared right
there, in the header of the function, is the nearest declaration, and
the nearest declaration winning is the rule everywhere else (3.1).
A quantity is still written with its unit, `3.3 V`, inside such a
generic: that is a number followed by a unit, and not a type.

### 3.16b The types are worked out at each call, and never written there

There is no `first[int](xs)`. The checker finds `T`:

1. from the arguments;
2. from an argument that is a function, by calling it with what the
   others gave: in `convert(bytes, function(b) { ... })`, what the
   function returns is `U`;
3. from the place the result goes, for what is still unknown:
   `let s: Stack[int] = Stack(items=[])`.

If none of the three says, it is an error that shows how to write the
third.

**Why.** Zeno works types out wherever it can (0.2), and a call is
where it can do it best. The cases where the arguments say nothing are
a constructor with an empty list and a function with no parameters,
and both have a result that is going somewhere with a type on it.
Writing the type on the variable is already how `let xs: list[int] =
[]` works. A second syntax for the same thing can wait until a
program needs it. *(Principle 3, and "keep what works".)*

**Why arguments first.** When the two disagree, the mistake is
reported against the type that was written: `let x: str = first(ns)`
with `ns` a `list[int]` says "`x` should be str, but this is int",
which is where the eye already is.

### 3.16c A written number waits for the others to speak

`ring(4, 0)` is a `Ring[int]`. `let bytes: Ring[u8] = ring(4, 0)` is a
`Ring[u8]`. With `small` a `u8`, `same(small, 9)` is a `u8`, and
`same(small, 300)` is an error.

**Why.** It is 2.8c again. A number written in the program can be an
`int` or a `u8` and is checked while compiling, so it should not be
the thing that decides. Without this rule every generic used for bytes
would need `u8(0)` where `0` is meant, and sized numbers would be the
one corner of the language where generics are awkward.

### 3.16d No bounds: the body is checked for each type it is used with *(was provisional: 11.1)*

`largest[T]` uses `>`. Nothing in its header says `T` must have a `>`.
The checker checks the body once for each type the program uses it
with: for ints and for text it passes, and for a class with no `>` it
is an error, reported inside `largest` with a note naming the copy and
the line that asked for it.

**Why.** A bound needs something to name: `T: Ordered` needs
`Ordered`, and that is interfaces (or traits), a feature Zeno does not
have yet and should design on its own terms. Checking each use is what
C++ did for thirty years, and it finds every mistake before the
program runs. That is the promise that matters. *(Principle 1.)*

**Cost.** The error is reported where the `>` is and not at the call,
and a generic that nothing uses is only checked loosely (5.14d). C++
error messages show how badly this can go. The note under the error
is the guard against that, and bounds are the real fix.

### 3.16e What generics leave out *(was provisional: 11.1)*

Generic enums, methods with type parameters of their own, a generic
kept as a value (`let f = first`), an owned or borrowed value as a
type argument, and raising an object of a generic class. Each is
refused with a message that says so.

**Why.** Each needs a decision that deserves a use case first. An enum
is `T or none` in its commonest form, which exists. A generic value
would have to be every copy at once, or be turned into one copy by the
place it goes. An owned type argument means the container frees what
it holds, which is a rule about ownership and not about generics.
`raises` and `except` take a plain name. None of them is needed for a
ring buffer, a stack or a `largest`, which are what this stage was
for. *("We don't have to over-engineer things.")*

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
out, most programs with no types written still pass. *(Was provisional (11.2): a
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
toolchain in use (ARM, RISC-V, AVR, ESP). *(Was provisional (11.2): a plain-C fallback
can be added for other compilers.)*

### 5.5f Compiled strings were not freed at first

Until the memory management stage, memory for text built at run time was
taken and never returned. **Replaced by 5.7.**

### 5.5g In C, objects, enum values and lists are pointers

**Why.** Zeno says objects and lists are shared, not copied (3.10): after
`let b = a`, changing `b` changes `a`. A C struct is copied on assignment; a
pointer to one is not. So the pointer is the faithful translation. The cost
is that each one needs memory from the heap, which the collector (5.7)
gives back. *(Was provisional (11.2) in one respect: an object that never leaves the
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
trip round, which is not what `zeno run` says. *(5.5d. Was provisional (11.2): once
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

**Was provisional (11.2): a function that can raise cannot be used as a value** in a
firmware build. The two sorts of function are called differently, and the
type `function(str) -> int` does not say which sort it is. The fix is to
let function types say `raises`, which the checker needs anyway: today it
does not follow errors through function values in any build.

**What this does not make true.** The C of a firmware build is not thereby
MISRA-compliant. Still to do, each its own piece of work: the heap (Rule
21.3; answered for `--no-collector` builds by the pool, 5.9h), variable
argument lists in the runtime (17.1),
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

**Was provisional (11.2), in three respects.**

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

*Superseded by 5.19a to 5.19c:* the word on a parameter is now `take`,
it marks the parameter that takes, and both words are reserved.

*Turned down:* `&x` and `&mut x` at each use, as in Rust. Punctuation, and
it puts a mark at every call where one in the signature says the same.
*Turned down:* a mark at the call (`send(move p)`). It would make each
move visible where it happens, which is worth something, but the checker
already reports any use after a move by line, and the calls stay plain.
Was provisional (11.2): if moves prove easy to miss when reading, this is the first
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

*Superseded by 5.19a:* a parameter looks unless it says `take`. `self`
still always looks.

*Turned down:* borrow by default, with a word for taking (Mojo, Swift). It
makes the common case shorter. But then `p: Packet` would mean two
different things on a `let` and on a parameter. Was provisional (11.2).

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

#### 5.8i What is refused for now *(was provisional: 11.2)*

- An owned value inside ~~a list,~~ an `or none`, an enum case or a function
  type, and moving a field out of an object. Each needs a rule for what is
  left behind in the hole. *(A list that has an owner can hold owned
  values since 5.15c: `pop()` takes one out and leaves no hole.)*
- Replacing an owned field through a borrow, including `self`. Somebody
  else may have been lent the old value at that moment. Telling "may
  change" borrows from "may only read" borrows would lift this, and is the
  first growth to consider (5.8d).
- A shared list becoming an owned one. Only a list that was just made
  can start life owned (one written out, a comprehension, or what
  `split`, `map`, `filter` or `+` gives, since 5.9s): those are the only
  lists certain to have no other name.
- `match` on an owned value, raising one, and a method of one kept as a
  value.

Each is an error with a message that says what to do instead. Allowing any
of them later breaks no program that works now.

One refusal that was on this list is gone. `print(look(make()))` used
to be "this Packet has no owner yet. Give it a name with `let` first".
It is accepted since 5.9r: the statement owns the value.

#### 5.8k `xs += ys` on an owned list is `xs = xs + ys`

The checker refused `+=` where the left side was an owned list: `+`
was asked to add an `owned list[int]` to a `list[int]`. It now looks at
the left side as any other reader does, and the list that `+` makes
takes the old one's place, which the owner frees.

**Why.** `x += y` means `x = x + y` for everything else in Zeno, and
`xs = xs + ys` was already accepted for an owned list. One of the two
being refused was a surprise with no reason behind it. *(Principle:
unambiguous and intuitive.)*

**What it does not do.** It does not grow the list in place. `+` makes
a new list in every build, and `xs.append(y)` is the way to add one
item without that. Through a borrow (`self.items += more` in a method)
it is still refused, by 5.8i: replacing an owned field needs the owner.

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

**The heap.** At first this build still took owned values from the
heap, and MISRA C bans `malloc` and `free` (Rule 21.3). It now takes
them from a pool of a fixed size, and the heap is not used: 5.9b and
5.9h to 5.9k.

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

### 5.9b Owned values came from `calloc` and `free` at first; now from a pool

The first no-collector build used `calloc` and `free`. **Why then.** It
was the least that worked, and the compiler's part of the job, a free
on every path, is the same whatever sits underneath. A fixed pool was
turned down "for now", as a change to runtime.c that touches nothing in
the compiler.

That is how it went. The pool (5.9h) replaced the two functions, and
not a line of either compiler changed.

### 5.9c Built text lives for one statement, in a room of fixed size

Text built while the program runs (`f"..."`, `str(n)`, `a + b`,
`.upper()`) is written into one fixed array and given back when the
statement that built it is over.

**Why text needs an answer at all.** Firmware prints. A build that
refused `print(f"temperature {t}")` would be of no use.

**Why one statement.** It is the shortest life that still lets text be
built and used, and it has an end the compiler can see without following
the text anywhere. Giving it back is setting one number, so there is
nothing to search and nothing to fragment. Calls nest inside statements,
so the room behaves as a stack, and its size is the answer to "how much
text can be in the making at once", which the report measures.

**Text that has to last longer is copied out of the room** (5.9l). At
first it could not be kept at all: not in a variable, a field or a
list, not returned, not passed to a function. That rule was cheap, one
flag on a compiled value, and it was too tight: a program could not
build a line and keep it for later.

**Turned down:** printing piece by piece with no text at all. It only
helps `print`, and `if name.upper() == "ON"` still needs somewhere to put
`"ON"`.

### 5.9d The `except` that catches an error frees it

*(Changed by 5.17: the `except` name owns the error now, and frees it
only if it still has it when the block ends.)*

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
one. *(Was provisional (11.2): passing it on could be a move.)*

Since 5.15h the checker says this, and the error may be lent to a
`borrow` parameter.

### 5.9e The compiler refuses, not the checker

*(Taken back in part by 5.15a: who owns what is the checker's question
now. What cannot be stored yet is still refused here.)*

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

### 5.9g What a build with no collector refuses *(was provisional: 11.2)*

*(The first two entries are gone since 5.15: a plain list and a plain
class have an owner now. The list as it stands is in 5.15j.)*

A list kept by something that is not an owner (a plain variable,
parameter or field, or another list); objects of a class that is not an
`owned class`; enum cases that carry values; functions written as
values; and lists of numbers that may be `none`.

Keeping built text was on this list until 5.9l. `split`, `map`,
`filter` and `+` on lists, `for` over the characters of a str, and a
list written where it is only looked at were on it until 5.9r. The
rest are being taken off it one step at a time, under one rule:
whoever receives a value owns it.

**Why refuse and not support.** Each could be given an owner or held for
a moment, and each would be more machinery. None was needed to build the
example or the tests, so none was built. *(Principle: keep what works.)*
The error for each names the line, and where there is one, the thing to
write instead.

### 5.9h The pool: fixed sizes that double, and a list of spares for each

In a build with no collector, lists, objects and errors come from one
static array of `ZENO_POOL` bytes. Blocks come in sizes of 16, 32, 64
bytes and so on. A request gets the smallest size it fits. A block
given back goes on a list of spare blocks of its size, and the next
request of that size takes it. When there is no spare one, a new block
is cut from the part of the array that has never been used. A block is
never cut in two and two are never joined. `docs/pool.md` is the
walkthrough.

**Why a pool at all.** MISRA C:2012 Rule 21.3 bans `malloc`, `calloc`,
`realloc` and `free`. A small board may have no heap. And the two
things firmware cannot plan for with a heap are how long a request
takes and whether the heap has broken into pieces too small to use.

**Why these sizes and no joining.** Three properties follow, and each
can be said in a sentence.

- Taking and giving are a few instructions, the same however full the
  pool is. Nothing is searched.
- The pool cannot break up. A block of a size the program has given
  back is there, whole, for the next request of that size. So a piece
  of work that fitted once fits every time after.
- How much pool a run needed is one number, `zeno_pool_top`, and a
  pool of exactly that size is enough.

**Cost.** Memory cut for one size never serves another. A list that
grows by doubling leaves each old place behind as a spare block, so it
costs about twice its final size. A request is rounded up to the next
size. `buffer.zeno` takes 25,384 bytes of pool for 13,336 bytes in use
at once. *(principle: firmware would sooner pay a known cost than risk
an unknown one.)*

**Turned down: a free list with joining** (the allocator in K&R, and
most small `malloc`s). It wastes less. It searches, and it can break
up, which are the two things the pool is there to remove.

**Turned down: a buddy allocator,** which cuts a block in two halves
and joins them again when both are spare. It keeps the doubling sizes
and gets back the memory a growing list leaves behind. It is about
three times the code, and giving back is then a loop of joins. Worth
another look if a real program is short of RAM because of growing
lists. *(was provisional: 11.2)*

**Turned down: TLSF,** the usual answer when a real-time system must
have a general allocator. It is constant-time and wastes little, and
is some hundreds of lines. Too much machinery for what is asked of it
here.

**Turned down: one pool for each type,** each with a count chosen by
the programmer (`pool Frame[8]`). That is the strictest design and the
one a certifying project may want, because the compiler could then say
when it builds that the program cannot run out. It needs new syntax,
and a list has no one size. It can be added beside this pool later.

**Turned down: one block size for everything.** A list of a thousand
items would not fit, or every object would take a thousand items' room.

**The sizes start at 16 bytes.** A spare block keeps the address of
the next spare one in its own first bytes, so it must hold a pointer,
and 16 keeps every block lined up for a 64-bit number.

**A new block is wiped whole,** not only the bytes asked for. The
compilers rely on a new list or object being all zeros, and wiping the
whole block means a spare block never shows the next owner what the
last one left.

### 5.9i Eight bytes in front of each block say its size and how much was asked for

`zeno_free` is handed an address and nothing else. In front of the
block are two 32-bit numbers: `step` (the block is `16 << step` bytes)
and `asked` (how many of them the program asked for, and 0 while the
block is spare).

**Why not have the caller pass the size.** The compilers know the size
of an object, but not of a list's items or an error's message when
they free them. A size passed in is also one more thing that can be
wrong.

**Why `asked`.** Two uses for four bytes that alignment would have
left empty anyway. The memory report needs it to count bytes in use,
so a build with `--memory` lays memory out exactly as a build without
it does, and the numbers it reports are true of the program that
ships. (The heap version added two words to each block only when
reporting.) And 0 means "spare", which is how a block given back twice
is caught.

**Why catch that,** when the compiler writes one free for each block.
Because of what it would do: the block would be handed to two owners,
and the fault would show up somewhere else, later. The check is one
comparison. *(principle: a fault stops at the line that caused it.)*

**Cost:** eight bytes a block, which is half as much again for the
smallest.

### 5.9j A full pool stops the program, and the size is chosen with `-DZENO_POOL`

When a request does not fit, the program prints `error: the memory
pool is full (ZENO_POOL is N bytes)` and stops, as it does when a
statement overflows the text room (5.9c).

**Why stop, and not raise a `MemoryError`.** In a firmware build what
the runtime finds for itself, such as an index past the end, is a
fault that stops the program (5.5l), and running out of memory is one
of those. An error is also itself a block from the pool.

**The size is a C macro, 32768 unless given.** **Why a macro.** The
text room is chosen the same way (5.9c), and one build of the C can be
compiled for boards of different sizes. **Why 32768.** It is the
smallest round number in which every example runs. On a board it is
meant to be changed, and `--memory` prints the number to change it to.
*(was provisional (11.2): `zeno build --pool N` is an exercise in `docs/pool.md`.)*

**The report says `pool: N bytes taken`,** which is how much of the
array was ever cut into blocks. **Why that number,** and not the most
bytes in use at once (which the line above it already gives). It is
the one that answers "how big must the pool be", and a test checks
that it is exact: the program runs in a pool of N bytes and not in
N - 1.

### 5.9k The heap is gone from a no-collector build, not switched off

There is no flag that puts `calloc` and `free` back. The runtime of a
build with no collector does not contain the words, comments included,
and the tests search the C, the IR and the runtime file for them.

**Why.** The same reason the collector's code is left out and not
switched off (5.9): a reviewer who searches a firmware build for
`malloc` must find nothing, and must not have to work out which
`#ifdef` is live.

**Both backends get it for nothing.** The pool is in the runtime, and
the LLVM backend links the same runtime (5.11d). The IR did not
change. The two builds must still write the same memory report
(5.11t), which now includes the pool.

**Cost: a heap checker cannot see inside the pool.** Valgrind and
AddressSanitizer watch `malloc` and `free`. To them the pool is one
array that is always in use, so a block read after it was given back
is no longer reported. What stands in for them: the compilers null an
owner when it gives its value away, the report must end with 0 blocks
left, the two backends must agree on every number, and a block given
back twice is caught by the pool itself. Poisoning spare blocks in a
checking build is an exercise in `docs/pool.md`. *(was provisional: 11.2)*

**What this does not make true.** MISRA Directive 4.12 is against
dynamic memory of any kind, and a pool is still that. The pool removes
the directive's stated reasons one by one, and a project would still
record a deviation.

### 5.9l With no collector, text is kept by copy

Whatever stores text has a copy of its own, in the pool, and frees it
when it is done: a variable at the end of its block, a field with its
object, a list item with its list. `let a = b` leaves both usable.
Text written in the program is never copied. `docs/text.md` is the
walkthrough.

**Why a copy, when a list is moved.** A list can be changed after it
is made, so two names for one list can tell that they share it. That
is what moving and borrowing are for (5.8). Text cannot be changed
after it is made, so no program can tell a copy from the original.
Copying therefore needs no rule at all: nothing new is refused, the
checker is not changed by a line, and a program means under
`--no-collector` exactly what it means under `zeno run`. *(Principles:
unambiguous and intuitive; one meaning in every build.)*

**The cost** is the copying: a `strlen`, a block from the pool and a
`memcpy` for each keep, and no sharing, so ten variables with the same
built line hold ten copies. Each cost is at a line that can be pointed
to and takes a time that depends only on that text's length, which is
what firmware asks of memory (5.9).

**Turned down: moving text, as Rust moves a `String`.** `let a = b`
would end `b`. That is the most surprising rule in Rust for people who
arrive from every other language, and Zeno's text gives it no reason:
there is nothing a second holder could do to the first. **Turned down:
a count of holders in front of each text** (reference counting). It
saves the copies and it is the natural next step if copying proves too
dear, but it adds a count to every keep and drop and a way for text
to stay alive that no line of the program shows. *(was provisional: 11.2)*
**Turned down: `borrow str`**, which 5.9c used to suggest. It would
bring the borrow rules to the most common value in the language to
save a copy.

### 5.9m Where text is tells who must free it

Text is in one of three places: written in the program, in the text
room, or in the pool. The runtime tells which by the address, with one
subtraction and one comparison for each array.

**Why not a flag beside the pointer.** A `str` stays one pointer, which
is what the C code that reads it expects, and what a list slot and a
field already hold. A flag would have to travel with every `str`
through every call, and could disagree with the truth. The address
cannot.

**What rests on it.** `zeno_text_keep` copies only what is in the room
or the pool. `zeno_text_drop` frees only what is in the pool. So a
variable may hold a literal at one moment and a copy the next, and the
same line frees it correctly either way. **Cost:** two comparisons on
each keep, drop and lend. The casts to `uintptr_t` are there because C
does not define comparing pointers into different arrays.

### 5.9n A function's text answer travels in the text room

`return text` moves the answer down to where the top of the room was
when the `return` began, and leaves the top just above it. The
caller's statement gives it back with the rest of its text.

**Why the room.** The function's own copies are about to be freed, and
the caller has not yet said whether it will keep the answer.
`print(label(3))` never does. The room is the one place that needs no
owner. It stays a stack: a function leaves exactly its answer on it.

**Turned down: the answer in the pool, owned by the caller.** Then
every call that returns text would need a statement-long owner to free
it, on every way out of the statement, including a failure half way.
That is what `hold` does for lists (5.9f), and it would have to be
written for every text call in the language. **Cost of the choice:** an
answer that is kept is copied twice, into the room and then into the
pool, and the room must be big enough for the longest answer. Text
written in the program, and the caller's own text passed in and
returned, are given back as they are.

### 5.9o Text read from a place a function can change is lent

A read of a `var` at the top of the file, a field or a list item gives
the statement a copy in the text room, if the text there is a kept
copy. A read of a local variable, a parameter or an unchanging `let`
does not.

**Why.** In `print(last, note("x"), last)` the function gives `last`
other text, which frees what the first read is still holding. A
collector would have kept it alive. Here the statement must have text
of its own. The places listed are exactly those a called function can
assign to (the same test 3.12 uses for looking again at a value that
was checked for `none`).

**The short cut.** Text that is read and stored at once
(`let before = last`) is copied straight from the place: nothing can
run in between. In C the compiler sees that the value is still the
bare read. In IR the `zeno_text_lend` has already been written, as the
last line, and is taken out again.

**Cost, and what is turned down for now.** `print(channel.label)`
copies the label though nothing is called after the read. The compiler
knows which statements call functions and could skip the copy.
*(was provisional (11.2): an exercise in `docs/text.md`.)* **Turned down: copy
only if a later part of the statement calls something.** It is right,
and it makes whether a copy is made depend on what else is on the
line. The simple rule came first.

### 5.9p Objects, lists and errors free the text they keep

The function that makes an object keeps the text for its fields, and
the function that ends it drops them. A list of text drops its items
with itself, and the copy a `for` loop walks (3.10a) has copies of the
items too. An error that carries an object is told which function ends
that object (`zeno_error_of`).

**Why the copy for `for` is deep.** The body may give the list a new
item, which frees the old one, while the loop is still holding it. A
shallow copy would be a list of pointers to freed text. **Cost:** a
block for each item before the first trip. *(was provisional (11.2): the checker
could say when the body cannot change the list.)*

**Why the error needs a function.** To the runtime an object is a
`void *`. Before this stage freeing one was `zeno_free`, because an
object that could be raised held no memory of its own. With text in
its fields that leaks. **Turned down: refusing text in a raised
object.** `raise TooHot(channel.label, reading)` is the natural thing
to write.

**In a list literal every item is lent and then kept,** without the
short cut of 5.9o. All the items are worked out before any is stored,
so something may run between a read and its keep. The two backends do
the same so that their memory reports agree (5.11t).

### 5.9q `match` and `for` give built text back as soon as they can

A `for` whose first line builds text gives it back before the first
trip: by then it has its numbers, or its copy of the list. A `match`
gives it back when a case is chosen, and a name the pattern gives
(`case other`) keeps a copy first. Both were refused before (5.11r).

**Why not hold the text for the whole statement.** A loop that runs
for ever would hold it for ever, and every `break`, `continue` and
`return` inside would have to give it back. Giving it back at the one
point every path passes through needs no such list.

**Known gap, for the audit.** C does not say in which order the
arguments of a call are worked out. Showing a list uses the room for a
moment while it works, so a statement that shows a list and builds
other text can fill the room a few bytes higher in the C build than in
the LLVM build. What is printed is the same. The test programs avoid
the mix so that 5.11t can stay strict. The fix is to work each
argument out into a temporary in order, which changes most of the C
this project's pages quote, so it waits for the audit stage.

### 5.9r A value that is made and given to nobody belongs to its statement

`print(count(line.split()))` makes a list and gives it to nobody. The
statement owns it and frees it when the statement is over. The same
goes for an owned value that a function gives back and that is only
looked at, `print(size(make()))`, and that one in every build. The
walkthrough is `docs/temporaries.md`.

**Why.** With no collector the question for every block is "which line
frees it?". For a value with no name there are few candidates, and the
end of the statement is the only one that is both certainly late
enough (nothing in the statement is still looking) and certainly
reachable (nothing after the statement can mention the value). It is
also the rule a reader would guess, and the rule Rust and C++ have for
what they call temporaries. *(Principle: unambiguous and intuitive.
Nothing new is written in the program, and a line means the same in
every build.)*

**Turned down: keep refusing, and ask for a name.** That was the rule
until now. `let words: owned list[str] = line.split()` followed by
`count(words)` is two lines and a type for what reads as one thought,
and it holds the list to the end of the block and not the statement.
The refusal taught nothing that the rule does not.

**Turned down: free it after its last use.** In
`print(len(a.split()), slow())` the list could go before `slow()`
runs. That needs the compiler to know which calls keep looking at what
they were given, for built-ins and for the program's own functions
alike, and then two lines that look the same would free at different
moments. One fixed moment is easier to reason about, and a `let` in a
block of its own says anything else.

**Turned down: free it at the end of the block.** A loop with such a
statement in it would hold one list per trip until it ended.

**Turned down: count references.** Decision 5.7 turned it down for the
whole language, and nothing here is different.

**How it leaves early.** The value is held (5.9f) from the line that
makes it. So a `return`, a `break` or a failed call in the middle of
the statement frees it by the path that already frees what the owners
hold. No second mechanism was built.

### 5.9s A made list is the statement's unless an owned list is wanted there

Six things make a new list: writing one out, a comprehension, `split`,
`map`, `filter` and `+`. The checker notes each (`Types::made`). Where
the place a made list arrives at wants an **owned** list (a `let` with
the type, an owned parameter or field, a `return` from a function that
gives one back), it starts life owned there (`Types::fresh`, which
5.9e already had for the first two). Anywhere else, in a build with no
collector, it is the statement's (5.9r), and `stays` refuses to let
anything keep it: "this list has no owner".

**Why the type of the place decides.** It is already written in the
program, by the person who knows whether the list is meant to last.
The alternative is to work out from how a name is used whether it
needs to own, which is 5.8j's escape analysis, turned down there.

**A plain `list[T]` parameter may not be given a made list**
*(was provisional: 11.2)*. A `borrow` parameter may: the checker already makes
sure such a function cannot keep what it was lent. A plain parameter
makes no such promise, so the list could be kept past the statement.
The next step makes a plain list in this build an owned one, and then
the function takes the list over.

**With a collector** nothing changes for a plain made list: it is
shared, and collected.

### 5.9t A condition is over when it has its answer; a `for` and a `match` are over after their last block

The end of the statement is the end of the line for a `let`, an
assignment and a call. The others:

- The condition of an `if`, `else if` or `while`, each side of `and`
  and `or`, and what `not` is in front of, are over as soon as they
  have their one-bit answer. What they made is freed there, before any
  block runs, and on every trip of a `while`.
- One item of a comprehension is over when it is in the list.
- A `for` and a `match` keep what their first line made until their
  last block is over.

**Why the conditions free early.** Nothing can be looking: the answer
is a bool. And a `while` that held its condition's list for the whole
loop would hold one for every trip.

**Why `for` and `match` hold on, when text is given back before the
first trip (5.9q).** Text is copied when it is kept, so the loop has
its own. A list is not. In `for x in make().data` the loop walks the
very list inside the object the statement owns, so the object must
outlive the loop. Rust has the same rule for the same reason.
**Turned down: free what can be freed before the first trip.** Telling
which values the loop still looks into is the analysis 5.9r turned
down.

**A change that came with it.** In C, the sides of `and` and `or` used
to share the statement's text mark. They are conditions now, as they
always were in the LLVM backend, so the two backends give text back at
the same points.

### 5.9u The statement's variable is declared where the statement begins, holding nothing

```c
zeno_list *t1 = NULL;
t1 = zeno_split_blanks(z_line);
...
zeno_list_drop_text(t1);
```

**Why not `zeno_list *t1 = zeno_split_blanks(z_line);`.** The value
may be made inside an `if` that the C needs for another reason
(`a < f(x) < g([1, 2])` is nested `if`s), and the line that frees it
is after that `if`. A variable declared inside would be out of reach.
Declared first and set to `NULL`, it is in reach everywhere, and
freeing `NULL` does nothing if the `if` was skipped. In IR the same
thing is a slot with `store ptr null` (a name from a block that may be
skipped cannot be used after it). Both lines are inserted where the
statement began once it has been compiled: only then is it known how
many there are.

**Turned down: free at the end of whichever nested block made it.**
Every place the compilers open a block inside an expression would need
to know about it, now and whenever one is added. One rule at the
statement, plus `NULL`, covers them all.

### 5.9v A list that nobody else can reach is walked without a copy

A `for` loop, `map`, `filter` and `reduce` walk a copy of their list
(3.10a), so that a body which changes the list cannot confuse them.
Two kinds of list need no copy: one that was made right there (the
loop frees it after the last trip), and one that the statement owns
(the statement frees it after the loop). Neither has a name, so no
body can change it.

**Why bother.** In a build with a pool of a fixed size, the copy of a
list of text is a block for the list, a block for its items and a
block for every text. `for word in line.split()` would have made all
of them twice.

### 5.9w `for ch in text` makes a list of the characters, which the loop owns *(was provisional: 11.2)*

It was refused, with advice to count with `range(len(text))` instead.
It is accepted: the list is a made list like any other.

**Why it was provisional (11.2).** It costs a block of the pool for every character
while the loop runs. Walking the text itself, one character at a time,
would cost none, and is an exercise in `docs/temporaries.md`. It was
not done here because the text may be in the text room, which a `for`
gives back before the first trip (5.9q).

### 5.9x Lists made from text have their own copies, made the cheapest way

By 5.9p a list frees the text it holds, so each made list must own
what it holds. `split` and the characters of a `for` cut their pieces
straight into the pool (`zeno_piece`), never into the text room.
`map` copies each answer and gives the text room back before the next
call, so a `map` over a thousand items needs room for one answer.
`filter` copies what it takes. `+` joins and then copies
(`zeno_list_keep_text`).

**Turned down for now: move text out of a list that is about to be
freed**, as `filter` on a statement's own list could. It saves a copy
and a free per item and needs each of these loops in two forms. An
exercise.

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

### 5.10c An instruction is a Rust enum, not bytes *(was provisional: 11.2)*

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

### 5.10g The interpreter stays the default *(was provisional: 11.2)*

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

### 5.11c Overflow is checked in the IR, and `//`, `%` and `/` are written in IR

`+`, `-` and `*` on ints use `llvm.sadd.with.overflow` and its
relations. `//` and `%` on ints, and `/`, are small functions written
in IR (`zeno.floordiv`, `zeno.mod`, `zeno.div`, in the table `WRITTEN`),
copied into a program that uses them. `**`, and `//` and `%` on floats,
still call `runtime.c`.

**Why.** The first version called the runtime for all of them, because
the runtime already had their rules right (which way `//` rounds, the
sign of `%`). That was measured: the runtime is in another file, so the
calls were not inlined, and a loop of 300 million `%` took 0.76 s where
the C build took 0.29 s. Written in IR they are in the same file as the
program, LLVM inlines them, and the loop takes 0.28 s. They follow the
C line for line, and the tests compare both with the interpreter.

**Turned down:** `-flto`, which optimises the two files together. It
got the loop to 0.45 s, not 0.29, and needs the LLVM linker installed.
**Cost:** two copies of three small functions to keep in step.

### 5.11d The runtime is the C backend's, with `static` taken off *(was provisional: 11.2)*

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

Three small functions are added at the end for `try` (`FOR_TRY` in
`emit_llvm.rs`, see 5.11l). They are only in the copy an LLVM build
writes, so a C build is as it was.

There are three runtimes, as for C: ordinary, firmware, and firmware
with the collector left out (5.9). `emit_llvm::runtime(build)` takes
whichever `emit_c::runtime` would give that build.

### 5.11e The LLVM backend compiles what the C backend compiles

It began with what the C backend began with: numbers, bools, text,
functions, `if`, `while`, `for` over a range, f-strings. That was the
smallest part that teaches all of IR's ideas (types, blocks, SSA, `phi`,
calls into C). The rest of the language followed in one step: classes,
lists, enums and `match`, `none`, `try` and `raise`, functions as
values, units and `import math` (5.11h to 5.11n).

**Why in one step.** The pieces lean on each other. Showing an object
shows its fields, which may be lists of enum values that may be `none`.
Text, comparing and list slots had to exist for all kinds before any
one of them could be tested against the interpreter.

What it refuses is what `zeno build` refuses (a named function inside a
function, the `time` module, a default that is not a literal, a method
used as a value). `--firmware`, `--no-collector` and `--memory` were
refused at first and came later (5.11o, 5.11p, 5.11q).
One thing it takes that the C backend does not: an assignment through
any place, such as `grid[0][1] = 5`. In IR the order of the steps is the
order they are written, so the C backend's reason for refusing it (the
value must be worked out before the place) does not arise.

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

Every program the C backend's tests compile is also built through LLVM
(`same_as_interpreter` in `emit_c.rs` ends by calling the one in
`emit_llvm.rs`). **Why.** Those 52 programs were written one hard case
at a time while the C backend grew: order of evaluation, errors inside
loops, closures that outlive their function. Writing them again would
find fewer.

### 5.11h An object is a struct type, and an enum value starts with its tag

A class becomes `%z_Point = type { double, double }`. An object is the
address of a block from `zeno_alloc`, and a field is reached with
`getelementptr`. An enum value is the address of a block that starts
with an `i64` tag. Each case that carries values has a struct type of
its own (`%z_Shape.Circle = type { i64, ptr, double }`), and a case
that carries nothing is one fixed constant.

**Why.** It is 5.5g again: in Zeno these values are shared, so they
are held by address. `getelementptr` with a named struct type lets LLVM
work out where each field is, on any processor. **Turned down:** a C
`union` for the cases of an enum, as `emit_c` has. IR has no unions. It
does not need one: after the tag has been read, the block is simply
looked at through the struct type of that case. A block is as big as
its own case, not as big as the biggest.

### 5.11i A list item crosses into the runtime as an `i64` *(was provisional: 11.2)*

Lists are the runtime's (5.5h): one list type, each item in a
`zeno_slot`, a C union one register wide. The IR calls
`zeno_list_append(ptr, i64)` and puts an item into the `i64` with
`bitcast` (a float), `zext` (a bool) or `ptrtoint` (an address).

**Why.** One runtime for both backends (5.11d). **This leans on how C
hands a small union to a function:** as one whole number in an ordinary
register. That was checked by asking clang for the IR of `runtime.c` on
x86-64, and it is the rule on 64-bit ARM too. It also takes the
processor to be little-endian, where a C `bool` in the union is its
lowest byte. **When it changes:** on a processor where either is not
so. Then the runtime gets entry points that take `int64_t`.

### 5.11j `T or none` is null for an address and a `{ i1, T }` pair for a number

As 5.5j. An address can already say "nothing", so `Point or none` is a
`ptr` that may be null. An `int or none` is the pair `{ i1, i64 }`:
a flag, then the number. IR passes such a pair to a function and
returns it like any other value (`insertvalue` and `extractvalue` put a
part in and take one out).

Inside a list a pair does not fit a slot, so it is put in a small block
and the slot holds the address. The runtime has functions for that
(`zeno_box_int`), which take a C struct. **They are not used:** how a
struct is handed to a C function differs from processor to processor,
and getting it wrong is silent. The same six functions are written in
IR (`zeno.box.int` and its relations), where the pair never leaves IR.

### 5.11k What shows and compares a value is written the first time it is needed

`print(point)` calls `@zeno.show.Point`, a function the compiler writes
in IR. So with `@zeno.same.Point` for `==`, the same two for each enum
and each `T or none`, and the small functions the runtime's list code
is handed to show or compare one item. Each is written when the first
use of it is compiled, and the set `written` remembers.

**Why.** `emit_c` writes show and same for every class as it meets the
class, used or not, and the C compiler drops the unused ones. Here that
would be IR the reader has to scroll past. **One care:** a class can
hold itself (`next: Node or none`), so showing a `Node` needs the
function that shows a `Node`. The name goes into `written` before the
body is compiled, and the inner use becomes a plain call.

### 5.11l Errors jump, as in the ordinary C build; slots are `volatile` near a `try`

`try` and `raise` use the runtime's `setjmp` and `longjmp` (5.5i). The
IR calls `_setjmp` itself, marked `returns_twice`, and the runtime's
`zeno_raise` does the jump.

**Why.** Errors raised by the runtime (an index out of range, a
division by zero) already travel this way, and a `try` in IR has to
catch those too. **Turned down:** LLVM's own exceptions (`invoke` and
`landingpad`). They are what C++ uses, cost nothing until something is
raised, and need a "personality" function and the system's unwinder,
which `runtime.c` would have to be rebuilt around. Errors as values, as
in a firmware build (5.5l), are not the ordinary way here for the reason
they are not in C (5.5i): a test after every call that can raise. They
are what `--llvm --firmware` does (5.11o).

**Two things this costs.**

1. *How big is a `jmp_buf`?* Only C knows, and it differs by processor
   and C library. The IR asks while the program runs
   (`zeno_try_size()`), makes that much room on the stack, and leaves
   the fields to two more small functions. All three are added to the
   runtime copy an LLVM build writes.
2. *`volatile`.* After the jump back, a `var` changed inside the `try`
   may hold its old value, because LLVM kept it in a processor register
   and the jump puts the registers back. So in a function that has a
   `try` in it, every slot of the function is read and written
   `volatile`, which keeps it in memory. The C backend marks only the
   variables changed inside a `try`. Marking all of them is simpler and
   costs speed only in those functions. It was tested by taking the
   word out of the IR by hand: the `except` block then sees the wrong
   numbers.

*(was provisional: 11.2)* `_setjmp` is the name on Linux and macOS. It is the one
that does not save the signal mask, which is what C's `setjmp` means
there.

### 5.11m A function value is a block: its code, its name, its variables

As 5.5k. A function value is the address of a block whose first field
is the address of its code. It is called with itself as a hidden first
argument. A function written inside another finds the variables it uses
after the two fields every function value has. A named function used as
a value gets a small wrapper that takes the hidden argument and ignores
it.

A `let` is copied into the block: it cannot change, so a copy is as
good. A `var` that an inner function mentions lives in a box from
`zeno_alloc` from the start, and the block holds the box's address, so
the two functions see each other's changes, and the variable outlives
the function that declared it.

**Why a box for every such `var`, even one the inner function only
reads:** finding out whether it is ever changed after the inner
function was made needs a pass over the outer function that the
compiler does not have. **Cost:** one small allocation for each such
variable.

### 5.11n In an LLVM build, an owned value is freed where its owner ends

At first the LLVM backend compiled an owned list or object exactly as a
shared one and left it to the collector, because every LLVM build had
the collector. That was provisional, and it is gone. An owned value is
now freed where its owner ends, in every LLVM build, as in every C
build (5.8). 5.11p says how.

**Why in every build,** and not only where the collector is left out.
One way of compiling owned values is one thing to test: the ordinary
and firmware builds exercise the same frees that `--no-collector`
depends on, with the collector underneath to catch a mistake. And
`--memory` then means the same in both backends: the same program
frees the same number of blocks without the collector.

### 5.11o `--llvm --firmware`: the firmware design again, in IR

`zeno build --llvm --firmware` carries errors as values, as the C
firmware build does (5.5l), and for its reason: no `setjmp`.
`docs/llvm.md` walks through the IR.

- A function that says `raises` gives back an `i1`, and takes one more
  argument, `ptr %result`, where it stores its answer.
- A call to it is followed by a `br` on that `i1`. A failure goes to the
  `except` of the `try` around it, or out of the function with
  `ret i1 false`, or, at the top of the file, to `zeno_uncaught`.
- `raise` calls `zeno_note` and fails the same way.
- A `try` is plain blocks and branches. There is no `_setjmp`, no
  `zeno_try`, and no slot is `volatile`.
- The runtime file written beside the IR is the firmware one, so an
  error the runtime finds stops the program.

**Why the same design,** and not something LLVM is better at. Three
reasons. The firmware runtime already exists and is used as it is: not
one line of C was written for this. What a firmware build means stays
one thing, whichever backend makes it: the same programs are accepted,
the same ones are refused in the same words, and a fault stops both
*(principle 1)*. And the tests come free: every test program was
already built for firmware through C and compared with `zeno run`, so
building it once more through LLVM needed no new programs.

**Why this is the better half of the LLVM backend.** In an ordinary
LLVM build, every slot of a function with a `try` in it is `volatile`
(5.11l), which stops the optimiser from keeping those variables in
registers. Here nothing jumps, so nothing is `volatile`, and `opt` sees
ordinary calls, tests and returns, which it can inline and simplify.

**Turned down: giving back a pair, `{ i1, T }`.** IR can return two
values at once, and for small ones they travel in registers, which is
what Rust does with a `Result`. Three things against it today. The C
runtime's own functions that can fail (`zeno_parse_int`) use the
pointer form, so a build would have both forms in it. A pair type is
needed for every return type. And how a pair is returned depends on the
processor, where a pointer is the same everywhere. After inlining, LLVM
removes the pointer for small functions anyway. *(was provisional (11.2): worth
measuring when there is a real board to measure on.)*

**Turned down: `invoke` and `landingpad`,** as in 5.11l. They need the
system's unwinder, which is the very thing a firmware build is there to
avoid.

**The room for the answer is made at the top of the function,** one
`alloca` for each call that can fail. An `alloca` inside a loop would
take new stack on every turn. **Cost:** a function with many such calls
has that many slots, until `opt` merges them.

`--llvm` with `--no-collector` or `--memory` was still refused when
this was written. Both needed owned values freed where their owner
ends. That came next: 5.11p and 5.11q.

### 5.11p An owner is a slot, set to null when it gives its value away

In an LLVM build a variable that owns a value is always memory: an
`alloca` in a function, a global at the top of the file. That holds
for a `let` too, and for a parameter that takes a value over, which is
stored into a slot on the first line of the function. Giving the value
away is `load` and then `store ptr null`. Each way out of a block
(its `}`, `return`, `break`, `continue`, `raise`, a failed call in a
firmware build) loads every owner of the blocks being left and calls
`zeno_list_drop` or the class's `zeno.drop.Name`, which do nothing for
null.

**Why a slot,** when 5.11b makes a `let` a plain value. A value in IR
is written once. "This owner is empty now" is a second write, so it
needs memory. `mem2reg` turns the slot back into values, and the null
is then a constant the optimiser can see. It removes the drop of an
object that is certainly null. The drop of a list stays as a call that
does nothing, because `zeno_list_drop` is in the runtime file
*(was provisional (11.2): a null test written in IR would let it go, and is an
exercise in `docs/llvm.md`)*.

**Why null, and not working out which owners are full.** The emitter
could follow each owner through the function and write a drop only
where one is certainly full. It would need the checker's whole flow
analysis again (an owner given away in one branch of an `if` is full
on one path and empty on the other), and a mistake there is a block
freed twice. With null the pointer answers the question when the
program runs. This is the C backend's design (5.8), and the two
backends now free the same blocks at the same lines.

**Turned down: a `phi` for each owner.** That is what the slot becomes
after `mem2reg`, and writing it by hand is 5.11b's turned-down choice
again.

**Turned down: drop flags** (a separate `i1` for "still full"), which
is what Rust uses. The pointer already holds that bit.

**A value between owners is noted** (`hold`, `let_go`): the list a
comprehension is filling, the copy a `for` walks, an owned argument
while later arguments are worked out. Whatever is noted is freed with
the owners when a block is left early. **Why.** Without it a failure
in that moment leaves a block nobody frees, and with no collector that
is a leak. The C backend does the same (5.9).

**In an ordinary build a jump still skips frees** in the functions it
passes over, and the collector takes those, as in C. In the function
with the `try`, owner slots are covered by the `volatile` rule that
already applies to every slot there (5.11l), so an owner emptied
before a call that raises is still empty afterwards.

### 5.11q `--llvm --no-collector`: the same design, the same runtime, the same refusals

`zeno build --llvm --no-collector` is 5.9 in IR. Owned values are
freed by their owners (5.11p). Built text goes in the text room and is
given back at the end of its statement. An `except` frees the error it
caught. The runtime written beside the IR is the C build's no-collector
runtime. The IR has no `zeno_roots`, no stack base and no call that
marks anything.

**Why not a design of its own.** The reasons are those of 5.11o. What
`--no-collector` means stays one thing whichever backend makes it. The
runtime is used as it is. And the test programs already exist.

**Refusals are made by one function.** Every "this build cannot do
that" error in both backends goes through `emit_c::without_collector`,
which adds the same closing words. A table in the tests gives each
refused program to both backends and requires the same message. **Why.**
A user who moves a program from one backend to the other should not
meet a new error, or the old error in new words.

**Which text is in the room is tracked on the value** (`fixed` in
`Val`): set for a literal and a variable that cannot change, clear for
text that is in the text room. (Until 5.9l the bit decided what could
be kept at all. Now it decides what the statement must give back.) **Why on the value.** The
C backend asks the same question of the syntax tree. In the LLVM
emitter the value is what every "keep" site already has in its hand,
and the bit travels with it through `if` expressions and `or none`.
**Cost:** each place that makes a `Val` has to say which it is. The
default is "built", which is the safe side: a missed case is a refusal
the C build would not make, and the test table shows it.

**`--memory` is asked for in the IR**: `main` calls
`atexit(zeno_memory_report)`, and the runtime file is compiled with
`-DZENO_MEMORY_REPORT`. **Why `atexit`,** and not a call at the end of
`main`: an error that stops the program should still print the report,
and the C build does it this way.

### 5.11r The text mark is a `load` put in afterwards

A statement that builds text is compiled with `%mark = load i64, ptr
@zeno_text_top` before it and `store i64 %mark, ptr @zeno_text_top`
after it. Whether a statement builds text is known only once it has
been compiled. So the emitter remembers the line where the statement
began, and inserts the `load` there afterwards if it is needed.

**Why not write both lines for every statement.** They cannot be
removed later. `zeno_text_top` is a global the runtime changes, so
LLVM must assume any call in between touches it, and a loop of
arithmetic that calls one function would load and store on every trip.

**Why not ask the syntax tree first,** as the C backend does with a
walk over the expression. The walk has to agree exactly with what the
emitter then does, for every kind of expression, and a disagreement is
a leak in the text room. Here the emitter sets one flag at the two
places where text is made, and cannot disagree with itself.

**Cost:** an instruction is inserted into lines already written. That
is safe because names in IR do not depend on position, and the mark is
inserted at a point where the block is open.

**A `for` or a `match` whose first line builds text** was refused at
first, as in C. Now each gives the text back at the first point where
it is no longer needed (5.9q).

### 5.11s In an f-string a number waits to become text

`f"{a} then {next()}"` works out `a`, then `next()`, and only then
turns the two into text. A list, an object or an optional is still
turned into text at once.

**Why.** With no collector, text made for `{a}` sits in the text room
while `next()` runs. If that call builds text of its own, or recurses
into this line, the room holds more at once than it needs to. The C
backend had never done that, and one test program used 44 bytes of
text through LLVM where C used 41. The comparison of memory reports
(5.11t) found it.

**Why only numbers, bools, quantities and text.** The value read for
`{a}` cannot be changed by a later call, so showing it later prints
the same. A list can be appended to by `next()`, and the f-string must
show it as it was when its turn came *(principle 1: same output as
the interpreter)*.

It is done in every LLVM build, not only with no collector. **Why.**
One shape of IR for an f-string, and the order of the runtime calls is
not something a program can see.

### 5.11t With no collector, the two backends must write the same memory report

Each no-collector test program is built through C and through LLVM
with `--memory`, and the two reports are compared as text: blocks
made, bytes, the most in use at once, blocks left, and the most text
in the room.

**Why so strict.** "Prints the same and leaves no block" does not
notice a free that comes late, or text held a statement too long. A
board with a 256-byte text room notices. The C build's numbers were
checked by hand when 5.9 was built, so they are the reference.

**Turned down: fixed numbers in the tests.** They would have to be
changed by hand whenever the runtime changes the size of a header.
Comparing the two builds needs no numbers.

With `ZENO_VALGRIND=1` the same tests run each program under valgrind.
**Why valgrind and not AddressSanitizer:** clang's sanitizer library
is not installed everywhere clang is, and valgrind needs nothing at
build time. It is off by default because it is slow. (Since the pool,
5.9k, it sees less: blocks no longer come from the heap.)

### 5.12 The board is four functions

Everything a compiled program does to hardware goes through four C
functions, and a count:

```c
void   zeno_board_write(int pin, bool level);
bool   zeno_board_read(int pin);
void   zeno_board_sleep(double seconds);
double zeno_board_now(void);          /* milliseconds */
#define ZENO_PINS 64
```

`on`, `off`, `toggle`, `is_on`, the check in `pin(n)` and the check in
`sleep` are written once in the runtime, on top of these.

**Why so few.** A port to a new board is these four functions and
nothing else, and each is one call to the vendor's library. Everything
that can be decided without knowing the board (is the pin number in
range, is the time positive, what does toggle mean) is decided above
the line, once, and tested once. **Turned down:** a function for each
thing a pin can do (`zeno_board_toggle` and so on). Faster on some
processors, which have a register for toggling, and four more
functions for every port to get wrong.

### 5.12a The simulated board is written twice, in Rust and in C

`src/board.rs` for `zeno run`, and thirty lines of `runtime.c` for a
compiled program. Each stage holds its own board: the interpreter and
the VM through the `Machine` trait, a compiled program in a C array.

**Why twice.** The interpreter is Rust and a compiled program is C, and
the tests hold the two to the same output, line for line, so they
cannot drift apart unnoticed. **Why not one board for the whole
process** in the interpreter: the tests run a program on the
interpreter and then on the VM, and the second must start with every
pin low.

### 5.12b A real board is a header, named with `-DZENO_BOARD`

```sh
cc -O2 -DZENO_BOARD='"my_board.h"' -o blink blink.c -lm
```

The runtime includes that file in place of the simulated board. A test
builds a program on a two-pin board of its own this way, for all three
kinds of C build.

**Why a header and not a library to link.** A Zeno program is one C
file with nothing to link (5.5). A header keeps that true, and lets
the C compiler inline a register write into the loop that does it.

### 5.12c `zeno build` does not cross-compile

It writes the C and builds it for the computer it is on. Building for
a microcontroller is done with that microcontroller's own compiler,
flags and linker script, by hand or from its project.

**Why.** Those tools exist, differ for every vendor, and are what a
firmware engineer already has set up. A `--target` option would be a
second, worse copy of each of them. *(Principle: keep what works.)*

### 5.13 A sized number is an ordinary int when the program runs

In the interpreter and the VM a `u8` is a `Value::Int`. In C it is an
`int64_t`, and in LLVM an `i64`. Only the checker knows its size. The
two things that make one, `u8( )` and `u8.wrap( )`, are the only code
that runs because of it.

**Why.** It is how units (1.15) and pins (3.15) already work: the
type exists to stop mistakes, and is gone afterwards. Because
arithmetic gives an `int` (2.8a), there is nothing a sized number does
differently once it exists, so there is nothing for a second kind of
value to do. The four engines agree for free, where a `Value::U8`
would have needed every operator taught about it four times.
*(Principle 5, and "keep what works".)*

**Cost.** In a program that is not strict, a value whose type is not
known until run time can be put in a `u8` variable unchecked, as it
can be put in an `int` variable today (0.2). A compiled program is
always strict (5.5c), so this is the interpreter only.

### 5.13a Only a field of an object is stored small *(was provisional: 11.2)*

A class with the fields `kind: u8` and `length: u16` is a struct of a
`uint8_t` and a `uint16_t` in C, and `{ i8, i16 }` in LLVM. A value is made
small as it goes into the field and an `int64_t` again as it comes
out. Variables, parameters and list items stay 64 bits wide.

**Why.** Memory is spent on what there are many of, and what there are
many of lives in objects. A local variable is in a register, where a
`u8` saves nothing and can cost an extra instruction. So the one place
the size reaches the generated code is the one place it pays.

**Why the read is cast.** In C, a `uint32_t` field used straight in a
sum is unsigned arithmetic: `-field` and `field < -1` come out wrong.
The cast to `int64_t` at the read is what keeps 2.8a true in C.

**Lists are the gap.** A `list[u8]` still uses eight bytes an item,
because the runtime has one list type (5.5h). A packed list, or a
`bytes` type, is the stage that fixes it.

### 5.14 A generic is compiled once for each set of types

`first[T]` called with a `list[int]` and a `list[str]` becomes two
functions, `first[int]` and `first[str]`. `Stack[int]` and `Stack[str]`
are two classes, each with its own struct. The generic itself is never
compiled.

**Why.** There are two ways to build generics. The other is *erasure*,
which Java uses: one function for every type, with each value behind a
pointer and its type forgotten. Erasure makes small programs. It also
means a `Stack[u8]` holds pointers to bytes and not bytes, every `>`
inside a generic is a call that is looked up while the program runs,
and a `Cell[float]` cannot hold its float in the struct. On a microcontroller those are the costs
that matter: memory per object, and time per operation. With copies, a
`Cell[u8]` is one byte, and `largest` over ints is a loop of integer
compares, exactly as if it had been written by hand. C++ and Rust made
the same choice for the same reason. *(Principle 5.)*

**Cost.** Code size. A generic used with ten types is ten functions.
The programmer can see this: each copy has a name in the C. Erasure
could be offered later for a generic where size matters more than
speed, as an explicit choice.

**Why it was cheap.** By the time the compilers run, every copy is an
ordinary function or class. The C backend and the LLVM backend each
changed in three small places: register the copies and not the
generic, call the copy the checker names, and spell the name. Sized
fields, owned classes, `--firmware` and `--no-collector` all worked
with generics without a line written for them.

### 5.14a A copy is a copy of the syntax tree

The checker makes `first[int]` by copying the tree of `first`
(`Function::fresh`), renaming it, and checking it in a scope where `T`
is a name for `int`.

**Why not substitute types in a table.** Every stage after the parser
keeps its notes by the address of a node: "this expression is a
`list[int]`", "this call gives its argument away". One body with two
sets of notes would need every table keyed by (node, copy), in the
checker and both compilers. A copy of the tree has addresses of its
own, so every table works as it is.

**Why `fresh` and not `clone`.** A function written inside another is
behind an `Arc`, which `clone` shares. Two copies would share the
inner function's notes, and the second would overwrite the first. A
test (`doubled`, in `generics_match`) fails if `fresh` is replaced by
`clone`.

**Cost.** Memory in the compiler, in proportion to the copies. The
checker stops with an error when a generic needs a bigger copy of
itself each time (`class Node[T] { next: Node[list[T]] or none }`),
which is the one way the copies can fail to end.

### 5.14b The interpreter and the VM run the generic itself

Neither engine changed. They run `first` as one function.

**Why.** When a program runs under `zeno run`, every value carries its
type, so there is nothing for a copy to add. This is erasure, in the
one place where erasure is free. The tests hold all four engines to
the same output, so a difference between "one function" and "many
copies" would show.

### 5.14c In C a copy is called `z_Stack_int`; in LLVM, `"z_Stack[int]"`

**Why.** C names allow letters, digits and `_`, so `Stack[int]` loses
its brackets: each run of other characters becomes one `_`. LLVM IR
allows any name in quotes, so there the copy keeps the name the error
messages use.

**Cost.** In C, `Stack[int]` and a class the programmer called
`Stack_int` would get the same name. The build refuses the program
and says why. A scheme that could never clash (C++ mangles `Stack<int>`
to `5StackIiE`) would make the C unreadable, and reading the C is half
the point of the C backend (5.5). *(Was provisional: 11.2.)*

### 5.14d A generic that nothing uses is checked with `T` as "anything"

**Why.** With no call there is no type to check the body with, and a
misspelt name inside it would wait for the first user to find. So the
checker makes one more copy in which every type parameter is the type
that fits everywhere, and checks that. It finds wrong names, wrong
counts of arguments and missing returns. It cannot find a `>` that
only some types have, because "anything" has every operator. That is
the gap bounds would close (3.16d).

### 5.15 With no collector, whoever receives a value owns it

In `zeno build --no-collector` every list and every object has one
owner, without the word `owned`. A list owns its items, in every build.
The walkthrough is `docs/one-owner.md`.

**Why.** 5.9g refused every value that had no owner and told the
programmer to write `owned`. A rule that is the same on every line is a
rule the compiler can apply itself, and a word that must be written
everywhere says nothing. *(Principle: punctuation, and words, have to
earn their place.)* Rust is built this way, and Rust is the proof that
the model is enough to write firmware in.

**Why only in that build.** With a collector, sharing is the easy
default and costs nothing to offer: `let b = a` gives two names for one
list, as in Python. Taking that away from `zeno run` would make the
first hour with Zeno harder for no gain there.

### 5.15a The checker is told which build it is checking for, and may only refuse

`checker::Rules { strict, one_owner }` replaces the single `strict`
flag. `Build::rules()` says what each build asks for, and
`zeno check --no-collector` asks for the same without building. This
takes back 5.9e ("the compiler refuses, not the checker") for one
question: who owns what.

**Why take it back.** 5.9e kept the checker the same for every build.
But the answer to "who owns this list?" decides which later lines are
errors (`print(a)` after `let b = a`), and only the checker follows a
value from line to line. Doing that a second time in two compilers
would be the borrow checker written three times.

**What keeps the language one language.** The mode adds errors and
changes nothing else. A program it accepts gives no value two owners,
and such a program cannot tell "owned" from "shared" by what it
prints. So the same file means the same under `zeno run`, `zeno build`
and `zeno build --no-collector`, and the tests run each accepted
program all three ways. *(Principle: one way to say a thing, with one
meaning.)*

**Turned down: a different default per build that changes behaviour**,
such as copying a list on `let b = a` when there is no collector. The
program would print different things in different builds.

**What the compilers still refuse themselves:** what has no place to
live yet (an enum case with values, a function value that captures, a
list of `int or none`). Those are about how a value is stored, not who
owns it, so 5.9e still holds for them.

### 5.15b A plain parameter takes, and `borrow` looks, as before

No new word came with this step. `function retire(task: Task)` takes
the task and frees it on return. `function report(task: borrow Task)`
looks.

**Why not borrow by default** (5.8c turned it down once already, as
provisional). It was weighed again, since in this build most
parameters only look and each now needs the word. It stays as it is:
`task: Task` then means the same on a `let`, a field and a parameter
("this holds the task"), and the error for a forgotten `borrow` names
the fix in its first line. *(Was provisional (11.2), and a question for the
audit: count how often each is written in real programs.)*

*Superseded by 5.19a.* The count was made: see there.

### 5.15c A list owns its items, in every build

An owned list may hold values that have one owner: objects of an owned
class, and with no collector other lists. Rule 6 of 5.8 ("shared may
not hold owned") stands. What changed is that an owned list is no
longer barred from it, which lifts the first line of 5.8i.

**Why.** A list of objects is the most ordinary data structure there
is: tasks, packets, readings. A no-collector build that cannot hold
one is a demonstration and not a tool.

**Why the list and not each item's maker.** The list is the one place
that knows when the items are no longer reachable: when it is freed
itself. That is rule 1 of ownership applied once more, with the list in
the role a field has in an object.

**The cost in an ordinary build.** A list of owned things must itself
be owned, and the annotation has to say so: `owned list[Packet]`.
`list[Packet]` of an `owned class Packet` is an error that says what
to write.

### 5.15d `[ ]` and `for` look, `append` gives, `pop()` takes, `xs[i] = v` replaces

Each way of touching an item has one meaning, whatever the item is.
`xs[i]` and the loop variable are looks: they can read, change and
lend the item and cannot keep it. `append` hands the list a new item.
`pop()` hands the item to whoever receives it. `xs[i] = v` frees the
old item and puts `v` in its place.

**Why `[ ]` does not take.** `let first = xs[0]` would have to leave a
hole in the list, or shift every later item, and neither is what
`xs[0]` reads as. `pop()` already means "take it out" in every
language that has it. *(Principle: read like plain words.)*

**Why `xs[i] = v` frees on the spot.** The old item has no other owner
and no name, so that line is the last that can free it. The new value
is worked out first, so a failure while making it leaves the list as
it was.

**`+`, `+=` and `filter` are refused** for lists whose items have one
owner: each would put one item in two lists. The message for each says
what to write. `map` and `reduce` lend each item to the function, so
they stay.

### 5.15e Taking an item out needs the owner, no loop over the list, and a line of its own

Three rules guard `pop()`, `xs[i] = v` and `obj.field = v` when what
leaves has one owner. The path must start at an owner, not at a
borrow, which includes `self`. No `for` or comprehension around it may
be walking a list reached from that owner. And the line must not have
looked at that owner already. After a `pop()`, the rest of the line
may not use the owner either. Inside such a loop the owner itself
cannot be given away or given a new value: both free every item.

**Why these three.** A look lives for one statement at most (5.8d rule
1), or for a loop's body. So there are exactly three places a look can
be alive when an item leaves: in a caller that lent it, in a loop
around the line, and earlier on the same line. Each rule closes one.
This is 5.8i's "only the owner replaces a field", said once for every
way an owned part can leave.

**The cost.** Safe programs are refused. `xs.append(xs.pop())` must be
two lines. A method cannot take an object out of a list that its own
object holds, so `Scheduler` cannot have `remove_last(self) -> Task`
yet. (Popping a number or text is fine: those are copied out.) The
second is the one that will be felt.

**Turned down: a second kind of borrow that may take out** (Rust's
`&mut`, with the rule that only one exists at a time). It is the
complete answer and the largest single piece of Rust's checker: every
borrow then has a region and two may not overlap. 5.8d chose four
rules a reader can hold in their head. *(Was provisional (11.2). If methods that
remove turn out to matter, the smallest growth is "a method may take
out when the object was not lent to anything else on that line", which
needs no new word.)*

**Turned down: check it when the program runs** (a count of looks in
each list, as Rust's `RefCell` does). It moves a mistake from build
time to run time. *(Principle 1.)*

### 5.15f One free function for each kind of list, written when a program needs it

`zeno_list_drop_1_Task` frees a list of tasks: each task with
`zeno_drop_Task`, then the list. `zeno_list_drop_2_Task` frees a list
of those. Lists of plain lists and of lists of text are
`zeno_list_drop_lists1` and `zeno_list_drop_texts1`. `Drops` in
`emit_c.rs` picks the name, both backends use it, and each function is
written only if something calls it.

**Why not one runtime function that takes "how to free an item".** It
would be shorter. The generated function has a name that says what it
frees, is a direct call the C compiler can inline, and shows up by
that name in a profile or a stack trace. *(5.5: reading the C is half
the point.)*

**Why the compilers do not carry "owned" in each type.** By the time
code is written, the checker has settled who owns what. All a compiler
needs is whether a kind of item must be freed with its list, and that
follows from the kind alone: an object of an owned class always, a
list when there is no collector. So `Kind` stays as it was and one
function, `Drops::owns`, answers.

### 5.15g The copy a loop walks is freed alone

`for task in tasks` walks a copy (3.10a). The copy is freed with
`zeno_list_drop`, never with the function that frees items.

**Why.** The copy holds the same items as the real list, which still
owns them. Freeing them through the copy would free them twice.
**Turned down: walk the real list when the items have one owner.** An
`append` in the body may move the list's storage, which is what 3.10a
is for. Skipping the copy when the body cannot append is an exercise
in `docs/one-owner.md`.

### 5.15h The `except` owns the error it caught, and the checker says so

5.9d's rule moved into the checker for this build: `e` in
`except Bad as e` is a look at the error. It can be read and lent to a
`borrow` parameter, which is new. Keeping it is refused in words about
errors, not about borrows.

**Why.** It is the same rule as a loop's item: something else owns the
value and frees it when the block ends. One mechanism now covers both.

### 5.15i The C backend assigns through an index, value first

`points[1].x = 30` and `grid[1][0] = 5` were refused by `zeno build`
and accepted by `zeno run` and the LLVM backend. They compile now. The
value is worked out first, then the list or object, then the index.

**Why now.** Lists of objects make these everyday lines. **Why that
order.** It is the interpreter's, and four engines must agree. The
test uses functions that print when they are called, so a change of
order fails it.

### 5.15j What a no-collector build still refuses *(was provisional: 11.2)*

This replaces the list in 5.9g. *(Step 4 took the first row off it. The
list as it stands is in 5.16i.)*

- A list or object inside an `or none`, an enum case with values, a
  list of `int or none`. *(Step 4.)*
- A function written where a value goes, a function type that mentions
  a list or an object, an error that is kept or raised again.
  *(Step 5.)*
- A list or object as what a type parameter stands for. The copy made
  for `T` would have to be checked as a value that moves.
- A function that uses a list or object declared at the top of the
  file (5.8d rule 4). A table of constants that several functions read
  is the commonest shape of firmware data, so this one matters. A
  top-level `let` that the program never gives away could be lent to
  every function without being passed. *(For the audit.)*
- A method that takes out what its object owns (5.15e).

**A note for 5.9q's known gap.** It is wider than was written there:
one `print` that shows two lists is enough for the C and LLVM builds
to report text-room peaks a byte or more apart. What is printed is the
same. The fix described there still stands, and the test programs
still avoid the mix.

### 5.16 With no collector, an enum value that carries something has one owner

`Wait(250)` and `Blink([2, 3, 5])` are kept in a block from the pool,
as an object is, and whoever receives one owns it and frees it. The
walkthrough is `docs/enums-and-none.md`.

**Why.** It is 5.15 again, for the last kind of value that was made
and never freed. One rule for lists, objects and enum values means
there is nothing new to learn: a programmer who has read
`docs/one-owner.md` already knows what `let b = a` does to a command.
*(Principle: unambiguous and intuitive. One rule, not one per kind of
value.)*

**Turned down: keep the value in place, with no block, like a C tagged
union passed by value.** It would need no pool at all for a `Wait`. But
a case may carry a list, which has one owner, so a copy of the value
would give that list two. Copying would then have to be refused for
some enums and not others, depending on what the cases carry. List
slots are 8 bytes, so an enum bigger than that could not sit in a list
without a second way of storing it. And the two backends would each
need a second representation of every enum. A block of the case's own
size (5.16d) costs a `Wait` 16 bytes of pool and keeps one
representation everywhere.

### 5.16a A case that carries nothing is a fixed value, and a plain enum is copied

There is one `Stop` in a program, placed before it starts. Making it
takes no memory and freeing it does nothing. An enum in which no case
carries anything (a `Mode` that is `Idle`, `Busy` or `Halted`) is
not owned at all: its values are copied like numbers in every build.

**Why.** A state or a mode is the commonest enum in firmware, and
making every `Idle` cost a block, or making `let b = a` take `a` away
for a value with nothing in it, would be a surprise with no safety
bought. **Why the type decides and not the case.** A variable of type
`Command` has one owner even at a moment when it holds `Stop`. Which
case a value is at some line is not known when the program is checked,
and a rule that depended on it could not be read off the page.

### 5.16b What may be `none` can have one owner, in every build

`Command or none`, `Packet or none` and `list[int] or none` are owned
when the thing inside is. `none` owns nothing. This holds in a build
with a collector too, when the words are written
(`owned list[int] or none`, a `Packet or none` of an `owned class`).

**Why now.** The old refusal (`docs/ownership.md` said "taking one
back out would have to leave a hole") mixed up two things. A *variable* that may be `none` is an ordinary
owner: it is given away whole, and the checker already knows a
given-away variable holds nothing. Only a *field* has the hole
problem, and a field is already protected by the rule that nothing
moves out of an object. So nothing new was needed, only the refusal
taken away.

**How it is written down in the checker.** `or none` is always the
outermost part of the type: an owner is "maybe (owned X)" and a look
is "maybe (look at X)". So `borrow Packet or none` reads as it is
written, a look at a packet or at nothing, and `is not none` narrows
both the same way. **Turned down: an owned "maybe X".** Narrowing would
then have to rebuild the type from the inside, and a look at it would
be a third shape.

**Still refused inside a function type and the `[ ]` of a generic**,
with every other value that has one owner (5.15j).

### 5.16c `match` looks, and a name in a pattern is a copy or a look

`match` never takes anything out of the value it is given. In
`case Blink(pins)` the name is a look at the list. In `case Wait(ms)`
and `case Say(text)` the number and the text are copied. `case other`
is a look at the whole value.

**Why.** Reading a command must not use it up: `cost(command)` and
then `run(command)` is the ordinary shape. Rust lets a `match` move
parts out, and then has "partially moved" values that can be used in
some ways and not others. That is hard to hold in the head, and the
unambiguity rule is against it. If taking a part out is ever wanted
it will be a word of its own, not a second meaning of `match`.

**Why text is copied.** Text is copied whenever it is kept (5.9l), so
this is the existing rule and not a new one. The copy is freed when
the `case` ends.

### 5.16d While a `case` holds a look, the owner cannot change

If a pattern gave a name to a part that is looked at, then for the
length of that `case` the owner of the matched value cannot be given
away, be given a new value, or have items taken out. A `case` with no
such name is free to do all three.

**Why.** The look points into the owner's block. `state = Stop` would
free that block under it. This is the loop rule of 5.15e, for the same
reason, and the checker keeps both in one list (`owned.walked`, with
`Looking::Loop` or `Looking::Case`), so the messages differ and the
mechanism does not.

**Why only when there is a look.** A state machine is
`match state { case Waiting(ms) { state = Running } ... }`. `ms` is a
number, copied out, and nothing points into the old state. Refusing
that would make the most natural use of an enum impossible. The
message for the refused case says what to do: write `_` for a name
that is not needed.

### 5.16e `borrow` can be written on an enum that carries values, in every build

`function cost(command: borrow Command)`. With a collector it changes
nothing about memory.

**Why.** The same file has to build with and without a collector
(5.15a). If `borrow` on an enum were an error with a collector, a
program written for `--no-collector` would not run under `zeno run`.
On an enum that carries nothing it is still an error, with a message
that says to pass the value as it is: there is nothing to lend.

### 5.16f In C, each case takes its own size from the pool

`zeno_new_Command_Wait` asks for `offsetof(z_Command, as) +
sizeof(as.c1)`, not `sizeof(z_Command)`. This is in every C build.

**Why.** The LLVM backend has always made a struct for each case
(5.11h), and the two backends must give the same memory report
(5.11t). And on a small pool it matters: one large case would
otherwise make every small one pay for it. **The cost.** A value can
never change its case in place, since its block may be too small for
another. Zeno has no way to write that, and "a new value takes the
place of the old one" is the rule everywhere else, so nothing is lost.

### 5.16g Found on the way: a case pattern on a value that may be `none`

`case Stop` and `case Wait(ms)` on a `Command or none` were compiled
wrongly by both backends, with or without a collector: the first as a
new name that matches anything, the second refused. The interpreter
and the VM were right. Both backends now test "is it there" and then
the tag.

**Why it is written down.** Four engines must agree, and this one
slipped through because no test matched an enum case against an
optional subject. `a_case_of_an_enum_matches_a_value_that_may_be_none`
does now.

### 5.16h `list[int or none]` stays refused with no collector *(was provisional: 11.2)*

A number that may be `none` is kept in a small block of its own when
it sits in a list (5.11i). Giving those blocks an owner would mean a
deep copy in every place a list is copied, as text needed (5.9x).

**Why not now.** There is a way to write it today that costs less and
says more: an enum `Reading` with a `Missing` case and a
`Value(n: int)` case, and a `list[Reading]`. `Missing` takes no memory and the list owns each
`Value`. The compiler's message says so. The right fix is to keep the
pair in the slot without a block, which touches the slot's size, and
is for later. *(Keep whatever works.)*

### 5.16i What a no-collector build still refuses *(was provisional: 11.2)*

This replaces the list in 5.15j. *(Replaced in turn by 5.17g: the
step 5 items below build now.)*

- A function written where a value goes, a function type that mentions
  a list or an object, an error that is kept or raised again.
  *(Step 5.)*
- A value that has one owner inside a function type or as what a type
  parameter stands for.
- `list[int or none]` (5.16h).

**For the audit that follows these steps**, all of them about what is
intuitive rather than what is safe:

- A function cannot use a list or object declared at the top of the
  file (5.15j). A top-level `let` that is never given away could be
  lent to every function without being passed.
- A method cannot take out what its object owns, because `self` is a
  borrow (5.15e). `machine.waiting.pop()` has to be written outside
  the class today.
- A plain parameter takes and `borrow` looks (5.15b). Most functions
  only look, so most parameters carry the word. `examples/tour.zeno`
  and `examples/shapes.zeno` do not build with no collector for want
  of it. Whether looking should be the plain form is a question about
  the direction of the language, and is the owner's to answer.
  *(Answered: 5.17h.)*
- After `case none`, a later `case name` still has the type
  `T or none`. The checker could narrow it.

### 5.17 With no collector, an error has one owner: `raise` takes it and `except` owns it

An error used to be the one value that lived by a special rule: its
`except` freed it, so it could not be kept, handed on or raised again,
and it could only be made by `raise`. Now it is a value like any other.
`raise x` takes `x`. The `except` that catches it receives it, and the
name after `as` owns it. At the end of the block the name frees it,
unless it was kept, returned, given away or raised again.

**Why.** Every other rule in this build is "whoever receives a value
owns it". One exception to that is one more thing to remember, and the
programs it refused are ordinary firmware: log the last fault, keep a
list of them, pass a fault up after noting it. With lists and objects
owned (5.15) nothing stood in the way any more.

**Turned down: keep the old rule and add `e.copy()`.** A copy of an
object that holds a list is a deep copy, which costs pool and time the
reader cannot see from the word.

**Unambiguous and intuitive.** There is one rule for every value now.
Nothing new is written: `raise fault`, `faults.append(fault)` and
`last = fault` mean what they mean for any owner, and the messages for
"given away" and "only borrowed" are the ones already known.

### 5.17a An error of a kind Zeno knows has one owner too, and can be made anywhere

`ValueError("x")` makes an error whether or not `raise` stands in front
of it. `let ready = ValueError("made early")` is allowed, and so is a
parameter of type `ValueError`, a `list[ValueError]`, and
`borrow ValueError`.

**Why.** "Only `raise` can make one" existed because nothing else could
free it. With an owner, that reason is gone, and an error made ahead of
time is useful (one error for every caller of a table of checks).

**In the checker** such an error is an owned `Error(kind)`. An error
that is an object of a class needed nothing: with no collector an
object already has one owner.

### 5.17b With no collector, a function value has one owner, and a function of the program is a fixed value

A function written where a value goes (`function(n: int) -> int { ... }`)
is kept in a small block with what it uses. That block has one owner,
like a list. A function of the program, used by name (`double`), is not
in a block: it is one fixed value that is never freed, like an enum case
that carries nothing (5.16a). The **type** says which rules apply:

- `var f: function(int) -> int` can hold either kind. It is an owner:
  giving it away empties it, and a new value frees the old one. Freeing
  a function of the program does nothing.
- `let f = double`, with no type written, holds that one function. It is
  not an owner, and can be used any number of times.

**Why the type decides and not the value.** The checker must know at
every line whether a name still holds something, without running the
program. A variable that can hold either kind has to follow the stricter
rule.

**Turned down: counting who holds each function.** That is a small
collector, and this build promises none. **Turned down: copying the
block when it is kept twice.** A copy is easy for numbers, but a function
that keeps text would copy the text too, and a hidden copy is exactly
what this build is for avoiding.

**For the audit.** `var f = double` followed by `f = other` is refused,
because without a written type the variable holds that one function. The
message says to write the type. Inferring `function(int) -> int` for a
`var` would be less surprising.

### 5.17c What a function written where a value goes may use from outside it

| It uses | What happens | Why |
|---|---|---|
| a `let` number, bool or plain enum | the value is copied into the block | it never changes, so a copy is the same thing |
| a `let` text | a copy of the text is kept in the block and freed with it | text is kept by copy everywhere in this build (5.9l) |
| a `var` of the function around it | **refused** | the two would share one variable, and with no collector nothing is shared |
| a `var` or `let` at the top of the file | used where it is | it is the program's, and lives as long as the program |
| a list, an object, or anything else with one owner | it is **looked at** (5.17d) | taking it would silently empty the variable |

**Why refuse the `var`.** In `zeno run` the inner function and the outer
one see the same variable, so a change by one is seen by the other. To
do that, the variable has to live in a block that both hold, which is
two owners. Copying it instead would compile, and print something else
than `zeno run` prints. A refusal is the only honest answer. The message
says what to write: `let` if it does not change, an object if it does.

**Turned down: a word such as `move` on the function, as Rust has.**
It would be a second way to write a function, for a case that has an
answer already: give the function a parameter.

### 5.17d A function that looks at a value belongs to the line that writes it

A function that uses a list or an object from outside looks at it, and
a look must be gone before the owner is. So the function is itself a
look: the statement that writes it owns it and frees it. It can be
passed to `map`, `filter`, `reduce` or a `borrow` parameter. It cannot
be put in a variable, returned or given away, and it cannot take an item
out of what it looks at. While that statement runs, the value it uses
counts as being looked at, so it cannot be given away on the same line.

**Why.** This is the common case: `readings.map(function(r) { ... })`
with something from the surroundings in the body. It is safe with no
new rule, because a borrow that lasts one statement is what a `borrow`
parameter already is.

**Turned down: let it be kept, and stop the owner from changing until
the function is gone.** That is a borrow with a lifetime, which Rust
needs names and annotations for. One statement is a lifetime nobody has
to write.

**Was provisional (11.2).** A function that uses another function value which has
an owner (`function(n) { return f(f(n)) }` where `f` is a parameter)
looks at it, so it cannot be returned. Writing "compose" needs a way to
take a value into a function. To be looked at with real programs.

### 5.17e A function type says what each parameter does, in every build

`function(borrow list[int]) -> int` is a function that looks at a list.
`function(Packet) -> int` takes the packet. A function can return a list
or an object, and the caller owns it. Each parameter inside a function
type is read by the same rule as a parameter of a function, so a call
through a function value lends or gives exactly as a call by name does.
A function fits such a type only if its parameters do the same thing.

This holds with a collector as well, for `owned class` and
`owned list`. It replaces the refusal of a value with one owner inside a
function type (5.16i).

**Why.** Before, a function type could not say `borrow`, so no function
value could be handed a list, and `map` over a list of objects was the
only way in. **Still refused:** a value with one owner as what a type
parameter stands for (`same[T](x: T)` with a list), because the copy for
`T` would have to know that it moves.

### 5.17f In C and IR: a `drop` entry in the block, only with no collector

A function value is `{ code, name }` followed by what it uses. With no
collector there is a third entry, `drop`: the function that frees this
block. It is `zeno_closure_release` when the block keeps no text, a
function written for that one literal when it does (it frees the text
first), and NULL for a function of the program. The owner calls
`zeno_closure_drop`, which does nothing for NULL.

Every literal gets a block of its own, even one that uses nothing.
**Why.** Two values made by the same literal are different functions in
`zeno run` (`==` says so), and one rule for every literal is simpler
than two. It costs one small block of three words.

A build with a collector has no `drop` entry and is unchanged.

### 5.17g What a no-collector build still refuses *(was provisional: 11.2)*

This replaces the list in 5.16i.

- A function that uses a `var` of the function around it (5.17c), and
  keeping a function that looks (5.17d). Both by design.
- A value that has one owner as what a type parameter stands for.
- `list[int or none]` (5.16h).
- A function that can raise, used as a value (the `--firmware` rule,
  since errors travel as values in this build).
- A method used as a value (`let m = counter.bump`).

### 5.17h Decided: a plain parameter will look, and a word will take

Asked of the owner of the language on 2026-11-15, and answered: a plain
parameter should look at a list or an object, and a word should mark
the parameter that takes it. Today it is the other way round (5.15b):
plain takes and `borrow` looks.

**Why.** Most functions only look, so today most parameters carry a
word and the rare one that takes carries none. The common thing should
be the plain thing, and the thing that empties the caller's variable
should be the one that is visible.

**When.** In the audit stage, after manual memory. It touches the
checker, both backends' tests, every example and every page that shows
`borrow`, so it is done in one piece and not mixed into another stage.
The word is to be chosen then, with the reasons written here.

*(Done: 5.19 to 5.19o.)*

**The audit's list**, carried on from 5.16i: functions reaching
top-level lists and objects, methods taking out what the object owns,
narrowing after `case none`, the order C evaluates arguments in (5.9q),
`list[int] + list[str]` under `--strict`, `list[int or none]`, and
`var f = double` (5.17b).

### 5.18 Memory by hand lives beside the other two, and only inside `unsafe { }`

Asked by the owner of the language: can C-like manual memory still be
used, with the collector and with one owner already in place? Yes. A
program asks for memory with `allocate`, uses it through a pointer, and
gives it back with `free`. The lines that Zeno cannot check go inside
`unsafe { }` (docs/unsafe.md).

**Why have it.** Firmware needs two things that neither the collector
nor one owner can give: a register at an address the data sheet names,
and now and then a block of bytes laid out exactly as the program says.
Without them the language stops one step short of the hardware.

**Why the block.** A wrong pointer damages something far from the line
that is wrong. If every such line is inside a marked block, the search
for the mistake has an end: `grep -n unsafe`. Everything outside the
blocks keeps every promise Zeno made before this stage.

**Turned down: no block, as in C.** Then any line may be the one, and
the promises hold nowhere. **Turned down: a flag for the whole file or
build.** It says "somewhere in here", which is what the block is for
avoiding.

**Checked against "unambiguous and intuitive".** One word, one meaning
(5.18a). One way to write each thing (5.18c, 5.18d). The messages say
what to write. The one thing that is not intuitive, that a compiled
program does not check, is said in every message of `zeno run` (5.18e).

### 5.18a `unsafe` is a reserved word, and the block is a statement that opens a scope

`unsafe { ... }` is a statement, like the body of an `if`. What is
declared inside ends with it. The word cannot be used as a name.

**Why reserved.** The first version made `unsafe` an ordinary name
except in front of `{`. It worked, and it meant that `if unsafe {` was
a condition while `unsafe {` was the block: one word with two meanings,
told apart by where it stands. Nothing is lost by reserving it.

**Why a statement.** `if` and `match` are statements in Zeno. A block
that gives a value would be the only one. `let x = unsafe { p[0] }` is
refused, and the message says to put the whole line inside.

**Why it opens a scope.** Every `{ }` in Zeno does. One that did not
would be a surprise of its own. The cost is real: a value read inside
the block and used after it needs a `var` declared before the block, or
the `return` goes inside, as `Queue.get` does in `driver.zeno`.

**Turned down: `unsafe function`, as Rust has.** It would be a second
way to mark the same thing. A function that takes a pointer is safe to
call. Only the lines that use the pointer are not.

**For the audit.** `owned`, `borrow` and `strict` are still ordinary
names except in one place each. They should be looked at with the same
eyes.

### 5.18b Five things need the block, and nothing else does

Reading through a pointer, writing through one, `free`, reading a
register and writing a register. Making a pointer or a register,
passing it, keeping it in a field or a list, `p + n`, `p - n`, `==` and
`print` are safe anywhere.

**Why.** Those five are exactly the things that can damage memory or
hardware. An address that points nowhere harms nothing until it is
used, so working one out is not marked. If harmless lines needed the
block too, the blocks would grow, and a big block says less.

**Why `allocate` is safe.** It can fail in one way, by running out,
and that stops the program with a message. It damages nothing.

A function written inside the block starts safe again. It is a new
function and may be called from anywhere, so its body needs its own
block. The checker counts blocks per function (`Frame.unsafe_blocks`).

### 5.18c `pointer[T]` points at numbers of one size

`T` is `int`, `float`, or one of `u8 u16 u32 i8 i16 i32`. `p[i]` is
item `i`. `p + n` and `p - n` move by items. Two pointers compare with
`==`.

**Why `pointer[u8]`.** It is written like `list[int]`, which every
Zeno programmer already reads. **Turned down: `*u8`, `ptr[u8]`.** The
syntax direction is whole words and little punctuation.

**Why numbers only.** A bool has no size the language promises. An
object or a list has an owner or the collector, and a raw pointer to
one would be a second way to reach it that neither knows about.

**Why items and not bytes.** `(p + 1)[0]` and `p[1]` must be the same
thing. It is also what C does, so nothing has to be unlearned.

**Why `1 + p` is refused.** One way to write it: the pointer, then how
far. The message says so.

**Turned down for now: `<` between pointers, `p - q`, reading a
`pointer[u8]` as a `pointer[u32]`, the address of a variable.** Nothing
written so far needs them, and each is a new way to be wrong. They can
be added when a program asks.

What is written through a pointer follows the sized-number rules
(2.8 to 2.8c and docs/sized-ints.md), so `p[0] = 300` on a `pointer[u8]` is an
error as you compile, and arithmetic needs `u8(...)`.

### 5.18d `allocate(u8, 16)` and `free(p)`

The type of one item first, then how many. The memory is all zero.
Fewer than one item stops the program. `free` wants the pointer that
`allocate` gave.

**Why the type first.** The type decides what comes back, so it is
read first. `register(u32, address)` has the same order.

**Why whole words.** **Turned down: `malloc`**, a name that says
nothing to someone who has not met C. **Turned down: `alloc`**, an
abbreviation. `free` is already the whole word.

**Why zero.** C's `malloc` gives whatever was there before. A program
that read it would print one thing under `zeno run` and another when
compiled. Zeroing costs one `calloc`.

**Why running out stops the program.** Giving back `none` would put a
check on every `allocate`, for a case most programs cannot recover
from. *Was provisional (11.2)*: firmware that must survive it will want
`pointer[u8] or none`.

There is no null pointer. A pointer that may be missing is
`pointer[u8] or none`, checked like any other value that may be `none`.

### 5.18e `zeno run` checks every use of a pointer, and a compiled program checks nothing

`zeno run` and the VM keep the block behind each pointer: its length
and whether it was freed. Out of bounds, use after `free`, a second
`free`, and `free` of a pointer into the middle each stop the program
at that line. A compiled program has the address only, as in C.

**Why not check when compiled.** A check needs the length and a freed
mark to travel with every pointer. Then a pointer is no longer an
address: it cannot be handed to C or taken from it, and every access
costs a comparison. Zeno has a checked sequence already. It is called
a list. A pointer is for when the check is what you want to leave out.

**Why check in `zeno run`.** It is the cheapest place to find these
mistakes, and it makes `zeno run` the tool that C programmers reach
for a sanitizer for. Every message says that a compiled program would
not have stopped, so nobody learns the wrong thing from it.

This is the one place where `zeno run` and a compiled program may
differ, and only for a program that is wrong.

### 5.18f Memory by hand comes from the heap, outside the collector, or from the pool

In an ordinary build and with `--firmware`, `allocate` is `calloc`. The
collector does not scan the block and never frees it. With
`--no-collector` there is no heap (5.9k), so it comes from the pool.

**Why outside the collector.** The program said it would free it. A
collector that freed it too would free it twice. The block holds only
numbers (5.18c), so there is nothing in it for the collector to find.

`--memory` adds one line where there is a collector: how many blocks
`allocate` gave and how many were never freed. With no collector the
ordinary report already says it: blocks left at the end.

An object that holds a pointer does not free what it points at when
its owner frees the object. Memory asked for by hand is given back by
hand.

### 5.18g A register is its own type: `Register[u32]`, from `board`

`register(u32, 0x4002_0014)` gives a `Register[u32]`. `r.read()` and
`r.write(v)` go inside `unsafe { }`. The size is `u8`, `u16` or `u32`.

**Turned down: a register as a pointer at a fixed address, as C writes
it.** That needs turning a number into a pointer, which is the most
dangerous thing a pointer can do, and it would be needed in every
driver. And memory and a register are not the same: the C compiler may
skip or repeat a memory access, and must not do that to a register.
Two types let each be compiled as what it is: `[ ]` for one, one
`volatile` access for the other.

**Why `read()` and `write(v)`, and not a field.** Reading a register
can change the hardware. It should look like something that happens,
which a call does and a field does not.

**Why the capital letter.** What the language has built in is written
small (`int`, `list`, `pointer`). What comes from a module is written
like a class (`Pin`, `Register`).

**Why unsigned only.** A register is a pattern of bits.

**No helpers such as `set_bit`.** `|`, `&` and `~` already do it, and
keep whatever works.

### 5.18h The simulated board prints every write to a register

A pin that is driven high twice prints one line, because it is still
high. A register prints a line for every write, also one that changes
nothing.

**Why.** On real hardware a write is an event. The same byte written
twice to a serial port is two bytes sent. The first version printed
only on a change, and the driver example then lost a line whenever two
equal bytes followed each other.

### 5.18i Found on the way

- `bool` used as a value (`allocate(bool, 4)`) said "`bool` is not
  defined". It is defined, as a type. The message now says that.
- The C backend wrote an unreachable second `return` after
  `unsafe { return p[0] }`. A block that ends in `return` now ends the
  function.

**For the audit**, added to the list in 5.17h: the three words that are
names except in one place (5.18a); a `--firmware` build cannot catch an
`OverflowError` that `u8(...)` raises, which `zeno run` can; and
messages that say "1 blocks" or "1 item(s)".

### 5.19 The audit: every part of the language is held against "unambiguous and intuitive"

The owner of the language asked for it in one sentence: the entire
language must be unambiguous and intuitive. Each feature was read again
with five questions, and what failed one was changed or written down
with its reason (docs/audit.md).

1. Is there one obvious way to write it?
2. Can any line be read two ways, by the parser or by a person?
3. Does anything happen that the line does not say?
4. Is every word a plain word, with one meaning?
5. When it is refused, does the message say what to write instead?

**Why these five.** They are principles 1, 2 and 4 turned into
questions that have a yes or a no. "Intuitive" on its own cannot be
tested. "Does the line say what happens" can.

**What an audit may change.** Anything, including what earlier
decisions settled, as long as the earlier entry stays in this file with
a line that says what replaced it. The reasons of the time are part of
the record.

### 5.19a A parameter looks, unless it says `take`

```zeno
function report(task: Task) -> str { ... }            # looks
function retire(task: take Task) -> int { ... }       # takes, and frees
```

A parameter written with its type alone looks at the list, object,
function or error it is given. The caller still has it afterwards. A
parameter that takes the value over says `take` in front of its type,
and then the caller's variable holds nothing. This carries out 5.17h,
and replaces 5.8c and 5.15b, where plain took and `borrow` looked.

**Why.** Question 3. `retire(task)` emptied the caller's variable, and
nothing on either line said so: the reader had to know that a parameter
with no word takes. Now the line that empties a variable is the one
that carries a word.

**The count that 5.15b asked for.** In the example programs 10
parameters only looked and 7 took. In the programs the C backend is
tested with, which are longer, 57 looked and 12 took. Under the old
rule the larger group carried a word. Under this one the smaller does.
The count alone would not have settled it for the examples. The reason
above does.

**Why it is safe to make looking the default.** A look cannot be kept,
returned or given away, and the checker already enforced that for
`borrow`. So a forgotten `take` is always an error at the line that
tries to keep the value, and the message names the word. A forgotten
`borrow` used to be silent until the caller used its variable again.

**`self` always looks**, as before. A method that made its own object
vanish would surprise everyone.

**Turned down: a mark at the call as well** (`retire(take task)`). It
would show the move where it happens. But the checker reports any use
after a move with the line of the move, and one way to write a call is
worth more than a second reminder. *(Was provisional (11.2), as in 5.8a.)*

### 5.19b The word is `take`, and it stands in front of the type

`frame: take Frame`, and in a function type `function(take Frame) -> int`.

**Why `take`.** It is what the function does, said from the function's
side, in the word the messages already used ("takes the value over").
It reads as a sentence: "send takes a frame".

**Turned down: `owned`.** It already says what a value *is* ("this list
has one owner"). On a parameter it would also have to say what the
function *does*, and in an ordinary build `xs: owned list[int]` is
needed for the first meaning. One word, two meanings: question 4.
**Turned down: `move`.** It describes the call from the caller's side,
and it is a term from C++ and Rust more than a plain word.
**Turned down: `keep`.** A function that takes a frame and sends it
does not keep it. **Turned down: `own`.** One letter from `owned`.

**Why in front of the type and not the name.** `owned` goes there, and
a function type has no parameter names to put it in front of.

**`take` on a number is refused.** A number, a `bool` and a text are
copied, so there is nothing to take, and a word that did nothing would
be noise. The message says to write the type alone.

### 5.19c `owned`, `take` and `strict` are reserved words, and `borrow` is gone

A word of the language is never also a name. `let take = 3`,
`let owned = 1` and `queue.take()` are refused, and the message says
where the word goes. This replaces the part of 5.8a that left the
words unreserved, and finishes what 5.18a began with `unsafe`.

**Why.** Question 2. `strict` on the first line set a mode and on the
second line was a variable. `owned` in front of `class` was a word and
in front of `=` was a name. The parser could always tell, so nothing
was ambiguous to the machine. A person had to stop and work it out.
One rule with no exceptions is easier to hold than "reserved, except
these three".

**What it costs.** `take` is a good name for a method of a queue.
`driver.zeno` had one and now calls it `get`. That is the whole cost
found so far.

**Type names stay unreserved** (`int`, `list`, `pointer`). They are
names, of types, and a type and a variable never stand in the same
place.

**`borrow` is no longer a word.** Writing it in front of a type gives
"`borrow` is no longer a word. A parameter looks at what it is given
without one: write the type alone. A parameter that takes the value
over says `take`". Everywhere else it is an ordinary name again.

**Why not keep `borrow` as an optional word.** Then there would be two
ways to write a parameter that looks: question 1.

### 5.19d Where values are shared, `owned list[T]` on a parameter looks at any list

In a build with a collector there are two kinds of list, and a
parameter can ask for either:

| Written | Means | Accepts |
|---|---|---|
| `xs: list[int]` | a shared list, which the function may keep | shared lists |
| `xs: owned list[int]` | a look at a list | any list, owned or shared |
| `xs: take owned list[int]` | the function takes an owned list over | owned lists |

An owned list passed to a plain `list[int]` is refused, and the message
says to write `owned list[int]`. An `owned class` needs no word: its
type already says that it has one owner, so `frame: Frame` looks and
`frame: take Frame` takes.

**Why the middle row accepts a shared list.** Anyone may look at a
shared value. A function that only reads a list should be usable with
every list, and this is how to write that function once.

**`take` on a shared value changes nothing.** `take Point`, where
`Point` is a plain class in a build with a collector, is allowed and
does what `Point` does. So a program written for a build with no
collector, where the word matters, is the same program in every build.

### 5.19e One program that a build with no collector accepts and an ordinary build refuses

5.15a promised that the rules for one owner only ever refuse. There is
now one case the other way round:

```zeno
function total(xs: list[int]) -> int { return len(xs) }
let window: owned list[int] = [0, 0]
print(total(window))
```

With no collector every list has one owner and `xs` looks, so this is
fine. With a collector `xs: list[int]` is a shared list that `total`
may keep, and `window` cannot go there.

**Why it is left.** Accepting it with a collector would let a shared
place hold an owned list (rule 6 of 5.8). Refusing it with no collector
would mean that the word `owned` changes something in that build,
which 5.15 says it does not. The message names the fix, and the fixed
program (`xs: owned list[int]`) builds both ways. A program that never
writes `owned`, or writes it on every list it means, never meets this.

### 5.19f What `take` cannot do yet

- A type parameter cannot be taken: `function keep[T](x: take T)` is
  refused, because `T` may be a number. A value with one owner cannot
  fill a type parameter at all yet (noted in docs/generics.md).
- A parameter with no type is only allowed where values are shared
  (`--no-collector` needs every type written), so the question of what
  it does with an owner does not come up.


### 5.19g `+` on two lists follows the rule `append` has

`xs.append("three")` on a `list[int]` was always refused. `xs += ["three"]`
and `[1, 2] + ["three"]` were not, even under `--strict`. Now `+` and
`+=` refuse two lists whose item types are both known and have nothing
in common: "`+` joins two lists that hold the same kind of thing, and
these are list[int] and list[str]".

**Why.** Two ways to add to a list with two different rules fails
question 1 of the audit (one way to write each thing, and the same
answer from each way). The compilers also had to give such a list the
kind "anything", which a program that wrote types everywhere never
asked for.

**What still joins.** A list that may hold `none` with one that does
not (the result may hold `none`), and a list whose items are not known
yet, such as `[]`. The test is exactly: neither side holds "anything",
and the joined type would.

**Turned down.** Refusing only under `--strict`. `append` refuses
without it, so `+` would still disagree with `append` in an ordinary
check.

### 5.19h After `case none`, the later cases know the value is there

In a `match` on a value that may be `none`, every case after
`case none` sees the value without the `none`. It works for a new name
(`case point { point.name }`) and for the variable itself
(`case _ { found.name }`). If every `case none` leaves (`return`,
`raise`, `break`, `continue`), the lines after the `match` know too.
An enum that may be `none` is complete with `case none` and a case for
each of its own, and needs no `case _`. A missing `none` is named:
"this `match` does not handle `none`".

**Why.** `if found is none { return } ` already taught the lines below
it. A `match` that said the same thing and was not believed made the
writer ask twice, and there were two spellings of one test with two
different results (questions 1 and 3).

**How.** The checker narrows the name in its scope, the same record
that `if x is not none` makes, so both compilers unwrap the value with
the code they already had. A new name is declared with the whole type
and then narrowed, because the compilers store an `int or none`
differently from an `int` and need to know that the box is still there.

**Turned down.** Narrowing only the new name and not the variable. The
`else` of an `if` narrows the variable, so `match` would have been the
odd one.

### 5.19i Every function can look at a `let` at the top of the file

With one owner (an `owned` value, or any list or object with no
collector), a `let` at the top of the file can be read by any function,
any method and any function that is kept in a variable. They may change
what is inside it (`table[0] = 5`, `tasks.append(t)`), but not take an
item out or put a new value in an owned field. In return the top of the
file may never give that `let` away, before or after the function is
written: "`table` is used by a function, on line 3, so it cannot be
given away".

**Why.** A table or a configuration object at the top of the file,
read by the functions below it, is the first thing a firmware program
writes. The old refusal was sound and nobody would guess it (question
3: nothing surprising). A `let` cannot be given a new value, so if it
is never given away it holds the same value until the program ends,
and a look at it can never outlive it. That is a proof, and it needs no
new word.

**Why not a `var`.** A `var` can be given a new value, which frees the
old one while a function may be looking at it. The message says so and
names `let` as the fix when the value never changes.

**Why a function may not take out of it.** The top of the file owns
it. A function that popped from it would be doing the owner's work
from a place that cannot see what the owner's loops are looking at.
A method of the object can (5.19k), because the call is then checked
where the owner is.

**Found on the way.** A name used above the line that declares it said
"`table` is not defined". It now says "`table` is declared on line N,
which is further down", and that the declaration has to come first.
The rule itself (a name is usable below its declaration, in functions
and methods too) stays: one rule for every name.

**Turned down.** A word such as `global` or `static`. `let` at the top
of the file already says everything the word would.

### 5.19k A method can take out what its own object holds

`function next(self) -> Task { return self.tasks.pop() }` and
`function remember(self, task: take Task) { self.last = task }` are
allowed. They were refused because `self` only looks, and taking out or
replacing is for the owner.

**The rule moves to the caller.** The checker reads the method's body.
If it takes out or replaces something `self` owns, the method is
marked, and each call of it is checked as the same act done by hand:
only the owner of the object may call it, not inside a loop or a
`match` that is looking at something the object owns, not on a line
that has already looked at it, and not with an argument that looks at
the same object. Inside the method the loops and cases over `self`
count in the same way.

**Why.** A class that cannot pop from its own list cannot look after
its own parts, and the only way round was a free function that took
the whole object, which is not what a class is for. The danger (a loop
is looking at the item that goes) is at the call, so that is where the
check belongs.

**Why it is not written down.** A word on the method (`function next(take self)`
or `changes`) would be a second thing to keep in step with the body,
and a method that said it and did not do it would be a new kind of
mistake. The checker already works out a return type that is not
written in the same way. The message at the call names the method and
says what it does: "call `next`, which takes out or replaces something
the object owns".

**Turned down.** Letting any parameter that looks do this. `self` is
special because the call names the object (`queue.next()`), so the
caller's own checks can be run on it. For a plain parameter the word
`take` already says "this function is the owner now".

*(There is no 5.19j: the letter is left out so it is not read as an i.)*

### 5.19l `var f = double` gets the type of the function

`let f = double` holds the one function `double`, as before. `var f = double`
with no type written now has the type `function(int) -> int`, so
`f = triple` is allowed when `triple` fits.

**Why.** A `var` is something that will be given another value. A type
that only one value fits made the `var` useless and the message absurd:
"the new value should be a function, but this is a function". For a
`let` the narrow type is the useful one, since a call through it is a
direct call.

**Turned down.** Asking for the type to be written
(`var f: function(int) -> int = double`). It still can be, but
`var n = 1` does not need `: int`, and this is the same case.

### 5.19m In a firmware build, `raises` of a fault that nothing raises is refused

A firmware build stops at a division by zero, an index out of range or
a number that does not fit (5.5l). A `try` around such a line was
already refused there. A function could still say
`raises ZeroDivisionError`, divide, and be called inside a `try`:
`zeno run` caught the error and the firmware program stopped. Now a
function that declares `ZeroDivisionError`, `IndexError` or
`OverflowError`, with no `raise` of that kind in its body and no call
that raises it as a value, is refused when the build is firmware (and
so with no collector), with what to do: test before it happens and
take the `raises` off, or build without `--firmware`.

**Why.** The two ways of running a program may differ only where the
compiler has said so (question 4 of the audit). This was a place where
they differed in silence.

**Why not make the fault travel as a value.** Then every division and
every index in a firmware program would carry a status and a branch,
and a function that divides could fail in a way its type does not say.
5.5l chose a stop for exactly that reason, and MISRA wants the test
written before the division anyway.

**What still works.** `raise ZeroDivisionError("...")` written by
hand travels as a value in every build, so a function that tests
`b == 0` and raises may say `raises ZeroDivisionError`.

### 5.19n Messages count in words

"takes 2 argument(s), but 1 were given" is now "takes 2 arguments, but
1 was given", and a compiled program's memory report says "1 block"
and "2 blocks". Two small functions in `error.rs` (`counted`, `were`)
do it for the interpreter, the VM and the checker, so the three cannot
drift apart again.

**Why.** A message is part of the language. One that is not a sentence
makes the reader stop at the grammar and not at the mistake.

### 5.19o Looked at in the audit and left as it is

Each was held against the five questions of 5.19 and kept.

- **`list[int or none]` with no collector** stays refused (5.16h).
  Every number would need a block of its own. The message names what
  to write, so it is a limit and not a surprise.
- **A function that is kept cannot take over a list or an object from
  outside it** (5.17c). It may look for one line, or look at a `let`
  at the top of the file (5.19i). The kept function has no place to
  say `take`, and a parameter does.
- **A value made inside `unsafe { }` and used after it needs a `var`
  before the block** (5.18a). Every pair of braces opens a scope. A
  block that did not would be the one exception.
- **The order C works out the arguments of a call** (5.9q). What is
  printed never depends on it, and calls that change something are
  already put in order. Only the highest mark of the text room in a
  `--memory` report can move by a few bytes, when one statement shows
  a list and builds other text. Fixing it means a temporary for every
  piece of text, which makes all the C harder to read. The page on the
  audit says to leave the room some headroom.
- **A function or a method sees only what is declared above it.** One
  rule for every name, and the message now names the line (5.19i).

**What is left for later**, none of it part of the audit: the limits
in 5.19f, and the lists in 5.16i and 5.17g.

---

## 6. The standard library

### 6.1 What goes into the library, and in what order it is built

The standard library is everything a program can use without writing
it: the functions that have no module (`print`, `len`), the methods of
text and lists, and the modules (`math`, `time`, `board`). It is built
in this order:

1. Text and lists (this stage).
2. Reading and writing: a line from the keyboard, files, the arguments
   a program was started with.
3. More of `math`.
4. A dictionary, which is a new type of the language and a stage of
   its own.

**Why text and lists first.** Everything after them is written with
them: a file is read as text and cut into lines, and a line is found,
sliced and joined.

**What gets in.** A method is added only if (a) most programs in the
big languages use it, and (b) it cannot be said in one short expression
with what Zeno already has. That keeps out more than it lets in:

| Left out | Because Zeno already says it |
|---|---|
| `text.contains(part)`, `xs.contains(x)` | `part in text`, `x in xs` |
| `text.is_digit()` | `"0" <= ch and ch <= "9"` |
| `sorted(xs)`, `reversed(xs)` | `xs.slice(0)`, then `sort()` or `reverse()` |
| `xs.remove(x)` | `xs.find(x)`, then `pop(at)` (6.5) |
| `xs.extend(ys)` | `xs += ys` |

Python's `str` has 47 methods and its `list` 11. Zeno's text now has
10 and its list 12. Every one of them was held against the five
questions of the audit (5.19): one way to write it, one way to read
it, nothing surprising, the same in every build, and a message that
says what to write.

### 6.2 A method is asked of a value, and a function makes a value of another kind

`text.find(" ")`, `xs.sort()`, `words.join(", ")` are methods.
`int("12")`, `str(3)`, `float("1.5")`, `u8(n)` and now `char(65)` are
functions, each named for what it gives.

**Why.** A method reads in the order things happen (3.6b), and the
value in front says what is being worked on. A conversion has no
value of the new kind to be asked of yet, so it is a function, and its
name is the kind it makes.

**So** the number of a character is `"A".code()` (asked of the
character), and the character of a number is `char(65)` (made from a
number, as `str(65)` makes `"65"`).

**Turned down.** `ord` and `chr`, Python's names: abbreviations that
say nothing to someone who has not met them (principle 2). A function
`code("A")`: it would take a name, `code`, that half of all programs
want for a variable.

**Kept as they were.** `len`, `min`, `max` and `sum` are functions and
stay functions. They were decided with the first interpreter, every
example uses them, and making them methods would add a second way to
write each for nothing.

### 6.3 `find` answers a position or `none`, never -1

```zeno
let space = line.find(" ")
if space is none { return line }
return line.slice(0, space)
```

`text.find(part)` and `xs.find(item)` give an `int or none`.

**Why.** -1 is a number. In Python `text[text.find("x")]` quietly
reads the last character when there is no `x`. In Zeno a negative
position is an error (2.6), so the mistake would at least stop the
program, but far from where it was made. `none` cannot be used as a
position at all: the checker asks for the test first (3.12), and `if`
or `match` then gives the number (5.19h). The mistake is caught before
the program runs (principle 1).

**Turned down.** Raising an error when the thing is not there
(Python's `index`): "not there" is an ordinary answer, not a failure.
Having both, as Python does: two ways to ask one question.

**With one owner.** `tasks.find(Task("d", 4))` only looks at what it
is given. An item made in the brackets belongs to the statement, which
frees it when it is over.

### 6.4 A part is taken with `slice(from, to)`, and `xs[1:3]` is not Zeno

`text.slice(1, 3)` and `xs.slice(1, 3)` take positions 1 and 2. `to`
is not taken, as in `range`. `slice(from)` goes on to the end.

**Why a method.** `[1:3]` would be the one place where `:` does not
mean "a type follows". Python's slice has three optional parts and
negative positions, and each needs a rule that has to be remembered.
A method needs a name and nothing else.

**Outside the text or the list is an `IndexError`.** Python cuts the
slice to fit without a word, so `"abc"[2:9]` is `"c"`. A position that
is out of range is nearly always a sum that went wrong, and Zeno says
so, as it does for `xs[9]`.

**Writing `xs[1:3]` gets told what to write**: "a part of a list or a
str is taken with `.slice(from, to)`, as in `xs.slice(1, 3)`".

**Positions in text count characters**, as `len` and `text[i]` do, and
not bytes. In UTF-8 a character is one to four bytes, so finding
position 7 means walking past seven characters. That is the price of
`"wörld".slice(1, 2)` being `"ö"` and never half of it.

**With one owner.** A slice of numbers or of text is a new list with
copies. A slice of objects is refused, as `filter` is: the items would
be in two lists, and each has one owner.

### 6.5 `pop(at)`, `insert(at, item)`, `clear()` and `reverse()`

`xs.pop()` takes the last item out, as before, and `xs.pop(2)` takes
out the one at position 2. `xs.insert(0, x)` puts `x` in front; the
position may be the length of the list, which is what `append` does.
`clear()` empties the list. `reverse()` turns it round where it is.

**Why `pop(at)` and no `remove`.** `remove` means "by value" in Python
and "by position" in Rust, and in Java `list.remove(1)` on a list of
numbers is a well-known trap, because it could be either. `pop(at)`
can only be a position. Taking out by value is `find` and then `pop`,
two lines that have to say what happens when the value is not there.

**Why `clear()`.** Most lists are held by a `let`, which cannot be
given a new list.

**Why `reverse()` and `sort()` change the list and give nothing back.**
A new list is a new block of memory, which a program on a small board
may not want to spend. And a method that gives the list back invites
`let ys = xs.sort()`, where it is not clear whether `xs` changed. A
copy is asked for by name: `xs.slice(0)`.

**With one owner** `pop(at)` and `clear()` take items out, so they are
held to the rules of `pop()` (5.15e): only the owner, not in a loop
over the list, and on a line of its own. `insert` keeps what it is
given, as `append` does. A method of a class may do all of them to a
list its object holds (5.19k).

### 6.6 `sort()` is for numbers and text, smallest first, and is heapsort in every engine

Only a list of ints, floats, sized numbers or text can be sorted.
Anything else is refused: "`sort` puts numbers or text in order,
smallest first, and this is list[bool]. There is no one order for
those items, so write the loop that puts them in yours". (Since chapter 21 it
says to give `sort` a function, which is that chapter.)

**Why only those.** They are what `<` compares, and each has one order
that nobody has to be told. Tasks could be ordered by name or by
priority, and a `sort()` that picked one would be a guess. Sorting by
a function that the program gives is left for when a real program
needs it *(was provisional: 11.3)*.

**Why heapsort.** It sorts inside the list, so it asks for no memory.
It is a loop and not a function that calls itself, so it needs no
deep stack, and MISRA has nothing to say against it. It takes about
n log n steps whatever the order was to begin with, so there is no bad
case to find on the bench.

**Why the interpreter does not use Rust's `sort`.** Decision 5.5d says
a compiled program prints what `zeno run` prints. Heapsort may swap two
items that compare equal, and that can be seen: `0.0` and `-0.0` are
equal and print differently. So `zeno run` sorts with the same steps
as the compiled program, written once in Rust (`builtins.rs`) and once
in C (`runtime.c`), and a test sorts such a list in every build.

**Turned down.** C's `qsort`: it is missing from some firmware C
libraries, and the order it leaves equal items in is not defined.

**Text is ordered by the code of each character**, as `<` orders it,
so `"B"` comes before `"a"`.

### 6.7 `join` is a method of the list

`words.join(", ")` reads in order: the words, joined with a comma.
Python's `", ".join(words)` starts with the least important part.
JavaScript and Rust write it Zeno's way.

**Only a list of text.** A list of numbers is refused, with what to do:
"Make each item text first, with `str( )`". Nothing in Zeno turns into
text by itself (2.3).

### 6.8 `code()` needs exactly one character, and `char` refuses 0

`"A".code()` is 65 and `char(65)` is `"A"`. The numbers are Unicode's,
so `"€".code()` is 8364.

`code()` on text that is empty, or longer than one character, is a
`ValueError`. Taking the first character without a word would hide a
mistake.

`char(n)` is a `ValueError` for a number that is no character: below
1, above 1114111, or in the range Unicode keeps free (55296 to 57343).

**Why not `char(0)`** *(was provisional: 11.3)*. In a compiled program text ends
at the first zero byte, as in C, so text with a zero in it would be
cut short there and not in `zeno run`. Bytes that may be zero belong
in a `list[u8]`.

### 6.9 In a firmware build, a bad position or a bad code stops the program

A slice out of range, `insert` or `pop` at a position that is not
there, `code()` of two characters and `char(0)` are faults, as a bad
index always was (5.5l): `zeno run` and an ordinary build raise an
error that can be caught, and a firmware build stops.

The rule of 5.19m now covers `ValueError` as well: a function that
says `raises ValueError` with nothing in it that raises one as a value
is refused in a firmware build. `int(text)` does raise one as a value
there, so `raises ValueError` beside `int(text)` is still right.

### 6.10 These methods are written in Rust and C, not in Zeno *(was provisional: 11.3)*

Each method is written once for `zeno run` and the VM (`builtins.rs`),
described to the checker (`checker.rs`), and compiled by both backends
(`emit_c.rs`, `emit_llvm.rs`) into a call of one C function in
`runtime.c`.

**Why not in Zeno itself.** Two things are missing. A library written
in Zeno has to be brought into the program that uses it, which is the
next stage (modules across files). And `pop(at)`, `insert` and `clear`
take items out of a list or put them in, which a function that was
only lent the list may not do when each item has one owner (5.15e).
They have to be built in.

Once modules exist, what can be written in Zeno will be.

### 6.11 The world outside is files, the keyboard and the command line

Part 2 of the library lets a program reach what is outside it. There
are three such things on a computer, and each has one way in:

| Outside | In Zeno |
|---|---|
| The screen | `print(...)`, as before |
| The keyboard | `input()` |
| Files | `import files`, then `files.read(path)` and four more |
| The command line | `import program`, then `program.arguments()` |

**Why `print` and `input` are plain words and the rest are modules.**
Nearly every first program prints and asks. Files and arguments are
for some programs, and on a board there may be neither, so they are
asked for by name, as `board` and `time` are (3.15).

**Unambiguous and intuitive.** Each function does one thing and its
name is the plain word for it. There is no second way to do any of
them.

### 6.12 A file is read whole and written whole, with no handle

```zeno
let text = files.read("notes.txt")
files.write("notes.txt", "a new start\n")
files.append("notes.txt", "one more line\n")
```

There is no `open`, and so no `close`.

**Why.** A file that is opened is a thing that has to be closed, on
every way out of the block, by somebody. That is the question of who
owns a list (5.8) asked again about something else, and the small
files that Zeno programs read so far do not need it asked. A whole
file as one `str` is also the form the text methods of part 1 already
work on.

**Turned down.** C's and Python's `open(path, "r")`: the second
argument is text that picks what the function does, so a wrong letter
is found when the program runs and not when it is checked. Python's
`with open(...) as f`: a new kind of block for one use.

*(was provisional: 11.3)* A file too large to hold at once needs reading in
pieces. That will be added, with an owner for the open file, when a
real program needs it.

### 6.13 `write` replaces, `append` adds, and neither adds a line end

`write` makes the file, or empties one that is there. `append` makes
the file, or adds to the end of one that is there. What is written is
the text and nothing more: a program that wants lines ends them with
`"\n"` itself.

**Why two functions and not a flag.** `files.write(path, text, true)`
does not say what `true` means. Two names do.

**Why no line end is added.** `print` adds one because it writes lines
to a screen. A file is whatever the program says it is, and
`files.write(path, files.read(other))` must make an exact copy.

### 6.14 A file that cannot be used is a `FileError`, and it is a value in every build

Reading a file that is not there, writing into a folder that is not
there, removing a file twice: each raises `FileError`, with a message
that names the file (``there is no file `notes.txt` ``, ``cannot read
`notes.txt` ``, ``cannot write `notes.txt` ``, ``cannot remove
`notes.txt` ``).

**It can be caught in a firmware build too**, unlike a bad index
(5.5l, 6.9). **Why.** A bad index is a mistake in the program: the
program could have looked at the length first. A missing file is not:
no test beforehand rules it out, because the file can go between the
test and the read. It is an error in the sense of 4.1, something that
goes wrong in a correct program, like `int(text)` on text from
outside. So a function that reads a file says `raises FileError` or
catches it, and the checker holds it to that in every build.

**Turned down.** `files.read` answering `str or none`: `none` cannot
say whether the file was missing or was not text. A number that says
how it went, as C's `errno` does: nothing makes the caller look at it.

**One kind of error, not several.** `FileNotFoundError`,
`PermissionError` and the rest of Python's family are told apart by
programs that go on to do something different for each. The message
tells a person which it was. *(was provisional: 11.3)*

### 6.15 `exists` asks whether there is a file that can be read

`files.exists(path)` is `true` for a file that `files.read` could
open. It is `false` for a folder, and for a path with nothing at it.
It cannot fail, so it does not raise.

**Why a folder is not there.** The module is about files. A program
asks `exists` to decide whether to read, and a `true` that is followed
by "cannot read" would be a wrong answer to that question.

### 6.16 A file holds text, and what is not text is a `FileError`

`files.read` gives a `str`, and a `str` is characters (6.4). So the
bytes of the file must be UTF-8, and none of them may be zero, because
text in a compiled program ends at a zero byte (6.8). A file that is
something else, such as a picture, raises ``FileError: `logo.png` does
not hold text``.

**Why not read it anyway.** `zeno run` and a built program would
disagree about its length, and the checks that make `slice` and `code`
count characters would be working on bytes that are no characters.

**Nothing is changed on the way in or out.** Files are opened as
bytes, so `"\r\n"` in a file from Windows arrives as `"\r\n"`, and
`write` puts down exactly what it was given. `lines()` is where line
ends are dealt with (6.17).

*(was provisional: 11.3)* Bytes that are not text belong in a `list[u8]`, and a
`files.read_bytes` that gives one is for the stage that needs it.

### 6.17 `lines()` is a method of text, and knows both line ends

`text.lines()` gives a `list[str]`, one item to a line, without the
line ends.

- A line ends at `"\n"`. A `"\r"` just before that `"\n"` goes with it.
- The last line need not end: `"a\nb"` and `"a\nb\n"` are both two lines.
- Empty text has no lines. `"\n"` is one empty line.

**Why not `split("\n")`.** It gives `"a\nb\n"` a third, empty, piece,
and leaves a `"\r"` on the end of every line of a file from Windows.
Both are the kind of surprise that is found late.

**Why on text and not `files.lines(path)`.** Lines also come from the
keyboard, from a serial port, from a string in a test. One method on
`str` serves all of them, and `files.read(path).lines()` reads as what
it does.

### 6.18 `input()` gives a line, or `none` when there are no more

```zeno
while true {
    let line = input()
    if line is none {
        break
    }
    print(line.upper())
}
```

`input()` gives the next line without its line end. `input("Name: ")`
prints the question first and stays on that line. At the end of what
there is to read (Ctrl-D at a keyboard, the end of a file piped in) it
gives `none`.

**Why `str or none`.** The end comes to every program that reads until
there is no more, and that is a correct program, so it is not an error
to catch. And the type makes the checker ask for the test: `input()`
used as text is refused with "Check it with `is not none`".

**Turned down.** Python's `EOFError`: the ordinary end of the loop
written as `try`/`except`. An empty `str` at the end, as in Lua and
shell: an empty line in the middle looks the same.

**Why the name.** `print` puts a line out and `input` takes one in. It
is the word Python programmers already know. `read_line` was the other
candidate, and `files.read` would then have had a namesake that reads
something else.

**It gives text and nothing else.** A number is `int(line)`, which can
fail, and says so (2.3).

### 6.19 `program.arguments()` gives the words after the program's name

```
zeno run copy.zeno a.txt b.txt        ["a.txt", "b.txt"]
./copy a.txt b.txt                    ["a.txt", "b.txt"]
```

**The program's own name is not in the list.** In C `argv[0]` and in
Python `sys.argv[0]` it is, and every program that uses them starts by
stepping over it. `words[0]` in Zeno is the first thing the user
wrote.

**A function, giving a new list each time.** `board.led` is a value
because there is one LED. A list is something a program appends to and
gives away, so each call makes one that is the caller's own, which is
also what one owner needs (5.15).

**With `zeno run`, the words after the file are the program's**, even
ones that look like Zeno's options, and Zeno's own options go before
the file. `zeno run copy.zeno --strict` gives the program `--strict`.
**Why.** Otherwise a program could never be given a word Zeno also
knows, and which words those are would change with each version.

**`main` takes the command line only in a program that asks for it**,
so the C and the IR of every other program are what they were.

**Left out** *(was provisional: 11.3)*: `program.exit(code)`. A program ends by
reaching its end, or with an error nobody caught, which ends it with
status 1. Environment variables: no program has needed one yet.

### 6.20 With no collector and on a board

**With no collector** the text of a file, and a line from `input()`,
are built text like any other (5.9c): they are in the text room until
the statement ends, and kept text is copied to the pool. So a file
larger than `ZENO_TEXT_ROOM` (1024 bytes unless given) stops the
program with the message that names the macro, and so does a typed
line that is longer than the room. A line is read straight into the
room, a byte at a time, so it takes its own length and no more.
`--memory` prints how much of the room was used.

*(was provisional: 11.3)* Reading a large file a line at a time, straight into
the pool, belongs with the open file of 6.12.

**On a board** these functions are the C library's: `fopen` for files,
standard input for `input()`, which firmware C libraries usually tie
to a serial line. A board with no file system gets a `FileError` from
every call, which a program can catch. They are not behind
`ZENO_BOARD` (5.12): a board that has a C library has them already.

### 6.21 What `math` holds

Part 3 fills in the `math` module. It had `pi`, `sqrt`, `floor`, `ceil`,
`sin` and `cos`.

| Added | What it is |
|---|---|
| `math.e` | 2.718281828459045 |
| `math.tan(angle)` | beside `sin` and `cos` |
| `math.asin(x)`, `math.acos(x)`, `math.atan(x)`, `math.atan2(y, x)` | an angle from a ratio |
| `math.exp(x)` | e to the power x |
| `math.log(x)`, `math.log2(x)`, `math.log10(x)` | logarithms |
| `math.round(x)` | the nearest whole number |

**How it was chosen.** Each is something firmware arithmetic uses and
that cannot be written in a line with what was there: a heading from
two field readings, a temperature from a thermistor, decibels, the
charge of a capacitor, a sine table (`examples/instruments.zeno`).

**Left out, because Zeno has one way already.**

| In other languages | In Zeno |
|---|---|
| `pow(x, y)` | `x ** y` |
| `trunc(x)` | `int(x)` |
| `fabs(x)` | `abs(x)` |
| `degrees(x)`, `radians(x)` | `angle.deg`, `angle.rad` (6.22) |
| `round(x, 2)` | `f"{x:.2f}"`: digits after the point are a matter of showing |
| `log(x, base)` | `log2`, `log10`, or `log(x) / log(base)` |

**Left out until something needs it** *(was provisional: 11.3)*: `hypot`, the
hyperbolic functions, `gcd`, `factorial`, `clamp`, `inf` and `nan` as
names, and random numbers, which are not arithmetic and want a module
of their own.

### 6.22 `.rad` and `.deg` change an angle; degrees and radians still never meet by themselves

`(180 deg).rad` is `3.141592653589793 rad`, and
`math.atan2(1, 1).deg` is `45 deg`.

This changes 1.15, which kept the two apart altogether because the
factor between them, pi / 180, is not an exact number.

**Why change it.** The functions of 6.24 give radians and people read
degrees, so every program with an angle in it needs the change, and
without it each would write `* 180 / math.pi` by hand. `.ms` is
already how a quantity is changed (1.15), so this is that way and not
a new one.

**Why they still do not meet in `+` or `<`.** When two units meet, the
answer is in the finer one, because that loses nothing. Between
degrees and radians neither is finer and either way loses something,
so `90 deg + 1 rad` has no answer that is obviously right. The message
says what to write:

```
`+` does not work between a quantity in rad and a quantity in deg. Change one of them first, with `.rad` or `.deg`
```

**The factor is one multiplication**, by `pi / 180` or by `180 / pi`,
which is what Python's `radians` and `degrees` do, so `(180 deg).rad`
is exactly `math.pi` and `(math.pi * 1 rad).deg` is exactly `180 deg`.

### 6.23 `sin`, `cos` and `tan` take an angle: a plain number is radians

```zeno
math.sin(30 deg)       # 0.49999999999999994
math.sin(0.5 rad)      # 0.479425538604203
math.sin(0.5)          # the same
```

**Why a unit is accepted.** Degrees handed to a function that counts
in radians is the oldest mistake in this corner, and it gives a wrong
number, not an error. With the unit on the value the function cannot
be handed the wrong thing.

**Why a plain number is accepted too, when `sleep(500)` is refused.**
`sleep(500)` is refused because 500 of what is anyone's guess. The
sine of a plain number is a thing mathematics defines, and it means
radians in every language and every textbook. Signal code is full of
it: in `sin(2 * math.pi * f * t)` the argument was worked out and
never was an angle somebody measured.

**Turned down.** Refusing plain numbers: every formula would need
`* 1 rad` on the end. A second family, `sind` and `cosd`, as MATLAB
has: two names for one thing.

A quantity that is not an angle is refused by the checker:

```
`sin` needs a number or an angle, such as `30 deg`, but this is a quantity in s
```

### 6.24 `asin`, `acos`, `atan` and `atan2` give an angle in radians *(was provisional: 11.3)*

Their answer is a quantity in `rad`, not a float.

**Why.** A float that is secretly an angle is what units are there to
end (1.15). `math.atan2(y, x) > 45` would compare radians with a
number the writer meant as degrees, and run. As a quantity it is
refused, and `math.atan2(y, x).deg > 45 deg` says what is meant.

**What it costs.** Arithmetic with a plain number needs the unit put
on or taken off, as with any quantity: `angle + 2 * math.pi * 1 rad`,
`angle / 1 rad`. Passing it on to `sin` needs nothing.

**It was provisional (11.3)** because it was the first time a library function gives
a quantity back that is not a time. If it gets in the way of real
signal code, a plain float is the other choice.

**`atan2(y, x)` takes y first**, as in C, Python, JavaScript and Rust.
It is the angle of the point (x, y), and the order is the one in
`atan(y / x)`. It takes plain numbers only *(was provisional: 11.3)*: two
lengths would be reasonable, and can come later.

### 6.25 `round` sends a half away from zero, and gives an int

`math.round(2.5)` is 3 and `math.round(-2.5)` is -3.

**Why.** It is the rule taught at school, and the one C's `round` and
Rust's `round` follow. **Turned down.** Python's rule, a half to the
nearest even number, so that `round(2.5)` is 2 and `round(3.5)` is 4:
better for adding up many rounded numbers, and a surprise to nearly
everyone who meets it.

**Why an int.** `floor` and `ceil` give one (their answer is a whole
number, and an index or a register value is wanted next). A float too
large for an int gives the largest int, as they do.

### 6.26 `log` is the natural logarithm, with `log2` and `log10` beside it

**Why `log` and not `ln`.** It is `log` in C, Python, JavaScript and
Java, which is where the people Zeno is for come from. On a calculator
`log` is base ten, so the two others have their base in their name and
the walkthrough says which is which.

**Why three functions and not `log(x, base)`.** `math.log(125) /
math.log(5)` is `3.0000000000000004`. `log2(8)` and `log10(1000)` are
exactly 3, because the C library works them out directly. Bases other
than e, 2 and 10 are rare.

### 6.27 A number that has no answer is a `ValueError`

`math.log(0)`, `math.log(-1)`, `math.asin(1.5)` and `math.acos(-2)`
raise `ValueError`, with the number in the message, as `math.sqrt(-1)`
always did.

**Why not `nan`.** C gives a `nan` here, which then spreads through
every sum it touches and is printed twenty lines later. An error at
the line that asked is easier to find.

**In a firmware build they are faults** and stop the program, as a bad
index does (5.5l, 6.9): the program can test the number first.

**An answer too big for a float is `inf`**, as it is for `*` and `**`:
`math.exp(710)` is `inf`.

### 6.28 The answers are the C library's, in every engine

A compiled program calls `sin`, `log` and the rest in the C library.
`zeno run` and the VM reach the same functions through Rust. So every
engine and build gives the same digits on one machine, which the tests
hold them to.

Two machines with different C libraries can differ in the last digit
of a result that is not exact. **Why accept that.** The other choice
is Zeno's own `sin`, a few hundred lines that would be slower and no
more right than the one already on the machine.

### 6.29 A dict is written `["Ada": 36]`, and one with nothing in it is `[:]`

```zeno
let ages = ["Ada": 36, "Alan": 41]
let seen: dict[str, int] = [:]
```

A dict keeps one value under each key. It is written like a list, in
square brackets, with `key: value` where a list has an item. Partha
chose this form on 2026-11-15, from the forms that were put to him.

**Why square brackets.** In Zeno a pair of braces is always a block of
code, and a pair of square brackets always holds values. Somebody
reading a line never has to work out which of the two a bracket is.

**Turned down.** `{"Ada": 36}`, as in Python and JavaScript: braces
would then mean a block in one place and a value in another, and `{}`
after `if ready` could be read as either. JavaScript lives with exactly
that puzzle. A word in front, as in `dict("Ada": 36)`: longer, and it
looks like a call, which it is not.

**Why `[:]` for the empty one.** `[]` is the empty list. The colon is
what makes a dict a dict, so the empty dict keeps it. The parser knows
which it is reading as soon as it has seen the first thing after `[`.

**Unambiguous and intuitive.** After the first value comes `,` or `]`
(a list) or `:` (a dict), and every entry after that must be the same
kind. Mixing them is refused with a message that says how each is
written. A dict prints exactly as it is written.

### 6.30 Its type is `dict[str, int]`: the keys, then the values

The same shape as `list[int]`. The name is Python's, which more people
read than any other (the principles, last paragraph).

**Turned down.** `map[str, int]`, as in Go: `map` is already the list
method that changes every item. `[str: int]`, as in Swift: a type that
looks like a value, where every other type in Zeno is a word.

### 6.31 A key is a `str` or an `int`

Names and numbers: `ages["Ada"]`, `meanings[404]`. Both compare
exactly, which a key has to.

**Why not a float.** `0.1 + 0.2` is not `0.3`, so a value stored under
one could not be found under the other. That is a mistake the language
can simply not offer.

**Why not a bool, a list or an object.** Two keys are an `if`. A list
or an object can change while it is a key, and then it is filed in the
wrong place. An enum case would be a good key and is the likely next
one.

**All the keys of one dict are the same type**, and the checker refuses
`["a": 1, 2: 3]`. That is what lets it say what a loop variable is.

### 6.32 A value is anything a list can hold, and never `none`

This section used to say "an `int`, a `float`, a `bool` or a `str`",
and was marked provisional until a real program asked for more. One
did, and 10.11 has the rule as it is now and the reasons. With no
collector the first rule still holds (10.14).

**All the values of one dict are the same type**, like the items of a
list that says what it holds.

### 6.33 A dict remembers the order its keys were put in

A loop over a dict, `keys()`, `values()` and `print` all follow the
order in which the keys were first stored. Storing under a key that is
there does not move it.

**Why.** It is the one order a reader can predict from the program, and
it is the same on every run, in every engine and in every build. The
order a hash table happens to have differs between Rust and C, and the
tests compare the output of eight builds line for line.

**What it costs.** `pop` closes the gap, so it takes longer the more
keys come after the one taken out. Finding, storing and `in` do not
depend on the size.

**Turned down.** No order at all (cheaper `pop`, and a program whose
output changes when a key is added). Sorted by key (every store pays
for it, whether or not anything is ever printed).

### 6.34 `d[key]` of a key that is not there is a `KeyError`

There are three ways to ask, and each says what the programmer knows:

| Written | Means | A key that is not there |
|---|---|---|
| `d[key]` | it is there | `KeyError` |
| `d.get(key)` | it may not be | `none` |
| `key in d` | is it? | `false` |

**Why not `none` from `d[key]`.** Every read would then be an
`int or none` to test first, even where the key was stored on the line
above. **Why not 0 or `""`.** That invents a value nobody stored, and
a misspelt key becomes a wrong total (principle 1).

**In a firmware build it is a fault** and stops the program where it
happens, as a bad index does (6.9). `get` and `in` are how a program
that must not stop asks first.

### 6.35 `get(key)` gives the value or `none`, and `get(key, instead)` gives a value

```zeno
let fan = seen.get(40)              # int or none
seen[code] = seen.get(code, 0) + 1  # int
```

With one argument the answer may be `none`, so the checker asks for
the test before it is used, as it does for `find` (6.3). With two, the
second is what to give when the key is not there, and it must be of
the dict's value type, so the answer is a plain value.

**Why one name for both.** Both ask without failing. The second
argument does not change what is asked, only what is given instead.
It is Python's `get`.

**It never adds the key.** `get` only looks.

### 6.36 `d[key] = value` stores: a new key is added, one that is there gets the new value

**Why one form.** Counting, the commonest use, is
`seen[word] = seen.get(word, 0) + 1`, where the program does not care
which of the two happens. A program that does care asks with `in`.

**Turned down.** A method (`d.set(key, value)`) beside the brackets:
two ways to write one thing.

**The same key written twice in a dict that is written out is
refused**: `["a": 1, "a": 2]`. Stored one after the other the second
would replace the first, which is fine in a loop and a slip of the
hand in one pair of brackets.

### 6.37 `pop(key)` takes a key out and gives its value

The word a list already uses for "take it out and hand it to me"
(6.5). A key that is not there is a `KeyError`, as for `d[key]`.

**Turned down.** `remove`, which would give nothing back: then getting
the value and removing it are two lookups. A `del d[key]` statement: a
new kind of statement for one type. A second argument for a key that
is missing: `in` asks.

`clear()` empties the dict, as it empties a list.

### 6.38 A loop over a dict is given its keys; `keys()` and `values()` are new lists

```zeno
for code in seen {
    print(code, seen[code])
}
```

**Why the keys.** With the key the value is one lookup away, and with
only the value the key is lost. `in` asks about keys for the same
reason.

**Pairs.** `for code, count in seen` is given each key and what is
under it (10.12). It was added without changing what the loop with
one name means, as this section said it could be.

**The loop walks the keys as they were when it began**, like a loop
over a list. The body may add keys and take them out. `keys()` and
`values()` give lists that are the caller's own for the same reason:
changing the dict afterwards does not change them.

### 6.39 Two dicts are equal when the same keys have the same values

`["x": 1, "y": 2] == ["y": 2, "x": 1]` is `true`. The order says how a
dict was built, and `==` asks what it holds. `+` and `<` are not
defined for dicts: there is no one meaning for either.

### 6.40 A dict is a list or two and an index, and is owned like a list

**How it is kept.** In a compiled program: a list of keys, a list of
values beside it, and a table of positions that a key's hash leads
into (open addressing, at most half full, doubled when it fills; text
is hashed with FNV-1a). In `zeno run` and the VM: a `Vec` of entries
and Rust's `HashMap` of positions. Both give the order of 6.33.

**Why two lists.** `keys()`, `values()` and the loop are then the list
code that already exists, and so is freeing them.

**Who frees it.** In an ordinary build a dict is shared and the
collector frees it, like a list. `owned dict[str, int]` gives it one
owner by request. With no collector it always has one owner, and
everything written in docs/ownership.md about a list holds for a dict:
a second name takes it over, a parameter looks unless it says `take`,
and the end of the owner's block frees it. The dict has copies of its
own of any text in it (docs/text.md), and frees them too.

**`[:]` with no type beside it** cannot say what it will hold, and a
build asks for the type, as it does for `[]`.

**A program that uses no dict pays nothing for them** in a build for a
board: the C compiler drops the functions nobody calls.

---

## 7. Programs in several files

A program that has grown past a few hundred lines wants more than one
file. This section says how one file uses what another declares. The
walk through the code is in [`modules.md`](modules.md).

**Checked against "unambiguous and intuitive".** One name is one file
(7.2). What is written at the import is what is written at the use
(7.3). A file means the same in every program that imports it (7.7).
A name that could be read two ways is refused with a message that
says what to write (7.8). And there is one mechanism, not two: a file
of your own is imported with the words that import `math` (7.1).

### 7.1 A module is a file, and `import` is how it is brought in

```zeno
import shapes                     # then: shapes.area(s), shapes.Point
from shapes import area, Point    # then: area(s), Point
```

`import shapes` reads the file `shapes.zeno`. These are the two forms
that `import math` and `from math import sqrt` have had since the
first stage, and a file of your own is used exactly as a module that
comes with Zeno is.

**Why a file is the module.** There is nothing to declare: no
`module` line, no header, no list of exports to keep in step with the
code. The file's name is the module's name, so the reader of
`shapes.area` knows which file to open.

**Turned down.** Pasting the other file in, as C's `#include` does:
the names of two files meet, and what a line means depends on what was
pasted above it. A header beside each file (C, C++): two places that
describe one thing, and they drift apart. `mod` and `use` as two steps
(Rust): the tree of modules is written out in one file and the files
have to match it.

### 7.2 A module is looked for from the folder of the program's first file

`import shapes` is `shapes.zeno` in the folder of the file that was
given to `zeno run` or `zeno build`. A dot is a folder:
`import drivers.uart` is `drivers/uart.zeno`. That holds for every
file of the program, wherever the file is: a file inside `drivers/`
that wants its neighbour writes `import drivers.pins`.

**Why from the first file, and not from the file that imports.** One
name is then one file, everywhere in the program. If each file looked
beside itself, `import pins` would mean one file in `drivers/` and
another in `sensors/`, and one file could be reached under two names
and be read twice.

**Turned down.** A list of folders to search, set outside the program
(Python's path, C's `-I`): which file is read then depends on the
machine. Where other people's files are is said in the program's
`zeno.package`: see section 8.

**The five modules that come with Zeno keep their names**: `math`,
`time`, `board`, `files`, `program`. A file called `math.zeno` is
never looked for by `import math`. *(Was provisional (11.4): once library
modules are themselves Zeno files, they will be found by one rule.)*

### 7.3 `import a.b` is used as `a.b.name`

```zeno
import drivers.uart
drivers.uart.send("ready")
```

What is written at the import is what is written at the use. A reader
who meets `drivers.uart.send` halfway down the file can find its
import by searching for the same words.

**Turned down.** Only the last word, `uart.send` (Go, Rust): two
modules called `uart` in two folders then collide, and the fix is a
second name for one of them. A second name by request,
`import drivers.uart as uart`: two spellings for one module, and
`from drivers.uart import send` already gives the short form.
*(Was provisional (11.4): if long names turn out to hurt in real programs, `as`
is the way to add.)*

### 7.4 Everything declared at the top of a file can be imported, unless it says `private`

*(Was provisional. The word that hides a declaration is `private`,
and it arrived in 10.1. The reasoning below is as it was written, and
it held: no program had to change.)*

Functions, classes, enums and their cases, and `let`, `var` and
`const` variables. What the file itself imports is not passed on:
`shapes.math` is not a name, even though `shapes.zeno` imports `math`.

**Why everything.** The first program anyone splits in two then
works with no new word to learn. And it is the choice that can be
changed without breaking programs: a word that hides a declaration
(`private function helper()`) can be added later and every program
written before it still means the same. Starting hidden and opening
up later can be added too, but starting hidden means a mark on almost
every line of every module from the first day.

**Turned down.** A leading underscore that means "leave this alone"
(Python): a convention the compiler does not check (principle 1). A
capital first letter that means "visible" (Go): spelling that carries
meaning (principle 2). `pub` or `export` on each declaration: see
above.

**The cases of an enum are names of their own**, as they are in one
file. `from shapes import Shape` brings in the enum's name. Its cases
are `shapes.Circle`, or `Shape.Circle`, or imported by name:
`from shapes import Shape, Circle`.

### 7.5 An imported file runs once, before the file that imports it

A file can have lines at its top that do something: fill a table,
print a banner. In a program of several files each file runs once,
and a file's imports have all run before its own first line does. The
file given to `zeno run` therefore runs last.

**Why once.** There is then one copy of each of the module's
variables, whoever imports it.

**So an `import` of a file goes at the top of the file**, above
everything that is not an import, and never inside a function or a
block. The imported file runs first wherever the line is, and a line
further down would say something else about the order than what
happens. (`import math` may still be written anywhere. It has no
lines of its own to run.)

**Turned down.** Running the imported file when the import line is
reached (Python). The order in which two modules run then depends on
which file happened to import one of them first.

### 7.6 Files that import each other are refused

```
error: these files import each other: `a` imports `b`, and `b` imports `a`.
```

Each would have to run before the other. Python runs one of them
half-way and hands the other a module with some of its names missing.
C has the programmer declare things ahead. Zeno says which files form
the circle and what to do: move what they share into a file of its
own, and import that from both.

### 7.7 A file sees what it declares and what it imports, and nothing else

**An imported file cannot see the file that imports it.** If
`shapes.zeno` uses a name it does not declare, and the program's
first file happens to declare one, that is an error. Otherwise what
`shapes.zeno` means would depend on which program imports it.

**Only the file of a module gives its variables a new value.**

```zeno
import counter
print(counter.total)     # reading is fine
counter.total = 0        # refused: give `counter` a function that does it
```

Where a variable can change is then one file, which is the first
thing anyone debugging it wants to know. Python's
`from counter import total` followed by `total = 0` changes a copy
and leaves the module's own variable alone, silently. In Zeno there
is one variable, and the assignment is refused under either
spelling. A list or object that the variable holds can still be
changed through it, exactly as through a `let`.

### 7.8 A name that would have two readings is refused

| Written | Refused because |
|---|---|
| `from a import x` and `from b import x` | which `x`? |
| `from a import x` and `let x = 1` at the top of the same file | which `x`? |
| `import shapes` and `let shapes = 1` at the top of the same file | `shapes.x` is a name of the module, or a field of the variable |
| `import a` and `import a.b`, where the module `a` declares a `b` | `a.b.x` is `x` of `a.b`, or `b.x` of `a`. (With no `b` in `a` there is one reading, and both can be imported: see 8.11) |
| `let s = shapes` | a module is not a value |

A name declared inside a function or a block hides an imported one,
as it hides any name from the top of the file. That is the rule Zeno
already has, and a parameter called `area` should not stop working
because an import was added.

### 7.9 A value prints by the name its file gave it

`print(shapes.Point(1, 2))` prints `Point(x=1, y=2)`, and an enum
case prints as `Dot`. An error that nothing catches is reported as
`uncaught BadShape`.

**Why not `shapes.Point(...)`.** Then what `shapes.zeno` prints would
change when it is imported instead of run by itself.

**The compiler's own messages use the full name**, `shapes.Point`,
also for a mistake inside `shapes.zeno`. There the question is which
`Point`, and two files may each have one.

### 7.10 `strict` is asked of one file

A file that begins with the line `strict` asks for every type in that
file to be known before the program runs. Its imports are not made
strict by it, and it does not make the file that imports it strict.
`zeno check --strict` and `zeno build` ask it of every file.

**Why.** A typed library must be usable from a quick script, and a
script's looseness must not leak into the library.

### 7.11 The files are joined into one program before anything checks it

The stage that reads the files gives every name that an imported
file declares at its top the module's name in front, `shapes.area`,
in that file and in every file that uses it, and then puts all the
statements in one list, an imported file before its importer
([`src/modules.rs`](../src/modules.rs)). Nobody can write
`shapes.area` as one word in a source file, so the name cannot
collide with one that somebody did write.

**Why.** The type checker, the interpreter, the VM and both compilers
then take a program in ten files exactly as they take a program in
one. Types that are worked out from use are worked out across files
with no extra work. A C build is still one C file, and the C compiler
still drops what nothing calls.

**Turned down, for now.** Compiling each file by itself and linking
the results. That needs a written-down description of what every
module offers, and generics and worked-out types make that
description large. It is what makes a big program quick to rebuild,
and it will be wanted then. *(Was provisional: 11.4.)*

**In C a module's name is part of the C name**: `shapes.area` is
`z_shapes_area`. Two classes or enums that would get the same C name
(`shapes.Point` and a class called `shapes_Point`) are refused with a
message, as `Stack[int]` and `Stack_int` already were.

### 7.12 One number says the file and the line

Every part of a program remembers where it was written: a line and a
column. With several files that is not enough, and adding a third
number would have touched every place that passes a position along,
down to each call into the runtime of a compiled program.

Instead the lines of the whole program are numbered in one run. The
first file's lines are 1 to 999,999, as the editor shows them. The
second file's start at 1,000,001, the third's at 2,000,001. Only the
places that show a position to a person take the number apart again.
Clang and the Rust compiler keep positions the same way, as one
number into all the source text.

**The cost.** A file can have at most 999,999 lines and a program at
most 2,000 files, so that the number fits in a C `int`. Both are
checked.

## 8. Packages

A module is a file of the program itself. A **package** is a folder of
modules that lives somewhere else: beside the program, or in somebody
else's git repository. This section says how a program names the
packages it needs, how an import finds a module of one, and how
`zeno fetch` gets them. The walk through the code is in
[`using-packages.md`](using-packages.md). Finding a package by its
name alone came later, in chapter 27.

**Checked against "unambiguous and intuitive".** The project file is
read aloud as it is written (8.2). A version is one exact thing, never
a range that something has to solve (8.4). An import still names one
file, and a file still has one name (8.5, 8.8). A package means the
same in every program that needs it (8.6). There is one copy of each
package in a program (8.7). Running a program never reaches for the
network: one command does, and it is the one you typed (8.9). And
there is still one mechanism: a module of a package is imported with
the words that import a file of your own.

### 8.1 A package is a folder with a `zeno.package` in it

```
filters/
    zeno.package        package filters
    filters.zeno        the module `filters`
    lowpass.zeno        the module `filters.lowpass`
```

The file `zeno.package` makes the folder a package and gives it its
name. Every `.zeno` file in the folder is a module of it. Nothing is
listed: no table of modules, no exports. That is 7.1 again, one step
up: the folder is the package as the file is the module.

A program is a package too. It has a `zeno.package` from the day it
needs another package, and not before: a program in one folder that
needs nothing has nothing to write.

**Turned down.** A list of the package's modules in the project file
(a Cargo `[lib]` path, a `package.json` `files` list): a second place
that has to be kept in step with the folder.

### 8.2 `zeno.package` is written in plain words

```
# What this program is called, and the packages it needs.
package thermostat

needs filters in "../filters"
needs units from "https://github.com/someone/zeno-units" at "v1.2.0"
```

One line names the package. One line for each package it needs. A
comment starts with `#` and a string is written as in a program,
because the file is split into words by Zeno's own lexer.

**Why.** *Decided by Partha on 2026-11-15*, from a card with three
options and three more in conversation. The file has two kinds of
line, and both can be read aloud. It needs no second syntax to learn
and no parser to add: Zeno has no dependencies, and a TOML reader
would be the first. Minimal punctuation is the rule of the language
(section 1), and it would be odd for the first file a newcomer opens to
break it.

**Turned down.**
- TOML, `zeno.toml` (Cargo, Python): well known, but a second syntax,
  and a parser for all of it to read two kinds of line.
- A Zeno program, `package.zeno` (Swift, Zig): no new syntax, but then
  the file can compute, and finding out what a program needs means
  running code.
- JSON: no comments, and more punctuation than words.
- The address in the import line, `import units from "https://..."`
  (Go before modules, Deno): the version is repeated in every file
  that imports the package, and two files can disagree.
- `needs` lines at the top of `main.zeno`: a package that is only a
  library has no `main.zeno`.

A mistake in a `zeno.package` is shown as a mistake in any file is,
with the file, the line and what to write:

```
error: this does not say which version of `units` is needed. Add `at` and a tag,
a branch or a commit of that repository, as in `needs units from "..." at "v1.0.0"`
 --> zeno.package:4:1
```

### 8.3 A needed package is in a folder, or in a git repository

| Line | Means |
|---|---|
| `needs filters in "../filters"` | the package in that folder, used where it is. The folder is counted from the folder of this `zeno.package` |
| `needs units from "address" at "v1.2.0"` | a git repository, and a tag, a branch or a commit of it. `zeno fetch` gets it |

`in` and `from` are words Zeno already has, and they say which of the
two it is. A line with `from` and no `at` is refused: see 8.4.

**Why a folder at all.** A package is written beside the first
program that uses it, long before it has a repository of its own. And
firmware projects often keep everything in one tree. The folder form
makes a package something you can have in the first five minutes.

**Why git, and no registry.** A registry is a server, accounts,
uploads and a name-squatting problem, for a language whose goal is
learning by building. Git is on every machine that builds firmware,
it already knows how to reach a private repository with the keys that
are there, and a commit hash already says exactly what was fetched.
So `zeno fetch` runs `git`, and Zeno still links nothing.

**Turned down.** Downloading an archive over HTTPS: Zeno would need a
TLS library, its first dependency. A central registry with short
names (`needs units "1.2"`): see above. *(Was provisional (11.4): a registry
could be added later as a third kind of line. The two here would stay
as they are.)* *(Added in chapter 27: `needs units at "v1.2.0"`, and the
two here are as they were.)*

### 8.4 A version is one exact thing: what stands after `at`

`at "v1.2.0"` is a tag, a branch or a commit, looked for in that
order. There are no ranges (`^1.2`, `>=1.0`), and a `zeno.package`
has no `version` line of its own.

**Why.** A range needs a solver, and what the solver picks depends on
what was published this morning. Nobody can read the answer off the
file. With one exact name after `at`, the file says what is built.
And a `version` line in the package would be a second place for what
the tag already says, with nothing to check that the two agree.

**The cost.** Two packages that need the same package must name the
same version (8.7). When they do not, the message says which lines to
make agree, and a person decides. That is the work a solver would
hide.

### 8.5 An import whose first word is a needed package reads that package

```zeno
import filters              # filters.zeno in the package's folder
import filters.lowpass      # lowpass.zeno in the package's folder
import control              # control.zeno of the program itself
```

`zeno.package` says `needs filters ...`, so in this program an import
that begins with `filters` is about the package. The words after it
are a file inside the package's folder, with a dot for a folder as
ever. `import filters` on its own is the package's module of its own
name. Every other import is a file of the program, as before.

What is written at the import is still what is written at the use
(7.3): `filters.lowpass.start(...)`.

**Why the package's name is the first word.** The reader of
`filters.lowpass.start` sees which package it comes from, and
`zeno.package` says where that package is. No import has to be looked
up in a search path.

**Why `import filters` is `filters.zeno`.** A small package is one
file, and its users should write one word. Python's `__init__.py` and
Rust's `lib.rs` do this with a special file name. Here no name is
special: it is the file with the package's name.

**Refused, each with what to write.** `import filters.filters` (it is
`import filters`). A file `filters.zeno` of the program beside a
needed package `filters`: `import filters` would have two readings.

### 8.6 Inside a package, an import means the same in every program

The files of the package `filters` import each other as `import
lowpass` and `import filters`: files of their own folder. A package
reaches another package only if its own `zeno.package` needs it, and
it cannot reach a module of the program that uses it.

This widens 7.2 by one word. A module is looked for from the folder
of the **package** the importing file is in. For a file of the
program that is the folder of the program's first file, as it always
was.

**Why.** A package is written once and used by many programs. If an
import inside it were looked for from the program's folder, what the
package means would depend on who uses it. This is 7.7 for packages.
It also means a package can be run on its own: `examples/filters` has
a `main.zeno` that tries its modules, and they are found the same way.

**The program's folder is the folder of its first file.** Its
`zeno.package` is looked for there and nowhere else. *Turned down:*
walking up through the folders above until a project file is found
(Cargo, npm). What an import means would then depend on a file in a
folder you may not know is there.

### 8.7 A program has one package of each name

If the program needs `units` and so does `filters`, they get the same
`units`. Fetched packages all go into one folder, `packages/`, beside
the program's `zeno.package`, whichever package needs them. Two
`needs` lines for one name that point at different folders, different
repositories or different versions are refused, and the message names
the lines.

**Why.** One name is one file (7.2). With two copies of `units` in
one program there would be two classes called `units.Length`, and a
value of one would not be a value of the other, with both printing
the same. npm allows this and it is a well known source of puzzles.
In firmware it would also put the code in flash twice.

### 8.8 A module of a package has the package's name in front

In the joined program (7.11) the module `lowpass` of the package
`filters` is `filters.lowpass`, and its `start` is
`filters.lowpass.start`, whoever wrote the import. That is the name a
compiler message uses. The package's own files write `lowpass.start`.
The modules of the program itself keep their plain names.

Because one name must be one file and one file one name, three things
are refused when the loader meets them:

| Met | Refused because |
|---|---|
| two different files that would both be the module `units` | which one is `units.x`? |
| one file reached as two modules, `filters` and `lib.filters` | it would run twice, and its types would exist twice |
| an import of the file the program starts in | it would run twice |

### 8.9 Only `zeno fetch` uses the network *(widened in 27.7)*

```
zeno fetch              in the folder of the program
zeno fetch --update     look every version up again
```

`zeno run`, `zeno check` and `zeno build` read files and nothing
else. A needed package that is not in `packages/` is a mistake with a
message: "the package `units` has not been fetched ... Run `zeno
fetch` in the folder of the program".

**Why.** A build that may or may not download something is two
different commands under one name. On a build machine with no
network, or with firmware that must be the same next year, the one
that downloads has to be a step somebody chose.

**Turned down.** Fetching on the first `zeno run` (Cargo, Go).
`zeno add units ...` that edits the project file: the file is two
kinds of line, and writing one is the simplest way to say it.
`zeno new` has come since (10.22), and the file it writes shows both
kinds of `needs` line in a comment, so the line to write is in front
of whoever needs it.

### 8.10 `zeno.lock` pins what was fetched

`zeno fetch` writes, for each fetched package, the commit it got:

```
units from "https://github.com/someone/zeno-units" at "v1.2.0" commit "9f8e7d6..."
```

The next `zeno fetch`, on this machine or another, gets that commit
even if the tag or the branch has moved since. `--update` looks each
`at` up again and writes the file afresh. The lock is kept with the
program. A package in a folder (`needs ... in`) is not in it: it is
used where it is, and there is nothing to pin.

**Why.** `at "main"` is allowed (8.4), and a branch moves. Without
the lock two people would build two programs from one source. With
it, what `at` says is what you asked for and what the lock says is
what you have.

The lock of a needed package is not read: the program's own lock pins
everything in the program. One file answers "what exactly is built?".

### 8.11 A file can import a module and a module below it *(revises 7.8)*

`import filters` and `import filters.lowpass` in one file was refused
in 7.8, because `filters.lowpass.x` could be `x` of the module
`filters.lowpass` or the field `x` of a `lowpass` that the module
`filters` declares. With packages, that pair of imports is the most
natural thing to write.

It is now refused only when there really are two readings: when the
module `filters` declares a `lowpass` of its own. Otherwise
`filters.lowpass` can only be the module. The message, when it comes,
names the name that clashes.

### The rules, all of them

1. **A package is a folder with a `zeno.package`**, which names it
   (8.1, 8.2).
2. **A program says what it needs** in its own `zeno.package`: a
   folder, or a git repository and one exact version (8.3, 8.4).
3. **An import whose first word is a needed package** reads a file of
   that package. Any other import reads a file of the package the
   importing file is in (8.5, 8.6).
4. **One package of each name** in a program, and one name for each
   module (8.7, 8.8).
5. **`zeno fetch` fetches. Nothing else does** (8.9). `zeno.lock`
   remembers what it got (8.10).

## 9. Editor support

What an editor needs from a language: a formatter, so that every file
looks the same, and a language server, so that mistakes show as
you type. The walks through the code are in [`format.md`](format.md)
and [`lsp.md`](lsp.md).

**Checked against "unambiguous and intuitive".** There is one layout
and nothing to configure (9.1). Formatting cannot change what a
program means, and the formatter proves that to itself on every run
(9.2). Each rule of spacing is the one the examples and the documents
have used since the first stage, so `zeno format` surprises nobody
who has read Zeno (9.5). The language server shows what `zeno check`
shows, says about a name what the checker knows of it, and answers
nothing where it would have to guess (9.10, 9.11).

### 9.1 One layout, and no options

```
zeno format main.zeno        write the file again, laid out
zeno format examples         every .zeno file in the folder and below
zeno format --check .        change nothing; say which files are not laid out
```

There is no setting for the width of an indent, where a brace goes or
how many blank lines are allowed.

**Why.** "One obvious way to write each thing" is the first principle
of the language, and layout is where programmers disagree most and
gain least. With no options there is nothing to decide per project,
no file of settings to copy around, and a diff shows what changed and
never how somebody likes their spaces. Go showed that this works.

**Turned down.** A settings file (clang-format, rustfmt.toml): every
project then has its own Zeno.

### 9.2 The formatter changes white space, and nothing else

It works on the tokens of the file. Every token stays as it was
written, in the order it was written, on the line it was written on.
Numbers keep their spelling (`0x4000_4400`), texts keep their quotes.

**Why.** A formatter that works from the syntax tree has to print
every construct of the language, and the tree has already forgotten
the comments, the blank lines and how a number was spelled. Each new
piece of syntax would need a new piece of printer, and a slip in one
would quietly change a program. Working on tokens, the formatter
needs to know only which token it is looking at and which came
before. It cannot change a meaning, because it never adds, removes or
moves a token.

**Checked on every run.** After laying a file out, the formatter
reads its own output back and compares the tokens and the comments
with the ones it started from. If they differ it writes nothing and
says that the mistake is Zeno's.

**A file with a mistake is not formatted.** It has to lex and parse.
Indentation is counted from brackets that match, and the parser is
what knows that they do. The mistakes are shown as `zeno check` shows
them.

### 9.3 Line breaks are the author's *(was provisional: 11.4)*

The formatter never joins two lines and never splits one. There is no
line width.

**Why.** `case Off { return 0 }` on one line and a long block on
several are both right, and which is right depends on the code around
it. Choosing for the author needs a width limit and a search for the
best way to break each line, which is most of the code of prettier
and rustfmt, and still produces layouts people fight. A line break in
Zeno also ends a statement, so moving one is no longer only layout.

**The cost.** Two people can break one long call in two ways. If that
turns out to matter, a rule for where a too-long line breaks is the
thing to add.

### 9.4 Four spaces for each bracket open, counted in lines

```zeno
each(items, function(item) {
    print(item)
})
```

A line is indented once for each line above it that has left a `{`,
`(` or `[` open. The first line here opens two brackets and the body
is still indented once. A line that starts by closing brackets goes
back to where the line that opened them is. A statement that goes on
after an operator is indented once more:

```zeno
let total = price +
    tax
```

Tabs are replaced. Four spaces is what every example has used, and a
file that mixes tabs and spaces looks different in every editor.

### 9.5 Where a space goes

| Rule | Example |
|---|---|
| one space around an operator, `=` and `->` | `a + b`, `x = 1`, `-> int` |
| one space after `,` and `:`, none before | `f(a, b)`, `port: int`, `["a": 1]` |
| none inside `( )` and `[ ]` | `(a + b)`, `[1, 2]` |
| none before the `(` of a call or the `[` of an index or a type | `print(x)`, `items[0]`, `list[int]` |
| one space before any other `(` or `[` | `if (a or b) and c`, `return [1, 2]` |
| none around `.` | `shapes.area` |
| one inside `{ }` on one line, and before `{` | `if ready { go() }`, `case _ { }` |
| a `-` that is a sign holds on to its number | `-1`, `a - -1`, `f(-x)` |
| one space between two words | `500 ms`, `not ready` |

These are the spaces the examples and the documents already had. The
proof is a test: every file in `examples/` is taken apart into its
tokens, put together again with three spaces between each two, and
formatted. What comes out is the file as it is in the repository.

**How a `-` is told apart.** If the token ahead of it can end a value
(a name, a number, a text, `)` or `]`), it subtracts. Otherwise it is
a sign. The same question, "can a value end here?", tells a call's
`(` from a grouping one and an index's `[` from a list's.

### 9.6 The `=` that names an argument has no spaces

```zeno
let here = Point(x=3.0, y=4.0)
function greet(who: str, excited: bool = false, times=1) { }
```

Inside `( )`, an `=` straight after a name that begins an argument
names that argument, and is written tight. Every other `=` has a
space on each side, including a parameter's value after its type.

**Why.** A call with named arguments reads as a list of pairs, and
the tight `=` keeps each pair together and apart from the next:
`Point(x=3.0, y=4.0)` against `Point(x = 3.0, y = 4.0)`. It also
tells naming an argument from assigning to a variable at a glance.
Python settled on the same two forms for the same reason.

### 9.7 At most one blank line

Never two in a row. None at the top of the file or of a block, and
none at the bottom of either. A blank line after a comment stays.

**Why.** One blank line parts two things. A second one says nothing
more, and "one or two?" is a choice nobody needs to make.

### 9.8 Comments

A comment gets a space after its `#` when a word follows: `#note`
becomes `# note`. What stands after that is the author's, so a
commented-out example keeps its indentation, and `#!` and `#---` are
left alone. A comment on a line of its own is indented as code there
would be.

A comment after code keeps the distance the author gave it, with at
least two spaces. So comments that were lined up stay lined up:

```zeno
bytes: pointer[u8]    # `size` bytes, asked for with `allocate`
size: int             # how many fit
```

*(Was provisional: 11.4.)* The formatter does not line comments up by itself.
gofmt does, and needs rules for when a long line should break the
column. Keeping the author's distance is simple and never fights a
layout somebody made on purpose.

### 9.9 `zeno format` writes the file, and `--check` only looks

`zeno format main.zeno` changes the file in place and says
"Formatted main.zeno", or that it was laid out already. With
`--check` nothing is written, each file that would change is named,
and the command fails if there is one: that is the form for a script
that guards a repository.

A folder stands for every `.zeno` file in it and below it, except the
folder `packages`: what `zeno fetch` put there is somebody else's.

**Turned down.** Printing the result and leaving the file (gofmt's
default): the common wish is to fix the file, and the common wish
should be the short command.

### 9.10 A language server, and it is the `zeno` program itself

`zeno lsp` speaks the Language Server Protocol on its standard input
and output. An editor starts it and is then told the mistakes in each
open file as it is typed, and can ask for a file to be laid out.

**Why the protocol.** Writing a plugin for each editor is one piece
of work per editor, each in that editor's own language. The protocol
turns that into one server, which VS Code, Neovim, Helix, Zed, Emacs
and others can all use. What is left per editor is a few lines that
say "for `.zeno` files, start `zeno lsp`", and the colours.

**Why inside `zeno`.** The server shows exactly what `zeno check`
shows, because it runs the same loader and the same checker in the
same program. A separate server program would be a second thing to
build, install and keep at the same version. There is still no
library: the protocol is JSON with a length in front, and
`src/json.rs` reads and writes it in 250 lines.

**Turned down.** A server written in TypeScript on a ready-made
library: quick to start, but a second implementation of "what is a
mistake in Zeno", which is the one thing that must not exist twice.

### 9.11 The server answers from the checker, and nothing it has to guess

| The editor says | The server does |
|---|---|
| a file was opened, or its text changed | checks every open file, and sends the mistakes of each file whose mistakes changed |
| lay this file out | answers with the file as `zeno format` writes it |
| where is this name declared? | answers with the file and the line of its declaration |
| what is this name? | answers with the name and its type: `count: int` |
| the same two about a field, a method, or the name of a module | answers "nothing" |

A mistake is marked from where the checker says it is to the end of
the word there, with the message `zeno check` prints.

**Where the answers about a name come from.** To say rightly where a
name was declared, the server must know which declaration the name
means, with every rule of scope the checker applies: inside
`function twice(count: int)`, `count` is the parameter, and not the
`count` at the top of the file. Guessing from the spelling is right
most of the time, and an editor that is wrong some of the time
teaches you not to trust it. So the checker, which works this out
anyway, now writes it down: for every place a name is used, what it
means there, its type, and where it was declared (`checker::Use`).
The server looks the cursor's place up in that record and adds
nothing of its own.

For a function the type is shown the way its first line is written,
`twice: function(count: int) -> int`, with the names of the
parameters, because Zeno lets a call name them (`twice(count=3)`).

**Why a field gets no answer yet.** *(Was provisional: 11.4.)* The record holds
names: variables, parameters, functions, classes, enums, and the
names of a module (`shapes.area` is one name). A field or a method
(`p.x`, `queue.put`) is found from the type of what stands before the
dot, and the checker does not write that down yet. An empty answer
is honest, and an editor shows nothing for it. A guess would be
shown as if it were known.

**Turned down.**
- *Finding the declaration by searching the file for `let name` or
  `function name`.* Wrong as soon as two scopes use one name, which
  is ordinary code.
- *A second, small scope walker in the server.* Two sets of scope
  rules that have to agree for ever. The checker's are the rules.
- *Completion and rename now.* Completion has to work on a file that
  does not parse (the line being typed), and the checker never sees
  such a file. Rename changes many files at once, and needs the uses
  in files that are not open. Both wait until a real program misses
  them.

### 9.12 The editor's text is the program, and its folder is worked out

The server checks what is in the editor, not what is on disk: a file
that is open is read from the editor, also when another file imports
it, and only a file that is not open is read from disk. So a mistake
shows as it is typed, before anything is saved.

`zeno check` is given the file a program starts in, and that file's
folder is the program's (8.6). An editor opens any file, perhaps
`drivers/uart.zeno`. The server takes as the program's folder the
nearest folder, from the file's own upward, that has a `zeno.package`
or a `main.zeno`, and no folder above the one the editor has open.
With neither, it is the file's own folder, as for `zeno check`.

**Why this is not the walking up that 8.6 turned down.** 8.6 is about
what a program *means*, and that must not depend on a folder above.
This is about which program a file on the screen most likely belongs
to, so that its imports can be found at all. If the guess is wrong
the worst is a mistake shown that `zeno check` on the real first file
would not show, and a `zeno.package` in the right folder settles it.

**Every open file is checked at every change.** A change in one file
can make or mend a mistake in a file that imports it. Programs are
small and the checker is fast, so the simple thing is done:
everything open is checked again, and only what changed is sent.

### 9.13 Colours are per editor, and are not the server's work

The repository has a grammar for VS Code and a syntax file for Vim
and Neovim, in `editors/`. Both know Zeno's words from a list.

**Why not from the server.** The protocol can carry colours
("semantic tokens"), but an editor asks for them after the file is
checked, so the text would flash from plain to coloured. A grammar in
the editor colours at once and needs no server at all. *(Was provisional (11.4):
semantic tokens can be added on top for what a grammar cannot know,
such as whether a name is a variable or a function.)*

## 10. What real programs needed

The last stage before the language is called version 0.1. Each
section is something a program of more than a few files ran into, and
the README's roadmap lists them.

**Checked against "unambiguous and intuitive".** `private` is one
word with one meaning wherever it stands: only the file it is written
in can use this (10.1). There is no second level and no exception to
remember. The message for a private name says that it is private and
what to change, and never "no such name" (10.2).

An interface is declared with one word, and a class says which
interfaces it is with one more (10.4). Whether a class is a `Shape`
is written on its first line and checked there. The class writes
each function exactly as the interface does (10.6), so there is one
way to write it and nothing to work out. A list of circles is not
quietly a list of shapes (10.7), and nothing is cast: a list that is
written out is told by the place it goes what it is a list of.

A test is a block with a name, and `assert` says what must be so
(10.16, 10.17). There is one way to write each, a failure shows the
two values that were compared, and no test runs by surprise: `zeno
run` and a build go past them.

`zeno new` does one thing, and it is the same thing every time
(10.22): it asks nothing, it has one option, and what it writes is a
program you could have typed. A name that will not do is refused with
one that will, and is never changed quietly (10.23).

`x += y` means `x = x + y`, for every operator and every type, and it
means it in the same order in every build (10.25, 10.28). Where that
reading would do something nobody meant, a call in the place that
would run twice, the statement is refused and the message says what
to write (10.26). On a sized number it is the checked conversion, the
safe one of the two, and cutting stays a word in the program (10.27).
A format on a quantity never loses the unit (10.29).

A dict holds what a list holds (10.11), so there is one answer to
"what can I keep in a collection" and not two. The one thing it does
not hold is `none`, because `get` already means something by it. A
loop with two names reads the same over everything that can be
looped over: the second name is what is at the first (10.12).

### 10.1 `private`: for the file it is written in

```zeno
private let highest = ["temperature": 60.0]

private function wrapped(place: int, size: int) -> int {
    return place % size
}

class Ring {
    private slots: list[int]
    private count: int

    function size(self) -> int {
        return self.count
    }

    private function slot(self, steps: int) -> int { ... }
}
```

The word goes in front of a declaration at the top of a file
(`function`, `class`, `enum`, `let`, `var`, `const`), and in front of
a field or a function of a class. What it marks can be used by the
file it is written in, and by no other.

**Why at all.** 7.4 let every file use everything, and said a word
that hides could come later without breaking a program. Later is
now: a module of any size has parts that hold only as long as nobody
else touches them. A ring's count has to agree with its slots. If
any file can write `queue.count = 0`, the module cannot promise that,
and whoever reads it cannot tell what is safe to change.

**Why the file is the boundary, for a field as well.** In Java and
C++ a private field is for the class. In Rust and Go it is for the
module. Zeno takes the module, because then the word means one thing
in both places it can stand, and "who can use this?" has the same
answer everywhere: look at which file you are in. It also fits how
Zeno code is written: a function that builds a `Ring` or prints one
sits beside the class in its file, is not a method, and needs the
fields. With privacy by class it would need a second word such as
C++'s `friend`.

**Why the word `private`.** It is the word Java, C++, C#, Kotlin,
Swift and TypeScript use, so it is the one most readers know. It is
a whole word (principle 2). `pub` on what is open (Rust) was turned
down in 7.4: a mark on nearly every line. A leading underscore and a
capital first letter were turned down there too.

**Everything is still open unless it says otherwise.** A program in
one file never needs the word, and a first module works with no mark
on it. Hiding is something a module does once it has something to
protect.

**An enum is private whole.** `private enum Mode` hides the enum and
its cases. A single case cannot be private: a `match` on the enum has
to name every case, so a file that can see the enum must be able to
see them all.

**Not inside a block.** `private let x` in a function is refused,
with the reason: a name declared in a block is seen only in the block
already.

**Turned down.**
- *Three levels* (`private`, `protected`, `public`). `protected` is
  about inheritance, which Zeno does not have. Two levels need one
  word.
- *Private to the package*, so that the files of a package share what
  other packages cannot see. A real need in a big package, and it
  can be added as its own word. No program here has needed it.
- *Checking it when the program runs.* It is checked by `zeno check`,
  as types are. It is there to keep a program's parts honest with
  each other, and is not a lock.

### 10.2 A private name is said to be private

```
error: `wrapped` is private in the module `ring`: only its own file
can use it. To use it from another file, take the word `private` off
where it is declared
```

**Why not "module `ring` has no `wrapped`".** That would be the
safer thing to say to a stranger and the less useful thing to say to
the person who wrote both files, which is who reads it. The name is
there, the reader can see it, and a message that denies it sends them
looking for a spelling mistake.

When a module has no such name at all, the names it is said to have
are the ones another file can use.

### 10.3 Only its own file builds a class that has a private field

```zeno
# ring.zeno
function ring(size: int) -> Ring raises ValueError {
    if size < 1 {
        raise ValueError("a ring has room for one byte at least")
    }
    return Ring([0 for slot in range(size)], 0, 0)
}

# main.zeno
let queue = ring.ring(4)          # yes
let mine = ring.Ring([1], 0, 1)   # no
```

A class gets a constructor for free, which takes a value for each
field (3.7). Another file cannot see the private fields, so it cannot
fill them in, so it cannot call that constructor. The module writes a
function that builds one, and the other files call it.

**Why not leave the private fields out of the constructor for other
files**, so that `ring.Ring()` fills them from their defaults. Then
one class would take different arguments depending on which file the
call is in, and the third argument here would be the fifth there. One
name, one list of arguments.

**Why this is no burden.** It is what a module with something to
protect wants anyway: `ring.ring(4)` asks for a size, and how the
slots, the first place and the count start is the module's business.
This is the rule Rust has for a struct with a private field.

A class whose only private parts are functions is built as ever:
there is nothing to fill in.

### 10.4 `interface`, and a class says which ones it is

```zeno
interface Shape {
    function area(self) -> float
    function scale(self, by: float)
}

class Circle is Shape {
    radius: float

    function area(self) -> float {
        return 3.0 * self.radius * self.radius
    }

    function scale(self, by: float) {
        self.radius = self.radius * by
    }
}

function total(shapes: list[Shape]) -> float { ... }
```

An interface is a list of functions. A class says, on its first
line, which interfaces it is, and from then on an object of the class
goes wherever one of them is wanted. One function, `total`, serves
every class that is a `Shape`, the ones written and the ones not
written yet.

**Why the class has to say so.** The other design is that any class
with an `area` and a `scale` is a `Shape` without a word (Go, and
Python's protocols). It saves one line, and costs two things. A class
that happens to have an `area` meaning something else is a `Shape` by
accident. And a class that was meant to be one and has `scale`
misspelt is found out far away, at some call that passes it, with a
message about that call. With `is Shape` the promise is written where
the reader of the class sees it, and it is checked there:

```
error: `Dot` says it is a `Shape`, and a `Shape` has
`function scale(self, by: float)`. Give `Dot` that function
```

*(Principle 1: found before the program runs, at the place the
mistake is. And "unambiguous": whether a class is a `Shape` is
written down, never worked out.)*

**Why the words `interface` and `is`.** `interface` is the word Java,
C#, Go and TypeScript use for this, and the word a firmware engineer
uses for the thing itself: what a part shows to the rest. `trait` and
`protocol` mean it only to people who already know those languages.
`is` is already a word of the language, it reads as the sentence it
is (`class Circle is Shape`), and it cannot be taken for the test
`x is none`: one comes after the name of a class being declared, the
other between two values. `implements` says the same in four more
letters, and a colon (`class Circle: Shape`) is punctuation where a
word will do (1.4).

**A class has no parent class.** `class B is A` where `A` is a class
is refused, and the message says what to do instead: declare what
the two share as an interface. Inheriting fields and code brings the
questions every language with it has had to answer (which `area`
runs, what the object looks like in memory, what happens when two
parents have the same field), and none of them is needed to write
`total`. A class can be several interfaces: `class Box is Named,
Sized`.

### 10.5 An interface has functions, and nothing about how they are done

No fields, no bodies, `self` first, and a type on every parameter and
on the result.

**Why no fields.** How a class keeps its data is the class's own
business, and in firmware the layout of an object is something its
author decides. A field in an interface would decide it for every
class. If other code needs the value, the interface has a function
that returns it.

**Why no bodies.** A body in an interface is code that every class
gets without writing it, which is inheritance again by another door,
with its question of which body runs. With none, the code of a
function is in exactly one place: the class.

**Why everything is typed.** Elsewhere a type that is left out is
worked out from the body. Here there is no body. The interface
is all a caller has, so all of it is written.

Also refused, each with a message that says so *(was provisional: 11.5)*: a
default for a parameter, type parameters on an interface or on one of
its functions, an interface that is another interface, and a private
function (an interface lists what other code may ask).

### 10.6 The class writes each function as the interface does

The same names for the parameters, the same types, the same result.

```
error: `scale` of `Dot` is `function scale(self, factor: float)`, and
`Shape` asks for `function scale(self, by: float)`. Write the two the
same
```

**Why the names count.** An argument can be given by name (3.6):
`shape.scale(by=2.0)`. Through the interface that has to mean the
same for every class, so every class calls it `by`.

**Why the types are the same and not merely fitting.** A class could
safely take more than the interface asks, or return less. Allowing it
means a reader has to work out whether two different first lines
agree. The same line twice needs no working out. *("One obvious way
to write each thing.")*

**A class may raise less than its interface says, never more.**
Whoever calls through the interface knows only what the interface
says: `function send(self, text: str) -> int raises Full`. A class
whose `send` never fills up raises nothing, and that is fine. One
that raises something the interface does not list is an error at the
class, because the caller could not have been warned. In a firmware
build this is also what tells the compiler that a call through the
interface carries an error as a value (5.5l).

### 10.7 A class goes where its interface is wanted, and a list of it does not

A `Circle` is a `Shape` in a variable, a parameter, a result, a
field, and in each of those with `or none`. A `list[Circle]` is not a
`list[Shape]`.

**Why not the list.** Whoever holds it as a list of shapes may append
a `Square`, and the first holder would then find a square in a list
of circles. This is the reason a `list[u8]` is not a `list[int]`
(2.8c), and it is given in the message:

```
error: `shapes` should be list[Shape], but this is list[Circle].
Whoever holds it as a list of Shape could append another class to it.
Say what the list holds where it is made, as in
`let items: list[Shape] = [...]`
```

**A list that is written out is a list of what the place wants.**
`[Circle(1.0), Square(2.0)]` has items of two classes, and the items
do not say that they are shapes. The place does: a `let` with a type,
a parameter, a result, a field, an `append`. So the list is told what
it is a list of, and each item is checked against that. Nothing new
has to be written, and there is no cast.

The same rule repaired something older. `show([1, 2])`, where `show`
takes a `list[int or none]`, was accepted, and the compiled program
crashed: the list was built as a list of ints and read as a list of
`int or none`, which are kept differently (5.5j). It worked in a
`let` and not in an argument. Now a written list is built as what the
place wants wherever it is written.

**A generic takes the interface from the place its result goes.**
`let pair: Pair[Shape] = Pair(Circle(1.0), Square(2.0))` is a pair of
shapes, and `let box: Box[int or none] = Box(count)` can hold `none`
later. This holds when each argument goes straight into a `T`. A
`list[Circle]` given for a `list[T]` stays a list of circles, for the
reason above.

### 10.8 There is no way back from an interface to a class *(was provisional: 11.5)*

A `Shape` cannot be asked which class it is, and it cannot go where a
`Circle` is wanted. Through an interface only the interface's
functions are there.

**Why.** With a way back, code that was given a `Shape` begins to
ask "are you a `Circle`?", and from then on it works only for the
classes it asks about. The interface stops being the whole
agreement. For a value that is one of a few known things there is
already a better tool: an `enum`, where `match` has to name every
case (3.11). An interface is for the open set, an enum for the closed
one, and each is checked in its own way.

It also keeps one promise simple: what a function is given is all it
can use.

**What it costs.** Now and then a program does know what it has and
has to keep the object in a second variable of its own class. If
real programs keep running into that, a checked way back
(`match shape { case Circle as c ... }`) can be added without
changing anything written before. The message says what to do today.

Two shapes are equal when they are objects of the same class and
that class says they are equal. An object prints as the class it
really is.

### 10.9 In a compiled program: a number in the object, and a `switch`

```c
struct z_Circle {
    int32_t zeno_class;     /* 1: this is a Circle */
    double z_radius;
};

static double z_Shape_area(z_Shape *a0) {
    switch (a0->zeno_class) {
    case 1: return z_Circle_area((z_Circle *)a0);
    default: return z_Square_area((z_Square *)a0);
    }
}
```

A class that is some interface carries a small number as its first
field, which says which class the object is. A call through the
interface goes to one function per function of the interface, which
looks at the number and calls the function of that class.

**Why not a table of pointers to functions**, which is how C++, Java
and Rust do it. Three reasons, and the first is the one that decides.

- Zeno compiles a whole program at once (7.11), so every class that
  is a `Shape` is known. A `switch` over a known set is something the
  C compiler and LLVM can see through: they can put the class's
  function inline, and they know every place a call can go. A call
  through a pointer hides both.
- No pointer to a function sits in memory where a stray write can
  change where the program jumps. Firmware coding rules (MISRA among
  them) ask for that, and a tool that works out the deepest the stack
  can get needs to know every call.
- It costs four bytes in an object, and nothing in an object of a
  class that is no interface: such a class is exactly what it was.
  Rust and Go instead make every reference to an interface two words
  wide, which would change the size of every slot of every list
  (5.5h).

**What it costs.** Adding a class to an interface compiles the
`switch` again, which a whole-program compiler does anyway. And the
number is in the object, so a class cannot be made an interface
after the fact from outside its own declaration, which 10.4 already
rules out.

The interpreter and the VM needed no change at all: they look a
method up by its name on the object in front of them, and always
did.

To every stage after the parser an interface is a class with no
fields whose functions have no bodies. That is why the change is
small: the checker fills in what the functions are from their first
lines, and the two compilers write the `switch` where they would
have compiled a body.

### 10.10 What interfaces leave out *(was provisional: 11.5)*

Each is refused with a message that says so.

- **An `owned class` that is an interface.** In a build with no
  collector an owned object is freed by its one owner, through code
  written for its class. Freeing through an interface works (the
  example does it, and gives every block back), and an ordinary class
  can be used. What has not been thought through is an owned class
  whose functions take something out of the object, called through
  an interface that says nothing about it.
- **A function that takes out or replaces what its object owns**
  cannot stand for one of an interface, for the same reason.
- **Type parameters** on an interface or on its functions
  (`interface Stack[T]`). Bounds on a generic (`T is Ordered`, 3.16d)
  wait for the same thing.
- **An interface that is another interface.** A class names both.

A dict whose values are of an interface was on this list, and is in
the language since 10.11.

### 10.11 A value of a dict is anything a list can hold, except `none`

```zeno
let readings: dict[str, list[float]] = [:]
let limits = ["voltage": Limit(11.5, 12.5)]
let shapes: dict[str, Shape] = ["round": Circle(1.0), "square": Square(2.0)]
let ranges: dict[str, dict[str, float]] = [:]
```

A list, an object, a value of an interface, an enum, another dict, a
quantity, a function, a sized number. Keys are as they were: a `str`
or an `int` (6.31).

**Why.** The first real programs with a dict wanted the readings of
each channel under its name, and an object for each name. The way
round it (the objects in a list, and the dict holding positions) is
two collections kept in step by hand, which is where mistakes come
from. And "a list holds anything, a dict holds four types" was a
rule to remember with no reason a reader could see.

**What is under a key is the thing itself**, as it is in a list.
`readings["voltage"].append(12.0)` adds to the list in the dict, and
`limits["voltage"].high = 13.5` changes the object in the dict. There
is no copy that the change could be lost in.

**Why not `none`.** `get(key)` gives `none` for a key that is not
there (6.35). In a `dict[str, int or none]` a `none` from `get` would
mean two things, "no such key" and "the key is there and holds
`none`", and no program could tell which.

```
error: a value of a dict cannot be `int or none`: `get` gives `none` for a
key that is not there, and that could not be told from a value that is
`none`. Make it a `dict[..., int]`, and leave the key out where there is
no value
```

Leaving the key out says the same thing with what is already there.
Inside a value `none` is as anywhere: `dict[str, list[int or none]]`
is fine.

**Turned down.** Allowing it and saying "use `in` to tell them apart":
that is the surprise this language is meant not to have. A third
answer from `get` for a missing key: one more thing to check on every
call, for a dict nobody has asked for.

**In a generic** the key and the value can be type parameters:
`function flip[K, V](d: dict[K, V]) -> dict[V, K]`. What `K` and `V`
are is checked for each copy, with the same rules and the same
messages, and the message says which copy it is in.

**In a compiled program** a value that is not a plain one is a
pointer in the dict's list of values. How such a value is shown and
compared depends on its type, so the compiler writes a function for
each, as it does for a list of them, and hands it to the runtime
(`zeno_dict_str_with`, `zeno_dict_eq_with`).

### 10.12 `for key, value in ages`: a loop with two names

```zeno
for name, age in ages {
    print(name, age)
}
for at, word in words {
    print(at, word)
}
```

Over a dict the loop is given each key and what is under it. Over a
list, text or a range it is given the position, counted from 0, and
what is there.

**One rule for all of them: the second name is what is at the
first.** In a dict `age` is `ages[name]`, and in a list `word` is
`words[at]`. Nothing else has to be remembered, and it is why the
position comes first: it is the key of a list.

**Why.** `for name in ages` and then `ages[name]` on the next line is
what every loop over a dict did, and it is a lookup the loop has
already done. For a list the position was a `var` counted by hand
beside the loop, or `for i in range(len(words))` and `words[i]`: two
ways, both longer than what they mean.

**Over a range** the first name counts the trips: `for trip, n in
range(10, 0, -4)` gives `0 10`, `1 6`, `2 2`. That is the position of
the number in the list the range would be.

**The pairs are as they were when the loop began**, like the keys of
the loop with one name (6.38). The body may store into the dict and
take keys out.

**Each name is the loop's own**, cannot be changed, and is gone after
the loop, as the one name is. With no collector the value is looked
at, and the dict is whole again after the loop.

**Turned down.**
- `for (name, age) in ages`, or a pair as a value of its own: Zeno has
  no tuple, and a new kind of value for one loop is more than the
  loop needs.
- `for name, age in ages.items()` and `for at, word in
  enumerate(words)`, as in Python: two more names to learn, and each
  builds a list of pairs that the language has no type for.
- The item first and the position second over a list: then the two
  names mean one thing over a dict and the other way round over a
  list.
- Three names, or two that are the same, are refused with a message
  that says what two names are for.

### 10.13 A dict that is written out is a dict of what the place wants

```zeno
let shapes: dict[str, Shape] = ["round": Circle(1.0), "square": Square(2.0)]
let bytes: dict[str, u8] = ["start": 200, "stop": 7]
```

The same rule as for a list (10.7): the place says what the values
are, and each written value is checked against that. A dict that is
made already stays what it is, so a `dict[str, Circle]` is not a
`dict[str, Shape]`, for the reason a list of circles is not a list
of shapes: a `Square` could be stored into it through the other name.

With no type to go by, all the values are one type, and a mix says
so. A message for a number names the fix:

```
error: the values of this dict are float here, but this one is int.
Write a number with a decimal point, as in `1.0`, and convert a value with `float( )`
```

### 10.14 With no collector a dict holds plain values *(was provisional: 11.5)*

```
error: with no collector a value of a dict is an int, a float, a bool or
a str for now, and this is list[int]. Keep those in a list, and the
position of each in the dict
```

In a build with no collector a value of a dict is an `int`, a
`float`, a `bool`, a `str`, a sized number or a quantity: the values
that are copied when they are stored. Lists, objects and the rest
need the collector.

**Why.** With no collector everything has one owner (5.15). A dict of
lists would own its lists, and each way into the dict needs a rule:
`d[key]` would lend, `d[key] = value` would free the old value,
`pop` would hand its value over, `clear` and the end of the dict
would free every one, `values()` could not give a second list of
them, and `get(key, instead)` would have to say who owns `instead`.
Those are the rules of a list of owned items (5.15e) written out again
for a second collection, and so far no program for a board has needed
them: such programs keep a few objects in a list or in fields, and a
dict for looking a number or a name up.

**Meanwhile.** The objects go in a list, and the dict holds the
position of each: `channels[index["voltage"]]`.

**When this changes.** When a program that has to run with no
collector needs it. The rules above are the plan.

### 10.15 `print` makes its texts in the order they are written

Not a rule of the language, but a repair that this stage found. The C
for `print(table, x)` made the text of each value inside the call of
`zeno_print`, and C does not say in which order the arguments of a
call are worked out. Making the text of a list, a dict or an object
takes room for its pieces while it is made. So how much of the text
room was in use at most depended on the C compiler, and the report of
a build with no collector differed by a few bytes between the C and
the LLVM backend, which 5.11t says must not happen.

Where a `print` has two or more values with work to do, and one of
them is a list, a dict, an object, a call or text that is made there,
the work is now done on lines of its own, in the order the LLVM
backend does it: every value first, and then the text of each, as
written. Numbers alone stay inside the call, since their text takes
no room while it is made. What is printed never differed, and does
not change.

### 10.16 A test is `test "what it checks" { ... }`, at the top of a file

```zeno
test "a full ring keeps what it has" {
    let queue = ring(2)
    assert queue.put(1)
    assert queue.put(2)
    assert not queue.put(3)
}
```

A block with a name. The name is plain text in quotes, so it can say
what is checked in ordinary words, and it is what a failure is
reported under. Two tests of one file cannot have the same name.

**Where.** At the top of a file, and in any file. A test that stands
in the file it tests sees what is private to that file (10.1), which
is where the things worth testing usually are. A file of tests alone
(`ring_test.zeno`, which imports `ring`) is the same thing with
nothing else in it.

**What a test is to the rest.** A block of its own: what it declares
is gone when it ends. An error that leaves it fails that test, so a
test needs no `try` around what should work. It is checked with the
program, by `zeno check` and `zeno run`, so a test cannot go stale
unseen. It is run only by `zeno test`.

**`test` is not a reserved word.** `test "` at the start of a
statement can be nothing else, since a name followed by text means
nothing in Zeno. Everywhere else `test` is a name, and programs use
it as one: `examples/generics.zeno` has a parameter called `test`.
`owned`, `take` and `strict` are reserved (5.19c) because each stands
where a name could, and here no name could.

**Turned down.**
- Functions whose names begin with `test_`, as in Python and Go: a
  name that changes what a function is for is a rule nobody can see
  in the program, and names with spaces say more.
- A mark above a function, `@test`: Zeno has no marks, and one kind
  of them for one use is a new piece of syntax to learn.
- Tests only in files of their own: then nothing private could be
  tested, or `private` would need an exception for test files.
- Reserving `test`: it takes a common name away and settles nothing.

### 10.17 `assert` says what must be so, and a failure shows both sides

```
FAILED: a push is counted
error: this is false. The left of `==` is 1, and the right is 2
  --> ring.zeno:33:12
   |
33 |     assert length(r) == 2
   |            ^
```

`assert` and a condition. If the condition is a comparison (`==`,
`!=`, `<`, `<=`, `>`, `>=`, `in`, `not in`, `is`, `is not`), a
failure says what was on each side, shown as `print` would show it
inside a list, so that text has its quotes.

**Why one statement.** Other languages grow a family: `assert_eq`,
`assert_ne`, `assertTrue`, `assertIn`. Each exists so that a failure
can show the values. The interpreter has the comparison in front of
it, so it can show them for the plain `assert`, and there is one
thing to learn.

**A failed `assert` ends its test, and nothing catches it.** It is
not an error of the program, so `except Error` does not see it. A
test that expects an error can then be written without care for
what its own asserts raise.

**Only in a test.** In the program itself, `if` and `raise` say what
is wrong and to whom, and can be caught. An `assert` that vanishes
from a build, or that stops a board, would be a second way to report
a mistake with rules of its own. The word is reserved, so that
`assert (a or b)` is never read as a call.

**Turned down.** A message after the condition (`assert x > 0,
"..."`): the test has a name and the failure shows the line and the
values. It can be added without changing anything here.

### 10.18 `assert raises ValueError { ... }`: the block must raise that

```zeno
test "a ring of no size is refused" {
    assert raises ValueError {
        ring(0)
    }
}
```

The test goes on if the block raises an error of that kind, or of a
kind that is one (`Error` takes any). It fails if the block raises
nothing, or raises something else, and says which.

**Why a form of its own.** Fault paths are half of what firmware has
to get right, so tests for them should be the easy ones to write.
Without this it is `try`, the call, an `assert false` that says
nothing when it fails, and an `except`. `raises` is the word a
function's first line already uses for the same thing.

### 10.19 `zeno test` runs a file and then the tests written in it

```
$ zeno test examples/ring
examples/ring/ring.zeno
  ok      a new ring is empty
  ok      bytes come out in the order they went in
  ok      a full ring keeps what it has
  ok      the ring goes round
  ok      a ring of no size is refused

5 tests: 5 passed, 0 failed.
```

`zeno test ring.zeno` runs the file as `zeno run` would, and then
each test in it, in the order written. `zeno test folder` does that
for every file in the folder and below it that has a test, and `zeno
test` alone means the folder it is run in. The command fails (for a
script or a CI job) if any test failed or any file could not be run.

**Each file is tested as a program of its own.** Its tests start
from the state the file leaves, and the tests of two files cannot
disturb each other. A file with no test in it is not run at all, so
`main.zeno` does not blink its LED because its folder was tested.

**The tests of an imported file are not run** when the file that
imports it is tested. They are that file's own, and run when it is
tested. So a test runs once, however many files import its module.

**Where a file's imports are looked for.** In the folder of its
program: the nearest folder, from the file's own upward, with a
`zeno.package` or a `main.zeno` in it. That is the rule the language
server has (9.12), and it is what lets `drivers/uart.zeno` be tested
with the `import sensors` it has in the real program.

**Tests of one file share what the file declares.** A test that
changes a variable of the file leaves it changed for the next. That
is what a `var` at the top of a file is, and a test that needs a
fresh one makes it inside itself.

**The tests run in `zeno run`'s engine.** The compiler's own tests
hold every engine and build to the same output, so a second run of
a program's tests in the VM or in C would test Zeno and not the
program.

### 10.20 A build leaves the tests out

`zeno build`, with any of its options, takes the tests out before
the program is checked. They are not in the C, the IR or the
program, so a board pays nothing for them. And they are not held to
the rules of the build: a test may share a list, or use a dict of
lists, in a program that is built with no collector. `zeno check
--no-collector` checks what that build would build, so it leaves
them out too. `zeno run` and plain `zeno check` check them.

**Why.** A test is run on the computer where the program is written.
Asking it to follow the rules of a board would make tests harder to
write than the code they test, and fewer would be written.

### 10.21 What tests leave out *(was provisional: 11.5)*

- A message on `assert` (10.17).
- `assert` in a function outside a test. A helper gives back a
  `bool` and the test asserts it.
- Lines to run before and after each test. A function that builds
  what the tests need does that today.
- Choosing tests by name on the command line, and running them side
  by side.
- Running the tests of a compiled program, on a board or in the VM.

### 10.22 `zeno new blinker` makes a program that runs and has a test

```
zeno new blinker
```

makes the folder `blinker`, and three files in it:

| File | What it is |
|---|---|
| `zeno.package` | `package blinker`, under a comment that shows the two kinds of `needs` line |
| `main.zeno` | imports `greeting` and prints what `greeting.hello("world")` gives |
| `greeting.zeno` | the function `hello`, and one test of it |

Then it says what it made, and the two commands to try: `zeno run
blinker/main.zeno` and `zeno test blinker`.

**Why these three.** Whoever runs `zeno new` wants to see something
run, and then wants to know where things go. A lone `main.zeno` with
a `print` in it answers the first and not the second, and nobody
needs a command to write one line. The three files are the three
things a program of more than one file is made of: the file it starts
in (7.2), a module that is imported (7.1), and the project file
(8.1). The test is there so that `zeno test` works in the first
minute, and so that the first test a person writes has one to copy
that stands where tests stand (10.16).

**Why it is so small.** What is written is read once and then
changed. Every line more is a line to delete. Each file starts with a
comment that says what the file is, and that is all the teaching
there is room for.

**It is checked.** The compiler's own tests make a program and a
package with `zeno new`, check them, run them, run their tests and
compare each file with what `zeno format` would write. What `zeno
new` writes cannot fall behind the language without a test failing.

**Turned down.**

| Idea | Why not |
|---|---|
| Templates: `zeno new --template firmware` | Several starting points means choosing one before knowing the language, and each one is a program that has to be kept right. A program for a board differs in what `main.zeno` says, and [`board.md`](board.md) says it |
| Questions at the prompt (name? licence? author?), as `npm init` asks | The command would have two ways to be run, and a script could not run it. The name is the one thing needed, and it is on the command line |
| `git init` and a `.gitignore`, as Cargo does | Whether a program is in git, and in which repository, is not the compiler's to decide. A folder made inside a repository would get a second one |
| A `README.md`, a licence, a `src/` folder | Not Zeno's files. The folder of a program is the folder its `main.zeno` is in (7.2), so a `src/` would be a second answer to "where does the program start" |
| `zeno init`, into the folder you are in | See 10.23 |
| `zeno add units ...` | 8.9: a `needs` line is one line, and the new `zeno.package` shows it |

### 10.23 The name is a Zeno name, and a folder that is there is left alone

The last part of the path is the name: of the folder, of the program
on the `package` line, and, for a package, of the module `import`
reads. So it has to be a name of the language. These are refused:

| `zeno new ...` | Why |
|---|---|
| `my-app`, `2fast` | not a name: "A name is letters, digits and `_`, and does not start with a digit ... Try `my_app`" |
| `while` | a word of the language |
| `math` | a module that comes with Zeno, which an `import math` already means |
| a path where something already is | "`zeno new` makes a new folder, and writes nothing into one that is there" |

**Why refuse and not repair.** Cargo takes `my-app`, keeps it as the
package's name and calls the crate `my_app`, so one thing has two
names from its first day. Here the folder, the `package` line and the
`import` say the same word. The message gives the word to use, so the
repair is one more try.

**Why nothing is written into a folder that is there.** A command
that makes files must never be able to replace one. If the folder is
there, either it has work in it or the name was a slip, and in
neither case is writing into it what was wanted. Checking each file
instead ("write `main.zeno` unless there is one") leaves a folder
that is half what was there and half new, which nobody asked for.

**Turned down.** `--force`. `zeno init` for a folder that is there:
the files are three, and in a folder with work in it the right ones
to add depend on the work.

### 10.24 `zeno new --package filters` makes a package

```
zeno new --package filters
```

makes `filters/zeno.package` and `filters/filters.zeno`: the module
with the package's own name, which is the file `import filters` reads
(8.5), with the same `hello` and the same test. There is no
`main.zeno`, because a package is not run. The comment in its
`zeno.package` shows the line a program writes to need it.

**Why an option, and why this word.** A package and a program differ
in which files they start with, and in nothing else, so one command
with one option says it. The option is the word the language already
uses for the thing: `package` is the first line of the file that
makes a folder one.

**Turned down.** `zeno new package filters`: then `zeno new package`
would have two readings, a program called `package` or a package
with no name. `--lib`, Cargo's word: Zeno has no libraries, it has
packages. Writing both a `main.zeno` and the named module into every
new folder: a program would start with a file it does not use.

### 10.25 `x += y` is `x = x + y`: what is there is read first

```zeno
var count = 1

function bump() -> int {
    count = 10
    return 1
}

count += bump()     # 2
```

`count` is read, then `bump()` runs, then the two are added and the
sum is stored. It is the order of `count = count + bump()`, which is
left to right like every other expression.

**What was wrong.** This program printed 2 when it was built through
C, and 11 in the interpreter, in the VM and when it was built through
LLVM: those three worked the value out first and read `count`
afterwards. No test had a value that changes the place it is added
to, so the four engines had never been asked. It was found while
`+=` on a sized number was being made (10.27), and it was mended
first.

**Why this order.** `x += y` is explained everywhere in Zeno as "`x =
x + y`" (5.8k), and now it is that, instruction for instruction: the
VM's code for the two statements is the same four instructions. It
is also the answer of Python, Java, JavaScript and C#. The other
order has nothing to explain it by.

**Turned down.** Leaving it to each engine, as C leaves it to each
compiler: a program must do the same thing however it is run.

### 10.26 The place of a `+=` has no call in it

```zeno
xs[next()] += 5
```

```
error: `+=` reads this place and then writes it, so a call in the
       place would run twice. Keep what the call gives in a `let`
       first, and use that name here
```

```zeno
let position = next()
xs[position] += 5
```

**What was wrong.** `x += y` is `x = x + y`, so `xs[next()] += 5`
was `xs[next()] = xs[next()] + 5`: two calls. With a `next` that
counts, it read item 1 and wrote item 2. Every engine did this
alike, so no test caught it, and nobody who writes that line means
it.

**Why refuse.** There are two things the line could mean, one call or
two, and the one that follows from the rule (two) is the one nobody
wants. A line with two readings is refused, and the message gives the
line that has one. The `let` is what a compiler would have to add in
secret to call once. Here it is in the program, where the reader can
see which value was kept.

**It can be lifted.** If working the place out once turns out to be
wanted, allowing it later changes no program that is accepted today.
Keeping the two calls would have been the choice that cannot be
undone.

**Cost.** `xs[len(xs) - 1] += 1` needs a `let` too, though `len`
changes nothing. The rule is "no call", with no list of calls that
are harmless to remember, and a function of the program could never
be on such a list.

**Turned down.**

| Idea | Why not |
|---|---|
| Work the place out once, with a hidden variable (C, Python, Rust) | Right, and four engines each have to do it and be kept alike. The `let` costs one line and is visible |
| Allow calls that change nothing, such as `len` | A list to learn, and the checker cannot know it of a function the program wrote |
| Leave it: the rule says two calls | The rule is for reading programs, and this reading is a trap |

### 10.27 `count += 1` on a sized number is `count = u8(count + 1)`

```zeno
var count: u8 = 250
count += 5      # 255
count += 1      # OverflowError: 256 does not fit in a u8 (0 to 255)
```

The sum is an int, as 2.8a says every sum is. Then it is stored
through the checked conversion (2.8b): it goes in if it fits, and it
is an `OverflowError` if it does not. This is so for every operator
with an `=` form, and for every place a sized number is kept: a
variable, a field, an item of a `list[u8]`.

```zeno
flags |= 0x80                       # always fits
left -= 3                           # an error below 0, for a u8
count = u8.wrap(count + 1)          # going round is still said in a word
```

**Why.** 2.8a made the place a number is stored the one place it may
not fit, "where the program has to say what it wants". `count += 1`
names the place and what goes into it. The only thing it leaves out
is whether to check or to cut, and 2.8b already answers that: the
short spelling is the safe one, and cutting is a word that a reviewer
can search for. So the shortest spelling of all is the check. A
counter that passes 255 without anybody deciding it should is a bug
in most programs, and the ones that mean it say `wrap`.

Before this, the most common thing done to a counter needed the
longest line, `count = u8(count + 1)`, and the first programs with a
`u8` in them ran into it.

**`/=` is still refused** on a sized number, as on an `int`: `/`
gives a float. `//=` is the one that divides whole numbers (10.28).

**How it is built.** The checker is the only stage that knows `count`
is a `u8`. It notes each such statement, and `checker::checked`
rewrites it into the long form before anything runs or compiles it
(`Program::with_fits`). The interpreter, the VM and both compilers
only ever see `count = u8(count + 1)`, which they could all do
already, so there is nothing new that four engines must do alike.
`zeno parse` and `zeno format` show the program as it is written.

**Turned down.**

| Idea | Why not |
|---|---|
| `+=` cuts at the size, as in C | `250 + 10` is quietly `4`. That is the bug 2.2 and 2.8a exist to stop |
| Keep refusing it | It refused the line everybody writes, to protect a choice that 2.8b had already made |
| A second operator for cutting, such as Zig's `+%=` | New punctuation where a word, `wrap`, is there already |
| Teach each engine the check | Four pieces of code to keep alike, for what one rewrite says once |

### 10.28 Every operator on two numbers has an `=` form

`+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `**=`, `&=`, `|=`, `^=`, `<<=`,
`>>=`. Each is `x = x op y`.

`//=`, `%=` and `**=` were missing. `count //= 2` was "expected an
expression, found `=`", and `/=` on a whole number can never be
right, because `/` gives a float. Halving an `int` in place had no
short spelling at all.

**Why.** The rule is now one sentence with no exceptions to remember.
Whoever knows `//` and `+=` will write `//=` without looking it up,
and it should work.

**Left out.** `and=` and `or=`: they are words, they work on `bool`,
and nobody expects them. `=` forms of the comparisons mean nothing.

### 10.29 A format on a quantity is the number's, and the unit stays

```zeno
let angle = 30.456 deg
let supply = 3.3 V
print(f"{angle:.1f}")        # 30.5 deg
print(f"[{supply:8.2f}]")    # [  3.30 V]
```

A quantity takes the formats a float takes. The decimals are the
number's. The unit follows it after a space, as it does whenever a
quantity is shown. The width counts both, and a quantity lines up on
the right, as numbers do.

Before, `f"{angle:.1f}"` was refused ("`.1f` needs a number"), and
the way round it was `f"{angle / 1 deg:.1f} deg"`: the unit taken
off by hand and written back as text, where nothing checks that the
two agree.

**Why the unit stays.** A quantity that prints as `30.5` has lost
the one thing that made it a quantity, at the moment it reaches a
person. Whoever wants the bare number divides by one of the unit
(`angle / 1 deg`), which says which unit the number is in.

**Why the width counts the unit.** "At least this many characters"
should be true of what is printed. With it, a column of quantities
ends at the same place whatever their units are, when the width is
enough for the longest.

**Changed with it.** `f"{wait:10}"`, a width and nothing else, was
already allowed, and laid the quantity out as text, on the left. It
is on the right now, like `f"{n:10}"` for a number. `x`, `b` and `d`
are still refused: a quantity is not a whole number.

**Turned down.** Dropping the unit when there is a format. A second
format for the unit (`{angle:.1f:unit}`): there is nothing to choose
about a unit except whether it is there, and it always is.

## 11. Version 0.1: every provisional decision, read again

A decision was marked provisional when it was made on less than it
deserved: before a later stage existed, before a real program had
tried it, or to keep a stage small. The last thing on the list for
version 0.1 was to read each one again and say what it is now. There
were about eighty. This chapter has every one of them.

Each has one of three answers.

- **Settled.** It is the rule, and it is no longer waiting for
  anything. The reason says what was learned since.
- **A limit of 0.1.** The thing that was left out stays out of this
  version. It is a known edge of the language and not an open
  question about it: the program is refused with a message that says
  so, or the page that describes the feature says what to write
  instead. The reason names what would bring it in.
- **Changed.** It is not as it was. The row says which decision
  changed it.

A mark in the chapters above now reads "was provisional", and points
here. The text around it is left as it was written, because the
reasons a thing was once uncertain are worth keeping.

**How each was judged.** By the two tests every decision has had
since 2026-11-14. Is the rule one sentence that a reader can hold,
with no second reading? And where something is left out, does Zeno
say so in plain words, with what to write instead? A provisional
decision that passes both is settled. A thing left out that would
need a new rule is a limit, and adding it later must not change what
any program that is accepted today means. Nothing on the lists below
fails that last test, which is why none of them had to be decided in
a hurry.

**One limit is silent**, and it is stated here because the rest are
not. In a program that is not strict, a value whose type is not
known until the program runs can be put where a `u8` is wanted, and
nothing checks it, just as it can be put where an `int` is wanted.
`--strict`, a file that begins with `strict`, and every build check
it. (5.13, [`sized-ints.md`](sized-ints.md).)

### 11.1 The language: chapters 0 to 3

| Decision | What was left open | Now | The reason |
|---|---|---|---|
| 0.1, 3.7 | Classes have no inheritance | Settled: there is none | Interfaces (10.4) give what inheritance is mostly used for, one function that serves many classes. What they leave out is a class whose fields and behaviour are written in another class, and that is the part that makes a class hard to read. A class in Zeno is read in one place |
| 1.13 | General unions, `int or str` | A limit of 0.1 | `T or none` and an enum whose cases carry values cover the uses, and each has one way to be taken apart (`is none`, `match`). A union would be a second way to write an enum |
| 1.15 | Derived units (`V * A` is `W`), and units a program declares | A limit of 0.1 | Two quantities cannot be multiplied, and the message says so. Doing it right is an algebra of dimensions, which is a feature of its own size |
| 2.8c | A `u8` could go where a `u16` is wanted without `u16( )` | Settled: it is written | "A sized number is an `int` for free, and everything else is written" is one rule. The loosened rule is a table of thirty pairs (does a `u8` fit an `i8`? an `i8` a `u16`?) that every reader would have to know |
| 2.8d | No `u64` | A limit of 0.1 | Half its values do not fit in an `int`, so "the answer of an operator is an `int`" (2.8a) would stop being true. It needs its own answer to what `u64 + u64` is |
| 3.6a | A shorter way to write a one-line function | Settled: there is one way to write a function | Comprehensions and a named function handed to `map` cover the short cases in every example. A second syntax for the same thing is what principle 4 is against |
| 3.7 | No constructor written by the program | Settled: a function builds one | The fields, in order, are the constructor. A class that must check what it is given keeps a field `private` and offers a function with a name (`ring.ring(4)`, 10.3), which says what it does better than a constructor could |
| 3.9 | `let` fixes the name, not what it names | Settled | It is one sentence: the name always means the same thing. It is what `final` means in Java and `const` in JavaScript, so it is what most readers expect. A list that nobody may change would be a second kind of list, and no program has asked for one |
| 3.10 | Lists and objects are shared | Changed, long ago: kept, with a collector to make it safe (5.7), and with one owner where there is none (5.15) | |
| 3.10a | The copy a `for` loop takes could be skipped | Settled as it is | The loop sees the list as it was when it began, always. The copy is given back when the loop ends (5.8j). Skipping it where the body cannot touch the list would change no program, so it is not a question about the language |
| 3.12 | A shared variable that was narrowed is looked at again as it is used | Settled | The other rule, Kotlin's, refuses to narrow such a variable at all, which makes `if port is not none` useless for every `var` at the top of a file. The second look is one comparison, made only for variables that something else can reach |
| 3.14 | `upper`, `lower`, `strip` and `split` know plain letters and blanks | Settled for the language | The full rules need tables of tens of kilobytes, and a method must do the same on a board as on a computer. They belong in a module for programs with the room |
| 3.15b | One LED and numbered pins, and no inputs | A limit of 0.1, and the one item taken off the list for it | Reading a pin, an ADC or a UART is designed against a real board. A design made without one would be a guess, and the simulated board would agree with whatever was guessed |
| 3.16d | A generic has no bounds: its body is checked for each type it is used with | A limit of 0.1 | The error is in the generic, with a note that names the copy and the call. Bounds (`T is Ordered`) wait for interfaces that have type parameters (10.10), since a bound is an interface |
| 3.16e | Generic enums, a method with type parameters of its own, a generic kept as a value, an owned value as a type argument | A limit of 0.1 | Each is refused with a message that says so |

### 11.2 How it is built: chapter 5

| Decision | What was left open | Now | The reason |
|---|---|---|---|
| 5.5c | A value of unknown type in a compiled program | Settled: a build is strict | Types that are not written are worked out, so most programs with none written build as they are. One that truly needs a value of any type runs with `zeno run` |
| 5.5e | Checked arithmetic uses built-ins of GCC and Clang | A limit of 0.1 | They cover the toolchains for ARM, RISC-V, AVR and ESP. Another C compiler needs the checks written out, in one place in `runtime.c` |
| 5.5g | An object that never leaves its function could live on the stack | Settled as it is | It would be an optimisation that changes no program. With no collector the object is freed where its owner ends in any case |
| 5.5k | A function value is always a block, even one that keeps no variables | Settled | Two function values are equal only when they are the same one, in every engine. The fixed value would save a block and break that for a function written in a loop |
| 5.5l | A function that can raise cannot be used as a value in a firmware build | A limit of 0.1 | The type `function(str) -> int` does not say whether it raises, and the two sorts are called differently there. The way in is `raises` in a function type |
| 5.7 | Three things about the collector | Changed, twice, and settled once | A build with no heap at all exists: `--no-collector` (5.9). It takes its memory from one fixed pool (5.9h). Counting references was not needed: a program that cannot pause is built with no collector, and one that can keeps the collector as it is |
| 5.8a, 5.19a | No mark at the call for a value that is taken (`send(take p)`) | Settled | The function's own header says `take` (5.19b), and a use after it is an error that names the line of the call. One way to write a call was worth more than a second reminder |
| 5.8c, 5.15b | Whether a plain parameter takes or looks | Changed in 5.19a: a plain parameter looks, and `take` takes | The count asked for in 5.15b was made in the audit: most parameters only look |
| 5.8i | What an owned value cannot do: sit in an `or none`, an enum case or a function type, be moved out of a field, be replaced through a borrow, be matched, be raised | A limit of 0.1 | Each is an error that says what to do instead, and allowing any of them later breaks no program |
| 5.9d | `raise e`, to pass a caught error on, is refused with no collector | A limit of 0.1 | The `except` that caught it frees it. Raise a new error, or lend the caught one to a function |
| 5.9g, 5.15j, 5.16i, 5.17g | What a build with no collector refuses | A limit of 0.1. The list is the one in 5.17g, with 5.16h and 10.14 | Two of them are the design and not a gap: a function that shares a `var` with the function around it, and keeping a function that only looks. The rest each need a rule for one more kind of owner |
| 5.9h | The pool does not join spare blocks back together | Settled | Sizes double, so a spare block is used again by the next thing of its size. Joining is three times the code, and no example is short of memory for the lack of it |
| 5.9j | The pool's size is one number in the C, and not an option of `zeno build` | A limit of 0.1 | `--memory` prints the number to change it to. `--pool N` is an exercise in [`pool.md`](pool.md) |
| 5.9k | Spare blocks are not poisoned in a checking build | A limit of 0.1 | A block given back twice is caught already. The rest is an exercise in the same page |
| 5.9l | With no collector text is kept by copying it, not by counting who holds it | Settled | A count would keep text alive by a route no line of the program shows, which is what this build exists to rule out |
| 5.9o | Text read from a place a function can change is copied even when nothing is called | Settled | "Read from a place, it is copied" is one rule. Whether a copy is made should not depend on what else is on the line |
| 5.9p | The copy a `for` loop takes of a list of text is deep | Settled | The body may replace an item while the loop still holds it |
| 5.9s | A plain list parameter could not be given a list that was just made | Changed in 5.15: a list has an owner in this build, and since 5.19a the parameter looks at it for the length of the call | |
| 5.9w | `for ch in text` makes a list of the characters | Settled | It is a made list like any other, and it is freed when the loop ends. Counting with `range(len(text))` is there for the program that cannot spare the blocks |
| 5.10c | A bytecode instruction is a Rust enum, not bytes | Settled | The VM is there to be read and to check the interpreter against. Speed is what the two compilers are for |
| 5.10g | `zeno run` uses the interpreter unless told `--vm` | Settled | It is the plainest statement of what a program means, it is the engine `zeno test` uses, and every other engine is tested against it |
| 5.11d | The LLVM build's runtime is the C build's, with `static` taken off | Settled | One runtime means one place for a mistake to be, and the test that the two builds use memory alike depends on it |
| 5.11i | A list item crosses into the runtime as an `i64` | Settled | One list type in the runtime (5.5h) is what both builds share |
| 5.11l | `_setjmp` is the name on Linux and macOS | A limit of 0.1 | Those are the systems Zeno is built and tested on. A firmware build has no `setjmp` at all |
| 5.11o | In `--llvm --firmware` an error comes back through a pointer and not as a pair | Settled until a board says otherwise | A pointer is the same on every processor, and LLVM removes it for small functions |
| 5.11p | The drop of a list that is certainly empty stays a call | Settled as it is | It is a call that does nothing, and removing it changes no program. It is an exercise in [`llvm.md`](llvm.md) |
| 5.13a | Only a field of an object is stored small. A `list[u8]` takes eight bytes an item | A limit of 0.1 | The runtime has one list type. A packed list of bytes is the first thing a real driver will ask for, and is a stage of its own |
| 5.14c | Two copies of a generic that would get the same C name are refused | Settled | Names that could never clash would be unreadable, and reading the C is half the point of that build |
| 5.15e | A method cannot take out what its object owns while the object is lent | A limit of 0.1 | The four rules of 5.8d stay four. The smallest growth is written in that section |
| 5.16h | `list[int or none]` with no collector | A limit of 0.1 | A number that may be `none` sits in a block of its own inside a list, and those blocks have no owner yet |
| 5.17d | A function that uses another function value which has an owner cannot be returned | A limit of 0.1 | "compose" needs a way to take a value into a function |
| 5.18d | `allocate` that finds no memory stops the program | Settled for 0.1 | Giving back `none` would put a check on every `allocate` for a case most programs cannot recover from. Firmware that must survive it will want `pointer[u8] or none` |

### 11.3 The standard library: chapter 6

| Decision | What was left open | Now | The reason |
|---|---|---|---|
| 6.6 | `sort()` by a function the program gives | A limit of 0.1 | Numbers and text have one order that nobody has to be told. Tasks could be ordered by name or by priority, and a `sort()` that picked one would be a guess |
| 6.8 | `char(0)` is refused | Settled | In a compiled program text ends at the first zero byte. A byte that may be zero belongs in a `list[u8]` |
| 6.10 | The methods of text and lists are written in Rust and C, not in Zeno | Settled for 0.1 | Each is written once for the interpreter and once for the runtime, and the tests hold the two together. A library in Zeno is the natural next thing after 0.1, and 7.2 waits for it |
| 6.12, 6.20 | A file is read whole. One too large to hold cannot be read in pieces | A limit of 0.1 | Reading in pieces needs an open file, which is a value with an owner and a moment at which it closes |
| 6.14 | One kind of error for a file, `FileError` | Settled | The message says which it was, for a person. A program that must tell them apart asks first, with `files.exists` |
| 6.16 | A file holds text. There is no `files.read_bytes` | A limit of 0.1 | It gives a `list[u8]`, and is worth having when that list is packed (5.13a) |
| 6.19 | No `program.exit(code)`, and no environment variables | A limit of 0.1 | A program ends by reaching its end, or with an error nobody caught, which ends it with status 1 |
| 6.21 | `hypot`, the hyperbolic functions, `gcd`, `clamp`, and random numbers | A limit of 0.1 | Each is a few lines of Zeno today. Random numbers want a module of their own, and on a board a source of them |
| 6.24 | `asin`, `acos`, `atan` and `atan2` give a quantity in `rad` | Settled | A float that is secretly an angle is what units are there to stop. Since 10.29 it prints with any number of decimals, which was the one thing that made it awkward |
| 6.24 | `atan2` takes plain numbers, not two lengths | A limit of 0.1 | Divide each by one of its unit first |
| 6.32 | A dict held only ints, floats, bools and text | Changed in 10.11: it holds what a list holds | |

### 11.4 Files, packages and editors: chapters 7 to 9

| Decision | What was left open | Now | The reason |
|---|---|---|---|
| 7.2 | The five modules that come with Zeno keep their names | Settled | `zeno new` refuses those names for a program (10.23), so the rule is met on the first day and not by an import that finds the wrong thing |
| 7.3 | No `import drivers.uart as uart` | Settled | One module has one name. `from drivers.uart import send` gives the short form of what is used, and since this pass an `as` after an `import` gets a message that says so |
| 7.4 | Everything at the top of a file could be imported | Changed in 10.1: `private` | |
| 7.11 | The files of a program are joined and compiled together | A limit of 0.1 | Compiling each file by itself needs a written description of what every module offers. It is what a large program will want, and no program is that large yet |
| 8.3 | No registry of packages with short names | Settled for 0.1. Added since, in chapter 27 | A package is a folder or a git address. A registry could be added as a third kind of `needs` line, and the two that exist would stay as they are |
| 8.9 | No `zeno new`, no `zeno add` | Changed in 10.22: `zeno new`. `zeno add` is settled: there is none | A `needs` line is one line, and a new `zeno.package` shows both kinds |
| 9.3 | The formatter never joins or splits a line | Settled | Where a line breaks is the one thing about layout that carries the author's meaning. Everything else has one answer |
| 9.8 | The formatter does not line comments up | Settled | It keeps the distance the author chose, and never fights a column somebody made on purpose |
| 9.11 | Hover on a field or a method (`p.x`, `queue.put`) gives no answer | A limit of 0.1 | The checker notes names, and does not yet note what a dot found. An empty answer is better than a guess |
| 9.13 | Colours come from a grammar in the editor, not from the server | Settled | The text is coloured at once and needs no server. Colours from the server could be added on top without changing either |

### 11.5 What real programs needed: chapter 10

| Decision | What was left open | Now | The reason |
|---|---|---|---|
| 10.5 | A function of an interface has no default for a parameter, no type parameters, and cannot be private | A limit of 0.1 | Each is refused with a message. The last is settled: an interface lists what other code may ask |
| 10.8 | No way back from an interface to a class | Settled | A function that needs to know the class is a function the interface should have. Asking "which class are you?" is how code that was written against an interface comes to depend on every class behind it |
| 10.10 | An owned class as an interface, a function that takes out of its object as one of an interface, type parameters on an interface, an interface that is another interface | A limit of 0.1 | Each is refused with a message. The first two are the same question about who frees what, and need the same answer |
| 10.14 | With no collector a dict holds ints, floats, bools, text, sized numbers and quantities | A limit of 0.1 | A list or an object under a key needs the dict to be its owner, as a list is of its items (5.15c). That is one more owner, built the same way |
| 10.21 | No message on `assert`, no `assert` outside a test, nothing run before each test, no choosing tests by name, no tests in a compiled program | A limit of 0.1 | The failure already shows both sides of the comparison (10.17), which is what a message is usually written to say |

### 11.6 What 0.1 is, then

The language is what chapters 0 to 10 decide, with the limits in the
tables above, and nothing in it is waiting to be decided.

Everything marked "a limit of 0.1" is one list, and it is the list
for whatever comes after: inputs on a board, a packed `list[u8]`,
the last few things a build with no collector refuses, interfaces
with type parameters and the bounds that come with them, a file read
in pieces, and a library written in Zeno. Each can be added without
changing what a program written today means, because each is refused
today and not quietly given some other meaning.

---

## 12. Releases

The walkthrough is [`release.md`](release.md).

### 12.1 A release is a tag, and a machine builds it

```sh
git tag v0.1.0
git push origin v0.1.0
```

Pushing a tag starts `.github/workflows/release.yml`, which runs the
tests, builds `zeno` for every kind of machine, checks each build,
packs it and makes the release. Nobody builds a released `zeno` by
hand.

**Why.** A release built by hand depends on what was on that machine
that day, and nobody can say afterwards what that was. Built from a
tag, a release is exactly the source at that tag, built the same way
each time, and the file that says how is in the repository beside the
source.

**The tag and `Cargo.toml` must agree.** The tag is `v` and the
version in `Cargo.toml`, and the workflow stops if they differ.
Otherwise a `zeno` that prints `0.1.0` could be on a page that says
`0.2.0`, and the first question about any problem ("which version?")
would have two answers.

**All or nothing.** If the build for one kind of machine fails, no
release is made. A table with a hole in it tells somebody with that
machine that Zeno is not for them, when the truth is that a build
broke.

**Started by hand, it releases nothing, unless asked.** The same
workflow can be started from the Actions page. It then builds and
checks everything and stops, which is how a change to the workflow is
tried. With the "publish" box ticked it goes on to make the release,
and the tag with it, from the version in `Cargo.toml`. The box is
there because a tag cannot be pushed from everywhere the source can
be, and a release should not wait for one machine. It is off unless
ticked, so a trial can never turn into a release by accident.

**Turned down.** Building everything on one Linux machine: macOS
programs cannot be linked there without Apple's files, and nothing
would have been run on macOS or Windows. A `curl ... | sh` installer,
and packages for apt, Homebrew and winget: each is a thing to keep
working, and one file to unpack is enough for 0.1.

### 12.2 Ten kinds of machine

Linux on x86-64, ARM64, 32-bit x86, ARMv7 and ARMv6. macOS on Apple
silicon and Intel. Windows on x86-64, 32-bit x86 and ARM64.

**The rule.** A kind of machine is in the list if Rust builds for it
with nothing more than Rust brings, and people have it on a desk. The
two 32-bit ARM builds are there for the Raspberry Pi, which is where
firmware people are most likely to meet Linux on ARM.

**Left out.** Linux on RISC-V: Rust needs a separate C toolchain to
link it, and no build would have been run. The BSDs, for the same
reason. Both can build Zeno from source with `cargo build --release`,
because Zeno depends on nothing but Rust.

**The names say the system and the processor.** The file is
`zeno-0.1.0-linux-arm64.tar.gz`, not
`zeno-0.1.0-aarch64-unknown-linux-musl.tar.gz`. The second is Rust's
name for the same thing, and only means something to a person who
already knows Rust. *(Principle: unambiguous and intuitive.)*

### 12.3 A released `zeno` is one file that needs nothing

The Linux builds are linked against musl, a C library that is put
into the program. The Windows builds carry Microsoft's C library the
same way.

**Why.** A Linux program normally borrows the C library of the system
it runs on, and runs only where that library is at least as new as
the one it was built against. A `zeno` built on this year's Ubuntu
would not start on a Raspberry Pi with a system from three years ago.
With the library inside it, one file runs on every Linux. On Windows
the same choice means no "install the Visual C++ runtime" before the
first run.

**What it costs.** A few hundred kilobytes in each file.

**The names for a debugger are taken out** (`strip = true` in
`Cargo.toml`), which makes each file about a seventh smaller. A report
of a crash in `zeno` itself then has no function names in it. Zeno's
own mistakes are reported as messages, not crashes, so this costs
little.

### 12.4 A build is run before it is released

`tools/release-check.sh` runs each build: the version it says, every
example through both engines, the tests in the examples, the
formatter, and `zeno new`. The ARM builds for Linux run under an
emulator. The Intel build for macOS runs under Apple's translation.

**Why.** "It compiled" says that the compiler was content. It does
not say that the program starts. The check is small on purpose: it
needs no C compiler and takes seconds, so it can run ten times.

**The one build that is not run** is Windows on ARM64: there is no
machine to run it on and no emulator there. It is released anyway,
because the same source built by the same compiler for the other two
Windows machines does run, and the release page says plainly that
this one was not. Leaving it out would help nobody who has such a
machine, and saying nothing would be claiming a check that was not
made.

**The whole test suite** must pass on Linux before anything is
released. It is also run on macOS and on Windows, where a failure is
shown and does not stop the release: those runs are there to find out
what is different on those systems. For 0.1.0 all the tests pass on
macOS, and what Windows says is in 12.7. On macOS three examples are
also compiled through C and through LLVM and must print what `zeno
run` prints.

### 12.5 A compiled program rounds as `zeno run` does

Making the first release found a program that printed
`74.30000000000001` with `zeno run` and `74.3` when compiled on a
Mac. The line was `c * 1.8 + 32`.

Some processors have one instruction for "multiply, then add" that
rounds once instead of twice. It is faster, and now and then the last
digit is different. ARM64 has it, and a C compiler is allowed to use
it without being asked. On 32-bit x86, gcc does a similar thing for
another reason: it uses the old float instructions, which work to
more digits than a float holds.

**The decision.** The C that Zeno writes tells the C compiler not to
do either. Every float operation is one operation, rounded once, on
every machine, as it is in `zeno run`.

**Why.** "A compiled program prints what `zeno run` prints" is the
promise the tests are built on, and a program that gives one answer
on the desk and another on the board is exactly what Zeno is for
avoiding. The speed given up is small, and code that needs the fused
instruction can ask for it by name when Zeno has a function for it.

**Where it is said.** In the C itself (two `#pragma` lines at the top
of the runtime), not in a flag that `zeno build` passes. A firmware
build is usually compiled by the board's own toolchain from the C
that Zeno wrote, with flags Zeno never sees. What is written in the
file goes wherever the file goes.

**A limit of 0.1.** clang on 32-bit x86 has no such line to be given,
and on the systems where it is set up for processors older than 2001
(Debian and Ubuntu) it needs `-msse2`. gcc on the same machine is
told by the file.

### 12.6 A compiled program on Windows prints `\n`

On Windows a C program that prints `"\n"` writes `"\r\n"`, unless it
asks for its bytes to be left alone. `zeno run` writes `"\n"` on
every system. So a compiled program asks, before `main` starts.

**Why.** The same promise as 12.5. A program whose output is
compared, counted or sent down a wire should give the same bytes
however it was run. A Windows terminal shows both the same way.

**Reading is unchanged.** A program in a file with Windows line ends
runs (the lexer skips `"\r"`), `lines()` and `input()` take a
`"\r\n"` as one line end, and `zeno format` writes `"\n"`, so a file
has one layout on every system.

### 12.7 What Windows does not have yet

`zeno.exe` is checked like every other build, and `zeno build` makes
programs that print what `zeno run` prints, through C with MinGW's
gcc and through LLVM with the clang that comes with Visual Studio
(three examples are compared each way). These are the limits:

- `zeno build` needs a C compiler that takes gcc's options.
  Microsoft's own compiler (`cl`) takes different ones and has not
  been tried.
- A path in a Zeno program or a `zeno.package` is text, where `\`
  starts an escape, so it is written with `/`, which Windows accepts.
- What a compiled program prints to a Windows console has not been
  looked at. The bytes are UTF-8, as they are into a file or another
  program, where they are the same as `zeno run` gives. A console set
  to an older code page may show a word in Tamil wrongly. No machine
  at hand has a Windows console to look at, and a mend that cannot be
  tried is not one to put in.

The first release (0.1.0) had two more, both mended since: 12.8 and
12.9.

### 12.8 A compiled program on Windows asks Windows

The first trial on Windows found that `zeno build --llvm` did not
work there. Four things were in the way, and each is a place where C
on Windows is not C elsewhere.

**Waiting and the clock.** The runtime used `nanosleep` and
`clock_gettime`. Linux, macOS and MinGW have them. Microsoft's C
library does not. On Windows the runtime now calls `Sleep` and the
performance counter, which every C compiler there can reach.

**`setjmp`.** A `try` is built on it. Microsoft's takes a second
thing, the frame of the calling function, which its `longjmp` uses
to run clean-up code on the way back. A C compiler adds it unasked,
because in C `setjmp` is a macro. LLVM IR has no macros, so on
Windows the IR that Zeno writes passes `null` there, which asks for
a plain jump, as on every other system.

**`-lm` and `.exe`.** The maths library is part of the C library on
Windows, and Visual Studio's linker stops when asked for it. And a
program there is called `hello.exe`, which gcc adds and clang does
not. Zeno leaves out the first and writes the second.

**Arguments.** `main` is given its words through the machine's code
page, so a word in Tamil arrived as `?????`. The runtime asks Windows
for the command line as Windows keeps it and turns each word into
UTF-8, in a buffer on the stack: a build with no collector has no
heap on Windows either.

**Why this way.** One runtime, with the Windows calls behind
`#ifdef _WIN32`, for both compilers. The three functions are declared
by hand in the runtime and `<windows.h>` is not read: it is very
large and defines thousands of names, `min` and `max` among them,
that a program's own names could meet.

**Turned down.** Using MinGW's `nanosleep` where there is one and
`Sleep` otherwise: two ways to wait on one system, and only one of
them tried by any given build. Telling people to use MinGW's clang:
the clang most Windows machines have is the one Visual Studio brings.

**Tried how.** Here, with a Windows compiler and an emulator. Then
on a Windows machine (12.10), where every test of Zeno now passes.

### 12.9 A path Zeno makes is written with `/`

`import drivers.uart` reads `drivers/uart.zeno`. On Windows that was
`drivers\uart.zeno`, since Windows puts `\` between the parts of a
path, and takes `/` as well.

**The decision.** Every path Zeno puts together is written with `/`,
on every system: in a message, and wherever a path is kept as text
(`modules::shown`). A path the person typed is shown as they typed
it.

**Why.** Three things went wrong with two ways. A path in a
`zeno.package` is written with `/` (12.7), so joined the Windows way
it came out as `lib/filters\zeno.package`. An editor names a file
with `/` on every system, so the file it has open and the file an
`import` asks for were two names for one file, and the editor's text
was not used. And every message with a path in it had two spellings,
one of which no test saw. It is the reasoning of 12.6: one way, on
every system.

**Turned down.** Writing `\` throughout on Windows. It is what
Windows people are used to, but the `zeno.package` would have to be
turned round too, and `\` in Zeno text is an escape.

**With it.** `zeno fetch` tells git to leave line ends alone, so a
fetched package is the same bytes on every machine. Git on Windows is
usually set to write `\r\n`.

### 12.10 A trial on one machine

`.github/workflows/trial.yml` builds Zeno on one kind of machine,
chosen when it is started, compiles three examples through C and
through LLVM, and runs every test. It writes what it found on the
summary page of the run. Nothing is packed or released.

**Why.** A mend for Windows can only be seen to work on Windows. The
release workflow shows it too, on ten machines, and a month of
GitHub's free time is about twenty such runs. A trial on Windows
costs a quarter of one, and on Linux less.

---

## 13. What writing the book found

The Zeno Book runs every sample it prints through the real compiler.
Writing it was the first time the whole language was used by somebody
who was trying to explain it and not to build it, and it found places
where the checker let through what the rest of Zeno then refused, or
said less than it knew. Each is a place where a program was accepted
and then failed, which is the thing the checker is for preventing.

### 13.1 `zeno check --no-collector` says what the build would say

`var readings: list[int or none] = [1, none, 3]` passed `zeno check
--no-collector` and was refused by `zeno build --no-collector`.

Most of what a build with no collector refuses, the checker finds. A
few things only the compiler meets, as it works out where each value
is kept. `check --no-collector` now runs the compiler as well and
throws the C away.

**Why.** "No errors found" from a command named after a build has to
mean that the build goes through. Otherwise the command answers a
question nobody asked.

**Turned down.** Teaching the checker each of the compiler's
refusals: two lists of the same rules, which would drift apart again.

### 13.2 `none` is tested for with `is`, and only where it can be `none`

```zeno
let x: int or none = find(xs, 3)
if x is none { ... }       # the way to ask
if x == none { ... }       # refused: write `is none`
let n = 3
if n is none { ... }       # refused: an int is never `none`
```

Both were accepted before. `x == none` gave the same answer as `x is
none`, so there were two ways to ask one question. `n is none` gave
`false` every time, which is a mistake that looks like a test.

**Why `is` only.** `==` compares two values, and `none` is the lack
of one. Decision 1.13 says absence is tested with `is none`, and the
parser already refuses `is` in front of anything but `none`. The
other half belongs with it: `none` is tested with `is` and nothing
else. Two values that may each be `none`
are still compared with `==`, because that compares values.

**Why `n is none` is an error.** It is the rule of 2.5a: a comparison
whose answer is already known is a mistake.

**A name that was narrowed may be tested again.** After `if x is not
none`, or after `x = 3`, the checker knows `x` is there. `x is none`
is then also always false, but it is not refused: the name was
declared `int or none`, and a second test is caution, not a mistake.
The rule looks at the type the name was declared with.

### 13.3 A field of an enum case is taken out with `match`

```zeno
enum Shape {
    Circle(radius: float)
    Square(side: float)
}
let shape = Circle(2.0)
print(shape.radius)        # refused
```

This was accepted outside strict mode and failed when the program
ran, if the value was a `Square`. It is refused now, in every mode,
and the message writes the `match` out:

```
`radius` belongs to the case `Circle`, and which case a `Shape` is, is
only known inside a `match`. Take it out there:
`match ... { case Circle(radius) { ... } }`
```

**Why.** An enum value is one of its cases, and `match` is where the
program finds out which. A field read outside it is a guess about the
case that nothing checks. There is one way to open an enum value now,
and it is the one that cannot be wrong.

### 13.4 A `const` is written from what the compiler can work out

Decision 1.7 says a `const` is fixed when the program is built. The
checker treated it as a `let`, so `const N = len(name)` and `const P
= k` for a `var k` were accepted.

A `const` is now written from: numbers, text, `true` and `false`,
quantities, other `const`s, a case of an enum that carries nothing, a
constant of a module (`math.pi`), operators on those, a list or a
dict of them, and the conversions between kinds of number (`u8(n)`,
`u8.wrap(n)`, `int(x)`, `float(n)`). Anything else is refused, with
`let` named as the word for a value worked out once while the program
runs.

**Why a list, when the list can still be added to (3.9).** A table of
numbers is the commonest `const` in firmware, and what `const`
promises is where the value comes from, not that it never changes.
A list nobody may change is still the open question of 3.9.

**Why no calls.** `len("abc")` could be worked out by the compiler,
and so could any function of constants, with enough work. The rule
that fits in a sentence is "no calls, except to change the kind of a
number", and nothing has needed more.

### 13.5 A parameter with a default comes after those without

`function f(a = 1, b)` is refused where it is declared.

**Why.** A call gives its arguments in order, so `f(2)` gives `a` and
leaves `b` without a value: the default of `a` could only ever be
used by naming every argument after it. It is the rule of Python and
C++, and for the same reason.

**Not for the fields of a class.** A class lists what it holds in the
order it holds it, and objects with several fields are usually made
with names. `class P { x: int = 0  y: int }` stays.

**One mistake, one message.** `f(b=5, 1)` used to give three errors:
the order, "`b` was given twice", and "needs a value for `a`". The
second and third came from counting a plain argument that was in the
wrong place. After the first, nothing more is said about that call.

### 13.6 Messages that say what Zeno writes

- `None`, `null`, `nil`, `NULL`, `nullptr` and `undefined` are "not
  defined. Zeno spells it `none`", and `True` and `False` the same
  for `true` and `false`. A program may still use those names for
  things of its own: the hint is only given when the name means
  nothing.
- `elif` at the start of a line gets "Zeno spells `elif` as `else
  if`". It did only after a `}` on the same line.
- A whole number where a float goes: "Write a number with a decimal
  point, as in `1.0`, and convert a value with `float( )`". Before,
  only the first half was said, also when the number was in a
  variable and had no decimal point to add.
- `min`, `max` and `sum` on quantities say that they work on plain
  numbers, and that quantities are compared with `<` and `>` and
  added with `+`. They do not take quantities yet (a limit of 0.1).

### 13.7 A whole quantity prints without `.0`

`print(3.0)` is `3.0`, and `print(3 V)` is `3 V`, although a quantity
is a float inside.

**Settled as it is.** The `.0` of a float is there to say "this is a
float and not an int", because both exist and behave differently. A
quantity has only one kind, so there is nothing to tell apart, and
`3 V` is how a person writes three volts. A format gives the decimals
when they are wanted: `f"{supply:.1f}"` is `3.0 V` (10.29).

### 13.8 Found and left for later

(Three more were on this list at first, and are in 13.9. The three
below were taken up after the release: chapter 17.)

- For `let x: str = first([1])` the error is reported at the
  argument (`items` should be `list[str]`), not at `x`. A list that
  is written out waits for the place to say what kind of number it
  holds (3.16b, so that `[1, 2]` can be a `list[u8]`), and here the
  place said `str`. The message is true and in the wrong order.
- `Box([])` outside strict mode has a `T` that nothing says, and
  becomes a value of unknown type without a word. Strict mode asks
  for the type.
- `min`, `max` and `sum` do not take quantities. The message says
  what does (13.6).

### 13.9 Three messages that were left, and then were not

13.8 first listed five things. Three were mended a few hours later,
once the release was no longer waiting on them.

**A field that may be `none`.** After `if user.name is not none {`,
`user.name + "!"` is still refused: a field belongs to an object that
any call may reach and change, so a test of it is not remembered
(3.12 is about what a test says of a variable). The message said
"Check for `none` first", to somebody who just had. Now it says what
to do:

```
`name` is a field, and a field can be given a new value between a
test and its use, so a test of it is not remembered. Copy it into a
variable with `let`, and test that for `none`
```

**Why not remember the test.** It would be right only until the
first call between the test and the use, and the checker would have
to prove there is none. A variable of the function's own cannot be
changed by anybody else, which is the whole reason a test of one is
remembered.

**Two enums with a case of one name.** `Dot` in `Shape` and `Dot` in
`Mark` gave "`Dot` is already declared in this block", which is true
of a `let` and says nothing about enums. Now: "`Dot` is already a
case of the enum `Shape`. A case is written by its name alone, so two
enums in one file cannot both have a `Dot`".

**A method without `self`.** It was shown on the line of the class,
with every other mistake in a method's first line. A function in the
syntax tree now knows where it is written (`ast::Function::span`),
and the mistake is shown on the method's own line.

---

## 14. A program in a web page

Zeno was made for firmware, and is not a language for the web. But a
language somebody is learning to build should be able to show
something, and the thing everybody can open is a page. The question
of this chapter is what the least is that makes a page written in
Zeno, without a second Zeno to keep in step with the first.
[`web.md`](web.md) is the walkthrough.

### 14.1 The interpreter runs in the page, as WebAssembly

`zeno build --web` compiles nothing. The page it writes carries
`zeno.wasm`, which is Zeno itself (lexer, parser, checker,
interpreter) compiled by Rust for a browser, and the program as the
text that was written. The browser runs the program as `zeno run`
does.

**Why.** Every other way is a new engine. Zeno already has five that
must agree (the interpreter, the VM, C, LLVM, and each of the last
two in its firmware forms), and most of the work of every stage
since the first has been holding them to one answer. This way adds
no engine. A program in a page means what it means on a desk because
the same code decides it, and the whole language is there on the
first day: classes, generics, quantities, modules, packages.

**What it costs.** `zeno.wasm` is 1.1 MB and every page carries it.
A program runs at the speed of `zeno run`. Both are fine for a page
that waits for a click.

**Turned down.**
- *Compile Zeno to JavaScript.* A sixth engine, and JavaScript's
  numbers are not Zeno's: no 64-bit whole number without `BigInt`, so
  `int` would be slow or wrong.
- *Compile through C, and C to WebAssembly.* It reuses `emit_c`, and
  it needs clang with a WebAssembly target or Emscripten on the
  machine of everybody who builds a page. Compiled Zeno is also
  strict Zeno (the builds refuse some of what `zeno run` runs), so a
  page would have been a smaller language than a desk.
- *Emit WebAssembly from `emit_llvm`.* The same, with LLVM in place
  of C.

One of the last two is the way to a page that is small and fast, if
that is ever wanted. Nothing here is in its way: the `web` module
would be the same.

### 14.2 The page is a module, and an element is a value

`import web`, then `web.element("#count")` gives an `Element`, which
has fields and methods. It is the shape of `board` and `Pin`
(3.15): the outside world is a module, and a thing in it is a value
of a type the checker knows.

**Why.** It was already decided once, and a second answer to the
same question would be the ambiguity the language is trying not to
have. It also means the checker checks a page: `count.text = 3` is
refused before anything runs.

**Turned down.**
- *The browser's own names* (`document.querySelector`,
  `addEventListener`, `classList.add`). Familiar to those who know
  them, and three ways of spelling for those who do not.
- *A way to call any JavaScript* (`js.call("alert", ...)`). It makes
  everything possible and nothing checked, and under `zeno run`
  there is no JavaScript to call.
- *Describing the page in Zeno* and letting a library work out what
  to change, as the big front-end libraries do. That is a library to
  write in Zeno one day, on top of this.

### 14.3 An element that is not there is an error

`web.element("#count")` raises a `ValueError` if the page has no
such element. `web.elements("li")` gives a list, which may be empty.

**Why.** The usual cause is a misspelt id, and the first run finds
it. Giving `Element or none` would make every program test for
`none` around every element it uses, for the sake of a mistake that
shows itself. `pin(64)` on a board with 64 pins is an error for the
same reason.

**Turned down.** `Element or none` (above). A silent element that
swallows what is done to it, which hides the misspelling.

### 14.4 `text` and `value` are fields. The rest are methods

`count.text = "3"` and `box.value` are read and written as fields.
Everything else (`add_class`, `hide`, `append`, `on`) is a method.

**Why.** A field is something an object has, and a method is
something it does. The text of an element and what has been typed
into it are had: a program reads them and writes them, and
`count.text += "!"` works with no new rule. A class is added and
taken away, which is doing.

**Turned down.** `set_text("3")` and `get_text()`: two names for
what `=` says. Fields for everything (`item.hidden = true`,
`item.classes.append("done")`): a list of classes that the program
changes would have to be watched for changes.

### 14.5 A function the page calls takes nothing

`button.on("click", pressed)` calls `pressed()`. `web.after(500 ms,
tick)` calls `tick()`. Only `web.on_key(pressed)` hands something
over: the key, as a str. The checker says so, in its own words, if
the function takes anything else.

**Why.** An event object is mostly a way to find out which element
it happened to, and a Zeno function knows that already: it is
written where the element is in reach, and keeps it (5.5k is how a
function does). What is left is what cannot be known any other way,
and of that only the key was needed to write a page. So that is all
there is, until a program needs more.

**Turned down.** An event object with fields (`event.key`,
`event.x`, `event.target`): a new type, most of whose fields mean
nothing for most events.

### 14.6 On a desk the page is simulated, and calls nothing

Under `zeno run` and `zeno test` there is a simulated page
(`src/page.rs`). It has whatever element is asked for, keeps what is
done to it, and prints a line for each change that could be seen,
as the simulated board prints a line when a pin changes (3.15b). It
never calls a function it was given.

**Why.** A program for a page should be testable where every other
program is, in the same `zeno test`, with no browser installed. And
the two engines are held to one output for a page as for everything
else. Calling nothing is the honest version of "nobody is here": a
test calls the function it wants to test, by its name.

A selector the simulated page cannot read (`a[href]`, `ul > li`) is
an error in `web.elements`, and not an empty list. An empty list
would say the page has none, and the simulated page does not know.

**Turned down.**
- *Refuse to run a program that imports `web`.* Then nothing of it
  could be tested without a browser.
- *Read the HTML file,* to know which elements there are. It would
  find the misspelt id, which is worth having. It needs an HTML
  parser, which is larger than everything else in this stage. Left
  for later (14.13).
- *Call each function once,* to see that it runs. A click that
  nobody made is not something the program can explain.

### 14.7 A built page is one file

`todo.web.html` holds the page, the program, `web.js`, and
`zeno.wasm` written as base64.

**Why.** A page opened from the disk may not load another file: the
browser refuses, to keep a page from reading the disk. With three
files, the first thing a learner would see is a blank page, and the
fix is to run a web server. One file opens with a double click, and
is hosted by copying it.

**What it costs.** Base64 makes `zeno.wasm` a third larger (1.5 MB),
and two pages on one site each carry their own.

**Turned down.** Three files and a small web server in `zeno`
(`zeno serve`): a server is a lot to add to make a file open.

### 14.8 `todo.html` is written, and `todo.web.html` is built

The page for `todo.zeno` is `todo.html`, beside it. If there is
none, the page is an empty one with a place for what the program
prints. The built page is `todo.web.html`, in the folder the command
is run in, where every build puts what it makes.

**Why.** The file somebody writes has the plain name, as
`hello.zeno` does, and what is built is named after it. They cannot
be the same name: built in the program's own folder, the page would
be written over its own source.

The scripts go at the end of the body, so that every element is
there when the program's first line runs.

**Turned down.** A name for the page given on the command line, and
an `-o` for where the result goes: two more things to type, for a
choice with one sensible answer. A line in the HTML that names the
program (`<script src="todo.zeno">`): then the page would say which
program it has, the command would say which program to build, and
they could disagree.

### 14.9 The program travels as its text, named from its own folder

The page carries every file of the program as it was written, under
the name it has from the program's folder: `todo.zeno`,
`drivers/uart.zeno`, `../filters/zeno.package`. The browser reads,
joins and checks them as a desk does.

**Why the text.** It is what `zeno.wasm` already knows how to read,
and an error in the page can show the line it is on. A syntax tree
written out and read back would be a new format to keep right.

**Why from its folder.** `zeno build --web /home/partha/site/todo.zeno`
would otherwise write `/home/partha/site/` into a page that is then
put on the web. The names are also the ones the browser's reading of
the program asks for, so nothing has to translate them.

`<` in the program is written `<` in the page. A browser ends
a script at `</script>` wherever it finds one, even in a str.

### 14.10 An error that nothing catches stops the page's program

It is shown in a box at the foot of the page, with its line, and in
the console. After it, the page calls no more of the program's
functions.

**Why.** It is what `zeno run` does, and for the reason it does it:
the function that failed left its work half done, and whatever runs
next runs on that. A page that goes on after an error is a page
that is wrong in a way nobody is shown.

**Turned down.** Only the console: a learner does not have it open.
Going on with the next click: above.

### 14.11 No `sleep` in a page, and `input` is the browser's question

`time.sleep` in a page stops the program with a message that says
to use `web.after`. `input("Name: ")` asks with the browser's own
question box.

**Why.** A page has one thread. A program that waits on it stops
the page from drawing, and a `sleep` in a loop, which is what
`blink.zeno` is, would hang the tab. `input` can work, because the
browser has a question box that waits; it is ugly, and it lets every
program that asks for a name run in a page.

### 14.12 200 nested calls in a page

On a desk a program may nest 1000 calls. In a page, 200.

**Why.** The interpreter calls itself for each nested call, on the
browser's stack. Chromium gave out between 700 and 1000 calls of a
plain function, and between 400 and 500 of one that calls itself
from inside a loop, a `match` and a `map`. When the browser stops a
program, Zeno cannot say which line it was. 200 is where it always
still can, with room for a browser whose stack is half the size.

### 14.13 `--web` is a build of its own

`zeno build --web` goes with none of `--llvm`, `--firmware`,
`--no-collector` and `--memory`, and says so. `zeno build` without
it refuses `import web`, and says to use `--web`.

**Why.** Those options say how to compile, and a page is not
compiled. A page built with `--firmware` would be a page that is no
different, from a command that suggested it was.

### 14.14 Left for later

- The simulated page does not read the HTML, so it does not find a
  misspelt id (14.6).
- Events carry nothing but the key (14.5): no place of the mouse.
- No style of one element, no drawing, no network, nothing kept
  between visits. Each is a name in `web.js` and a few lines in
  `page.rs`.
- Only Chromium has run it.
- A compiled page, small and fast (14.1).

### 14.15 This is version 0.2

`Cargo.toml` says 0.2.0 from the commit that brought the `web` module
into `main`, which was the first commit after 0.1.0 was published.

**Why 0.2 and not 0.1.1.** It adds a module, a type and a build. A
third number is for a mend, and nothing here mends anything.

**Why not 1.0.** Nothing that ran under 0.1 changes, so nothing was
learnt about whether the language holds up. 1.0 should mean that
programs written by somebody else have been through it.

---

## 15. Installing

A release is ten archives (chapter 12). This chapter is about not
having to know which one is yours. [`install.md`](install.md) is the
walkthrough.

### 15.1 Two installers: a setup program on Windows, a line elsewhere

Windows gets `zeno-X.Y.Z-windows-setup.exe`. macOS and Linux get a
script, `install.sh`, that one line downloads and runs.

**Why two.** Each is what its system's users already do. On Windows a
program arrives as a setup `.exe` with Next buttons, and a line to
paste into a terminal is foreign. On macOS and Linux the people who
would install a compiler have a terminal open, and the line is how
Rust, Homebrew and Deno arrive.

**Turned down.**
- *A setup program for macOS too* (`.pkg`). macOS refuses an unsigned
  one more firmly than it refuses a script, and signing wants a paid
  Apple account.
- *A script for Windows too* (PowerShell). The setup program does more
  (an uninstaller in Windows' own list, a Start menu entry), and two
  ways to install on one system is one too many.
- *Packages* for Homebrew, apt, winget and the rest. They are the best
  answer in the end, and each has a place where it must be published
  and kept current. Not yet (15.7).

### 15.2 For one user, with no administrator

The setup program installs into `%LOCALAPPDATA%\Programs\Zeno`, the
script into `~/.zeno`, and each changes that user's PATH only.

**Why.** Nothing about Zeno needs the whole machine, and a password
prompt is a reason to stop. It also means a student on a machine that
is not theirs can install it.

**Turned down.** `C:\Program Files` and `/usr/local/bin`: an
administrator for no gain. A choice between the two on a page of the
setup program: a question most people cannot answer.

### 15.3 The PATH is added, and the box is ticked

Both installers put Zeno's folder on the PATH. In the setup program it
is a box, already ticked. The script just does it, and says which file
it changed.

**Why.** A `zeno` that the terminal cannot find is not installed in
any sense a beginner means. Python's installer leaves its box
unticked, and the result is the best-known first error in Python.

The script adds one line to one startup file, the one for the shell
in `SHELL`, and ends the line with a comment that names the
installer. That comment is how it knows not to add the line twice and
how `--uninstall` finds it again.

### 15.4 The tools Zeno hands work to are offered, never assumed

If there is no C compiler, or no git, the installer says which command
needs it and offers to install it: with winget on Windows, with the
system's package manager on Linux, with Apple's command line tools on
macOS. On Windows the box is ticked, and can be unticked. In the
script it is a question, and with nobody to ask (no terminal), the
answer is no and the command is printed.

**Why offer.** Partha asked for an installer "which installs whatever
needed", and without a C compiler `zeno build`, which is what Zeno is
for, does not work.

**Why not just install.** A compiler is hundreds of megabytes, and
on Linux it is installed as root. Doing that unasked, from a script
piped out of curl, is the thing people are right to fear about such
scripts. And everything but `build` and `fetch` works without.

**Why not carry a compiler inside the installer.** A small one exists
(TinyCC), and Zeno's C has not been tried with it. gcc is too large to
carry and has to be kept current. The system's package manager
already does both.

### 15.5 `zeno build` looks for `cc`, then `gcc`, then `clang`

With no `CC`, the first of the three that is installed compiles the C.

**Why.** Windows has no `cc`, so every Windows user had to learn `set
CC=gcc` before the first build. An installer that installs gcc and
then needs a setting to use it has not finished its job. The order
puts the system's own choice first.

### 15.6 Not signed

Neither the setup program nor `zeno` is signed. Windows shows a
warning the first time, and the walkthrough says what it is.

**Why.** A certificate for Windows costs a few hundred dollars a
year, and Apple's needs a paid developer account. The goal of Zeno is
to learn by building it. If strangers start to install it, this is
the first thing to change.

### 15.7 One setup program for three processors

The setup program holds the x86-64, the x86 and the ARM64 `zeno.exe`
and installs the one for the machine.

**Why.** Three setup programs would bring back the question the
installer exists to remove. The cost is a file a few megabytes
larger.

### 15.8 The script carries its version, and the release checks it

`install.sh` has `version=0.2.0` written in it, and is a file of the
release of that version. `tools/release-check.sh` refuses a release in
which that line and `Cargo.toml` differ.

**Why.** The script has to know which files to ask for, and their
names have the version in them. Asking GitHub which release is the
newest needs a second request and a way to read its answer, in a
script that should need nothing but `curl` and `tar`. A plain line
that a check keeps true is less to go wrong.

### 15.9 The download is checked before it is unpacked

The script compares the archive's SHA-256 with the line for it in
`SHA256SUMS.txt`, and stops if they differ or if there is no way to
compute one.

**Why.** A download that was cut short unpacks into a `zeno` that
fails in some way nobody can explain. The check does not protect
against somebody who can change the release itself, since they would
change both files. It protects against everything less than that.

### 15.10 Left for later

- Signing (15.6).
- Packages for package managers came next: 15.12.
- The winget step of the setup program is not run by any test:
  GitHub's Windows machines have no winget.
- `zeno` does not update itself. The installer, run again, does.

### 15.11 An earlier release gets installers added, and nothing in it changed

Zeno 0.1.0 was released before the installers. A workflow,
`add-installers.yml`, makes them for it from the files of that
release: the setup program from the three `zeno.exe` that were
released, and `install.sh` with `version=0.1.0` in it. It adds the two
files to the release, and two lines to the list of checksums.

**Why.** Somebody who has an archive of 0.1.0 and its checksum must
find the same file and the same checksum there tomorrow. So a release
is added to and never changed. Making the installers from the released
files, and not from the source of that day, means there is one
`zeno 0.1.0` and not two that should be the same.

**Turned down.** *A 0.1.1 that differs only by having installers*: a
new version says that Zeno changed, and it did not. *Building 0.1.0
again from its tag*: a second build is not the first one byte for
byte, so the setup program would install a `zeno` that nobody checked.
*Leaving 0.1.0 as it was*: it is the only release there is until 0.2.0
is published.

The installers are today's, so they had to allow for what 0.1.0 lacks:
no `zeno.wasm`, and a `zeno build` that knows only `cc` (15.5). The
setup program leaves the first out and says the second in the box that
offers a compiler.

### 15.12 Packages for six package managers, made from the files of a release

`tools/package.sh` makes a package of Zeno for apt (a `.deb`), pacman,
snap and flatpak, and writes the recipe that Homebrew and winget read.
Each holds the `zeno` of a release's archive. Nothing is built for a
package.

**Why.** People who use a package manager want to use it for
everything: it knows what is installed, updates it and removes it
cleanly. And each of these is the usual way to install a program on
some system that Zeno runs on. Making them from the released files,
as the installers are (15.11), means one `zeno` for each processor and
not seven that should be the same. It also means the script can be
tried on a release that exists, which is how it was tried.

**Turned down.** *Building in each manager's own way* (a Debian source
package, `cargo build` in the PKGBUILD and the formula, snapcraft, a
flatpak manifest): each would compile Zeno again, in a way that
nothing else here checks, and four of the six would need Rust on the
machine that installs. *Only the two or three most used*: all six were
asked for, and after the first, each was an afternoon and not a week.

`docs/packages.md` walks through it.

### 15.13 The `.deb` recommends a C compiler and git

Its `Recommends` line is `gcc | c-compiler, libc6-dev | libc-dev,
git`, and `clang` is in `Suggests`.

**Why.** apt installs what a package recommends unless it is told not
to, and that is 15.4 in apt's own words: what Zeno hands work to is
offered, and somebody who does not want it says so. `Depends` would
force a compiler on a machine that only runs `zeno run`. `gcc |
c-compiler` leaves a machine that has clang alone.

pacman has only `optdepends`, which it shows and does not install.
Nearly every Arch machine that builds anything has both programs.

### 15.14 The `.deb` for `armhf` holds the build for ARMv6

**Why.** Debian's `armhf` is ARMv7, and Raspberry Pi OS gives the same
name to a Pi 1 and a Pi Zero, which are ARMv6. A package cannot ask
which. The ARMv6 build runs on both. The ARMv7 build would stop on a
Pi Zero with "Illegal instruction", which says nothing to anybody.
What is lost is a little speed on ARMv7, in an interpreter that was
never the fast way to run a program.

The snap for `armhf` holds the ARMv7 build: Ubuntu, where snaps are at
home, has no ARMv6.

### 15.15 Where a package puts Zeno

`/usr/lib/zeno/zeno` and `zeno.wasm` beside it, a link at
`/usr/bin/zeno`, the examples and the editor setup in
`/usr/share/zeno`, the README and the docs in `/usr/share/doc/zeno`.

**Why.** `zeno build --web` looks for `zeno.wasm` beside the program
(`docs/web.md`), and `/usr/bin` is for programs only. A link keeps both rules:
Linux tells a program where it really is, so Zeno finds its
neighbour. The examples are apart from the docs because small systems,
containers above all, are often set up to leave `/usr/share/doc` out,
and the first thing anybody is told to do is run an example. The first
trial of the `.deb`, in a container, found exactly that.

### 15.16 The snap is `classic`

**Why.** A snap is otherwise shut in, and a compiler cannot work shut
in. It reads a program from wherever it is kept, and runs the C
compiler of the machine. The strict kind can be given the home folder
and nothing more. `classic` has a price, which is that the Snap Store
gives it only on request (15.21).

### 15.17 The flatpak runs on the Sdk

Its runtime is `org.freedesktop.Sdk`, and it is given the home folder
and the network.

**Why.** A flatpak has no `classic`: it cannot see the machine's
compiler at all. So either `zeno build` does not work in it, or the
compiler is in the box. The Sdk is the runtime that programs are built
on, and it has gcc and git. With it, everything Zeno does works in the
box with nothing else to install, which is the whole point of the
installers (15.4).

**Turned down.** *The Platform runtime*, which is a third of the size:
then `zeno build` says there is no C compiler, to somebody who can see
one on their machine. *Running the machine's compiler from inside the
box* (`flatpak-spawn --host`): it needs the box opened so far that it
is not a box.

Flatpak is made for programs with a window, and is the poorest fit of
the six. It is here because it was asked for and because it works.

### 15.18 The recipes are kept in this repository, and written after a release

`Formula/zeno.rb`, `packaging/arch/PKGBUILD` and `packaging/winget`
are written by `tools/package.sh` and pushed to `main` by the last
step of a release (`tools/keep-recipes.sh`).

**Why here.** Homebrew reads a formula from a git repository and from
nowhere else. Keeping it in this one means there is nothing more to
make: `brew tap` is told the address. A repository of its own, named
`homebrew-zeno`, would make the command shorter, and is one of the
steps left for later.

**Why after.** A recipe holds the checksums of the files it downloads,
so it cannot be written before they exist. The alternative, writing it
by hand after each release, is the kind of step that is forgotten.

The commit is made in Partha's name, as every commit here is.

### 15.19 The MIT licence, from 0.2.0

Zeno is under the MIT licence. The file `LICENSE` is in the
repository, and from 0.2.0 it is in every archive, in what the setup
program installs, and in every package. The recipes and the packages
name it.

**Why MIT.** Partha chose it, on 16 November 2026. It is the shortest
of the common licences and the one most people have already read: use
it, change it, pass it on, and keep the notice. That suits a language
that was built to be learnt from. The stores of 15.21 each ask what
licence a program has, which is why the question came up when it did.

**Turned down.** *MIT or Apache-2.0, as Rust has*: Apache adds words
about patents, which matter to a company and are two more pages to
read. *GPL*: a compiler under it is fine, but the runtime that every
compiled program carries (`runtime.c`) would then need an exception
written for it, or every program built with `zeno build` would be
bound by it. *None*: then nobody may lawfully do anything with Zeno,
which is not what a language for learning wants.

**0.1.0 is left as it was.** It was released the day before, with no
licence in it, and a release is added to and never changed (15.11).
So `tools/package.sh` asks the archive: one that has `LICENSE` in it
gets packages that say MIT and carry the file, and one that has not
gets packages that say no licence was chosen. Each says what was true
of that release.

### 15.20 Run by winget, the setup program installs Zeno and nothing else

The winget files pass `/MERGETASKS="!compiler,!git"`.

**Why.** The setup program's two boxes install a compiler and Git by
running winget (15.4). winget runs the setup program without showing
it, so the boxes would be ticked with nobody asked, and a second
winget would start inside the first. Somebody who installs with winget
installs those two with winget.

### 15.21 What packages do not do yet

- **Only Homebrew finds Zeno with no file.** The others are ready up
  to an account, a key or a pull request, each of which is in Partha's
  name and so is his to do (15.22 to 15.28, and `docs/stores.md`).
- The winget files are written and not run: GitHub's Windows machines
  have no winget.
- The packages for processors other than x86-64 are made and not
  installed by a test.
- No `.rpm`, Nix, Scoop or Chocolatey.

### 15.22 The apt repository is the release itself

**Decision.** What apt reads to find the `.deb` files, a list of them
and the list signed, are four more files of each release
(`tools/apt-repo.sh`). apt is given the address that the newest
release's files are downloaded from, as a flat repository.

**Why.** apt needs only three things at one address: `InRelease`,
`Packages` and the `.deb` files, and a release is an address that has
its `.deb` files already. So nothing is hosted and nothing is copied,
and the list cannot fall behind the release or point at a file that is
gone. It was tried: apt reads it, checks the signature and installs.

**What it costs.** apt sees the newest version only, which is what
`apt upgrade` wants. And once people have added the address, every
release must have the four files, or their `apt update` complains. So
the release workflow makes them in the same step that publishes.

**Turned down.**
- *A tree of folders on zeno-lang.org* (`dists/stable/main/...`), the
  usual shape. It is a second place that has to be built and deployed
  for each release, by another repository, and it either copies the
  `.deb` files or redirects each one back to the release.
- *A PPA on Launchpad.* It builds from source on Launchpad's machines,
  from a Debian source package that Zeno does not have, and serves
  Ubuntu only: no Debian, no Raspberry Pi OS.
- *A service that hosts apt repositories.* One more account, and a
  company to depend on, for four small files.

### 15.23 The key that signs for apt is a secret of the repository

**Decision.** The private half of the key is the secret
`APT_SIGNING_KEY`. It has no passphrase and does not expire. The
public half is a file of the release, `zeno-apt-key.asc`, written from
the private half each time.

**Why.** A workflow has to sign with no person there, and a secret is
the one place a workflow can read and nobody can read back. A key that
expired would stop every machine's `apt update` on a day nobody
remembers. Partha makes the key, on his own machine, because whoever
has it can sign for Zeno.

**Turned down.** A key made by a workflow: it would be gone when the
run ends, or would have to be written somewhere to be kept. Asking
people to trust the address with no key (`trusted=yes`): then anybody
between the release and the machine could hand out another `zeno`.

### 15.24 On Arch the package is `zeno-lang-bin`

**Decision.** `pkgname=zeno-lang-bin`, from the next release on, and
in the AUR from the start. It says `conflicts=('zeno-bin')`.

**Why.** The AUR has a package `zeno` already, a node system for
simulation, which has nothing to do with this. On Arch `foo-bin` means
"`foo`, already built", so `zeno-bin` would have claimed to be that
program. `zeno-lang` is the name the repository and the website have.
The package files of 0.1.0 and 0.2.0 are named `zeno-bin` and stay as
they are (15.11). The new one takes the place of such a one.

**Turned down.** Keeping `zeno-bin` and seeing whether anybody minds:
a name in the AUR is hard to change after people have installed it.

### 15.25 The recipe for the AUR: `.SRCINFO` from `makepkg`, and a licence of its own

**Decision.** `tools/package.sh` has `makepkg --printsrcinfo` write
`.SRCINFO` in the Arch container that makes the package, and it is
kept beside the `PKGBUILD`. `packaging/arch/LICENSE` is 0BSD.

**Why.** The AUR reads `.SRCINFO` and does not run a `PKGBUILD`, so
the two must say the same, and only `makepkg` knows for certain what
a `PKGBUILD` says. The `stores` workflow compares them before it sends
anything. The AUR asks that a recipe has a licence, apart from the
licence of the program, and suggests 0BSD: the recipe is twenty lines
that anybody may copy.

### 15.26 A store is switched on by its secret, in a workflow of its own

**Decision.** The workflow `stores` tries each store with no login,
and with "send" ticked sends to each store that the repository has a
secret for: `APT_SIGNING_KEY`, `AUR_SSH_KEY`,
`SNAPCRAFT_STORE_CREDENTIALS`. A job with no secret says so and does
nothing. A release starts it as its last step. The snaps are uploaded
as `snap pack` made them.

**Why.** Each store needs an account that only Partha can make, and
they will not all be made on one day. So each has to work alone, and
giving the secret is the whole of switching one on. It is a workflow
of its own because a version that is already released has to be sent
too, 0.2.0 first of all, and because a store that is down must not
stop a release.

**Turned down.**
- *Steps in the release workflow only.* Then nothing could be sent
  without a new release.
- *A `snapcraft.yaml`*, which is how most snaps are built. It would be
  a second way of making the same file, and the one way there is has
  been tried.
- *Sending by hand, from a list of commands.* It would be done once
  and forgotten at the next release.

### 15.27 winget: today's form, and a pull request by hand

**Decision.** The three files are in the form 1.28.0, name the schema
they are checked against, and have `ProductCode`: the id of the setup
program with the `_is1` that Inno Setup adds, which is the name
Windows keeps the installed Zeno under. Sending them to Microsoft's
list stays a pull request that Partha opens.

**Why.** The list wants the newest form, and its own schema says what
that is, so the files are checked against it in the `stores` workflow.
`ProductCode` is how winget knows that a Zeno installed by the setup
program is the same one. The pull request is made from his account and
signs Microsoft's terms for contributors in his name, which a secret
in this repository should not do for him.

### 15.28 Zeno is not asked for on Flathub

**Decision.** No request to Flathub. The `.flatpak` stays a file of
each release.

**Why.** Flathub's rules say that programs run in a terminal are not
taken, with an exception only for tools about flatpak itself, and that
everything must be built from source on its own machines. Zeno is run
in a terminal, and its flatpak holds the `zeno` of the release.

**Turned down.** Asking anyway. It would spend the time of the people
who review there on a request their rules already answer.

## 16. A page that does more

Chapter 14 made the least that is a page. Its last section (14.14)
listed what a real front end would miss: where the mouse is, the
style of one element, drawing, the network, something kept between
visits, a simulated page that knows the page, and a smaller file.
This chapter is those, and a playground.
[`web.md`](web.md) and [`playground.md`](playground.md) are the
walkthroughs.

### 16.1 A function given to `on` may ask for the `Event`

`button.on("click", function() { ... })` is called with nothing, as
before. `paper.on("click", function(event: Event) { ... })` is
called with what happened. The checker allows those two shapes and
says so if it is given a third.

**Why.** 14.5 held that a function knows its element already, and
that stays true for most of a page. It is not true of where the
mouse was, and not of which item of a list was clicked when one
function is given to the whole list. Those can be known no other
way, so there has to be a way to be told.

Letting the function ask keeps 14.5 for everybody who needs
nothing: no program that ran under 14.5 changes, and the first
example anyone reads still has `function()`.

**Why by the number of parameters.** The function's first line
already says what it wants. A second method (`on_event`) would be
two names for one thing, and a flag would say twice what the
parameter says once.

**Turned down.** `web.mouse()`, a question the function can ask: it
answers for "now", and a function called a moment after the click,
from the queue (14, `waiting`), would be told where the mouse went
since. Two parameters `(x, y)`: right for the mouse and wrong for
every other event, and no room for the target.

### 16.2 An `Event` has five fields, and they are read only

`kind`, `target`, `x`, `y`, `key`. `x` and `y` are floats, in pixels
from the top left corner of the element that `on` was given to. For
an event with no mouse they are 0, and `key` is `""` for an event
with no key.

**Why these five.** Each is something a program cannot find out any
other way, and each was needed by an example: the sketch needs `x`
and `y`, one function for a whole list needs `target`, a function
given to two events needs `kind`, and `key` is what `on_key` hands
over, for an element that wants its own keys.

**Why from the element's corner.** A program draws on the element
it listens to, and a canvas counts from its own corner. From the
corner of the window, every program would first have to take away
where the element is, which it has no way to ask.

**Why 0 and `""` and not `none`.** 14.5 turned down an event object
because most of its fields mean nothing for most events. `none`
would make every use of `event.x` begin with a test that a click
never fails. A program that asks for `x` of a `"submit"` has made a
mistake that 0 shows as well as anything.

**Why read only.** It says what happened. Changing it would change
nothing on the page, and a field that can be written and does
nothing is a trap.

**Left out for now.** Which button of the mouse, Shift and Ctrl,
the wheel, touches. Each is a field, three files away.

### 16.3 `fire` calls the program's functions, at once

`button.fire("click")` calls every function that `on` was given for
`"click"` on that element, in the order given, before it returns.
One that asks for the `Event` gets one with the element as `target`
and 0 for `x` and `y`. It does the same in a browser and on the
simulated page.

**Why.** 14.6 left a test one way to press a button: call the
function by name. A function written in place
(`item.on("click", function() { ... })`) has no name, so the most
natural way to write a page could not be tested. `fire` is the
press.

**Why it does not send the browser an event.** In a browser an
event that is sent is handled later, after the function that sent
it has returned (the queue, 14). On a desk it would have to be
handled at once, because there is no later. A test that passes on a
desk and means something else in the browser is worse than no test.
So `fire` is the same everywhere: it is a call. It does not tick a
checkbox or send a form, and docs/web.md says so.

**Turned down.** `button.click()`: one method for each event, and
the browser has hundreds.

### 16.4 The style of one element: `set_style` and `get_style`

`e.set_style("color", "red")`, with the name CSS uses.
`e.get_style("color")` gives what was set on the element itself, or
`none`.

**Why at all.** 14 said "use a class", and that is still the right
answer for a look that is known when the page is written. It cannot
do a look that is worked out: a bar as wide as a reading, a colour
the person chose.

**Why CSS's names.** `background-color`, and not `backgroundColor`
or `background_color`. The page beside the program is written in
CSS, and one thing should have one name in both files.

**Why a wrong name is an error.** A browser ignores a style it does
not know, and the program goes on with nothing changed and nothing
said. Zeno stops with a `ValueError`. (The simulated page does not
know CSS and cannot do this. It is found when the page is opened.)

**Why `get_style` is only what was set.** What an element finally
looks like comes from every rule of the page, in units the browser
chooses (`rgb(255, 0, 0)` for `red`). Handing that back would be a
second, bigger thing with the same name.

### 16.5 Drawing is five methods of an element

`draw_rect`, `draw_circle`, `draw_line`, `draw_text`, `erase`.
Places and sizes are numbers (int or float), in pixels from the top
left corner. The colour is last, and is whatever CSS calls a
colour. On an element that is not a `<canvas>` they raise a
`ValueError`.

**Why so few.** A browser's canvas has a pen with a state: a
colour, a width, a path that is begun and ended. Every call depends
on the calls before it, which is exactly what makes drawing code
hard to read and to test. Here each call says everything it needs
and leaves nothing behind, so a line printed by the simulated page
(`#paper draw_rect 10 10 50 20 red`) is the whole of what happened.

**Why the colour is last and not left out.** A default colour is a
state by another name: the reader has to know what it is.

**Why `draw_text` counts from the top left.** A browser counts text
from its baseline, so text drawn at `0, 0` is off the top of the
canvas. Every other shape here starts at its corner.

**Why methods of `Element`, and no `Canvas` type.** A canvas is an
element: it has a class, can be hidden, hears clicks. A second type
would need a way to get from one to the other, for no gain but a
check the page cannot make anyway (the checker does not read the
page).

**Left out for now.** Curves, images, line width, drawing that is
not filled. Games that redraw everything sixty times a second are
too slow for an interpreter in any case (14.1).

### 16.6 `web.every`, beside `web.after`

`web.every(1 s, tick)` calls `tick` that often until the page is
closed. Less than 1 ms is a `ValueError`.

**Why.** A clock, a poll, anything that moves. With `after` alone
it is written as a function that asks for itself again, and every
program that does it gets it slightly wrong (the time drifts by
however long the function takes).

**Why it cannot be stopped.** Stopping needs a value to stop
(`let timer = web.every(...)`, `timer.stop()`), which is a new
type. A function that has nothing to do can return at once, and
that has been enough. It is the first thing to add when it is not.
(It was: chapter 19.)

### 16.7 What is kept between visits: `save`, `load`, `forget`

`web.save("name", text)`, `web.load("name")` (a str, or `none`),
`web.forget("name")`. Names and values are text.

**Why text only.** The browser keeps text. A number is `str(n)` on
the way in and `int(text)` on the way out, where the program can
see it and where a wrong value raises the error it should.

**Why `load` gives `none`.** Nothing kept is the ordinary case, on
every first visit, and the checker then makes the program say what
happens.

**Why a browser that refuses keeps nothing, silently.** A private
window may refuse. The program already has to work on a first
visit, when nothing is kept, so "nothing is ever kept" is a case it
handles already.

**On a desk** what is saved is kept until the run ends, so a test
can save and load. It is not written to the disk: a test should not
depend on the run before it.

### 16.8 The network: `fetch` and `post`, with a function for the answer

`web.fetch(address, done)` and `web.post(address, text, done)`.
`done` is called later with two things: the status (an int: 200,
404) and the body (a str). An address that could not be reached at
all gives status 0 and `""`.

**Why a function for the answer.** The answer comes later, and a
page must not wait (14.11 is the same rule for `sleep`). Zeno has
one way to say "later": hand over a function, as `on` and `after`
do. A second way (a value that stands for an answer to come) is a
feature of a language, with rules of its own, and this is one
module.

**Why two parameters and no `Reply` type.** There are two things to
know. A type with two fields is longer at every use and adds a name
to learn.

**Why status 0 and no error.** An error raised later, inside
nothing the program called, has nowhere to be caught. And "could
not be reached" has to be handled beside 404 and 500 anyway, in the
same `if`.

**Why the body is text.** There is no JSON in the library yet.
When there is, it reads a str.

**On a desk** nothing is fetched and `done` is not called, as a
timer is not (14.6). A test calls `done` itself with the answer it
wants to try, which is the test worth writing: what the page does
with a 404.

### 16.9 The simulated page reads the page that was written

If `todo.html` is beside `todo.zeno`, `zeno run` and `zeno test`
read it, and the simulated page has those elements and no others.
With no such file it has every element that is asked for, as in
14.6.

**Why.** 14.6 said that the simulated page "cannot say whether the
page has the elements the program asks for", and a misspelt id is
the commonest mistake there is in a page. It was found only in a
browser, by hand.

**Why by the file's name.** It is the file `zeno build --web`
already takes as the page (14.8), so the page that is tested is the
page that is built, and there is nothing to configure.

**Why not in the checker.** The checker could look up
`web.element("#list")` when the selector is written out. It would
then need the page, which the language server and `zeno check` do
not have in hand, and it would still miss a selector that is built
(`"#row" + str(n)`). On the first run, the same message comes with
the same line.

**Why a reader of our own (`src/html.rs`).** Zeno has no
dependencies (5.1). The reader is 200 lines, because it reads pages
that are written right and does not mend the others as a browser
must. Text that is only the spaces a page is laid out with counts
as none.

### 16.10 `zeno.wasm` is built for size, where that costs no speed

`.cargo/config.toml` builds it with `opt-level=s`, one code
generation unit and whole-program `lto`. 1.1 MB became 800 KB.

**Why `s` and not `z`.** Measured, on a loop of 300,000 steps and a
recursion: as built before, 173 ms; `s` with the rest, 167 ms; `z`,
262 ms, for 650 KB. A page that is 150 KB smaller and half again as
slow is a bad trade once the page is packed (16.11), when the
difference is 40 KB.

**Why in `.cargo/config.toml`.** It holds for this one target, and
`cargo build --release --target wasm32-unknown-unknown` stays the
command, in the docs and in the release workflow. `zeno` for a desk
is built as before.

### 16.11 `zeno.wasm` is packed in the page, by a packer of our own

A built page holds `zeno.wasm` as DEFLATE in base64, and the
browser unpacks it with `DecompressionStream("deflate-raw")`.
`src/deflate.rs` is the packer. A built page went from 1.5 MB to
0.4 MB.

**Why pack.** Base64 makes bytes a third longer, and `zeno.wasm` is
most of every page. A web server can pack a page as it sends it,
but 14.7 promised one file that works from the disk and from a
mail, where no server is.

**Why DEFLATE.** It is the one format every browser unpacks by
itself, so nothing is added to the page to undo it. A better
format would need its unpacker in every page, in JavaScript.

**Why write it.** No dependencies (5.1), and a release must not need
`gzip` on the machine that builds a page, least of all on Windows.
It is 300 lines, and it comes within two parts in a hundred of
`gzip -9` on `zeno.wasm` (292 KB against 288 KB) in a tenth of a
second. The test unpacks what it packs with Python's zlib, where
there is a Python.

**The cost.** The page needs a browser from 2023 or later. An older
one is told so, in the box where errors go.

### 16.12 A playground, written by `zeno playground`

`zeno playground` writes `playground.html`: a box for a program, a
box for its page, Run, a few examples, and a link that holds the
program. Each run is a new frame.

**Why.** "What does Zeno look like?" should have an answer that is
a link. Everything else needs an install first.

**Why a command, and not a file in the repository.** The page holds
`zeno.wasm`, which is built, and the examples, which change. A file
that is checked in is out of date the day after. The command writes
it from the Zeno that is running, so it cannot disagree with it.

**Why a new frame for each run.** A program in a page does not end
(14). Its timers and the functions it gave away are the browser's
to hold, and there is no list of them to go through. Throwing the
frame's page away is the one thing that is certain to stop it all,
and it is what the browser does best.

**Why the program is in the link, after `#`.** Sharing then needs
no server and no account, and what follows `#` is never sent
anywhere. The price is a long link for a long program.

**Left out.** Colours in the program text, several files, a Stop
button for a program that never returns (it needs a Worker, and a
program in a Worker cannot reach the page).

### 16.13 Left for later

- JSON read into values, which `fetch` wants (done since: chapter 18).
- A timer that can be stopped (done since: chapter 19), and an
  event with the keys held down and the button of the mouse (16.6,
  16.2).
- Images and curves on a canvas.
- The address bar: a page with several views that the back button
  walks through.
- Other browsers. Only Chromium has run any of this.
- A compiled page, small and fast (14.1). It is still the big one.

## 17. What 0.1 left, taken up

13.8 and the last list of [`version-0.1.md`](version-0.1.md) named
things that were found and left. This chapter is the ones that have
been taken up since, each with what was done and why, and one that
was looked at and left as it is.

### 17.1 A written list says what it holds, when the place could never agree

`let x: str = first([1])` used to say that `items` of `first[str]`
should be `list[str]`, at the argument. Now it says "`x` should be
str, but this is int", at `x`.

The rule of 3.16b stays: a list that is written out waits for the
place the result goes, so that `let b: list[u8] = twice([1, 2])` is
a list of bytes. What changed is that the place is listened to only
if the written values could be what it asks for. A written number
can be an int, a float, a sized number or a quantity. It can never
be a str. Where it never can, the list is what it looks like, and
the mismatch is reported where it really is: an int is being put
into a str.

**Why.** The old message was true and in the wrong order (13.8). It
named a type, `first[str]`, that the reader never wrote, and pointed
at the half of the line that was right.

### 17.2 `Box([])` outside strict mode stays of unknown type

`let b = Box([])`, with `class Box[T] { items: list[T] }`, gives a
`Box` whose `T` nothing says. In strict mode that is an error that
asks for the type. Outside it, `T` is unknown, and no message is
given. 13.8 called that "without a word" and left it open. It is
now settled as it is.

**Why.** It is the rule of 0.2 (types are optional) and nothing
else. `let xs = []` is a list of unknown type outside strict mode
and an error in it, and `Box([])` is the same list with a class
around it. A message for the one and not the other would be a rule
with an exception, and a message for both would end optional types
for the commonest line in a first program.

### 17.3 `min` and `max` take quantities in one unit. `sum` does not

`max(3 V, 2 V)` is `3 V`, and `min(readings)` of a `list[V]` is in
volts, in every engine. Quantities in two units are refused, with
the conversion to write (`x.V`), and so is a quantity beside a plain
number.

**Why now.** 13.6 refused them with a message that said to compare
with `<`. That was honest and it was a loop to write for something
every list of readings wants.

**Why one unit.** `min` gives back one of the values it was given,
as it is. `min(3 V, 200 mV)` would be `200 mV`, and `min(1 V, 2000
mV)` would be `1 V`: the unit of the answer would depend on the
numbers, and the checker has to know the unit before the program
runs (1.15). With one unit going in, one unit comes out.

**Why not `sum`.** The sum of no volts is `0 V`. A list that is
running does not know its unit when it is empty: the unit is in the
values. `zeno run` would print `0` where the compiled program, which
knows the type, prints `0 V`, and the two must print the same
(5.19). So `sum` still refuses, and its message now gives the three
lines to write: start from `var total = 0 V`, and add each in a
loop. `min` and `max` have no such case, because the least of
nothing is an error already.

### 17.4 Resting the mouse on a field says its type

In an editor, the `x` of `p.x` answers `x: int`, and a method what
it takes and returns. `ExprKind::Field` now keeps where the name
after the dot is written (`at`), and the checker notes each one it
reads with its type.

**Why only the type, and not where it was declared.** A use of a
variable points back to its declaration through a number the
checker gives each one (9.11). A field has no such number and
`ast::Field` has no place. Giving it one is a change to the tree
that every stage reads, for a question ("where is `x` declared?")
whose answer is nearly always "in the class two screens up", which
"go to declaration" on the class already finds.

### 17.5 `zeno test --only "text"` chooses tests by their name

It runs the tests whose name has the text in it, exactly as
written, and passes over the rest. The file itself runs whole
first, as always.

**Why text in the name, and not a pattern.** A test's name is a
sentence (10.16), so a word or two of it is how anyone would point
at it. A pattern language is a second thing to learn for a tool
that is used in a hurry.

**Why the capitals count.** "The same text" has one meaning.
"The same, give or take capitals" has one for English and several
for other languages (3.14 is the same choice for `upper`).

**Why the program still runs whole.** Tests use what the file
declares and what its top level did (10.19). Running less would
make a test pass alone and fail in company, or the other way.

## 18. JSON

A page that can ask the network (16.8) gets text back, and nearly
always the text is JSON. 16.13 left reading it for later, and this
is that. [`json.md`](json.md) is the walkthrough.

### 18.1 `json.parse` gives a value whose type is known when the program runs

`json.parse(text)` gives dicts, lists, text, numbers, bools and
`none`, as the text has them. The checker treats the result as it
treats a variable with no type written: anything may be asked of
it, and a mistake is found when the line runs. Strict mode and
`zeno build` refuse it, with the message they have for any value of
unknown type.

**Why.** What the text holds is not known until it arrives. A
language has two honest answers to that. One is a type for "some
JSON" with a way to ask what each part is. The other is a value
whose type is found out at run time. Zeno has the second already:
it is what a type left out means (0.2), and every program a
beginner writes uses it. Using it here adds nothing to learn.

**The cost.** JSON is out of firmware builds. A board does not
parse JSON from a server, and the day one must, the answer is
reading into a class (18.5), which has types.

**Turned down.** A `Json` enum with a case for each kind
(`Text(str)`, `Number(float)`, `List(...)`): right for a language
with only static types, and four lines of `match` to read one
price.

### 18.2 `json.write` writes an object field by field

`json.write(order)` writes `{"who": ..., "lines": [...]}`: each
field by its name, in the order the class declares them. It also
takes what `parse` gives. A dict's keys are written as text.

**Why objects.** A JSON object nearly always has values of several
types under its names: a name, a count, a list. A dict written in a
Zeno program cannot, because its values have one type (6.31,
10.11), and that rule is right for dicts. The thing in Zeno that
has named parts of different types is an object. So a program says
what it sends by declaring a class, and the checker holds every
value to it before anything is sent.

**Why an int key becomes text.** JSON's keys are text. The other
choice is to refuse, and then a dict that counts by number cannot
be written at all without being rebuilt by hand.

**Why a quantity is refused.** `3.3 V` would be written `3.3`, and
whoever reads it has to guess the unit. Dropping a unit silently is
what quantities exist to stop (1.15). The message says to write the
number and put the unit in the name.

### 18.3 A number with nothing after the point is an int

`12` and `12.0` both give an int. `12.5` gives a float.

**Why.** JSON has one kind of number, and what writes it often
does not keep the difference: a price of two is `2` from one server
and `2.0` from another, and `2` from JavaScript always. If the
`.0` decided the type, the same program would get an int on Monday
and a float on Tuesday from the same server. Deciding by the value
is at least the same every day. A program that wants a float says
`float(...)`, which takes either.

**The cost.** `json.write(json.parse("2.0"))` is `2`.

### 18.4 A member that is `null` is left out of the dict

`{"note": null}` gives a dict with no `note`.

**Why.** A dict holds no `none` (10.11): `get` gives `none` for a
key that is not there, and one way to say "nothing here" is the
rule. JSON has two (the member is missing, or it is `null`), and
programs that write JSON use them for the same thing. With the
member left out, `data.get("note")` is `none` either way, and
`data["note"]` is a `KeyError` either way.

**The cost.** A text that is read and written again loses its
`null` members. In a list, where position matters, `null` stays as
`none`.

### 18.5 Left for later

- Reading into a class: `json.read(text, Order)`, with each field
  checked as it is read. It would give JSON to strict mode and to
  compiled programs.
- The place of a mistake in the text.
- JSON laid out over several lines.

## 19. A timer that can be stopped

16.6 gave `web.every` and said why what it starts could not be
stopped: stopping needs a value to stop. This chapter is that value.

### 19.1 `web.after` and `web.every` give a `Timer`, and it has `stop()`

```zeno
let ticking = web.every(1 s, tick)
stop_button.on("click", function() { ticking.stop() })
```

**Why.** A stopwatch, a poll that ends when the answer has come, a
message that should not appear because the user has moved on. With
no way to stop, each is written as a `var running` that the
function reads first, and a timer that goes on calling for as long
as the page is open to be told no.

**Why a value with a method, and not `web.stop(number)`.** A number
is something a program can add to, print and get wrong, and
`web.stop(3)` says nothing a reader can check. `ticking.stop()`
reads as what it does, and the checker knows a `Timer` from an
`int`. It is the shape `Element` has already: a value the page
knows, with methods.

**Why one type for both.** What is stopped is a call that waits,
whether it waits once or again and again. Two types would be two
names for one method.

**The alternatives.** The function could return `false` to say
"no more": it is short, and it makes every function given to
`every` return something, and the stop can then only come from
inside. A `stop` word inside the function has the same limit.

### 19.2 Stopping what has stopped is nothing

`stop()` on a timer that was stopped before, or on an `after` that
has made its call, does nothing and is not an error.

**Why.** "Make sure this is not running" is what the program means,
and it is true afterwards either way. An error would make every
caller keep a `var stopped` beside the timer, which is the
bookkeeping the timer was added to take away. A browser treats its
own timers the same way.

### 19.3 A program that does not keep the timer is as it was

`web.after(500 ms, tick)` on a line by itself still means what it
did. What it gives is dropped, as the answer of `list.pop()` on a
line by itself is.

**Why.** Most timers are never stopped, and every program written
for 0.2 before this has such lines.

### 19.4 On a desk

Under `zeno run` and `zeno test` the page calls nothing (14.6), so
`stop()` has nothing to call off, and prints nothing: a line is
printed for a change that could be seen (14.7), and this is not
one. A `Timer` prints as `Timer`, and two are equal when they are
one.

### 19.5 Left for later

- Asking a timer whether it is still waiting.
- Starting a stopped timer again. `web.every` makes a new one.

## 20. The playground on zeno-lang.org

The playground (16.12) goes on the website, at
zeno-lang.org/playground, where anybody can open it. Two things had
to change first.

### 20.1 In a page, one turn of a program may run for 5 seconds

A program's first run, and each call from the page (a click, a key,
a timer, an answer from the network), is a turn. In a browser a turn
that has gone on for 5 seconds is stopped, with an error at the line
it was on: a loop, or the call it was making. On a desk nothing
changes: a program runs for as long as it likes.

**Why.** While Zeno runs, the page does nothing else. It does not
draw, scroll or let a button be pressed, and the playground runs
each program in a frame that shares its thread, so a loop that never
ends held the whole playground, with nothing to press to stop it.
That is the first thing somebody tries in a playground. A program
that stops on its own, with its line and a question, is the answer
`zeno run` would give if it could.

**Why 5 seconds.** A page that does not answer for longer than that
looks broken, and browsers start offering to close it at about
that point. Every program in `examples/` takes a small part of a
second in a page, so the limit is far from anything that is meant.

**Why it is not an error the program can catch.** It is not about
the program's data, and a `try` around the loop would only start the
loop again. It is like going past the nesting of 200 calls (14.12).

**How it costs nothing.** The interpreter counts loop rounds and
calls, and asks the clock once in 65536 of them, because asking it
crosses into JavaScript. Counting is one addition.

**Rejected.** A Stop button that works at once: it needs Zeno in a
Worker, where a program cannot reach the page (16.12). A count of
steps instead of a clock: a step is fast on one machine and slow on
another, and what matters to the person waiting is time. A frame
from another origin, which some browsers run on another thread:
not all do, phones least of all, and the program would no longer be
able to share the Zeno the playground read in.

### 20.2 The playground's own page for what a program prints

A program with no page of its own (most programs) is shown in a page
the playground writes: what it prints, in the playground's type,
and under the last line, the error that stopped it. It follows the
dark setting of the browser. `zeno build --web` keeps its plain page
and its error at the foot (14).

**Why.** In the plain page the error box is pinned to the bottom of
the frame, which is right for a page somebody designed, where it
must not be lost among the buttons. For a program that prints, the
error belongs where the next line would have been, as in a terminal;
at the foot of an empty frame it reads as a thing apart.

**The rest is looks.** The colours of zeno-lang.org, links to Learn,
the Book and the Docs, and on a phone the boxes are in 16px type,
because a phone zooms into a box with smaller letters and does not
zoom out again.

## 21. `sort` with a function

`sort` put numbers and text in order and refused everything else,
and told the reader to "write the loop that puts them in yours"
(6.6). That loop is a sorting algorithm, which is the thing a
library is for. This chapter is `sort` for every list.

### 21.1 The function gives a key, and is not asked to compare

```zeno
people.sort(function(person: Person) -> int { return person.age })
```

The function takes one item and returns what it is ordered by: an
int, a float, a sized number or text. Smallest key first.

**Why.** There are two shapes in the big languages. C, JavaScript
and Java ask for a function of two items that says which comes
first. Python and Rust's `sort_by_key` ask for a key. The first
can say anything, and can also be wrong in ways nobody sees: one
that answers "before" both ways round, or is not the same from one
call to the next, gives an order that depends on how the sorting
was written, and in C it can read past the end of the list. A key
cannot be wrong like that. Whatever the function returns, numbers
and text have one order.

It is also what people mean. "By age" is a key. `a.age < b.age`
is the same thought written twice, with a chance to write `a` both
times.

**The cost.** An order that is not "by something" needs a key
built for it, and biggest first is a minus (or two sorts for
text). A function of two items is refused with the reason, since
it is what someone coming from JavaScript will write first.

**The alternative** of taking both shapes, by how many parameters
the function has, is two things under one name. It was not taken.

### 21.2 The same name: `sort()` and `sort(by)`

**Why.** It is the same act, on the same list, in place. A second
name (`sort_by`) would be a second thing to learn for it, and
`sort()` is then `sort` by the item itself. `pop()` and `pop(at)`
are one name the same way.

### 21.3 Items with the same key stay as they were

**Why.** It is what a person expects of "sort the table by this
column", and it is what makes two sorts add up: by name, then by
age, is by age and by name within an age. Without it the order of
equal items is whatever the algorithm did, which is nothing the
program said.

**How, with heapsort.** Heapsort does not keep equal items in
order. The library has it because it needs no second list and no
deep stack (library.md), and a compiled program must print what
`zeno run` prints (5.5d), so both sides have the same steps. So
each item's place is kept beside its key, and two keys that are
neither before the other are settled by place. No two items are
equal any more, and any correct way of sorting gives one answer.

**The cost.** Two lists while it sorts, of keys and of places.
`sort()` with no function still needs none.

**`sort()` with no function is left as it is.** Two equal ints
cannot be told apart, so nobody can see their order. The one case
that can be seen, `0.0` and `-0.0`, is the same in every engine and
is written down (library.md).

### 21.4 The function is called once for each item, first

All the keys are worked out, and then the list is put in order.

**Why.** n calls, where asking during the sorting would be about
n log n and would call the function with the list half sorted. A
function that prints, or counts, does it once for each item, in
the order the list was in.

**If the function changes the list.** It may, as one given to
`map` may. If the list then has more or fewer items than there are
keys, `sort` raises a `ValueError` and the list is as the function
left it. There is no right place for an item with no key.

**Found on the way.** A function given to `map`, `filter` or
`reduce` that changed the list it was walking crashed `zeno run`
(the built programs were right). The interpreter held the list
while it called the function. It lets go first now, and there is a
test.

### 21.5 Left for later

- A key that is a quantity: `readings.sort(function(r) { return
  r.temperature })` where that is `20 C`. `sort()` does not take
  quantities either (17.3 is about `sum`, and the same care is
  needed).
- Two keys at once, where two sorts are written now.
- `sorted`, which gives a new list and leaves the first. `slice(0)`
  and `sort` do it in two lines.

### 20.3 A program without `import web` gets the page that shows what it prints

In the playground, the page in `page.html` is used only when the
program says `import web`. Any other program is shown in the page
that shows what it prints, whatever `page.html` holds.

**Why.** A program that does not import `web` cannot reach a page,
so the page can only get in the way: after opening the list of
things to do and writing a program of one's own over it, the old
page stayed, and what the new program printed went nowhere. Asking
the program is the one rule that is right both ways, and there is
nothing to remember: the tab is still there for a program that
wants it.

### 20.4 The playground is an editor, and checks as you write

The playground was a box and a frame. It is now an editor with
colours and line numbers, indentation that follows the braces, a
problem list that is kept up to date as the program is written,
Format, Share, fifteen examples in four groups, halves that can be
dragged, and a phone layout with one half at a time. Partha asked
for "more features" and a "modern minimal and elegant" look
(2026-11-16); this is what was chosen, and why.

**Checking as you write, by Zeno itself.** `zeno.wasm` gained two
calls, `zeno_check` and `zeno_format`, and the playground starts a
second copy of the module to make them. Every message is the one
`zeno check` would print, so the playground can never disagree with
the compiler. A problem found while typing is worth more than one
found by pressing Run, which is why the error line is marked in the
gutter at once.

**Colours from a list of words, not from the lexer.** Colouring runs
on every key and has to keep up; a short pattern in the page does,
and a word coloured wrongly does no harm. The checker, which must be
right, is Zeno's own.

**No editor library.** A text box with the coloured text drawn under
it is a few hundred lines, keeps the page one file that works with
nothing fetched, and leaves the browser's own typing, selection,
spelling-off, Undo and phone keyboard alone. A full editor library
would be several times the size of Zeno's own page for things a
playground of one file does not need (completion, folding, many
cursors).

**Kept in the browser.** What was written is kept in the browser's
storage and comes back on the next visit, because losing a program
to a closed tab is the worst thing a playground can do. A shared
link wins over it, because somebody who opens a link wants what is
in the link.

**One half at a time on a phone.** Two halves of a phone screen are
each too small to write or read in. A switch between Code and Result,
with Run going to Result, is how phone apps with two views work.

**The look is the website's.** The same type (Hanken Grotesk and
JetBrains Mono), the same colours (Windows accent blue, #0078d4, as the one colour since 2026-11-16, red
only for problems), the same logo and the same light or dark choice,
which the two share. Panels with a thin rule instead of shadows,
icons drawn as single lines, and the labels of Format and Share
dropped to icons on a phone.

### 20.5 Drawing with a finger: pointer events, and a canvas counted in its own pixels

The sketch listens for `"pointerdown"`, `"pointermove"` and
`"pointerup"` instead of the mouse's events, and on a `<canvas>`
`event.x` and `event.y` are counted inside its border, in the
canvas's own pixels, however large the page shows it.

**Why pointer events.** A browser sends them for a mouse, a finger
and a pen alike, with the same place in them, so one program draws
on a desk and on a phone. Nothing in Zeno had to change for that:
`on` already takes any event of the browser (14). The canvas says
`touch-action: none`, so a finger draws instead of scrolling.

**Why the canvas's own pixels.** On a phone the canvas is shown
narrower than it is (`max-width: 100%`), and a place counted in the
page's pixels would draw a line away from the finger. `draw_line`
takes the canvas's pixels, so the event gives the same kind of
number, and a program never has to know how large the canvas is
shown. Before, the place also counted the border, one pixel off.
Only a canvas is counted this way: for anything else there are no
pixels of its own.

## 22. Speed: choosing how memory is freed, `--release`, and a faster collector

The first benchmarks (bench/README.md, 2026-11-16) put Zeno next to C,
Rust and Python on six programs. Arithmetic and arrays came out 1.2 to
1.5 times slower than C. Anything that made many blocks of memory came
out 10 to 26 times slower, and the cause was the collector. A build
with no collector was faster than C on the same program, but it was
made for a chip and had a fixed pool of 32 KB. This chapter is what
was done about it. The aim, in Partha's words, is that a Zeno program
"has to run faster ... as fast as C and or Rust", while each project
chooses how it frees memory.

### 22.1 A program chooses how memory is freed: `memory owned` or `memory collector`

```text
package server
memory owned
```

A line in `zeno.package` says how the program frees memory when it is
built. `memory collector`, which is what a program gets when nothing is
said, is the garbage collector, and lists and objects can be shared
freely. `memory owned` is one owner for every list and object, freed on
the line where its owner ends, with no collector in the program: the
rules of `--no-collector` (5.15), which are Rust's. `zeno build --owned`
and `zeno build --collector` say the same for one build, over the file.

`zeno check` and `zeno run` of a program that says `memory owned` check
it by the owned rules too, so that a program that runs also builds.

**Why in `zeno.package`.** How memory is freed is a property of the
whole program, not of one build of it: an owned program is written
differently (it says `take`, it does not share a list between two
objects), so the choice belongs with the program, where the next person
finds it. The command line can still say otherwise for one build.

**Why two words and not a third for C.** C's way, asking for memory and
freeing it by hand, is already in the language: `unsafe { }` with
`allocate` and `free` (5.18). It is a block, not a program, because it
is the one way in which a mistake is not caught, and a mistake should be
looked for in a few lines and not in the whole program.

**Why not per line.** A value is made on one line and used on many,
and how it is freed has to go with the value, not with a line: a list
made "with the collector" on line 3 and freed "by its owner" on line 9
is a list nobody can say the rules of. So the choice is made where it
can be kept: for the program here, and for one value with `owned`
(5.8), which works in a collector program too.

**Alternatives.** A flag only (`--no-collector` as it was), which every
build must remember. An attribute in the source file, which a program
of several files would say several times, and could say differently.
(22.9 later gave a file a line of its own, for the classes it declares.)

### 22.2 `memory owned` on a computer: the pool and the text room grow

`--no-collector` is for a chip: one array of a fixed size for lists and
objects, another for text, and the size is known when the program is
built (5.9h, 5.9l). On a computer nobody knows in advance how much a
program will need, so `memory owned` keeps the same blocks, sizes and
lists of spare blocks, and when the piece of memory it is cutting from
runs out it asks the system for another, twice as big as the last. The
text room becomes a chain of rooms in the same way. `--firmware` with
`memory owned`, and `--no-collector`, keep the fixed arrays.

**Why doubling.** A program that needs a gigabyte asks the system
about twenty times, and what is left at the end of a piece is at most
half of it, and usually far less.

**Why a chain of rooms and not one room that moves.** Text in the
room is pointed at by the statement that is building it. A room that
moved would leave those pointers behind. A chain never moves anything,
and the compiled program does not change at all: it saves the top of
the room and puts it back, and the top counts bytes across the chain.

### 22.3 `--release`: whole numbers wrap round instead of being checked

`zeno build --release` (and `--llvm --release`) compiles `+`, `-` and
`*` of whole numbers to the processor's own instructions, which wrap
round on overflow, in place of the checked ones that stop the program
with an `OverflowError` (5.5c). Nothing else changes: `//` and `%` by
zero, an index past the end, a key that is not there and `u8(300)`
still stop the program, and `zeno run` checks everything as before.

**Why.** It is what Rust does: a debug build checks, a release build
wraps. The check costs a test and a jump on every step of a tight loop,
and on the benchmarks it was most of what was left between Zeno and C
on integer code. A program that has been tested can drop it.

**Why not drop the index checks too.** An index past the end in C reads
or writes memory that belongs to something else, and the program goes
on, wrong, with no message. That is the mistake that safe languages
exist to prevent; Rust keeps it in its release builds, and so does Zeno.
A wrapped number is wrong too, but it is a number: nothing else in the
program is broken by it.

**The cost.** A release build of a program that overflows prints
something else than `zeno run` does. That is the one place where the
engines are allowed to disagree, and it is only with the word written.

### 22.4 A loop walks a list in place when its body cannot change it

`for item in items` walks the list as it was when the loop began
(5.5c), so a compiled loop has always walked a copy, made for it. The
copy is made on every loop: in bench/nbody.zeno, ten million copies of
a list of five planets, a quarter of the run.

Now the compilers look at the body first (`ast::walks_in_place`). If it
never writes the name the list is reached from, calls nothing but the
built-in functions that only look at their arguments, replaces no item
of a list, and makes no function value, then nothing in it can change
the list, the copy would be the list itself, and the loop walks the
list where it is.

**Why decide from the words and not from the types.** It is a rule a
reader can check by eye, with nothing to get wrong. A body that does
more (calls a function of the program, say) still gets its copy, and
behaves exactly as before; nothing is ever walked in place that could
change.

### 22.5 The collector keeps its blocks in pages

The collector of 5.7 wrote every block in one table, sorted the table
at each collection, and found the block a word points at by halving
the table. On a program that keeps two million blocks and makes
hundreds of millions (bench/trees.zeno), the sorting and the searching
were most of the run.

Blocks now live in pages of 64 KB, each at an address that is a
multiple of 64 KB, and a page holds blocks of one size (16 sizes, from
16 bytes to 2 KB; bigger blocks get pages of their own). The page keeps
a bit for each block in use and a bit for each block reached. A hash
table says which page covers each 64 KB of memory, so "is this word one
of my blocks?" is one look and a division, and freeing a block is
finding its page from its own address. A block has no header any more.
This is the arrangement of most collectors that, like Zeno's, look at
memory with no help from the compiler (the Boehm collector is one).

**Results.** bench/trees.zeno went from 57.8 to 12.0 seconds, and
bench/strings.zeno from 24.5 to 3.4 (with 22.6). C takes 5.9 and 0.9.

**What stayed.** It is still mark and sweep, conservative, and
collects when the memory in use passes twice what survived the last
collection (the first limit is now 4 MB, up from 1 MB, since blocks
are cheaper to keep). Every test that ran the old collector before
each allocation (`ZENO_COLLECT_ALWAYS`) runs the new one the same way.

**Left for later.** A collector that looks at young blocks more often
than old ones (a generational one) would make trees faster still: most
of what is left is marking the same long-lived tree at every
collection.

### 22.6 Text of one ASCII character is made once

`for ch in text`, `split("")` and the like hand out a text for each
character. Each was a block of its own; now a text of one ASCII
character is taken from a table of all 128 that is part of the
program, and no block is made. With no collector nothing frees them,
which is right, since only text in the pool is ever freed.

### 22.7 `for ch in text` walks the text and makes no list

**Decision.** A loop over text with one name walks the text where it is,
one character at a time. The loop variable is a character of ASCII from
the table of 22.6, or, for any other character, a copy of it. With a
collector the copy is the collector's. With no collector the loop holds
the last such copy and frees it when it makes the next one, or when the
loop ends or is left; whatever keeps the character has its own copy
already (a list, a variable outside the loop). With no collector the
text must stay where it is while it is walked, so this is done when the
loop is over a variable the body never names, or over a literal. Every
other loop over text, and every loop with two names (`for i, ch in
text`), walks a list of the characters, as before.

**Why.** The list was the whole cost: ten million characters made a list
of ten million slots, grown by doubling, then freed. The strings
benchmark went from 3.5 to 1.1 seconds (C takes 0.9). A character as a
str of its own keeps the language as it was: `ch` is still a str, and
`ch == ","` still means what it meant.

**Rejected.** A separate `char` type: faster still, but a second kind of
text for a beginner to learn, and every `ch` in every program would
change. Asking the body not to name the text even with a collector:
there the text cannot go away while the loop holds it, so it is not
needed.

### 22.8 A field a function never writes is read once

**Decision.** In a function that calls nothing but the built-ins that
only look (`print`, `len`, `abs`, `math.sqrt`, ...), a number in an
object that a parameter holds is read once, at the start, and kept in
a local, when no statement of the function writes a field of that name
on any object. The LLVM build reads it in the function's entry block.
`math.sqrt` in the LLVM build is written in the program's own IR, as
`/` already was (`WRITTEN` in `src/emit_llvm.rs`), so that it can be
inlined.

**Why.** `pull(b, c)` in the n-body benchmark reads `b.mass` after it has
written `b.vz`. C must read it again in case `b` and `c` are the same
object, and the compiler joined that read with the write before it in
a way the processor stalls on. Zeno knows more than C here: nothing in
the function writes `mass`, and nothing it calls can. Read once, the
benchmark went from 1.0 to 0.44 seconds, where C takes 0.43. It is what
a C programmer does by hand, `double m = b->mass;`, done for them.

**What it keeps.** The meaning of the program: the field cannot change
while the function runs, so the first value is the only value. A field
that is written anywhere in the function, through any object, is read
each time; so is every field in a function that calls one of the
program's own functions or a method.

### 22.9 One file can say `memory owned`

```text
memory owned

class Node {
    left: Node or none
    right: Node or none
}
```

**Decision.** A file may begin with the line `memory owned` (or `memory
collector`), alone or beside `strict`, in either order. In a file that
says `memory owned`, every class is an owned class, as if it were
written `owned class` (5.8): each object has one owner and is freed on
the line its owner ends, also in a program that has a collector. When
every file of a program says `memory owned` and its `zeno.package` says
nothing, the whole program is owned: it is built with no collector, as
22.1. A file that says `memory collector` in a program built with no
collector is a mistake, and the message says so.

**Why the file, and why its classes.** Partha asked for the choice to be
finer than the program: "very modular", "more power to the user". The
file is the finest place that has a clear meaning, for the reason 22.1
gives against one line: how a value is freed has to go with the value,
and a value's kind is declared in one file. A class declared in a
`memory owned` file is owned wherever its objects go, and the rule for
where the two ways meet is the one owned values already have in a
collector program (5.8): a parameter that looks can be given any
object, a value with one owner cannot be kept in a shared place. So
nothing new has to be learned at the boundary, and nothing about the
boundary is left unsaid. The trees benchmark with its file saying
`memory owned` runs in 4.5 seconds in a collector program, against 11
with the collector and 4.1 with no collector at all.

**Why not lists too.** A list is made by a line, not declared, and is
passed between files more than any other value; making every list of a
file owned would mean that half of a program's lists follow one set of
rules and half the other, with the line between them invisible. A list
that should have one owner says so where it is made: `owned list[int]`.
In a program where every file says `memory owned`, every list is owned
anyway (22.1).

**Why not per function.** A function's values are made from the
classes of its file, and go back to its callers; a function that freed
by different rules from the file around it would be a second boundary
inside one file, for a gain the file already gives. Per value (`owned`)
and per block (`unsafe { }`) are the finer choices.

**Alternatives.** `owned` written in front of every class (it still
can be, one at a time). A list of files in `zeno.package`: far from the
code it is about, and wrong as soon as a file is renamed.

### 22.10 Two generations, watched with `mprotect`; and a leak found

**The leak.** A word that pointed just past the end of a block counted
as pointing into it (22.5), so that a loop holding only the end of a
list's items would keep them. But in a page of blocks laid end to end,
that word is also the start of the next block, so every block reached
kept the one before it alive, and that one the one before it. The trees
benchmark held 2.3 GB at once where it needed 130 MB. Now the rows of
items of lists have pages of their own, and only there does a word just
past a block count for it.

**Decision.** The collector has two generations. A block that lived
through a collection is old; most collections are small, look only at
the young, and stop at an old block without looking inside it. Once the
old have grown to twice what lived through the last full collection,
the next collection is full. After each collection the full pages of
blocks with pointers are made read-only with `mprotect`; the first write
to one is caught by a signal handler, which opens the page and marks it
as written. A small collection looks inside the old blocks of every page
that is open. On a chip, and for now on Windows, every collection is
full (22.11).

**Why.** The trees benchmark keeps a tree of two million nodes for the
whole run, and every collection marked all of it again: 140 times. With
generations it is marked a few times, and the run went from 11 to 7
seconds (C takes 6.2).

**Why `mprotect`, not a write barrier in the compiled code.** The usual
way is for every store of a pointer to say so, in code the compiler adds.
Here pointers are stored by compiled C, by compiled LLVM IR and by a few
hundred places in the runtime; one place missed is a block freed while
in use, found much later and far away. The processor's own protection of
pages misses nothing, costs nothing on a page nobody writes, and needs no
change to either compiler. Pages of text are never guarded, so a file
read into memory by the system cannot meet a read-only page.

**Rejected.** Moving young blocks to another place (a copying nursery),
which is faster still but needs to know exactly which words are
pointers, and this collector does not (5.7). A bigger limit between
collections, which trades memory for time without changing the work.

### 22.11 Left for later

- Generations on Windows, with `VirtualProtect` and a handler of its
  own.
- Generations on macOS (done after 0.3.0). They were on at first, and
  in the 0.3.0 release rehearsal three compiled test programs hung on a
  Mac with Apple silicon while Linux passed, so 0.3.0 shipped with every
  collection full on macOS. The cause was found after the release
  (22.14): a `try` in an LLVM build left the signal mask junk. With a
  matched pair the whole suite passes on a Mac with generations, and
  they are on there again.

### 22.12 Each release keeps its benchmark times

`bench/record.py --version X` times the benchmarks with every engine
that version has and writes `bench/results/versions/X.json`; the
release steps (docs/release.md) say to run it after a tag. 0.1.0 and
0.2.0 were recorded afterwards from their tags. Partha asked for a
history (2026-11-16).

**Why a script and not a step of the release workflow.** GitHub's
machines are shared; the same program's time moves by a quarter
between runs there, which hides most changes worth seeing. One
machine, run by hand, gives numbers that can be compared.

**Why each time is shown against C's.** C, Rust and Python run in the
same record. A version's time divided by C's time on the same machine
stays meaningful when the machine is a new one, so records made a
year apart still line up.

**Why a missing engine is left out.** 0.1.0 has no `--release`; a
build that fails because the flag does not exist is not a slow build,
and a "failed" in the history would read as a bug.

### 22.13 The VM gets superinstructions, quick arithmetic and fields by place

Three small changes to the VM, and one shared with the interpreter.
`bytecode::fuse` puts superinstructions over the commonest runs of
instructions (local and local or constant into an operator, then a
`SetLocal` or a `JumpIfFalse`; a local's field). `fast_binary` answers
two ints or two floats in the VM itself. An object keeps its fields in
a list in class order (`Instance.fields: Vec<Value>`), found by
`ClassDef::slot`. A method call given every argument becomes a frame
straight away. At the small sizes `zeno run --vm` went from 15.5 to
7.5 s on `loop` (Python: 11.3), 1.04 to 0.70 on `fib`, 18.0 to 9.3 on
`trees`, 23.2 to 15.4 on `nbody`.

**Why superinstructions are put over the code and not in place of
it.** Fusing in place would shorten the code and move every jump
target; the compiler would have to work out the jumps again. Laid over
the first instruction of a run, with the rest left under it, nothing
moves, a jump into the middle still works, and `zeno bytecode` still
prints what the compiler wrote, which is what docs/vm.md teaches from.

**Why the quick arithmetic gives up on every error.** An overflow, a
division by zero or a mixed pair goes to `ops::binary`, which is where
every message is written. One meaning stays in one place (5.10).

**Rejected for 0.3.** A register machine and 16-byte values, which
would be the next big step; Partha chose to finish 0.3 first and decide
after seeing its benchmarks (2026-11-16).

### 22.14 A `try` jumps with `_setjmp` and `_longjmp`

Outside Windows, the runtime's `zeno_raise` jumps with `_longjmp`, and
the C build's `try` calls `_setjmp` (the macros `zeno_setjmp` and
`zeno_longjmp` in runtime.c). Windows keeps `setjmp` and `longjmp`.

**Why.** The LLVM build has always called `_setjmp`, since IR cannot
call the macro `setjmp`. The runtime answered with plain `longjmp`. On
macOS `longjmp` also sets the signal mask from the jmp_buf, and
`_setjmp` stores none there, so after a caught error the mask was
whatever bytes were in that place. The 0.3.0 release rehearsal found
it: in an LLVM build that caught an error, the write-fault signal of
the collector's generations (22.10) was held back, and the next write
to a guarded page stopped the program for good. glibc's `_setjmp` marks
the mask as not saved, so Linux never showed it. The release thread
traced it on a Mac (lldb on the hung program, decisions 22.11). A
matched pair is correct on every system, and `_setjmp` saves a system
call for each `try` on macOS. Zeno never changes the signal mask, so
there is nothing for a jump to put back.

### 20.6 The playground shows the C a program becomes

The playground has a C tab: the program as `zeno build` writes it,
by the same compiler, with a choice of the three builds and the
runtime hidden unless asked for. Partha asked for it (2026-11-16).

**Why.** Zeno compiles to C, and the C is the plainest answer to
"what does this line cost?". A class is a C struct, `match` is a
chain of `if`s on a tag, a firmware build passes errors as values
instead of jumping. Seeing that next to the program, changing as the
program changes, teaches it faster than any page of docs. The three
builds are side by side because the difference between them is the
point of Zeno on a chip.

**Why the runtime is hidden.** It is the same 2,700 or so lines for
every program, and the program's own C comes after it. Shown, it
pushes the part worth reading off the screen. The line numbers keep
counting from the true top of the file, so they match `hello.c` on a
desk, and Copy copies the whole file, which builds as it is.

**Why by the compiler itself.** `zeno.wasm` already holds the C
emitter (`main.rs` is compiled for the browser too), so the C in the
page is exactly the C `zeno build` writes, and cannot drift from it.

## 23. 0.4: a build as fast as C and Rust

Partha asked (2026-11-16) for Zeno to match C and Rust one to one on the
benchmarks, with a new option to build that way, so that the builds
there are keep every check they have.

### 23.1 `zeno build --fast`

`--fast` leaves out the checks a correct program never fails: a list
index out of range, a float divided by zero, the square root of a
number below zero, and the overflow of a whole number (as `--release`
does; 23.6 says how the two differ). The C compiler
gets `-O3 -fno-math-errno`, and so does clang for `--llvm`. The C file
starts with `#define ZENO_UNCHECKED 1`; the runtime's checks sit behind
`#ifndef ZENO_UNCHECKED`. In LLVM IR an item is found with a
`getelementptr`, `/` of floats is `fdiv` and `math.sqrt` is
`llvm.sqrt.f64`, with nothing in front.

**Why a new option and not a faster `--release`.** Partha asked for the
existing builds to stay as they are. `--release` drops one check, which
can only change what an overflowing program prints. `--fast` drops
checks whose failure means reading memory that is not the list's, which
is a different promise; it gets its own word, and the help says "as
unforgiving as C".

**Why the word `fast`.** It says what it is for, which is how people look
for it. `--unchecked` says what it does and was the other choice; the
help text and docs/memory-modes.md say exactly which checks go, in a
table, so nothing is hidden.

**Why a whole number divided by zero is still caught.** In C it is not a
wrong answer but a signal that kills the program, or undefined behaviour
an optimiser may build on. The check is one compare, usually against a
constant the C compiler folds away.

**Why `-fno-math-errno`.** C's `sqrt` must set `errno` for a number below
zero, so the compiler calls the library instead of using the one
instruction. No Zeno program can read `errno`. On nbody the flag alone
took a fifth off.

**Rejected.** `-march=native`: the program would not run on an older
processor of the same family, and a build is often copied to another
machine. `restrict` on parameters that are objects: two parameters may
be the same object in a correct Zeno program (`pull(b, b)`), and
`restrict` would make that program print a wrong answer, which is a
different thing from leaving out a check that only a wrong program
fails. It is also what keeps nbody behind Rust, whose borrow rules prove
the two bodies apart (23.4).

### 23.2 Numbers become text without `snprintf`

`str(n)` and every `{n}` in an f-string write the digits from the right,
two at a time from a table of the hundred pairs, in every build. On the
strings benchmark, `snprintf` (which reads its format string every
time) was more than a third of the run.

### 23.3 A big list grows with `realloc`

In an owned build on a computer (a pool that grows), the items of a list
of 64 KB or more come from the system with `calloc`, not from the pool,
and grow with `realloc`, as a C program's growing array does. `realloc`
of a big block usually moves no bytes: the system maps the same pages
at a new address. Before, each doubling took a new pool block, cleared
it, copied the items over and gave the old one back. The block carries
its size in front and a head whose step is `ZENO_FROM_SYSTEM`, so
`zeno_free` knows where it goes back. Kept text is no longer cleared
before it is copied in (`zeno_alloc_raw`), and `join` finds the end of
each piece while copying it.

**Why only big lists, and only with a pool that grows.** A small list
from the pool is cheaper than a call to the system. A build for a chip
has no system to ask: its pool is all the memory there is.

### 23.4 Where Rust is ahead on nbody, and why nothing is done about it

This section first said that Rust wins nbody because its borrow rules
prove the two bodies apart. That was a guess, and measuring it showed
it wrong (2026-11-16). It is kept here, corrected, because the wrong
guess would have led to the wrong work.

Counted in instructions for 200,000 steps (`bench/run.py --count`,
23.7): C 120.5 million, Zeno `--fast` 119.5 million, Rust 82.8 million.

**Where Rust's lead comes from.** In `bench/nbody.rs` the bodies are a
`vec!` of five, written in `main`. LLVM sees through to it: it knows
there are exactly five, opens both loops out for five, and does the
arithmetic of two pairs at a time with the instructions that work on
two numbers at once (`sqrtpd`, `divpd`). With the list hidden behind
`std::hint::black_box`, so that its length is a number met while
running (as the length of a Zeno list is), the same Rust runs 125.8
million instructions and takes 0.51 s: behind C (0.46 s) and behind
Zeno `--fast` (0.38 to 0.44 s). C does not get the lead either, though
its bodies are a fixed array of five: gcc and clang both leave it at
about 120 million.

**What was tried, and measured.** `restrict` on both parameters of
`pull`: 119.1 million instructions against 123.9 million, and no
difference in time over eleven runs. Reading `bodies[i]` once, outside
the inner loop: gcc already does. So telling the C compiler that the
two bodies are different, which is what this section used to ask for,
buys nothing here, and 23.1 was right to reject it for another reason.

**Why nothing is done.** To get what Rust gets, Zeno would have to
know the length of a list while compiling and open the program out
for that length. That is a compiler made faster for one benchmark with
five bodies, not for programs. A fixed-size array of values is a
feature the language may want for its own sake (a table of readings on
a chip), and it is to be weighed for that, not for this.

### 23.5 `//` and `%` cost no more than they must

Zeno's `//` rounds down and `%` takes the sign of the right side
(decision 2.1, as Python's). C's round toward zero. Two changes, in
every build, that change no result:

- **By 2, 4, 8 and so on, written as a number.** `x // 8` is the bits
  of `x` moved three places down, keeping the sign (`zeno_halved`,
  `ashr` in IR), and `x % 8` is its low three bits (`x & 7`). Both are
  exact for a number below zero, and here Zeno's rule is the cheap one:
  C needs three instructions to divide by two toward zero and Zeno
  needs one to divide by two downward. Neither can fail, so neither is
  checked. Only a number written in the expression counts
  (`emit_c::power_of_two`); a variable, or `-8`, goes the usual way.
- **The signs first.** `zeno_mod` mended its result `if (r != 0 &&
  signs differ)`. The C compiler tested `r != 0` with a jump (taken one
  time in seven for `% 7`) and, past it, worked the mended number out
  every time, to choose between the two afterwards. It is now `if
  (signs differ && r != 0)`: for a right side above zero that is one
  test of the sign of `r`, a jump never taken while the numbers are
  above zero, and nothing after it.
  `zeno_floordiv` has the same order. On the loop benchmark built with
  `--release`: 391.7 million instructions before, 340.3 million after.

**Rejected.** Mending without a jump (`r += b & mask`): measured
slower, more instructions on the path every number takes. Having
Zeno's compiler prove that the left side is not below zero and write
C's `%`: the C compiler already does this, once it is told the truth
about overflow (23.6), and it does it for every shape of loop, where a
rule here would know one or two.

### 23.6 In a `--fast` build, a whole number that overflows has no defined result

**What was found.** After 23.1, `--fast` still ran 13% more
instructions than C on loop and 17% more on sort. The cause was one
thing in both. `--fast` did its `+`, `-` and `*` as `--release` does:
on unsigned numbers, which wrap round. A C compiler must then allow
for the wrap. It cannot know that `i`, which starts at 0 and only goes
up by one, is never below zero, so it kept the mending of `(i * i) %
7` for a number below zero; and in the scans of quicksort it carried
the index and the address side by side, one more copy on every step.
A program written in C does not pay for this, because in C a signed
number that overflows has no defined result, and the compiler takes it
that none does.

**The decision.** `--fast` writes C's signed `+`, `-` and `*` as they
stand (`(z_i + 1)`), and `add nsw`, `sub nsw`, `mul nsw` in LLVM IR,
which is what clang writes for a C program. `--release` is unchanged:
there a sum wraps, and the answer is defined.

**What it gave** (instructions, `--fast --owned`): loop 300.27 million
against C's 300.26 million, the same instructions as C's loop; sort
78.7 million (C 73.3, Rust 78.7; it was 87.6); nbody 119.5 million (C
120.5).

**Why this is right for `--fast`.** An overflow is an error in Zeno:
`zeno run` and every checked build stop with `OverflowError`. So a
correct program has none, and `--fast` is by its own rule (23.1) the
build that leaves out what only a wrong program meets. It already
reads memory that is not the list's when an index is wrong; an
overflow with no defined result is the same kind of promise and a
smaller one. It is also the honest meaning of "as fast as C": C is
that fast because it makes this assumption, and nothing else gives
the C compiler the same knowledge.

**What it costs.** A program that overflows, built with `--fast`, may
print a wrong number or go round a loop for ever, and not the same way
on two compilers. The table in docs/memory-modes.md says so. Someone
who wants speed and a defined answer builds with `--release`.

**Rejected.** Keeping the wrap and working out ranges in Zeno's own
compiler: it could not show `i * i` to be above zero either, since a
product that wraps may be below zero; the knowledge that is missing is
"this does not overflow", and no analysis can supply what the build
does not promise. A third option between `--release` and `--fast`:
two words for speed are enough to choose from. Making `%` round as C's
does: that changes what programs print.

### 23.7 A speed is counted as well as timed

On the machine the benchmarks run on, the time of one program moves by
a tenth and more between runs. The loop of C and the loop of `--fast`
after 23.6 are the same instructions, and one session timed them at
0.74 s and 0.89 s. The first table of 23.1 showed nbody a fifth
behind C; counted, it was level. The guess in 23.4 was made from such
a table.

So `bench/run.py --count` runs each program once under valgrind and
writes how many instructions it ran (`bench/results/now-count.json`).
The count is the same on every run to five figures. It is not a time:
a move between registers costs nothing and a square root costs twenty
times an addition. The README shows both, and a speed claim in this
file is made from the two together. Timed runs now go in rounds, every
engine once per round, so a slow minute of the machine falls on every
engine and not on one.

### 23.8 Appending to a list is done where the program appends

After 23.6 the quicksort of the sort benchmark ran fewer instructions
than C's (65.9 million against 66.3 million), and the program as a
whole still ran 7% more. The rest was in building the list: each
`xs.append(seed)` was a call to `zeno_list_append`, 24 instructions and
a call, where the C program writes `if (len == cap) grow; xs[len++] =
seed` in the loop itself.

The growing is now a function of its own, `zeno_list_grow`, marked
`noinline`. What is left of `zeno_list_append` is a compare, a store
and a count, and the C compiler puts those in the loop. In every build
that goes through C. Sort with `--fast --owned`: 72.7 million
instructions, C 73.3 million. Strings: 174.7 million from 180.4
million (Rust 173.5, C 304.2).

**Why `noinline` on the part that grows.** Left to itself the C
compiler folded the growing into `zeno_list_append`, which made it too
big to put in a loop, so every append paid for a call to reach the one
in thousands that grows.

**Not done.** The same for a build through LLVM: its runtime is a
separate file, so every runtime function is a call there. Writing
append in IR, as `zeno.mod` is, is the way, and it waits until
something measures the need.

### 23.9 The assumption of 23.6 stays, and what it would cost to drop it

Partha asked (2026-11-16) whether the trade of 23.6 can be overcome:
C's speed, and a defined result when a whole number overflows. It
cannot, and this is the measuring behind that answer. He chose to keep
23.6 as it is.

**Why it cannot be had for nothing.** C's loop is fast because the
compiler uses a fact it cannot check: `i * i` does not overflow, so it
is not below zero. A build that keeps a defined result when the fact
is false must either test the fact or not use it. Testing costs
instructions and not using it costs instructions; there is no third
way to be right when it is false.

**What each way costs** (the loop benchmark, millions of instructions,
`--fast --owned` with only the overflow rule changed):

| What an overflow does | gcc | clang |
|---|---|---|
| C itself | 300.3 | 220.3 |
| assumed not to happen (23.6) | 300.3 | 220.3 |
| wraps round | 340.3 | 330.3 |
| stops the program (`-fsanitize=signed-integer-overflow`, trapping) | 420.3 | 400.3 |
| `-ftrapv` | 740.3 | |
| `OverflowError` (`zeno build`) | 460.3 | |

Five ways of writing `% 7` with no jump, so that a wrapped number
needs no mending afterwards, were tried under the wrap: 400 to 440
million, all worse than the jump of 23.5.

**What wrapping would cost on all six** (instructions against C, gcc):
fib 0.98, loop 1.13, sort 1.13, strings 0.57, trees 0.41, nbody 1.03.
A model of the processor (llvm-mca) puts loop's 13% at 4% of the time
on the machine the benchmarks run on and 12% on an older one. With
clang, which is the C compiler on a Mac, loop is 50% more.

**Found on the way.** Under the wrap, sort comes back from 82.8 to 74.9
million (C 73.3) if the unchecked `zeno_list_at` tells the C compiler
that the index is in range (`__builtin_unreachable()` when it is not),
which `--fast` promises already. With 23.6 it changes nothing, so it is
not in; it is the first thing to do if `--fast` ever wraps.

**Why 23.6 stays.**

- `--fast` already trusts every index of every list, which is the
  greater promise: a wrong index reads or writes memory that is not
  the list's. Taking back the lesser promise would not make the build
  a safe one.
- An `int` is 64 bits. A count that goes up a thousand million times a
  second overflows after 292 years. An overflow that can happen in
  testing is caught, with its line, by `zeno run` and by every build
  that is not `--fast` or `--release`.
- There is a build for speed with a defined result already:
  `--release`. Zig (`ReleaseFast` beside `ReleaseSafe`) and Swift
  (`-Ounchecked` beside `-O`) divide their builds the same way, for
  the same reason.

**Rejected.** `--fast` wrapping, with a second option to assume: to
match C would then take two words, and the word `fast` would mean
"nearly". Checking only the multiplications whose result goes into a
`%`: one rule for one benchmark.


## 24. 0.5: the program written in other languages

Partha asked (2026-11-16) for `zeno build` to write a program "in
almost all mainstream languages", HTML among them, by fixed rules and
with no AI when it builds. `zeno build --target <language>` does it;
docs/targets.md is the walkthrough.

### 24.1 Fixed rules, the same file every time

Each target is a printer of a lowered program (24.3) plus a runtime
(24.2). Nothing is guessed: the same program gives the same file, byte
for byte, on every machine.

**Why.** Partha asked for it, and it is the only way the written
program can be trusted: a rule that is right once is right every time,
and `tools/targets.py` (24.9) can prove it for every program it runs.
A translation that varies cannot be tested this way.

### 24.2 The rules of Zeno go with the program: a runtime per language

Each target has a runtime (`src/runtime.js`, `src/runtime.py`) that
does what `ops.rs` and `builtins.rs` do, step for step and message for
message, and every operator goes through it: `a + b` is `Z.add(a, b,
line)`. The runtime is put at the top of the written file.

**Why.** The promise is "it prints what `zeno run` prints", including
where it stops. A `+` of the target language would wrap (C++, Go,
Java, C#, Rust in release), lose precision (JavaScript) or never
overflow (Python), and print floats in its own way. Zeno's `+` is the
one that has to happen.

**Why at the top of the file, not a package beside it.** One file that
runs by itself, with nothing to install, which is what a person who
asked for "the program in Python" expects to get. The runtime is about
900 lines; it costs nothing to carry.

**Rejected.** Typed code (a plain `+` where the checker knows both
sides are ints), for now: it is faster, but each shortcut is a place
where the target could disagree with `zeno run`, and correctness comes
first. It is the next step for speed, one proven case at a time.

### 24.3 One lowering for every target

`src/lower.rs` turns the checked program into a small one any language
can say: unique names, each block's variables listed at its top,
functions moved to the top of their block, and what Zeno writes as one
expression but most languages need statements for (comprehensions,
`0 <= t < 10`, `and`/`or` whose right side needs statements) made into
statements, in Zeno's order of evaluation. Pattern matching becomes a
chain of `if`s; `try` becomes the target's own `try` with a chain of
tests that throws on what no `except` catches.

**Why.** Every hard question (what order things happen in, what a
name refers to, which variable an inner function shares) is answered
once, where it can be tested once. The printers only spell.

### 24.4 Names

Every variable gets a name no other variable in the program has
(`total`, `total_2`). A name that is a word of any target language
(one list for all of them) gets a `_` after it. Names starting with
`z_`, `Z` or `__`, which the runtime and the printers use, get `v_` in
front.

**Why one list for all targets.** A program's names are then the same
in every language it is written in, which makes the files easy to
compare with each other and with the Zeno.

### 24.5 Functions before they are written

A function declared in a block is set at the top of the block, so it
can be called from a line above it, as in Zeno. A `let` or `import`
it uses that is written further down is made at the top of the block
too, empty until its line runs.

**Why.** It is Zeno's rule, and the only order in which two functions
that call each other can both be set.

### 24.6 Ints in JavaScript

A Zeno int is a JavaScript number while it is a safe integer (below
2^53), and a `BigInt` beyond that, held to 64 bits by the runtime. A
float is a `Float` object, so that `2.0` stays a float.

**Why.** Almost every int a program uses is small, and a number is many
times faster than a `BigInt`. A `BigInt` for every int was simpler and
three to ten times slower. A float has to be told from an int (`2.0`
prints as `2.0`), and JavaScript has one kind of number.

### 24.7 Variables an inner function shares, in Python

A variable that an inner function uses is kept in a `Cell` (a new one
each time its block starts, so each round of a loop has its own), and
the inner function is handed its cells as keyword parameters of its own
(`def f(x, *, total=total)`). A function made inside an expression is
written as a `def` on the lines before the statement.

**Why.** Python's closures share one variable across every round of a
loop, where Zeno gives each round its own; handing the cell over when
the function is made gives Zeno's rule. A `lambda` holds one
expression and a Zeno function holds statements, so it must be a `def`;
making a function has no effect that can be seen, so it can be made a
moment early.

### 24.8 What a target cannot do: board, web, unsafe

A program that uses `import board`, `import web` or `unsafe` is
refused with a message naming the line.

**Why.** The pins of a board, the page of `--web` and manual memory
belong to a program compiled for that place. Pretending (a pin that
does nothing) would print something `zeno run` never does.

### 24.9 `tools/targets.py`: every target held to `zeno run`

The tool runs every example, every sample of the book and the corner
programs in `tools/targets/` with `zeno run`, then builds each for each
target, runs it with that language's own tools and compares what it
printed and whether it stopped with an error. A weekly job on GitHub
runs it on the newest version of every language.

**Why.** "Prints what `zeno run` prints" is a claim about 300 programs
and a dozen languages; only a machine can check it, and only on the
newest versions can it catch a language changing under us.

### 24.10 `--target html`: a page, built on the JavaScript

`--target html` writes one `.html` file with the JavaScript program and
its runtime in a `<script>`. What the program prints goes into the
element `zeno-output` (a `<pre>` is added if the page has none); an
error is shown under it. The program's own page (`counter.html` beside
`counter.zeno`) is used if there is one, and Zeno refuses to write over
it. A page cannot write files or `sleep`, and says so when the program
tries; `input` asks with the browser's question box, as in `--web`.

**Why a page and not "HTML".** HTML is not a language a program can be
written in: it describes a page. What a person asking for "Zeno in
HTML" wants is a file that opens in a browser and runs the program,
and that is a page with the program in it, in JavaScript, which is the
browser's language.

**Why one file.** It opens from a disk, an email or any host, with
nothing beside it to lose.

**Why beside `--web` and not in place of it.** `--web` runs Zeno itself
in the page, so `import web` works and the program can answer buttons;
it is a bigger file and the program runs at `zeno run` speed.
`--target html` is the program compiled: small and fast, but without
`import web` for now.

**Why `sleep` is refused.** A page does one thing at a time; a program
that slept would freeze it. `--web` refuses `sleep` for the same reason
(14.11).

### 24.11 The order of the targets

JavaScript first, since the page is built on it and Node runs it
everywhere; then HTML; then Python, whose model (one kind of int that
never overflows, dicts that keep order) is closest to JavaScript's and
so tests the shared lowering against a second language early; then Go,
Java, C#, C++ and Rust, the statically typed ones; then WebAssembly.

**Why Python earlier than the order first suggested.** It shares the
most with JavaScript, so it was the cheapest way to find what in the
lowering was really JavaScript's; it found nothing, which is the
evidence that the lowering is not tied to one language.

### 24.12 Go, Java and C#: every value is the language's "anything"

In Go every Zeno value is an `any`, in Java and C# an `Object`: a
whole number is a 64-bit `int64`/`long`, a float a `float64`/`double`,
and a list, dict, object or function a class of the runtime. Each
operator is a call of the runtime that looks at what it was given, as in
JavaScript and Python.

**Why not the language's own types.** The checker often knows that a
variable is an `int`, and could write `int64`. But a Zeno program also
has values whose type is only known as the program runs (`json.parse`,
a list of mixed things, an `Optional`), and both kinds have to meet in
the same call. One kind of value everywhere keeps the printer small and
the three runtimes the same as the other two. Typed code where the type
is known is the step for speed after this one (24.2).

### 24.13 What each of the three languages needed

- **Go** refuses a variable that is never read, so every variable is
  also given to `_` once (`_ = total`). Go has no `try`: Zeno's `try`
  is a call of `Ztry` with two functions, the body and the handler,
  which answer with a number for what happens next (go on, `break`,
  `continue` or `return`), and the code after the call does it.
- **Java** lets a lambda use only a variable that never changes, so a
  variable an inner function uses is kept in a `Z.Cell`, as in Python
  (24.7). Java refuses a statement it can prove is never reached, so
  `return x` is written `if (true) return x;`. And Java reads `\u000a`
  in a string as a new line in the source, so a character below a space
  is written as an octal escape.
- **C#** needs neither: a lambda can change a variable outside it, and
  a statement never reached is only a warning, which the file turns
  off. It refuses a name in an inner block that one outside has, which
  the lowering's unique names (24.4) already rule out.

**Why the lowering stays the same.** All three are things a printer can
do by itself. The one change the lowering needed is 24.14.

### 24.14 The order arguments are worked out in

Zeno works out a call's arguments from left to right. JavaScript,
Python, Java and C# do too; Go and C++ do not promise it. For those two
the lowering copies every argument before the last one that calls
something into a variable of its own, first, in Zeno's order: so in
`show(count, next())` the `count` that is shown is the one before
`next()` ran, as in Zeno, even if `next` changes it.

**Why copy plain variables too.** Go reads a variable whenever it
likes, so a plain `count` beside a call is not safe either; the first
version left plain variables alone, and `examples/labels.zeno` printed
the wrong count in Go.

### 24.15 Nesting: 1000 calls, in every target

`zeno run` stops a program whose calls nest more than 1000 deep, with a
message that asks if the function calls itself forever. The Go, Java
and C# runtimes count calls the same way and stop with the same
message, and they run the program on a thread with a big stack so that
1000 calls always fit.
(JavaScript and Python count too since 24.23.)

**Why.** A program that recurses forever should end the same way in
every language, with Zeno's message, not with a Go "goroutine stack
exceeds" or a Java `StackOverflowError`.

### 24.16 The last digit of `sin`, `exp` and `pow`

`zeno run` takes `math.sin`, `math.exp`, `math.pow` and the other
functions of floats from the C library of the computer. Python, C#, C++
and Rust use the same library, so they print the same digits. Others
have their own: JavaScript's (in Node) and Java's `StrictMath` give a
different last digit for about one input in ten, and Go's own `pow` for
about one in three. To keep the difference as small as can be:

- Go uses its own `sqrt`, `exp` and `log` (which agree with the C
  library on the programs here) and, for `sin`, `cos` and `tan`, a copy
  of the functions JavaScript uses, written in Go in runtime.go.
- Java uses `Math`, not `StrictMath`: `Math` is allowed to use the
  computer's own, which agree far more often.

Where the last digit can still differ, it is listed as a known limit
(in `docs/targets.md`): the answer is right to the 16th digit, but
the 17th, which a float's printing shows, may be one off.

**Why not exact everywhere now.** Making every runtime give the
correctly rounded answer means writing those functions in each language
with extra precision, a project of its own. None of the 319 programs
shows a difference; the fix waits until a program needs it.

### 24.17 C++: a small value, shared pointers, cells

In C++ a Zeno value is a `Z::V`: a tag, a union for an int, a float or
a bool, and a `shared_ptr` for anything else (a str, a list, an object).
A value is freed when the last pointer to it goes. A variable an inner
function uses lives in a `Z::Cell` (a shared box), because a lambda
written `[=]` copies what it uses. Arguments are put in variables first
(24.14). The file builds with any C++17 compiler and needs nothing
beside it.

**Why shared pointers and not a collector.** They free almost
everything at the moment it is no longer used, with nothing to add to
the file. What they miss is a cycle, such as a list that holds itself;
a program that makes many of those would grow. None of the 319 does,
and a collector can come later without changing the printer.

**Why `[=]` and cells rather than `[&]`.** A lambda that kept references
to the variables around it would point at nothing once the function
that made it returned, and a Zeno function can outlive the one that
made it.

### 24.18 Rust: failures as results, `try` as a closure

Rust has no exceptions. Every step of runtime.rs that can fail gives a
`z::R`, the value or the failure (an error on its way to a `try`, or a
mistake that stops the program), and the program passes a failure on
with `?`. A `try` body is a closure called at once; the code after it
looks at how it ended. A `break`, `continue` or `return` in the body
cannot reach outside the closure, so the body ends with which one it
was and the code after it does it, as in Go (24.13). A variable an inner
function uses lives in a `z::Cell`, and each function gets its own copy
of the cells it uses where it is made: the lowering lists them for each
function (`captures`).

**Why results and not panics.** A panic can be caught, but it is meant
for bugs: it prints to the screen unless told not to, and it is slow to
catch. A result costs nothing until something fails, and `?` keeps the
written program as short as the others.

**Why the same printing.** Zeno is written in Rust, so the Rust runtime
shows floats with `{:?}`, fixed decimals with `{:.2}`, and text with
the same quoting, exactly as `zeno run` does. It is the one target that
cannot differ in the last digit.

### 24.19 WebAssembly: through Rust, with WASI

`--target wasm` writes the program in Rust, has `rustc` build it for
`wasm32-wasip1`, and keeps the `.wasm` file. WASI is the system
interface that WebAssembly hosts outside a browser give a program: the
keyboard, the screen, the command line and files. The file runs in
wasmtime, wasmer, or Node (`tools/run-wasi.js`). WebAssembly has no
threads, so there the program runs on its one thread, built with a
32 MB stack, which holds 1000 nested calls.

**Why through Rust.** Rust builds WebAssembly well and has done so for
years, and the Rust target already prints what `zeno run` prints; a
WebAssembly printer of its own would be a second compiler to keep in
step. This is also the one target where `zeno build` runs a compiler
itself, because a `.wasm` file is not text a person would keep: what
they want is the file that runs.

**Why WASI and not the browser.** The page target (24.10) is already
the browser's. `zeno build --web` puts Zeno itself in a page as
WebAssembly. What was missing was a program that runs anywhere a
WebAssembly host does, the same file on every computer, and that is
WASI.

### 24.20 Text in quotes: Zeno's own rule

A text shown inside a list, a dict or an object, or named in the
message of an error, is in quotes. Inside them `"`, `\`, the new line,
the carriage return and the tab are written as a program writes them
(`\"`, `\\`, `\n`, `\r`, `\t`). Any other control character (below a
space, and U+007F to U+009F) is written as its code, `\u{1b}`, because
it would not show on a screen. Every other character is written as it
is.

Until now `zeno run` used Rust's `{:?}` for this, which escapes
whatever Rust's tables of Unicode call "not printable" or "extending".
That made `["தமிழ்"]` print as `["தமிழ\u{bcd}"]`: the last sign of
the word is a combining one. The C that `zeno build` writes escaped
five characters and nothing else, and the runtimes of the targets
already followed the rule above. Three ways of running one program
gave three answers. Now `zeno run`, the C runtime and every target
quote a text the same way.

**Why not Rust's rule everywhere.** It needs Unicode's tables, which
change with each version of Unicode, so the answer would depend on
the version of Rust that built Zeno, and each runtime would need a
copy of the tables. It also breaks words in Tamil, Hindi, Thai and
every script that uses combining signs. `strip` and `upper` already
keep to ASCII so that no table is needed (3.14); this is the same
choice.

**Why escape control characters at all.** A list is shown to be read.
An escape character or a zero-width control inside a text would
change what the terminal does, or show nothing, and the reader could
not tell `"a"` from `"a\u{1b}"`.

**Rejected.** Escaping everything that is not ASCII: exact, but a
program that prints a list of names in Tamil would print codes.

### 24.21 A float between two shortest texts takes the even one

A float is printed with the fewest digits that name it and no other
float. Now and then two texts of that length are equally near: the
float `1e15 + 0.25` lies exactly between `1000000000000000.2` and
`1000000000000000.3`. Zeno prints the one whose last digit is even.

**Why.** C, JavaScript, Python, Java and C# do, and so did the C that
`zeno build` writes. `zeno run` printed the upper one, because Rust
does, so `zeno run` and `zeno build` disagreed, and so did five of the
targets. The even digit is also what the rest of Zeno does with a
half: `{x:.1f}` rounds the exact value to even. It can only happen to
a float that needs 16 or 17 digits. `value::float_text` puts Rust's
answer right in that one case; the Rust target carries the same
function.

**Rejected.** Teaching five runtimes to round up as Rust does: more
code in more places, for the rarer habit.

### 24.22 A variable is read when its turn comes

Zeno works a line out from left to right, and reads a variable at the
moment its turn comes. The lowering (24.3, 24.14) now keeps to that in
four places where it did not:

- `f(b=two(), a=one())` runs `two()` first, as written. On JavaScript,
  Python, Java, C# and Rust it ran `one()` first, because the
  arguments had already been put in the order of the parameters.
- `x += f()` reads `x` before `f()` runs, so a change `f` makes to `x`
  is lost, as in `zeno run`. Go and C++ read `x` after.
- `[x, bump()]`, `x < bump() < y`, `f"{x} {bump()}"`, `[x: bump()]`:
  wherever a later part needs statements of its own (24.14), an
  earlier plain variable is copied first. Before, only earlier calls
  were copied, and a variable was read after the later part had
  changed it, on every target.
- `list[i] = value` and `object.field = value` work out the list, the
  index and the value in that order.

**Why.** It is the rule of `zeno run`, and "it prints what `zeno run`
prints" has no exceptions. None of the 319 programs changed a variable
in the middle of a line that also read it, so none showed it.

**Rejected.** Copying every variable always: right, but each line of
the written program would grow, and most lines change nothing midway.
A variable is copied only when something after it on the line can run
code.

### 24.23 Nesting is counted in JavaScript and Python too

24.15 had JavaScript and Python stop "where their own limit is". That
is not the same program: `zeno run` stops at the 1000th nested call,
with the name of the function and the line, and JavaScript ran more
than 10,000 deep before it stopped with another message. Now both
count as the other targets do: a call of a function of the program is
`Z.direct(f, line, ...)`, which counts it in, calls it, and counts it
out again whether it returned or raised.

**Why.** A program that prints how deep it got, or that recurses 5000
deep on purpose, did something else in two of nine targets.

**Cost.** A call in Python is slower by about a third (`fib`: 2.3 s to
3.5 s), in JavaScript a little (0.12 s to 0.16 s). Correct first
(24.2).

### 24.24 Corners of the library, pinned down

The review of the targets found places where `zeno run` did whatever
the Rust it is built with does, and a target did what its own language
does. Each now has a rule, which `zeno run` and all targets follow:

- **`lines()`.** A line ends at `\n`. A `\r` just before that `\n`
  goes with it. A `\r` anywhere else, and one at the very end with no
  `\n` after it, stays in its line. It is written out in
  `builtins.rs`, not left to Rust's `str::lines`.
- **`input()`.** The same for the line end. Bytes that are not UTF-8
  become U+FFFD the way Rust, Python and browsers do it: one for each
  run of bytes that began a character and did not finish it, one for
  each byte that could not begin one.
- **`range`.** A step past the last int ends the range. `zeno run`
  added the step without looking, which is a stop in a debug build
  and a wrong, endless list in a release build.
- **`math.floor`, `math.ceil`.** Always an int: too big a float gives
  the biggest int, not-a-number gives 0, as `math.round` and
  `zeno run` do. Five targets gave the float back.
- **`//` and `%` of floats.** `-0.0` is kept where the arithmetic
  gives it (Python lost it).
- **JSON.** The four hex digits of `\u` are digits (`zeno run` took
  `\u+041`); the second half of a pair is DC00 to DFFF; the blanks
  before a text that is not JSON, which the message leaves out, are
  JSON's four.
- **`files`.** A path with nothing in it is "no file". A path through
  a file (`notes.txt/x`) "cannot be read".

**Why list them.** Each was found by a program written to look for it
(`tools/targets/edge-*.zeno`), not by a user. They are small, but a
rule that is whatever one compiler happens to do is not a rule
(principle: unambiguous).

### 24.25 File names, and names in capitals

- A program may have any file name. The Rust file names its own crate
  (`#![crate_name]`), so `my prog.rs` and `2048.rs` build; the command
  that `zeno build` prints has `-o`, and quotes a name a shell would
  split.
- `go run` refuses a file that ends in `_test.go` and does not see one
  that begins with `_` or `.`. For those, the Go file gets `program`
  in its name: `speed_test.zeno` is written as
  `speed_test_program.go`.
- Python looks for a module in the program's folder before its own
  library. A program called `struct.py` or `re.py` was taken for
  Python's module of that name. The runtime takes the folder off the
  list before it imports anything.
- A name in capitals may be a macro of a C header (`ERANGE`, `SIGINT`,
  `OVERFLOW`). The C++ file says `#undef` for each such name of the
  program, after the runtime and before the program.
- The words the list of 24.4 had missed (`args`, `delegate`, the
  sized ints of C++ and others) are in it.

**Why `#undef` and not a longer list.** The macros differ from system
to system and there are hundreds; no list is complete. After the
runtime nothing needs them.

### 24.26 The check holds the error to `zeno run` too

`tools/targets.py` (24.9) compared what was printed and whether the
program stopped with an error. Now, when it stops, the message, the
file and the line must be the same too. It also takes a corner
program in several files (a folder with `main.zeno`), the words a
program is started with (`.args`), and typed input that is not text
(`.stdin` is bytes).

**Why.** "The same error" was promised (24.2) and not checked. The
nesting difference of 24.23 would have been caught the first day.

### 24.27 TypeScript: the JavaScript, with the type `any`

`zeno build --target ts` writes the program as TypeScript. It is the
program `--target js` writes, with a type on every variable, parameter
and caught error, after `src/runtime.ts`, which is `src/runtime.js`
with types. The type is `any`. The file passes `tsc --strict`, and
Node (22.18 or newer) runs it as it is; `tools/targets.py` does both
for every program.

**Why `any`.** What a Zeno value is, is found out while the program
runs, by the runtime, in every target (24.2): that is how an int that
overflows, a wrong unit or a missing field gives Zeno's message at
Zeno's line. Go has `any` for this, Java `Object`, Rust an enum.
TypeScript's types say what JavaScript would do with a value, and Zeno
does not do what JavaScript does (`1 + "a"` is an error, an int is 64
bits). A type like `number` on a Zeno int would be a promise the
program does not keep.

**Why a typed copy of the runtime, and not `// @ts-nocheck`.** A
TypeScript project that takes the file should not have to switch its
checker off for it. The copy costs little: a test
(`the_typed_runtime_is_the_runtime_with_types`) takes the types out of
`runtime.ts` and holds what is left to `runtime.js`, line by line, so
the two cannot drift and what is tried for JavaScript holds for
TypeScript.

**Why the file is a module** (it ends in `export {};`). In a
TypeScript project every file that is not a module shares one set of
names, so two built programs would both declare `Z` and neither would
pass. A module's names are its own.

**What changed in the JavaScript for it.** The runtime took Node's
file module with `require("fs")`, which a module does not have. It now
asks Node for it by name (`process.getBuiltinModule("fs")`), which
works in a script and in a module, so the `.js` file also runs where
`"type": "module"` is set. It needs Node 22.3 or newer (20.16 in the
20 line); the oldest Node still kept up is 22.

**Alternatives rejected.** Real types worked out from the Zeno types
(a second printer and a second runtime, and still wrong at the edges:
a Zeno `int` is not a `number`). Running `tsc` in `zeno build` and
keeping only the `.js` (then it is the `js` target).

### 24.28 PHP: its own ints and floats, lists in objects, `use (&$x)`

`zeno build --target php` writes a `.php` file that `php` runs (8.0 or
newer), with `src/runtime.php` at its top. It is a port of the
JavaScript runtime, and holds all 337 programs to `zeno run`.

- **An int is a PHP int, a float a PHP float.** PHP keeps the two
  apart (`3` and `3.0` are different values), and its int is 64 bits,
  so neither needs a box. PHP turns `+`, `-` and `*` that do not fit
  into a float instead of wrapping; the runtime sees that the answer
  is no longer an int and raises `OverflowError`.
- **A list and a dict are objects** (`ZList`, `ZDict`) around a PHP
  array. A PHP array is copied when it is handed on; a Zeno list is
  shared, and `xs.append(1)` inside a function must be seen outside.
  PHP makes the key `"1"` the int `1`; Zeno keeps them apart, so a str
  key is stored with an `s` before it.
- **An inner function names what it uses**: `function ($x) use
  (&$total)`. The `&` shares the variable. A PHP variable lasts for
  its whole function, so a shared variable declared in the body of a
  loop is `unset` where its block begins: the next round's is then a
  new variable, and the functions made in the round before keep
  theirs, as in Zeno.
- **No extension is needed.** `mbstring`, `ctype`, `bcmath` and `gmp`
  are not in every PHP. Characters are counted and split with `preg`
  (always there) and by hand, and `{x:.2f}` works out the exact
  decimal text of the float with plain ints, a half going to the even
  digit as in `zeno run`.
- **A long chain is let go in a loop.** PHP frees an object's fields
  from inside freeing the object, so a chain of 200,000 objects
  crashed it. `ZInstance` and `ZCase` hand their fields to a list
  that one loop empties, as the Rust runtime does.
- **PHP's settings are fixed at the start**: no memory limit, floats
  written with all their digits, and a PHP warning stops the program
  as a fault of the runtime instead of being printed among the
  program's lines.

**Alternatives rejected.** Cells for shared variables, as the Python
target has (`$total->v` everywhere, where PHP has a word for exactly
this). `mb_strlen` and `number_format` (the first is not always
installed, the second rounds a half up after rounding the float
first, which is not what `zeno run` prints).

### 24.29 Zeno is built as one piece, so its speed does not move when code is added

`Cargo.toml` has `codegen-units = 1` for a release build: the Rust
compiler works on all of Zeno at once.

**What happened.** 0.5.0 added the targets, about 20,000 lines that
nothing runs unless `--target` is given. No line of the VM changed,
and `bench/run.py --count` showed it: within two in a hundred of
0.4.0's instructions, fewer on some programs and more on others. But
timed, in four rounds on a quiet machine, `zeno run --vm` in the 0.5.0
build was slower than in 0.4.0's on every benchmark: fib 7 %, loop
5 %, sort 9 %, strings 1 %, trees 4 %, nbody 4 %. The built programs
were not touched by it (their C is the same), nor was plain `zeno
run`.

**Why.** By default the compiler cuts a program into sixteen pieces
and works on each alone. Which functions land in one piece, and so
which can be folded into which and what ends up beside what in the
file, depends on everything else in the program. Adding code that
never runs moved the VM's loop and its helpers about, and the
processor's guesses and its cache did a little worse on the same
instructions.

**With one piece** there is no cutting, so nothing unrelated decides
it. The same source then runs fewer instructions than 0.4.0 (fib 2 %
fewer, sort 1 %, strings 1 %) and, timed the same way, is within two
in a hundred of 0.4.0 on five benchmarks (loop −2 %, sort −1 %,
strings −2 %, trees −1 %, nbody −1 %) and 3 % above on fib, which is
two hundredths of a second. Building Zeno takes 47 seconds where it
took 36. The program is half a megabyte smaller.

**The rule this is for.** Partha, 2026-11-17: writing a program in
other languages is a flag, and Zeno itself must stay as fast as it
was. A count of instructions is not enough to show that; the check
before a release that adds code is now both, the count and timed
rounds against the release before.

**Alternatives rejected.** Moving the targets into a crate of their
own (the VM's crate still changes with every other feature, so the
same thing would come back). Whole-program `lto` on top of it (Zeno has no
crates beside itself, so there is little for it to join; it took a
minute to build and was not timed). Leaving
it, as within the noise of this machine: it was the same direction on
all six programs, four rounds running.

### 24.30 Lua: its own ints, a `none` of its own, `try` as a coroutine

`zeno build --target lua` writes a `.lua` file that `lua` runs (5.4),
with `src/runtime.lua` at its top. The target's name is the
language's, in full, as `rust` is: `lua`, `ruby`, `dart` and `kotlin`
are short already, and `rb` or `kt` would have to be learnt.

- **An int is a Lua integer, a float a Lua float.** Lua 5.4 keeps the
  two apart and its integer has 64 bits. Its `+`, `-` and `*` wrap
  around when the answer does not fit; the runtime looks for that
  (the signs of the two sides and of the answer) and raises
  `OverflowError`. That is why it is 5.4 and not LuaJIT or 5.1, where
  every number is a float.
- **`none` is `Z.none`, not `nil`.** A Lua table cannot hold `nil`:
  `{1, nil, 3}` has a hole and no certain length. A Zeno list can
  hold `none`, and a dict can have it as a value. So `none` is one
  value the runtime makes, and `nil` never reaches the program.
- **A list is a table from 1, a dict a table of keys in order and a
  table of values.** Lua's own table forgets the order keys were
  added in, and Zeno's dict keeps it.
- **`try` is a function handed to `Z.try`.** Lua has no `try`; it has
  `pcall`, which calls a function and says whether it failed. A
  `return`, `break` or `continue` inside that function cannot leave
  what is around it, so the function gives back how it ended (1, 2, or
  3 and a value) and the lines after the call do it, as in Go (24.13).
- **The body runs as a coroutine.** `pcall` may be inside itself about
  190 times and then Lua stops with "C stack overflow". A Zeno program
  may be 999 calls deep with a `try` in each (24.15). So `Z.try` does
  not call the body: it hands it to the loop in `Z.start`, which runs
  it as a coroutine of its own while the one that asked waits. However
  many `try`s are inside one another, Lua sees one `pcall`.
- **`continue` is `goto`.** Lua has `goto` and no `continue`; the
  label is at the end of the loop's body.
- **Past 100 variables, a block keeps its own in a table.** A Lua
  function may have 200 `local`s, and what an expression is worked out
  in counts toward a limit of 255. A program is one function, and a
  generated one can pass that.
- **Time.** Lua has `os.time`, in whole seconds, and no `sleep`. If
  LuaSocket is there the runtime uses it. If not, `time.now()` is
  `os.clock`, the time the processor worked, and `sleep` counts it.
  A wait that keeps the processor busy is a poor wait; the other
  choices were to stop the program with a message (then a program
  that sleeps cannot be written in Lua at all) or to start `sleep`,
  the program of the system, for each wait (not there on Windows, and
  slow).

**Alternatives rejected.** `nil` for `none`, with a list that keeps
its own length (every step that reads a list would have to know).
`pcall` as it is, with the limit of 190 written down (a limit `zeno
run` does not have). A `repeat ... until true` around a loop's body
for `continue` (a `break` inside it then means `continue`, and the
real `break` needs a flag).

### 24.31 Ruby: lambdas, loops that are blocks, and room for calls

`zeno build --target ruby` writes a `.rb` file that `ruby` runs (3.0
or newer, no gem), with `src/runtime.rb` at its top.

- **A Zeno function is a lambda**, `->(r) { ... }`. Ruby has three
  things like a function. A `def` does not see the variables around
  it, and a Zeno function does. A `proc` takes any number of arguments
  and its `return` leaves the method around it. A lambda takes the
  arguments it names and its `return` leaves the lambda.
- **An int is a Ruby Integer.** It grows without end, so each step
  looks whether the answer still fits in 64 bits, as the Python
  runtime does.
- **`for` is a block**: `Z.each(xs, line) do |x| ... end`. A block's
  parameter is a new variable each round, so the functions made in
  three rounds see three values. Ruby's own `for x in xs` has one `x`
  for all rounds.
- **`while` is Ruby's `while`, unless its body declares a variable an
  inner function uses.** A variable first set inside a Ruby `while`
  belongs to what is around the loop: one for all rounds. So such a
  loop is written `Z.loop do ... break unless cond ... end`, a block,
  whose variables are new each round. Only such a loop, because a
  block costs a call each round and most loops make no function.
- **A name that begins with a capital gets `z_` before it.** In Ruby
  that is a constant: it cannot be set inside a function, and setting
  it twice is a warning. `Total` becomes `z_Total`. This is done in
  the Ruby printer and not in the shared list of names, since no other
  target minds.
- **`#` in a text is written `\#`.** `"#{x}"` in Ruby fills in `x`.
- **The program starts itself once more.** Ruby gives a program one
  megabyte for calls inside one another, and 999 Zeno calls, each
  some Ruby calls, can pass it. How much there is, is decided before
  Ruby starts (`RUBY_THREAD_VM_STACK_SIZE`), so the runtime starts the
  same file again asking for 64 megabytes. If that cannot be done, the
  program runs with what it has.

**Alternatives rejected.** Cells, as in Python (Ruby's blocks already
do it). Every `while` as a block (slower for nothing). Running the
program in a `Thread` with a bigger stack (a thread's stack size
cannot be set in Ruby). Telling the reader to set the variable (the
file must run by `ruby hello.rb` and nothing more).

### 24.32 Dart: `Object?`, one list of arguments, and the VM only

`zeno build --target dart` writes a `.dart` file that `dart run` runs
(Dart 3 or newer), with `src/runtime.dart` at its top.

- **Every value is an `Object?`.** Dart checks types before the
  program runs, and what a Zeno value is, is found out while it runs
  (24.2). `Object?` is Dart's `any`: `dynamic` would let a wrong call
  through to fail with Dart's message, and with `Object?` every use
  goes through the runtime, which gives Zeno's.
- **A function takes one list.** `(List<Object?> z_a) { Object? r =
  z_a[0]; ... }`. All functions then have one type, and a call with
  names, defaults or a wrong count is lined up by the runtime, as in
  Go, Java and C#.
- **For the Dart VM only.** There an `int` has 64 bits and wraps, and
  the runtime sees the overflow. Compiled to JavaScript for a page, a
  Dart `int` is a JavaScript number, and the checks would be wrong.
  `--target html` is the target for a page.
- **`$` in a text is written `\$`.** `"$x"` in Dart fills in `x`.
- **`log10` is worked out as the C library does it.** Dart has `log`
  and no `log10`, and `log(x) / ln10` is one digit off for some
  inputs (`log10(1000)` gives `2.9999999999999996`). The runtime does
  the steps of the C library's `log10`, so it prints what `zeno run`
  prints.
- **Printing waits.** Dart's `stdout.write` hands the text to be
  written later, and a program that stops with an error could lose
  its last lines. The runtime writes the bytes itself and waits for
  them.

**Alternatives rejected.** `dynamic` (above). Real Dart types from the
Zeno types (a Zeno `int` that overflows is still not a Dart `int`,
and a second printer for every target is not worth what it gives).

### 24.33 Kotlin: the Java runtime in Kotlin, and `fun`, not a lambda

`zeno build --target kotlin` writes a `.kt` file. `kotlinc` compiles
it to a `.jar` and `java` runs that. `src/runtime.kt` is
`src/runtime.java` in Kotlin, for the same machine: an int is a
`Long`, a list an `ArrayList`, a dict a `LinkedHashMap`, and `sin` and
`exp` are Java's `Math` (24.16).

- **No cells.** A Java lambda may only read the variables around it,
  so the Java target keeps a shared variable in a `Z.Cell` (24.13). A
  Kotlin function may change them, and a variable declared in a loop's
  body is new each round. The program is plain variables.
- **A Zeno function is `fun(z_a: Array<Any?>): Any? { ... }`.** Kotlin
  has a shorter way, a lambda `{ a -> ... }`. But a plain `return` is
  not allowed in a lambda, and its value is its last line. A `fun`
  with no name has `return` as Zeno has it.
- **A block of its own is `if (true) { ... }`.** `{ ... }` alone is a
  lambda in Kotlin, made and never called.
- **The file turns Kotlin's warnings off** (`@file:Suppress`). A Zeno
  function that ends without a `return` gives `none`, so there is a
  `return null` after its last line, which Kotlin calls unreachable
  when that line was a `return` too; and a `var` that is set once
  "could be `val`". Both are what the rules write, as in C# (24.13).
- **The program runs in a thread with room for calls**, as in Java
  (24.15): 999 Zeno calls, each some calls of the runtime, do not
  always fit in what a JVM gives its first thread.
- **The program comes first in the file, the runtime after**, as in
  Java and C#, so the reader finds `main` at the top.
- **`$` in a text is written `\$`**, as in Dart.

**The cost.** `kotlinc` takes about ten seconds for any program,
because the runtime, some 2,200 lines, is compiled with it each time.
A jar of the runtime made once would be faster, and then the `.kt`
file would not run by itself, which every target's file does (24.2).

**Alternatives rejected.** Calling the Java runtime from Kotlin (two
files and a build). Kotlin for JavaScript or native (other machines,
with other ints and other limits; one Kotlin is enough to say the
program in Kotlin).

### 24.34 The words of the four languages, and a program that tries them

The list of words that get a `_` (24.4, 24.25) took those of Lua
(`end`, `local`, `then`, `until`, `repeat`, `elseif`), Ruby (`begin`,
`rescue`, `ensure`, `unless`, `redo`, `retry`, `alias`, `undef`,
`defined`, `elsif`, and `_1` to `_9`, which in a Ruby block mean its
arguments), Dart (`late`, `required`, `factory`, `mixin`, `on`,
`show`, `hide`, `part`, `library`, `external`, `covariant`,
`deferred`, `extension`, `rethrow`, `sync`, and `List` and
`identical`, which the printed program itself uses) and Kotlin
(`fun`, `val`, `it`, `typealias`, `arrayOf`, `booleanArrayOf`).

It is still one list for every target, so a variable `end` is `end_`
in JavaScript too, where it need not be. That keeps a program's names
the same in every language it is written in (24.4), and it means the
old targets are checked again with the new words: all of them still
print what `zeno run` prints.

`tools/targets/edge-names-2.zeno` is the twenty-third corner program.
It has a variable for each of about a hundred such words and for
names that look like the runtimes' own (`Z`, `ZRaise`, `z_ok`, `z_a`,
`_G`, `self`, `it`); variables, a class, its fields and a function
with capitals; texts that say `$5`, `${x}`, `#{y}`, `#$z`, `#@w` and
`[[x]]`; functions made in the rounds of a `while` and of a `for`;
`break`, `continue` and `return` from inside a `try` inside a loop; an
error raised again through the `try`s of calls inside one another; and
a function with 230 variables. The Lua coroutines, Ruby's `Z.loop` and the table of
variables each came from a program like these failing first.

### 24.35 Multiply, then add, is two steps in C++ and Go too; and the check fails when a program differs

The first run of the weekly check with the four new languages
(2026-11-17) showed a line that was not theirs: with the newest g++
(15.2), `tools/targets/edge-ints.zeno` printed in C++

```
1e308 % 3.0    4.9896007738368e291     where `zeno run` prints 0.0
7.0 % 0.1      -3.885780586188048e-16  where `zeno run` prints 0.0
1 % 0.1        -5.551115123125783e-17  where `zeno run` prints 0.0
```

**Why.** `a % b` on floats is `a - b * floor(a / b)`. Zeno multiplies
and rounds, then takes away and rounds. A processor with a fused
multiply-add does both in one instruction and rounds once, and what
is left is the tiny error of the product, where two steps leave 0. A
C++ compiler may pick that instruction without being asked, and that
g++ did. The C of `zeno build` has said "two steps" to its compiler
since the first release (decision 12.5); `runtime.cpp` did not.
With g++ 13 on the same kind of machine nothing showed, because that
compiler was not using the instruction.

**The rule.** `src/runtime.cpp` now says it to each family of
compiler (`#pragma STDC FP_CONTRACT OFF` for clang, `#pragma GCC
optimize("fp-contract=off")` for g++, `#pragma fp_contract(off)` for
Microsoft's), for the whole file, the program included. In
`src/runtime.go` the product in `%` is written `float64(y * q)`: Go
may fuse too, on ARM64, and its rule is that a conversion written out
rounds first. `edge-ints.zeno` has a line of `a * b + c` in a
function, for every target. The other languages take two steps by
their own rules (Java, Kotlin, C#, Rust, JavaScript, Dart) or run
each step as an instruction of their own (Python, PHP, Lua, Ruby).

**Not done.** The Go runtime's own `sin`, `cos` and `tan` (24.16) are
sums of products, and on ARM64 Go may fuse those: a last digit could
differ there. It has not been seen and cannot be tried here; it is
with the known limits.

**The check did not fail.** The step was `python3 tools/targets.py |
tee targets-report.txt`, and without `pipefail` a step has the status
of its last command, `tee`. So the run was green with a failed
program in its report. The step now runs under `bash` with
`pipefail`, and is red when any program differs.

**Alternatives rejected.** `-ffp-contract=off` in the command the
user is told to run (the file must be right by itself, however it is
compiled). A `volatile` in `%` only (a compiler that fuses there may
fuse elsewhere in the file).

## 25. Starting another program

Partha asked (2026-11-17) for a program in Zeno that downloads a video
from YouTube. [`examples/download.zeno`](../examples/download.zeno) is
that program, and `program.run` is what Zeno needed for it.
[`running-programs.md`](running-programs.md) is the walkthrough.

### 25.1 The example starts yt-dlp, and does not fetch the video itself

`download.zeno` checks the address, starts
[yt-dlp](https://github.com/yt-dlp/yt-dlp) with the right words, and
says how it went.

**Why.** How YouTube hands out a video is not written down anywhere,
and it changes every few weeks. yt-dlp is kept working by many people
who follow each change; the same thing written in Zeno would work for
a month. It would also need an encrypted connection, which Zeno has
no library for. What the example shows instead is something nearly
every real program does sooner or later: it uses a tool that is
already there.

**What its first lines say.** That it is for videos the user has the
right to keep: their own, one whose licence allows it, or one whose
owner agreed. YouTube's terms of service allow no other download, and
a reader of the example should not have to find that out afterwards.

**Alternatives rejected.** Fetching the video in Zeno (above). An
`http` module first: the larger part of it is the encryption, and the
video would still be out of reach.

### 25.2 `program.run(name, words)` starts a program, waits for it, and gives the number it ended with

```zeno
import program

let ended = program.run("yt-dlp", ["--paths", folder, address])
if ended != 0 {
    print("that did not work")
}
```

**Why a number.** Every program on Linux, macOS and Windows ends with
a whole number, and 0 means that all went well. A `bool` would throw
away what the other numbers say: `grep` ends with 1 for "nothing
found" and 2 for "could not look".

**Why it waits.** A line of a Zeno program is done when the next line
begins. A program that went on while another ran would need a value
for the one still running, and ways to ask it, wait for it and stop
it. That is left for later (25.7).

**Why in `program`.** `program.arguments()` is the words this program
was started with. `program.run` gives another program the words it is
started with, and if that program is written in Zeno, its
`program.arguments()` is exactly the list that was passed. The two
are one idea seen from its two ends, so they are in one module, and
`import program` is still the one line for "the programs on this
computer".

**Alternatives rejected.** A module of its own (`process`, `system`):
a module for one function, and a second name to learn beside
`program`. `run` as a plain word like `print`: it would take a common
name away from every program for something few of them do.

### 25.3 The words are a list, and no shell reads them

Each item of the list is one word for the other program, whatever is
in it.

**Why.** The other way is one line of text that a shell takes apart:
`system("yt-dlp " + address)` in C. Then a blank in a folder's name
makes two words of it, and an address with `&` or `;` in it is the
end of one command and the start of another, which may be `rm`. That
is one of the oldest ways a program is broken into, and it is there
whenever the text comes from outside, as an address does. With a list
there is nothing to quote and nothing to get wrong: `"my videos"` is
one word because it is one item.

A program that wants what a shell does asks for a shell, and it shows
on the line: `program.run("sh", ["-c", "ls | wc -l"])`.

**Alternatives rejected.** One str, split at blanks (then a blank
inside a word needs quoting rules, which differ between Windows and
the rest). Both a str and a list (two ways to write one thing, and
the shorter one is the one with the hole).

### 25.4 The other program uses this program's keyboard and screen

What it prints appears where this program's lines appear, and what is
typed while it runs goes to it. What this program printed before is
on the screen first.

**Why.** yt-dlp redraws a line of progress as it goes, and a tool
that asks a question can be answered. Nothing has to be kept in
memory, however much the other program prints. It is also what
happens when one program is started from another at a command line,
so it is what a user expects to see.

### 25.5 A program that cannot be started is a `FileError`, and one that ends badly is not

```
FileError: there is no program `yt-dlp` to run
FileError: cannot run `notes.txt`
```

**Why an error that is raised as a value.** Whether the program is
installed is known only when it is started, as whether a file is
there is known only when it is opened (6.14). So a function that
calls `program.run` says `raises FileError` or catches it, in a
`--strict` check too.

**Why `FileError`.** A program is a file that is looked for in the
folders the system keeps for programs, and what goes wrong is what
goes wrong with a file: it is not there, or it may not be used. A
kind of its own would be an eighth name for one function, and a
program that handles both would write two `except` blocks that say
the same.

**Why a number other than 0 is not an error.** The program was
started and ran, which is all `run` promised. What its number means is
the other program's business: for `grep`, 1 is an answer.

**Stopped from outside.** A program that was stopped by Ctrl-C or
`kill` has no number of its own. `run` gives 128 plus the number of
the signal, which is what a shell reports, so 137 for `kill -9`.

### 25.6 Only `zeno run` has it so far (provisional)

`program.run` works under `zeno run`, with and without `--vm`, and
under `zeno test`. `zeno build`, in C and in LLVM, and `zeno build
--target` refuse a program that uses it, with a message that names
the line and says that `zeno run` can run it. That is how `json` is
treated by `zeno build` (18.1). In a page it raises a `FileError`: a
browser starts no programs.

**Why provisional.** In C, starting a program is `fork` and `exec` on
Linux and macOS and `CreateProcess` on Windows, where the words have
to be joined into one line by quoting rules of their own, and the
list has to be read in each of the three ways memory is kept. For
`--target` it is ten runtimes. None of it is hard, and all of it
needs trying on three systems. The example that asked for `run` is a
script, and `zeno run` is how a script is run.

**Not provisional.** A firmware build will never have it: a board has
no other programs.

### 25.7 Left for later

- `program.run` in a built program and in the targets (25.6).
- Reading what the other program printed, and giving it text to
  read.
- Going on while the other program runs; stopping it; giving up
  after a time.
- The folder it starts in, and its environment variables.
- `program.exit(n)`: `download.zeno` can say in words that the
  download failed, and cannot yet end with a number that says so.

## 26. The libraries of other languages

Partha asked (2026-11-18): "Why should I leave Python?" A program in
Zeno should be able to use what `pip install` put on the computer, and
then the same for "every single language", done "without affecting the
existing": whatever runs today runs the same, and the rest is there
only "when we use some specific words, or import something". This
chapter is the plan and its first part, Python.
[`python.md`](python.md) is the walkthrough.

### 26.1 Three versions, in the order the languages allow

1. Python, JavaScript (npm) and Java.
2. The other languages with an interpreter of their own: Ruby, PHP,
   C#, Lua.
3. C, Go and Rust.

**Why this split.** The first two groups have a runtime that can load
a library by its name while it runs, so one way of working serves them
all: start that runtime beside the program and talk to it (26.2).
C, Go and Rust have none. A Rust crate is compiled into the program
that uses it, and its generic functions do not exist until then. For
those the way in is the C interface: Zeno declares the types of a
function and calls a compiled library. That is a different piece of
work, and it belongs to `zeno build`, which already writes C.

**Why Python first.** It is what was asked for, and its helper is the
shortest. The other two of the first version use the same lines.

### 26.2 Python runs beside the program, as a program of its own

`import python` does not put Python inside Zeno and does not turn
Python into Zeno. The first `python.module` starts the Python that is
installed, and the two programs talk.

**Why.** Zeno is one file with nothing to install beside it. Python
inside it would mean finding the Python library of each machine and
loading it, which differs between Linux, macOS and Windows and
between one version of Python and the next; a fault inside a library
written in C would then end Zeno too. Started as a program, any Python
3 that is on the machine works, with every package it has, and a crash
there is an error here.

**The cost.** Every field read and every call goes to another program
and back, some tens of microseconds each. A loop of a million small
calls is slow; a call that does real work, which is what a library is
for, does not notice.

**Alternatives rejected.** Python inside Zeno (above). Rewriting
libraries in Zeno, by hand or by a translator: most of what makes them
worth having is written in C underneath. Only in `--target py` builds,
where the program is Python anyway: `zeno run` could then not run the
program, and running is how a program is written.

### 26.3 They talk over a socket of this machine, so Python keeps the screen

Zeno listens on a port that only this computer can reach, and hands
Python the port and a word made up for the run. Python connects, says
the word, and is listened to. Anything else that connects is hung up
on.

**Why not the two pipes every program has.** They are Python's
keyboard and screen. If the conversation used them, a `print` inside a
library would be a line of the conversation, a progress bar would have
no screen to redraw, and `input()` would read Zeno's questions. Each
of those can be patched around, and each patch is something a library
can see through. With a line of their own to talk on, Python's screen
and keyboard are the user's, as they are when Python is started by
hand, and as they are for a program that `program.run` starts (25.4).

**Why JSON, one line each way.** Every language of 26.1 can read and
write it with what it comes with, and a line can be read by a person
when something goes wrong.

### 26.4 `python.module(name)`, and from there what is written is what Python does

```zeno
let yt = python.module("yt_dlp")
let downloader = yt.YoutubeDL(options)
downloader.download([address])
```

**Why a function, and not `import python.yt_dlp`.** `import a.b` is a
file of the program, `a/b.zeno`, found and checked before the program
runs (7.1). A module of Python is found when Python is asked, and may
not be there. A call that can raise says that; an `import` that can
fail at run time would be a second meaning for one word.

**Why nothing more than a dot and brackets.** `thing.name`,
`thing(...)` and `thing[...]` already mean a field, a call and an
item. On a value of Python they mean Python's. There is no second way
to write them and nothing to learn but the library.

**A named argument is a keyword argument**, `dict(quiet=true)`,
because that is what it is in both languages.

### 26.5 Numbers and text cross as themselves; everything else stays where it is

`none`, a bool, an int, a float and a str are the same thing in both
languages and cannot be changed in place, so a copy is the value.
Anything else that Python returns stays in Python, and Zeno holds the
number it is kept under.

**Why a list stays too.** A list can be changed. If
`sys.path.append(folder)` ran on a copy of `sys.path`, Python's list
would not change, and nothing would say so. Bridges between languages
that copied lists across have that trap, and the later ones do not
copy. So `append` on a list of Python is Python's `append`, on the one
list.

**`python.copy(value)` when a list of Zeno is wanted.** It says in the
program that a copy is made, at the line where it is made.

**What is exact.** Only Python's own `int`, `float`, `str`, `bool` and
`None` cross by value, not kinds built on them. A number of a library
that is not one of these stays in Python, and `builtins.float(x)`
makes a float of it. An int too big for Zeno's 64 bits stays as well.

### 26.6 What Zeno gives Python is a copy

A list or dict of Zeno that is passed arrives as a new list or dict of
Python, with what is in it.

**Why not the same list.** For Python to hold a list of Zeno, Python
would have to call into Zeno each time it reads an item, and Zeno is
waiting for Python's answer at that moment. That is the same piece of
work as a function of Zeno called from Python (26.11).

### 26.7 A Python value is not a condition, and not a number

`if thing { }` is refused as it is for any value that is not a bool
(2.4), and so is `thing + 1`.

**Why.** Python decides for itself what counts as true: an empty list
is false, a number is false when it is zero, and a class can say what
it likes. Zeno has one rule, and a second rule for values that came
from elsewhere would be the thing the language was made to leave out.
`len(thing) > 0` and `thing is not none` say which test is meant.

### 26.8 An error of Python is an error of Zeno

Python's `ValueError`, `IndexError`, `KeyError`, `ZeroDivisionError`
and `OverflowError`, and the kinds built on them, are Zeno's kinds of
the same names. `OSError` and its kinds are `FileError`. Every other
is an `Error`. The message begins with Python's name for the error
when that is not the kind's own name.

**Why.** A program should not have to know which language raised
`ValueError` to catch it. And `except Error` catches everything
already, with the name in the message for whoever reads it.

**No Python, or no such module, is a `FileError`**, like a program
that is not installed (25.5): it is the same fact, something that was
looked for on this computer and is not there.

### 26.9 The types are known when the program runs (provisional where marked)

A value from Python has the type `json.parse` gives (18.1). The
checker lets it through, and `--strict` refuses it.

`zeno run` and `zeno test` have `python`, in both engines. `zeno
build` in C and LLVM, `--firmware` and every `--target` refuse it. In
a page it is a `FileError`.

**Provisional:** `--target py`. A program written out as Python could
import the module directly, with no helper. It would have to behave as
`zeno run` does at every line, which needs the same care as the rest
of that target.

### 26.10 Which Python

`python3`, then `python`; on Windows `python`, `python3`, then `py`.
`ZENO_PYTHON` names one outright. A name that may be something else
(`python` can be Python 2, and on Windows a stand-in that only says
where to get Python) is asked quietly for its version first.

**Why the first on the `PATH`.** It is the one the user's `pip
install` installed into, and the one an active virtual environment
puts first.

### 26.11 Nothing changes for a program that does not ask

`import python` itself starts nothing. Python is started by the first
`python.module`, once. A program without the import runs the code it
ran before: the new kind of value is one more case in places that
already ask what kind of value they have.

**The one thing that does change.** `python` is now the name of a
module that comes with Zeno, so a file of the program called
`python.zeno` can no longer be imported under that name, and a package
cannot have it. It is what happened to `json` and `web` when they were
added, and it is the price of any new module. The message that
lists the modules has one more name in it.

**How that is held to.** Every test from before passes, with that
message as the only line changed in them, and before a release the
speed is compared with the release before, by count and by timed
rounds (24.29).

### 26.12 Left for later

- A function of Zeno given to Python: progress callbacks, sort keys.
- A name that is a word of Zeno (`re.match`). For now Python's own
  `getattr` reaches it. Whether a word may follow a dot is a question
  for the whole language, and it was decided once (5.19c).
- A loop that takes items one at a time, for what never ends.
- `python` in a built program, and in `--target py` (26.9).
- The rest of 26.1. JavaScript is chapter 28 and Java chapter 29.

## 27. The registry: a package by its name

Partha asked (2026-11-18) for a central place to find packages, as
crates.io is for Rust and npm for JavaScript. Until now a `needs`
line said where a package is: a folder, or a git repository (8.3).
8.3 turned a registry down for 0.1, and said one could come later as
a third kind of line with the two others left as they are. This is
that. [`registry.md`](registry.md) is the walkthrough.

**Checked against "unambiguous and intuitive".** The new line is an
old line with the address left out (27.1). A version is still one
exact thing, and now it cannot even move (27.2, 27.4). A name is
looked up in one registry, which the program's own file names
(27.13). A command that uses the network is one that was typed to do
that (27.7). And publishing has one step that changes something, and
it is the registry's, not the one on your machine (27.10).

### 27.1 `needs units at "v1.2.0"`: a name, and a version

```
needs filters in "../filters"
needs units from "https://github.com/someone/zeno-units" at "v1.2.0"
needs units at "v1.2.0"
```

The third line names no place. The registry says which repository
`units` is, and which commit of it is `v1.2.0`.

**Why this spelling.** It is the second line without `from` and its
address. `at` means what it meant, and nothing new has to be learned:
somebody who moves a package into the registry deletes an address
from the lines that need it. A line with a name and nothing else is
refused, with both ways to finish it.

**Alternatives rejected.** `needs units "1.2"` (the first idea, in
8.3): two texts side by side in the git line would then mean an
address and a version by their order alone. `needs units from
registry at "v1.2.0"`: a word that says what the missing address
already says. A separate file for registry needs: 8.2 again.

### 27.2 A version in the registry is the letter v and three numbers

`v1.2.0`. It is a tag of the package's repository, exactly as
written. No number has a zero in front, so a version has one
spelling.

**Why the v.** What stands after `at` is then the same text in the
git line and in the registry line, and the same text as the tag that
`git tag` made. With `1.2.0` in one line and `v1.2.0` in the other,
one version would have two spellings, and somebody would have to
remember which line takes which.

**Why three numbers and nothing else.** A registry has to say which
version is the newest, so versions need an order, and three numbers
have one that everybody knows. `main` or a commit after `at` is still
possible, in the line that names the repository: those are things a
repository has, not things a registry lists.

**Alternatives rejected.** `1.2` standing for the newest `1.2.x`: a
range under another name, and 8.4 has no ranges. Any tag at all:
then "the newest version" has no meaning. `-rc1` and the like: left
out until somebody needs one.

### 27.3 The registry is a git repository with one small file for each package

The packages stay in their authors' repositories. The registry holds
`index/<name>` for each: its name, what it is for, where it is, who
published it, and a line for each version with its commit.

**Why.** 8.3 gave four reasons against a registry: a server,
accounts, uploads, and people taking names. This shape answers the
first three and leaves the fourth small.

- *No server.* A git repository on GitHub is free to host and is
  kept running by somebody else. Zeno has no library for encrypted
  connections (8.3), and needs none: it reads the registry with the
  `git` it already uses for every package.
- *No accounts of its own.* Whoever publishes is somebody with an
  account where the registry is hosted (27.11).
- *No uploads.* A package is fetched from where it already is. The
  registry never holds anybody's code, so it stays a few hundred
  bytes a package, and there is nothing to store, scan or pay for.
- *Names.* See 27.11.

It is also something a reader can understand whole: open the
repository and read a file.

**Why one file for each package.** Two people who publish at the same
time change two files, and git joins that by itself. A version is one
line added to one file, which is what a history of the registry
should look like.

**Why in Zeno's plain words.** The file is read by the lexer that
reads `zeno.package` and `zeno.lock`, so there is no second reader
(8.2).

**Alternatives rejected.** A server that packages are uploaded to
(crates.io, npm, PyPI): the usual shape, and all four of the reasons
above. An index in git plus archives on a server (crates.io's own
split): still the server. One big file: every change touches it.

### 27.4 The registry names the commit, and a published version never changes

The line for a version holds the commit the tag named on the day it
was published. `zeno fetch` checks out that commit. It does not ask
the repository what the tag names today.

**Why.** A tag can be moved, by mistake or on purpose. With the
commit written down, moving it changes nothing for anybody: the
version everybody has is the version everybody keeps getting. This
is what an upload gives a registry with a server, and here it costs
forty letters. Taking the same version in a second time is refused,
and the message says so if the tag has moved.

If the repository no longer has the commit, the fetch stops and says
that, and uses nothing.

### 27.5 The lock line begins as its `needs` line does, and with a lock nobody is asked

```
units at "v1.2.0" from "https://github.com/someone/zeno-units" commit "9f8e7d6..."
```

**Why this order.** A line of `zeno.lock` is the `needs` line it is
for, without the `needs`, and then what `zeno fetch` found. For a
line with an address that was the commit. For a line with a name it
is the address and the commit, both from the registry.

**Why the address is in the lock.** So that a fetch with a lock does
not need the registry. The lock already answers "what exactly is
built?" (8.10), and where it comes from is part of the answer. A
program with its lock builds next year, whatever has become of the
registry.

### 27.6 This machine keeps one copy of the registry, where things are kept that can be fetched again

`~/.cache/zeno` on Linux, `~/Library/Caches/zeno` on macOS,
`%LOCALAPPDATA%\zeno` on Windows, or the folder `ZENO_CACHE` names.
Only the newest state is kept. It is brought up to date when a name
has to be looked up, once in a run.

**Why not in the program's folder**, where `packages/` is. `zeno
search` is run where there is no program yet. And each program would
fetch its own copy of the same list.

**Why this is not 8.6 again.** 8.6 turned down a file "in a folder
you may not know is there" that changes what a program means. This
copy changes nothing: it is a copy of what the registry says, the
first line `zeno fetch` prints says the registry was asked, and
deleting the folder only makes the next fetch get it again.

### 27.7 Three commands use the network, and each is typed to do that *(widens 8.9)*

`zeno fetch`, `zeno search` and `zeno publish`. `run`, `check`,
`build` and `test` still read files and nothing else.

`zeno search` with no network shows the copy from last time, and
begins by saying that this is what it shows.

### 27.8 `about "..."`: one line that says what a package is for

A new line of `zeno.package`, at most 120 characters. A package
needs it to be published, and not before.

**Why.** A list of names tells a searcher nothing. The line is in
the package's own file, so it travels with the version: the registry
shows the one of the newest version.

**Alternatives rejected.** Asking for it in the form: then it is in
the registry and not in the package. The first line of a README: a
second file with a rule about its first line.

### 27.9 A name in the registry is small letters, digits and `_`

2 to 40 of them, starting with a letter. Not a word of the language,
and not a module that comes with Zeno.

**Why small letters.** The registry is a folder of files named after
packages, and Windows and macOS take `Units` and `units` for one
file. And nobody can publish a `Units` to be mistaken for somebody
else's `units`. A package that is never published may still be
called what any name of the language may be.

### 27.10 `zeno publish` checks, and changes nothing

It checks the package (the table in `registry.md`), and ends with
how to ask the registry: the address of a form, for a registry on
GitHub, or the line for whoever keeps one that is elsewhere.

**Why it does not do the publishing itself.** To add to a registry,
`zeno` would have to hold something that proves who you are: a token
in a file on your machine, a `zeno login`, and an encrypted
connection to send it over. That is a secret to leak and Zeno's first
dependency. As it is, the only thing that proves who you are is that
you are signed in to GitHub in your own browser.

It also keeps the command honest. `zeno publish` can be run as often
as you like, at any stage, to see what is still missing.

**Alternatives rejected.** Pushing a branch to a fork of the
registry and opening a pull request: needs the fork, and GitHub's
API to open the request. `zeno login`: above.

### 27.11 Zeno's own registry is asked by a form on GitHub, and the first to publish a name owns it

The form is an issue with two answers: the repository and the
version. A workflow reads it and runs `zeno registry accept` (27.12).

- **Who is asking** is the account that sent the form. GitHub says
  so, and it cannot be typed in.
- **They must publish their own repository**: one that belongs to
  that account, or to an organization that shows them as a member.
- **The first to publish a name owns it.** Later versions come from
  the same repository and the same account.

**Why.** This is the answer to "accounts" and to most of "people
taking names" (8.3). Nobody can publish another person's repository
under a name they would then own, and nobody can add a version to a
package that is not theirs. What is left is somebody taking a good
name for an empty package of their own. No rule stops that at the
door without a person reading every request, so that is for a keeper
of the registry to undo when it is reported.

**Why a form and not a pull request** (as Homebrew and winget do).
A pull request has the asker write the registry's file, which then
has to be checked line by line, by a program or a person. A form has
two answers, and the registry writes its own file.

**Provisional.** The form takes repositories on GitHub. A package
that lives elsewhere is added by a keeper by hand. And an owner is
one account: handing a package on is a keeper's edit.

### 27.12 `zeno registry accept` is part of `zeno`

```
zeno registry accept <folder> <address> <version> --owner <who>
```

It fetches the repository at the tag, checks the package, and writes
`index/<name>` and `index.json` in a clone of a registry. It commits
nothing.

**Why in `zeno`.** What a package must be to be published is asked
twice: by `zeno publish`, of a folder, and by the registry, of what
the repository really holds. Both ask the same function (`fits`),
and both read `zeno.package` with the one reader there is. A script
in the registry's repository would be a second reader of that file,
to keep in step by hand.

**Why it commits nothing.** Who commits, and how, belongs to whoever
keeps the registry: a workflow for Zeno's own, a person with a
terminal for a small one.

**Why `--owner` is a word and not an account.** What an owner is
depends on where the registry is. The workflow passes the GitHub
account; a company's registry can pass whatever it calls its people.

### 27.13 A program can name another registry, and there is one for each program

```
registry "https://git.example.com/zeno-packages"
```

Every name in the program is looked up there, in place of Zeno's
own. It is the line in the program's own `zeno.package` that counts,
as with `memory`.

**Why.** A company's packages are often not for the world. Since a
registry is any git repository, one of your own costs nothing.

**Why in the file, and not a setting of the machine.** What `needs
units at "v1.2.0"` means would then depend on a variable somebody
set last year. In the file, whoever reads the program can see where
its names come from.

**Why one, and not a list to look through in order.** With two
registries, a name that is in both has two meanings, and which one a
program gets depends on the order. That is how a package published
in public under the name of a private one ends up in a company's
build. One registry for a program, and a name has one meaning.

**Alternatives rejected.** A registry on each `needs` line. A list
of registries. An environment variable (`ZENO_CACHE` is one, but it
says where a copy is kept, and changes what no program means).

### 27.14 `index.json` is the same list for a browser, and the page reads it there

`zeno registry accept` writes `index.json` beside `index`: every
package on a line of its own. zeno-lang.org/packages reads it from
the registry when the page is opened.

**Why.** A page in a browser cannot run git. And a page that read the
list when the site was built would be out of date until the site was
built again, which is a step somebody has to take. This way a
package is on the page a few minutes after it is published.

`zeno` itself does not read `index.json`.

### 27.15 Left for later

- Taking a version out, or marking one as not to be used.
- Handing a package to another owner, and more than one owner.
- The form for packages that are not on GitHub.
- A licence line, and running a package's tests before it is taken.
- Versions before a release (`v1.2.0-rc1`).
- Saying in the lock which registry a line came from.

## 28. The libraries of JavaScript

The second language of the first version (26.1): `import node`, for
the library that comes with Node and for the packages of npm.
[`node.md`](node.md) is the walkthrough. Everything in chapter 26
holds here unless this chapter says otherwise: Node runs beside the
program (26.2), they talk over a socket (26.3), numbers and text cross
and everything else stays (26.5), what Zeno gives is a copy (26.6), a
value from there is not a condition (26.7), the types are known when
the program runs (26.9), and nothing changes for a program that does
not ask (26.11). What follows is what JavaScript made different.

### 28.1 The module is called `node`, and the values "JavaScript"

`import node`, `node.module("fs")`. A message says "a JavaScript
value".

**Why `node`.** It is the program that is started, and the one whose
rules decide where a package is looked for. JavaScript in a browser
has no `fs` and no npm. `import javascript` would promise more than
is there, and `import npm` would name the place the packages come
from and leave out the library Node has without it.

**Why the values are "JavaScript".** A message about a value speaks
of the language the value is written in. `ZENO_NODE` names the Node
to use, as `ZENO_PYTHON` does (26.10). Only `node` is looked for: it
has no second name.

### 28.2 `node.global` and `node.new`, because JavaScript has both

Python has everything in a module, `builtins` included, and makes an
object by calling its class. JavaScript has neither: `JSON`, `Math`,
`Date` and `process` are in no module, and a class has to be given to
`new`.

`node.global(name)` gives what JavaScript has under that name
everywhere. `node.new(Class, a, b)` is `new Class(a, b)`.

**Why not make `Class(a, b)` do `new`.** Calling and `new` are two
things in JavaScript, and one value can take both with different
answers: `Date()` gives a str and `new Date()` a date. Guessing which
was meant would be right most of the time, and wrong with nothing to
say so.

**Alternatives rejected.** A module of pretend, `node.module("global")`:
it reads like a package that might not be installed. A `new` word in
the language: one module's need is not the language's.

### 28.3 A number that is whole is an int

JavaScript has one kind of number. One that is whole arrives as an
int, and any other as a float.

**Why.** Nearly every whole number a library returns is a count, a
size, an index or a year, and in Zeno those are ints: `items[n]` needs
one. Had every number arrived as a float, each of them would need
`int(...)` around it. The other way round costs less: an int goes
where a float is wanted.

**The cost.** `Math.sqrt(16.0)` is `4` and prints so, where Zeno's own
would print `4.0`. JavaScript does not keep which was written, so
there is nothing to go by.

**Past what is exact.** A whole number above 9007199254740991 arrives
as a float: by then JavaScript's number is not exact, and an int would
say that it is. An int of Zeno above that goes as a `BigInt`, since a
number would silently round it, and a `BigInt` that fits 64 bits comes
back as an int.

**`null` and `undefined` are both `none`.** Zeno has one nothing. A
field that an object does not have is therefore `none`, with no error.
That is JavaScript's rule and it is kept, because a library's own
documentation is written to it.

### 28.4 Named arguments are the object of options

`thing(1, key=3)` is `thing(1, {key: 3})`.

**Why.** JavaScript has no named arguments, and its libraries agree
on what stands in for them: one object, last. So the Zeno call reads
as the same call does with Python (26.4), and a program does not have
to build an object first for the common case.

**Why refuse nothing.** Whether a function takes options is not known
before it is called. Giving named arguments to one that does not is
one argument more than it reads, which JavaScript ignores.

`node.global("Object")(a=1, b="x")` makes such an object by itself,
which is `python.module("builtins").dict(a=1, b="x")` for JavaScript.
`node.new` takes no named arguments: no function that comes with Zeno
takes any, and one exception would be a second rule.

### 28.5 A promise is waited for

A call or a field that gives a promise gives what the promise
resolves to. One that is rejected is an error at that line.

**Why.** A Zeno program is one line after another. There is nothing
it could do with a promise but wait for it, and writing that wait on
every call would be a word that never changes. Because of it the
whole promise side of Node works today: `fs/promises`,
`timers/promises`, `fetch`.

**The cost.** A program cannot start two things and wait for both.
That waits for the day a function of Zeno can be given to JavaScript
(28.10).

### 28.6 What a method is called on

`thing.name` in JavaScript gives a function that no longer knows
`thing`. The helper keeps, with each value, the object it was read
from, and calls it on that. So `start.getFullYear()` works, and so
does keeping `start.getFullYear` in a name and calling it later, which
in JavaScript itself fails.

**Why go further than JavaScript does.** In Zeno a method taken from
an object stays that object's: `let get = box.get` and then `get()`
is `box.get()`. A Zeno program that reads a method and calls it later
is written to that rule.

### 28.7 `==` is the same object; `print` is what the console shows

`a == b` on two values of JavaScript is `a === b`.

**Why not compare what is in them.** JavaScript has no such
comparison, and each library that needs one writes its own. Inventing
one here would give answers no JavaScript program gets.
`node.copy(a) == node.copy(b)` compares what is in them, by Zeno's
rule, and says so.

`print(thing)` shows what `console.log(thing)` shows, because that is
what a person who uses the library has seen. A value that writes
itself, a date or an error or a URL, is shown as it writes itself:
`str(thing)` should be the text the value means.

### 28.8 An error of JavaScript is `Error`, or `FileError`

What JavaScript throws is an `Error` of Zeno, and its message begins
with JavaScript's name for it (`TypeError: ...`). An error that the
system gave, which is one with an `errno` and a `syscall`, is a
`FileError`. No Node, and no such module, are a `FileError` too
(26.8).

**Why no `ValueError` or `IndexError`.** JavaScript's `RangeError`
and `TypeError` do not mean what Zeno's kinds mean: `RangeError` is
thrown for a bad length and for a stack that is too deep. Matching
names that only look alike would make `except ValueError` catch
things it was not written for. Reading past the end of an array is
not an error in JavaScript at all.

### 28.9 A module is looked for from the folder of the program

`node.module("dayjs")` looks in `node_modules` beside the `.zeno`
file, then in each folder above. `node.module("./helper.js")` is a
file beside the program. For Python the folder of the program is the
first place looked in, before the installed packages, so a
`helper.py` beside the program is `python.module("helper")`.

**Why.** It is where each language looks for a program of its own,
and it is where Zeno's own `import` looks (7.1). A program then finds
its modules wherever it is run from. Looking from the folder Zeno was
started in, which is what a helper started with `-e` or `-c` does by
itself, would make `zeno run project/main.zeno` fail for a program
that runs when started inside `project`.

This changed Python as chapter 26 first had it. It was in no release.

### 28.10 Left for later

- A function of Zeno given to JavaScript. More of JavaScript needs it
  than of Python: `map`, `on("data", f)`, a server.
- Node ends when the program does. A server or a timer started there
  does not keep the program alive.
- Starting two things and waiting for both (28.5).
- `node` in a built program, and in `--target js`, as 26.9 has it for
  Python.

## 29. The libraries of Java

The third language of the first version (26.1): `import java`, for the
classes that come with Java and for libraries that are jars.
[`java.md`](java.md) is the walkthrough. Chapter 26 holds here as it
does for JavaScript (28): Java runs beside the program, they talk over
a socket, numbers and text cross and everything else stays, what Zeno
gives is a copy, a value from there is not a condition, the types are
known when the program runs, and nothing changes for a program that
does not ask. What follows is what Java made different.

### 29.1 `java.type(name)` gives a class, by its whole name

```zeno
let ArrayList = java.type("java.util.ArrayList")
```

**Why `type` and not `class`.** `class` is a word of Zeno and cannot
follow a dot (5.19c). `type` is also what the bridges from JavaScript
to Java call it (`Java.type` in Nashorn and GraalVM), so it is the
word a person who has done this before looks for.

**Why the whole name.** Java's own short names come from an `import`
at the top of each file, which this program does not have. One rule,
the name the documentation gives, is less to know than a list of
packages that are looked in without being asked.

### 29.2 Calling a class is `new`

`ArrayList()` is `new ArrayList()`.

**Why here and not for JavaScript (28.2).** In JavaScript a class can
also be called without `new`, with another answer, so the two had to
be told apart. A class of Java cannot be called at all, so the call
has one meaning. It is also how an object is made in Zeno itself, and
in Python.

### 29.3 A dot after a class is what is `static`; a method is kept until it is called

`Math.PI`, `Math.max(3, 9)`, `Thread.State.NEW`. On an object,
`thing.name` is its public field, or its method.

**What `java.type` gives is not Java's `Class` object.** That object
has methods of its own (`getName`, `getMethods`), and `Math.max` would
have to be looked for among them. So the helper has a value of its
own for "the class as its name is written" (`Type`), and gives Java
the `Class` object when a method asks for one.
`thing.getClass()` still gives Java's.

**A method not yet called is a value too** (`Bound`), because Zeno
reads `names.add` before it calls it, and because a method can be
kept in a name and called later, as in Zeno.

**Only what is public.** A program of Java outside the class could
use nothing else, and a library's private parts are not its promise.

### 29.4 Of several methods with one name, the arguments choose

Java can have `valueOf(int)`, `valueOf(double)` and `valueOf(Object)`
side by side. Each is given a number for how well the arguments of
this call fit what it takes, and the lowest is called.

**The order is Java's own** as far as it has one: an int of Zeno is an
`int` before a `long`, a `long` before a `double`, and a plain type
before its boxed one; a method that takes its arguments one by one
comes before one that gathers them (`Object...`).

**Two that fit equally are an error**, which lists what there is, and
so is none that fits. A guess would call something, and the program
would go on with the wrong answer.

**Alternatives rejected.** Saying the types in the call, as
`thing.method["int", "Object"](1, x)`: it is exact, and it is a
second language to learn for the few calls that need it. It can be
added for those if they turn up. Taking the first method that fits:
which is first is not something Java promises.

### 29.5 An int that goes as an `Object` is an `Integer` if it fits

A collection of Java takes anything (`Object`), so the method does not
say which whole number it wants. An int of Zeno goes as an `Integer`
when it fits 32 bits and as a `Long` when it does not.

**Why.** It is what the number is when written in a Java program: `5`
is an `int` there and `5000000000L` a `long`. A library compares what
it is given with numbers of its own, and `Integer` 5 does not equal
`Long` 5 in Java. Always `Long` would be simpler to say and would fail
quietly inside `contains` and `get`.

**Every whole number that comes back is an int**, whatever its size
in Java, and `float` and `double` are a float. A `float` is written
the short way it prints, `0.1`, and not with the digits it gains as a
`double`.

### 29.6 `==` is `equals`

**Why not the same object, as for JavaScript (28.7).** Java has both,
and its own `==` on objects is the one programs are told not to use
for values: two `String`s or two `Integer`s with one value can be two
objects. `equals` is what a class of Java says its equality is.

`print` shows `toString()`. An array has none worth reading
(`[I@1b6d3586`), so its items are shown.

### 29.7 An exception of Java is an error of Zeno

`IndexOutOfBoundsException` is `IndexError`,
`IllegalArgumentException` is `ValueError`, `IOException` is
`FileError`, each with the kinds built on it, and every other is
`Error`. The message begins with Java's name for it.

**Why these and no more.** Each of the three means in Java what the
kind means in Zeno, by Java's own description of it. That is not so
for `ArithmeticException`, which is both a division by zero and an
overflow, two kinds in Zeno; it stays an `Error` (28.8 has the same
reasoning for JavaScript).

### 29.8 A library is a jar, and the program says where it is

`java.jar("lib/gson.jar")`, or a folder: every jar in it.

**Why the program says it.** Python and Node each have one place
where `install` puts packages, and each finds them there by itself.
Java has none: a jar is found only if the class path names it. So
the program names it, at a line that can be read, with a path from
its own folder (28.9).

`CLASSPATH` is read as Java reads it, and with none the folder of the
program is looked in for classes, which is what `java Main` does in
that folder.

**Alternatives rejected.** Every jar beside the program, without
being asked: which jars are used would depend on what happens to lie
in the folder. A name as Maven writes it
(`com.google.code.gson:gson:2.11.0`), found in Maven's own folder: a
library there usually needs others, and which ones is worked out by
Maven from files Zeno would have to read as Maven does. A folder
filled by Maven or by hand, and named, is whole.

### 29.9 The helper is compiled once, and kept

Java runs classes and not source. The helper's source is in the
`zeno` file; the first run makes classes of it with `javac`, in the
folder this machine has for what can be made again (the registry's,
27), and later runs start those.

**Why.** Compiling takes about a second and starting takes a fifth of
one. Paying the second on every run would make a short program slow
for good.

**How it stays right.** The folder is named by a number worked out
from the source, so another version of Zeno makes its own. The
classes are made in a folder of the run's own and given their name
when whole, so two runs at once never share half of them. They are
compiled for Java 11, so a later Java runs them as they are.

**With no `javac`**, Java is given the source, which Java 11 and
later compile as they start. With no folder to keep them in, they are
made in a folder of this run's own and taken away after.

**Alternative rejected.** The classes inside the `zeno` file, ready
made: building Zeno would then need Java, and Zeno is built with
Rust and nothing else.

### 29.10 A name in brackets, where a word of Zeno is in the way

`System.in` does not parse (5.19c). On a class, and on an object that
has no items of its own, `thing["name"]` is `thing.name`:
`System["in"]`.

**Why this is safe to mean two things.** It is taken so only where
`[ ]` has no other meaning: a class has no items, and neither has an
object that is not an array, a list, a map or text. For JavaScript the
same writing is the language's own rule ([`node.md`](node.md)).

### 29.11 The bridge is kept out of the functions every program runs

Every function of `bridge.rs` that the rest of Zeno calls is marked
`#[cold]` and `#[inline(never)]`, and so is what letting go of a value
from another language calls.

**What happened.** With Java added, a sort on `zeno run --vm` took a
tenth longer than in 0.5.1 (3.63 s to 4.02 s for a million numbers,
best of 8 rounds each), while the count of instructions was 1 % lower.
No line of the VM had changed. The places that hand a value to the
bridge are `ops::item`, `ops::set_item`, `ops::field` and
`value::equal`, which every program runs through, and the compiler
had folded the bridge into them. So their code changed, and moved,
each time the bridge did: `ops::item` was 2170 bytes with JavaScript
added and 2129 with Java. It is 1800 with the bridge kept out, and
does not change when the bridge does.

**Why this is the fix and not a patch.** 26.11 promises that a
program which asks for no other language runs the code it ran before.
The source said so already; the compiled program did not. `#[cold]`
tells the compiler that these calls are rare, which they are in every
program without the import and cheap beside the trip to another
program in one with it.

**After it.** The same sort is within 2 % of 0.5.1. All six
benchmarks, both engines, timed in alternating rounds against 0.5.1,
are between 6 % faster and 4 % slower. The 4 % is nbody on the VM
(16.51 s to 17.08 s); about half of it was there before the bridge
was begun, and the rest is no more than this machine gives for one
build timed twice (24.29).

**What it does not fix.** Where the compiler puts the VM's own loop
still moves by a few bytes when code is added anywhere, and that
alone is worth a few percent on one benchmark or another. Stable Rust
has no way to pin it.

### 29.12 Left for later

- A function of Zeno given to Java: lambdas, `Comparator`, `Runnable`.
- A class of Zeno that extends a class of Java.
- Saying the types of a call where two methods fit equally (29.4).
- Zeno's checker reading Java's types, which Java has.
- `java` in a built program, and in `--target java`, as 26.9 has it
  for Python.

## 30. The least C: `zeno build --minimal`

Partha asked (2026-11-18) for one more way to build: one that goes
past the checks and writes only the C the program itself needs, so that
the file is small and easy to read, with everything that is there now
kept as it is. `zeno build` writes about 3,000 lines of C for a program
of one line, because the whole runtime is pasted in front of it and
every operation goes through a function that checks it.

### 30.1 A new option, and the builds that were there do not change

`zeno build --minimal` writes the least C that runs the program. Six
things are done, each with its own section below: no check of any kind
(30.2), no place in the source carried along (30.4), only the parts of
the runtime the program uses (30.5), `print` as C's `printf` (30.6),
each statement written the short way (30.7), and the C compiler asked
for a small program (30.8). docs/minimal.md is the walkthrough.

**Why a new option.** He asked for what is there to stay. The C that
every other build writes is, byte for byte, what it was before this
chapter: 2,028 C files were compared, the 363 programs of the check
(30.10) built nine ways, with the compiler before and after.

**Why the word `minimal`.** It is his word for it ("the minimum code"),
and it says what the option is for, which is how `--fast` was named
(23.1). `--small` promises a small program, and it is the C that gets
small (30.11). `--unchecked` names one of the six things. `--bare` and
`--direct` say less than `minimal` does to someone who has not read
this.

**Why not `--target c`.** `--target` writes the program in another
language with the rules of Zeno carried along in a runtime (24.2),
checks and all. This is the opposite promise, and it is the same C
backend as `zeno build`, so it is an option of that.

### 30.2 No check of any kind: a mistake does what the C does

| | other builds | `--minimal` |
|---|---|---|
| `xs[i]` with `i` out of range | `IndexError` | reads or writes whatever memory is there |
| `ages["Eve"]` with no such key | `KeyError` | reads an item that is not there |
| overflow of `+ - *`, of `-n`, `abs(n)`, `**`, `sum` | `OverflowError` | no defined result |
| `n // 0`, `n % 0` | `ZeroDivisionError` | what the processor does: on a computer the system stops the program |
| `x / 0.0` | `ZeroDivisionError` | infinity, or NaN |
| `math.sqrt(-1.0)`, `math.log(0.0)`, `math.asin(2.0)` | `ValueError` | NaN or infinity |
| `u8(300)` | `ValueError` | 300 goes on as it is |
| `n << 70`, `n << -1` | `ValueError` | no defined result |
| `xs.pop()`, `min(xs)` of an empty list, a slice from past the end | an error | no defined result |
| `int(x)` of infinity or NaN | `ValueError` | the biggest int, the smallest, or 0 |
| a variable that became `none` again after it was checked | an error | used as if it held a value |
| `free(p)` twice | stops, and says so | no defined result |

**Why further than `--fast`.** `--fast` is about time. It leaves out the
checks that cost something in a tight loop and keeps the rest: a key
that is not there, a whole number divided by zero. `--minimal` is about
what is in the file. A check brings its message, the message is put
together from pieces of text, and text needs memory: one check left in
keeps `zeno_fail`, the error value, `zeno_join`, `zeno_str_int` and an
allocator in a program that adds two numbers. It is all of them or the
file is not small.

**Why a whole number divided by zero is not caught here, when `--fast`
catches it (23.1).** There the reason was that the check is nearly free
beside a division. Here the cost is not time. The table says plainly
what happens, and it is what a C programmer expects of `/`.

**What is not a check, and stays.** `-7 % 3` is 2 in Zeno and -1 in C,
and `-7 // 2` is -4 and not -3. A correct program relies on that, so
`%` and `//` are still the runtime's `zeno_mod` and `zeno_floordiv`,
each a few lines with the check taken out. `x // 8` and `x % 8` are
still a shift and a mask (23.5).

### 30.3 The errors a correct program can meet stay

`int("ten")` raises `ValueError`, a file that is not there raises
`FileError`, `raise` raises, `try` and `except` catch, and an error
that nothing catches stops the program and says what it was. Running
out of memory still stops the program and says so.

**Why.** These are not mistakes in the program. What a person types
and whether a file is there are known only when the program runs, and
the program was written to deal with them. A minimal build of a
correct program must do what the checked build does (30.10), and it
could not if these went.

### 30.4 No place in the source

A runtime function that can stop the program takes the line and the
column it was called from as its last two arguments, for the message.
In a minimal build no function has those two parameters and no call
passes them, and the table of file names is gone. An error nothing
caught prints its kind and its message:

```
error: uncaught ValueError: cannot turn "ten" into an int
```

**Why.** Two numbers at every call are code at every call, and they are
the first thing that makes the C look unlike C a person writes. With
the checks gone, the only reader left for them was an error the
program raised itself.

**What it costs.** That message does not say where. The checked build
does: build that way to find it.

**How it is done.** The emitter writes a mark in place of `line, col`
(a byte no C has in it), and the mark goes with the comma before it
when the file is put together. In the runtime the parameters and
arguments are taken out by name, token by token (`without_positions`
in src/minimal.rs); the test that compiles the whole runtime for each
kind of build holds that to being C.

### 30.5 Only what the program uses

The runtime and the program are cut into items (a function, a type, a
variable, a `#define`, an `#include`), each asked for by its name.
Starting from the statements at the top of the program, every item
that is named is kept, and every item those name, until nothing new is
named. The rest is not written. A header is kept when one of the names
it brings is used. Comments between items are dropped, since they
explain the runtime across all its builds; comments inside a function
stay with it. A function is declared ahead only if something written
before it names it.

**Why not leave it to the C compiler.** It already does this for the
machine code: an unused `static` function is not in the program of any
build, which is why the program shrinks much less than the C (30.11).
What was asked for is the file: C that can be read, reviewed, and
carried to another project.

**Why the program's own unused functions go too.** One rule, "what the
program uses". Every function Zeno writes is `static`, so nothing
outside the file could have called it.

**Why by name, and not with a C parser.** The runtime is ours and is
written one way. Three things hold the cutting to its word: a unit
test gives the whole minimal runtime of each kind of build to the C
compiler; `tools/minimal.py` reads the C of every build it makes with
the compiler's warnings on, and a function or variable left in unused,
or used and not declared, is a failure; and the programs then have to
run right.

**The marks.** What a minimal build leaves out of the runtime is
between `#ifndef ZENO_MINIMAL` and `#endif /* ZENO_MINIMAL */`, with
what stands in its place after `#else /* ZENO_MINIMAL */`. They are
decided when the C is written, as the marks of `ZENO_FIRMWARE` are, so
no built program has them in it, and every other build gets the text
between them unchanged.

### 30.6 `print` is `printf`

```zeno
print(f"{n} is prime")
print("total", total, "of", len(xs))
```

```c
printf("%lld is prime\n", (long long)z_n);
printf("total %lld of %lld\n", (long long)z_total, (long long)z_xs->len);
```

Words written in the program go into the format. A whole number is
`%lld`, text and a bool are `%s`, and `print` of words alone is `puts`.
`{n:5}`, `{n:<5}`, `{n:05}` and `{x:8.2f}` are `printf`'s own. A float
is `%s` of `zeno_float_text(x, (char[64]){0})`, which writes it into
bytes of the statement's own, because `printf` has no format for "the
fewest digits that read back as the same float". `{n:x}` and
`{reg:08b}` are written the same way by `zeno_radix_text`, and a
quantity is `%s ms`. What `printf` would print differently is left to
the runtime, and shown with `%s`: text lined up to a width (Zeno
counts characters, `printf` bytes), `^`, a float filled with zeros
(`printf` does not fill `inf`), and a list, a dict or an object.

**Why.** `print` in every other build turns each value into text first,
and text takes memory, so `print(n)` brought an allocator into a
program that otherwise has none. With `printf`, a program of numbers
has no runtime in it: `examples/primes.zeno` is 100 lines of C, and
hello is one `puts`.

**What it costs, measured.** `printf` is a large function. On Linux a
program whose `print` never ran it before has about 250 KB more of the
C library in memory while it runs (`ring`: 1,564 KB to 1,812 KB):
pages of the library that nothing had touched. What the program
itself asks for goes down (30.11). On a board nothing is paged, and
the function is in the image of the checked build as well, which
writes its error messages with `fprintf`; there the cost is the stack
`printf` uses while it runs. A program that prints words alone gets
`puts`, and has no `printf` in it at all.

### 30.7 Statements the short way, and what stays as it was

`x = (x + 1);` is written `x += 1;`, and `return (a * b);` is `return a
* b;`. The emitter puts every operation in brackets so that what is
around it cannot change its meaning; around a whole statement nothing
is left that could (`tidy` in src/emit_c.rs). The statements at the
top of the program are the body of `main`, unless the program has a
collector in it, which has to find the program's variables on the
stack below `main` (docs/memory.md).

**What stays.** Names keep their `z_`: a Zeno program may call a
variable `printf`, `main` or `int`. A whole number is still `int64_t`,
because that is what an int is in Zeno. A temporary is still written
where two values have to be worked out in the order Zeno gives them. A
list still holds `zeno_slot`s and not an array of `int64_t` (30.12).

### 30.8 The C compiler is asked for a small program

`-Os` in place of `-O2`. With `--fast` as well it is `-O3
-fno-math-errno`, since that was asked for by name.

### 30.9 What it goes with

Every kind of build that writes C: the collector, `--owned`,
`--no-collector`, `--firmware`, and `--memory` and `--fast` beside
them. `--release` adds nothing to it. `--llvm`, `--web` and `--target`
refuse it with a sentence that says why.

**Why not LLVM.** Nobody reads the IR, so the half of this that is for
a reader has no reader there, and the half that is for the machine
`--fast --llvm` already does.

**The smallest.** `--minimal --no-collector` or `--minimal --owned`. A
program that makes a list has to free it somehow, and with the
collector that is about 500 lines of C that no option can make small.

### 30.10 How it is held to its word

`tools/minimal.py` builds every program of the check of the targets
(24.9: the examples, the corner programs, every sample of the book)
both ways for each of the four kinds of build, and the two must print
the same, end the same way, and stop with the same message. A program
that a check stops has no defined result in a minimal build, so it is
found and left out: the C of the checked build is given a line that
writes a mark in `zeno_fail` and `zeno_bug`, where every check ends.
On 2026-11-18: 213, 193, 193 and 210 programs for the four kinds, all
the same, and 17 to 20 left out for each. It runs on GitHub on a push
that changes the C backend (.github/workflows/minimal.yml), on Linux
with gcc and on macOS with clang.

### 30.11 What it buys, measured

`python3 tools/minimal.py --sizes` makes the table in docs/minimal.md.
In short, for the examples:

- **The C** is 23 lines for hello, about 100 for a program of numbers,
  and 300 to 1,000 for a program with lists, objects and text, from
  about 3,000 every time. Over all the programs of the check it is 7%
  of the bytes with no collector and 14% with one.
- **The program** shrinks less, because the C compiler never put an
  unused function in it. Its code and constants (`text`, as `size`
  counts it) are a quarter to a third smaller for most of the examples:
  `ring` 8.1 KB to 4.5 KB with no collector, `words` 16.5 KB to 12.8
  KB. At the ends, `blink` loses 6% (it had almost nothing to lose) and
  `primes` 82%, 11.4 KB to 2.1 KB. The file on disk moves in steps of
  4 KB and often does not move at all.
- **Memory.** What the program holds from start to end changes little,
  unless the program stops needing the pool (`primes`: 34.8 KB to 0.6
  KB). What it asks for while it runs goes down wherever `print` made
  text: `thermostat` 1,360 bytes to 240.

### 30.12 Left for later

- A list of whole numbers as an array of `int64_t`, and an object's
  fields read without a slot, which is what a person would write.
- A top-level `let` of a number as `static const`, with its value.
- `%` as C's own where both sides are known not to be below zero.
- A `print` for a board with no `printf`.
- The uncaught error's place, for a build that wants only that back.

## 31. A field with the name of a type, in C

Found while `zeno build --minimal` was checked (2026-11-18):
`tools/targets/edge-names.zeno` ran with `zeno run` and every
`--target`, and `zeno build` wrote C for it that the C compiler
refused.

### 31.1 The parameter steps aside, and the field keeps its name

A class may have a field with the name of a class or an enum:

    class point {
        point: int
    }

Zeno has no rule against it, and `zeno run` runs it. In C a class is a
type `z_point`, and the function that makes an object had one parameter
for each field, called as the field is in the struct: `z_point`. Inside
that function the parameter hid the type, so `z_point *self` was read
as a multiplication. A field named as another class broke the list of
parameters itself, and a case of an enum that carries such a value
broke the same way. `edge-names.zeno` has it by another road: its class
`new_` is `z_new` in C, which is what its field `new` is.

**Decision.** Such a parameter is `z2_point`. The field stays
`z_point` in the struct, where C keeps the names of fields apart from
the names of types, so `self->z_point = z2_point;` is right.

**Why `z2_`.** It is how a second variable of one name is already
written (`z2_x`), so nothing new is to be learned when reading the C.
And no type can have that name: every type begins with `z_`.

**Why only where the names meet.** For every program without such a
field the C is what it was, byte for byte: 2,028 C files compared, the
363 programs of the corpus built nine ways, and only the nine of
`edge-names.zeno` differ. The names of all classes and enums are
gathered before the first class is written, because the type may be
written below the class that has the field.

**Why not an error.** "The entire language must be unambiguous and
intuitive": a name that is right for `zeno run` and wrong for `zeno
build` would be a rule of C showing through. Two classes that C would
know by one name are still refused (`same_in_c`), since there a name
would have to be made up for a type the program prints and compares.
