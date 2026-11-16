#!/bin/sh
# Tries one kind of package that tools/package.sh made: installs it as
# somebody would, runs the `zeno` that was installed, and takes it off
# again.
#
#     tools/try-packages.sh deb 0.2.0 out
#     tools/try-packages.sh snap 0.2.0 out
#     tools/try-packages.sh flatpak 0.2.0 out
#     tools/try-packages.sh pacman 0.2.0 out
#     tools/try-packages.sh brew 0.2.0 out files
#
# `out` is the folder package.sh wrote into. Homebrew downloads what
# its formula names, so it is also told where the files of the release
# are: the formula is tried with those in place of the release.
#
# Each needs its system: apt on Debian or Ubuntu, snap and flatpak
# where they are installed, docker (to run Arch in) for pacman, and
# Homebrew. The release workflow runs all five.

set -eu

[ $# -ge 3 ] || { echo "try-packages.sh: say the kind of package, the version, and the folder of packages" >&2; exit 1; }
kind=$1
version=$2
out=$(cd "$3" && pwd)

say() { printf '%s\n' "$*"; }
stop() { printf 'try-packages.sh: %s\n' "$*" >&2; exit 1; }
as_root=
if [ "$(id -u)" != 0 ]; then as_root=sudo; fi

# `runs <folder of examples> <zeno...>`: the checks that are the same
# for every kind. The command may be several words.
runs() {
    examples=$1
    shift
    said=$("$@" --version)
    [ "$said" = "zeno $version" ] || stop "it installed \`$said\`, and not zeno $version"
    hello=$("$@" run "$examples/hello.zeno")
    [ "$hello" = "Hello, Zeno!" ] || stop "hello.zeno printed: $hello"
    # A program in a web page came with 0.2.0: `zeno.wasm` must have
    # been installed where `zeno` looks for it.
    case $version in
        0.1.*) say "  it is zeno $version, and runs a program" ;;
        *)
            (cd "$page" && "$@" build --web "$examples/todo.zeno" >/dev/null && [ -f todo.web.html ]) ||
                stop "\`zeno build --web\` wrote no page"
            say "  it is zeno $version, runs a program and builds a page"
            ;;
    esac
}

# Where a page is built. It is in the home folder because a flatpak
# sees that and little else.
page=$(mktemp -d "$HOME/zeno-try.XXXXXX")
trap 'rm -rf "$page"' EXIT

case $kind in
    deb)
        deb=$out/zeno_${version}_$(dpkg --print-architecture).deb
        [ -f "$deb" ] || stop "there is no $deb"
        $as_root apt-get install -y "$deb" >/dev/null
        [ "$(command -v zeno)" = /usr/bin/zeno ] || stop "zeno is not at /usr/bin/zeno"
        runs /usr/share/zeno/examples zeno
        # The licence goes with it, from 0.2.0, where Debian keeps one.
        case $version in
            0.1.*) ;;
            *) [ -f /usr/share/doc/zeno/copyright ] || stop "the package has no licence in it" ;;
        esac
        $as_root apt-get remove -y zeno >/dev/null
        if [ -e /usr/bin/zeno ] || [ -e /usr/lib/zeno ]; then stop "after removing it, Zeno is still there"; fi
        say "  removed: nothing is left"
        ;;

    snap)
        case $(dpkg --print-architecture) in
            amd64 | arm64 | armhf) ;;
            *) stop "there is no snap for this processor" ;;
        esac
        snap=$out/zeno_${version}_$(dpkg --print-architecture).snap
        [ -f "$snap" ] || stop "there is no $snap"
        # `--dangerous`: it comes from a file and not from the store, so
        # nobody has signed it.
        $as_root snap install --dangerous --classic "$snap" >/dev/null
        runs /snap/zeno/current/examples /snap/bin/zeno
        $as_root snap remove zeno >/dev/null
        if [ -e /snap/bin/zeno ]; then stop "after removing it, Zeno is still there"; fi
        say "  removed: nothing is left"
        ;;

    flatpak)
        flatpak=$out/zeno-$version-$(uname -m).flatpak
        [ -f "$flatpak" ] || stop "there is no $flatpak"
        # The runtime comes from Flathub. The file says so itself, but
        # only asks, and nobody is here to answer.
        flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
        flatpak install --user --noninteractive -y "$flatpak" >/dev/null
        runs /app/share/zeno/examples flatpak run org.zeno_lang.Zeno
        # The box has a C compiler and git in it, so a program compiles.
        (
            cd "$page"
            flatpak run org.zeno_lang.Zeno build /app/share/zeno/examples/hello.zeno >/dev/null
            [ "$(./hello)" = "Hello, Zeno!" ]
        ) || stop "\`zeno build\` did not compile a program inside the flatpak"
        flatpak run --command=git org.zeno_lang.Zeno --version >/dev/null || stop "there is no git inside the flatpak"
        say "  it compiles a program, with the compiler in the runtime"
        flatpak uninstall --user --noninteractive -y org.zeno_lang.Zeno >/dev/null
        if flatpak info --user org.zeno_lang.Zeno >/dev/null 2>&1; then stop "after removing it, Zeno is still there"; fi
        say "  removed: nothing is left"
        ;;

    pacman)
        package=zeno-bin-$version-1-x86_64.pkg.tar.zst
        [ -f "$out/$package" ] || stop "there is no $out/$package"
        # In Arch, in a container. It is root there, so no sudo.
        case $version in 0.1.*) web=no ;; *) web=yes ;; esac
        docker run --rm -v "$out:/out:ro" archlinux:latest sh -c "
            set -e
            pacman -U --noconfirm /out/$package >/dev/null
            [ \"\$(zeno --version)\" = 'zeno $version' ]
            [ \"\$(zeno run /usr/share/zeno/examples/hello.zeno)\" = 'Hello, Zeno!' ]
            if [ $web = yes ]; then
                cd /tmp && zeno build --web /usr/share/zeno/examples/todo.zeno >/dev/null && [ -f todo.web.html ]
            fi
            pacman -R --noconfirm zeno-bin >/dev/null
            [ ! -e /usr/bin/zeno ] && [ ! -e /usr/lib/zeno ]
        " || stop "the package for pacman did not install, run and come off again"
        say "  it is zeno $version, it runs a program, and it comes off again"
        ;;

    brew)
        [ $# = 4 ] || stop "for brew, also say the folder of the release's files"
        files=$(cd "$4" && pwd)
        formula=$out/recipes/Formula/zeno.rb
        [ -f "$formula" ] || stop "there is no $formula"
        # A tap of its own, with the formula downloading from the folder.
        export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ENV_HINTS=1
        brew tap-new --no-git zeno/try >/dev/null
        tap=$(brew --repository zeno/try)
        mkdir -p "$tap/Formula"
        sed "s|https://github.com/partham20/zeno-lang/releases/download/v$version|file://$files|" \
            "$formula" > "$tap/Formula/zeno.rb"
        brew install zeno/try/zeno >/dev/null
        runs "$(brew --prefix)/share/zeno/examples" "$(brew --prefix)/bin/zeno"
        # The formula's own test.
        brew test zeno/try/zeno >/dev/null
        brew uninstall zeno/try/zeno >/dev/null
        if [ -e "$(brew --prefix)/bin/zeno" ]; then stop "after removing it, Zeno is still there"; fi
        brew untap zeno/try >/dev/null
        say "  removed: nothing is left"
        ;;

    *) stop "there is no kind of package \`$kind\`. The kinds are deb, snap, flatpak, pacman and brew" ;;
esac
say "ok: $kind"
