# Libraries of PHP

PHP comes with a function for nearly everything a web page needs,
text, dates, patterns, JSON, HTML, images, and Composer has a package
for most of the rest. A Zeno program can use all of it as it is.

```zeno
import php

let DateTime = php.type("DateTime")
let day = DateTime("2026-11-18")
print(day.format("l"), php.callable("ucwords")("hello from zeno"), php.constant("PHP_INT_MAX"))
```

```
Wednesday Hello From Zeno 9223372036854775807
```

`php.type("DateTime")` is the class `DateTime`, and calling it is what
`new DateTime(...)` is in PHP. `php.callable("ucwords")` is the
function `ucwords`, and `php.constant` a constant. From there the
program is written as it would be in PHP, with a dot where PHP writes
`->` or `::`.

This is the second language of the second version, after Ruby
([`ruby.md`](ruby.md)), and it works as the others do
([`python.md`](python.md) has the whole story). Nothing changes for a
program that does not say `import php`. The decisions behind it are
chapter 35 of [`decisions.md`](decisions.md).

| Written | Gives |
|---|---|
| `php.type(name)` | the class of that name, with its namespace: `"DateTime"`, `"Carbon\\Carbon"` |
| `php.callable(name)` | the function of that name: `"strrev"`, `"Shop\\total"`, `"DateTime::createFromFormat"` |
| `php.constant(name)` | the constant of that name: `"PHP_VERSION"`, `"M_PI"`, `"DateTime::ATOM"` |
| `php.require(path)` | nothing; reads a file of PHP, once, from the folder of the program |
| `php.variable(value)` | a variable of PHP that holds the value, for a function that changes what it is given |
| `php.copy(value)` | what PHP can go through, or an object that only holds values, as a list or a dict of Zeno |

## What it needs

PHP 8 or later on the computer. Zeno looks for `php`, and is tested
with PHP 8.3. To use another one, name it:

```sh
ZENO_PHP=/opt/php-8.4/bin/php zeno run program.zeno
```

PHP is started the first time one of the functions of `php` is
called, once, and stays until the program ends. A program that
imports `php` and never calls it starts nothing.

## Functions, classes and packages

What comes with PHP needs nothing more: every function and class of
the extensions this PHP has (`php -m` lists them).

A package from Composer is looked for where PHP would find it for a
program of PHP in the same place: `vendor/autoload.php` in the folder
the `.zeno` file is in, then in each folder above. So a package is
installed in the folder of the program, and the program can be run
from anywhere:

```sh
composer require nesbot/carbon
zeno run program.zeno
```

```zeno
import php

let Carbon = php.type("Carbon\\Carbon")
let day = Carbon.parse("2026-11-18 10:00", "UTC")
print(day.format("l jS F Y"), day.addMonth().toDateString(), day.year)
```

```
Wednesday 18th November 2026 2026-12-18 2026
```

A class is named with its namespace, as `use` would name it, and `\`
is written `\\` in a str of Zeno. A package that is not there says
where it was looked for:

```
FileError: PHP has no class `Carbon\Carbon`. Is its package installed in this folder (`composer require`)? This folder is /home/ann/project
```

**A file of PHP beside the program** is read with `php.require`:
`php.require("helper.php")` reads `helper.php` from the folder of the
`.zeno` file, once, and what it defines is then there for `php.type`
and `php.callable`.

## A function is a value

PHP keeps functions apart from values, and calls one by its name.
`php.callable` gives the function as a value of Zeno, which is called
with brackets as any function is:

```zeno
import php

let explode = php.callable("explode")
let parts = explode(",", "pear,apple,fig")
parts.append("plum")
print(parts, len(parts), php.callable("implode")(" and ", parts))
print(php.callable("array_flip")(parts), php.callable("array_sum")([1, 2, 3.5]))
print(php.callable("htmlspecialchars")("<b>Tom & Ann</b>"), php.callable("number_format")(1234567.891, 2, thousands_separator=" "))
```

```
["pear", "apple", "fig", "plum"] 4 pear and apple and fig and plum
["pear": 0, "apple": 1, "fig": 2, "plum": 3] 6.5
&lt;b&gt;Tom &amp; Ann&lt;/b&gt; 1 234 567.89
```

`echo`, `isset`, `empty`, `list` and the other words that look like
functions in PHP are part of its language, and `php.callable` says
so.

**Named arguments** are PHP's own named arguments, by the name of the
parameter: `thousands_separator=" "` above.

## Arrays come across whole

An array of PHP is a value, as a list or a dict of Zeno is, and not
an object that lives somewhere. So an array that PHP gives comes
across whole: a list where its keys are 0, 1, 2 and so on, a dict for
any other. `parts` above is a list of Zeno, and `append` is Zeno's.
A list or a dict that Zeno gives arrives as an array.

## A function that changes what it is given

Some functions of PHP change their argument instead of giving a
value: `sort(&$array)`, `preg_match($pattern, $text, &$matches)`.
Given a list of Zeno, such a function would change a copy that nobody
sees. So it is given a variable of PHP, `php.variable`, and `.value`
is what it holds after the call:

```zeno
import php

let found = php.variable()
if php.callable("preg_match")("/(\\d+)-(\\d+)/", "tel 12-34", found) == 1 {
    print(found.value, found.value[1] + found.value[2])
}
let numbers = php.variable([3, 1, 2])
php.callable("sort")(numbers)
print(numbers.value)
```

```
["12-34", "12", "34"] 1234
[1, 2, 3]
```

Given anything else, the function is not called:

```
Error: `sort` changes its `array` (PHP writes `&$array`), so it needs a variable of PHP to change: `php.variable(value)`
```

## Objects stay in PHP

**`none`, a bool, a number, a str and an array go as themselves.**
PHP's `null` is `none`, an int is an int, a float is a float, a
string is a str.

**An object stays in PHP.** Zeno holds a number that PHP knows the
object by, and each time the program writes a property, a call or an
item of it, Zeno asks PHP. When nothing in the program holds it any
more, PHP is told and lets it go.

```zeno
import php

let DateTime = php.type("DateTime")
let utc = php.type("DateTimeZone")("UTC")
let day = DateTime("2026-11-18", utc)
day.modify("+1 month")
print(day.format("Y-m-d"), DateTime.ATOM, DateTime.createFromFormat("d/m/Y", "02/01/2026", utc).format("l"))
print(day == DateTime("2026-12-18", utc), day)
let thing = php.callable("json_decode")("{\"name\": \"pen\", \"tags\": [\"a\", \"b\"]}")
thing.name = thing.name + "cil"
print(thing.name, thing.tags, thing, php.copy(thing))
```

```
2026-12-18 Y-m-d\TH:i:sP Friday
true DateTime {"date":"2026-12-18 00:00:00.000000","timezone_type":3,"timezone":"UTC"}
pencil ["a", "b"] stdClass {"name":"pencil","tags":["a","b"]} ["name": "pencil", "tags": ["a", "b"]]
```

| Written | What PHP does |
|---|---|
| `object.name` | the method `name`, to be called now or later, if there is one; else the property `name` |
| `Class.NAME`, `Class.name` | the constant `Class::NAME`, the static property, or the static method |
| `object.name = value` | sets the property |
| `thing(1, 2)` | on a class, `new`; on a function or a method, the call |
| `thing[index]`, `thing[index] = value` | an `ArrayAccess` object's item; on another object, the property of that name |
| `len(thing)` | `count` of a `Countable`; of bytes, how many |
| `for item in thing { }` | everything `foreach` gives, taken when the loop begins |
| `item in thing` | `offsetExists` of an `ArrayAccess`, or whether `foreach` gives it |
| `thing == other` | `==` of two objects: the same class and the same properties |
| `print(thing)`, `str(thing)`, `f"{thing}"` | `__toString` if the class has one; else its class and its properties |

**A name that is a word of Zeno**, `Enum::from`, a property called
`for`, is the name in brackets: `Size["from"]("l")`, `thing["for"]`.
On an object the brackets reach the property first, where the dot
reaches the method first, so a name that is both can be reached
either way.

What it cannot be: the test of an `if`, or one side of `+` or `<`.

**Bytes that are not text** stay in PHP: a string of PHP is bytes,
and where they read as UTF-8 they are a str. `bytes[0]` is the first
byte as an int, `len(bytes)` how many, and `for` goes through them.
Given back to a function of PHP they are the same bytes.

## Collections of PHP

`ArrayObject`, `SplStack` and the classes of packages that hold items
are objects, and stay in PHP:

```zeno
import php

let ages = php.type("ArrayObject")(["ann": 31, "bob": 40])
ages["cy"] = 25
print(len(ages), ages["ann"], "bob" in ages, "zed" in ages)
for age in ages {
    print(age)
}
let copy = php.copy(ages)
print(copy, copy["cy"] + 1)
```

```
3 31 true false
31
40
25
["ann": 31, "bob": 40, "cy": 25] 26
```

`php.copy` makes a list or a dict of what `foreach` goes through, and
of an object that only holds values (`stdClass`, what `json_decode`
makes), with everything inside copied the same way. A value that is
one of Zeno's already is given back as it is.

## Errors

An exception of PHP is an error of Zeno, and `try` catches it. So is
a warning: in PHP `file_get_contents` of a file that is not there
prints a warning and gives `false`, and a program of Zeno would go on
with the `false`.

```zeno
import php

try {
    php.callable("intdiv")(1, 0)
} except ZeroDivisionError as error {
    print(error.message)
}
try {
    php.callable("file_get_contents")("/no/such/file")
} except FileError as error {
    print(error.message)
}
try {
    php.type("DateTime")("the day after never")
} except Error as error {
    print(error.message)
}
```

```
DivisionByZeroError: Division by zero
file_get_contents(/no/such/file): Failed to open stream: No such file or directory
DateMalformedStringException: Failed to parse time string (the day after never) at position 0 (t): The timezone could not be found in the database
```

| In PHP | In Zeno |
|---|---|
| `DivisionByZeroError` | `ZeroDivisionError` |
| `ValueError`, `InvalidArgumentException`, `DomainException`, `JsonException` | `ValueError` |
| `OutOfBoundsException`, `OutOfRangeException` | `IndexError` |
| a warning or an exception that says a file could not be used | `FileError` |
| any other exception, and any other warning | `Error` |

The message begins with what PHP called the exception, unless that is
the name of the kind already, or the plain `Exception` or `Error`. A
notice and a deprecation only advise, and PHP prints them as it
always does.

Two things can go wrong before any library is reached, and both are a
`FileError`:

```
FileError: there is no PHP to run. Zeno looked for `php`. `ZENO_PHP` can name one that is somewhere else
FileError: PHP has no class `Carbon\Carbon`. Is its package installed in this folder (`composer require`)? This folder is /home/ann/project
```

A function that declares what it raises and calls into PHP says
`raises Error`, which is every kind.

## Types

As for the other languages: a value from PHP has the type that
`json.parse` gives. The checker lets everything through, a wrong name
is found when the program gets there, and `zeno check --strict`
refuses a program that uses `php`.

## With the other languages in one program

A program can import several. Each value stays where it was made: a
str, a number, a list or a dict that one gave can be passed to
another, because by then it is a value of Zeno. An object that stayed
in one cannot be given to another:

```
error: a PHP value cannot be given to Python. Each stays where it was made
```

## What is not there yet

- **A function of Zeno cannot be given to PHP**: `array_map`,
  `usort`, `preg_replace_callback`. Zeno says so at the line. The
  loop is in Zeno instead: `for item in list { }`.
- **A class of Zeno cannot extend a class of PHP**, or be one of its
  interfaces.
- **A loop takes everything first**, as for Python.
- **Built programs.** `zeno run` and `zeno test` have `php`, with and
  without `--vm`. `zeno build`, in every form, refuses it.

## The example

[`examples/find.zeno`](../examples/find.zeno) prints every line of a
file that a pattern is found in, with what was found. Zeno has no
patterns for text, and PHP has `preg_match_all`.

```
$ zeno run examples/find.zeno "[0-9]+ ms" log.txt
1: boot took 120 ms
    120 ms
3: error: no answer from the sensor after 30 ms
    30 ms
4: wrote 3 files in 8 ms
    8 ms
The pattern is in 3 of 4 lines.
```

```zeno
let preg_match_all = php.callable("preg_match_all")
let found = php.variable()
...
if preg_match_all("{" + pattern + "}u", line, found) > 0 {
    for part in found.value[0] {
        print(f"    {part}")
    }
}
```

## How it is built

It is the bridge that the other languages use
([`python.md`](python.md), "How it is built"), with a helper in PHP.
`src/bridge.php` is part of the `zeno` file. The first call starts
PHP with it (`php -r`), and the two talk over a socket of this
machine, one line of JSON each way, so PHP keeps the screen and the
keyboard: `echo` and `fgets(STDIN)` work as if PHP had been started
by hand.

The lines are the same as Python's, with four more:

```
{"op":"import","name":"DateTime"}                      php.type("DateTime")
{"ok": ["p", 1]}

{"op":"function","name":"strrev"}                      php.callable("strrev")
{"ok": ["p", 2]}

{"op":"constant","name":"M_PI"}                        php.constant("M_PI")
{"ok": 3.141592653589793}

{"op":"variable","value":["l",[3,1,2]]}                php.variable([3, 1, 2])
{"ok": ["p", 3]}
```

`php.require` is a fifth, `require`, which gives `null`.

**A class, a function and a method not called yet are values the
helper makes up**, since PHP has a value for none of them. The
helper's own functions and classes begin with `zeno_` and `Zeno`, so
that a package cannot have one of the same name.

In `src/bridge.rs` PHP is one more `Lang`. The tests are in
`interpreter.rs` (`a_class_of_php_is_used_from_zeno`,
`what_is_not_a_value_stays_in_php`,
`an_error_of_php_is_an_error_of_zeno`), in `checker.rs` and in
`lower.rs`. Each test that runs a program runs it in both engines, and
says nothing on a machine with no PHP 8.

## Things to try

1. Run the first program of this page. Then misspell `DateTime` and
   read the message. Which folder does it name?

2. Print `php.constant("PHP_VERSION")` and
   `php.constant("PHP_BINARY")`. Set `ZENO_PHP` to another PHP and
   run it again.

3. Call `php.callable("sort")` on a list of Zeno and read what it
   says. Then do it with `php.variable`, and print the list you
   started with. Did it change?

4. `php.callable("str_split")("zeno")` gives a list. Is it a list of
   Zeno, or does it stay in PHP? How can you tell?

5. With `composer require cocur/slugify`, turn the title of your
   favourite book into the last part of a web address.

6. Run `examples/find.zeno` on a file of your own, with a pattern that
   finds every word that begins with a capital letter.
