# Mixed 4 KiB / 1 MiB random reads

fio/libaio, O_DIRECT. One QD32 random reader, 4-KiB/1-MiB requests
50/50 by count (99.61% large-read bytes). Sequential 1-MiB writer QD32.
Initialized 8-GiB read and 4-GiB write files. Two shuffled rounds,
3 s warmup + 15 s per case. p99 is nearest-rank completion latency,
computed separately for each request size. Short screening runs.

| Round | Write cap MB/s | Read MB/s | Actual write MB/s | 4 KiB requests % | 4 KiB p99 ms | 1 MiB p99 ms |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 0 | 3,879.0 | 0.0 | 50.18 | 5.538 | 5.857 |
| 1 | 450 | 3,991.3 | 450.1 | 49.94 | 5.626 | 5.933 |
| 1 | unlimited | 3,244.9 | 1,195.1 | 50.15 | 6.837 | 7.297 |
| 2 | 0 | 3,879.0 | 0.0 | 50.12 | 5.550 | 5.845 |
| 2 | 450 | 3,878.7 | 450.1 | 49.81 | 5.656 | 5.974 |
| 2 | unlimited | 3,244.3 | 1,201.0 | 49.89 | 6.816 | 7.284 |
