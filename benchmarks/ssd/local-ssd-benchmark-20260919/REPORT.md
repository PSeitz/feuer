# m8g-32cpu-local-ssd: local NVMe benchmarks

All throughput values are **decimal MB/s** (1 MB = 1,000,000 bytes). Block sizes are binary KiB/MiB.

## Method

- Host: `m8g-32cpu-local-ssd`, EC2 `m8gd.8xlarge`, 32 Graviton4 vCPUs, 128 GiB advertised RAM.
- Drive: one 1.9 TB Amazon EC2 NVMe instance-store SSD, `/dev/nvme1n1`, ext4 at `/mnt/local-ssd`. Not EBS.
- fio-3.36, Linux libaio, direct I/O (`O_DIRECT`) bypassing page cache, one job per stream.
- Each measured case: 5 s warmup followed by 30 s measurement. Case order randomized with a fixed seed.
- Standalone tests: queue depth (QD) 1 and 32. QD1 means one outstanding request, not maximum device throughput.
- Mixed tests: one sequential reader at QD32 plus one sequential writer at up to QD32 using **1 MiB writes**. Writer caps apply to decimal bytes/s; actual achieved rates are reported separately.
- Separate, fully sequentially initialized 128 GiB read and 64 GiB write files. Reads never access sparse/unwritten extents. Streams wrap at file boundaries.
- Existing data untouched; only the two benchmark data files are removed after successful completion. No raw-device writes, random writes, tuning, or cache dropping.
- SSD initially empty and idle. These are low-occupancy file overwrite measurements, not a fully preconditioned/full-drive endurance benchmark.
- Writes use direct I/O, **not per-write fsync**. This measures throughput rather than transactional durable-write latency.
- Application block sizes above 128 KiB can be split by the kernel: device `max_sectors_kb=128`.
- Each main-table cell is one run, not an average of repeats. Selected contention cases are repeated for 60 seconds in the validation section. Raw fio JSON, configs, per-second bandwidth logs, and iostat are retained.
- Results describe these specific stream counts and queue depths, not a proven maximum across all concurrency levels. Small-block sequential reads can also approach one fio CPU core's limit.
- UTC start: 2026-09-19T15:39:16.934546+00:00; end: 2026-09-19T16:20:35.689125+00:00.

## Random read throughput

| Block size | QD1 MB/s | QD32 MB/s |
| --- | --- | --- |
| 4 KiB | 59.0 | 1,432.8 |
| 16 KiB | 199.6 | 3,837.3 |
| 64 KiB | 678.6 | 3,878.7 |
| 256 KiB | 1,701.9 | 3,878.7 |
| 1 MiB | 3,338.9 | 3,879.3 |
| 4 MiB | 3,878.6 | 3,882.7 |

## Sequential write throughput

| Block size | QD1 MB/s | QD32 MB/s |
| --- | --- | --- |
| 4 KiB | 216.7 | 999.9 |
| 16 KiB | 646.6 | 1,828.5 |
| 64 KiB | 1,471.4 | 1,828.5 |
| 256 KiB | 1,828.5 | 1,825.7 |
| 1 MiB | 1,828.5 | 1,829.1 |
| 4 MiB | 1,830.3 | 1,832.5 |

## Sequential reads during sequential writes

Cells below are **read MB/s**. Columns are the **requested write caps in MB/s**, not guaranteed achieved write throughput. The writer always uses 1 MiB blocks.

| Read block size | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 1,987.9 | 1,959.8 | 1,930.7 | 1,864.6 | 1,661.4 | 1,586.4 | 1,561.0 |
| 16 KiB | 3,896.0 | 3,969.7 | 2,182.1 | 1,786.2 | 1,469.3 | 1,362.7 | 1,358.5 |
| 64 KiB | 3,878.7 | 3,878.7 | 3,878.7 | 3,795.9 | 3,179.5 | 3,092.0 | 3,166.9 |
| 256 KiB | 3,878.7 | 3,878.6 | 3,878.7 | 3,839.7 | 3,146.1 | 3,248.3 | 3,102.3 |
| 1 MiB | 3,879.3 | 3,879.3 | 3,879.4 | 3,793.7 | 3,168.8 | 3,092.4 | 3,086.0 |
| 4 MiB | 3,882.6 | 3,882.6 | 3,882.7 | 3,856.6 | 3,254.5 | 3,100.5 | 3,102.6 |

### Actual concurrent write throughput (MB/s)

Same cases as above. A cap is a ceiling; read contention may prevent the writer from reaching it.

| Read block size | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,640.0 | 1,870.2 |
| 16 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,616.5 | 1,613.9 |
| 64 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,355.7 | 1,397.1 | 1,359.7 |
| 256 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,366.1 | 1,321.4 | 1,386.5 |
| 1 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,371.1 | 1,395.5 | 1,402.0 |
| 4 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,324.4 | 1,398.8 | 1,385.1 |

## Validation repeats

Selected contention cases repeated with 5 s warmup + **60 s measurement**, after recreating and fully initializing the same-sized dedicated files. Main-table values above remain the original 30-second runs.

| Read block | Write cap MB/s | Original read MB/s | Repeat read MB/s | Repeat write MB/s | Read change |
| --- | --- | --- | --- | --- | --- |
| 16 KiB | 0 | 3,896.0 | 3,887.2 | 0.0 | -0.2% |
| 16 KiB | 450 | 2,182.1 | 2,426.9 | 450.0 | +11.2% |
| 16 KiB | 910 | 1,786.2 | 1,777.7 | 910.0 | -0.5% |
| 64 KiB | 910 | 3,795.9 | 3,839.2 | 910.0 | +1.1% |

## Files

- `summary.csv`: achieved throughput, IOPS, p99 completion latency, CPU usage, and runtimes.
- `throughput.png`: standalone throughput and read/write contention plots.
- `METHODOLOGY.md`: test design, validation, limitations, and reproduction instructions.

Raw artifacts and runners are retained outside this folder at `~/Development/benchmarks/results/local-ssd-20260919/` and on the benchmark instance at `/home/ubuntu/ssd-bench-results/local-ssd-20260919/`.
