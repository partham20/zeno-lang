#!/usr/bin/env python3
"""Time two builds of Zeno against each other: is the new one slower?

    python3 bench/compare.py --old path/to/old/zeno [--new target/release/zeno]
                             [--rounds 4] [--engines vm,run]

Runs every benchmark with `zeno run --vm` (and `zeno run`, if asked) in
both builds, in rounds: old, new, old, new. The best time of each is
kept, so a machine that is busy for a while slows both and not one.
It is the check before a release that adds code anywhere: the release
before is built from its tag in a `git worktree` and given as `--old`
(decision 24.29). `run.py --count` counts instructions; this times
them, and it takes both to know.
"""
import argparse, os, subprocess, time

HERE = os.path.dirname(os.path.abspath(__file__))
SIZES = {"fib": 32, "loop": 100000000, "sort": 1000000, "strings": 1000000, "trees": 16, "nbody": 1000000}
ENGINES = {"vm": ["run", "--vm"], "run": ["run"]}

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--old", required=True, help="the `zeno` to compare with")
    ap.add_argument("--new", default=os.path.join(HERE, "..", "target", "release", "zeno"))
    ap.add_argument("--rounds", type=int, default=4)
    ap.add_argument("--engines", default="vm")
    ap.add_argument("--only", default=",".join(SIZES))
    a = ap.parse_args()
    builds = {"old": os.path.abspath(a.old), "new": os.path.abspath(a.new)}
    best = {}
    for _ in range(a.rounds):
        for engine in a.engines.split(","):
            for prog in a.only.split(","):
                for name, zeno in builds.items():
                    t0 = time.perf_counter()
                    r = subprocess.run([zeno, *ENGINES[engine], os.path.join(HERE, prog + ".zeno"), str(SIZES[prog])],
                                       capture_output=True, text=True)
                    t = time.perf_counter() - t0
                    if r.returncode:
                        raise SystemExit(f"{prog} failed in the {name} build:\n{(r.stdout + r.stderr)[-400:]}")
                    key = (engine, prog, name)
                    best[key] = min(best.get(key, t), t)
    for engine in a.engines.split(","):
        for prog in a.only.split(","):
            old, new = best[(engine, prog, "old")], best[(engine, prog, "new")]
            print(f"{prog:8} zeno {' '.join(ENGINES[engine]):9} old {old:7.2f}  new {new:7.2f}  {(new / old - 1) * 100:+5.1f} %")

if __name__ == "__main__":
    main()
