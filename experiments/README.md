# Experiments

This directory is for high-quality, evidence-backed reports that shape the project's
direction. It is not a development log or an archive of completed tasks.

A report should include:

- A concrete design or performance question and the alternatives being evaluated.
- Reproducible methods: tested revisions, configurations, workloads, and commands.
- Relevant measurements, comparisons, and enough context to interpret them.
- Conclusions that inform a project decision, with tradeoffs and limitations stated.

Do not add feature implementation summaries, routine test or Clippy results, LOC
counts, or task status reports. Feature behavior belongs in the code, tests, and
project documentation. Passing tests alone does not make an experiment.

Keep each experiment's configurations, source changes, raw data, and report under
`experiments/`. Commit only small reports, settings, and source changes.
Keep raw data and execution workspaces local and ignored by Git.

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
