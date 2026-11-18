# Libraries of C, Go and Rust

Most of the code a computer runs is C, or is reached through C: the
library of the system, zlib, SQLite, OpenSSL, a vendor's driver, and
the code a board's firmware already has. Go and Rust can both make a
library that C can call. A Zeno program can call any of them, as they
are.

```zeno
import c

let m = c.library("m", ["double cbrt(double x);", "double hypot(double x, double y);"])
print(m.cbrt(27.0), m.hypot(3.0, 4.0))
```

```
3.0000000000000004 5.0
```

`c.library` takes two things: the library, and the functions of it
that the program uses, declared as C declares them. The declarations
are the lines a header has, copied as they are. From there, each
function is called as a function of Zeno is.

This is the third version of using other languages' libraries, after
Python, JavaScript and Java ([`python.md`](python.md),
[`node.md`](node.md), [`java.md`](java.md)) and Ruby, PHP, Lua and C#.
Nothing changes for a program that does not say `import c`. The
decisions behind it are chapter 44 of [`decisions.md`](decisions.md).

## What it needs

A C compiler: `cc`, `gcc` or `clang`, the first that is installed, or
the one `CC` names, as for `zeno build`. The first time a program runs
a `c.library`, Zeno writes a small file of C for the functions it
declares and builds it with the library. That takes a moment, about
half a second, and is kept: the next run starts at once. It is built
again when the declarations change, or a file of the library does.

The library itself has to be there with what building with it needs.
On Debian and Ubuntu that is the package whose name ends in `-dev`:
`zlib1g-dev` for zlib, `libsqlite3-dev` for SQLite.

## Naming the library

The first thing given is one text, or a list of them:

| Given            | What it is                                                         |
|------------------|--------------------------------------------------------------------|
| `"z"`            | A library of the system, by its name: `-lz`, as C is built with it  |
| `"crc.c"`        | A file of C, built with the rest                                   |
| `"libtemp.so"`   | A library that was built already: `.so`, `.a`, `.o`, `.dylib`, `.dll` |
| `"-DSTEP=3"`     | Anything that starts with `-` goes to the compiler as it is        |
| `none`           | Nothing more: for the C library every program has (`strlen`, `abs`) |

A file is found from the folder of the program, wherever the program
is started from, and so is a header it includes.

```zeno
let crc = c.library(["crc.c", "-DCRC_TABLE"], "uint16_t crc16(const uint8_t *data, size_t length);")
let z = c.library("z", ["#include <zlib.h>", "uLong crc32(uLong crc, const Bytef *buf, uInt len);"])
```

A library that is not there is a `FileError`, which a program can
catch:

```
error: uncaught FileError: C has no library `sqlite`. Is it installed, with what
building with it needs (on Debian and Ubuntu, its package that ends in `-dev`)?
```

## Declaring the functions

The second thing given is the declarations: one text, or a list of
texts, each declaration ending with `;`. They are C, and only the
functions the program uses are needed.

- A line that starts with `#`, such as `#include <zlib.h>`, is given to
  C as it is. With the header included, the compiler checks each
  declaration against it, and a type the header names (`uLong`,
  `Bytef`) is known.
- A `typedef`, a `struct`, a `union` or an `enum` is given to C as it
  is too: `typedef struct counter counter;`.
- `int8_t` to `uint64_t`, `size_t` and `bool` can be used with no
  `#include`.

A declaration that names a function the library does not have is an
error when `c.library` is called, not later:

```
error: uncaught Error: the library `m` has no function `cube_root`, which was declared
```

`print` shows a library by its name, and a function by its
declaration:

```zeno
print(m, m.cbrt)
```

```
<C library m> double cbrt(double x)
```

## Values go across

| C                                    | Zeno                                                         |
|--------------------------------------|--------------------------------------------------------------|
| `int`, `uint8_t`, `long`, `size_t`... | `int`, checked to fit: `OverflowError` if it does not         |
| `uint64_t` above the top of `int`     | `u64`                                                         |
| `float`, `double`                    | `float`                                                       |
| `bool`, `_Bool`                      | `bool`                                                        |
| `const char *` given                 | a `str`, lent for the call, with the `\0` C expects           |
| `char *` given back                  | a `str`, copied (a list of bytes if it is not UTF-8)          |
| `T *` given                          | a list, lent for the call: copied in, and back after          |
| any other pointer given back         | kept: a pointer Zeno gives back to C, and never looks inside  |
| `NULL`                               | `none`                                                        |
| `void`                               | `none`                                                        |

**A list for a pointer is lent.** Its items are copied into memory of
the type the pointer is to, the function is called, and the items are
copied back into the list afterwards, so a function that fills a
buffer fills the list. A pointer to `const` is not copied back: the
function promised not to change it.

```zeno
let m = c.library("m", "double frexp(double x, int *exponent);")
let exponent = [0]
print(m.frexp(8.0, exponent), exponent)
```

```
0.5 [4]
```

**A pointer C gives back is kept as it is.** It can be given back to
C, compared with `==`, and printed, and that is all. This is how a
library that hands out a handle (`FILE *`, `sqlite3 *`, a driver's
context) is used:

```zeno
let counter = mine.counter_new()
mine.counter_add(counter, 5)
print(mine.counter_get(counter))
mine.counter_free(counter)
```

A function that gives a handle back through a pointer to a pointer
(`sqlite3 **db`) is given a list of one `none`, and finds the handle
in it afterwards:

```zeno
let sqlite = c.library("sqlite3", [
    "#include <sqlite3.h>",
    "int sqlite3_open(const char *filename, sqlite3 **db);",
    "int sqlite3_close(sqlite3 *db);",
])
let db = [none]
print(sqlite.sqlite3_open(":memory:", db))
sqlite.sqlite3_close(db[0])
```

```
0
```

Zeno never frees memory that C gave it. A library that says the caller
frees what it returns has a function to do it, and the program calls
that, as it would in C.

## Libraries of Go and Rust

Go and Rust each build a library that C can call, and `c.library`
takes that library. The build is done by their own tools.

**Go**: mark each function `//export`, and build with
`-buildmode=c-shared` (a `.so`) or `-buildmode=c-archive` (a `.a`):

```go
package main

import "C"

//export Celsius
func Celsius(f float64) float64 { return (f - 32) * 5 / 9 }

func main() {}
```

```
go build -buildmode=c-shared -o libtemp.so temp.go
```

```zeno
import c

let temp = c.library("libtemp.so", "double Celsius(double f);")
print(temp.Celsius(98.6))
```

```
37.0
```

Go writes `libtemp.h` beside the library, with the declarations.

**Rust**: mark each function `#[no_mangle] pub extern "C"`, and build
it as a `cdylib` (a `.so`) or a `staticlib` (a `.a`). With Cargo, that
is `crate-type = ["cdylib"]` under `[lib]` in `Cargo.toml`, and
`cargo build --release`.

```rust
#[no_mangle]
pub extern "C" fn average(values: *const f64, count: usize) -> f64 {
    let values = unsafe { std::slice::from_raw_parts(values, count) };
    values.iter().sum::<f64>() / count as f64
}
```

```
rustc -O --crate-type cdylib filter.rs -o libfilter.so
```

```zeno
import c

let filter = c.library("libfilter.so", "double average(const double *values, size_t count);")
let readings = [20.0, 22.0, 21.0, 25.0]
print(filter.average(readings, len(readings)))
```

```
22.0
```

## Errors

A mistake Zeno can see is an error of Zeno, at the line that made it:

```
error: uncaught Error: the 1st parameter of `sqrt` (`double`) takes a number, and this is a str
error: uncaught OverflowError: the 1st parameter of `same8` (`int8_t`) holds -128 to 127, and this is 300
error: uncaught ValueError: `printf` takes any number of arguments (`...`), and Zeno cannot know
their types. Declare it with the ones the program gives, under another name of C, or write a
function of C that calls it
```

A mistake inside C, such as reading memory that is not there, stops
C. It runs beside Zeno, so the program stops with an error at the
line that called it, and Zeno itself does not crash:

```
error: C stopped while Zeno was waiting for its answer
```

A declaration that does not agree with the header is the compiler's
error, with its first lines shown.

## What is not there yet

- **`zeno build`.** A program that says `import c` runs with
  `zeno run`. For `zeno build`, which writes C already, calling the
  functions directly from the C it writes, with no helper, is the next
  step: that is the one firmware needs, for a vendor's HAL.
- **A function of Zeno given to C**, as a callback.
- **A struct given or given back whole**, not through a pointer.
- **A function that takes `...`**, such as `printf`.
- **Reading the declarations from a header by itself.** They are
  copied by hand for now, and only the ones used are needed.

## The example

[`examples/modbus.zeno`](../examples/modbus.zeno) makes a request of
Modbus RTU and checks the frames that come back, with the checksum
written in C as a board's firmware already has it
([`examples/modbus.c`](../examples/modbus.c)):

```
zeno run examples/modbus.zeno
```

```
request:  01 03 00 6B 00 03 74 17
good:     true
reply:    01 03 06 02 2B 00 00 00 64 05 7A true
damaged:  01 03 06 02 3B 00 00 00 64 05 7A false
```

## How it is built

C has no way to ask a library what its functions take: a library
knows the names of its functions and nothing of their types. That is
why the program declares them. Zeno reads the declarations, and
writes a small file of C with a function for each, which takes its
arguments from a table of numbers and calls the real one. The C
compiler builds that with the library, so it is the compiler that
works out what `uLong` or `size_t` is on this machine, and checks the
declarations against the header.

That library is loaded by a small helper written in C
([`src/bridge.c`](../src/bridge.c)), which runs beside the program and
is spoken to as the helpers of the other languages are. A fault in C
stops the helper and not Zeno, and a program that never says
`import c` never starts it.

## Things to try

1. Call `crc32` of zlib on a file read with `files.read_bytes`.
2. Open a database with SQLite's `sqlite3_open`, keeping the
   `sqlite3 *` it gives back.
3. Write a function in Go that formats a number with
   `strconv.FormatFloat`, and call it from Zeno.
