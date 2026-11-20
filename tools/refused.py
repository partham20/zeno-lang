#!/usr/bin/env python3
"""Find what `zeno run` takes and a build refuses, before a reviewer does.

    python3 tools/refused.py                  # check: exit 1 on anything new
    python3 tools/refused.py --write          # write tools/refused.txt again
    python3 tools/refused.py --show           # every program, by its refusal
    python3 tools/refused.py --zeno path/to/zeno

Each program is one operation on one type: `print(a)`, `a == b`,
`a in xs`, `a` given to a parameter, returned, kept in a list, a field
or a dict, captured by a function, used as a default, walked by a
`for`, and so on, for each of the types a program can have (OPERATIONS
and TYPES below). Programs that `zeno run` refuses are dropped: they are
not Zeno. Each of the others is built six ways, C and LLVM each with a
collector, with `--firmware` and with `--no-collector`, with `CC` and
`CLANG` set to `true`, so that nothing is compiled and the whole matrix
takes a minute.

Two things are found.

1. A refusal. Each must be one that a table lists: the table of what a
   build with no collector refuses in docs/no-collector.md, or of what
   every build refuses in docs/compiler.md. KNOWN below ties the words
   of each refusal to its row, and a row that is not on its page fails.
   A refusal that matches nothing in KNOWN is new, and fails.
2. A disagreement between C and LLVM: a program that one compiler builds
   and the other refuses, the same way, or that both refuse in different
   words.

tools/refused.txt holds what was found, one line for each refusal, with
the row it belongs to. The test `what_a_build_with_no_collector_refuses_says_so`
reads it, and fails on a row that is not in its tables, so the file, the
pages and the test are held to each other; this tool fails if the file is
not what it finds now (decision 56.19).
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FOUND = REPO / "tools" / "refused.txt"

# The six builds, by their names in tools/same.py. C first, and the
# LLVM build of each at the same place in the second half.
BUILDS = {
    "plain": ["build"],
    "firmware": ["build", "--firmware"],
    "no-collector": ["build", "--no-collector"],
    "llvm": ["build", "--llvm"],
    "llvm-firmware": ["build", "--llvm", "--firmware"],
    "llvm-no-collector": ["build", "--llvm", "--no-collector"],
}
PAIRS = [("plain", "llvm"), ("firmware", "llvm-firmware"), ("no-collector", "llvm-no-collector")]

# What every program may use. A class, an owned class, an enum, and a
# generic enum (decision 58.1).
PRELUDE = """class P {
    x: int
}
owned class Q {
    x: int
}
enum Color {
    Red
    Green
}
enum Opt[T] {
    Empty
    Has(value: T)
}
"""

# Each type: its name in a program, two values of it written out, and
# what it can do. `order` is `<`, `add` is `+`, `key` a key of a dict,
# `items` a list that `sort` and `slice` take, `eq` is `==`.
TYPES = [
    ("int", "1", "2", {"eq", "order", "add", "key"}),
    ("float", "1.5", "2.5", {"eq", "order", "add"}),
    ("str", '"a"', '"b"', {"eq", "order", "add", "key"}),
    ("bool", "true", "false", {"eq", "key"}),
    ("u8", "u8(1)", "u8(2)", {"eq", "order", "add"}),
    ("u64", "u64(1)", "u64(2)", {"eq", "order", "add"}),
    ("int or none", "1", "none", {"eq", "maybe"}),
    ("str or none", '"a"', "none", {"eq", "maybe"}),
    ("float or none", "1.5", "none", {"eq", "maybe"}),
    ("P or none", "P(1)", "none", {"maybe"}),
    ("list[int]", "[1, 2]", "[3]", {"eq", "add", "items", "len"}),
    ("list[str]", '["a", "b"]', '["c"]', {"eq", "add", "items", "len"}),
    ("list[float]", "[1.5]", "[2.5]", {"eq", "add", "items", "len"}),
    ("list[int or none]", "[1, none]", "[none, 2]", {"eq", "len"}),
    ("list[str or none]", '["a", none]', '[none]', {"eq", "len"}),
    ("list[list[int]]", "[[1], [2]]", "[[3]]", {"eq", "len"}),
    ("list[P]", "[P(1)]", "[P(2)]", {"len"}),
    ("list[u8]", "[u8(1), u8(2)]", "[u8(3)]", {"eq", "len"}),
    ("dict[str, int]", '["k": 1]', '["j": 2]', {"eq", "len"}),
    ("dict[str, list[int]]", '["k": [1]]', '["j": [2]]', {"len"}),
    ("dict[int, str]", '[1: "a"]', '[2: "b"]', {"eq", "len"}),
    ("P", "P(1)", "P(2)", set()),
    ("Q", "Q(1)", "Q(2)", set()),
    ("Color", "Color.Red", "Color.Green", {"eq", "key"}),
    ("Opt[int]", "Has(1)", "Empty", {"eq"}),
    ("Opt[str]", 'Has("a")', "Empty", {"eq"}),
    ("Opt[list[int]]", "Has([1])", "Empty", {"eq"}),
    ("Opt[P]", "Has(P(1))", "Empty", set()),
    ("function(int) -> int", "function(n: int) -> int { return n + 1 }", "function(n: int) -> int { return n * 2 }", {"call"}),
]

# Each operation: a name, what the type must be able to do (or None),
# and the program, where T is the type, A and B two values written out,
# and a and b two variables that hold them. `show(x)` prints what any
# value shows: for a function, what it gives for 3.
OPERATIONS = [
    ("print", None, "show(a)\n"),
    ("print-written", None, "show(A)\n"),
    ("let", None, "let c = a\nshow(c)\n"),
    ("var", None, "var v = a\nv = b\nshow(v)\n"),
    ("var-written", None, "var v: T = A\nv = B\nshow(v)\n"),
    ("parameter", None, "function f(x: T) -> int {\n    show(x)\n    return 1\n}\nprint(f(a), f(A))\n"),
    ("take-parameter", None, "function f(x: take T) -> int {\n    show(x)\n    return 1\n}\nprint(f(A))\n"),
    ("return", None, "function g() -> T {\n    return A\n}\nshow(g())\n"),
    ("return-parameter", None, "function g(x: T) -> T {\n    return x\n}\nshow(g(A))\n"),
    ("default", None, "function f(x: T = A) {\n    show(x)\n}\nf()\nf(B)\n"),
    ("list", None, "var xs: list[T] = [A]\nxs.append(B)\nxs[0] = B\nprint(len(xs))\nshow(xs[0])\n"),
    ("list-pop", None, "var xs: list[T] = [A, B]\nshow(xs.pop())\n"),
    ("list-walk", None, "for x in [A, B] {\n    show(x)\n}\n"),
    ("list-print", "eq", "print([A, B])\n"),
    ("list-comprehension", None, "let xs = [x for x in [A, B]]\nprint(len(xs))\n"),
    ("map", None, "let xs = [A, B].map(function(x: T) -> T { return x })\nprint(len(xs))\n"),
    ("filter", None, "let xs = [A, B].filter(function(x: T) -> bool { return true })\nprint(len(xs))\n"),
    ("field", None, "class Holder {\n    v: T\n}\nvar h = Holder(A)\nh.v = B\nshow(h.v)\n"),
    ("field-default", None, "class Holder {\n    v: T = A\n}\nlet h = Holder()\nshow(h.v)\n"),
    ("dict-value", None, 'var d: dict[str, T] = ["k": A]\nd["j"] = B\nshow(d["k"])\nprint(len(d))\n'),
    ("dict-key", "key", "let d: dict[T, int] = [A: 1]\nprint(d[A], A in d, B in d)\n"),
    ("maybe", None, "var o: T or none = none\no = A\nif o is not none {\n    show(o)\n}\n"),
    ("narrow", "maybe", "if a is not none {\n    print(a)\n}\nprint(b is none)\n"),
    ("closure", None, "let f = function() -> int {\n    show(a)\n    return 0\n}\nprint(f())\n"),
    ("kept-closure", None, "function make() -> function() -> int {\n    let c = A\n    return function() -> int {\n        show(c)\n        return 0\n    }\n}\nprint(make()())\n"),
    ("equal", "eq", "print(a == b, a != b, a == A)\n"),
    ("in", "eq", "let xs: list[T] = [a]\nprint(b in xs, a in xs, A in xs)\n"),
    ("in-written", "eq", "print(A in [A, B], B not in [A])\n"),
    ("order", "order", "print(a < b, a >= b)\n"),
    ("add", "add", "show(a + b)\n"),
    ("len", "len", "print(len(a), len(A))\n"),
    ("sort", "items", "var xs = B + A\nxs.sort()\nprint(xs)\n"),
    ("slice", "items", "print(a.slice(1), A.slice(0, 1))\n"),
    ("text", "eq", 'print(f"{a}", str(b))\n'),
    ("call", "call", "print(a(3), A(4))\n"),
    ("pass-function", None, "function apply(f: function(T) -> int, x: T) -> int {\n    return f(x)\n}\nprint(apply(function(x: T) -> int { return 1 }, A))\n"),
    # Several values from one function (decision 57): first, later,
    # dropped, and a swap.
    ("several-first", None, "function two(x: take T) -> (T, int) {\n    return x, 1\n}\nlet c, n = two(A)\nshow(c)\nprint(n)\n"),
    ("several-later", None, "function two(x: take T) -> (int, T) {\n    return 1, x\n}\nlet n, c = two(A)\nshow(c)\nlet m, _ = two(B)\nprint(n + m)\n"),
    ("swap", None, "var v: T = A\nvar w: T = B\nv, w = w, v\nshow(v)\nshow(w)\n"),
]


def show_for(ty):
    """`show(x)`, which prints a value of the type."""
    if ty.startswith("function"):
        return f"function show(x: {ty}) {{\n    print(x(3))\n}}\n"
    if ty in ("P", "Q"):
        return f"function show(x: {ty}) {{\n    print(x.x)\n}}\n"
    if ty == "list[P]":
        return f"function show(x: {ty}) {{\n    print(len(x))\n}}\n"
    if ty == "P or none":
        return f"function show(x: {ty}) {{\n    print(x is none)\n}}\n"
    return f"function show(x: {ty}) {{\n    print(x)\n}}\n"


def programs():
    """Every program of the matrix, by its name."""
    for ty, a, b, can in TYPES:
        for name, needs, body in OPERATIONS:
            if needs is not None and needs not in can:
                continue
            if name == "maybe" and "maybe" in can:
                continue
            text = body.replace("T", ty).replace("A", a).replace("B", b)
            source = PRELUDE + show_for(ty) + f"let a: {ty} = {a}\nlet b: {ty} = {b}\n" + text
            yield f"{name} of {ty}", source


def run(zeno, words, source):
    """What `zeno` says: (it took it, the first line of its error)."""
    with tempfile.TemporaryDirectory() as place:
        path = Path(place) / "p.zeno"
        path.write_text(source)
        env = dict(os.environ, CC="true", CLANG="true")
        try:
            done = subprocess.run([str(zeno), *words, "p.zeno"], cwd=place, capture_output=True, env=env, timeout=60)
        except subprocess.TimeoutExpired:
            return False, "timed out"
        if words == ["run"]:
            return done.returncode == 0, done.stderr.decode(errors="replace").strip()
        errors = done.stderr.decode(errors="replace")
        first = next((line for line in errors.splitlines() if line.startswith("error: ")), "")
        # A build with CC=true writes the file and "builds" it: what it
        # writes is all that is wanted. Only an `error:` is a refusal.
        return not first, first.removeprefix("error: ")


def same_words(message):
    """The words of a refusal as both compilers should say them."""
    return message.replace("`zeno build --llvm`", "`zeno build`")


# Each refusal that a table lists: the words, as a pattern, and the
# beginning of its row on its page. (page, row, pattern)
NO_COLLECTOR = "docs/no-collector.md"
EVERY_BUILD = "docs/compiler.md"
KNOWN = [
    (NO_COLLECTOR, "A list, object or enum value used after it was given away", r"was given away on line \d+ and holds nothing now"),
    (NO_COLLECTOR, "A function that uses a `var` of the function around it", r"is a `var` of the function around this one"),
    (NO_COLLECTOR, "Keeping an item of a list of lists or of objects, which `[ ]` only looks at", r"which has one owner: the list\. `\[ \]` only looks at it"),
    (NO_COLLECTOR, "A function that is kept and uses a `var`", r"this function is kept, so it takes `\w+`, but `\w+` is a `var`"),
    (NO_COLLECTOR, "Replacing a list or an object that a parameter only looks at", r"^only the owner can replace what `\w+` holds"),
    (NO_COLLECTOR, "Taking an item out of a list while a `for` walks it", r"is walking a list that `\w+` owns"),
    (NO_COLLECTOR, "Returning or keeping what a parameter only looks at", r"is only looked at here: its owner still has it"),
    (NO_COLLECTOR, "`filter` on a list of lists or of objects", r"^`filter` would put the items it keeps in a second list"),
    (NO_COLLECTOR, "Keeping an item of a list of lists or of objects that a `for`", r"is a look at an item of the list that the loop is walking"),
    (NO_COLLECTOR, "A function with a name, written inside a function", r"is a function inside a function that uses variables of the function around it"),
    (EVERY_BUILD, "A class or an enum inside a function", r"cannot compile (a class|an enum) inside a function or block"),
    (EVERY_BUILD, "A generic function inside a function", r"cannot compile a generic function inside a function"),
    (EVERY_BUILD, "Two functions inside a function that call each other", r"cannot compile two functions inside a function that call each other"),
    (EVERY_BUILD, "A default that is more than a literal", r"cannot compile a default that is not a plain literal, for a function inside a function"),
    (EVERY_BUILD, "Named arguments to a function held in a variable", r"cannot compile named arguments to a function held in a variable"),
    (EVERY_BUILD, "A built-in function used as a value", r"cannot compile `[a-z_.]+` used this way"),
    (EVERY_BUILD, "A method that comes with Zeno used as a value", r"cannot compile a method that comes with Zeno used as a value"),
    (EVERY_BUILD, "A method of an object that no variable holds", r"cannot compile a method used as a value of (an object that no variable holds|a copy of a generic class)"),
    (EVERY_BUILD, "A list whose items are not all one type", r"cannot compile a list whose items are not all one type"),
    (EVERY_BUILD, "A variable that only ever holds `none`", r"needs to know what a list holds|needs a type for a variable that only ever holds `none`"),
    (EVERY_BUILD, "A u64 that may be `none`", r"cannot compile a u64 that may be `none`"),
    (EVERY_BUILD, "A dict of objects of an `owned class`", r"keeps a dict where anything may share it"),
    (EVERY_BUILD, "`program.run`", r"cannot compile `program\.run`"),
]


def rows_of(page, after):
    """The rows of the table that follows `after` on `page`."""
    text = (REPO / page).read_text()
    section = text.split(after, 1)[1]
    table = next(part for part in section.split("\n\n") if part.startswith("|"))
    return table.splitlines()[2:]


def row_for(message):
    for page, row, pattern in KNOWN:
        if re.search(pattern, same_words(message)):
            return page, row
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--zeno", default=str(REPO / "target" / "release" / "zeno"))
    parser.add_argument("--write", action="store_true", help="write tools/refused.txt")
    parser.add_argument("--show", action="store_true", help="every program, by its refusal")
    args = parser.parse_args()
    zeno = Path(args.zeno).resolve()

    every = list(programs())
    with ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
        ran = list(pool.map(lambda item: run(zeno, ["run"], item[1]), every))
        taken = [item for item, (ok, _) in zip(every, ran) if ok]
        jobs = [(name, source, build) for name, source in taken for build in BUILDS]
        built = list(pool.map(lambda job: run(zeno, BUILDS[job[2]], job[1]), jobs))
    result = {}
    for (name, _, build), outcome in zip(jobs, built):
        result.setdefault(name, {})[build] = outcome

    pages = {
        NO_COLLECTOR: rows_of(NO_COLLECTOR, "## What is refused\n"),
        EVERY_BUILD: rows_of(EVERY_BUILD, "## What it compiles so far\n"),
    }
    problems = []
    for page, row, _ in KNOWN:
        if not any(line.startswith("| " + row) for line in pages[page]):
            problems.append(f"no row on {page} begins `{row}`")
    by_message = {}
    for name, builds in result.items():
        for build, (ok, message) in builds.items():
            if not ok:
                by_message.setdefault(same_words(message), []).append((name, build))
        for c, llvm in PAIRS:
            (c_ok, c_said), (llvm_ok, llvm_said) = builds[c], builds[llvm]
            if c_ok != llvm_ok:
                said = llvm_said if c_ok else c_said
                problems.append(f"{name}: {'C' if c_ok else 'LLVM'} builds it `{c}` and the other refuses: {said}")
            elif not c_ok and same_words(c_said) != same_words(llvm_said):
                problems.append(f"{name}: `{c}` C and LLVM refuse in different words:\n    {c_said}\n    {llvm_said}")

    lines = []
    for message in sorted(by_message):
        row = row_for(message)
        if row is None:
            problems.append(f"a refusal that no table lists: {message}\n    " + "\n    ".join(f"{n} ({b})" for n, b in by_message[message][:6]))
            continue
        lines.append(f"{row[0]} | {row[1]} | {message}")
    found = "".join(line + "\n" for line in sorted(set(lines)))
    if args.show:
        for message, where in sorted(by_message.items()):
            print(message)
            for name, build in where:
                print(f"    {name} ({build})")
    print(f"{len(every)} programs, {len(taken)} taken by `zeno run`, {len(jobs)} builds, "
          f"{sum(1 for ok, _ in built if not ok)} refused, {len(by_message)} different refusals")
    if args.write:
        FOUND.write_text(found)
    elif FOUND.exists() and FOUND.read_text() != found:
        problems.append("tools/refused.txt is not what was found: run `python3 tools/refused.py --write`")
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
