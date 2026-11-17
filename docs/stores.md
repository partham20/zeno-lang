# Stores: where each package manager looks for Zeno

[`packages.md`](packages.md) is about one half: a release has, among
its files, a package for each of six managers, and each can install
Zeno from that file.

This is the other half. `apt install zeno` with no file works only if
Zeno is in the place apt looks in, and the same holds for the others.
Each manager has such a place, and each place has somebody who decides
what goes into it. So this half is less about files and more about
accounts: most of what is left can only be done by Partha, signed in
as himself.

Everything that needs no account is done, and is tried by the workflow
`stores`. For the rest, each part below ends with **What is left**:
the steps, in order, with the page for each.

| Manager | Where it looks | State | The line, once it is there |
|---|---|---|---|
| Homebrew | a repository on GitHub, here this one | **works now** | `brew install zeno-src/zeno-lang/zeno` |
| apt | a signed list beside the `.deb` files, here in the release | made and tried, waits for a key | `sudo apt install zeno` |
| pacman | the AUR | files ready, waits for an account | `yay -S zeno-lang-bin` |
| winget | Microsoft's `winget-pkgs` | files ready, waits for a pull request | `winget install ZenoLang.Zeno` |
| snap | the Snap Store | upload ready, waits for an account and for leave to be `classic` | `sudo snap install zeno --classic` |
| flatpak | Flathub | **will not be there** (below) | the file from the release |

The decisions are 15.22 to 15.28 of [`decisions.md`](decisions.md).

## Homebrew

Homebrew reads a formula from any git repository, which it calls a
tap. The formula is in this repository, in `Formula/zeno.rb`, and the
repository is public, so there is nothing more to do:

```sh
brew tap zeno-src/zeno-lang https://github.com/zeno-src/zeno-lang
brew install zeno-src/zeno-lang/zeno
```

The two lines were run as they stand on a Mac and on Linux, on 16
November 2026, and Zeno 0.2.0 was installed, run and removed.

Being in Homebrew's own collection, so that `brew install zeno` works
with no tap, is another matter: it takes programs that many people
already use, and asks for proof of that.

**What is left:** nothing.

## apt

apt takes a package from a place only when two things are there. One
is a list, `Packages`, which says for each `.deb` what it is, how big
it is and its checksum. The other is `InRelease`: the checksum of that
list, signed with a key that the machine was told to trust. A `.deb`
that was changed on the way does not match the list, and a list that
was changed does not match its signature.

The usual apt repository is a tree of folders on a web server. Zeno's
is smaller: the list and its signature are files of the release, beside
the `.deb` files. apt is given the address that the newest release's
files are downloaded from, and finds everything there. apt calls this a
flat repository. Nothing has to be hosted, and it cannot fall behind
the release, because it is the release.

[`tools/apt-repo.sh`](../tools/apt-repo.sh) writes the four files:

| File | What it is |
|---|---|
| `Packages`, `Packages.gz` | the list, plain and packed |
| `InRelease` | the checksums of the list, signed |
| `zeno-apt-key.asc` | the public half of the key |

What somebody types, once, and from then on apt updates Zeno with
everything else:

```sh
curl -fsSL https://github.com/zeno-src/zeno-lang/releases/latest/download/zeno-apt-key.asc |
    sudo tee /usr/share/keyrings/zeno.asc > /dev/null
echo "deb [signed-by=/usr/share/keyrings/zeno.asc] https://github.com/zeno-src/zeno-lang/releases/latest/download/ /" |
    sudo tee /etc/apt/sources.list.d/zeno.list
sudo apt update
sudo apt install zeno
```

The first line gives the machine the key. The second names the place,
and says that this key, and no other, may sign for it. The ` /` at the
end is what says "flat".

**The key.** A signature needs a key with two halves. The private half
signs and must stay secret. The public half checks a signature and is
given to everybody. The private half is kept as a secret of the
repository, `APT_SIGNING_KEY`, where a workflow can use it and nobody
can read it back.

It was tried with a key made for the trial and thrown away: the lists
were made from the four `.deb` files of 0.2.0, and apt read them,
checked the signature, installed Zeno and removed it.

**What is left**

1. Make the key. In Git Bash on Windows, or any Linux or macOS shell:

   ```sh
   gpg --batch --passphrase "" --quick-generate-key "Zeno packages" rsa4096 sign never
   gpg --armor --export-secret-keys "Zeno packages" > zeno-apt-private.asc
   ```

2. Open
   [New secret](https://github.com/zeno-src/zeno-lang/settings/secrets/actions/new),
   name it `APT_SIGNING_KEY`, and paste the whole of
   `zeno-apt-private.asc` as its value.
3. Keep `zeno-apt-private.asc` somewhere safe and nowhere public. If
   it is lost, a new key has to be made, and everybody who added the
   old one has to add the new one.
4. Run the workflow `stores` with "send" ticked (below). It adds the
   four files to the release and then installs Zeno with the lines
   above, from the release itself.

## pacman: the AUR

Arch has a place where anybody can put a recipe, the AUR. What is put
there is not a package but the `PKGBUILD` that makes one, and a helper
such as `yay` downloads it and runs it. Each package of the AUR is a
small git repository, and putting something there is a `git push`.

The package is `zeno-lang-bin`. `-bin` is how Arch names a package of
a program that is already built. It is not `zeno-bin`, because the AUR
has a `zeno` already, which is another program, and `zeno-bin` would
say "that one, already built".

Three files go there, from [`packaging/arch`](../packaging/arch):

| File | What it is |
|---|---|
| `PKGBUILD` | the recipe |
| `.SRCINFO` | the same as plain lines. The AUR reads this and does not run the recipe. `makepkg --printsrcinfo` writes it |
| `LICENSE` | the licence of the recipe itself, which the AUR asks for: 0BSD, "do as you like" |

It was tried in Arch, in a container: `makepkg` downloads the archive
from the release, checks it, makes the package, and pacman installs
it, and `.SRCINFO` says what the `PKGBUILD` says.

**What is left**

1. [Make an account on the AUR](https://aur.archlinux.org/register).
   The form asks for the answer of a command run on Arch, to keep
   robots out. Anybody without Arch can run it in a container:
   `docker run --rm archlinux sh -c '<the command>'`.
2. Make a key for it:

   ```sh
   ssh-keygen -t ed25519 -N "" -C "zeno-lang AUR" -f aur
   ```

   That writes two files. `aur.pub` is the public half: paste it into
   "SSH Public Key" on the AUR's
   [My Account](https://aur.archlinux.org/account) page.
3. Open
   [New secret](https://github.com/zeno-src/zeno-lang/settings/secrets/actions/new),
   name it `AUR_SSH_KEY`, and paste the whole of the file `aur`, the
   private half.
4. Run the workflow `stores` with "send" ticked. The first push makes
   the package.

## winget

winget's list is a repository of Microsoft's on GitHub,
[`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs),
with three small files for each version of each program. A program
gets in by a pull request that adds its files. Robots there download
the setup program, scan it, install it without showing it, and a
person approves.

The files are in [`packaging/winget`](../packaging/winget), in the form
the list takes today (1.28.0). They are checked against Microsoft's own
schema, and that they name the setup program of the release and its
checksum. They have still never been run by a winget: GitHub's Windows
machines have none.

**What is left**

1. Try them once, on a Windows machine, in a terminal opened as
   administrator, from a copy of this repository:

   ```
   winget settings --enable LocalManifestFiles
   winget validate --manifest packaging\winget
   winget install --manifest packaging\winget
   zeno --version
   winget uninstall ZenoLang.Zeno
   ```

2. [Fork `winget-pkgs`](https://github.com/microsoft/winget-pkgs/fork).
3. Put the three files into the fork, in a folder named exactly
   `manifests/z/ZenoLang/Zeno/0.2.0/`, on a branch of their own:

   ```sh
   git clone --filter=blob:none --no-checkout https://github.com/partham20/winget-pkgs
   cd winget-pkgs
   git sparse-checkout set manifests/z/ZenoLang
   git checkout -b zeno-0.2.0
   mkdir -p manifests/z/ZenoLang/Zeno/0.2.0
   cp ../zeno-lang/packaging/winget/*.yaml manifests/z/ZenoLang/Zeno/0.2.0/
   git add manifests
   git commit -m "New package: ZenoLang.Zeno version 0.2.0"
   git push -u origin zeno-0.2.0
   ```

4. Open the pull request that GitHub then offers. A robot there may
   ask, once, for agreement to Microsoft's terms for contributors,
   which is given by a comment that it spells out. If the pull request
   is marked as needing an answer, there are ten days to give one.

Each later version is the same with its own folder, or one line of
Microsoft's `wingetcreate update`.

The setup program is not signed, so Windows shows its warning to
somebody who downloads it by hand. winget installs it all the same.

## snap

The Snap Store is one company's, Canonical's, and a snap gets there by
an upload from an account. Two things stand before the upload.

**The name.** `zeno` has to be registered to the account. No snap of
that name is published today. Somebody may still hold it, and then the
store says so.

**`classic`.** Zeno's snap is not shut in ([`packages.md`](packages.md)
says why), and the store gives that only when asked, after a person has
read the reasons. Compilers and programming languages are in their list
of what it is given for. The request is a post in their forum, and its
text is written: [`packaging/snap/classic-request.md`](../packaging/snap/classic-request.md).

The three snaps of a release are uploaded as they are. `snapcraft`,
which most snaps are built with, is not used to build them, only to
send them.

**What is left**

1. [Make an account](https://snapcraft.io/account) (it is an Ubuntu
   One account).
2. [Register the name `zeno`](https://snapcraft.io/register-snap).
3. Post the request as a new topic in
   [store-requests](https://forum.snapcraft.io/c/store-requests/19),
   and wait for the answer. It can take days.
4. Make a login for the workflow. This needs `snapcraft`, which runs
   on Linux (or in WSL on Windows):

   ```sh
   sudo snap install snapcraft --classic
   snapcraft export-login --snaps=zeno --acls=package_access,package_push,package_update,package_release login.txt
   ```

5. Open
   [New secret](https://github.com/zeno-src/zeno-lang/settings/secrets/actions/new),
   name it `SNAPCRAFT_STORE_CREDENTIALS`, and paste the whole of
   `login.txt`. Then delete the file.
6. Once `classic` is given, run the workflow `stores` with "send"
   ticked. Before that the store holds an upload back.

## flatpak: not on Flathub

Flathub is the place nearly every flatpak comes from, and it does not
take programs that are run in a terminal. Its rules say so in as many
words, with an exception only for tools that are about flatpak itself.
It also wants every program built from its source on Flathub's own
machines, where Zeno's flatpak holds the `zeno` of the release.

So Zeno is not asked for there. The flatpak stays what it is: a file of
each release, which `flatpak install` takes
([`packages.md`](packages.md)).

**What is left:** nothing.

## The workflow `stores`

[`.github/workflows/stores.yml`](../.github/workflows/stores.yml) is
started by hand from the Actions page, saying the version. It has a
job for each store, and each job first tries what can be tried with no
login:

| Job | What it tries |
|---|---|
| apt | makes the lists, signs them, and has apt install from them |
| AUR | makes the package from the `PKGBUILD` in Arch, downloading from the release, and compares `.SRCINFO` |
| snap | asks the store whether a snap named `zeno` is published |
| Homebrew | the two lines above, against this repository (with "mac" ticked, on a Mac as well) |
| winget | the three files against Microsoft's schema, and the checksum |

With **"send"** ticked, a job then sends to its store, if the
repository has the secret for it, and otherwise says that it has not
and does nothing:

| Secret | What is sent |
|---|---|
| `APT_SIGNING_KEY` | the four files for apt are added to the release, and apt then installs from the release itself |
| `AUR_SSH_KEY` | `PKGBUILD`, `.SRCINFO` and `LICENSE` are pushed to the AUR |
| `SNAPCRAFT_STORE_CREDENTIALS` | the three snaps are uploaded to `stable` |

So a store is switched on by giving its secret, and one that has none
is left alone.

A release does this by itself. Its `Publish` step makes the files for
apt with everything else, when there is a key, so that apt never finds
a release without them. Its last step starts `stores` with "send"
ticked, for the AUR and the Snap Store. winget is the one that stays a
pull request by hand.

## What has not been tried

- **The three sends.** No workflow has ever pushed to the AUR, uploaded
  to the Snap Store, or added lists signed with the real key, because
  there is no account and no key yet. Each is a few lines, and the
  first run with a secret is their trial.
- **The winget files** have not been run by a winget.
- **apt from the release itself.** apt was shown to follow GitHub's
  way of answering a download, and to install from the same files
  handed out by another server. The two together run for the first
  time when the real key is there.

## Things to try

1. Run the workflow `stores` with nothing ticked, and read what each
   job says.
2. In `tools/apt-repo.sh`, find where the checksum of `Packages` is
   written. Change one byte of `Packages` after it is signed, and see
   what `apt update` says.
3. Read `packaging/arch/.SRCINFO` beside `packaging/arch/PKGBUILD`, and
   find each line of one in the other.
