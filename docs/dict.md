# The standard library, part 4: a dictionary

A list finds a value by its position. A dict finds a value by its key:
a name, or a number that is not a position. This page adds the type,
and follows it through every stage of the compiler, because a new type
touches all of them.

```sh
cargo run -- run examples/faults.zeno
```

```
23: over temperature (x4)
17: over voltage (x2)
40: fan stalled (x1)
99: a code nobody wrote down (x1)
most often: over temperature
true false
the fan stalled 1 time
left out 1 reading with an unknown code
[23: 4, 17: 2, 40: 1]
["mode": "manual", "unit": "celsius", "limit": "85"]
["mode", "unit", "limit"]
["manual", "celsius", "85"]
3
```

[`examples/faults.zeno`](../examples/faults.zeno) counts the fault
codes a device has sent and looks up what each one means. The
decisions are 6.29 to 6.40 in [`decisions.md`](decisions.md).

## What a dict does

```zeno
let ages = ["Ada": 36, "Alan": 41]      # written out
let seen: dict[int, int] = [:]          # empty: the type says what it will hold
```

| Written | Gives |
|---|---|
| `ages["Ada"]` | the value under that key. A key that is not there is a `KeyError` |
| `ages["Grace"] = 45` | stores: adds the key, or replaces its value |
| `"Ada" in ages`, `"Bob" not in ages` | `bool`: is there such a key? |
| `len(ages)` | how many keys |
| `for name in ages { }` | each key, in the order they were put in |
| `ages.get("Bob")` | the value, or `none` |
| `ages.get("Bob", 0)` | the value, or the second argument |
| `ages.pop("Alan")` | takes the key out and gives its value |
| `ages.keys()`, `ages.values()` | a new list of each |
| `ages.clear()` | empties it |
| `a == b` | `true` when the same keys have the same values, in any order |
| `print(ages)` | `["Ada": 36, "Alan": 41]`, as it is written |

A key is a `str` or an `int`. A value is an `int`, a `float`, a `bool`
or a `str`. All the keys of one dict are one type and so are all the
values, so `dict[str, int]` says everything about what is inside.

### Three ways to ask for a key

```zeno
print(ages["Ada"])            # I know it is there. If not: KeyError
let age = ages.get("Bob")     # it may not be: int or none
if "Bob" in ages { ... }      # is it?
```

Zeno does not answer `none` or `0` for a key that was never stored
when you wrote `ages["Bob"]`. A misspelt key would then be a wrong
total at the end of the program instead of an error at the line with
the spelling mistake (6.34).

### One bracket for both

Braces in Zeno are always a block of code, and square brackets always
hold values. So a dict is written in square brackets, and what makes
it a dict is the colon:

```zeno
[1, 2, 3]            # a list
["a": 1, "b": 2]     # a dict
[]                   # the empty list
[:]                  # the empty dict
```

## How it is built

A new type goes through every stage. Here they are in order, with the
place in the source where each one does its part.

### The parser: which one is it?

`list` in `src/parser.rs` is called when the parser meets `[`. It reads
one expression and looks at what follows:

```
[ ]                 an empty list
[ : ]               an empty dict
[ expr ,  or  ]     a list
[ expr :            a dict: read the value, then more `key: value`
```

One token decides, and the parser never has to go back. The tree gets
a new node, `ExprKind::Dict`, which holds the pairs. A list item in the
middle of a dict, or the other way round, gets a message that says how
an entry is written:

```
error: every entry of a dict is written `key: value`, and this one has no `:`
```

### The checker: what does it hold?

`Ty::Dict(key, value)` in `src/checker.rs` is the type. `dict_written`
works out the type of a dict that is written out: it looks at each
pair, refuses a key that is not text or an int, and refuses a pair that
does not match the ones before it.

```
error: the values of a dict are all of one type. This one is float, and the
ones before it are int
```

It also refuses the same key written twice, which in any other
language quietly keeps the second:

```
error: this key is written twice in the dict. A dict has each key once, so
the second would replace the first. Take one of them out
```

Everything else is a few lines each, next to what a list does there:
`ages["Ada"]` checks the key against the key type and gives the value
type; a loop variable gets the key type; `get` with one argument gives
`V or none`, which the rest of the checker already knows must be tested
before it is used.

### `zeno run` and the VM: a `Vec` and a `HashMap`

`Dict` in `src/value.rs`:

```rust
pub struct Dict {
    entries: Vec<(Value, Value)>,     // in the order the keys were put in
    places: HashMap<Key, usize>,      // where each key is in `entries`
}
```

The `Vec` gives the order, and the `HashMap` finds a key without
looking at all of them. `Key` is a small enum with two cases, `Int`
and `Str`, because a Zeno `Value` in general cannot be hashed (a float
has no sensible hash) and a key never needs to be.

The VM has one new instruction. `zeno bytecode` shows it:

```
0000    1  Const        0          ; "Ada"
0001    |  Const        1          ; 36
0002    |  Const        2          ; "Alan"
0003    |  Const        3          ; 41
0004    |  Dict         2
```

`Dict 2` takes two pairs off the stack and leaves one dict. Reading and
storing use `GetIndex` and `SetIndex`, which lists already had: the
work is in `ops::item` and `ops::set_item`, which now look at what kind
of value they were handed.

### The C runtime: a hash table

A compiled program cannot lean on Rust's `HashMap`, so `src/runtime.c`
has its own, under the heading "Dicts". It is about 300 lines, half
of them comments.

```c
typedef struct {
    zeno_list *keys;
    zeno_list *values;
    int64_t *index; /* position + 1 of a key in `keys`, or 0 for an empty place */
    int64_t room;   /* how many places `index` has: 0, or a power of two */
    char key_kind;
    char value_kind;
} zeno_dict;
```

The keys are in one list and the values in another, side by side.
`values->items[i]` belongs to `keys->items[i]`. Those two lists are the
order, and they are ordinary lists, so `keys()`, `values()` and the
`for` loop are code that was already there.

The index is what makes finding fast:

```
keys     "Ada"   "Alan"   "Grace"
values    36      41       45
index    [0, 3, 0, 0, 1, 0, 2, 0]
```

To find `"Alan"`:

1. Work out a number from the key, its *hash*. For text this is FNV-1a:
   start with a fixed number, and for each byte, XOR it in and multiply
   by a prime. Two different texts almost never give the same number.
2. Keep the low bits of the hash. With 8 places that is a number from
   0 to 7: the place to start at.
3. Look at that place in the index. 0 means empty: the key is not in
   the dict. Otherwise it is a position (plus one). Is the key at that
   position the one we want? If so, found. If not, two keys landed on
   the same place: step to the next place and look again.

The index always has at least twice as many places as there are keys.
When a store would make it more than half full, `zeno_dict_index`
makes one twice the size and puts every key in again. That is why step
3 ends quickly: half the places are empty, so a run of full ones is
short.

`pop` takes the key and the value out of the two lists, which moves
everything after them down by one. Every position in the index after
that point is now wrong, so the index is written again from the list.
That is the price of keeping the order (6.33).

The two letters, `key_kind` and `value_kind`, say what the dict holds:
`'s'` text, `'i'` int, `'f'` float, `'b'` bool. With them the runtime
can compare two keys, show a dict and compare two dicts without help
from the compiler.

### The C backend

`let ages = ["Ada": 36, "Alan": 41]` and three lines that use it
become:

```c
z_ages = zeno_dict_of('s', 'i', 2, (zeno_slot){.s = "Ada"}, (zeno_slot){.i = 36},
                      (zeno_slot){.s = "Alan"}, (zeno_slot){.i = 41});
zeno_dict_set(z_ages, (zeno_slot){.s = "Grace"}, (zeno_slot){.i = t1});
... zeno_dict_at(z_ages, (zeno_slot){.s = "Ada"}, 3, 7).i ...
... (zeno_dict_find(z_ages, (zeno_slot){.s = "Bob"}) >= 0) ...
```

A `zeno_slot` is the 64-bit box that an item of a list already
travels in. The `3, 7` is the line and column, for the `KeyError`.

A loop over a dict is a loop over a list of its keys, made for the
loop:

```c
zeno_list *t2 = zeno_dict_keys(z_ages);
for (int64_t t3 = 0; t3 < t2->len; t3++) {
    const char *z_name = t2->items[t3].s;
    ...
}
zeno_list_drop(t2);
```

So the body can add keys and take them out, and the loop still walks
the keys it began with. The work is in `open_list_loop` in
`src/emit_c.rs`: a dict is turned into its list of keys, and the code
for a list takes over.

### The LLVM backend

The same calls, written as IR. `let ages = ["Ada": 36]` and
`print(ages["Ada"])`:

```llvm
%t1 = call ptr @zeno_dict_new(i32 115, i32 105)        ; 's' and 'i'
%t2 = ptrtoint ptr @.str.1 to i64                      ; "Ada" as a slot
call void @zeno_dict_set(ptr %t1, i64 %t2, i64 36)
store ptr %t1, ptr @z_ages
%t3 = load ptr, ptr @z_ages
%t4 = ptrtoint ptr @.str.1 to i64
%t5 = call i64 @zeno_dict_at(ptr %t3, i64 %t4, i32 2, i32 7)
```

115 and 105 are the letters `s` and `i` as numbers. One thing differs
from the C backend: `get(key)` on a dict of ints. The answer is an
`int or none`, which travels as a pair, a flag and a number. In C the
runtime gives back that pair as a struct. In IR the backend asks the
runtime where the key is (`zeno_dict_find`, which answers -1 for
"nowhere"), asks for the value there, and builds the pair itself with
`insertvalue`. It is the same trick `find` on a list uses.

### With no collector

`zeno build --no-collector` has no collector to tidy up, so every
value has one owner that frees it (docs/one-owner.md). A dict follows
the rules a list follows. The checker needed a line in each place
where it says "a list has one owner": a dict has one too.

```zeno
function tally(words: list[str]) -> dict[str, int] {
    let seen: dict[str, int] = [:]
    for word in words {
        seen[word] = seen.get(word, 0) + 1
    }
    return seen                       # gives the dict to the caller
}

let counts = tally(["a", "b", "a"])   # `counts` is its owner now
print(counts)
                                      # end of the block: the dict is freed
```

When the owner's block ends, the compiled program calls
`zeno_dict_drop`, which frees the two lists, the index and the dict.

Text needs one more thing. With no collector, whatever keeps text has
a copy of its own (docs/text.md). So a dict has its own copy of every
key and every value that is text, and `zeno_dict_drop` frees those as
well. Storing under a key the dict already has does not copy the key a
second time, and frees the old value.

`zeno build --no-collector --memory examples/faults.zeno` shows that
it adds up:

```
memory: 30 blocks made (1032 bytes), at most 784 bytes in use at once, no collector, 0 blocks left at the end
pool: 1104 bytes taken, in a pool of 32768
text: at most 148 bytes in use at once, in a room of 1024
```

### In a firmware build

A key that is not there stops the program, as a bad index does:

```zeno
let ages = ["Ada": 36]
print(ages["Bob"])
```

```
error: uncaught KeyError: this dict has no key "Bob"
 --> ages.zeno:2:7
```

It cannot be caught there, and a `try` around it is refused when the
program is built, with a message that says so. A program that must not
stop asks first, with `in` or `get`.

## What a dict cannot do yet

- A value cannot be a list, an object or another dict. Keep those in a
  list and the position of each in the dict (6.32).
- A key cannot be an enum case.
- There is no loop over pairs. `for key in d` and `d[key]` do it.
- A generic function cannot take `dict[str, T]`.

## Things to try

1. Run `examples/faults.zeno`. Then add a code to `arrived` that is
   already there, and one that is new. Where in the output does each
   show up, and why there?

2. Write `let d = ["a": 1, "a": 2]` and read the message. Then store
   the two one after the other, with `d["a"] = 1` and `d["a"] = 2`.
   Why is one refused and the other not (6.36)?

3. Change `meanings[worst]` in the example to `meanings[99]` and run
   it. Then build it with `--firmware` and run that. Now wrap the line
   in `try` / `except KeyError` and build both ways again.

4. Write a function that takes a sentence and gives back a
   `dict[str, int]` of how often each word is in it. Use `split()`.
   Print the word that comes most often.

5. In a loop over a dict, add a new key on every trip. Does the loop
   ever end? Find the line in `open_list_loop` (`src/emit_c.rs`) that
   makes it so, and the one in `src/ops.rs` (`items`) that does the
   same for `zeno run`.

6. Run `zeno bytecode` on a program with `d["k"] = d.get("k", 0) + 1`.
   Which instruction stores? Find where the VM runs it and follow it
   into `ops::set_item`.

7. In `src/runtime.c`, make `zeno_dict_hash` return 0 for every key.
   Run `cargo test dict`. Do the tests still pass? What has changed
   for a dict with ten thousand keys? Put it back.

8. Build a program that fills a dict with 1000 int keys using
   `--no-collector --memory`. How many bytes are in use at most? The
   index doubles each time it fills: which sizes did it go through?
   Then raise `ZENO_POOL` (docs/pool.md) and try 10000.

9. `pop` rewrites the whole index. Think of a way to take a key out
   without moving the ones after it (a "tombstone": a mark that says
   "something was here"). What would a loop and `print` then have to
   skip, and what would `len` count?

10. Add a method `has(key)` that means the same as `key in d`. Follow
    `get` through `builtins.rs`, `checker.rs`, `emit_c.rs` and
    `emit_llvm.rs`. Then take it out again: why is one way to ask
    better than two?
