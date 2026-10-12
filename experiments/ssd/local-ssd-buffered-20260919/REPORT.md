# Buffered I/O screening versus the original direct-I/O benchmark

**All throughput is decimal MB/s.** This is a short 14-case screening, not a repeat of the entire original matrix.

## Main findings

- 4 KiB random reads fell from **1,432.8 to 57.8 MB/s** at nominal QD32. Average submission time rose from **1.36 to 70.29 microseconds**, consistent with blocking buffered submission. The buffered rate is close to the original direct QD1 rate, not its asynchronous QD32 rate.
- 64 KiB sequential writes reported **3,666.6 MB/s**, but left **21,402 MB** to write during a subsequent **11.71-second** fdatasync. This is page-cache-assisted throughput, not sustained durable SSD bandwidth.
- **3 of the four mixed cases had less than 1 MB/s physical write traffic during the fio invocation**, despite application write rates around 450 or 910 MB/s. Most of those writes were flushed afterward. These short cases do **not** establish a sustained buffered read/write disk-contention curve.

## Important interpretation

- Same host, local SSD, file sizes, fio/libaio, and nominal QD32. Changed `direct=1` to `direct=0`; shortened measurements to 2 s warmup + 10 s.
- **Buffered libaio can block in submission, so configured QD32 does not guarantee 32 concurrent disk requests.** Cold buffered random reads are not directly equivalent to asynchronous O_DIRECT reads. Their QD1 direct-I/O baseline is included for context.
- `invalidate=1` remains enabled: each case starts by invalidating its target file cache. Cache fills during the case; this is not an intentionally fully warm-cache test.
- Buffered writes measure acceptance into the page cache, not durable completion. Each case is followed by file-specific fdatasync **outside** the measurement, before the next case.
- The 10-second samples can include cache bursts and dirty-page throttling. They must not be treated as steady-state SSD bandwidth. Read the device-I/O and flush results alongside application throughput.
- Direct baselines are the original 30-second measurements, not simultaneous paired repeats. Dataset contents/initialization source and timing differ; see METHODOLOGY.md.

## Random reads

| Block | Original direct QD32 MB/s | Buffered nominal QD32 MB/s | Original direct QD1 MB/s |
| --- | --- | --- | --- |
| 4 KiB | 1,432.8 | 57.8 | 59.0 |
| 16 KiB | 3,837.3 | 194.4 | 199.6 |
| 64 KiB | 3,878.7 | 616.3 | 678.6 |
| 1 MiB | 3,879.3 | 992.9 | 3,338.9 |

## Sequential writes

| Block | Original direct QD32 MB/s | Buffered nominal QD32 MB/s |
| --- | --- | --- |
| 4 KiB | 999.9 | 1,591.1 |
| 16 KiB | 1,828.5 | 3,307.4 |
| 64 KiB | 1,828.5 | 3,666.6 |
| 1 MiB | 1,829.1 | 2,908.0 |

## Sequential reads with sequential writes

Writer uses 1 MiB blocks, nominal QD32. Write targets are not guarantees; achieved rates are shown.

| Read block | Write target MB/s | Direct read MB/s | Buffered read MB/s | Direct actual write MB/s | Buffered actual write MB/s |
| --- | --- | --- | --- | --- | --- |
| 16 KiB | 0 | 3,896.0 | 2,806.4 | 0.0 | 0.0 |
| 16 KiB | 450 | 2,182.1 | 2,800.7 | 450.0 | 450.2 |
| 16 KiB | 910 | 1,786.2 | 2,804.2 | 910.0 | 913.4 |
| 64 KiB | 0 | 3,878.7 | 2,624.8 | 0.0 | 0.0 |
| 64 KiB | 450 | 3,878.7 | 2,292.7 | 450.0 | 450.1 |
| 64 KiB | 910 | 3,795.9 | 2,619.7 | 910.0 | 910.1 |

## Physical I/O and deferred writeback

**Device rates below cover the entire fio invocation, including startup and warmup**, unlike the application rates above. They are diagnostic, not exactly time-aligned throughput comparisons. Post-case writeback is measured during fdatasync and is excluded from application timing.

| Case | Device read MB/s | Device write MB/s | Post-case writeback MB | Post-case flush seconds |
| --- | --- | --- | --- | --- |
| mixed-bs65536-rate450 | 2,394.9 | 413.3 | 297.8 | 0.11 |
| write-bs1048576-qd32 | 0.0 | 1,536.0 | 19,424.1 | 10.62 |
| mixed-bs65536-rate910 | 2,631.8 | 0.0 | 10,922.0 | 4.76 |
| mixed-bs16384-rate450 | 2,729.7 | 0.0 | 5,403.3 | 2.07 |
| randread-bs1048576-qd32 | 869.8 | 0.0 | 0.0 | 0.00 |
| mixed-bs16384-rate0 | 2,536.0 | 0.0 | 0.0 | 0.00 |
| randread-bs4096-qd32 | 52.5 | 0.0 | 0.0 | 0.00 |
| write-bs4096-qd32 | 0.0 | 511.4 | 12,543.9 | 5.88 |
| write-bs65536-qd32 | 0.0 | 1,252.3 | 21,401.6 | 11.71 |
| mixed-bs65536-rate0 | 2,626.1 | 0.0 | 0.0 | 0.00 |
| randread-bs65536-qd32 | 552.5 | 0.0 | 0.0 | 0.00 |
| write-bs16384-qd32 | 0.0 | 1,260.4 | 20,835.4 | 11.41 |
| randread-bs16384-qd32 | 177.7 | 0.0 | 0.0 | 0.00 |
| mixed-bs16384-rate910 | 2,648.7 | 0.0 | 10,875.8 | 4.61 |

## Files

- `comparison.csv`: application rates, physical-I/O diagnostics, CPU usage, and flush accounting.
- `comparison.png`: direct versus buffered throughput.
- `METHODOLOGY.md`: settings, limitations, validation, and reproduction.
- Raw results and runner remain outside this folder: `~/Development/benchmarks/results/local-ssd-buffered-20260919/`; instance copy: `/home/ubuntu/ssd-bench-results/local-ssd-buffered-20260919/`.
