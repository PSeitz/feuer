# I/O queue overhead: Linux before/after

Historical before/after comparison, O_DIRECT. Baseline b8881dc.
Candidate coalesces wakeups, avoids unnecessary kernel calls, and avoids
oversized unpooled allocations. Predates the later buffer-pool API.
Three alternating runs per case, 2 s warmup + 5 s measurement.
Cells are medians, including per-run p99s. CPU 100% is one core.
Callers are application concurrency. Syscall counts come from separate
8-second windows of pooled 4-KiB reads with 32 callers.

## Results

| Read size | Callers | Pool | Read MB/s before → after | CPU % before → after | Read p99 µs before → after |
|---|---:|---|---:|---:|---:|
| 4 KiB | 1 | reused | 51.6 → 51.4 | 21.1 → 21.3 | 87 → 87 |
| 4 KiB | 32 | reused | 1273.0 → 1293.4 | 232.5 → 230.0 | 176 → 174 |
| 4 KiB | 128 | reused | 1713.7 → 1882.5 | 266.5 → 263.0 | 425 → 400 |
| 1 MiB | 32 | reused | 3872.8 → 4000.9 | 12.4 → 12.5 | 9701 → 9122 |
| 4 KiB | 32 | unpooled | 1262.7 → 1294.9 | 248.4 → 234.4 | 175 → 174 |
| 1 MiB | 32 | unpooled | 2637.0 → 3889.4 | 346.9 → 145.7 | 27787 → 9598 |

## Syscall check

| Syscall | Baseline | Candidate |
|---|---:|---:|
| write (eventfd notifications during this read-only interval) | 2,410,766 | 997,017 |
| ppoll | 1,046,292 | 761,099 |
| io_uring_enter | 1,000,356 | 989,937 |
