# Testing write chunking, not assuming its effect

## Question

Does splitting a 1 MiB write into sixteen 64 KiB requests improve mixed-read
latency even when all chunks can be outstanding together? Compare against
1 MiB/QD1 and 64 KiB/QD1 rather than inferring behavior from outstanding bytes.

## Findings

The run finished in **190 seconds**, including initialization. Each range below
covers the two independent short rounds, not a confidence interval:

| Writer | Read MB/s | Write MB/s | 4 KiB read p99 ms |
| --- | ---: | ---: | ---: |
| None | 3,878.7–3,879.1 | 0 | 5.528–5.544 |
| 1 MiB / QD1 | 3,241.2–3,313.5 | 1,157.3–1,191.8 | 6.684–6.831 |
| 64 KiB / QD16, batched | 3,252.4–3,313.5 | 1,159.8–1,181.7 | 6.736–6.853 |
| 64 KiB / QD16, rolling | 3,274.3–3,291.1 | 1,162.6–1,183.7 | 6.731–6.794 |
| 64 KiB / QD1 | 3,297.8–3,300.3 | 1,171.0–1,172.8 | 6.617–6.629 |

**Submitting sixteen 64 KiB chunks together did not show a repeatable benefit
over one 1 MiB request.** Batched chunks improved small-read p99 in round 1 but
worsened it in round 2; throughput ranges overlap. The rolling QD16 variant was
also similar, rather than a clear improvement.

**64 KiB/QD1 preserved write throughput and showed a small latency improvement**
(about 1–3% lower small-read p99 than the corresponding 1 MiB/QD1 round). It did
not remove the interference: small-read p99 remained about 20% above read-only,
and aggregate reads about 15% below read-only. Two short rounds are not enough
to establish a reliable small advantage.

Thus, for this mixed reader and device, chunking alone did not produce a large
read-latency improvement at roughly 1.17 GB/s writes. This is now a measured
comparison, not an inference from equal maximum outstanding bytes. It does not
rule out benefits with other write sizes, reader mixtures, devices, or an
explicit scheduler that delays new chunks in response to read demand.

## Validation outcome

All **12 fio invocations** succeeded: two initialization passes and ten measured
cases. Both batched writers reported **100%** in the 16-request submission and
completion buckets. Achieved small-read counts were **49.77–50.32%**. Per-size
log counts and combined latency means matched fio JSON; ramp/drain accounting
differences stayed within QD32. Raw evidence was archived, local CSV/report
regeneration matched the instance output, and the dedicated SSD data directory
was removed. Production scheduling is unchanged.

## Workload

- Same idle EC2 `m8gd.8xlarge` host (`m8g-32cpu-local-ssd`), ext4 on local
  NVMe instance store `/dev/nvme1n1`. No concurrent benchmark was observed.
- fio 3.36, libaio, O_DIRECT; same reader and instrumentation as the preceding
  [mixed-size screening](../local-ssd-mixed-read-sizes-20260926/REPORT.md).
- One random reader at **QD32**: `bssplit=4k/50:1m/50`, 50/50 **by request
  count**, not bytes or in-flight occupancy. About 99.61% of bytes are large
  reads. Random offsets with replacement, 4 KiB alignment, `randrepeat=0`.
- Separate **sequential writer with no write-rate cap**:
  - No writer (read-only control).
  - **1 MiB/QD1:** one outstanding 1 MiB request.
  - **64 KiB/QD16 batched:** submit 16 adjacent 64 KiB chunks in one batch;
    retrieve 16 completions before the next batch. Uses
    `iodepth_batch_submit=16`, `iodepth_batch_complete_min=16`, and
    `iodepth_batch_complete_max=16`. This directly tests submitting all chunks
    together, with the same maximum 1 MiB payload as 1 MiB/QD1.
  - **64 KiB/QD16 rolling:** default single-request submission/reaping,
    continuously refilling up to QD16. Same maximum 1 MiB outstanding bytes,
    but no group-completion boundary.
  - **64 KiB/QD1:** one chunk outstanding at a time.
- Fully initialized **8 GiB read / 4 GiB write files**. Initialization uses
  1 MiB direct writes and ending fsync. No per-write fsync during measurement.
- Two shuffled rounds, seeded RNG `20260926`, **3 seconds warmup + 15 seconds
  measurement** per case. Approximately three minutes including initialization.
- Per-read completion latency logs include actual request size. No averaging;
  262,144 preallocated log entries. Report nearest-rank p99 separately by size.
  Aggregate throughput comes from fio JSON. CPU totals in CSV use 100% per core.
- Logs and raw fio JSON are on EBS; `iostat -dxm -y 1` monitors the devices.

The kernel may split or merge requests (`max_sectors_kb=128`); application QD
is not physical device QD. That does not invalidate the comparison: request
shape and batching are precisely what this experiment varies. A fixed maximum
payload does not guarantee the same average outstanding bytes or write rate.

## Validation and reproduction

The runner reuses the sibling
[`../local-ssd-random-read-write-20260925/run_bench.py`](../local-ssd-random-read-write-20260925/run_bench.py)
for fio execution/validation. Keep that sibling path when copying. On the same
idle host, in a fresh EBS output directory with >20 GiB free on the SSD:

```sh
python3 -u run_bench.py > progress.log 2>&1
python3 summarize.py
```

Host, mount/device identity, fresh paths, initialized bytes, job errors, and
measurement durations are checked. Success removes only the dedicated SSD
files/directory before writing `DONE`; failures retain evidence. Do not use
Python `-O` or overlap storage benchmarks.

The summarizer checks all ten measured cases, requested writer sizes/QDs, no
rate caps, and >=99% in the fio 16-request submission/completion buckets for
batched writers. Per-size log counts and combined means must match fio JSON
completion-latency statistics. Differences from bandwidth counters at ramp/drain
boundaries must fit within QD32. The achieved request mixture must be 49–51%
small reads.

Raw evidence paths:

- Instance: `/home/ubuntu/ssd-bench-results/local-ssd-write-chunks-20260926/`
- Local archive: `~/Development/benchmarks/results/local-ssd-write-chunks-20260926/`

## Limits

Short low-occupancy screening, not aged/full-drive steady state. O_DIRECT avoids
the host page cache, not device caches. Smaller files and shorter runs differ
from the earlier long sweeps. All cases use identical per-I/O reader logging.
Latency is fio completion latency, not isolated device service time or Feuer
end-to-end latency. This is not an implementation of Feuer chunking, RMW,
read prioritization, or adaptive scheduling; no production code is changed.
