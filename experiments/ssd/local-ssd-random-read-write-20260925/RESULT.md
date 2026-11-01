# Random reads with sequential writes

- **Reads:** random requests, with up to 32 reads in flight. Each row uses a different request size.
- **Writes:** sequential 1 MiB requests, with up to 32 writes in flight. Columns vary the write-rate limit.
- **Tool:** fio with libaio and direct I/O.
- **Duration:** 5 seconds of warmup and 30 seconds of measurement per case. Validation repeats measure 60 seconds.
- **Tables:** achieved write rates are listed separately from their limits.

## Random read throughput (MB/s)

| Read block | No writes | 180 MB/s cap | 450 MB/s cap | 910 MB/s cap | 1,370 MB/s cap | 1,640 MB/s cap | Uncapped |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 1,434.8 | 1,280.8 | 1,071.5 | 835.6 | 703.8 | 645.7 | 549.0 |
| 16 KiB | 3,806.6 | 3,292.6 | 2,669.5 | 2,044.1 | 1,684.6 | 1,573.7 | 1,553.7 |
| 64 KiB | 3,878.7 | 3,878.7 | 3,929.7 | 3,425.4 | 3,249.5 | 3,293.5 | 3,265.1 |
| 256 KiB | 3,878.7 | 3,878.7 | 3,894.2 | 3,584.9 | 3,619.2 | 3,592.5 | 3,589.3 |
| 1 MiB | 3,879.3 | 3,879.4 | 3,879.4 | 3,597.9 | 3,360.6 | 3,290.3 | 3,355.9 |
| 4 MiB | 3,882.6 | 3,882.6 | 3,882.6 | 3,825.8 | 3,190.6 | 3,218.3 | 3,220.4 |

## Achieved sequential write throughput (MB/s)

| Read block | No writes | 180 MB/s cap | 450 MB/s cap | 910 MB/s cap | 1,370 MB/s cap | 1,640 MB/s cap | Uncapped |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,370.0 | 1,640.0 | 1,871.4 |
| 16 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,369.4 | 1,546.9 | 1,570.9 |
| 64 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,021.2 | 999.2 | 1,019.0 |
| 256 KiB | 0.0 | 180.0 | 450.0 | 910.0 | 919.3 | 946.6 | 950.8 |
| 1 MiB | 0.0 | 180.0 | 450.0 | 908.7 | 1,072.9 | 1,099.0 | 1,059.8 |
| 4 MiB | 0.0 | 180.0 | 450.0 | 910.0 | 1,332.4 | 1,308.2 | 1,316.2 |

## Random read p99 completion latency (ms)

| Read block | No writes | 180 MB/s cap | 450 MB/s cap | 910 MB/s cap | 1,370 MB/s cap | 1,640 MB/s cap | Uncapped |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 0.167 | 0.305 | 0.362 | 0.477 | 0.498 | 0.545 | 0.586 |
| 16 KiB | 0.289 | 0.461 | 0.553 | 0.659 | 0.692 | 0.700 | 0.709 |
| 64 KiB | 0.643 | 0.946 | 1.204 | 1.434 | 1.466 | 1.483 | 1.466 |
| 256 KiB | 2.245 | 2.736 | 2.638 | 3.293 | 3.391 | 3.359 | 3.359 |
| 1 MiB | 8.585 | 8.978 | 9.110 | 10.945 | 11.076 | 11.207 | 11.076 |
| 4 MiB | 33.817 | 33.817 | 34.865 | 41.157 | 43.254 | 42.729 | 42.729 |

## Longer validation runs

| Read block | Write cap MB/s | Main read MB/s | Repeat read MB/s | Change | Repeat write MB/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| 16 KiB | 0 | 3,806.6 | 3,807.0 | +0.0% | 0.0 |
| 16 KiB | 450 | 2,669.5 | 2,655.5 | -0.5% | 450.0 |
| 16 KiB | 910 | 2,044.1 | 2,025.3 | -0.9% | 910.6 |
| 64 KiB | 910 | 3,425.4 | 3,402.9 | -0.7% | 910.0 |
