# Libraries of Python

Python has a library for nearly everything, and `pip install` puts any
of them on the computer. A Zeno program can use them as they are.

```zeno
import python

let statistics = python.module("statistics")
let readings = [21.5, 22.0, 23.5, 22.5]
print(statistics.mean(readings), statistics.median(readings))
```

```
22.375 22.25
```

`python.module("statistics")` is what `import statistics` is in
Python. What it gives is the module, and from there the program is
written as it would be in Python: a dot for what the module has,
brackets for a call.

Nothing changes for a program that does not say `import python`. This
is something added beside the language, and it is there only when it
is asked for. The decisions behind it are chapter 26 of
[`decisions.md`](decisions.md).

| Written | Gives |
|---|---|
| `python.module(name)` | the module of that name, as Python's `import` finds it |
| `python.copy(value)` | a list or a dict of Python as a list or a dict of Zeno |

## What it needs

Python 3 on the computer, and the library installed for it. Zeno
looks for `python3` and then `python` (on Windows `python`, `python3`,
then `py`), which is the Python a virtual environment puts first when
it is active. To use another one, name it:

```sh
ZENO_PYTHON=/opt/python3.13/bin/python3 zeno run program.zeno
```

Python is started the first time `python.module` is called, once, and
stays until the program ends. A program that imports `python` and
never calls it starts nothing.

A module is looked for where Python would look for it if the program
were a program of Python in the same place: among the packages that
are installed, and first in the folder the `.zeno` file is in. So a
`helper.py` beside the program is `python.module("helper")`, wherever
the program is run from.

## Values go across in two ways

**`none`, a bool, an int, a float and a str go as themselves.** What
Python returns of these kinds is a value of Zeno at once, and what
Zeno passes arrives as Python's own.

```zeno
let datetime = python.module("datetime")
let day = datetime.date(2026, 11, 18)
print(day, day.year + 1, day.strftime("%A"))
```

```
2026-11-18 2027 Wednesday
```

`day.year` is an int of Zeno, so `+ 1` is Zeno's. `day.strftime("%A")`
is a str of Zeno. Python's `None` is `none`, and is tested with `is
none`.

**Everything else stays in Python.** `datetime` and `day` above are
not copies of anything. Zeno holds a number that Python knows the
value by, and each time the program writes a field, a call or an item
of it, Zeno asks Python. When nothing in the program holds the value
any more, Python is told and lets it go.

A value that stays in Python can be:

| Written | What Python does |
|---|---|
| `thing.name` | `thing.name` |
| `thing.name = value` | the same |
| `thing(1, 2, key=3)` | the call, with `key` as a keyword argument |
| `thing[index]`, `thing[index] = value` | the same |
| `len(thing)` | `len(thing)` |
| `for item in thing { }` | every item, taken when the loop begins |
| `item in thing`, `item not in thing` | `item in thing` |
| `thing == other`, `thing != other` | `thing == other` |
| `print(thing)`, `str(thing)`, `f"{thing}"` | `str(thing)` |

```zeno
let later = day.replace(month=12)
print(later.isoformat(), later == day, [day, later])
```

```
2026-12-18 false [datetime.date(2026, 11, 18), datetime.date(2026, 12, 18)]
```

Inside a list a Python value is shown as Python writes it (`repr`),
as a str of Zeno is shown with its quotes there.

What it cannot be: the test of an `if` (a condition is `true` or
`false` in Zeno, so write `len(thing) > 0` or `thing is not none`), or
one side of `+` or `<` (take the number out first: `builtins.float(x)`
gives a float of Zeno).

## Lists and dicts

A list of Python stays in Python too, and that is on purpose.

```zeno
let builtins = python.module("builtins")
let names = builtins.list(["cy", "ann"])
names.append("bob")
names.sort()
print(names, len(names), names[0], "bob" in names)
for name in names {
    print(name.upper())
}
```

```
['ann', 'bob', 'cy'] 3 ann true
ANN
BOB
CY
```

`names` is the one list there is. `append` and `sort` here are
Python's, and they change the list that Python has. If the list came
across as a copy, `names.append("bob")` would add to the copy and
Python would never see it, with nothing to say so (26.5).

When a list of Zeno is what the program wants, to pass to a function
written `list[str]` or to use Zeno's own methods on, `python.copy`
makes one:

```zeno
let mine = python.copy(names)
mine.append("dee")
print(mine, len(names))
```

```
["ann", "bob", "cy", "dee"] 3
```

`copy` takes a list, a tuple or a dict of Python and gives a list or a
dict of Zeno, with every list and dict inside it copied as well. A
tuple becomes a list. Whatever is not one of these stays in Python
inside the copy. A dict of Python whose keys are not all str or int
cannot be a dict of Zeno, and `copy` raises a `ValueError` that names
the key.

**The other way is always a copy.** A list or dict of Zeno that is
passed to Python arrives as a new list or dict of Python:

```zeno
let xs = [3, 1, 2]
python.module("random").shuffle(xs)
print(xs)                               # [3, 1, 2]: Python shuffled its copy
```

To have Python work on a list and see the result, make the list in
Python (`builtins.list(xs)`), and copy it back.

A dict of Zeno has values of one type, and a library often wants
options of several. Python's own `dict` makes one from named
arguments:

```zeno
let options = builtins.dict(quiet=true, retries=3, outtmpl="%(title)s.%(ext)s")
```

## Errors

An error that Python raises is an error of Zeno, and `try` catches it.

```zeno
let json = python.module("json")
try {
    json.loads("{oops")
} except ValueError as error {
    print(error.message)
}
```

```
JSONDecodeError: Expecting property name enclosed in double quotes: line 1 column 2 (char 1)
```

| In Python | In Zeno |
|---|---|
| `ValueError`, `IndexError`, `KeyError`, `ZeroDivisionError`, `OverflowError`, and their own kinds | the kind of the same name |
| `OSError` and its kinds (`FileNotFoundError`, `PermissionError`, ...) | `FileError` |
| anything else | `Error` |

The message begins with what Python called the error when that is not
the name of the kind: `TypeError: object of type 'int' has no len()`.

Two things can go wrong before any library is reached, and both are a
`FileError`, as a program that is not installed is for `program.run`:

```
FileError: there is no Python to run. Zeno looked for `python3`, then `python`. `ZENO_PYTHON` can name one that is somewhere else
FileError: Python has no module `yt_dlp` (No module named 'yt_dlp'). Is it installed for this Python? It is /usr/bin/python3
```

What Python can raise is not known before the program runs. A
function that declares what it raises and calls into Python says
`raises Error`, which is every kind.

## Types

What a module has is known when the program runs and not before, so a
value from Python has the type that `json.parse` gives (18.1): the
checker lets everything through, and a wrong name is an error at its
line when the program gets there.

```zeno
let title: str = about["title"]       # checked when it happens
```

So `zeno check --strict` refuses a program that uses `python`, as it
refuses one that uses `json.parse`.

## What is not there yet

- **A function of Zeno cannot be given to Python.** A library that
  calls back, for progress or for a sort key, cannot call into Zeno
  yet. Zeno says so at the line.
- **A name that is a word of Zeno.** `re.match` does not parse,
  because `match` is a word of the language. Python's own `getattr`
  reaches it: `builtins.getattr(re, "match")("[a-z]+", text)`.
- **A loop takes everything first.** `for line in thing` asks Python
  for all the items, then loops. Over something that never ends, it
  never begins.
- **Built programs.** `zeno run` and `zeno test` have `python`, with
  and without `--vm`. `zeno build`, in every form, refuses it.

## The example

[`examples/video.zeno`](../examples/video.zeno) does what
[`download.zeno`](../examples/download.zeno) does, with yt-dlp as a
library and not as a program. Because the answers come back as values,
it can say what a video is before it saves anything:

```
$ zeno run examples/video.zeno https://youtu.be/VIDEO
Big Buck Bunny
  length    10:35
  from      Blender
  licence   Creative Commons Attribution license (reuse allowed)
```

```zeno
let about = downloader.extract_info(address, download=false)
print(about.get("title", "(no title)"))
let seconds = about.get("duration")
if seconds is not none {
    print(f"  length    {as_minutes(int(seconds))}")
}
```

`about` is a dict of Python, and `get` is Python's. The title arrives
as a str, the length as a number or `none`.

## How it is built

**Python is a program of its own, beside Zeno's.** `src/bridge.py` is
a small Python program that is part of the `zeno` file. The first
`python.module` starts Python with it. It keeps every value Zeno has
been given a number for in a dict, `kept`, and does what it is asked.

**They talk over a socket of this machine.** Zeno opens one that only
this computer can reach, on a port the system picks, and gives Python
the port and a word made up for this run. Python connects and says the
word, and only then is it listened to. Because the conversation has a
line of its own, Python keeps the screen and the keyboard: its
`print`, a progress bar that redraws itself and `input()` all work as
if Python had been started by hand (26.3).

**One line asks, one line answers.** Each is JSON:

```
{"op":"get","on":["p",3],"name":"year"}             day.year
{"ok": 2026}

{"op":"call","on":["p",5],"args":["%A"],"named":[]}
{"ok": "Wednesday"}

{"op":"call","on":["p",9],"args":["{oops"],"named":[],"free":[5,7]}
{"err": ["ValueError", "JSONDecodeError: Expecting ..."]}
```

`["p", 3]` is the value Python keeps under 3. A list that goes by
value is `["l", [...]]` and a dict `["d", [[key, value], ...]]`, so
that nothing can be taken for something else. `"free"` rides along
with the next thing asked: the numbers Zeno has let go of since.

**In Zeno** the value is `Value::Foreign`, which holds the number.
`src/bridge.rs` has the socket and the lines, and one function for
each row of the table above. They are called from the places both
engines already share (`ops::field`, `ops::item`, `ops::items`, `len`,
`equal`) and from the one arm each engine has for a call. Nothing in
`bridge.rs` knows Python but the name of the helper, so another
language is another helper that speaks the same lines.

The checker's part is two lines of `builtin`: `python.module` and
`python.copy` give a value whose type is known when the program runs.

The tests are in `interpreter.rs` (`a_module_of_python_is_used_from_zeno`,
`what_is_not_a_number_or_text_stays_in_python`,
`an_error_of_python_is_an_error_of_zeno`), in `bridge.rs` for the
lines, in `checker.rs` and in `lower.rs`. Each test that runs a
program runs it in both engines, and says nothing on a machine with no
Python.

## Things to try

1. Run the first program of this page. Then misspell `statistics` and
   read the message. Which Python does it name?

2. Print `python.module("sys").version` and
   `python.module("sys").executable`. Start a virtual environment and
   run it again.

3. `builtins.sorted([3, 1, 2])` gives a list that stays in Python.
   Print it, then print `python.copy` of it. What is different about
   the two lines, and why?

4. Write a function `mean(xs: list[float]) -> float` in Zeno and pass
   it `python.copy(builtins.list([1.0, 2.0]))`. Then pass it the list
   without `copy`, and read what the checker and the program say.

5. With `pip install requests`, fetch a page:
   `python.module("requests").get("https://zeno-lang.org").status_code`.
   What kind of error do you get with the network off, and what does
   its message begin with?

6. Run `examples/video.zeno` with the address of a video of your own,
   first with no folder, then with one.
