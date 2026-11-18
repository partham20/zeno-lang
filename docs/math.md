# The standard library, part 3: more of `math`

`math` had `pi`, a square root, `floor`, `ceil`, `sin` and `cos`. This
page adds the rest of what firmware arithmetic reaches for, and joins
the module to something Zeno already had: numbers that carry units.

```sh
cargo run -- run examples/instruments.zeno
```

```
compass
  x   1.0  y   0.0  heading    0.0 deg
  x   1.0  y   1.0  heading   45.0 deg
  x   0.0  y   1.0  heading   90.0 deg
  x  -1.0  y   1.0  heading  135.0 deg
  x  -1.0  y   0.0  heading  180.0 deg
  x   0.0  y  -1.0  heading  270.0 deg
  x   3.0  y  -4.0  heading  306.9 deg
thermistor
     32650 ohm     0.6 degC
     10000 ohm    25.0 degC
      3603 ohm    49.9 degC
      1070 ohm    85.5 degC
level meter
   1.0 V     0.0 dB
   0.5 V    -6.0 dB
   0.1 V   -20.0 dB
   2.0 V     6.0 dB
capacitor, tau 0.5 s
  after 0.0 s    0.0 %
  after 0.5 s   63.2 %
  after 1.0 s   86.5 %
  after 1.5 s   95.0 %
  after 2.0 s   98.2 %
  after 2.5 s   99.3 %
sine table
  [128, 176, 218, 245, 255, 245, 218, 176, 128, 79, 37, 10, 0, 10, 37, 79]
```

[`examples/instruments.zeno`](../examples/instruments.zeno) is five
small instruments: a compass (`atan2`), a thermistor (`log`), a level
meter (`log10`), a charging capacitor (`exp`) and a sine table (`sin`
and `round`). The decisions are 6.21 to 6.28 in
[`decisions.md`](decisions.md).

## What `math` holds now

| Written | Takes | Gives |
|---|---|---|
| `math.pi`, `math.e` | | `float` |
| `math.sqrt(x)` | a number, 0 or more | `float` |
| `math.floor(x)`, `math.ceil(x)`, `math.round(x)` | a number | `int` |
| `math.sin(a)`, `math.cos(a)`, `math.tan(a)` | an angle: a number (radians), `deg` or `rad` | `float` |
| `math.asin(x)`, `math.acos(x)` | a number from -1 to 1 | `rad` |
| `math.atan(x)` | a number | `rad` |
| `math.atan2(y, x)` | two numbers | `rad` |
| `math.exp(x)` | a number | `float` |
| `math.log(x)`, `math.log2(x)`, `math.log10(x)` | a number above 0 | `float` |

`log` is the natural logarithm, the one with base e, as in C and
Python. The other two have their base in their name.

What is not here, and what to write instead (6.21):

| Elsewhere | In Zeno |
|---|---|
| `pow(x, y)` | `x ** y` |
| `trunc(x)` | `int(x)` |
| `fabs(x)` | `abs(x)` |
| `degrees(x)`, `radians(x)` | `angle.deg`, `angle.rad` |
| `round(x, 2)` | `f"{x:.2f}"` |

## Angles

Zeno numbers can carry units (`500 ms`, `3.3 V`), and two of the units
are for angles: `deg` and `rad`. Until now `math` ignored them.

### An angle goes in

```zeno
math.sin(30 deg)       # 0.49999999999999994
math.sin(0.5 rad)      # 0.479425538604203
math.sin(0.5)          # the same: a plain number is radians
```

Degrees handed to a function that counts in radians is the classic
mistake here, and in C it gives a wrong number with no complaint. A
value that says `deg` cannot be misread.

A plain number is still taken, and means radians, because that is what
the sine of a number means in mathematics, and signal code is full of
`math.sin(2 * math.pi * f * t)` (6.23).

`math.sin(30 deg)` is not exactly 0.5 because a float cannot hold
pi / 6 exactly. Python prints the same digits.

### An angle comes out

```zeno
let heading = math.atan2(1, -1)
print(heading)              # 2.356194490192345 rad
print(heading.deg)          # 135 deg
```

`asin`, `acos`, `atan` and `atan2` answer with a quantity in `rad`, so
the value says what it is. Comparing it with a bare number, or with
degrees, is refused before the program runs:

```
error: `+` does not work between a quantity in rad and a quantity in deg. Change one of them first, with `.rad` or `.deg`
```

To use an angle as a plain number, divide by one of its unit, as with
any quantity: `heading / 1 rad` is `2.356194490192345`, and
`heading.deg / 1 deg` is `135.0`.

`atan2(y, x)` takes y first. It is the angle of the point (x, y) seen
from (0, 0), and unlike `atan(y / x)` it knows which side of the
circle the point is on:

| Point | `atan2(y, x).deg` |
|---|---|
| (1, 1) | `45 deg` |
| (-1, 1) | `135 deg` |
| (-1, -1) | `-135 deg` |
| (1, -1) | `-45 deg` |

### `.rad` and `.deg`

`(3 s).ms` has always changed a time into milliseconds. Degrees and
radians were the one pair of units for the same thing that could not
be changed, because the factor between them is not a whole number.
Now they can, when the program asks (6.22):

```zeno
print((180 deg).rad)                 # 3.141592653589793 rad
print((math.pi * 1 rad).deg)         # 180 deg
```

They still do not meet by themselves. `1 s + 500 ms` is `1500 ms`,
because the finer unit loses nothing. Between degrees and radians
neither is finer, so `90 deg + 1 rad` is an error and not a guess.

The whole of the change is in `src/units.rs`. `convert` answers "how
do I turn this unit into that one", and has two new lines:

```rust
match (from, to) {
    ("deg", "rad") => return Some(Scale { times: std::f64::consts::PI / 180.0, over: 1.0 }),
    ("rad", "deg") => return Some(Scale { times: 180.0 / std::f64::consts::PI, over: 1.0 }),
    _ => {}
}
```

`common` answers "what unit do these two meet in", and now refuses two
units of different kinds before it asks `convert`:

```rust
if a_kind != b_kind {
    return None;
}
```

Every engine asks these two functions, so `.rad` works in `zeno run`,
the VM, and both compilers with no change to any of them.

## How an angle compiles

A quantity is a `double` in a compiled program. The unit is known to
the compiler and is gone by the time the program runs. So the change
of unit is one multiplication by a number written into the program:

```zeno
function height(angle: deg, length: float) -> float {
    return length * math.sin(angle)
}

function bearing(x: float, y: float) -> deg {
    return math.atan2(y, x).deg
}
```

```c
static double z_height(double z_angle, double z_length) {
    return (z_length * sin((z_angle * 0.017453292519943295)));
}

static double z_bearing(double z_x, double z_y) {
    return (atan2(z_y, z_x) * 57.29577951308232);
}
```

```llvm
define internal double @z_height(double %arg.angle, double %arg.length) {
entry:
  %t1 = fmul double %arg.angle, 0.017453292519943295
  %t2 = call double @sin(double %t1)
  %t3 = fmul double %arg.length, %t2
  ret double %t3
}
```

`0.017453292519943295` is pi / 180 and `57.29577951308232` is
180 / pi, each printed with every digit a float holds, so the C
compiler reads back the very same number.

Had `height` been given `angle: rad`, or a plain float, there would be
no multiplication at all.

## `round`

```zeno
print(math.round(2.5), math.round(-2.5), math.round(2.4))    # 3 -3 2
```

A half goes away from zero, as taught at school (6.25). Python sends a
half to the nearest even number, so its `round(2.5)` is 2.

The answer is an `int`, like `floor` and `ceil`. In C:

```c
zeno_whole(round(2.5))
```

`round` is the C library's and gives a float. `zeno_whole` makes it an
int, and gives the largest or smallest int for a float that is beyond
them, where a plain C cast would be undefined.

## Logarithms and `exp`

```zeno
print(math.log(math.e), math.log2(8), math.log10(1000))    # 1.0 3.0 3.0
print(math.exp(1))                                         # 2.718281828459045
```

There are three functions and not `log(x, base)` because the C library
works out base 2 and base 10 directly, and dividing two natural
logarithms does not always land on the whole number:

```zeno
print(math.log(125) / math.log(5))     # 3.0000000000000004
```

## A number with no answer

```
error: uncaught ValueError: cannot take the logarithm of 0.0
error: uncaught ValueError: `asin` needs a number from -1 to 1, and this is 1.5
```

C would give `nan`, which then travels through every sum it touches.
Zeno stops at the line that asked (6.27). In the runtime each of these
is a test in front of the C library's function:

```c
static double zeno_log(double x, int base, int line, int col) {
    if (x <= 0.0) {
        zeno_fail("ValueError", zeno_join(2, "cannot take the logarithm of ", zeno_str_float(x)), line, col);
    }
    return base == 2 ? log2(x) : base == 10 ? log10(x) : log(x);
}
```

With `zeno run` and in an ordinary build the error can be caught with
`try`. In a firmware build it is a fault, like a bad index, and stops
the program: test the number first.

An answer that is too big is `inf`, as it is for `*`: `math.exp(710)`
prints `inf`.

## The files

| File | What part 3 added |
|---|---|
| `src/units.rs` | `deg` and `rad` convert when asked, and never meet by themselves |
| `src/builtins.rs` | The functions for `zeno run` and the VM; `angle` reads a number or an angle |
| `src/checker.rs` | Their types: an angle in, `rad` out; the hint for `rad + deg` |
| `src/runtime.c` | `zeno_log`, `zeno_asin`, `zeno_acos`: the tests in front of the C library |
| `src/emit_c.rs`, `src/emit_llvm.rs` | `math()`: one call each, with the angle scaled first |

## How it is tested

- `more_of_math_is_the_same_in_every_build` (emit_c.rs) runs every
  function, with plain numbers and with angles, and `instruments.zeno`,
  through the interpreter, the VM, and C and LLVM in three builds each. Every digit has to agree. The answers
  are the C library's in every engine (6.28), which is why they can.
- The same test stops four programs on a number with no answer, and
  compares the message, line and column in every build.
- `more_of_math` (interpreter.rs) pins down the values that are exact:
  the four quarters of `atan2`, `round` on halves, `log2(8)`.
- `more_of_math_is_checked` (checker.rs) has each message.
- `an_angle_converts_when_asked_and_never_by_itself` (units.rs).

There are 377 tests in all.

## What it does not do

- A quantity could not take a format when this page was written, and
  `examples/instruments.zeno` wrote `f"{heading / 1 deg:6.1f} deg"`.
  It can now: `f"{heading:10.1f}"` prints the same ten characters,
  number and unit (decision 10.29).
- `atan2` takes plain numbers, not two lengths or two voltages (6.24).
- `hypot`, the hyperbolic functions, `gcd`, `clamp` and random numbers
  came later, written in Zeno itself:
  [`library-in-zeno.md`](library-in-zeno.md).

## Things to try

1. In `examples/instruments.zeno`, take `.deg` off the first line of
   `heading`. Read the message. Which decision explains why Zeno does
   not simply add 360 degrees to the radians?

2. `math.sin(30 deg)` prints `0.49999999999999994`. Print
   `math.round(1000 * math.sin(30 deg))`. Then find where the
   multiplication by pi / 180 happens for `zeno run` (`angle` in
   `builtins.rs`) and for a built program (the C above).

3. Print `math.round(x)` for 2.5, 3.5, -2.5 and 0.49999999999999994.
   What would Python print for each? The last one is why `round` is
   not written `floor(x + 0.5)`: try that too.

4. This says how many bits a number needs:

   ```zeno
   function bits(n: int) -> int {
       return math.floor(math.log2(n)) + 1
   }
   ```

   It is right for 255 and 256. Try `2 ** 48 - 1` and `2 ** 49 - 1`.
   What happened, and how would you write `bits` with whole numbers
   only?

5. Put `math.log(x)` inside `try` / `except ValueError` and build with
   `--firmware`. Read the message, then rewrite it the way the message
   asks.

6. In `src/units.rs`, take the test of `a_kind != b_kind` out of
   `common` and print `90 deg + 1 rad` and `1 rad + 90 deg`. Why is
   that worse than an error? Put it back and run `cargo test`.

7. `math.hypot(x, y)` is not in `builtins.rs` at all: it is written
   in Zeno, in `library/math.zeno`
   ([`library-in-zeno.md`](library-in-zeno.md)). Add `math.lcm(a, b)`
   there, the least whole number that both divide, from `gcd`. Which
   of `builtins.rs`, `checker.rs`, `emit_c.rs` and `emit_llvm.rs` did
   you have to open? Which one test told you what you forgot?
