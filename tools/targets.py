#!/usr/bin/env python3
"""Hold every target language to `zeno run`.

    python3 tools/targets.py                 # every target, every program
    python3 tools/targets.py js py           # only these targets
    python3 tools/targets.py js --only ch12  # only programs whose name has ch12
    python3 tools/targets.py --jobs 1        # one program at a time (one per processor otherwise)
    python3 tools/targets.py --list          # the programs, and nothing run

The programs are every example in examples/, the programs in
tools/targets/ (written to try the corners: the edges of an int, escapes,
errors; a `.stdin` file beside one is what is typed at it), and every
sample of the book that `zeno run` runs (book/chapters/*.tex). Each is run with
`zeno run`, which says what it must print. Then for each target it is
built with `zeno build --target <target>`, the result is run with that
language's own tools, and what it printed is held to what `zeno run`
printed: the same text, and the same answer to "did it stop with an
error". A program that a target refuses to build because it uses what no
target can do (`import board`, `unsafe`) is counted as left out, not as
a failure.

A target whose tools are not installed is left out with a note. The
weekly check on GitHub (.github/workflows/targets.yml) installs them all.
"""

import os
import re
import shutil
import subprocess
from concurrent.futures import ThreadPoolExecutor
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ZENO = Path(os.environ.get("ZENO", REPO / "target/release/zeno"))

# For each target: what has to be installed, and how to run what was
# built. {name} is the program's name, {file} the file `zeno build` wrote.
TARGETS = {
    "js": {"needs": ["node"], "run": ["node", "{file}"]},
    "py": {"needs": ["python3"], "run": ["python3", "{file}"]},
    "go": {"needs": ["go"], "run": ["go", "run", "{file}"]},
    "java": {"needs": ["java"], "run": ["java", "{file}"]},
    "cs": {"needs": ["dotnet"], "run": ["dotnet", "run", "{file}"]},
    # Compiled beside the file it wrote, then run.
    "cpp": {"needs": ["g++"], "run": ["sh", "-c", "g++ -std=c++17 -O2 -o \"$0.bin\" \"$0\" && exec \"$0.bin\" \"$@\"", "{file}"]},
    # A page has no keyboard, no command line and no files to write, and it
    # cannot stand still: a program that is typed at, given words, writes a
    # file or sleeps is left out. tools/run-page.js needs Playwright.
    "html": {"needs": ["node"], "run": ["node", str(REPO / "tools/run-page.js"), "{file}"], "page": True},
}

PAGE_CANNOT = re.compile(r"\bsleep\(|\bfiles\.(write|append)\(")
SAMPLE_RE = re.compile(r"\\begin\{zeno\}(?:\[(.*?)\])?\n(.*?)\\end\{zeno\}", re.S)


def options(text):
    result = {}
    if not text:
        return result
    parts, depth, current = [], 0, ""
    for ch in text:
        depth += ch == "{"
        depth -= ch == "}"
        if ch == "," and depth == 0:
            parts.append(current)
            current = ""
        else:
            current += ch
    parts.append(current)
    for part in parts:
        key, _, value = part.strip().partition("=")
        value = value.strip()
        if value.startswith("{") and value.endswith("}"):
            value = value[1:-1]
        if key.strip():
            result[key.strip()] = value
    return result


class Program:
    """One program to try: the folder it is in, the file to run, what is
    typed at it, and the words after it."""

    def __init__(self, name, folder, file, stdin="", args=()):
        self.name, self.folder, self.file, self.stdin, self.args = name, folder, file, stdin, list(args)

    def source(self):
        """The text of every file of the program."""
        if self.file == "main.zeno" or (self.folder / "main.zeno").exists():
            return "\n".join(path.read_text() for path in sorted(self.folder.rglob("*.zeno")))
        return (self.folder / self.file).read_text()


def programs(scratch):
    found = []
    for path in sorted((REPO / "examples").iterdir()):
        if path.suffix == ".zeno":
            found.append(Program(f"examples/{path.name}", path.parent, path.name))
        elif path.is_dir() and (path / "main.zeno").exists():
            found.append(Program(f"examples/{path.name}", path, "main.zeno"))
    # Programs written to try the corners: big numbers, escapes, errors.
    for path in sorted((REPO / "tools/targets").glob("*.zeno")):
        typed = path.with_suffix(".stdin")
        stdin = typed.read_text() if typed.exists() else ""
        found.append(Program(f"tools/targets/{path.name}", path.parent, path.name, stdin))
    for chapter in sorted((REPO / "book/chapters").glob("*.tex")):
        shared = {}
        for number, match in enumerate(SAMPLE_RE.finditer(chapter.read_text())):
            opts, body = options(match.group(1)), match.group(2)
            if "src" in opts:
                body = (REPO / opts["src"]).read_text()
            folder_name = opts.get("dir") or f"{chapter.stem}-{number}"
            folder = scratch / "book" / folder_name
            file = opts.get("file", "main.zeno")
            path = folder / file
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(body.rstrip("\n") + "\n")
            shared[folder_name] = True
            if opts.get("run") == "no" or "cmd" in opts or not file.endswith(".zeno"):
                continue
            stdin = opts.get("stdin", "").replace("\\n", "\n")
            args = opts.get("args", "").split()
            found.append(Program(f"book/{chapter.stem}#{number}", folder, file, stdin, args))
    return found


def run(command, cwd, stdin="", timeout=60):
    try:
        done = subprocess.run(command, cwd=cwd, input=stdin, capture_output=True, text=True, timeout=timeout,
                              env=dict(os.environ, NO_COLOR="1"))
        return done.returncode, done.stdout, done.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "timed out"
    except FileNotFoundError as error:
        return -1, "", str(error)


def main():
    wanted = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
    only = None
    if "--only" in sys.argv:
        only = sys.argv[sys.argv.index("--only") + 1]
        wanted = [w for w in wanted if w != only]
    if "--jobs" in sys.argv:
        wanted = [w for w in wanted if w != sys.argv[sys.argv.index("--jobs") + 1]]
    targets = [name for name in TARGETS if not wanted or name in wanted]
    if not ZENO.exists():
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=REPO, check=True)
    scratch = Path(tempfile.mkdtemp(prefix="zeno-targets-"))
    found = [p for p in programs(scratch) if only is None or only in p.name]
    if "--list" in sys.argv:
        for program in found:
            print(program.name, program.folder / program.file)
        return 0

    # Programs are tried a few at a time: one per processor, unless
    # `--jobs N` says otherwise. Results are printed in the same order.
    jobs = int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else (os.cpu_count() or 1)
    pool = ThreadPoolExecutor(max_workers=jobs)

    # What `zeno run` prints is what every target must print.
    def run_zeno(program):
        status, out, _ = run([str(ZENO), "run", program.file] + program.args, program.folder, program.stdin)
        return status == 0, out

    expected = dict(zip((p.name for p in found), pool.map(run_zeno, found)))

    def try_program(target, spec, program):
        """"pass", "left out", or the reason it failed."""
        if spec.get("page") and (program.stdin or program.args or PAGE_CANNOT.search(program.source())):
            return "left out"
        out_dir = scratch / "out" / target / program.name.replace("/", "_").replace("#", "_")
        out_dir.mkdir(parents=True, exist_ok=True)
        status, built, errors = run([str(ZENO), "build", "--target", target, str(program.folder / program.file)], out_dir)
        if status != 0:
            if "a program built for another language cannot use" in errors or "manual memory" in errors:
                return "left out"
            ok, _ = expected[program.name]
            if not ok and "error" in errors:
                # A program that `zeno run` refuses is refused here too.
                return "pass"
            return "the build failed:\n" + errors.strip()
        file = re.search(r"Wrote (\S+?)\.? ", built + " ")
        file = file.group(1) if file else ""
        command = [part.replace("{file}", str(out_dir / file)).replace("{dir}", str(out_dir))
                   for part in spec["run"]]
        status, out, errors = run(command + program.args, program.folder, program.stdin)
        ok, want = expected[program.name]
        if (status == 0) == ok and out == want:
            return "pass"
        why = f"zeno run {'succeeded' if ok else 'failed'}, the {target} program {'succeeded' if status == 0 else 'failed'}"
        return f"{why}\n--- zeno run printed\n{want}--- {target} printed\n{out}--- {target} said\n{errors}"

    failed_any = False
    for target in targets:
        spec = TARGETS[target]
        missing = [tool for tool in spec["needs"] if shutil.which(tool) is None]
        if missing:
            print(f"{target}: left out, because {', '.join(missing)} is not installed")
            continue
        passed, left_out, failures = 0, [], []
        results = pool.map(lambda program: try_program(target, spec, program), found)
        for program, result in zip(found, results):
            if result == "pass":
                passed += 1
            elif result == "left out":
                left_out.append(program.name)
            else:
                failures.append((program, result))
        total = passed + len(failures)
        print(f"{target}: {passed} of {total} programs print what `zeno run` prints"
              + (f" ({len(left_out)} left out: they use the board, a page, manual memory"
                 + (", the keyboard, the command line, files or sleep" if spec.get("page") else "") + ")" if left_out else ""))
        for program, why in failures:
            failed_any = True
            print(f"\n  FAILED {program.name} ({program.folder / program.file})\n    " + why.replace("\n", "\n    "))
    shutil.rmtree(scratch, ignore_errors=True)
    return 1 if failed_any else 0


if __name__ == "__main__":
    sys.exit(main())
