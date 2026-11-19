# A method used as a value

`counter.bump`, written without the brackets of a call, is a function:
the method, with `counter` already in it. `zeno run` always took it.
Until 0.8.0 both compilers refused it, in every build:

```zeno
class Counter {
    count: int
    function bump(self, by: int) -> int {
        self.count += by
        return self.count
    }
}

function apply(f: function(int) -> int, n: int) -> int {
    return f(n)
}

function counter(start: int) -> function(int) -> int {
    let c = Counter(start)
    return c.bump
}

let c = Counter(0)
print(apply(c.bump, 5), [1, 2, 3].map(c.bump), c.count)

let next = counter(100)
print(next(1), next(1))
```

```sh
cargo run -- build --no-collector --memory counter.zeno
./counter
```

```
5 [6, 8, 11] 11
101 102
memory: 9 blocks made (208 bytes), at most 168 bytes in use at once, no collector, 0 blocks left at the end
pool: 288 bytes taken, in a pool of 32768
text: at most 23 bytes in use at once, in a room of 1024
```

Every build prints the same two lines, and so does `zeno run`. The
reasons for each choice are in 56.9 of [`decisions.md`](decisions.md).
It is the second half of part 9.2 of [`plan-9-14.md`](plan-9-14.md),
and follows on from [`kept-functions.md`](kept-functions.md).

## The rule

> `c.bump` is the function `function(by: int) -> int { return c.bump(by) }`.

So everything [`kept-functions.md`](kept-functions.md) says of a
function written where a value goes is true of it, in the same words:

- **Passed along**, to `map` or to a parameter that looks, it looks at
  `c`, and is gone at the end of its line.
- **Kept**, returned or put in a variable, a field or a list, it takes
  `c` where `c` has one owner. After that line `c` holds nothing, and
  the function frees the object when it is freed. That is how
  `counter(100)` above keeps its own `Counter`.
- **With a collector**, where the object is shared, it shares it.

Its type is the method's, without `self`: `c.bump` is a
`function(int) -> int`. What the method raises is in the type too
([`raises-in-types.md`](raises-in-types.md)), so a call through it is
checked as a call of the method is.

## It holds the object of that line

`zeno run` reads `c` when it meets `c.bump`, and keeps that object. A
`var` given another object later does not change it:

```zeno
var c = Counter(0)
let m = c.bump
c = Counter(100)
print(m(1), c.bump(1))
```

This prints `1 101` in every build. The function the compilers make
holds what `c` held on the `let m` line, not the variable `c`.

## What it cannot be

The object must be one a variable holds: `x.f`, with `x` a name. It can
be a `let`, a `var`, a parameter, a loop's variable or `self`. The
object can also be one of an interface, and then it calls the method
of the object's own class.

With one owner, the rules of a kept function hold, in their words. A
parameter that only looks has nothing to give:

```
error: this function is kept, so it takes `p`, but `p` is only looked at here: its owner still has it. If it is a parameter, write `take` in front of its type to take the value over
```

`self` cannot be taken, since `take` cannot be written on it:

```
error: this function is kept, so it takes `self`, but `self` is the object the method was called on, and its owner still has it. Put what the function needs in a `let` first, and use that
```

Three things are still refused by both compilers, as they were, and
`zeno run` still takes them:

- **A method of an object that no variable holds**: `make().size`,
  `a.b.size`, `xs[0].size`. Put the object in a `let` first.
- **A method of a copy of a generic class**, or a method with types of
  its own.
- **A method that comes with Zeno**: `xs.append`, `text.upper`.

## In the C

The checker writes the function out, and the compilers build it as
they build any function written where a value goes. For `counter`
above, with no collector:

```c
static int64_t zeno_fn_3(zeno_closure *zeno_self, int64_t z_a0) {
    struct zeno_env_3 *env = (struct zeno_env_3 *)zeno_self;
    return z_Counter_bump(env->z_c, z_a0);
}

static zeno_closure *z_counter(int64_t z_start) {
    z_Counter *z2_c = zeno_new_Counter(z_start);
    struct zeno_env_3 *t19 = (struct zeno_env_3 *)zeno_alloc(sizeof(struct zeno_env_3));
    t19->base.code = (zeno_code)zeno_fn_3;
    t19->base.name = "bump";
    t19->base.drop = zeno_fn_drop_3;
    t19->z_c = z2_c;
    z2_c = NULL;
    zeno_closure *t20 = (&t19->base);
    zeno_drop_Counter(z2_c);
    return t20;
}
```

The block holds the object it took, and its `drop` frees it. The name
in the block is the method's, so `print(m)` shows `<function bump>`, as
`zeno run` does.

## How it is tested

- **The checker**: `a_method_used_as_a_value_is_a_function_that_calls_it`
  covers its type, what it raises, passed along, kept, taken, a `var`
  and `self` refused with no collector, an interface, and
  `make().size` refused as before.
- **Both compilers**: `a_method_used_as_a_value_every_way` builds
  [`tools/targets/edge-method-values.zeno`](../tools/targets/edge-method-values.zeno)
  with no collector, through C and through LLVM. Both give the same
  memory report, with 0 blocks left, and it is also built in the
  ordinary and firmware builds of both. The program:
  - passes methods along;
  - keeps one, and takes one with its object;
  - goes through an interface;
  - fails a thousand times through a raising method;
  - makes a thousand of them in a loop.

  The same test holds the binding to the object of that line, at the
  top of a file and inside a function. It also takes an object of an
  `owned class` with a collector.
- **The targets** run the same program.

## Things to try

1. Build `counter.zeno` with `--no-collector` and open `counter.c`. Find
   `zeno_fn_drop_3`. Whose `Counter` does it free?
2. In `counter`, print `c.count` after `return c.bump`. Read the
   message. Why is it the one `let d = c` would get?
3. Write `let m = c.bump` in a function where `c` is a parameter
   without `take`, and return `m`. Which word does the message ask for?
4. Run the program of "It holds the object of that line" with
   `zeno run`, then build it. Now write the function out by hand,
   `let m = function(by: int) -> int { return c.bump(by) }`. Why does
   it print something else, and why can that never happen with
   `c.bump`?
5. Give `Counter` a method `check(self, text: str) -> int raises
   ValueError` and keep `c.check` in a variable. Call it in a function
   that does not say `raises`. What does a strict check say?
6. Try `let f = [Counter(1)][0].bump` in a build. Then put the object
   in a `let` first. Why does the second build and the first not?
