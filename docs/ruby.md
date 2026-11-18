# Libraries of Ruby

Ruby comes with a large library of its own, for dates, YAML, CSV, the
network and much else, and RubyGems has a gem for most of what is
left. A Zeno program can use all of it as it is.

```zeno
import ruby

ruby.require("date")
let Date = ruby.constant("Date")
let day = Date.new(2026, 11, 18)
print(day.strftime("%A"), day.year() + 1, day.next_month())
```

```
Wednesday 2027 2026-12-18
```

`ruby.require("date")` is what `require "date"` is in Ruby: it loads
the library. `ruby.constant("Date")` is the class that the library
brought, by the name Ruby writes it with. From there the program is
written as it would be in Ruby, with one difference: **a call has
brackets**, also when it is given nothing (`day.year()`).

This is the first language of the second version. Python, JavaScript
and Java were the first ([`python.md`](python.md),
[`node.md`](node.md), [`java.md`](java.md)), and Ruby works the same
way. Nothing changes for a program that does not say `import ruby`.
The decisions behind it are chapter 34 of
[`decisions.md`](decisions.md).

| Written | Gives |
|---|---|
| `ruby.require(name)` | nothing; from then on Ruby has what that library or gem brings |
| `ruby.constant(name)` | the class, module or other constant of that name: `"Date"`, `"Net::HTTP"`, `"Kernel"` |
| `ruby.symbol(name)` | what `:name` is in Ruby |
| `ruby.copy(value)` | an Array, a Set or a Hash of Ruby as a list or a dict of Zeno |

## What it needs

Ruby on the computer. Zeno looks for `ruby`, and is tested with Ruby
3.1, 3.2 and 3.3. To use another one, name it:

```sh
ZENO_RUBY=/opt/ruby-3.3/bin/ruby zeno run program.zeno
```

Ruby is started the first time one of the functions of `ruby` is
called, once, and stays until the program ends. A program that
imports `ruby` and never calls it starts nothing.

## Libraries and gems

A library that comes with Ruby needs nothing more: `date`, `json`,
`yaml`, `csv`, `set`, `time`, `net/http`, `uri`, `digest`,
`fileutils`, `securerandom` and the rest.

A gem is installed as for any program of Ruby, and then required by
its name:

```sh
gem install rainbow
zeno run program.zeno
```

```zeno
import ruby

ruby.require("rainbow")
let Rainbow = ruby.constant("Rainbow")
Rainbow.enabled = true
print(ruby.constant("Kernel").send("Rainbow", "ready").green().bright())
```

That prints `ready` in bright green.

**A file of Ruby beside the program** is found too. The folder the
`.zeno` file is in is the first place Ruby looks, so
`ruby.require("helper")` loads `helper.rb` from there, and the program
can be run from anywhere.

**A `Gemfile` beside the program** is the one Bundler reads. A program
that wants exactly the gems the `Gemfile` names says so as a program
of Ruby does, before any other `require`:

```zeno
ruby.require("bundler/setup")
```

## Constants

What a library brings to Ruby is a constant: a class, a module, or a
plain value with a name that begins with a capital letter.
`ruby.constant` gives it by the name Ruby writes, with `::` for what
is inside something else. After a constant, a dot and a capital letter
is the constant inside it.

```zeno
import ruby

let Math = ruby.constant("Math")
print(Math.PI, Math.sqrt(16), ruby.constant("RUBY_VERSION") != "", ruby.constant("Float::MAX") > 1.0)
ruby.require("net/http")
print(ruby.constant("Net::HTTP"), ruby.constant("Net").HTTP)
```

```
3.141592653589793 4.0 true true
Net::HTTP Net::HTTP
```

`Math`, `Kernel`, `File`, `Time`, `Array`, `Hash`, `String` and
`Integer` are there with no `require`. A constant that a library
brings is there after the `require`:

```
Error: Ruby has no constant `Date`. One that a library brings is there after `ruby.require` of the library
```

An object is made as Ruby makes it, with `.new(...)`.

## A call has brackets

In Ruby `words.size` is a call. In Zeno a call has brackets, always,
and that holds here: `words.size()`. Without them, what the program
has is the method, to be called later, as a method of Zeno would be.

```zeno
import ruby

let words = ruby.constant("Array").new()
words.push("pear")
words.push("apple")
print(words.size(), words.sort(), words.sort().first(), words.join(" and "))
print(words.size)
```

```
2 ["apple", "pear"] apple pear and apple
method `size` of an Array. In Zeno a call has brackets: `.size()`
```

**A name Zeno cannot write after a dot** goes through Ruby's own
`send`, which calls a method by its name as text. That is every name
that ends in `?` or `!`, every operator, and the names that are words
of Zeno, `class`, `raise`, `in`, `not` and the others:

```zeno
print(words.send("empty?"), words.send("include?", "pear"), words.send("class"), words.send("+", words))
```

```
false true Array ["pear", "apple", "pear", "apple"]
```

A name that only lacks its `?` is pointed out:

```
Error: an Array has no method `frozen`. It has `frozen?`, which is written `.send("frozen?")` in Zeno
```

**What Ruby writes with no object in front**, `Integer("42")`,
`format(...)`, `rand(6)`, `puts`, is a method of `Kernel`:

```zeno
let Kernel = ruby.constant("Kernel")
print(Kernel.format("%05.1f", 3.14159), Kernel.send("Integer", "ff", 16))
```

```
003.1 255
```

`Integer` there begins with a capital letter, so after a dot it would
be read as the constant, the class `Integer`. `send` says that the
method is meant.

## Values go across in two ways

**`none`, a bool, a number and a str go as themselves.** Ruby's `nil`
is `none`, an Integer is an int, a Float is a float, and a String is a
str.

**Everything else stays in Ruby.** `day` and `words` above are not
copies of anything. Zeno holds a number that Ruby knows the value by,
and each time the program writes a call or an item of it, Zeno asks
Ruby. When nothing in the program holds the value any more, Ruby is
told and lets it go.

A value that stays in Ruby can be:

| Written | What Ruby does |
|---|---|
| `thing.name` | the method `name`, to be called now or later; after a class or a module, the constant `Name` |
| `thing.name = value` | `thing.name = value`, the method `name=` |
| `thing(1, 2)` | on a method, the call; on a Proc or anything else that has `call`, that |
| `thing[index]`, `thing[index] = value` | the same, `[]` and `[]=` |
| `len(thing)` | `thing.length`, or `thing.size` |
| `for item in thing { }` | everything `each` gives, taken when the loop begins; of a Hash, its keys |
| `item in thing` | `thing.include?(item)` |
| `thing == other` | `thing == other` |
| `print(thing)`, `str(thing)`, `f"{thing}"` | `thing.to_s` |

What it cannot be: the test of an `if` (write `len(thing) > 0`, or
call a method that gives true or false), or one side of `+` or `<`.
`send("+", other)` is the operator of Ruby.

Four things are Ruby's own.

**Text and bytes are one String in Ruby.** Text comes across as a
str, whatever encoding Ruby had it in. Bytes that are not text, the
inside of an image or a zip, stay in Ruby, where `bytes()`,
`unpack1` and the rest read them. Bytes that happen to be text are a
str as well, and given back to Ruby they are the same bytes.

**An Integer of Ruby has no largest value.** One that fits 64 bits is
an int. A larger one stays in Ruby, prints in full, and can be used
there.

**A symbol stays in Ruby.** `:up` is not the text `"up"` there, and a
Hash that has the key `:up` does not have the key `"up"`. So a symbol
that Ruby gives stays a symbol, and `ruby.symbol("up")` makes one.
Printed, it is its name.

**An Array or a Hash that Zeno gives is a copy.** A list of Zeno
arrives as a new Array, and a dict as a new Hash with the same keys.
What Ruby then does to it, the list in Zeno does not see.

```zeno
import ruby

let text = ruby.constant("String").new("zeno")
print(text.upper(), len(text))
let bytes = ruby.constant("Array").new(2, 255).pack("C*")
print(bytes, len(bytes), bytes.unpack1("H*"))
let big = ruby.constant("Kernel").send("Integer", "123456789012345678901234567890")
print(big, big.send("*", 2), big.bit_length())
print(ruby.symbol("up"), ruby.symbol("up") == "up", str(ruby.symbol("up")) == "up")
```

```
ZENO 4
"\xFF\xFF" 2 ffff
123456789012345678901234567890 246913578024691357802469135780 97
up false true
```

## Named arguments are keyword arguments

`name=value` in a call is `name: value` in Ruby.

```zeno
import ruby

let Kernel = ruby.constant("Kernel")
print(Kernel.send("Integer", "ff", 16), Kernel.send("Integer", "x", exception=false))
let Point = ruby.constant("Struct").new(ruby.symbol("x"), ruby.symbol("y"), keyword_init=true)
let point = Point.new(x=1, y=2)
point.x = 10
print(point.x() + point.y(), point == Point.new(x=10, y=2))
```

```
255 none
12 true
```

## Arrays and hashes

```zeno
import ruby

let ages = ruby.constant("Hash").new()
ages["ann"] = 31
ages[ruby.symbol("bob")] = 40
print(len(ages), ages["ann"], ages[ruby.symbol("bob")], ages["bob"], "ann" in ages)
for name in ages {
    print(name)
}
let copy = ruby.copy(ages)
print(copy, copy["bob"] + 1)
```

```
2 31 40 none true
ann
bob
["ann": 31, "bob": 40] 41
```

`ages["bob"]` is `none`: the key is the symbol, and Ruby gives `nil`
for a key a Hash does not have.

`ruby.copy` makes a list of an Array or a Set, and a dict of a Hash,
with everything inside copied the same way. A copy is plain data, so
a symbol in it is copied as its name, as a key and as a value. A key
that is neither text, a symbol nor an int cannot be a key of a dict,
and the copy says so. A value that is one of Zeno's already is given
back as it is.

A loop over a Hash with two names gives a count and a key, as over a
list. For the keys with their values, loop over the copy:

```zeno
for name, age in ruby.copy(ages) {
    print(name, age)
}
```

## Errors

An exception of Ruby is an error of Zeno, and `try` catches it.

```zeno
import ruby

let Kernel = ruby.constant("Kernel")
try {
    Kernel.send("Integer", "twelve")
} except ValueError as error {
    print(error.message)
}
try {
    ruby.constant("File").read("/no/such/file")
} except FileError as error {
    print(error.message)
}
try {
    Kernel.send("raise", "it broke")
} except Error as error {
    print(error.message)
}
```

```
ArgumentError: invalid value for Integer(): "twelve"
Errno::ENOENT: No such file or directory @ rb_sysopen - /no/such/file
it broke
```

| In Ruby | In Zeno |
|---|---|
| `ZeroDivisionError` | `ZeroDivisionError` |
| `KeyError` | `KeyError` |
| `IndexError`, `StopIteration` | `IndexError` |
| `ArgumentError` | `ValueError` |
| `Errno::...` (what the system refused), `IOError` | `FileError` |
| anything else | `Error` |

The message begins with what Ruby called the exception, unless that
is the name of the kind already, or the plain `RuntimeError` that
`raise "text"` makes.

Two things can go wrong before any library is reached, and both are a
`FileError`:

```
FileError: there is no Ruby to run. Zeno looked for `ruby`. `ZENO_RUBY` can name one that is somewhere else
FileError: Ruby has no library `nokogiri` (cannot load such file -- nokogiri). Is its gem installed for this Ruby? It is /usr/bin/ruby
```

A function that declares what it raises and calls into Ruby says
`raises Error`, which is every kind.

## Types

As for the other three: a value from Ruby has the type that
`json.parse` gives. The checker lets everything through, a wrong name
is found when the program gets there, and `zeno check --strict`
refuses a program that uses `ruby`.

## With the other languages in one program

A program can import several. Each value stays where it was made: a
str or a number that one gave can be passed to another, because by
then it is a value of Zeno. A value that stayed in one cannot be
given to another:

```
error: a Ruby value cannot be given to Python. Each stays where it was made
```

`ruby.copy` makes it a list or dict of Zeno first, and that can go
anywhere.

## What is not there yet

- **A block cannot be given.** Much of Ruby is written with blocks:
  `items.map { ... }`, `File.open(name) { ... }`, `each`. A function
  of Zeno cannot be the block yet, and Zeno says so at the line. Most
  of them have another form: a `for` loop in Zeno goes through
  anything that has `each`, `File.read` and `File.write` need no
  block, and `items.map()` with no block gives something a `for` can
  go through.
- **A loop takes everything first**, as for Python. Over something
  that has no end, `1..`, `loop`, Zeno says so and does not begin.
- **A class of Zeno cannot inherit from a class of Ruby.**
- **Built programs.** `zeno run` and `zeno test` have `ruby`, with and
  without `--vm`. `zeno build`, in every form, refuses it.

## The example

[`examples/settings.zeno`](../examples/settings.zeno) prints a
settings file written in YAML as JSON. Zeno reads JSON and not YAML,
and Ruby has `yaml`.

```yaml
name: sensor-node
uart:
  baud: 115200
  parity: none
pins: [2, 4, 5]
sleep: true
```

```
$ zeno run examples/settings.zeno board.yml
{"name":"sensor-node","uart":{"baud":115200,"parity":"none"},"pins":[2,4,5],"sleep":true}
$ zeno run examples/settings.zeno board.yml uart
{"baud":115200,"parity":"none"}
```

```zeno
function settings_of(name: str) raises Error {
    ruby.require("yaml")
    let YAML = ruby.constant("YAML")
    return ruby.copy(YAML.safe_load(files.read(name)))
}
```

`YAML` and the Hash it makes stay in Ruby. `ruby.copy` makes the Hash
a dict of Zeno, and from there the program is all Zeno.

## How it is built

It is the bridge that Python, JavaScript and Java use
([`python.md`](python.md), "How it is built"), with a helper in Ruby.
`src/bridge.rb` is part of the `zeno` file. The first call starts
Ruby with it, and the two talk over a socket of this machine, one
line of JSON each way, so Ruby keeps the screen and the keyboard:
`puts` and `gets` work as if Ruby had been started by hand.

The lines are the same as Python's, with one more:

```
{"op":"import","name":"date"}                          ruby.require("date")
{"ok": null}

{"op":"global","name":"Date"}                          ruby.constant("Date")
{"ok": ["p", 1]}

{"op":"symbol","name":"up"}                            ruby.symbol("up")
{"ok": ["p", 2]}
```

**A method not called yet is a value the helper makes up.** In Ruby a
dot is the call, so there is nothing that `day.year` could give but
the year. The helper has `Bound`, the object and the name, and
`public_send`s it when Zeno calls. So only what is public is reached,
and `send` is there for the rest.

In `src/bridge.rs` Ruby is one more `Lang`. The tests are in
`interpreter.rs` (`a_library_of_ruby_is_used_from_zeno`,
`what_is_not_a_number_or_text_stays_in_ruby`,
`an_error_of_ruby_is_an_error_of_zeno`), in `checker.rs` and in
`lower.rs`. Each test that runs a program runs it in both engines, and
says nothing on a machine with no Ruby. No test compares what Ruby
words itself, a Hash as Ruby prints it or the text of an exception,
because each Ruby words those differently.

## Things to try

1. Run the first program of this page. Then take the brackets off
   `day.year()` and read what is printed.

2. Print `ruby.constant("RUBY_VERSION")`. Set `ZENO_RUBY` to another
   Ruby and run it again.

3. `ruby.constant("Array").new(3, 0)` gives an Array that stays in
   Ruby. Print it, then print `ruby.copy` of it. Change an item of
   the copy. Which of the two changed?

4. Make a Hash with the key `ruby.symbol("name")`, and read it with
   `"name"`. Then read it from `ruby.copy` of the Hash. Why does one
   find it and not the other?

5. `ruby.require("securerandom")`, then print
   `ruby.constant("SecureRandom").uuid()` three times.

6. Write a small YAML file and run `examples/settings.zeno` on it.
   Put a mistake in the file and read the message.
