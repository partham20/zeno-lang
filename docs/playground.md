# The playground: Zeno in a page, with nothing installed

```sh
zeno playground        # writes playground.html: open it in a browser
```

`playground.html` is one file. On the left is a box to write a
program in, on the right is what the program does, and Run (or
Ctrl+Enter) runs it. Nothing is installed and nothing is sent
anywhere: the program is checked and run in the browser, by the same
Zeno that `zeno run` is.

It is the page to send to somebody who asks what Zeno looks like.

## What is in it

| | |
|---|---|
| **Program** | the program. Tab indents by four spaces, as `zeno format` does |
| **Page (HTML)** | the page the program is in, for a program that says `import web`. Left empty, the page is one that shows what the program prints |
| **Example** | a few of the programs in `examples/`, to start from. Two of them are pages: the list of things to do, and the sheet to draw on |
| **Run** | check the program and run it. A mistake the checker finds is shown where the output would be, with its line |
| **Copy link** | a link that opens the playground with this program in it |

The program is one file. A program in several files, and a package,
need `zeno build --web` ([`web.md`](web.md)).

## How it works

[`web.md`](web.md) says how a program runs in a page: `zeno.wasm` is
Zeno itself, built for a browser, and `src/web.js` starts it with the
program's text. A page that `zeno build --web` writes does that once,
for one program. The playground does it again for every press of Run.

### Each run gets a new page

A Zeno program in a page does not end. It runs to its last line, and
then its functions are called when buttons are pressed and timers
run out. So to run the next program, the last one has to be got rid
of, with its timers, the functions it gave to buttons, and whatever
it did to the page.

The playground does not try to undo any of that. What the program
does is shown in a **frame**: a page inside the page (`<iframe>`).
Run throws the frame's page away and gives it a new one, written
then and there (`srcdoc`):

```
the page you wrote (or the empty one)
+ <script> src/web.js </script>
+ <script> Zeno.start(parent.zenoModule, {the program}) </script>
```

That is what `zeno build --web` writes into a file, less `zeno.wasm`.
The browser throws away the old page with everything that belonged
to it, which is the one way to stop a program that is certain to
have stopped it (16.12).

### Zeno is read in once

`zeno.wasm` is in the playground packed, as it is in a built page.
Unpacking it and having the browser read it in takes a moment, and
is done once, when the playground opens:

```js
window.zenoModule = Zeno.unpacked(PACKED).then((bytes) => WebAssembly.compile(bytes));
```

What that gives is a **module**: Zeno, ready to be started. Starting
it is quick, and each run starts it afresh, so no run can see what
the one before left in memory. The frame finds the module in the
page around it (`parent.zenoModule`).

### A link holds the program

Run writes the program into the address of the page, after the `#`:
the program and its page as JSON, in base64. Opening that address
opens the playground with that program. The part after `#` is never
sent to a server, so a link is the whole program and nothing keeps
a copy.

### How it is written

`zeno playground` is a few lines in `src/main.rs`
(`playground_page`). The page is `src/playground.html`, with four
places marked where it writes things in: `src/web.js`, `zeno.wasm`,
the examples and the empty page. Each is written as JSON, with `<`
written as `\u003c`, for the reason a built page does it: a browser
reads the end of a script wherever it finds one, even in the middle
of a str.

`zeno playground` looks for `zeno.wasm` where `zeno build --web`
does.

## How it was tried

In Chromium, driven by a script: the first example ran when the page
opened; a program was typed and run with Ctrl+Enter; a program with
a mistake showed the checker's message; a program that prints
`</script>` and text in Tamil printed them; both page examples
worked inside the frame (a thing was added to the list); and the
link was opened in a new page, which ran the same program.

Again before it went on zeno-lang.org (20): a mouse drew ten lines
on the sketch, `while true { }` was stopped after 5 seconds with its
line, an error after some output showed under the output, and the
page was tried at the size of a phone (iPhone 13), where a thing was
added to the list by tapping, and in the dark.

## What it does not do

- One file. No `import` of a file of your own.
- No colours in the program text, and nothing is underlined as you
  type. The box is a plain box. `zeno lsp` does that in an editor
  ([`lsp.md`](lsp.md)).
- There is no Stop button. A program that never stops
  (`while true { }`) holds the page for 5 seconds, and is then
  stopped with the line it was on (20.1).
- A long program makes a long link.

## Things to try

1. Run `zeno playground`, open the page, and write a program that
   prints the first ten squares. Then copy the link, open it in a
   new window, and change the program there. Which window has which
   program, and why?
2. Choose "A page: things to do". Change the page (the second tab)
   so that the list is numbered. What did you change, and did the
   program need to know?
3. Write `while true { }` and run it. It is stopped after 5 seconds:
   find `tick` in `src/interpreter.rs`. What would the playground
   need for a Stop button that works at once? (Look up what a Worker
   is, and ask what a program in one cannot reach.)
4. Find `run` in `src/playground.html`. It builds the frame's page
   as text. What would go wrong if the program were written into it
   without `quoted`? Try a program that prints `</script>`.
5. Add an example to the list in `playground_page` (`src/main.rs`).
   Which test tells you that the page is still one script?
6. The box has no colours. `src/lexer.rs` already knows which words
   are keywords. How could the playground ask `zeno.wasm` for the
   tokens of the program, and what would it draw them on?
