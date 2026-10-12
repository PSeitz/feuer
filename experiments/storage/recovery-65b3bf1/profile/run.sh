#!/usr/bin/env bash
set -euo pipefail
root=/mnt/local-ssd/feuer-recovery-retest-65b3bf1/profile
for scenario in small medium large; do
  case "$scenario" in
    small) bytes=1024; entries=2097152; batch=4096;;
    medium) bytes=65536; entries=262144; batch=4096;;
    large) bytes=4194304; entries=16384; batch=128;;
  esac
  fixture="$root/$scenario-data"
  test ! -e "$fixture"
  args=(RECOVERY_ROOT="$fixture" RECOVERY_FORMAT=65b3bf1-profile
    RECOVERY_CAPACITY_GIB=128 RECOVERY_ENTRY_BYTES="$bytes"
    RECOVERY_ENTRIES="$entries" RECOVERY_BATCH_ENTRIES="$batch")
  env "${args[@]}" RECOVERY_SETUP=1 RECOVERY_ROUND=0 "$root/benchmark" \
    disk_cache::recovery::benchmark::benchmark_recovery --exact --ignored --nocapture \
    > "$root/$scenario-setup.log" 2>&1
  for round in 1 2 3; do
    prefix="$root/$scenario-$round"
    fifo="$prefix.fifo"
    mkfifo "$fifo" "$fifo.ack"
    timeout 120 sudo -n perf record -o "$prefix.data" -F 499 -e cpu-clock:uk \
      --call-graph fp --delay=-1 --control="fifo:$fifo,$fifo.ack" \
      -- sudo -n -u "$(id -un)" env "${args[@]}" RECOVERY_ROUND="$round" \
      RECOVERY_PERF_FIFO="$fifo" "$root/benchmark" \
      disk_cache::recovery::benchmark::benchmark_recovery --exact --ignored --nocapture \
      > "$prefix.log" 2>&1
    sudo -n chown "$(id -u):$(id -g)" "$prefix.data"
    rm "$fifo" "$fifo.ack"
    sudo -n perf report -f --stdio --no-inline --no-children -g none --percent-limit 0.5 \
      --sort symbol -i "$prefix.data" > "$prefix-self.txt" 2> "$prefix-report.log"
    sudo -n perf report -f --stdio --no-inline --children -g none --percent-limit 1 \
      --sort symbol -i "$prefix.data" > "$prefix-children.txt" 2>> "$prefix-report.log"
    grep -E 'RECOVERY,|test result|perf record:' "$prefix.log"
  done
  rm -r "$fixture"
done
