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
#
# Three more try the places a package manager finds Zeno in by itself,
# which the `stores` workflow runs:
#
#     tools/try-packages.sh apt 0.2.0 lists files
#     tools/try-packages.sh apt 0.2.0 lists https://github.com/partham20/zeno-lang/releases/latest/download/
#     tools/try-packages.sh tap 0.2.0
#     tools/try-packages.sh aur 0.2.0 packaging/arch
#
# `apt` is given the folder tools/apt-repo.sh wrote into, and either
# the folder of the .deb files, which it then hands out from this
# machine as a release would, or the address of a release that has the
# lists already. `tap` is the two lines of Homebrew that somebody
# types, against the repository itself. `aur` builds the package from
# the PKGBUILD as somebody on Arch would, downloading from the release.

set -eu

[ $# -ge 3 ] || [ "${1:-}" = tap ] && [ $# -ge 2 ] || { echo "try-packages.sh: say the kind of package, the version, and the folder of packages" >&2; exit 1; }
kind=$1
version=$2
out=$(cd "${3:-.}" && pwd)

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
# What the `apt` kind starts and leaves behind, to be taken away again.
server=
served=
tidy() {
    rm -rf "$page" $served
    if [ -n "$server" ]; then kill "$server" 2>/dev/null || true; fi
    $as_root rm -f /etc/apt/sources.list.d/zeno-try.list /usr/share/keyrings/zeno-try.asc
}
trap tidy EXIT

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
        package=zeno-lang-bin-$version-1-x86_64.pkg.tar.zst
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
            pacman -R --noconfirm zeno-lang-bin >/dev/null
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

    apt)
        [ $# = 4 ] || stop "for apt, also say the folder of .deb files, or the address of a release"
        case $4 in
            https://*)
                place=$4
                curl -fsSL "${place%/}/zeno-apt-key.asc" | $as_root tee /usr/share/keyrings/zeno-try.asc >/dev/null
                ;;
            *)
                # The lists and the .deb files in one folder, as they
                # are in a release, handed out from this machine.
                served=$(mktemp -d)
                cp "$out/InRelease" "$out/Packages" "$out/Packages.gz" "$4"/zeno_*.deb "$served"
                (cd "$served" && exec python3 -m http.server 8734 --bind 127.0.0.1 >/dev/null 2>&1) &
                server=$!
                sleep 1
                place=http://127.0.0.1:8734/
                $as_root cp "$out/zeno-apt-key.asc" /usr/share/keyrings/zeno-try.asc
                ;;
        esac
        $as_root chmod 644 /usr/share/keyrings/zeno-try.asc
        # The line that tells apt of the place, and of the one key that
        # may sign for it.
        say "deb [signed-by=/usr/share/keyrings/zeno-try.asc] $place /" |
            $as_root tee /etc/apt/sources.list.d/zeno-try.list >/dev/null
        # Only this one place is read again, and what is known of the
        # others is kept.
        $as_root apt-get update -q \
            -o Dir::Etc::sourcelist=sources.list.d/zeno-try.list \
            -o Dir::Etc::sourceparts=- -o APT::Get::List-Cleanup=0 >/dev/null ||
            stop "apt could not read the lists at $place"
        apt-cache policy zeno | grep -q "$version" || stop "apt does not know of zeno $version at $place"
        $as_root apt-get install -y --no-install-recommends zeno >/dev/null
        [ "$(command -v zeno)" = /usr/bin/zeno ] || stop "zeno is not at /usr/bin/zeno"
        runs /usr/share/zeno/examples zeno
        $as_root apt-get remove -y zeno >/dev/null
        if [ -e /usr/bin/zeno ] || [ -e /usr/lib/zeno ]; then stop "after removing it, Zeno is still there"; fi
        say "  apt found it at $place, checked the signature, and removed it again"
        ;;

    tap)
        export HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ENV_HINTS=1
        brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang >/dev/null
        brew install partham20/zeno-lang/zeno >/dev/null
        runs "$(brew --prefix)/share/zeno/examples" "$(brew --prefix)/bin/zeno"
        brew test partham20/zeno-lang/zeno >/dev/null
        brew uninstall partham20/zeno-lang/zeno >/dev/null
        if [ -e "$(brew --prefix)/bin/zeno" ]; then stop "after removing it, Zeno is still there"; fi
        brew untap partham20/zeno-lang >/dev/null
        say "  removed: nothing is left"
        ;;

    aur)
        [ -f "$out/PKGBUILD" ] && [ -f "$out/.SRCINFO" ] || stop "there is no PKGBUILD and .SRCINFO in $out"
        grep -qx "pkgver=$version" "$out/PKGBUILD" || stop "the PKGBUILD in $out is not for $version"
        # In Arch, in a container. `makepkg` will not run as root, so a
        # user is made for it. `.SRCINFO` is what the AUR reads in
        # place of the PKGBUILD, so it must say the same.
        docker run --rm -v "$out:/recipe:ro" archlinux:latest sh -c "
            set -e
            pacman -Sy --noconfirm --needed fakeroot diffutils >/dev/null
            useradd -m maker
            cp -r /recipe /home/maker/zeno && chown -R maker /home/maker/zeno
            cd /home/maker/zeno
            su maker -c 'makepkg --printsrcinfo' | diff - .SRCINFO
            su maker -c makepkg >/dev/null
            pacman -U --noconfirm zeno-lang-bin-$version-1-x86_64.pkg.tar.zst >/dev/null
            [ \"\$(zeno --version)\" = 'zeno $version' ]
            [ \"\$(zeno run /usr/share/zeno/examples/hello.zeno)\" = 'Hello, Zeno!' ]
            pacman -R --noconfirm zeno-lang-bin >/dev/null
        " || stop "the PKGBUILD did not make a package from the release, or .SRCINFO says something else"
        say "  .SRCINFO says what the PKGBUILD says, and it makes zeno $version from the release"
        ;;

    *) stop "there is no kind of package \`$kind\`. The kinds are deb, snap, flatpak, pacman, brew, apt, tap and aur" ;;
esac
say "ok: $kind"
