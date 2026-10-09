#!/usr/bin/env bash
set -euo pipefail
root=${1:?Pass an isolated checkout with instrumentation.patch applied and profile-build.jsonl present}
cd "$root"
root=$PWD
binary=$(python3 -c 'import json; rows=[json.loads(line) for line in open("profile-build.jsonl")]; print(next(row["executable"] for row in rows if row.get("reason")=="compiler-artifact" and row.get("executable") and row["target"]["name"]=="feuer_storage"))')
cp "$binary" benchmark
mkfifo cpu.fifo cpu.fifo.ack io.fifo io.fifo.ack
sudo -n perf record -a -o io.data --clockid mono --delay=-1 \
  -e io_uring:io_uring_submit_req -e io_uring:io_uring_complete \
  --control="fifo:$root/io.fifo,$root/io.fifo.ack" > io-record.log 2>&1 &
io_pid=$!
trap 'sudo -n kill -INT "$io_pid" 2>/dev/null || true; rm -f cpu.fifo cpu.fifo.ack io.fifo io.fifo.ack' EXIT
sudo -n perf record -o cpu.data -F 4999 -e cycles:uk --call-graph fp \
  --clockid mono --delay=-1 --control="fifo:$root/cpu.fifo,$root/cpu.fifo.ack" \
  -- sudo -n -u "$(id -un)" env TMPDIR=/mnt/local-ssd RECOVERY_100_GB=1 NUM_ITER_GROUP=100 \
    RECOVERY_PERF_FIFOS="$root/cpu.fifo,$root/io.fifo" "$root/benchmark" \
    disk_cache::recovery::benchmark::benchmark_recovery --exact --ignored --nocapture \
    > profile.log 2>&1
sudo -n kill -INT "$io_pid"
wait "$io_pid" || true
sudo -n chown "$(id -u):$(id -g)" cpu.data io.data
sudo -n perf report -f --stdio --no-inline --no-children -g none --percent-limit 0.1 \
  --sort symbol -i cpu.data > cpu-self.txt 2> report.log
sudo -n perf report -f --stdio --no-inline --children -g none --percent-limit 0.1 \
  --sort symbol -i cpu.data > cpu-children.txt 2>> report.log
sudo -n perf report -f --stdio --no-inline --no-children -g none --percent-limit 0.5 \
  --sort symbol,srcline -i cpu.data > cpu-lines.txt 2> lines-report.log
sudo -n perf script -f --ns -F trace:comm,pid,tid,time,event,trace -i io.data \
  > io-events.txt 2> io-script.log
