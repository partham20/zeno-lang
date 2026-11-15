#!/bin/sh
# Checks a built `zeno` before it is released.
#
#     tools/release-check.sh target/release/zeno
#     tools/release-check.sh path/to/zeno qemu-aarch64
#
# The first argument is the program to check. The second, if there is
# one, is a program that runs it: an emulator, for a `zeno` built for a
# processor this machine does not have. Run it from the top folder of
# the repository.
#
# It checks that `zeno` says the version Cargo.toml says, that every
# example runs and prints the same through the interpreter and the
# virtual machine, that the tests written in the examples pass, that
# every example is laid out as `zeno format` lays it out, and that
# `zeno new` makes a program that runs. If Zeno has also been built for
# a browser (`cargo build --release --target wasm32-unknown-unknown`),
# it checks that `zeno build --web` writes a page with the program in
# it. It needs no C compiler and no browser, because none of these do.

set -eu

zeno=$1
with=${2:-}
case $zeno in /*) ;; *) zeno=$PWD/$zeno ;; esac
run() { $with "$zeno" "$@"; }

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
said=$(run --version | tr -d '\r')
if [ "$said" != "zeno $version" ]; then
    echo "wrong version: \`zeno --version\` says \`$said\`, Cargo.toml says $version"
    exit 1
fi
echo "$said"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

hello=$(run run examples/hello.zeno | tr -d '\r')
if [ "$hello" != "Hello, Zeno!" ]; then
    echo "examples/hello.zeno printed \`$hello\`"
    exit 1
fi

# blink.zeno blinks until Ctrl-C, so it is the one example not run here.
count=0
for program in examples/*.zeno examples/*/main.zeno; do
    case $program in examples/blink.zeno) continue ;; esac
    run run "$program" </dev/null >"$work/interpreter.txt" 2>&1 || { echo "$program fails:"; cat "$work/interpreter.txt"; exit 1; }
    run run --vm "$program" </dev/null >"$work/vm.txt" 2>&1 || { echo "$program fails with --vm:"; cat "$work/vm.txt"; exit 1; }
    if ! cmp -s "$work/interpreter.txt" "$work/vm.txt"; then
        echo "$program prints one thing through the interpreter and another through the virtual machine"
        exit 1
    fi
    count=$((count + 1))
done
echo "$count examples run, and print the same through both engines"

run test examples >"$work/tests.txt" 2>&1 || { cat "$work/tests.txt"; exit 1; }
tail -1 "$work/tests.txt"

run format --check examples
echo "every example is laid out as \`zeno format\` lays it out"

(
    cd "$work"
    run new blinker >/dev/null
    cd blinker
    run run main.zeno >/dev/null
    run test >/dev/null
)
echo "\`zeno new\` makes a program that runs and whose tests pass"

wasm=$PWD/target/wasm32-unknown-unknown/release/zeno.wasm
if [ -f "$wasm" ]; then
    examples=$PWD/examples
    (
        cd "$work"
        ZENO_WASM=$wasm run build --web "$examples/todo.zeno" >/dev/null
        # The page that was written by hand, the program, and Zeno itself.
        grep -q '<form id="add">' todo.web.html
        grep -q 'Zeno.start(Zeno.bytes("AGFzbQ' todo.web.html
        grep -q '"main":"todo.zeno"' todo.web.html
    ) || { echo "\`zeno build --web\` did not write the page of examples/todo.zeno"; exit 1; }
    echo "\`zeno build --web\` writes a page with the program and Zeno in it"
fi
echo "ok"
