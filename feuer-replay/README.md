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
assumption, not original cancellation, actual source latency, retry/coalescing behavior,
expanded downloads, or the original cache's initial contents. Replays start empty.
The decoder accepts whole-object events, including empty objects, but replay
rejects the unknown-length `0..u64::MAX` sentinel.
Parseable packages do not establish trace completeness or detect missing packages.

Keep private traces, object identities, and raw results in ignored local experiment
directories under `experiments/cache/`. Commit only small settings and summaries.
Nothing from the historical experiment directories is needed to build this tool.

## Full-object simulation

`--full-objects` treats each `(recorded object hash, start, end)` as a separate
immutable whole object with length `end - start`, stored at offset zero. Repeated
identical ranges share one derived identity.
The transformation runs after recorded outcomes are joined and before replay.
Arrival times, request counts, returned bytes and synthetic source allocation are
preserved, while the cache receives whole-object demand and performs whole-object
lookups. Covering or overlapping ranges no longer share entries or demand counters.

This simulates full-object caching of each exact recorded range, not full-file
retrieval or a native whole-object capture. Arrival timing still comes from the
original deployment. Native unknown-length whole-object starts remain unsupported.

Both range and full-object replays use the current
[retrieval-cost scorer](../eviction.md#current-scoring).
Compare the same transformation with matched settings and fresh caches.

## Replay cache configurations (Linux)

One executable uses the real `TieredMemoryDiskCache` for all configurations:
**`--disk-bytes 0` disables disk, a positive capacity enables it.** Request timing,
concurrency, synthetic downloads, and metrics collection are identical in both cases.
There is no separate memory-policy simulator or disk build feature. Opening the
real cache currently requires Linux, even when disk is disabled.

One sender preserves original arrival timing and starts one lookup task per arrival,
without waiting for earlier lookups to finish. Tokio uses one runtime thread per
available CPU. Each thread can run multiple outstanding lookups. There is no additional
lookup-admission limit. Recorded starts already reflect application admission, and
Pomsky's split warmup permits do not limit individual cache reads. Feuer's real I/O
queues provide their normal backpressure. Descriptors and unstarted tasks hold no
payload. Every synthetic source miss independently acquires a buffer through
`TieredMemoryDiskCache::allocate_buffer` and zero-fills its exposed payload.
`into_download` preserves the actual backing-capacity charge, including unused
capacity in the pool's size bucket. Concurrent misses do not share a download.
Identities enter through the cache's `benchmark`-feature API without an additional
cache-side hash. With no requested range, this passes the same allocation charge
as `get_or_fetch_object_with_allocation_charge`. Full-object mode derives new
identities from the recorded hash and exact bounds during input preparation.

This matches the adapter's buffer allocation and charging, not its download
coalescing or sharing of larger merged downloads between slice-objects. The trace
does not record backing capacities or shared-allocation identities. Historical
replays using exact-length `Vec` allocations remain a different source model.

```sh
cargo build --locked --release -p feuer-replay --bin replay
# Replay the first 10 minutes with fresh cache and output directories:
target/release/replay \
  --input /private/path/searcher-0.tar.zst \
  --cache-dir /mnt/local-ssd/fresh-replay-cache \
  --output /private/path/fresh-results \
  --memory-bytes 100000000000 --disk-bytes 500000000000 \
  --trace-seconds 600
```

Comparisons require **100 GB memory and 500 GB disk**, using the exact decimal byte
counts above. Do not vary these capacities without an explicit user request.
The executable's defaults are 16 GiB memory, disk disabled, and 256,000 fixed
retrieval-equivalent bytes, so comparisons must pass both capacity arguments.
`--fixed-retrieval-equivalent-bytes` sets the fallback `cost_per_request` for both tiers.
`--memory-cost-per-request` and `--disk-cost-per-request` independently override it
with finite nonnegative floating-point values. Both `cost_per_requested_byte`
coefficients remain 1. `--access-count-half-life` sets a positive number of caller-demand
requests via `CacheConfig`, overriding `FEUER_ACCESS_COUNT_HALF_LIFE`. Omit it to
use the cache's environment/default setting. Both tiers share the same historian.
Eviction sample size and buffer retention use the current cache environment variables.

### Simulated source

Downloads are immediate by default. `--simulate-s3` makes every independent source
callback wait for one of **48 slots**, then sleep for
`0.108599 seconds + requested_bytes / (80.205 * 1048576 bytes/second)` before
independently acquiring and zero-filling its payload buffer. The slot is held through
allocation and filling. This leaves the arrival scheduler unchanged and adds no coalescing, retries, failures,
or arrival throttling. It is a deterministic delay model.
The fixed calibration comes from [the S3 benchmark](../experiments/s3_benchmark.md),
on another host with larger objects. Runtime scheduling, allocation and slot waiting
add measured latency beyond the formula, and the calibration may not represent these
trace ranges or this hardware.

Both source modes export `replay_source_gets_total` (callbacks started),
`replay_source_bytes_total` (bytes allocated after service),
`replay_source_queue_seconds` and `replay_source_service_seconds` histograms,
and `replay_source_waiting_requests` / `replay_source_active_requests` gauges.
Queue duration measures slot acquisition, service includes delay and allocation.
Histograms use second-based buckets from 0.001 to 1048.576, doubling each step.
These source metrics are separate from lookup queue/service metrics and count only
callbacks actually invoked by the cache.

Replay writes all registered cache/process metrics in `metrics.prom`, phase
snapshots in `before.prom`, `requests-finished.prom`, and `settled.prom`, and
exact sender/queue/service/completion latency distributions in `summary.csv`.
The summary records `scheduling,task_per_arrival`, and `workers` counts runtime threads.
It also records effective `memory_cost_per_request`, `disk_cost_per_request`, both
`*_cost_per_requested_byte` coefficients, `access_count_half_life`, `simulate_s3`,
`source_slots`, `source_latency_seconds`, and `source_bytes_per_second`.
`source_buffer_pool,1` identifies allocation through the cache's pool in both modes.
Immediate
mode records unlimited slots/throughput and zero modeled latency.
Queue wait runs from sender emission to the lookup task starting, including descriptor
and runtime scheduling wait. Service time includes waiting inside Feuer, but excludes
that scheduling wait. `replay_queued_requests` counts arrivals whose task has not started,
and `replay_active_requests` counts started lookups awaiting completion. Samples are
not atomic across these gauges. It verifies exactly-once completion. The five-second
observation tail is not a forced drain, flush, or durability guarantee.

Use `ssh m8g-32cpu-local-ssd-2` for SSD tests and follow
[the timed replay procedure](../experiments/storage/timed-replay.md) for allocator matching,
host checks, OS/device measurements, and reporting. Replay the first 10 minutes
by default with `--trace-seconds 600`. Do not run additional comparison repetitions
without an explicit request. Historical pinned comparisons continue to use their
historical harness and inputs.

## Validation

```sh
cargo test --locked -p feuer-replay
cargo clippy --locked -p feuer-replay --all-targets -- -D warnings
```

On Linux, these include round trips through the current writer, recorded-hash
lookup checks, sender timing tests, concurrent scheduling on a single runtime thread,
lookup-error propagation, scorer/source CLI validation, buffer-capacity charging
and zero-filling of reused buffers in both lookup modes, simulated-source slot waiting
and independent completion, and replaying the same input with disk disabled and enabled.
The replay binary's Linux-only tests do not run on macOS.
Decoder tests cover malformed packages, all event kinds, chronological prefixes and full captures, cross-package outcome joins, incomplete requests,
mixed sessions, and duplicate/inconsistent events.
