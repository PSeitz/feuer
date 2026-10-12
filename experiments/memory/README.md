# Feuer memory benchmark

This package compares the current `feuer-memory` cache with the
[`PSeitz/foyer`](https://github.com/PSeitz/foyer) fork, pinned to revision
`14c2d88b9d7dd2135bfc723d0967debb59532b4b`, in a single-threaded,
memory-only replay. The fork includes the exact-key cost-aware policy used by
this benchmark; no local Foyer checkout is required. The checked-in gate run is documented in
[`results.md`](results.md).

## Input

The benchmark replays all 165,435 operations in
[`access_pattern.ndjson`](../access_pattern.ndjson). `--operations` can truncate
the trace for a quick run.

Every engine uses the same downloader rules. The benchmark runs two policies:

- `expanded`: the first unbatched request looks 5 ms ahead in trace time and
  combines same-object ranges separated by less than
  `COALESCING_DISTANCE_BYTES`. Every request assigned to that batch expands to
  the exact same combined range, and the initiating callback downloads it
  before following requests replay. Splits below
  `WHOLE_SPLIT_THRESHOLD_BYTES` are selected whole. The defaults are 10 MB and
  8 MiB respectively.
- `exact`: every callback downloads only its requested range.

## History-only contention benchmark

```bash
cargo test --release -p feuer-memory-bench history_recording_contention -- --ignored --nocapture
```

This isolates `ObjectAccessHistories::record_access` using the same dump, with 1 and 32 threads
sharing one history object. Each trial distributes one complete trace across the workers, taking
every Nth request per worker; concurrent execution does not preserve the global request order.
It compares empty history with history prepopulated by one untimed trace pass, so the latter
updates only existing counters. Parsing, prepopulation, thread startup, and thread joining are
outside the timed section; start/finish barriers are included. There are no cache operations or
payload allocations. Output reports median throughput and the timing range over five trials.

Measured on a 16-core Apple M4 Max (32 threads oversubscribe this host), using 165,435 requests,
13,036 object keys, and 90,108 distinct key/range pairs:

| History locking | Threads | Initial history | Median time | Aggregate requests/s |
| --- | --- | --- | --- | --- |
| Previous single mutex | 1 | Empty | 13.500 ms | 12,254,294 |
| Previous single mutex | 1 | Prepopulated | 11.284 ms | 14,660,915 |
| Previous single mutex | 32 | Empty | 69.109 ms | 2,393,811 |
| Previous single mutex | 32 | Prepopulated | 59.754 ms | 2,768,617 |
| 64 history shards, DefaultHasher selector | 1 | Empty | 17.034 ms | 9,712,284 |
| 64 history shards, DefaultHasher selector | 1 | Prepopulated | 15.047 ms | 10,994,703 |
| 64 history shards, DefaultHasher selector | 32 | Empty | 31.223 ms | 5,298,477 |
| 64 history shards, DefaultHasher selector | 32 | Prepopulated | 31.019 ms | 5,333,380 |
| 64 history shards, FNV selector | 1 | Empty | 16.572 ms | 9,982,602 |
| 64 history shards, FNV selector | 1 | Prepopulated | 12.691 ms | 13,036,001 |
| 64 history shards, FNV selector | 32 | Empty | 31.901 ms | 5,185,874 |
| 64 history shards, FNV selector | 32 | Prepopulated | 31.791 ms | 5,203,859 |

With prepopulated history, switching only the shard selector to FNV improves single-thread throughput
by 19% versus DefaultHasher; 32-thread throughput is similar in these runs. Compared with the original
single mutex, the FNV-sharded version has 1.88x the 32-thread throughput and 11% less single-thread
throughput. History shards are selected by object key, independent of cache shards; the maps keep their
standard hashers and the global request clock remains shared through an atomic. This measures recording
throughput, not end-to-end cache latency.

## Compared engines

- `feuer-value-density`: `MemoryCache` with independent exact-range access counts
  decaying with a 262,144-request half-life across all keys by default. Sampled eviction
  compares decayed retrieval value per payload byte, crediting requests fully
  covered by each cached range. Trimming uses a separate history of at most 64 events per object by default.
  After a 64-request grace, pressure trims the selected victim to its observed
  request ranges when useful. The replay driver records requests before lookup in standalone history;
  distinct counters survive all cache evictions for the duration of the run.
- `foyer-native-exact-key`: requested ranges are native Foyer keys. The
  complete callback payload is stored under that exact request key, but
  native Foyer does not perform containment lookup.
- `foyer-native-expanded-key`: included with the `expanded` downloader. The
  application expands before lookup and uses that exact expanded range as the
  native Foyer key. Distinct requests therefore hit when they expand to exactly
  the same bytes; unlike Feuer, a merely containing cached range is not enough.
- `foyer-cost-aware-exact-key` and `foyer-cost-aware-expanded-key`: use the same
  native exact keys as the two Foyer baselines, but replace S3FIFO with the
  fork's `CostAwareConfig`. The policy estimates an exact key's access rate as
  its successful access count divided by its shard-clock residence time, then
  weights that rate by `10,000,000 + entry weight`. At pressure, it evicts the
  lowest estimated retrieval cost saved per payload byte from a rotating
  sample of 64 entries. The estimator uses only a count and admission clock per
  entry; idle value decays continuously instead of crossing a fixed lifetime
  threshold. It has no containment lookup, range evidence, or compaction.

All use payload length as the capacity weight. Each run gives every engine the
same requested shard count; the native Foyer baselines use default
`S3FifoConfig`. Every engine starts empty. By
default it executes one measured trace pass; `--warmup-iterations N` first
executes `N` untimed passes against that same cache, preserving the resulting
cache and policy state for the measured pass.

## Running

The checked gate matrix crosses eight capacities, both downloader policies, and
1, 4, 16, and 64 shards. The 32-GiB capacity is the upper-limit
payload-accounting case:

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB \
  --shards 1,4,16,64 \
  --downloader expanded,exact
```

The default output is a human-readable table. Add `--csv` for machine-readable
output. To isolate the no-range, exact-key comparison:

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB \
  --shards 16 \
  --downloader exact
```

`COALESCING_DISTANCE_BYTES` and `WHOLE_SPLIT_THRESHOLD_BYTES` override the
default 10-MB coalescing distance and 8-MiB whole-split threshold. Both accept
raw bytes or units accepted by `--capacity`; for example:

```bash
COALESCING_DISTANCE_BYTES=5MB WHOLE_SPLIT_THRESHOLD_BYTES=8MiB \
  cargo run --release -p feuer-memory-bench -- --capacity 1GiB
```

To change Feuer's range-trimming evidence lifetime without rebuilding:

```bash
FEUER_MAX_ACCESS_AGE_ACCESSES=65536 cargo run --release -p feuer-memory-bench -- \
  --capacity 1GiB --shards 1 --downloader exact --warmup-iterations 1
```

The limit counts requests across all keys, not milliseconds. It does not
change the per-object history cap. To change that cap independently (default 64):

```bash
FEUER_MAX_ACCESS_EVENTS_PER_KEY=256 cargo run --release -p feuer-memory-bench -- \
  --capacity 1GiB --shards 1 --downloader exact --warmup-iterations 1
```

Both history settings are read once per process and do not change either Foyer
policy or Feuer's decayed scoring counters. Larger event histories increase
metadata memory and range-trimming work.

`FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` changes Feuer's fixed per-request scoring
credit (default `10000000`) in this replay only; the library defaults to zero.
It is read once per process and accepts nonnegative byte counts with size suffixes,
including `0` for bytes-only scoring. For example:

```bash
FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES=0 FEUER_MAX_ACCESS_EVENTS_PER_KEY=256 \
  cargo run --release -p feuer-memory-bench -- \
  --capacity 1GiB --shards 1 --downloader exact --warmup-iterations 1
```

This setting does not change Foyer's scoring configuration or the benchmark's
reported source-cost model, which both retain the fixed 10,000,000-byte cost.

To measure a cache warmed by one complete trace iteration, add
`--warmup-iterations 1`. Warm-up traffic is excluded from the reported rates
and elapsed time. Coalescing uses trace lookahead rather than sleeping, so its
5-ms wait is not included in throughput.

For a quick check:

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB \
  --shards 16 \
  --downloader expanded \
  --operations 1000
```

## Accounting and metrics

The human-readable table reports request and source-cost hit rates, source GETs
and bytes, cached payload bytes, and throughput. `--csv` additionally emits
requested-byte hit rate, raw byte counts, target utilization, and elapsed time.

The source-cost model is 125 ms per GET plus transfer at 80 MB/s, represented
without floating-point accumulation as:

```text
source cost = GETs * 10,000,000 + downloaded bytes
```

The cache-disabled denominator is independent of downloader policy: every
request incurs one GET and transfers exactly its requested bytes. Actual cost
uses the selected downloader's callback bytes, so overfetch can produce
negative source-cost savings when its extra transfer costs exceed the cache's
savings. Payload values share one immutable benchmark source allocation, so
`used_payload_bytes` is cache accounting rather than a process-RSS
measurement. Foyer's internal record and allocator metadata is not exposed by
the benchmarked API and is excluded.

This is a policy replay, not an end-to-end latency benchmark. It simulates
coalescing deterministically but does not measure real scheduling, concurrent
scaling, tail latency, disk I/O, or recovery.
