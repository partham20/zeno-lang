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
zeno-owned-checked vm run python`. Each time is the best of three runs (one
run for the slow interpreters). C is built with `cc -O2`, Rust with `rustc -O`.

## Results (2026-11-16, 4-core Xeon 2.1 GHz, seconds)

Big sizes, compiled:

| Program | C | Rust | Zeno before | `zeno build` | `--release` | `--release --owned` |
|---|---|---|---|---|---|---|
| fib 40 | 0.153 | 0.248 | 0.460 | 0.463 | 0.143 | **0.116** |
| loop 10⁹ | 0.840 | 0.885 | 1.249 | 1.181 | 0.894 | 1.044 |
| sort 10⁷ | 1.04 | 1.16 | 1.29 | 1.33 | 1.23 | 1.30 |
| strings 10⁷ | 0.872 | 0.684 | 24.5 | 1.088 | 1.097 | 1.259 |
| trees 20 | 6.16 | 7.08 | 57.8 | 11.06 | 11.54 | **4.09** |
| nbody 10⁷ | 0.515 | 0.389 | 1.325 | **0.416** | 0.508 | 0.490 |

("Zeno before" is `zeno build` before this round of work;
`results/before-big.json`. Times move by about 10 % from run to run on
this machine, so a difference smaller than that is noise.)

Small sizes, interpreted, against Python 3.13:

| Program | Python | `zeno run --vm` | `zeno run` |
|---|---|---|---|
| fib 32 | 0.221 | 0.98 | 2.84 |
| loop 10⁸ | 11.0 | 14.9 | 30.8 |
| sort 10⁶ | 1.98 | 4.63 | 13.0 |
| strings 10⁶ | 0.41 | 1.82 | 2.91 |
| trees 16 | 3.59 | 19.2 | 29.8 |
| nbody 10⁶ | 4.07 | 23.2 | 52.3 |

## What the numbers say

- Compiled Zeno is now within about 25 % of C on every program, and
  faster than C on `fib` and `trees` with `--release --owned`, and on
  `nbody` as it is.
- What closed the gaps: a collector that keeps blocks in pages (22.5),
  `memory owned` (22.1), `--release` (22.3), loops that walk a list in
  place (22.4) or text in place (22.7), and fields read once (22.8).
- The collector is still about 2.7 times slower than ownership on
  `trees`, which makes millions of small objects. A generational
  collector would close more of that gap.
- The interpreters are slower than Python. They exist for checking and for
  the playground; `zeno build` is the fast path.

The results live in `results/*.json`, and zeno-lang.org/benchmarks draws its
page from them.
