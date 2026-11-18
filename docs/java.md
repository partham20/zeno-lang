# Libraries of Java

Java comes with a very large library, and nearly everything else
written for it is a jar file that can be downloaded. A Zeno program
can use both as they are.

```zeno
import java

let ArrayList = java.type("java.util.ArrayList")
let names = ArrayList()
names.add("cy")
names.add("ann")
java.type("java.util.Collections").sort(names)
print(names, names.size())
```

```
[ann, cy] 2
```

`java.type("java.util.ArrayList")` is what `import
java.util.ArrayList` is in Java: the class. Calling it makes an
object, as `new ArrayList()` does there, and from then on the program
is written as it would be in Java: a dot for what the object has,
brackets for a call.

This is the third language that works this way, after Python
([`python.md`](python.md)) and JavaScript ([`node.md`](node.md)), and
this page follows those two section by section. Nothing changes for a
program that does not say `import java`. The decisions behind it are
chapter 29 of [`decisions.md`](decisions.md).

| Written | Gives |
|---|---|
| `java.type(name)` | the class of that whole name |
| `java.jar(path)` | nothing; from then on Java finds the classes of that jar, or of every jar in that folder |
| `java.copy(value)` | a list, a set, an array or a map of Java as a list or a dict of Zeno |

Kotlin, Scala and Clojure libraries are jars as well, and are used
the same way.

## What it needs

Java 11 or later on the computer. Zeno looks for `java`, and then in
the folder `JAVA_HOME` names. To use another one, name it:

```sh
ZENO_JAVA=/opt/jdk-21/bin/java zeno run program.zeno
```

Java is started the first time `java.type` or `java.jar` is called,
once, and stays until the program ends. A program that imports `java`
and never calls it starts nothing.

**The first run takes a second longer.** Java runs classes, which are
made from source by `javac`. Zeno's own part on the Java side is made
into classes the first time, and kept in the folder this machine has
for what can be made again (`~/.cache/zeno` on Linux,
`~/Library/Caches/zeno` on macOS, `%LOCALAPPDATA%\zeno` on Windows;
`ZENO_CACHE` names another). Every run after that starts them as they
are, in about a fifth of a second. Where there is a `java` and no
`javac`, Java makes them again at each start.

## Classes

A class is asked for by its whole name. There is no short form: it is
the name Java's own documentation gives.

```zeno
let Math = java.type("java.lang.Math")
print(Math.PI, Math.max(3, 9), Math.floorMod(-7, 3))

let LocalDate = java.type("java.time.LocalDate")
let day = LocalDate.of(2026, 11, 18)
print(day, day.getDayOfWeek(), day.plusMonths(1), day.getYear() + 1)

let text = java.type("java.lang.StringBuilder")("zeno")
text.append(" and java").reverse()
print(text, len(text))
```

```
3.141592653589793 9 2
2026-11-18 WEDNESDAY 2026-12-18 2027
avaj dna onez 13
```

- **A dot after a class** is what is `static` in it: `Math.PI`,
  `Math.max(3, 9)`, `LocalDate.of(...)`.
- **Calling a class** makes an object: `StringBuilder("zeno")` is `new
  StringBuilder("zeno")`.
- **A class inside a class** is named as Java writes it,
  `java.type("java.util.Map.Entry")`, or reached with a dot from the
  outer one: `java.type("java.lang.Thread").State.NEW`.

## Libraries that are jars

A library that does not come with Java is a `.jar` file. `java.jar`
says where it is, and after that its classes are found:

```zeno
import java

java.jar("lib/gson-2.11.0.jar")
let gson = java.type("com.google.gson.GsonBuilder")().setPrettyPrinting().create()
print(gson.toJson(["name": "ann", "tags": "a b"]))
```

```
{
  "name": "ann",
  "tags": "a b"
}
```

The path begins at the folder the `.zeno` file is in, so the program
can be run from anywhere. A folder can be given in place of a jar,
`java.jar("lib")`: that is every jar in it, and any classes that lie
in it as `.class` files.

**Getting a jar.** Java has no `pip install`. A jar is downloaded from
where the library is published, most often Maven Central
([search.maven.org](https://search.maven.org)), or fetched by Maven
if that is installed:

```sh
mvn dependency:copy -Dartifact=com.google.code.gson:gson:2.11.0 -DoutputDirectory=lib
```

A library that needs other libraries needs their jars too. Put them
all in one folder and give the folder.

Two more places are looked in without being named, as Java itself
does: what the `CLASSPATH` variable names, and with no `CLASSPATH`,
the folder of the program, for classes that were compiled there.

## Values go across in two ways

**`none`, a bool, a number and a str go as themselves.** Java's
`null` is `none`. Its `byte`, `short`, `int` and `long` are an int,
its `float` and `double` a float, and its `char` and `String` a str.

```zeno
let Integer = java.type("java.lang.Integer")
print(Integer.parseInt("42") + 1, Integer.MAX_VALUE, java.type("java.lang.Character").toUpperCase('a'))
```

```
43 2147483647 A
```

**Everything else stays in Java.** `names`, `day` and `text` above
are not copies of anything. Zeno holds a number that Java knows the
value by, and each time the program writes a field, a call or an item
of it, Zeno asks Java. When nothing in the program holds the value any
more, Java is told and lets it go.

A value that stays in Java can be:

| Written | What Java does |
|---|---|
| `thing.name` | the field `name` if it is `public`, or the method, to be called now or later |
| `thing.name = value` | sets the field |
| `thing(1, 2)` | on a class, `new`; on a method, the call |
| `thing[index]`, `thing[index] = value` | an array's item; `get` and `set` of a `List`; `get` and `put` of a `Map` |
| `len(thing)` | an array's `length`; `size()` of a collection or a map; `length()` of text |
| `for item in thing { }` | every item, taken when the loop begins; of a map, its keys |
| `item in thing` | `contains` of a collection, `containsKey` of a map |
| `thing == other` | `thing.equals(other)` |
| `print(thing)`, `str(thing)`, `f"{thing}"` | `thing.toString()`; of an array, its items |

Java has no named arguments, so a call with one is an error that says
so.

**A name that is a word of Zeno.** `System.in` does not parse, because
`in` is a word of the language, and Java has `from`, `and`, `or` and
`not` as names of methods too. On a class, and on an object that has
no items of its own, the name in brackets is the name after a dot:
`System["in"]`, `LocalDate["from"](other)`, `big["not"]()`.

What it cannot be: the test of an `if` (write `len(thing) > 0` or
`thing is not none`), or one side of `+` or `<`.

## What Zeno gives, Java's method decides

Java is told the type of everything, and Zeno's values are made to
fit what the method asks for. An int of Zeno can be an `int`, a
`long`, a `short` or a `double` there; a str can be a `String` or,
with one character, a `char`; a list can be a `List` or an array; a
dict is a `Map`.

Java can have several methods of one name that differ in what they
take. The one whose types the arguments fit best is called, by the
order Java itself would choose in: an int is an `int` before it is a
`long`, and a `long` before a `double`.

```zeno
let String = java.type("java.lang.String")
print(String.valueOf(5), String.valueOf(2.5), String.valueOf(true))
```

```
5 2.5 true
```

Those are three different methods of Java. When none fits, or two fit
equally, the error lists what there is:

```
Error: Java has no `add` that takes an int, an int, an int. There is add(Object), add(int, Object)
```

Where a method takes anything at all (`Object`, which is what a
collection of Java holds), an int goes as an `Integer` if it fits one
and as a `Long` if it does not, which is what a number written in a
Java program is.

**One trap is Java's own.** A `List` has `remove(int)`, which takes
the item at a place, and `remove(Object)`, which takes an item out.
`list.remove(30)` is the first, in Java as here. To take the number
30 out of a list of numbers, write `list.remove(list.indexOf(30))`.

## Lists and maps

A list of Java stays in Java, and that is on purpose.

```zeno
let names = java.type("java.util.ArrayList")(["cy", "ann"])
names.add("bob")
java.type("java.util.Collections").sort(names)
print(names, len(names), names[0], "bob" in names)
for name in names {
    print(name.upper())
}
```

```
[ann, bob, cy] 3 ann true
ANN
BOB
CY
```

`names` is the one list there is. `add` and `sort` are Java's, and
they change the list that Java has. Each `name` in the loop is a str
of Zeno, so `upper` is Zeno's.

When a list of Zeno is what the program wants, `java.copy` makes one:

```zeno
let mine = java.copy(names)
mine.append("dee")
print(mine, len(names))
```

```
["ann", "bob", "cy", "dee"] 3
```

`copy` takes a list, a set or an array and gives a list; it takes a
map and gives a dict. Every collection inside is copied as well.
Whatever is not one of these stays in Java inside the copy. A map
whose keys are not all text or whole numbers cannot be a dict of Zeno,
and `copy` raises a `ValueError` that names the key.

**The other way is always a copy.** A list of Zeno that is passed to
Java arrives as a new `ArrayList`, and a dict as a new
`LinkedHashMap`:

```zeno
let xs = [3, 1, 2]
java.type("java.util.Collections").sort(xs)
print(xs)                               # [3, 1, 2]: Java sorted its copy
```

That is also the short way to make a collection of Java from what
Zeno has: `ArrayList(["cy", "ann"])` above, and
`java.type("java.util.TreeMap")(["cy": 7, "ann": 31])`.

`==` is Java's `equals`, so two lists with the same items are equal,
and a list of Java equals a list of Zeno with the same items:

```zeno
let a = java.type("java.util.List").of(1, 2)
let b = java.type("java.util.ArrayList")([1, 2])
print(a == b, a == [1, 2])              # true true
```

## Errors

An exception that Java throws is an error of Zeno, and `try` catches
it.

```zeno
try {
    java.type("java.lang.Integer").parseInt("forty")
} except ValueError as error {
    print(error.message)
}
try {
    java.type("java.util.ArrayList")().get(3)
} except IndexError as error {
    print(error.message)
}
```

```
NumberFormatException: For input string: "forty"
IndexOutOfBoundsException: Index 3 out of bounds for length 0
```

| In Java | In Zeno |
|---|---|
| `IndexOutOfBoundsException` and its kinds | `IndexError` |
| `IllegalArgumentException` and its kinds (`NumberFormatException`, ...) | `ValueError` |
| `IOException` and its kinds (`NoSuchFileException`, `ZipException`, ...) | `FileError` |
| anything else | `Error` |

The message begins with what Java called the exception.

Two things can go wrong before any library is reached, and both are a
`FileError`:

```
FileError: there is no Java to run. Zeno looked for `java`. `ZENO_JAVA` can name one that is somewhere else
FileError: Java has no class `org.yaml.snakeyaml.Yaml`. A class is asked for by its whole name, as `java.util.ArrayList`. One that does not come with Java is in a jar, which `java.jar` adds. This Java is /usr/lib/jvm/java-21-openjdk-amd64
```

A function that declares what it raises and calls into Java says
`raises Error`, which is every kind.

## Types

As for Python and JavaScript: a value from Java has the type that
`json.parse` gives. The checker lets everything through, a wrong name
is found when the program gets there, and `zeno check --strict`
refuses a program that uses `java`.

Java knows its own types exactly, and they are checked when each call
is made. Zeno's checker does not read them yet.

## With Python and JavaScript in one program

A program can import all three. Each value stays where it was made: a
str or a number that one gave can be passed to another, because by
then it is a value of Zeno. A value that stayed in one cannot be given
to another:

```
error: a Java value cannot be given to Python. Each stays where it was made
```

`java.copy` makes it a list or dict of Zeno first, and that can go
anywhere.

## What is not there yet

- **A function of Zeno cannot be given to Java.** What Java writes as
  a lambda, `list.forEach(...)`, a `Comparator`, a `Runnable`, cannot
  call into Zeno yet, and Zeno says so at the line.
- **A class of Zeno cannot extend a class of Java**, or be one of its
  interfaces.
- **A loop takes everything first**, as for Python.
- **Built programs.** `zeno run` and `zeno test` have `java`, with and
  without `--vm`. `zeno build`, in every form, refuses it.

## The example

[`examples/zipped.zeno`](../examples/zipped.zeno) says what is in a
zip file. Zeno cannot read one by itself, and Java can, with
`java.util.zip`.

```
$ zeno run examples/zipped.zeno photos.zip
    2.1 MB  beach.jpg
    1.8 MB  hill.jpg
     412 B  notes.txt
3 files, 3.9 MB in all, 3.8 MB in the zip (2% smaller).
```

```zeno
let ZipFile = java.type("java.util.zip.ZipFile")
let zip = ZipFile(name)
for entry in zip.entries() {
    if not entry.isDirectory() {
        print(f"{as_size(entry.getSize()):>10}  {entry.getName()}")
    }
}
```

The zip and each entry stay in Java. A name arrives as a str and a
size as an int.

## How it is built

It is the bridge that Python and JavaScript use
([`python.md`](python.md), "How it is built"), with a helper in Java.
`src/bridge.java` is part of the `zeno` file. The first call compiles
it if that was not done before, starts Java with it, and the two talk
over a socket of this machine, one line of JSON each way, so Java
keeps the screen and the keyboard: `System.out.println` and
`System.in` work as if Java had been started by hand.

The lines are the same as Python's, with one more:

```
{"op":"import","name":"java.util.ArrayList"}           java.type("java.util.ArrayList")
{"ok": ["p", 1]}

{"op":"jar","name":"lib/gson-2.11.0.jar"}              java.jar("lib/gson-2.11.0.jar")
{"ok": null}

{"op":"call","on":["p",1],"args":[],"named":[]}        ArrayList()
{"ok": ["p", 2]}
```

Three things are Java's own.

**A class and a method are values the helper makes up.** Java has no
value for "the class as its name is written", which is what has
`static` things after a dot, and none for "this method of this
object, not called yet". The helper has a small class for each
(`Type` and `Bound`). `names.add` gives a `Bound`, and calling it is
when Java's method is chosen.

**The method is chosen when it is called** (`fit` and `called` in the
helper). Every public method of that name is given a number for how
well the arguments fit it, and the lowest is called. Two with the
same number are an error and not a guess.

**A method is called through a class that may be used.** What
`List.of(1, 2)` gives is an object of a class inside Java that no
program may touch, and Java refuses a call made through it. The
helper finds the same method in a class or interface that is public,
here `List`, and calls it through that.

Java comes with nothing that reads JSON, so the helper reads and
writes its lines itself.

In `src/bridge.rs` Java is one more `Lang`. What it adds there is
`java_helper`, which keeps the compiled classes (above). The tests
are in `interpreter.rs` (`a_class_of_java_is_used_from_zeno`,
`what_is_not_a_number_or_text_stays_in_java`,
`an_error_of_java_is_an_error_of_zeno`), in `checker.rs` and in
`lower.rs`. Each test that runs a program runs it in both engines, and
says nothing on a machine with no Java.

## Things to try

1. Run the first program of this page twice, and time both runs. Why
   is the second faster?

2. Print `java.type("java.lang.System").getProperty("java.version")`.
   Set `ZENO_JAVA` to another Java and run it again.

3. `java.type("java.util.List").of(3, 1, 2)` gives a list that stays
   in Java. Print it, then print `java.copy` of it. Then call
   `add(4)` on each, and read what happens.

4. `Math.max(3, 9)` and `Math.max(3, 2.5)` call two different methods
   of Java. What does each print? Now call `Math.max("a", "b")` and
   read the list in the message.

5. Download a jar from Maven Central, put it in a folder `lib` beside
   a program, and use one class of it.

6. Run `examples/zipped.zeno` on a zip of your own, and on a `.jar`.
