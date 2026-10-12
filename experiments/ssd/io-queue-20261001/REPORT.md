# I/O queue overhead: Linux before/after

These are historical measurements from **before** the subsequent buffer-pool API
revision separating retention from allocation and resizing. Do not attribute these
numbers to that later revision.

Host: `m8g-32cpu-local-ssd`, aarch64, Linux 7.0.0-1013-aws, local NVMe,
O_DIRECT. Baseline: `b8881dc9f3e8ebf536b5c0279b127b7b7c623fed`.
Candidate: the queue optimizations and zero-capacity-pool allocation fast path
described below; the latter has since been replaced by requester-owned allocation.
The measured executable is preserved as `candidate` in the remote working directory.

## Changes measured

- Coalesce eventfd writes until the queue's next receive pass. Clear the pending
  flag **before** receiving, not after receiving or while clearing eventfd.
  Requests arriving after that reset arrange another wakeup. If active slots are
  full, their completions provide progress for waiting requests.
- Do not enter the kernel with an empty submission queue; do not call `poll`
  when completions are already available.
- Zero-capacity buffer pools allocate the requested size directly. Previously,
  an unpooled 1-MiB read allocated and zeroed a 4-MiB bucket, then freed it.
  Pools with a nonzero capacity keep their existing size classes and accounting.

No busy polling, increased queue depth, registered buffers, additional kernel
requirements, or changes to oneshot results, cancellation, and buffer ownership.
A cooperative-taskrun experiment did not establish a compelling additional gain
and was not adopted.

## Results

Medians of three unprofiled runs per case, alternating baseline/candidate order.
CPU 100% means one core; MB/s is decimal. QD is application caller count, not
necessarily device queue depth. Percentiles are medians of individual run
percentiles, not pooled distributions. Raw observations are in `results.csv`.

| Read size | Callers | Pool | Read MB/s before → after | CPU % before → after | Read p99 µs before → after |
|---|---:|---|---:|---:|---:|
| 4 KiB | 1 | reused | 51.6 → 51.4 | 21.1 → 21.3 | 87 → 87 |
| 4 KiB | 32 | reused | 1273.0 → 1293.4 | 232.5 → 230.0 | 176 → 174 |
| 4 KiB | 128 | reused | 1713.7 → 1882.5 | 266.5 → 263.0 | 425 → 400 |
| 1 MiB | 32 | reused | 3872.8 → 4000.9 | 12.4 → 12.5 | 9701 → 9122 |
| 4 KiB | 32 | unpooled | 1262.7 → 1294.9 | 248.4 → 234.4 | 175 → 174 |
| 1 MiB | 32 | unpooled | 2637.0 → 3889.4 | 346.9 → 145.7 | 27787 → 9598 |

Mixed case: 32 callers reading 4 KiB and four callers writing 1 MiB in disjoint
regions. Read throughput was 587.7 → 584.3 MB/s; write throughput was
1525.9 → 1637.2 MB/s; read p99 was 575 → 576 µs; CPU was 210.9 → 207.8%.
Mixed throughput varied considerably between runs; do not interpret it as a
reliable write-speed improvement.

The clear improvement is unpooled large reads: about **48% more throughput,
58% less CPU, and 65% lower p99**. Pooled high-concurrency small reads improved
about 10% in this comparison. Low-concurrency latency was unchanged. Small
percentage changes and pooled large-read throughput vary with the host/device;
these short synthetic runs are not evidence of an equivalent application gain.
The pooled large-read path was already near the observed SSD throughput limit.

## Syscall check

Separate eight-second `perf stat` windows, after initialization and warmup,
4-KiB pooled reads with 32 callers. The syscall runs are not included in the
median table. Exact perf output is saved in `final-syscalls-*.txt`.

| Syscall | Baseline | Candidate |
|---|---:|---:|
| write (eventfd notifications during this read-only interval) | 2,410,766 | 997,017 |
| ppoll | 1,046,292 | 761,099 |
| io_uring_enter | 1,000,356 | 989,937 |

Approximately 59% fewer notification writes and 27% fewer poll calls, despite
slightly higher IOPS. An earlier version skipped ready-completion polling but
still issued empty submissions; removing those empty submissions avoided an
increase in `io_uring_enter` calls.

## Method and reproduction

The harness is the existing `feuer-storage/examples/direct_io.rs` with
`benchmark.patch` applied **only in a disposable checkout**, identically for
baseline and candidate. It retains the original four-worker Tokio runtime,
random-read generator, disjoint sequential write lanes, data checks, and latency
sampling. The patch selects one case and chooses either the normal buffer pool
(a 4-GiB budget with the default 7% idle limit) or explicitly unpooled buffers.
No payload is stored in the memory cache.

Each invocation initializes its own fresh 8-GiB file, warms up for two seconds,
measures five seconds, then removes only its temporary directory. No cache
dropping, raw-device writes, affinity settings, or host tuning. Timing includes
DataFile, allocation, admission, queue handoff, completion, and result release.
CPU includes the caller and I/O threads and the final completion drain, as in
the existing example. Latencies are sampled at 1/32 operations. The benchmark
measures I/O completion, not durable-write latency.

Build both versions with the same root Cargo.lock:

```sh
# Apply benchmark.patch to the example in each disposable source checkout.
git apply /path/to/benchmark.patch
PATH="$HOME/.cargo/bin:$PATH" cargo build --release --locked -p feuer-storage --example direct_io
# Preserve the two executables as ./baseline and ./candidate in the run directory.
python3 /path/to/run.py results 3
```

For syscall counts, attach after initialization and the two-second warmup:

```sh
sudo perf stat -p "$pid" \
  -e syscalls:sys_enter_write,syscalls:sys_enter_ppoll,syscalls:sys_enter_io_uring_enter \
  -- sleep 8
```

Remote working directory, binaries, intermediate experiments, and test logs:
`/mnt/local-ssd/feuer-io-opt.DLGmNg/`. The final comparison is `measured-results/`;
earlier result directories contain superseded experiments.

Validation: Linux `cargo test --locked --workspace` passed (231 tests, one
ignored). This includes 113 storage tests and 57 memory tests. The new concurrent
read/idle-wakeup stress test also passed 20 additional consecutive runs. Tests
cover notification coalescing/rearming, idle shutdown, exact unpooled buffer
sizes, and the existing cancellation, disk-region ownership, and partial-I/O paths.
