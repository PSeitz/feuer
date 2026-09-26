# Writer queue depth during random reads

## Question

Does limiting outstanding writes protect small random reads, and is 64 KiB a
useful size threshold for selectively throttling writes? The previous benchmark
varied write **rates**, not queue depth; it did not establish that four
outstanding writes was a useful limit.

This experiment isolates writer QD. It does **not** implement or validate an
adaptive scheduler in Feuer.

## Findings and policy implications

- **Writer QD is not an effective bandwidth throttle for this workload.**
  Standalone 1 MiB writes achieve 1,828.5 MB/s at QD1 and 1,829.1 MB/s at QD32.
- With 16 KiB reads, write QD1/4/32 produces **1,558.0 / 1,581.5 / 1,560.7 MB/s**
  reads versus **3,821.3 MB/s** without writes. Read p99 is **0.709 ms** for all
  three, versus **0.289 ms** without writes. Four outstanding writes is not a
  measured protection threshold; even one continuously outstanding write is too
  much if the goal is near-baseline small-read throughput.
- Rate caps change the tradeoff: 16 KiB reads reach **3,235.2 MB/s** at 180 MB/s
  writes and **2,588.6 MB/s** at 450 MB/s writes. These are still approximately
  **15% and 32% below baseline**; neither is a no-interference setting.
- **32 KiB reads are sensitive too:** 3,878.7 MB/s without writes versus
  2,496.4 MB/s at write QD4 (**36% lower**). This supports testing `<64 KiB` as a
  small-read trigger, but does not establish an exact boundary.
- Large reads are not unaffected: at write QD4, 64 KiB and 1 MiB read throughput
  falls approximately **17% and 14%**. At 64 KiB, p99 rises from **0.643 to
  1.483 ms**. Even the 450 MB/s control preserves throughput but raises p99 to
  **1.204 ms**.

For the proposed policy, test **size-triggered write-rate limiting**, not merely
a smaller write-QD cap. Separate read/write queues or budgets address scheduling
fairness; they do not remove SSD contention. An unrestricted large-read mode
would deliberately accept the measured throughput/latency tradeoff. This sweep
does not choose the allowable read slowdown, rate cap, or switching behavior.
Production scheduling was left unchanged.

## Validation outcome

All **50 fio invocations** succeeded: two full-file initialization passes and
48 measured cases. All job errors were zero; initialization sizes, measurement
durations, directions, block sizes, requested QDs, and rate settings passed
checks. Uncapped writers reported at least 99% in the expected fio QD bucket.
The four longer read-throughput repeats were within **1.3%** of their main runs.
Local report/CSV regeneration matched the instance outputs byte-for-byte. Raw
results were archived locally and the dedicated SSD data directory was removed.

## Workload

- Same host: `m8g-32cpu-local-ssd`, EC2 `m8gd.8xlarge`, 32 Graviton4 vCPUs,
  128 GiB RAM. ext4 on local NVMe instance store `/dev/nvme1n1`, not EBS.
- fio 3.36, `libaio`, O_DIRECT, one thread per stream. Engine and fio options
  retained from the previous random-read benchmark so QD is the changed variable.
- Fully initialized, separate **128 GiB read** and **64 GiB write** files.
  Initialization uses 1 MiB sequential direct writes with ending fsync.
- One uniform random reader, QD32: **4, 16, 32, 64 KiB, 1 MiB** requests.
  Random block-aligned offsets with replacement, `randrepeat=0`, `norandommap=1`.
- One sequential writer, **1 MiB** requests, wrapping at file boundaries.
  No rate limit in the QD sweep. Maximum outstanding fio requests:
  **1, 2, 4, 8, 16, 32**. No writer for the read-only baseline.
- **35 mixed/read-only cases**, **six write-only controls**, and **three rate
  controls** (16 KiB reads at 180/450 MB/s, 64 KiB reads at 450 MB/s, writer QD32).
  All 44 are shuffled together with seed `20260926`.
- Main cases: **5 seconds warmup + 30 seconds measurement** each.
- Four longer repeats: 16 KiB reads at write QD0/1/4, 64 KiB reads at write QD4;
  **5 seconds warmup + 60 seconds measurement** each, after the main sweep.
- Host load and device I/O were idle before launch; no concurrent benchmark
  was observed. `iostat -dxm -y 5` and per-second fio bandwidth logs are retained.
- No polling, device tuning, cache dropping, per-write fsync, or raw-device writes.
  Kernel splitting of 1 MiB writes means fio QD is not physical device QD.

Read/write throughput is decimal MB/s. Latency is fio completion latency; CPU
is the sum of fio job user/system percentages (100% = one core). Record achieved
write throughput alongside reads: a low QD need not imply a low write rate.

## Reproduction and evidence

[`run_bench.py`](run_bench.py) reuses the fio execution helper from
[`../local-ssd-random-read-write-20260925/run_bench.py`](../local-ssd-random-read-write-20260925/run_bench.py).
Keep that sibling directory when copying the runner. Python orchestrates; fio
issues all measured I/O. [`summarize.py`](summarize.py) checks the complete set of
48 measured cases before generating `summary.csv` and `REPORT.md`.

Allow approximately 30–35 minutes, >250 GiB free space, and several TB of writes.
Use an idle benchmark host; do not overlap with other storage tests. Run from a
fresh directory on EBS, with the historical helper in its sibling directory:

```sh
cd /home/ubuntu/ssd-bench-results/local-ssd-write-qd-20260926
python3 -u run_bench.py > progress.log 2>&1
python3 summarize.py
```

The runner verifies hostname, mount/device identity, free space, fresh paths,
fio exit/job errors, initialized byte counts, nonempty I/O, and measurement
durations. Do not use Python `-O`. Success removes only the two dedicated data
files and their directory before writing `DONE`. Failure retains logs and may
leave those files; inspect active I/O before cleaning up only this run's data.
These checks validate execution/accounting, not byte-for-byte data contents.

Raw evidence paths:

- Instance: `/home/ubuntu/ssd-bench-results/local-ssd-write-qd-20260926/`
- Local archive: `~/Development/benchmarks/results/local-ssd-write-qd-20260926/`

Retain the historical helper in the local archive's sibling directory too.
Regenerate the report there; keep only scripts, CSV, report, and this methodology
in the Feuer repository.

## Limits

This is a fio device microbenchmark, not Feuer's API. It has separate stream
budgets and does not exercise admission, shared-slot fairness, request copying,
RMW, cancellation, overlapping I/O, or transitions between read sizes. Testing a
size-triggered policy in Feuer remains a separate step. Results apply to this
low-occupancy SSD and 1 MiB writer; they do not identify a universal optimum.
