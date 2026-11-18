# Targets: a Zeno program written in another language

`zeno build --target <language>` writes your program in another
language: a file you can read, keep, and run with that language's own
tools.

```
$ zeno build --target js examples/hello.zeno
Wrote hello.js. Run it with `node hello.js`
$ node hello.js
Hello, Zeno!

$ zeno build --target py examples/hello.zeno
Wrote hello.py. Run it with `python3 hello.py`

$ zeno build --target go examples/hello.zeno
Wrote hello.go. Run it with `go run hello.go`

$ zeno build --target html examples/hello.zeno
Wrote hello.html. Open it in a browser
```

| `--target` | Writes | Run it with |
|---|---|---|
| `js` | `hello.js` | `node hello.js` |
| `ts` | `hello.ts`, TypeScript | `node hello.ts` (Node 22.18 or later), or `tsc`, Bun, Deno |
| `html` | `hello.html`, a page that runs the program | a browser |
| `py` | `hello.py` | `python3 hello.py` |
| `php` | `hello.php` | `php hello.php` (PHP 8.0 or later, no extension needed) |
| `lua` | `hello.lua` | `lua hello.lua` (Lua 5.4) |
| `ruby` | `hello.rb` | `ruby hello.rb` (Ruby 3.0 or later, no gem needed) |
| `dart` | `hello.dart` | `dart run hello.dart` (Dart 3 or later) |
| `kotlin` | `hello.kt` | `kotlinc hello.kt -include-runtime -d hello.jar && java -jar hello.jar` |
| `go` | `hello.go` | `go run hello.go` |
| `java` | `hello.java` | `java hello.java` (JDK 11 or later) |
| `cs` | `hello.cs` | `dotnet run hello.cs` (.NET 10 or later) |
| `cpp` | `hello.cpp` | `g++ -std=c++17 -O2 hello.cpp -o hello && ./hello` |
| `rust` | `hello.rs` | `rustc -O --edition 2021 hello.rs -o hello && ./hello` |
| `wasm` | `hello.wasm`, built by `rustc` | `wasmtime hello.wasm`, or any WebAssembly host with WASI |

`--target wasm` is the one that builds as well as writes: it writes the
program in Rust, has `rustc` build that for WebAssembly, and keeps the
`.wasm` file. It needs Rust and its WebAssembly target
(`rustup target add wasm32-wasip1`). The file runs in wasmtime, wasmer,
or Node (`node tools/run-wasi.js hello.wasm`), with the keyboard, the
command line and the files of the folder it runs in.

The file is written by fixed rules, the same every time: the same
program gives the same file, letter for letter. No guessing and no AI
is involved anywhere in it (decision 24.1).

## What the written program does

Exactly what `zeno run` does. It prints the same text, stops at the same
line with the same message, and counts the same way:

```zeno
let big = 9223372036854775807
print(big)
print(big + 1)
```

```
$ zeno run big.zeno
9223372036854775807
error: uncaught OverflowError: the result is too big for an int
 --> big.zeno:3:7
  |
3 | print(big + 1)
  |       ^
$ zeno build --target js big.zeno && node big.js
9223372036854775807
error: uncaught OverflowError: the result is too big for an int
 --> big.zeno:3
```

(The written program names the line but does not show it: it does not
carry the program's text.)

JavaScript has no 64-bit whole number of its own, and Python's never
overflows; the written program still overflows where Zeno's does,
because the rules of Zeno come with it.

That is held to by a tool, not by hope. `tools/targets.py` runs every
example, every sample of the book and twenty-three programs written to try
the corners (`tools/targets/`: the edges of an int, floats as text,
text in every script, the order a line is worked out in, errors, deep
calls, JSON, files, typed lines that are not text, names that are words
of other languages) with `zeno run`, then builds each for every target,
runs it, and compares what it printed and, if it stopped with an error,
the message, the file and the line:

```
$ python3 tools/targets.py
js: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
ts: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
py: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
php: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
lua: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
ruby: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
dart: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
kotlin: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
go: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
java: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
cs: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
cpp: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
rust: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
wasm: 338 of 338 programs print what `zeno run` prints (23 left out: they use the board, a page, manual memory, another program, Python)
html: 329 of 329 programs print what `zeno run` prints (32 left out: ...)
```

## How it is done

Three steps, in three files.

1. **Lowering** (`src/lower.rs`), shared by every target. The checked
   program is turned into a small, plain program that any language can
   say: every name made unique, each block's variables listed at its
   top, functions moved to the top of their block (so a function can be
   called before the line it is written on, as in Zeno), and
   everything Zeno writes as one expression but other languages need
   statements for (`[x * x for x in xs if x > 0]`, `0 <= t < 10`) made
   into statements, in the order Zeno would do them.
2. **The runtime** (`src/runtime.js`, `src/runtime.py`,
   `src/runtime.php`, `src/runtime.lua`, `src/runtime.rb`,
   `src/runtime.dart`, `src/runtime.kt`, `src/runtime.go`,
   `src/runtime.java`, `src/runtime.cs`, `src/runtime.cpp`,
   `src/runtime.rs`): what every Zeno
   value is in that language, and what each operator and built-in does
   with it, written to do what `ops.rs` and `builtins.rs` do for
   `zeno run`, step for step and message for message. It goes in the
   same file as the program, so the file needs nothing else.
3. **Printing** (`src/emit_js.rs`, `src/emit_py.rs`, `src/emit_php.rs`,
   `src/emit_lua.rs`, `src/emit_ruby.rs`, `src/emit_dart.rs`,
   `src/emit_kotlin.rs`, `src/emit_go.rs`, `src/emit_java.rs`,
   `src/emit_cs.rs`, `src/emit_cpp.rs`, `src/emit_rust.rs`): the
   lowered program in that language's words.

Here is a function, and what the JavaScript and the Python say:

```zeno
import math

function area(r: float) -> float {
    return math.pi * r * r
}
print(area(2.0))
```

```js
Z.start(["area.zeno"], function main() {
  let area, math_;
  area = Z.fn("area", ["r"], [false], function (r) {
    return Z.mul(Z.mul(Z.builtin("math.pi"), r, 4), r, 4);
  });
  math_ = Z.module("math");
  Z.b.print([Z.direct(area, 6, Z.float(2.0))], 6);
});
```

```python
def main():
    area = None
    math_ = Z.Cell()
    def z_f1(r, *, math_=math_):
        return Z.mul(Z.mul(Z.builtin("math.pi"), r, 4), r, 4)
    area = Z.fn("area", ["r"], [False], z_f1)
    math_.v = Z.module("math")
    Z.b.print([Z.direct(area, 6, 2.0)], 6)
```

`Z.mul(a, b, 4)` multiplies as Zeno multiplies, and if the answer does
not fit, stops with Zeno's message for line 4. `Z.direct(area, 6, ...)`
calls `area` straight away, since it is known where it is written, and
counts the call: the 1000th call inside another stops the program at
line 6 with Zeno's message, as `zeno run` does (decision 24.23). `math`
is the module, renamed `math_` because `math` is a name the targets'
own libraries use.
`area` is moved above the `import` line, as Zeno lets a function be
used before the line it is written on. A function is a real
function of the language, with its Zeno name and parameters kept on it
so that `area(r: 2.0)` can be lined up.

In Go, Java and C# every value is the language's "anything" (`any`,
`Object`, `object`), and the program is at the bottom of the file, after
the runtime in Go and at the top in Java and C#. The Go:

```go
func main() {
	Zstart([]string{"area.zeno"}, func() ZV {
		var area, math_ ZV
		_, _ = area, math_
		area = Zfn("area", []string{"r"}, []bool{false}, func(z_args []ZV) ZV {
			r := z_args[0]
			_ = r
			return Zmul(Zmul(Zbuiltin("math.pi", 0), r, 4), r, 4)
		})
		math_ = Zmodule("math")
		_ = Zb_print([]ZV{Zdirect(area, 6, float64(2.0))}, 6)
		return nil
	})
}
```

and the C#:

```csharp
Z.start(new string[] {"area.zeno"}, args, () => {
    object area = null, math_ = null;
    area = Z.fn("area", new string[] {"r"}, new bool[] {false}, (object[] z_args1) => {
        object r = z_args1[0];
        return Z.mul(Z.mul(Z.builtin("math.pi", 0), r, 4), r, 4);
        return null;
    });
    math_ = Z.module("math");
    Z.b_print(new object[] {Z.direct(area, 6, new object[] {2.0})}, 6);
    return null;
});
```

The `_ = r` lines are there because Go refuses a variable that is never
read. The `return null;` after a `return` is there because a Zeno
function that ends without one gives `none`; C# only warns about it, and
the file turns that warning off. Java needs `if (true) return ...;` for
the same line, since it refuses it (decision 24.13).

### Names

Every variable gets a name that no other one in the program has
(`total`, then `total_2`), and a name that is a word of any of the
target languages gets a `_` after it (`math` is fine in Zeno, and is
`math_` in every target). One list of words serves every target, so a
program's names are the same in every language it is written in.

### Variables an inner function uses

```zeno
var makers: list[function() -> int] = []
for i in range(3) {
    makers.append(function() -> int { return i })
}
for m in makers {
    print(m())
}
```

In Zeno each round of the loop has its own `i`, so the three functions
give 0, 1 and 2. A JavaScript `const` in a `for ... of` does the same.
Python's closures would share one `i` and give 2, 2, 2, so for Python a
variable that an inner function uses is kept in a box (`Z.Cell`), a
new one each round, and the inner function is handed the box when it is
made:

```python
    for z_item in Z.items(Z.b.range([3], 2), 2):
        i = Z.Cell(z_item)
        def z_f1(*, i=i):
            return i.v
        Z.call_method(makers, "append", [Z.fn("", [], [], z_f1)], 3)
```

## TypeScript: `--target ts`

`--target ts` writes the program `--target js` writes, as TypeScript:
`src/runtime.ts` is `runtime.js` with types, and every variable,
parameter and caught error of the program has its type.

```ts
Z.start(["area.zeno"], function main() {
  let area: any, math_: any;
  area = Z.fn("area", ["r"], [false], function (r: any) {
    return Z.mul(Z.mul(Z.builtin("math.pi"), r, 4), r, 4);
  });
  math_ = Z.module("math");
  Z.b.print([Z.direct(area, 7, Z.float(2.0))], 7);
});

export {};
```

The type is `any`. What a Zeno value is, is found out while the
program runs, by the runtime, as in every target: that is what gives
Zeno's message at Zeno's line for an int that overflows or a unit
that does not fit. A `number` on a Zeno int would promise what
JavaScript does, and Zeno does not do what JavaScript does (decision
24.27).

The file passes `tsc --strict`, so a TypeScript project can take it as
it is, and it is a module (it ends in `export {};`), so two built
programs in one project do not share their names. Node 22.18 or later
runs a `.ts` file as it is; `tools/targets.py` checks each program
with `tsc --strict` and then runs it with Node.

## PHP: `--target php`

```php
Z::start(["area.zeno"], function () {
    $area = $math_ = null;
    $area = Z::fn("area", ["r"], [false], function ($r) use (&$math_) {
        return Z::mul(Z::mul(Z::builtin("math.pi"), $r, 4), $r, 4);
    });
    $math_ = Z::module("math");
    Z::b_print([Z::direct($area, 7, 2.0)], 7);
});
```

An int is a PHP int and a float a PHP float: PHP keeps the two apart
and its int has 64 bits. A sum too big for an int becomes a float in
PHP, which is how the runtime sees the overflow and raises Zeno's
error. A list and a dict are objects around a PHP array, because PHP
copies an array when it is handed on and a Zeno list is shared.

An inner function names the variables around it that it works with,
after `use`, with `&` so that it is the same variable and not a copy.
The file needs PHP 8.0 or later and no extension: not `mbstring`, not
`bcmath` (decision 24.28).

## Lua: `--target lua`

```lua
Z.start({"area.zeno"}, function()
  local area, math_
  area = Z.fn("area", {"r"}, {false}, function(r)
    return Z.mul(Z.mul(Z.builtin("math.pi"), r, 4), r, 4)
  end)
  math_ = Z.module("math")
  Z.b_print({Z.direct(area, 6, 2.0)}, 6)
end)
```

An int is a Lua integer and a float a Lua float: Lua 5.4 keeps the two
apart, and its integer has 64 bits. Lua's own `+` wraps around when
the answer does not fit, so `Z.add` looks for that and raises Zeno's
error. Zeno's `none` is a value of the runtime's own, `Z.none`, and
not Lua's `nil`: a Lua table cannot hold `nil`, and a Zeno list can
hold `none`.

A Lua function shares the variables around it, and a `local` in the
body of a loop is a new variable each round, so a function made in a
loop needs no box. Three things Lua does not have are written with
what it has (decision 24.30):

```lua
  for m in Z.items(makers, 8) do
    Z.b_print({Z.call(m, {}, 9)}, 9)
  end
  do
    local z_ok, z_how = Z.try(function()
      Z.b_print({Z.index(makers, 5, 12)}, 12)
    end)
    if not z_ok then
      local t = z_how
      if Z.catches(t, "IndexError") then
```

- **`try`.** The body is a function handed to `Z.try`, which says
  whether it raised. A `return`, `break` or `continue` in the body
  cannot leave the function around it from in there, so the body says
  how it ended and the lines after the call do it.
- **`continue`** is a `goto` to a label at the end of the loop's body.
- **More than 200 variables.** A Lua function may have 200 `local`s. A
  block that would pass that keeps its variables in a table.

Lua by itself has no clock finer than a second and nothing to wait
with. If LuaSocket is installed the runtime uses its clock and its
`sleep`; if not, `time.now()` counts the time the processor worked and
`sleep` waits by counting it (see "Known limits").

## Ruby: `--target ruby`

```ruby
Z.start(["area.zeno"], -> {
  area = math_ = nil
  area = Z.fn("area", ["r"], [false], ->(r) {
    return Z.mul(Z.mul(Z.builtin("math.pi"), r, 4), r, 4)
  })
  math_ = Z.module_of("math")
  Z.b_print([Z.direct(area, 6, 2.0)], 6)
})
```

A Zeno function is a Ruby lambda (`->(r) { ... }`): a lambda takes
exactly the arguments it names, `return` leaves it, and it shares the
variables around it. An int is a Ruby Integer; Ruby's never overflows,
so each step looks whether the answer still fits in 64 bits. `none` is
`nil`, a list an Array, and a dict a Hash, which keeps its keys in the
order they were added, as Zeno's does.

A `for` is a block, so each round has its own variable. A `while`
stays Ruby's `while`, unless its body makes a variable that an inner
function uses: Ruby's `while` has one such variable for all its
rounds, so that loop is a block too (decision 24.31):

```ruby
  Z.loop do
    break unless Z.lt(i, 3, 3)
    n = nil
    n = Z.mul(i, i, 4)
    Z.call_method(makers, "append", [Z.fn("", [], [], ->() {
      return n
    })], 5)
    i = Z.add(i, 1, 6)
  end
  Z.each(makers, 8) do |m|
    Z.b_print([Z.call(m, [], 9)], 9)
  end
```

A name that begins with a capital is a constant in Ruby, which cannot
be set twice or inside a function, so a Zeno variable `Total` is
written `z_Total`. Ruby gives a program one megabyte for calls inside
one another, less than 999 Zeno calls can need, and how much is
decided before Ruby starts: the program starts itself once more,
asking for 64.

## Dart: `--target dart`

```dart
void main(List<String> z_args) {
  Z.start(<String>["area.zeno"], z_args, () {
    Object? area, math_;
    area = Z.fn("area", <String>["r"], <bool>[false], (List<Object?> z_a) {
      Object? r = z_a[0];
      return Z.mul(Z.mul(Z.builtin("math.pi"), r, 4), r, 4);
    });
    math_ = Z.module("math");
    Z.b_print(<Object?>[Z.direct(area, 6, <Object?>[2.0])], 6);
    return null;
  });
}
```

Every value is an `Object?`, Dart's "anything, or nothing", as it is
`any` in Go and `Object` in Java. An int is a Dart `int`, 64 bits on
the Dart VM, and a float a `double`. Every function takes one list of
its arguments, so that all of them have one type and a call can be
lined up by the runtime. A variable declared in the body of a Dart
loop is a new one each round, so a function made in a loop needs no
box.

The file is for the Dart VM (`dart run`, or `dart compile exe`). It is
not for a web page: compiled to JavaScript, a Dart `int` is a
JavaScript number and no longer has 64 bits. For a page there is
`--target html` (decision 24.32).

## Kotlin: `--target kotlin`

```kotlin
fun main(args: Array<String>) {
    Z.start(arrayOf<String>("area.zeno"), args, fun(): Any? {
        var area: Any? = null
        var math_: Any? = null
        area = Z.fn("area", arrayOf<String>("r"), booleanArrayOf(false), fun(z_a: Array<Any?>): Any? {
            var r: Any? = z_a[0]
            return Z.mul(Z.mul(Z.builtin("math.pi", 0), r, 4), r, 4)
        })
        math_ = Z.module("math")
        Z.b_print(arrayOf<Any?>(Z.direct(area, 6, arrayOf<Any?>(2.0))), 6)
        return null
    })
}
```

Kotlin runs on Java's machine, and `src/runtime.kt` is the Java
runtime said in Kotlin: every value is an `Any?`, an int a `Long`, a
list an `ArrayList`. Two things are simpler than in Java. An inner
function may change the variables around it, so there is no `Z.Cell`.
And a line after a `return` is only a warning; the file turns Kotlin's
warnings off in its first line, as the C# file does, so `return` needs
no `if (true)` before it.

A Zeno function is written `fun(...): Any? { ... }` and not as a
Kotlin lambda `{ ... }`: in a lambda a plain `return` is not allowed,
and in a `fun` it means what it means in Zeno (decision 24.33).

`kotlinc` makes a `.jar` with Kotlin's own library in it, and `java`
runs that. Compiling takes some seconds for any program, because the
runtime is compiled with it each time.

## The page: `--target html`

`--target html` writes one `.html` file: the program in JavaScript,
with its runtime, in a `<script>` at the end of a page. What it prints
appears in the page, in the element with `id="zeno-output"` (a `<pre>`
if the page has none), and an error that stops it is shown under what it
printed. Open the file and it runs: there is nothing else to load, so it
works from a disk, an email, or any web host.

If there is an `.html` file beside the program with the same name
(`counter.html` beside `counter.zeno`), that page is used, and the
script is put before its `</body>`. Building in the program's own
folder would then write over that page, so Zeno refuses and says to
build from another folder.

A page has no command line and no files to write, and must never stand
still: `files.write` and `sleep` stop the program with a message that
says so, and `program.arguments()` is empty. `input("Name: ")` asks with
the browser's own question box, as in `--web` (decision 14.11).

How this differs from `zeno build --web`: `--web` puts Zeno itself
(as WebAssembly) in the page and runs the program as `zeno run` does,
so `import web` works and the program can reach the page's buttons.
`--target html` has the program compiled to JavaScript: a smaller
file and a faster program, but no `import web` yet.

## What cannot be written for another language

- `import board`: the pins of a board belong to a program compiled for
  that board.
- `import web`, except in `--web` builds.
- `unsafe` blocks and manual memory: they are only for a program
  compiled to machine code.
- `program.run`, which starts another program: only `zeno run` has
  it so far (decision 25.6).
- `import python`: Python runs beside the program, and `zeno run` is
  what starts it (decision 26.9).

Zeno refuses such a program with a message that names the line.

In C++ every value is a `V`, a small struct holding an int, float or
bool itself and anything else by a shared pointer. A variable an inner
function uses is kept in a `Z::Cell`, as in Java, since a C++ lambda
copies what it uses. C++ does not say in which order it works out a
call's arguments, so the lowering puts them in variables first, as for
Go (decision 24.14). C++, like Python, C# and Rust, uses the C
library's `sin` and `exp`, so its floats print the very digits
`zeno run` prints.

Rust has no exceptions, so every step that can fail gives a result and
the program passes a failure on with `?`; a `try` body is a closure
whose failure the code after it looks at, as in Go. Zeno is itself
written in Rust, so the Rust runtime prints and reads floats, and
quotes text, with the very code `zeno run` uses.

## Corners that have a rule

A review of every target (2026-11-16) wrote programs to look for
places where a target did what its own language does and not what
`zeno run` does. It found about thirty, all in corners the first 319
programs did not reach. Each now has a rule that `zeno run` and every
target keep, and a program in `tools/targets/` that holds them to it:

- **The order of a line.** Named arguments are worked out in the order
  they are written; `x += f()` reads `x` first; a variable is read
  when its turn comes, whatever a later part of the line changes
  (decision 24.22).
- **Text in quotes.** `["தமிழ்"]` prints as it is written. Only
  control characters are shown as codes (24.20).
- **Floats.** A float exactly between two shortest texts takes the
  even one (24.21); the smallest floats print as `5e-324` in Java too.
- **Nesting.** Every target counts calls and stops at the 1000th, with
  the function's name and the line (24.23).
- **The library.** `lines()` and `input()` and a lone `\r`; typed bytes
  that are not UTF-8; `range` at the last int; `math.floor` of what is
  not a number; `-0.0`; `\u` in JSON; a path with nothing in it
  (24.24).
- **Names.** Any file name builds (`my prog.zeno`, `2048.zeno`,
  `speed_test.zeno`, `struct.zeno`); a name in capitals is safe from
  the macros of C; `args` and `delegate` are safe in Java and C#
  (24.25). `end`, `local`, `rescue`, `it`, `fun`, `val` and `List`
  are safe in Lua, Ruby, Dart and Kotlin, and so is text that says
  `#{x}` or `${x}`, which those languages would fill in (24.34).
- **Multiply, then add.** `a * b + c` is two steps, each rounded, on
  a processor that could do both in one: the C++ file says so to its
  compiler, as the C of `zeno build` does, and `7.0 % 0.1` is `0.0`
  everywhere (24.35).
- **Long chains.** A linked list of a million objects is let go link
  by link in Rust and WebAssembly, not by a million calls inside one
  another, which a WebAssembly host has no stack for.
- **A page.** A text that says `</script>` or `<!--` cannot end or
  hide the script of the page it is printed in.

## Known limits

- **The last digit of `sin`, `exp`, `pow` and the like.** `zeno run`
  asks the computer's C library for these. Python, C# (and C++ and Rust)
  use the same library and print the same digits. Node, Java and Go
  carry their own, which can give a different 17th digit, the last one
  a float prints, for some inputs. Go uses a copy of JavaScript's `sin`,
  `cos` and `tan`, and Java uses `Math` rather than `StrictMath`, which
  makes it rare; none of the 338 programs shows it (decision 24.16).
  Lua and Ruby use the C library. Kotlin uses Java's `Math`, as the
  Java target does. Dart has its own; its `log10` is worked out the
  way the C library does it, because `log(x) / ln10` is one digit off
  for some inputs. On ARM64, Go may also do a multiply and an add of
  its own `sin`, `cos` and `tan` in one step, which could move a last
  digit there (24.35).
- **Time in Lua.** Lua has a clock in whole seconds and no `sleep`.
  With LuaSocket installed, `time.now()` and `sleep` are exact. Without
  it, `time.now()` counts the time the processor worked, which is
  right for timing a piece of the program and wrong across a wait, and
  `sleep` keeps the processor busy until the time is over.
- **A very long line.** A sum of some hundreds of terms written as one
  expression (`a1 + a2 + ... + a300`) becomes as many calls inside one
  another, which is more than the parsers of Python and Lua take. Put
  the terms in a list and use `sum`, or add them in steps.
- **Kotlin takes seconds to compile.** About ten for any program:
  `kotlinc` compiles the runtime with it each time.
- **Node 22 and a big WebAssembly program.** Node 22 (seen with
  22.23) can stop with a segmentation fault while it runs a `.wasm`
  that asks for hundreds of megabytes: the strings and sort benchmarks
  at a million items stopped in about half of their runs. The fault is
  inside Node, in its garbage collector, when the program's memory
  grows; a WebAssembly program cannot do that to its host by any
  mistake of its own. Node 24 ran the same files every time. If it
  happens, use a newer Node, or wasmtime.
- **A device as a file.** On WebAssembly a program sees only the folder
  it was started in, so `/dev/null` and the like are not there.

## Speed

The written program is as correct as `zeno run` first, and as fast as
that allows. Seconds, on the benchmarks in `bench/`, against `zeno run
--vm`:

| | `zeno run --vm` | `js` (Node 22) | `py` (Python 3.13) | `php` (PHP 8.3) | `go` (Go 1.24) | `java` (JDK 21) | `cs` (.NET 10) | `cpp` (g++ 13) | `rust` (1.97) | `wasm` (in Node) |
|---|---|---|---|---|---|---|---|---|---|---|
| fib | 0.81 | 0.14 | 3.4 | 2.1 | 0.30 | 0.18 | 0.90 | 0.27 | 0.46 | 0.66 |
| loop | 9.0 | 4.7 | 60.1 | 28.5 | 6.2 | 1.5 | 11.2 | 2.2 | 6.8 | 10.5 |
| sort | 4.4 | 1.1 | 19.7 | 10.2 | 1.9 | 1.2 | 4.0 | 1.6 | 2.3 | 3.1 |
| strings | 1.8 | 0.57 | 1.9 | 1.2 | 0.93 | 0.97 | 1.9 | 2.0 | 1.4 | 1.4 |
| trees | 10.7 | 2.9 | 39.1 | 22.3 | 4.5 | 1.2 | 6.7 | 5.5 | 5.8 | 5.7 |
| nbody | 17.8 | 10.4 | 134 | 67.0 | 12.8 | 4.3 | 26.5 | 12.4 | 13.3 | 19.3 |

(Go, Java, C#, C++ and Rust are built first and the built program is
timed; for C# that is `dotnet run --no-build`, about half a second of
each figure being `dotnet` starting.)

TypeScript is not in the table because it is the `js` column: Node
takes the types off and runs the same JavaScript. PHP has no JIT as it
is installed, and is slower than the VM and faster than Python, except
for strings, where PHP's own text functions do the work. The table is
the record of 0.5.0 (`bench/results/versions/0.5.0.json`, made by
`python3 bench/record.py`).

Lua, Ruby, Dart and Kotlin came after 0.5.0 and are not in that
record. Timed once on the same machine (`python3 bench/run.py
--engines c,vm,to-lua,to-ruby,to-dart,to-kotlin`, 2026-11-17):

| | `zeno run --vm` | `lua` (Lua 5.4) | `ruby` (Ruby 3.3) | `dart` (Dart 3.14) | `kotlin` (Kotlin 2.4, JDK 21) |
|---|---|---|---|---|---|
| fib | 0.74 | 2.3 | 5.2 | 0.60 | 0.31 |
| loop | 8.1 | 45.8 | 57.0 | 1.1 | 1.5 |
| sort | 3.5 | 17.3 | 16.9 | 1.4 | 1.3 |
| strings | 1.6 | 3.7 | 3.3 | 1.4 | 0.83 |
| trees | 9.9 | 48.9 | 44.1 | 2.5 | 1.4 |
| nbody | 17.6 | 91.7 | 126.9 | 9.4 | 4.7 |

Dart and Kotlin are compiled to machine code as they run, as
JavaScript and Java are, and are two to seven times faster than the
VM. Lua and Ruby are interpreted, and every `+` is a call of the
runtime: they are about as slow as Python. (The Kotlin program is
compiled first and the jar is timed.)

JavaScript is two to six times faster than the VM and Java two to
nine, because Node and the JVM compile the program to machine code as
it runs and learn which kinds of value each `Z.add` really sees. Go is
one and a half to three times faster: it is compiled before it runs,
but every value is an `any` and every `+` a call that looks at both
sides. Python is
slower: every `+` is a call of `Z.add`, which checks what Zeno checks.
Where the checker knows both sides are ints, a plain `+` (with the
overflow check) would do; that is the next step for speed.

## Things to try

1. Build `examples/hello.zeno` for `js` and `py` and read both files
   from the bottom up. Find `main`. How much of each file is the
   runtime, and how much the program?
2. Write a program with a function inside a loop that uses the loop's
   variable, as above. Build it for Python and find the `Z.Cell`. Take
   the cell out by hand and run it: what does it print now, and why?
3. `0.1 + 0.2` prints `0.30000000000000004` in Zeno, JavaScript and
   Python, but `1e16` is `1e16` in Zeno and `10000000000000000` in
   JavaScript. Find `showFloat` in `src/runtime.js` and say how it
   makes JavaScript print it Zeno's way.
4. Add a program of your own to `tools/targets/` that tries something
   the others do not, and run `python3 tools/targets.py js py --only
   tools/`. If a target prints something else, you have found a bug:
   find which runtime function is wrong.
5. Make a `counter.html` with a heading and a `<div id="zeno-output">`,
   and a `counter.zeno` that prints a few lines. Build it with
   `--target html` from another folder and open it.
6. The page refuses `sleep`. What would it take to allow it? (A page
   runs one thing at a time; look at how `web.after` works in
   `docs/web.md`.)
7. Build `examples/hello.zeno` for `go`, `java` and `cs`, and find the
   line of the program in each. Count the lines above and below it:
   which language needs the most words to say the same thing?
8. Take out one of the `_ = x` lines in a Go file and run it with
   `go run`. What does Go say? Then take the `if (true)` off a
   `return` in the Java file, and the `#pragma` line off the C# file,
   and compare what each language thinks of a line after `return`.
9. Build the loop above for `ruby` and find `Z.loop`. Then make the
   function inside it use `i` and not a variable of the body. Is the
   loop still a block? Why does it not need to be?
10. Build a program with a `try` inside a `for` that has a `continue`
   in it, for `lua`. Find the `goto`, and find how the `continue`
   gets out of the function `Z.try` was given.
11. Write `function f(n: int) -> int { return f(n + 1) }` and call it.
   Build it for `go`, `java` and `cs`. Do they all stop with the same
   message as `zeno run`? Find `1000` in `src/runtime.go`.
