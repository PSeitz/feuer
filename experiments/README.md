# Experiments

Keep each experiment's tested configurations and source changes, raw data, and
summary in this repository under `experiments/`. Commit only the small experiment
record. Keep raw data and execution workspaces local and ignored by Git, not in
`../replay-feuer`.

## Results and procedures

- [Cost modelling, 2026-10-11](cache/cost-model-10d51f2-20261011T2130/README.md):
  usable-fallback memory scoring, whole-entry read costs, reuse-based write
  admission, and decimal 32-MB/64-MB source-admission limits. The Git record contains
  summaries, model descriptions, settings, and compressed source patches.
  Raw measurements and execution workspaces are local-only and ignored by Git.
- [Memory policy comparisons](memory/README.md) and [results](memory/results.md).
- [SSD read/write measurements](ssd/ssd-concurrent-read-write.md).
- [Storage I/O](storage/README.md), [read priority](storage/read-priority.md), and
  [recovery](storage/recovery.md).
- [Timestamp-paced cache replay procedure](storage/timed-replay.md).
- [Source retrieval measurements](s3_benchmark.md).

The directory was renamed from `benchmarks/`. Captured logs, patches, source
snapshots, and historical external archive paths retain their original contents.
