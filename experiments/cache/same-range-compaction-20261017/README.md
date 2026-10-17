# Same-range pressure compaction: matched SSD replay

## Result

**The candidate read 35.416 GB less from SSD, a 28.02% reduction, without increasing source traffic.** DataFile reads fell from 126.381 GB to 90.965 GB. Device counters independently show the same 28.02% reduction. Source downloads fell 1.66% and disk write bytes fell 2.91%.

The cost was 21 additional request-phase CPU seconds, or 11.29%. OS maximum RSS increased 0.32%. Scheduled-to-completion mean and p99 improved, but p95 and caller-queue latency worsened. This is one matched pair, not a repeatability estimate or a general production guarantee.

The benchmark ran in isolated snapshots. The tested source change and three tests have since been applied to the main checkout, without unrelated working-tree changes. The exact benchmark patch is retained locally as `candidate.patch`, alongside the source snapshots.

## Candidate

Keep existing range trimming unchanged. When it produces no plan, copy the same byte range if an exact-size allocation saves at least 25% of its current allocation charge. Existing shard-pressure and 64-request maturity conditions still apply. The candidate reuses the existing copy-outside-lock, publication, and compaction-callback paths.

The production-code diff is seven added lines and two removed lines in `feuer-memory/src/store/shard.rs`. It does not change capacities, arrival timing, source behavior, scoring, disk lookup, checksum preparation, or accounting rules. It changes memory allocation ownership through a real copy, not by lowering the charge on an unchanged buffer.

## Setup and provenance

- Host: `benchmark_arm_128g_local_ssd_2`, reached as `m8g-32cpu-local-ssd-2`, 32 runtime threads, SSD `nvme1n1`.
- Baseline: `2f381acdb13e114593e98d1078f29b341279380d` plus the frozen existing working tree. Candidate: that exact snapshot plus the saved patch.
- First 600 seconds, range mode, **100000000000 memory bytes**, **500000000000 disk bytes**, 64 shards, fresh cache per case.
- Identical immediate independent exact-range source downloads through the existing buffer pool. No simulated S3, coalescing, expanded source ranges, bypass, or task throttling.
- Both tiers use request cost 256000, requested-byte cost 1, access-count half-life 262144, recent-event limit 64, history age 262144, eviction sample 64, idle ceiling 7%.
- Matched system jemalloc through `LD_PRELOAD`, `MALLOC_CONF` unset. Actual commands, allocator/toolchain versions and hashes are in each case's manifest.
- Baseline first, candidate second, one replay per case. No repetitions. No build ran concurrently with either timed replay.
- Standard replay/cache metrics only, without the earlier diagnostic event logging.
- Sources, input, builds and output on EBS. Only the cache files were on the measured SSD. About 1.41 TB was available before each case. Host checks found no competing benchmark and negligible pre-run device activity.

Settings and source/binary hashes are in [settings.json](settings.json). Raw source archives, per-file hashes, commands, test/build logs, and all telemetry are ignored under `raw/`. The remote workspace is `/home/ubuntu/feuer-same-range-compaction-20261017`.

## Captured costs

Decimal GB. DataFile calls are API operations, not kernel or device request counts. Read/write totals here include the five-second observation tail. There were no tail reads.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Requests | 1,606,645 | 1,606,645 | same |
| Returned GB | 2,800.015 | 2,800.015 | same |
| DataFile read GB | 126.381 | 90.965 | -28.02% |
| DataFile read calls | 30,983 | 12,922 | -58.29% |
| Process/device read GB | 126.513 | 91.059 | -28.02% |
| DataFile write GB | 105.037 | 101.983 | -2.91% |
| DataFile write calls | 31,783 | 32,407 | +1.96% |
| Source GB | 140.331 | 138.001 | -1.66% |
| Source callbacks | 129,592 | 127,076 | -1.94% |
| Memory hits | 1,444,349 | 1,465,232 | +1.45% |
| Disk/buffer hits | 32,704 | 14,337 | -56.16% |
| Memory request hit rate | 89.899% | 91.198% | +1.300 percentage points |
| Memory byte hit rate | 92.587% | 93.666% | +1.079 percentage points |
| Combined request hit rate | 91.934% | 92.091% | +0.157 percentage points |
| Combined byte hit rate | 94.988% | 95.071% | +0.083 percentage points |
| Request-phase CPU seconds | 186 | 207 | +11.29% |
| Average CPU cores | 0.310 | 0.345 | +11.29% |
| Sampled peak RSS GB | 107.182 | 108.590 | +1.31% |
| OS maximum RSS GB | 108.344 | 108.690 | +0.32% |
| Execution seconds | 599.372 | 599.372 | unchanged at shown precision |
| Backlog drain milliseconds | 12.100 | 12.172 | +0.60% |
| Scheduled completion mean ms | 4.858 | 3.975 | -18.17% |
| Scheduled completion p95 ms | 3.352 | 3.660 | +9.19% |
| Scheduled completion p99 ms | 102.416 | 78.386 | -23.46% |
| Caller-queue mean ms | 1.894 | 2.459 | +29.85% |
| Service mean ms | 2.926 | 1.468 | -49.84% |
| Service p99 ms | 12.741 | 2.600 | -79.59% |
| Peak pending write payload GB | 9.670 | 8.650 | -10.54% |
| Peak active lookups | 1,100 | 822 | -25.27% |
| Device read await ms | 5.299 | 5.654 | +6.70% |
| Device average queue size | 8.677 | 6.650 | -23.36% |
| Device utilization | 7.900% | 6.694% | -15.25% |

[METRICS.md](METRICS.md) presents all exact latency distributions, cache/buffer scalars, write outcomes, histogram counts/means/quantile intervals, device statistics and time windows. Raw per-case inventories retain phase deltas, tail activity and sampled peaks for every metric. Histogram intervals are not exact percentiles.

## Retention and write effects

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Compaction publications | 122 | 93,846 |
| Cumulative allocation charge released by compaction GB | 2.312 | 26.361 |
| Memory eviction-triggering insertions | 13,485 | 5,256 |
| Entries at request completion | 88,339 | 104,925 |
| Final resident allocation charge GB | 99.159 | 98.499 |
| Final idle buffers GB | 0.839 | 1.495 |
| Write admission queued | 100,117 | 107,155 |
| Queue-full write admission skips | 26,904 | 37,655 |
| Writes published after observation tail | 99,857 | 104,220 |
| Already covered at disk publication | 260 | 2,935 |
| Disk eviction / no-capacity outcomes | 0 / 0 | 0 / 0 |

Compaction released more charged capacity, more entries remained resident, eviction-triggering insertions fell 61.02%, and both disk-served and source-served traffic decreased. This supports the proposed retention mechanism for this replay. The released-charge counter is cumulative, not peak physical memory freed, and does not measure copied bytes. Held callers and pending writes can keep an original allocation alive.

The existing compaction callback was deliberately preserved. It can enqueue another disk write if the copied range is not yet covered. Queue-full skips increased 39.96%, and already-covered publication outcomes increased. These are write-admission outcomes, not dropped lookup requests. Actual write bytes still fell 2.91%, while write calls rose 1.96%. There is no additional candidate separating the callback's contribution from retention, so do not attribute every changed disk admission exclusively to lower memory pressure.

Approximate cold/later windows, using nearest one-second snapshots:

| Phase | Baseline read GB | Candidate read GB | Baseline write GB | Candidate write GB |
| --- | ---: | ---: | ---: | ---: |
| First 180 seconds | 21.862 | 11.771 | 66.580 | 64.078 |
| After 180 seconds through request completion | 104.519 | 79.194 | 38.389 | 37.839 |
| Five-second observation tail | 0 | 0 | 0.067408 | 0.066351 |

## Validation and build correction

- Both replay and runner exits were zero. Each completed all 1,606,645 requests exactly once and returned 2,800,015,397,526 bytes.
- No I/O errors, checksum failures, callback errors, invalid downloads, failed/canceled writes, pending payload, queued writes or caller backlog remained after the tail.
- Baseline: 56 memory tests. Candidate: 59 memory tests, including all three new focused tests. Both: 12 historian tests, 31 tiered-cache tests and 15 replay tests passed. Clippy passed with warnings denied.
- Both full storage suites reported **150 passed, one failed, one ignored**. The failure is `disk_cache::adversarial_tests::overflowing_fields_preserve_chunk_ownership`, reproduced by the unchanged baseline. Do not describe the full storage suite as passing.
- Initial release compilation reused a stale baseline executable for the candidate through a shared Cargo target. Identical SHA-256 hashes exposed this before any candidate replay. The pair wrapper was stopped without interrupting the running baseline. After the baseline completed, the candidate was rebuilt and retested in a fresh target directory. Its actual replay binary has a distinct recorded hash. No candidate run used the stale binary, and neither replay was repeated. Initial logs are preserved under `raw/stale-candidate-build/`. `build-case.sh` now uses separate target directories and `run-pair.sh` rejects identical executables.
- Both runner-owned SSD caches and all monitor processes were removed afterward.
- After applying the exact tested source files to the main checkout, all 59 memory tests, memory Clippy with warnings denied, formatting and whitespace checks passed again.

Phase counters exclude cache opening and trace loading. Process totals include startup and final reporting. Device counters can include other processes. CPU phase counters have one-second resolution and sampled peaks can miss short spikes. The five-second tail is observation, not a forced flush or durability guarantee.

The measurement establishes a substantial SSD-byte reduction in this exact-range, immediate-source replay. One pair does not establish a small latency/RSS difference or predict every production workload. No additional replay was launched.

## Reproduction

Source archives and runner/reporting helpers are retained locally, not committed. `raw/source.tar` and `raw/candidate.tar` contain the exact source snapshots. Stage them into separate `baseline/` and `candidate/` directories on the assigned host, alongside the preserved input, reporting helpers, and scripts. Use a fresh output root. `run-pair.sh` builds and tests both cases before running baseline then candidate, and refuses result directories that already exist through `run-case.sh`.

Regenerate the comparison without running any replay:

```sh
python3 experiments/cache/same-range-compaction-20261017/compare.py \
  experiments/cache/same-range-compaction-20261017
```
