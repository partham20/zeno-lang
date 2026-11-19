#!/usr/bin/env python3
"""Hold `zeno build --minimal` to `zeno build`.

    python3 tools/minimal.py                  # every kind of build, every program
    python3 tools/minimal.py owned firmware   # only these kinds
    python3 tools/minimal.py --only ch12      # only programs whose name has ch12
    python3 tools/minimal.py --jobs 1         # one program at a time (one per processor otherwise)
    python3 tools/minimal.py --sizes          # how big the two builds of each example are

A minimal build has no checks, so a correct program must do in it exactly
what it does in the build that checks. The programs are the ones
tools/targets.py tries: every example, the programs in tools/targets/,
and every sample of the book that runs. Each is built twice for each
kind of build (the collector, `--owned`, `--no-collector`, `--firmware`),
once as it is and once with `--minimal`, and the two must print the same
text, give the same answer to "did it stop with an error", and if it
did, give the same message.

A program that one of the checks stops, or would stop if nothing caught
the error, has no defined result in a minimal build, as it has none in
C. It is left out, and counted. To find those, the C of the checked
build is changed in one place before it is compiled: the two functions
every check ends in, `zeno_fail` and `zeno_bug`, first write a mark.
(One check ends in neither: memory given back twice. It is known by
what it says.) The samples of the book that show a mistake no build
checks, such as reading memory after it was given back, are named in
`UNDEFINED` below and left out too.

The C of each minimal build is also read by the C compiler with its
warnings on, and a function or a variable that is left in it unused is a
failure: only what the program uses belongs there.

`--sizes` compares nothing. It builds each example both ways and prints
a table. For the two builds of each: the lines of C; the bytes of code
and constants in the program (`text`, as `size` shows it) and the size
of its file with the names taken out (`strip`), both compiled with
`-O2`, and for the minimal one with `-Os` as well, which is how `zeno
build --minimal` compiles it; the memory the program holds from start
to end (`.data` + `.bss`); the most bytes it had asked for at one time
(what `--memory` reports); and the most memory the system gave it.

`zeno build` is asked for the C alone (`CC=true`: the C compiler it
calls does nothing), and the C compiler is called from here: the checked
build needs no more than `-O0` to say what the program does, which
takes a tenth of the time, and the minimal build gets the `-Os` that
`zeno build --minimal` gives it.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.dont_write_bytecode = True  # no `__pycache__` beside the tools
import targets  # noqa: E402  (the programs, and how one is run)

ZENO = targets.ZENO
CC = os.environ.get("CC", "cc")
ASKED = "compiler-was-asked"  # what `zeno build` asked the compiler, one word a line

# Each kind of build, and the words that ask `zeno build` for it.
KINDS = {
    "collector": ["--collector"],
    "owned": ["--owned"],
    "no-collector": ["--no-collector"],
    "firmware": ["--firmware"],
}

# A program that has not ended after this many seconds never will: one
# example blinks a light for ever. Both builds of it are stopped, and
# the two count as having done the same.
LONGEST = 20

MARK = "[stopped at a check]"
# What the check of `free` says, which stops the program by itself.
GIVEN_BACK_TWICE = "a block of memory was given back twice"

# Programs with a mistake in them that no build checks: what they do is
# not defined in any build, and two builds need not do the same.
UNDEFINED = {
    "book/33-unsafe#4": "reads memory after it was given back",
    # `--owned` and `--no-collector` look for this, and then it is "at a
    # check". With the collector the memory is the C library's, and what
    # its `free` does the second time differs from one system to the next.
    "book/33-unsafe#5": "gives memory back twice",
}
# Programs the system may stop, in a build that checks too: nothing in
# Zeno's C looks for these, and a signal is how the program ends. Any
# other program that ends with a signal has found a mistake in the C.
THE_SYSTEM_STOPS = {
    "tools/targets/edge-deep.zeno": "calls itself until the stack is full",
    "tools/targets/edge-deep-3.zeno": "calls itself until the stack is full",
    "book/a6-errors#5": "gives back memory at an address nothing handed out",
}
# Put in front of the checked build's C: the mark, written with nothing
# of the C library's that the program could have changed.
MARKER = (
    "#include <unistd.h>\n"
    "static void zeno_marks_a_check(void) {\n"
    f"    static const char mark[] = \"\\n{MARK}\\n\";\n"
    "    if (write(2, mark, sizeof mark - 1) < 0) return;\n"
    "}\n"
)
ENDS_IN = re.compile(r"^(ZENO_NORETURN static void zeno_(?:fail|bug)\([^)]*\) \{)$", re.M)


def written(kind, program, out_dir, minimal):
    """The C file `zeno build` wrote and the program to make of it, or why it wrote none."""
    out_dir.mkdir(parents=True, exist_ok=True)
    # The "compiler" `zeno build` is given only writes down what it was
    # asked to compile with, so that a library of C the program declares
    # (`import c`) is built in here as `zeno build` builds it in.
    recorder = out_dir / "record-cc"
    recorder.write_text(f"#!/bin/sh\nprintf '%s\\n' \"$@\" > '{out_dir / ASKED}'\n")
    recorder.chmod(0o755)
    command = [str(ZENO), "build"] + KINDS[kind] + (["--minimal"] if minimal else [])
    done = subprocess.run(command + [str(program.folder / program.file)], cwd=out_dir, capture_output=True, text=True,
                          env=dict(os.environ, CC=str(recorder), NO_COLOR="1"))
    if done.returncode != 0:
        return None, done.stderr.strip()
    name = re.search(r"Built (\S+?)\.? ", done.stdout + " ")
    c_file = re.search(r"Wrote (\S+?\.c)", done.stdout)
    if not name or not c_file:
        return None, "`zeno build` did not say what it wrote:\n" + done.stdout
    return (out_dir / c_file.group(1), out_dir / name.group(1)), ""


def libraries(c_file):
    """What `zeno build` asked the compiler for beyond the program's own C:
    the libraries of C the program declares, and where their headers are."""
    asked = c_file.parent / ASKED
    if not asked.exists():
        return []
    words = asked.read_text().splitlines()
    kept, skip = [], False
    for word in words:
        if skip:
            skip = False
        elif word == "-o":
            skip = True
        elif word != c_file.name and not word.startswith(("-O", "-D", "-fno-math")):
            kept.append(word)
    return kept


def compiled(c_file, binary, flags):
    """What the C compiler said, if it could not make the program."""
    done = subprocess.run([CC] + flags + ["-o", str(binary), str(c_file)] + libraries(c_file) + ["-lm"],
                          capture_output=True, text=True)
    return done.stderr[:3000] if done.returncode != 0 else ""


def message(errors):
    """The line that says what stopped a program."""
    return next((line for line in errors.splitlines() if line.startswith("error")), "")


def checked_run(kind, program, folder):
    """What the build that checks does with a program:
    ("left out" | "at a check" | "no C" | "ran" | why it failed, its status, what it printed, what it said, its C).

    "at a check" is a program that a check stops, or would stop if
    nothing caught the error, or that has a mistake no build checks:
    without the checks it has no defined result."""
    wrote, why = written(kind, program, folder / "checked", False)
    if wrote is None:
        # `--owned` and its like refuse a program that shares its lists;
        # `zeno build` refuses what only `zeno run` can do.
        return "left out", 0, "", "", ""
    c_file, binary = wrote
    text = c_file.read_text()
    marked, count = ENDS_IN.subn(lambda found: found.group(1) + "\n    zeno_marks_a_check();", text)
    if count == 0:
        return "the checked build has no `zeno_fail` to mark", 0, "", "", text
    c_file.write_text(MARKER + marked)
    if compiled(c_file, binary, ["-O0", "-w"]):
        # Not a matter of `--minimal`: the checked build's own C is wrong.
        return "no C", 0, "", "", text
    status, out, errors = targets.run([str(binary)] + program.args, program.folder, program.stdin, timeout=LONGEST)
    if MARK in errors or GIVEN_BACK_TWICE in errors or program.name in UNDEFINED:
        return "at a check", status, out, errors, text
    if status < -1 and program.name not in THE_SYSTEM_STOPS:
        # Not an error of Zeno's: the system stopped it, for reading
        # memory that is not its own, say. Two builds that both do that
        # are not two builds that agree.
        return f"the system stopped the checked build (signal {-status}), after it printed\n{out[-1000:]}", status, out, errors, text
    return "ran", status, out, errors, text


def try_program(kind, program, scratch):
    """("pass" | "left out" | "at a check" | "no C" | the reason it failed, the sizes of the two C files)."""
    folder = scratch / kind / program.name.replace("/", "_").replace("#", "_")
    result, status, out, errors, text = checked_run(kind, program, folder)
    if result != "ran":
        return result, None

    minimal, why = written(kind, program, folder / "minimal", True)
    if minimal is None:
        return "the minimal build failed:\n" + why, None
    small_c, small = minimal
    sizes = (len(text), small_c.stat().st_size)
    wrong = compiled(small_c, small, ["-Os"])
    if wrong:
        return "the minimal C does not compile:\n" + wrong, sizes
    headers = [word for word in libraries(small_c) if word.startswith("-I")]
    warned = subprocess.run([CC, "-fsyntax-only", "-Wall", str(small_c)] + headers, capture_output=True, text=True)
    # A variable of the program's own that it never reads is the
    # program's business. What is looked for is a function or a variable
    # at the top of the file that nothing uses, or one that is used and
    # not there.
    unused = [line for line in warned.stderr.splitlines() if "defined but not used" in line or "implicit" in line]
    if unused:
        return "the minimal C has something in it that is not used, or not declared:\n" + "\n".join(unused[:10]), sizes
    status_small, out_small, errors_small = targets.run([str(small)] + program.args, program.folder, program.stdin, timeout=LONGEST)
    same_end = (status == 0) == (status_small == 0)
    if same_end and out == out_small and (status == 0 or message(errors) == message(errors_small)):
        return "pass", sizes
    why = f"the checked build {'succeeded' if status == 0 else 'failed'}, the minimal build {'succeeded' if status_small == 0 else 'failed'}"
    if same_end and out == out_small:
        why = f"the checked build stopped with {message(errors)!r}, the minimal build with {message(errors_small)!r}"
    return f"{why}\n--- the checked build printed\n{out}--- the minimal build printed\n{out_small}--- and said\n{errors_small}", sizes


def sections(binary):
    """(text, data + bss) of a program, as `size` counts them."""
    done = subprocess.run(["size", str(binary)], capture_output=True, text=True)
    numbers = done.stdout.splitlines()[1].split()
    return int(numbers[0]), int(numbers[1]) + int(numbers[2])


# A program that runs another and says, as its last line, the most
# memory the other had, in KB. Python cannot ask this itself: what the
# system counts for a program includes the one that started it, up to
# the moment it started, and Python is far bigger than these programs.
WATCHER = r"""
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>
static pid_t child;
static void too_long(int signal) {
    (void)signal;
    kill(child, SIGKILL);
    _exit(3);
}
int main(int argc, char **argv) {
    (void)argc;
    child = fork();
    if (child == 0) {
        execv(argv[2], argv + 2);
        _exit(127);
    }
    signal(SIGALRM, too_long);
    alarm((unsigned)atoi(argv[1]));
    int status;
    struct rusage used;
    wait4(child, &status, 0, &used);
    fprintf(stderr, "\n%ld\n", used.ru_maxrss);
    return 0;
}
"""


def ran(watcher, binary, program):
    """What a program said as it ran, and the most memory it had, in KB. None if it never ends."""
    status, out, errors = targets.run([str(watcher), str(LONGEST // 4), str(binary)] + program.args, program.folder,
                                      program.stdin, timeout=LONGEST)
    if status != 0:
        return None
    return out + errors, int(errors.splitlines()[-1])


def size_table(found, scratch, kinds):
    """A table of how big the two builds of each program are."""
    watcher = scratch / "watcher"
    (scratch / "watcher.c").write_text(WATCHER)
    subprocess.run([CC, "-O2", "-o", str(watcher), str(scratch / "watcher.c")], check=True)
    print("| program | build | lines of C | code, `-O2` | code, `-Os` | file, `-O2` | file, `-Os` | `.data` + `.bss` | most in use | most memory |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for program in found:
        for kind in kinds:
            folder = scratch / "sizes" / kind / program.name.replace("/", "_")
            row = []
            for minimal in (False, True):
                wrote, _ = written(kind, program, folder / ("minimal" if minimal else "checked"), minimal)
                if wrote is None:
                    break
                c_file, binary = wrote
                measured = {"lines": len(c_file.read_text().splitlines())}
                for flag in ["-O2"] + (["-Os"] if minimal else []):
                    built = binary.with_name(binary.name + flag)
                    if compiled(c_file, built, [flag, "-w"]):
                        break
                    if flag == "-O2":
                        measured["ran"] = ran(watcher, built, program)
                    subprocess.run(["strip", str(built)], check=True)
                    measured[flag] = (built.stat().st_size,) + sections(built)
                # The same build again, saying how much memory it asked for.
                KINDS[kind].append("--memory")
                wrote, _ = written(kind, program, folder / ("minimal-counted" if minimal else "checked-counted"), minimal)
                KINDS[kind].pop()
                if wrote is not None and not compiled(wrote[0], wrote[1], ["-O2", "-w", "-DZENO_MEMORY_REPORT"]):
                    said = ran(watcher, wrote[1], program)
                    if said is not None:
                        measured["in use"] = sum(int(n) for n in re.findall(r"at most (\d+) bytes in use", said[0]))
                row.append(measured)
            if len(row) < 2 or "-O2" not in row[0] or "-Os" not in row[1]:
                continue
            checked, small = row
            kb = lambda measured: "runs for ever" if measured["ran"] is None else f"{measured['ran'][1]:,} KB"
            in_use = lambda measured: f"{measured['in use']:,}" if "in use" in measured else "-"
            name = program.name.replace("examples/", "").replace(".zeno", "")
            print(f"| {name} | {kind} | {checked['lines']:,} → {small['lines']:,} "
                  f"| {checked['-O2'][1]:,} → {small['-O2'][1]:,} | {small['-Os'][1]:,} "
                  f"| {checked['-O2'][0]:,} → {small['-O2'][0]:,} | {small['-Os'][0]:,} "
                  f"| {checked['-O2'][2]:,} → {small['-O2'][2]:,} "
                  f"| {in_use(checked)} → {in_use(small)} "
                  f"| {kb(checked)} → {kb(small)} |")


def main():
    words = sys.argv[1:]
    only = words[words.index("--only") + 1] if "--only" in words else None
    jobs = int(words[words.index("--jobs") + 1]) if "--jobs" in words else (os.cpu_count() or 1)
    kinds = [kind for kind in KINDS if kind in words] or list(KINDS)
    if not ZENO.exists():
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=targets.REPO, check=True)
    scratch = Path(tempfile.mkdtemp(prefix="zeno-minimal-"))
    found = [p for p in targets.programs(scratch) if only is None or only in p.name]
    pool = ThreadPoolExecutor(max_workers=jobs)
    if "--sizes" in words:
        size_table([p for p in found if p.name.startswith("examples/") or only], scratch, kinds)
        shutil.rmtree(scratch, ignore_errors=True)
        return 0

    failed_any = False
    for kind in kinds:
        passed, left_out, at_a_check, no_c, failures, checked_size, minimal_size = 0, 0, 0, [], [], 0, 0
        results = pool.map(lambda program: try_program(kind, program, scratch), found)
        for program, (result, sizes) in zip(found, results):
            if result == "pass":
                passed += 1
                checked_size += sizes[0]
                minimal_size += sizes[1]
            elif result == "left out":
                left_out += 1
            elif result == "at a check":
                at_a_check += 1
            elif result == "no C":
                no_c.append(program.name)
            else:
                failures.append((program, result))
        total = passed + len(failures)
        share = f", in {100 * minimal_size // max(checked_size, 1)}% of the C" if passed else ""
        print(f"{kind}: {passed} of {total} programs do in a minimal build what they do in a checked one{share}"
              f" ({at_a_check} left out: a check stops them, or they have a mistake no build checks;"
              f" {left_out}: this kind of build refuses them)")
        if no_c:
            print(f"  not tried, because the C compiler refuses the C of the checked build: {', '.join(no_c)}")
        for program, why in failures:
            failed_any = True
            print(f"\n  FAILED {program.name} ({program.folder / program.file})\n    " + why.replace("\n", "\n    "))
    shutil.rmtree(scratch, ignore_errors=True)
    return 1 if failed_any else 0


if __name__ == "__main__":
    sys.exit(main())
