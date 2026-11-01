# Write chunking during mixed 4 KiB / 1 MiB random reads

fio/libaio, O_DIRECT. One QD32 random reader, 4-KiB/1-MiB requests
50/50 by count (99.61% large-read bytes). Uncapped sequential writer.
Initialized 8-GiB read and 4-GiB write files. Two shuffled rounds,
3 s warmup + 15 s per case. p99 is nearest-rank completion latency by
request size. QD16 batch submits and completes 16 requests together.
QD16 rolling submits/reaps one at a time. Short screening runs.

| Round | Writer | Read MB/s | Write MB/s | 4 KiB requests % | 4 KiB p99 ms | 1 MiB p99 ms |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 1MiB-qd1 | 3,241.2 | 1,191.8 | 49.89 | 6.831 | 7.260 |
| 1 | 64KiB-qd1 | 3,300.3 | 1,171.0 | 50.13 | 6.617 | 7.080 |
| 1 | 64KiB-qd16-batch | 3,313.5 | 1,159.8 | 49.86 | 6.736 | 7.144 |
| 1 | 64KiB-qd16-rolling | 3,291.1 | 1,162.6 | 49.77 | 6.731 | 7.180 |
| 1 | read-only | 3,879.1 | 0.0 | 50.10 | 5.544 | 5.855 |
| 2 | 1MiB-qd1 | 3,313.5 | 1,157.3 | 50.19 | 6.684 | 7.136 |
| 2 | 64KiB-qd1 | 3,297.8 | 1,172.8 | 50.32 | 6.629 | 7.074 |
| 2 | 64KiB-qd16-batch | 3,252.4 | 1,181.7 | 49.85 | 6.853 | 7.309 |
| 2 | 64KiB-qd16-rolling | 3,274.3 | 1,183.7 | 49.83 | 6.794 | 7.304 |
| 2 | read-only | 3,878.7 | 0.0 | 49.95 | 5.528 | 5.821 |
