#!/usr/bin/env python3
"""Time the programs in this folder in Zeno, C, Rust and Python.

    python3 bench/run.py [--zeno path/to/zeno] [--only fib,loop] [--engines c,rust,...]
                         [--runs 3] [--slow-runs 1] [--json out.json]

Every program is written once per language, doing the same steps. Each
one is built, run, and its output compared with the C program's: a
time only counts when the output is the same. The time is the best of
a few runs, measured from outside (wall clock), so it includes starting
the program. See bench/README.md for what each engine is.
"""
import argparse, json, os, platform, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
PROGRAMS = ["fib", "loop", "sort", "strings", "trees", "nbody"]
# Each program takes its size as its first argument. "big" is for the
# compiled engines (C takes about half a second or more), "small" for
# the engines that interpret (zeno run, Python), which would take many
# minutes at the big sizes.
SIZES = {
    "fib": {"big": 40, "small": 32},
    "loop": {"big": 1000000000, "small": 100000000},
    "sort": {"big": 10000000, "small": 1000000},
    "strings": {"big": 10000000, "small": 1000000},
    "trees": {"big": 20, "small": 16},
    "nbody": {"big": 10000000, "small": 1000000},
}

# The pool and the text room of a build with no collector are made for a
# microcontroller (32 KB). These programs need far more, so the C is
# built again with room for them.
BIG = ["-DZENO_POOL=2147483648", "-DZENO_TEXT_ROOM=16777216"]

def sh(args, cwd, timeout=600):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout)

def engines(zeno):
    """Each engine: (name, how to build in a folder -> command to run, slow?)."""
    def zeno_build(*flags, recompile=None):
        def build(src, out):
            r = sh([zeno, "build", *flags, src], out)
            if r.returncode:
                raise RuntimeError(r.stdout + r.stderr)
            name = os.path.splitext(os.path.basename(src))[0]
            if recompile is not None:
                r = sh(["cc", "-O2", *recompile, "-o", name, name + ".c", "-lm"], out)
                if r.returncode:
                    raise RuntimeError(r.stderr)
            return [os.path.join(out, name)]
        return build
    def c(src, out):
        r = sh(["cc", "-O2", "-o", "p", src, "-lm"], out)
        if r.returncode: raise RuntimeError(r.stderr)
        return [os.path.join(out, "p")]
    def rust(src, out):
        r = sh(["rustc", "-C", "opt-level=3", "-o", "p", src], out)
        if r.returncode: raise RuntimeError(r.stderr)
        return [os.path.join(out, "p")]
    return {
        "c": ("C (cc -O2)", ".c", c, False),
        "rust": ("Rust (rustc -O)", ".rs", rust, False),
        "zeno": ("zeno build", ".zeno", zeno_build(), False),
        "zeno-llvm": ("zeno build --llvm", ".zeno", zeno_build("--llvm"), False),
        "zeno-nc": ("zeno build --no-collector", ".zeno", zeno_build("--no-collector", recompile=BIG), False),
        "zeno-owned-checked": ("zeno build --owned", ".zeno", zeno_build("--owned"), False),
        "zeno-release": ("zeno build --release", ".zeno", zeno_build("--release"), False),
        "zeno-owned": ("zeno build --release --owned", ".zeno", zeno_build("--release", "--owned"), False),
        "zeno-fast": ("zeno build --fast --owned", ".zeno", zeno_build("--fast", "--owned"), False),
        "zeno-fast-gc": ("zeno build --fast", ".zeno", zeno_build("--fast"), False),
        "vm": ("zeno run --vm", ".zeno", lambda src, out: [zeno, "run", "--vm", src], True),
        "run": ("zeno run", ".zeno", lambda src, out: [zeno, "run", src], True),
        "python": ("Python 3", ".py", lambda src, out: [sys.executable, src], True),
    }

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zeno", default=os.path.join(HERE, "..", "target", "release", "zeno"))
    ap.add_argument("--only", default=",".join(PROGRAMS))
    ap.add_argument("--engines", default="c,rust,zeno,zeno-release,zeno-owned,python")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--slow-runs", type=int, default=1)
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument("--size", choices=["big", "small"], default="small")
    ap.add_argument("--json")
    a = ap.parse_args()
    zeno = os.path.abspath(a.zeno)
    table = engines(zeno)
    chosen = a.engines.split(",")
    results = {"size": a.size, "sizes": {p: SIZES[p][a.size] for p in PROGRAMS}, "machine": machine(zeno), "engines": {k: table[k][0] for k in chosen}, "times": {}}
    for prog in a.only.split(","):
        results["times"][prog] = {}
        expected = None
        for key in ["c"] + [k for k in chosen if k != "c"]:
            label, ext, build, slow = table[key]
            src = os.path.join(HERE, prog + ext)
            out = tempfile.mkdtemp(prefix=f"zeno-bench-{prog}-{key}-")
            try:
                cmd = build(src, out) + [str(SIZES[prog][a.size])]
                best, text = None, None
                for _ in range(a.slow_runs if slow else a.runs):
                    t0 = time.perf_counter()
                    r = subprocess.run(cmd, cwd=out, capture_output=True, text=True, timeout=a.timeout)
                    t = time.perf_counter() - t0
                    if r.returncode:
                        raise RuntimeError((r.stdout + r.stderr)[-400:])
                    best, text = (t if best is None else min(best, t)), r.stdout
                if expected is None:
                    expected = text
                elif text != expected:
                    raise RuntimeError("printed something else than C did:\n" + text[-400:])
                value = round(best, 3)
            except subprocess.TimeoutExpired:
                value = f"over {a.timeout} s"
            except RuntimeError as why:
                value = "failed: " + str(why).strip().splitlines()[0][:160]
            finally:
                shutil.rmtree(out, ignore_errors=True)
            if key in chosen:
                results["times"][prog][key] = value
            print(f"{prog:8} {label:36} {value}", flush=True)
    if a.json:
        with open(a.json, "w") as f:
            json.dump(results, f, indent=2)

def machine(zeno):
    def first(args):
        try:
            return subprocess.run(args, capture_output=True, text=True).stdout.strip().splitlines()[0]
        except Exception:
            return "?"
    cpu = "?"
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip(); break
    except OSError:
        pass
    return {"cpu": cpu, "cores": os.cpu_count(), "os": platform.platform(), "zeno": first([zeno, "--version"]),
            "cc": first(["cc", "--version"]), "rustc": first(["rustc", "--version"]), "python": platform.python_version(),
            "date": time.strftime("%Y-%m-%d")}

if __name__ == "__main__":
    main()
