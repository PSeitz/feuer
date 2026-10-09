# Binggan recovery benchmark

On Linux with io_uring and O_DIRECT support, run the small control cases:

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd \
  cargo test --locked --release -p feuer-storage --lib \
  disk_cache::recovery::benchmark::benchmark_recovery -- --exact --ignored --nocapture
```

The entry point is an ignored test so fixture setup can use the internal metadata
writer without adding a production API. Ordinary tests do not run the benchmark.
Binggan may warn about the test harness's CLI flags; it falls back to its defaults.
Use `BINGGAN_FILTER` to select cases and `NUM_ITER_GROUP` to change repetitions.

## 100-GB fixture with recorded download-size frequencies

```sh
PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd RECOVERY_100_GB=1 \
  cargo test --locked --release -p feuer-storage --lib \
  disk_cache::recovery::benchmark::benchmark_recovery -- --exact --ignored --nocapture
```

This runs only the mixed-size case. The distribution comes from **121,023 actual
source-download completions**, totaling 150.425 GB, in
`../replay-feuer/new-bench-searcher-0/buffer-sizes/source-sizes.bin`.
`buffer-sizes/analyze.py` extracted these lengths from source completion events;
they are not request-length proxies. This is read-only analysis of an existing
capture, not a new full-trace replay.

| Recorded aligned download size | Downloads | Probability by count | Share of payload bytes |
| --- | ---: | ---: | ---: |
| <128 KiB | 69,757 | 57.64% | 0.74% |
| 128–<512 KiB | 24,746 | 20.45% | 4.53% |
| 512 KiB–1 MiB | 6,146 | 5.08% | 3.16% |
| >1 MiB | 20,374 | 16.83% | 91.57% |

The source population's mean payload is 1.243 MB; median 43,960 bytes; p95
5,569,525 bytes; maximum 104,034,539 bytes. Sizes are weighted by **download
count**, not by bytes or all requests. The current writer buffers aligned sizes
below 512 KiB; this differs from the historical capture's 128-KiB cutoff.

### Reproducing the aggregate profile

Only the identifier-free histogram is checked in, not private identifiers, raw
length sequences, or trace event order:

```sh
python3 benchmarks/storage/recovery_sizes.py \
  ../replay-feuer/new-bench-searcher-0/buffer-sizes/source-sizes.bin \
  > benchmarks/storage/recovery-sizes.csv
```

The script groups subchunk downloads by their 4-KiB-aligned size and larger
downloads by their whole-chunk count. Each of the 329 classes records its mean
logical length, rounded down to bytes, and observed count. This preserves the
storage-size classes, frequencies, and almost all of the original byte total
(150,424,762,262 versus 150,424,838,103 bytes).

The benchmark expands those counts, shuffles with fixed-seed xorshift/Fisher-Yates
(seed 42), and takes a prefix without exceeding 100 GB. The resulting fixture has
**80,778 entries and 99,971,787,650 payload bytes**. Sample frequencies are 57.57%,
20.45%, 5.12%, and 16.86% for the four rows above. Its 256-GiB logical capacity
leaves room for whole-chunk padding and shard imbalance without evicting entries.

**Payload contents remain synthetic (`0x5a`).** These captures contain lengths,
not actual object bytes, and recovery does not read payload contents. Each entry
has a distinct integer key and a range starting at zero. This is a size-distribution
fixture, not a reconstruction of the production resident cache: admission,
eviction, repeated keys, overlapping ranges, and recorded arrival order are not
modeled. Batches use the production writer and its packing policy.

### SSD result

On `m8g-32cpu-local-ssd-2`, ten timed opens produced:

| Metric | Value |
| --- | ---: |
| Average open/recovery | 15.556 ms |
| Median open/recovery | 15.191 ms |
| Minimum / maximum | 14.099 ms / 18.290 ms |
| Recovered entries, verified each run | 80,778 |
| Payload bytes | 99,971,787,650 |
| Metadata chunks | 64 (64 MiB) |
| Actual disk bytes occupied | 110,779,342,848 |
| File capacity bytes | 274,877,906,944 |
| Whole test, including setup, verification, and teardown | 60.54 s |
| Whole-process user / system CPU | 13.52 s / 5.76 s |
| Whole-process maximum RSS | 4,768,100 KiB (4.55 GiB) |

The binary was built before measurement. OS CPU/RSS totals include fixture
creation and verification, not just recovery. All timed opens and the extra
untimed reporting open passed verification; the temporary fixture was removed.
[Captured output](recovery-distribution.log),
[OS resource measurements](recovery-distribution-time.txt).
Binggan's percentage deltas in the log compare an intermediate histogram, not a
matched workload comparison; use the absolute measurements above.

The previous [uniform 4-KiB 100-GB result](recovery-100gb.log) had 24,414,062
entries and 1,152 metadata chunks. It is an artificial many-small-entries workload,
**not representative of this recorded download population**. The current 100-GB
command uses the recorded frequencies instead.

### Retest after replacing the free-position B-tree

An unprofiled run on the idle `m8g-32cpu-local-ssd-2` used the same fixture and ten
repeated opens. The tested implementation stores decoded metadata in contiguous
slot arrays, with a free list linked through unused slots; it is not just a
standalone `Vec` of free positions. No production source was changed for this test.

| Metric | Previous unprofiled retest | Slot-array retest |
| --- | ---: | ---: |
| Mean open/recovery | 16.2023 ms | **14.5452 ms** |
| Median open/recovery | 15.6655 ms | **14.4567 ms** |
| Minimum / maximum | 14.9665 / 20.3673 ms | 13.3344 / 16.1602 ms |

Mean latency is 10.2% lower and median latency 7.7% lower than the previous
[unprofiled retest](recovery-rebench.log). Mean latency is 6.5% below the original
15.556-ms measurement. These are historical comparisons, not a matched isolated
change: revisions differ beyond the free-position representation, and ten opens
do not establish statistical significance. This run does not measure recovery
CPU separately; the earlier 62.75% inclusive CPU attribution was not a wall-time
percentage and does not predict a corresponding wall-time reduction.

The fixture again contained 80,778 entries, 99,971,787,650 payload bytes and 64
metadata chunks. Actual disk occupation was 110,779,355,136 bytes. All ten timed
opens plus the reporting open verified successfully; the fixture and its dedicated
temporary parent were removed. Ordinary storage tests: **127 passed, 1 ignored**.
Whole-process measurements, including fixture creation, verification and teardown:
61.20 s wall, 12.27 s user CPU, 6.87 s system CPU, and 4,878,704 KiB peak RSS
(4.65 GiB). [Captured output](recovery-slots.log),
[OS resource measurements](recovery-slots-time.txt).
A subsequent [profile of this exact snapshot](recovery-profile/slots/README.md)
separates CPU, wall-time phases, scheduler waits and physical SSD service.

Snapshot: `b77db4c2b6d3a834e25b99e523825886399bc059` plus working-tree patch
SHA-256 `0e62cfa0aab382b00364fa4a238d513209102d76b6cb429581f4d894a73cfd86`.
Lockfile SHA-256:
`95cc47f11e13839bf0ee33b395107cd70aa78886a0694266fb40b44dc7de04e8`.
The exact source, patch, binary, build/test logs and host snapshots remain at
`/home/ubuntu/feuer-recovery-slots.qIH8qF`. Release build with configured debug
information; no profiler, extra frame-pointer flags or allocator preload.

### Retest with lowest-free-slot scanning — 3b37ccc

Revision `3b37ccc91be7883f5c555dc0199f95cdf3364926` replaces free-list linking with
free-slot counting and lowest-position scanning. On the idle SSD host, the same
100-GB fixture and ten unprofiled opens produced **13.5524 ms mean / 13.8193 ms
median**, range **10.1087–14.7891 ms**. Mean is 6.8% below the preceding 14.5452-ms
run, median 4.4% below; this is a historical comparison, not an isolated randomized
experiment. All opens verified; **128 tests passed, 1 ignored**; fixture removed.

Whole-test resources, including setup and verification: **60.73 s wall**, **12.36 s
user / 6.63 s system CPU**, **4,913,792 KiB peak RSS**. Disk occupation remained
110,779,355,136 bytes. [Output](recovery-scan.log),
[resource measurements](recovery-scan-time.txt),
[detailed profile and provenance](recovery-profile/scan/README.md).

The profile shows free-slot bookkeeping falling from 7.069 to **1.936 CPU-ms/open**,
but total process CPU did not improve in that recording (116.411 → 118.509 ms).
Slot size remains **64 bytes**; this revision does not reduce slot-array memory.

## Small control cases

Without `RECOVERY_100_GB`, the small fixed-size controls still use distinct keys
and 4-KiB payloads:

| Entries | Shards | Capacity | Metadata chunks scanned |
| ---: | ---: | ---: | ---: |
| 0 | 1 | 128 MiB | 1 unwritten head |
| 10,000 | 1 | 128 MiB | 1 |
| 50,000 | 1 | 240 MiB | 3 |
| 50,000 | 4 | 512 MiB | 4 |
| 100,000 | 4 | 512 MiB | 8 |

## Timing and verification

Each case creates a temporary cache through production batch writes, explicitly
writes dirty metadata pages, and closes the cache before benchmarking. `TMPDIR`
selects the fixture filesystem. Fixture size and metadata chunk count are printed
before timing; each fixture is deleted after its case.

Binggan measures wall time for `DiskCache::open`: file/queue initialization,
metadata I/O, and rebuilding the entry index and chunk ownership. Open waits for
all shards' recovery. Recovery does not read or checksum payloads.

Each timed run performs exactly one open. A Binggan plugin then verifies the
exact recovered entry count and the expected range of every key, and drops the
cache **outside the measured interval**. `NUM_ITER_BENCH` must remain 1; use
`NUM_ITER_GROUP` instead (default: 10). Binggan performs one additional untimed
open to obtain the output column, which reports the fixture's entry count.

These are repeated opens in one process with direct I/O, not fresh-process or
cold-device startup measurements. Allocator and device state carry over between
runs. Do not compare them directly with the historical fresh-process results in
[recovery-65b3bf1](recovery-65b3bf1/README.md).
