#!/bin/sh
# Runs the test suite under a clock, for the `probe` workflow.
#
#     tools/probe-tests.sh <seconds> <name>
#
# If the suite ends in time, it says how. If it is still running when
# the clock ends, it says which tests had not ended, and for each
# program that a test compiled and is waiting for: which test it belongs
# to, and where it is (`sample` on macOS, gdb elsewhere). What was made
# of each such program, in C or for LLVM, is kept in `found/<name>/`.
# Then it stops everything, and fails.
set -u

limit=$1
name=$2
kept=$PWD/found/$name
mkdir -p "$kept"

cargo test > "$kept/tests.txt" 2>&1 &
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
      tail -n 30 "$folder/test.$kind" | cut -c1-160
    fi
  done
  echo "-- where it is"
  if command -v sample > /dev/null; then
    sample "$pid" 2 2>&1 | sed -n 1,60p | cut -c1-200
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
