#!/usr/bin/env python3
"""Hold the plain C of a build to what GCC and Clang are asked for by name.

    python3 tools/plain.py                  # every kind of build, every program
    python3 tools/plain.py owned collector  # only these kinds
    python3 tools/plain.py --only ch12      # only programs whose name has ch12
    python3 tools/plain.py --jobs 1         # one program at a time (one per processor otherwise)

The C that `zeno build` writes asks GCC and Clang for a few things by
their own names for them: whether a sum overflowed, two things the
collector does, and a word in front of a function that never comes
back. Each is written a second time in plain C for every other
compiler, and `ZENO_PLAIN_C` says which is used (the top of
src/runtime.c, decision 55).

No such other compiler is needed to try the plain C. The programs are
the ones tools/targets.py tries: every example, the programs in
tools/targets/, and every sample of the book that runs. The C of each,
for each kind of build (the collector, `--owned`, `--no-collector`),
is compiled as it is, and then:

  - with `-DZENO_PLAIN_C`, which asks the same compiler for the plain C,
    read as the C of 1999 and nothing else (`-std=c99
    -pedantic-errors`), with `-O2`;
  - with tcc, the Tiny C Compiler, if it is here. It is a C compiler
    that is neither GCC nor Clang and has none of their names, so it
    takes the plain C without being asked.

Each must print what the first printed, end as it ended, and if an
error stopped it, say the same message. A program that a check stops
is kept in: the checks are what is being tried.

`--firmware` by itself is not among the kinds: it is a firmware build
with a collector, which plain C cannot build and says so (decision
55.5). `--no-collector` is a firmware build too, the one a board gets.

`zeno build` is asked for the C alone (`CC`: the "compiler" it calls
only writes down what it was asked), and the C compiler is called from
here.
"""

import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.dont_write_bytecode = True  # no `__pycache__` beside the tools
import minimal  # noqa: E402  (how the C of a program is written and compiled)
import targets  # noqa: E402  (the programs, and how one is run)

CC = minimal.CC
KINDS = {kind: words for kind, words in minimal.KINDS.items() if kind != "firmware"}
LONGEST = minimal.LONGEST
TCC = shutil.which("tcc")

# What asks the compiler for the plain C, and for C as the standard of
# 1999 has it. (`_DEFAULT_SOURCE` is for the C library of Linux, which
# otherwise leaves out of its headers what is not in that standard:
# `sigaction` and `mmap`, which the collector uses on a computer.)
PLAIN = ["-DZENO_PLAIN_C", "-O2", "-w"]
STRICT = ["-std=c99", "-pedantic-errors", "-D_DEFAULT_SOURCE"]

# Programs with a mistake in them that no build checks, and programs the
# system may stop: two builds of one need not do the same.
NOT_DEFINED = dict(minimal.UNDEFINED, **minimal.THE_SYSTEM_STOPS)


def made(compiler, flags, c_file, binary):
    """What a C compiler said, if it could not make the program."""
    done = subprocess.run([compiler] + flags + ["-o", str(binary), str(c_file)] + minimal.libraries(c_file) + ["-lm"],
                          capture_output=True, text=True)
    return done.stderr[:3000] if done.returncode != 0 else ""


def try_program(kind, program, scratch):
    """("pass" | "left out" | "no C" | the reason it failed, how many ways it was compiled)."""
    folder = scratch / kind / program.name.replace("/", "_").replace("#", "_")
    wrote, _ = minimal.written(kind, program, folder, False)
    if wrote is None:
        # `--owned` and its like refuse a program that shares its lists;
        # `zeno build` refuses what only `zeno run` can do.
        return "left out", 0
    c_file, binary = wrote
    if made(CC, ["-O0", "-w"], c_file, binary):
        return "no C", 0
    status, out, errors = targets.run([str(binary)] + program.args, program.folder, program.stdin, timeout=LONGEST)
    # A library of C the program brings (`import c`) is its own C, which
    # may be of a later standard: only the plain C is asked for there.
    strict = [] if minimal.libraries(c_file) else STRICT
    ways = [("plain C", CC, PLAIN + strict)]
    if TCC:
        ways.append(("tcc", TCC, ["-w"]))
    for name, compiler, flags in ways:
        other = binary.with_name(binary.name + "-" + name.replace(" ", "-"))
        wrong = made(compiler, flags, c_file, other)
        if wrong:
            return f"{name}: the C does not compile:\n{wrong}", len(ways)
        if program.name in NOT_DEFINED:
            continue
        status_other, out_other, errors_other = targets.run([str(other)] + program.args, program.folder, program.stdin,
                                                            timeout=LONGEST)
        same_end = (status == 0) == (status_other == 0)
        if same_end and out == out_other and minimal.message(errors) == minimal.message(errors_other):
            continue
        why = f"as built it {'succeeded' if status == 0 else 'failed'}, with {name} it {'succeeded' if status_other == 0 else 'failed'}"
        if same_end and out == out_other:
            why = f"as built it stopped with {minimal.message(errors)!r}, with {name} with {minimal.message(errors_other)!r}"
        return f"{why}\n--- as built it printed\n{out[-2000:]}--- with {name} it printed\n{out_other[-2000:]}--- and said\n{errors_other[-2000:]}", len(ways)
    return "pass", len(ways)


def main():
    words = sys.argv[1:]
    only = words[words.index("--only") + 1] if "--only" in words else None
    jobs = int(words[words.index("--jobs") + 1]) if "--jobs" in words else (os.cpu_count() or 1)
    kinds = [kind for kind in KINDS if kind in words] or list(KINDS)
    if not minimal.ZENO.exists():
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=targets.REPO, check=True)
    scratch = Path(tempfile.mkdtemp(prefix="zeno-plain-"))
    found = [p for p in targets.programs(scratch) if only is None or only in p.name]
    pool = ThreadPoolExecutor(max_workers=jobs)
    with_what = f"`{CC} -DZENO_PLAIN_C`" + (" and with tcc" if TCC else " (no tcc here, so not with it)")

    failed_any = False
    for kind in kinds:
        passed, left_out, no_c, failures = 0, 0, [], []
        results = pool.map(lambda program: try_program(kind, program, scratch), found)
        for program, (result, _) in zip(found, results):
            if result == "pass":
                passed += 1
            elif result == "left out":
                left_out += 1
            elif result == "no C":
                no_c.append(program.name)
            else:
                failures.append((program, result))
        total = passed + len(failures)
        print(f"{kind}: {passed} of {total} programs do in plain C what they do as built, with {with_what}"
              f" ({left_out} left out: this kind of build refuses them)")
        if no_c:
            print(f"  not tried, because the C compiler refuses the C as built: {', '.join(no_c)}")
        for program, why in failures:
            failed_any = True
            print(f"\n  FAILED {program.name} ({program.folder / program.file})\n    " + why.replace("\n", "\n    "))
    shutil.rmtree(scratch, ignore_errors=True)
    return 1 if failed_any else 0


if __name__ == "__main__":
    sys.exit(main())
