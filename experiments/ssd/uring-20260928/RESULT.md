# Historical Feuer io_uring benchmark

Direct I/O. Medians of three runs, including per-run p99s.
Write-only 1-MiB requests:

| Write QD | MB/s | Write p99 ms |
|---|---:|---:|
| 1 | 1,568 | 0.744 |
| 8 | 1,829 | 4.639 |
| 32 | 1,828 | 22.415 |
| 64 | 1,828 | 47.135 |

Beyond QD8, queued writes increased latency without improving throughput.
Read-only 64-KiB requests reached 3,879 MB/s at QD32 and QD64, while
p99 rose from 0.641 to 1.182 ms.

This historical I/O path includes allocation, admission, and reply overhead.
Its latency scope differs from fio completion latency.
