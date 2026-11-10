#!/usr/bin/env bash
# Installs Ubuntu packages on a CI runner: a mirror that hangs or fails is given up on after a few
# minutes and tried again, instead of holding the job until GitHub's six-hour limit.
#
#   scripts/ci-apt.sh zsh fish
set -uo pipefail

apt() { sudo timeout 300 apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=30 "$@"; }
for attempt in 1 2 3; do
    apt update -qq && apt install -y -qq "$@" >/dev/null && exit 0
    echo "apt-get failed (attempt $attempt of 3); trying again" >&2
    sleep 15
done
exit 1
