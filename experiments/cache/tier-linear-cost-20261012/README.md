# Historical linear tier-cost comparisons

These results were previously recorded in `eviction.md`. They predate the
[corrected-scheduler retest](../cost-scorer-retest-20261012T1324/README.md) and are
not current defaults or evidence that the consolidated scorer is faster.

## Model and settings

```text
disk_seconds(bytes) = 0.00006246274509803923 + bytes × 8.187806372549019e-9
source_seconds(bytes) = 0.108599 + bytes × 1.1890459652219313e-8
memory_score = demand × disk_seconds(payload_bytes) / max(memory_charge, 1)
disk_score = demand × max(source_seconds(payload_bytes) - disk_seconds(payload_bytes), 0)
             / max(payload_bytes, 1)
```

The baseline used 256,000 fixed-equivalent bytes. Both tiers used all caller
demand, with sample size 256, history half-life and maximum age 262,144 requests,
64 recent events per key, and 7% idle buffers. Each pair used the same executable,
600-second prefix, fresh caches, full-object transformation and synthetic
zero-filled misses, without network delay or download coalescing.

Disk coefficients fit the median per-run p50s of 96 µs at 4 KiB and 8,648 µs at
1 MiB from the [DataFile benchmark](../../ssd/io-queue-20261001/REPORT.md), using
32 read callers, reused buffers, and no writers. Source coefficients use median
TTFB 108.599 ms and per-object throughput 80.205 MiB/s from
[warmed S3 GETs](../../s3_benchmark.md), using 48 workers and roughly 110-MiB objects.

These are median-based approximations from different load regimes, not measured
mean costs. They omit alignment, checksum work and contention, extrapolate source
costs to smaller objects, and assume memory misses have usable disk copies.
Modeled seconds below are summed logical retrieval work, not elapsed time.

## Required 100-GB memory / 500-GB disk pair

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Memory request / byte hit % | 91.980 / 95.129 | 91.794 / 95.108 |
| Disk request / byte hit % | 0.149 / 0.230 | 0.305 / 0.242 |
| Combined request / byte hit % | 92.129 / 95.359 | 92.099 / 95.350 |
| SSD read / write GB | 6.433 / 98.701 | 6.773 / 98.465 |
| DataFile read calls | 2,394 | 4,899 |
| Source callbacks / GB | 126,458 / 129.943 | 126,940 / 130.200 |
| Useful memory at finish GB | 97.084 | 96.847 |
| Request-phase CPU seconds | 47 | 47 |
| Maximum RSS GB | 12.018 | 12.178 |
| Memory eviction-triggering insertions | 2,245 | 2,830 |
| Disk eviction-triggering insertions | 0 | 0 |
| Queue-full write skips | 22,058 | 21,551 |
| Scheduled-completion p99 ms | 0.403 | 0.357 |
| Modeled retrieval seconds | 15,331.116 | 15,389.457 |

GB is decimal. The modeled cost increased 0.381%. No disk eviction occurred, so
this pair did not exercise disk victim selection. Roughly 66.4 MB of writes
occurred in each five-second observation tail, which is not a flush guarantee.

## Validation and evidence

Both cases completed 1,606,645 requests exactly once, exited 0, and had zero
lookup/I/O/write errors and zero pending payload after the observation tail.
This single pair is not a significance test or a production-latency result.
Useful synthetic payload accounting is not physical page residency.

The original private workspace is `../replay-feuer/tier-slices-100GB-500GB-600s-2d987e2/`,
relative to the project checkout. It retains the reports, metric inventories,
process/device observations, hashes and commands. No measurements were rerun while extracting this summary.
