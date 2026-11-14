#!/usr/bin/env bash
# Compare every example's outputs and renders, byte for byte, between a
# base commit and the working tree: the check that a change to evaluation
# (the interpreter, compiled code, solvers) keeps results the same bits.
#
#   scripts/compare_outputs.sh [BASE]        # BASE defaults to HEAD
#
# Builds the CLI at BASE (in a git worktree) and in the working tree, into
# one target folder so the second build reuses the first's dependencies
# (51 s instead of 189 s, 0.7 GB in all; 2026-11-14), runs each example to t = 30 as CSV and JSON and renders
# it, and prints a DIFF or RDIFF line per difference, FAILED for a run
# that failed or was killed on either side, and a summary:
#   outputs same=48 diff=0 failed=0 renders same=24 diff=0
# The build folder is removed at the end unless KEEP=1.
set -euo pipefail
cd "$(dirname "$0")/.."
base=${1:-HEAD}
work=${COMPARE_DIR:-target/compare}
mkdir -p "$work"
work=$(cd "$work" && pwd)
rm -rf "$work/base"
git worktree add -q --detach "$work/base" "$base"
trap 'git worktree remove --force "$work/base" 2>/dev/null || true' EXIT
# Each build's binary copied out before the next replaces it.
exe() {
  for f in "$work/target/release/simsandbox" "$work/target/release/simsandbox.exe"; do
    if [ -f "$f" ]; then echo "$f"; return; fi
  done
  echo "no simsandbox binary in $work/target/release" >&2
  return 1
}
(cd "$work/base" && CARGO_TARGET_DIR="$work/target" cargo build -q --release -p simsandbox-cli)
old="$work/simsandbox-base"
cp "$(exe)" "$old"
CARGO_TARGET_DIR="$work/target" cargo build -q --release -p simsandbox-cli
new="$work/simsandbox-new"
cp "$(exe)" "$new"
rm -rf "$work/rb" "$work/rn"
mkdir -p "$work/rb" "$work/rn"
same=0 diff=0 failed=0 rs=0 rd=0
# One run's output digest, or FAILED and its exit status: a run that is
# killed (out of memory, say) or fails is reported, never compared.
digest() {
  local status=0 out
  out=$("$@" 2>&1 | md5sum; exit "${PIPESTATUS[0]}") || status=$?
  if [ "$status" -ne 0 ]; then echo "FAILED($status)"; else echo "$out"; fi
}
for f in examples/*.ssbx.json; do
  for fmt in csv json; do
    a=$(digest "$old" run "$f" --t-end 30 --format "$fmt")
    b=$(digest "$new" run "$f" --t-end 30 --format "$fmt")
    if [[ "$a" == FAILED* || "$b" == FAILED* ]]; then
      failed=$((failed + 1)); echo "FAILED $f $fmt: base $a, new $b"
    elif [ "$a" = "$b" ]; then same=$((same + 1)); else diff=$((diff + 1)); echo "DIFF $f $fmt"; fi
  done
  n=$(basename "$f" .ssbx.json)
  "$old" render "$f" --t-end 30 --out "$work/rb/$n.png" >/dev/null 2>&1 || true
  "$new" render "$f" --t-end 30 --out "$work/rn/$n.png" >/dev/null 2>&1 || true
  if cmp -s "$work/rb/$n.png" "$work/rn/$n.png"; then rs=$((rs + 1)); else rd=$((rd + 1)); echo "RDIFF $n"; fi
done
echo "outputs same=$same diff=$diff failed=$failed renders same=$rs diff=$rd"
if [ "${KEEP:-0}" != 1 ]; then
  rm -rf "$work/target" "$work/rb" "$work/rn" "$old" "$new"
fi
