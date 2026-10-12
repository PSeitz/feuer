# FETR access-trace replay

Reads the current [FETR v1 format](../access-trace.md) without converting object
hashes into strings. Accepts one `.feuer` package or a `.tar.zst` archive containing
packages from one trace session. Archives are read directly, without extraction.
The older QWFA format and its derived `requests.bin`/`objects.txt` inputs remain
in the private `../replay-feuer/` workspace.

## Workload

Replay defaults to **all recorded request arrivals**. Use `--trace-seconds N`
to select a half-open original-time prefix beginning at the earliest recorded start.
Starts are sorted by timestamp, then request ID. Reading scans all packages twice
to join outcomes that arrive after a selected prefix, while retaining only selected
starts. Archives containing multiple session directories are rejected because
request IDs and clock origins are session-local.

Only request starts drive the replay. The configured cache determines hits,
misses, eviction, admission, and disk writes independently of recorded outcomes.
Recorded outcome counts and unmatched starts are reported separately for diagnostics.
An unmatched start is not an established cache miss. Replay executes **every selected
start with a successful synthetic exact-range download** on a replay miss,
including starts originally failed or incomplete. This measures demand under that
assumption, not original cancellation, source latency, retry/coalescing behavior,
expanded downloads, or the original cache's initial contents. Replays start empty.
The decoder accepts whole-object events, including empty objects, but replay
rejects the unknown-length `0..u64::MAX` sentinel rather than allocate it.
Parseable packages do not establish trace completeness or detect missing packages.

Keep private traces, object identities, and raw results **outside this repository**.
Nothing from the historical experiment directories is needed to build this tool.

## Replay cache configurations (Linux)

One executable uses the real `TieredMemoryDiskCache` for all configurations:
**`--disk-bytes 0` disables disk, a positive capacity enables it.** Request timing,
concurrency, synthetic downloads, and metrics collection are identical in both cases.
There is no separate memory-policy simulator or disk build feature. Opening the
real cache currently requires Linux, even when disk is disabled.

One sender preserves original arrival timing and starts one lookup task per arrival,
without waiting for earlier lookups to finish. Tokio uses one runtime thread per
available CPU, not one outstanding lookup per thread. There is no additional
lookup-admission limit. Recorded starts already reflect application admission, and
Pomsky's split warmup permits do not limit individual cache reads. Feuer's real I/O
queues provide their normal backpressure. Descriptors and unstarted tasks hold no
payload. Every synthetic source miss independently allocates zero-filled payload bytes.
Recorded hashes enter through the cache's `benchmark`-feature API, without a second hash.

```sh
cargo build --locked --release -p feuer-replay --bin replay
# Example 180-second prefix with fresh cache and output directories:
target/release/replay \
  --input /private/path/searcher-0.tar.zst \
  --cache-dir /mnt/local-ssd/fresh-replay-cache \
  --output /private/path/fresh-results \
  --memory-bytes 17179869184 --disk-bytes 0 \
  --trace-seconds 180
```

To test memory and disk together, use the same executable and trace with a positive
`--disk-bytes`, for example `137438953472` (128 GiB), and fresh directories.
Defaults: 16 GiB memory, disk disabled, and 256,000 fixed retrieval-equivalent bytes.
`--fixed-retrieval-equivalent-bytes` sets `RetrievalCostScorer.fixed_cost`, with
`cost_per_byte = 1`. Eviction sample size, buffer retention, and access-history
settings use the current cache environment variables.

Replay writes all registered cache/process metrics in `metrics.prom`, phase
snapshots in `before.prom`, `requests-finished.prom`, and `settled.prom`, and
exact sender/queue/service/completion latency distributions in `summary.csv`.
The summary records `scheduling,task_per_arrival`, and `workers` counts runtime threads.
Queue wait runs from sender emission to the lookup task starting, including descriptor
and runtime scheduling wait. Service time includes waiting inside Feuer, but excludes
that scheduling wait. `replay_queued_requests` counts arrivals whose task has not started,
and `replay_active_requests` counts started lookups awaiting completion. Samples are
not atomic across these gauges.
It verifies exactly-once completion. The five-second observation tail is not a
forced drain, flush, or durability guarantee.

Use `ssh m8g-32cpu-local-ssd-2` for SSD tests and follow
[the timed replay procedure](../experiments/storage/timed-replay.md) for allocator matching,
host checks, OS/device measurements, and reporting. Do not run a full capture or
additional comparison repetitions without an explicit request. Historical pinned
comparisons continue to use their historical harness and inputs.

## Validation

```sh
cargo test --locked -p feuer-replay
cargo clippy --locked -p feuer-replay --all-targets -- -D warnings
```

On Linux, these include round trips through the current writer, recorded-hash
lookup checks, sender timing tests, concurrent scheduling on a single runtime thread,
lookup-error propagation, and replaying the same input with disk disabled and enabled.
Decoder tests cover malformed packages, all event kinds, chronological prefixes and full captures, cross-package outcome joins, incomplete requests,
mixed sessions, and duplicate/inconsistent events.
