# I/O queue overhead: Linux before/after

- **Comparison:** baseline `b8881dc` versus changes that combine wakeups, skip unnecessary kernel calls, and avoid oversized unpooled allocations.
- **I/O:** direct I/O through Feuer. This test predates the later buffer-pool API.
- **Duration:** 2 seconds of warmup and 5 seconds of measurement per case.
- **Repetitions:** three runs per version, alternating baseline and candidate. Tables report medians.
- **Syscall counts:** separate 8-second measurements of pooled 4 KiB reads with 32 callers.

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
