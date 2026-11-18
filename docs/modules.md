# Programs in several files

Until now a Zeno program was one file. This page adds the second file,
and the tenth: `import shapes` brings in `shapes.zeno`, and
`shapes.area(s)` calls the `area` that file declares.

```sh
cargo run -- run examples/station/main.zeno
```

```
uart> temperature     21.5 C
uart> humidity       104.0 %   suspect: over 100.0
uart> pressure      1013.2 hPa
uart> wind            12.0 km/h   suspect: no limit is known for it
uart> 2 of 4 readings are good
180 bytes sent at 9600 baud
```

[`examples/station`](../examples/station) is a small weather station
in four files:

```
examples/station/
    main.zeno            the file the program starts in
    sensors.zeno         the module `sensors`
    report.zeno          the module `report`
    drivers/
        uart.zeno        the module `drivers.uart`
```

The decisions are 7.1 to 7.12 in [`decisions.md`](decisions.md).

## What a module is

A module is a file. There is nothing to declare and nothing to
register: the file `sensors.zeno` is the module `sensors`.

```zeno
import sensors                       # the whole module
import drivers.uart                  # a dot is a folder: drivers/uart.zeno
from sensors import Reading, Good    # single names

let health = sensors.check(Reading("wind", 12.0, "km/h"))
drivers.uart.send("ready")
```

These are the two forms that `import math` and `from math import sqrt`
have always had. A file of your own is used exactly as a module that
comes with Zeno is.

| Written | Means |
|---|---|
| `import shapes` | read `shapes.zeno`. Its names are then written `shapes.area`, `shapes.Point` |
| `import drivers.uart` | read `drivers/uart.zeno`. Its names are written `drivers.uart.send` |
| `from shapes import area, Point` | read `shapes.zeno`, and let `area` and `Point` be written with nothing in front |
| `let p: shapes.Point = ...` | a type from a module |
| `case shapes.Circle(r) { }` | an enum case from a module, in a `match` |
| `except shapes.BadShape as bad { }` | an error class from a module |

### The rules, all of them

1. **Where a module is looked for.** From the folder of the file you
   gave to `zeno run`, whichever file does the importing. One name is
   one file, everywhere in the program (7.2). (An import whose first
   word is a package the program needs reads that package:
   [`packages.md`](packages.md).)
2. **What can be imported.** Everything a file declares at its top:
   functions, classes, enums and their cases, `let`, `var` and `const`.
   What the file itself imports is not passed on (7.4). A declaration
   with `private` in front is for its own file only
   ([`private.md`](private.md)).
3. **When an imported file runs.** Once, before the file that imports
   it. So an `import` of a file goes at the top of the file, above
   everything that is not an import (7.5).
4. **Files cannot import each other**, directly or in a longer circle
   (7.6).
5. **A file sees what it declares and what it imports.** An imported
   file cannot use a name of the file that imports it. And only the
   file of a module gives its variables a new value: others read them
   (7.7).
6. **No name has two readings.** Importing `x` from two modules, or
   declaring your own `x` beside an imported one, is refused. A name
   declared inside a function or a block hides an imported one, as it
   hides any name from the top of the file (7.8).
7. **A value prints by the name its file gave it.**
   `print(shapes.Point(1, 2))` prints `Point(x=1, y=2)` (7.9).

Each of these has a message that says what to write instead. A few:

```
error: there is no module `shapes`. `import shapes` reads the file `demo/shapes.zeno`,
and there is no such file. So far Zeno has `math`, `time`, `board`, `files`, `program`,
`json`, `web` and `python`

error: module `sensors` has no `chek`. It has `Reading`, `Health`, `Good`, `Suspect`,
`highest` and `check`

error: these files import each other: `a` imports `b`, and `b` imports `a`. An imported
file runs before the file that imports it, and here none of them can be the first.
Move what they share into a file of its own, and import that

error: `drivers.uart.sent` is a variable of the module `drivers.uart`, and only the
file of `drivers.uart` gives it a new value. Give `drivers.uart` a function that
changes it, and call that
```

A mistake in an imported file is shown in that file:

```
error: the returned value should be int, but this is float
 --> shapes.zeno:2:12
  |
2 |     return n / 2
  |            ^
```

## How it is built

### One new stage, and none of the old ones change

A compiler for several files usually means that every stage learns
about files: the checker keeps a table of names for each module, the
interpreter a set of variables for each, the C backend a name scheme.
Zeno has five stages after the parser (the checker, the interpreter,
the VM, the C backend and the LLVM backend), so that would be the
same work five times.

Instead there is one new stage, [`src/modules.rs`](../src/modules.rs),
between the parser and everything else:

```
main.zeno ----> lexer -> parser --+
sensors.zeno -> lexer -> parser --+--> modules --> ONE program --> checker
report.zeno --> lexer -> parser --+                               interpreter, VM
uart.zeno ----> lexer -> parser --+                               C, LLVM
```

What comes out is one ordinary program, as if it had been written in
one file. The stages after it do not know there were ever several.

You can see that program:

```sh
cargo run -- join examples/station/main.zeno
```

```
class sensors.Reading
    field name: str
    field value: float
    field unit: str
enum sensors.Health
    sensors.Good
    sensors.Suspect(why: str)
let sensors.highest = ["temperature": 60.0, "humidity": 100.0, "pressure": 1100.0]
function sensors.check(reading: sensors.Reading) -> sensors.Health
    ...
function report.line(reading: sensors.Reading, health: sensors.Health) -> str
    ...
let drivers.uart.speed = 9600
var drivers.uart.sent = 0
function drivers.uart.send(text: str)
    print(f"uart> {text}")
    drivers.uart.sent += (len(text) + 1)
...
let readings = [sensors.Reading("temperature", 21.5, "C"), ...]
```

Three things happened: the files were **loaded**, their names were
**renamed**, and their statements were **joined**.

### Load

`Loader::file` reads one file, and calls itself for each file that one
imports. That is a walk down a tree, and three small tables keep it
honest:

```rust
struct Loader<'a> {
    files: Vec<File>,                 // every file read, the first file first
    numbers: HashMap<String, usize>,  // which file each module is: read once
    order: Vec<usize>,                // the files in the order they will run
    loading: Vec<String>,             // the modules being read right now
    ...
}
```

- `numbers` is looked at before a file is read. If `report.zeno` and
  `main.zeno` both import `sensors`, the second import finds it there
  and reads nothing.
- `loading` is the path from the first file down to the file being
  read. If an import names a module that is already on that path, the
  files form a circle, and the path itself is the error message.
- `order` gets a file's number *after* its imports have been
  followed. So everything a file imports is in the list before the
  file is. That is the order the statements are joined in, and
  therefore the order they run in.

### Rename

Two files may each have a function called `check`. In one program
they need two names. The renamer gives every name that an imported
file declares at its top the module's name in front:

```
check        in sensors.zeno             becomes   sensors.check
sensors.check   in main.zeno             becomes   sensors.check
check        after `from sensors import check`   becomes   sensors.check
```

All three end as the one word `sensors.check`. To the stages after
this one that is a single name with a dot in it, no different from
`total`. It can never collide with a name somebody wrote, because
nobody can write `sensors.check` as one word: the parser reads that
as a field of a variable called `sensors`. That reading is exactly
what the renamer looks for:

```rust
// `shapes.area`: the parser read a field of a variable `shapes`.
// It is the name `area` of the module `shapes`.
if let ExprKind::Field { object, name } = &expr.kind {
    if let Some(module) = self.module_named(object) {
        if self.starts.contains(&format!("{module}.{name}")) {
            // Still the name of a module: `drivers.uart` of `drivers.uart.send`.
        } else if let Some(global) = self.whole.get(&module) {
            let names = &self.declared[global];
            match names.contains(name) {
                true => expr.kind = ExprKind::Name(format!("{global}.{name}")),
                false => self.error(no_such_name(&module, name, names), span),
            }
            return;
        }
        ...
    }
}
```

`whole` holds the modules the file imported with `import`, each with
the name it has in the whole program. In a program of one folder the
two are the same word. They differ inside a package, which is the
subject of [`packages.md`](packages.md).

The one hard part is knowing when a name is *not* the module's. In

```zeno
from shapes import area

function paint(area: int) {
    print(area)          # the parameter, not shapes.area
}
```

the `area` inside `paint` is the parameter. So the renamer walks the
file keeping a list of what the functions and blocks around it have
declared (`scopes`), the same way the checker and the interpreter do,
and leaves those names alone.

It is also where a file is kept from seeing too much. The first
file's names stay as they are written, so after the join an imported
file could reach one of them just by spelling it. `Renamer::plain`
refuses that:

```
error: this file declares no `total`. The program's first file has one, and an
imported file cannot see what the file that imports it declares. Hand the value
to the function that needs it, as an argument
```

### Join

The statements of the files, in `order`, become one list. The import
lines of files are taken out: they have done their work. `import math`
stays, because the stages after this one handle that themselves.

### Which file is this error in?

Every part of the syntax tree remembers where it was written, as a
`Span`: a line and a column. Error messages are built from it. With
several files, "line 12" is no longer enough.

The obvious fix is a third number, the file. But a position is passed
along everywhere, down to the runtime of a compiled program, where
every operation that can fail takes `line, col`:

```c
zeno_floordiv(z_a, z_b, 31, 12)
```

So instead the lines of the whole program are numbered in one run
(7.12). The first file's lines are 1 to 999,999, as your editor shows
them. The second file's start at 1,000,001:

```rust
pub const LINES_A_FILE: usize = 1_000_000;

impl Span {
    pub fn file(self) -> usize { self.line / LINES_A_FILE }
    pub fn line_in_file(self) -> usize { self.line % LINES_A_FILE }
}
```

The lexer is told which file it is reading and starts counting from
there. Nothing else changes, and the same line in a compiled program
is now

```c
zeno_floordiv(z_a, z_b, 1000031, 12)     /* line 31 of the second file */
```

Only the few places that show a position to a person take the number
apart: `report` in `src/main.rs`, and two lines of the C runtime,
which has been given the names of the files:

```c
static const char *zeno_file_of(int line) {
    return line < 1000000 ? zeno_file : zeno_modules[line / 1000000 - 1];
}
```

Clang and the Rust compiler keep positions the same way.

### What the compilers do with a dotted name

A C name cannot have a dot in it. The C backend already had a
function for names that C does not allow, written for generics, where
`Stack[int]` becomes `Stack_int`. The same function turns
`shapes.area` into `shapes_area`:

```c
static double z_shapes_area(z_shapes_Shape *z_shape);
static int64_t z_drivers_uart_sent;
```

LLVM IR allows a dot in a name, so there the function is
`@z_shapes.area`.

What a program *prints* must not have the module in it (rule 7).
`shown_name` in `src/ast.rs` cuts a name down to what its file
called it, and the three places that print a class or a case (the
interpreter's `Display`, the C `zeno_show_` functions and the IR ones)
use it.

## What is left out

- **Hiding a declaration** was left out of this stage and has come
  since: `private function helper()` is for its own file only. See
  [`private.md`](private.md).
- **No second name for a module** (`import drivers.uart as uart`).
  `from drivers.uart import send` gives the short form (7.3).
- **The whole program is compiled together.** Change one file and
  everything is checked and built again. That is fast for the
  programs Zeno has now, and it is what lets types be worked out
  across files (7.11).
- **A module that comes with Zeno wins over a file of the same
  name.** `math.zeno` is never read by `import math` (7.2).
- **Other people's modules** are not on this page. A module of the
  program is found from the program's own folder and nowhere else. A
  folder of modules that lives somewhere else is a package, and
  [`packages.md`](packages.md) is about those.

## Things to try

1. Run `cargo run -- join examples/station/main.zeno` and find the
   first line that came from `main.zeno`. Which file's statements are
   first, and why that one?
2. In `main.zeno`, move `import report` below the `let readings`
   line. Read the message. Then put it back.
3. Add `import report` to the top of `sensors.zeno`. `report.zeno`
   already imports `sensors`. What does Zeno say, and which file does
   it point at?
4. In `main.zeno`, write `drivers.uart.sent = 0` before the last
   line. Then add a function `reset()` to `drivers/uart.zeno` that
   does it, and call that instead.
5. Change `sensors.check(reading)` to `sensors.chek(reading)`. The
   message lists what the module has. Where does that list come from
   in `src/modules.rs`?
6. Make a mistake inside `report.zeno`, such as `return good + 1` in
   `summary`. Which file and line does the error name? Now look at
   `Span::file` in `src/token.rs` and work out what number the
   checker saw as the line.
7. Add a class `Health` to `main.zeno` with one field, and print one.
   The program now has a class `Health` and an enum `Health`, in two
   files. Run it and build it: both work. Now call your class
   `sensors_Health` and build again. Read the message, and work out
   from "What the compilers do with a dotted name" why `zeno run`
   does not mind.
8. Build the station with no collector:
   `cargo run -- build --no-collector --memory examples/station/main.zeno`
   and run `./main`. How many blocks are left at the end?
9. Open the `main.c` that step 8 wrote and search for `1000`. Find a
   line number of the second file, and find where `zeno_modules` is
   set.
10. Put `strict` as the first line of `sensors.zeno` and take the
    type off `reading` in `check`. Run it. It still passes: why? (Read
    7.10, then look at how `check` is called.) Now add a function
    there that nothing calls, with a parameter that has no type.
