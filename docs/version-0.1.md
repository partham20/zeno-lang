# Zeno 0.1: what is in it, and what is not

This is the last page. It builds nothing. It says where the language
stands now that the list for version 0.1 is done, and it is the map
for anyone who wants to read the rest in order.

```sh
cargo run -- --version
```

```
zeno 0.1.0
```

## What 0.1 means

Every question that was open about the language has an answer.

All through the project a decision was marked *provisional* when it
was made on less than it deserved: before a later stage existed,
before a real program had tried it, or to keep a stage small. There
were about eighty. The last item on the list was to read each one
again, and chapter 11 of [`decisions.md`](decisions.md) has them all,
each with one of three answers:

| Answer | How many | What it means |
|---|---|---|
| Settled | 36 | It is the rule, and it is not waiting for anything |
| A limit of 0.1 | 30 | The thing left out stays out of this version, and the program that asks for it is told so |
| Changed | 7 | It is not as it was, and the row says which decision changed it |

Nothing was changed by the reading itself except one message. `import
drivers.uart as uart` used to say "expected the end of the line,
found `as`". A module having one name is a rule of the language
(7.3), so the message now says the rule and what to write.

The two tests each decision was held to are the ones every feature
has been held to: is the rule one sentence with one reading, and
where something is left out, does Zeno say so and say what to write
instead?

## The language, by the page that built it

| What | Pages |
|---|---|
| Text to tree | [`lexer.md`](lexer.md), [`parser.md`](parser.md) |
| Running it | [`interpreter.md`](interpreter.md), [`vm.md`](vm.md) |
| Checking it | [`checker.md`](checker.md), [`audit.md`](audit.md) |
| Compiling it | [`compiler.md`](compiler.md), [`llvm.md`](llvm.md), [`firmware.md`](firmware.md) |
| Memory | [`memory.md`](memory.md), [`ownership.md`](ownership.md), [`no-collector.md`](no-collector.md), [`one-owner.md`](one-owner.md), [`pool.md`](pool.md), [`text.md`](text.md), [`temporaries.md`](temporaries.md), [`enums-and-none.md`](enums-and-none.md), [`functions-and-errors.md`](functions-and-errors.md), [`unsafe.md`](unsafe.md) |
| Types beyond the first few | [`sized-ints.md`](sized-ints.md), [`generics.md`](generics.md), [`interfaces.md`](interfaces.md), [`private.md`](private.md) |
| The library | [`library.md`](library.md), [`reading-and-writing.md`](reading-and-writing.md), [`math.md`](math.md), [`dict.md`](dict.md), [`board.md`](board.md) |
| More than one file | [`modules.md`](modules.md), [`packages.md`](packages.md), [`new.md`](new.md) |
| Tools | [`format.md`](format.md), [`lsp.md`](lsp.md), [`testing.md`](testing.md) |
| The last gaps | [`gaps.md`](gaps.md) |

Every one of them ends with things to try, and the reasons for all of
it are in [`decisions.md`](decisions.md).

## What is not in 0.1

The limits, gathered from the five tables of chapter 11. Each is
refused today with a message, so each can be added later without
changing what any program written now means.

- **On a board:** reading a pin, an ADC or a UART. This is the one
  item that was taken off the list for 0.1, because it has to be
  designed against real hardware.
- **Sized numbers:** a `list[u8]` that takes one byte an item, a
  `u64`, and reading a file as bytes.
- **With no collector:** a dict whose values are lists or objects, a
  `list[int or none]`, and the few moves of an owned value that have
  no rule yet for what is left behind.
- **Types:** interfaces with type parameters, and the bounds on a
  generic that come with them. Unions other than `or none`. Units
  that are made from other units.
- **The library:** `sort` by a function, a file read in pieces,
  `program.exit`, random numbers, and a library written in Zeno
  itself.
- **Tools:** compiling one file at a time, hover on a field, choosing
  tests by name, and tests of a compiled program. (Hover on a field
  and `zeno test --only` came in 0.2: decisions 17.4 and 17.5.)

And three things are not limits, because they are decided: Zeno has
no inheritance, no second way to write a function, and no second name
for a module.

## Things to try

1. Open chapter 11 of `decisions.md` and pick one row that says
   "a limit of 0.1". Write the smallest program that asks for it, and
   run it. Does the message say what to write instead? If it does
   not, that message is the first thing to mend.
2. Pick a row that says "settled" and that you disagree with. Find
   the decision it points at and read what was turned down. Write
   your case as that decision does: what, why, and what it costs.
3. `grep -c "was provisional" docs/decisions.md`. Each of those marks
   points at a section of chapter 11. Take three at random and check
   that the row is there.
4. Choose one limit and work out which stages it touches: the lexer,
   the parser, the checker, the four engines, the runtime. Count them
   before you start. `u8.max` touches the fewest, and a packed
   `list[u8]` the most.
5. Read the last paragraph of chapter 11. Find a feature in another
   language that could not have been added the same way: one whose
   arrival changed what existing programs mean.
