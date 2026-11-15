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
