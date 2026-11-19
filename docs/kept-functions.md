# A function that is kept takes what it uses

A function written where a value goes can use the values around it.
Until 0.8.0 a build with no collector let such a function only look at
a list, an object or another function from outside it, so it had to be
gone by the end of its line. `compose` could not be written:

```zeno
function compose(f: take function(int) -> int, g: take function(int) -> int) -> function(int) -> int {
    return function(n: int) -> int { return f(g(n)) }
}

function add(k: int) -> function(int) -> int {
    return function(n: int) -> int { return n + k }
}

class Sensor {
    name: str
    readings: list[int]
}

function watcher(sensor: take Sensor) -> function() -> int {
    return function() -> int {
        var total = 0
        for r in sensor.readings {
            total += r
        }
        return total
    }
}

let h = compose(add(1), add(10))
print(h(5), h(100))

let w = watcher(Sensor("t", [1, 2, 3]))
print(w())

let names = ["low", "high"]
let label = function(n: int) -> str { return names[n % 2] }
print(label(3))
```

```sh
cargo run -- build --no-collector --memory compose.zeno
./compose
```

```
16 111
6
high
memory: 10 blocks made (264 bytes), at most 264 bytes in use at once, no collector, 0 blocks left at the end
pool: 400 bytes taken, in a pool of 32768
text: at most 7 bytes in use at once, in a room of 1024
```

`zeno run` prints the same three lines, and so does every other build.
The reasons for each choice are in 56.8 of
[`decisions.md`](decisions.md). It is part 9.2 of
[`plan-9-14.md`](plan-9-14.md), and follows on from
[`functions-and-errors.md`](functions-and-errors.md).

## The rule

> A function that is kept takes what it uses.

"Kept" means the function outlives its line: it is returned, put in a
variable or a field, appended to a list, or given to a parameter that
says `take`. Then each value with one owner that it uses is given to
it, as `let b = a` would give it. The function owns them from then on,
and frees them when it is freed.

A function that is only passed along, to `map` or to a parameter that
looks, looks as before. It is gone at the end of its line, and the
owner still has what it looked at:

```zeno
let xs = [1, 2]
print([1, 2].map(function(n: int) -> int { return n + len(xs) }), len(xs))
```

**No word marks it.** Rust writes `move` in front of such a function.
In Zeno the line already says the function is kept: it follows
`return` or `=`, or goes to a parameter whose type says `take`. A
function that is kept and still only looked would be gone before it
could be called, so there is nothing for a word to tell apart.

**A `let` at the top of the file is the exception**, as it always was.
It is there until the program ends, so `label` above only looks at
`names`.

## What it can take

Only what the function it is written in has to give: a value of its
own, or a parameter that says `take`.

A parameter that only looks has nothing to give:

```
error: this function is kept, so it takes `xs`, but `xs` is only looked at here: its owner still has it. If it is a parameter, write `take` in front of its type to take the value over
```

A value that belongs to a function further out cannot be taken by a
function written two levels in. The function in between may be called
again, and would have nothing left to give the second time:

```
error: this function is kept, so it takes `xs`, but `xs` belongs to a function further out, and the function in between may be called again with nothing left to give. Hand `xs` to the function in between as a parameter that says `take`
```

Everything else is the rule of `let b = a`, word for word, because it is
the same code (`give_away` in the checker). After the line, the owner
holds nothing:

```
error: `xs` was given away on line 3 and holds nothing now. Use it before it goes, or give it only to what looks: a parameter without `take`
```

It cannot be taken on every trip round a loop, nor while a loop walks
it, nor on a line that also uses it, and each of those says so in the
words `let b = a` gets.

A `var` of the function around stays refused: with no collector nothing
is shared, and a function that changed it would change the other's.

## In the C

The function's block holds what it took. Making the block moves each
value in, and leaves `NULL` behind, so the end of `watcher` frees
nothing:

```c
static zeno_closure *z_watcher(z_Sensor *z_sensor) {
    struct zeno_env_4 *t11 = (struct zeno_env_4 *)zeno_alloc(sizeof(struct zeno_env_4));
    t11->base.code = (zeno_code)zeno_fn_4;
    t11->base.name = "";
    t11->base.drop = zeno_fn_drop_4;
    t11->z_sensor = z_sensor;
    z_sensor = NULL;
    zeno_closure *t12 = (&t11->base);
    zeno_drop_Sensor(z_sensor);
    return t12;
}
```

The block's own `drop` frees each value by its kind, then the block:

```c
static void zeno_fn_drop_4(zeno_closure *zeno_self) {
    struct zeno_env_4 *env = (struct zeno_env_4 *)zeno_self;
    zeno_drop_Sensor(env->z_sensor);
    zeno_free(env);
}
```

For `compose` the same function frees `f` and `g` with
`zeno_closure_drop`. Through LLVM it is the same: the value is loaded
from its slot, `null` is stored there, and the block's drop function
calls what frees that kind.

With a collector, and no `owned` written, nothing has one owner, so
nothing moves and the C is what it was. `tools/same.py` shows that
every program that was there before 0.8.0 builds into the same C, IR
and bytecode.

## A function called straight from the call that made it

`make(2)("5")` calls the function that `make` returned, and keeps it
nowhere. With no collector it was never freed, in 0.7.0 as well: one
block was left for each such call. It is now the statement's, like any
value made in the middle of a line, and is freed when the line is over,
whether the call worked or raised.

## How it is tested

- **The checker**: `a_function_that_is_kept_takes_what_it_uses` has
  `compose`, and each refusal above in its words.
  `with_one_owner_a_function_value_has_one` and
  `an_owned_value_stays_in_the_function_that_holds_it` changed where a
  function that was refused now takes what it uses.
- **Both compilers**: `a_kept_function_takes_what_it_uses_every_way`
  builds [`tools/targets/edge-kept-functions.zeno`](../tools/targets/edge-kept-functions.zeno)
  with no collector, through C and through LLVM, with the same memory
  report from both and 0 blocks left, and in the ordinary and firmware
  builds of both. The program keeps functions in a variable, a field
  and a list, gives one to a parameter that says `take`, makes them a
  thousand times in a loop, calls them after the function that made
  them has returned, fails a thousand times through a type that says
  `raises`, and calls `make(2)("5")` on both paths. It also holds the C
  of `compose` to what this page shows.
- **The targets** run the same program.

## Things to try

1. Build `compose.zeno` with `--no-collector` and open `compose.c`. Find
   `zeno_fn_drop_2`. What would go wrong if it did not free `f` and `g`?
   What if `z_compose` did not set them to `NULL`?
2. In `watcher`, print `sensor.name` after the `return function`. Read
   the message. Why is it the same message as for `let s = sensor`?
3. Take `take` off the parameter of `watcher`. Which line is refused,
   and what does the message tell you to write? Why does an ordinary
   build take the program as it is?
4. Write `function counter() -> function() -> int` that keeps a count.
   Try a `var` first and read the message. Then keep the count in an
   object that the function takes. Does each call of `counter()` get
   its own count?
5. Put `let label = function(n: int) -> str { return names[n % 2] }`
   inside a function, with `names` a `let` of that function. Build with
   `--memory`. Who frees `names` now?
6. Call `make(2)("x")` inside a `try`, a thousand times, in a build with
   no collector. How many blocks are left at the end? Build the same
   program with the `zeno` of 0.7.0 and compare.
