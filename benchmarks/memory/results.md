# Memory-only comparison

Run on 2026-08-10. See [`README.md`](README.md) for workload and accounting
definitions.

## Configuration

- original gate command: `target/release/feuer-memory-bench --csv --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB --shards 1,4,16,64 --downloader expanded,exact`
- original warm-cache command: the original gate command plus `--warmup-iterations 1`
- local cost-aware exact-key command: `target/release/feuer-memory-bench --csv --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB --shards 16 --downloader exact`, also run with `--warmup-iterations 1`
- input: all 165,435 operations from [`access_pattern.ndjson`](../access_pattern.ndjson)
- expanded downloader defaults: the first unbatched request looks 5 ms ahead, coalesces same-object gaps below 10 MB, and assigns the identical combined range to every request in that batch; whole split below 8 MiB, otherwise exact request (`COALESCING_DISTANCE_BYTES=10MB`, `WHOLE_SPLIT_THRESHOLD_BYTES=8MiB`)
- exact downloader: callback range equals requested range
- Feuer: sample 64 cached ranges. Add fetch costs for recorded accesses, then divide by stored bytes. Evict lowest. Fetch cost = 10,000,000 + requested bytes. Keep 64 accesses/object; expire after 32,768 same-shard accesses. After 64 same-shard accesses, trim if at least 25% smaller; else evict.
- Foyer: measured with a local clone based on `165cde3d4e638aaf2680384c02f57222b40be128`, now pinned to the equivalent benchmark policy in [`PSeitz/foyer` at `14c2d88b9d7dd2135bfc723d0967debb59532b4b`](https://github.com/PSeitz/foyer/commit/14c2d88b9d7dd2135bfc723d0967debb59532b4b); the baseline uses default `S3FifoConfig`, and the added exact-key cost-aware policy uses a 64-candidate sample, an online residence-time access-rate estimate, and 10,000,000 fixed retrieval-equivalent bytes, but has no history-window or evidence-lifetime parameter, range lookup, shared object-range evidence, or compaction
- host: Apple M4 Max (16 logical CPUs, 64 GiB), arm64 macOS 26.4.1
- compiler: `rustc 1.96.0 (ac68faa20 2026-05-25)`, release profile

`Used Memory` is retained payload, not RSS. `Foyer (expanded key)` expands
before lookup and reuses the one identical native range key assigned to its
coalesced batch; it does not perform Feuer's containing-range lookup. Throughput
excludes the simulated wait.

Source-Cost Hit uses a 125-ms fixed cost per GET and 80 MB/s transfer speed:

```text
source time = GETs * 125 ms + bytes / 80 MB/s
Source-Cost Hit = 1 - cached source time / exact-request no-cache source time
```

## Local Foyer cost-aware policy — exact keys, 16 shards

This run isolates exact callback ranges. Both Foyer policies use
`(ObjectKey, requested ByteRange)` as a native exact key. The cost-aware policy
only tracks that exact record; it has no range containment or compaction. These
tables were rerun after replacing the provisional fixed evidence lifetime with
the adaptive access-rate estimator described above.

### Cold cache

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer | 41.83% | 34.72% | 206.5 MiB | 985.83 K/s |
| 256 MiB | Foyer S3FIFO | 42.05% | 34.91% | 222.2 MiB | 7.25 M/s |
| 256 MiB | Foyer cost-aware | 42.31% | 35.12% | 213.3 MiB | 4.27 M/s |
| 512 MiB | Feuer | 42.34% | 35.15% | 460.0 MiB | 941.23 K/s |
| 512 MiB | Foyer S3FIFO | 42.39% | 35.18% | 426.5 MiB | 7.29 M/s |
| 512 MiB | Foyer cost-aware | 42.68% | 35.42% | 438.9 MiB | 4.13 M/s |
| 1 GiB | Feuer | 44.01% | 36.53% | 941.4 MiB | 408.28 K/s |
| 1 GiB | Foyer S3FIFO | 44.16% | 36.65% | 955.1 MiB | 6.92 M/s |
| 1 GiB | Foyer cost-aware | 44.28% | 36.75% | 949.4 MiB | 2.81 M/s |
| 2 GiB | Feuer | 44.70% | 37.10% | 1993.9 MiB | 432.17 K/s |
| 2 GiB | Foyer S3FIFO | 44.38% | 36.84% | 1967.6 MiB | 6.73 M/s |
| 2 GiB | Foyer cost-aware | 44.78% | 37.17% | 1970.1 MiB | 3.02 M/s |
| 4 GiB | Feuer | 45.01% | 37.36% | 4015.2 MiB | 477.17 K/s |
| 4 GiB | Foyer S3FIFO | 44.42% | 36.87% | 4022.5 MiB | 6.98 M/s |
| 4 GiB | Foyer cost-aware | 45.02% | 37.37% | 4030.8 MiB | 3.15 M/s |
| 8 GiB | Feuer | 45.16% | 37.49% | 8122.5 MiB | 499.00 K/s |
| 8 GiB | Foyer S3FIFO | 44.44% | 36.90% | 8114.2 MiB | 6.57 M/s |
| 8 GiB | Foyer cost-aware | 45.18% | 37.51% | 8150.5 MiB | 3.42 M/s |
| 16 GiB | Feuer | 45.24% | 37.57% | 16328.2 MiB | 637.47 K/s |
| 16 GiB | Foyer S3FIFO | 44.48% | 36.97% | 16307.7 MiB | 6.19 M/s |
| 16 GiB | Foyer cost-aware | 45.25% | 37.59% | 16310.6 MiB | 3.26 M/s |
| 32 GiB | Feuer | 45.33% | 37.65% | 32715.4 MiB | 889.57 K/s |
| 32 GiB | Foyer S3FIFO | 44.58% | 37.13% | 32675.9 MiB | 7.15 M/s |
| 32 GiB | Foyer cost-aware | 45.32% | 37.68% | 32692.6 MiB | 4.35 M/s |

### One-iteration warm cache

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer | 41.82% | 34.71% | 206.5 MiB | 924.66 K/s |
| 256 MiB | Foyer S3FIFO | 42.07% | 34.92% | 222.2 MiB | 7.20 M/s |
| 256 MiB | Foyer cost-aware | 42.32% | 35.12% | 213.3 MiB | 4.37 M/s |
| 512 MiB | Feuer | 42.35% | 35.15% | 460.0 MiB | 777.47 K/s |
| 512 MiB | Foyer S3FIFO | 42.40% | 35.19% | 426.5 MiB | 7.25 M/s |
| 512 MiB | Foyer cost-aware | 42.69% | 35.43% | 438.9 MiB | 3.81 M/s |
| 1 GiB | Feuer | 47.78% | 39.66% | 963.6 MiB | 321.90 K/s |
| 1 GiB | Foyer S3FIFO | 51.18% | 42.48% | 954.8 MiB | 6.95 M/s |
| 1 GiB | Foyer cost-aware | 46.51% | 38.60% | 969.0 MiB | 2.69 M/s |
| 2 GiB | Feuer | 52.91% | 43.92% | 1971.0 MiB | 297.11 K/s |
| 2 GiB | Foyer S3FIFO | 52.66% | 43.72% | 1967.6 MiB | 7.04 M/s |
| 2 GiB | Foyer cost-aware | 49.06% | 40.73% | 1992.9 MiB | 2.69 M/s |
| 4 GiB | Feuer | 56.91% | 47.24% | 4031.8 MiB | 322.01 K/s |
| 4 GiB | Foyer S3FIFO | 52.68% | 43.75% | 4022.5 MiB | 6.62 M/s |
| 4 GiB | Foyer cost-aware | 52.36% | 43.46% | 4045.4 MiB | 2.82 M/s |
| 8 GiB | Feuer | 61.71% | 51.24% | 8113.0 MiB | 381.17 K/s |
| 8 GiB | Foyer S3FIFO | 52.75% | 43.88% | 8117.5 MiB | 6.79 M/s |
| 8 GiB | Foyer cost-aware | 58.01% | 48.17% | 8144.6 MiB | 3.33 M/s |
| 16 GiB | Feuer | 69.39% | 57.63% | 16309.8 MiB | 479.31 K/s |
| 16 GiB | Foyer S3FIFO | 52.94% | 44.19% | 16315.9 MiB | 6.92 M/s |
| 16 GiB | Foyer cost-aware | 66.42% | 55.16% | 16317.2 MiB | 3.70 M/s |
| 32 GiB | Feuer | 75.76% | 63.07% | 32669.9 MiB | 629.66 K/s |
| 32 GiB | Foyer S3FIFO | 53.57% | 45.07% | 32700.1 MiB | 6.62 M/s |
| 32 GiB | Foyer cost-aware | 73.80% | 61.35% | 32665.6 MiB | 3.99 M/s |

On a cold pass the local exact-key policy beats S3FIFO at every measured
capacity and remains close to Feuer. After one warm-up pass it leads both at
256 MiB and 512 MiB, trails both from 1 GiB through 4 GiB, and then beats
S3FIFO while trailing Feuer from 8 GiB upward. Its continuously decaying rate
estimate deliberately avoids a workload-scale lifetime constant. The bounded
candidate scan is slower than S3FIFO but remains substantially faster than
Feuer's range-aware cache in this single-threaded replay.

## One-iteration warm cache — 16 shards

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer (expanded) | 53.67% | 36.32% | 200.5 MiB | 1.31 M/s |
| 256 MiB | Foyer (expanded key) | 55.30% | 41.38% | 186.0 MiB | 8.78 M/s |
| 256 MiB | Feuer (exact) | 41.82% | 34.71% | 206.5 MiB | 0.90 M/s |
| 256 MiB | Foyer (exact) | 42.07% | 34.92% | 222.2 MiB | 7.23 M/s |
| 512 MiB | Feuer (expanded) | 54.48% | 37.28% | 434.9 MiB | 1.00 M/s |
| 512 MiB | Foyer (expanded key) | 55.91% | 42.14% | 462.2 MiB | 8.86 M/s |
| 512 MiB | Feuer (exact) | 42.35% | 35.15% | 460.0 MiB | 0.77 M/s |
| 512 MiB | Foyer (exact) | 42.40% | 35.19% | 426.5 MiB | 6.74 M/s |
| 1 GiB | Feuer (expanded) | 59.74% | 46.31% | 968.5 MiB | 0.42 M/s |
| 1 GiB | Foyer (expanded key) | 57.13% | 44.28% | 986.4 MiB | 8.61 M/s |
| 1 GiB | Feuer (exact) | 47.78% | 39.66% | 963.6 MiB | 0.31 M/s |
| 1 GiB | Foyer (exact) | 51.18% | 42.48% | 954.8 MiB | 6.69 M/s |
| 2 GiB | Feuer (expanded) | 62.71% | 49.61% | 1994.4 MiB | 0.34 M/s |
| 2 GiB | Foyer (expanded key) | 57.88% | 45.36% | 1983.3 MiB | 7.82 M/s |
| 2 GiB | Feuer (exact) | 52.91% | 43.92% | 1971.0 MiB | 0.31 M/s |
| 2 GiB | Foyer (exact) | 52.66% | 43.72% | 1967.6 MiB | 6.78 M/s |
| 4 GiB | Feuer (expanded) | 64.72% | 51.68% | 4025.7 MiB | 0.32 M/s |
| 4 GiB | Foyer (expanded key) | 58.27% | 45.92% | 4020.1 MiB | 8.29 M/s |
| 4 GiB | Feuer (exact) | 56.91% | 47.24% | 4031.8 MiB | 0.34 M/s |
| 4 GiB | Foyer (exact) | 52.68% | 43.75% | 4022.5 MiB | 6.96 M/s |
| 8 GiB | Feuer (expanded) | 67.90% | 54.54% | 8117.8 MiB | 0.35 M/s |
| 8 GiB | Foyer (expanded key) | 58.53% | 46.41% | 8121.8 MiB | 8.20 M/s |
| 8 GiB | Feuer (exact) | 61.71% | 51.24% | 8113.0 MiB | 0.39 M/s |
| 8 GiB | Foyer (exact) | 52.75% | 43.88% | 8117.5 MiB | 6.54 M/s |
| 16 GiB | Feuer (expanded) | 73.25% | 59.56% | 16327.6 MiB | 0.29 M/s |
| 16 GiB | Foyer (expanded key) | 58.70% | 46.68% | 16276.8 MiB | 7.30 M/s |
| 16 GiB | Feuer (exact) | 69.39% | 57.63% | 16309.8 MiB | 0.47 M/s |
| 16 GiB | Foyer (exact) | 52.94% | 44.19% | 16315.9 MiB | 6.72 M/s |
| 32 GiB | Feuer (expanded) | 78.34% | 64.68% | 32702.0 MiB | 0.42 M/s |
| 32 GiB | Foyer (expanded key) | 58.88% | 46.92% | 32677.0 MiB | 6.86 M/s |
| 32 GiB | Feuer (exact) | 75.76% | 63.07% | 32669.9 MiB | 0.61 M/s |
| 32 GiB | Foyer (exact) | 53.57% | 45.07% | 32700.1 MiB | 6.54 M/s |

## Cold cache — 16 shards

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer (expanded) | 53.57% | 36.19% | 200.5 MiB | 1.34 M/s |
| 256 MiB | Foyer (expanded key) | 55.31% | 41.38% | 186.0 MiB | 8.30 M/s |
| 256 MiB | Feuer (exact) | 41.83% | 34.72% | 206.5 MiB | 0.97 M/s |
| 256 MiB | Foyer (exact) | 42.05% | 34.91% | 222.2 MiB | 7.28 M/s |
| 512 MiB | Feuer (expanded) | 54.36% | 37.13% | 434.9 MiB | 1.08 M/s |
| 512 MiB | Foyer (expanded key) | 55.92% | 42.16% | 462.2 MiB | 8.59 M/s |
| 512 MiB | Feuer (exact) | 42.34% | 35.15% | 460.0 MiB | 0.88 M/s |
| 512 MiB | Foyer (exact) | 42.39% | 35.18% | 426.5 MiB | 7.12 M/s |
| 1 GiB | Feuer (expanded) | 57.54% | 44.54% | 972.0 MiB | 0.46 M/s |
| 1 GiB | Foyer (expanded key) | 57.15% | 44.30% | 986.4 MiB | 8.06 M/s |
| 1 GiB | Feuer (exact) | 44.01% | 36.53% | 941.4 MiB | 0.41 M/s |
| 1 GiB | Foyer (exact) | 44.16% | 36.65% | 955.1 MiB | 6.89 M/s |
| 2 GiB | Feuer (expanded) | 58.31% | 45.63% | 1967.2 MiB | 0.42 M/s |
| 2 GiB | Foyer (expanded key) | 57.90% | 45.37% | 1983.3 MiB | 8.21 M/s |
| 2 GiB | Feuer (exact) | 44.70% | 37.10% | 1993.9 MiB | 0.40 M/s |
| 2 GiB | Foyer (exact) | 44.38% | 36.84% | 1967.6 MiB | 6.53 M/s |
| 4 GiB | Feuer (expanded) | 58.97% | 46.52% | 4056.5 MiB | 0.41 M/s |
| 4 GiB | Foyer (expanded key) | 58.29% | 45.95% | 4020.1 MiB | 8.43 M/s |
| 4 GiB | Feuer (exact) | 45.01% | 37.36% | 4015.2 MiB | 0.46 M/s |
| 4 GiB | Foyer (exact) | 44.42% | 36.87% | 4022.5 MiB | 6.87 M/s |
| 8 GiB | Feuer (expanded) | 59.36% | 47.05% | 8116.3 MiB | 0.44 M/s |
| 8 GiB | Foyer (expanded key) | 58.55% | 46.44% | 8123.3 MiB | 8.13 M/s |
| 8 GiB | Feuer (exact) | 45.16% | 37.49% | 8122.5 MiB | 0.54 M/s |
| 8 GiB | Foyer (exact) | 44.44% | 36.90% | 8114.2 MiB | 6.65 M/s |
| 16 GiB | Feuer (expanded) | 59.57% | 47.35% | 16307.4 MiB | 0.53 M/s |
| 16 GiB | Foyer (expanded key) | 58.71% | 46.70% | 16302.0 MiB | 7.76 M/s |
| 16 GiB | Feuer (exact) | 45.24% | 37.57% | 16328.2 MiB | 0.66 M/s |
| 16 GiB | Foyer (exact) | 44.48% | 36.97% | 16307.7 MiB | 6.65 M/s |
| 32 GiB | Feuer (expanded) | 59.72% | 47.55% | 32695.9 MiB | 0.60 M/s |
| 32 GiB | Foyer (expanded key) | 58.94% | 47.01% | 32711.9 MiB | 7.66 M/s |
| 32 GiB | Feuer (exact) | 45.33% | 37.65% | 32715.4 MiB | 0.90 M/s |
| 32 GiB | Foyer (exact) | 44.58% | 37.13% | 32675.9 MiB | 7.24 M/s |

## Shard-count sensitivity

The same matrix was run at 1, 4, 16, and 64 shards. The table compares the
simplified cost-weighted policy with the preceding exponentially aged policy;
each row summarizes both downloader policies and all eight capacities. Positive
values favor the simplified policy.

| Cache state | Shards | Worst Request-Hit Δ | Mean Request-Hit Δ | Best Request-Hit Δ | Worst Source-Cost-Hit Δ | Mean Source-Cost-Hit Δ |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Cold | 1 | -0.55 pp | -0.08 pp | +0.29 pp | -0.46 pp | -0.07 pp |
| Cold | 4 | -0.24 pp | -0.07 pp | +0.02 pp | -0.20 pp | -0.08 pp |
| Cold | 16 | -0.04 pp | -0.01 pp | +0.02 pp | -0.06 pp | -0.01 pp |
| Cold | 64 | 0.00 pp | 0.00 pp | 0.00 pp | 0.00 pp | 0.00 pp |
| Warm | 1 | -0.52 pp | -0.06 pp | +0.11 pp | -0.43 pp | -0.04 pp |
| Warm | 4 | -0.32 pp | +0.52 pp | +7.09 pp | -0.26 pp | +0.44 pp |
| Warm | 16 | -0.01 pp | +0.98 pp | +3.79 pp | -0.01 pp | +0.82 pp |
| Warm | 64 | 0.00 pp | +0.07 pp | +0.48 pp | 0.00 pp | +0.06 pp |

Feuer's existing 32,768-access lifetime is deliberately documented rather than
presented as a final aging model. It remains same-shard-traffic-dependent. The
local exact-key Foyer policy above does not inherit this cutoff; replacing it in
Feuer's range-aware evidence model remains separate work.
