#!/usr/bin/env bash
# Compare every example's outputs and renders, byte for byte, between a
# base commit and the working tree: the check that a change to evaluation
# (the interpreter, compiled code, solvers) keeps results the same bits.
#
#   scripts/compare_outputs.sh [BASE]        # BASE defaults to HEAD
#
# Builds the CLI at BASE (in a git worktree, its own target folder) and in
# the working tree, runs each example to t = 30 as CSV and JSON and renders
# it, and prints a DIFF or RDIFF line per difference, FAILED for a run
# that failed or was killed on either side, and a summary:
#   outputs same=47 diff=0 failed=1 renders same=24 diff=0
# Release builds of both take several GB of disk; they are removed at the
# end unless KEEP=1.
set -euo pipefail
cd "$(dirname "$0")/.."
base=${1:-HEAD}
work=${COMPARE_DIR:-target/compare}
mkdir -p "$work"
work=$(cd "$work" && pwd)
rm -rf "$work/base"
git worktree add -q --detach "$work/base" "$base"
trap 'git worktree remove --force "$work/base" 2>/dev/null || true' EXIT
(cd "$work/base" && CARGO_TARGET_DIR="$work/base-target" cargo build -q --release -p simsandbox-cli)
CARGO_TARGET_DIR="$work/new-target" cargo build -q --release -p simsandbox-cli
exe() { ls "$1"/release/simsandbox* | grep -v '\.d$' | head -1; }
old=$(exe "$work/base-target")
new=$(exe "$work/new-target")
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
  rm -rf "$work/base-target" "$work/new-target" "$work/rb" "$work/rn"
fi
