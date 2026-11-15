#!/usr/bin/env python3
"""Build The Zeno Book: run every sample, then make the PDF and the HTML.

    python3 book/tools/build.py            # samples, PDF and HTML
    python3 book/tools/build.py --samples  # only run the samples
    python3 book/tools/build.py --pdf      # samples and PDF
    python3 book/tools/build.py --html     # samples and HTML

The book is written once, in LaTeX, in book/zeno-book.tex and
book/chapters/. A code sample in a chapter is written as

    \\begin{zeno}[file=ring.zeno, dir=ring]
    ...
    \\end{zeno}

This script runs each sample with the real `zeno` and puts what it
printed under it, so every output in the book is what Zeno printed. A
sample that fails when it should not, or works when it should fail,
stops the build. The finished LaTeX goes to book/build/gen/, and from
there `xelatex` makes book/dist/zeno-book.pdf and pandoc makes
book/dist/html/.

Keys a sample can have:

    file=NAME     the file the code is written to (default main.zeno).
                  An explicit name is shown above the code
    dir=NAME      a folder shared by the samples of one chapter that
                  name it. Without one, each sample has its own
    src=PATH      take the code from this file of the repository,
                  and leave the body empty
    run=no        only write the file, and show it
    cmd=TEXT      run this in place of `zeno run FILE`. {file} is the
                  file. Several commands are separated by ` ;; `, and
                  each is shown with a `$` in front of what it printed
    args=TEXT     words after the file, for `zeno run`
    stdin=TEXT    what is typed. \\n is a line end
    fails=yes     the sample must fail (exit with a status other than 0)
    output=no     run it, but do not show what it printed
    lines=A-B     show only these lines of the code (all are run)
    show=no       run the code, and show only what it printed

`zenofrag` is a piece of a program: it is not run, but it must parse,
unless it says parse=no. `session` holds commands, one per line after
`$ ` (or `$! ` for one that must fail), and shows each with what it
printed. With at=PATH they run in that folder of the repository.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

BOOK = Path(__file__).resolve().parent.parent
REPO = BOOK.parent
BUILD = BOOK / "build"
GEN = BUILD / "gen"
SAMPLES = BUILD / "samples"
DIST = BOOK / "dist"

ENV_RE = re.compile(
    r"\\begin\{(zeno|zenofrag|session)\}(?:\[(.*?)\])?\n(.*?)\\end\{\1\}",
    re.S,
)

failures = []
counts = {"run": 0, "parsed": 0, "commands": 0}


def options(text):
    """`a=1, b=two words` -> dict. A value may be in braces to hold commas."""
    result = {}
    if not text:
        return result
    parts, depth, current = [], 0, ""
    for ch in text:
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(current)
            current = ""
        else:
            current += ch
    parts.append(current)
    for part in parts:
        part = part.strip()
        if not part:
            continue
        key, _, value = part.partition("=")
        value = value.strip()
        if value.startswith("{") and value.endswith("}"):
            value = value[1:-1]
        result[key.strip()] = value
    return result


def zeno_binary():
    for candidate in [os.environ.get("ZENO"), REPO / "target/release/zeno"]:
        if candidate and Path(candidate).exists():
            return Path(candidate)
    print("building zeno with cargo ...")
    subprocess.run(["cargo", "build", "--release"], cwd=REPO, check=True)
    return REPO / "target/release/zeno"


def run(command, cwd, stdin=""):
    """Run one shell command with `zeno` on the PATH. Gives (status, output)."""
    env = dict(os.environ)
    env["PATH"] = f"{BIN}:{env['PATH']}"
    env["NO_COLOR"] = "1"
    process = subprocess.run(
        command,
        shell=True,
        cwd=cwd,
        input=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=env,
        timeout=120,
    )
    output = process.stdout.replace("\t", "    ")
    return process.returncode, output.rstrip("\n")


def listing(code, language, caption=None):
    code = code.rstrip("\n")
    style = "zenostyle" if language == "Zeno" else "outputstyle"
    body = f"\\begin{{lstlisting}}[language={language},style={style}]\n{code}\n\\end{{lstlisting}}"
    if caption:
        return f"\\begin{{zenofile}}{caption}\\end{{zenofile}}\n{body}"
    return body


def output_listing(text):
    if not text.strip():
        return "\\begin{nooutput}This prints nothing.\\end{nooutput}"
    return listing(text, "Output")


def where(chapter, body):
    first = body.strip().splitlines()[0] if body.strip() else ""
    return f"{chapter}: sample starting `{first[:60]}`"


def do_zeno(chapter, opts, body, folder):
    if "src" in opts:
        body = (REPO / opts["src"]).read_text()
    file = opts.get("file", "main.zeno")
    path = folder / file
    path.parent.mkdir(parents=True, exist_ok=True)
    code = body.rstrip("\n") + "\n"
    path.write_text(code)
    shown = code
    if "lines" in opts:
        a, b = (int(x) for x in opts["lines"].split("-"))
        shown = "\n".join(code.splitlines()[a - 1 : b]) + "\n"
    language = "Zeno" if file.endswith(".zeno") or file.endswith(".package") else "Output"
    caption = file if ("file" in opts or "src" in opts) else None
    if "src" in opts and "file" not in opts:
        caption = opts["src"]
    # show=no runs the code without printing it, for a sample that only
    # wants to show what a command did to the file before it.
    parts = [] if opts.get("show") == "no" else [listing(shown, language, caption)]
    if opts.get("run") == "no":
        return "\n".join(parts)

    stdin = opts.get("stdin", "").replace("\\n", "\n")
    if "cmd" in opts:
        commands = [c.strip().replace("{file}", file) for c in opts["cmd"].split(";;")]
        shown_lines = []
        status = 0
        for command in commands:
            status, out = run(command, folder, stdin)
            counts["commands"] += 1
            shown_lines.append(f"$ {command}")
            if out:
                shown_lines.append(out)
        output = "\n".join(shown_lines)
    else:
        args = opts.get("args", "")
        status, output = run(f"zeno run {file} {args}".strip(), folder, stdin)
        counts["run"] += 1
    expect_fail = opts.get("fails") == "yes"
    if expect_fail and status == 0:
        failures.append(f"{where(chapter, body)} should have failed, and printed:\n{output}")
    if not expect_fail and status != 0:
        failures.append(f"{where(chapter, body)} failed:\n{output}")
    if opts.get("output") != "no":
        parts.append(output_listing(output))
    return "\n".join(parts)


def do_frag(chapter, opts, body, folder):
    code = body.rstrip("\n") + "\n"
    if opts.get("lang") == "shell":
        return listing(code, "Output")
    if opts.get("parse") != "no":
        path = folder / "fragment.zeno"
        path.write_text(code)
        status, output = run("zeno parse fragment.zeno", folder)
        counts["parsed"] += 1
        if status != 0:
            failures.append(f"{where(chapter, body)} does not parse:\n{output}")
    return listing(code, "Zeno")


def do_session(chapter, opts, body, folder):
    # at=examples runs the commands in that folder of the repository.
    if "at" in opts:
        folder = REPO / opts["at"]
    lines = []
    for raw in body.strip("\n").splitlines():
        if raw.startswith("$! ") or raw.startswith("$ "):
            must_fail = raw.startswith("$!")
            command = raw[3:] if must_fail else raw[2:]
            status, out = run(command, folder, opts.get("stdin", "").replace("\\n", "\n"))
            counts["commands"] += 1
            if must_fail and status == 0:
                failures.append(f"{chapter}: `{command}` should have failed:\n{out}")
            if not must_fail and status != 0:
                failures.append(f"{chapter}: `{command}` failed:\n{out}")
            lines.append(f"$ {command}")
            if out:
                lines.append(out)
        elif raw.strip():
            failures.append(f"{chapter}: a session line must start with `$ `: {raw}")
    return listing("\n".join(lines), "Output")


def process_chapter(source, target):
    chapter = source.stem
    text = source.read_text()
    root = SAMPLES / chapter
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    number = [0]

    def replace(match):
        kind, raw_opts, body = match.group(1), match.group(2), match.group(3)
        opts = options(raw_opts)
        number[0] += 1
        folder = root / (opts.get("dir") or f"s{number[0]:03d}")
        folder.mkdir(parents=True, exist_ok=True)
        if kind == "zeno":
            return do_zeno(chapter, opts, body, folder)
        if kind == "zenofrag":
            return do_frag(chapter, opts, body, folder)
        return do_session(chapter, opts, body, folder)

    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(ENV_RE.sub(replace, text))


def samples(only=None):
    GEN.mkdir(parents=True, exist_ok=True)
    shutil.copy(BOOK / "zeno-book.tex", GEN / "zeno-book.tex")
    for extra in ["zeno-book.sty"]:
        shutil.copy(BOOK / extra, GEN / extra)
    for source in sorted((BOOK / "chapters").glob("*.tex")):
        if only and not source.stem.startswith(only):
            continue
        process_chapter(source, GEN / "chapters" / source.name)
    print(
        f"samples: {counts['run']} programs run, {counts['commands']} commands, "
        f"{counts['parsed']} pieces parsed"
    )
    if failures:
        print(f"\n{len(failures)} samples went wrong:\n")
        for failure in failures:
            print(failure, "\n")
        sys.exit(1)


def pdf():
    subprocess.run(
        ["latexmk", "-xelatex", "-interaction=nonstopmode", "-halt-on-error", "zeno-book.tex"],
        cwd=GEN,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    DIST.mkdir(exist_ok=True)
    shutil.copy(GEN / "zeno-book.pdf", DIST / "zeno-book.pdf")
    print(f"pdf: {DIST / 'zeno-book.pdf'}")


def html():
    out = DIST / "html"
    if out.exists():
        shutil.rmtree(out)
    tools = BOOK / "tools"
    # pandoc would read zeno-book.sty if the main file asked for it, and
    # the look of the printed book is no business of the HTML.
    main = (GEN / "zeno-book.tex").read_text().replace("\\usepackage{zeno-book}\n", "")
    # The preface stands before the first part, and pandoc would cut its
    # sections into pages of their own. As a part of its own it is one page.
    preface = (GEN / "chapters/00-preface.tex").read_text().replace("\\chapter*{", "\\part*{", 1)
    preface = re.sub(r"\\subsection\*\{(.*?)\}", r"\\begin{prefacehead}\1\\end{prefacehead}", preface)
    (GEN / "chapters/00-preface-html.tex").write_text(preface)
    main = main.replace("chapters/00-preface}", "chapters/00-preface-html}")
    (GEN / "zeno-book-html.tex").write_text(main)
    subprocess.run(
        [
            "pandoc",
            "zeno-book-html.tex",
            "--from=latex",
            "--to=chunkedhtml",
            "--split-level=2",
            "--number-sections",
            "--toc",
            "--toc-depth=2",
            f"--syntax-definition={tools / 'zeno.xml'}",
            f"--template={tools / 'template.html'}",
            f"--lua-filter={tools / 'chapters.lua'}",
            "--highlight-style=pygments",
            "--css=book.css",
            "--metadata=title:The Zeno Book",
            "--metadata=lang:en",
            "-o",
            str(out),
        ],
        cwd=GEN,
        check=True,
    )
    shutil.copy(tools / "book.css", out / "book.css")
    tidy_names(out)
    sidebar(out)
    if (DIST / "zeno-book.pdf").exists():
        shutil.copy(DIST / "zeno-book.pdf", out / "zeno-book.pdf")
    print(f"html: {out}/index.html")


def tidy_names(out):
    """pandoc names a page `-ch-tour.html`. Make it `tour.html`, everywhere."""
    renames = {}
    for page in out.glob("-*.html"):
        name = page.name[1:]
        for prefix in ("ch-", "app-"):
            if name.startswith(prefix):
                name = name[len(prefix):]
        renames[page.name] = name
    for page in out.glob("*.html"):
        text = page.read_text()
        for old, new in renames.items():
            text = text.replace(f'href="{old}', f'href="{new}')
        page.write_text(text)
    for old, new in renames.items():
        (out / old).rename(out / new)
    (out / "sitemap.json").unlink(missing_ok=True)


def sidebar(out):
    """Put the contents of the front page beside every page, marking the page."""
    index = (out / "index.html").read_text()
    start = index.index('<nav id="TOC"')
    contents = index[start : index.index("</nav>", start)]
    contents = contents[contents.index("<ul") :]
    for page in out.glob("*.html"):
        text = page.read_text()
        here = contents.replace(f'href="{page.name}"', f'class="here" href="{page.name}"', 1)
        text = text.replace("<!--SIDEBAR-->", here)
        page.write_text(text)


if __name__ == "__main__":
    BIN = Path(tempfile.mkdtemp(prefix="zeno-book-bin-"))
    os.symlink(zeno_binary(), BIN / "zeno")
    wanted = set(sys.argv[1:])
    # --chapter=11 checks the samples of the chapters whose files start so.
    only = next((w.split("=", 1)[1] for w in wanted if w.startswith("--chapter=")), None)
    samples(only)
    if only:
        sys.exit(0)
    if "--samples" in wanted:
        sys.exit(0)
    if not wanted or "--pdf" in wanted:
        pdf()
    if not wanted or "--html" in wanted:
        html()
