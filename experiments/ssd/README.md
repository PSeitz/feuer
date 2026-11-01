# SSD experiments

Each dated directory contains a compact `RESULT.md` for that experiment.
No raw data, CSVs, plots, or benchmark sources are retained here.

Tests used EC2 m8gd.8xlarge, 32 Graviton4 CPUs, local NVMe instance-store
SSD and ext4. MB/s is decimal. KiB/MiB are binary. QD is per-stream
outstanding requests. The SSD had low occupancy, without aged/full-drive
preconditioning or per-write fsync unless stated. Results describe these
workloads, not universal SSD behavior or current cache performance.

Original reports remain at `0a02014^:experiments/ssd/` in Git history.
Commit `0a02014` removed them on October 29, 2026 at 14:11 CET.
These concise summaries were reconstructed from those reports. No tests
were rerun. These summaries are tracked in Git. Other experiment artifacts
remain ignored.
