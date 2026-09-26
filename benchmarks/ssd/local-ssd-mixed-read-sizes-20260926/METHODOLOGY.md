# Short mixed-read-size screening

## Findings and policy implications

The benchmark completed in **117 seconds**, including initialization. Both rounds
agree on the main result:

- No writes: **3,879 MB/s** aggregate reads, **5.54–5.55 ms** 4 KiB read p99.
- Writes capped at 450 MB/s: **3,879–3,991 MB/s** reads, **5.63–5.66 ms**
  4 KiB p99. Read throughput is roughly preserved and small-read p99 rises only
  about **2%**. The first round's higher throughput is not evidence that writes
  improve reads; this is a short screening run.
- Uncapped writes achieve **1,195–1,201 MB/s** while reads fall to
  **3,244–3,245 MB/s** (**16% lower**), with **6.82–6.84 ms** 4 KiB p99
  (**23% higher**).

The mixed stream behaves differently from a pure 4 KiB reader: 1 MiB requests
account for approximately 99.61% of bytes. These results argue against deciding
to stop all writes solely because a 4 KiB request exists. A moderate write-rate
allowance can coexist with this mix with little measured incremental penalty.
That does not establish 450 MB/s as an optimum or a portable default.

Small reads already have approximately 5.5 ms p99 in the mixed workload **without
writes**. Pausing writes alone therefore cannot recover pure-small-read latency
in this setup. These are fio completion latencies in a mixed-size QD32 workload,
not isolated device service times or Feuer API measurements. No adaptive policy
or read-size prioritization was tested.

## Validation outcome

All eight fio invocations succeeded (two initialization passes, six measured
cases). Achieved small-read counts were **49.81–50.18%**. All six per-size log
sets matched fio's completion-latency sample counts and combined means; bandwidth
boundary differences stayed within one QD32 batch. The data directory was
removed, raw evidence archived, and local report/CSV regeneration matched the
instance output byte-for-byte.

## Workload and scope

- Same idle `m8g-32cpu-local-ssd` host, EC2 `m8gd.8xlarge`, ext4 on local NVMe
  instance store `/dev/nvme1n1`. No other benchmark was observed before launch.
- fio 3.36, libaio, O_DIRECT. **One reader at QD32** selects each request size
  using `bssplit=4k/50:1m/50`: **50/50 by request count**, not bytes or queue
  occupancy. About **99.61% of requested bytes** belong to 1 MiB reads.
- Uniform random reads with replacement, `norandommap=1`, `randrepeat=0`;
  physical offsets aligned to 4 KiB for both sizes. No separate fixed-depth
  reader for each size: independent readers would not enforce a 50/50 mixture.
- Separate sequential 1 MiB writer at QD32. Three settings: no writer,
  **450 MB/s** write cap, and uncapped writes.
- Two rounds, each independently shuffled with the same seeded RNG
  (`20260926`); **3 seconds warmup + 15 seconds measurement** per case.
- To keep this run short, use fully initialized **8 GiB read / 4 GiB write**
  files instead of the prior 128/64 GiB files. Initialization uses direct writes
  and ending fsync. Measured cases do not fsync each write.
- Direct I/O bypasses the host page cache, not device caches. Small files,
  short runs, low occupancy, and no steady-state preconditioning make this a
  screening test, not a sustained-device benchmark or a direct repeat of the
  previous large-file experiments.
- Per-I/O latency logging on the reader (`log_avg_msec=0`, 262,144 preallocated
  log entries) records actual request size. Split completion latencies by size
  and compute nearest-rank p99; do not use a blended percentile to describe
  small reads. All six cases use identical instrumentation.
- Latency sample counts and combined mean are validated against fio JSON
  `clat_ns.N` and `clat_ns.mean`. fio latency and bandwidth counts differ slightly
  at warmup/drain boundaries; differences are checked against the maximum
  QD32 in-flight count/bytes. Reported bandwidth comes from fio JSON, not logs.
- No Feuer code or scheduler policy is exercised. This cannot establish whether
  stopping writes whenever a small read exists is necessary or starvation-free.

## Reproduce

[`run_bench.py`](run_bench.py) imports the sibling
[`../local-ssd-random-read-write-20260925/run_bench.py`](../local-ssd-random-read-write-20260925/run_bench.py)
for fio execution and validation. Retain that path when copying. On the same idle
host, use a fresh output directory on EBS with >20 GiB free on the local SSD:

```sh
python3 -u run_bench.py > progress.log 2>&1
python3 summarize.py
```

Allow about two minutes of benchmark runtime. The runner checks host, mount,
device identity, fresh paths, fio errors, initialization bytes, and measurement
durations. Do not use Python `-O`. Success removes only its dedicated SSD files
and directory before writing `DONE`; failure retains evidence and may leave
those files. Inspect active I/O before any cleanup.

[`summarize.py`](summarize.py) requires all six cases and validates the achieved
request-size mixture and latency records before generating the CSV and report.

Raw evidence:

- Instance: `/home/ubuntu/ssd-bench-results/local-ssd-mixed-read-sizes-20260926/`
- Local archive: `~/Development/benchmarks/results/local-ssd-mixed-read-sizes-20260926/`
