# Partial-read checksums: 10-minute replay comparison

Question: does learning requested-range checksums from verified disk reads reduce
I/O and request cost compared with whole-entry verification on the same trace?
This is a historical comparison of the exact sources below, not current behavior.

## Conclusion

This replay shows little workload-level benefit. DataFile read bytes fell from
134.293 GB to 132.242 GB (1.53%), while read calls fell 0.86% and mean read size fell
0.68%. Device counters confirm the read-byte difference. Read bytes per byte returned
by the disk/buffer tier remained about 2:1, moving from 2.025 to 1.993.

CPU was slightly higher, RSS and hit rates were effectively unchanged, and latency
results were mixed. This one pair does not establish a small performance improvement.
Concurrent scheduling changes admissions and which tier serves a request, so not all
of the aggregate read difference can be attributed to partial checksum reuse.

The change requires another disk lookup covered by a learned checksum to save a read.
The first partial read still verifies the covering entry, and promotion can serve
subsequent requests from memory. This replay uses exact-range synthetic downloads,
so it does not reproduce amplification from expanded production downloads.
The synthetic storage test's 32-fold reduction, preserved in the local
[implementation validation archive](raw/IMPLEMENTATION-VALIDATION.md), does not
appear at workload level here.

## Setup

One baseline/candidate pair on `m8g-32cpu-local-ssd-2`, baseline first, fresh cache each time.
Baseline `033284d264062e438010784cb6a2118568130786`, candidate is that snapshot plus the saved working-tree patch.
Range mode, 100,000,000,000-byte memory target, 500,000,000,000-byte disk, immediate independent exact-range sources.
Matched jemalloc, 32 runtime threads, 64 shards, default scoring/history/eviction settings, first 600 seconds only.
No expanded downloads, source coalescing, simulated S3 delay, or full-object transformation.
See [replay-settings.json](replay-settings.json) for settings and hashes, and
[reproduce/README.md](reproduce/README.md) for commands, the exact candidate patch,
and captured reporting scripts. Source snapshots and telemetry in `raw/replay/`
are local-only. The input trace and built binaries are not retained in this directory.
The replay patch predates the later naming/test cleanup in the review archive.

## Requests, CPU, and memory

| Metric | Baseline | Partial checksums | Change |
| --- | ---: | ---: | ---: |
| Requests | 1.60664e+06 | 1.60664e+06 | +0.00% |
| Execution seconds | 599.374 | 599.372 | -0.00% |
| Backlog-drain seconds | 0.0142906 | 0.0121781 | -14.78% |
| Public request hit rate (%) | 91.9431 | 91.9496 | +0.01% |
| Public byte hit rate (%) | 94.9868 | 94.9888 | +0.00% |
| Memory hits | 1.44409e+06 | 1.44449e+06 | +0.03% |
| Disk/buffered hits | 33,109 | 32,816 | -0.88% |
| Source callbacks | 129,446 | 129,342 | -0.08% |
| Memory-returned GB | 2,593.34 | 2,593.36 | +0.00% |
| Disk/buffer-returned GB | 66.3057 | 66.3399 | +0.05% |
| Source GB | 140.371 | 140.313 | -0.04% |
| CPU seconds during requests | 272 | 275 | +1.10% |
| Average CPU cores | 0.453807 | 0.458813 | +1.10% |
| Sampled peak RSS GB | 112.135 | 112.087 | -0.04% |
| OS maximum RSS GB | 112.186 | 112.159 | -0.02% |
| Peak cache-accounted GB | 100.039 | 100.041 | +0.00% |
| Peak pending payload GB | 9.82021 | 9.69683 | -1.26% |
| Peak unstarted requests | 3,781 | 3,903 | +3.23% |
| Peak active lookups | 1,002 | 957 | -4.49% |
| Source queue mean ms | 5.3852e-05 | 4.81456e-05 | -10.60% |
| Source service mean ms | 0.776074 | 0.770415 | -0.73% |

## I/O and admission

| Metric | Baseline | Partial checksums | Change |
| --- | ---: | ---: | ---: |
| DataFile read GB | 134.293 | 132.242 | -1.53% |
| DataFile read calls | 31,322 | 31,054 | -0.86% |
| DataFile read errors | 0 | 0 | — |
| Device read GB | 134.358 | 132.306 | -1.53% |
| Device read await ms | 5.30619 | 5.30992 | +0.07% |
| DataFile write GB | 104.558 | 104.334 | -0.21% |
| DataFile write calls | 31,998 | 32,091 | +0.29% |
| DataFile write errors | 0 | 0 | — |
| Device write GB | 104.586 | 104.37 | -0.21% |
| Device write await ms | 0.186942 | 0.191889 | +2.65% |
| Mean DataFile read size (KiB) | 4,187 | 4,158.64 | -0.68% |
| DataFile read bytes / disk-and-buffer-returned byte | 2.02536 | 1.9934 | -1.58% |
| Total DataFile transfer GB | 238.85 | 236.575 | -0.95% |
| Device average queue size | 9.18641 | 9.07788 | -1.18% |
| Device average utilization (%) | 8.11066 | 8.20697 | +1.19% |
| Memory eviction-triggering insertions | 13,453 | 13,563 | +0.82% |
| Disk eviction-triggering insertions | 0 | 0 | — |
| Disk no-capacity outcomes | 0 | 0 | — |
| Disk queue-full skips | 26,693 | 26,625 | -0.25% |
| Pending payload bytes after observation tail | 0 | 0 | — |
| Tail read bytes | 0 | 0 | — |
| Tail write bytes | 6.74079e+07 | 6.74038e+07 | -0.01% |
| Tail CPU seconds | 0 | 0 | — |
| OS user seconds | 44.4 | 44.01 | -0.88% |
| OS system seconds | 247.65 | 251.29 | +1.47% |
| Process read bytes (/usr/bin/time inputs × 512) | 1.34358e+11 | 1.32306e+11 | -1.53% |
| Process written bytes (/usr/bin/time outputs × 512) | 1.04578e+11 | 1.04354e+11 | -0.21% |

## Write outcomes and final memory accounting

| Metric | Baseline | Partial checksums | Change |
| --- | ---: | ---: | ---: |
| write_admission: queued | 100,292 | 100,310 | +0.02% |
| write_admission: queue_full | 26,693 | 26,625 | -0.25% |
| write_admission: queue_closed | 0 | 0 | — |
| write_admission: canceled | 0 | 0 | — |
| write_admission: already_covered | 293 | 232 | -20.82% |
| write_admission: redundant | 2,182 | 2,192 | +0.46% |
| write_terminal: published | 100,036 | 100,078 | +0.04% |
| write_terminal: already_covered | 256 | 232 | -9.38% |
| write_terminal: no_capacity | 0 | 0 | — |
| write_terminal: failed | 0 | 0 | — |
| write_terminal: canceled | 0 | 0 | — |
| finished resident_charge_bytes (GB) | 99.3124 | 99.2952 | -0.02% |
| finished idle_bytes (GB) | 0.654541 | 0.693174 | +5.90% |
| finished used_bytes (GB) | 99.9669 | 99.9884 | +0.02% |
| settled resident_charge_bytes (GB) | 99.3124 | 99.2952 | -0.02% |
| settled idle_bytes (GB) | 0.654541 | 0.693174 | +5.90% |
| settled used_bytes (GB) | 99.9669 | 99.9884 | +0.02% |

## Exact request latency distributions

| Metric | Baseline | Partial checksums | Change |
| --- | ---: | ---: | ---: |
| sender_lateness mean (ms) | 0.0491262 | 0.0611905 | +24.56% |
| sender_lateness p50 (ms) | 0.03047 | 0.030672 | +0.66% |
| sender_lateness p95 (ms) | 0.053338 | 0.053451 | +0.21% |
| sender_lateness p99 (ms) | 0.625824 | 1.32865 | +112.30% |
| sender_lateness max (ms) | 9.22636 | 6.05283 | -34.40% |
| queue_wait mean (ms) | 3.42532 | 3.2274 | -5.78% |
| queue_wait p50 (ms) | 0.059458 | 0.061913 | +4.13% |
| queue_wait p95 (ms) | 6.89896 | 5.82039 | -15.63% |
| queue_wait p99 (ms) | 74.7844 | 74.729 | -0.07% |
| queue_wait max (ms) | 485.774 | 440.877 | -9.24% |
| service mean (ms) | 3.17932 | 3.16615 | -0.41% |
| service p50 (ms) | 0.001016 | 0.001004 | -1.18% |
| service p95 (ms) | 0.032456 | 0.027281 | -15.94% |
| service p99 (ms) | 13.6871 | 12.3849 | -9.51% |
| service max (ms) | 1,763.23 | 1,777.51 | +0.81% |
| scheduled_to_complete mean (ms) | 6.65377 | 6.45475 | -2.99% |
| scheduled_to_complete p50 (ms) | 0.098497 | 0.102711 | +4.28% |
| scheduled_to_complete p95 (ms) | 10.7319 | 9.81917 | -8.50% |
| scheduled_to_complete p99 (ms) | 126.291 | 117.507 | -6.95% |
| scheduled_to_complete max (ms) | 1,764.27 | 1,778.23 | +0.79% |

## Startup versus later behavior

The 180-second boundary uses the nearest one-second snapshot. These interval values
are approximate, unlike the final request totals. Both runs first observed memory
eviction at about 16.9 seconds, and neither observed disk eviction.

| Phase | Baseline read GB | Partial-checksum read GB | Baseline write GB | Partial-checksum write GB |
| --- | ---: | ---: | ---: | ---: |
| First 180 seconds | 21.125 | 21.684 | 66.447 | 65.603 |
| 180 seconds to request completion | 113.168 | 110.558 | 38.043 | 38.663 |
| Five-second observation tail | 0 | 0 | 0.067408 | 0.067404 |

## Selected I/O latency histograms

These means come from request-phase histogram sum/count deltas, not the observation tail.
Quantiles below are bucket intervals, not exact percentiles.

| Metric | Baseline | Partial checksums |
| --- | ---: | ---: |
| Successful DataFile read count | 31,322 | 31,054 |
| Successful DataFile read mean (ms) | 100.553 | 101.573 |
| Successful DataFile read p50 interval (ms) | (4.096, 8.192] | (4.096, 8.192] |
| Successful DataFile read p95 interval (ms) | (524.288, 1,048.576] | (524.288, 1,048.576] |
| Successful DataFile read p99 interval (ms) | (1,048.576, 2,097.152] | (1,048.576, 2,097.152] |
| Successful DataFile write count | 31,861 | 31,955 |
| Successful DataFile write mean (ms) | 1.493 | 1.482 |
| Disk-cache hit mean (ms) | 148.499 | 149.715 |

## Validation and interpretation

- Both binaries completed all 1,606,645 requests exactly once and returned 2,800,015,397,526 bytes.
- Both exited 0, with no I/O errors, checksum failures, invalid downloads, or failed writes.
- No queued writes or pending payload bytes remained after the five-second tail.
- No disk eviction or no-capacity outcome occurred. Both runs did exercise memory eviction and queue-full write skips.
- Captured pre-run host activity and device samples are available for inspection. The runner rejected named conflicting replay/build processes and installed cleanup for its cache and monitors. These checks do not prove the host was entirely idle.
- Release builds used separate target directories.
- No additional replay repetitions were run.

DataFile counters count API calls and requested payload bytes, not kernel submissions or physical-device bytes.
The read/returned-byte ratio includes buffered disk-tier hits in its denominator, so it is not a pure SSD-read amplification ratio.
Phase counter deltas exclude opening/input loading. OS process totals include startup and final reporting.
Device totals include other processes. Inspect the retained host-activity, iostat, and pidstat files.
All requests must finish exactly once. The five-second observation tail is not a forced flush or durability guarantee.
CPU phase counters have one-second resolution. Sampled peaks can miss short spikes.
One pair does not establish a small speedup or regression, especially for scheduling and latency differences.
Full scalar metrics, histograms (with quantile bucket intervals), per-bucket buffer memory, and cold/later windows are in the local [metric archive](raw/replay/METRICS.md).

Curation recalculated both scalar reports from the captured Prometheus snapshots,
summary CSVs, and device/OS observations. They match the saved reports apart from
insignificant floating-point summation differences in iostat averages. The baseline
archive matches `git archive` of the stated commit, and the candidate patch hash
matches the settings and run manifest. No new build or replay was performed.
This check validates internal consistency of the retained captures, not independent
trace identity or a performance result on a newer revision.
