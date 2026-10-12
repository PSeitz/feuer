# Lane 01: corrected-scheduler scorer pair

Completed one same-host pair, order **baseline → candidate**. Workload: **full objects**. Memory **100,000,000,000 bytes**, disk **500,000,000,000 bytes**. Disk behavior: **observed no disk pressure**.

Both tiers use fixed equivalent bytes 256000 and per-byte cost 1. The consolidation preserves scoring arithmetic, not new weighting. Both sources use identical corrected one-task-per-arrival scheduling, 32 Tokio runtime threads and 64 shards. Outstanding lookups are not limited to thread count.

Each case completed **1,606,645 requests exactly once**, returned **2,800,015,397,526 bytes**, exited **0**, and had zero callback/invalid-download/read/write errors and zero failed/canceled writes. Active and unstarted lookup gauges were zero at completion. Pending payload and queued write entries were zero after the five-second observation tail. Disk no-capacity counts below are disk-admission outcomes, not failed source fetches.

First-zone creation failed from insufficient capacity. A deliberate same-shape alternate-zone create succeeded. The user interrupted agent sessions while detached replays continued. The coordinator collected and verified both cases and terminated this sole owned instance. Terraform state is empty. No repeated measurements ran.

## Caller outcomes and public hit rates

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| memory_hit requests | 1,478,328 | 1,478,371 |
| disk_hit requests | 2,362 | 2,400 |
| callback requests | 125,955 | 125,874 |
| memory request hit % | 92.013357 | 92.016033 |
| disk request hit % | 0.147014 | 0.149380 |
| Combined request hit % | 92.160371 | 92.165413 |
| memory byte hit % | 95.133552 | 95.132567 |
| disk byte hit % | 0.225750 | 0.226722 |
| Combined byte hit % | 95.359302 | 95.359289 |
| memory returned GB | 2,663.754107 | 2,663.726523 |
| disk returned GB | 6.321024 | 6.348259 |
| callback returned GB | 129.940267 | 129.940615 |

Candidate − baseline combined request hit difference: **+0.005042 percentage points**. Combined byte hit difference: **-0.000012 percentage points**.

## CPU, memory accounting and unfinished work

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Request-phase CPU seconds | 43 | 44 |
| Average request-phase cores | 0.071743 | 0.073412 |
| Whole-process CPU seconds | 52.870000 | 53.840000 |
| OS maximum RSS GB | 11.854242 | 11.862901 |
| Sampled RSS peak GB | 11.841384 | 11.851006 |
| Memory payload_bytes at finish GB | 97.063447 | 97.024343 |
| Memory used_bytes at finish GB | 99.263521 | 99.248406 |
| Memory idle_bytes at finish GB | 0.205554 | 0.221250 |
| Memory resident_charge_bytes at finish GB | 99.057968 | 99.027156 |
| Sampled pending payload peak GB | 9.985975 | 10.007735 |
| Sampled unstarted lookup peak | 1,220 | 21 |
| Sampled active lookup peak | 568 | 579 |
| Execution seconds | 599.360123 | 599.360054 |
| Backlog drain seconds | 0.000208 | 0.000139 |

## Exact observed latencies

Milliseconds, **mean / p50 / p95 / p99 / maximum**. These distributions cover all caller requests, not just disk hits.

| Latency | Baseline | Candidate |
| --- | --- | --- |
| sender_lateness | 0.029900 / 0.030003 / 0.053258 / 0.054135 / 3.483570 | 0.029632 / 0.029961 / 0.053230 / 0.053989 / 2.651760 |
| queue_wait | 0.185883 / 0.034964 / 0.812802 / 2.587716 / 26.811289 | 0.226367 / 0.037868 / 0.980176 / 3.138303 / 26.671535 |
| service | 0.248208 / 0.001311 / 0.003706 / 0.028068 / 823.343482 | 0.252623 / 0.001309 / 0.003762 / 0.028273 / 833.223351 |
| scheduled_to_complete | 0.463991 / 0.068065 / 0.852042 / 2.755480 / 829.303730 | 0.508623 / 0.069459 / 1.018263 / 3.237629 / 836.902623 |

## Disk admission, eviction and measured I/O

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Memory eviction-triggering insertions | 2,290 | 2,242 |
| Disk eviction-triggering insertions | 0 | 0 |
| Write admission queued | 103,503 | 103,955 |
| Write admission queue_full | 21,051 | 20,554 |
| Write admission already_covered | 31 | 28 |
| Write admission redundant | 1,370 | 1,337 |
| Write admission queue_closed | 0 | 0 |
| Write admission canceled | 0 | 0 |
| Write terminal published | 103,503 | 103,955 |
| Write terminal already_covered | 0 | 0 |
| Write terminal no_capacity | 0 | 0 |
| Write terminal failed | 0 | 0 |
| Write terminal canceled | 0 | 0 |
| DataFile read calls | 2,362 | 2,400 |
| DataFile read bytes | 6,321,023,851 | 6,348,259,379 |
| DataFile read errors | 0 | 0 |
| DataFile write calls | 30,046 | 30,338 |
| DataFile write bytes | 99,429,986,304 | 98,919,501,824 |
| DataFile write errors | 0 | 0 |
| Physical SSD read GB | 6.325862 | 6.353183 |
| Physical SSD write GB | 99.457294 | 98.954056 |
| SSD r_await_ms_request_weighted | 4.289371 | 4.099817 |
| SSD w_await_ms_request_weighted | 0.128238 | 0.127466 |
| SSD aqu-sz | 0.504812 | 0.491160 |
| SSD %util | 3.516297 | 3.532353 |
| Observation-tail DataFile read calls | 0 | 0 |
| Observation-tail DataFile read bytes | 0 | 0 |
| Observation-tail DataFile write calls | 144 | 148 |
| Observation-tail DataFile write bytes | 76,857,344 | 76,873,728 |
| Observation-tail CPU seconds | 0 | 0 |

## All histogram families

Raw units from the metric name. Each cell is **count / mean / p50 interval / p95 interval / p99 interval**. Quantile intervals are `(lower, upper]`, not exact percentiles. Counter deltas exclude opening and the observation tail.

| Histogram | Baseline | Candidate |
| --- | --- | --- |
| `feuer_disk_io_duration_seconds{operation="read",outcome="success"}` | 2,362 / 0.166877 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] | 2,400 / 0.167132 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] |
| `feuer_disk_io_duration_seconds{operation="write",outcome="success"}` | 29,902 / 0.001000 / (0.000128, 0.000256] / (0.002048, 0.004096] / (0.016384, 0.032768] | 30,190 / 0.000990 / (0.000128, 0.000256] / (0.002048, 0.004096] / (0.016384, 0.032768] |
| `feuer_disk_lookup_duration_seconds` | 2,362 / 0.167075 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] | 2,400 / 0.167329 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] |
| `feuer_disk_read_size_bytes` | 2,362 / 2,676,132.028366 / (262,144, 524,288] / (8,388,608, 16,777,216] / (8,388,608, 16,777,216] | 2,400 / 2,645,108.074583 / (262,144, 524,288] / (8,388,608, 16,777,216] / (8,388,608, 16,777,216] |
| `feuer_disk_write_queue_duration_seconds` | 103,499 / 0.107296 / (0.001024, 0.002048] / (0.524288, 1.048576] / (1.048576, 2.097152] | 103,951 / 0.106559 / (0.001024, 0.002048] / (0.524288, 1.048576] / (1.048576, 2.097152] |
| `feuer_lookup_duration_seconds{outcome="callback"}` | 125,955 / 0.000013 / (0.000002, 0.000004] / (0.000032, 0.000064] / (0.000128, 0.000256] | 125,874 / 0.000014 / (0.000002, 0.000004] / (0.000032, 0.000064] / (0.000128, 0.000256] |
| `feuer_lookup_duration_seconds{outcome="disk_hit"}` | 2,362 / 0.167165 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] | 2,400 / 0.167417 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] |

## Startup and later windows

Nearest one-second samples, approximate windows. Cells show **memory hits / disk hits / callbacks / read GB / write GB / unstarted / active**. Windows are not independent repetitions.

| Window seconds | Baseline | Candidate |
| --- | --- | --- |
| 0–30 | 81,212 / 0 / 39,658 / 0 / 34.505073 / 0 / 0 | 81,244 / 0 / 39,626 / 0 / 33.960956 / 0 / 0 |
| 30–90 | 117,192 / 0 / 22,545 / 0 / 13.284647 / 0 / 0 | 117,183 / 0 / 22,554 / 0 / 13.296153 / 0 / 0 |
| 90–180 | 280,337 / 57 / 18,567 / 0.074148 / 16.098451 / 0 / 0 | 280,328 / 54 / 18,579 / 0.069629 / 16.129880 / 0 / 0 |
| 180–599.36 | 999,587 / 2,305 / 45,185 / 6.246875 / 35.464958 / 0 / 0 | 999,616 / 2,346 / 45,115 / 6.278631 / 35.455640 / 0 / 0 |

## Scope and evidence

One pair does not establish small speedups, regressions or statistical significance. Outcomes can vary with concurrent admission even when scores preserve the same formula. Synthetic immediate zero-filled misses, cold start and absent source coalescing/other-cache traffic do not reproduce Pomsky source behavior. Original and replay disk-hit populations differ. No disk delay was injected.

Useful synthetic payload bytes are not physical residency. Capacity is cache accounting, not an RSS cap. Sampled peaks can miss spikes. CPU phase counts have one-second resolution. OS totals include input loading/reporting. Device counters cover the whole device. DataFile calls are not device requests. The observation tail does not prove forced draining, flushing or durability.

- [Settings and executable identities](settings.json)
- [Every paired captured metric series](../raw/agents/lane-01/coordinator-finish/METRICS-COMPARISON.md)
- [Baseline full inventory and histograms](../raw/agents/lane-01/coordinator-finish/artifacts/results/baseline/METRICS.md)
- [Candidate full inventory and histograms](../raw/agents/lane-01/coordinator-finish/artifacts/results/candidate/METRICS.md)
- Private raw Prometheus samples, snapshots, per-bucket buffer bytes, packing metrics, process/device evidence and validated source archives: `../raw/agents/lane-01/coordinator-finish/artifacts/`.
- Only this summary and small settings are intended for Git. No commits or staging were performed.
