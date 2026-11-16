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

$ zeno build --target html examples/hello.zeno
Wrote hello.html. Open it in a browser
```

| `--target` | Writes | Run it with |
|---|---|---|
| `js` | `hello.js` | `node hello.js` |
| `html` | `hello.html`, a page that runs the program | a browser |
| `py` | `hello.py` | `python3 hello.py` |

More languages follow (Go, Java, C#, C++, Rust, and WebAssembly); this
page grows with each.

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
example, every sample of the book and four programs written to try the
corners (`tools/targets/`) with `zeno run`, then builds each for every
target, runs it, and compares:

```
$ python3 tools/targets.py
js: 319 of 319 programs print what `zeno run` prints (21 left out: they use the board, a page, manual memory)
py: 319 of 319 programs print what `zeno run` prints (21 left out: they use the board, a page, manual memory)
html: 311 of 311 programs print what `zeno run` prints (29 left out: ...)
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
2. **The runtime** (`src/runtime.js`, `src/runtime.py`): what every Zeno
   value is in that language, and what each operator and built-in does
   with it, written to do what `ops.rs` and `builtins.rs` do for
   `zeno run`, step for step and message for message. It goes at the
   top of the file, so the file needs nothing else.
3. **Printing** (`src/emit_js.rs`, `src/emit_py.rs`): the lowered
   program in that language's words.

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
  Z.b.print([area(Z.float(2.0))], 6);
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
    Z.b.print([area(2.0)], 6)
```

`Z.mul(a, b, 4)` multiplies as Zeno multiplies, and if the answer does
not fit, stops with Zeno's message for line 4. `area` is called as a
plain function, since it is known where it is written; `math` is the
module, renamed `math_` because `math` is a name the targets' own libraries
use.
`area` is moved above the `import` line, as Zeno lets a function be
used before the line it is written on. A function is a real
function of the language, with its Zeno name and parameters kept on it
so that `area(r: 2.0)` can be lined up.

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

Zeno refuses such a program with a message that names the line.

## Speed

The written program is as correct as `zeno run` first, and as fast as
that allows. Seconds, on the benchmarks in `bench/`, against `zeno run
--vm`:

| | `zeno run --vm` | `--target js` (Node 22) | `--target py` (Python 3.13) |
|---|---|---|---|
| fib | 0.81 | 0.12 | 2.3 |
| sort | 3.8 | 1.2 | 18.1 |
| strings | 2.0 | 0.6 | 1.6 |
| trees | 10.7 | 2.2 | 28.3 |

JavaScript is three to seven times faster than the VM, because Node
compiles it to machine code as it runs. Python is slower: every `+` is
a call of `Z.add`, which checks what Zeno checks. Where the checker
knows both sides are ints and the sum cannot overflow, a plain `+`
would do; that is the next step for speed (`docs/gaps.md`).

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
