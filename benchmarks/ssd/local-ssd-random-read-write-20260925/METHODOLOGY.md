# Random reads during sequential writes: methodology

This follow-up changes the original fio contention test's reader from `rw=read`
to **`rw=randread`**, while retaining a separate **`rw=write`** writer. It does not
use `randrw`, which would make the writes random too. Historical sequential-read
results remain unchanged.

## Environment and workload

- Host: `m8g-32cpu-local-ssd`, EC2 `m8gd.8xlarge`, 32 Graviton4 vCPUs, 128 GiB RAM.
- Local NVMe instance store `/dev/nvme1n1`, ext4 at `/mnt/local-ssd`; not EBS.
- Linux `6.17.0-1007-aws`, aarch64; fio **3.36** with `ioengine=libaio`,
  `direct=1`, one thread per stream, no host tuning. Scheduler `none`,
  `max_sectors_kb=128`, `nr_requests=127`; larger requests may be split by the kernel.
- Separate fully initialized **128 GiB read file** and **64 GiB write file**.
  Initialization uses sequential 1 MiB direct writes and an ending fsync.
- Reader: uniform random block-aligned offsets with replacement (`norandommap=1`),
  `randrepeat=0`, QD32. Sizes: **4, 16, 64, 256 KiB; 1, 4 MiB**.
- Writer: sequential **1 MiB** requests, up to QD32, wrapping at file boundaries.
- Seven writer settings: **no writer; 180, 450, 910, 1,370, 1,640 MB/s; uncapped**.
  These are the original numeric caps, not recalibrated percentages.
- **42 main cases**, each **5 seconds warmup + 30 seconds measurement**,
  shuffled with seed `20260919`. The smaller matrix has a different execution
  order from the original 66-case experiment despite retaining the shuffle seed.
- A standalone 1 MiB/QD32 write control precedes the matrix.
- Four longer repeats follow it: 16 KiB reads at caps 0/450/910 MB/s and 64 KiB
  reads at 910 MB/s, each **5 seconds warmup + 60 seconds measurement**. These
  reuse the initialized files and are reported separately.
- All other fio options are retained from the original runner: `thread=1`,
  `numjobs=1`, `invalidate=1`, `fallocate=none`, `refill_buffers=1`,
  `exitall_on_error=1`, and no file creation during measured cases.
- No concurrent benchmark was observed before launch; device I/O was idle.
  `iostat -dxm -y 5` and per-second fio bandwidth logs are retained.

Both achieved read and write rates must be compared. A writer may fail to reach
its cap under contention. Reported throughput is decimal MB/s; read p99 is fio
completion latency in milliseconds. This is direct-I/O throughput, not per-write
durability, a full/aged SSD test, or an application-level Feuer benchmark.

## Validation outcome

All **49 fio invocations** completed successfully: two full-file initialization
passes, one standalone write control, 42 main cases, and four repeats. All job
errors were zero; initialization sizes and measurement durations passed checks.
All four longer read-throughput repeats were within **1%** of their main runs.
The dedicated data files were removed. CSV and report regeneration matched
exactly. This checks execution and accounting, not byte-for-byte data contents.

## Reproduce

Sources are retained here: [`run_bench.py`](run_bench.py) (adapted from the
2026-09-19 runner) and [`summarize.py`](summarize.py). Python only orchestrates;
fio issues all measured I/O. No third-party Python dependencies are needed.

Use the same idle host with fio and sysstat installed. Allow **about 30 minutes**,
**more than 250 GiB free**, and several TB of writes. Do not overlap with another
storage benchmark. Never format or write a raw device for this test.

Copy these two scripts into a **fresh directory on EBS**, then run in tmux:

```sh
RUN=/home/ubuntu/ssd-bench-results/random-read-write-$(date -u +%Y%m%dT%H%M%SZ)
mkdir "$RUN"
cp /path/to/source/{run_bench.py,summarize.py} "$RUN/"
cd "$RUN"
python3 -u run_bench.py > progress.log 2>&1 && python3 summarize.py
```

The runner checks hostname, mount/device identity, free space, and fresh output
and data paths. It checks all fio job errors, full initialization byte counts,
nonempty measurements, and measurement durations. Do not use Python `-O`.
Success creates `DONE` only after removing the two dedicated data files and
their directory. Failure retains evidence and may leave those files; inspect
logs and active I/O before cleaning up only that run's directory.

Raw results for the 2026-09-25 run are retained outside this compact folder:

- Instance: `/home/ubuntu/ssd-bench-results/local-ssd-random-read-write-20260925/`
- Local archive: `~/Development/benchmarks/results/local-ssd-random-read-write-20260925/`

Copy the completed raw directory to the local archive and run `summarize.py`
there. Copy only the generated `REPORT.md` and `summary.csv` back here.
The original runs are documented in
[`../local-ssd-benchmark-20260919/METHODOLOGY.md`](../local-ssd-benchmark-20260919/METHODOLOGY.md).
