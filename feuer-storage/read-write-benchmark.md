# Storage read/write benchmark

Run with the observed read-size mix and print results to the terminal:

```sh
cargo bench --locked -p feuer-storage --bench read_write -- \
  --directory /mnt/local-ssd
```

The default preserves 6,076 sampled sizes and 1,377,574 operation weights from our
observed logical accesses, including memory hits. Sizes were rounded to 4 KiB
for the original experiment, adding 0.1383% to sampled bytes. The source capture
had disk caching disabled, so this represents payload sizes rather than observed
physical disk demand. The mix is compiled into the benchmark.

Override it for your own payloads with `--sizes '4KiB:50,32KiB:30,1MiB:20'`.
This selects sizes with weights of 50%, 30%, and 20% of read **operations**, rather
than bytes. Weights need not sum to 100. Sizes accept units such as `B`, `KiB`,
and `MiB`. Sizes and integer weights must be positive. No input file or output
directory is required.

## What it tests

The benchmark calls **`feuer-storage::DataFile::read_at` and `write_at`**.
It exercises Feuer's pooled buffers, read admission, large-read splitting,
io_uring queues, and completion handling. Cache lookup, eviction, packing, and
metadata updates are outside this storage-path benchmark.

By default, two cases run once each, with ten seconds warmup and sixty measured seconds:

1. Concurrent reads, without writes.
2. Concurrent reads and continuous **1-MiB writes**, with no benchmark rate cap.

Feuer's internal concurrency limits and backpressure remain unchanged. The
benchmark defaults to 32 whole-read callers and eight write callers. Each caller
starts its next operation after the previous one completes. Whole-read caller
count, active 1-MiB read pieces, and device queue depth are different quantities.
The reported write throughput is what Feuer and the device achieve.

Use `--writers 1,2,3,4,5,6,7,8` to run a read-only baseline followed by each
listed write concurrency, once each, in that order. Each write caller has at most
one 1-MiB request outstanding. Up to eight callers therefore limit outstanding
writes to the selected count. More callers wait behind the production limit of
eight active writes. The production read-pressure limiter also applies: while
reads wait for channel capacity, new writes wait for active writes to finish and
are paced at 1 GiB/s. The restriction remains for 100 ms after the last read
stops waiting, then normal admission resumes. A new read wait restarts this hold
when it ends. While restricted, the worker rechecks every millisecond, subject to
thread scheduling. The benchmark varies caller concurrency
and measures the resulting admission behavior. Each case has its own
warmup and measurement window.

Read sizes are sampled with replacement according to the supplied weights.
Offsets are uniformly random and 4-KiB-aligned. Sizes are passed unchanged to
DataFile, which performs its normal padding and splitting. Each reader starts
each case with seed `20261102 + worker_index`. This tests sustained capacity for
a size mix, rather than reproducing application timing or locality.

## Run conditions

Use Linux with io_uring, O_DIRECT, and STATX_DIOALIGN support. For project SSD
measurements, use `ssh m8g-32cpu-local-ssd-2` and check for other jobs first.
Run the command from the repository root on EBS. To retain results, redirect
output to a file under the repository's ignored `experiments/` directory.

Defaults:

- A newly created DataFile with a 64-GiB read region and an 8-GiB write region.
  Allow at least **80 GiB free SSD space**. Eight write callers initialize the
  entire file through DataFile, which is then synced before testing.
- Each writer cycles through its own disjoint byte range, reusing a 1-MiB buffer.
  Reads and writes never overlap. Writers never overwrite another writer's
  in-flight bytes. No per-write stable-storage sync.
- Four Tokio workers, production I/O workers, and a 4-GiB buffer-pool retention
  budget. Live buffers and latency samples also consume memory.
- Five seconds idle before each case. File and pool persist across cases.
  Each case gets its own warmup.

`--readers`, `--seconds`, and `--warmup` adjust concurrency and timing.
`--read-mib` and `--write-mib` adjust the disk regions. Read sizes must fit the
read region. The write region must contain at least one MiB per writer in the
largest selected case. These options also allow small correctness checks
without a full SSD performance run.

Only the benchmark's temporary data directory is removed afterward. I/O failures
abort the run. A successful exit follows all selected cases and cleanup.

## Report format

The terminal report starts with the input distribution: exact size, operation
weight, expected operation percentage, and expected byte percentage. Repeated
sizes are combined. Measured results include overall read/write throughput,
IOPS, counts, bytes, and mean/p50/p95/p99 latency, followed by a row for each
exact read size with completed operations, sorted by size. There are no fixed
size bands. Sizes without completions have no latency row.

The default mix can produce thousands of rows. Check sample counts when
interpreting percentiles, especially for rare sizes.

The report also includes process CPU, including I/O workers, and pool usage
after drain. CPU 100% means one core. Its interval includes warmup and drain,
and excludes initialization and report generation.

Throughput counts completions during the fixed measurement window. Outstanding
operations drain afterward and are excluded. Read latency includes the DataFile
call and returning the result buffer. Size/offset selection is outside this
latency. CPU includes workload generation and sample collection. Results are
printed after each case. Requested bytes exclude any direct-I/O padding.

For SSD measurements, retain `iostat -dx -y` and `pidstat -u -r -d` output, source
revision and working-tree patch, Rust version, host/device details, and command.
[Experiment results](../experiments/ssd/read-write-results.md) are kept locally
under the ignored `experiments/` directory.
