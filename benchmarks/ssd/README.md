# Read-size histogram SSD benchmark

Supply a histogram of read lengths. Measure random-read throughput and latency
with no writes, with 450 MB/s of writes, and with uncapped writes. Writes are
always sequential **1-MiB requests**, matching Feuer's payload chunk size.

This uses native **fio**, so it measures SSD capacity rather than Feuer's
implementation overhead. Reads use their whole sampled size. They are not split
into Feuer's 1-MiB submissions by this driver. Neither object caching nor metadata
updates are simulated.

## Run

Requires Linux, Python 3.9+, fio with `io_uring`, `iostat`/`pidstat` (sysstat), and
GNU `/usr/bin/time`. The tested fio version is 3.36. Use the SSD test host
`ssh m8g-32cpu-local-ssd-2`. Check for other jobs before starting.

From a checkout on EBS:

```sh
python3 benchmarks/ssd/read_write.py benchmarks/ssd/read-sizes.csv \
  --directory /mnt/local-ssd \
  --output experiments/ssd/read-write-$(date -u +%Y%m%dT%H%M%SZ)
```

The default run measures each of three cases once: ten seconds warmup, sixty
seconds measured. It initializes 64 GiB for reads and 8 GiB for writes, uses
separate files, and removes only its temporary directory afterward. Allow at
least 80 GiB free SSD space. Peak memory can approach 32 times the largest read,
plus fio's parsed input and per-I/O logs. The included histogram used roughly
2.1 GiB RSS in the original experiment.

Put output on EBS under the repository's ignored `experiments/` directory.
The output directory must be new. Paths must not contain whitespace or colons.
Do not run other workloads on the SSD during measurement. No automatic reruns
are performed. This is a sustained capacity test, distinct from the ten-minute
[timestamp-paced cache replay](../../timed-replay.md).

## Histogram format

```csv
size_bytes,count
4096,500
1048576,499
33554432,1
```

`count` is an **operation weight**, not a byte weight. Rows describe exact lengths,
not bucket boundaries. Weights need not sum to 100. Positive integer sizes are
rounded upward to 4 KiB for direct I/O. Rows that align to the same length are
combined. Supply exact-size counts where available. For bucketed input, explicitly
choose and document representative sizes before using this format.

The runner samples lengths with replacement using seed 20261102 and generates
uniform random aligned offsets. All cases start with the same generated stream.
Their measured prefixes and completion order may differ. Original arrival times,
spatial locality, and correlations between read sizes are not preserved.

### Included histogram

`read-sizes.csv` contains 6,076 exact aligned sizes and 1,377,574 operation counts.
It was derived from a deterministic 1/64 sample of 88,151,303 successful logical
accesses. The capture used 710 GB of memory and disabled disk caching, so this
is a **logical read-size mix**, including memory hits, rather than a physical
SSD-read distribution. Alignment added 0.1383% to the sampled bytes.

| Aligned size | Operations % | Bytes % |
|---|---:|---:|
| 4 KiB | 57.61 | 0.14 |
| >4–64 KiB | 11.16 | 0.17 |
| >64 KiB–1 MiB | 8.84 | 2.05 |
| >1–8 MiB | 17.68 | 31.57 |
| >8 MiB | 4.71 | 66.07 |

The CSV contains aggregated size counts. It contains no object keys or access
records. See the [original results](../../experiments/ssd/read-size-mix-20261102/RESULT.md)
and [setup](../../experiments/ssd/read-size-mix-20261102/SETUP.md) for provenance.
The reusable runner resamples this histogram with Python's RNG. Its sequence is
not the original experiment's NumPy-shuffled sequence.

## Fixed test settings

- One reader at depth 32 **whole requests**. This is neither Feuer's active-read
  limit nor actual device queue depth.
- One optional writer at depth eight, sequential 1-MiB requests, to a separate
  initialized file on the same SSD. The capped case uses 450,000,000 bytes/s.
- Direct `io_uring` I/O. No registered buffers/files, SQPOLL, or per-write fsync.
- Five seconds idle before each case. Ten seconds warmup, sixty measured seconds.
- Per-I/O latency logging, without interval averaging. CPU 100% means one core.
- A finite generated read stream with one million requests by default.

Use `--seconds`, `--warmup`, and `--requests` to change measurement duration and
input length. Warmup must be shorter than measurement, so exhaustion during
warmup cannot satisfy the measured-duration check. The generated stream must
cover warmup **and** measurement.
fio 3.36 does not loop an exhausted `read_iolog`, even with `time_based=1` or
`loops`. The runner rejects measurements that end early. Small-read-only mixes
need many more requests. For example, 600,000 IOPS over 70 seconds needs more
than 42 million input requests and substantially more input/log storage.

The native `bssplit` alternative accepts only 64 sizes and integer percentage
weights in fio 3.36. A replay stream preserves rare sizes and the full histogram.
Input preparation is outside measurement. All input is parsed before replay,
rather than synchronously feeding a generator during measurement.

## Outputs and interpretation

`RESULT.md` reports read/write throughput, read IOPS, mean/p99 latency, CPU, and
per-size latency/count/byte-share tables. `summary.json` also includes p50/p95.
The directory retains the runner, settings, source histogram, sampled histogram,
generated input and checksum, exact fio jobs/commands, initialization results,
fio JSON, per-I/O latency logs, and process/device monitoring. `DONE` is written
only after all cases pass and temporary data is removed.

Latency sample counts and means are checked against fio's JSON latency statistics.
fio's throughput and latency accounting can differ at measurement boundaries.
Both counts and byte totals are retained separately. iostat/pidstat/time output
includes startup and warmup, unlike the reported steady-state fio interval.

Compare both bandwidth and per-size latency. Similar saturated throughput does
not establish acceptable latency. A lower saturated read rate with writes does
not establish unmet application demand or justify a production write cap.
Choose concurrency using observed disk arrivals and application latency targets.
One run per case does not establish small improvements or optimal settings.

## Driver checks

These checks exercise input generation, configuration, and result accounting
without performing disk benchmarks:

```sh
python3 -m unittest discover -s benchmarks/ssd -p 'test_*.py'
```
