# Random reads during sequential writes

fio/libaio, direct I/O. Random reader QD32, separate sequential 1-MiB
writer QD32. 5 s warmup + 30 s per case, selected 60-second repeats.

| Read size | Read-only MB/s | Read MB/s with 450-MB/s writer |
|---|---:|---:|
| 4 KiB | 1,435 | 1,072 |
| 16 KiB | 3,807 | 2,670 |
| 64 KiB | 3,879 | 3,930 |

The writer achieved its cap in these cases. Small-read throughput was
more sensitive to writes. At uncapped writes, 4-KiB read p99 increased
from 0.167 to 0.586 ms. These are fio completion latencies, not cache latency.
