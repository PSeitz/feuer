# Mixed 4 KiB / 1 MiB random reads

- **Reads:** equal numbers of random 4 KiB and 1 MiB requests, with up to 32 reads in flight.
- **Writes:** sequential 1 MiB requests, with up to 32 writes in flight. The table varies the write-rate limit.
- **Tool:** fio with libaio and direct I/O.
- **Duration:** two rounds, each with 3 seconds of warmup and 15 seconds of measurement per case.

| Round | Write cap MB/s | Read MB/s | Actual write MB/s | 4 KiB requests % | 4 KiB p99 ms | 1 MiB p99 ms |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 0 | 3,879.0 | 0.0 | 50.18 | 5.538 | 5.857 |
| 1 | 450 | 3,991.3 | 450.1 | 49.94 | 5.626 | 5.933 |
| 1 | unlimited | 3,244.9 | 1,195.1 | 50.15 | 6.837 | 7.297 |
| 2 | 0 | 3,879.0 | 0.0 | 50.12 | 5.550 | 5.845 |
| 2 | 450 | 3,878.7 | 450.1 | 49.81 | 5.656 | 5.974 |
| 2 | unlimited | 3,244.3 | 1,201.0 | 49.89 | 6.816 | 7.284 |
