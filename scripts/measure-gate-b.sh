#!/bin/sh
# Gate B measurements (PRD section 15). Builds the release and `measure` profiles, prepares a
# private HOME with a workspace of 5 projects x 2 panes, and reports start time, memory, CPU at
# idle, hook latency and echo latency. Nothing outside the temporary folder is touched.
#
#   scripts/measure-gate-b.sh
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
cargo build --release >/dev/null 2>&1
cargo build --profile measure >/dev/null 2>&1
WORK=$(mktemp -d "${TMPDIR:-/tmp}/tc-gate-b.XXXXXX")
DATA="$WORK/home/Library/Application Support/ternarycode"
mkdir -p "$DATA"
# Five project folders (copies are not needed: panes only open shells in them).
for i in 1 2 3 4 5; do mkdir -p "$WORK/projects/p$i"; done
python3 - "$WORK" "$DATA" <<'PY'
import json, sys
work, data = sys.argv[1], sys.argv[2]
def ws(n_projects, panes):
    projects, pid, next_id = [], 1, 100
    for i in range(n_projects):
        ids = list(range(next_id, next_id + panes)); next_id += panes
        layout = {"pane": ids[0]}
        for x in ids[1:]:
            layout = {"split": {"dir": "horizontal", "ratio": 0.5, "a": layout, "b": {"pane": x}}}
        projects.append({"id": pid, "name": f"p{i+1}", "path": f"{work}/projects/p{i+1}",
                         "tabs": [{"layout": layout, "focused": ids[0]}],
                         "panes": [{"id": x} for x in ids]})
        pid += 1
    return {"version": 1, "projects": projects, "active": 1, "next_id": next_id}
json.dump(ws(1, 1), open(f"{work}/ws-1x1.json", "w"))
json.dump(ws(5, 2), open(f"{work}/ws-5x2.json", "w"))
PY
run_release() { # $1 workspace file, $2 label
    cp "$1" "$DATA/workspace.json"
    HOME="$WORK/home" TERNARYCODE_STARTUP_LOG=1 "$ROOT/target/release/ternarycode" > "$WORK/out.log" 2>&1 &
    PID=$!
    sleep 4
    RSS=$(ps -o rss= -p $PID)
    CPU=$(top -l 3 -s 1 -pid $PID -stats cpu | tail -1)
    kill $PID; wait $PID 2>/dev/null || true
    echo "$2: $(grep -h 'first frame' "$WORK/out.log") | $(grep -h 'shells ready' "$WORK/out.log") | RSS $((RSS / 1024)) MB | CPU idle ${CPU}%"
}
echo "== release build ($(du -h target/release/ternarycode | cut -f1) binary)"
run_release "$WORK/ws-1x1.json" "1 project x 1 pane"
run_release "$WORK/ws-5x2.json" "5 projects x 2 panes"

echo "== measure build: scrollback, hook and echo latency"
cp "$WORK/ws-5x2.json" "$DATA/workspace.json"
STEPS="sleep 2500"
for i in 0 1 2 3 4; do
  STEPS="$STEPS;; action select $i;; sleep 400;; focus-pane 0;; sleep 300;; keys seq 1 2000\\r;; sleep 700;; focus-pane 1;; sleep 300;; keys seq 1 2000\\r;; sleep 700"
done
STEPS="$STEPS;; sleep 1000;; log-mem 5x2 with 2000 lines each;; action select 0;; sleep 500;; focus-pane 0;; sleep 400"
for i in 1 2 3 4 5; do
  STEPS="$STEPS;; hook-latency claude PermissionRequest needs-input;; hook-latency claude Stop idle|done;; hook-latency codex PermissionRequest needs-input;; hook-latency codex notify idle|done agent-turn-complete"
done
for i in 1 2 3 4 5 6 7 8 9 10; do STEPS="$STEPS;; echo-latency;; clear-line;; sleep 150"; done
STEPS="$STEPS;; quit"
TERNARYCODE_DATA_DIR="$DATA" HOME="$WORK/home" TERNARYCODE_DEV_SCRIPT="$STEPS" \
  perl -e 'alarm shift; exec @ARGV' 120 "$ROOT/target/measure/ternarycode" > "$WORK/measure.log" 2>&1 || true
grep -h "DEVTEST MEM" "$WORK/measure.log"
grep -h "DEVTEST FAIL" "$WORK/measure.log" || true
python3 - "$WORK/measure.log" <<'PY'
import re, sys, statistics
text = open(sys.argv[1]).read()
def stats(pattern, label):
    v = [float(x) for x in re.findall(pattern, text)]
    if v:
        v.sort()
        print(f"{label}: n={len(v)} median {statistics.median(v):.0f} ms, max {v[-1]:.0f} ms")
stats(r"PASS: hook-latency claude PermissionRequest[^\(]*\((\d+) ms", "hook -> status, Claude PermissionRequest")
stats(r"PASS: hook-latency claude Stop[^\(]*\((\d+) ms", "hook -> status, Claude Stop")
stats(r"PASS: hook-latency codex PermissionRequest[^\(]*\((\d+) ms", "hook -> status, Codex PermissionRequest")
stats(r"PASS: hook-latency codex notify[^\(]*\((\d+) ms", "hook -> status, Codex notify")
stats(r"PASS: wait-screen tc\d+ \((\d+) ms", "echo latency (write -> on screen)")
PY
echo "(logs in $WORK)"
