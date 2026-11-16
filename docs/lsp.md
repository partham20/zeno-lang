# The language server

`zeno lsp` lets an editor show the mistakes in a Zeno file as you
type, lay the file out when you save, jump to where a name was
declared, and show what a name is. This page says what it does,
how to watch it work with no editor at all, and walks through
`src/lsp.rs` and `src/json.rs`.

Setting up your editor is in [`editors/README.md`](../editors/README.md).
The decisions are 9.10 to 9.13 in [`decisions.md`](decisions.md).

## What it is

An editor does not know Zeno. What it can do is start a program, the
**language server**, and talk to it in a fixed way, the Language
Server Protocol. The editor says "this file was opened" and "this
file now reads so". The server says "these are the mistakes in it".
VS Code, Neovim, Helix, Zed and Emacs all speak the protocol, so one
server serves them all.

Zeno's server is the `zeno` program itself, started as `zeno lsp`. It
runs the loader and the checker that `zeno check` runs, so what the
editor underlines is exactly what `zeno check` would print.

| The editor says | The server does |
|---|---|
| a file was opened, or its text changed | checks every open file, and sends the mistakes of each file whose mistakes changed |
| lay this file out | answers with the file as `zeno format` writes it |
| where is this name declared? | answers with the file and the line of its declaration |
| what is this name? | answers with the name and its type, such as `count: int` |
| anything else that wants an answer | says it has no answer to that |

The two questions about a name are answered for variables,
parameters, functions, classes, enums and the names of a module. A
field or a method (`p.x`) says what it is, and not yet where it was
declared (17.4).

## Watching it work

You do not need an editor to see the server. `editors/talk.py` plays
the editor's part and prints both sides:

```sh
cargo build
python3 editors/talk.py target/debug/zeno examples/thermostat/main.zeno
```

```
--> {"jsonrpc": "2.0", "method": "initialize", "id": 1, "params": {"rootUri": "file:///.../examples/thermostat", ...}}
<-- {"jsonrpc": "2.0", "id": 1, "result": {"capabilities": {"textDocumentSync": {"openClose": true, "change": 1},
     "documentFormattingProvider": true, "definitionProvider": true, "hoverProvider": true},
     "serverInfo": {"name": "zeno", "version": "0.1.0"}}}
--> {"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": "file:///.../main.zeno", "text": "# A program..."}}}
<-- {"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": "file:///.../main.zeno", "diagnostics": []}}
--> {"jsonrpc": "2.0", "method": "textDocument/didChange", ...}
<-- {"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": "file:///.../main.zeno", "diagnostics": [
     {"range": {"start": {"line": 31, "character": 6}, "end": {"line": 31, "character": 18}},
      "severity": 1, "source": "zeno", "message": "`no_such_name` is not defined"}]}}
--> {"jsonrpc": "2.0", "method": "textDocument/didChange", ...}
<-- {"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": "file:///.../main.zeno", "diagnostics": []}}
--> {"jsonrpc": "2.0", "method": "textDocument/hover", "id": 4, "params": {"textDocument": {"uri": "file:///.../main.zeno"},
     "position": {"line": 32, "character": 6}}}
<-- {"jsonrpc": "2.0", "id": 4, "result": {"contents": {"kind": "plaintext", "value": "answer: int"}}}
--> {"jsonrpc": "2.0", "method": "textDocument/definition", "id": 5, "params": {... "position": {"line": 32, "character": 6}}}
<-- {"jsonrpc": "2.0", "id": 5, "result": {"uri": "file:///.../main.zeno",
     "range": {"start": {"line": 31, "character": 0}, "end": {"line": 31, "character": 0}}}}
```

Reading it from the top:

1. **`initialize`.** The editor says hello, and the server answers
   with what it can do: it wants to be told when files open and close,
   it wants the whole text at every change (`"change": 1`), it can
   lay a file out, and it can say where a name was declared and what
   it is.
2. **`didOpen`.** The editor sends the file's text. The server checks
   it and sends back its mistakes: an empty list.
3. **`didChange`.** The script "types" a line, `print(no_such_name)`.
   The server checks again and sends one mistake, with where it is:
   line 31, characters 6 to 18. Both count from 0, so that is line 32
   in your editor. An editor draws the red line from this.
4. **`didChange` again.** The script takes the mistake out and types
   two good lines, `let answer = 6 * 7` and `print(answer)`. The
   server sends an empty list, and the editor takes the red line
   away.
5. **`hover`.** The script asks what stands at line 32, character 6:
   the `answer` inside `print(answer)`. The server says
   `answer: int`.
6. **`definition`.** The same place, and the other question: where
   was it declared? On line 31, the line with the `let`.

After that the script spoils the layout and asks for the file to be
laid out, which the last lines of its output show.

A message with an `id` is a request and gets an answer with the same
`id`. A message with none is a notification, and gets none.

## How it works

### JSON

Every message is JSON. Zeno uses no library, so `src/json.rs` reads
and writes it. JSON has six kinds of value and the type says so:

```rust
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Json>),
    Object(Vec<(String, Json)>),
}
```

Reading is a function for each kind, picked by the first character:
`"` begins a text, `[` a list, `{` an object. Writing is the same
`match` the other way round. The fiddly part is text: `\n` and `\"`
going out, and `é` coming in.

`at` saves a lot of code in the server. A message is objects inside
objects, and

```rust
params.at(&["textDocument", "uri"])
```

walks down them and gives `None` if any step is missing.

### The wire

Ahead of each message stands a header that says how long it is:

```
Content-Length: 44\r\n
\r\n
{"jsonrpc":"2.0","id":3,"method":"shutdown"}
```

`read_message` reads header lines up to the empty one, then exactly
that many bytes. `write_message` does the reverse. The length is in
bytes and not in characters, which matters as soon as a message holds
an `é`.

### The server

```rust
pub struct Server {
    open: BTreeMap<String, String>,              // the text of each open file, by path
    uris: HashMap<String, String>,               // what the editor calls each file
    shown: BTreeMap<String, Vec<Diagnostic>>,    // the mistakes the editor was last told
    workspace: Option<PathBuf>,
    stopping: bool,
}
```

`Server::handle` takes one message and gives back the messages to
send. It reads no input and writes no output. That is why the tests
can hand it a message and look at what comes back, with no process
and no pipe:

```rust
let said = told(&open(&mut server, "main.zeno", "let count = 3\nprint(cuont + 1)\n"));
assert!(said[0].1[0].starts_with("1:6-11 "));
```

`handle` is one `match` on the method's name. The two that do the
work are `didOpen` and `didChange`: they put the new text into `open`
and call `publish`.

### Checking the editor's text

`publish` checks every open file and compares with `shown`. Only a
file whose list of mistakes changed is sent, and a file that had
mistakes and has none now is sent an empty list, which is how an
editor learns to take the red lines away.

The check is the one `zeno check` does, with one difference in where
the text comes from:

```rust
fn read(&self, path: &str) -> Option<String> {
    self.open.get(path).cloned().or_else(|| std::fs::read_to_string(path).ok())
}
```

The loader of [`modules.md`](modules.md) never read files itself. It
was always handed a function that does, so that the tests could give
it files that exist only in memory. The server hands it this one: the
editor's text first, the disk second. Nothing in the loader changed
to make unsaved files work.

### Which program a file belongs to

`zeno check main.zeno` knows the program's folder: it is the folder
of the file it was given. The editor may have `drivers/uart.zeno`
open, whose imports are found from the folder above. `folder_of`
walks up from the file to the nearest folder that has a
`zeno.package` or a `main.zeno`, and stops at the folder the editor
has open. The loader got a second door for this, `modules::load_in`,
which is told the folder and does not take it from the file.

### What a name means

Put the cursor on a name and press "go to definition", and the editor
asks `textDocument/definition` with a line and a character. Rest the
mouse on it, and the editor asks `textDocument/hover`. Both come to
one question: what does the name at this place mean?

The checker has always known. To check `count * 2` it has to find the
`count` that is meant, by the rules of scope. It used to forget the
answer as soon as it had the type. Now it writes each one down:

```rust
pub struct Use {
    pub at: Span,                 // where the name is written
    pub name: String,
    pub ty: String,               // its type at this place
    pub declared: Option<Span>,   // where it was declared
}
```

Two small pieces in `src/checker.rs` fill the record. `declare`, which
every `let`, parameter, function and class goes through, notes where
the declaration is. And the arm of the checker that reads a name
notes the use (the one for a name that is assigned to does the same):

```rust
ExprKind::Name(name) => match env.get(name) {
    Some(variable) => {
        self.used(name, &variable, span);
        ...
```

So `Server::meaning` has little to do:

1. Find the token under the cursor, with the lexer. If it is not a
   name, there is no answer.
2. Load and check the program, as `publish` does.
3. Find the `Use` that stands at that token, and give back its type
   and where it was declared.

```rust
let found = [here, first].into_iter().find_map(|place| {
    let at = written.tokens[place].span;
    uses.iter().find(|used| used.at == at && shown_name(&used.name) == word)
})?;
```

Why two places, `here` and `first`? In `shapes.area(3)` the loader of
[`modules.md`](modules.md) has made `shapes.area` into one name, and
the checker sees it at the place of `shapes`, where the line of dots
begins. So if nothing is used at the cursor's own token, the server
looks at the start of the dots, and takes what it finds only if it is
the word under the cursor. That is why `area` has an answer and
`shapes` has none: `shapes` alone is not a name of anything.

Nothing here reads the spelling of a declaration. In

```zeno
let count = 3
function twice(count: int) -> int {
    return count * 2
}
```

the `count` in the third line goes to the parameter, because that is
what the checker looked up.

### Where a mistake is

The checker gives a line and a column, both from 1, counted in
characters. The protocol wants them from 0, and counts a character as
UTF-16 does: most are one unit and an emoji is two. `diagnostic` does
that sum, and stretches the mark to the end of the word, because the
checker gives only where a mistake starts.

## What is left out

- **Where a field or a method was declared.** Resting the mouse on
  the `x` of `p.x` says `x: int`, and on `queue.put` what `put` takes
  and returns: the checker notes each read of a field with its type
  (`Checker::field_used`, and `at` in `ExprKind::Field` is where the
  name is written). "Go to declaration" on a field still gets no
  answer, because a field does not yet keep where it was written
  (17.4).
- **Names in other places than code.** A type after a colon
  (`p: Point`), a case in a `match`, and a name inside an `f"..."`
  text get no answer either.
- **Completion and rename** (9.11).
- **Colours from the server.** A grammar in the editor does that
  (9.13).
- **Changes sent piece by piece.** The editor sends the whole text
  each time. For files of the size Zeno has that is fast enough.
- **`zeno.package`** is not checked as you type. `zeno check` shows
  its mistakes.
- **The settings for VS Code, Neovim and Helix have not been run in
  those editors.** The server is tested by its own tests and by
  `editors/talk.py`, and the Vim colours were tried in Vim.

## Things to try

1. Run `python3 editors/talk.py target/debug/zeno examples/shapes.zeno`
   and find the three kinds of message: a request, its answer, and a
   notification. How do you tell them apart?
2. In `editors/talk.py`, change the typed line to
   `print(1 + "one")`. Which characters does the server mark, and
   what is the message?
3. Change it to two mistakes on two lines. How many items does
   `diagnostics` have? Now make the second mistake a mistake of
   syntax, such as `let = 3`. How many now, and why? (Which stage
   stops first?)
4. Start `target/debug/zeno lsp` in a terminal and type a message by
   hand: `Content-Length: 44`, Enter twice, then
   `{"jsonrpc":"2.0","id":3,"method":"shutdown"}`. You need the
   length right. What comes back?
5. In `src/lsp.rs`, find the test
   `a_program_in_several_files_is_checked_from_the_editor_s_text`.
   `shapes.zeno` in it is on no disk. Which three lines of the server
   make that work?
6. Add a method to `handle`: answer `textDocument/references` with
   every place the name under the cursor is used. `Types::uses()` has
   all of them: the ones you want have the same `declared` as the one
   `meaning` found. Add `("referencesProvider", Json::Bool(true))` to
   the capabilities, and try it in your editor or with a line more in
   `talk.py`.
7. `Json::Object` keeps its members in a `Vec` and not in a
   `HashMap`. Find the test in `src/json.rs` that would fail with a
   `HashMap`, and say why.
8. In the test `a_name_says_what_it_is_and_where_it_was_declared`,
   `count` is asked about on two lines and goes to two places. Which
   line of the checker made the difference? Then say why asking about
   `shapes` gives nothing while `area` right after it gives a line of
   another file.
9. Set your editor up from `editors/README.md`, open
   `examples/thermostat/main.zeno`, and misspell `filters.lowpass`.
   Then open `examples/filters/lowpass.zeno` and break it. In which
   file does the mistake show?
