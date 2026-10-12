# Local SSD: concurrent sequential reads and writes

> **Random-read follow-up (2026-09-25):** see [random reads with sequential writes](local-ssd-random-read-write-20260925/REPORT.md).
> That 42-case sweep measures the cache-relevant access pattern.
>
> **Writer-QD follow-up (2026-09-26):** see the [QD1–32 sweep, including 32 KiB reads](local-ssd-write-qd-20260926/REPORT.md)
> and [policy implications](local-ssd-write-qd-20260926/METHODOLOGY.md#findings-and-policy-implications).
> With 1 MiB writes, even QD1 saturates standalone write bandwidth and substantially slows small reads;
> a concurrency cap is not a substitute for a write-rate cap.
>
> **Mixed-size screening (2026-09-26):** [50/50 4 KiB / 1 MiB reads by request count](local-ssd-mixed-read-sizes-20260926/REPORT.md).
> In two short rounds, 450 MB/s writes roughly preserve read throughput and raise 4 KiB read p99 by about 2%.
> The presence of small reads alone does not predict the pure-small-read slowdown.
>
> **Write-chunk screening (2026-09-26):** [1 MiB/QD1 versus 64 KiB chunks](local-ssd-write-chunks-20260926/REPORT.md).
> Submitting sixteen chunks together showed no repeatable improvement in the mixed-read workload;
> 64 KiB/QD1 preserved write throughput with only a small read-latency improvement in these short runs.
> Historical results below are unchanged.

Measurements from **2026-09-19** on `m8g-32cpu-local-ssd` (EC2 **m8gd.8xlarge**).

**Scope:** sequential reads at different block sizes while a separate stream writes sequentially at different rates. **The writer always uses 1 MiB blocks.** These tests do not cover a cross-product of read and write block sizes, or random reads during writes.

All throughput is **decimal MB/s** (1 MB = 1,000,000 bytes). Block sizes are binary KiB/MiB.

## Main findings

- **64 KiB–4 MiB reads:** approximately **3,879 MB/s** without writes. Concurrent writes at **450 MB/s** leave read throughput essentially unchanged; at **910 MB/s**, reads are about **0.7–2.2% slower** in the original fio runs.
- **16 KiB reads:** much more sensitive to contention. Reads fall from **3,896.0 MB/s** to **2,182.1 MB/s** with 450 MB/s writes (**44% lower**), and to **1,786.2 MB/s** with 910 MB/s writes (**54% lower**). Independent Rust AIO and io_uring measurements reproduce this behavior.
- **4 KiB reads:** lower baseline (**1,987.9 MB/s**); with 910 MB/s writes, read throughput is **1,864.6 MB/s**, about **6% lower**. The small-block reader can approach one CPU core's limit.
- At higher write targets, both streams contend. With reads of **64 KiB or larger**, uncapped writes achieve only **1,359.7–1,402.0 MB/s**, while reads achieve **3,086.0–3,166.9 MB/s** in the original fio runs.
- These are throughput observations, **not a guarantee of unchanged latency**. For example, 64 KiB read p99 completion latency increases from **0.569 ms** without writes to **0.946 ms** with 450 MB/s writes, even though read throughput is unchanged.

## Methodology

- One nominal **1.9 TB Amazon EC2 NVMe instance-store SSD**, `/dev/nvme1n1`, ext4 at `/mnt/local-ssd`; **not EBS**. Host has 32 Graviton4 vCPUs and 128 GiB advertised RAM.
- Original measurements use **fio 3.36**, Linux `libaio`, **direct I/O (`O_DIRECT`)** to bypass the host page cache.
- One sequential reader at **QD32**, plus one sequential writer at **up to QD32**. QD is the maximum outstanding requests per stream.
- Separate, fully initialized **128 GiB read file** and **64 GiB write file** on the same SSD; streams wrap at file boundaries.
- Each main-table cell is **one run: 5 seconds warmup + 30 seconds measurement**. Cases were executed serially in randomized order. Selected longer repeats are below.
- Write columns specify requested rate caps, **not guaranteed achieved throughput**. Actual write throughput is reported separately.
- No per-write `fsync`; this measures I/O throughput, not transactional durable-write latency.
- SSD was initially empty/idle and at low occupancy, not full or aged. Application requests larger than 128 KiB may be split by the kernel (`max_sectors_kb=128`).

## Original fio results

### Read throughput (MB/s)

Each column is a different **background sequential-write load**; each cell is the resulting **read speed in MB/s**. The writer uses **1 MiB blocks** in every concurrent case.

For example: with **64 KiB reads** and writes capped at **910 MB/s**, the reader achieved **3,795.9 MB/s** while the writer achieved **910.0 MB/s**.

The write caps represent roughly **10%, 25%, 50%, 75%, and 90%** of the measured standalone write speed (~1,829 MB/s). They are requested limits, not guaranteed achieved speeds. **Uncapped writes** means the writer runs as fast as contention permits.

| Read block size | No writes (baseline) | Write cap: 180 MB/s | Write cap: 450 MB/s | Write cap: 910 MB/s | Write cap: 1,370 MB/s | Write cap: 1,640 MB/s | Uncapped writes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 1,987.9 | 1,959.8 | 1,930.7 | 1,864.6 | 1,661.4 | 1,586.4 | 1,561.0 |
| 16 KiB | 3,896.0 | 3,969.7 | 2,182.1 | 1,786.2 | 1,469.3 | 1,362.7 | 1,358.5 |
| 64 KiB | 3,878.7 | 3,878.7 | 3,878.7 | 3,795.9 | 3,179.5 | 3,092.0 | 3,166.9 |
| 256 KiB | 3,878.7 | 3,878.6 | 3,878.7 | 3,839.7 | 3,146.1 | 3,248.3 | 3,102.3 |
| 1 MiB | 3,879.3 | 3,879.3 | 3,879.4 | 3,793.7 | 3,168.8 | 3,092.4 | 3,086.0 |
| 4 MiB | 3,882.6 | 3,882.6 | 3,882.7 | 3,856.6 | 3,254.5 | 3,100.5 | 3,102.6 |

### Achieved concurrent write throughput (MB/s)

Same background-write settings as above, but each cell now shows the **actual write speed in MB/s**. For example, with 64 KiB reads and a 1,640 MB/s write cap, the writer achieved only **1,397.1 MB/s** because of contention.

| Read block size | No writes (baseline) | Write cap: 180 MB/s | Write cap: 450 MB/s | Write cap: 910 MB/s | Write cap: 1,370 MB/s | Write cap: 1,640 MB/s | Uncapped writes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,640.0 | 1,870.2 |
| 16 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,616.5 | 1,613.9 |
| 64 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,355.7 | 1,397.1 | 1,359.7 |
| 256 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,366.1 | 1,321.4 | 1,386.5 |
| 1 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,371.1 | 1,395.5 | 1,402.0 |
| 4 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,324.4 | 1,398.8 | 1,385.1 |

### Longer fio validation runs

Separate runs with **5 seconds warmup + 60 seconds measurement**; not averaged into the main tables.

| Read block | Write cap MB/s | Original read MB/s | Repeat read MB/s | Repeat actual write MB/s |
| --- | ---: | ---: | ---: | ---: |
| 16 KiB | 0 | 3,896.0 | 3,887.2 | 0.0 |
| 16 KiB | 450 | 2,182.1 | 2,426.9 | 450.0 |
| 16 KiB | 910 | 1,786.2 | 1,777.7 | 910.0 |
| 64 KiB | 910 | 3,795.9 | 3,839.2 | 910.0 |

The 16 KiB/450 MB/s result varies by **11.2%** between these runs, but the substantial slowdown remains. These measurements are not confidence intervals.

## Independent Rust / io_uring validation

Same direct-I/O workload, file sizes, stream counts, queue depths, and 1 MiB writer blocks. Selected 30-second results below; full read/write matrices for both backends are in the [Rust report](local-ssd-rust-uring-20260919/REPORT.md).

| Read block | Write cap MB/s | fio libaio read MB/s | Rust AIO read MB/s | Rust io_uring read MB/s |
| --- | ---: | ---: | ---: | ---: |
| 16 KiB | 0 | 3,896.0 | 3,878.8 | 3,878.8 |
| 16 KiB | 450 | 2,182.1 | 2,182.3 | 2,192.1 |
| 16 KiB | 910 | 1,786.2 | 1,780.5 | 1,808.2 |
| 64 KiB | 0 | 3,878.7 | 3,878.8 | 3,878.8 |
| 64 KiB | 450 | 3,878.7 | 3,878.7 | 3,878.8 |
| 64 KiB | 910 | 3,795.9 | 3,784.4 | 3,795.5 |

Actual write throughput in these selected cases was 0, 450, or 910 MB/s respectively, rounded to one decimal. Switching to io_uring did **not** remove the 16 KiB contention slowdown. The implementations differ in buffer refill and accounting, and were measured in separate phases; do not interpret small differences as isolated engine effects.

## Buffered-I/O caveat

The [buffered screening](local-ssd-buffered-20260919/REPORT.md) used only **2 seconds warmup + 10 seconds measurement**. Three of its four concurrent read/write cases had **less than 1 MB/s physical device writes during the fio invocation**, despite application write rates around 450 or 910 MB/s. Most writes were flushed afterward. Those results **do not establish sustained simultaneous SSD read/write performance** and should not replace the direct-I/O contention tables above.

## Sources

- [Original report](local-ssd-benchmark-20260919/REPORT.md), [methodology](local-ssd-benchmark-20260919/METHODOLOGY.md), and [CSV with throughput, IOPS, and p99 latency](local-ssd-benchmark-20260919/summary.csv).
- [Rust AIO / io_uring report](local-ssd-rust-uring-20260919/REPORT.md), [methodology](local-ssd-rust-uring-20260919/METHODOLOGY.md), and [CSV](local-ssd-rust-uring-20260919/comparison.csv).
- [Buffered-I/O report](local-ssd-buffered-20260919/REPORT.md).
- Raw fio results and runners: `~/Development/benchmarks/results/local-ssd-20260919/`.
- Original Pi session: `01a0ba4f-4f57-701a-bfec-b5140a811ced` in `~/Development/benchmarks`.

The tables in this document summarize the original sequential-read runs. The linked random-read follow-up is a separate measurement.
