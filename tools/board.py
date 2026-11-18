#!/usr/bin/env python3
"""Hold a program built for a board to the same program built for a computer.

    python3 tools/board.py                  # every program, checked and minimal
    python3 tools/board.py checked          # only the build that checks (or: minimal)
    python3 tools/board.py --only ch12      # only programs whose name has ch12
    python3 tools/board.py --jobs 1         # one program at a time (one per processor otherwise)
    python3 tools/board.py --sizes          # how much flash and RAM each example takes

`zeno build --board boards/qemu-cortex-m3` makes a program for an ARM
Cortex-M3: a 32-bit processor with no floating point of its own, 256 KB
of flash, 64 KB of RAM and no system under the program. QEMU emulates
the board, so no hardware is needed. The C that `zeno build` writes must
do there what it does on a computer.

The programs are the ones tools/targets.py tries: every example, the
programs in tools/targets/, and every sample of the book that runs.
Each is built twice, with `zeno build --no-collector` for the computer
and with `zeno build --board` for the board (a board build has no
collector), and the two must print the same text, give the same answer
to "did it stop with an error", and if it did, give the same message.
Then the same again with `--minimal`, for the programs no check stops
(tools/minimal.py says which those are, and why they are left out).

Some programs are left out, and counted:

  - one that a build with no collector refuses, or that only `zeno run`
    can run;
  - one that is given words on its line, or reads or writes a file, or
    reads what is typed to its end: a board has no line, no files, and
    a serial port has no end, so the program waits there for ever;
  - one that does not fit in the board's memory. These are named.

A number with a fraction may differ after its fifteenth digit: `cosh`
and its like are the C library's, the board has another C library than
the computer, and the two need not round the last digit the same way.

This needs `arm-none-eabi-gcc` with its C library (newlib) and
`qemu-system-arm`. On Debian and Ubuntu:

    sudo apt-get install gcc-arm-none-eabi libnewlib-arm-none-eabi qemu-system-arm
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
import minimal  # noqa: E402  (which programs a check stops)
from minimal import THE_SYSTEM_STOPS, UNDEFINED  # noqa: E402  (programs with a mistake no build checks)

ZENO = targets.ZENO
BOARD = targets.REPO / "boards" / "qemu-cortex-m3"
NEEDS = ["arm-none-eabi-gcc", "qemu-system-arm"]

# The two builds that are compared on the board and on the computer.
KINDS = {"checked": [], "minimal": ["--minimal"]}

# A program that has not ended after this many seconds never will: one
# example blinks a light for ever. Both builds of it are stopped, and
# the two count as having done the same. The emulated chip is slower
# than the computer, so it gets longer.
LONGEST = 20
LONGEST_ON_BOARD = 90

# What QEMU itself says as this board starts.
QEMU_SAYS = "Timer with period zero, disabling\n"

# A board has no files, and what is typed to it has no end.
NEEDS_A_SYSTEM = re.compile(r"\bimport files\b|\bfrom files import\b|\bfiles\.|\bread_all\(|\bprogram\.environment\(")

# A program that reads what is typed, from the keyboard or a serial port.
READS = re.compile(r"\binput\(|\.read_byte\(|\.has_byte\(")

# A number with a fraction, as `print` writes one.
FRACTION = re.compile(r"-?\d+\.\d+(?:e[-+]?\d+)?")

# The computer's C compiler, asked for no optimising: what the program
# does is the same, and it is compiled in a tenth of the time.
QUICK_CC = """#!/bin/sh
for word; do
    shift
    case "$word" in
        -O*) ;;
        *) set -- "$@" "$word" ;;
    esac
done
exec ${REAL_CC:-cc} -O0 -w "$@"
"""


def built(program, out_dir, words, quick_cc=None):
    """The program `zeno build` made, or (None, what it said)."""
    out_dir.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, NO_COLOR="1")
    if quick_cc:
        env["REAL_CC"] = os.environ.get("CC", "cc")
        env["CC"] = str(quick_cc)
    done = subprocess.run([str(ZENO), "build"] + words + [str(program.folder / program.file)], cwd=out_dir,
                          capture_output=True, text=True, env=env)
    if done.returncode != 0:
        return None, (done.stdout + done.stderr).strip()
    name = re.search(r"Built (\S+?)\.? ", done.stdout + " ")
    if not name:
        return None, "`zeno build` did not say what it built:\n" + done.stdout
    return out_dir / name.group(1), ""


def on_board(binary, program):
    """What a program built for the board does in QEMU: (status, printed, said)."""
    command = ["qemu-system-arm", "-M", "lm3s6965evb", "-nographic", "-semihosting", "-kernel", str(binary)]
    status, out, errors = targets.run(command, program.folder, program.stdin, timeout=LONGEST_ON_BOARD)
    return status, out, errors.replace(QEMU_SAYS, "")


def same_text(one, other):
    """Is this the same text, but for the last digits of its fractions?"""
    if one == other:
        return True
    if FRACTION.split(one) != FRACTION.split(other):
        return False
    close = lambda a, b: abs(float(a) - float(b)) <= 1e-14 * max(abs(float(a)), abs(float(b)))
    return all(close(a, b) for a, b in zip(FRACTION.findall(one), FRACTION.findall(other)))


def message(errors):
    """The line that says what stopped a program."""
    return next((line for line in errors.splitlines() if line.startswith("error")), "")


def try_program(kind, program, scratch):
    """"pass" | "left out" | "needs a system" | "too big" | "stopped" | the reason it failed."""
    if program.name in UNDEFINED:
        return "left out"
    if program.args or NEEDS_A_SYSTEM.search(program.source()):
        return "needs a system"
    folder = scratch / kind / program.name.replace("/", "_").replace("#", "_")
    if kind == "minimal":
        # What a check stops, or would stop if nothing caught the
        # error, has no defined result without the check. The build
        # that checks says which programs those are, as in tools/minimal.py.
        first = minimal.checked_run("no-collector", program, folder / "first")[0]
        if first == "left out":
            return "left out"
        if first != "ran" or program.name in THE_SYSTEM_STOPS:
            return "stopped"
    here, why = built(program, folder / "computer", ["--no-collector"] + KINDS[kind], scratch / "quick-cc")
    if here is None:
        # A build with no collector refuses a program that shares its
        # lists; `zeno build` refuses what only `zeno run` can do.
        return "left out"
    status, out, errors = targets.run([str(here)], program.folder, program.stdin, timeout=LONGEST)
    if status < -1:
        return f"the system stopped the build for the computer (signal {-status}), after it printed\n{out[-1000:]}"
    there, why = built(program, folder / "board", ["--board", str(BOARD)] + KINDS[kind])
    if there is None:
        if re.search(r"region `?(RAM|FLASH)'? overflowed|will not fit in region", why):
            return "too big"
        return "the build for the board failed:\n" + why[-3000:]
    status_there, out_there, errors_there = on_board(there, program)
    if status_there == -1 and status != -1 and READS.search(program.source()):
        # It read to the end of what was typed, and a serial port has none.
        return "needs a system"
    same_end = (status == 0) == (status_there == 0) and (status == -1) == (status_there == -1)
    if same_end and same_text(out, out_there) and (status in (0, -1) or message(errors) == message(errors_there)):
        return "pass"
    ended = lambda status: "never ended" if status == -1 else "succeeded" if status == 0 else "failed"
    why = f"on the computer it {ended(status)}, on the board it {ended(status_there)}"
    if same_end and same_text(out, out_there):
        why = f"on the computer it stopped with {message(errors)!r}, on the board with {message(errors_there)!r}"
    return f"{why}\n--- on the computer it printed\n{out[-2000:]}--- on the board it printed\n{out_there[-2000:]}--- and said\n{errors_there[-2000:]}"


def size_table(found, scratch):
    """A table of the flash and the RAM each program takes on the board."""
    print("| program | flash, checked | flash, `--minimal` | RAM, checked | RAM, `--minimal` |")
    print("|---|---|---|---|---|")
    for program in found:
        row = []
        for kind in KINDS:
            folder = scratch / "sizes" / kind / program.name.replace("/", "_")
            there, _ = built(program, folder, ["--board", str(BOARD)] + KINDS[kind])
            if there is None:
                break
            done = subprocess.run(["arm-none-eabi-size", str(there)], capture_output=True, text=True)
            text, data, bss = (int(number) for number in done.stdout.splitlines()[1].split()[:3])
            # The first values of the variables are kept in flash too.
            row.append((text + data, data + bss))
        if len(row) == 2:
            name = program.name.replace("examples/", "").replace(".zeno", "")
            print(f"| {name} | {row[0][0]:,} | {row[1][0]:,} | {row[0][1]:,} | {row[1][1]:,} |")


def main():
    words = sys.argv[1:]
    only = words[words.index("--only") + 1] if "--only" in words else None
    jobs = int(words[words.index("--jobs") + 1]) if "--jobs" in words else (os.cpu_count() or 1)
    kinds = [kind for kind in KINDS if kind in words] or list(KINDS)
    missing = [tool for tool in NEEDS if shutil.which(tool) is None]
    if missing:
        print(f"tools/board.py needs {' and '.join(missing)}, and there is no such command here. On Debian and Ubuntu:\n"
              "    sudo apt-get install gcc-arm-none-eabi libnewlib-arm-none-eabi qemu-system-arm")
        return 1
    if not ZENO.exists():
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=targets.REPO, check=True)
    scratch = Path(tempfile.mkdtemp(prefix="zeno-board-"))
    quick_cc = scratch / "quick-cc"
    quick_cc.write_text(QUICK_CC)
    quick_cc.chmod(0o755)
    found = [p for p in targets.programs(scratch) if only is None or only in p.name]
    if "--sizes" in words:
        size_table([p for p in found if p.name.startswith("examples/") or only], scratch)
        shutil.rmtree(scratch, ignore_errors=True)
        return 0

    pool = ThreadPoolExecutor(max_workers=jobs)
    failed_any = False
    for kind in kinds:
        counts = {"pass": 0, "left out": 0, "needs a system": 0, "stopped": 0}
        too_big, failures = [], []
        results = pool.map(lambda program: try_program(kind, program, scratch), found)
        for program, result in zip(found, results):
            if result in counts:
                counts[result] += 1
            elif result == "too big":
                too_big.append(program.name)
            else:
                failures.append((program, result))
        total = counts["pass"] + len(failures)
        stopped = f"; {counts['stopped']}: a check stops them" if kind == "minimal" else ""
        print(f"{kind}: {counts['pass']} of {total} programs do on the board what they do on a computer"
              f" ({counts['left out']} left out: a build with no collector refuses them;"
              f" {counts['needs a system']}: they need files, words on their line, or an end to what is typed{stopped};"
              f" {len(too_big)}: too big for the board)")
        if too_big:
            print(f"  too big for 256 KB of flash and 64 KB of RAM: {', '.join(too_big)}")
        for program, why in failures:
            failed_any = True
            print(f"\n  FAILED {program.name} ({program.folder / program.file})\n    " + why.replace("\n", "\n    "))
    shutil.rmtree(scratch, ignore_errors=True)
    return 1 if failed_any else 0


if __name__ == "__main__":
    sys.exit(main())
