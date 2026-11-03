# Histogram-driven storage read/write benchmark

A Rust benchmark using **`feuer-storage::DataFile`**, the production buffer pool,
and the production read/write io_uring queues. Supply a read-size histogram,
then measure reads alone and alongside fixed **1-MiB writes**.

It exercises direct I/O, read admission, buffer allocation/reuse, large-read
splitting, scheduling, and completion. It measures the storage path rather than
cache lookup, eviction, packing, or metadata updates. It does not use fio.

## Run

Requires Linux with io_uring, O_DIRECT, and STATX_DIOALIGN support. Use
`ssh m8g-32cpu-local-ssd-2` and check for other jobs first. From the repository
root on EBS:

```sh
cargo bench --locked -p feuer-storage --bench read_write -- \
  "$PWD/feuer-storage/benches/read-sizes.csv" \
  --directory /mnt/local-ssd \
  --output "$PWD/experiments/ssd/read-write-$(date -u +%Y%m%dT%H%M%SZ)"
```

Absolute input/output paths work regardless of Cargo's benchmark working
directory. Keep output under the repository's ignored `experiments/` directory,
on EBS rather than the measured SSD. The output directory must be new.

Each of three cases runs once, with ten seconds warmup and sixty measured seconds:

1. Read only.
2. Reads plus writes scheduled at **450,000,000 bytes/s** in aggregate.
3. Reads plus uncapped writes.

The capped rate is a historical comparison point, not measured production write
demand. Reported write throughput shows the achieved rate. No automatic
repetitions are performed. This sustained-capacity test is separate from the
[ten-minute timestamp-paced cache replay](../timed-replay.md).

## Histogram

```csv
size_bytes,count
4096,500
1048576,499
33554432,1
```

`count` is a positive integer **operation weight**, not a byte weight. Rows give
exact positive lengths, rather than bucket boundaries. Lengths are passed to
`DataFile` unchanged, which performs its normal alignment and splitting.
For bucketed input, explicitly choose and document representative sizes first.

Each reader samples sizes with replacement using a seeded RNG and chooses
uniformly random 4-KiB-aligned offsets. Worker `n` starts each case with seed
`20261102 + n`. Worker interleaving and completed prefixes may vary. There is no
finite generated trace to exhaust. Original arrival times, spatial locality,
and correlations between sizes are not preserved.

### Included histogram

[`benches/read-sizes.csv`](benches/read-sizes.csv) has 6,076 exact aligned sizes
and 1,377,574 operation counts. It came from a deterministic 1/64 sample of
88,151,303 successful logical accesses. The capture used 710 GB memory with disk
caching disabled. This is a **logical read-size mix**, including memory hits,
rather than an observed physical SSD-read distribution.

The included sizes were rounded to 4 KiB for the earlier native-fio experiment,
adding 0.1383% to sampled bytes. Custom inputs can retain unaligned lengths.
The CSV contains aggregate size counts, with no object keys or access records.

| Included size | Operations % | Bytes % |
|---|---:|---:|
| 4 KiB | 57.61 | 0.14 |
| >4–64 KiB | 11.16 | 0.17 |
| >64 KiB–1 MiB | 8.84 | 2.05 |
| >1–8 MiB | 17.68 | 31.57 |
| >8 MiB | 4.71 | 66.07 |

The [historical fio results](../experiments/ssd/read-size-mix-20261102/RESULT.md)
remain a separate native-I/O baseline. They are **not results from this Rust
benchmark**. The engines, write placement, and random sequences differ.

## Workload and resources

- Four Tokio workers and the production io_uring worker threads.
- **32 concurrent whole reads** by default. Production limits remain unchanged:
  64 active read pieces and eight active writes, with submissions at most 1 MiB.
  Caller count, submission limits, and device queue depth are different quantities.
- A 4-GiB buffer-pool budget with no cached objects. This is a retention budget,
  not a bound on all live buffers. Read results are returned to the pool after
  each operation. Memory demand also includes per-operation latency samples.
- One newly created DataFile: 64 GiB for reads, followed by 8 GiB for writes.
  Initialize the entire file through DataFile and sync it before measurement.
  Allow at least **80 GiB free SSD space** with default settings.
- Eight writers each cycle sequentially through their own disjoint byte range
  in the write region. Each reuses an aligned 1-MiB buffer. The capped case staggers
  their scheduled starts. Reads and writes never overlap, and writers never
  overwrite one another's in-flight ranges. Any remainder below eight MiB is
  unused when dividing the write region among writers.
- Five seconds idle before each case. The file and pool persist across cases.
  Each case has its own warmup. No per-write stable-storage sync.
- Only the benchmark's temporary directory is removed afterward. Failed I/O
  aborts the run. `DONE` is written only after all cases and cleanup succeed.

`--readers`, `--seconds`, and `--warmup` control caller concurrency and timing.
`--read-mib` and `--write-mib` control the disk regions. The write region must
contain at least eight MiB. All histogram lengths must fit the read region.
These options also support small correctness checks without a full SSD run.

## Results and measurement scopes

- `summary.csv`: read/write counts, bytes, MB/s, IOPS, and mean/p50/p95/p99 latency.
  Includes all reads, all writes, and five read-size classes. These rows are also
  printed to stdout. MB/s is decimal and latency is milliseconds.
- `process.csv`: CPU usage including all benchmark and io_uring threads, plus
  retained/used pool bytes after draining. CPU 100% means one core. Its interval
  includes warmup and drain, and excludes initialization and report generation.
- Per-case CSVs: operation, requested size, completion timestamp, and latency
  for each operation completed during the measured window. Timestamps are
  nanoseconds relative to the end of warmup. Rows are collected per worker and
  are not globally timestamp-sorted.
- `histogram.csv`, `input-sizes.csv`, `settings.txt`, and `read_write.rs`: input,
  grouped input counts/bytes, exact arguments, and the compiled benchmark source.

Bandwidth counts operations **completed during the fixed measured window**,
including operations started just before its beginning. In-flight operations
are drained afterward and excluded from measured counts. Read latency includes
`DataFile::read_at` and returning its result buffer. Size/offset selection and
write-rate waiting are outside the operation latency. Full phase CPU still
includes workload generation and sample collection. Raw CSV output occurs after
each case, outside measurement.

Requested read bytes may exclude direct-I/O padding. Device metrics have a
different scope, including split requests. For SSD runs, retain `iostat -dx -y`
and `pidstat -u -r -d` output alongside these artifacts. Record the source revision,
working-tree patch, Rust version, host/device details, and command before testing.

Similar saturated throughput does not establish acceptable small-read latency.
Reduced read capacity with writes does not establish unmet production demand or
justify a write cap. Choosing limits requires actual disk arrivals and latency
targets. One run per case does not establish small improvements or optimal settings.
