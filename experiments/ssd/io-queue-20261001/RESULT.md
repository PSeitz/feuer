# Historical queue and allocation optimization

Direct I/O. Three alternating baseline/candidate runs per case,
2 s warmup + 5 s measurement. Candidate coalesced wakeups, avoided
unnecessary kernel calls, and fixed oversized unpooled allocations.

Unpooled 1-MiB reads, 32 callers:
- Throughput: 2,637 → 3,889 MB/s.
- CPU: 347% → 146% of one core.
- Read p99: 27.787 → 9.598 ms.

Low-concurrency latency was unchanged. Separate syscall measurements
showed 59% fewer notification writes and 27% fewer poll calls.
These results predate the later buffer-pool API revision and should not
be attributed to it or treated as current application performance.
