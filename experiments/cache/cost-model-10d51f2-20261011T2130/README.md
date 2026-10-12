# Cache cost-model experiments — 2026-10-11

Seven matched baseline/candidate pairs tested five changes to tiered-cache
retention and write admission. No production policy was adopted.

- [Findings](FINDINGS.md): tradeoffs and limitations.
- [Results](REPORT.md): traffic, hits, CPU, latency, memory accounting, and eviction.

## What was tested

| Test | Baseline → candidate | Pairs | Description | Settings |
| --- | --- | ---: | --- | --- |
| Usable fallback | Fixed256000 → dynamic linear fallback, assumed-disk linear → dynamic linear fallback | 2 | [Model](lanes/fallback/design.md) | [Settings](lanes/fallback/experiment.json) |
| Whole-entry read work | Fixed256000 → dynamic aligned/submission/checksum fallback, assumed-disk whole-entry → dynamic whole-entry | 2 | [Model](lanes/read-work/design.md) | [Settings](lanes/read-work/experiment.json) |
| Reuse-based write admission | Default → prior conditional-demand savings gate | 1 | [Model](lanes/reuse-admission/design.md) | [Settings](lanes/reuse-admission/experiment.json) |
| 32-MB admission limit | No limit → inclusive 32000000-byte maximum | 1 | [Model](lanes/tail-32mb/design.md) | [Settings](lanes/tail-32mb/experiment.json) |
| 64-MB admission limit | No limit → inclusive 64000000-byte maximum | 1 | [Model](lanes/tail-64mb/design.md) | [Settings](lanes/tail-64mb/experiment.json) |

Every case used **100000000000 memory bytes**, **500000000000 disk bytes**, the
capture's first **600 seconds**, original pacing, exact immutable whole slices,
32 workers, 64 shards, independent zero-filled source downloads, fresh caches,
and a five-second observation tail. Each completed **1606645 requests**, returning
**2800015397526 bytes**, with zero errors and zero pending/queued writes.
The disk victim scorer stayed fixed256000, with no disk eviction.

## Source changes

[patches.tar.zst](patches.tar.zst) contains the five candidate patches and
`baseline.patch`, the working-tree changes on top of commit
`10d51f253ecd13e05592a9f80512dbf4d237b93f`. Original source paths are preserved.
The archive was checked against all six original patches.

```sh
shasum -a 256 -c patches.sha256
mkdir -p /tmp/feuer-cost-model-patches
zstd -dc patches.tar.zst | tar -xf - -C /tmp/feuer-cost-model-patches
```

## Raw data stays local

Only the small experiment record above belongs in Git. These larger archives
and their checksum files are retained in this directory but ignored by Git:

- `measurements.tar.zst`: all fourteen runs' raw measurements, checksum calibration,
  baseline source, detailed reports, and original audit receipts. All 479 selected
  files were checked, and all fourteen reports reproduced from this archive.
- `workspace.tar.zst`: the complete original workspace, including input capture,
  binaries, source snapshots, and build/provisioning history. All 5952 original
  files were checked before consolidation.

The input capture's SHA-256 is
`a13aeae5c77da3709814fef42708d6cb55a867a553ba01dfc8c7e8338e7807f7`.
All five experiment instances and root volumes were deleted after measurement.
No archived provisioning, cleanup, or measurement scripts should be run.
