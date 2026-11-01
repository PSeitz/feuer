# SSD experiments

Each dated directory contains a `RESULT.md` with the original report's
measurement tables and short setup notes. Table rows are preserved verbatim.
Per-request logs, CSVs, plots, and benchmark sources are omitted.

Tests used EC2 m8gd.8xlarge, 32 Graviton4 CPUs, local NVMe instance-store
SSD and ext4. MB/s is decimal. KiB/MiB are binary. QD is per-stream
outstanding requests. The SSD had low occupancy, without aged/full-drive
preconditioning or per-write fsync unless stated. Results describe these
workloads, not universal SSD behavior or current cache performance.

Original reports remain at `0a02014^:experiments/ssd/` in Git history.
Commit `0a02014` removed them on October 29, 2026 at 14:11 CET.
No tests were rerun. These result tables are tracked in Git. Other
experiment artifacts remain ignored.
