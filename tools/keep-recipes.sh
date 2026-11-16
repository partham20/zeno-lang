#!/bin/sh
# Puts the recipes that tools/package.sh wrote into this repository, on
# `main`, and pushes them: the Homebrew formula, the PKGBUILD and the
# files for winget.
#
#     tools/keep-recipes.sh out/recipes 0.2.0
#
# They are kept in the repository because that is where Homebrew reads
# a formula from, and they are written after a release and not before
# because they hold the checksums of its files. The release workflow
# runs this as its last step.

set -eu

[ $# = 2 ] || { echo "keep-recipes.sh: say the folder of recipes and the version" >&2; exit 1; }
recipes=$(cd "$1" && pwd)
version=$2

# The commit is Partha's, as every commit here is.
git config user.name "Partha"
git config user.email "123355647+partham20@users.noreply.github.com"

# A release is made from a tag, or from a commit that `main` may have
# moved on from. The recipes go on top of `main` as it is now. If
# somebody pushes in between, that is tried again.
for turn in 1 2 3 4; do
    git fetch -q origin main
    git checkout -q -B main FETCH_HEAD
    cp -r "$recipes/." .
    git add Formula packaging
    if git diff --cached --quiet; then
        echo "the recipes in the repository are already those of $version"
        exit 0
    fi
    git commit -q -m "Zeno $version for Homebrew, pacman and winget: its files and their checksums"
    if git push -q origin main; then
        echo "kept the recipes of $version in the repository"
        exit 0
    fi
    sleep $((turn * 3))
done
echo "keep-recipes.sh: could not push the recipes to main" >&2
exit 1
