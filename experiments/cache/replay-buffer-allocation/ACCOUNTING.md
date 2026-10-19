# Whole-read buffer retention under piece-level admission

## Question and finding

Does limiting queued and active I/O pieces also limit the number of whole-read
buffers retained by concurrent callers, or is admission needed at the whole-read
level? A controlled reproduction of the historical `ReadQueue::read` path observed
235 live 3-MiB buffers with a 64-piece channel, exceeding even the combined
64-active/64-waiting piece limit.

This is evidence of a read-buffer admission gap under that source revision, not an
idle-counter arithmetic defect, a permanent leak, or proof of every replay OOM's
cause. No alternative admission implementation was measured. The result supports
considering whole-read admission independently of SSD piece concurrency, but does
not select a waiting, rejection, or reclamation policy.

## Method and source

The test uses the actual `ReadQueue::read` and 64-slot channel with 256 concurrent
3-MiB reads and a pool obtained from a cache with a 16-MiB target. It polls callers
and consumes one queued piece per loop. Piece completions are simulated immediately:
no SSD I/O is issued and completed caller results are dropped. Weak references track
observed live buffer owners without retaining them.

The tested frozen source is `raw/source.tar.gz`, based on
`cce6ce5b0ce1d6341d384921d1eef49780918711` with the working-tree changes retained
in `raw/source.patch`. Archive SHA-256:
`a2075db34c972befa7ab4bfea1f63cbea3d7538b40d5132d783e421c594cad34`.
The archive's `uring.rs`, `uring/tests.rs`, `buffer.rs`, `Cargo.toml`, and `Cargo.lock`
match that base commit. The archive, not today's implementation, defines the tested
source. The inspected `uring.rs` SHA-256 is
`1dd7af70dfc12000db6c871742f9b17a88aa2821fd753fe5b812fcde5b2b7764`.

Host: `m8g-32cpu-local-ssd-2`. Historical workspace:
`/home/ubuntu/feuer-read-admission.WvDtiT`. The retained test log uses a shared target
directory, so this was not an independently clean build. A toolchain version is not
recorded in the focused reproduction's setup or test log.

## Measurements

| Measurement | Observed |
| --- | ---: |
| Concurrent reads | 256 |
| Bytes per whole read | 3,145,728 |
| Piece-channel slots | 64 |
| Peak observed live whole-read buffers | 235 |
| Corresponding buffer capacity | 739,246,080 bytes (705 MiB) |
| Cache charge during the test | 0 |
| Idle bytes during the test | 0 |
| Completed reads | 256 |
| Surviving tracked owners after completion | 0 |

The capacity figure is the observed owner count multiplied by the 3-MiB read size,
not an RSS measurement. The test checks that the peak exceeds 128, rather than
requiring precisely 235 on every execution. Owners are discovered when their pieces
are dequeued, so the peak need not include every allocated buffer at every instant.
The simulated completion order is not an SSD throughput or latency benchmark.

The historical path reserves one piece slot, allocates the whole destination, then
waits for slots for later pieces while retaining that destination. Completed pieces
release I/O slots rather than whole-read ownership. The cache charge excludes these
checked-out buffers until cache admission. The reproduction therefore distinguishes
live allocation retention from cache-entry and idle accounting.

## Corroborating historical capture analysis

The retained derived data in [raw/pool-accounting.json](raw/pool-accounting.json)
contains these simultaneous samples from earlier full-trace runs. Values are decimal
GB. Pool capacity sums used and idle bucket gauges, overlaps cache charge, and must
not be added to it.

| Case / sample | Cache charge | Pool capacity | Idle portion | RSS |
| --- | ---: | ---: | ---: | ---: |
| Range baseline, 1488.239 s | 99.958 | 122.660 | 1.694 | 128.165 |
| Full-object baseline, 1488.217 s | 99.987 | 122.345 | 1.523 | 128.810 |
| Range half-life 4096, 1517.322 s | 99.939 | 125.086 | 1.842 | 130.401 |

These numbers were checked against the derived JSON during curation, not recomputed
from the original full-trace captures. The archived
[analysis script](raw/analyze-pool-accounting.py) identifies those external inputs.
This correlation does not attribute buffers to individual readers or establish the
cause of a final OOM spike. No full-trace replay was rerun.

## Reproduction

The small [test source](read-admission-test.rs) is retained outside the raw archive.
To reproduce the historical setup on the Linux host, use a fresh local workspace:

1. Extract the hash-verified `raw/source.tar.gz` into it. This archive is local-only
   and is required for exact source reproduction. The base commit alone omits the
   captured working-tree changes.
2. Append `read-admission-test.rs` to `feuer-storage/src/uring/tests.rs`, which is the
   existing test module with the queue helper. Do not append it to `uring.rs`.
3. From that workspace run:

   ```sh
   PATH="$HOME/.cargo/bin:$PATH" TMPDIR=/mnt/local-ssd \
     cargo test --locked -p feuer-storage --lib \
     uring::tests::split_read_admission_retains_more_buffers_than_io_slots -- --exact --nocapture
   ```

This command is reconstructed from the saved test and log. It was not rerun during
curation. The helper opens a direct-I/O temporary file even though completions are
simulated, so Linux and a compatible temporary filesystem are still required.

## Evidence and limits

- [Focused test log](raw/read-admission-test.log) records the 235-buffer result.
- [Setup capture](raw/read-admission-setup.txt) records the source hash and workspace.
- [Original investigation](raw/ACCOUNTING-INVESTIGATION.md) preserves the broader
  ownership audit and historical fix discussion.
- [Allocation-reclamation review](raw/ALLOCATION-RECLAIM-REVIEW.md) is an archived
  static design review, not a measured fix or release report.

Raw links require the local archive. Later admission or reclamation changes can
invalidate this reproduction's expectations. This report makes no claim about
current source behavior, total process-memory control, or production OOM prevention.
