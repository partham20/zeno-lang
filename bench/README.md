# Benchmarks

Six small programs, each written four times: in Zeno, C, Rust and Python.
Every version prints the same answer, and `run.py` checks each output
against the C output before it trusts a time.

| Program | What it stresses |
|---|---|
| `fib` | function calls (naive recursive Fibonacci) |
| `loop` | a tight loop of whole-number arithmetic |
| `sort` | a list of made-up numbers, sorted by a hand-written quicksort |
| `strings` | building and joining many small texts |
| `trees` | making and dropping millions of small objects (binary trees) |
| `nbody` | floating-point maths on fields of objects |

## Running them

```
cargo build --release
cd bench
python3 run.py --size big                       # the sizes on the website
python3 run.py --size small --engines c,python,vm,run
python3 run.py --json results/mine.json         # keep the numbers
```

`--engines` picks from `c rust zeno zeno-llvm zeno-nc zeno-release zeno-owned
zeno-owned-checked zeno-fast zeno-fast-gc vm run python` (`zeno-fast` is
`--fast --owned`, `zeno-fast-gc` is `--fast` with the collector). Each time is the best of three runs (one
run for the slow interpreters), taken in rounds: every engine once,
then every engine again. `--count` counts instructions instead. C is built with `cc -O2`, Rust with `rustc -O`.

## Results (2026-11-16, 4-core Xeon 2.1 GHz, seconds)

Big sizes, compiled:

| Program | C | Rust | `zeno build` | `--release` | `--release --owned` | `--fast --owned` |
|---|---|---|---|---|---|---|
| fib 40 | 0.167 | 0.204 | 0.473 | 0.144 | 0.156 | **0.167** |
| loop 10⁹ | 0.875 | 0.874 | 1.110 | 0.921 | 0.970 | **0.913** |
| sort 10⁷ | 1.009 | 1.160 | 1.422 | 1.243 | 1.096 | **1.095** |
| strings 10⁷ | 0.936 | 0.714 | 0.867 | 0.788 | 0.741 | **0.689** |
| trees 20 | 6.162 | 6.889 | 7.740 | 6.826 | 3.441 | **3.599** |
| nbody 10⁷ | 0.477 | 0.359 | 0.510 | 0.510 | 0.472 | **0.473** |

(Times move by a tenth and more from run to run on this machine, so a
difference smaller than that is noise: `--release` and `--release
--owned` run the very same loop on `loop`. The first `zeno build`
times, before any of the speed work, are in `results/before-big.json`.)

The same programs counted, not timed: millions of instructions run,
under valgrind (`python3 run.py --count`, at smaller sizes because
valgrind is slow). A count is the same on every run, so it shows what
the clock cannot. It is not a time: a copy between registers costs
nothing, a square root costs twenty additions, and memory that is far
away costs more than either. The last column is `--fast --owned`
divided by C.

| Program | C | Rust | `zeno build` | `--release` | `--release --owned` | `--fast --owned` | × C |
|---|---|---|---|---|---|---|---|
| fib 30 | 26.5 | 26.8 | 38.0 | 24.1 | 24.0 | **28.1** | 1.06 |
| loop 2×10⁷ | 300.3 | 290.4 | 460.3 | 340.3 | 340.3 | **300.3** | 1.00 |
| sort 3×10⁵ | 73.3 | 78.7 | 121.5 | 106.2 | 92.9 | **72.7** | 0.99 |
| strings 3×10⁵ | 304.2 | 173.5 | 188.1 | 184.9 | 176.2 | **174.7** | 0.57 |
| trees 14 | 699.1 | 924.8 | 499.1 | 467.5 | 328.6 | **288.5** | 0.41 |
| nbody 2×10⁵ | 120.5 | 82.8 | 126.3 | 129.9 | 129.1 | **119.5** | 0.99 |

(`results/now-count.json`; `results/before-count.json` is the same
count before decisions 23.5, 23.6 and 23.8.)

Small sizes, interpreted, against Python 3.13:

| Program | Python | `zeno run --vm` | `zeno run` |
|---|---|---|---|
| fib 32 | 0.23 | 0.79 | 2.86 |
| loop 10⁸ | 11.33 | 6.81 | 32.80 |
| sort 10⁶ | 2.06 | 3.31 | 12.85 |
| strings 10⁶ | 0.40 | 1.76 | 2.91 |
| trees 16 | 3.95 | 10.01 | 22.01 |
| nbody 10⁶ | 3.85 | 15.78 | 52.31 |

## What the numbers say

- `zeno build --fast --owned` (0.4, decisions 23.1, 23.6 and 23.8) runs
  no more instructions than C on five of the six programs, far fewer
  on strings and trees, and 6 % more on fib, where the time is the
  same. On `loop` it runs the instructions C runs (300.27 million
  against 300.26 million): it gives the C compiler C's own signed `+`,
  `-` and `*`, so the compiler knows what it knows of a C program, that
  a count going up from 0 is never below zero, and Zeno's `%` (which
  rounds the other way from C's for numbers below zero) needs no
  mending.
- Rust is ahead of C and of Zeno on `nbody` alone, by a third. Its
  compiler sees that the list of bodies is exactly five long and builds
  both loops for five, two pairs of bodies at a time. With the list
  behind `std::hint::black_box` the same Rust takes 125.8 million
  instructions and 0.51 s, behind C and Zeno. Nothing in Zeno was
  changed to chase it (23.4).

- Compiled Zeno is now within about 25 % of C on every program, and
  faster than C on `fib` and `trees` with `--release --owned`, and on
  `nbody` as it is.
- What closed the gaps: a collector that keeps blocks in pages (22.5),
  `memory owned` (22.1), `--release` (22.3), loops that walk a list in
  place (22.4) or text in place (22.7), and fields read once (22.8).
- Since the collector has two generations (22.10), `trees` with it is
  7.7 s instead of 11.1 s, within 25 % of C. Ownership is still twice
  as fast there.
- `zeno run --vm` is ahead of Python on `loop` since its superinstructions
  (22.13), and two to four times slower on objects, calls and text. The
  interpreters exist for checking and for the playground; `zeno build` is
  the fast path.

`python3 record.py --version X` keeps a record of one version in
`results/versions/` (see docs/release.md).

The results live in `results/*.json`, and zeno-lang.org/benchmarks draws its
page from them.
