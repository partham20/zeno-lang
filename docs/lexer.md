# How the lexer works

The lexer is the first stage of the compiler. It turns a string of characters
into a list of **tokens**, the words of the language. Every later stage works
with tokens and never looks at raw characters again.

```
let count = 3     ->     Let  Ident("count")  Assign  Int(3)  Newline  Eof
```

Try it on any file:

```sh
cargo run -- tokens examples/hello.zeno
```

## The files

- [`src/token.rs`](../src/token.rs): the list of token kinds, and the keyword table.
- [`src/lexer.rs`](../src/lexer.rs): the lexer and its tests.

## The main loop

`Lexer::run` is one loop. Each time round it looks at the next character,
without consuming it, and that one character is enough to decide what kind of
token starts here:

| Next character | What it starts | Handled by |
|---|---|---|
| space, tab | nothing, skip it | |
| line break | maybe the end of a statement | `end_statement` |
| `#` | a comment, skipped to the end of the line | `comment` |
| `"` or `'` | a string | `string` |
| `f` followed by a quote | a formatted string | `string` |
| a digit | a number | `number` |
| a letter or `_` | a keyword or a name | `word` |
| anything else | punctuation or an operator | `symbol` |

Each handler consumes exactly the characters of its token and pushes one
token. Then the loop goes round again.

Three small helpers do all the reading: `peek` looks at the next character,
`advance` consumes it (and keeps the line and column counters right), and
`eat(c)` consumes it only if it is `c`. Two-character operators fall out of
`eat`: after reading `=`, `eat('=')` tells you whether it was `==` or `=`.

## Keywords are just names in a table

`word` reads letters, digits and underscores, then asks `keyword()` whether the
result is reserved. `function` becomes the `Function` token; `functions` is an
ordinary name. Adding a keyword to Zeno is one line in the enum and one in the table.

## Line breaks instead of semicolons

This is the only subtle part. A line break ends a statement, so the lexer
emits a `Newline` token for it. But not every line break ends a statement, so
`end_statement` stays quiet in three cases:

1. **Inside `( )` or `[ ]`.** The lexer counts open brackets in `depth`. While
   it is above zero, line breaks are ignored, so long calls and lists can wrap.
2. **After a token that cannot end a statement**, such as `+`, `,`, `=` or `and`
   (see `TokenKind::continues_line`). The statement carries on to the next line.
3. **After another `Newline`, or at the top of the file.** Blank lines and
   comment-only lines produce nothing.

The parser therefore sees exactly one `Newline` per statement and never has to
think about blank lines.

## Errors

The lexer does not stop at the first problem. It records each one with its
line and column, skips the bad character, and carries on, so you see every
mistake in one run. Habits from other languages get a hint:

```
error: Zeno has no semicolons. A line break ends a statement
 --> main.zeno:1:10
  |
1 | let x = 1;
  |          ^
```

## What the lexer leaves for later

- **Formatted strings.** `f"Hello, {who}"` becomes one string token marked
  `formatted`. Splitting out the `{who}` part is the parser's job.
- **Units.** `500 ms` is two tokens, a number and a name. The parser joins them.
- **Negative numbers.** `-5` is `Minus` then `Int(5)`.
- **Meaning.** The lexer is happy with `let let = = 3`. Deciding that it makes
  no sense is the parser's job.

## Things to try

Good exercises for getting to know the code:

1. Add a `%=` operator. (One enum line, one branch in `symbol`, one test.)
2. Add an `unless` keyword.
3. Allow multi-line strings in triple quotes, `"""like this"""`.
4. Make `0x` with no digits after it say "expected digits after 0x".
