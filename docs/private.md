# `private`

A module of any size has parts it needs and parts it only uses to
get its work done. `private` in front of a declaration keeps it to
the file it is written in. This page says what the word does, and
walks through the three small pieces of the compiler that do it.

```sh
cargo run -- run examples/ring/main.zeno
```

The decisions are 10.1 to 10.3 in [`decisions.md`](decisions.md).
Programs in several files are in [`modules.md`](modules.md).

## What it looks like

`examples/ring/ring.zeno` is a queue of a fixed size:

```zeno
class Ring {
    private slots: list[int]
    private first: int       # the slot the oldest byte is in
    private count: int       # how many bytes are waiting

    function put(self, byte: int) -> bool {
        if self.count == len(self.slots) {
            return false
        }
        self.slots[self.slot(self.count)] = byte
        self.count += 1
        return true
    }

    private function slot(self, steps: int) -> int {
        return wrapped(self.first + steps, len(self.slots))
    }
}

function ring(size: int) -> Ring {
    return Ring([0 for slot in range(size)], 0, 0)
}

private function wrapped(place: int, size: int) -> int {
    return place % size
}
```

`examples/ring/main.zeno` imports it:

```zeno
import ring

let queue = ring.ring(4)
queue.put(10)
print(queue.size())
```

and each of these lines in `main.zeno` is a mistake:

```zeno
queue.count = 0                      # `count` is private in the class `ring.Ring`
print(queue.slots)                   # `slots` is private in the class `ring.Ring`
print(ring.wrapped(5, 4))            # `wrapped` is private in the module `ring`
let mine = ring.Ring([1, 2], 0, 2)   # `ring.Ring` has the private field `slots`, so only
                                     # the file that declares the class can build one
```

The count of a ring has to agree with its slots. Because no other
file can write `queue.count = 0`, a reader of `ring.zeno` knows that
every line that changes the count is in the file in front of them.

## The rules, all of them

1. **Where the word goes.** In front of `function`, `class`, `enum`,
   `let`, `var` or `const` at the top of a file, and in front of a
   field or a function of a class.
2. **What it means.** Only the file it is written in can use the
   name. It means the same in both places (10.1).
3. **Everything else is open**, as before. A program in one file
   never needs the word.
4. **An enum is private whole**, with its cases.
5. **A class with a private field is built in its own file.** Another
   file cannot fill in a field it cannot see. The module gives it a
   function that builds one (10.3).
6. **In its own file nothing is hidden.** A function beside the class
   uses the private fields as a method does.

## How it works

`private` is checked in two places, and neither of the two was new.
The compilers, the interpreter and the VM do not know the word at
all: a program that passes the check runs as it would without it.

### The parser

The lexer has one more reserved word, `Private`. The parser, when it
meets it at the start of a statement, reads the statement after it
and notes the name it declares:

```rust
TokenKind::Private => {
    self.advance(); // private
    let declared = self.statement()?;
    let name = match &declared.kind {
        StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => name.clone(),
        StmtKind::Function(function) => function.name.clone(),
        StmtKind::Class(class) => class.name.clone(),
        StmtKind::Enum(decl) => decl.name.clone(),
        _ => return Err(Error::new(what, span)),
    };
    ...
    self.private.push(name);
    declared.kind
}
```

The statement itself is stored as it always was. The names go into a
list of their own, `Program::private`, and a class has the same list
for its fields and functions, `Class::private`. That is why no stage
after the checker had to change: none of them reads those lists.

The parser also counts how many blocks it is inside (`depth`), so
that `private let x` in a function can be refused with the reason.

### Names at the top of a file: the loader

[`modules.md`](modules.md) says how the loader gives every name of an
imported file its full name, `ring.wrapped`, and that it keeps a list
of what each module declares, to say "module `ring` has no `size`"
when a file asks for something that is not there.

That list is now two lists:

```rust
let private = private_names(&unit.program);
for (name, _, is_case) in declarations(&unit.program) {
    match private.contains(&name) {
        true => kept.push(name),
        false => names.push(name),
    }
}
declared.insert(unit.module.clone(), names);   // what other files can use
hidden.insert(unit.module.clone(), kept);      // what the module keeps
```

A file that asks for `ring.wrapped` is looked up in the first list,
as before, and not found. The second list is only there to choose
the message: the name is private, and not missing (10.2).

The module's own file is not looked up in either. It sees everything
it declares, as it always did.

### Fields and functions of a class: the checker

A field is not a name the loader knows. `queue.count` means
something only once the type of `queue` is known, and that is the
checker's work. So the checker's note about a class has two more
things in it:

```rust
struct ClassInfo {
    ...
    private: Vec<String>,   // the fields and methods declared `private`
    file: usize,            // the file the class is written in
}
```

Every place in a program knows which file it is in (`Span::file`:
the lines of the second file are numbered on from the first, see
`modules.md`). So the question "is this use in another file?" is one
comparison:

```rust
fn kept_private(&mut self, class: &ClassInfo, name: &str, span: Span) -> bool {
    let kept = span.file() != class.file && class.private.iter().any(|private| private == name);
    if kept {
        self.error(...);
    }
    kept
}
```

It is asked in the three places the checker meets a field: reading
`object.name` (which is also how a method is found), giving
`object.name` a value, and calling the class to build an object.

## What is left out

- **Private to a package**: a name the files of one package share and
  other packages cannot see (10.1).
- **A private case of an enum.** A `match` has to name every case.
- **A check when the program runs.** `private` is checked by
  `zeno check`, as types are.
- **"Go to declaration" on a private field** in the editor: fields
  have no answer there yet ([`lsp.md`](lsp.md)).

## Things to try

1. Run `cargo run -- run examples/ring/main.zeno`. Then take the `#`
   off one of the four lines at the end of `main.zeno` and run
   `cargo run -- check examples/ring/main.zeno`. Do it for each line.
   Which two pieces of the compiler gave the four messages?
2. Take `private` off `count` in `ring.zeno` and write
   `queue.count = 0` in `main.zeno` before the loop that takes bytes
   out. What does the program print now, and what has gone wrong
   inside the ring?
3. `ring` and `wrapped` are both functions at the top of `ring.zeno`.
   Write a test function in `ring.zeno` itself, below them, that
   calls `wrapped(5, 4)` and reads `ring(2).count`. Why is neither
   refused?
4. Write `private let x = 1` inside a function and read the message.
   Then find `depth` in `src/parser.rs`: where does it go up, and
   where down?
5. Give `first` and `count` the default `= 0` in `ring.zeno`. Can
   `main.zeno` build a ring with `ring.Ring([0, 0, 0, 0])` now? Read
   decision 10.3 for why the answer does not depend on the defaults.
6. Run `cargo run -- parse examples/ring/ring.zeno`. Where does the
   tree show `private`? Now run `cargo run -- build examples/ring/main.zeno`
   and look for the word in the C file it writes. Why is it not
   there?
7. In `src/modules.rs`, find the test
   `what_is_private_stays_in_its_file`. Its last line has a `limit`
   in two files, both private. Why do they not clash? (`zeno join`
   on such a program shows you.)
8. Make `Health` in `examples/station/sensors.zeno` a `private enum`
   and run `cargo run -- check examples/station/main.zeno`. Which
   files complain, and about which names? Put it back.
