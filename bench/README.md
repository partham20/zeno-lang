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
run for the slow interpreters). C is built with `cc -O2`, Rust with `rustc -O`.

## Results (2026-11-16, 4-core Xeon 2.1 GHz, seconds)

Big sizes, compiled:

| Program | C | Rust | `zeno build` | `--release` | `--release --owned` | `--fast --owned` |
|---|---|---|---|---|---|---|
| fib 40 | 0.146 | 0.273 | 0.483 | 0.138 | 0.119 | **0.129** |
| loop 10⁹ | 0.841 | 0.870 | 1.350 | 0.948 | 0.995 | **0.987** |
| sort 10⁷ | 1.067 | 1.117 | 1.360 | 1.470 | 1.088 | **1.106** |
| strings 10⁷ | 0.899 | 0.725 | 0.847 | 0.799 | 0.745 | **0.704** |
| trees 20 | 6.294 | 7.076 | 7.468 | 6.732 | 3.816 | **3.735** |
| nbody 10⁷ | 0.413 | 0.366 | 0.495 | 0.503 | 0.484 | **0.496** |

(Times move by 10 to 15 % from run to run on this machine, so a
difference smaller than that is noise. The first `zeno build` times,
before any of the speed work, are in `results/before-big.json`.)

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

- `zeno build --fast --owned` (0.4, decision 23.1) is level with C or
  ahead of it on fib, sort, strings and trees, and with Rust on all of
  them. Two gaps are left. On loop, Zeno's `%` rounds toward minus
  infinity (it pairs with `//`), which costs a few instructions that C's
  and Rust's `%` do not pay. On nbody, Rust knows the two bodies of its
  inner loop are different and keeps one in registers (23.4).

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
