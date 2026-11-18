# Libraries of JavaScript

npm has more packages than any other place, and Node comes with a
library of its own for files, the network, compression and
cryptography. A Zeno program can use all of it as it is.

```zeno
import node

let path = node.module("path")
print(path.basename("/home/ann/notes.txt"), path.extname("notes.txt"))
```

```
notes.txt .txt
```

`node.module("path")` is what `require("path")` is in JavaScript.
What it gives is the module, and from there the program is written as
it would be in JavaScript: a dot for what the module has, brackets for
a call.

This is the second language of three that work the same way. Python
was the first ([`python.md`](python.md)), and this page follows that
one section by section. Nothing changes for a program that does not
say `import node`. The decisions behind it are chapter 28 of
[`decisions.md`](decisions.md).

| Written | Gives |
|---|---|
| `node.module(name)` | the module of that name, as `require` finds it |
| `node.global(name)` | what JavaScript has under that name everywhere: `JSON`, `Math`, `Date`, `process` |
| `node.new(Class, ...)` | a new object of the class, as `new Class(...)` makes it |
| `node.copy(value)` | an array or an object of JavaScript as a list or a dict of Zeno |

## What it needs

Node on the computer. Zeno looks for `node`, and is tested with Node
22 and 24. To use another one, name it:

```sh
ZENO_NODE=/opt/node-22/bin/node zeno run program.zeno
```

Node is started the first time `node.module` or `node.global` is
called, once, and stays until the program ends. A program that imports
`node` and never calls it starts nothing.

## Modules and packages

A module that comes with Node needs nothing more: `fs`, `path`, `os`,
`crypto`, `zlib`, `http`, `child_process`, `fs/promises` and the
rest.

A package from npm is looked for where Node would look for it if the
program were a program of JavaScript in the same place: in
`node_modules` of the folder the `.zeno` file is in, then of each
folder above it. So a package is installed in the folder of the
program, and the program can be run from anywhere:

```sh
npm install dayjs
zeno run program.zeno
```

```zeno
import node

let dayjs = node.module("dayjs")
let day = dayjs("2026-11-18")
print(day.format("dddd"), day.year() + 1, day.add(1, "month").format("YYYY-MM-DD"))
```

```
Wednesday 2027 2026-12-18
```

A package that JavaScript can only `import`, and not `require`, is
loaded as well. What such a package calls its default is its
`.default`: `node.module("chalk").default`.

A file of JavaScript beside the program is a module too, by its path
from the program: `node.module("./helper.js")`.

## What is not in a module

Much of JavaScript is not in a module at all. `JSON`, `Math`, `Date`,
`Intl`, `Buffer` and `process` are there everywhere, and
`node.global` gives them. A class is made into an object with `new`
in JavaScript, and with `node.new` here.

```zeno
let JSON = node.global("JSON")
let about = JSON.parse("{\"name\": \"ann\", \"age\": 31, \"tags\": [\"a\", \"b\"]}")
print(about.name, about.age + 1, about.tags, about.email is none)

let Date = node.global("Date")
let start = node.new(Date, 2026, 10, 18)
print(start.getFullYear(), start.toDateString())
```

```
ann 32 [ 'a', 'b' ] true
2026 Wed Nov 18 2026
```

`about.email` is `none` and not an error. That is JavaScript's rule:
a field an object does not have is `undefined`. JavaScript's `null`
and its `undefined` are both `none` in Zeno.

## Values go across in two ways

**`none`, a bool, a number and a str go as themselves.**

JavaScript has one kind of number where Zeno has two. A number that
is whole arrives as an int, and any other as a float:

```zeno
let Math = node.global("Math")
print(Math.max(3, 9, 4), Math.sqrt(16.0), Math.sqrt(2.0))
```

```
9 4 1.4142135623730951
```

`Math.sqrt(16.0)` is `4`, an int, because JavaScript does not keep
whether a whole number was written with a point. Where a float is
needed, `float(x)` makes one.

A number of JavaScript is exact up to 9007199254740991 (2 to the power
of 53, less one). A whole number past that arrives as a float, which
says that it is no longer exact. An int of Zeno past that goes to
JavaScript as a `BigInt`, which is how JavaScript writes an exact
large number, and a `BigInt` comes back as an int.

**Everything else stays in JavaScript.** `path`, `about` and `start`
above are not copies of anything. Zeno holds a number that Node knows
the value by, and each time the program writes a field, a call or an
item of it, Zeno asks Node. When nothing in the program holds the
value any more, Node is told and lets it go.

A value that stays in JavaScript can be:

| Written | What JavaScript does |
|---|---|
| `thing.name` | `thing.name` |
| `thing.name = value` | the same |
| `thing(1, 2)` | the call |
| `thing(1, key=3)` | `thing(1, {key: 3})`: the named ones are one object, which goes last |
| `thing[index]`, `thing[index] = value` | the same; on a `Map`, `get` and `set` |
| `len(thing)` | `thing.length`, or `thing.size`, or how many keys an object has |
| `for item in thing { }` | every item, taken when the loop begins; of an object, its keys |
| `item in thing` | `includes` of an array or a str, `has` of a `Set` or a `Map`, `in` of an object |
| `thing == other` | `thing === other` |
| `print(thing)`, `str(thing)`, `f"{thing}"` | what `console.log(thing)` shows, or what the value writes itself as (a date, an error, a URL) |

`thing["name"]` is `thing.name` in JavaScript, and so it is here.
That is the way to a name that is a word of Zeno:
`Array["from"](items)`.

Two objects are equal when they are the same object. Two arrays with
the same items are not, as in JavaScript. To compare what is in them,
compare their copies: `node.copy(a) == node.copy(b)`.

What it cannot be: the test of an `if` (write `len(thing) > 0` or
`thing is not none`), or one side of `+` or `<`.

## Named arguments are the options

JavaScript has no named arguments. A function there takes its options
as one object, which is its last argument. So the named arguments of
a call are gathered into that object:

```zeno
let money = node.global("Intl").NumberFormat("en-IN", style="currency", currency="INR")
print(money.format(1234567.5))

let text = node.module("fs").readFileSync("notes.txt", encoding="utf8")
```

```
₹12,34,567.50
```

The first is `Intl.NumberFormat("en-IN", {style: "currency",
currency: "INR"})`.

To make such an object by itself, to keep it or to add to it,
JavaScript's own `Object` makes one from named arguments:

```zeno
let options = node.global("Object")(recursive=true, force=true, retries=3)
print(options, options.retries + 1)
```

```
{ recursive: true, force: true, retries: 3 } 4
```

`node.new` takes no named arguments. A class that wants options is
given such an object.

## What takes time is waited for

Much of Node answers later: a function gives a promise, and the answer
arrives in it. Zeno waits. A call that gives a promise gives its
answer, and the program goes on when it is there.

```zeno
let files = node.module("fs/promises")
let text = files.readFile("notes.txt", "utf8")
print(len(text))
```

`text` is a str. There is no `await` to write, and no promise to hold.
A promise that fails is an error at the line of the call.

## Arrays and objects

An array of JavaScript stays in JavaScript, and that is on purpose.

```zeno
let Array = node.global("Array")
let names = Array.of("cy", "ann")
names.push("bob")
names.sort()
print(names, len(names), names[0], "bob" in names)
for name in names {
    print(name.upper())
}
```

```
[ 'ann', 'bob', 'cy' ] 3 ann true
ANN
BOB
CY
```

`names` is the one array there is. `push` and `sort` are JavaScript's,
and they change the array that Node has. Each `name` in the loop is a
str of Zeno, so `upper` is Zeno's.

When a list of Zeno is what the program wants, `node.copy` makes one:

```zeno
let mine = node.copy(names)
mine.append("dee")
print(mine, len(names))
```

```
["ann", "bob", "cy", "dee"] 3
```

`copy` takes an array, a `Set` or an array of bytes and gives a list;
it takes a plain object or a `Map` and gives a dict. Every array and
object inside is copied as well. Whatever is not one of these, a date
or an object of a class, stays in JavaScript inside the copy. A `Map`
whose keys are not all str or int cannot be a dict of Zeno, and `copy`
raises a `ValueError` that names the key.

**The other way is always a copy.** A list of Zeno that is passed to
JavaScript arrives as a new array, and a dict as a new object:

```zeno
let xs = [3, 1, 2]
Array.prototype.sort.call(xs)
print(xs)                               # [3, 1, 2]: JavaScript sorted its copy
```

To have JavaScript work on a list and see the result, make the array
there (`Array["from"](xs)`), and copy it back.

## Errors

An error that JavaScript throws is an error of Zeno, and `try` catches
it.

```zeno
try {
    node.global("JSON").parse("{oops")
} except Error as error {
    print(error.message)
}
try {
    node.module("fs").readFileSync("/no/such/file")
} except FileError as error {
    print(error.message)
}
```

```
SyntaxError: Expected property name or '}' in JSON at position 1 (line 1 column 2)
ENOENT: no such file or directory, open '/no/such/file'
```

| In JavaScript | In Zeno |
|---|---|
| an error the system gave: no such file, no leave to read it, no network | `FileError` |
| anything else | `Error` |

The message begins with what JavaScript called the error, unless that
was only `Error`. JavaScript can throw a value that is not an error at
all, and that is an `Error` whose message is `thrown: ` and the value.

JavaScript has no kinds of error that match Zeno's `ValueError` or
`IndexError`. Reading past the end of an array is not an error there:
it gives `undefined`, which is `none`.

Two things can go wrong before any library is reached, and both are a
`FileError`:

```
FileError: there is no Node to run. Zeno looked for `node`. `ZENO_NODE` can name one that is somewhere else
FileError: Node has no module `yaml`. Is it installed in this folder (`npm install yaml`)? This folder is /home/ann/project
```

A function that declares what it raises and calls into JavaScript says
`raises Error`, which is every kind.

## Types

As for Python: a value from JavaScript has the type that `json.parse`
gives. The checker lets everything through, a wrong name is found when
the program gets there, and `zeno check --strict` refuses a program
that uses `node`.

## Python and JavaScript in one program

A program can import both. Each value stays where it was made: a str
or a number that one gave can be passed to the other, because by then
it is a value of Zeno. A value that stayed in Python cannot be given
to JavaScript, or the other way:

```
error: a JavaScript value cannot be given to Python. Each stays where it was made
```

`python.copy` or `node.copy` makes it a list or dict of Zeno first,
and that can go anywhere.

## What is not there yet

- **A function of Zeno cannot be given to JavaScript.** Much of
  JavaScript is written with callbacks: `items.map(f)`, `on("data",
  f)`, a server that calls a function for each request. None of that
  can call into Zeno yet, and Zeno says so at the line. What gives a
  promise instead works (above), and most of Node has that form:
  `fs/promises`, `timers/promises`, `fetch`.
- **Node ends when the program ends.** A timer or a server that was
  started in Node does not keep the program running after its last
  line.
- **A loop takes everything first**, as for Python.
- **Built programs.** `zeno run` and `zeno test` have `node`, with and
  without `--vm`. `zeno build`, in every form, refuses it.

## The example

[`examples/checksum.zeno`](../examples/checksum.zeno) prints the
SHA-256 of a file, the number a download page shows beside a file.
Zeno has no SHA-256 of its own, and Node has one in `crypto`.

```
$ zeno run examples/checksum.zeno zeno.tar.gz
43bc7d3552e7c8d65f1dcca8c0a5a02256158dc045769eb8f690189d096d0336  zeno.tar.gz
```

```zeno
function sha256_of(name: str) -> str raises Error {
    let hash = node.module("crypto").createHash("sha256")
    hash.update(node.module("fs").readFileSync(name))
    return hash.digest("hex")
}
```

The hash and the bytes of the file stay in JavaScript. Only the answer
comes across, as a str.

## How it is built

It is the bridge that Python uses ([`python.md`](python.md), "How it
is built"), with another helper. `src/bridge.js` is a small JavaScript
program that is part of the `zeno` file. The first call starts Node
with it, and the two talk over a socket of this machine, one line of
JSON each way, so Node keeps the screen and the keyboard:
`console.log` and a progress bar work as if Node had been started by
hand.

The lines are the same, with three more:

```
{"op":"global","name":"JSON"}                          node.global("JSON")
{"ok": ["p", 2]}

{"op":"new","on":["p",4],"args":[2026,10,18],"named":[]}
{"ok": ["p", 5]}

{"op":"call","on":["p",7],"args":[["i","9007199254740993"]],"named":[]}
```

`["i", "9007199254740993"]` is an int that JavaScript's own number
cannot hold exactly, written as text so that nothing is lost on the
way.

**What a method is called on.** In JavaScript `thing.name` gives a
function that has forgotten `thing`. So the helper keeps, with each
value, the object it was read from, and calls it on that.
`start.getFullYear()` works, and so does `let year =
start.getFullYear` with `year()` later.

**One thing at a time.** Node does many things at once by nature. The
helper answers each line before it reads the next, and waits for a
promise before it answers, so the program in Zeno stays one line after
another.

In `src/bridge.rs` the language is one value, `Lang`, that picks the
helper, the program to start and the name used in messages. Everything
else there is shared. The tests are in `interpreter.rs`
(`a_module_of_node_is_used_from_zeno`,
`what_is_not_a_number_or_text_stays_in_node`,
`an_error_of_node_is_an_error_of_zeno`), in `checker.rs` and in
`lower.rs`. Each test that runs a program runs it in both engines, and
says nothing on a machine with no Node.

## Things to try

1. Run the first program of this page. Then misspell `path` and read
   the message. Which folder does it name?

2. Print `node.global("process").version` and
   `node.global("process").execPath`. Set `ZENO_NODE` to another Node
   and run it again.

3. `node.global("Array").of(3, 1, 2)` gives an array that stays in
   JavaScript. Print it, then print `node.copy` of it. What is
   different about the two lines, and why?

4. Print `node.global("Math").sqrt(16.0)` and
   `node.global("Math").sqrt(2.0)`. One is an int and one a float.
   Pass each to a function written `half(x: float) -> float` and read
   what happens.

5. With `npm install dayjs`, print the day of the week you were born
   on.

6. Run `examples/checksum.zeno` on a file, then run `sha256sum` (or
   `shasum -a 256`) on the same file and compare.
