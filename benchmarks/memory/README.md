# Feuer memory benchmark

This package compares the current `feuer-memory` cache with Foyer revision
`165cde3d4e638aaf2680384c02f57222b40be128` in a single-threaded,
memory-only replay. The checked-in gate run is documented in
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

## Compared engines

- `feuer-value-aware`: `MemoryCache` with source-cost-weighted, ageable access
  evidence and sampled value-aware eviction. After a 64-successful-access grace,
  pressure trims the selected victim to its observed request ranges when useful.
- `foyer-native-exact-key`: requested ranges are native Foyer keys. The
  complete callback payload is retained under that exact request key, but
  native Foyer does not perform containment lookup.
- `foyer-native-expanded-key`: included with the `expanded` downloader. The
  application expands before lookup and uses that exact expanded range as the
  native Foyer key. Distinct requests therefore hit when they expand to exactly
  the same bytes; unlike Feuer, a merely containing cached range is not enough.

All use payload length as the capacity weight. Each variant gives every engine
16 shards; Foyer uses default `S3FifoConfig`. Every engine starts empty. By
default it executes one measured trace pass; `--warmup-iterations N` first
executes `N` untimed passes against that same cache, preserving the resulting
cache and policy state for the measured pass.

## Running

The default gate matrix crosses eight capacities and both downloader policies
at 16 shards. The 32-GiB capacity is the upper-limit payload-accounting case:

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB \
  --shards 16 \
  --downloader expanded,exact
```

The default output is a human-readable table. Add `--csv` for machine-readable
output.

`COALESCING_DISTANCE_BYTES` and `WHOLE_SPLIT_THRESHOLD_BYTES` override the
default 10-MB coalescing distance and 8-MiB whole-split threshold. Both accept
raw bytes or units accepted by `--capacity`; for example:

```bash
COALESCING_DISTANCE_BYTES=5MB WHOLE_SPLIT_THRESHOLD_BYTES=8MiB \
  cargo run --release -p feuer-memory-bench -- --capacity 1GiB
```

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
and bytes, retained payload, and throughput. `--csv` additionally emits
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
the pinned API and is excluded.

This is a policy replay, not an end-to-end latency benchmark. It simulates
coalescing deterministically but does not measure real scheduling, concurrent
scaling, tail latency, disk I/O, or recovery.
