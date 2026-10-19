# Full-trace baseline replay

Question: does the full trace exercise enough cache reuse and disk eviction to support scoring comparisons beyond the ten-minute cold-fill experiment?

Exactly one baseline replay in each mode, on two isolated `m8gd.8xlarge` workers.
No search, repetitions, source changes, or rebuilds. Full-object mode treats each
exact recorded range as one object, not an original whole file.

**Outcome: both request replays finished, but range disk-drain validation is
blocked.** Range retains 8,191,403 pending payload bytes after the five-second
tail. Full-object validation passes. Both captures are preserved.
No rerun or validation override was made.

- Full trace: **17,369,893 requests**, **31,592,172,620,377 returned bytes** and
  **2,694.824824702 seconds** of arrivals. `--trace-seconds` is omitted.
- Fixed decimal capacities: **100 GB memory / 500 GB disk**.
- Request costs: **256,000** in both tiers, per-byte costs **1**, half-life
  **262,144**. 32 runtime threads, 64 shards, fresh caches.
- Unchanged simulated source: 48 slots, `0.108599 + bytes / 84101038.08` seconds
  per independent callback. No coalescing, admission throttle, or repeat lookup.
- Every request must complete. The five-second observation tail follows full
  request drain and is not a durability or forced-flush guarantee.

[settings.json](settings.json) records the settings and pinned binary/input hashes.
[RESULTS.md](RESULTS.md) reports captured metrics and the validation blocker.

## Local evidence

Private raw captures and execution state are ignored by Git:

- `raw/start.json`: captured arguments, environment and effective settings.
- `raw/cases/{range,full_objects}/`: complete phase snapshots, one-second metrics,
  latency summaries, I/O/CPU/RSS captures, manifests, validation and metric inventory.
- `raw/coordinator-completion.json`: original range validation rejection.
- `raw/eviction-timing.json`: first disk eviction and last-arrival samples.
- `raw/curation-originals/`: unedited reports and operational notes.

The runner and reporters are minimally adapted from the completed
`adaptive-scoring-20261012T1831` experiment. Reporting streams large captures and
retains only scalar phase-window frames in memory. All original telemetry stays
in `metrics.prom`. Memory charge minus idle buffers is **not payload occupancy**.
The earlier first-ten-minute baselines are a different workload duration, not a
paired performance comparison.

## Historical source and reproduction

This uses the adaptive experiment's base `eacaf12ad1fa2870ddcf55fba6ed7d05c32659fc` plus its working-tree patch, not the current checkout. See [its source identity](../adaptive-scoring-20261012T1831/README.md#source-and-binary-identity) for archive and patch hashes. Those source files are local under that experiment's `raw/`.

Stage the binary and trace pinned in `settings.json` at `$ROOT/bin/replay` and `$ROOT/input/trace.tar.zst`, and both Python reporters at `$ROOT/`. On the recorded Linux worker type with jemalloc, sysstat, util-linux, `/usr/bin/time` and `/mnt/local-ssd`, use a root outside the SSD mount:

```sh
bash run-case.sh "$ROOT" range range 256000 256000 262144
bash run-case.sh "$ROOT" full_objects full_objects 256000 256000 262144
```

These commands describe the historical method, not completed runs of the current code. The private input and historical binary are not included in the committed files. `report.py` preserves the range validation failure with exit 1, while `report-metrics.py` can inventory its captured snapshots. Operational notes remain under `raw/`, including `raw/curation-originals/`.
