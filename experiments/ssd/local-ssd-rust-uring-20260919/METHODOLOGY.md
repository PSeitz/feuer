# Rust validation and io_uring: methodology

## Purpose and environment

1. Independently reproduce the original fio measurements using **Rust-issued I/O** through Linux AIO.
2. Change only the Rust I/O backend to **io_uring**, retaining the same workload loop.
3. Cross-check selected workloads with adjacent native fio measurements for each backend.

Host: `m8g-32cpu-local-ssd`, EC2 `m8gd.8xlarge`, 32 Graviton4 vCPUs, 128 GiB advertised RAM. The device is the local Amazon EC2 NVMe instance-store SSD `/dev/nvme1n1`, ext4 at `/mnt/local-ssd`, not EBS. Linux `6.17.0-1007-aws`, aarch64; fio 3.36. Scheduler `none`, `max_sectors_kb=128`, `nr_requests=127`. No host tuning or cache dropping.

The Rust program was compiled with **rustc 1.98.0**, release optimization, and `panic=abort`. Locked dependencies include `io-uring 0.7.15`, `libc 0.2.189`, and `serde_json 1.0.151`.

**Python only orchestrates processes and produces reports.** The Rust binary itself allocates buffers, generates offsets, submits I/O, waits for completion, paces writers, and measures throughput. It does not call fio. This is a storage microbenchmark, not a benchmark of foyer's application-level cache/storage path.

## Rust implementation

- Regular files opened with `O_DIRECT`; buffers allocated with 4096-byte alignment, one buffer per outstanding slot.
- One worker thread per stream; no CPU affinity. Mixed reader/writer threads synchronize their start after initialization.
- Linux AIO backend (named `libaio` in the results) calls `io_setup`, `io_submit`, `io_getevents`, and `io_destroy` via Linux syscalls. It does **not** use the libaio userspace library.
- io_uring backend uses the `io-uring` crate with ordinary `Read`/`Write` operations. No SQPOLL, IOPOLL, fixed/registered buffers, registered files, or forced async dispatch.
- Both backends submit **one operation per submission call** and consume **one completion at a time**, maintaining up to the specified queue depth. This intentionally does not test submission batching optimizations.
- Every completion is checked for the requested byte count and a currently active slot ID. Buffers are not reused until completion. All requests are drained before resources are freed; short/failed I/O aborts the run.
- Sequential offsets advance by the block size and wrap at file boundaries. Random reads use a fixed-seed xorshift generator with replacement over block-aligned offsets.
- Every write buffer is refreshed with a changing deterministic 64-bit-word pattern. This is **not fio's random-buffer generator**.
- Writer pacing follows cumulative submitted bytes against elapsed time; the writer sleeps only with no outstanding requests. Caps are average submission targets, not instantaneous limits. Warmup backlog/catch-up can cause small differences between the cap and achieved measured bandwidth.

## Files, matrix, and timing

A new, dedicated directory on the SSD contains a **128 GiB read file** and **64 GiB write file**. Rust initializes both completely with sequential 1 MiB direct writes and `sync_all`; it does not merely allocate sparse files. The same files are used for both phases. Results and logs go to EBS. Measured writes are always sequential; no random-write throughput test is run.

Each Rust backend runs the same **66-case** matrix:

| Workload | Cases |
|---|---|
| Random reads | 4, 16, 64, 256 KiB; 1, 4 MiB × QD1 and QD32 |
| Sequential writes | Same six block sizes × QD1 and QD32 |
| Mixed sequential reads/writes | Six read sizes at QD32 × seven writer settings |
| Writer settings | No writer; 180, 450, 910, 1,370, 1,640 MB/s; uncapped |

The concurrent writer always uses **1 MiB blocks**, up to QD32. Rate `-1` in the CSV means uncapped; rate `0` in mixed cases means no writer. Caps are fixed to the original experiment, not recalibrated.

Each main measurement has **5 seconds warmup + 30 seconds measurement**. Cases use the same shuffled order (seed `20260919`) for both phases. The full Linux AIO phase runs first, then io_uring. The phases are **not interleaved**, so time/device-state drift remains a potential confounder.

For each backend, **nine fio controls** run immediately before matching Rust cases: 4 KiB random reads at QD1/QD32; 1 MiB random reads at QD32; 4 KiB and 1 MiB writes at QD32; 16 KiB sequential reads with write caps 0/450/910 MB/s; 64 KiB sequential reads with a 910 MB/s write cap. fio uses the same files, timing, queue depths, and caps, with submit/completion batches explicitly set to one. Its io_uring controls use non-vectored I/O with polling and registration disabled.

At the end of each phase, 16 KiB reads with write caps 450 and 910 MB/s are repeated with **5 seconds warmup + 60 seconds measurement**. Main table values remain the original 30-second measurements. Eight two-second smoke tests precede the full matrix; these are tagged `smoke` in the CSV and excluded from report tables.

## Correctness and measurement checks

Before benchmarking, Rust tests check ABI struct sizes, sequential wrapping, random-offset bounds/alignment, and buffer alignment/refill. An on-device self-test exercises both backends with eight concurrent requests, distinct offsets/patterns, unique completion IDs, fsync, and **byte-for-byte readback**. These checks validate the custom I/O implementation; the full throughput workload does not verify read contents on every operation.

Rust throughput counts successfully completed bytes inside the shared measurement window, divided by exactly 30 or 60 seconds. Completions outside that window—including final draining—are excluded. fio has its own boundary accounting; differences are small relative to a 30-second run but make this an independent replication rather than identical implementations.

Per-worker CPU is measured with `CLOCK_THREAD_CPUTIME_ID` over the measured phase. fio CPU is user + system CPU. Mixed-case CPU totals sum both workers; 100% corresponds to one CPU core. Per-second completed-byte buckets and submitted/completed totals are archived outside this results folder. `iostat -dxm -y 5` monitors the disk throughout.

All throughput is **decimal MB/s**: completed bytes / seconds / 1,000,000. Both achieved read and achieved write rates must be compared, particularly when high write caps cannot be reached.

## Interpretation limits

- This compares the tested default-style backends, not fully tuned io_uring with batching, SQPOLL, or registered resources.
- The Rust and fio implementations have different buffer refill, random generation, instrumentation, pacing, and timing overhead. Small-block throughput/CPU differences are not evidence that the original test was “Python-limited.”
- Single main runs plus selected repeats are not confidence intervals. Mixed-workload differences can reflect a different read/write bandwidth balance.
- Low-occupancy files on an initially empty SSD; not full-drive/aged steady-state testing. The device is not reinitialized between backend phases.
- Direct I/O bypasses host page cache, not device caches. No per-write fsync in the measured workload. Application blocks larger than 128 KiB may be split by the kernel.

## Reproduce

Source, `Cargo.lock`, raw results, and logs are stored **outside the compact results folder**:

- Local: `~/Development/benchmarks/results/local-ssd-rust-uring-20260919/`
- Instance: `/home/ubuntu/ssd-bench-results/local-ssd-rust-uring-20260919/`
- Original fio baseline: `~/Development/benchmarks/results/local-ssd-20260919/`

Prerequisites: the same idle host, recorded Rust toolchain, fio 3.36, Python 3, and `iostat` from `sysstat`. Allow about 95 minutes and >250 GiB free space. The suite creates about 192 GiB of temporary data and writes several TB; do not overlap it with other benchmarks.

On the instance, create a fresh directory on EBS and copy **source only**, not existing outputs or completion markers:

```sh
. "$HOME/.cargo/env"
BASE=/home/ubuntu/ssd-bench-results/local-ssd-rust-uring-20260919
RUN=/home/ubuntu/ssd-bench-results/rust-uring-repro-$(date -u +%Y%m%dT%H%M%SZ)
mkdir "$RUN"
cp "$BASE/Cargo.toml" "$BASE/Cargo.lock" "$BASE/run_suite.py" "$RUN/"
cp -R "$BASE/src" "$RUN/"
cd "$RUN"
cargo test --release --locked
cargo build --release --locked
./target/release/ssd-bench selftest "/mnt/local-ssd/$(basename "$RUN")-selftest"
python3 -u run_suite.py > progress.log 2>&1
```

Run inside `tmux` or another persistent session. The runner checks the hostname, device model, mount identity, free space, and absence of prior outputs/data. Do not disable these checks with Python `-O`. If device enumeration changes, verify the device before adapting them.

Successful completion produces `DONE` and removes only the dedicated data files/directory. On failure, inspect logs and confirm no I/O is active before cleaning up only that run's files.

For reporting, copy the new outputs to a working directory outside `foyer2`, with the original archive available alongside it as `local-ssd-20260919`. Run the archived `summarize.py` there using Python and `matplotlib==3.10.9`. It regenerates `REPORT.md`, `comparison.csv`, and `comparison.png`; copy only those and this methodology into the final results folder.
