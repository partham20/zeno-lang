# The gaps programs ran into: `+=`, and a format on a quantity

The last features of version 0.1 are small. Each one is something a
real program tripped over, and two of them turned out to be hiding a
mistake in the compiler. This page goes through all five.

```zeno
var count: u8 = 250
count += 5                 # 255. It was refused before
count //= 2                # 127. There was no `//=` before
let angle = 30.456 deg
print(count, f"{angle:.1f}")
```

```
127 30.5 deg
```

The decisions are 10.25 to 10.29 in [`decisions.md`](decisions.md).

## `x += y` is `x = x + y`, in that order

Every page that mentions `+=` says it means `x = x + y`. This program
asks how exactly:

```zeno
var count = 1

function bump() -> int {
    count = 10
    return 1
}

count += bump()
print(count)
```

If `count` is read first, the sum is `1 + 1`. If `bump()` runs first,
it is `10 + 1`. Zeno printed 2 when the program was built through C,
and 11 in the interpreter, in the VM and through LLVM. Four engines
that are tested against each other on every program had never been
given one where the value changes the place it is added to.

Now all four read what is there first, which is the order of `count =
count + bump()` and the answer of Python, Java and JavaScript
(decision 10.25). In the interpreter the mend is the order of two
lines:

```rust
Some(op) => {
    let old = self.eval(target, env)?;
    let new = self.eval(value, env)?;
    ops::binary(*op, old, new, span)?
}
```

In the VM it removed an instruction. `x += 1` used to compile to
`Const, GetLocal, Swap, Binary, SetLocal`: the value, then the
variable, then a `Swap` to put them the right way round for `+`. It
is `GetLocal, Const, Binary, SetLocal` now, the same four as `x = x +
1`, and `Swap` is gone from the VM because nothing else used it.

## The place of a `+=` has no call in it

The same question, asked of the other side:

```zeno
xs[next()] += 5
```

`x += y` is `x = x + y`, so this was `xs[next()] = xs[next()] + 5`:
two calls. With a `next` that counts up, it read item 1 and wrote
item 2. Every engine did the same thing, so it was "right" by the
tests and wrong by anybody's reading.

It is refused now (decision 10.26):

```
error: `+=` reads this place and then writes it, so a call in the place would run twice. Keep what the call gives in a `let` first, and use that name here
 --> twice.zeno:9:4
  |
9 | xs[next()] += 5
  |    ^
```

```zeno
let position = next()
xs[position] += 5
```

The check is `call_in` in `src/checker.rs`: it walks the place (a
name, a field of a place, an item of a place) and gives back the
first call it meets. A plain `=` is not asked: `xs[next()] = 5` reads
nothing, so `next` runs once.

Other languages work the place out once behind the scenes. Zeno asks
for the `let`, which is the same thing where you can see it, and
which leaves the door open: allowing the line later would change no
program that is accepted today.

## `count += 1` on a sized number

[`sized-ints.md`](sized-ints.md) ends its rules with a cost: the
answer of `count + 1` is an int, an int does not go into a `u8` by
itself, and so `count += 1` did not compile. It does now, and it
means the checked conversion (decision 10.27):

```zeno
var count: u8 = 250
count += 5                    # count = u8(count + 5)
count += 1                    # OverflowError: 256 does not fit in a u8 (0 to 255)
count = u8.wrap(count + 1)    # going round is still said in a word
```

No engine learned anything for this. The checker is the only stage
that knows `count` is a `u8`. It writes down where each such
statement is, and before the program is run or compiled the statement
is rewritten into the long form:

```
zeno run ──► load ──► check ──► with_fits ──► check ──► the engine
                        │           ▲
                        └─ where ───┘
```

`checker::checked` is the whole of that pipeline, and
`Program::with_fits` in `src/ast.rs` is the rewrite. The second check
is there because the checker keeps its notes by where each node of
the tree is in memory, and the rewrite makes new nodes. A program
with no `+=` on a sized number is checked once, as before.

You can see the long form with `zeno bytecode`: there is a call of
`u8` that the program did not write.

This is why the refusal in the last section had to come first. The
long form names the place twice, `count = u8(count + 5)`, and that is
only the same as naming it once if working the place out does
nothing but find it.

## Every operator has an `=` form

`//=`, `%=` and `**=` were missing (decision 10.28). `count //= 2`
was a mistake with an unhelpful message, and `count /= 2` cannot be
right for a whole number because `/` gives a float. Three tokens in
the lexer and three lines in the parser were all it took: the engines
already do `x = x op y` for any operator.

## A format on a quantity

```zeno
let supply = 3.3 V
print(f"[{supply:8.2f}]")     # [  3.30 V]
```

A quantity takes the formats a float takes (decision 10.29). The
decimals are the number's, the unit follows as it always does, and
the width counts both, so a column of readings ends in a straight
line (this is `{reading:8.2f}`):

```
  3.30 V
 12.00 V
  0.75 V
```

Before this the line was `f"{supply / 1 V:8.2f} V"`, with the unit
taken off by hand and typed back as text.

In a compiled program a quantity is a `double`, and its unit is known
only to the compiler ([`math.md`](math.md)). So the compiler writes
the unit into the call:

```c
zeno_fixed_quantity(z_supply, 2, "V")
```

which is one line in `src/runtime.c`: the number with two decimals, a
space, the unit. `zeno_pad` then makes it eight wide, as it does for
any text.

## The tests

- `interpreter::tests::a_compound_assignment_is_the_long_form` and
  `emit_c::tests::what_is_there_is_read_before_the_value_is_worked_out`:
  the order, in every engine and build.
- `checker::tests::the_place_of_a_compound_assignment_has_no_call_in_it`.
- `checker::tests::a_sized_number_takes_an_operator_and_an_equals`,
  `emit_c::tests::compound_assignment_on_sized_numbers_matches` and
  `with_no_collector_a_sized_number_takes_an_operator_and_an_equals`.
- `a_quantity_takes_the_formats_of_a_float`, in the interpreter's
  tests and in `emit_c`'s, where the build with no collector must
  also use the same memory through C and through LLVM.

## Things to try

1. Write the `bump` program above, and before you run it, write
   `count = count + bump()` under it and predict both answers. Then
   run `zeno bytecode` on it and compare the two statements.
2. Write `xs[next()] += 5` and read the message. Mend it with a
   `let`. Then change `+=` to `=`. Why is that allowed with the call
   in the place?
3. `xs[len(xs) - 1] += 1` is refused too, though `len` changes
   nothing. Read the "Cost" paragraph of decision 10.26, and say what
   the checker would need to know to allow it.
4. Run `var count: u8 = 250`, `count += 10`. Catch the error with
   `try` and print `count` afterwards. What does it hold, and why?
   Then build the same program with `--firmware` and read what the
   compiler says about the `try`.
5. In `Program::with_fits`, use `width.wrap` in place of `width.name`
   and run the tests. Which fail, and what would every `u8` counter
   in the examples do differently? Put it back.
6. `count /= 2` is refused for a `u8`. Would it be a good idea for
   `/=` to mean `//=` on whole numbers? Look at what `7 / 2` is in
   Zeno before you answer.
7. Print a table of three voltages with `{reading:10.2f}`. Change one
   of them to `mV`. Do the lines still end together? Now make the
   width count the number alone (in `builtins::format`, and in
   `formatted` in both compilers) and look again. Put it back.
8. `f"{supply:x}"` is refused, in the interpreter with one message
   and by `zeno build` with another. Make the build say what the
   interpreter says.
