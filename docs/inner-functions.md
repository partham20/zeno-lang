# A function inside a function, and a default that is worked out

`zeno run` has always taken a function with a name written inside
another function, and a default value that is more than a literal.
Until 0.8.0 `zeno build` refused both:

```
error: `zeno build` cannot compile a function inside a function or block yet. `zeno run` can still run this program
error: `zeno build --llvm` cannot compile a default value that is not a plain literal yet. `zeno run` can still run this program
```

Now every build takes them, through C and through LLVM, with a
collector, for firmware and with no collector:

```zeno
let base = 10

function scaled(readings: list[int], factor: int) -> list[int] {
    let offset = base * 2
    function adjust(r: int) -> int {
        return clamp(r * factor) + offset
    }
    function clamp(r: int, top: int = base * 100) -> int {
        if r > top {
            return top
        }
        return r
    }
    var out: list[int] = []
    for r in readings {
        out.append(adjust(r))
    }
    return out
}

print(scaled([3, 400, 7], 5))
```

```sh
cargo run -- build --no-collector --memory scaled.zeno
./scaled
```

```
[35, 1020, 55]
memory: ...
```

`zeno run` prints the same line, and so does every other build. The
reasons for each choice are in 56.16 of [`decisions.md`](decisions.md).
It is part 9.8 of [`plan-9-14.md`](plan-9-14.md).

Nothing changes for a program that has neither: its C and its IR are
what 0.7.0 wrote. Only programs that a build refused before build in a
new way.

## The rule

> A function with a name, written inside a function, is a function of
> the file if it uses nothing of the function around it. If it uses
> something, it is a closure, made once, after the last thing it uses
> is declared.

Each of the three cases follows from that:

| The function inside | Built as | So |
|---|---|---|
| uses only its parameters, the file's names and other functions like itself (`clamp` above) | a function of the file, under a name of its own (`zeno_inner_1_clamp` in C, `@zeno.inner.1.clamp` in LLVM) | it can be called above its line, two of them can call each other, and it costs what any function costs |
| uses a `let`, a `var` or a parameter of the function around it (`adjust` uses `factor` and `offset`) | a closure, made once, just after the last of those is declared | the checker already refuses any use above that line ("`k` is declared further down than the first use of ..."), so every use the checker takes is below it |
| two of the second kind that call each other | refused | see below |

Inside a closure its own name is the closure, so it can call itself.

## Defaults that are worked out

A default that is more than a literal is worked out at each call that
leaves its argument out, in the order of the parameters. It reads what
it would read where the function is written: the file's variables and
functions, the parameters before it, and `self` in a method. Never a
variable of the function that calls:

```zeno
let base = 10
var calls = 0

function bump() -> int {
    calls += 1
    return calls
}

function total(a: int, b: int = a * 2, c: int = base + bump()) -> int {
    return a + b + c
}

function elsewhere() -> int {
    let base = 1000     # not the `base` that `total` sees
    return total(1)
}

class Meter {
    scale: float = 1.0
    function read(self, raw: float, at: float = raw * self.scale) -> float {
        return at
    }
}
```

`bump()` runs once for each call that leaves `c` out, and not at all
for one that gives it. The arguments that are given are worked out
first, left to right as they are written, and then the defaults.

## What is refused, and why

Each with a message that says what to do:

- **Two functions inside a function that call each other and use
  variables of the function around them.** Each would have to be made
  before the other. Give one of them to the other as a parameter. Left
  for later.
- **A default that is more than a literal, on a function of the second
  kind** (one that uses something around it). Left for later; give the
  argument at the call.
- **A generic function inside a function.** Write it at the top of the
  file.
- **With no collector, a function of the second kind used as a value**
  (`return add`, `xs.map(add)`): the block owns the closure and frees
  it as it ends, so it can be called and not given away. Write it as
  `let add = function(...) { ... }`, which has one owner and can be
  given. Nor can a function written where a value goes call it, since
  that one may outlive the block.
- **With no collector, one that changes a `var` around it**: as for
  any function written inside a function, the two would share the
  variable, and with no collector nothing is shared
  ([`no-collector.md`](no-collector.md)).

A function of the first kind has none of these limits: it is a
function of the file.

## What it costs

A function of the first kind costs nothing more than a function
written at the top of the file. One of the second kind is a closure: a
block that holds the values it uses, made once when its block reaches
the line, and given back when the block ends. A call through it is a
call through a function value. A default that is worked out costs what
its expression costs, at each call that leaves the argument out.

## In the C

```c
static int64_t z_g(int64_t z_n) {
    int64_t z_a = zeno_inner_1_sq(2);
    int64_t z_k = zeno_mul(z_n, 2, 6, 13);
    struct zeno_env_1 *t1 = (struct zeno_env_1 *)zeno_alloc(sizeof(struct zeno_env_1));
    t1->base.code = (zeno_code)zeno_fn_1;
    t1->base.name = "add";
    t1->z_k = z_k;
    zeno_closure *z_add = (&t1->base);
    return zeno_add(z_a, ((int64_t (*)(zeno_closure *, int64_t))z_add->code)(z_add, 1), 10, 12);
}
```

is the C of

```zeno
function g(n: int) -> int {
    let a = sq(2)
    function sq(x: int) -> int {
        return x * x
    }
    let k = n * 2
    function add(x: int) -> int {
        return x + k
    }
    return a + add(1)
}
```

`sq` uses nothing around it and is called above its line; `add` is
made after `let k`, the last thing it uses.

## How it is tested

- **Every build**: `functions_inside_functions_every_way` builds
  [`tools/targets/edge-inner-functions.zeno`](../tools/targets/edge-inner-functions.zeno)
  through C and LLVM, with a collector, for firmware and with no
  collector, and with memory that grows; each prints what `zeno run`
  prints, with 0 blocks left. The program has a call above the line,
  two functions of the first kind that call each other, one that calls
  itself, one made in a loop, one inside a method, one inside one
  inside one, one that raises, and defaults that read earlier
  parameters, the file's variables, a function with a side effect and
  `self`.
- A `var` changed after the function was written, and a function
  returned as a value, with a collector.
- Where each kind goes in the C and the IR, and every refusal above,
  word for word, through both compilers.
- **The targets** run the same program.

## Things to try

1. Build the program at the top and open its C. Which of the two
   functions inside is a function of the file, and why?
2. Move `let offset` below `function adjust`. What does the checker
   say, and why does the build not need to say anything more?
3. Make `clamp` read `offset`. What changes in the C?
4. Write `return adjust` at the end of `scaled`, with a return type to
   match, and build with and without `--no-collector`.
5. Give `total` a default that calls `bump()` twice. How many times
   does `bump` run for `total(1)`, and for `total(1, 2, 3)`?
