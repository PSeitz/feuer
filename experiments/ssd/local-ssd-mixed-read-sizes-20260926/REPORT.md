# Mixed 4 KiB / 1 MiB random reads

Measured **2026-09-26** on `m8g-32cpu-local-ssd` (`m8gd.8xlarge`).

**50/50 by request count**, selected within one QD32 random reader
using `bssplit=4k/50:1m/50`. This is approximately **99.61% large-read bytes**.
A separate sequential writer uses 1 MiB requests at QD32.
fio libaio, O_DIRECT, initialized 8 GiB read and 4 GiB write files.
Two shuffled rounds; each case has **3 seconds warmup + 15 seconds measurement**.
These are short screening runs, not sustained-performance estimates.

| Round | Write cap MB/s | Read MB/s | Actual write MB/s | 4 KiB requests % | 4 KiB p99 ms | 1 MiB p99 ms |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 0 | 3,879.0 | 0.0 | 50.18 | 5.538 | 5.857 |
| 1 | 450 | 3,991.3 | 450.1 | 49.94 | 5.626 | 5.933 |
| 1 | unlimited | 3,244.9 | 1,195.1 | 50.15 | 6.837 | 7.297 |
| 2 | 0 | 3,879.0 | 0.0 | 50.12 | 5.550 | 5.845 |
| 2 | 450 | 3,878.7 | 450.1 | 49.81 | 5.656 | 5.974 |
| 2 | unlimited | 3,244.3 | 1,201.0 | 49.89 | 6.816 | 7.284 |

Latency percentiles are nearest-rank p99 from individual fio completion-latency
records, separated by actual request size. Sample counts and combined mean
are checked against fio JSON completion-latency statistics. Latency and
bandwidth counters differ slightly at ramp/drain boundaries (bounded by QD32).
Throughput uses fio JSON bandwidth. All cases use the same per-I/O logging.

See [methodology and interpretation](METHODOLOGY.md) and [CSV](summary.csv).
