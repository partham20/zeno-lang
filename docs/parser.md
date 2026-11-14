# How the parser works

The parser is the second stage. The lexer gave us a flat list of tokens; the
parser works out how they nest and builds a **syntax tree**.

```
let total = 1 + 2 * 3

tokens:  Let  Ident("total")  Assign  Int(1)  Plus  Int(2)  Star  Int(3)

tree:    Let total
           └─ Binary +
                ├─ Int 1
                └─ Binary *
                     ├─ Int 2
                     └─ Int 3
```

The tree is where meaning starts to appear. The tokens do not say that `2 * 3`
happens before the `+`; the tree does.

See it for any file. Blocks are shown by indentation, and every operator gets
its own brackets so you can see how it grouped:

```sh
cargo run -- parse examples/tour.zeno
```

## The files

- [`src/ast.rs`](../src/ast.rs): the shapes a tree node can have. Read this first.
- [`src/parser.rs`](../src/parser.rs): the parser and its tests.
- [`src/printer.rs`](../src/printer.rs): prints a tree back out as text.
- [`src/error.rs`](../src/error.rs): the error type the lexer and parser share.

## Statements and expressions

Everything in the tree is one of two things:

- A **statement** does something: `let x = 1`, `if ... { }`, `return x`. See `StmtKind`.
- An **expression** produces a value: `1 + 2`, `area(shape)`, `user.name`. See `ExprKind`.

Statements contain expressions (`return` holds the value to return), and some
statements contain blocks of more statements (`if` holds its body).

## Recursive descent

The parser has one function per piece of grammar, and each function parses
exactly that piece. The grammar rule for `while` is "the word `while`, a
condition, a block", and the code says the same thing:

```rust
fn while_loop(&mut self) -> Result<StmtKind, Error> {
    self.advance(); // while
    let condition = self.expression()?;
    let body = self.block()?;
    Ok(StmtKind::While { condition, body })
}
```

`block` calls `statement`, which may call `while_loop`, which calls `block`
again. That circle is the "recursive" in recursive descent, and it is why
nesting to any depth just works.

`statement` is the hub. It looks at one token and picks the function:
`let` goes to `declaration`, `if` to `if_statement`, and so on. Anything else
must be an expression or an assignment.

The same few helpers as the lexer do the reading, on tokens this time:
`peek`, `advance`, `eat(kind)`, plus `expect(kind)`, which consumes a token or
fails with "expected X, found Y".

## Precedence without a table

How does `1 + 2 * 3` become `1 + (2 * 3)`? Each precedence level is its own
function, and each one calls the next tighter level to get its operands:

```
expression → or → and → not → comparison → sum → product → negation → power → postfix → primary
  loosest                                                                          tightest
```

`sum` asks `product` for its left side. `product` consumes `1`, sees a `+`,
which is not its business, and returns. `sum` takes the `+`, then asks
`product` for the right side. This time `product` consumes `2 * 3` whole
before returning. So the multiplication ends up lower in the tree, and things
lower in the tree happen first.

A loop inside each level makes operators group to the left: `a - b - c` is
`(a - b) - c`. Power is the exception. It recurses on its right side instead
of looping, so `2 ** 3 ** 2` is `2 ** (3 ** 2)`.

## Places where one token is not enough

Mostly the next token decides everything. A few spots need a little more:

- **Assignment or expression?** `x = 1` and `x.go()` both start with an
  expression. `assignment_or_expression` parses the expression first, then
  checks whether `=` follows.
- **List or comprehension?** `[a, b]` and `[a for a in b]` start the same.
  `list` parses the first element, then checks for `for`.
- **Named argument?** In a call, a name followed by `=` is `name=value`. This
  is the one place the parser peeks two tokens ahead.
- **`is not` and `not in`** are each two tokens that make one operator.

## f-strings

The lexer hands over `f"Hello, {who}!"` as one piece of text. `format_parts`
cuts out each `{...}` and runs the lexer and parser on it as a tiny program of
its own, so anything that works in normal code works inside the braces.

## Errors

The parser stops at the first error, because after one mistake everything
that follows tends to be noise. Messages say what was expected and what was
found, in words rather than token names, and common habits get a hint:

```
error: expected `{`, found `:`. Zeno uses `{ }` around blocks
 --> main.zeno:2:13
  |
2 | if total > 5:
  |             ^
```

## What the parser leaves for later

The parser only checks shape. All of these parse without complaint:

- `let x = y` where `y` was never declared.
- `"abc" - 1`.
- `return` outside a function.
- `case Idle`, which could be an enum case or a new variable. The tree stores
  the bare name and a later stage decides.

Those are jobs for name resolution and the type checker.

## Things to try

1. Add a `loop { }` statement that repeats forever. (A keyword in the lexer,
   a `StmtKind`, a branch in `statement`, a line in the printer, a test.)
2. Add `x if cond else y` as an expression. Where does it go in the precedence chain?
3. Let enum cases be separated by commas as well as line breaks.
4. Make the parser carry on after an error and report several. After a
   failure, skip tokens until the next `Newline` and try again. What makes
   this harder inside a block?
