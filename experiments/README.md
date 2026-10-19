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

Reports describe their recorded revisions and workloads, not necessarily the
current implementation. In particular, the range-aware cache investigations below
predate removal of range support in `d965556`. Failed or incomplete runs are
retained for their diagnostic findings, not as successful performance comparisons.
Recomputation generally requires the local ignored captures or external artifacts
identified by each report.

- [Reclaiming buffer capacity, 2026-10-17](cache/same-range-compaction-20261017/README.md):
  capacity-based memory compaction reduced SSD reads by 28% at 11% more CPU in one matched replay pair.
- [Tier retrieval cost model, 2026-10-11](cache/tier-retrieval-cost-20261011/README.md):
  memory retention based on usable disk or source fallback, with whole-entry
  alignment, I/O submission, and checksum costs. Two matched replay pairs.
- [Memory policy comparisons](memory/README.md) and [results](memory/results.md).
- [SSD read/write measurements](ssd/ssd-concurrent-read-write.md).
- [Storage I/O](storage/README.md), [read priority](storage/read-priority.md), and
  [recovery](storage/recovery.md).
- [Timestamp-paced cache replay procedure](storage/timed-replay.md).
- [Source retrieval measurements](s3_benchmark.md).

## Cache investigations

- [Production compaction](cache/production-compaction-20261018/README.md):
  observational retention, write-admission and CPU-profile evidence, with workload confounders.
- [Disk-bypass queue evidence](cache/disk-bypass-queue-review/README.md):
  the deployed benchmark predates the payload-admission queue and cannot establish its pressure behavior.
- [Exact-request amplification](cache/disk-read-amplification-2cf2f27/README.md):
  a deterministic 256× read-amplification reproduction at a pinned range-aware revision.
- [Partial-read checksum replay](cache/partial-read-checksums/REPLAY.md) and
  [range geometry](cache/partial-read-checksums/static-analysis/README.md):
  measured read-learning effects and conditional opportunities for further checksum preparation.
- Read-volume investigations:
  [admission tradeoffs](cache/read-volume-admission/README.md),
  [missing checksum coverage](cache/read-volume-amplification/README.md),
  [concurrent duplicates](cache/read-volume-duplicates/README.md),
  [range fragments](cache/read-volume-ranges/REPORT.md), and
  [memory retention](cache/read-volume-retention/README.md).
- [Adaptive scoring](cache/adaptive-scoring-20261012T1831/README.md):
  exploratory ten-minute sweeps, limited by cold fill and source backlog.
- [Full-trace comparison](cache/full-trace-20261013T0734/README.md):
  completed caller work, but incomplete range-mode write drain.
- [Full-trace scoring failures](cache/full-trace-scoring-20261013T1745/RESULTS.md) and
  [read-buffer accounting](cache/replay-buffer-allocation/ACCOUNTING.md):
  OOM evidence and a controlled reproduction of excess concurrent read-buffer retention.

The directory was renamed from `benchmarks/`. Captured logs, patches, source
snapshots, and historical external archive paths retain their original contents.
