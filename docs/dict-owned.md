# A dict that owns its values

With no collector, every list and object has one owner. Until 0.8.0 a
dict could not be one: its values had to be numbers, bools or text,
and a dict of lists or objects was refused with "with no collector a
value of a dict is an int, a float, a bool or a str for now". Now the
dict owns them, as a list owns its items:

```zeno
class Channel {
    name: str
    readings: list[int]
}

function average(channel: Channel) -> int {
    var sum = 0
    for r in channel.readings {
        sum += r
    }
    return sum // len(channel.readings)
}

let channels: dict[str, Channel] = [:]
channels["volts"] = Channel("volts", [12, 13, 11])
channels["amps"] = Channel("amps", [2, 3])
channels["volts"] = Channel("volts", [14, 14])     # the old one is freed
channels["amps"].readings.append(4)

for key, channel in channels {
    print(key, average(channel))
}

let amps = channels.pop("amps")
print(amps.name, len(amps.readings), len(channels))
```

```sh
cargo run -- build --no-collector --memory channels.zeno
./channels
```

```
volts 14
amps 3
amps 3 1
memory: 20 blocks made (504 bytes), at most 424 bytes in use at once, no collector, 0 blocks left at the end
pool: 624 bytes taken, in a pool of 32768
text: at most 4 bytes in use at once, in a room of 1024
```

`zeno run` prints the same three lines, and so does every other build
and every target. The reasons for each choice are in 56.12 of
[`decisions.md`](decisions.md). It is part 9.5 of
[`plan-9-14.md`](plan-9-14.md).

## The rule

> A dict owns its values, as a list owns its items.

Each way into the dict follows from that, and each is what the same
thing does to a list of owned items ([`one-owner.md`](one-owner.md)):

| Written | With one owner |
|---|---|
| `d[key]` | a look at the value. Use it where it is, or pass it to a parameter that looks |
| `d[key] = value` | the dict takes the new value, and frees the one that was there |
| `d.pop(key)` | hands the value to whoever receives it |
| `d.clear()`, and the end of the dict | free every value |
| `for key, value in d` | a look at each value, for the body |
| `d.get(key)` | a look at the value, or `none` |
| `d.get(key, instead)` | refused |
| `d.values()` | refused |

A look is the same look a parameter gets. It can be read, changed in
place (`channels["amps"].readings.append(4)`) and passed on, but not
kept:

```
error: this value is an owned Channel, which has one owner: the dict. `[ ]` only looks at it. Use it where it is, pass it to a parameter that looks, or take it out with `pop(key)`
 --> keep.zeno:7:13
  |
7 | let volts = channels["volts"]
  |             ^
```

## `get`

`get(key)` gives a look at the value, or `none` if the key is not
there. Pass it to a parameter that may be `none`, and test it there:

```zeno
function show(c: Channel or none) {
    if c is none {
        print("no channel")
        return
    }
    print(c.name, len(c.readings))
}

show(channels.get("volts"))
show(channels.get("amps"))
```

`get(key, instead)` is refused. It would give either a look at a value
the dict owns or the value `instead`, which nobody else owns, and the
line after it could not tell which one to free:

```
error: a value of this dict is an owned list[int], which has one owner: the dict. `get` with a second value would give either a look at the dict's or the value given, and nobody could tell who frees it. Write `get(key)`, which gives a look or `none`, and test it with `is not none`
```

## `values()`

A list of the values would hold the same objects as the dict, and so
be a second owner of each. It is refused, and the message says what to
write:

```
error: a value of this dict is an owned list[int], which has one owner: the dict. A list of the values would be a second owner of each. Loop over the dict with `for key, value in` instead, which lends each value in turn
```

`keys()` is fine: a key is a number or text, which is copied.

## The three guards

`pop`, `d[key] = value` and `clear` free a value or hand it over, so
they are held to the three guards a list's `pop()` is held to: only
the owner may do it, not while a loop walks the dict, and not on a
line that has already looked at it.

```
error: the `for` on line 6 is walking a dict that `d` owns, and is still looking at its values. Nothing can take a value out of this dict until the loop is over
 --> vals.zeno:7:5
  |
7 |     d.pop("a")
  |     ^
```

## With a collector

Nothing changes. A dict of lists or of shared objects was fine there
and still is. A dict whose values are objects of an `owned class` is
still refused by `zeno build`, as before ("cannot compile a dict with
these keys and values yet"); `zeno run` runs it.

## In the C

A dict is two lists and an index ([`dict.md`](dict.md)), and
`runtime.c` is not changed. Each program that has a dict that owns its
values gets three small functions of its own, one set for each kind of
value, which free a value through the free function its kind already
has:

```c
static void zeno_dict_put_values_Channel(zeno_dict *dict, zeno_slot key, zeno_slot value) {
    zeno_drop_Channel(((z_Channel *)zeno_dict_value(dict, zeno_dict_find(dict, key)).p));
    zeno_dict_set(dict, key, value);
}

static void zeno_dict_drop_values_Channel(zeno_dict *dict) {
    if (dict == NULL) return;
    zeno_dict_clear_values_Channel(dict);
    zeno_dict_drop(dict);
}

static void zeno_dict_clear_values_Channel(zeno_dict *dict) {
    for (int64_t i = 0; i < dict->values->len; i++) zeno_drop_Channel(((z_Channel *)dict->values->items[i].p));
    zeno_dict_clear(dict);
}
```

`d[key] = value` calls the first. For a key that is not there yet,
`zeno_dict_find` gives -1, `zeno_dict_value` gives an empty slot, and
freeing `NULL` does nothing. The end of the dict calls the second, and
`clear` the third. The LLVM build defines the same three in IR. A loop
over a dict written right where the loop walks it,
`for key, frame in ["p": Frame(10, [1])]`, frees the values with the
dict when the loop ends.

## How it is tested

- **The checker**: `a_dict_owns_its_values` checks each row of the
  table, the two refusals, the three guards, a look that is kept, and
  that a build with a collector is as it was.
- **Both compilers**: `a_dict_owns_its_values_every_way` builds
  [`tools/targets/edge-dict-owned.zeno`](../tools/targets/edge-dict-owned.zeno)
  with no collector through C and LLVM, which give the same memory
  report with 0 blocks left. It is also built in the ordinary and
  firmware builds of both. The program has:
  - a dict of objects and a dict of lists, each way in the table;
  - a dict of enum values that carry values;
  - a dict in a field, and dicts in a list;
  - a dict made right where a loop walks it;
  - a thousand stores over one key, in a pool that does not fill.
- **The targets** run the same program.

## Things to try

1. Build `channels.zeno` with `--no-collector` and open `channels.c`.
   Find `zeno_dict_put_values_Channel`. What does it free the first
   time `"volts"` is stored, and what the second time?
2. Write `let volts = channels["volts"]` and read the message. Then
   write `print(average(channels["volts"]))`. Why is one refused and
   not the other?
3. Change `show(channels.get("amps"))` to
   `channels.get("amps", Channel("none", [0]))`. Who would free the
   `Channel` made there if the key were found?
4. Inside the `for key, channel in channels` loop, write
   `channels.pop("amps")`. Read the message. Why would the loop be
   left looking at a freed object without it?
5. Store a thousand channels over one key in a loop, and build with
   `--memory`. How many blocks are left at the end, and how much of
   the pool is in use at most?
6. Make a `dict[str, list[int]]` and loop over `keys()`, printing
   `len(d[key])` for each. Why does `keys()` build when `values()`
   does not?
