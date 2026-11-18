# The registry: a package by its name

A program can use a package that somebody else keeps. Until now it
had to say where the package is:

```
needs units from "https://github.com/someone/zeno-units" at "v1.2.0"
```

With the registry it can say only what the package is called:

```
needs units at "v1.2.0"
```

The **registry** is a list that says, for each name, which git
repository the package is, and which commit of it each version is.
`zeno fetch` asks it, `zeno search` reads it, and `zeno publish` is
how a package gets into it. Zeno's own registry is at
[github.com/zeno-src/packages](https://github.com/zeno-src/packages),
and a page that shows it is at
[zeno-lang.org/packages](https://zeno-lang.org/packages/).

The two lines that were there before work as they did, and
[`using-packages.md`](using-packages.md) is the page about them and
about how an import finds a package's files. The decisions for this
page are chapter 27 of [`decisions.md`](decisions.md). The code is
[`src/registry.rs`](../src/registry.rs), and `fetch` in
[`src/package.rs`](../src/package.rs).

## Three ways to need a package

| Line | The package is | Who knows which commit |
|---|---|---|
| `needs filters in "../filters"` | in that folder, and is used where it is | nobody has to: it is the files that are there |
| `needs units from "address" at "v1.2.0"` | a git repository, at a tag, a branch or a commit | the repository, when `zeno fetch` asks it |
| `needs units at "v1.2.0"` | whatever the registry says `units` is | the registry, which wrote the commit down when the version was published |

A version in the registry is written `v1.2.0`: the letter v and three
numbers. That is a tag of the package's repository, so the third line
is the second with the address left out, and somebody who moves a
package into the registry deletes one word and one address from the
lines that need it.

A version is one exact thing here too. There are no ranges such as
`^1.2`, and `zeno fetch` never picks a version for you (decision 8.4).

## What a registry is

A git repository. There is no server, no database and no account to
make. It holds one small file for each package, in a folder called
`index`. This is `index/units`:

```
package units
about "Lengths, masses and times that the checker keeps apart"
from "https://github.com/someone/zeno-units"
owner "someone"

version "v1.0.0" commit "0123abc..." dated "2026-11-02"
version "v1.2.0" commit "9f8e7d6..." dated "2026-11-18"
```

It is written in the same plain words as `zeno.package` and
`zeno.lock`, and read by Zeno's own lexer. The four lines at the top
say what the package is called, what it is for, where it is, and who
published it. Then there is one line for each version.

The packages are not in the registry. `units` stays in its author's
repository, and the registry only says where that is. So the registry
stays small: a few hundred bytes for each package, however big the
package is.

**The commit is what is fetched, not the tag.** A tag can be moved. If
`v1.2.0` is moved to another commit after it was published, everybody
still gets `9f8e7d6`, because that is what the registry wrote down.
A published version stays what it was.

`index.json`, at the top of the registry, is the same list as one
file of JSON. `zeno` does not read it. It is there for the page on
zeno-lang.org, which a browser can only give JSON to.

## `zeno fetch`, with a name

```
$ zeno fetch
Asked the registry https://github.com/zeno-src/packages
Fetched units v1.2.0 (9f8e7d6) into packages/units
Fetched prefix v1.0.0 (d4461d9) into packages/prefix
Wrote zeno.lock
```

For a line that names no address, `fetch` does this:

1. **It gets the registry**, with git, into a folder of this machine
   where things are kept that can be fetched again (`Index::open`).
   Only the newest state is fetched, not the registry's history. It
   is asked once in a run, however many names there are.
2. **It reads `index/units`** and finds the line for `v1.2.0`
   (`Index::version`). Now it has an address and a commit.
3. **It goes on as for a line with an address**: it clones the
   repository into `packages/units`, and checks out that commit
   (`get`).
4. **It reads the `zeno.package` of what it got**, and does the same
   for whatever that needs. `units` needs `prefix`, so that came too.
5. **It writes `zeno.lock`**.

The line in the lock begins as the `needs` line does, and then says
what was found:

```
prefix at "v1.0.0" from "https://github.com/someone/zeno-prefix" commit "d4461d9..."
units at "v1.2.0" from "https://github.com/someone/zeno-units" commit "9f8e7d6..."
```

**With the lock, nobody is asked.** The next `zeno fetch`, here or on
another machine, finds the address and the commit in `zeno.lock`. It
does not get the registry at all. If the packages are in `packages/`
at those commits, it does not use the network. A program with its
lock builds the same next year, whatever has happened to the registry
since.

`zeno fetch --update` looks every version up again, as before.

What can go wrong says what there is:

```
zeno: the registry has no version `v9.0.0` of `units`. It has v1.2.0 and v1.0.0
zeno: the registry https://github.com/zeno-src/packages has no package called
`unit`. `zeno search` and a word shows the packages it has
```

### Where the copy of the registry is

| On | In |
|---|---|
| Linux | `~/.cache/zeno/registry` (or under `$XDG_CACHE_HOME`) |
| macOS | `~/Library/Caches/zeno/registry` |
| Windows | `%LOCALAPPDATA%\zeno\registry` |

`ZENO_CACHE` names another folder. The copy can be deleted at any
time: it is made again when it is next needed.

## `zeno search`

```
$ zeno search noisy
1 package in the registry https://github.com/zeno-src/packages:

filters v1.1.0    Filters for noisy readings
    needs filters at "v1.1.0"
    https://github.com/someone/zeno-filters
    older: v1.0.0
```

It gets the registry, and lists the packages that have every word
you gave in their name or in their `about` line. With no word it
lists them all. The second line of each package is the line to put
in a `zeno.package`.

With no network it shows the copy from last time, and says that this
is what it is doing.

## Publishing a package

A package that can be published is a git repository with a
`zeno.package` at its top, which says what the package is for:

```
package filters
about "Filters for noisy readings"
```

`about` is one line of at most 120 characters. It is what `zeno
search` and the page show.

Then a commit is given a version, which is a tag, and both are
pushed:

```
git tag v1.0.0
git push origin main v1.0.0
```

and, in the package's folder:

```
$ zeno publish
`filters` v1.0.0 (32a7444) is ready to be published, from https://github.com/someone/zeno-filters

Open this address, signed in to GitHub, and press the button under the form:

    https://github.com/zeno-src/packages/issues/new?template=publish.yml&title=...

The registry checks the package and answers on that page, most often within a few minutes.
```

### What `zeno publish` checks

It changes nothing, on this machine or any other. It checks, and each
thing it finds wrong comes with what to do about it:

| It checks that | Because |
|---|---|
| the name is 2 to 40 small letters, digits and `_` | a registry is a folder of files named after packages, and Windows and macOS take `Units` and `units` for one file |
| there is an `about` line | a list of names alone tells a searcher nothing |
| the folder is the top of a git repository | the registry reads `zeno.package` at the top of what it fetches |
| nothing is changed and not committed | a version is a commit, and what was checked must be what is published |
| the commit has a version tag (`zeno publish v1.2.0` says which, if it has two) | |
| the tag is at `origin`, on the same commit | the registry fetches it from there |
| no `needs ... in` line points outside the package | nobody who fetches the package would have that folder |
| every `needs ... at` line names a version the registry has | a package in the registry can be fetched whole from it |
| the name is free, or is this repository's already | a name has one owner |
| the version is not in the registry already | a published version stays what it was |

### What the registry does

Zeno's own registry is on GitHub, and asking it is a form there: an
issue with two answers, the repository and the version. `zeno
publish` gives the address of the form with both filled in.

When the form is sent, a workflow in the registry's repository starts
[`tools/publish.py`](https://github.com/zeno-src/packages/blob/main/tools/publish.py),
which

1. reads the two answers, and takes them only if they are exactly an
   address of a repository on GitHub and a version;
2. checks **who sent the form**. GitHub says which account that was.
   The repository must belong to that account, or to an organization
   that shows the account as a member;
3. runs `zeno registry accept`, below, which checks the package again
   from the repository, since what `zeno publish` saw was a folder on
   somebody's machine;
4. commits and pushes the two files that wrote, answers on the issue,
   and closes it.

So the accounts are GitHub's, and the registry needs none of its own.

**The first to publish a name owns it.** The file says `owner
"someone"`, and a later version of `units` is taken only from the
same repository, asked for by the same account.

## The registry's side: `zeno registry accept`

```
zeno registry accept <folder> <address> <version> --owner <who>
```

takes one version of the package at `address` into the clone of a
registry in `folder`. This is the whole of what a registry does to
take a package (`accept` in `registry.rs`):

1. It clones the repository at that tag into a folder that it takes
   away again, and refuses a branch of that name: a branch moves.
2. It notes the commit, and the day of the commit.
3. It reads the `zeno.package` there and checks the package as `zeno
   publish` does (`fits`).
4. If the name is in the registry already, the address must be the
   same, the owner must be the same, and the version must be new.
5. It writes `index/<name>` and `index.json`.

It commits nothing. Whoever ran it looks at what was written, and
commits and pushes.

```
$ zeno registry accept . https://github.com/someone/zeno-filters v1.0.0 --owner someone
Accepted filters v1.0.0 (32a7444) from https://github.com/someone/zeno-filters
Wrote index/filters and index.json
```

`zeno registry index <folder>` writes `index.json` again from the
files in `index`. It is for after a change by hand, such as taking a
package out, and it reads every file on the way, so it is also how a
registry is checked.

## A registry of your own

Because a registry is any git repository, a company can keep one
inside. An empty repository is a registry with no packages. A program
names it:

```
package logger
registry "https://git.example.com/zeno-packages"

needs units at "v1.2.0"
```

and every name in that program is looked up there, in place of
Zeno's own. Like `memory`, it is the line in the program's own
`zeno.package` that counts: a `registry` line in a needed package
says nothing about the program that needs it.

For a registry that is not on GitHub there is no form, so `zeno
publish` ends with the line for whoever keeps it:

```
The registry https://git.example.com/zeno-packages is added to by whoever keeps it. In a clone of it, they run:

    zeno registry accept . https://git.example.com/units v1.2.0 --owner NAME
```

Who may publish is then who may push to that repository.

## Where each part is

| In | Is |
|---|---|
| `package.rs`, `parse` | the lines `needs name at "v1.2.0"`, `about "..."` and `registry "..."` |
| `package.rs`, `fetch_with` | `zeno fetch`: the lock first, then the registry, then `get` |
| `package.rs`, `parse_lock`, `write_lock` | the two kinds of line in `zeno.lock` |
| `registry.rs`, `Index` | this machine's copy of a registry, and looking a name up |
| `registry.rs`, `parse_entry`, `write_entry`, `write_list` | a file of the index, and `index.json` |
| `registry.rs`, `search`, `publish`, `accept`, `relist` | the four commands |
| `registry.rs`, `fits` | what a package must be to be published, asked by both `publish` and `accept` |
| `modules.rs`, `need` | where an import finds a fetched package: `packages/<name>`, whichever line needed it |

## What it does not do yet

- **Nothing takes a version out.** A version that should not be used
  is taken out by a keeper of the registry, by hand. There is no
  `zeno unpublish`, and no way to mark a version as one to avoid.
- **The form takes packages on GitHub only.** One that lives
  elsewhere is added by a keeper, with `zeno registry accept`.
- **An owner is one account.** A package cannot yet be handed to
  somebody else, or have two owners, except by a keeper's hand.
- **No count of downloads, no page for each package.** The registry
  never sees a fetch: git goes straight to the package's repository.
- **`zeno publish` does not run the package's tests.** It checks that
  the package can be fetched, not that it is good.
- **A package is a whole repository.** Two packages cannot share one.
- **A program that changes its `registry` line** should run `zeno
  fetch --update`: the lock does not say which registry a line came
  from.

## Things to try

1. Make a registry in a folder: `git init --bare reg.git`, and `git
   clone reg.git keeper`. Make a package in another folder with an
   `about` line, commit it, tag it `v0.1.0`, and take it in with `zeno
   registry accept keeper file:///full/path/to/package v0.1.0 --owner
   me`. Read `keeper/index/<name>` and `keeper/index.json`.
2. Commit and push in `keeper`. Write a program whose `zeno.package`
   has `registry "file:///full/path/to/reg.git"` and needs your
   package by name. Run `zeno fetch`, and read `zeno.lock`.
3. Delete `packages/` and the copy of the registry (the table above
   says where it is). Run `zeno fetch` again. Which lines does it
   print, and what did it not have to ask?
4. Move the tag: change the package, commit, `git tag --force
   v0.1.0`. Run `zeno fetch --update` in the program. Which commit do
   you get, and why? Now try to take `v0.1.0` in again with `accept`.
5. Run `zeno registry accept` for the same package with `--owner
   someone_else` and a new tag. What does it say?
6. Give the package a `needs helper in "../helper"` line and run
   `zeno publish`. Read the message, and find the function in
   `registry.rs` that it comes from. Why is `needs helper in
   "helper"` allowed?
7. In `registry.rs`, `numbers` puts versions in order. What would go
   wrong in `zeno search` if versions were sorted as text? Write
   down two versions that show it.
8. A big one: add `zeno registry remove <folder> <name> <version>`.
   What should happen to a program whose lock names that version?
   What should `zeno fetch --update` say?
