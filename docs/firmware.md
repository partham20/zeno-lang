# How a firmware build handles errors

`zeno build --firmware` compiles the same Zeno program to different C. In
this first version of it, one thing changes: **how an error travels**.

```sh
cargo run -- build examples/sensor.zeno              # errors jump
cargo run -- build --firmware examples/sensor.zeno   # errors are values
```

Both programs print the same thing. Open `sensor.c` after each and search
for `setjmp`: the first has it, the second does not.

## Why a second way

An ordinary build compiles `try` with C's `setjmp` and `longjmp`
([`compiler.md`](compiler.md), "Errors"). A `raise` jumps straight back to
the `try`, out of however many functions are in between. That is cheap and
short, and it is exactly what coding standards for safety-critical
firmware do not allow. MISRA C:2012, Rule 21.4, bans `<setjmp.h>`:

- the jump skips whatever the functions in between still meant to do,
- a local variable changed since the `setjmp` can be left with no defined
  value,
- tools that work out the deepest the stack can get, or prove that every
  path has been tested, cannot follow a jump they cannot see.

So firmware held to such a standard needs errors it *can* see: ordinary
return values, tested by ordinary `if`s.

## The files

- [`src/emit_c.rs`](../src/emit_c.rs), the part headed "Errors as values
  (firmware)": `failed`, `checked_call` and `try_with_values`. About 150
  lines.
- [`src/runtime.c`](../src/runtime.c): the parts between
  `#ifdef ZENO_FIRMWARE` and `#endif`. The compiler keeps one side or the
  other when it pastes the runtime in (`runtime` in `emit_c.rs`), so a
  firmware build's C file does not contain the word `setjmp` at all.

## A function that can raise

```
function parse_line(line: str) -> float or none raises ValueError {
    let text = line.strip()
    if text == "--" {
        return none
    }
    let value = float(text)
    if not -40.0 <= value <= 125.0 {
        raise ValueError(f"{value} is outside what the sensor can measure")
    }
    return value
}
```

In a firmware build it becomes:

```c
static bool z_parse_line(const char *z_line, zeno_opt_float *zeno_result) {
    const char *z_text = zeno_strip(z_line);
    if ((strcmp(z_text, "--") == 0)) {
        *zeno_result = (zeno_opt_float){false, 0.0};
        return true;
    }
    double t16;
    if (!zeno_parse_float(z_text, 19, 17, &t16)) return false;
    double z_value = t16;
    if (!(((-40.0) <= z_value) && (z_value <= 125.0))) {
        zeno_note(zeno_error_new("ValueError", ...), 21, 9);
        return false;
    }
    *zeno_result = (zeno_opt_float){true, z_value};
    return true;
}
```

Three things changed.

1. **What it returns.** `true` means it worked, `false` means it raised.
   The real answer goes out through one more parameter, `zeno_result`.
2. **`raise`** notes the error in one variable of the runtime
   (`zeno_raised`, with the line and column) and returns `false`.
3. **A call to something that can raise** (`float(text)` here) is followed
   at once by a test. If the callee failed, so does this function. The
   error is already noted, so there is nothing to copy.

Which functions get this treatment? The ones that say `raises`. The
checker already insists that they say so (strict mode, which `zeno build`
always uses). A function that cannot raise is compiled exactly as before,
with no tests after calls to it. In `sensor.c`, `in_range` and `counter`
are untouched.

## try and except

```
try {
    readings.append(parse_line(line))
} except ValueError as err {
    faults += 1
    ...
}
```

```c
{
    zeno_opt_float t19;
    if (!z_parse_line(z_line, &t19)) goto zeno_except_2;
    zeno_list_append(z2_readings, (zeno_slot){.p = zeno_box_float(t19)});
}
zeno_except_2: ;
if (zeno_raised != NULL) {                   // did the body fail?
    zeno_error *zeno_caught_2 = zeno_raised;
    zeno_raised = NULL;                      // it is being dealt with
    if (zeno_is(zeno_caught_2, "ValueError")) {
        zeno_error *z_err = zeno_caught_2;
        ...the except block...
    } else {
        zeno_raised = zeno_caught_2;         // not ours
        zeno_uncaught();
    }
}
```

Inside a `try`, a failed call does not return `false`. It goes forward to
the `except` blocks of that `try`. This is the one use of `goto` in the
generated C, and it only ever goes **forward, to a label in a block around
it**. That is the form MISRA's required rules (15.2, 15.3) allow, though
its advisory Rule 15.1 would rather there were no `goto` at all.

After the label, the body either finished (`zeno_raised` is still `NULL`,
and the whole `if` is skipped) or failed. Each `except` is one arm of an
`if`. The last `else` is for an error no `except` here wants. What goes
there depends on where the `try` is, and it is the same choice as for any
failed call (`failed` in `emit_c.rs`):

| Where | What a failure does |
|---|---|
| Inside a `try` in this function | `goto` its `except` blocks |
| In a function that says `raises` | `return false;` |
| At the top of the file | `zeno_uncaught();` which prints the error and stops |

In the example, every kind the body can raise has an `except`, so the last
`else` cannot happen. It is written anyway: the C compiler cannot know
that, and Rule 15.7 wants every `else if` chain to end in an `else`.

Because nothing is skipped, the awkward parts of the jumping version are
gone. `return`, `break` and `continue` inside a `try` need no care, and no
variable has to be `volatile`.

It also means a failure passes every line where something could be freed.
If the block that fails holds owned values, the failure frees them first:
`{ zeno_drop_Frame(z_frame); return false; }`.
[`docs/ownership.md`](ownership.md) has the details.

## Errors that stop the program instead

Zeno has two sorts of error that can be caught under `zeno run`:

- ones **the program raises itself**: `raise ValueError(...)`, raising an
  object, and `int(text)` / `float(text)` on text that is not a number.
  A function that can let one out must say so with `raises`.
- ones **the runtime finds**: a division by zero, an index past the end,
  an int that overflows, `pop` on an empty list. These can happen on
  almost any line, so no function has to declare them.

A firmware build carries the first sort as values, as shown above. The
second sort it treats as a **fault**: the program stops with the same
message `zeno run` gives for an uncaught error, and no `except` sees it.

The reason is cost. To carry a division by zero as a value, every
function would have to be one that can fail, and nearly every line would
need a test after it. The firmware answer is to test first:

```
if divisor != 0 {
    rate = total // divisor
}
```

(On a real board a fault would call the board's fault handler. Today it
prints and exits, as on a PC.)

The compiler does not leave you to find this out when the board resets:

```
error: nothing in this `try` can be caught. In a firmware build, an error
the program raises itself (and `int` or `float` of text) travels as a value
and can be caught. A division by zero, a bad index, a key that is not in a
dict or an overflow stops the program. Test for it before it happens, or
build without `--firmware`
```

That is what `try { print(1 // n) } except ZeroDivisionError { ... }` gets
in a firmware build. An `except` that names a built-in kind which nothing
in the body raises as a value is refused the same way.

So is the fault moved one function away, which used to get through:

```zeno
function half(a: int, b: int) -> int raises ZeroDivisionError { return a // b }

try { print(half(1, 0)) } except ZeroDivisionError { print("no") }
```

`zeno run` prints `no`. A firmware build compiled this, and then stopped
at the division. Now it says so before anything is built:

```
error: `half` says `raises ZeroDivisionError`, but nothing in it raises
one itself. In a firmware build a division by zero stops the program
where it happens, and cannot be caught by a caller. Test for it before it
happens and take `raises ZeroDivisionError` off, or build without
`--firmware`
```

A function that raises the error itself (`if b == 0 { raise
ZeroDivisionError("nothing to halve") }`) is fine: that one travels as a
value ([`audit.md`](audit.md)).

One case is not refused. `except` with no kind (or `except Error`) around
a body that *does* raise something as a value is accepted, and catches
those. A fault in the same body still stops the program. This is the one
place where a firmware build and an ordinary build of the same source can
behave differently, and there is a test in `emit_c.rs` that pins it down.

## What a firmware build does not accept yet

- **A function that can raise, used as a value.** `texts.map(parse)` where
  `parse` says `raises`. A firmware build calls a function that can raise
  differently from one that cannot, and the type `function(str) -> int`
  does not say which it is. Until function types can say `raises`, only
  functions that cannot raise may be passed around. Call the others by
  name.

## What this does not claim

Removing `setjmp` removes one obstacle to MISRA, not all of them. The C a
firmware build writes today still:

- takes memory from the heap (`malloc` is banned by Rule 21.3).
  `zeno build --no-collector` answers this one: it leaves the garbage
  collector out, frees every block at a known line, and takes its
  blocks from a fixed pool in place of the heap
  ([`no-collector.md`](no-collector.md), [`pool.md`](pool.md)),
- uses variable argument lists in the runtime (`zeno_print`, `zeno_join`:
  Rule 17.1),
- uses `printf` and `exit` (Rules 21.6 and 21.8). A board that brings
  its own printing takes both away (`ZENO_BOARD_PRINTS`,
  [`board.md`](board.md)), and a float is made into text with no
  `snprintf` (decision 53),
- asks GCC and Clang for their built-ins for checked arithmetic (a
  language extension, Rule 1.2). Built with `-DZENO_PLAIN_C`, or by a
  compiler that is neither, the checks are plain C
  ([`plain-c.md`](plain-c.md)),
- uses the forward `goto` described above (advisory Rule 15.1).

None of these is hidden. Each is a later piece of work, and
`decisions.md` 5.5l keeps the list.

## The same, through LLVM

`zeno build --llvm --firmware` writes the same design in LLVM IR in
place of C: a function that can raise gives back an `i1` and puts its
answer where a pointer says, each call is followed by a `br` on what
came back, and a `try` is a label that failures branch to. The runtime
file beside the IR is the firmware one, so neither file contains
`setjmp`. It refuses what this build refuses, in the same words.
[`llvm.md`](llvm.md), "Errors as values", walks through the IR.

`--no-collector` goes with `--llvm` too: [`llvm.md`](llvm.md), "With no
collector".

## How it is tested

Every test that compares a compiled program with `zeno run` now builds it
**twice**, once each way, and both must agree with the interpreter. A
program the firmware build refuses on purpose must be refused with a
message that says `--firmware`; any other failure fails the test. Two
larger programs written for this stage exercise errors through methods,
loops, nested `try` blocks, conditions, recursion and function values, and
one test checks that the C of a firmware build contains no `setjmp`.

## Things to try

1. Build `examples/sensor.zeno` both ways and compare `read_all` in the two
   C files side by side. Which version is longer? Which could you step
   through in a debugger more easily?
2. In the firmware `sensor.c`, follow one damaged line from `zeno_note` in
   `parse_line` to the `except` in `read_all`. Count the tests it passes.
3. Write a function that divides, and catch `ZeroDivisionError` around a
   call to it. Read the error from `zeno build --firmware`, then rewrite
   the function to test first.
4. Add a second `except` to a `try`, for a kind the body does not raise.
   When is it refused, and when is it let through? Find the line in
   `try_with_values` that decides.
5. Time a loop that calls a function that can raise ten million times,
   built both ways. Then put the `try` inside the loop and time again.
   Which build wins each time, and why?
6. Let function types say `raises` (`function(str) -> int raises
   ValueError`). The parser, the checker's `Callable`, `FunctionKind` and
   `invoke` in `emit_c.rs` all need to learn it. Then `texts.map(parse)`
   can compile in a firmware build.
