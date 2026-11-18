#!/usr/bin/env bash
# Installs Ubuntu packages on a GitHub runner without waiting on a bad package mirror.
#
# The runner's apt asks azure.archive.ubuntu.com first, then archive.ubuntu.com and
# security.ubuntu.com (/etc/apt/apt-mirrors.txt). Some Azure nodes send at 40–75 kB/s without ever
# stopping, so apt's own timeout never moves on to the next mirror (actions/runner-images#14823),
# and a failover can leave apt-get update silent until the job's limit (#15040). Here the package
# lists and the download each get a time limit; when one runs past it or fails, the mirror apt
# tried first is dropped from the list and that part runs again. Files already fetched are kept.
# Arm runners get their packages from ports.ubuntu.com, which is not in the list: there a part is
# simply tried again.
#
#   scripts/ci-apt.sh zsh fish
set -uo pipefail

mirrors=/etc/apt/apt-mirrors.txt
log=$(mktemp)
trap 'rm -f "$log"' EXIT

# apt-get within $1 seconds; its output is folded in the job's log, with what it fetched and its
# errors printed below the fold.
apt() {
    local limit=$1; shift
    echo "::group::apt-get $* (up to ${limit}s)"
    sudo DEBIAN_FRONTEND=noninteractive timeout -k 10 "$limit" apt-get -q "$@" 2>&1 | tee "$log"
    local status=${PIPESTATUS[0]}
    echo "::endgroup::"
    grep -E '^(Fetched|E:|W:)' "$log"
    [ "$status" != 124 ] || echo "apt-get $1 ran past ${limit}s"
    return "$status"
}

# Drops the mirror apt tries first (the lowest priority number; one with none comes last) when
# the sources go through the list and another mirror is left in it.
drop_mirror() {
    grep -rqsF "mirror+file:$mirrors" /etc/apt/sources.list /etc/apt/sources.list.d || return 0
    [ "$(grep -c '^[^#[:space:]]' "$mirrors")" -gt 1 ] || return 0
    local first
    first=$(awk '/^[^#[:space:]]/ {
        p = 1e9; for (i = 2; i <= NF; i++) if ($i ~ /^priority:/) p = substr($i, 10) + 0
        if (best == "" || p < low) { best = $1; low = p }
    } END { print best }' "$mirrors")
    awk -v drop="$first" '$1 != drop' "$mirrors" > "$log" && sudo cp "$log" "$mirrors"
    echo "::warning::apt-get: leaving $first for $(awk '/^[^#[:space:]]/ { print $1 }' "$mirrors" | paste -sd, -)"
}

# apt-get up to three times, each within $1 seconds, with the next mirror after each failure.
retry() {
    local limit=$1; shift
    for attempt in 1 2 3; do
        apt "$limit" "$@" && return 0
        echo "apt-get $1 failed (attempt $attempt of 3)" >&2
        [ "$attempt" = 3 ] || { drop_mirror; sleep 5; }
    done
    return 1
}

# Lists and packages normally come down in seconds. The install itself needs no mirror.
retry 90 update &&
    retry 120 install -y --download-only "$@" &&
    apt 900 install -y --no-download "$@"
