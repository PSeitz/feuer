# SSD transfer-size sweep

fio/libaio, direct I/O. One stream, initialized 128-GiB read and 64-GiB
write files. 5 s warmup + 30 s per case. Single runs.

| Request | Random read QD1 | Random read QD32 | Sequential write QD1 |
|---|---:|---:|---:|
| 4 KiB | 59 | 1,433 | 217 |
| 16 KiB | 200 | 3,837 | 647 |
| 64 KiB | 679 | 3,879 | 1,471 |
| 256 KiB | 1,702 | 3,879 | 1,829 |
| 1 MiB | 3,339 | 3,879 | 1,829 |
| 4 MiB | 3,879 | 3,883 | 1,830 |

MB/s. Larger transfers need less concurrency to approach the bandwidth
ceiling. Writes plateau by 256 KiB at QD1. This supports batching,
without establishing 1 MiB as uniquely optimal.
