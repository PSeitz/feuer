# SSD recovery rerun — 2026-10-02

**For startup recovery, use [the fresh-process benchmark](startup.md).** The results below
reuse one process and underestimate recovery in a fresh process.

Commit: `88d90cbf005f5040024f7656cefea4bfa603ee8c` (tracked source snapshot; untracked files excluded).
Host: `m8g-32cpu-local-ssd`, ARM64 Linux, local NVMe/ext4 at `/mnt/local-ssd`.
Release build, locked dependencies, real O_DIRECT/io_uring. No profiling or production changes.

## Results

Five full opens/recoveries per workload, 128 GiB configured capacity, four Tokio workers.

| Entry bytes / count | Saved `497301c` median | Current median | Speedup | Current first reopen | Current median process CPU time |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 KiB / 2,097,152 | 697.629 ms | **61.569 ms** | 11.33× | 584.735 ms | 791.954 ms |
| 64 KiB / 262,144 | 144.144 ms | **15.478 ms** | 9.31× | 98.573 ms | 108.699 ms |
| 4 MiB / 16,384 | 93.115 ms | **25.723 ms** | 3.62× | 34.771 ms | 230.054 ms |

All 15 recoveries passed: exact entry counts, every key present, three sampled payloads correct
per reopen, and zero read errors. Temporary fixture directories were removed.

The workloads retain the previous entry sizes/counts, batch sizes (4,096 / 4,096 / 128), keys
(hex strings hashed through the current key API), capacity, payload bytes (0x5a), and host.
Occupied chunks remain 32,896 / 18,325 / 65,600. Metadata I/O remains 128 / 64 / 64 reads,
128 / 64 / 64 MiB; process physical-read accounting adds a 4-KiB open probe.

The current implementation recovers shards in parallel. Process CPU time sums all threads:
it is not elapsed time or whole-host CPU. The first small-entry reopen consumed **16.738 CPU
seconds** in **584.735 ms** elapsed. Lower wall time does not imply lower CPU cost.

These are repeated reopens after creating and syncing fixtures, not cold boots. No OS cache
flush was performed. First reopens differ materially from later ones. Comparison uses saved
unprofiled baseline measurements, not fresh interleaved controls; code, format, key representation,
checksum, and recovery concurrency have changed. This measures their combined effect, not any
single optimization. The benchmark ran, not the full storage test suite.

## Artifacts

- `results.csv`: all 15 current measurements.
- `baseline-497301c.csv`: original measurements recovered from saved logs.
- `small.log`, `medium.log`, `large.log`, `build.log`: raw current logs.
- `benchmark.rs`: saved harness updated to the current `DiskCache` naming.
- Remote source, binary, logs and build output: `/mnt/local-ssd/feuer-recovery-rerun-88d90cb/`.

Fixture creation, final `sync_all`, all-key checks, sampled payload reads, and teardown are outside
recovery timing. Each timed `DiskCache::open` waits for every shard's recovery. Read counters
are captured before payload verification. The final sync is benchmark fixture preparation, not a
new production durability guarantee.

## Reproduce

The harnesses and commands use the current `DiskCache` and `disk_cache` names. Raw logs and
profiles retain the symbols emitted by the measured commits; measurements are unchanged.

Archive the commit into a new isolated directory and apply the workspace's naming-only changes
to that snapshot. Copy `benchmark.rs` to `feuer-storage/src/disk_cache/recovery/benchmark.rs`.
Append the following to that snapshot's `feuer-storage/src/disk_cache/recovery.rs`:

```rust
#[cfg(test)]
mod benchmark;
```

On the SSD host, with `root` pointing to a new artifact directory and the snapshot under `$root/source`:

```sh
export PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd
export CARGO_TARGET_DIR="$root/target"
cargo test --manifest-path "$root/source/Cargo.toml" --locked --release \
  -p feuer-storage --lib --no-run
binary=$(find "$root/target/release/deps" -maxdepth 1 \
  -name 'feuer_storage-*' -type f -executable)
for scenario in small medium large; do
  case "$scenario" in
    small) bytes=1024; entries=2097152; batch=4096;;
    medium) bytes=65536; entries=262144; batch=4096;;
    large) bytes=4194304; entries=16384; batch=128;;
  esac
  RECOVERY_ROOT="$root" RECOVERY_FORMAT=88d90cb RECOVERY_CAPACITY_GIB=128 \
  RECOVERY_ENTRY_BYTES=$bytes RECOVERY_ENTRIES=$entries \
  RECOVERY_BATCH_ENTRIES=$batch RECOVERY_ROUNDS=5 \
  "$binary" disk_cache::recovery::benchmark::benchmark_recovery \
    --exact --ignored --nocapture > "$root/$scenario.log" 2>&1 || exit 1
done
```

Check free SSD space and competing workloads first. Each fixture uses a sparse 128-GiB file;
the largest payload workload writes 64 GiB. Fixtures are automatically removed after each test.
