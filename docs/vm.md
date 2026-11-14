# How the bytecode VM works

`zeno run` has a second engine. With `--vm` it does not walk the syntax
tree. It compiles the program to **bytecode**, a flat list of small
instructions, and runs that.

```sh
cargo run -- run --vm examples/shapes.zeno     # same output as without --vm
cargo run -- bytecode examples/hello.zeno      # see the bytecode itself
```

```
== main() ==
0000    2  Const        0          ; <function print>
0001    |  Const        1          ; "Hello, Zeno!"
0002    |  Call         1
0003    |  Pop          1
0004    |  Const        2          ; none
0005    |  Return
```

The program does exactly what it did before. That is the rule the whole
stage is built around, and most of the tests are there to hold it.

## Why

The interpreter ([`interpreter.md`](interpreter.md)) answers the same
questions again every time a line runs. To read `count` it looks the name
up in a hash table, and if it is not there, in the table of the block
outside, and so on. To run `a + b * 2` it follows three pointers into the
tree before it has added anything.

None of those answers change while the program runs. A compiler can work
them out once, and leave behind only the work that has to happen: get
slot 0, get slot 1, multiply, add. That is what bytecode is.

It is also how most languages you have used run: Python, Java, C#, Lua,
Ruby and the JavaScript engines all compile to a bytecode of their own
first. This is the smallest honest version of that.

A VM is not what puts Zeno on a microcontroller. `zeno build` does that,
which is why it came first (decision 5.5a).

## The files

- [`src/bytecode.rs`](../src/bytecode.rs): the instructions (`Op`), what a
  compiled function holds (`Proto`), and the printer behind
  `zeno bytecode`. Start here. Every instruction has a comment.
- [`src/emit_bytecode.rs`](../src/emit_bytecode.rs): the compiler, from
  the syntax tree to bytecode. It is `emit_c.rs`'s smaller sibling.
- [`src/vm.rs`](../src/vm.rs): the machine that runs it. `execute` is the
  loop.
- [`src/ops.rs`](../src/ops.rs): what each step *does*: `+`, `<`, reading
  a field, lining arguments up with parameters. The interpreter and the
  VM both call it.

## A stack, and instructions that push and pop

The VM keeps one row of values, the **stack**. Every instruction does
something small to the end of it. Take this program:

```zeno
let a = 2
let b = 3
print(a + b * 2)
```

```
0000    1  Const        0          ; 2
0001    2  Const        1          ; 3
0002    3  Const        2          ; <function print>
0003    |  GetLocal     0          ; a
0004    |  GetLocal     1          ; b
0005    |  Const        0          ; 2
0006    |  Binary       *
0007    |  Binary       +
0008    |  Call         1
0009    |  Pop          1
```

The first column is the instruction's number, the second the line of the
program it came from. Follow the stack down the listing:

| After | The stack |
|---|---|
| `0000` | `2` |
| `0001` | `2  3` |
| `0002` | `2  3  print` |
| `0003` | `2  3  print  2` |
| `0004` | `2  3  print  2  3` |
| `0005` | `2  3  print  2  3  2` |
| `0006` | `2  3  print  2  6` |
| `0007` | `2  3  print  8` |
| `0008` | `2  3  none` (and `8` is on the screen) |
| `0009` | `2  3` |

Three things to see here.

**The tree is gone, and the order it stood for is still there.** `*` comes
before `+` in the listing because the parser put it deeper in the tree,
and the compiler writes the inner parts first. The whole of that, in
`emit_bytecode.rs`:

```rust
ExprKind::Binary { op, left, right } => {
    self.expr(left);
    self.expr(right);
    self.emit(Op::Binary(*op), span);
}
```

Code for the left side, code for the right side, then one instruction.
Whatever the two sides are, each leaves exactly one value on the stack,
and `Binary` takes two and leaves one. Every expression keeps that
promise, which is why they can be nested without thinking.

**A variable is a place on the stack.** `let a = 2` did not store anything
anywhere. It pushed `2` and the compiler made a note: `a` is slot 0. From
then on `a` is `GetLocal 0`. No name is looked up while the program runs.

**Constants are kept beside the code.** `Const 0` means "push constant
number 0 of this function". An instruction only ever holds small numbers.

## The compiler counts

To know that `b` is slot 1, the compiler has to know how tall the stack is
at every instruction. It keeps a count (`height`), and `emit` updates it
for each instruction written: `Const` is one more, `Binary` is one fewer,
`Call 1` takes two and leaves one.

A block's variables are dropped when the block ends. The compiler notes
the height at the `{`, and at the `}` writes a `Pop` for the difference
(`open` and `close`).

When this count is wrong by one, every variable declared afterwards is
read from its neighbour's slot. That is the commonest bug in a stack VM,
and the first of the new tests is built to catch it.

## Jumps

There is no `if` or `while` in bytecode. There is "carry on from
instruction n".

```zeno
var n = 3
while n > 0 {
    n -= 1
}
```

```
0000    1  Const        0          ; 3
0001    2  GetLocal     0          ; n
0002    |  Const        1          ; 0
0003    |  Binary       >
0004    |  JumpIfFalse  11
0005    3  Const        2          ; 1
0006    |  GetLocal     0          ; n
0007    |  Swap
0008    |  Binary       -
0009    |  SetLocal     0          ; n
0010    2  Jump         1
0011    |  Const        3          ; none
```

`JumpIfFalse 11` leaves the loop, `Jump 1` goes round again. When the
compiler writes the `JumpIfFalse` it does not know yet where the loop
ends. It writes `JumpIfFalse 0`, remembers where, and fills the number in
when it gets there (`patch`).

`break` is a jump to the end of the loop, `continue` a jump to the top.
Either can be deep inside other blocks, with their variables on the
stack, so each is preceded by `PopTo`: drop everything above the height
the loop started at.

(`n -= 1` works out the `1` before it reads `n`, because that is the order
the interpreter does it in. Hence the `Swap`.)

`for` keeps two values on the stack that the program cannot see: a copy
of the list, and how far along it is. `ForNext` pushes the next item or
jumps out.

## Calls and frames

```zeno
function double(n) {
    return n * 2
}
print(double(21))
```

```
== main() ==
0000    1  Closure      0          ; double
0001    4  Const        0          ; <function print>
0002    |  GetLocal     0          ; double
0003    |  Const        1          ; 21
0004    |  Call         1
0005    |  Call         1
...

== double(n) ==
0000    1  Required     0 0        ; "`double` needs a value for `n`"
0001    |  Enter
0002    2  GetLocal     0          ; n
0003    |  Const        1          ; 2
0004    |  Binary       *
0005    |  Return
```

Each function is compiled to a list of its own. At `0004` the stack is

```
double  print  double  21
```

`Call 1` means: the top value is one argument, and under it is the thing
to call. The VM does not copy the argument anywhere. It starts a new
**frame**, which says "this function's slot 0 is *there*", pointing at the
`21`. Inside `double`, `GetLocal 0` reads it. That is the reason
parameters are the first slots.

A frame (`Frame` in `vm.rs`) is what the VM remembers about one function
that is running: which function, which instruction it is at, and where
its stretch of the stack begins. `Return` takes the result, removes the
frame and everything it had on the stack, and pushes the result where
the call was. The caller's frame is now on top again, and carries on
from the instruction after its `Call`.

So a Zeno call does not use the Rust stack at all. In the interpreter a
call is a Rust function calling itself, and deep recursion needs a deep
Rust stack. Here it is one more item in a list.

The first instructions of a function settle its parameters. `Required`
complains if the caller left one out. A parameter with a default gets
`Default n m`: if the caller gave it, jump over the code that works the
default out. Then `Enter` counts the call, for the limit of 1000 nested
calls that both engines share.

Calls with named arguments, methods, classes and built-ins all go through
`invoke` in `vm.rs`. A class is called like a function whose parameters
are its fields: the compiler writes a small function for each class that
fills in the defaults and ends with `Object`.

## Variables that outlive their function

```zeno
function counter() {
    var count = 0
    return function() {
        count += 1
        return count
    }
}
```

`count` cannot live in a slot. The slot is gone when `counter` returns,
and the inner function goes on using `count` after that.

```
== counter() ==
   cells: count
0001    2  NewCell      0          ; count
0002    |  Const        0          ; 0
0003    |  DefineCell   0          ; count
0004    3  Closure      0          ; function
0005    |  Return

== function() ==
   captures: count
0001    4  Const        0          ; 1
0002    |  GetCaptured  0          ; count
0003    |  Swap
0004    |  Binary       +
0005    |  SetCaptured  0          ; count
0006    5  GetCaptured  0          ; count
0007    |  Return
```

So `count` lives in a **cell**: a box of its own, off the stack, that is
kept for as long as anything points at it. `Closure` makes the function
value, and at that moment hands it the cells it uses. The function's code
plus those cells is a **closure**. Call `counter` twice and you get two
closures with the same code and a cell each.

How does the compiler know `count` needs a cell? Before compiling a
function it collects every name used by the functions written inside it
(`shared_names`). A variable with one of those names gets a cell. That is
cruder than it could be, since it goes by name alone, and it is never
wrong.

At the top level, every function another function calls is in a cell for
the same reason. There are no global variables in the VM. The program is
a function called `main`, and what looks like a global is a variable of
`main` that other functions captured.

`NewCell` runs at the top of the block, each time the block is entered.
That is what gives every trip round a loop its own variables:

```zeno
for i in range(3) {
    makers.append(function() { return i })     # three functions, three cells
}
```

## Functions above where they are written

A function can be called from above the line it is written on (decision
3.4). So at the top of each block the compiler writes a `Closure` for
every function in it, before anything else (`hoist`).

It does not compile the function's *body* there. It does that when it
reaches the line the function is written on. At that point the compiler's
own table of variables holds exactly what is declared above that line,
which is exactly what the function is allowed to see (decision 3.6d).
Until then the function has a number and no code, which is fine, because
nothing runs until the whole program is compiled.

## match

A pattern like `Pair(Rect(w, 2.0), Rect(_, h))` is a series of tests, and
any of them can fail after some have passed and put values on the stack
(`w` here). So every test in one `case` jumps to the same place when it
fails, and the first instruction there is `PopTo`, which drops whatever
that attempt left behind. Then the next `case` starts clean.

## Errors

```zeno
try {
    print(1 // 0)
} except ZeroDivisionError as e {
    print("caught:", e.message)
}
```

```
0000    1  Try          9
0001    2  Const        0          ; <function print>
0002    |  Const        1          ; 1
0003    |  Const        2          ; 0
0004    |  Binary       //
0005    |  Call         1
0006    |  Pop          1
0007    1  EndTry
0008    |  Jump         22
0009    |  Except       3 21       ; "ZeroDivisionError"
0010    |  GetLocal     0          ; the error
0011    |  Unwrap
0012    4  Const        0          ; <function print>
...
0021    |  Reraise
```

`Try 9` adds a **handler** to a list: "if an error is raised, go to
instruction 9". It also notes how tall the stack is and how many frames
there are. `EndTry` removes it again.

When `Binary //` raises, the VM takes the newest handler, cuts the stack
and the frames back to what the handler noted, pushes the error and jumps
(`catch` in `vm.rs`). If the error came from five calls deep, those five
frames are simply dropped. Nothing has to be passed back up through them
one at a time.

Each `except` is a test on the error's kind. `Except` jumps to the next
one if it does not fit, and after the last one `Reraise` sends the error
on its way to the next handler out.

A bug in the program (`"abc" - 1`) is not an error that can be caught. It
ends the loop and the program.

## A built-in that calls back

`[1, 2, 3].map(double)` is Rust code that has to call a Zeno function.
The loop is already running, further down the Rust stack, in the middle
of the `Call` that called `map`. So `map` starts a second loop
(`Machine::call` in `vm.rs`), which runs until `double` returns and then
hands the result back.

This is the one place a Zeno call does use the Rust stack. An error
raised inside `double` with no handler there comes back out through `map`
as a Rust `Err`, and the outer loop catches it as usual.

## A mistake the compiler can see

```zeno
print("before")
print(missing)
```

```
0004    2  Const        0          ; <function print>
0005    |  Fatal        2          ; "`missing` is not defined"
```

The compiler knows `missing` does not exist. It does not stop. It writes
a `Fatal` instruction where the name would have been read, so the program
prints `before` and then stops with the message, as it does in the
interpreter. Finding mistakes before the program runs is the checker's
job, and `zeno run --vm` runs the checker first like `zeno run` does. The
bytecode compiler's only job is to mean the same thing as the tree.

## One meaning, two engines

`Binary +` in `vm.rs` is three lines:

```rust
Op::Binary(op) => {
    let right = pop!();
    let left = pop!();
    self.stack.push(ops::binary(op, left, right, span)?);
}
```

`ops::binary` is the function the interpreter calls for `a + b`. The same
goes for fields, items, units, `raise`, `except`, argument binding and
every built-in. Before this stage that code was inside the interpreter.
It moved to `ops.rs` so that neither engine owns it.

The values are shared too (`Value` in `value.rs`). A function value is
the one thing that differs: in the interpreter it carries the scope it
was defined in, in the VM its code and its cells (`Home`).

The VM turned up one place where the interpreter itself was wrong. A
function with a name could see a variable declared *below* it in the same
block, which the checker did not expect. The VM got it right by
construction, the two disagreed, and the interpreter was fixed. That is
decision 3.6d.

## How it is tested

Every test that runs a program now runs it on both engines (`run_both` in
`vm.rs`) and requires three things to be equal: what was printed, and if
the program stopped, the message and the line and column it points at.
That covers every program in the interpreter's tests and the C
compiler's, and every file in `examples/`.

Fourteen new tests aim at what a VM can get wrong and a tree-walker
cannot:

- `break` and `continue` from inside nested blocks, a `match` and a `try`,
  two hundred times round;
- variables shared between functions, one set per trip round a loop;
- a pattern that fails after binding half its names;
- an error that crosses several frames, and one that crosses `map`;
- nine hundred nested calls abandoned by an error, fifty times, which
  must not count toward the limit afterwards;
- ten mistakes the compiler sees early, each of which must still let the
  lines above it run.

## What it does not do

- **It is not fast for a VM.** On this machine, in a release build,
  `fib(27)` takes 0.26 s in the interpreter and 0.09 s in the VM, and a
  loop of two million additions 0.70 s and 0.39 s. Three times and twice.
  A tuned VM would be ten times quicker again. Values are the
  interpreter's 32-byte `Value`, every `+` goes through the general
  `ops::binary`, and a method call makes a small object for the method
  first. Each was left alone because it keeps one meaning in one place.
- **It does not optimise.** `2 * 3` is compiled as two constants and a
  multiply. `Pop 1` twice in a row stays two instructions.
- **The bytecode is not bytes.** An `Op` is a Rust enum, 12 bytes wide.
  It cannot be saved to a file and run later (decision 5.10c).
- **It is not the default.** `zeno run` without `--vm` still uses the
  interpreter, which is the simpler statement of what Zeno means
  (decision 5.10g).

## Things to try

1. Run `zeno bytecode` on `examples/primes.zeno` and follow one trip
   round the inner loop on paper, writing the stack after each
   instruction.
2. Compile `print(2 * 3 + 4)`. The answer is known before the program
   runs. In `expr`, when both sides of a `Binary` are number constants,
   work it out with `ops::binary` and emit one `Const`. What should
   happen for `1 // 0`?
3. `x += 1` compiles to `Const, GetLocal, Swap, Binary, SetLocal`. When
   the value is a constant like `1`, nothing can tell which side was
   read first. Compile that case as `GetLocal, Const, Binary, SetLocal`
   and drop the `Swap`. Why is it not safe for `x += f()`?
4. Add an instruction `AddInt(n)` for "add the small number n to the
   top", and use it for `i + 1`. Time `fib(27)` before and after. Where
   must it give up and call `ops::binary`?
5. `items.append(x)` makes a `Value::Method` and then calls it. Add
   `CallMethod(name, count)`, which looks at the receiver and calls
   `builtins::call_method` or the class's method directly. `invoke` has
   most of what you need.
6. Print the stack after every instruction when an environment variable
   is set (`ZENO_TRACE=1`). It is ten lines in `execute`, and it is how
   every VM gets debugged.
7. The compiler puts a variable in a cell if any inner function mentions
   its *name*. Make it exact: only the variable that the inner function's
   name actually resolves to. `resolve_in` already finds it. The hard
   part is that the variable was given its place before the inner
   function was compiled.
8. `function outer() { print(inner())` / `let x = 5` /
   `function inner() { return x } }` is refused by `zeno check`, because
   `inner` is used above the `x` it needs (decision 3.6d). Skip the
   checker by writing the program in a test that calls `vm::run_both`,
   and find the instruction in `zeno bytecode` that stops it. How does
   the VM know the cell is still empty?
9. The big one: make the bytecode bytes. Give each `Op` a one-byte
   number, write operands after it, and make `execute` read from a
   `Vec<u8>`. Then write the constants and the code to a file, and add
   `zeno run program.zbc`.
