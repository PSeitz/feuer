# Lane 02: corrected-scheduler scorer pair

Completed one same-host pair, order **candidate → baseline**. Workload: **original ranges**. Memory **100,000,000,000 bytes**, disk **500,000,000,000 bytes**. Disk behavior: **observed no disk pressure**.

Both tiers use fixed equivalent bytes 256000 and per-byte cost 1. The consolidation preserves scoring arithmetic, not new weighting. Both sources use identical corrected one-task-per-arrival scheduling, 32 Tokio runtime threads and 64 shards. Outstanding lookups are not limited to thread count.

Each case completed **1,606,645 requests exactly once**, returned **2,800,015,397,526 bytes**, exited **0**, and had zero callback/invalid-download/read/write errors and zero failed/canceled writes. Active and unstarted lookup gauges were zero at completion. Pending payload and queued write entries were zero after the five-second observation tail. Disk no-capacity counts below are disk-admission outcomes, not failed source fetches.

First-zone creation failed from insufficient capacity. A deliberate same-shape alternate-zone create succeeded. The user interrupted agent sessions while detached replays continued. The coordinator collected and verified both cases and terminated this sole owned instance. Terraform state is empty. No repeated measurements ran.

## Caller outcomes and public hit rates

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| memory_hit requests | 1,472,151 | 1,472,171 |
| disk_hit requests | 9,276 | 9,382 |
| callback requests | 125,218 | 125,092 |
| memory request hit % | 91.628891 | 91.630136 |
| disk request hit % | 0.577352 | 0.583950 |
| Combined request hit % | 92.206243 | 92.214086 |
| memory byte hit % | 94.434753 | 94.452637 |
| disk byte hit % | 0.813501 | 0.822737 |
| Combined byte hit % | 95.248254 | 95.275374 |
| memory returned GB | 2,644.187613 | 2,644.688392 |
| disk returned GB | 22.778155 | 23.036759 |
| callback returned GB | 133.049630 | 132.290247 |

Candidate − baseline combined request hit difference: **+0.007842 percentage points**. Combined byte hit difference: **+0.027121 percentage points**.

## CPU, memory accounting and unfinished work

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Request-phase CPU seconds | 64 | 66 |
| Average request-phase cores | 0.106779 | 0.110116 |
| Whole-process CPU seconds | 74.410000 | 76.590000 |
| OS maximum RSS GB | 18.758128 | 18.919948 |
| Sampled RSS peak GB | 18.437001 | 18.655080 |
| Memory payload_bytes at finish GB | 94.580425 | 94.755624 |
| Memory used_bytes at finish GB | 99.985788 | 99.984369 |
| Memory idle_bytes at finish GB | 3.027239 | 2.842165 |
| Memory resident_charge_bytes at finish GB | 96.958549 | 97.142204 |
| Sampled pending payload peak GB | 9.988770 | 9.990220 |
| Sampled unstarted lookup peak | 1,477 | 2,958 |
| Sampled active lookup peak | 543 | 549 |
| Execution seconds | 599.366561 | 599.366766 |
| Backlog drain seconds | 0.006646 | 0.006851 |

## Exact observed latencies

Milliseconds, **mean / p50 / p95 / p99 / maximum**. These distributions cover all caller requests, not just disk hits.

| Latency | Baseline | Candidate |
| --- | --- | --- |
| sender_lateness | 0.029793 / 0.030008 / 0.053232 / 0.054273 / 3.681503 | 0.031066 / 0.029992 / 0.053224 / 0.054315 / 3.538943 |
| queue_wait | 0.251457 / 0.038579 / 1.062264 / 3.489959 / 44.563090 | 0.295656 / 0.039122 / 1.107037 / 3.707376 / 55.458438 |
| service | 0.652426 / 0.000959 / 0.005141 / 0.069725 / 862.975559 | 0.671424 / 0.000966 / 0.005164 / 0.089888 / 844.555911 |
| scheduled_to_complete | 0.933677 / 0.070336 / 1.165863 / 4.018817 / 863.728300 | 0.998146 / 0.070578 / 1.231524 / 4.585295 / 845.429075 |

## Disk admission, eviction and measured I/O

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Memory eviction-triggering insertions | 4,389 | 4,456 |
| Disk eviction-triggering insertions | 0 | 0 |
| Write admission queued | 100,709 | 100,636 |
| Write admission queue_full | 23,117 | 23,157 |
| Write admission already_covered | 30 | 34 |
| Write admission redundant | 1,362 | 1,265 |
| Write admission queue_closed | 0 | 0 |
| Write admission canceled | 0 | 0 |
| Write terminal published | 100,452 | 100,373 |
| Write terminal already_covered | 257 | 263 |
| Write terminal no_capacity | 0 | 0 |
| Write terminal failed | 0 | 0 |
| Write terminal canceled | 0 | 0 |
| DataFile read calls | 8,299 | 8,443 |
| DataFile read bytes | 61,036,102,532 | 61,555,962,234 |
| DataFile read errors | 0 | 0 |
| DataFile write calls | 30,808 | 30,752 |
| DataFile write bytes | 98,980,700,160 | 98,761,474,048 |
| DataFile write errors | 0 | 0 |
| Physical SSD read GB | 61.052912 | 61.573014 |
| Physical SSD write GB | 99.013624 | 98.788037 |
| SSD r_await_ms_request_weighted | 5.390635 | 5.305436 |
| SSD w_await_ms_request_weighted | 0.140239 | 0.141215 |
| SSD aqu-sz | 4.303246 | 4.274356 |
| SSD %util | 5.012268 | 4.998842 |
| Observation-tail DataFile read calls | 0 | 0 |
| Observation-tail DataFile read bytes | 0 | 0 |
| Observation-tail DataFile write calls | 139 | 139 |
| Observation-tail DataFile write bytes | 67,416,064 | 67,416,064 |
| Observation-tail CPU seconds | 0 | 0 |

## All histogram families

Raw units from the metric name. Each cell is **count / mean / p50 interval / p95 interval / p99 interval**. Quantile intervals are `(lower, upper]`, not exact percentiles. Counter deltas exclude opening and the observation tail.

| Histogram | Baseline | Candidate |
| --- | --- | --- |
| `feuer_disk_io_duration_seconds{operation="read",outcome="success"}` | 8,299 / 0.084098 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] | 8,443 / 0.084208 / (0.001024, 0.002048] / (0.524288, 1.048576] / (0.524288, 1.048576] |
| `feuer_disk_io_duration_seconds{operation="write",outcome="success"}` | 30,669 / 0.001090 / (0.000128, 0.000256] / (0.002048, 0.004096] / (0.016384, 0.032768] | 30,613 / 0.001088 / (0.000128, 0.000256] / (0.002048, 0.004096] / (0.016384, 0.032768] |
| `feuer_disk_lookup_duration_seconds` | 9,276 / 0.111598 / (0.002048, 0.004096] / (0.524288, 1.048576] / (0.524288, 1.048576] | 9,382 / 0.113484 / (0.002048, 0.004096] / (0.524288, 1.048576] / (0.524288, 1.048576] |
| `feuer_disk_read_size_bytes` | 8,299 / 7,354,633.393421 / (262,144, 524,288] / (33,554,432, 67,108,864] / (33,554,432, 67,108,864] | 8,443 / 7,290,768.948715 / (262,144, 524,288] / (33,554,432, 67,108,864] / (33,554,432, 67,108,864] |
| `feuer_disk_write_queue_duration_seconds` | 100,709 / 0.114000 / (0.002048, 0.004096] / (0.524288, 1.048576] / (1.048576, 2.097152] | 100,636 / 0.114108 / (0.002048, 0.004096] / (0.524288, 1.048576] / (1.048576, 2.097152] |
| `feuer_lookup_duration_seconds{outcome="callback"}` | 125,218 / 0.000033 / (0.000004, 0.000008] / (0.000032, 0.000064] / (0.000128, 0.000256] | 125,092 / 0.000038 / (0.000004, 0.000008] / (0.000032, 0.000064] / (0.000256, 0.000512] |
| `feuer_lookup_duration_seconds{outcome="disk_hit"}` | 9,276 / 0.112204 / (0.002048, 0.004096] / (0.524288, 1.048576] / (0.524288, 1.048576] | 9,382 / 0.114139 / (0.002048, 0.004096] / (0.524288, 1.048576] / (0.524288, 1.048576] |

## Startup and later windows

Nearest one-second samples, approximate windows. Cells show **memory hits / disk hits / callbacks / read GB / write GB / unstarted / active**. Windows are not independent repetitions.

| Window seconds | Baseline | Candidate |
| --- | --- | --- |
| 0–30 | 81,259 / 1 / 39,610 / 0.052092 / 34.271699 / 0 / 0 | 81,356 / 1 / 39,513 / 0.055107 / 34.715226 / 0 / 0 |
| 30–90 | 117,356 / 19 / 22,362 / 0.123406 / 13.295657 / 0 / 0 | 117,359 / 15 / 22,363 / 0.117797 / 13.297746 / 0 / 0 |
| 90–180 | 279,445 / 939 / 18,577 / 5.414987 / 14.352863 / 0 / 0 | 279,436 / 943 / 18,582 / 5.642202 / 14.170444 / 0 / 0 |
| 180–599.367 | 994,091 / 8,317 / 44,669 / 55.445618 / 36.993065 / 0 / 0 | 994,020 / 8,423 / 44,634 / 55.740856 / 36.510642 / 0 / 0 |

## Scope and evidence

One pair does not establish small speedups, regressions or statistical significance. Outcomes can vary with concurrent admission even when scores preserve the same formula. Synthetic immediate zero-filled misses, cold start and absent source coalescing/other-cache traffic do not reproduce Pomsky source behavior. Original and replay disk-hit populations differ. No disk delay was injected.

Useful synthetic payload bytes are not physical residency. Capacity is cache accounting, not an RSS cap. Sampled peaks can miss spikes. CPU phase counts have one-second resolution. OS totals include input loading/reporting. Device counters cover the whole device. DataFile calls are not device requests. The observation tail does not prove forced draining, flushing or durability.

- [Settings and executable identities](settings.json)
- [Every paired captured metric series](../raw/agents/lane-02/coordinator-finish/METRICS-COMPARISON.md)
- [Baseline full inventory and histograms](../raw/agents/lane-02/coordinator-finish/artifacts/results/baseline/METRICS.md)
- [Candidate full inventory and histograms](../raw/agents/lane-02/coordinator-finish/artifacts/results/candidate/METRICS.md)
- Private raw Prometheus samples, snapshots, per-bucket buffer bytes, packing metrics, process/device evidence and validated source archives: `../raw/agents/lane-02/coordinator-finish/artifacts/`.
- Only this summary and small settings are intended for Git. No commits or staging were performed.
