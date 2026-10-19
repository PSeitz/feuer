# Does reclaiming buffer slack reduce SSD reads?

## Decision

**Use allocation capacity as an additional compaction criterion.** In one matched
10-minute replay, it reduced SSD reads by **35.416 GB (28.02%)** without increasing
source downloads. The tradeoff was **21 additional CPU seconds (11.29%)**, more
write-admission pressure, and mixed latency changes.

## Alternatives and method

- **Baseline:** compact before eviction only when replacement request ranges
  contain fewer payload bytes than the cached range.
- **Candidate:** keep baseline trimming, but also copy an unchanged range when
  an exact-size allocation saves at least 25% of its backing capacity. Keep the
  existing pressure and 64-request maturity conditions and compaction callback.

Baseline source is `2f381acdb13e114593e98d1078f29b341279380d` plus the saved
pre-experiment working-tree changes, not that commit alone. Candidate source is
that snapshot plus `raw/candidate.patch`. The source archives and binary hashes
are recorded in [settings.json](settings.json). The memory change is committed
in `4e7a7d1`.

One baseline/candidate pair, baseline first, on `benchmark_arm_128g_local_ssd_2`
(`ssh m8g-32cpu-local-ssd-2`). Both used the first 600 seconds of the same capture,
**100000000000 memory bytes**, **500000000000 disk bytes**, 64 shards, 32 runtime
threads, matched jemalloc, and a fresh cache. No repetitions or concurrent builds.
Host checks found no competing benchmark and negligible pre-run device activity.

The replay preserved arrivals and used independent, immediate, exact-range source
downloads through the buffer pool. No source expansion, coalescing, simulated S3,
or disk bypass. Scoring and history settings were identical. Sources, builds,
input and output were on EBS, with only cache files on the measured SSD.

## Results

Both runs completed **1,606,645 requests exactly once**, returning
**2,800,015,397,526 bytes**, with no replay I/O/checksum errors or pending work after
the five-second observation tail. Decimal GB below. Read/write totals include the
tail, which had no reads and approximately 0.067 GB of writes per case.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| DataFile read GB | 126.381 | 90.965 | -28.02% |
| Device read GB | 126.513 | 91.059 | -28.02% |
| DataFile read calls | 30,983 | 12,922 | -58.29% |
| DataFile write GB | 105.037 | 101.983 | -2.91% |
| DataFile write calls | 31,783 | 32,407 | +1.96% |
| Source download GB | 140.331 | 138.001 | -1.66% |
| Source callbacks | 129,592 | 127,076 | -1.94% |
| Memory byte hit rate | 92.587% | 93.666% | +1.079 percentage points |
| Combined byte hit rate | 94.988% | 95.071% | +0.083 percentage points |
| Request-phase CPU seconds | 186 | 207 | +11.29% |
| OS maximum RSS GB | 108.344 | 108.690 | +0.32% |
| Scheduled completion mean ms | 4.858 | 3.975 | -18.17% |
| Scheduled completion p95 ms | 3.352 | 3.660 | +9.19% |
| Scheduled completion p99 ms | 102.416 | 78.386 | -23.46% |
| Caller-queue mean ms | 1.894 | 2.459 | +29.85% |
| Cumulative allocation charge released by compaction GB | 2.312 | 26.361 | — |
| Memory eviction-triggering insertions | 13,485 | 5,256 | -61.02% |
| Entries at request completion | 88,339 | 104,925 | +18.78% |
| Queue-full disk-write admission skips | 26,904 | 37,655 | +39.96% |

More charged capacity was reclaimed, more entries remained resident, and both
disk-served and source-served traffic decreased. This supports the retention
benefit rather than a reduction achieved by sending more requests to the source.
Read reductions occurred both before 180 seconds (21.862 → 11.771 GB) and later
(104.519 → 79.194 GB), using approximate one-second sample boundaries.

## Tradeoffs and limits

- The extra copying costs CPU. Completion mean and p99 improved, but p95 and
  caller-queue time worsened. One pair does not establish repeatability or small
  latency/RSS differences.
- Compaction's existing callback can enqueue another disk write when coverage is
  absent. Queue-full skips rose 40%, and already-covered disk publication outcomes
  rose from 260 to 2,935. These are write-admission outcomes, not dropped lookups.
  Write bytes still decreased. This experiment does not separate the callback's
  contribution from the retention change.
- Released allocation charge is cumulative, not simultaneous physical memory
  freed. Outstanding readers and writes can keep old buffers alive. RSS did not
  decrease as residency improved.
- No disk eviction or no-capacity outcome occurred. Results apply to this
  immediate, exact-range replay, not expanded production downloads or restarts.
- DataFile counters exclude startup and count API operations. Device totals
  include startup and can include other processes. CPU phase counters have
  one-second resolution. The observation tail is not a forced flush.

## Reproduction and retained evidence

Follow [the timed replay procedure](../../storage/timed-replay.md). Frozen source
archives, the candidate patch, capture/binary hashes, actual commands and full
telemetry are retained locally under ignored `raw/`. The remote workspace is
`/home/ubuntu/feuer-same-range-compaction-20261017`.

Build each snapshot in a separate target directory with
`cargo build --locked --release -p feuer-replay --bin replay`. With the environment
below, run once per case, baseline then candidate, using fresh cache/output paths:

```sh
export FEUER_ACCESS_COUNT_HALF_LIFE=262144 FEUER_MAX_ACCESS_EVENTS_PER_KEY=64
export FEUER_MAX_ACCESS_AGE_ACCESSES=262144 FEUER_RECLAIM_SAMPLE_SIZE=64
export FEUER_IDLE_BUFFER_POOL_PERCENT=7
unset FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES

env -u MALLOC_CONF LD_PRELOAD=/lib/aarch64-linux-gnu/libjemalloc.so.2 \
  "$ROOT/bin/$CASE" --input "$ROOT/input/trace.tar.zst" \
  --cache-dir "/mnt/local-ssd/$CASE-fresh/data" --output "$ROOT/results/$CASE" \
  --memory-bytes 100000000000 --disk-bytes 500000000000 --trace-seconds 600 \
  --memory-cost-per-request 256000 --disk-cost-per-request 256000 \
  --access-count-half-life 262144
```

`raw/METRICS.md` contains the complete comparison. `raw/results/{baseline,candidate}/`
contains per-case metrics, exact latency distributions, histogram intervals,
process/device observations, manifests and validation reports. `raw/` also holds
local runners and reporting helpers for reproduction, not as maintained code.
