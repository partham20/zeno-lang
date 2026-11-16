#!/bin/sh
# Makes the packages of a release of Zeno, for the programs that install
# other programs: apt, pacman, snap, flatpak, Homebrew and winget.
#
#     tools/package.sh 0.2.0 files out
#
# `files` is a folder that holds the files of that release: the
# archives, and the Windows setup program if there is one. Nothing is
# built here. Each package holds the `zeno` that an archive holds, laid
# out the way its system wants it.
#
# Into `out` go the packages that are installed from a file:
#
#     zeno_0.2.0_amd64.deb  _arm64  _armhf  _i386      apt install ./that
#     zeno_0.2.0_amd64.snap  _arm64  _armhf            snap install --classic --dangerous that
#     zeno-0.2.0-x86_64.flatpak  -aarch64              flatpak install that
#     zeno-bin-0.2.0-1-x86_64.pkg.tar.zst              pacman -U that
#
# and into `out/recipes` the three that are text, which say where the
# files of the release are and what their checksums must be:
#
#     Formula/zeno.rb            Homebrew
#     packaging/arch/PKGBUILD    pacman (and the AUR)
#     packaging/winget/*.yaml    winget
#
# `recipes` is laid out as this repository is, because that is where
# the release workflow puts them.
#
# A kind of package whose tool is not on this machine is left out, with
# a line that says so: `snap` makes a .snap, `flatpak` a .flatpak, and
# `docker` (to run Arch in) the package for pacman.
# ZENO_URL names another place than the release for the recipes to
# download from, which is how they are tried before there is a release.
#
# docs/packages.md walks through it.

set -eu

[ $# = 3 ] || { echo "package.sh: say the version, the folder of the release's files, and where to put the packages" >&2; exit 1; }
version=$1
files=$(cd "$2" && pwd)
mkdir -p "$3"
out=$(cd "$3" && pwd)
url=${ZENO_URL:-https://github.com/partham20/zeno-lang/releases/download/v$version}

say() { printf '%s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
# The checksum of a file of the release, or nothing if it is not there.
sum() { [ -f "$files/$1" ] && sha256sum "$files/$1" | cut -d ' ' -f 1; }

summary="The Zeno programming language"
# One line, then the same in three.
about="A small, readable, statically typed language for firmware and for everything around it. One \`zeno\` checks a program, runs it, tests it, formats it, and compiles it to C or to LLVM IR."
site=https://zeno-lang.org
maintainer="Partha <123355647+partham20@users.noreply.github.com>"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# `unpacked <kind>` unpacks the archive for `linux-<kind>` and says
# where, or fails if the release has no such archive.
unpacked() {
    name=zeno-$version-linux-$1
    [ -f "$files/$name.tar.gz" ] || return 1
    [ -d "$work/$name" ] || tar -xzf "$files/$name.tar.gz" -C "$work"
    say "$work/$name"
}

# `laid_out <from> <root>` puts Zeno under <root> as a Linux system
# keeps a program: the program and `zeno.wasm` (which `zeno build
# --web` looks for beside it) in a folder of their own, a link to the
# program where the PATH looks, the docs with the other docs, and the
# examples and the editor setup in /usr/share/zeno. The examples are
# not with the docs because a small system (a container, mostly) is
# often set up to leave out /usr/share/doc, and `zeno run` of the
# first example should work there too.
laid_out() {
    mkdir -p "$2/usr/lib/zeno" "$2/usr/bin" "$2/usr/share/zeno" "$2/usr/share/doc/zeno"
    cp "$1/zeno" "$2/usr/lib/zeno/zeno"
    chmod 755 "$2/usr/lib/zeno/zeno"
    if [ -f "$1/zeno.wasm" ]; then
        cp "$1/zeno.wasm" "$2/usr/lib/zeno/zeno.wasm"
        chmod 644 "$2/usr/lib/zeno/zeno.wasm"
    fi
    ln -s ../lib/zeno/zeno "$2/usr/bin/zeno"
    cp -r "$1/examples" "$1/editors" "$2/usr/share/zeno/"
    cp -r "$1/README.md" "$1/docs" "$2/usr/share/doc/zeno/"
}

# ------------------------------------------------------------------- apt
#
# A .deb is an archive of the files and a folder, DEBIAN, that says
# what the package is. `Recommends` are installed with it unless
# somebody says not to: that is how apt installs what Zeno hands work
# to. `c-compiler` is any of gcc, clang and tcc.
#
# The one for `armhf` holds the build for ARMv6, which runs on ARMv6
# and on everything later. Raspberry Pi OS calls a Pi Zero `armhf`
# too, and the build for ARMv7 would not start on it.

deb() {
    from=$(unpacked "$2") || { say "no .deb for $1: the release has no archive for linux-$2"; return 0; }
    root=$work/deb-$1
    laid_out "$from" "$root"
    mkdir "$root/DEBIAN"
    cat > "$root/DEBIAN/control" <<EOF
Package: zeno
Version: $version
Architecture: $1
Maintainer: $maintainer
Installed-Size: $(du -sk "$root/usr" | cut -f 1)
Recommends: gcc | c-compiler, libc6-dev | libc-dev, git
Suggests: clang
Section: devel
Priority: optional
Homepage: $site
Description: $summary
 A small, readable, statically typed language for firmware and for
 everything around it. One \`zeno\` checks a program, runs it, tests it,
 formats it, and compiles it to C or to LLVM IR.
 .
 \`zeno build\` hands the C it writes to a C compiler, and \`zeno fetch\`
 gets packages with git. Everything else needs nothing.
EOF
    dpkg-deb --build --root-owner-group "$root" "$out/zeno_${version}_$1.deb" >/dev/null
    say "made zeno_${version}_$1.deb"
}

if have dpkg-deb; then
    deb amd64 x86_64
    deb arm64 arm64
    deb armhf armv6
    deb i386 x86
else
    say "no .deb: this machine has no dpkg-deb"
fi

# ------------------------------------------------------------------ snap
#
# A snap is one file that holds a folder, with `meta/snap.yaml` to say
# what it is. `classic` means that Zeno sees the machine as any other
# program does: it has to, to read a program wherever it is kept and to
# run the C compiler that is installed.

snap_of() {
    from=$(unpacked "$2") || { say "no .snap for $1: the release has no archive for linux-$2"; return 0; }
    root=$work/snap-$1
    mkdir -p "$root/meta" "$root/bin"
    cp "$from/zeno" "$root/bin/zeno"
    if [ -f "$from/zeno.wasm" ]; then cp "$from/zeno.wasm" "$root/bin/zeno.wasm"; fi
    cp -r "$from/README.md" "$from/docs" "$from/examples" "$from/editors" "$root/"
    cat > "$root/meta/snap.yaml" <<EOF
name: zeno
version: '$version'
summary: $summary
description: |
  A small, readable, statically typed language for firmware and for
  everything around it. One \`zeno\` checks a program, runs it, tests it,
  formats it, and compiles it to C or to LLVM IR.

  \`zeno build\` hands the C it writes to a C compiler, and \`zeno fetch\`
  gets packages with git. Everything else needs nothing.
architectures:
  - $1
base: core24
confinement: classic
grade: stable
apps:
  zeno:
    command: bin/zeno
EOF
    snap pack "$root" "$out" >/dev/null
    say "made zeno_${version}_$1.snap"
}

if have snap; then
    snap_of amd64 x86_64
    snap_of arm64 arm64
    snap_of armhf armv7
else
    say "no .snap: this machine has no snap"
fi

# --------------------------------------------------------------- flatpak
#
# A flatpak runs in a box of its own, on top of a "runtime": a set of
# files that stands in for the system. The runtime here is the one
# that programs are built on, the Sdk, because it has gcc and git in
# it. So `zeno build` and `zeno fetch` work inside the box, where the
# machine's own compiler cannot be seen. The box is given the home
# folder, where programs are kept, and the network, for `zeno fetch`.

runtime=25.08
flatpak_of() {
    from=$(unpacked "$2") || { say "no .flatpak for $1: the release has no archive for linux-$2"; return 0; }
    root=$work/flatpak-$1
    mkdir -p "$root/files/bin" "$root/files/share/zeno" "$root/files/share/doc/zeno" "$root/export"
    cp "$from/zeno" "$root/files/bin/zeno"
    if [ -f "$from/zeno.wasm" ]; then cp "$from/zeno.wasm" "$root/files/bin/zeno.wasm"; fi
    cp -r "$from/examples" "$from/editors" "$root/files/share/zeno/"
    cp -r "$from/README.md" "$from/docs" "$root/files/share/doc/zeno/"
    cat > "$root/metadata" <<EOF
[Application]
name=org.zeno_lang.Zeno
runtime=org.freedesktop.Sdk/$1/$runtime
sdk=org.freedesktop.Sdk/$1/$runtime
command=zeno

[Context]
shared=network;
filesystems=home;
EOF
    flatpak build-export --arch="$1" "$work/flatpak-repo" "$root" stable >/dev/null
    flatpak build-bundle --arch="$1" \
        --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
        "$work/flatpak-repo" "$out/zeno-$version-$1.flatpak" org.zeno_lang.Zeno stable
    say "made zeno-$version-$1.flatpak"
}

if have flatpak; then
    flatpak_of x86_64 x86_64
    flatpak_of aarch64 arm64
else
    say "no .flatpak: this machine has no flatpak"
fi

# -------------------------------------------------------------- Homebrew
#
# A formula is a small Ruby file: where to download from for each kind
# of machine, what the download's checksum must be, and what to do
# with what is in it. Homebrew reads formulas from a repository (a
# "tap"), so this one is kept in this repository, in `Formula`.

recipes=$out/recipes
mkdir -p "$recipes/Formula" "$recipes/packaging/arch" "$recipes/packaging/winget"

# `brew_for <os> <processor>`: the two lines of the formula for one
# kind of machine, if the release has an archive for it.
brew_for() {
    archive=zeno-$version-$1-$2.tar.gz
    if checksum=$(sum "$archive"); then
        say "      url \"$url/$archive\""
        say "      sha256 \"$checksum\""
    else
        say "      odie \"There is no Zeno $version for this kind of machine.\""
    fi
}

cat > "$recipes/Formula/zeno.rb" <<EOF
# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
#     brew install partham20/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "$site"
  version "$version"

  on_macos do
    on_arm do
$(brew_for macos arm64)
    end
    on_intel do
$(brew_for macos x86_64)
    end
  end

  on_linux do
    on_arm do
$(brew_for linux arm64)
    end
    on_intel do
$(brew_for linux x86_64)
    end
  end

  def install
    bin.install "zeno"
    # Zeno built for a browser, which \`zeno build --web\` looks for
    # beside \`zeno\`.
    bin.install "zeno.wasm" if File.exist?("zeno.wasm")
    doc.install "README.md", "docs"
    pkgshare.install "examples", "editors"
  end

  def caveats
    <<~EOS
      \`zeno build\` hands the C it writes to a C compiler, and \`zeno fetch\`
      gets packages with git. On macOS both come with Apple's command
      line tools: xcode-select --install
      The examples are in #{opt_pkgshare}/examples
    EOS
  end

  test do
    assert_equal "zeno #{version}", shell_output("#{bin}/zeno --version").strip
    assert_equal "Hello, Zeno!", shell_output("#{bin}/zeno run #{pkgshare}/examples/hello.zeno").strip
  end
end
EOF
say "wrote recipes/Formula/zeno.rb"

# ---------------------------------------------------------------- pacman
#
# A PKGBUILD is a shell script that `makepkg` runs to make a package:
# where the files come from, their checksums, and a function,
# `package`, that lays them out. The name ends in `-bin` because it
# packs a program that is already built, which is what such a package
# is called on Arch. pacman has nothing like apt's `Recommends`, so
# the compiler and git are named as optional and are not installed.

{
    cat <<EOF
# Zeno, for pacman. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     makepkg -si
#
# Maintainer: $maintainer
pkgname=zeno-bin
pkgver=$version
pkgrel=1
pkgdesc="$summary: checks, runs, tests, formats and compiles Zeno programs"
url="$site"
# No licence has been chosen for Zeno yet.
license=('custom')
provides=('zeno')
conflicts=('zeno')
optdepends=('gcc: for \`zeno build\`'
            'clang: for \`zeno build --llvm\`'
            'git: for \`zeno fetch\`')
# The program is already built, and is not to be changed.
options=('!strip' '!debug')
EOF
    arches=
    sources=
    for pair in x86_64:x86_64 aarch64:arm64 armv7h:armv7 armv6h:armv6 i686:x86; do
        arch=${pair%%:*}
        archive=zeno-$version-linux-${pair##*:}.tar.gz
        if checksum=$(sum "$archive"); then
            arches="$arches '$arch'"
            sources="$sources
source_$arch=(\"$url/$archive\")
sha256sums_$arch=('$checksum')"
        fi
    done
    say "arch=(${arches# })"
    say "${sources#?}"
    cat <<'EOF'

package() {
    cd "$srcdir"/zeno-$pkgver-linux-*
    install -Dm755 zeno "$pkgdir/usr/lib/zeno/zeno"
    # Zeno built for a browser, which `zeno build --web` looks for
    # beside `zeno`.
    if [ -f zeno.wasm ]; then
        install -Dm644 zeno.wasm "$pkgdir/usr/lib/zeno/zeno.wasm"
    fi
    install -d "$pkgdir/usr/bin" "$pkgdir/usr/share/zeno" "$pkgdir/usr/share/doc/zeno"
    ln -s ../lib/zeno/zeno "$pkgdir/usr/bin/zeno"
    cp -r examples editors "$pkgdir/usr/share/zeno/"
    cp -r README.md docs "$pkgdir/usr/share/doc/zeno/"
}
EOF
} > "$recipes/packaging/arch/PKGBUILD"
say "wrote recipes/packaging/arch/PKGBUILD"

# The package itself is made by `makepkg`, which only Arch has. Where
# there is docker, Arch is run in a container for it. `makepkg` uses
# an archive that lies beside the PKGBUILD and does not download it,
# and it will not run as root, so a user is made for it.
if have docker && docker info >/dev/null 2>&1 && [ -f "$files/zeno-$version-linux-x86_64.tar.gz" ]; then
    mkdir "$work/arch"
    cp "$recipes/packaging/arch/PKGBUILD" "$files/zeno-$version-linux-x86_64.tar.gz" "$work/arch"
    chmod -R a+rwX "$work/arch"
    docker run --rm -v "$work/arch:/made" archlinux:latest sh -c '
        set -e
        pacman -Sy --noconfirm --needed fakeroot >/dev/null
        useradd -m maker
        cp -r /made /home/maker/zeno && chown -R maker /home/maker/zeno
        su maker -c "cd /home/maker/zeno && makepkg" >/dev/null
        cp /home/maker/zeno/*.pkg.tar.zst /made
    '
    cp "$work/arch"/*.pkg.tar.zst "$out"
    say "made zeno-bin-$version-1-x86_64.pkg.tar.zst"
else
    say "no pacman package: this machine has no docker to run Arch in"
fi

# ---------------------------------------------------------------- winget
#
# winget installs a program by running its setup program without
# showing it. What it needs to know is in three small files: which
# version this is, where the setup program is and its checksum, and
# what to show somebody who searches for it. The setup program is one
# file for three processors, so it is named three times.

setup=zeno-$version-windows-setup.exe
if checksum=$(sum "$setup"); then
    id=ZenoLang.Zeno
    checksum=$(say "$checksum" | tr a-f A-F)
    cat > "$recipes/packaging/winget/$id.yaml" <<EOF
# Written by tools/package.sh when a version is released.
PackageIdentifier: $id
PackageVersion: $version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
EOF
    {
        cat <<EOF
# Written by tools/package.sh when a version is released.
PackageIdentifier: $id
PackageVersion: $version
InstallerType: inno
Scope: user
InstallModes:
  - interactive
  - silent
  - silentWithProgress
UpgradeBehavior: install
# Run without being shown, the setup program must not start a winget of
# its own to install a compiler and Git: winget is already running.
InstallerSwitches:
  Custom: /MERGETASKS="!compiler,!git"
Commands:
  - zeno
Installers:
EOF
        for processor in x64 x86 arm64; do
            cat <<EOF
  - Architecture: $processor
    InstallerUrl: $url/$setup
    InstallerSha256: $checksum
EOF
        done
        cat <<EOF
ManifestType: installer
ManifestVersion: 1.6.0
EOF
    } > "$recipes/packaging/winget/$id.installer.yaml"
    cat > "$recipes/packaging/winget/$id.locale.en-US.yaml" <<EOF
# Written by tools/package.sh when a version is released.
PackageIdentifier: $id
PackageVersion: $version
PackageLocale: en-US
Publisher: The Zeno language
PublisherUrl: $site
PackageName: Zeno
PackageUrl: $site
# No licence has been chosen for Zeno yet.
License: All rights reserved
ShortDescription: $summary
Description: $about
Moniker: zeno
Tags:
  - compiler
  - firmware
  - language
  - programming-language
ManifestType: defaultLocale
ManifestVersion: 1.6.0
EOF
    say "wrote recipes/packaging/winget, three files"
else
    rmdir "$recipes/packaging/winget"
    say "no winget files: the release has no $setup"
fi
