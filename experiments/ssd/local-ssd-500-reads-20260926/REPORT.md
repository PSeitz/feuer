# Actual latency to complete 500 random 4 KiB reads

Measured **2026-09-26** on `m8g-32cpu-local-ssd` (EC2 `m8gd.8xlarge`).
The successful run took **33 seconds**, including initialization. This measures
**first submission to last completion of all 500 reads**, not throughput-derived
estimates or the percentile of individual reads.

Two rounds, each with 100 warmup batches and **1,000 measured batches per case**.
Ranges below cover the two rounds; they are not confidence intervals.

| Maximum outstanding reads | Background writes | Mean batch latency ms | Batch p99 ms |
| ---: | --- | ---: | ---: |
| 32 | None | 1.499–1.504 | 1.629–1.636 |
| 64 | None | 1.053–1.056 | 1.242–1.262 |
| 32 | Uncapped 1 MiB writes, QD32 | 3.743–4.073 | 4.928–4.939 |
| 64 | Uncapped 1 MiB writes, QD32 | 2.545–2.607 | 2.971–3.318 |

**Answer:** approximately **1–1.5 ms** to complete 500 parallel random 4 KiB
reads without competing I/O. Heavy writes increase that to about **2.5–4.1 ms
on average**, depending on read concurrency; QD32 batch p99 is about **4.94 ms**.
These are measurements on this device/workload, not a latency guarantee.

## Method

- ext4 on local NVMe instance store `/dev/nvme1n1`, not EBS. Host/device were idle
  before launch. Fully initialized **8 GiB read / 4 GiB write** files, with ending
  fsync after direct-write initialization. No formatting or raw-device writes.
- [`batch_reads.c`](batch_reads.c) uses Linux native AIO syscalls with O_DIRECT.
  Each batch contains exactly **500 random, 4 KiB-aligned 4 KiB reads** (1.95 MiB).
  Random offsets are sampled with replacement; no 1 MiB reads are included.
- A batch is fully drained before the next begins. Maintain at most QD32 or
  QD64 by reusing a slot only after its completion. Submit one request per
  `io_submit`; retrieve 1 through QD completions per `io_getevents` call.
- `CLOCK_MONOTONIC_RAW` spans the initial submissions through processing the
  final completion. Includes submission, completion handling, and refilling
  the queue; excludes opening files, buffer allocation, random-offset generation,
  printing, and batch setup. Aligned buffers are preallocated and faulted in.
- Every completion must return exactly 4,096 bytes with no error. Each measured
  case logs 1,000 batch times. Percentiles are nearest-rank over these batches.
- The separate background writer uses fio 3.36, libaio, O_DIRECT, sequential
  **1 MiB writes at QD32**, no rate cap. Start it three seconds before read tests
  and stop it only after both QD cases finish. It remains running throughout.
  Whole-writer-invocation throughput was 1,995 / 2,031 MB/s; this includes the
  three-second write-only lead-in and is **not** per-read-case write throughput.
- Round 1: QD32 then QD64; round 2 reverses QD order. No-writer cases precede
  writer cases within each round. Short runs on a low-occupancy device, not an
  aged/full-device or sustained-load test. O_DIRECT bypasses host page cache,
  not device caches. Batch p99 describes this sample, not a guaranteed bound.
- This is a native-AIO device benchmark, **not Feuer API latency**: no cache
  lookup, admission wait, result copying, or Tokio scheduling. No production
  code changed. No sequential-QD1 or mixed-read-size case was run here.

## Validation and evidence

All **8,000 measured batches** completed successfully: **4,000,000 measured
read requests**, plus warmups. Raw CSV means and percentiles were independently
recomputed and matched the summary. Both background writers returned zero fio
job errors. Only the dedicated SSD files/directory were removed after success.

An initial attempt completed its first four read cases but stopped in Python
while parsing the writer result: fio prepended a SIGINT shutdown notice to JSON.
The parser was fixed and the entire experiment rerun. The table uses only the
complete rerun; initial evidence is preserved separately, not silently discarded.

Reproduce on the same idle host in a fresh EBS output directory:

```sh
python3 -u run_bench.py > progress.log 2>&1
```

The runner compiles C with `cc -O2 -Wall -Wextra -Werror`, checks host/device and
fresh paths, initializes the files, runs the cases, validates outputs, and creates
`summary.csv` and `DONE`. Requires >20 GiB free on the SSD, fio, Python, and Linux
AIO headers/compiler. Do not use Python `-O`. The runner imports the sibling
[`../local-ssd-random-read-write-20260925/run_bench.py`](../local-ssd-random-read-write-20260925/run_bench.py)
for initialization and fio checks; retain that relative path when copying.

Raw evidence:

- Instance: `/home/ubuntu/ssd-bench-results/local-ssd-500-reads-20260926/`
- Local archive: `~/Development/benchmarks/results/local-ssd-500-reads-20260926/`
- Initial attempt: the same paths with `-attempt1` appended.

See [`summary.csv`](summary.csv) for each round's mean, p50, p95, p99, and maximum.
