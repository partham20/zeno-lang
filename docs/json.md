# JSON: text that other programs read and write

A page asks a server for prices, and the server answers in JSON. A
program keeps its settings in a file, and the file is JSON. It is the
text that programs hand each other, and the `json` module reads it
and writes it.

```zeno
import json

let data = json.parse("{\"name\": \"Ada\", \"age\": 36, \"tags\": [\"a\", \"b\"]}")
let name: str = data["name"]
print(name, data["age"] + 1)
for tag in data["tags"] {
    print(tag.upper())
}
```

```
Ada 37
A
B
```

The module has two functions.

| | |
|---|---|
| `json.parse(text)` | the value the text describes. A `ValueError` if the text is not JSON |
| `json.write(value)` | the value as JSON text, on one line |

## What `json.parse` gives

JSON has six kinds of value, and each is a value Zeno already has:

| In the JSON | In Zeno |
|---|---|
| `{"name": "Ada", "age": 36}` | a dict |
| `[1, 2, 3]` | a list |
| `"text"` | a str |
| `12` | an int |
| `12.5` | a float |
| `true`, `false` | a bool |
| `null` | `none` |

Two of those rows need a second sentence.

**A number with nothing after the point is an int.** JSON has one
kind of number and Zeno has two. `12` is an int, and so is `12.0`,
because many programs write a whole price as `2` one day and `2.0`
the next and mean nothing by it. Where a float is wanted whatever
came, say so: `float(data["price"])` (18.3).

**A member that is `null` is left out of the dict.** A dict in Zeno
holds no `none` ([`dict.md`](dict.md)): `get` gives `none` for a key
that is not there, and that is already how a program asks "is there
one?". So `{"note": null}` is a dict with no `note`, and
`data.get("note")` is `none`, as it would be had the member not been
written (18.4). In a list, `null` is `none`: `[1, null]` has two
items.

## The type is known when the program runs

Zeno works out types before a program runs, and it cannot work out
what is in a text that arrives later. So what `json.parse` gives has
no type the checker knows, as a variable has when its type is left
out ([`checker.md`](checker.md)). Everything can be asked of it, and
what is wrong is found when the line runs:

```zeno
let data = json.parse("{\"age\": \"thirty\"}")
print(data["age"] + 1)
```

```
error: `+` does not work between a str and an int
```

Say the type where the value leaves the JSON, and the rest of the
program is checked as usual:

```zeno
let age: int = data["age"]
```

Two things follow from this.

**Strict mode refuses `json.parse`.** A file that begins `strict`,
and every `zeno build`, wants each type before the program runs.
The message is the one for any value of unknown type. JSON is for
`zeno run` and for a page (18.1).

**A dict from JSON can hold values of several types.** One written
in a program cannot: `["name": "Ada", "age": 36]` is refused,
because the checker gives the values of a dict one type. The dict
`json.parse` makes is not seen by the checker, and holds what the
text holds.

## What `json.write` takes

What `json.parse` gives, and objects:

```zeno
class Line {
    item: str
    count: int
}

class Order {
    who: str
    lines: list[Line]
    note: str or none
}

print(json.write(Order("Ada", [Line("tea", 2), Line("milk", 1)], none)))
```

```
{"who":"Ada","lines":[{"item":"tea","count":2},{"item":"milk","count":1}],"note":null}
```

An object is written as JSON calls an object: each field by its
name, in the order the class declares them. That is how a program
writes JSON with a str here and an int there. It declares a class
that says so, and the checker then holds every `Order` to it (18.2).

A dict is written with its keys as text (`[7: "seven"]` is
`{"7":"seven"}`). A float that is whole keeps its `.0`. A quantity
is refused, because JSON has no units and the reader would not know
whether `3.3` was volts: write the number, and put the unit in the
name. A function, a pin or an element has no JSON to be.

## With a page

```zeno
import web
import json

let list = web.element("#prices")

function show(status: int, body: str) {
    if status != 200 {
        list.text = f"could not get the prices ({status})"
        return
    }
    for name, price in json.parse(body)["prices"] {
        list.append(web.create("li", f"{name}: {price}"))
    }
}

web.fetch("prices.json", show)

test "a 404 says so" {
    show(404, "")
    assert list.text == "could not get the prices (404)"
}
```

`web.fetch` hands over text ([`web.md`](web.md)), and `json.parse`
makes values of it. To send, `web.post("/orders",
json.write(order), done)`.

## How it is built

Zeno had a JSON reader already. The language server talks JSON to
the editor ([`lsp.md`](lsp.md)), and `src/json.rs` is its reader and
writer: a hundred and fifty lines, since JSON is six kinds of value.

So the module is two functions in `src/builtins.rs`. `json_parse`
reads the text with `Json::parse` and walks the result
(`from_json`), making a Zeno value of each part. `json_write` goes
the other way (`to_json`), and writes whole numbers itself so that
an int too big for a float is still exact.

Both engines call the same two functions, as they do for every
built-in ([`library.md`](library.md)), and the checker has four
lines for them: `json.parse` takes a str and gives a value of
unknown type, and `json.write` takes anything and gives a str.

## What is left out

- **Reading into a class.** `json.parse` gives dicts, and a program
  takes them apart by hand. Reading straight into an `Order`, with
  every field checked as it is read, is the other half of
  `json.write`, and is not written.
- **Where the mistake is.** "This text is not JSON" shows how the
  text begins, and not the place where it went wrong.
- **Layout.** `json.write` writes one line. There is no version
  with line breaks for people to read.
- **Compiled programs.** `zeno build` has no `json`, for the reason
  above: it needs every type first.
- **Very large whole numbers.** A whole number past 9 thousand
  million million is read as a float, because the reader reads
  every number as one.

## Things to try

1. Parse `{"a": 1, "a": 2}`. Which value does the dict have? Look
   at `from_json` and `Dict::set` and say why. What do other
   languages do?
2. `json.parse("[1, 2.0, 2.5]")` gives an int, an int and a float.
   Write a function `number(value) -> float` that takes any of them.
   Why does Zeno not turn all three into floats?
3. Write a class `Reading` with a `float` and a `str`, and write a
   list of them with `json.write`. Parse the text again. What do you
   get back, and what would you have to write to get `Reading`s?
4. Make `json.write` refuse a dict with an int key, with a message
   that says what to write. Is that better or worse than writing
   `"7"`? Decision 18.2 chose; write the case for the other side.
5. Find where `json.parse` says "this text is not JSON". `Reader`
   in `src/json.rs` knows how far it got (`at`). Hand that back and
   put it in the message.
6. A bigger one: `json.read(text, Order)`. What should it do with a
   member the class does not have? With a field the text does not
   have? With `"count": "two"`? Each answer is a rule. Write the
   three rules before any code.
