# Feuer memory benchmark

This package compares the current `feuer-memory` cache with the
[`PSeitz/foyer`](https://github.com/PSeitz/foyer) fork, pinned to revision
`14c2d88b9d7dd2135bfc723d0967debb59532b4b`, in a single-threaded,
memory-only replay. No local Foyer checkout is required.
[`results.md`](results.md) preserves historical results from the previous
range-aware benchmark, not measurements of this full-object replay.

## Input

The benchmark replays all 165,435 operations in
[`access_pattern.ndjson`](../access_pattern.ndjson). `--operations` can truncate
the trace for a quick run.

The legacy JSON trace records requests for portions of original files. At load
time, each `(object_id, requested_range_start, requested_range_end)` becomes an
independent full-object key. The original key followed by both offsets encoded
as fixed-width, 16-digit hexadecimal values is hashed with `ObjectKeyHash`.
The new object's payload length is `end - start`.

All engines receive these same objects. Identical triples reuse the same key,
but different triples remain independent even when the original requests overlap.
The original file size is used only to validate the input. The replay never
fetches or allocates whole original files. Cache operations and access histories
use only object keys, with no range lookup, expansion, coalescing, or trimming.

## Compared engines

- `feuer-value-density`: `MemoryCache` with per-object access counts decaying
  with a 262,144-request half-life across all keys by default. Sampled eviction
  compares decayed retrieval value per payload byte. The replay driver records
  each request before lookup in standalone history, so counters survive cache
  evictions for the duration of the run.
- `foyer-native`: Foyer's native key-value cache with default `S3FifoConfig`.
- `foyer-cost-aware`: the same native key-value cache with the fork's
  `CostAwareConfig`. It estimates an object's access rate as its successful
  access count divided by its shard-clock residence time, then weights that
  rate by `10,000,000 + entry weight`. Under pressure, it evicts the lowest
  estimated retrieval cost saved per payload byte from a rotating sample of
  64 entries.

All engines use payload length as the capacity weight and receive the same
requested shard count. Every engine starts empty. By default it executes one
measured trace pass. `--warmup-iterations N` first executes `N` untimed passes
against that same cache, preserving cache and policy state for the measured pass.

## Running

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB \
  --shards 1,4,16,64
```

The default output is a human-readable table. Add `--csv` for machine-readable
output. The default shard count is 16. For a quick run:

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB --shards 16 --operations 1000
```

`FEUER_ACCESS_COUNT_HALF_LIFE` changes Feuer's scoring half-life, measured in
requests across all keys. `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` changes its
fixed per-request scoring credit (default `10000000` in this replay, while the
library defaults to zero). Both are read once per process and accept size
suffixes. The fixed credit accepts zero for bytes-only scoring:

```bash
FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES=0 \
  cargo run --release -p feuer-memory-bench -- \
  --capacity 1GiB --shards 1 --warmup-iterations 1
```

These settings do not change Foyer's scoring configuration or the reported
source-cost model, which retains the fixed 10,000,000-byte cost.

## History-only contention benchmark

```bash
cargo test --release -p feuer-memory-bench history_recording_contention -- --ignored --nocapture
```

This isolates `ObjectAccessHistories::record_access` using the same converted
object keys, with 1 and 32 threads sharing one history object. Each trial
distributes one complete trace across the workers, taking every Nth request per
worker. Concurrent execution does not preserve global request order.

It compares empty history with history prepopulated by one untimed trace pass,
so the latter updates only existing counters. Parsing, prepopulation, thread
startup, and thread joining are outside the timed section. Start/finish barriers
are included. There are no cache operations or payload allocations. Output
reports median throughput and the timing range over five trials. This benchmark
is ignored by ordinary `cargo test` runs.

## Accounting and metrics

The human-readable table reports request and source-cost hit rates, source GETs
and bytes, cached payload bytes, and throughput. `--csv` additionally emits
requested-byte hit rate, raw byte counts, target utilization, and elapsed time.
Warm-up traffic is excluded from all reported traffic and elapsed time.

The source-cost model is 125 ms per GET plus transfer at 80 MB/s, represented
without floating-point accumulation as:

```text
source cost = GETs * 10,000,000 + downloaded bytes
```

The cache-disabled denominator charges one GET and the full converted object's
payload for every request. Cache misses incur that same cost, while hits avoid
it entirely. Payload values share one immutable source allocation sized to the
largest converted object, so `used_payload_bytes` is cache accounting rather
than a process-RSS measurement. Foyer's internal record and allocator metadata
is not exposed by the benchmarked API and is excluded.

This is a policy replay, not an end-to-end latency benchmark. It does not measure
real downloads, scheduling, concurrent cache scaling, tail latency, disk I/O,
or recovery.
