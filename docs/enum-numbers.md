# An enum whose cases stand for numbers

On a wire, in a register or in a log, a command, a status or an error
is a number. In a program it should be a name. An enum can now be
both: each case is given its number where it is written, and the
program turns a case into its number and a number back into a case.

```zeno
enum Reply {
    Ack = 6
    Nak = 21
}

let byte: u8 = u8(Ack)        # 6, to send
let reply = Reply(byte)       # Reply or none, from what came back
if reply is not none {
    print(reply)              # Ack
}
```

| Write | Gives |
|---|---|
| `Ack = 6` in the enum | the case `Ack` stands for 6 |
| `int(Ack)` | `6` |
| `u8(Ack)`, `u16(Ack)`, ... | `6`, checked to fit, as every conversion is |
| `Reply(6)` | `Ack`, as a `Reply or none` |
| `Reply(7)` | `none`: no case stands for 7 |

The reasons for each choice are in chapter 35 of
[`decisions.md`](decisions.md). The example
[`examples/protocol.zeno`](../examples/protocol.zeno) is a small
serial protocol built on two such enums.

## The rules

**A number for every case, or for none.**

```
error: `Busy` has no number, and the other cases of `Reply` have one.
Give every case a number, or none
```

C counts on from the case before for a case with no number, so adding
a case in the middle changes the numbers of the cases after it. A
program that sends `Busy` as 7 today would send 8 after someone adds a
case above it, and the board at the other end would read something
else. In Zeno every number is written down, so none can move.

**Written out**, with or without a minus, in any base: `Ack = 0x06`,
`Low = -40`. Not a `const` and not a sum: the numbers are meant to be
read off the page like the table in a datasheet.

**Each number once.** `Reply(6)` has to give back one case.

**Only cases that carry nothing.** `Fault(code: int) = 3` is refused:
a case that carries values is many values, not one number.

## A case as its number

`int(Ack)` cannot fail. `u8(Ack)` is checked as `u8(n)` is, so a case
whose number does not fit is an `OverflowError` where it is converted:

```zeno
enum Level {
    Low = -40
    High = 125
}
print(u8(High))     # 125
print(u8(Low))      # OverflowError: -40 does not fit in a u8 (0 to 255)
```

A case is not a number in disguise. `Ack == 6` is refused, as is
`Ack + 1`; write `int(Ack) == 6` and the reader sees which is meant.

## A number as a case

`Reply(n)` gives `Reply or none`, because most numbers are no reply,
and a byte that came over a wire can be anything. The checker makes
the program say what happens then:

```zeno
function describe(code: int) -> str {
    let reply = Reply(code)
    if reply is none {
        return "no such reply"
    }
    match reply {
        case Ack { return "accepted" }
        case Nak { return "refused" }
    }
}
```

## How it is built

Nothing in the interpreter, the VM, C, LLVM or the fifteen target
languages knows about it. The checker finds each `int(Ack)` and
`Reply(6)`, because it knows that `Ack` is a case of an enum with
numbers and that `Reply` is such an enum, and notes where each call is
(`Checker::numbered` in `checker.rs`). Then `checker::checked` has the
program rewritten (`Program::with_numbers` in `ast.rs`). Beside the
enum, in the same block, go two functions:

```zeno
function Reply..number(value: Reply) -> int {
    match value {
        case Ack { return 6 }
        case Nak { return 21 }
    }
}
function Reply..case(number: int) -> Reply or none {
    if number == 6 { return Ack }
    if number == 21 { return Nak }
    return none
}
```

and the calls become `int(Reply..number(Ack))` and `Reply..case(6)`.
A name with two dots in a row is one no program can write, so these
cannot clash with anything. The program is checked again, and from
there on it is a program like any other, which every engine and every
target already runs: `zeno join` does not show the two functions, but
`zeno build` compiles them.

This is the same road that `count += 1` on a `u8` takes (decision
10.27): the one stage that knows the types says where, and the tree is
changed into what the program means.

An enum whose numbers no call uses gets no functions, so writing the
numbers only to document them changes nothing in what is built.

## How it is tested

- `cargo test`: `the_numbers_of_an_enum_are_the_same_in_every_build`
  runs one program in both engines, in a C build, a firmware build and
  through LLVM, and holds all of them to the same output.
  `the_numbers_of_an_enum_follow_the_rules` checks each message.
- `tools/targets.py`: `tools/targets/edge-enum-numbers.zeno` and
  `examples/protocol.zeno` in all fifteen languages.

## Things to try

1. Add `Stop = 0x30` to `Command` in `examples/protocol.zeno`, send a
   frame with it, and see what the `match` in `handle` says before you
   handle it.
2. Give two cases the same number. Read the message. Why does Zeno not
   allow what C allows here?
3. Write `enum Month` with `January = 1` to `December = 12`, and a
   function that reads a month as a number with `input()` and prints
   its name, or says that there is no such month.
4. Try `u8(Low)` for a case whose number is negative, inside a `try`.
   Which error is it, and where does it point?
5. Write a number for only some cases of an enum, and read the
   message. Then think of a protocol that changed its numbers when a
   command was added. What would have caught it?
6. Run `zeno build --target py` on a program that uses `Reply(6)`, and
   find the function `Reply..case` in the Python. What is it called
   there?
7. An enum whose cases are flags (`Read = 1`, `Write = 2`, `Run = 4`)
   is not one number per value: `Read | Write` is 3, which is no case.
   How would you keep a set of flags in a program today?
