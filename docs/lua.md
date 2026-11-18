# Libraries of Lua

Lua is small, and is everywhere a program is put inside another
program: games, editors, network equipment, embedded devices.
LuaRocks has its modules. A Zeno program can use Lua's own library and
any module that is installed, as they are.

```zeno
import lua

let string = lua.global("string")
print(string.format("%-6s|%5.2f", "pi", 3.14159), string.rep("ab", 3), string.upper("zeno"))
```

```
pi    | 3.14 ababab ZENO
```

`lua.global("string")` is the table `string` that every program of Lua
has. From there the program is written as it would be in Lua: a dot
for what a table has, brackets for a call.

This is the third language of the second version, after Ruby and PHP
([`ruby.md`](ruby.md), [`php.md`](php.md)), and it works as the others
do ([`python.md`](python.md) has the whole story). Nothing changes for
a program that does not say `import lua`. The decisions behind it are
chapter 39 of [`decisions.md`](decisions.md).

| Written | Gives |
|---|---|
| `lua.require(name)` | the module of that name, as `require` gives it |
| `lua.global(name)` | what Lua has under that name everywhere: `string`, `table`, `math`, `os`, `io`, `print` |
| `lua.copy(value)` | a table of Lua as a list or a dict of Zeno |

## What it needs

Lua on the computer: 5.1, 5.2, 5.3, 5.4 or LuaJIT, all of which are
tested. Zeno looks for `lua`, then `lua5.4`, then `luajit`. To use
another one, name it:

```sh
ZENO_LUA=/usr/bin/lua5.3 zeno run program.zeno
```

On Windows Lua also needs LuaSocket (`luarocks install luasocket`),
because Lua comes with no way to talk to another program there. On
Linux and macOS it needs nothing more ("How it is built", below).

Lua is started the first time `lua.require` or `lua.global` is called,
once, and stays until the program ends. A program that imports `lua`
and never calls it starts nothing.

## Modules

What comes with Lua needs no `require`: `lua.global` gives it. A
module from LuaRocks is installed for the Lua that Zeno starts, and
then required by its name:

```sh
luarocks install dkjson
zeno run program.zeno
```

A file of Lua beside the program is found too. The folder the `.zeno`
file is in is the first place Lua looks, so `lua.require("shapes")`
loads `shapes.lua` from there, and the program can be run from
anywhere. A module that is not there says which Lua looked:

```
FileError: Lua has no module `lpeg`. Is it installed for this Lua (`luarocks install`)? It is Lua 5.4
```

## Several values

A function of Lua can give several values. One value is that value,
none is `none`, and several are a list of Zeno, in order:

```zeno
import lua

let string = lua.global("string")
print(string.find("hello", "l"), string.gsub("hello", "l", "L"), string.gsub("hello", "l", "L")[0])
print(string.find("hello", "z"))
```

```
[3, 3] ["heLLo", 2] heLLo
none
```

**`nil` and a message is an error.** Most functions of Lua say they
failed by giving `nil` and then why, which a program of Lua passes to
`assert`. Zeno does that itself: such a call raises an `Error` with the
message, or a `FileError` when a number of the system comes third, as
`io.open` gives ("Errors", below).

## Methods

Lua writes `object:name(x)` to call a method, which gives the object
to the function as `self`, and `module.name(x)` to call a function.
Zeno has one dot, and tells the two apart by the function:

- A function written `function Class:name()` has a first parameter
  called `self`, and is given the object.
- On Lua 5.1, and for a function written in C, which cannot say, a
  function that the table has from its metatable is given the object.
  That is how Lua makes an object, where a module holds its functions
  itself.
- A function read from anything that is not a table, a file or
  another userdata, is given it, as `file:read()`.

```zeno
import lua

let shapes = lua.require("shapes")
let p = shapes.Point(3, 4)
print(p, p.length(), p.moved(1, 1), p.x)
print(p["length"](p))
```

```
Point(3, 4) 5.0 Point(4, 5) 3
5.0
```

**The name in brackets is what the table holds**, as `t["name"]` is in
Lua, and nothing is given to it. So `p["length"](p)` is the same call
written as Lua's dot, and it reaches a function that the rule above
gets wrong. A name that is a word of Zeno is written that way too:
`string["match"](text, pattern)`.

## Values go across in two ways

**`none`, a bool, a number and a str go as themselves.** Lua's `nil`
is `none`. On Lua 5.3 and later a number knows if it is an int or a
float, and arrives as what it is. On 5.1, 5.2 and LuaJIT every number
is a float, and a whole one arrives as an int.

**A table, a function and a userdata stay in Lua.** Zeno holds a
number that Lua knows the value by, and each time the program writes
a field, a call or an item of it, Zeno asks Lua. When nothing in the
program holds it any more, Lua is told and lets it go.

A value that stays in Lua can be:

| Written | What Lua does |
|---|---|
| `thing.name` | `thing.name`; a method is given `thing` when called (above) |
| `thing.name = value` | `thing.name = value` |
| `thing(1, 2)` | the call; named arguments are a table of options, given last |
| `thing[key]`, `thing[key] = value` | `thing[key]`, which counts from 1 |
| `len(thing)` | `#thing` |
| `for item in thing { }` | of a table that is a list, its items; of any other table, its keys; of a function, what it gives until it gives `nil` |
| `item in thing` | of a table that is a list, whether it holds the item; of any other, whether it has the key |
| `thing == other` | `thing == other`, with the table's `__eq` |
| `print(thing)`, `str(thing)`, `f"{thing}"` | `tostring(thing)` where there is a `__tostring`; else the table's items as Lua writes a table |

What it cannot be: the test of an `if`, or one side of `+` or `<`.

**Lua counts from 1.** A table stays in Lua, and its items are where
Lua has them: `t[1]` is the first. A list of Zeno counts from 0, and
`lua.copy` makes one of a table.

```zeno
import lua

let list = lua.global("string").byte("abc", 1, 3)
print(list, list[0], len(list))
let table = lua.global("table")
let t = table.pack(30, 10, 20)
t.n = none
table.sort(t)
print(t, t[1], len(t), 10 in t, lua.copy(t))
for item in t {
    print(item)
}
```

```
[97, 98, 99] 97 3
{10, 20, 30} 10 3 true [10, 20, 30]
10
20
30
```

`list` is three values that `string.byte` gave, so it is a list of
Zeno and counts from 0. `t` is a table, and counts from 1.

`lua.copy` makes a list of a table whose keys are 1, 2, and so on to
its length, and a dict of any other, with everything inside copied
the same way. A key that is not text or a whole number cannot be a
key of a dict, and the copy says so. A list or a dict that Zeno gives
arrives as a new table. A `none` in a list leaves a hole in the
table, which Lua's `#` may or may not count.

**Bytes that are not text** stay in Lua: a string of Lua is bytes,
and where they read as UTF-8 they are a str. Lua's own `string.len`
counts the bytes of either.

## Named arguments are the options

Lua has no named arguments. What a library of Lua takes instead is a
table of options, last, and that is what named arguments become:

```zeno
import lua

let shapes = lua.require("shapes")
print(shapes.greet("ann", times=2, loud=true))
for word in lua.global("string").gmatch("one two three", "%a+") {
    print(word)
}
```

```
HELLO ANN, HELLO ANN
one
two
three
```

`shapes.greet("ann", times=2, loud=true)` is
`shapes.greet("ann", {times = 2, loud = true})`.

## Errors

An error of Lua is an error of Zeno, and `try` catches it.

```zeno
import lua

try {
    lua.global("io").open("/no/such/file")
} except FileError as error {
    print(error.message)
}
try {
    lua.global("error")("it broke")
} except Error as error {
    print(error.message)
}
```

```
/no/such/file: No such file or directory
it broke
```

Lua has no kinds of error: what it raises is a value, usually a
string. So an error of Lua is an `Error` with that value as its
message, and the `nil` and a message that a function gives back is
one too. A `FileError` is the one other kind: `nil`, a message and a
number of the system, which is how Lua's functions for files say
what the system refused.

Two things can go wrong before any library is reached, and both are a
`FileError`:

```
FileError: there is no Lua to run. Zeno looked for `lua`, then `lua5.4`, then `luajit`. `ZENO_LUA` can name one that is somewhere else
FileError: Lua has no module `lpeg`. Is it installed for this Lua (`luarocks install`)? It is Lua 5.4
```

A function that declares what it raises and calls into Lua says
`raises Error`, which is every kind.

## Types

As for the other languages: a value from Lua has the type that
`json.parse` gives. The checker lets everything through, a wrong name
is found when the program gets there, and `zeno check --strict`
refuses a program that uses `lua`.

A field that a table does not have is `nil` in Lua, so it is `none`,
and calling it says that `none` cannot be called.

## With the other languages in one program

A program can import several. Each value stays where it was made: a
str or a number that one gave can be passed to another, because by
then it is a value of Zeno. A table cannot be given to another:

```
error: a Lua value cannot be given to Python. Each stays where it was made
```

`lua.copy` makes it a list or dict of Zeno first, and that can go
anywhere.

## What is not there yet

- **A function of Zeno cannot be given to Lua**: `table.sort(t, f)`,
  `string.gsub(s, p, f)`. Zeno says so at the line.
- **A loop takes everything first.** Over a function that never gives
  `nil`, it does not end.
- **Built programs.** `zeno run` and `zeno test` have `lua`, with and
  without `--vm`. `zeno build`, in every form, refuses it.
- **Windows** is not tested: no Lua with LuaSocket was there to try.

## The example

[`examples/records.zeno`](../examples/records.zeno) prints the
records of a binary file field by field, as a logger or a radio wrote
them. Zeno reads text, and Lua 5.3 has `string.unpack`.

```
$ zeno run examples/records.zeno log.bin "<I4 i2 i2 B" 2
log.bin: 45 bytes, 5 records of 9 bytes
1: [0, 215, -40, 0]
2: [1000, 216, -37, 1]
```

```zeno
let fields = string.unpack(layout, bytes, at)
at = fields.pop()
```

`string.unpack` gives each field and then where the next record
begins, so the list it comes as ends with that.

## How it is built

It is the bridge that the other languages use
([`python.md`](python.md), "How it is built"), with a helper in Lua.
`src/bridge.lua` is part of the `zeno` file. The first call starts Lua
with it (`lua -e`), and the two talk one line of JSON each way, so Lua
keeps the screen and the keyboard: `print` and `io.read` work as if
Lua had been started by hand.

**Pipes, where the others have a socket.** Lua comes with no sockets.
What it can do is open a file, and on Linux and macOS a named pipe is
a file: Zeno makes two in a folder of their own (`mkfifo`), one for
each way, gives Lua their names, and removes them once both ends are
open. Windows has named pipes too, but not ones a file of Lua can
open, so there Lua connects over a socket with LuaSocket, as the
other helpers do. So does Lua where `mkfifo` is not there.

Lua also comes with no JSON, so the helper reads and writes its lines
itself.

In `src/bridge.rs` Lua is one more `Lang`. What it adds there is
`Pipes`. The tests are in `interpreter.rs`
(`a_module_of_lua_is_used_from_zeno`,
`what_is_not_a_number_or_text_stays_in_lua`,
`an_error_of_lua_is_an_error_of_zeno`), in `checker.rs` and in
`lower.rs`. Each test that runs a program runs it in both engines, and
says nothing on a machine with no `lua`. They print the same with
each of the five Luas.

## Things to try

1. Run the first program of this page. Then print
   `lua.global("_VERSION")`, set `ZENO_LUA` to another Lua, and run it
   again.

2. `lua.global("os").date("*t")` gives a table. Print it, then print
   `lua.copy` of it. Which keys does it have?

3. In the program that sorts `t` above, print `t[0]`, `t[1]` and
   `lua.copy(t)[0]`. Why are two of them the same? Then take out the
   line `t.n = none` and print `lua.copy(t)` again. What is `n`?

4. Print `lua.global("string").gsub("a-b-c", "-", "+")`. What is the
   second value?

5. Write a file `shapes.lua` beside your program with a `Point` that
   has a method `function Point:area()`, and call it from Zeno with a
   dot.

6. Make a binary file with Python's `struct.pack` and read it with
   `examples/records.zeno`.
