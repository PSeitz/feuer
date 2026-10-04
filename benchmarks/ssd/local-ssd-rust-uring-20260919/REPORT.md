# Rust validation and io_uring comparison

All throughput is **decimal MB/s**. QD is maximum outstanding requests per stream. Python only launches processes; all measured I/O is performed by the Rust binary or native fio.

## Main findings

- Rust reproduces the large-block throughput plateau: 1 MiB/QD32 random reads are **3,878.8 MB/s** with Linux AIO and **3,878.8 MB/s** with io_uring.
- 1 MiB/QD32 sequential writes are **1,828.8 MB/s** with Linux AIO and **1,828.6 MB/s** with io_uring.
- At 4 KiB/QD32, io_uring changes Rust random-read throughput by **+0.3%**, and sequential-write throughput by **-1.0%**. There is no across-the-board large throughput gain.
- The 16 KiB contention slowdown remains: with 910 MB/s writes, reads are **1,780.5 MB/s** (Linux AIO) versus **1,808.2 MB/s** (io_uring). At 64 KiB, the corresponding read rates are **3,784.4 / 3,795.5 MB/s**.
- Rust/fio differences are workload-dependent; adjacent controls and longer repeats below expose them. Do not attribute differences in buffer generation or instrumentation to the I/O engine or language alone.

## Conditions

- Same `m8gd.8xlarge` host and `/mnt/local-ssd` instance-store SSD as the original archive.
- Independent Rust implementation: raw Linux AIO syscalls first, then the same workload loop using the `io-uring` crate.
- Both Rust backends use direct I/O, one worker per stream, 4096-byte-aligned buffers, submission/completion batches of one, and complete every request before freeing buffers.
- io_uring uses ordinary non-vectored reads/writes: no SQPOLL, IOPOLL, registered buffers/files, or forced async dispatch.
- Fully initialized 128 GiB read file and 64 GiB write file. No random writes. Dedicated files only.
- Main measurements: 5-second warmup + 30 seconds. Longer repeats: 5 + 60 seconds. Two-second smoke tests are excluded from the result tables.
- Same 66-case matrix per Rust backend; 9 adjacent fio controls per engine. Linux AIO phase precedes io_uring, so phase/time is a possible confounder.
- Rust buffer refill/PRNG and completion accounting differ from fio; this is an independent workload replication, not a bit-for-bit port. See METHODOLOGY.md.
- Per-cell single-run results, with selected repeats; not confidence intervals or a general guarantee that one engine is faster.

## Random reads

### QD1

| Block | Original fio | Rust AIO | Rust vs original | Rust io_uring | io_uring vs Rust AIO |
| --- | --- | --- | --- | --- | --- |
| 4 KiB | 59.0 | 59.4 | +0.6% | 59.2 | -0.2% |
| 16 KiB | 199.6 | 203.2 | +1.8% | 202.9 | -0.2% |
| 64 KiB | 678.6 | 681.1 | +0.4% | 679.6 | -0.2% |
| 256 KiB | 1,701.9 | 1,704.5 | +0.2% | 1,699.9 | -0.3% |
| 1 MiB | 3,338.9 | 3,333.2 | -0.2% | 3,329.8 | -0.1% |
| 4 MiB | 3,878.6 | 3,878.8 | +0.0% | 3,878.8 | +0.0% |

### QD32

| Block | Original fio | Rust AIO | Rust vs original | Rust io_uring | io_uring vs Rust AIO |
| --- | --- | --- | --- | --- | --- |
| 4 KiB | 1,432.8 | 1,447.3 | +1.0% | 1,451.2 | +0.3% |
| 16 KiB | 3,837.3 | 3,868.2 | +0.8% | 3,873.1 | +0.1% |
| 64 KiB | 3,878.7 | 3,878.8 | +0.0% | 3,878.8 | -0.0% |
| 256 KiB | 3,878.7 | 3,878.8 | +0.0% | 3,878.8 | +0.0% |
| 1 MiB | 3,879.3 | 3,878.8 | -0.0% | 3,878.8 | -0.0% |
| 4 MiB | 3,882.7 | 3,878.8 | -0.1% | 3,878.9 | +0.0% |

## Sequential writes

### QD1

| Block | Original fio | Rust AIO | Rust vs original | Rust io_uring | io_uring vs Rust AIO |
| --- | --- | --- | --- | --- | --- |
| 4 KiB | 216.7 | 225.1 | +3.9% | 223.1 | -0.9% |
| 16 KiB | 646.6 | 683.2 | +5.7% | 681.0 | -0.3% |
| 64 KiB | 1,471.4 | 1,614.0 | +9.7% | 1,589.7 | -1.5% |
| 256 KiB | 1,828.5 | 1,828.6 | +0.0% | 1,828.6 | +0.0% |
| 1 MiB | 1,828.5 | 1,828.6 | +0.0% | 1,828.5 | -0.0% |
| 4 MiB | 1,830.3 | 1,828.6 | -0.1% | 1,828.6 | +0.0% |

### QD32

| Block | Original fio | Rust AIO | Rust vs original | Rust io_uring | io_uring vs Rust AIO |
| --- | --- | --- | --- | --- | --- |
| 4 KiB | 999.9 | 1,112.5 | +11.3% | 1,101.1 | -1.0% |
| 16 KiB | 1,828.5 | 1,828.6 | +0.0% | 1,828.6 | -0.0% |
| 64 KiB | 1,828.5 | 1,832.2 | +0.2% | 1,828.6 | -0.2% |
| 256 KiB | 1,825.7 | 1,828.6 | +0.2% | 1,828.6 | +0.0% |
| 1 MiB | 1,829.1 | 1,828.8 | -0.0% | 1,828.6 | -0.0% |
| 4 MiB | 1,832.5 | 1,829.0 | -0.2% | 1,828.9 | -0.0% |

## Sequential reads with sequential writes: Rust libaio

Writer block size is fixed at 1 MiB, max QD32; reader max QD32. Column headings are write caps, not guaranteed achieved rates.

### Read throughput (MB/s)

| Read block | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 2,267.8 | 2,260.0 | 2,171.1 | 1,993.8 | 1,771.0 | 1,628.3 | 1,412.5 |
| 16 KiB | 3,878.8 | 3,974.2 | 2,182.3 | 1,780.5 | 1,490.5 | 1,372.8 | 1,361.0 |
| 64 KiB | 3,878.8 | 3,878.7 | 3,878.7 | 3,784.4 | 3,166.2 | 3,190.0 | 3,185.8 |
| 256 KiB | 3,878.8 | 3,878.7 | 3,878.8 | 3,785.5 | 3,265.9 | 3,292.8 | 3,285.7 |
| 1 MiB | 3,878.8 | 3,878.8 | 3,878.8 | 3,796.0 | 3,165.7 | 3,163.1 | 3,286.4 |
| 4 MiB | 3,878.8 | 3,878.8 | 3,878.9 | 3,788.3 | 3,163.6 | 3,307.3 | 3,268.9 |

### Actual write throughput (MB/s)

| Read block | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,640.0 | 1,874.8 |
| 16 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,643.2 | 1,668.7 |
| 64 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,351.9 | 1,340.1 | 1,353.3 |
| 256 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,317.2 | 1,309.5 | 1,306.5 |
| 1 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,354.0 | 1,369.6 | 1,304.5 |
| 4 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,354.7 | 1,292.6 | 1,318.9 |

## Sequential reads with sequential writes: Rust io_uring

Writer block size is fixed at 1 MiB, max QD32; reader max QD32. Column headings are write caps, not guaranteed achieved rates.

### Read throughput (MB/s)

| Read block | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 2,219.0 | 2,244.1 | 2,264.4 | 2,057.7 | 1,740.6 | 1,630.5 | 1,470.2 |
| 16 KiB | 3,878.8 | 3,966.8 | 2,192.1 | 1,808.2 | 1,459.0 | 1,363.1 | 1,363.9 |
| 64 KiB | 3,878.8 | 3,878.8 | 3,878.8 | 3,795.5 | 3,220.9 | 3,275.9 | 3,241.7 |
| 256 KiB | 3,878.8 | 3,878.8 | 3,878.8 | 3,827.4 | 3,170.6 | 3,309.8 | 3,163.6 |
| 1 MiB | 3,878.8 | 3,878.8 | 3,878.8 | 3,784.0 | 3,223.7 | 3,218.5 | 3,232.9 |
| 4 MiB | 3,878.8 | 3,878.6 | 3,878.6 | 3,846.6 | 3,226.1 | 3,172.2 | 3,190.7 |

### Actual write throughput (MB/s)

| Read block | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,640.0 | 1,877.3 |
| 16 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,661.7 | 1,654.4 |
| 64 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,325.4 | 1,306.2 | 1,330.1 |
| 256 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,344.0 | 1,307.3 | 1,350.0 |
| 1 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,336.2 | 1,345.0 | 1,326.0 |
| 4 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,335.5 | 1,349.3 | 1,353.5 |

## Adjacent fio controls: libaio

Each fio case runs immediately before its matching Rust case on the same initialized files. CPU is summed across workers; 100% is one CPU core.

| Workload | Block | QD | Write cap | fio primary MB/s | Rust primary MB/s | Difference | fio write MB/s | Rust write MB/s | fio CPU % | Rust CPU % |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mixed | 16 KiB | 32 | 0 | 3,894.5 | 3,878.8 | -0.4% | 0.0 | 0.0 | 74.3 | 67.7 |
| mixed | 16 KiB | 32 | 450 | 2,307.1 | 2,182.3 | -5.4% | 450.0 | 450.0 | 51.1 | 42.1 |
| mixed | 16 KiB | 32 | 910 | 1,769.8 | 1,780.5 | +0.6% | 910.0 | 910.0 | 48.5 | 38.9 |
| mixed | 64 KiB | 32 | 910 | 3,778.1 | 3,784.4 | +0.2% | 908.7 | 910.0 | 41.7 | 33.1 |
| randread | 4 KiB | 1 | 0 | 59.0 | 59.4 | +0.6% | 0.0 | 0.0 | 4.9 | 4.5 |
| randread | 4 KiB | 32 | 0 | 1,432.0 | 1,447.3 | +1.1% | 0.0 | 0.0 | 83.3 | 75.3 |
| randread | 1 MiB | 32 | 0 | 3,879.2 | 3,878.8 | -0.0% | 0.0 | 0.0 | 12.8 | 12.6 |
| write | 4 KiB | 32 | 0 | 988.0 | 1,112.5 | +12.6% | 988.0 | 1,112.5 | 72.4 | 59.2 |
| write | 1 MiB | 32 | 0 | 1,835.5 | 1,828.8 | -0.4% | 1,835.5 | 1,828.8 | 28.4 | 12.5 |

## Adjacent fio controls: io_uring

Each fio case runs immediately before its matching Rust case on the same initialized files. CPU is summed across workers; 100% is one CPU core.

| Workload | Block | QD | Write cap | fio primary MB/s | Rust primary MB/s | Difference | fio write MB/s | Rust write MB/s | fio CPU % | Rust CPU % |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mixed | 16 KiB | 32 | 0 | 3,893.3 | 3,878.8 | -0.4% | 0.0 | 0.0 | 69.7 | 67.1 |
| mixed | 16 KiB | 32 | 450 | 2,186.8 | 2,192.1 | +0.2% | 450.0 | 450.0 | 46.8 | 42.2 |
| mixed | 16 KiB | 32 | 910 | 1,789.7 | 1,808.2 | +1.0% | 910.0 | 910.0 | 46.8 | 39.2 |
| mixed | 64 KiB | 32 | 910 | 3,799.0 | 3,795.5 | -0.1% | 910.0 | 910.0 | 40.6 | 33.4 |
| randread | 4 KiB | 1 | 0 | 59.2 | 59.2 | +0.1% | 0.0 | 0.0 | 4.7 | 4.6 |
| randread | 4 KiB | 32 | 0 | 1,446.8 | 1,451.2 | +0.3% | 0.0 | 0.0 | 76.8 | 73.2 |
| randread | 1 MiB | 32 | 0 | 3,879.3 | 3,878.8 | -0.0% | 0.0 | 0.0 | 12.6 | 12.4 |
| write | 4 KiB | 32 | 0 | 1,073.1 | 1,101.1 | +2.6% | 1,073.1 | 1,101.1 | 71.6 | 58.1 |
| write | 1 MiB | 32 | 0 | 1,829.1 | 1,828.6 | -0.0% | 1,829.1 | 1,828.6 | 28.2 | 12.3 |

## Longer Rust repeats

| Engine | Write cap MB/s | 30s read MB/s | 60s read MB/s | Change | 60s actual write MB/s |
| --- | --- | --- | --- | --- | --- |
| libaio | 450 | 2,182.3 | 2,301.5 | +5.5% | 450.0 |
| libaio | 910 | 1,780.5 | 1,812.4 | +1.8% | 910.0 |
| io_uring | 450 | 2,192.1 | 2,367.9 | +8.0% | 450.0 |
| io_uring | 910 | 1,808.2 | 1,783.6 | -1.4% | 910.0 |

## Files

- `comparison.csv`: long-form data including original fio, Rust, adjacent fio, repeats, and explicitly tagged smoke tests.
- `comparison.png`: QD32 throughput and read/write contention plots.
- `METHODOLOGY.md`: implementation details, correctness checks, limitations, and reproduction instructions.

Source, locked dependencies, raw results, and logs are archived **outside the results folder**, at `~/Development/benchmarks/results/local-ssd-rust-uring-20260919/` and on the instance at `/home/ubuntu/ssd-bench-results/local-ssd-rust-uring-20260919/`.
