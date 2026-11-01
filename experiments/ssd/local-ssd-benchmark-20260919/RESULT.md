# m8g-32cpu-local-ssd: local NVMe benchmarks

- **Workloads:** random reads, sequential writes, and sequential reads alongside writes.
- **Concurrency:** standalone tests use 1 or 32 requests in flight. Mixed tests allow 32 reads and 32 writes, using 1 MiB writes.
- **Tool:** fio with libaio and direct I/O.
- **Duration:** 5 seconds of warmup and 30 seconds of measurement per case. Validation repeats measure 60 seconds.
- **Tables:** mixed-workload columns show write-rate limits. Achieved write rates are listed separately.

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

| Read block size | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 1,987.9 | 1,959.8 | 1,930.7 | 1,864.6 | 1,661.4 | 1,586.4 | 1,561.0 |
| 16 KiB | 3,896.0 | 3,969.7 | 2,182.1 | 1,786.2 | 1,469.3 | 1,362.7 | 1,358.5 |
| 64 KiB | 3,878.7 | 3,878.7 | 3,878.7 | 3,795.9 | 3,179.5 | 3,092.0 | 3,166.9 |
| 256 KiB | 3,878.7 | 3,878.6 | 3,878.7 | 3,839.7 | 3,146.1 | 3,248.3 | 3,102.3 |
| 1 MiB | 3,879.3 | 3,879.3 | 3,879.4 | 3,793.7 | 3,168.8 | 3,092.4 | 3,086.0 |
| 4 MiB | 3,882.6 | 3,882.6 | 3,882.7 | 3,856.6 | 3,254.5 | 3,100.5 | 3,102.6 |

### Actual concurrent write throughput (MB/s)

| Read block size | No writer | 180 | 450 | 910 | 1370 | 1640 | Uncapped |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,640.0 | 1,870.2 |
| 16 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,616.5 | 1,613.9 |
| 64 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,355.7 | 1,397.1 | 1,359.7 |
| 256 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,366.1 | 1,321.4 | 1,386.5 |
| 1 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,371.1 | 1,395.5 | 1,402.0 |
| 4 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,324.4 | 1,398.8 | 1,385.1 |

## Validation repeats

| Read block | Write cap MB/s | Original read MB/s | Repeat read MB/s | Repeat write MB/s | Read change |
| --- | --- | --- | --- | --- | --- |
| 16 KiB | 0 | 3,896.0 | 3,887.2 | 0.0 | -0.2% |
| 16 KiB | 450 | 2,182.1 | 2,426.9 | 450.0 | +11.2% |
| 16 KiB | 910 | 1,786.2 | 1,777.7 | 910.0 | -0.5% |
| 64 KiB | 910 | 3,795.9 | 3,839.2 | 910.0 | +1.1% |
