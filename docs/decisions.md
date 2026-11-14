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

**How it will work, once the type checker exists.**
1. Where a type is written, it is checked before the program runs.
2. Where it is left out, the compiler works it out from the value when it
   can. `let n = 3` is an `int` without being told.
3. Where it cannot be worked out, the value is *dynamic*: it carries its type
   with it and is checked while the program runs.

**The cost, stated plainly.** A dynamic value needs a tag saying what it is,
and usually memory from the heap. A small microcontroller cannot afford
either. *(Principle 5.)* So there will be a **strict mode**, in which step 3
is an error: everything must be known before the program runs. Strict mode
will be the default when compiling for a microcontroller, and optional
elsewhere. In practice this means untyped code still works on a
microcontroller wherever the compiler can infer the types, which is most of
the time; it is only truly dynamic code that needs a bigger machine.

**Where this stands today.** The interpreter treats every value as dynamic,
so typed and untyped programs both run, and annotations are read but not yet
checked. Steps 1 and 2 and strict mode arrive with the type checker.

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

**Provisional.** Today a quantity only knows its unit's name. Conversions
(`1 s == 1000 ms`) and derived units (`V * A = W`) belong to the type checker.

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

**Provisional.** The interpreter does not yet check `raises`.

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

### 3.11 Enums carry data, and `match` takes them apart

**Why.** "A reading is either a value or a fault code" is one of the most
common shapes in real programs, and most languages make it awkward. An enum
whose cases hold their own data says it directly, and `match` guarantees
every case is considered.

**Provisional.** The interpreter reports a missing case only when it is
reached. The type checker will report it before the program runs.

---

## 4. Errors

### 4.1 Bugs and errors are different things

An **error** is something that can go wrong in a correct program: bad input,
a division by a value that turned out to be zero, an index from outside.
These can be caught with `try / except`.

A **bug** is a mistake in the program itself: a misspelt name, `"abc" - 1`,
calling something that is not a function. These stop the program and cannot
be caught.

**Why.** Catching a bug hides it. And once the type checker exists, almost
every bug in the second group will be reported before the program runs, so
there would be nothing to catch. *(Principle 1.)*

### 4.2 Error messages say what to do

`x` declared with `let` and then assigned gives "`x` cannot be changed.
Declare it with `var` to allow that". Habits from other languages (`;`, `&&`,
`def`, `elif`, `if x:`) each get a message naming the Zeno spelling.

**Why.** An error message is the language teaching its user. The moment
something goes wrong is when the explanation is most useful. *(Principle 2.)*

### 4.3 The lexer reports every error; the parser stops at the first

**Why.** After a bad character the lexer can simply skip it and carry on, so
all its errors are real. After a syntax error the parser no longer knows
where it is, and the errors that follow are usually nonsense caused by the
first. One accurate error beats ten misleading ones.

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

### 5.5 The compiler will output C before anything else

**Why.** Every microcontroller vendor ships a C compiler, so C output runs
on every target from day one, works with existing debuggers, and can be read
to see what the compiler did. Writing an optimiser can wait.

### 5.6 The interpreter does not check types yet

**Why.** One stage, one job. Types written in the source are parsed and kept
in the tree; the type checker is the next stage and will use them. Until
then, a type mistake is found when the line runs, as in Python. This is also
what makes untyped programs run today (0.2).
