# Recovery benchmark

Run on `ssh m8g-32cpu-local-ssd-2`, with Linux io_uring and O_DIRECT support.
The benchmark is an ignored test so fixture setup can use the internal metadata
writer without adding a production API. Ordinary tests leave it disabled.

Build before measuring:

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo test --locked --release -p feuer-storage --lib --no-run
```

Run the small control cases:

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd \
  cargo test --locked --release -p feuer-storage --lib \
  disk_cache::recovery::benchmark::benchmark_recovery -- --exact --ignored --nocapture
```

Set `RECOVERY_100_GB=1` to run only the recorded-size fixture instead. Binggan
may warn about test-harness flags and fall back to its defaults. Use
`BINGGAN_FILTER` to select cases. `NUM_ITER_BENCH` must remain 1.
`NUM_ITER_GROUP` controls repeated opens within the benchmark (default: 10).
Run additional benchmark repetitions only when explicitly requested.

## Recorded-size fixture

The identifier-free [size histogram](src/disk_cache/recovery/recovery-sizes.csv)
comes from 121,023 source-download completions totaling 150.425 GB. It groups
subchunk downloads by 4-KiB-aligned size and larger downloads by whole-chunk
count. Each of its 329 classes records mean logical length, rounded down to
bytes, and observed count. These are download-count frequencies, rather than
byte weights or request-length proxies.

The benchmark expands the counts, shuffles with fixed-seed xorshift/Fisher-Yates
(seed 42), and takes a prefix without exceeding 100 GB. The fixture contains
80,778 entries and 99,971,787,650 payload bytes. A unit test checks both totals
and repeatability. Its logical capacity is 256 GiB plus the header chunk.

Payload contents are synthetic (`0x5a`), with distinct integer keys. This models
a download-size distribution rather than a production resident cache. It omits
recorded arrival order, repeated keys, and eviction history. The raw capture and
historical measurements are outside the maintained benchmark.

## Timing and verification

Each case creates a temporary cache through production batch writes, writes
dirty metadata pages, and closes the cache before timing. `TMPDIR` selects the
fixture filesystem. Fixture payload bytes, metadata chunk count, actual disk
bytes, and capacity are printed. Each fixture is deleted after its case.

Binggan measures wall time for `DiskCache::open`: file/queue initialization,
metadata I/O, and rebuilding the entry index and chunk ownership. Open waits
for all shards' recovery. Recovery leaves payload contents unread.

Each timed run performs exactly one open. A plugin verifies the recovered entry
count and every expected key, then drops the cache outside the measured interval.
Binggan performs one additional untimed open for its output column.

These are repeated opens in one process with direct I/O. Allocator and device
state carry over between runs. Fresh-process startup and cold-device recovery
require separate measurements. Whole-process CPU/RSS measurements include
fixture creation, verification, and teardown.
