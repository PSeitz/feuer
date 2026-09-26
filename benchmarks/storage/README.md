# Direct io_uring driver smoke benchmark

Implementation: [`feuer-storage/examples/direct_io.rs`](../../feuer-storage/examples/direct_io.rs).
Initial FIFO-driver results: [`direct-io-20260925.csv`](direct-io-20260925.csv).
The measurements below predate read-priority scheduling; see the
[read-priority comparison](read-priority.md) for the updated driver under a write backlog.

## Reproduce

On Linux with usable io_uring and a direct-I/O-capable filesystem:

```sh
cargo run -p feuer-storage --release --example direct_io -- /mnt/local-ssd 5
```

An optional final argument selects write-caller concurrency (default: sweep 0 and 4).
The example now also ends with a 64-caller write-only control:

```sh
cargo run -p feuer-storage --release --example direct_io -- /mnt/local-ssd 5 64
```

The example creates a fresh temporary directory under the supplied directory, fully initializes an
8-GiB file, and removes only its own directory on successful exit. It does not open an existing cache or
write a raw device. Allow at least 8 GiB of free space. An interrupted process can leave its named temporary
directory behind.

## Conditions (2026-09-25)

- Host: `m8g-32cpu-local-ssd`, Amazon EC2 `m8gd.8xlarge`, 32 Graviton4 vCPUs.
- Linux `6.17.0-1007-aws`, aarch64, ext4 on `/dev/nvme1n1` (Amazon EC2 NVMe Instance Storage), **not EBS**.
- Release build, one io_uring driver thread, maximum **64 total outstanding operations**, four Tokio workers.
- No polling modes, registered buffers, CPU pinning, compression, checksums, cache lookup, or eviction.
- Every operation uses O_DIRECT. The benchmark includes alignment-buffer allocation/copying, admission,
  scheduling, completion delivery, and compact read-result allocation through the actual `DataFile` API.
- Uniform pseudorandom reads from the first 6 GiB; zero or four sequential 1-MiB writer streams, each using
  its own lane in the final 2 GiB. All requests are aligned and nonoverlapping between streams.
- Two seconds warmup and five seconds measurement per cell; one run per cell in the CSV's order.
  No write-rate cap or per-write synchronization; initialization is synchronized before the sweep.
- Caller concurrency is **not measured device QD**. The ring limit remains 64 even with 128 callers.
  The 128-MiB staging budget can further limit large-operation concurrency.
- Throughput uses decimal MB/s. Read latency includes API queueing and is sampled every 32nd completed
  measured operation per reader. CPU is process user+system time; 100% means one CPU core.
- No concurrent fio/Feuer benchmark was running when the sweep started. No thread-pool or adjacent fio
  control was run, so this does not establish superiority over either backend or Foyer.

## Selected results

| Read size | Read callers | Write callers | Read MB/s | Write MB/s | Read p99 (µs) | CPU % |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 1 | 0 | 50.7 | 0 | 88 | 23.0 |
| 4 KiB | 64 | 0 | 1,357.3 | 0 | 280 | 222.5 |
| 4 KiB | 64 | 4 | 756.3 | 1,697.9 | 708 | 217.8 |
| 4 KiB | 128 | 0 | 1,371.1 | 0 | 472 | 225.1 |
| 64 KiB | 32 | 0 | 3,878.4 | 0 | 628 | 122.0 |
| 64 KiB | 32 | 4 | 3,313.6 | 1,248.0 | 1,167 | 141.9 |
| 64 KiB | 128 | 0 | 3,877.1 | 0 | 2,283 | 130.7 |
| 1 MiB | 32 | 0 | 3,913.5 | 0 | 10,503 | 152.6 |
| 1 MiB | 128 | 0 | 3,851.2 | 0 | 36,274 | 166.9 |

The implementation overlaps reads and writes successfully. For this sweep, 64 callers capture most small-read
throughput, while 32 already saturate large-read throughput. More callers mainly add latency. Concurrent writes
increase total traffic but reduce small-read throughput and increase read latency; there is no universal
read/write split that maximizes every objective.

These are short smoke measurements, not sustained-device limits or statistically established tuning results.
Use longer repeated runs, realistic size mixtures, write-rate controls, unaligned/RMW workloads, and a matched
baseline before selecting production defaults. The public cache remains memory-only.

All 76 workspace tests passed on the same host; storage test files were placed on the local SSD. The 16 storage
tests include deterministic QD64 mixed submission, byte/count limits, overlap ordering, short-I/O transitions,
unaligned reads/writes, cancellation, synchronization, and reopening.
