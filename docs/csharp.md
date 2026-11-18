# Libraries of C# and .NET

.NET is what C# runs on, and F# and Visual Basic too. It comes with
a library for nearly everything a program does: files, text, dates,
numbers, checksums, compression, the network. NuGet has the packages.
A Zeno program can use .NET's own types and any package, as they are.

```zeno
import csharp

let Builder = csharp.type("System.Text.StringBuilder")
let text = Builder("Zeno")
text.Append(" and .NET ").Append(8)
print(text, text.Length, csharp.type("System.Math").Round(3.14159, 2))
```

```
Zeno and .NET 8 15 3.14
```

`csharp.type("System.Text.StringBuilder")` is the class, by its name
with its namespace. Calling it makes an object, as `new` does in C#,
and from there the program is written as it would be in C#.

This is the fourth language of the second version, after Ruby, PHP
and Lua ([`ruby.md`](ruby.md), [`php.md`](php.md), [`lua.md`](lua.md)),
and it works as the others do ([`python.md`](python.md) has the whole
story, and [`java.md`](java.md) the one closest to this). Nothing
changes for a program that does not say `import csharp`. The decisions
behind it are chapter 37 of [`decisions.md`](decisions.md).

| Written | Gives |
|---|---|
| `csharp.type(name)` | the type of that name: `"System.IO.File"`, `"List<int>"`, `"string"` |
| `csharp.assembly(path)` | nothing; the types of that .dll, or of every .dll in that folder, are found after it |
| `csharp.copy(value)` | a collection of .NET as a list or a dict of Zeno |

## What it needs

The .NET SDK, 6 or later, which has `dotnet`: the runtime alone runs
programs but cannot build them, and the first run builds a small
program of .NET that Zeno talks to ("How it is built", below). Zeno
looks for `dotnet`, then in `DOTNET_ROOT`, then in `~/.dotnet`, where
Microsoft's script puts it. To use another one, name it:

```sh
ZENO_DOTNET=/opt/dotnet/dotnet zeno run program.zeno
```

.NET is started the first time `csharp.type` or `csharp.assembly` is
called, once, and stays until the program ends. A program that
imports `csharp` and never calls it starts nothing. The very first
run on a computer takes some seconds more, while `dotnet build` makes
the helper. Every later one starts in a tenth of a second.

## Names of types

A type is named as C# names it:

- With its namespace: `"System.Text.StringBuilder"`, `"System.IO.File"`.
- Without one, where a new program of C# needs no `using`: the
  namespaces `System`, `System.Collections.Generic`, `System.IO`,
  `System.Linq`, `System.Net.Http`, `System.Threading` and
  `System.Threading.Tasks`. So `"File"`, `"Math"` and `"List<int>"`
  are enough.
- With C#'s words: `"int"`, `"string"`, `"double"`, `"bool"`.
- Made of others: `"List<int>"`, `"Dictionary<string, List<int>>"`,
  `"int[]"`, `"int?"`.
- A type inside another, with a dot: `"Shapes.Point.Kind"`.

```zeno
import csharp

let list = csharp.type("List<int>")()
list.AddRange([30, 10, 20])
list.Sort()
print(list, list[0], len(list), 10 in list, csharp.copy(list))
let ages = csharp.type("Dictionary<string, int>")()
ages["ann"] = 31
ages["bob"] = 40
print(ages, ages["ann"], "bob" in ages, csharp.copy(ages))
```

```
[10, 20, 30] 10 3 true [10, 20, 30]
{"ann": 31, "bob": 40} 31 true ["ann": 31, "bob": 40]
```

`list` is a `List<int>` of .NET, and `csharp.copy(list)` is a list of
Zeno with what it holds. A list of Zeno given to `AddRange` is made
into what `AddRange` takes.

## Packages of NuGet

A package is added to a project of .NET, and the program uses the
project beside it. Two commands make one, in the folder of the
program:

```sh
dotnet new classlib -n libraries -o .
dotnet add package Humanizer.Core
```

```zeno
import csharp

let words = csharp.type("Humanizer.StringHumanizeExtensions")
let numbers = csharp.type("Humanizer.NumberToWordsExtension")
print(words.Humanize("PascalCaseInputString"), "/", numbers.ToWords(1234))
```

```
Pascal case input string / one thousand two hundred and thirty-four
```

`dotnet add package` writes down in `obj/project.assets.json` each
.dll of the package, and of every package it needs, and where NuGet
keeps them. Zeno reads that file from the folder of the program, or
the nearest folder above it that has one, so the program can be run
from anywhere. `dotnet new classlib` also writes a `Class1.cs`, which
can be removed.

## Code of your own, and other .dll files

`csharp.assembly(path)` loads a .dll, or every .dll in a folder, and a
path that does not begin at the top is from the folder of the program.
The project above builds what is written in C# beside it: put a file
`Shapes.cs` there, run `dotnet build`, and its types are in
`bin/Debug/net8.0`.

```csharp
namespace Shapes
{
    public class Point
    {
        public int X { get; set; }
        public int Y;
        public Point(int x, int y) { X = x; Y = y; }
        public double Length() => System.Math.Sqrt(X * X + Y * Y);
        public Point Moved(int dx = 1, int dy = 1) => new Point(X + dx, Y + dy);
        public override string ToString() => $"Point({X}, {Y})";
        public static string Greet(string name, int times = 1, bool loud = false)
        {
            var said = string.Join(", ", System.Linq.Enumerable.Repeat("hello " + name, times));
            return loud ? said.ToUpperInvariant() : said;
        }
    }
}
```

```zeno
import csharp

csharp.assembly("bin/Debug/net8.0")
let Point = csharp.type("Shapes.Point")
let p = Point(3, 4)
print(p, p.Length(), p.Moved(), p.Moved(dy=5), p.X)
print(Point.Greet("ann", times=2, loud=true))
```

```
Point(3, 4) 5.0 Point(4, 5) Point(4, 9) 3
HELLO ANN, HELLO ANN
```

## Of several methods, the arguments choose

C# can have several methods of one name, which take different
things, and so can a constructor. The one called is the one the
arguments fit, as C# chooses it. An int of Zeno fits an `int` before
a `long`, and both before a `double`; a list fits an array or a
`List<T>` when its items fit; a parameter that has a default may be
left out, and `params` takes the rest.

```zeno
import csharp

let math = csharp.type("System.Math")
print(math.Max(3, 7), math.Max(2, 3.5), math.Round(2.567, digits=1))
let text = csharp.type("string")
print(text.Join("-", ["a", "b"]), text.Join("+", "x", "y", "z"))
```

```
7 3.5 2.6
a-b x+y+z
```

**A named argument is given to the parameter of that name**, as in
C#: `math.Round(2.567, digits=1)`, `p.Moved(dy=5)`. The names are
those of the method, and an error that says no method fits lists them.

When two fit as well, Zeno does not guess: the error says which two.
A generic method, such as `Task.FromResult<T>(T value)`, is given its
type from the arguments, as C# works it out.

## Values go across in two ways

**`none`, a bool, a number and a str go as themselves.** `null` is
`none`. Every whole number of .NET that fits 64 bits (`int`, `long`,
`byte`, `short`) arrives as an int, a `float` and a `double` as a
float, and a `char` as a str. Given to a method, an int becomes what
the parameter is, if it fits there.

**Everything else stays in .NET**: an object, an array, a collection,
an enum, a `decimal`. Zeno holds a number that .NET knows it by, and
each time the program writes a field, a call or an item of it, Zeno
asks .NET. When nothing in the program holds it any more, .NET is told
and lets it go.

A value that stays in .NET can be:

| Written | What .NET does |
|---|---|
| `thing.Name` | the property or the field; of a method, the method, called with brackets |
| `thing.Name = value` | sets the property or the field |
| `Type(1, 2)` | `new Type(1, 2)` |
| `Type.Name` | what is `static`: `Math.PI`, `File.ReadAllText` |
| `thing[key]`, `thing[key] = value` | the item of an array, a list or a dictionary, counting from 0 |
| `len(thing)` | `Count` or `Length` |
| `for item in thing { }` | of a dictionary, its keys; of anything else that can be gone through, its items |
| `item in thing` | of a dictionary, whether it has the key; of anything else, whether it holds the item |
| `thing == other` | `Equals` |
| `print(thing)`, `str(thing)`, `f"{thing}"` | `ToString()`, and the items of a collection whose `ToString` only names its type |

What it cannot be: the test of an `if`, or one side of `+` or `<`.

**A `Task` is waited for.** A method that gives a `Task` is one that
takes time, such as reading from the network, and the call gives what
the task gives when it is done: `none` for a plain `Task`.

```zeno
import csharp

let task = csharp.type("Task")
print(task.FromResult(7), task.Delay(10))
let day = csharp.type("DateTime")(2026, 11, 18)
print(day.DayOfWeek, day.AddDays(10).ToString("yyyy-MM-dd"), day.DayOfWeek == csharp.type("DayOfWeek").Wednesday)
let made = csharp.type("Array").CreateInstance(csharp.type("int"), 3)
print(made, csharp.type("int[]"))
```

```
7 none
Wednesday 2026-11-28 true
[0, 0, 0] System.Int32[]
```

A type that Zeno asked for by name goes where .NET wants a `Type`:
`csharp.type("int")` above.

**The name in brackets** is the same as the name after a dot, on a
type and on an object that has no items of its own: a name that is a
word of Zeno is written `thing["match"]`.

## Errors

An exception of .NET is an error of Zeno, and `try` catches it.

```zeno
import csharp

try {
    csharp.type("File").ReadAllText("/no/such/file")
} except FileError as error {
    print(error.message)
}
try {
    csharp.type("Convert").ToInt32("twelve")
} except ValueError as error {
    print(error.message)
}
```

```
DirectoryNotFoundException: Could not find a part of the path '/no/such/file'.
FormatException: The input string 'twelve' was not in a correct format.
```

| Exception of .NET | Error of Zeno |
|---|---|
| `IOException` and those under it, `UnauthorizedAccessException` | `FileError` |
| `KeyNotFoundException` | `KeyError` |
| `IndexOutOfRangeException`, `ArgumentOutOfRangeException` | `IndexError` |
| `ArgumentException`, `FormatException` | `ValueError` |
| `DivideByZeroException` | `ZeroDivisionError` |
| `OverflowException` | `OverflowError` |
| every other | `Error` |

The message begins with the name of the exception. Two things can go
wrong before any library is reached, and both are a `FileError`:

```
FileError: there is no .NET to run. Zeno looked for `dotnet`. `ZENO_DOTNET` can name one that is somewhere else
FileError: .NET has no type `Humanizer.Inflector`. A type of a package is there once the project beside the program has the package (`dotnet add package`), and one of a .dll after `csharp.assembly`
```

A function that declares what it raises and calls into .NET says
`raises Error`, which is every kind.

## Types

As for the other languages: a value from .NET has the type that
`json.parse` gives. The checker lets everything through, a wrong name
is found when the program gets there, and `zeno check --strict`
refuses a program that uses `csharp`.

## With the other languages in one program

A program can import several. Each value stays where it was made: a
str or a number that one gave can be passed to another, because by
then it is a value of Zeno. An object of .NET cannot be given to
another:

```
error: a C# value cannot be given to Python. Each stays where it was made
```

`csharp.copy` makes a collection a list or dict of Zeno first, and
that can go anywhere.

## What is not there yet

- **A function of Zeno cannot be given to .NET**: an event, a
  `Func<T>`, the lambdas of LINQ (`Where`, `Select`). Zeno says so at
  the line. LINQ that takes no function works: `Enumerable.Sum(list)`.
- **An extension method is called on its class**:
  `StringHumanizeExtensions.Humanize(text)`, not `text.Humanize()`,
  since a str is Zeno's own.
- **`out` and `ref` parameters** are given what the program gives, and
  what the method puts back in them is not seen.
- **A generic method whose type no argument says**, such as
  `JsonSerializer.Deserialize<T>(text)`, is called with `object`.
- **Built programs.** `zeno run` and `zeno test` have `csharp`, with
  and without `--vm`. `zeno build`, in every form, refuses it.

## The example

[`examples/clock.zeno`](../examples/clock.zeno) prints what time it is
now in each place named, with how far ahead of UTC it is there: for a
meeting, a log stamped in another country, or a board that keeps UTC.
Zeno has the time as a number and no time zones; .NET has them all,
and knows when each place changes its clocks.

```
$ zeno run examples/clock.zeno Asia/Kolkata Asia/Taipei America/St_Johns
2026-11-18 14:33  UTC+05:30  Asia/Kolkata
2026-11-18 17:03  UTC+08:00  Asia/Taipei
2026-11-18 05:33  UTC-03:30  America/St_Johns
```

```zeno
let zone = zones.FindSystemTimeZoneById(place)
let there = zones.ConvertTimeFromUtc(now, zone)
let minutes = int(zone.GetUtcOffset(now).TotalMinutes)
```

`there` is a `DateTime` of .NET, and its `ToString` writes it as the
program asks: `there.ToString("yyyy-MM-dd HH:mm")`.

## How it is built

It is the bridge that the other languages use
([`python.md`](python.md), "How it is built"), with a helper in C#.
`src/bridge.cs` is part of the `zeno` file. .NET cannot run a program
from its source, as Python and Ruby do, so the first time it is
needed Zeno writes the helper and a project for it into the folder it
keeps for what can be made again, where `zeno build` keeps what it
compiles, and runs `dotnet build` there once. It is built for the
.NET that builds it, and runs on that one and on every later one.
Every later run starts what was built, in a tenth of a second. A
newer `zeno` with another helper builds it again, beside the old one.

The helper connects back over a socket of this machine, and the two
talk one line of JSON each way, so .NET keeps the screen and the
keyboard: `Console.WriteLine` prints where Zeno prints.

In `src/bridge.rs` .NET is one more `Lang`. The tests are in
`interpreter.rs` (`a_type_of_dotnet_is_used_from_zeno`,
`what_is_not_a_number_or_text_stays_in_dotnet`,
`an_error_of_dotnet_is_an_error_of_zeno`), in `checker.rs` and in
`lower.rs`. Each test that runs a program runs it in both engines,
and says nothing on a machine with no .NET SDK.

## Things to try

1. Run the first program of this page. Then print
   `csharp.type("System.Environment").Version`.

2. `csharp.type("Directory").GetFiles(".")` gives an array. Print it,
   its `len`, and `csharp.copy` of it.

3. Make a `HashSet<string>` from `["a", "b", "a"]`. How many items
   does it have? What does `.Add("a")` give, and `.Add("c")`?

4. Call `csharp.type("Math").Max("a", 2)` in a `try`, and read the
   message. Which `Max` would take an int and a float?

5. Add the package `Newtonsoft.Json` to the project beside your
   program, and print
   `csharp.type("Newtonsoft.Json.JsonConvert").SerializeObject(["a": [1, 2]])`.

6. Write `Shapes.cs` with a `Point` as above, build it, and give it a
   method `Area()`. Call it from Zeno.
