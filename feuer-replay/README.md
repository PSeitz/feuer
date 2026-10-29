# Access-trace replay

Replay recorded cache requests against Feuer with a fresh cache. The tool reads
[FETR v1 packages](../access-trace.md) and measures how the current cache
handles the workload. Recorded outcomes are used for diagnostics and object lengths.
The replay cache determines its own hits, misses, evictions, admissions, and writes.

Running the cache requires **Linux**, including memory-only replay.

## Quick start

Build the executable, then replay the first 10 minutes of a capture:

```sh
cargo build --locked --release -p feuer-replay --bin replay

target/release/replay \
  --input /private/path/searcher-0.tar.zst \
  --cache-dir /mnt/local-ssd/fresh-replay-cache \
  --output /private/path/fresh-results \
  --memory-bytes 100000000000 --disk-bytes 500000000000 \
  --trace-seconds 600
```

Use fresh cache and output directories for each run. Input can be a single `.feuer`
package or a `.tar.zst` archive containing packages from one trace session. Archives
are read directly, without extraction.

### Comparison runs

- Use **100 GB memory and 500 GB disk**, with the exact decimal byte counts above.
  Change these capacities only when explicitly requested.
- Use `ssh m8g-32cpu-local-ssd-2` for SSD tests. Follow the
  [timed replay procedure](../experiments/storage/timed-replay.md) for allocator
  matching, host checks, OS/device measurements, and reporting.
- Replay the first 10 minutes by default with `--trace-seconds 600`. Run additional
  comparison repetitions only when explicitly requested.
- Compare matched settings with fresh caches. Replay uses the current
  [retrieval-cost scorer](../eviction.md#current-scoring).

Keep private traces, object identities, and raw results in ignored local experiment
folders under `experiments/cache/`. Commit only small settings and summaries.

## Selecting requests

By default, replay includes **all recorded request arrivals**. `--trace-seconds N`
selects requests from the earliest recorded start up to, but excluding, `N` seconds
later. Requests are sorted by start timestamp, then request ID.

The reader scans all packages twice. It retains the selected starts and joins their
outcomes, including outcomes recorded after the selected time window. An archive
must contain a single session because request IDs and clock origins are local to
each session. Archives with multiple session directories are rejected.

Every selected start is replayed, including requests that originally failed or
never completed. On a replay miss, the synthetic source successfully downloads the
whole object. Recorded outcome counts and starts without matching outcomes are
reported separately. A start without a matching outcome has an unknown outcome.
Missing events or packages can still leave the remaining input parseable.

### Object identities and lengths

Replay preserves recorded object identities and learns object lengths, including
zero, from successful outcomes anywhere in the capture. Objects with unknown or
conflicting lengths cannot be replayed.

## Cache settings

All configurations use `TieredMemoryDiskCache`. Set `--disk-bytes 0` for memory-only
replay or a positive capacity to enable disk. Both modes use the same request timing,
concurrency, synthetic downloads, and metrics collection.

| Option | Meaning | Default |
| --- | --- | --- |
| `--memory-bytes` | Memory capacity in bytes | 16 GiB |
| `--disk-bytes` | Disk capacity in bytes | `0` (disabled) |
| `--fixed-retrieval-equivalent-bytes` | Fallback `cost_per_request` for both tiers | `256000` |
| `--memory-cost-per-request` | Override the memory tier's request cost | Uses the fallback |
| `--disk-cost-per-request` | Override the disk tier's request cost | Uses the fallback |
| `--access-count-half-life` | Access-count half-life in caller-demand requests | Cache environment/default setting |

**Comparison runs must pass both capacity options.** The executable defaults differ
from the comparison settings above.

Per-tier request costs accept finite, nonnegative floating-point values. Both
`cost_per_requested_byte` coefficients are fixed at `1`. Both tiers share the
historian.

`--access-count-half-life` must be positive. It sets `CacheConfig` and overrides
`FEUER_ACCESS_COUNT_HALF_LIFE`. Eviction sample size and buffer retention use the
cache environment variables.

## Scheduling and source behavior

One sender follows the original arrival times and starts a lookup task for each
request. Earlier lookups may still be running. Tokio uses one runtime thread per
available CPU, and each thread can run multiple outstanding lookups.

Recorded starts reflect the original application's admission decisions. Replay
admits every arrival immediately, with backpressure provided by Feuer's I/O queues.
Descriptors and tasks that have not started hold no payload. Matching concurrent
misses share cache fetch work.

### Download buffers

Each synthetic source callback acquires a buffer through
`TieredMemoryDiskCache::allocate_buffer` and zero-fills its exposed payload.
`into_download` preserves the charge for the buffer's backing capacity, including
unused capacity in the pool's size bucket. This matches the adapter's buffer
allocation and charging.

Identities enter through the cache's `benchmark`-feature API, avoiding an additional
cache-side hash. This API passes the same allocation charge as
`get_or_fetch_object_with_allocation_charge`.

### Optional S3 delay

Downloads are immediate by default. Add `--simulate-s3` to limit the source to
**48 concurrent downloads** and apply a deterministic delay:

```text
service delay = 0.108599 seconds + requested_bytes / (80.205 × 1048576 bytes/second)
```

Each callback acquires a download permit, sleeps for this delay, then allocates and
zero-fills its buffer. It holds the permit through allocation and filling. Arrival
scheduling continues independently.

The calibration comes from [the S3 benchmark](../experiments/s3_benchmark.md), which
used another host and larger objects. It may not represent these trace objects or
this hardware. Runtime scheduling, buffer allocation, and permit waiting add latency
beyond the formula.

### Model limitations

Replay starts with an empty cache. It does not reproduce original cancellations,
source latency, retries, failures, or expanded downloads. The optional S3 delay is
a synthetic model rather than a reconstruction of recorded latency.

Feuer can share cache fetch work for matching misses. Source-level download
coalescing is outside the model. Traces do not record backing capacities or
shared-allocation identities.

## Results and metrics

Replay writes these files to the output directory:

| File | Contents |
| --- | --- |
| `metrics.prom` | All registered cache and process metrics |
| `before.prom` | Metrics snapshot before replay |
| `requests-finished.prom` | Metrics snapshot when requests finish |
| `settled.prom` | Metrics snapshot after the observation tail |
| `summary.csv` | Exact sender, queue, service, and completion latency distributions, plus effective settings |

Replay verifies that every selected request completes exactly once. After requests
finish, a five-second observation tail samples deferred writes. Writes can still
be pending when that tail ends.

### Lookup timing

- **Queue wait:** time from sender emission until the lookup task starts, including
  descriptor and runtime scheduling wait.
- **Service time:** time spent in the lookup, including waiting inside Feuer. It
  excludes the scheduling wait above.
- **`replay_queued_requests`:** arrivals whose lookup task has not started.
- **`replay_active_requests`:** started lookups awaiting completion.

The queued and active gauges are sampled separately. Recorded outcome counts in
`summary.csv` use the FETR v1 kind numbers, including `recorded_outcome_5` for misses.

### Source metrics

Both source modes export the following metrics. They count only callbacks invoked
by the cache and are separate from lookup queue and service metrics.

| Metric | Meaning |
| --- | --- |
| `replay_source_gets_total` | Source callbacks started |
| `replay_source_bytes_total` | Bytes allocated after service |
| `replay_source_queue_seconds` | Time spent acquiring a download permit |
| `replay_source_service_seconds` | Service time, including delay and allocation |
| `replay_source_waiting_requests` | Callbacks waiting for a permit |
| `replay_source_active_requests` | Active source callbacks |

The histograms use buckets from 0.001 to 1048.576 seconds, doubling at each step.

### Settings recorded in the summary

`summary.csv` records:

- `scheduling,task_per_arrival` and `workers` (the number of runtime threads).
- Effective `memory_cost_per_request`, `disk_cost_per_request`, both
  `*_cost_per_requested_byte` coefficients, and `access_count_half_life`.
- `simulate_s3`, `source_slots`, `source_latency_seconds`, and
  `source_bytes_per_second`.
- `source_buffer_pool,1`, identifying cache-pool allocation in both source modes.

Immediate mode records unlimited source concurrency and throughput, with zero
modeled latency.

## Validation

```sh
cargo test --locked -p feuer-replay
cargo clippy --locked -p feuer-replay --all-targets -- -D warnings
```

On Linux, tests cover:

- Round trips through the current writer and recorded-hash lookups.
- Sender timing, concurrent scheduling on a single runtime thread, and lookup-error
  propagation.
- Scorer and source CLI validation.
- Buffer-capacity charging and zero-filling of reused buffers.
- Simulated-source permit waiting and independent completion.
- Replaying the same input with disk disabled and enabled.

The replay binary's Linux-only tests do not run on macOS. Decoder tests cover
malformed packages, all event kinds, time prefixes and full captures, cross-package
outcome joins, incomplete requests, mixed sessions, and duplicate or inconsistent
events.
