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

A second example, `examples/sketch.zeno`, is a sheet to draw on with
the mouse: it asks where the mouse is, draws, sets the style of one
element and keeps the chosen colour for the next visit. And
[`playground.md`](playground.md) is about a page in which Zeno
programs are written and run with nothing installed.

## The `web` module

| | |
|---|---|
| `web.element("#count")` | the first element that answers to the selector. A `ValueError` if the page has none |
| `web.elements("li")` | every element that answers to it, as a list. The list may be empty |
| `web.create("li", "Buy milk")` | a new element with that text in it. It is not on the page until it is appended to one that is |
| `web.after(500 ms, tick)` | call `tick` once, that much later. The time must have a unit. It gives a `Timer` |
| `web.every(1 s, tick)` | call `tick` that often, for as long as the page is open. It gives a `Timer` |
| `timer.stop()` | call off what the timer was waiting to do. Stopping one that has stopped, or has made its one call, is nothing |
| `web.on_key(pressed)` | call `pressed` with the name of each key that is pressed: `"a"`, `"Enter"`, `"ArrowLeft"` |
| `web.save("name", "Partha")` | keep text under a name. It is there the next time the page is opened in the same browser |
| `web.load("name")` | the text kept under the name, or `none` |
| `web.forget("name")` | nothing is kept under the name any more |
| `web.fetch("/prices.txt", done)` | ask the network for what is at that address, and call `done(status, body)` when the answer comes |
| `web.post("/orders", text, done)` | send `text` to that address, and call `done(status, body)` with what comes back |

A selector is what CSS calls one: `#count` is the element with that
id, `.done` every element of that class, `li` every list item, and
`#list li.done` what it looks like.

And what an element can do:

| | |
|---|---|
| `e.text` | the text in it. Give it a new value to change what the page shows |
| `e.value` | what has been typed into it, for a box that can be typed into |
| `e.on("click", pressed)` | call `pressed` whenever that happens to it: `"click"`, `"input"`, `"submit"`, any event of the browser |
| `e.fire("click")` | call, now, what `on` was given for that event. It is how a test presses a button |
| `e.append(child)` | `child` becomes the last thing inside it |
| `e.remove()` | it leaves the page |
| `e.clear()` | everything inside it leaves the page |
| `e.add_class("done")`, `e.remove_class("done")`, `e.has_class("done")` | its classes, which is how a program changes how something looks: the look itself is CSS, in the page |
| `e.show()`, `e.hide()` | whether it is shown at all |
| `e.get("href")`, `e.set("href", "/about")` | an attribute. `get` gives `none` for one it does not have |
| `e.set_style("color", "red")`, `e.get_style("color")` | the style of this one element, named as CSS names it. `get_style` gives `none` for one that was not set |
| `e.draw_rect(x, y, width, height, "red")` | on a `<canvas>`: a filled rectangle |
| `e.draw_circle(x, y, radius, "red")` | a filled circle, with its middle at `x`, `y` |
| `e.draw_line(x1, y1, x2, y2, "red")` | a line from one place to another |
| `e.draw_text(x, y, "Hello", "red")` | text, with its top left corner at `x`, `y`, in the font CSS gave the canvas |
| `e.erase()` | the canvas is empty again |

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

**A function that the page calls takes nothing, unless it asks.**
Most functions given to `on` need to know nothing about what
happened. The function already knows which element it is for, because
it was written where that element was in reach: `item` in the
example above belongs to the function that was given to `item.on`.
So `function() { ... }` is enough (14.5).

Some need more: where the mouse was, or which of the things inside
the element was clicked. A function that takes one parameter, an
`Event`, is handed that (16.1):

```
let paper = web.element("#paper")

paper.on("click", function(event: Event) {
    paper.draw_circle(event.x, event.y, 4, "crimson")
})
```

| | |
|---|---|
| `event.kind` | the name of the event: `"click"` |
| `event.target` | the element it happened to. For a click on an `<li>` inside the `<ul>` that `on` was given to, the `<li>` |
| `event.x`, `event.y` | where the mouse was, in pixels from the top left corner of the element `on` was given to. 0 for an event of no mouse |
| `event.key` | the key, for `"keydown"` and `"keyup"`. `""` for any other event |

An `Event` says what happened, so its fields are read and never given
a new value.

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
it calls no function it was given. The tests do that. They call the
function themselves, or they say `fire`, which calls whatever `on`
was given for that event, in the order it was given:

```
test "an empty box adds nothing" {
    let before = remaining
    box.value = "   "
    sent()
    assert remaining == before
}

test "the mouse draws only while its button is down" {
    paper.fire("mousemove")
    assert drawn == 0
    paper.fire("mousedown")
    paper.fire("mousemove")
    assert drawn == 1
}
```

`fire` does the same in a browser: it calls the program's own
functions, at once, and does not send the browser an event. An
`Event` that `fire` makes has the element as its `target`, and 0 for
`x` and `y` (16.3).

Drawing prints a line each (`#paper draw_line 0 0 10 10 black`), and
so does a style (`#lines style color: red`). What `web.save` keeps is
kept until the run ends. `web.fetch` and `web.post` reach no network
and call nothing, as a timer calls nothing.

This is what the simulated page is for. What a page does can be
tested without a browser, in the same run as every other test, and
the two engines (`zeno run` and `zeno run --vm`) are held to the same
lines.

### The page that was written

If `todo.html` is beside `todo.zeno`, `zeno run` and `zeno test` read
it, and the simulated page has the elements it has: their ids,
classes, attributes and text, what a box holds, and which is inside
which. So a name that is spelt wrong is found on a desk, with the
same words as in the browser:

```
$ zeno run todo.zeno
error: uncaught ValueError: the page has no element that answers to `#lsit`. `web.elements` gives a list, which may be empty
  --> todo.zeno:14:12
   |
14 | let list = web.element("#lsit")
   |            ^
```

A program with no page beside it gets a page that has every element
it asks for, as before, because there is nothing to hold it to
(16.9).

`src/html.rs` is the reader. It is not a browser's: it reads a page
that is written right, and makes the best of one that is not.

There are two things the simulated page cannot tell you.

It does not know CSS. It cannot say what a class looks like, or that
`set_style("colour", "red")` names no style, which the browser says
when the page is opened.

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
2. **`zeno.wasm`**, packed the way a `.zip` file is packed, and then
   written out in base64, because a page is text.
3. **`src/web.js`**, three hundred lines of JavaScript that start the
   one with the other.

### How big the page is

`zeno.wasm` was 1.1 MB when this stage began, and base64 made that
1.5 MB in every page. Two things brought a built page down to 0.4 MB.

It is built for size where that costs no speed, as one piece, with
what nothing calls left out of the whole (`.cargo/config.toml` has
the four settings and what each does). That made it 800 KB, and a
loop and a recursion ran as fast as before. One more step, the
smallest setting there is, made it 650 KB and half again as slow,
and was not taken (16.10).

And it is packed. `src/deflate.rs` is the packer: it finds what the
bytes repeat, and gives the common things short codes, which is
DEFLATE, the format of `.zip` and `.gz`. 800 KB becomes 290 KB, within
two parts in a hundred of what `gzip -9` makes of it. A browser
unpacks DEFLATE by itself (`DecompressionStream`, in every browser
since 2023), so nothing is added to the page to undo it (16.11).

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
| `zeno_call(handler)` | call a function the program gave to `after` or `every` |
| `zeno_call_with(handler, place, length)` | the same, with what the page hands over, as JSON: `{"key": "Enter"}`, `{"event": {...}}` for `on`, `{"reply": [200, "..."]}` for `fetch` |

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
and a few lines in `src/page.rs`, and nothing else moves. Styles,
drawing, what is kept between visits and the network were all added
that way, and the line did not move.

**An event** crosses as JSON too, the other way. The page always
sends it. Zeno hands it to the function only if the function asked,
which it knows from when `on` was called (`Handler::takes_event`).

**The network** answers later, so `fetch` is a request with the
number of a function in it, like `on`. When the answer comes, the
page calls that function with the status and the text. An address
that cannot be reached at all answers with status 0.

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

The second stage was tried the same way: `examples/sketch.zeno`
(lines drawn with the mouse held down, a colour chosen, the page
opened again to see that the colour was kept), and a page served from
a web server that fetched a file that was there (200), one that was
not (404), an address nothing answers at (0), posted text, counted
with `every`, asked which `<li>` of a list was clicked, and was
refused a style and a drawing that cannot be.

## What it does not do yet

- **Only what is in the tables.** No images drawn on a canvas, no
  curves, no touch with more than one finger, no address bar (a
  page with several views shows and hides them). The text of an
  answer is a str, and `json.parse` makes values of it
  ([`json.md`](json.md)).
- **It is an interpreter in a page.** Every built page carries
  `zeno.wasm` (0.4 MB of the page). A program runs at the speed of
  `zeno run`, which is plenty for a page that waits for clicks and
  for a sketch, and not for a game that draws a thousand things
  sixty times a second.
- **The program is in the page as you wrote it**, comments and all.
  Anyone who opens the page can read it.
- **One browser was tried**: Chromium. The page needs a browser from
  2023 or later, for `DecompressionStream`. Firefox and Safari have
  it, and that is not the same as having run them.
- **`input` uses the browser's own question box**, which is ugly
  and stops the page while it is open. A page should have a box of
  its own and read its `value`.
- **`web.fetch` is held to what the browser allows.** A page opened
  from the disk can fetch almost nothing, and a page on a web server
  can fetch from another server only if that server permits it. The
  program is told status 0, and the browser's console says why.

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
8. `examples/sketch.zeno` draws a line for every move of the mouse.
   Make it draw dots (`draw_circle`) while Shift is held. An `Event`
   has no field for that. Where would you add one: which three files
   move?
9. `list.on("click", ...)` with `event.target` is one function for a
   whole list, where `todo.zeno` gives every item its own. Rewrite
   `todo.zeno` that way. Which is easier to test with `fire`?
10. Write a page that fetches a file and shows it, and a test for the
    function that is given the answer. The test cannot fetch. What
    does it call?
11. Pack a file of a million zeros with `deflate::pack` (the ignored
    test `how_small` takes a file name in `PACK_FILE`). Then find
    `TRIES` in `src/deflate.rs`, set it to 1, and pack `zeno.wasm`.
    How much bigger is it, and how much faster?
12. A bigger one: the built page carries the whole checker, and the
   program was checked before the page was built. What could
   `zeno.wasm` leave out, and what would have to travel in place of
   the program's text?
13. Write a stopwatch: a page with Start and Stop, and `web.every(100
    ms, ...)` that adds a tenth of a second to what is shown. Stop
    calls `stop()` on the `Timer`. What happens if Start is pressed
    twice, and how does the program make sure only one timer is
    running?
