# Experiments

Keep each experiment's tested configurations and source changes, raw data, and
summary in this repository under `experiments/`. Commit only the small experiment
record. Keep raw data and execution workspaces local and ignored by Git, not in
`../replay-feuer`.

## Results and procedures

- [Tier retrieval cost model, 2026-10-11](cache/tier-retrieval-cost-20261011/README.md):
  memory retention based on usable disk or source fallback, with whole-entry
  alignment, I/O submission, and checksum costs. Two matched replay pairs.
- [Memory policy comparisons](memory/README.md) and [results](memory/results.md).
- [SSD read/write measurements](ssd/ssd-concurrent-read-write.md).
- [Storage I/O](storage/README.md), [read priority](storage/read-priority.md), and
  [recovery](storage/recovery.md).
- [Timestamp-paced cache replay procedure](storage/timed-replay.md).
- [Source retrieval measurements](s3_benchmark.md).

The directory was renamed from `benchmarks/`. Captured logs, patches, source
snapshots, and historical external archive paths retain their original contents.
