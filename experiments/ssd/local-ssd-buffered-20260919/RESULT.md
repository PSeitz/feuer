# Buffered I/O screening versus the original direct-I/O benchmark

fio/libaio, buffered I/O, nominal QD32. 2 s warmup + 10 s per case.
Direct baselines are earlier 30-second runs. Buffered submission can block.
Application writes measure page-cache acceptance. Each file is flushed
with fdatasync afterward, outside measurement. Device rates cover the
whole fio invocation, including startup and warmup. They are not exactly
time-aligned with application rates. Write targets are caps.

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

| Read block | Write target MB/s | Direct read MB/s | Buffered read MB/s | Direct actual write MB/s | Buffered actual write MB/s |
| --- | --- | --- | --- | --- | --- |
| 16 KiB | 0 | 3,896.0 | 2,806.4 | 0.0 | 0.0 |
| 16 KiB | 450 | 2,182.1 | 2,800.7 | 450.0 | 450.2 |
| 16 KiB | 910 | 1,786.2 | 2,804.2 | 910.0 | 913.4 |
| 64 KiB | 0 | 3,878.7 | 2,624.8 | 0.0 | 0.0 |
| 64 KiB | 450 | 3,878.7 | 2,292.7 | 450.0 | 450.1 |
| 64 KiB | 910 | 3,795.9 | 2,619.7 | 910.0 | 910.1 |

## Physical I/O and deferred writeback

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
