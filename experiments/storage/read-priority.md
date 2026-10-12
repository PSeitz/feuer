# Read-priority scheduling under a write backlog

## Policy

- No read demand: writes can fill all 64 ring slots.
- Read demand: stop admitting new writes while four or more writes are outstanding. Fill the other
  slots with reads. Already-submitted writes (including their RMW/short-I/O continuations) must drain;
  they cannot be preempted. This is not an instantaneous cap on existing writes.
- A small write allowance prevents continuous reads from starving independent writes. When no writes
  are pending, reads can use all 64 slots. Overlapping operations retain their ordering.
- Read demand is signaled before admission and removed when the read future finishes or is canceled.
  Submitted reads remain visible in the driver's active set even if their callers cancel.
- Reads and writes each have 64 request permits and 64 MiB of staging-buffer capacity. Writes
  cannot exhaust read admission or queue ahead of reads on a shared buffer semaphore.
- Aligned buffers are charged once; RMW also charges its staging payload. Read-result allocations and
  caller inputs are outside the staging budget. Each class can stage a full ring of aligned 1-MiB requests;
  large RMW requests can reach the buffer limit before filling the ring.

Four is an initial write allowance, not a measured optimum. Four 1-MiB writes are not equivalent to four
4-KiB writes in device cost. No claim is made that background writes are free or that slots bound read p99.

## Comparison, 2026-09-25

These measurements and their validation used an older driver that synchronized initialization writes to
disk; the current example only waits for write completion. Overlap protection remains.

Same host/mount and methodology as the [initial benchmark](README.md): `m8g-32cpu-local-ssd`, Linux
6.17/aarch64, ext4 on local NVMe, release build, four Tokio workers, driver QD64, fully initialized 8-GiB
temporary file. Each cell has two seconds warmup and five seconds measurement. Throughput is decimal
MB/s; API latency is sampled every 32nd measured completion per reader, including admission/queueing.

Both binaries used the updated example with **64 concurrent sequential 1-MiB writers**, plus random
reads in disjoint regions. Each sweep ends with a 64-caller write-only control. The old driver was saved
before replacing its implementation, then run immediately before the new driver. Runs were sequential,
not concurrent. Each run created and removed its own temporary directory.

```sh
# Run once with the old FIFO implementation and once with the read-priority implementation:
cargo run -p feuer-storage --release --example direct_io -- /mnt/local-ssd 5 64
```

Raw results:

- [Before: FIFO and shared admission](fifo-64-writers-20260925.csv)
- [After: read priority and independent admission](read-first-64-writers-20260925.csv)
- [After: read-only control](read-first-read-only-20260925.csv), run with final argument `0`

| Read size | Read callers | Read MB/s before → after | Write MB/s before → after | Read p99 µs before → after |
| --- | ---: | ---: | ---: | ---: |
| 4 KiB | 32 | 5.1 → 506.3 | 1,811.5 → 1,815.3 | 35,457 → 603 |
| 4 KiB | 64 | 10.2 → 719.8 | 1,815.7 → 1,746.5 | 38,300 → 735 |
| 64 KiB | 32 | 82.3 → 2,952.8 | 1,829.8 → 846.2 | 35,724 → 2,064 |
| 64 KiB | 64 | 162.6 → 3,276.0 | 1,844.2 → 763.8 | 40,020 → 3,478 |
| 1 MiB | 32 | 1,136.4 → 3,307.8 | 1,817.8 → 873.0 | 37,599 → 15,107 |
| 1 MiB | 128 | 2,765.3 → 3,030.4 | 1,429.2 → 275.1 | 52,793 → 53,592 |
| No reads | 0 | — | 1,887.6 → 1,875.7 | — |

The old shared admission path let the write backlog delay reads severely. Independent admission plus
scheduling protection removes that behavior in these runs. Larger reads gain bandwidth at the expense
of write bandwidth. Write-only throughput is essentially unchanged. High caller concurrency can still
produce high read latency; the 1-MiB/128-reader case did not improve p99.

A subsequent read-only sweep reached 1,373.4 MB/s for 4-KiB reads at 64 callers and 3,878.4 MB/s for
64-KiB reads at 32 callers, consistent with the original peak read throughput. This is not a blanket
no-regression result: 4-KiB reads at 128 callers measured 1,171.3 MB/s versus 1,371.1 in the initial FIFO
sweep. Read admission is now capped at 64 requests rather than a shared 128, and the new demand tracking
also has a cost; these runs do not isolate their effects. Queue headroom at oversubscription remains a
separate tuning question.

This compares the whole policy change, including admission/buffer accounting, not scheduling alone.
These are single short runs in a fixed order, without steady-state device preconditioning or confidence
intervals. The slowest baseline cases produce particularly few latency samples. Use repeated longer runs
and realistic size mixes to tune the write allowance. This is not a cache-engine or Foyer comparison.

Validation: all 79 workspace tests passed on Linux, including 19 storage tests. Deterministic tests cover
full write-only submission, an arriving read taking the next freed slot, 60 reads plus four writes under
mixed saturation, pre-admission demand/cancellation, independent count/byte budgets, and preserved
RMW/sync ordering.
