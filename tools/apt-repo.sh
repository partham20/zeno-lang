#!/bin/sh
# Makes what apt reads to find Zeno in a release: the list of its .deb
# files, and that list signed.
#
#     APT_SIGNING_KEY="$(cat key.asc)" tools/apt-repo.sh <folder with the .deb files> <out>
#
# apt takes a package from a place only when two things are there. One
# is a list, `Packages`, which says for each .deb what it is, how big
# it is and its checksum. The other is `InRelease`: the checksum of
# that list, signed with a key that the machine was told to trust. So
# a .deb that was changed on the way does not match the list, and a
# list that was changed does not match its signature.
#
# The files this writes go into the release beside the .deb files
# themselves, and that is the whole repository: apt is given the place
# where the newest release's files are downloaded from, and finds
# `InRelease`, `Packages.gz` and the .deb there. apt calls that a flat
# repository. docs/packages.md has the lines somebody types.
#
#     Packages, Packages.gz    the list, plain and packed
#     InRelease                the checksums of the list, signed
#     zeno-apt-key.asc         the public half of the key, which a
#                              machine is given once
#
# APT_SIGNING_KEY is the private half of the key, as `gpg --armor
# --export-secret-keys` prints it, with no passphrase. It is a secret
# of the repository and is never written to a file that is kept.

set -eu

[ $# = 2 ] || { echo "apt-repo.sh: say the folder with the .deb files, and where to write" >&2; exit 1; }
[ -n "${APT_SIGNING_KEY:-}" ] || { echo "apt-repo.sh: APT_SIGNING_KEY is not set, and a list that is not signed is of no use to apt" >&2; exit 1; }
debs=$(cd "$1" && pwd)
mkdir -p "$2"
out=$(cd "$2" && pwd)

say() { printf '%s\n' "$*"; }
ls "$debs"/zeno_*.deb >/dev/null 2>&1 || { echo "apt-repo.sh: there is no zeno_*.deb in $debs" >&2; exit 1; }

# The list: for each .deb what it says of itself, then where it is
# (beside the list), its size and its checksum.
: > "$out/Packages"
processors=
for deb in "$debs"/zeno_*.deb; do
    name=$(basename "$deb")
    dpkg-deb -f "$deb" >> "$out/Packages"
    {
        say "Filename: $name"
        say "Size: $(wc -c < "$deb" | tr -d ' ')"
        say "SHA256: $(sha256sum "$deb" | cut -d ' ' -f 1)"
        say
    } >> "$out/Packages"
    processors="$processors $(dpkg-deb -f "$deb" Architecture)"
done
# `-n` leaves the time out of the packed list, so the same list packs
# to the same file.
gzip -9 -n -c "$out/Packages" > "$out/Packages.gz"

# What is signed: the checksum and size of the list, in both forms.
line() { say " $(sha256sum "$out/$1" | cut -d ' ' -f 1) $(wc -c < "$out/$1" | tr -d ' ') $1"; }
{
    say "Origin: Zeno"
    say "Label: Zeno"
    say "Date: $(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')"
    say "Architectures:$processors"
    say "Description: The Zeno programming language"
    say "SHA256:"
    line Packages
    line Packages.gz
} > "$out/Release"

# The key is used from a folder that is thrown away afterwards.
keys=$(mktemp -d)
trap 'rm -rf "$keys"' EXIT
chmod 700 "$keys"
say "$APT_SIGNING_KEY" | gpg --homedir "$keys" --batch --quiet --import
gpg --homedir "$keys" --batch --yes --quiet --clearsign --output "$out/InRelease" "$out/Release"
gpg --homedir "$keys" --batch --yes --quiet --armor --export --output "$out/zeno-apt-key.asc"
# `Release` was only what was signed: `InRelease` has it inside.
rm "$out/Release"

say "wrote Packages, Packages.gz, InRelease and zeno-apt-key.asc, for:$processors"
