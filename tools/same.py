#!/usr/bin/env python3
"""Show that a change to the compiler left the programs it writes as they were.

    python3 tools/same.py --old path/to/old/zeno            # every program, every way
    python3 tools/same.py --old old/zeno --new new/zeno     # (new: target/release/zeno)
    python3 tools/same.py --old old/zeno --only examples/   # programs whose name has this
    python3 tools/same.py --old old/zeno --ways firmware,llvm
    python3 tools/same.py --old old/zeno --show             # the first lines that differ
    python3 tools/same.py --list                            # the programs and the ways

Two programs that are given the same C, or the same LLVM IR, run the
same instructions and take the same bytes of flash. So a change to the
compiler that leaves every file it writes as it was cannot have made a
built program slower or larger, and that is shown here in a minute with
no clock (decisions 33.4; docs/plan-9-14.md, "Showing that nothing got
slower").

The programs are those of tools/targets.py: every example, every
program in tools/targets/, every sample of the book that `zeno run`
runs, and the benchmarks of bench/. Each is built by both `zeno`
programs each way a build can be asked for (WAYS below), with `CC` and
`CLANG` set to `true`, so that the C and the IR are written and nothing
is compiled. What each wrote is compared byte for byte: every file it
left in the folder, what it printed, and how it ended. So are the
outputs of `zeno join` and `zeno bytecode`.

A program that both refuse to build is the same if both refuse it in
the same words. The output names each program and way that differ, and
the exit code is 1 if any did.
"""

import argparse
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# The ways a build can be asked for. Each is the words given to `zeno`.
# The C builds come first, then the LLVM builds, then the two commands
# that write no build but show what the engines are given.
WAYS = {
    "plain": ["build"],
    "firmware": ["build", "--firmware"],
    "no-collector": ["build", "--no-collector"],
    "owned": ["build", "--owned"],
    "release": ["build", "--release"],
    "fast": ["build", "--fast"],
    "fast-owned": ["build", "--fast", "--owned"],
    "minimal": ["build", "--minimal"],
    "minimal-no-collector": ["build", "--minimal", "--no-collector"],
    "memory": ["build", "--memory"],
    "llvm": ["build", "--llvm"],
    "llvm-firmware": ["build", "--llvm", "--firmware"],
    "llvm-no-collector": ["build", "--llvm", "--no-collector"],
    "join": ["join"],
    "bytecode": ["bytecode"],
}


def load_targets():
    """tools/targets.py, for its list of programs."""
    spec = importlib.util.spec_from_file_location("targets", REPO / "tools/targets.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def every_program(scratch):
    """(name, folder, file) for each program to compare."""
    found = [(p.name, p.folder, p.file) for p in load_targets().programs(scratch)]
    for path in sorted((REPO / "bench").glob("*.zeno")):
        found.append((f"bench/{path.name}", path.parent, path.name))
    return found


def outcome(zeno, words, folder, file, place):
    """Run one `zeno` in a fresh copy of the program's folder. What it
    printed, how it ended, and every file it added, by name."""
    if place.exists():
        shutil.rmtree(place)
    shutil.copytree(folder, place, symlinks=True)
    before = {p.relative_to(place) for p in place.rglob("*")}
    env = dict(os.environ, CC="true", CLANG="true")
    try:
        run = subprocess.run([str(zeno), *words, file], cwd=place, capture_output=True, env=env, timeout=120)
        said = (run.returncode, run.stdout, run.stderr)
    except subprocess.TimeoutExpired:
        said = ("timed out", b"", b"")
    added = {}
    for path in sorted(place.rglob("*")):
        name = path.relative_to(place)
        if name not in before and path.is_file():
            added[str(name)] = path.read_bytes()
    return said, added


def first_difference(old, new):
    """The first line where two outputs differ, for --show."""
    old_lines, new_lines = old.decode(errors="replace").splitlines(), new.decode(errors="replace").splitlines()
    for number, (a, b) in enumerate(zip(old_lines, new_lines), 1):
        if a != b:
            return f"line {number}:\n      old: {a[:200]}\n      new: {b[:200]}"
    return f"one has {len(old_lines)} lines, the other {len(new_lines)}"


def compare(job):
    """Compare one program built one way by both. None if the same, or
    a few lines that say what differs."""
    (name, folder, file), way, old, new, scratch, show = job
    key = f"{name.replace('/', '_').replace('#', '_')}-{way}"
    said_old, files_old = outcome(old, WAYS[way], folder, file, scratch / "old" / key)
    said_new, files_new = outcome(new, WAYS[way], folder, file, scratch / "new" / key)
    shutil.rmtree(scratch / "old" / key, ignore_errors=True)
    shutil.rmtree(scratch / "new" / key, ignore_errors=True)
    problems = []
    # The old `zeno` and the new one call themselves by different
    # versions; a build does not write its version, so nothing else may.
    if said_old[0] != said_new[0]:
        problems.append(f"ended with {said_old[0]}, now {said_new[0]}")
    for which, a, b in (("printed", said_old[1], said_new[1]), ("said on the error stream", said_old[2], said_new[2])):
        if a != b:
            problems.append(f"{which} something else" + (f", at {first_difference(a, b)}" if show else ""))
    for written in sorted(set(files_old) | set(files_new)):
        if written not in files_new:
            problems.append(f"no longer writes {written}")
        elif written not in files_old:
            problems.append(f"now writes {written}")
        elif files_old[written] != files_new[written]:
            problems.append(f"writes another {written}" + (f", at {first_difference(files_old[written], files_new[written])}" if show else ""))
    if not problems:
        return None
    return f"{name} ({way}): " + "; ".join(problems)


def main():
    parser = argparse.ArgumentParser(description="Compare what two `zeno` programs write for every program, every way.")
    parser.add_argument("--old", help="the zeno to compare with, such as the last release")
    parser.add_argument("--new", default=str(REPO / "target/release/zeno"), help="the zeno being tried (target/release/zeno)")
    parser.add_argument("--only", default="", help="only programs whose name has this text")
    parser.add_argument("--ways", default="", help="only these ways, by name, with commas between")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1, help="how many at once (one per processor)")
    parser.add_argument("--show", action="store_true", help="say where each difference begins")
    parser.add_argument("--list", action="store_true", help="name the programs and the ways, and run nothing")
    args = parser.parse_args()

    ways = [w for w in args.ways.split(",") if w] or list(WAYS)
    unknown = [w for w in ways if w not in WAYS]
    if unknown:
        sys.exit(f"same.py: no way is called {', '.join(unknown)}. The ways are: {', '.join(WAYS)}")
    with tempfile.TemporaryDirectory(prefix="zeno-same-") as folder:
        scratch = Path(folder)
        found = [p for p in every_program(scratch) if args.only in p[0]]
        if args.list:
            for name, _, _ in found:
                print(name)
            print(f"\n{len(found)} programs; ways: {', '.join(ways)}")
            return
        if not args.old:
            sys.exit("same.py: say which zeno to compare with: --old path/to/zeno")
        old, new = Path(args.old).resolve(), Path(args.new).resolve()
        for zeno in (old, new):
            if not zeno.is_file():
                sys.exit(f"same.py: there is no zeno at {zeno}")
        jobs = [(program, way, old, new, scratch, args.show) for program in found for way in ways]
        with ThreadPoolExecutor(args.jobs) as pool:
            results = list(pool.map(compare, jobs))
        differ = [r for r in results if r]
        for line in differ:
            print(line)
        print(f"{len(found)} programs, {len(ways)} ways, {len(jobs)} builds by each zeno: "
              f"{len(jobs) - len(differ)} the same, {len(differ)} different")
        sys.exit(1 if differ else 0)


if __name__ == "__main__":
    main()
