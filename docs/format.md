# The formatter

`zeno format` lays a file out the one way Zeno lays programs out.
There is nothing to configure. This page says what the layout is and
walks through the 270 lines that do it, `src/format.rs`.

```sh
cargo run -- format main.zeno          # write the file again, laid out
cargo run -- format examples           # every .zeno file in a folder
cargo run -- format --check examples   # change nothing: say what would change
```

Before:

```zeno
class  Queue{

  size:int      # how many fit


  function put(self,byte:u8)->bool{
	if self.size==0{return false}
  return true
 }
}
```

After:

```zeno
class Queue {
    size: int      # how many fit

    function put(self, byte: u8) -> bool {
        if self.size == 0 { return false }
        return true
    }
}
```

The decisions are 9.1 to 9.9 in [`decisions.md`](decisions.md).

## The layout

1. **Indentation.** Four spaces for each line above that left a `{`,
   `(` or `[` open. A line that begins by closing brackets goes back
   to where they were opened. A statement that goes on after an
   operator is indented once more.
2. **Spaces.**

   | Rule | Example |
   |---|---|
   | one space around an operator, `=` and `->` | `a + b`, `x = 1`, `-> int` |
   | one space after `,` and `:`, none before | `f(a, b)`, `port: int`, `["a": 1]` |
   | none inside `( )` and `[ ]` | `(a + b)`, `[1, 2]` |
   | none before the `(` of a call or the `[` of an index or a type | `print(x)`, `items[0]`, `list[int]` |
   | none around `.` | `shapes.area` |
   | one inside `{ }` on one line, and before `{` | `if ready { go() }` |
   | a `-` that is a sign holds on to its number | `-1`, `a - -1` |
   | none around the `=` that names an argument | `Point(x=3.0, y=4.0)` |
3. **Blank lines.** Never two in a row. None at the top or the bottom
   of a block or of the file.
4. **Comments.** A space after the `#`. A comment after code keeps the
   distance the author gave it, and has at least two spaces.

And three things the formatter never does:

- It never changes a token. `0x4000_4400` stays spelled so, and `'it'`
  keeps its quotes.
- It never moves a line break. `case Off { return 0 }` stays on one
  line, and a call you broke over three lines stays on three (9.3).
- It never formats a file that has a mistake. The mistake is shown,
  as `zeno check` shows it.

## How it works

Most formatters print the program again from its syntax tree. Zeno's
does not look at the tree at all. It takes the **tokens** of the file,
keeps each one as it is and where it is, and decides only two things:
how far each line is indented, and whether there is a space between
two tokens that stand side by side.

That is why it is short, and why it is safe. No token is added,
removed or moved, so the program cannot come to mean something else.

### What the lexer keeps

The lexer used to throw away two things the formatter needs: the
comments, and how each token was spelled (`Int(255)` does not say
whether the file had `255` or `0xFF`). `lexer::lex_written` now gives
both back:

```rust
pub struct Written {
    pub tokens: Vec<Token>,
    pub texts: Vec<String>,              // each token as it stands in the source
    pub comments: Vec<(Span, String)>,   // each comment, and where it is
}
```

The lexer already knew where each token began. It now notes where it
ended, and the text between is the token as written.

### Lines

Every token knows the line it is on (`span.line`), so the tokens and
the comments are sorted into lines:

```rust
struct Line<'a> {
    tokens: Vec<usize>,         // places in Written::tokens
    comment: Option<&'a str>,
    before_comment: usize,      // how far the comment was from the code
}
```

A line with no tokens and no comment is a blank line.

### Indentation

The formatter walks down the lines and keeps a stack of the brackets
that are open. For each one it remembers how far the line that opened
it is indented:

```rust
struct Open {
    brace: bool,       // `{`, or else `(` or `[`
    indent: usize,     // of the line that opened it
}
```

Then the indentation of a line is one of two things (shortened
here):

```rust
let indent = if closes {
    // goes where the line that opened the bracket is
    open[open.len() - closed].indent
} else {
    open.last().map_or(0, |bracket| bracket.indent + 1) + usize::from(goes_on)
};
```

Remembering the *line's* indent, and not counting brackets, is what
makes this come out right:

```zeno
each(items, function(item) {
    print(item)
})
```

The first line opens `(` and `{`. Both remember indent 0. The body is
inside the `{`, so it gets 0 + 1. The last line begins with closers,
so it goes back to 0. Counting brackets would have indented the body
twice.

`goes_on` is true when the line of code above ended in a token that
cannot end a statement, such as `+` or `and`. The lexer has the same
question to answer, to know that the statement goes on, and the
formatter asks it the same way: `TokenKind::continues_line`.

### Spaces

`space_between` is asked about every two tokens side by side on a
line, and answers yes or no. It is a short list of exceptions and
then "yes" (the named argument of 9.6 is left out here):

```rust
if matches!(after, RParen | RBracket | Comma | Colon | Dot) { return false; }
if matches!(before, LParen | LBracket | Dot | Tilde) { return false; }
if matches!(before, Minus | Plus) && !earlier.is_some_and(ends_a_value) { return false; }
match after {
    LParen => !(ends_a_value(before) || *before == Function),
    LBracket => !ends_a_value(before),
    _ => true,
}
```

The one idea in it is `ends_a_value`: can a value end with this
token? A name can, and a number, a text, `)` and `]`. After such a
token a `-` subtracts, a `(` calls and a `[` picks an item. After
anything else (an operator, a keyword, a comma, the start of a line) a
`-` is a sign, a `(` groups and a `[` begins a list. Tokens alone are
enough to tell, and no syntax tree is needed.

### The check

```rust
let again = lexer::lex_written(&formatted)?;
if kinds(&again) != kinds(&written) || comments(&again) != comments(&written) {
    return Err(... "would have changed what this file means" ...);
}
```

The formatter lexes what it is about to write and compares. If a
space it took out glued two tokens into one, or one it put in split a
token, the lists differ, and the file is left alone.

### The tests

Three properties are tested on every file in `examples/`:

- **It is laid out already**: formatting it gives the same file.
- **Indentation comes from the brackets alone**: with every line's
  indentation taken away, formatting gives the same file.
- **Spaces come from the tokens alone**: with three spaces between
  every two tokens, formatting gives the same file.

So the examples are both the test of the formatter and the proof that
its rules are the ones Zeno code has always been written by.

## What is left out

- **No joining or splitting of lines**, and so no line width (9.3).
- **Comments after code are not lined up for you.** The distance you
  gave them is kept (9.8).
- **`zeno.package` is not formatted.** It is not a program.
- **A file with a mistake is not formatted**, not even the part above
  the mistake.
- **Editors are not set up yet.** A language server, which would
  format on save and show mistakes as you type, is the next stage.

## Things to try

1. Copy `examples/shapes.zeno` somewhere, take all the indentation
   out of the copy (in most editors: select all, then Shift-Tab a few
   times), and run `zeno format` on it. Compare with the original.
2. In the copy, write `let x=-1` and `let y=x-1`, and format. Find the
   line of `space_between` that made the two `-` come out differently.
3. Write `print (x)` and `return(x)` inside a function, and format.
   One loses a space and one gains one. Which rule does each?
4. Write a call over several lines with a function in it, as in the
   `each(items, function(item) {` example above, and indent it
   wrongly. Format it. Then change `bracket.indent + 1` in
   `src/format.rs` to count brackets (`open.len()`) and run
   `cargo test format`. Which test fails, and how does the output
   look?
5. Put three blank lines between two functions, and one straight
   after a `{`. Format. Then read rule 9.7.
6. Line up the comments of three fields in a class by hand, format,
   and see that they stay. Then put only one space before one of
   them and format again.
7. Make a mistake in the copy (take out a `}`) and format it. What is
   shown, and was the file changed?
8. Run `cargo run -- format --check examples` and look at the exit
   status (`echo $?`). Now add a space at the end of a line in one
   example and run it again. Undo the change with `zeno format`.
9. In `space_between`, take `Colon` out of the first list and run
   `cargo test format`. Read what `every_example_is_laid_out_this_way`
   says. Which example caught it?
10. The formatter keeps `0xFF` as it is. Find where the text of a
    token comes from (`Lexer::push` in `src/lexer.rs`), and say why
    printing `Int(255)` from the tree could not have kept it.
