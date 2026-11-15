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
them. Both pairs are kept apart on purpose. *(Changed in 6.22: `.rad` and
`.deg` now change an angle when the program asks. The two still never
meet by themselves.)*

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

**Cost.** `count += 1` on a `u8` does not compile. The message gives
the fix: `count = u8(count + 1)`, or `u8.wrap(count + 1)` for a counter
that is meant to go round.

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
written" has no table to remember. *(Provisional.)*

**A `list[u8]` is not a `list[int]`.** Lists are shared (3.10), so
whoever held it as a `list[int]` could append 300, and the first
holder would find a `u8` that is not one. Copy it: `[int(n) for n in
bytes]`.

### 2.8d No `u64` *(provisional)*

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
*(Provisional: one LED and numbered pins is the least a board can be.
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

### 3.16d No bounds: the body is checked for each type it is used with *(provisional)*

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

### 3.16e What generics leave out *(provisional)*

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

*Superseded by 5.19a to 5.19c:* the word on a parameter is now `take`,
it marks the parameter that takes, and both words are reserved.

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

*Superseded by 5.19a:* a parameter looks unless it says `take`. `self`
still always looks.

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
one. *(Provisional: passing it on could be a move.)*

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

### 5.9g What a build with no collector refuses *(provisional)*

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
lists. *(provisional)*

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
*(provisional: `zeno build --pool N` is an exercise in `docs/pool.md`.)*

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
checking build is an exercise in `docs/pool.md`. *(provisional)*

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
to stay alive that no line of the program shows. *(provisional)*
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
*(provisional: an exercise in `docs/text.md`.)* **Turned down: copy
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
block for each item before the first trip. *(provisional: the checker
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
*(provisional)*. A `borrow` parameter may: the checker already makes
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

### 5.9w `for ch in text` makes a list of the characters, which the loop owns *(provisional)*

It was refused, with advice to count with `range(len(text))` instead.
It is accepted: the list is a made list like any other.

**Why provisional.** It costs a block of the pool for every character
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

### 5.11i A list item crosses into the runtime as an `i64` *(provisional)*

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

*(provisional)* `_setjmp` is the name on Linux and macOS. It is the one
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
removes the pointer for small functions anyway. *(provisional: worth
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
*(provisional: a null test written in IR would let it go, and is an
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

### 5.13a Only a field of an object is stored small *(provisional)*

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
the point of the C backend (5.5). *(Provisional.)*

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
the fix in its first line. *(Provisional, and a question for the
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
rules a reader can hold in their head. *(Provisional. If methods that
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

### 5.15j What a no-collector build still refuses *(provisional)*

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

### 5.16h `list[int or none]` stays refused with no collector *(provisional)*

A number that may be `none` is kept in a small block of its own when
it sits in a list (5.11i). Giving those blocks an owner would mean a
deep copy in every place a list is copied, as text needed (5.9x).

**Why not now.** There is a way to write it today that costs less and
says more: an enum `Reading` with a `Missing` case and a
`Value(n: int)` case, and a `list[Reading]`. `Missing` takes no memory and the list owns each
`Value`. The compiler's message says so. The right fix is to keep the
pair in the slot without a block, which touches the slot's size, and
is for later. *(Keep whatever works.)*

### 5.16i What a no-collector build still refuses *(provisional)*

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

**Provisional.** A function that uses another function value which has
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

### 5.17g What a no-collector build still refuses *(provisional)*

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
from. *Provisional*: firmware that must survive it will want
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
worth more than a second reminder. *(Provisional, as in 5.8a.)*

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
those items, so write the loop that puts them in yours".

**Why only those.** They are what `<` compares, and each has one order
that nobody has to be told. Tasks could be ordered by name or by
priority, and a `sort()` that picked one would be a guess. Sorting by
a function that the program gives is left for when a real program
needs it *(provisional)*.

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

**Why not `char(0)`** *(provisional)*. In a compiled program text ends
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

### 6.10 These methods are written in Rust and C, not in Zeno *(provisional)*

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

*(provisional)* A file too large to hold at once needs reading in
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
tells a person which it was. *(provisional)*

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

*(provisional)* Bytes that are not text belong in a `list[u8]`, and a
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

**Left out** *(provisional)*: `program.exit(code)`. A program ends by
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

*(provisional)* Reading a large file a line at a time, straight into
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

**Left out until something needs it** *(provisional)*: `hypot`, the
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

### 6.24 `asin`, `acos`, `atan` and `atan2` give an angle in radians *(provisional)*

Their answer is a quantity in `rad`, not a float.

**Why.** A float that is secretly an angle is what units are there to
end (1.15). `math.atan2(y, x) > 45` would compare radians with a
number the writer meant as degrees, and run. As a quantity it is
refused, and `math.atan2(y, x).deg > 45 deg` says what is meant.

**What it costs.** Arithmetic with a plain number needs the unit put
on or taken off, as with any quantity: `angle + 2 * math.pi * 1 rad`,
`angle / 1 rad`. Passing it on to `sin` needs nothing.

**Provisional** because it is the first time a library function gives
a quantity back that is not a time. If it gets in the way of real
signal code, a plain float is the other choice.

**`atan2(y, x)` takes y first**, as in C, Python, JavaScript and Rust.
It is the angle of the point (x, y), and the order is the one in
`atan(y / x)`. It takes plain numbers only *(provisional)*: two
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

### 6.32 A value is an `int`, a `float`, a `bool` or a `str` *(provisional)*

```
error: a value of a dict is an int, a float, a bool or a str for now, and
this says list[int]. Keep those in a list, and the position of each in the
dict
```

**Why only these.** They are the values that are copied when they are
stored. So a dict holds nothing that anything else can reach, and a
build with no collector frees a dict in one step with no new rule. A
list or an object as a value raises a question each: who owns it, the
dict or the variable it came from? Is `d[key]` a look at it, and is
`pop` the way to take it back? Those have answers, modelled on a list
of owned items, and they will be given when a real program needs them
(keep what works, and add when a real program asks).

**Meanwhile.** Objects go in a list, and the dict holds the position of
each: `people[index["Ada"]]`.

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

**Why not pairs.** `for code, count in seen` needs a value that is two
values, and Zeno has no such thing yet. When it does, that loop can be
added without changing what this one means.

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
never looked for by `import math`. *(Provisional: once library
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
*(Provisional: if long names turn out to hurt in real programs, `as`
is the way to add.)*

### 7.4 Everything declared at the top of a file can be imported *(provisional)*

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
and it will be wanted then. *(Provisional.)*

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
[`packages.md`](packages.md).

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
names (`needs units "1.2"`): see above. *(Provisional: a registry
could be added later as a third kind of line. The two here would stay
as they are.)*

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

### 8.9 Only `zeno fetch` uses the network

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
*(Provisional: `zeno new` and `zeno add` can come if typing the lines
turns out to be a chore.)*

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

### 9.3 Line breaks are the author's *(provisional)*

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

*(Provisional.)* The formatter does not line comments up by itself.
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

**Why a field gets no answer yet.** *(Provisional.)* The record holds
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
the editor colours at once and needs no server at all. *(Provisional:
semantic tokens can be added on top for what a grammar cannot know,
such as whether a name is a variable or a function.)*
