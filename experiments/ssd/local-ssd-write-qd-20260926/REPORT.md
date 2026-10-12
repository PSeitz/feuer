# Random reads: sequential writer queue-depth sweep

Measured **2026-09-26** on `m8g-32cpu-local-ssd` (EC2 `m8gd.8xlarge`).

One random reader at **QD32**, with a separate sequential **1 MiB writer**.
**No write-rate cap** in the QD sweep. QD is the per-stream maximum outstanding fio requests,
not a global shared limit or a measured device queue depth.
fio libaio, O_DIRECT, same workload settings as the prior random-read sweep.
Main cases: **5 seconds warmup + 30 seconds measurement**, shuffled execution order.
Throughput is decimal MB/s; block sizes are binary KiB/MiB.

## Read throughput (MB/s)

| Read block | No writes | Write QD1 | Write QD2 | Write QD4 | Write QD8 | Write QD16 | Write QD32 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 1,433.2 | 603.3 | 600.3 | 588.4 | 572.8 | 557.2 | 555.3 |
| 16 KiB | 3,821.3 | 1,558.0 | 1,576.8 | 1,581.5 | 1,572.1 | 1,578.0 | 1,560.7 |
| 32 KiB | 3,878.7 | 2,484.2 | 2,493.4 | 2,496.4 | 2,507.1 | 2,481.0 | 2,472.1 |
| 64 KiB | 3,878.7 | 3,230.2 | 3,239.5 | 3,237.2 | 3,247.4 | 3,268.9 | 3,236.8 |
| 1 MiB | 3,879.3 | 3,317.5 | 3,345.3 | 3,352.6 | 3,329.2 | 3,329.4 | 3,330.9 |

## Achieved write throughput (MB/s)

| Read block | No writes | Write QD1 | Write QD2 | Write QD4 | Write QD8 | Write QD16 | Write QD32 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 0.0 | 1,830.2 | 1,828.6 | 1,837.8 | 1,832.3 | 1,871.3 | 1,869.5 |
| 16 KiB | 0.0 | 1,570.0 | 1,552.2 | 1,549.1 | 1,536.3 | 1,546.9 | 1,571.6 |
| 32 KiB | 0.0 | 1,193.4 | 1,192.9 | 1,190.6 | 1,193.1 | 1,210.2 | 1,210.2 |
| 64 KiB | 0.0 | 1,036.4 | 1,014.6 | 1,026.9 | 1,027.0 | 1,014.2 | 1,030.2 |
| 1 MiB | 0.0 | 1,099.1 | 1,079.1 | 1,073.0 | 1,078.4 | 1,093.6 | 1,093.0 |

## Read p99 completion latency (ms)

| Read block | No writes | Write QD1 | Write QD2 | Write QD4 | Write QD8 | Write QD16 | Write QD32 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 0.167 | 0.545 | 0.545 | 0.553 | 0.569 | 0.578 | 0.586 |
| 16 KiB | 0.289 | 0.709 | 0.709 | 0.709 | 0.709 | 0.709 | 0.709 |
| 32 KiB | 0.395 | 0.905 | 0.897 | 0.905 | 0.905 | 0.905 | 0.897 |
| 64 KiB | 0.643 | 1.466 | 1.483 | 1.483 | 1.466 | 1.466 | 1.483 |
| 1 MiB | 8.585 | 11.076 | 11.076 | 11.076 | 11.076 | 10.945 | 11.076 |

## Write-only controls

| Writer QD | Write MB/s | Write p99 ms |
| ---: | ---: | ---: |
| 1 | 1,828.5 | 0.461 |
| 2 | 1,828.6 | 1.036 |
| 4 | 1,828.6 | 2.179 |
| 8 | 1,828.5 | 4.489 |
| 16 | 1,828.6 | 9.110 |
| 32 | 1,829.1 | 24.248 |

## Rate-capped controls

Writer QD32; same initialized files and timing as the QD sweep.

| Read block | Write cap MB/s | Read MB/s | Actual write MB/s | Read p99 ms |
| --- | ---: | ---: | ---: | ---: |
| 16 KiB | 180 | 3,235.2 | 180.0 | 0.473 |
| 16 KiB | 450 | 2,588.6 | 450.0 | 0.561 |
| 64 KiB | 450 | 3,937.9 | 450.0 | 1.204 |

## Longer repeats

5 seconds warmup + 60 seconds measurement. Not averaged into main results.

| Read block | Write QD | Main read MB/s | Repeat read MB/s | Change | Repeat write MB/s | Repeat read p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 16 KiB | 0 | 3,821.3 | 3,822.0 | +0.0% | 0.0 | 0.289 |
| 16 KiB | 1 | 1,558.0 | 1,566.6 | +0.6% | 1,545.1 | 0.709 |
| 16 KiB | 4 | 1,581.5 | 1,562.5 | -1.2% | 1,564.0 | 0.709 |
| 64 KiB | 4 | 3,237.2 | 3,263.7 | +0.8% | 1,014.8 | 1.466 |

## Scope

- Single main runs plus selected repeats, not confidence intervals.
- Fully initialized 128 GiB read and 64 GiB write files; low-occupancy SSD.
- No aged/full-device preconditioning or per-write fsync.
- p99 is fio completion latency, not Feuer end-to-end latency.
- Only 1 MiB sequential writes; no small writes, RMW, or mixed read sizes.
- Separate fio streams, not Feuer scheduling or its shared 64-slot ring.
- This measures static concurrency limits, not adaptive-policy transitions.

See [methodology and interpretation](METHODOLOGY.md), [CSV](summary.csv), and
[the preceding rate-cap sweep](../local-ssd-random-read-write-20260925/REPORT.md).
