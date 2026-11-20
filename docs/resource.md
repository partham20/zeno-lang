# Cleaning up when a block ends

Some things have a second half that must happen on every way out of a
block: a chip-select line goes high again, a lock is let go, a file is
closed. A class can now say so once, and the end of the block does it:

```zeno
owned class Selected is Resource {
    name: str

    function release(self) {
        print("deselect", self.name)
    }
}

function read_id(name: str, fail: bool) -> int raises ValueError {
    let chosen = Selected(name)
    print("select", chosen.name)
    if fail {
        raise ValueError("no answer")
    }
    return 0x9F
}

print(read_id("flash", false))
try {
    print(read_id("sensor", true))
} except ValueError as e {
    print("caught:", e)
}
```

```
select flash
deselect flash
159
select sensor
deselect sensor
caught: no answer
```

`release` ran on both roads out of `read_id`: after the `return`, and
as the error went past. Nothing was written at either. The same lines
come out of `zeno run`, the VM, every build in C and LLVM, and every
target. The decisions are 59.1 to 59.5 in
[`decisions.md`](decisions.md).

## What was added

| | Written |
|---|---|
| A class whose objects are put back when their owner ends | `owned class Selected is Resource` |
| What putting back does | `function release(self) { ... }` in that class |

`Resource` comes with Zeno: `owned interface Resource { function
release(self) }`. It is not a reserved word: a program with a
`Resource` of its own means its own, and nothing is released.

## When `release` runs

When the owner of the object ends. The owner is the `let` that holds
it, or the parameter that took it, and it ends when its block is left:

| The block is left by | `release` runs |
|---|---|
| reaching its end | yes |
| `return`, `break` or `continue` | yes; after `return x + y`, once the value is worked out |
| an error passing through | yes, and the error goes on unchanged: same kind, message and line |
| giving the object away | no: whoever has it now releases it |
| `program.exit`, or a fault of a firmware build | no: stopping the program ends no block |

Of two in one block, the one made last is released first. A loop's body
is a block, so one made in each turn is released at the end of that
turn.

**Giving it away.** A resource has one owner, as every owned value
does. `return` gives it to the caller, a parameter that says `take`
takes it, `let b = a` moves it:

```zeno
function done(held: take Selected) {
    print("done with", held.name)
}                                   # released here

let chosen = Selected("flash")
done(chosen)                        # not released at the end of this block
```

A parameter that only looks, `held: Selected`, leaves it to its owner.

**To end one sooner**, give it to a function that takes it, as above.
`release` cannot be called by hand: it runs once, by itself.

## The top of a file, and tests

The top of a file is a block. A resource made there is released after
the last line, and before an error that nothing caught is reported.

Under `zeno test` and `zeno test --build`, the top of the file ends
after its **last test**, so the tests can use what the top made:

```zeno
let bus = Selected("bus")

test "the bus is there" {
    assert bus.name == "bus"
}
```

```
bus.zeno
  ok      the bus is there
deselect bus

1 test: 1 passed, 0 failed.
```

A failed `assert` leaves its test as an error would, so what the test's
own blocks hold is released, and the next test can open the same thing.

## Where a resource can be

In a `let`, or in a parameter. Everything else is refused, with a
message that says what to write:

| Written | Why it is refused |
|---|---|
| `var h = Selected("a")` | a new value would release the old one at a line where nothing says so |
| a list, a dict, `Selected or none` | an item has no block whose end could release it |
| a field of a class, a case of an enum, a type parameter | the same |
| `print(Selected("a").name)` | nothing keeps it, so it would be released at once |
| one of several values: `return Selected("a"), 1`, `let s, n = Selected("a"), 1` | the resource would have no `let` of its own to release it |
| a default that makes one, `function f(s: Selected = Selected("d"))` | it would be made at each call that leaves it out, where no `let` owns it; pass one at the call |
| a function that is kept, using one from its block | nothing would release it when the block ends |
| a parameter with no type written | the checker could not see that a resource goes there |
| the top of a file that imports `web` | a page never reaches the end of its top |

A call that gives one may be received by a `let`, by `return`, or by a
parameter that takes it, `done(Selected("passed"))`. A resource may be
held as the interface, `let r: Resource = chosen`, and its own
`release` runs.

And of the class:

- It must be an `owned class`: a shared object has no one owner whose
  end could be the moment.
- `release` takes `self` alone and gives nothing.
- `release` may not say `raises`: at the end of a block an error has
  nowhere to go. Catch it inside.

## What it costs a program that does not use it

Nothing. A program that never names `Resource` is not rewritten by the
checker, and is built into the same C, IR and bytecode as before, to
the byte.

## How it is built

The checker is the one part that knows which `let` holds a resource.
After its rules pass, it rewrites the program: the statements after
the `let`, to the end of the block, go into a statement of a new kind,
*scoped*, that says "release this name however this is left"; and each
place that gives the resource away becomes a *give*, which empties the
name. A program cannot write either. Each engine then does what a scoped
statement means in its own way:

| Engine | A scoped statement is |
|---|---|
| `zeno run` | one arm: run the body, then release, however it ended |
| the VM | a `try` of its own, a release at its end and in its handler, and one at each `return`, `break` and `continue` that leaves it; one new instruction, `Release` |
| C and LLVM | a release in front of the free that every way out of a block already does; in an ordinary build also a frame like a `try`'s, since an error there jumps over blocks |
| the fifteen targets | a `try` that releases and raises again, written by `lower.rs` |

A release takes the object out of its variable before it calls
`release`, so it runs once whatever happens inside it.

## What it does not do

- **A resource in a field, a list, a dict, `or none`, an enum or a
  type parameter.** Left for later; letting it in will change no
  program that builds today.
- **Several values that hold a resource.** Give it back alone, and
  receive it alone.
- **A `release` that raises.**

## How it is tested

- `a_resource_goes_only_where_it_is_released` (`checker.rs`): every
  refusal, with a collector and with none, and what is taken.
- `a_resource_of_the_program_s_own_is_the_program_s`,
  `a_program_with_no_resource_is_not_rewritten`,
  `the_rest_of_a_block_after_a_resource_is_scoped`.
- `an_error_that_ends_the_program_releases_on_its_way_out`,
  `under_test_the_top_of_a_file_ends_after_its_last_test`
  (`interpreter.rs`); `a_resource_is_released_at_its_edges_as_the_interpreter_releases_it`,
  `a_resource_is_released_on_the_way_out_and_the_way_out_goes_on`,
  `program_exit_does_not_release` (`vm.rs`): each on the interpreter
  and the VM, which must agree.
- `tools/targets/edge-resource.zeno`: every way out of a block, built
  every way and run by every target.

## Things to try

1. Put a `print` after the `raise` in `read_id` and before it. Which
   comes before `deselect sensor`?

2. Make two resources in one function and leave it by `return`. In
   what order are they released, and why is that the order that a lock
   taken inside another wants?

3. Write `function done(held: take Selected) { }` and give it a
   resource in one branch of an `if` only. When is the resource
   released if the other branch runs?

4. Change `let chosen` to `var chosen`, and read what `zeno check`
   says. Why would `chosen = Selected("other")` be a release that
   nobody wrote?

5. Build the program at the top with `--no-collector --memory`. How
   many blocks are left?
