# The audit: unambiguous and intuitive

Every stage so far added something. This one adds nothing. It reads the
whole language again with one sentence in mind, from the owner of the
language: **the entire language must be unambiguous and intuitive.**

"Intuitive" cannot be tested as it stands, so it was turned into five
questions that have a yes or a no:

1. Is there one obvious way to write it?
2. Can any line be read two ways, by the parser or by a person?
3. Does anything happen that the line does not say?
4. Is every word a plain word, with one meaning?
5. When it is refused, does the message say what to write instead?

What failed a question was changed, or kept and written down with the
reason. This page goes through each one. The reasons, and what was
turned down, are in [`decisions.md`](decisions.md) from 5.19 on.

## 1. A parameter looks, unless it says `take`

This is the largest change, and it failed question 3.

```zeno
function report(task: Task) -> str { ... }
function retire(task: Task) -> int { ... }

print(report(task))
print(retire(task))
print(task.name)          # is `task` still there?
```

Before the audit the answer depended on a word that was *missing*. A
plain parameter took the value over, and a parameter that only looked
said `borrow`. So `retire(task)` emptied the caller's variable, and
neither the call nor the signature had a word for it.

Now the plain thing is the harmless thing:

```zeno
function report(task: Task) -> str { ... }            # looks
function retire(task: take Task) -> int { ... }       # takes, and frees

print(report(task))       # as often as you like
print(retire(task))       # `task` is given away here
print(task.name)
```

```
error: `task` was given away on line 5 and holds nothing now. Use it
before it goes, or give it only to what looks: a parameter without
`take`
```

A function that forgets the word cannot do any harm. What it looks at
cannot be kept, returned or given away, and the message names the word:

```zeno
function keep(frame: Frame) -> Frame { return frame }
```

```
error: `frame` is only looked at here: its owner still has it. So it
cannot be kept, returned or given away. If it is a parameter, write
`take` in front of its type to take the value over
```

The rule matters wherever a value has one owner: everywhere in a build
with `--no-collector`, and for an `owned class` or an `owned list` in
an ordinary build. Where values are shared, `take` is allowed and
changes nothing, so one file builds every way.

| Written | In a build with no collector | In an ordinary build |
|---|---|---|
| `xs: list[int]` | looks | a shared list, which the function may keep |
| `xs: owned list[int]` | looks | looks, at any list |
| `xs: take list[int]` | takes | a shared list, as in the first row |
| `xs: take owned list[int]` | takes | takes an owned list |
| `frame: Frame` | looks | looks, if `Frame` is an `owned class` |
| `frame: take Frame` | takes | takes, if `Frame` is an `owned class` |
| `self` | looks | looks |

In a function type the word goes in the same place:
`function(take Frame) -> int`. A function fits such a type only if it
does the same with each parameter.

### `borrow` is gone

There is one way to write a parameter that looks, so the old word is
not kept as a second one. Writing it says what to write instead:

```
error: `borrow` is no longer a word. A parameter looks at what it is
given without one: write the type alone. A parameter that takes the
value over says `take`
```

## 2. A word of the language is never a name

`unsafe` was reserved when it arrived. Three older words were not:
`strict` on the first line of a file set a mode, and on any other line
it was a variable. `owned` in front of `class` was a word, and in front
of `=` it was a name. The parser could always tell which was meant. A
person had to stop and work it out, and that failed question 2.

All of them are reserved now, and the message says where the word goes:

```zeno
let take = 3
```

```
error: expected a name for the variable, found `take`. `take` is a word
of the language, so it cannot be a name. It goes in front of the type
of a parameter: `frame: take Frame`
```

The names of types (`int`, `list`, `pointer`) are not reserved. They
are names, and a type and a variable never stand in the same place.

## 3. After `case none`, the value is there

```zeno
match found {
    case none { return "nothing" }
    case point { return point.name }
}
```

`point` used to be `Point or none` here, although `none` had just been
dealt with, and the body had to ask `if point is not none` again. That
failed question 3 from the other side: the line said more than the
checker would believe.

Now the cases after `case none` know. It works by a new name
(`case point`) and by the variable's own (`case _ { return found.name }`),
exactly as the `else` of `if found is none` does. If `case none` always
leaves, what follows the `match` knows too:

```zeno
match reading {
    case none { return 0 }
    case _ { }
}
return reading + 1        # an int here
```

An enum that may be `none` no longer needs a final `case _`. It is
complete once `none` and each of its cases has a `case`, and a missing
one is named: "this `match` does not handle `none`".

Both compilers already knew how to read a narrowed variable, because
`if x is not none` needs it: an `int or none` is stored differently
from an `int`. The checker now tells them the same thing for `match`.

## 4. `+` on two lists follows the rule `append` has

```zeno
var xs = [1, 2]
xs.append("three")        # refused, as it always was
xs += ["three"]           # passed, also under --strict
```

Two ways to add to a list, and only one was checked: question 1. `+`
and `+=` now refuse two lists of known, unrelated things, with or
without `--strict`:

```
error: `+` joins two lists that hold the same kind of thing, and these
are list[int] and list[str]
```

A list that may hold `none` still joins one that does not, and a list
that is not known yet (`[]`) joins anything.

## 5. Every function can look at a `let` at the top of the file

This is the first thing a firmware program does, and with no collector
it was refused:

```zeno
let table = [10, 20, 30, 40]

function scaled(i: int) -> int {
    return table[i] * 3
}
```

The reason given was true: the top of the file owns `table`, and could
give it away before `scaled` runs. But the fix it asked for, passing the
table to every function that reads it, is not what anyone expects to
write.

A `let` cannot be given a new value. So if the top of the file never
gives it away, it holds its value until the program ends, and any
function may look at it for as long as it likes. That is the rule now:

- A function, a method and a function that is kept
  (`let pick = function(i: int) -> int { return table[i] }`) may all
  read a `let` at the top of the file, and change what is inside it
  (`table[0] = 5`, `config.gain += 1`, `tasks.append(...)`).
- In return the top of the file may not give it away, before or after:

  ```
  error: `table` is used by a function, on line 3, so it cannot be given
  away: the function could be called when `table` holds nothing. Pass
  `table` to that function instead, or give away a copy
  ```

- A function only looks. It cannot take an item out of the list or put a
  new value in an owned field: the top of the file is the owner.
- A `var` is still out of reach. It could be given a new value while a
  function is looking at the old one, and the message says to use `let`
  if it never changes.

Where there is a collector the same holds for an `owned` list or object.

One more message was fixed on the way. A name used above its
declaration said "`table` is not defined", which is not true. It says
now which line declares it, and that the declaration has to move up.

## 6. A method can take out what its own object holds

```zeno
class Queue {
    tasks: list[Task]
    last: Task or none

    function next(self) -> Task { return self.tasks.pop() }
    function remember(self, task: take Task) { self.last = task }
}
```

Both methods were refused, because `self` only looks, and taking an
item out (or replacing a value, which frees the old one) is for the
owner. So a class could not look after its own parts, which is what a
class is for.

The danger the rule guards against is real: somebody may be looking at
the item that is about to go.

```zeno
for task in queue.tasks {
    queue.next()          # frees the task the loop is looking at
}
```

So the rule is not dropped. It moves to the caller. A method that
takes out or replaces something its object owns is marked as doing so,
and calling it is checked exactly as doing it by hand would be:

| Written | Checked as | So |
|---|---|---|
| `queue.next()` | `queue.tasks.pop()` | only the owner of `queue` may call it |
| inside `for task in queue.tasks` | the same | refused: the loop is looking |
| `print(queue.size(), queue.next())` | the same | refused: the line already looked |
| `queue.swap(queue.tasks[0])` | | refused: the argument looks at the queue |

```
error: the `for` on line 3 is walking a list that `queue` owns, and is
still looking at its items. Nothing can call `next`, which takes out or
replaces something the object owns until the loop is over
```

Inside the method the same three rules hold for `self`. A method that
only reads can be called by anyone, as before. Nothing is written to
mark a method: the checker reads its body, as it does for a return type
that is not written.

## 7. `var f = double` can be given another function

```zeno
var f = double
f = triple
```

```
error: the new value should be a function, but this is a function. ...
```

With no type written, `f` held "the function `double`" and nothing
else, which is right for a `let` and makes no sense for a `var`. A
`var` now gets the type of the function, `function(int) -> int`, and
can be given any function that fits. A `let` still holds the one.

## 8. A firmware build never disagrees in silence

A firmware build stops at a division by zero, a bad index or a number
that does not fit. `zeno run` and an ordinary build raise an error that
can be caught. That difference is by design, and the compiler refused a
`try` around such a fault. One step away it got through:

```zeno
function half(a: int, b: int) -> int raises ZeroDivisionError { return a // b }

try { print(half(1, 0)) } except ZeroDivisionError { print("no") }
```

`zeno run` printed `no`. The firmware build compiled, and stopped at
the division. Now a function that says `raises` for one of the three
faults, with nothing in it that raises one itself, is refused in a
firmware build, with the reason and what to do
([`firmware.md`](firmware.md)).

## 9. Messages count in words

`` `f` takes 2 argument(s), but 1 were given `` is now
`` `f` takes 2 arguments, but 1 was given ``, and the memory report of
a compiled program says `1 block` and `2 blocks`. Small, and it was in
every message that counts.

## Looked at and left as it is

Each of these was read against the five questions and kept, with the
reason.

- **A list of numbers that may be `none`, with no collector**
  (`list[int or none]`). Still refused. Each number would sit in a block
  of its own, and the list would have to copy and free them everywhere.
  The message says what to write: an enum with a case for the missing
  value.
- **A function that is kept cannot take over a list or an object from
  outside it.** It can look for one line, or look at a `let` at the top
  of the file. To own something it is given it as a parameter.
- **A value read inside `unsafe { }` and used after it needs a `var`
  declared before the block.** Every `{ }` in Zeno opens a scope. One
  that did not would be the only one.
- **How full the text room gets can differ by a few bytes between C
  compilers**, when one statement shows a list and builds other text. C
  does not say in which order the arguments of a call are worked out.
  What is printed never differs, and a call that changes something is
  already worked out in order. Only the high-water mark in the
  `--memory` report can move, so leave the room a little headroom.
  Fixing it means writing every piece of text into a temporary first,
  which makes all the C harder to read for a few bytes in a report.
- **A method of a class sees only what is declared above the class**,
  and so does a function. One rule for both. The message now names the
  line of the declaration.

## How it is tested

- **The parser** (`parser.rs`): `a_parameter_that_takes_says_take` and
  `a_file_can_declare_itself_strict` for the words.
- **The checker** (`checker.rs`):
  `a_plain_parameter_looks_and_take_takes`,
  `where_owned_and_take_can_be_written`,
  `after_case_none_the_value_is_known_to_be_there`,
  `every_function_can_look_at_a_let_at_the_top_of_the_file`,
  `only_the_owner_takes_out_what_has_one_owner` (methods),
  `with_one_owner_a_function_value_has_one` (`var f = double`),
  `a_firmware_build_cannot_raise_a_fault_to_a_caller`, and the list
  tests for `+`.
- **Both compilers** (`emit_c.rs`, `emit_llvm.rs`): four programs are
  built every way (ordinary, `--firmware`, `--no-collector`, each
  through C and through LLVM), must print what `zeno run` prints, and
  with no collector must end with 0 blocks left:
  `after_case_none_the_value_is_there_in_every_build`,
  `a_let_at_the_top_of_the_file_can_be_read_by_every_function`,
  `a_method_can_take_out_what_its_object_owns` and
  `a_var_that_holds_a_function_can_be_given_another`.
- Every test program and example written before the audit was changed
  to the new parameter rule and still passes: 355 tests in all.

## Things to try

1. In `examples/buffer.zeno`, take `take` off `send`'s parameter and run
   `zeno check`. Which line complains, and what does it tell you to
   write?
2. Write `let take = 3` and read the message. Then find `unexpected` in
   `parser.rs`, where the message is made, and add a hint of your own
   for `unsafe`.
3. Write a function that returns the area of a `Shape or none` with a
   `match` and no `case _`. Leave out one case and read the message.
   Then leave out `case none`.
4. Put a table of numbers at the top of a file with `let`, read it from
   two functions, and build with `--no-collector --memory`. Which line
   of the C frees the table? Now change `let` to `var` and read the
   message.
5. Give the `Queue` on this page a method `clear(self)` that pops until
   the list is empty, and call it inside `for task in queue.tasks`. Then
   move the call out of the loop.
6. In `checker.rs`, find `takes_self`. Follow it from the `pop()` in a
   method's body to the call of that method. Which three checks does the
   call go through?
7. Write `function half(a: int, b: int) -> int raises ZeroDivisionError`
   two ways, so that one builds with `--firmware` and one does not.
