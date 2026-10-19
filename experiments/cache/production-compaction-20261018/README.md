# Production compaction: retention, write admission, and query queues

## Question

Why does Feuer 21 sometimes read fewer SSD bytes without improving query latency?
Does extra copying or disk-write admission pressure offset useful memory retention,
or is query execution and the differing workload responsible?

## Method and limits

Compare the existing Feuer 20 and 21 cold sweeps for `pascal-feuer`, with the same
14 searchers and configuration. Regular 21 changes the five Feuer dependencies
from `2f381acd` to `4e7a7d1`. Query execution and Tantivy are unchanged between these
regular releases.

- Feuer 20: `sweep_20261017T203247Z_cold`, 17 October 2026.
- Feuer 21: `sweep_20261017T220813Z_cold`, 18 October 2026.
- Datadog organization: `byoc-sim.datadoghq.com`. Times below are UTC.
- Fetch cache and runtime metrics across warmup and all measured stages. Use each
  workspace's recorded measurement window, not the entire supervisor stage.
- Discover tags separately for each metric and version. Validate the exact 14-pod
  population and reject duplicate logical emitters. Group by pod, cache and
  operation where applicable. Retain `host` and `version` during validation.
- Counter totals use 20-second sum rollups. Prorate boundary buckets by overlap.
  Distribution sums divided by distribution counts give mean write-queue wait.
  Gauge means use time-weighted averages of 20-second average rollups.
- Source bytes and process CPU totals come from the existing independently
  enriched reports. Source bytes are actual object-store downloads, not cache
  lookup callback bytes. Normalize by the report's total query count, including
  failed and skipped queries, not only successful queries.

These are **observational comparisons, not a matched causal experiment**. The
runs use moving absolute query times and slightly different request counts.
Twenty-second rollups and exporter timing obscure short bursts. Queue wait is
measured for admitted writes when dequeued, not for rejected admissions or query
completion. Missing metric samples are not treated as proof of zero activity.

The `QPS` labels are the load generator's configured batch rates. Per-query
normalization uses the reports' actual request counts.

## Retention and write pressure

The first table shows Feuer 20 → 21 for the split cache. Compaction counts combine
ordinary range trimming with the new same-range path. Reclaimed charge is
cumulative accounting, not simultaneous physical memory release.

| Stage | Compactions | Reclaimed charge, GB | Eviction-triggering insertions | Memory-byte hit share | Rejected write admissions | Mean write-queue wait |
|---|---:|---:|---:|---:|---:|---:|
| QPS 1 | 47,943 → 555,064 | 44.8 → 276.5 | 290,591 → 250,235 | 85.88% → 86.10% | 4.28% → 6.27% | 104 → 125 ms |
| QPS 2 | 188,758 → 3,217,432 | 93.6 → 1,859.5 | 1,705,718 → 1,575,862 | 72.89% → 71.91% | 23.11% → 42.80% | 147 → 192 ms |
| QPS 3 | 62,716 → 1,387,783 | 138.4 → 898.6 | 778,349 → 782,879 | 88.41% → 88.10% | 7.15% → 13.43% | 91 → 111 ms |
| QPS 4 | 118,201 → 2,124,590 | 222.3 → 1,364.1 | 1,614,645 → 1,013,251 | 80.11% → 83.17% | 11.10% → 7.02% | 106 → 52 ms |
| QPS 5 | 175,851 → 4,458,410 | 259.7 → 1,901.1 | 1,614,637 → 1,430,314 | 85.01% → 86.00% | 15.14% → 33.48% | 138 → 84 ms |
| QPS 10 | 176,406 → 3,281,061 | 489.3 → 1,604.9 | 1,027,945 → 776,834 | 92.59% → 93.09% | 17.90% → 34.61% | 145 → 183 ms |

Rejected admission share is `queue_full / (queued + queue_full)`. It counts
attempts, not unique byte ranges, payload bytes or failed queries. Skipped disk
writes leave the memory entry and query result valid.

The next table uses fleet-wide SSD, S3 and process CPU totals from the source
reports, not split-cache-only totals. Query p95 is the reported workload latency.

| Stage | SSD bytes/query change | S3 bytes/query change | CPU seconds/query change | Query p95, seconds |
|---|---:|---:|---:|---:|
| QPS 1 | −5.33% | +9.05% | +0.14% | 4.63 → 5.58 |
| QPS 2 | +5.28% | +13.19% | +13.10% | 9.88 → 8.27 |
| QPS 3 | +2.45% | +7.29% | +7.05% | 6.10 → 6.88 |
| QPS 4 | −14.86% | −35.58% | +18.56% | 7.33 → 6.73 |
| QPS 5 | −3.37% | +11.81% | −7.53% | 7.32 → 6.31 |
| QPS 10 | −11.27% | +14.41% | +3.77% | 9.64 → 10.63 |

QPS 4 is consistent with useful retention: fewer evictions, more memory hits,
less disk and source traffic, and less write-queue pressure. QPS 10 also has fewer
evictions and more memory hits, but more source traffic, rejected admissions and
write waiting. Compaction bursts followed by missed disk admission and later
source refetch are a plausible mechanism. These aggregates do not establish that
sequence for individual entries.

QPS 5 is an important counterexample: rejection more than doubles while write
wait and query latency improve. Rejection fraction alone does not explain query
latency, and increasing queue capacity is not justified by this comparison.

## Not a single bad pod or just a boundary effect

At QPS 10, rejection shares are 10.8–27.7% across Feuer 20 pods and 30.1–38.5%
across Feuer 21 pods. The largest Feuer 21 pod accounts for only 9.56% of all
rejections. Pressure is distributed across the fleet.

Removing the first and last 40 seconds of QPS 10 preserves the direction:
rejection rises from 6.41% to 19.51%, and mean queue wait from 72 to 147 ms.
The effect is larger near stage boundaries, but is not confined to them. QPS 4
retains lower rejection and waiting under the same sensitivity check. QPS 5
retains higher rejection and lower waiting.

## Query execution also queues

Runtime counters and queue gauges give another discriminator. The sampled
searcher has 20 main Tokio workers. Busy-worker figures below are summed worker
busy time divided by wall time and pod count, not process CPU utilization.

| Stage | Mean main-runtime busy workers/pod | Mean scheduled runtime tasks/pod | Mean pending search-pool tasks/pod |
|---|---:|---:|---:|
| QPS 4 | 4.11 → 3.78 | 5.02 → 5.93 | 28.60 → 44.28 |
| QPS 5 | 4.62 → 4.93 | 8.00 → 7.79 | 37.28 → 37.83 |
| QPS 10 | 5.13 → 5.49 | 9.37 → 15.00 | 128.83 → 144.07 |

CPU-pool queuing is substantial at QPS 10. The QPS 4 process CPU increase occurs
while main-runtime busy time decreases. It is therefore premature to attribute
all extra CPU to synchronous memory copying. A CPU profile should separate
query execution, compaction, checksumming and write publication before choosing
an ablation.

There is also a CPU telemetry caveat: archived 20-second Kubernetes CPU samples
occasionally exceed 64 cores on these 64-vCPU searchers, reaching about 94 cores
on one Feuer 21 pod. Do not interpret those instantaneous samples as physical
saturation. A read-only probe of searcher-0 confirms 64 online CPUs, affinity
`0-63`, no cgroup CPU quota and no recorded CPU throttling since that process
started. Between 07:38:01 and 07:38:22 UTC, its cgroup CPU counter advanced
13.95 CPU seconds over 21.96 seconds of uptime. This later, low-load observation
does not validate the earlier high-load CPU measurements.

## Historical full-host profiles: query CPU dominates

Existing eBPF profiles cover the actual QPS 10 stages. No new workload or
in-process sampling is needed to examine them. Access uses the Datadog MCP
`profiling` toolset, documented in [Available Tools & API Reference](https://datadoghq.atlassian.net/wiki/spaces/PROF/pages/6340905445).
The current endpoint is `https://mcp.datadoghq.com/v1/mcp?toolsets=core,profiling`.

Discover services, tag names, pod values and profile types separately for each
window. Both versions have the expected 14 searchers, with five or six profile
records per pod. The validated query is:

```text
service:quickwit family:ebpf pod_name:cloudprem-ws-pascal-feuer-searcher-*
```

Use `explore_profiling_flame_graph`, profile type `ebpf-cpu-time`, and these UTC
windows from the recorded measurements:

- Feuer 20: **2026-10-17 21:20:43.436683–21:26:10.131575**.
- Feuer 21: **2026-10-18 07:10:05.436308–07:15:30.675415**.

Compute each share from the response's `totalMatchingValue / totalValue`, after
converting their units to match. This uses the whole matching sample population,
not the sum of the displayed top stack fragments.

| Stack filter | Feuer 20 CPU share | Feuer 21 CPU share |
|---|---:|---:|
| `tantivy` | 81.93% | 80.87% |
| `feuer_memory` | 2.00% | 2.39% |
| `feuer_memory::store::MemoryCache::insert` | 1.73% | 2.14% |
| `feuer_storage` | 1.72% | 1.45% |

These are inclusive stack-filter shares and **must not be added**. Insertion is
part of the memory-cache share. It includes normal insertion, eviction scoring,
ordinary trimming and same-range copying, not just the new compaction path.
Inlining and unresolved symbols limit attribution. Feuer 21's unfiltered view
includes an unresolved Quickwit stack fragment accounting for 6.14%.

The visible CPU consumers are predominantly query-related. In Feuer 21, the
predicate-cache filler/intersection-advancement path accounts for 27.67% of total
sampled CPU, and an automaton/term-dictionary streaming path accounts for 14.39%.
These displayed paths are also inclusive, not a disjoint breakdown.

**Conclusion:** memory insertion and compaction are not the dominant aggregate
CPU consumer in this workload. Query CPU and search-pool scheduling are more
promising explanations for why fewer SSD bytes do not produce uniform latency
improvements. The modest increase in the insertion share is compatible with
extra compaction work, but the differing datasets prevent causal attribution.
A small average CPU share does not rule out short synchronous stalls, and CPU
sampling does not measure off-CPU waits or establish skipped-write-to-source-refetch
causality. The write-admission evidence therefore remains a separate hypothesis.

The inclusive shares are reproduced in `results/profile-shares.csv` from the
saved responses using `analyze-mcp.py`. Exact measured-window calls are in
`raw/mcp-measured-windows-calls.json`, with visualization links in the responses
and `raw/mcp-measured-summary.json`. Earlier rounded-window requests returned
the same aggregate shares.

### Independent disjoint check: one historical pod-minute

A downloaded, remotely symbolized profile independently checks the MCP result.
It covers **searcher-4, 2026-10-18 07:13:47.748–07:14:48.256 UTC**, with the
original Feuer 21 container and image digest verified in profile metadata. All
sample process labels are `quickwit`. Its 50-ms sampling period gives 49,515
weighted samples and 2,475.75 sampled CPU seconds over about 60.51 wall seconds.
This is existing 20-Hz full-host data, not the failed 99-Hz diagnostic.

Assign every sample exactly once using this precedence: memory insertion,
other memory-cache paths, storage, other Feuer/historian paths, Tantivy,
other Quickwit query/search paths, then unresolved or other named paths.
Classification uses the whole caller stack, not just the innermost function.

| Disjoint category | Sampled CPU share |
|---|---:|
| Tantivy paths outside cache paths | 85.05% |
| Memory insertion, eviction and compaction | 3.42% |
| Other memory-cache paths | 0.30% |
| Storage paths outside memory-cache paths | 1.64% |
| Other Feuer/historian paths | 1.41% |
| Other Quickwit query/search paths | 0.98% |
| Other named runtime/system paths | 3.16% |
| Unresolved leaf without a recognized subsystem | 4.05% |

The unrounded categories sum to 100%. A separate thread partition assigns
92.23% to `quickwit-search`, 6.20% to main runtime threads and 1.45% to
`feuer-read-io`. These partitions must not be added to each other.

Innermost frames are unresolved for 7.81% of CPU samples. Some still have known
callers and can be assigned to a subsystem. The remaining unresolved category
is 4.05%. No additional local symbolization was required for the named Rust
frames already present in the downloaded profile.

This minute has a larger insertion share than the fleet-wide window, but still
supports the same conclusion: query-related CPU dominates. It is a narrower
population, not a second version comparison. Raw input, `go tool pprof` output,
classification rules and results are retained under `raw/host-profiles/` and
`analyze-pprof.py`.

### Four interior pod-minutes per version

Repeat the same disjoint classification on the four downloaded profiles per
version wholly inside its measured QPS 10 window. Exclude the first,
boundary-overlapping profile from each version. All eight profiles are from the
same host and searcher ordinal, with one verified container and the correct
image digest per version.

Version shares below are weighted by sampled CPU time, not an arithmetic mean
of profile percentages. Parent/child frames are never added as separate samples.

| Disjoint category | Feuer 20 | Feuer 21 |
|---|---:|---:|
| Tantivy paths outside cache paths | 79.73% | 77.63% |
| Memory insertion, eviction and compaction | 1.59% | 2.17% |
| Other memory-cache paths | 0.20% | 0.24% |
| Storage paths outside memory-cache paths | 1.53% | 1.35% |
| Other Feuer/historian paths | 1.59% | 1.60% |
| Other Quickwit query/search paths | 0.89% | 0.94% |
| Other named runtime/system paths | 8.07% | 9.39% |
| Unresolved leaf without a recognized subsystem | 6.41% | 6.68% |

Each version sums to 100% before rounding. Per-profile variation:

| Version | Profile start, UTC | Sampled CPU seconds | Insertion share | Tantivy share | Storage share |
|---|---|---:|---:|---:|---:|
| 20 | Oct 17, 21:21:42.509 | 3,317.40 | 0.87% | 75.58% | 0.82% |
| 20 | Oct 17, 21:22:45.748 | 2,184.90 | 1.80% | 78.54% | 2.22% |
| 20 | Oct 17, 21:23:45.748 | 2,058.40 | 1.49% | 82.73% | 1.79% |
| 20 | Oct 17, 21:24:46.248 | 2,562.80 | 2.42% | 83.70% | 1.67% |
| 21 | Oct 18, 07:10:46.031 | 3,000.80 | 1.46% | 75.18% | 1.10% |
| 21 | Oct 18, 07:11:47.494 | 2,699.25 | 1.95% | 74.29% | 1.22% |
| 21 | Oct 18, 07:12:49.498 | 2,478.20 | 2.02% | 76.83% | 1.51% |
| 21 | Oct 18, 07:13:47.748 | 2,475.75 | 3.42% | 85.05% | 1.64% |

Feuer 20 contributes 202,470 samples and 10,123.50 sampled CPU seconds across
241.567 summed profile seconds. Feuer 21 contributes 213,080 samples and
10,654.00 CPU seconds across 242.443 summed profile seconds. Metadata windows
have small adjacent overlaps totaling 0.375 seconds and 0.218 seconds,
respectively, each below 0.2% of summed duration. Samples are aggregated as
provided, not purportedly de-duplicated without per-sample timestamps.

Innermost unresolved-frame shares are 9.60% and 9.77%. Known caller stacks allow
some of these samples to be attributed, leaving the smaller unresolved categories
in the table. These limits and the different datasets prevent a causal version
claim. The selected interior windows also omit the stage boundaries, where write
pressure was greater.

**The CPU-cost conclusion holds in every selected minute:** Tantivy accounts for
at least 74.29%, while the entire insertion path accounts for at most 3.42%.
Insertion work increases, but remains a small aggregate CPU share. This does not
exclude short copying stalls or write-admission effects on individual query tails.

Profile identities, hashes, classifications and per-profile ranges are in
`raw/host-profiles/pod4-version-attribution.json`. The selection and analysis are
reproducible with `analyze-pod4-profiles.py`.

## Warm-cache diagnostic and instrumentation limitation

A separate diagnostic used unchanged Feuer 21, existing warm caches, QPS 10,
seed 136, the `splunk` query profile and a fixed end time of
**2026-10-18T07:10:00Z**. The executed configuration targeted **searcher-4**.
The intended traffic duration was 180 seconds, with a 30-second, 99-Hz profile
starting after 60 seconds. No cache reset or queue change was made.

The profiler initializer returned `CreatingError`. Its handler unwrapped that
error and panicked without restarting the container. The profiler's collector
creates a temporary file. The process had no `TMPDIR`, a read-only root filesystem
and a non-writable `/tmp`, consistent with the initialization failure. Endpoint
presence therefore did not establish that sampling would work. Traffic was
stopped, no profile was collected, and the harness did not write its normal final
request CSV. This interrupted run is not a performance comparison.

The pre-profile counter window was 75.71 seconds, including setup and the initial
traffic. Searcher-4 showed only 2,868 split compactions, 0.739 GB reclaimed charge,
zero rejected writes, 3.64 ms mean write-queue wait and a 96.63% memory-byte hit
share. Its cgroup CPU counter advanced 755.4 CPU seconds. This was much lighter
cache pressure than the historical QPS 10 stage. Merely selecting the same QPS
and seed does not recreate the original pressure with retained caches.

For any future targeted experiment, verify the intended pressure during the
measurement window. A causal compaction ablation would preserve ordinary trimming,
queue settings and workload while disabling only the new fallback, with matched
cache-state preparation. The current evidence does not justify resizing queues
or assuming that reduced copying will materially improve query CPU.

## Evidence and reproduction

`settings.json` records the target, source reports, exact query windows and metric
families. `results/stage-cache-metrics.csv` retains split-cache stage totals,
query counts and measurement windows before display rounding.
`results/profile-shares.csv` retains the inclusive CPU numerators and denominators.
The other compact results are the tables above, rather than duplicate exports.

The seven Python scripts beside this report are offline analysis only. Local
ignored `raw/` contains exact API requests and responses, identity validation,
frozen stage summaries, per-pod results, boundary checks, downloaded CPU profiles,
CPU-counter probes and the interrupted diagnostic. Discovery clients and the
failed diagnostic's workload runner remain ignored, not reproduction entry points.

From this directory, with the archived `raw/` inputs restored, run:

```sh
python3 analyze.py
python3 check-stages.py
python3 analyze-runtime.py
python3 analyze-mcp.py
python3 analyze-pprof.py raw/host-profiles/pod4-raw.txt
python3 analyze-pod4-profiles.py
python3 analyze-diagnostic-counters.py raw/diagnostic-20261018T075955Z
```

Python 3 and its standard library are sufficient when the saved pprof text is
present. Otherwise the multi-profile analysis invokes `go tool pprof` with
`-symbolize=none` on the downloaded symbolized profiles. It does not fetch symbols.
Stage analysis reads the frozen `raw/stage-source-{20,21}.json` inputs, whose
hashes are recorded in settings, and the saved metric responses. No neighboring
checkout or live service is needed. Scripts regenerate the two CSV summaries and
write detailed derived outputs only under `raw/`.

A clean checkout alone cannot rerun the analysis because raw captures are not
committed. The original sweep reports remain provenance references, not runtime
dependencies. No API acquisition or workload command is part of these offline
reproduction steps.
