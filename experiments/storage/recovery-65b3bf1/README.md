# Fresh-process SSD recovery retest

Commit: `65b3bf1abfe0ee4c872aa4df14931e0f7a7732f3` (tracked source snapshot).
Host: `m8g-32cpu-local-ssd`. Same harness and parameters as
[the previous fresh-process run](../recovery-88d90cb/startup.md).
Release build, locked dependencies, direct I/O, no profiling or production changes.

Fixture creation and sync occur in a separate process that exits. Each of five recovery
runs starts a new process against the same fixture. Timing covers cache open through
completion of all shard recovery, excluding verification, process launch and teardown.
No OS cache flush or host reboot; this is not a cold-device benchmark.

| Entry size / count | Previous median | Retest median | Retest min–max | Median process CPU time |
| --- | ---: | ---: | ---: | ---: |
| 1 KiB / 2,097,152 | 702.760 ms | 598.312 ms | 565.619–635.690 ms | 16.235 s |
| 64 KiB / 262,144 | 167.447 ms | 97.478 ms | 95.641–101.432 ms | 2.088 s |
| 4 MiB / 16,384 | 50.372 ms | 34.498 ms | 34.208–36.882 ms | 0.319 s |

All 15 recoveries passed: exact entry counts, every key present, sampled payloads correct,
zero read errors. Fixture directories were removed after each workload. Metadata I/O
remains 128 / 64 / 64 reads and 128 / 64 / 64 MiB. Small round 4 recorded an additional
8 KiB in process physical-read accounting compared with the other small rounds.

CPU time sums all threads. Wall time decreased relative to the saved previous run,
but CPU time increased. These are separate runs on different commits, not interleaved
controls; observed differences cannot be attributed solely to code changes.

Linear estimates for **4 TiB occupied disk**, scaling by occupied chunks, are
**76.3 s / 22.3 s / 2.2 s**. Not validated at 4 TiB; index memory and allocation behavior
may invalidate linear extrapolation. Occupied chunks remain 32,896 / 18,325 / 65,600.

## Artifacts

- `results.csv`: all measurements.
- `small-*.log`, `medium-*.log`, `large-*.log`: recovery and setup logs.
- `build.log`: build output.
- `benchmark.rs`: injected harness updated to the current `DiskCache` naming.
- Remote source, copied executable and logs: `/mnt/local-ssd/feuer-recovery-retest-65b3bf1/`.

Follow the previous report's reproduction instructions, substituting this commit and
`benchmark.rs`. This run reused the previous Cargo target directory for compilation,
then copied the newly built executable into the new artifact directory before executing.
Capacity was 128 GiB, four Tokio workers; batch sizes 4,096 / 4,096 / 128. Each recovery
was a separate invocation with `RECOVERY_SETUP` unset. Only the benchmark was run,
not the full storage test suite.
