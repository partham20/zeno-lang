# How the bit operators and the sized whole numbers work

Firmware talks to hardware in bytes. A sensor sends six of them, a
register is thirty-two bits wide, and bit 3 of it turns something on.
This stage gives Zeno the two things that work needs:

- **Bit operators**: `&`, `|`, `^`, `~`, `<<`, `>>`.
- **Whole numbers with a size**: `u8`, `u16`, `u32`, `i8`, `i16`, `i32`.

`examples/packet.zeno` uses both. It packs sensor readings into
six-byte frames and unpacks them again:

```sh
cargo run -- run examples/packet.zeno
cargo run -- build examples/packet.zeno          # packet.c, and ./packet
cargo run -- build --llvm examples/packet.zeno   # packet.ll, and ./packet
```

```
a5 03 00 d7 fe 7d   Reading(sensor=3, flags=0, value=215, count=254) true
a5 83 03 ff ff 29   Reading(sensor=3, flags=8, value=1023, count=255) true
  alarm on sensor 3
a5 c3 fa 00 00 62   Reading(sensor=3, flags=12, value=64000, count=0) true
  alarm on sensor 3
the checksum should be a6, but the frame says 9b
```

The page has two parts, because the stage was built in two commits.
The reasons for every choice are in [`decisions.md`](decisions.md),
2.7 and 2.8 for the language and 5.13 for how it is built.

---

# Part one: the bit operators

| | | |
|---|---|---|
| `a & b` | and | a bit is 1 where both have a 1 |
| `a \| b` | or | a bit is 1 where either has a 1 |
| `a ^ b` | exclusive or | a bit is 1 where the two differ |
| `~a` | not | every bit turned over |
| `a << n` | shift left | `a` times two, `n` times |
| `a >> n` | shift right | `a` halved, `n` times, rounding down |

Each has a form that updates a variable: `reg |= 1 << 3`.

An `int` is 64 bits in two's complement, the way every processor made
in the last fifty years holds a signed number. So `~x` is `-x - 1`, and
`-1` is sixty-four ones: `-1 & 0xFF` is `255`.

## Through the stages

A new operator touches every stage, a little. That makes it a good way
to walk the whole compiler again.

**The lexer** (`src/lexer.rs`) gets eleven new tokens. The only care
needed is where one is the start of another. `<` could be `<`, `<=`,
`<<` or `<<=`, so the lexer looks one character further each time:

```rust
'<' => {
    if self.eat('<') {
        if self.eat('=') { ShlAssign } else { Shl }
    } else if self.eat('=') {
        LtEq
    } else {
        Lt
    }
}
```

`&&` and `||` were already errors with a hint ("Zeno spells this
`and`"). They still are. The check for the double character simply
comes before the single one.

**The parser** (`src/parser.rs`) gets four new precedence levels,
between the comparisons and `+`:

```
or → and → not → comparison → | → ^ → & → << >> → + - → * / // % → - ~ → **
```

Every level in this parser is a function with the same loop in it
([`parser.md`](parser.md)). Four more copies of the loop would be four
chances for a slip, so there is one function that takes the level as a
number:

```rust
fn bits_at(&mut self, level: usize) -> Result<Expr, Error> {
    if level == 4 {
        return self.sum();          // past the last bit level: arithmetic
    }
    let mut left = self.bits_at(level + 1)?;
    loop {
        let op = match (level, self.peek()) {
            (0, TokenKind::Pipe) => BinaryOp::BitOr,
            (1, TokenKind::Caret) => BinaryOp::BitXor,
            (2, TokenKind::Amp) => BinaryOp::BitAnd,
            (3, TokenKind::Shl) => BinaryOp::Shl,
            (3, TokenKind::Shr) => BinaryOp::Shr,
            _ => return Ok(left),
        };
        self.advance();
        let right = self.bits_at(level + 1)?;
        left = Self::binary(left, op, right);
    }
}
```

`~` joins unary minus in `negation`. Nothing else in the parser
changed: the tree already had `Binary` and `Unary` nodes, and these
are five more operators and one more.

**The checker** (`src/checker.rs`) allows them on whole numbers only.
Two bools are the likely slip, from someone who writes C, so that case
gets its own sentence:

```
error: `&` works on the bits of whole numbers, and these are bool and bool.
       For two bools Zeno spells this `and`
```

**What they do** is written once, in `src/ops.rs`, which the
interpreter and the VM share ([`vm.md`](vm.md)). `&`, `|`, `^` and `~`
are Rust's own operators on an `i64`. The shifts are the interesting
ones, below.

**The C backend** writes `(l & r)`, `(l | r)`, `(l ^ r)` and `(~x)`
straight into the C. **The LLVM backend** writes `and i64`, `or i64`
and `xor i64`. IR has no "not" instruction, and needs none: `~x` is
`xor i64 %x, -1`, because exclusive-or with all ones turns every bit
over.

## Why not C's precedence

In C, this line is a bug:

```c
if (flags & MASK == 0) { ... }
```

`==` binds tighter than `&`, so it means `flags & (MASK == 0)`. It
compiles without a word. The people who made C said later that it was
a mistake, kept from an older language where `&` also did the job of
`&&`.

Python moved the bit operators above the comparisons, and Zeno copies
it. So in Zeno the line means what it looks like, and the tests pin it
down:

```
print(flags & mask == 5, 1 << 2 + 1, 6 | 1 ^ 3 & 2)
```

`1 << 2 + 1` is `1 << 3`: arithmetic is tighter than a shift, in C,
Python and Zeno alike.

## A shift that cannot go quietly wrong

In C, three things can happen to a shift and none of them is an error:

- `1 << 40` on a 32-bit `int`: undefined.
- `x << 64` or `x << -1`: undefined.
- `-5 >> 1`: whatever the compiler likes.

Zeno already checks `+` and `*` for overflow (2.2). A left shift is a
multiplication by a power of two, so it gets the same treatment, and a
shift count must be 0 to 63. This is the whole of it, from the runtime
that every compiled program carries (`src/runtime.c`):

```c
static int64_t zeno_shl(int64_t a, int64_t n, int line, int col) {
    zeno_shift_count(n, line, col);
    int64_t shifted = (int64_t)((uint64_t)a << n);
    if ((shifted < 0 ? ~(~shifted >> n) : shifted >> n) != a) zeno_overflow(line, col);
    return shifted;
}

static int64_t zeno_shr(int64_t a, int64_t n, int line, int col) {
    zeno_shift_count(n, line, col);
    return a < 0 ? ~(~a >> n) : a >> n;
}
```

Three small tricks are in there.

1. **The shift is done on the unsigned number.** C says exactly what
   `<<` does to a `uint64_t`: the bits that leave the top are gone. On
   a signed number it says nothing.
2. **"Did it overflow?" is "does shifting back give what I started
   with?"** If any bit that mattered fell off the top, it does not.
3. **A negative number is shifted right upside down.** `~a` is not
   negative when `a` is, so `~a >> n` is safe, and turning the bits
   back over gives the answer that rounds down: `-5 >> 1` is `-3`, the
   same as `-5 // 2`.

The interpreter does the same thing in Rust, in a dozen lines of
`ops.rs`. Rust's `>>` on an `i64` already rounds down, so only the
shift-back test is needed there.
The LLVM backend calls these two functions too, where it writes `+` and
`*` in IR itself: a shift is rare enough that a call costs nothing
worth saving.

---

# Part two: whole numbers with a size

| | bits | holds |
|---|---|---|
| `u8` | 8 | 0 to 255 |
| `u16` | 16 | 0 to 65535 |
| `u32` | 32 | 0 to 4294967295 |
| `i8` | 8 | -128 to 127 |
| `i16` | 16 | -32768 to 32767 |
| `i32` | 32 | -2147483648 to 2147483647 |

`int` is still 64 bits, and still what you get when you write nothing.

## The rules

There are four, and the rest of this part is how each is built.

```
let a: u8 = 200            # 1. a number written out takes the size it is put in
let b: u8 = 100
let total = a + b          # 2. arithmetic gives an int: 300, never 44
let c: u8 = u8(total)      # 3. u8( ) checks. This one raises OverflowError
let d: u8 = u8.wrap(total) # 4. u8.wrap( ) keeps the low bits: 44
```

And one about going the other way: a `u8` can be used wherever an
`int` is wanted, with nothing written, because every `u8` is an `int`.

The thinking behind rule 2 is the heart of the design, so here it is
again in short (decision 2.8a has it in full). C and Rust do the sum
at the small size. Then either `200 + 100` is quietly `44`, or
`(a + b) // 2` fails for two numbers whose average fits. Zeno does
every sum on full ints, so the middle of an expression can never
overflow at 255. The one place a number may not fit is the place it is
stored, and there the program says which it wants: a check, or the low
bits.

The cost is that `count += 1` does not compile for a `u8`:

```
error: the new value should be u8, but this is int.
       Convert it with `u8(...)`, which checks that it fits
```

Write `count = u8(count + 1)`, or `u8.wrap(count + 1)` for a counter
that is meant to go round.

## One file knows the sizes

`src/ints.rs` is about 120 lines, tests included, and it is the only place that knows what a
`u8` holds:

```rust
pub struct Width {
    pub name: &'static str,   // "u8"
    pub wrap: &'static str,   // "u8.wrap"
    pub bits: u32,
    pub signed: bool,
}

pub const WIDTHS: [Width; 6] = [ ... ];

impl Width {
    pub fn min(self) -> i64 { if self.signed { -(1 << (self.bits - 1)) } else { 0 } }
    pub fn max(self) -> i64 { ... }
    pub fn holds(self, n: i64) -> bool { (self.min()..=self.max()).contains(&n) }
    pub fn wrap(self, n: i64) -> i64 { ... }
}
```

The checker, the interpreter and both compilers ask it. A seventh
size would be one more line in `WIDTHS`.

## A sized number is an ordinary int with a promise

This is the idea that made the stage small. Look at what rule 2 says:
arithmetic on sized numbers gives an `int`. So once a `u8` exists,
there is nothing it does differently from an `int`. It is added like
one, printed like one, compared like one.

So it *is* one. In the interpreter a `u8` is a `Value::Int`. In C it is
an `int64_t`. In LLVM it is an `i64`. The size is a promise that the
number is in range, and the promise is kept in exactly two places:
`u8( )` and `u8.wrap( )`, which make one, and the checker, which lets
nothing else into a `u8`.

Units work this way (`500 ms` is a `double`), and so do pins (`led` is
its number). The type stops the mistake, and then it is gone.

The other design would be a `Value::U8`, and with it a new row in
every operator, in four engines. That is a lot of code whose whole job
is to agree with the code beside it.

## In the checker

The checker gets one new type, `Ty::Sized(Width)`, and three small
rules.

**Arithmetic sees an int.** `widened` turns a sized type into `int`,
and `binary` applies it to both sides before it does anything else:

```rust
fn widened(&self) -> Ty {
    match self {
        Ty::Sized(_) => Ty::Int,
        other => other.clone(),
    }
}
```

After that line the whole of `binary` is unchanged. It only ever sees
ints, and answers `int`. The built-in functions do the same to their
arguments, which is why `abs`, `max` and `sum` work on sized numbers
without knowing they exist.

**A sized number fits where an int is wanted.** One line in `fits`:

```rust
(Ty::Sized(_), Ty::Int) => true,
```

There is no line the other way round, so an `int` going to a `u8` is
the ordinary "should be u8, but this is int" error, with a hint added
for the fix.

A `list[u8]` does *not* fit a `list[int]`, though each item would.
Lists are shared: whoever held it as a `list[int]` could append 300,
and the first holder would then find a `u8` that is not one. This is
the one place the checker has to think about it:

```rust
(Ty::List(from), Ty::List(to)) => !sized_into_int(from, to) && fits(from, to),
```

**A number written out adapts.** `let b: u8 = 7` must not need
`u8(7)`. But `7` is an `int`, and ints do not fit. So before the
checker asks whether a value fits, it asks whether the value is a
number that is *written there*:

```rust
enum Written {
    Number(i128),
    List(Vec<Written>),
}
```

`written(expr)` gives one for `7`, for `-1`, and for `[1, 2, 0xFF]`.
If the place wants a sized type, the number is checked against the
size there and then, and takes the type:

```
let b: u8 = 300
error: 300 does not fit in a u8 (0 to 255)
```

That is an error before the program runs, and it costs nothing when
it does. It works in every place a value can go: a `let`, an
assignment, an argument, a `return`, a default, an item appended to a
list.

A `const` is not a written number. `const START = 0xA5` is an `int`.
Give it the size: `const START: u8 = 0xA5`.

**`u8` is two things with one name.** It is a type, and it is the
built-in function that converts, `u8( )`.
The checker finds the type through the function: a type name that
resolves to the built-in `u8` is the sized type. That is why a class
of your own called `u8` wins, with no special case. `u8.wrap` is a
field of the built-in, the same way `math.sqrt` is a field of a
module.

## Making one: `u8( )` and `u8.wrap( )`

In the interpreter both are a few lines in `src/builtins.rs`:

```rust
fn fit_in(width: Width, args: Vec<Value>, span: Span) -> Result {
    arity(width.name, &args, 1, 1, span)?;
    match &args[0] {
        Value::Int(n) if width.holds(*n) => Ok(Value::Int(*n)),
        Value::Int(n) => raise("OverflowError", width.does_not_fit(n), span),
        other => wrong_type(width.name, "a whole number", other, span),
    }
}
```

Look at what comes back: `Value::Int(*n)`. The same number that went
in. All `u8( )` does is look.

`wrap` is the one piece of real arithmetic, in `ints.rs`:

```rust
pub fn wrap(self, n: i64) -> i64 {
    let low = n & ((1 << self.bits) - 1);
    if self.signed && low > self.max() { low - (1 << self.bits) } else { low }
}
```

`(1 << 8) - 1` is `255`, eight ones, so the `&` keeps the low eight
bits. For a signed size the top one of those bits counts as minus, so
`200` (which has it set) becomes `200 - 256`, which is `-56`. That is
what the bits `11001000` mean when they are read as an `i8`.

There is a Rust detail worth a look. A built-in is a plain function
pointer, and a function pointer cannot remember which size it is for.
So there has to be one function for each size. They are stamped out
by a number in the type:

```rust
fn fit<const N: usize>(_: &mut dyn Machine, args: Vec<Value>, span: Span) -> Result {
    fit_in(ints::WIDTHS[N], args, span)
}

const FITS: [Call; 6] = [fit::<0>, fit::<1>, fit::<2>, fit::<3>, fit::<4>, fit::<5>];
```

`fit::<0>` is a different function from `fit::<3>`, made by the
compiler from the one written here. This is called a const generic.

**In C**, the two are runtime functions:

```c
static int64_t zeno_fit(int64_t n, int64_t min, int64_t max, const char *size, int line, int col) {
    if (n < min || n > max) {
        zeno_fail("OverflowError", zeno_join(8, zeno_str_int(n), " does not fit in ", size, ...), line, col);
    }
    return n;
}

static int64_t zeno_wrap(int64_t n, int bits, bool is_signed) {
    uint64_t low = (uint64_t)n & ((UINT64_C(1) << bits) - 1);
    if (is_signed && (low >> (bits - 1)) != 0) return (int64_t)low - (INT64_C(1) << bits);
    return (int64_t)low;
}
```

So this line of `packet.zeno`:

```
u8(reading.flags << 4 | reading.sensor)
```

compiles to this:

```c
zeno_fit((zeno_shl(((int64_t)z2_reading->z_flags), 4, 42, 12) | ((int64_t)z2_reading->z_sensor)),
         INT64_C(0), INT64_C(255), "a u8", 42, 9)
```

**In LLVM**, `u8( )` calls the same `zeno_fit`. `u8.wrap( )` needs no
call at all. IR has an instruction that keeps the low bits and one
that widens again, and those two are the whole of it:

```llvm
%t9 = trunc i64 %t7 to i8        ; keep the low eight bits
%t10 = zext i8 %t9 to i64        ; and make it an i64 again, with zeros
```

For a signed size the second is `sext` ("sign extend"), which fills
the new bits with copies of the top one. That is the `low - 256` of
the Rust version, done by the processor in one instruction.

## Where the size is real: a field of an object

If a `u8` is an `int64_t` everywhere, what was the point? The point is
this struct, from `packet.c`:

```c
struct z_Reading {
    uint8_t z_sensor;
    uint8_t z_flags;
    uint16_t z_value;
    uint8_t z_count;
};
```

Six bytes (five, and one of padding), where four plain ints would
take thirty-two. In LLVM it is `%z_Reading = type { i8, i8, i16, i8 }`.

A field is the one place a sized number is stored small. Memory is
spent on what there are many of, and what there are many of lives in
objects. A local variable lives in a register, where a `u8` saves
nothing.

The checker tells the compilers which fields are sized
(`Types::widths`), and each compiler makes the value small on the way
in and an ordinary int again on the way out. In C:

```c
self->z_sensor = (uint8_t)z_sensor;        /* in: the constructor */
... ((int64_t)z_p->z_kind) ...             /* out: every read */
```

**That cast on the way out is not decoration.** Without it, C would do
arithmetic on a `uint32_t` field as *unsigned* arithmetic. `-field`
would be a huge positive number, and `field < -1` would be true. The
cast is what makes rule 2 hold in C. A test has a `u32` field holding
4294967295 and prints `~q.big`, `-p.big` and `p.big < -1` for this
reason.

In LLVM the same two steps are the `trunc`, and `zext` or `sext`, that
`wrap` uses. They live in two functions of `src/emit_llvm.rs`,
`load_field` and `store_field`, and every read or write of a field
goes through one of them:

```llvm
%t3 = getelementptr %z_Packet, ptr %t2, i32 0, i32 0
%t4 = load i8, ptr %t3
%t5 = zext i8 %t4 to i64
```

The `trunc` on the way in cannot lose anything: the checker made sure
the number is in range before it got there.

## What it does not do yet

- **A `list[u8]` is not packed.** Each item still takes eight bytes,
  because the runtime has one list type for everything
  ([`compiler.md`](compiler.md)). A packed list, or a `bytes` type, is
  its own stage.
- **No `u64`.** Half its values do not fit in an `int`, and then "the
  sum is an int" stops being true (decision 2.8d).
- **A `u8` does not go where a `u16` is wanted** without `u16( )`,
  though it always fits. The rule today is "sized to int is free,
  everything else is written". It could be loosened.
- **No `u8.max` or `u8.min`.** `ints.rs` knows them. Nothing lets a
  program ask.
- **Variables, parameters and enum cases are not small**, only fields
  of classes.
- **In a program that is not strict**, a value whose type is unknown
  until run time can land in a `u8` unchecked, as it can land in an
  `int` today. Compiled programs are always strict.
- **In a firmware build** a `u8( )` that does not fit stops the
  program, like any overflow ([`firmware.md`](firmware.md)). It cannot
  be caught there. Test first, or use `wrap`.

## The tests

Every program in the tests runs on the interpreter and the VM, then
compiled through C, C for firmware, and LLVM, and all five must print
the same thing. The new ones:

- `ints::tests`: the six ranges, and `wrap` at each edge.
- `checker::tests::sized_ints_are_types`,
  `a_written_number_must_fit_its_size`,
  `an_int_does_not_become_sized_by_itself`,
  `a_size_left_out_is_worked_out`: what is allowed, and the exact
  words of every refusal.
- `interpreter::tests::sized_ints` and `a_sized_int_must_fit`.
- `emit_c::tests::sized_ints_match`: one long program with every
  operator on sized numbers, every conversion at its edges, sized
  fields of every size, lists, defaults and `or none`.
- `emit_c::tests::a_sized_field_is_small_in_the_struct` and
  `emit_llvm::tests::the_ir_for_a_sized_field`: the struct really is
  small, and the casts are where this page says.
- `bit_operators_match` and `a_shift_too_far_is_an_error`, from part
  one, checked against Python.

## Things to try

1. In `packet.zeno`, make a reading with `flags=16`. Which line
   refuses it, and is it refused before the program runs or while it
   does? Now pass `flags=u8(16)`. What changed, and why?
2. Change `checksum` to return `u8(total)` and run it. Which frame
   fails? Change it back and say in one sentence why a checksum is the
   textbook case for `wrap`.
3. `1 << 2 + 1` is 8. Change `bits_at` so that the shifts bind tighter
   than `+`, run the tests, and read which ones fail. Put it back.
4. Add `u8.max` and `u8.min`. The checker's `field` and `ops.rs`'s
   `field` already handle `u8.wrap`, so start beside those. They are
   constants, so what should the two compilers write for them?
5. Add `>>>`, a right shift that brings in zeros and not copies of the
   sign. Follow part one's list of stages. Is it needed at all, now
   that `u32` exists? Try writing it with `u32.wrap` and `>>`.
6. Allow a `u8` to go where a `u16` is wanted. It is one more arm in
   `fits`. Write down the full table of which size fits in which
   first: does `u8` fit `i8`? Does `i8` fit `u16`? Then decide whether
   the table is worth teaching to everyone who reads a Zeno program.
7. Build `packet.zeno` and print `sizeof(z_Reading)` from the C (add a
   `printf` to `main` by hand). Then put a `u32` field first in the
   class and look again. Then put it last. This is "struct padding",
   and it is why field order matters in firmware.
8. Find `load_field` in `emit_llvm.rs`. Change `sext` to `zext` for
   every size and run the tests. Which value in which test goes wrong,
   and what does it print?
9. Write `rotate_left(x: u8, n: int) -> u8`, which shifts left and
   brings the bits that fall off back in at the bottom. You need `<<`,
   `>>`, `|` and one `wrap`. Check it against `0b1000_0001` rotated by
   one.
10. A big one: make `list[u8]` take one byte an item. The list in
    `runtime.c` holds `zeno_slot` items, each eight bytes. What would
    a second list type need from the runtime, and how many functions
    in `emit_c.rs` look inside a list? Count them before you start.
