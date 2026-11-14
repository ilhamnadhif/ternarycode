#!/usr/bin/env bash
# What TernaryCode itself costs in a few fixed situations, to compare a change with the build
# before it: CPU time used, frames drawn and memory over a 20 s window, and the launch with 29
# git projects. Dev-only: the `measure` profile (release + the scripted UI steps), temporary
# projects, a private data folder.
#
#   scripts/perf.sh [label]     results also in $TMPDIR/tc-perf-<label>.txt
#   BIN=… HOOK=… scripts/perf.sh base    measures another build (e.g. an older commit's)
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
LABEL="${1:-run}"
if [ -z "${BIN:-}" ]; then
    cargo build --profile measure >/dev/null 2>&1 || { echo "build failed"; exit 1; }
    BIN="$ROOT/target/measure/ternarycode"
    HOOK="$ROOT/target/measure/ternarycode-hook"
fi
HOOK="${HOOK:-$(dirname "$BIN")/ternarycode-hook}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/tc-perf.XXXXXX")
OUT="${TMPDIR:-/tmp}/tc-perf-$LABEL.txt"
: > "$OUT"
WINDOW=20000

for i in $(seq 1 29); do
    d="$WORK/p$i"; mkdir -p "$d"
    (cd "$d" && git init -q -b main && printf 'x\n' > a.txt && git add -A && git -c user.name=t -c user.email=t@t commit -qm init)
done

add() { # projects 1..n, each given time for its shell
    local s=""; for i in $(seq 1 "$1"); do s="$s action add-project $WORK/p$i;; sleep 700;;"; done; printf '%s' "$s"
}
FILL='seq 1 400\r'
# A shell loop starts a program for every line (the foreground changes all the time); a log or
# an agent is one program printing.
STREAM='while :; do echo streaming line $RANDOM $RANDOM $RANDOM; sleep 0.02; done\r'
PRINTER="perl -e '\$|=1; while (1) { print qq(streaming line ), int(rand(1e9)), chr(10); select(undef, undef, undef, 0.02) }'\\r"
HOOKS="while :; do $HOOK claude PreToolUse {} ; $HOOK claude PostToolUse {} ; sleep 0.05; done\\r"
MEASURE="log perf-start;; log-mem start;; sleep $WINDOW;; log-mem end;; log perf-end;; quit"

secs() { ps -o time= -p "$1" 2>/dev/null | awk -F: '{ if (NF==3) print $1*3600+$2*60+$3; else print $1*60+$2 }'; }

# name, script: runs the app, measures between the perf-start and perf-end marks.
measure() {
    local name="$1" script="$2"
    local data="$WORK/data-$name" log="$WORK/$name.log"
    mkdir -p "$data"
    printf '{"theme":"dark","window":[60,40,1440,900],"check_updates":false}\n' > "$data/settings.json"
    TERNARYCODE_DATA_DIR="$data" TERNARYCODE_DEBUG_FRAMES=1 TERNARYCODE_DEV_SCRIPT="$script" \
        perl -e 'alarm shift; exec @ARGV' 120 "$BIN" > "$log" 2>&1 &
    local pid=$! t0 t1 n=0
    until grep -q "DEVTEST LOG: perf-start" "$log" 2>/dev/null; do
        sleep 0.2; n=$((n + 1)); [ $n -gt 500 ] && { echo "$name: no start mark"; kill $pid 2>/dev/null; return; }
    done
    t0=$(secs $pid)
    until grep -q "DEVTEST LOG: perf-end" "$log" 2>/dev/null; do sleep 0.1; done
    t1=$(secs $pid)
    wait $pid 2>/dev/null
    local frames mem
    frames=$(awk '/DEVTEST LOG: perf-start/{on=1; next} /DEVTEST LOG: perf-end/{on=0} on && /^frame at/{n++} END{print n+0}' "$log")
    mem=$(grep "DEVTEST MEM end" "$log" | sed 's/.*: \([0-9.]*\) MB.*/\1/')
    awk -v n="$name" -v a="$t0" -v b="$t1" -v f="$frames" -v m="$mem" -v w="$WINDOW" \
        'BEGIN { printf "%-34s CPU %5.1f%%   frames/s %5.1f   memory %s MB\n", n, (b-a)/(w/1000)*100, f/(w/1000), m }' | tee -a "$OUT"
}

# (a) Grid 3×3: one tile streams ~50 lines a second (one program), eight full but idle.
S="sleep 1500;; $(add 9) "
for i in $(seq 1 9); do S="$S action select-name p$i;; sleep 300;; keys $FILL;; sleep 300;;"; done
S="$S action select-name p1;; sleep 300;; keys $PRINTER;; action grid;; sleep 500;; action grid-columns 3;; sleep 3000;; $MEASURE"
measure "grid-3x3-one-streaming" "$S"

# (b) Focus on an idle project while nine others stream out of view: one program each, then a
# shell loop each.
for kind in printer loop; do
    if [ $kind = printer ]; then CMD="$PRINTER"; else CMD="$STREAM"; fi
    S="sleep 1500;; $(add 10) "
    for i in $(seq 2 10); do S="$S action select-name p$i;; sleep 300;; keys $CMD;; sleep 200;;"; done
    S="$S action select-name p1;; sleep 3000;; $MEASURE"
    measure "focus-idle-9-hidden-$kind" "$S"
done

# (c) Agent hooks out of view: ~40 hook messages a second (PreToolUse and PostToolUse).
S="sleep 1500;; $(add 2) action select-name p1;; sleep 300;; keys $HOOKS;; action select-name p2;; sleep 3000;; $MEASURE"
measure "focus-idle-hooks-40-per-s" "$S"

# (d) Launch with 29 git projects, Grid view: time to the first frame and CPU of the first 8 s.
D="$WORK/data-launch"; mkdir -p "$D"
printf '{"theme":"dark","window":[60,40,1440,900],"check_updates":false}\n' > "$D/settings.json"
TERNARYCODE_DATA_DIR="$D" TERNARYCODE_DEV_SCRIPT="sleep 1500;; $(add 29) action grid;; sleep 1500;; quit" \
    perl -e 'alarm shift; exec @ARGV' 120 "$BIN" > "$WORK/launch-setup.log" 2>&1
for run in 1 2 3; do
    # `time -p` writes its user/sys lines into the same log.
    /usr/bin/time -p env TERNARYCODE_DATA_DIR="$D" TERNARYCODE_STARTUP_LOG=1 TERNARYCODE_DEV_SCRIPT="sleep 8000;; quit" \
        perl -e 'alarm shift; exec @ARGV' 60 "$BIN" > "$WORK/launch-$run.log" 2>&1
    first=$(grep -m1 "first frame after" "$WORK/launch-$run.log" | sed 's/.*after \([0-9]*\) ms.*/\1/')
    cpu=$(awk '/^user|^sys/{s+=$2} END{print s}' "$WORK/launch-$run.log")
    printf '%-34s first frame %5s ms   CPU in 8 s %5.2f s\n' "launch-29-projects (run $run)" "$first" "$cpu" | tee -a "$OUT"
done
echo "results in $OUT (logs in $WORK)"
