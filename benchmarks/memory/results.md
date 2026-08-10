# Memory-only comparison

Run on 2026-08-10. See [`README.md`](README.md) for workload and accounting
definitions.

## Configuration

- cold-cache command: `target/release/feuer-memory-bench --csv --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB --shards 1,4,16,64 --downloader expanded,exact`
- warm-cache command: the cold-cache command plus `--warmup-iterations 1`
- input: all 165,435 operations from [`access_pattern.ndjson`](../access_pattern.ndjson)
- expanded downloader defaults: the first unbatched request looks 5 ms ahead, coalesces same-object gaps below 10 MB, and assigns the identical combined range to every request in that batch; whole split below 8 MiB, otherwise exact request (`COALESCING_DISTANCE_BYTES=10MB`, `WHOLE_SPLIT_THRESHOLD_BYTES=8MiB`)
- exact downloader: callback range equals requested range
- Feuer: sample 64 cached ranges. Add fetch costs for recorded accesses, then divide by stored bytes. Evict lowest. Fetch cost = 10,000,000 + requested bytes. Keep 64 accesses/object; expire after 32,768 same-shard accesses. After 64 same-shard accesses, trim if at least 25% smaller; else evict.
- Foyer: `165cde3d4e638aaf2680384c02f57222b40be128`, default `S3FifoConfig`; exact requests use native exact keys, while expanded requests additionally report native expanded-range keys
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

The 32,768-access lifetime is deliberately documented rather than presented as
a final aging model. It remains same-shard-traffic-dependent. Replacing it with
wall-clock aging requires a lifecycle-based duration and is separate from the
restored source-cost weighting measured here.
