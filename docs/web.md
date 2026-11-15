# How a Zeno program runs in a web page

`examples/todo.zeno` is a list of things to do. You type one, press
Enter, and it joins the list. A click marks it done. It is a page a
browser opens, and what the page does is written in Zeno:

```
import web

let form = web.element("#add")
let box = web.element("#what")
let list = web.element("#list")
let left = web.element("#left")

var remaining = 0

function add(what: str) {
    let item = web.create("li", what)
    item.on("click", function() {
        if item.has_class("done") {
            item.remove_class("done")
            remaining += 1
        } else {
            item.add_class("done")
            remaining -= 1
        }
        show_remaining()
    })
    list.append(item)
    remaining += 1
    show_remaining()
}

function sent() {
    let what = box.value.strip()
    if what != "" {
        add(what)
        box.value = ""
    }
}

form.on("submit", sent)
```

```sh
zeno build --web examples/todo.zeno    # writes todo.web.html: open it in a browser
zeno run examples/todo.zeno            # the same program, on a simulated page
zeno test examples/todo.zeno           # its tests, on the simulated page
```

There are two files to write. `todo.html` is the page: a form, an
empty list and a line of text, in ordinary HTML and CSS, with no
script in it. `todo.zeno` is the program. `zeno build --web` puts the
two together into `todo.web.html`, which is one file that needs
nothing else: open it from the disk, mail it, or put it on a web
server.

This page is about what the program can ask of the page, and then
about how a language written in Rust comes to be running in a
browser.

## The `web` module

| | |
|---|---|
| `web.element("#count")` | the first element that answers to the selector. A `ValueError` if the page has none |
| `web.elements("li")` | every element that answers to it, as a list. The list may be empty |
| `web.create("li", "Buy milk")` | a new element with that text in it. It is not on the page until it is appended to one that is |
| `web.after(500 ms, tick)` | call `tick` once, that much later. The time must have a unit |
| `web.on_key(pressed)` | call `pressed` with the name of each key that is pressed: `"a"`, `"Enter"`, `"ArrowLeft"` |

A selector is what CSS calls one: `#count` is the element with that
id, `.done` every element of that class, `li` every list item, and
`#list li.done` what it looks like.

And what an element can do:

| | |
|---|---|
| `e.text` | the text in it. Give it a new value to change what the page shows |
| `e.value` | what has been typed into it, for a box that can be typed into |
| `e.on("click", pressed)` | call `pressed` whenever that happens to it: `"click"`, `"input"`, `"submit"`, any event of the browser |
| `e.append(child)` | `child` becomes the last thing inside it |
| `e.remove()` | it leaves the page |
| `e.clear()` | everything inside it leaves the page |
| `e.add_class("done")`, `e.remove_class("done")`, `e.has_class("done")` | its classes, which is how a program changes how something looks: the look itself is CSS, in the page |
| `e.show()`, `e.hide()` | whether it is shown at all |
| `e.get("href")`, `e.set("href", "/about")` | an attribute. `get` gives `none` for one it does not have |

`Element` is a type, so a function can ask for one, and a list can
hold them:

```
function mark(item: Element) {
    item.add_class("done")
}

for item in web.elements("li") {
    mark(item)
}
```

Three things about it were decided for a reason, and the reasons are
in [`decisions.md`](decisions.md), chapter 14.

**An element that is not there is an error.** `web.element("#cuont")`
stops the program and says the page has no such element. It does not
give `none`, because nearly every program would then write
`if count is not none` around everything, to guard against a spelling
mistake that the first run finds anyway. `web.elements` is the one to
ask when there may be none (14.3).

**A function that the page calls takes nothing.** There is no event
object. The function already knows which element it is for, because
it was written where that element was in reach: `item` in the
example above belongs to the function that was given to `item.on`.
The one thing an event carries that a program cannot get another
way is which key was pressed, so `on_key` hands that over (14.5).

**`text` and `value` are fields, and the rest are methods.** They
are the two things a program both reads and writes, and `count.text
= "3"` says that as a variable does (14.4).

## The page you write, and the page that is built

`zeno build --web todo.zeno` looks for `todo.html` beside the
program. If it is there, it is the page, and the program is written
into the end of its body. If there is none, the page is an empty one
that shows what the program prints, so every program that runs under
`zeno run` and reads no file can be put in a page:

```sh
zeno build --web examples/primes.zeno     # primes.web.html prints the primes
```

`print` goes to the browser's console, and into the element with the
id `zeno-output` if the page has one. Put `<pre id="zeno-output">`
into your own page to see it there.

An error that nothing catches stops the program, as it does under
`zeno run`, and is shown in a box at the foot of the page with the
line it happened on:

```
error: uncaught IndexError: index 7 is out of range for a list of length 3
  --> late.zeno:13:15
   |
13 |         print(numbers[7])
   |               ^
```

After that the page calls nothing more. A program that has gone
wrong in one function has variables that are half changed, and
pressing the next button would run on from there (14.10).

The mistakes the checker finds are found by `zeno build --web`,
before any page is written.

## On a desk: the simulated page

`zeno run` has no browser. So, as with the board
([`board.md`](board.md)), the page is a **simulated** one. It has
every element a program asks for, and prints a line for each change
that somebody looking at the page would see:

```
$ zeno run examples/todo.zeno
#list + <li> Read docs/web.md
#left text: 1 thing left
#list + <li> Build this page
#left text: 2 things left
```

An element is named by the selector it was asked for, or by its tag
if the program made it. A change to an element that is not on the
page yet prints nothing, and neither does writing what is already
there.

Nobody presses a button on a simulated page, and no time passes, so
it calls no function it was given. The tests do that:

```
test "an empty box adds nothing" {
    let before = remaining
    box.value = "   "
    sent()
    assert remaining == before
}
```

This is what the simulated page is for. What a page does can be
tested without a browser, in the same run as every other test, and
the two engines (`zeno run` and `zeno run --vm`) are held to the same
lines.

There are two things it cannot tell you.

It cannot say whether the page has the elements the program asks
for. It has no HTML to look in, so it has them all.
`web.element("#cuont")` works on a desk and stops the program in the
browser.

And it reads only plain selectors: a tag, `.class` and `#id`,
together (`li.done`) and one inside another (`#list li`). For
`web.elements("ul > li")` it stops the program and says that a
browser can read this and it cannot. An empty list would have said
the page has none, which it does not know (14.6).

## How it works

A browser runs two things: JavaScript, and **WebAssembly**, which is
a small instruction set that compilers can aim at as they aim at a
processor. Rust can. So `zeno.wasm` is the Zeno you already have
(the lexer, the parser, the checker and the interpreter) compiled
for a browser in place of a desktop:

```sh
cargo build --release --target wasm32-unknown-unknown
```

Nothing of Zeno is rewritten for it. That is the decision this whole
stage rests on (14.1): a program in a page is checked by the same
checker and run by the same interpreter as on a desk, so it means
the same.

The built page holds three things:

1. **The program**, as the text you wrote, one entry for each file.
2. **`zeno.wasm`**, written out in base64, because a page is text.
3. **`src/web.js`**, two hundred lines of JavaScript that start the
   one with the other.

### The line between them

WebAssembly can do arithmetic and call functions, and nothing else.
It cannot print, read a clock or touch a page. Whatever it needs of
the world it must be handed, as functions, when it starts. And the
only things that cross are numbers.

So there are two short lists. `src/web_host.rs` has both, and it is
the only file that is built for the browser alone.

What the page can call in `zeno.wasm`:

| | |
|---|---|
| `zeno_alloc(length)` | room for that many bytes |
| `zeno_start(place, length)` | check and run this program |
| `zeno_call(handler)` | call a function the program gave to `on` or `after` |
| `zeno_call_with(handler, place, length)` | the same, with one str: the key |

What `zeno.wasm` can call in the page:

| | |
|---|---|
| `zeno_host_print(place, length)` | show this text |
| `zeno_host_error(place, length)` | show this error. The program has stopped |
| `zeno_host_page(place, length)` | a question about the page, or a change to it |
| `zeno_host_input(place, length)` | ask the person for a line |
| `zeno_host_now()` | the clock |

**Text** crosses as two numbers: a place in the program's memory and
a length in bytes. The page can read and write that memory, so to
hand text over, it asks for room (`zeno_alloc`), writes the bytes
there, and passes the place.

**An element** crosses as one number. `web.js` keeps a table that
says which element of the page each number is, and Zeno keeps the
number in a `Value::Element`, as a pin is a number. Zeno never holds
anything of the browser's.

**A request** to the page is a line of JSON, and so is the answer:

```
["element", "#count"]            ->  0
["set_text", 0, "3 presses"]     ->  null
["has_class", 4, "done"]         ->  false
["on", 2, "click", 0]            ->  null
```

One function, `zeno_host_page`, carries them all. That keeps the
line between the two at five functions however many things the `web`
module learns to do: a new one is a new name in `ops` in `web.js`
and a few lines in `src/page.rs`, and nothing else moves.

**A function** crosses as a number too. `button.on("click", pressed)`
keeps `pressed` in a list in Zeno and tells the page its place in
that list. When the button is pressed, the page calls
`zeno_call(0)`.

### A program that has ended, and is called

Under `zeno run` a program runs from its first line to its last and
is over. In a page it runs to its last line and then the page is
still there, and somebody presses a button. The function has to run
with every variable as the program left it.

So `zeno_start` does not throw the interpreter away. It keeps it,
and the scope with the program's variables, in a `Session`
(`src/interpreter.rs`), and `zeno_call` runs a function in it. The
test `a_page_calls_a_function_of_the_program_later` does by hand what
a browser does.

A browser does one thing at a time, and so does a Zeno program. If
something calls while the program is in the middle of a function,
`web.js` puts the call in a queue (`waiting`) and makes it when the
function has returned.

### Two things a page cannot do

**`sleep`.** A browser has one thread for a page. A program that
waits on it stops the page from drawing and the buttons from
working. `time.sleep` in a page is an error that says to use
`web.after(500 ms, ...)`, which lets the page go on in between
(14.11).

**Deep recursion.** The interpreter calls itself for every nested
call of the program, and in a browser it does so on the browser's
stack, which is small. Chromium gave out between 400 and 1000 nested
calls, by how much each one holds. A program stopped by the browser
cannot be told which line did it, so Zeno counts, and stops at 200
with its own message (14.12). On a desk the limit is 1000.

## Building `zeno.wasm` yourself

A release of Zeno has `zeno.wasm` beside `zeno`. From the source:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
cargo build --release
target/release/zeno build --web examples/todo.zeno
```

`zeno build --web` looks for `zeno.wasm` where the variable
`ZENO_WASM` says, then beside `zeno`, then where the first of those
commands leaves it, which is why the last one finds it.

`.cargo/config.toml` asks for eight megabytes of stack in the
WebAssembly build, for the part of the stack that Rust keeps in the
program's own memory.

## How it was tried

The tests run on the simulated page, because a browser is not part
of `cargo test`. The real page was tried by hand in Chromium, driven
by a script: the to-do list (typing, Enter, the button, clicks,
Escape, text in Tamil), a counter with a timer and keys, `input`,
a program in several files with a package
(`examples/thermostat`, which prints the same in the browser as
under `zeno run`), an error in a function called by a timer, and
recursion past the limit.

## What it does not do yet

- **Only what is in the table.** No style of one element (use a
  class), no place of the mouse, no drawing, no network, nothing
  kept between visits. Each is a name in `ops` and a few lines in
  `page.rs` away.
- **It is an interpreter in a page.** `zeno.wasm` is 1.1 MB, and
  every built page carries it (1.5 MB as base64). A program runs at
  the speed of `zeno run`, which is plenty for a page that waits for
  clicks and not for a game that draws sixty times a second.
- **The program is in the page as you wrote it**, comments and all.
  Anyone who opens the page can read it.
- **One browser was tried**: Chromium. Nothing in `web.js` is new to
  any browser of the last five years, and that is not the same as
  having run it.
- **`input` uses the browser's own question box**, which is ugly
  and stops the page while it is open. A page should have a box of
  its own and read its `value`.

## Things to try

1. Build `examples/todo.zeno` and open it. Then run it with `zeno
   run`. Which lines of the simulated page match what you saw happen?
2. Spell `#list` wrong in the program. What does `zeno run` say?
   What does the page say? Why can the checker not find this, and
   what would it need to be given to find it?
3. Add a button to `todo.html` that removes every thing that is
   done. You need `web.elements` and one more thing to count. Write
   the test first.
4. `add` gives each item its own function. Open `todo.web.html`,
   add ten things, and think about what Zeno is keeping for each.
   Where does it keep them (`Page::keep`)? When are they given back?
5. Add `e.focus()`, which puts the cursor in a box. It is a name in
   `ops` in `src/web.js`, a method in `src/page.rs` and
   `src/builtins.rs`, and a line in the checker. What should the
   simulated page print?
6. Find `zeno_host_page` in `src/web.js`. A selector the browser
   cannot read, such as `li[`, makes it throw. What does the program
   get? Is that the right answer for `web.element`? For
   `web.elements`?
7. `print` adds to `#zeno-output` each time. Write a program that
   prints a thousand lines and watch the page. Then change `print`
   in `web.js` so that the element is found once.
8. A big one: an event that carries something. `canvas.on("click",
   ...)` is no use without knowing where the click was. How should
   the function get it: two parameters, an object, or a question it
   can ask (`web.mouse()`)? What does each do to the rule that a
   function the page calls takes nothing?
9. A bigger one: the built page carries the whole checker, and the
   program was checked before the page was built. What could
   `zeno.wasm` leave out, and what would have to travel in place of
   the program's text?
