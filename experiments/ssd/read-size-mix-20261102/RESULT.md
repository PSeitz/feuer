# SSD read/write capacity with the Feuer 27 size mix

## Summary

One run per case on the local SSD, using native fio 3.36 and direct io_uring I/O. Each case used ten seconds warmup and sixty measured seconds. No additional repetitions were run. Setup, all three jobs, and analysis succeeded. Temporary SSD files were removed.

At this size mix and concurrency, 450 MB/s of sequential writes preserved approximately 3.88 GB/s of reads with similar read latency. Uncapped writes reached 1.52 GB/s, while read capacity fell 28.6% and aggregate read p99 rose from 37.5 to 76.5 ms. This establishes interference at the saturated test point. It does not establish an application write limit or an optimal concurrency.

## Scope

The input is the new histogram's underlying successful-access trace, including memory hits and source fetches. The capture used 710 GB of memory capacity with disk caching disabled. It contains no recorded disk events. A deterministic sample retains actual sizes, rather than histogram midpoints. Sizes are rounded upward to 4 KiB, adding 0.1383% bytes. Offsets are synthetic and uniformly random over an initialized 64-GiB file. Arrival timing is replaced by sustained load.

Reader concurrency is 32 **whole fio requests**, with lengths up to 76.87 MiB after alignment. This is distinct from Feuer's 1-MiB submissions, its active-read limit, and device queue depth. The optional writer has depth eight and writes sequential 1-MiB requests to a separate initialized file on the same SSD. The 450-MB/s setting is a historical comparison point, not recorded production write demand.

This is an SSD capacity benchmark. It does not measure Feuer's read implementation or reproduce production disk traffic. See [SETUP.md](SETUP.md) for exact configuration and provenance.

## Measured throughput, latency, and CPU

MB/s uses decimal bytes. Latencies are whole-request submission-to-completion times. CPU 100% means one CPU core, summed across the reader and writer jobs.

| Case | Read MB/s | Read IOPS | Write MB/s | Read mean ms | Read p50 ms | Read p99 ms | CPU % |
|---|---:|---:|---:|---:|---:|---:|---:|
| Read only | 3,879.7 | 2,325 | 0.0 | 13.763 | 12.302 | 37.501 | 16.9 |
| 450 MB/s write cap | 3,879.7 | 2,325 | 450.0 | 13.763 | 12.316 | 37.677 | 18.7 |
| Uncapped writes | 2,768.9 | 1,664 | 1,516.4 | 19.230 | 14.314 | 76.505 | 17.9 |

The two first cases have identical fio byte counts and bandwidth to the reported precision. Their independent latency logs differ. This observation supplies no estimate of run-to-run variability.

| Writer | Write IOPS | Write mean ms | Write p99 ms |
|---|---:|---:|---:|
| 450 MB/s cap | 429 | 0.275 | 0.309 |
| Uncapped | 1,446 | 5.532 | 21.103 |

## Read latency by size

Percentiles below are calculated directly from per-I/O total-latency samples. Classes use aligned sizes, so the 4-KiB class includes logical reads smaller than 4 KiB.

| Aligned read size | Read-only p99 ms | 450 MB/s writes p99 ms | Uncapped writes p99 ms |
|---|---:|---:|---:|
| 4 KiB | 35.499 | 35.478 | 72.151 |
| >4–64 KiB | 36.429 | 36.253 | 72.891 |
| >64 KiB–1 MiB | 37.471 | 37.258 | 78.415 |
| >1–8 MiB | 39.197 | 39.418 | 81.134 |
| >8 MiB | 45.049 | 45.579 | 95.659 |

Small requests are numerous but carry little data. In the read-only measurement:

| Aligned read size | Latency samples | Operations % | Bytes % |
|---|---:|---:|---:|
| 4 KiB | 80,450 | 57.65 | 0.14 |
| >4–64 KiB | 15,533 | 11.13 | 0.17 |
| >64 KiB–1 MiB | 12,304 | 8.82 | 2.04 |
| >1–8 MiB | 24,640 | 17.66 | 31.17 |
| >8 MiB | 6,624 | 4.75 | 66.48 |

The uncapped-write case has 99,880 latency samples with a similar measured size mix. Its >8-MiB requests carry 66.47% of bytes. Full per-size counts, mean, p50, p95, and p99 are in `summary.json`.

The 35.5-ms read-only p99 for 4-KiB requests is an important qualification: preserving saturated bandwidth does not establish acceptable latency. These requests run alongside much larger reads at fixed whole-request concurrency. Choosing application limits requires the actual SSD arrival rate, size/timing correlations, and latency target.

## Device and process observations

These are arithmetic means of one-second iostat samples over process execution, including startup and warmup. They have a different scope from fio's measured interval. Device requests can split a whole fio read into multiple operations.

| Case | Device reads/s | Read await ms | Write await ms | Device average queue | Utilization % | Maximum RSS MiB |
|---|---:|---:|---:|---:|---:|---:|
| Read only | 31,683 | 3.867 | 0.003 | 122.3 | 98.6 | 2,093.8 |
| 450 MB/s write cap | 31,780 | 3.861 | 0.186 | 123.1 | 98.7 | 2,118.0 |
| Uncapped writes | 22,258 | 5.651 | 5.307 | 181.6 | 99.4 | 2,050.0 |

All fio jobs reported zero errors. fio reported depth 32 for 100% of the reader depth samples in all cases. The host had no competing benchmark jobs at preflight. Idle and process/device monitor outputs are retained.

## Accounting and retained artifacts

Read throughput/IOPS use fio JSON's byte and operation accounting. Latency analysis uses the complete per-I/O logs, whose counts and means match fio JSON's latency statistics. fio's latency sample counts exceed its reported read `total_ios` by 31 in each case. Logged byte totals also differ slightly from throughput-accounting byte totals. These scopes are retained separately in `summary.json` rather than forcing equality.

Artifacts include `prepare.py`, the sampled logical/aligned sizes, `reads.iolog`, source and stream SHA-256 hashes in `input.json`, exact `.fio` files, `run.py`, `commands.json`, environment/idle logs, initialization results, per-case fio JSON and per-I/O latency logs, iostat/pidstat/time output, `run.exit`, `DONE`, `analyze.py`, and `summary.json`.

Raw artifacts stay local and Git-ignored. This compact report and `SETUP.md` are tracked. No production code, settings, or website claims were changed by this experiment.
