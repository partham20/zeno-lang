#!/usr/bin/env python3
"""Time the programs in this folder in Zeno, C, Rust and Python.

    python3 bench/run.py [--zeno path/to/zeno] [--only fib,loop] [--engines c,rust,...]
                         [--runs 3] [--slow-runs 1] [--json out.json] [--count]

Every program is written once per language, doing the same steps. Each
one is built, run, and its output compared with the C program's: a
time only counts when the output is the same. The time is the best of
a few runs, measured from outside (wall clock), so it includes starting
the program. The runs go in rounds, every engine once per round, so a
machine that slows down for a while slows every engine and not one.
See bench/README.md for what each engine is.

`--count` does not time anything. It runs each program once under
valgrind and writes how many instructions the processor was given. That
number is the same on every run, so it shows a difference of one in a
hundred, which a clock on a shared machine cannot. It is not a time: an
instruction may cost a third of a cycle or twenty. Read the two together.
"""
import argparse, json, os, platform, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
PROGRAMS = ["fib", "loop", "sort", "strings", "trees", "nbody"]
# Each program takes its size as its first argument. "big" is for the
# compiled engines (C takes about half a second or more), "small" for
# the engines that interpret (zeno run, Python), which would take many
# minutes at the big sizes.
SIZES = {
    "fib": {"big": 40, "small": 32, "count": 30},
    "loop": {"big": 1000000000, "small": 100000000, "count": 20000000},
    "sort": {"big": 10000000, "small": 1000000, "count": 300000},
    "strings": {"big": 10000000, "small": 1000000, "count": 300000},
    "trees": {"big": 20, "small": 16, "count": 14},
    "nbody": {"big": 10000000, "small": 1000000, "count": 200000},
}
# `--count`: valgrind runs a program some fifty times slower, so the
# sizes are smaller, and it says how many instructions it ran.
VALGRIND = ["valgrind", "--tool=cachegrind", "--cache-sim=no", "--cachegrind-out-file=/dev/null"]

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
    def target(language, run, *steps):
        # The Zeno program written in another language, built there if
        # that language is built, and run. {name} is the program's name.
        def build(src, out):
            r = sh([zeno, "build", "--target", language, src], out)
            if r.returncode:
                raise RuntimeError(r.stdout + r.stderr)
            name = os.path.splitext(os.path.basename(src))[0]
            for step in steps:
                r = sh([part.format(name=name) for part in step], out)
                if r.returncode:
                    raise RuntimeError(r.stdout + r.stderr)
            return [part.format(name=name, out=out) for part in run]
        return build
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
        "to-js": ("--target js, in Node", ".zeno", target("js", ["node", "{name}.js"]), False),
        "to-py": ("--target py, in Python 3", ".zeno", target("py", [sys.executable, "{name}.py"]), True),
        "to-php": ("--target php, in PHP", ".zeno", target("php", ["php", "{name}.php"]), True),
        "to-go": ("--target go, built by go", ".zeno", target("go", ["{out}/p"], ["go", "build", "-o", "p", "{name}.go"]), False),
        "to-java": ("--target java, built by javac", ".zeno", target("java", ["java", "-cp", "classes", "ZenoProgram"], ["javac", "-nowarn", "-d", "classes", "{name}.java"]), False),
        "to-cs": ("--target cs, built by dotnet", ".zeno", target("cs", ["dotnet", "run", "--no-build", "{name}.cs"], ["dotnet", "build", "{name}.cs"]), False),
        "to-cpp": ("--target cpp, built by g++ -O2", ".zeno", target("cpp", ["{out}/p"], ["g++", "-std=c++17", "-O2", "-o", "p", "{name}.cpp"]), False),
        "to-rust": ("--target rust, built by rustc -O", ".zeno", target("rust", ["{out}/p"], ["rustc", "-O", "--edition", "2021", "-o", "p", "{name}.rs"]), False),
        "to-wasm": ("--target wasm, in Node", ".zeno", target("wasm", ["node", "--no-warnings", os.path.join(HERE, "..", "tools", "run-wasi.js"), "{name}.wasm"]), False),
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
    ap.add_argument("--count", action="store_true", help="count instructions with valgrind instead of timing")
    ap.add_argument("--json")
    a = ap.parse_args()
    zeno = os.path.abspath(a.zeno)
    table = engines(zeno)
    chosen = a.engines.split(",")
    size = "count" if a.count else a.size
    what = "counts" if a.count else "times"
    results = {"size": size, "sizes": {p: SIZES[p][size] for p in PROGRAMS}, "machine": machine(zeno), "engines": {k: table[k][0] for k in chosen}, what: {}}
    for prog in a.only.split(","):
        results[what][prog] = {}
        keys = ["c"] + [k for k in chosen if k != "c"]
        # Build every engine first. `built[key]` is the command to run,
        # `value[key]` is set early for an engine that failed.
        built, folders, value = {}, [], {}
        for key in keys:
            label, ext, build, slow = table[key]
            out = tempfile.mkdtemp(prefix=f"zeno-bench-{prog}-{key}-")
            folders.append(out)
            try:
                built[key] = (build(os.path.join(HERE, prog + ext), out) + [str(SIZES[prog][size])], out)
            except RuntimeError as why:
                value[key] = "failed: " + str(why).strip().splitlines()[0][:160]
        # Then run them in rounds: every engine once, and again.
        best, text = {}, {}
        rounds = 1 if a.count else max(a.runs, a.slow_runs)
        for round_ in range(rounds):
            for key in keys:
                slow = table[key][3]
                if key in value or round_ >= (1 if a.count else a.slow_runs if slow else a.runs):
                    continue
                cmd, out = built[key]
                try:
                    t0 = time.perf_counter()
                    r = subprocess.run((VALGRIND if a.count else []) + cmd, cwd=out, capture_output=True, text=True, timeout=a.timeout)
                    t = time.perf_counter() - t0
                    if r.returncode:
                        raise RuntimeError((r.stdout + r.stderr)[-400:])
                    if a.count:
                        refs = [line for line in r.stderr.splitlines() if "I refs:" in line or "I   refs:" in line]
                        if not refs:
                            raise RuntimeError("valgrind gave no count:\n" + r.stderr[-400:])
                        t = int(refs[-1].split(":")[-1].replace(",", ""))
                    best[key], text[key] = (t if key not in best else min(best[key], t)), r.stdout
                except subprocess.TimeoutExpired:
                    value[key] = f"over {a.timeout} s"
                except RuntimeError as why:
                    value[key] = "failed: " + str(why).strip().splitlines()[0][:160]
        for out in folders:
            shutil.rmtree(out, ignore_errors=True)
        for key in keys:
            if key not in value:
                if text[key] != text.get("c", text[key]):
                    value[key] = "failed: printed something else than C did: " + text[key][-160:].strip()
                else:
                    value[key] = best[key] if a.count else round(best[key], 3)
            if key in chosen:
                results[what][prog][key] = value[key]
            print(f"{prog:8} {table[key][0]:36} {value[key]}", flush=True)
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
