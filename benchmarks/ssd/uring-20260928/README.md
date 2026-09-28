# Benchmark of Feuer's actual io_uring queues

Run started 2026-09-28 on `m8g-32cpu-local-ssd`. Production code is unchanged.
The standalone release-mode binary imports `feuer-storage/src/uring.rs` and
`error.rs` directly. Exact copies, git revision, and the pre-existing tracked
worktree diff are retained in `source/`. The harness has its own Cargo.lock;
bytes, io-uring, libc, tokio, and tracing versions match the root lockfile.

## Workload

- Random reads: 4 KiB, 64 KiB, 1 MiB; application QD 1, 8, 32, 64.
- Sequential 1 MiB writes: application QD 1, 8, 32, 64.
- Mixed: 4 KiB or 1 MiB random reads at QD32, plus 1 MiB writes at QD1, 8, 64.
- Admission saturation: 4 KiB reads at application QD128.
- 23 cases, three repeats each. Seeded shuffled order within each repeat.
- Four adjacent fio io_uring controls per repeat: 4 KiB reads QD1/64,
  1 MiB reads QD32, and 1 MiB writes QD32.

One current-thread Tokio runtime drives FuturesUnordered: QD means outstanding
requests, not OS threads. Each completion is replaced by another request.
Production code supplies a separate read thread and write thread, with a
64-request admission limit per direction. Application QD128 therefore tests
admission waiting; it does not imply kernel/device QD128. Caller execution is
single-threaded and may itself limit throughput. There is no CPU affinity or
host tuning, and no polling/registered-resource optimization added to the ring.

The dedicated 24 GiB file is fully initialized using fio direct writes and
synced before tests. Reads use the first 16 GiB; writes sequentially wrap over
the remaining 8 GiB. All request lengths and offsets are 4 KiB aligned.
Random reads use a fixed xorshift sequence. Write requests copy a reused 1 MiB
payload into the implementation's freshly allocated aligned I/O buffer.
A smoke test verifies write/read equality for all three request sizes.
The harness bypasses DataFile and disk-region/cache management intentionally.

## Timing and reporting

Each case runs 5 seconds of warmup, drains, then measures a fresh 20-second
phase. Warmup and measurement each start with an empty queue. Successfully
completed operations timestamped inside the measurement phase count toward
throughput; final draining is excluded. Latency starts before execute and ends
when its future returns, including admission, allocation, payload copy,
submission, completion, and caller wakeup. Results are released promptly.
Per-direction HDR histograms record microseconds with three significant digits;
sub-microsecond observations are rounded up to one microsecond.

CLOCK_PROCESS_CPUTIME_ID includes the caller and both queue threads. CPU is
sampled around the timed loop, excluding final draining (timer scheduling can
slightly delay the endpoint). 100% represents one core. Histograms and workload
generation cost are included. iostat records device activity every two seconds.
No per-write fsync is timed; a sync_all after each Rust case is outside timing.
These are I/O completion results, not durable-write latency. O_DIRECT bypasses
the host page cache, not SSD caches. This is not an aged/full-device test.

fio controls use the same regions, sizes, application QD, and 5+20-second timing,
but native engine accounting and buffer behavior differ. Their latency is
completion latency, not Feuer API latency. They are reference points, not an
identical-loop backend comparison.

`summary.csv` retains all per-run metrics. `REPORT.md` reports medians of the
three runs; percentile medians are not pooled percentiles. Raw JSON and iostat
are in `results/`. `DONE` is only written after all cases pass validation.

## Reproduction

Preserve the repository layout (or restore source snapshots to the referenced
paths). On the same idle host, use fresh output/data directories in run.py:

```sh
cargo build --release --locked
python3 -u run.py
python3 summarize.py
```

The runner checks the host, local NVMe mount/model, available space, and known
competing benchmark processes. It refuses existing results/data directories.
It removes only its dedicated data file/directory, and only after success.
No raw device writes, formatting, global cache dropping, or unrelated cleanup.

Remote root for this run:
`/home/ubuntu/ssd-bench-results/feuer-uring-20260928/benchmarks/ssd/uring-20260928/`.
