# Recovery CPU and I/O profile — 1bcc3c3

Latest: [lowest-free-slot scanning profile — 3b37ccc](scan/README.md),
including unprofiled results, DWARF inline expansion, wall-time phases and off-CPU/SSD tracing.

Previous: [profile after replacing the free-position B-tree](slots/README.md).

For the earlier idle-host profile with **wall-time phases, off-CPU stacks and
physical SSD requests**, see [the detailed profile](detailed/README.md).

Profiled committed revision `1bcc3c3` on `m8g-32cpu-local-ssd-2`.
The pre-existing staged metadata-typing changes were not included. The fixture
uses the recorded download-size distribution: 80,778 entries, 99,971,787,650
payload bytes, 64 shards and 64 metadata chunks. Actual disk occupation in this
run was 110,779,424,768 bytes; capacity was 256 GiB.

## Top CPU functions

Percentages below are **self sampled CPU cycles**, including user and kernel
execution in the benchmark process, not percentages of elapsed wall time.
There were 124,193 samples and zero lost samples across 100 profiled opens.

| Symbol, shortened where necessary | Self cycles |
| --- | ---: |
| Shard-recovery loop, inlined into `CachedParkThread::block_on<recover_shard>` | 55.72% |
| `__aarch64_swp4_rel` (two symbol entries combined) | 7.10% |
| B-tree node `insert_recursing<EntryMetadataLocation>` | 5.08% |
| `page_format::encode_page` | 3.50% |
| `__memset_zva64` | 3.19% |
| `__memcpy_sve` | 2.70% |
| `__arch_clear_user` | 2.40% |
| `malloc_consolidate` | 2.11% |
| `malloc` | 2.08% |
| `_int_malloc` | 1.63% |

The 55.72% symbol is **not time spent parked in Tokio**. Optimized recovery and
B-tree operations are inlined into it. Source-line attribution shows B-tree
searches, comparisons and insertion code: `macros.rs:180` accounts for 14.83%,
`search.rs:226` for 9.45%, and the metadata-location definition for 5.94%.

This fixture scans 1,370,880 metadata record positions, of which **1,290,102 are
unused**. Each unused position is inserted into `MetadataPages::free_entry_positions`,
a `BTreeSet`. Rebuilding these free positions is a prominent CPU cost in this
mostly-empty-metadata workload. Do not extrapolate it as a fixed per-payload-byte cost.
The shard-recovery wrapper is 89.28% inclusive; `DiskCacheShard::insert_entry` is
6.32% inclusive but only 0.65% self. Inclusive percentages overlap and must not be added.

Reports: [self](cpu-self.txt), [inclusive](cpu-children.txt),
[source lines](cpu-lines.txt). Whitespace has been normalized; measurements and
symbol names are unchanged.

## I/O versus reconstruction

| Per-open measurement | Mean | Median |
| --- | ---: | ---: |
| Profiled wall time | 25.595 ms | 24.505 ms |
| Aggregate process CPU time | 152.059 ms | 149.284 ms |
| Opening 4-KiB probe, kernel submission to completion | 3.761 ms | 3.882 ms |
| Metadata I/O window, first submission to last completion | 10.050 ms | 9.859 ms |
| Individual 1-MiB metadata read, kernel submission to completion | 5.351 ms | 5.302 ms |

Individual metadata reads have p95 7.786 ms. At the application API boundary,
read duration averages 7.685 ms; this also includes buffer allocation, queueing
and waking the consumer. The sum of API read durations averages 491.833 ms per
open, but the 64 reads overlap: **that sum is not wall time**.

There is meaningful I/O latency as well as substantial reconstruction CPU work.
The CPU/wall ratio is 5.94, meaning roughly six CPU cores' worth of aggregate
execution during an average open; recovery uses blocking shard tasks in addition
to the four runtime workers. CPU work overlaps metadata requests, so the probe,
I/O window and CPU time do not form an additive wall-time breakdown. In-flight
I/O time is not an exact measure of time the process was off-CPU waiting for I/O.

Another SSD replay was running on the host. CPU samples target only the benchmark
process, and I/O statistics include only its requests, but their latency can be
affected by competing disk and CPU work. Frame pointers, metrics and sampling
also perturb timing. These are **not replacement timings** for the unprofiled
15.556-ms benchmark and do not establish a pure CPU-bound or pure I/O-bound workload.

All 100 profiled opens plus the extra untimed reporting open passed verification.
The profiling build's ordinary storage tests passed: 124 passed, 1 ignored.
No benchmark temporary directories remained after completion.

[Captured log](profile.log), [per-open measurements](opens.csv),
[summary](summary.json).

## Method and reproduction

Release debug information was already enabled (`debug = "full"`, equivalent to
`debug = true`). The isolated build also used `-C force-frame-pointers=yes`.
A profiling-only Binggan plugin enables both perf recorders immediately before
each timed open and disables them before verification and teardown, waiting for
FIFO acknowledgments. Its highest plugin priority ensures correct event ordering.
Fixture creation and the extra reporting open are not sampled.

CPU recording uses `cycles:uk`, 4,999 Hz and frame-pointer call stacks. A second,
system-wide recorder captures `io_uring_submit_req` and `io_uring_complete`.
Submissions are filtered to the benchmark PID; completions are matched by ring,
request pointer and user-data value, including completion in another context.
All 6,500 requests matched: 100 opening probes and 6,400 metadata reads, with no
unmatched requests or read errors. Metadata reads return exactly 1 MiB each.

In a fresh isolated checkout of `1bcc3c3`, apply [instrumentation.patch](instrumentation.patch)
with `git apply --unidiff-zero /path/to/instrumentation.patch`, then build from that checkout:

```sh
PATH="$HOME/.cargo/bin:$PATH" CARGO_PROFILE_RELEASE_DEBUG=2 \
  RUSTFLAGS='-C force-frame-pointers=yes' \
  cargo test --locked --release -p feuer-storage --lib --no-run --message-format=json \
  > profile-build.jsonl
```

Run `bash /path/to/recovery-profile/run.sh /path/to/isolated-checkout` on the SSD
host. The [script](run.sh) requires passwordless sudo for perf; the benchmark runs
as the normal user. It performs 100 repeated opens to obtain sufficient samples,
not a trace replay. The script writes raw profiles and reports into the isolated
checkout; keep the exact executable with the profiles for symbol resolution.

The measured checkout, executable and raw profiles remain on the host under
`/home/ubuntu/feuer-recovery-perf.zx8nbq`; the dedicated build directory is
`/home/ubuntu/feuer-recovery-perf-target`. No production profiling API was added.
