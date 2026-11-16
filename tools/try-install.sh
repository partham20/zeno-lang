#!/bin/sh
# Tries tools/install.sh on this machine, without a release to download.
#
#     tools/try-install.sh target/release/zeno
#
# It packs the `zeno` it is given the way a release packs one, with
# `zeno.wasm` if that has been built, and has install.sh install it
# from that folder into a home folder made for the trial. Then it
# checks that Zeno is there, runs, is on the PATH of a new shell and
# is not added twice, that a download that was changed is refused, and
# that `--uninstall` leaves nothing behind. Run it from the top folder
# of the repository.
#
# The release workflow runs it on Linux and on macOS before a release.

set -eu

zeno=$1
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case $(uname -s) in Darwin) system=macos ;; *) system=linux ;; esac
case $(uname -m) in
    x86_64 | amd64) processor=x86_64 ;;
    aarch64 | arm64) processor=arm64 ;;
    *) echo "try-install.sh is for x86-64 and ARM64"; exit 1 ;;
esac
name=zeno-$version-$system-$processor

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail() { echo "try-install.sh: $*"; exit 1; }

# A release, in a folder.
mkdir -p "$work/release/$name" "$work/home"
cp "$zeno" "$work/release/$name/zeno"
cp -r README.md docs examples editors "$work/release/$name"
wasm=target/wasm32-unknown-unknown/release/zeno.wasm
if [ -f "$wasm" ]; then cp "$wasm" "$work/release/$name"; fi
(
    cd "$work/release"
    tar -czf "$name.tar.gz" "$name"
    rm -rf "$name"
    if command -v sha256sum >/dev/null; then sha256sum zeno-* > SHA256SUMS.txt; else shasum -a 256 zeno-* > SHA256SUMS.txt; fi
)

# Everything below happens in a home folder of its own.
install() { HOME="$work/home" SHELL=/bin/bash ZENO_FROM="$1" sh tools/install.sh --no-tools; }
case $system in macos) startup=$work/home/.bash_profile ;; *) startup=$work/home/.bashrc ;; esac

install "$work/release" > "$work/said.txt" || { cat "$work/said.txt"; fail "install.sh failed"; }
home=$work/home/.zeno
[ "$("$home/bin/zeno" --version)" = "zeno $version" ] || fail "the installed zeno does not say $version"
[ -f "$home/examples/hello.zeno" ] && [ -d "$home/docs" ] || fail "the docs and examples were not installed"
[ "$(HOME="$work/home" bash -c ". '$startup'; zeno run ~/.zeno/examples/hello.zeno")" = "Hello, Zeno!" ] ||
    fail "a new shell does not find zeno"
if [ -f "$wasm" ]; then
    (cd "$work" && "$home/bin/zeno" build --web "$home/examples/todo.zeno" >/dev/null && [ -f todo.web.html ]) ||
        fail "the installed zeno does not build a page"
fi
echo "installed: it runs, a new shell finds it$( [ -f "$wasm" ] && echo ', and it builds a page')"

install "$work/release" >/dev/null
[ "$(grep -c 'Zeno installer' "$startup")" = 1 ] || fail "installing twice added it to the PATH twice"
echo "installed again: the PATH has it once"

# One byte more, and it is not what was released.
mkdir "$work/changed"
cp "$work/release"/* "$work/changed"
echo >> "$work/changed/$name.tar.gz"
if install "$work/changed" > "$work/said.txt" 2>&1; then fail "a download that was changed was installed"; fi
grep -q "not what was released" "$work/said.txt" || { cat "$work/said.txt"; fail "a changed download was refused for another reason"; }
echo "a download that was changed is refused"

HOME="$work/home" SHELL=/bin/bash sh tools/install.sh --uninstall >/dev/null
[ ! -e "$home" ] || fail "after --uninstall, $home is still there"
if grep -q 'Zeno installer' "$startup"; then fail "after --uninstall, the PATH line is still there"; fi
echo "uninstalled: nothing is left"
echo "ok"
