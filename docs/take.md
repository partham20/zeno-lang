# `take` in front of a field or an item

With one owner, an object could not hand on what one of its fields
holds. Reading the field only looks, and giving the value away would
leave nothing in the field. A list had `pop()`, but a field had
nothing. Since 0.8.0, `take` in front of a field or an item gives what
is there and leaves `none` in its place:

```zeno
class Buffer {
    bytes: list[int]
}

class Uart {
    filling: Buffer or none
    sent: int

    function hand_on(self) -> Buffer or none {
        return take self.filling
    }
}

function send(buffer: take Buffer) -> int {
    return len(buffer.bytes)
}

let uart = Uart(Buffer([1, 2, 3]), 0)
let full = uart.hand_on()
print(uart.filling is none)
if full is not none {
    print(send(full))
}

let queue: list[Buffer or none] = [Buffer([4]), none, Buffer([5, 6])]
let last = take queue[2]
print(queue[2] is none, last is not none)
```

```sh
cargo run -- build --no-collector --memory hand.zeno
./hand
```

```
true
3
true true
memory: 12 blocks made (208 bytes), at most 152 bytes in use at once, no collector, 0 blocks left at the end
pool: 280 bytes taken, in a pool of 32768
text: at most 2 bytes in use at once, in a room of 1024
```

`zeno run` prints the same three lines, and so does every other build
and every target. The reasons for each choice are in 56.10 of
[`decisions.md`](decisions.md). It is part 9.3 of
[`plan-9-14.md`](plan-9-14.md).

## The rule

> `take` in front of a field or an item gives what is there, and leaves `none`.

The place must be one that may hold `none`, such as `filling: Buffer
or none` or a `list[Buffer or none]`. What is left is then something
its type already allows. What comes back may be `none` too, and is
tested as any such value is.

It means the same in every build. Where values are shared, it still
leaves `none`. So a program runs the same way under `zeno run`, in an
ordinary build and with no collector.

`take` goes in front of the whole place after it, as a minus does:
`take slots[i].task`, `take make().task`. It is a word that was
reserved already, so no program that built before means anything new.

## What it cannot be in front of

A variable is not a place for it, since `let b = a` already gives `a`
away:

```
error: `take` goes in front of a field or an item, as in `take slot.task`, and leaves `none` there. `task` is a variable: `let other = task` already gives what it holds away
```

A place that cannot hold `none` is refused, and the message says how to
make it one. For a list, it also says that `pop()` takes the item out:

```
error: `take` leaves `none` in `id`, and `id` is written `int`, which does not allow `none`. If it may be empty, declare it as `id: int or none`
error: `take` leaves `none` in the place of the item, and the items of this list are written `Task`, which does not allow `none`. To take the item out, use `pop()`. To leave `none` in its place, make it a `list[Task or none]`
```

A value of a dict is not a place for it yet. A dict cannot hold a value
that may be `none`, so there would be nothing to leave.

## With one owner

The value leaves the object or the list it was in. So the three guards
of `pop()` hold, in the same words:

- **Only the owner may take it out.** A function that was lent the
  object cannot know who else was lent the field:

  ```
  error: only the owner can take out what is there: somebody else may have been lent it. Here `s` only looks at it
  ```

- **Not while a loop walks the owner.**
- **Not on a line that has already looked at it.** For example,
  `look(s.task, take s.task)` is refused. `look(take s.task, s.task)`
  is fine, since the field holds `none` by the time it is looked at.

A method may take out what its own object holds, as `hand_on` does
above. Whoever calls the method is then held to the rule in its place:
only the owner of `uart` may call `uart.hand_on()`.

## In the C

The value is read into a variable of its own, and `none` is written
where it was. Nothing is freed there. Whoever receives the value owns
it. If nobody does, as in `print(take s.task)`, the statement owns it
and frees it when the line is over.

```c
static z_Buffer *z_Uart_hand_on(z_Uart *z_self) {
    z_Buffer *t5 = z_self->z_filling;
    z_self->z_filling = NULL;
    return t5;
}
```

Through LLVM it is a `load` and a `store` of `null`. The interpreter
does the same, and the VM has two new instructions for it, `TakeField`
and `TakeIndex`. The targets get two statements, one that keeps what is
there and one that stores `none`, so no target's runtime changed.

## How it is tested

- **The parser**: `take_goes_in_front_of_a_field_or_an_item` checks
  where it binds and both of its messages.
- **The checker**: `take_leaves_none_where_a_value_was` checks:
  - the type in every kind of build;
  - a place that cannot hold `none`, and text;
  - the three guards;
  - a method that takes out of its own object.
- **Both compilers**: `take_leaves_none_every_way` builds
  [`tools/targets/edge-take.zeno`](../tools/targets/edge-take.zeno).
  It is built with no collector through C and LLVM, which give the same
  memory report with 0 blocks left. It is also built in the ordinary
  and firmware builds of both. The program has:
  - a scheduler that hands its task on;
  - fields and items holding an object, text or a number that may be
    `none`;
  - a list of a fixed length;
  - a thousand rounds;
  - a value that nobody receives.

  The same test checks that a bad index stops the program at the
  `take`, at the same line and column, in each build of C.
- **The targets** run the same program.

## Things to try

1. Build `hand.zeno` with `--no-collector` and open `hand.c`. Find
   `z_Uart_hand_on`. Why is nothing freed there, and who frees the
   buffer in the end?
2. Write `let full = uart.filling` instead of calling `hand_on`. Read
   the message. What would go wrong if it were allowed?
3. Take `or none` off `filling`. Which line is refused now, and why
   can `take` not be used on that field any more?
4. Write `function steal(u: Uart) -> Buffer or none { return u.hand_on() }`
   and build it with `--no-collector`. Which word does the message want,
   and where?
5. Write `print(take uart.filling, uart.filling)`, then the two the other
   way round. Why is one refused and the other not?
6. Use a `list[Buffer or none]` as a ring of buffers. Hand each one on
   with `take ring[i]`, and put a new one in its place. Build it with
   `--memory`. How many blocks are left at the end?
