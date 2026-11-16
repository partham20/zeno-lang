# The playground: Zeno in a page, with nothing installed

```sh
zeno playground        # writes playground.html: open it in a browser
```

`playground.html` is one file. On the left is the program, on the
right is what it does, and Run (or Ctrl+Enter, ⌘+Enter on a Mac) runs
it. Nothing is installed and nothing is sent anywhere: the program is
checked, formatted and run in the browser, by the same Zeno that
`zeno run` is.

It is the page to send to somebody who asks what Zeno looks like. It
is on the website at zeno-lang.org/playground.

## What is in it

| | |
|---|---|
| **main.zeno** | the program, in colour, with line numbers. Tab and Shift+Tab indent and outdent by four spaces, Enter keeps the indent and steps in after `{`, `}` steps out, Ctrl+/ turns lines into comments and back |
| **page.html** | the page the program is in, for a program that says `import web`. Left empty, or for a program without `import web`, the page is one that shows what the program prints |
| **Checking as you write** | the program is checked a moment after each change, as `zeno check` would. A line with a problem is marked, and the foot of the editor says how many there are |
| **Problems** | each problem, with its message. Clicking one puts the cursor on its line |
| **Examples** | fifteen programs from `examples/`, in four groups: Start here, The language, Firmware, Pages |
| **Run** | run the program in a fresh frame. What it prints is shown, with the error that stopped it under the last line; the line of that error is marked too |
| **C** | the program as `zeno build` writes it in C, made again as you write. Choose the build (`zeno build`, `--firmware`, `--no-collector`) to see how each one changes the C. The runtime, the 2,700 or so lines every program starts with, is hidden unless asked for; Copy takes the whole file, which a C compiler builds as it is |
| **Format** | lay the program out as `zeno format` does (Shift+Alt+F). Undo takes it back |
| **Share** | copy a link that opens the playground with this program in it (also Ctrl+S) |
| **Light or dark** | the same choice as on the website, which it shares |

What was written is kept in the browser, and is there on the next
visit. A shared link wins over what was kept.

On a phone one half is shown at a time, with a switch between Code
and Result, and Run shows the result. The halves can be dragged wider
or narrower on a larger screen.

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

Share writes the program into the address of the page, after the `#`:
the program and its page as JSON, in base64. Opening that address
opens the playground with that program. The part after `#` is never
sent to a server, so a link is the whole program and nothing keeps
a copy.

### A second Zeno checks and formats

Checking as you write, and Format, need no frame and no page. The
playground starts the same module once more, as its own helper, with
a page that answers nothing, and calls two things `zeno.wasm` has for
it (`src/web_host.rs`):

- `zeno_check(text)`: load and check the program as `main.zeno`.
  Each mistake comes back as an error, in the same words as on a
  desk, and the playground reads the line out of `--> main.zeno:2:7`.
- `zeno_format(text)`: the program as `zeno format` lays it out, or
  its mistakes.
- `zeno_c(text, build)`: the program in C, as `zeno build` writes it
  (0), with `--firmware` (1) or with `--no-collector` (2), and how
  many of its first lines are the runtime. A program that cannot be
  built gives its mistakes instead, as `zeno build` would.

Checking never runs the program, so it cannot loop, and it takes a
few milliseconds. The colours are not Zeno's: they come from a short
list of words in the page, because colouring has to keep up with
every key, and a word that is coloured wrongly harms nothing.

### How it is written

`zeno playground` is a few lines in `src/main.rs`
(`playground_page`). The page is `src/playground.html`, with three
places marked where it writes things in: `src/web.js`, `zeno.wasm`
and the examples. Each is written as JSON, with `<`
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

And the redesign (20.4): each of the fifteen examples was opened from
the menu and ran with no problem and no error; a program was typed
key by key, and the indent after `{` came by itself; a line with a
problem was marked while typing, and clicking the problem put the
cursor on it; Format laid out a program and Undo took it back; a
runtime error marked its line; a shared link and a reload each
brought the program back; the page of the list of things to do was
changed in its tab and ran; and the halves were dragged. On a phone
(iPhone 13): the menu, Run showing the result, and boxes in 16px type.

## What it does not do

- One file. No `import` of a file of your own.
- No completion and no hover. `zeno lsp` does that in an editor
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
6. The colours come from a list of words in the page, not from
   `src/lexer.rs`. Write a program that the page colours wrongly.
   What would it take to ask `zeno.wasm` for the tokens instead,
   and why might that be too slow to do on every key?
