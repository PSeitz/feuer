# Recovery in fresh processes — 2026-10-02

This supersedes the repeated-open benchmark for estimating process-startup recovery.
Same commit `88d90cbf005f5040024f7656cefea4bfa603ee8c`, SSD host
`m8g-32cpu-local-ssd`, release build, locked dependencies, 128-GiB capacity,
four Tokio workers, sizes/counts/batches, keys and payloads as in README.md.
No profiling or production changes.

Fixture creation and sync run in a separate process which exits. Each of five recoveries
then runs in its own fresh process against the same fixture. Timings cover cache open
through completion of all shard recovery, not process launch, verification or teardown.
Direct I/O; no OS cache flush or host reboot. These measure fresh-process recovery,
not cold-device recovery.

| Entry size | Entries | Median | Min–max | Linear estimate for 4 TiB occupied disk |
| --- | ---: | ---: | ---: | ---: |
| 1 KiB | 2,097,152 | 702.760 ms | 690.702–730.368 ms | 89.6 s |
| 64 KiB | 262,144 | 167.447 ms | 155.746–205.968 ms | 38.3 s |
| 4 MiB | 16,384 | 50.372 ms | 47.572–53.629 ms | 3.2 s |

All 15 recoveries passed: exact recovered counts, every key present, three sampled
payloads correct per recovery, zero read errors. Metadata I/O remains 128 / 64 / 64
reads and 128 / 64 / 64 MiB respectively. All three fixture directories were removed.

4-TiB estimates scale median wall time by `4,194,304 / occupied_chunks`, where occupied
1-MiB chunks are 32,896 / 18,325 / 65,600. They assume the same packing and hardware;
they are not measured at 4 TiB. Index-memory requirements, allocation behavior and
metadata I/O scaling could invalidate this extrapolation, especially for small entries.

## Artifacts and reproduction

- `startup.rs`: harness (setup and recovery are distinct invocations).
- `startup.csv`: all 15 measurements.
- `startup-{small,medium,large}-{1..5}.log`: individual recovery logs.
- `startup-*-setup.log`, `startup-build.log`: setup and build logs.

Use the isolated-source build instructions in README.md, injecting `startup.rs` instead
of `benchmark.rs`. For each scenario, set the size/count/batch environment as there,
and use a new `RECOVERY_ROOT` directory. Then run:

```sh
export RECOVERY_ROUND=0
RECOVERY_SETUP=1 "$binary" disk_cache::recovery::benchmark::benchmark_recovery \
  --exact --ignored --nocapture > "$root/startup-$scenario-setup.log" 2>&1 || exit 1
for round in 1 2 3 4 5; do
  export RECOVERY_ROUND=$round
  "$binary" disk_cache::recovery::benchmark::benchmark_recovery \
    --exact --ignored --nocapture > "$root/startup-$scenario-$round.log" 2>&1 || exit 1
done
```

Leave `RECOVERY_SETUP` unset for recovery invocations. Unlike the original harness,
this fixture persists; remove only the newly created fixture directory after successful
runs. The actual run removed each fixture before setting up the next workload.
