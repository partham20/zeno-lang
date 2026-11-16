#!/bin/sh
# Runs the test suite under a clock, for the `probe` workflow.
#
#     tools/probe-tests.sh <seconds> <name> [<tests>]
#
# `<tests>` is part of the name of the tests to run; without it, all.
# If the suite ends in time, it says how. If it is still running when
# the clock ends, it says which tests had not ended, and for each
# program that a test compiled and is waiting for: which test it belongs
# to, where it is (`sample` and lldb on macOS, gdb elsewhere), and what
# a runtime that the probe made talkative wrote about it in
# /tmp/zeno-probe.txt. What was made of each such program, in C or for
# LLVM, is kept in `found/<name>/`. Then it stops everything, and fails.
set -u

limit=$1
name=$2
kept=$PWD/found/$name
mkdir -p "$kept"

rm -f /tmp/zeno-probe.txt
cargo test ${3:-} > "$kept/tests.txt" 2>&1 &
suite=$!

waited=0
while kill -0 $suite 2> /dev/null && [ "$waited" -lt "$limit" ]; do
  sleep 5
  waited=$((waited + 5))
done

if ! kill -0 $suite 2> /dev/null; then
  status=0
  wait $suite || status=$?
  echo "the suite ended with $status after about $waited seconds"
  grep '^test result' "$kept/tests.txt"
  grep -E '^test .* (FAILED|ignored)' "$kept/tests.txt" | head -n 20
  if [ -f /tmp/zeno-probe.txt ]; then
    echo "the runtime wrote $(wc -l < /tmp/zeno-probe.txt) lines, by kind:"
    cut -d ' ' -f 3 /tmp/zeno-probe.txt | sort | uniq -c
    grep -E 'mprotect -1|blocked 1' /tmp/zeno-probe.txt | head -n 10
  fi
  exit $status
fi

echo "STILL RUNNING after $limit seconds"
echo
echo "== what the suite said last"
tail -n 5 "$kept/tests.txt" | cut -c1-200
echo
echo "== tests it says have run for over a minute"
grep 'has been running for over' "$kept/tests.txt" | cut -c1-200
echo
echo "== the folders of programs that tests compiled and have not removed"
for folder in "${TMPDIR:-/tmp}"/zeno-test-*/; do
  [ -d "$folder" ] && basename "$folder" | sed 's/^zeno-test-[0-9]*-//'
done
echo
echo "== processes"
ps -axo pid,ppid,etime,stat,command | grep -E 'zeno|/test( |$)|cargo' | grep -v grep | cut -c1-220

for pid in $(pgrep -x test); do
  echo
  echo "== program $pid"
  where=$(ps -o command= -p "$pid" | cut -d ' ' -f 1)
  echo "$where"
  folder=$(dirname "$where")
  test=$(basename "$folder" | sed 's/^zeno-test-[0-9]*-//')
  for kind in c ll; do
    if [ -f "$folder/test.$kind" ]; then
      cp "$folder/test.$kind" "$kept/$test.$kind"
      # The program as Zeno is not kept, but the end of what was made
      # of it says what it is.
      echo "-- the end of its test.$kind"
      tail -n 8 "$folder/test.$kind" | cut -c1-160
    fi
  done
  ls -l "$folder" | cut -c1-120
  if [ -f /tmp/zeno-probe.txt ]; then
    echo "-- what its runtime wrote: $(grep -c "^probe $pid " /tmp/zeno-probe.txt) lines, the first 12 and the last 12"
    grep "^probe $pid " /tmp/zeno-probe.txt | sed -n 1,12p | cut -c1-240
    echo "..."
    grep "^probe $pid " /tmp/zeno-probe.txt | tail -n 12 | cut -c1-240
  fi
  echo "-- where it is"
  if command -v sample > /dev/null; then
    sample "$pid" 2 2>&1 | sed -n '/Call graph/,/Binary Images/p' | cut -c1-200
    # The instruction it is at, what the registers hold, and whether
    # the memory they point at may be written. lldb gets 30 seconds.
    (sudo lldb -p "$pid" -b -o 'register read pc lr x0 x8 x9 x19 x20 x21 x22' -o 'disassemble --pc --count 3' \
      -o 'disassemble -n zeno_page_new' -o 'detach' 2>&1 | sed -n 1,110p | cut -c1-170) &
    looking=$!
    turns=0
    while kill -0 $looking 2> /dev/null && [ $turns -lt 30 ]; do sleep 1; turns=$((turns + 1)); done
    kill -9 $looking 2> /dev/null
    sudo pkill -9 lldb 2> /dev/null
    echo "-- its memory, the regions that may not be written"
    vmmap "$pid" 2> /dev/null | grep -E 'MALLOC' | grep -E ' r--/| ---/' | sed -n 1,20p | cut -c1-170
  else
    gdb -p "$pid" -batch -ex 'thread apply all bt' 2>&1 | tail -n 30 | cut -c1-200
  fi
done

# The test program itself: which of its threads wait, and for what.
for pid in $(pgrep -f 'debug/deps/zeno-'); do
  echo
  echo "== the tests themselves, $pid"
  if command -v sample > /dev/null; then
    sample "$pid" 1 2>&1 | grep -E 'Thread_|tests::' | cut -c1-200 | sed -n 1,60p
  fi
done

pkill -9 -x test
pkill -9 -f 'debug/deps/zeno-'
kill -9 $suite 2> /dev/null
exit 1
