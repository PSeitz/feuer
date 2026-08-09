# Feuer memory benchmark

This package compares the current `feuer-memory` cache with Foyer revision
`165cde3d4e638aaf2680384c02f57222b40be128` in a single-threaded,
memory-only replay. The checked-in gate run is documented in
[`results.md`](results.md).

## Input

The benchmark replays all 165,435 operations in
[`access_pattern.ndjson`](../access_pattern.ndjson). `--operations` can truncate
the trace for a quick run.

Every engine receives identical callback downloads selected independently of
cache state. The benchmark runs two downloader policies:

- `expanded`: splits smaller than 20 MiB are downloaded whole, requests larger
  than 4 MiB are downloaded exactly, and all other requests gain 4 MiB on each
  side before clipping to the split bounds. The whole-split rule takes
  precedence.
- `exact`: every callback downloads only its requested range.

## Compared engines

- `feuer-value-aware`: `MemoryCache` with prefetch probation
  measured in successful same-shard accesses, demonstrated-reuse/ghost
  promotion, and value-aware pressure-driven policy.
- `feuer-compaction-disabled`: the benchmark-only no-compaction control. It
  preserves Feuer's containment and eviction policy.
- `foyer-native-exact-key`: requested ranges are native Foyer keys. The
  complete callback payload is retained under that exact request key, but
  native Foyer does not perform containment lookup.

All use payload length as the capacity weight. Each variant gives every engine
16 shards; Foyer uses default `S3FifoConfig`. Every engine starts empty and
executes the trace exactly once.

## Running

The default gate matrix crosses seven capacities and both downloader policies
at 16 shards. The 128-GiB capacity is an upper-limit policy and accounting
case, not a claim that the process reaches 128 GiB of RSS:

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,128GiB \
  --shards 16 \
  --downloader expanded,exact \
  --feuer-compaction value-aware
```

The feature-gated `disabled` control remains available for explicit private
experiments but is not part of the checked-in comparison.

For a quick check:

```bash
cargo run --release -p feuer-memory-bench -- \
  --capacity 256MiB \
  --shards 16 \
  --downloader expanded \
  --feuer-compaction value-aware \
  --operations 1000
```

## Accounting and metrics

CSV rows report:

- request, requested-byte, and source-cost hit rates;
- source GETs and downloaded bytes;
- used payload weight at the end of the run (not merely the configured soft
  target), together with its percentage of that target; and
- elapsed time and operations per second.

The source-cost model is 80 ms per GET plus transfer at 80 MB/s, represented
without floating-point accumulation as:

```text
source cost = GETs * 6,400,000 + downloaded bytes
```

The cache-disabled denominator uses the same downloader policy as the measured
row. Payload values share one immutable benchmark source allocation, so
`used_payload_bytes` is cache accounting rather than a process-RSS
measurement. Foyer's internal record and allocator metadata is not exposed by
the pinned API and is excluded.

This is a policy replay, not an end-to-end latency benchmark. It does not
measure concurrent scaling, tail latency, callback coordination, disk I/O, or
recovery.
