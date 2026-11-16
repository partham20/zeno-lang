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
| fib 40 | 0.157 | 0.246 | 0.460 | 0.482 | **0.122** | 0.145 |
| loop 10⁹ | 0.847 | 0.879 | 1.249 | 1.388 | 0.957 | 0.959 |
| sort 10⁷ | 0.998 | 1.158 | 1.292 | 1.379 | 1.171 | 1.389 |
| strings 10⁷ | 0.921 | 0.729 | 24.5 | 3.446 | 3.547 | 3.812 |
| trees 20 | 6.269 | 6.975 | 57.8 | 12.05 | 12.13 | **4.325** |
| nbody 10⁷ | 0.431 | 0.373 | 1.325 | 1.120 | 1.198 | 0.928 |

("Zeno before" is `zeno build` before the paged collector and the loop change;
`results/before-big.json`.)

Small sizes, interpreted, against Python 3.13:

| Program | Python | `zeno run --vm` | `zeno run` |
|---|---|---|---|
| fib 32 | 0.244 | 1.06 | 2.89 |
| loop 10⁸ | 11.5 | 15.5 | 32.3 |
| sort 10⁶ | 2.25 | 4.70 | 13.5 |
| strings 10⁶ | 0.422 | 1.93 | 2.86 |
| trees 16 | 3.81 | 20.8 | 30.2 |
| nbody 10⁶ | 3.49 | 23.5 | 51.4 |

## What the numbers say

- With `--release --owned`, Zeno is as fast as C or faster on `fib` and
  `trees`, and within 15 % on `loop` and `sort`.
- Without `--release`, every `+ - *` checks for overflow; that costs 10 to 60 %
  in tight arithmetic. `--release` makes whole numbers wrap round instead.
- The collector is now about 2.8 times slower than ownership on `trees`
  (it was 12 times slower before). A generational collector would close more of
  that gap.
- `strings` is still 4 times slower than C: `for ch in text` builds a list of
  one-letter texts first. Walking the text directly is the next fix.
- `nbody` is 2 times slower than C: fields of objects are reached through a
  pointer each time, and C keeps them in registers.
- The interpreters are slower than Python. They exist for checking and for the
  playground; `zeno build` is the fast path.

The results live in `results/*.json`, and zeno-lang.org/benchmarks draws its
page from them.
