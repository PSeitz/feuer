#!/usr/bin/env bash
set -euo pipefail
umask 077
[[ $# == 6 ]] || { echo 'Usage: run-case.sh ROOT CASE_ID MODE MEMORY_COST DISK_COST HALF_LIFE' >&2; exit 2; }
root=$(cd "$1" && pwd -P)
case_id=$2 mode=$3 memory_cost=$4 disk_cost=$5 half_life=$6
[[ "$root" != /mnt/local-ssd && "$root" != /mnt/local-ssd/* ]]
[[ "$case_id" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]]
[[ "$mode" == range || "$mode" == full_objects ]]
[[ "$memory_cost" =~ ^[0-9]+([.][0-9]+)?$ && "$disk_cost" =~ ^[0-9]+([.][0-9]+)?$ ]]
[[ "$half_life" =~ ^[1-9][0-9]*$ ]]
mkdir -p "$root/results"
output="$root/results/$case_id"
mkdir "$output"
cache='' job='' monitors=() replay_status=not_started
cleanup() {
    local status=$?
    trap - EXIT
    if [[ -n "$job" ]]; then
        kill -- "-$job" 2>/dev/null || true
        wait "$job" 2>/dev/null || true
        replay_status=$status
    fi
    for pid in "${monitors[@]}"; do kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; done
    if [[ -n "$cache" ]]; then rm -rf -- "$cache" || true; fi
    printf '%s\n' "$replay_status" > "$output/exit-status.txt"
    printf '%s\n' "$status" > "$output/run-exit-status.txt"
    date -Is > "$output/end.txt"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# The host-wide lock excludes concurrent cases even when they use different roots.
exec 9>/tmp/feuer-adaptive-scoring.lock
flock -n 9
if pgrep -x 'replay|fio|direct_io|cargo|rustc' > "$output/conflicting-pids.txt"; then
    echo 'Refusing to run alongside existing benchmark or build processes' >&2
    exit 1
fi
[[ $(nproc) == 32 ]]
export FEUER_ACCESS_COUNT_HALF_LIFE=262144 FEUER_MAX_ACCESS_EVENTS_PER_KEY=64
export FEUER_MAX_ACCESS_AGE_ACCESSES=262144 FEUER_RECLAIM_SAMPLE_SIZE=256 FEUER_IDLE_BUFFER_POOL_PERCENT=7
unset FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES
memory=100000000000 disk=500000000000
jemalloc=/lib/aarch64-linux-gnu/libjemalloc.so.2
device=$(basename "$(findmnt -n -o SOURCE /mnt/local-ssd)")
[[ -e /sys/block/$device/stat ]]
available=$(df -B1 --output=avail /mnt/local-ssd | tail -1)
(( available >= disk + 32 * 1024**3 ))
cache=$(mktemp -d /mnt/local-ssd/full-trace-scoring-20261013T1745.XXXXXX)
printf '%s\n' "$device" > "$output/device-name.txt"
command=(env -u MALLOC_CONF LD_PRELOAD="$jemalloc" "$root/bin/replay"
    --input "$root/input/trace.tar.zst" --cache-dir "$cache/data" --output "$output"
    --memory-bytes "$memory" --disk-bytes "$disk"
    --simulate-s3 --memory-cost-per-request "$memory_cost" --disk-cost-per-request "$disk_cost"
    --access-count-half-life "$half_life")
if [[ "$mode" == full_objects ]]; then command+=(--full-objects); fi
printf '%q ' "${command[@]}" > "$output/command.txt"; printf '\n' >> "$output/command.txt"
env | LC_ALL=C sort | grep '^FEUER_' > "$output/environment.txt"
printf 'LD_PRELOAD=%s\nMALLOC_CONF=<unset>\n' "$jemalloc" >> "$output/environment.txt"
{
    printf 'case_id=%s\nmode=%s\nmemory_bytes=%s\ndisk_bytes=%s\ntrace_selection=full\nmemory_cost_per_request=%s\ndisk_cost_per_request=%s\naccess_count_half_life=%s\ncost_per_requested_byte=1\nscheduling=task_per_arrival\nsimulate_s3=1\ndevice=%s\n' "$case_id" "$mode" "$memory" "$disk" "$memory_cost" "$disk_cost" "$half_life" "$device"
    hostname; uname -a; lscpu
    if PATH="$HOME/.cargo/bin:$PATH" command -v rustc >/dev/null; then PATH="$HOME/.cargo/bin:$PATH" rustc -Vv; fi
    dpkg-query -W libjemalloc2
    sha256sum "$jemalloc" "$root/bin/replay" "$root/input/trace.tar.zst" "$0" "$root/report.py" "$root/report-metrics.py"
} > "$output/manifest.txt"
printf '%s  %s\n' a13aeae5c77da3709814fef42708d6cb55a867a553ba01dfc8c7e8338e7807f7 "$root/input/trace.tar.zst" | sha256sum -c - > "$output/input-check.txt"
printf '%s  %s\n' 4be4d5d412cef54eb70d4caa448707d60ca88b41599eca12a0fc67c7c501126a "$root/bin/replay" | sha256sum -c - > "$output/binary-check.txt"
date -Is > "$output/start.txt"
{ ps -eo pid,comm,pcpu,pmem --sort=-pcpu | head -20 || true; free -b; df -B1 /mnt/local-ssd; } > "$output/host-start.txt"
iostat -dx -y 1 3 > "$output/host-before-iostat.txt"
cp "/sys/block/$device/stat" "$output/device-before.txt"
iostat -x -y -t "$device" 1 > "$output/iostat.txt" & monitors=("$!")
setsid /usr/bin/time -v -o "$output/time.txt" "${command[@]}" > "$output/stdout.txt" 2> "$output/stderr.txt" & job=$!
while [[ ! -s "$output/pid.txt" ]] && kill -0 "$job" 2>/dev/null; do sleep 0.1; done
if [[ -s "$output/pid.txt" ]]; then
    read -r pid < "$output/pid.txt"
    pidstat -u -r -d -t -p "$pid" 1 > "$output/pidstat.txt" & monitors+=("$!")
fi
replay_status=0; wait "$job" || replay_status=$?; job=''
printf '%s\n' "$replay_status" > "$output/exit-status.txt"
cp "/sys/block/$device/stat" "$output/device-after.txt"
date -Is > "$output/replay-end.txt"
{ ps -eo pid,comm,pcpu,pmem --sort=-pcpu | head -20 || true; free -b; df -B1 /mnt/local-ssd; } > "$output/host-end.txt"
for pid in "${monitors[@]}"; do kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; done
monitors=()
if (( replay_status != 0 )); then exit "$replay_status"; fi
report_status=0
python3 "$root/report.py" "$output" "$case_id" "$mode" "$memory_cost" "$disk_cost" "$half_life" > "$output/report.json" 2> "$output/report-errors.txt" || report_status=$?
# Inventory all captured telemetry even when validation rejects the report.
inventory_status=0
python3 "$root/report-metrics.py" "$output" > "$output/metric-report.log" 2>&1 || inventory_status=$?
if (( report_status != 0 )); then exit "$report_status"; fi
exit "$inventory_status"
