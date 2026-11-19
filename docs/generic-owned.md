# A value with one owner in a generic

With no collector, every list and object has one owner. Until 0.8.0 a
generic could not hold one: `Stack[Frame]`, `same([1, 2])` and
`Box(Frame(1, []))` were refused in every build with no collector,
with "a value that has one owner cannot fill a type parameter yet".
Now they build:

```zeno
class Frame {
    id: int
    bytes: list[int]
}

class Stack[T] {
    items: list[T]

    function push(self, x: take T) {
        self.items.append(x)
    }

    function pop(self) -> T {
        return self.items.pop()
    }
}

function same[T](x: take T) -> T {
    return x
}

let frames: Stack[Frame] = Stack([])
frames.push(Frame(1, [1, 2]))
frames.push(same(Frame(2, [3])))
let top = frames.pop()
print(top.id, len(frames.items))

let counts = Stack([1, 2])
counts.push(3)
print(counts.pop(), same(5))
```

```sh
cargo run -- build --no-collector --memory stack.zeno
./stack
```

```
2 1
3 5
memory: 13 blocks made (248 bytes), at most 248 bytes in use at once, no collector, 0 blocks left at the end
pool: 408 bytes taken, in a pool of 32768
text: at most 4 bytes in use at once, in a room of 1024
```

`zeno run` prints the same two lines, and so does every other build
and every target. The reasons for each choice are in 56.11 of
[`decisions.md`](decisions.md). It is part 9.4 of
[`plan-9-14.md`](plan-9-14.md).

## The rule

> A copy of a generic is checked as if it had been written by hand,
> with the type in the place of `T`.

That was always the rule ([`generics.md`](generics.md)). With no
collector it was simply not followed: the checker stopped at the call.
Now it goes on. `Stack[Frame]` is a class with a list of frames, which
a build with no collector has taken since lists could own their items.
So everything [`one-owner.md`](one-owner.md) says of such a class is
true of the copy, in the same words.

## `take T`

A parameter written `T` looks at what it is given, as any parameter
without `take` does. A generic that keeps what it is given, or gives
it back, says so with `take T`:

```zeno
function same[T](x: take T) -> T {
    return x
}
```

Until 0.8.0 `take T` was refused, because `T` might be a number. Now
it is read for whatever `T` stands for. In a copy where `T` is a
number, text or anything else that is copied, the word changes
nothing, as `take` changes nothing where values are shared. So `same`
is right for an `int` and for a list, in every build.

A generic that does not say `take`, and is used with a list, is an
error inside the copy. The note names the copy and the line that asked
for it, as every error inside a copy does:

```
error: `x` is only looked at here: its owner still has it. So it cannot be kept, returned or given away. If it is a parameter, write `take` in front of its type to take the value over
 --> keep.zeno:6:12
  |
6 |     return x
  |            ^
  = note: this is inside `keep[Frame]`: `keep` with T = Frame, which line 8 asks for
```

## What the call does

What `T` stands for is only known once all the arguments are read. So
the call goes by what the generic wrote:

- **A field of a generic class, and a parameter written `take T`**,
  take the value. After the line, the variable that held it holds
  nothing, as with any `take`.
- **A parameter written `T`, or `list[T]`**, looks. The value is still
  there after the call.

```zeno
let fs = [Frame(1, [])]
print(count(fs))     # count[T](xs: list[T]) looks
print(len(fs))       # so fs still has its list
let b = Box(fs)      # a field takes it
```

`T` is always the value itself, never a look at it. A list that a
parameter only looks at, passed on, makes `T` the list. The copy then
says whether it may be kept.

## Names

With no collector every list has one owner, so `owned` would say
nothing in the name of a copy. The copy of `Box` for a list of ints is
`Box[list[int]]` in a message and `z_Box_list_int` in the C, as a copy
is named in any build.

## With a collector

An object of an `owned class` can fill `T` too. The copy is then held
to the rules of a class written for it. A class whose field holds such
an object must be an `owned class` itself, and so must the generic:

```
error: `value` holds an owned value, so `Box[Packet]` must be an `owned class` itself. A shared object cannot own anything
```

## In the C

Nothing is new. A copy is built as a class written by hand is. Its
free function is written as for any class, and frees what the copy
holds:

```c
static void zeno_drop_Stack_Frame(z_Stack_Frame *self) {
    if (self == NULL) return;
    zeno_list_drop_1_Frame(self->z_items);
    zeno_free(self);
}

static void z_Stack_Frame_push(z_Stack_Frame *z_self, z_Frame *z_x) {
    z_Frame *t4 = z_x;
    z_x = NULL;
    zeno_list_append(z_self->z_items, (zeno_slot){.p = t4});
    zeno_drop_Frame(z_x);
}
```

`z_Stack_int_push` beside it appends the number and frees nothing. The
LLVM build is the same, and the interpreter, the VM and the targets
needed no change.

## How it is tested

- **The checker**: `a_value_with_one_owner_can_fill_a_type_parameter`
  checks:
  - `same`, `Box` and `Stack` with a list, an object and an enum value;
  - a parameter written `T` that looks, and one written `take T` that
    takes;
  - `take T` with a number, in every build;
  - the error inside a copy, with its note;
  - an `owned class` with a collector.
- **Both compilers**: `a_value_with_one_owner_fills_a_type_parameter_every_way`
  builds
  [`tools/targets/edge-generic-owned.zeno`](../tools/targets/edge-generic-owned.zeno).
  It is built with no collector through C and LLVM, which give the same
  memory report with 0 blocks left. It is also built in the ordinary
  and firmware builds of both. The program has:
  - `same`, `Box`, `Stack`, `Pair` and `Slot` with a list, an object
    and an enum value;
  - a stack that hands its items back;
  - a slot that hands its item on with `take`;
  - copies printed;
  - a thousand rounds.

  The same test builds a kept function in a generic, which takes a list
  in one copy and a number in another, and an `owned class` copy with a
  collector.
- **The targets** run the same program.

## Things to try

1. Build `stack.zeno` with `--no-collector` and open `stack.c`. Find
   `z_Stack_Frame_push` and `z_Stack_int_push`. Why does one set `z_x`
   to `NULL` and the other not?
2. Take `take` off `x` in `same`. Read the message, and the note under
   it. Which copy is it in, and why is `same(5)` not the one refused?
3. Write `let f = Frame(3, [])`, then `frames.push(f)`, then
   `print(f.id)`. What does the build say, and what does `zeno run`
   print?
4. Write `function count[T](xs: list[T]) -> int { return len(xs) }`
   and call it twice on the same list. Why is that fine, when two
   calls of `same` on it are not?
5. Add `function peek(self) -> T { return self.items[0] }` to `Stack`.
   Use it on `Stack[int]`, then on `Stack[Frame]`. Why is one refused?
   What could `peek` return instead?
6. Make a `Stack[list[int]]` and push a thousand lists, popping each
   one again. Build it with `--memory`. How many blocks are left at the
   end?
