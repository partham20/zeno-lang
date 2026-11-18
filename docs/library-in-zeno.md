# A library written in Zeno: `math.hypot`, and the module `random`

Until now everything that comes with Zeno was written in Rust, and
again in C, and again for each language `zeno build --target` writes.
This page adds a second kind of library: a file of Zeno, kept inside
the `zeno` program, that every engine runs as it runs your own files.

```zeno
import math
import random

let reading = 512 + random.between(-40, 40)
print(math.clamp(reading, 0, 1023), math.hypot(3.0, 4.0), math.gcd(48, 18))
```

```
540 5.0 6
```

It prints that on every run, and in every engine. Why the same on
every run is at the end.

The decisions are chapter 33 of [`decisions.md`](decisions.md).

## The files

| File | What it holds |
|---|---|
| `library/math.zeno` | `hypot`, `sinh`, `cosh`, `tanh`, `gcd`, `clamp` |
| `library/random.zeno` | `seed`, `number`, `between`, `pick` |
| `src/library.rs` | The list of those files, and their text, kept inside `zeno` |
| `src/modules.rs` | Reads a part when a program asks, and joins in what the program reaches |
| `tools/targets/library.zeno` | A program that tries all of both, run by every engine, build and target |

Nothing else changed. There is no `hypot` in `builtins.rs`, in
`checker.rs`, in `runtime.c`, in either compiler or in any target.

## What one function used to cost

`math.log2` is in fifteen places: `builtins.rs` for `zeno run` and
the VM, `runtime.c` for both compilers, and thirteen runtimes for the
other languages. The checker has a line for it, and so has each
compiler. Each of those is small. Together they are why
[`math.md`](math.md) left `hypot` out "until something needs it".

Here is all of `gcd`:

```zeno
function gcd(a: int, b: int) -> int {
    var big = a
    var small = b
    if big < 0 {
        big = -big
    }
    if small < 0 {
        small = -small
    }
    while small != 0 {
        let rest = big % small
        big = small
        small = rest
    }
    return big
}
```

It is checked by the checker, because it is Zeno. It overflows the
way Zeno overflows. It is compiled by `--firmware`, `--no-collector`,
`--llvm`, `--minimal` and `--target lua`, because they compile Zeno.
And it gives one answer everywhere, where the `hypot`s of fifteen
libraries agree only nearly.

## A part is a file of your program

[`modules.md`](modules.md) describes how `import shapes` works:
`shapes.zeno` is read, every name it declares at its top gets
`shapes.` in front, and its statements are put ahead of yours. One
program comes out, and no stage after that knows there were two
files.

A part of the library is treated the same way. Ask Zeno to show you:

```sh
zeno join gcd.zeno
```

```
import math
function math.gcd(a: int, b: int) -> int
    var big = a
    ...
    return big
print(math.gcd(48, 18))
```

That is the program the checker and every engine are given. The name
`math.gcd` is one word to them, as `shapes.area` is.

`src/library.rs` is the list of parts:

```rust
Part {
    module: "math",
    path: "library/math.zeno",
    source: include_str!("../library/math.zeno"),
    names: &["hypot", "sinh", "cosh", "tanh", "gcd", "clamp"],
    built_in_too: true,
},
```

`include_str!` puts the text of the file into `zeno` when `zeno` is
built, so there is nothing to install beside it. `built_in_too` says
that `math` also has `sqrt` and the rest, which the checker knows:
a name the part does not declare is left alone for it.

## Only what you use

A program for a board pays for every function in it. So of a part's
functions, the program gets the ones it reaches, and no others:

```zeno
import math
print(math.tanh(1.0))
```

`zeno join` shows four functions: `tanh`, the `sinh` and `cosh` it is
made of, and `size`, which both of those use and which is `private`
to the file. `hypot`, `gcd` and `clamp` are not there.

The stage that gives names their full names visits every use of
every name. For a part it now writes down, for each function, the
names that function used. `Loader::join` starts from the names your
own files used and follows the list:

```rust
while let Some(name) = named.pop() {
    ...
    for names in by_function.get(&name).into_iter().chain(rest) {
        named.extend(names.used.iter().cloned());
    }
}
```

A function that was never reached is dropped before the checker sees
it. What is not a function, such as the four numbers `random` keeps,
comes whole with the first thing of the part that is reached.

## A program that uses none of it is unchanged

`names` in the list above has a second use. A file that does not
spell `hypot`, `sinh`, `cosh`, `tanh`, `gcd` or `clamp` anywhere
cannot be using them, and then the part is not even read. A file
that spells one and means something else (`let clamp = 3`) has the
part read, found to be of no use, and taken away again.

So every program from before this page is joined into exactly the
program it was. The parts are also the last files to be read, which
keeps the numbers of your own files, and so every place a message
can point at, what they would have been.

## A part keeps to itself

A program may have an `abs` of its own. `abs` is a name, and not a
word of the language. Joined to such a program, a part that said
`abs(x)` would call the program's. So the part of `math` does not
say it:

```zeno
private function size(x: float) -> float {
    if x < 0.0 {
        return -x
    }
    return x
}
```

`random.pick` cannot avoid `len`. A program that has its own `len`
and uses `pick` is told, at its `len`:

```
error: this program uses `random.pick`, which is written in Zeno and uses the `len`
that comes with Zeno. So the program cannot give `len` a meaning of its own here.
Give this one another name
```

## The functions

`hypot` is the root of `x * x + y * y`, which is exact where the
answer is: `hypot(3.0, 4.0)` is `5.0`. For sides near 1e200 the
squares do not fit in a float, so both sides are first multiplied by
2**-600. Multiplying a float by a power of two changes none of its
digits, so the answer is the one the plain sum would have given.

`sinh` is `(e**x - e**-x) / 2`, except near zero, where the two
powers are nearly equal and the subtraction would keep few digits.
There it adds up the first terms of `x + x**3/3! + x**5/5! + ...`.

`clamp` is written once, with a type parameter, for anything `<`
compares:

```zeno
print(math.clamp(300, 0, 255), math.clamp(6.2 V, 0.0 V, 5.0 V))
```

```
255 5 V
```

## `random`

```zeno
import random

print(random.between(1, 6))               # a die: 1 and 6 both come up
print(random.number())                    # from 0.0 up to 1.0, never 1.0
print(random.pick(["red", "amber", "green"]))
random.seed(2026)                         # start again from a number
```

The numbers come from four whole numbers that are stirred each time
one is asked for, with additions, shifts and one `^`:

```zeno
let sum = (a + b + d) & 4294967295
d = (d + 1) & 4294967295
a = b ^ (b >> 9)
b = (c + (c << 3)) & 4294967295
c = ((c << 21) | (c >> 11)) & 4294967295
c = (c + sum) & 4294967295
return sum
```

There is no multiplication and no division in it, so it is quick on
the smallest processor. It is good for a game, a test or a
simulation. It is **not** for keys or passwords.

**A program that does not seed gets the same numbers every time.**
A test that fails must fail again. A board has nothing to make a
first number from. And it is one rule for every engine. A game that
wants a different game each time calls `random.seed` with something
that differs, and Zeno has nothing to offer for that yet: the time of
day is on the list in [`missing.md`](missing.md).

`between` draws bits under a mask that just covers the span, and
throws a draw away if it is above the span. `bits % 6` would be
shorter, and would make 1 to 4 come up a little more often than 5
and 6, because 4294967296 is not a multiple of 6.

## Two mistakes this found

- Two files of one program could not both say `import math`. Joined,
  the program said it twice, and the checker refused the second. The
  part of `math` imports `math`, so this had to be mended first.
- `let f = shapes.area`, a function of an imported file used as a
  value, ran under `zeno run` and did not build: the C had a dot in a
  name. `[1.0, 2.0].map(math.sinh)` found it.

## The tests

- `modules::tests`: the eight from
  `a_part_of_the_library_is_joined_like_a_file_of_the_program` to
  `two_files_can_import_one_module_that_comes_with_zeno`. One of
  them, `the_parts_are_what_library_rs_says_they_are`, holds the list
  in `library.rs` to the files.
- `emit_c::tests::the_library_written_in_zeno_is_the_same_in_every_build`
  runs `tools/targets/library.zeno` in the interpreter, the VM, and
  the C and LLVM builds, with and without a collector.
- `python3 tools/targets.py --only library` holds every target
  language to what `zeno run` prints for that program.

## Things to try

1. Write a program that calls `math.cosh`, and run `zeno join` on it.
   Which functions came with it? Add a call of `math.clamp` and look
   again.
2. Add `math.lcm(a, b)`, the least whole number that both divide, to
   `library/math.zeno`. Run `cargo test modules`. One test fails and
   tells you the one other place to change. Then build a program
   that uses it with `--target py` and read the Python.
3. Add `import math` to a second file of a program that already
   imports it in the first, and run it with the `zeno` of version
   0.5.1 and with this one.
4. Write `function size(x: int) -> int` in your own program and call
   `math.hypot`. It works. Now write your own `len` and call
   `random.pick`. Read the message. Why can the first be allowed and
   not the second?
5. Throw a die 6,000 times with `random.between(1, 6)` and count each
   face. Then make your own `between` with `%` in place of the mask,
   for a die of 3,000,000,000 faces, and count how many throws land
   in the lower half.
6. Run the program at the top of this page twice. Then give it
   `random.seed(1)` as its first line. Find the paragraph of decision
   33.8 that says why the first two runs agree, and say what a game
   would have to do.
7. `random.number()` draws twice. Make one that draws once
   (`bits() / 4294967296.0`) and print a few. How many different
   floats can it give, and how many can the one with two draws?
8. `math.hypot(3, 4)` is refused and `math.sqrt(4)` is not. Read
   decision 33.7, then find in `checker.rs` what lets `sqrt` take a
   whole number.
