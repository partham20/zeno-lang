#!/usr/bin/env python3
"""Record the benchmark times of one Zeno version, to keep a history.

    python3 bench/record.py --version 0.3.0 [--zeno path/to/zeno]

Runs run.py three times (the big sizes for the compiled engines, the
small sizes for the engines that interpret, and the small sizes for the
program written in each other language) and writes
bench/results/versions/<version>.json. C, Rust and Python run each time
too, so a version's times can be read against C's on the same machine:
the history page shows times as "times C", which stays fair when the
machine changes between releases.

An engine the version does not have yet (0.1.0 has no --release, for
example) fails to build; it is left out of the file instead of being
recorded as a failure. docs/release.md says when to run this.
"""
import argparse, json, os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BIG = "c,rust,zeno,zeno-llvm,zeno-nc,zeno-release,zeno-owned,zeno-fast,zeno-fast-gc"
SMALL = "c,python,vm,run"
# Since 0.5: the Zeno program written in another language and run
# there, at the small sizes, beside C and the VM.
TARGETS = "c,vm,to-js,to-py,to-php,to-lua,to-ruby,to-dart,to-kotlin,to-go,to-java,to-cs,to-cpp,to-rust,to-wasm"

def run(zeno, size, engines, timeout):
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "r.json")
        subprocess.run([sys.executable, os.path.join(HERE, "run.py"), "--zeno", zeno, "--size", size,
                        "--engines", engines, "--timeout", str(timeout), "--json", out], check=True)
        with open(out) as f:
            r = json.load(f)
    # Keep only numbers: a failed or timed-out engine is a missing engine.
    for prog, times in r["times"].items():
        r["times"][prog] = {k: v for k, v in times.items() if isinstance(v, (int, float))}
    used = {k for times in r["times"].values() for k in times}
    r["engines"] = {k: v for k, v in r["engines"].items() if k in used}
    return r

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True, help="the version, such as 0.3.0")
    ap.add_argument("--zeno", default=os.path.join(HERE, "..", "target", "release", "zeno"))
    ap.add_argument("--timeout", type=int, default=900)
    a = ap.parse_args()
    zeno = os.path.abspath(a.zeno)
    big = run(zeno, "big", BIG, a.timeout)
    small = run(zeno, "small", SMALL, a.timeout)
    targets = run(zeno, "small", TARGETS, a.timeout)
    record = {"version": a.version, "machine": big["machine"],
              "big": {k: big[k] for k in ("sizes", "engines", "times")},
              "small": {k: small[k] for k in ("sizes", "engines", "times")}}
    # A version from before `--target` has none of these engines.
    if any(k.startswith("to-") for k in targets["engines"]):
        record["targets"] = {k: targets[k] for k in ("sizes", "engines", "times")}
    folder = os.path.join(HERE, "results", "versions")
    os.makedirs(folder, exist_ok=True)
    path = os.path.join(folder, a.version + ".json")
    with open(path, "w") as f:
        json.dump(record, f, indent=2)
    print("wrote", path)

if __name__ == "__main__":
    main()
