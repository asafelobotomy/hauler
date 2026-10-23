#!/usr/bin/env bash
# Compare hauler with aria2c (and curl as a single-connection baseline)
# against the local throttled test server.
#
#   scripts/bench.sh [size-mib] [kib-per-sec-per-connection] [slow-every] [connections]
#
# The test server caps each connection's speed, like many real servers and
# CDNs, so the results show how well each tool spreads work across connections.
# Files are written to $BENCH_DIR (default ~/.cache/hauler-bench) and deleted
# after each run.
set -euo pipefail

SIZE_MIB=${1:-1024}
RATE_KIB=${2:-8192}
SLOW_EVERY=${3:-0}
CONNS=${4:-16}
PORT=${PORT:-18181}
DIR=${BENCH_DIR:-$HOME/.cache/hauler-bench}
URL="http://127.0.0.1:$PORT/file.bin"

cd "$(dirname "$0")/.."
cargo build --release -q -p hauler-cli -p hauler-testserver
mkdir -p "$DIR"
rm -f "$DIR"/file.bin*

server_args=(--listen "127.0.0.1:$PORT" --size-mib "$SIZE_MIB" --kib-per-sec "$RATE_KIB")
[[ $SLOW_EVERY -gt 0 ]] && server_args+=(--slow-every "$SLOW_EVERY")
target/release/hauler-testserver "${server_args[@]}" >/dev/null &
server=$!
trap 'kill $server 2>/dev/null; rm -f "$DIR"/file.bin*' EXIT
sleep 0.5

echo "file ${SIZE_MIB} MiB, ${RATE_KIB} KiB/s per connection, slow every ${SLOW_EVERY:-none}, ${CONNS} connections"
reference=""

run() {
    local name=$1; shift
    rm -f "$DIR"/file.bin*
    local start end
    start=$(date +%s.%N)
    "$@" >/dev/null 2>&1
    end=$(date +%s.%N)
    local secs mibs sum
    secs=$(echo "$end - $start" | bc)
    mibs=$(echo "$SIZE_MIB / $secs" | bc -l)
    sum=$(sha256sum "$DIR/file.bin" | cut -c1-16)
    [[ -z $reference ]] && reference=$sum
    local ok="ok"; [[ $sum != "$reference" ]] && ok="HASH MISMATCH"
    printf '%-8s %8.2f s %9.1f MiB/s  %s\n' "$name" "$secs" "$mibs" "$ok"
}

run hauler target/release/hauler get -q -x "$CONNS" -d "$DIR" "$URL"
run aria2c aria2c -q -x "$CONNS" -s "$CONNS" -k 1M --file-allocation=falloc --allow-overwrite=true -d "$DIR" "$URL"
if [[ ${CURL:-0} == 1 ]]; then
    run curl curl -s -o "$DIR/file.bin" "$URL"
fi
