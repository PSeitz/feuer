# Disk-bypass decisions: Yahoo benchmark evidence

## Question and scope

Does the saved benchmark establish whether disk-read pressure persists long
enough to justify arrival-only bypass checks instead of reconsidering earlier
waiters when the queue grows?

Saved benchmark: `byoc-yahoo-sim/user-sim/reports/sweep_20261018T160235Z_cold`.
This review uses `pascal-feuer`, 14 searchers, image
`cp-rc-0.1.35-feuer-21`. Measured stages ran October 18, 2026,
**16:25:30–17:23:08 UTC**, following a ten-minute warmup. QPS is the configured
stage rate, not the number of split reads. Different stages use different seeds.
This is not a controlled QPS-only workload comparison.

**The deployed image does not contain the new payload-read admission/bypass queue.**
The benchmark's recorded digest, `sha256:1d5d24320d66412dfd4ce9e064ad4078119e4b697409da4aaa0f8f8dc3255c49`,
matches the live pod used for the schema check. Its `quickwit_build_info` identifies
Pomsky commit `85fe717a88aee29ee699270f84a818c75415a6ab`, tag `cp-rc-0.1.35-feuer-21`.
That commit's `quickwit/Cargo.lock` pins Feuer
`4e7a7d12478c645d4d534ca6ef0fc33d87d279ca`. That tree has no
`uring/read_admission.rs`, `PayloadReadQueue`, `acquire_payload_read_permit`, or
`should_bypass_disk`. Its 64-read limit applies to existing io_uring read pieces,
not to 64 concurrent whole payload reads plus a 64-waiter bypass trigger.
The benchmark therefore cannot be treated as a measurement of the new queue.
Pinned lockfile and source-check evidence are retained under `raw/`.

## What the benchmark measures

| QPS | Read TB | Mean read GB/s/searcher | Maximum sampled pending search-thread tasks |
| ---: | ---: | ---: | ---: |
| 1 | 3.534 | 0.840 | 506 |
| 2 | 14.669 | 3.260 | 360 |
| 3 | 7.685 | 1.755 | 514 |
| 4 | 14.000 | 3.232 | 417 |
| 5 | 12.176 | 2.759 | 382 |
| 6 | 5.495 | 1.301 | 438 |
| 7 | 8.097 | 1.838 | 906 |
| 8 | 11.333 | 2.658 | 560 |
| 9 | 11.853 | 2.660 | 547 |
| 10 | 13.567 | 2.941 | 515 |

Read figures are saved `feuer_disk_io_bytes_total{operation:read}` counts,
summed across all Feuer cache labels in the workspace. Mean divides each stage's
reported total by its actual measurement duration and 14 nodes. They are not
NVMe utilization or waiting-queue depth.

Saved per-node samples cover all 14 expected nodes, one series per node, with no
null values in these selected read series. The API reports `interval: 2` but
actual point timestamps are spaced **10 seconds**. `results.json` preserves both.
The offline sample-rate analysis divides count samples by observed ten-second
spacing, not the misleading interval field. Sampling/flush alignment can produce
peaks above physical device bandwidth, so do not interpret those as instantaneous
SSD throughput or as queue-pressure durations.

Within the 10-QPS stage, the fleet-mean ten-second count rate ranges from
**0.502 to 5.436 GB/s/searcher**. The read workload is not a flat throughput
plateau. That does **not** prove the payload waiting queue drains or crosses 64.

The saved pending-task metric is `quickwit_thread_pool_pending_tasks{pool:search}`.
Its maxima are **not** disk-admission waiters. Neither those maxima nor disk byte
rates can be used to count `PayloadReadQueue` episodes.

## Successful DataFile read latency and inter-stage recovery

Historical duration distributions for the same benchmark window from
`byoc-sim.datadoghq.com`, filtered to this workspace and `operation:read`, returned
28 series: 14 split-range and 14 predicate cache series. There were no
partial-request read-duration series. `cloudprem_node_id` and `kube_pod_name`
identified the expected nodes.

Mean latency is the sum of distribution durations divided by observation count,
using matching non-null ten-second buckets timestamped inside each stage. It is
not an average of per-pod means. Boundary attribution is approximate to a bucket.

| QPS | All-cache mean read ms | Split-range mean read ms |
| ---: | ---: | ---: |
| 1 | 456.1 | 458.1 |
| 2 | 407.7 | 408.8 |
| 3 | 362.8 | 378.9 |
| 4 | 338.3 | 341.7 |
| 5 | 235.2 | 238.8 |
| 6 | 216.2 | 227.6 |
| 7 | 138.3 | 147.3 |
| 8 | 249.4 | 257.6 |
| 9 | 177.4 | 194.7 |
| 10 | 180.7 | 186.8 |

In the deployed source, `DataFile::measure_io` times the awaited read path,
including read-queue work and buffer handling. This is **not pure SSD service
latency**, not request latency, and not a separate payload-admission waiting time.
Latency does not rise monotonically with QPS. Seeds, cache warming, and read-size
mix change, so this is not evidence that increasing QPS improves disk latency.

There is direct evidence of **read-throughput recovery between stages**. Full-window
read-counter samples returned all 14 nodes. Conservatively ignoring ten seconds
at both edges of each inter-stage gap, all seven gaps after QPS 2 through QPS 8
have interior samples with **exactly zero completed read bytes across all 14 nodes**.
Examples:

- After QPS 2: 16:36:30 and 16:36:40 UTC, both zero.
- After QPS 6: 16:59:30, 16:59:40 and 16:59:50 UTC, all zero.
- After QPS 9: 17:17:20 UTC, 740,452 bytes fleet-wide in the ten-second bucket,
  about 5.3 KB/s/searcher, nearly zero but not zero.
- After the final stage: 17:23:40 and 17:23:50 UTC, zero.

The first gap is too short to contain a sample under this conservative interior
rule. `results.json` records all gap durations and samples. Zero completed bytes
means observed read activity subsides, not proof that every pending read has
finished. It cannot distinguish an empty queue from a stalled queue. Counter
reporting delay and sampling prevent an exact drain-time estimate. No claim of
payload-depth recovery or persistence follows from these samples.

Historical queries, each grouped by
`{cloudprem_node_id,kube_pod_name,operation,cache,host}` for latency and without
`cache` for byte throughput, use the same independently validated workspace and
`operation:read` scope. The full responses retain exact query strings:

```text
sum:feuer_disk_io_duration_seconds{...}.rollup(sum,10)
count:feuer_disk_io_duration_seconds{...}.rollup(sum,10)
sum:feuer_disk_io_bytes_total{...}.as_count().rollup(sum,10)
```

Duration sums/counts are retained in `raw/latency-{sum,count}-full.json`, broad
and narrowed discovery in `raw/latency-*.json`, and inter-stage read samples in
`raw/disk-read-full.json`. The full query window is 16:25:30–17:24:00 UTC.

## What cannot be answered from the existing metrics

The saved benchmark report has no payload-read queue-depth series, queue-arrival
counter, permit-acquisition counter, or per-waiter enqueue/dequeue events. A
read-only post-benchmark `/metrics` schema check also found disk **write** queue
metrics, but no payload-read waiting-depth or permit counters. That schema probe
is not a historical time-series observation.

Therefore these remain unknown:

- Distribution of payload queue depth and time at or above 64.
- Number/duration of threshold episodes, and time to drain/recover.
- Arrival rate versus permit-service rate for that queue.
- Actual earlier waiters that an arrival-only pressure decision would miss.

The raw lookup/I/O counters and latency distributions do not identify queue
transitions or individual waiters. Completed disk calls also differ from queue
arrivals because of sharing, cancellation, and different I/O paths. The reviewed
source at `d965556` has no read-admission instrumentation to bridge this gap.

## Implication for the proposed simplification

**This benchmark does not establish that the waiting queue stays high.**
It shows substantial, variable read traffic, high DataFile read latency, and
read activity subsiding between stages. None is enough to choose queue-wide
versus arrival-only bypass behavior. More fundamentally, the deployed version
has no new payload-read admission queue to measure.

From queue semantics at `d965556`, not measurements:

- If the caller decides to bypass only when its arrival sees at least 64 waiting,
  the first clean 63-to-64 crossing leaves **63 earlier waiters** without a
  pressure-time decision under arrival-only checking. The reviewed queue-wide behavior
  schedules all 64. At later crossings, zero to 63 older waiters may still be
  unchecked because callbacks run at most once.
- Calling the callback on every arrival with queue length means those older
  callers did receive a callback, but at a lower queue depth. The lost behavior is
  **reconsideration when pressure later rises**, not necessarily callback omission.
- If one high-depth episode persists through very many arrivals, the initial
  older cohort can be a small fraction. Repeated build-up/drain cycles repeat
  that cohort problem. This benchmark cannot distinguish the cases.
- Bypass decisions themselves change the queue. If every arrival reaching 64
  bypasses immediately, the queue can fall back to 63 while the older waiters
  remain. Even a high queue in a no-bypass benchmark would not establish a high
  queue once the new policy is enabled.

To resolve the question exactly, record enqueue/dequeue/permit events and the
number of earlier unchecked waiters at each crossing, with timestamps and queue
identity. A sampled gauge alone can miss brief crossings and cannot identify
older cohorts. This report does not measure those events.

## Reproduction and retained evidence

Run `python3 experiments/cache/disk-bypass-queue-review/analyze.py` to re-read
existing benchmark artifacts without traffic. `settings.json` retains the source
path and hashes. `results.json` retains queries, timestamps, coverage and stage
statistics. `raw/` contains copies of the selected query responses, derived
count-rate time series, and the post-benchmark schema probe and is Git-ignored.
Recomputation requires both that local evidence and the external benchmark
report at the path in `settings.json`. They are not included in Git.

Datadog org: `byoc-sim.datadoghq.com`. The exact saved read query is:

```text
sum:feuer_disk_io_bytes_total{kube_cluster_name:yahoo-sim,cloudprem_node_id:cloudprem-ws-pascal-feuer-searcher-*,operation:read} by {kube_cluster_name,cloudprem_node_id,node_id,kube_pod_name,pod_name,container_name,host,version,operation,op}.as_count()
```

Each stage retains its own independently discovered and validated identity
population in the original benchmark's `metric-evidence/` directory. This review
checks the returned `tag_set`, not a dictionary made from `scope`, because
`scope` appends the wildcard filter with a duplicate `cloudprem_node_id` key.
