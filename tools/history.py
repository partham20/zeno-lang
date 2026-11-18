#!/usr/bin/env python3
"""Writes HISTORY.md: every commit of this repository, oldest first, under
the release it went into.

    python3 tools/history.py             write HISTORY.md
    python3 tools/history.py --check     write nothing; fail if HISTORY.md is not what would be written
    python3 tools/history.py --missing   print a line to fill in for each commit that has no note

What is taken from git: the commits and their order, their dates, and the
tags. A tag `vX.Y.Z` on a commit of the history ends a release, so a new tag
makes a new heading by itself. Commits after the last tag go under
"Unreleased".

What is taken from tools/history-notes.txt: the words. That file has

    = 0.5.2: what the release brought, in a few words
    A paragraph or more about the release.

    == A heading for a run of commits
    3e2c68f One plain line on what the commit changed.

A commit with no line there is listed with the first line of its own message,
under "Later commits", so the page is whole even when the notes are behind.
A release with no block there gets its heading and no introduction. The block
named Unreleased goes with whichever release now holds its first commit, so a
tag made before the notes were touched still gets its introduction.

A commit that changes nothing but HISTORY.md and the notes is not listed: the
commit that writes this file cannot name itself (decisions, chapter 49). So a
commit gets its line in a later commit, which holds the two files and no other.

Needs Python 3.9 and git, and a clone with the whole history and its tags
(`git fetch --unshallow --tags`). The site's tools/build.py turns HISTORY.md
into zeno-lang.org/history/.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NOTES = ROOT / "tools" / "history-notes.txt"
OUT = ROOT / "HISTORY.md"
GITHUB = "https://github.com/zeno-src/zeno-lang"
MONTHS = ["January", "February", "March", "April", "May", "June", "July",
          "August", "September", "October", "November", "December"]
OWN = {"HISTORY.md", "tools/history-notes.txt"}      # a commit of these alone is not listed
UNRELEASED = "Unreleased"
LATER = "Later commits"


def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], check=True, capture_output=True, text=True).stdout


def stop(message):
    sys.exit(f"history.py: {message}")


# --- What git knows -----------------------------------------------------------

def commits(rev):
    """Every commit that `rev` holds, oldest first: hash, date, subject, files."""
    if git("rev-parse", "--is-shallow-repository").strip() == "true":
        stop("this clone holds only part of the history. Run: git fetch --unshallow --tags")
    out = git("log", "--reverse", "--topo-order", "--date=short", "--name-only",
              "--format=%x01%H%x1f%ad%x1f%s", rev)
    found = []
    for entry in out.split("\x01")[1:]:
        head, _, files = entry.partition("\n")
        full, date, subject = head.split("\x1f")
        found.append({"hash": full, "date": date, "subject": subject.strip(),
                      "files": [f for f in files.split("\n") if f.strip()]})
    return found


def tags():
    """The release tags: the hash of a tagged commit to its version."""
    out = git("for-each-ref", "refs/tags", "--format=%(refname:short) %(objectname) %(*objectname)")
    found = {}
    for line in out.splitlines():
        name, *hashes = line.split()
        if re.fullmatch(r"v\d+\.\d+\.\d+", name):
            found[hashes[-1]] = name[1:]        # an annotated tag names its commit last
    return found


# --- What the notes say ----------------------------------------------------------

def read_notes():
    """The notes file: the opening text, a block for each release, a line for each commit."""
    notes = {"opening": [], "releases": {}, "order": [], "lines": {}, "sections": {}, "block": {}}
    release = section = None
    into = notes["opening"]
    for number, line in enumerate(NOTES.read_text().splitlines(), 1):
        if line.startswith("# ") or line == "#":
            continue
        if line.startswith("== "):
            section = line[3:].strip()
            continue
        if line.startswith("= "):
            name, _, brought = line[2:].partition(":")
            release, section = name.strip(), None
            if release in notes["releases"]:
                stop(f"{NOTES.name}, line {number}: the release {release} has two blocks")
            into = []
            notes["releases"][release] = {"brought": brought.strip(), "text": into}
            notes["order"].append(release)
            continue
        m = re.match(r"([0-9a-f]{7,40})\s+(\S.*)$", line)
        if m:
            if release is None:
                stop(f"{NOTES.name}, line {number}: a commit is noted before the first release block")
            short = m.group(1)
            if short in notes["lines"]:
                stop(f"{NOTES.name}, line {number}: {short} is noted twice")
            notes["lines"][short] = m.group(2).strip()
            notes["sections"][short] = section
            notes["block"][short] = release
            continue
        into.append(line)
    return notes


def paragraphs(lines):
    return "\n".join(lines).strip()


# --- Putting the two together -------------------------------------------------

def gather(rev):
    notes = read_notes()
    listed = [c for c in commits(rev) if not c["files"] or not set(c["files"]) <= OWN]
    tagged = tags()
    by_hash = {c["hash"]: c for c in listed}
    problems = []

    # Each note finds its commit by the start of the hash.
    for short in notes["lines"]:
        matches = [h for h in by_hash if h.startswith(short)]
        if len(matches) > 1:
            stop(f"{NOTES.name}: {short} is the start of more than one commit; write more of it")
        if not matches:
            problems.append(f"{NOTES.name} has a line for {short}, which is not a commit of this history")
            continue
        commit = by_hash[matches[0]]
        commit["note"] = notes["lines"][short]
        commit["section"] = notes["sections"][short]
        commit["block"] = notes["block"][short]

    # A release is the commits up to and with a tagged one.
    releases, current = [], []
    for commit in listed:
        current.append(commit)
        if commit["hash"] in tagged:
            releases.append({"name": tagged[commit["hash"]], "date": commit["date"], "commits": current})
            commit["released"] = tagged[commit["hash"]]
            current = []
    if current:
        releases.append({"name": UNRELEASED, "date": None, "commits": current})

    # A release the notes know and git does not: the clone has no tags, or the notes are wrong.
    names = {r["name"] for r in releases}
    for name in notes["releases"]:
        if name != UNRELEASED and name not in names:
            stop(f"{NOTES.name} has a release {name}, and no commit of this history is tagged v{name}. "
                 f"If the tag exists, run: git fetch --tags")

    # The Unreleased block goes with the release that holds its first commit.
    renamed = {}
    first = next((c for c in listed if c.get("block") == UNRELEASED), None)
    if first is not None:
        holder = next(r["name"] for r in releases if first in r["commits"])
        if holder != UNRELEASED and holder not in notes["releases"]:
            renamed[holder] = UNRELEASED
            problems.append(f"{holder} is tagged, and {NOTES.name} still calls its block {UNRELEASED}: "
                            f"rename the block to {holder} and begin a new {UNRELEASED}")
        elif holder != UNRELEASED:
            notes["releases"].pop(UNRELEASED, None)

    for release in releases:
        block = notes["releases"].get(renamed.get(release["name"], release["name"]))
        release["brought"] = block["brought"] if block else ""
        release["text"] = paragraphs(block["text"]) if block else ""
        if block is None and release["name"] != UNRELEASED:
            problems.append(f"{release['name']} has no block in {NOTES.name}, so no introduction")
        # A commit with no note that lies inside a section stays in it. One that lies
        # after the last noted commit, or between two sections, goes under a heading
        # of its own, so that no heading claims a commit it was not written for.
        row = release["commits"]
        for i, commit in enumerate(row):
            if "note" not in commit:
                before = row[i - 1]["section"] if i else None
                after = next((c["section"] for c in row[i + 1:] if "note" in c), LATER)
                commit["section"] = before if i and before == after else LATER
    missing = [c for c in listed if "note" not in c]
    if missing:
        problems.append(f"{counted(len(missing), 'commit')} with no line in {NOTES.name} "
                        f"(python3 tools/history.py --missing prints them): " +
                        ", ".join(c["hash"][:7] for c in missing[:8]) + (" ..." if len(missing) > 8 else ""))
    return notes, releases, listed, problems


# --- Writing ---------------------------------------------------------------------

def counted(n, word):
    return f"{n} {word}" if n == 1 else f"{n} {word}s"


def day(date, year=True):
    y, m, d = (int(part) for part in date.split("-"))
    return f"{d} {MONTHS[m - 1]}" + (f" {y}" if year else "")


def days(commits_):
    first, last = min(c["date"] for c in commits_), max(c["date"] for c in commits_)
    if first == last:
        return day(first)
    if first[:7] == last[:7]:
        return f"{int(first[8:])} to {day(last)}"
    return f"{day(first, year=first[:4] != last[:4])} to {day(last)}"


def anchor(heading):
    """The name GitHub and the site both give a heading."""
    return re.sub(r"\s", "-", re.sub(r"[^\w\s-]", "", heading).strip().lower())


def heading(release):
    return UNRELEASED if release["name"] == UNRELEASED else f"Zeno {release['name']}"


def render(notes, releases, listed):
    short = 7
    while len({c["hash"][:short] for c in listed}) < len(listed):
        short += 1
    out = ["# The history of Zeno", "", paragraphs(notes["opening"]), "", "## The releases", ""]
    for release in releases:
        when = "on `main` now" if release["date"] is None else day(release["date"])
        brought = f": {release['brought']}" if release["brought"] else ""
        out.append(f"- [**{heading(release)}**](#{anchor(heading(release))}), {when}, "
                   f"{counted(len(release['commits']), 'commit')}{brought}")
    out.append("")
    for release in releases:
        size = counted(len(release["commits"]), "commit")
        out += [f"## {heading(release)}", ""]
        if release["date"] is None:
            out.append(f"*Not in a release yet · {size} · {days(release['commits'])}*")
        else:
            out.append(f"*Released {day(release['date'])} · {size} · "
                       f"[the release on GitHub]({GITHUB}/releases/tag/v{release['name']})*")
        out.append("")
        if release["text"]:
            out += [release["text"], ""]
        section = object()
        for i, commit in enumerate(release["commits"]):
            if commit.get("section") != section:
                section = commit.get("section")
                run = release["commits"][i:]
                end = next((j for j, c in enumerate(run) if c.get("section") != section), len(run))
                if i:
                    out.append("")
                if section:
                    out += [f"### {section}", "", f"*{days(run[:end])}*", ""]
            line = commit.get("note") or commit["subject"]
            if not line.endswith((".", "?", "!")):
                line += "."
            if "released" in commit:
                line += f" **This commit is Zeno {commit['released']}.**"
            out.append(f"- [`{commit['hash'][:short]}`]({GITHUB}/commit/{commit['hash']}) {line}")
        out.append("")
    last = listed[-1]
    out += ["---", "",
            f"This file is written by [`tools/history.py`](tools/history.py) from the commits themselves and "
            f"the notes in [`tools/history-notes.txt`](tools/history-notes.txt). It lists "
            f"{counted(len(listed), 'commit')}, up to `{last['hash'][:short]}` of {day(last['date'])}. "
            f"A commit that only brings this file or its notes up to date is not listed.", ""]
    return "\n".join(out)


def main():
    parser = argparse.ArgumentParser(description="Writes HISTORY.md from git and tools/history-notes.txt.")
    parser.add_argument("--check", action="store_true", help="write nothing; fail if HISTORY.md is not what would be written")
    parser.add_argument("--missing", action="store_true", help="print a line to fill in for each commit with no note")
    parser.add_argument("--rev", default="HEAD", help="the newest commit to list (HEAD)")
    args = parser.parse_args()

    notes, releases, listed, problems = gather(args.rev)
    if args.missing:
        for commit in listed:
            if "note" not in commit:
                print(f"{commit['hash'][:7]} {commit['subject']}")
        return
    for problem in problems:
        print(f"history.py: note: {problem}", file=sys.stderr)
    text = render(notes, releases, listed)
    if args.check:
        if not OUT.exists() or OUT.read_text() != text:
            stop("HISTORY.md is not what would be written. Run: python3 tools/history.py")
        print(f"HISTORY.md is up to date: {counted(len(listed), 'commit')}, {counted(len(releases), 'heading')}")
        return
    OUT.write_text(text)
    print(f"wrote HISTORY.md: {counted(len(listed), 'commit')} under {counted(len(releases), 'heading')}")


if __name__ == "__main__":
    main()
