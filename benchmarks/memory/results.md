# Memory-only comparison

Run on 2026-08-10. See [`README.md`](README.md) for workload and accounting
definitions.

## Configuration

- cold-cache command: `target/release/feuer-memory-bench --csv --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB --shards 16 --downloader expanded,exact --feuer-compaction value-aware`
- warm-cache command: the cold-cache command plus `--warmup-iterations 1`
- input: all 165,435 operations from [`access_pattern.ndjson`](../access_pattern.ndjson)
- expanded downloader defaults: the first unbatched request looks 5 ms ahead, coalesces same-object gaps below 10 MB, and assigns the identical combined range to every request in that batch; whole split below 8 MiB, otherwise exact request (`COALESCING_DISTANCE_BYTES=10MB`, `WHOLE_SPLIT_THRESHOLD_BYTES=8MiB`)
- exact downloader: callback range equals requested range
- Feuer policy: pressure selection samples up to 64 extents and ranks them by estimated retrieval cost per retained byte. Exact access evidence is bounded to 64 events per object key and remains active for up to eight shard-local epochs of 4,096 successful accesses. Broader admissions receive 64 successful same-shard accesses of compaction probation, which does not prevent pressure eviction. Each extent keeps one replaceable, ageable demonstrated-prefetch-reuse signal.
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
| 256 MiB | Feuer (expanded) | 53.80% | 36.47% | 206.5 MiB | 1.20 M/s |
| 256 MiB | Foyer (expanded key) | 55.30% | 41.38% | 186.0 MiB | 8.58 M/s |
| 256 MiB | Feuer (exact) | 41.83% | 34.72% | 209.5 MiB | 0.89 M/s |
| 256 MiB | Foyer (exact) | 42.07% | 34.92% | 222.2 MiB | 7.13 M/s |
| 512 MiB | Feuer (expanded) | 54.65% | 37.47% | 446.1 MiB | 0.78 M/s |
| 512 MiB | Foyer (expanded key) | 55.91% | 42.14% | 462.2 MiB | 8.71 M/s |
| 512 MiB | Feuer (exact) | 42.36% | 35.15% | 444.2 MiB | 0.76 M/s |
| 512 MiB | Foyer (exact) | 42.40% | 35.19% | 426.5 MiB | 6.83 M/s |
| 1 GiB | Feuer (expanded) | 59.70% | 46.31% | 966.6 MiB | 0.31 M/s |
| 1 GiB | Foyer (expanded key) | 57.13% | 44.28% | 986.4 MiB | 8.21 M/s |
| 1 GiB | Feuer (exact) | 47.04% | 39.05% | 953.7 MiB | 0.28 M/s |
| 1 GiB | Foyer (exact) | 51.18% | 42.48% | 954.8 MiB | 6.73 M/s |
| 2 GiB | Feuer (expanded) | 61.72% | 48.74% | 1978.1 MiB | 0.26 M/s |
| 2 GiB | Foyer (expanded key) | 57.88% | 45.36% | 1983.3 MiB | 7.83 M/s |
| 2 GiB | Feuer (exact) | 50.17% | 41.64% | 1971.3 MiB | 0.24 M/s |
| 2 GiB | Foyer (exact) | 52.66% | 43.72% | 1967.6 MiB | 6.60 M/s |
| 4 GiB | Feuer (expanded) | 63.49% | 50.66% | 4035.9 MiB | 0.26 M/s |
| 4 GiB | Foyer (expanded key) | 58.27% | 45.92% | 4020.1 MiB | 8.04 M/s |
| 4 GiB | Feuer (exact) | 53.12% | 44.10% | 4029.3 MiB | 0.27 M/s |
| 4 GiB | Foyer (exact) | 52.68% | 43.75% | 4022.5 MiB | 6.68 M/s |
| 8 GiB | Feuer (expanded) | 66.72% | 53.57% | 8119.5 MiB | 0.27 M/s |
| 8 GiB | Foyer (expanded key) | 58.53% | 46.41% | 8121.8 MiB | 7.90 M/s |
| 8 GiB | Feuer (exact) | 58.84% | 48.85% | 8117.6 MiB | 0.36 M/s |
| 8 GiB | Foyer (exact) | 52.75% | 43.88% | 8117.5 MiB | 6.37 M/s |
| 16 GiB | Feuer (expanded) | 72.91% | 59.29% | 16300.1 MiB | 0.36 M/s |
| 16 GiB | Foyer (expanded key) | 58.70% | 46.68% | 16276.8 MiB | 7.43 M/s |
| 16 GiB | Feuer (exact) | 68.39% | 56.79% | 16319.9 MiB | 0.42 M/s |
| 16 GiB | Foyer (exact) | 52.94% | 44.19% | 16315.9 MiB | 6.75 M/s |
| 32 GiB | Feuer (expanded) | 76.84% | 63.02% | 32671.8 MiB | 0.35 M/s |
| 32 GiB | Foyer (expanded key) | 58.88% | 46.92% | 32677.0 MiB | 7.18 M/s |
| 32 GiB | Feuer (exact) | 75.11% | 62.46% | 32699.0 MiB | 0.56 M/s |
| 32 GiB | Foyer (exact) | 53.57% | 45.07% | 32700.1 MiB | 6.59 M/s |

## Cold cache — 16 shards

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer (expanded) | 53.75% | 36.41% | 196.8 MiB | 1.28 M/s |
| 256 MiB | Foyer (expanded key) | 55.31% | 41.38% | 186.0 MiB | 8.72 M/s |
| 256 MiB | Feuer (exact) | 41.84% | 34.73% | 208.8 MiB | 0.95 M/s |
| 256 MiB | Foyer (exact) | 42.05% | 34.91% | 222.2 MiB | 7.20 M/s |
| 512 MiB | Feuer (expanded) | 54.65% | 37.46% | 446.9 MiB | 0.97 M/s |
| 512 MiB | Foyer (expanded key) | 55.92% | 42.16% | 462.2 MiB | 8.83 M/s |
| 512 MiB | Feuer (exact) | 42.35% | 35.15% | 470.1 MiB | 0.90 M/s |
| 512 MiB | Foyer (exact) | 42.39% | 35.18% | 426.5 MiB | 7.35 M/s |
| 1 GiB | Feuer (expanded) | 57.73% | 44.78% | 947.2 MiB | 0.39 M/s |
| 1 GiB | Foyer (expanded key) | 57.15% | 44.30% | 986.4 MiB | 8.18 M/s |
| 1 GiB | Feuer (exact) | 44.05% | 36.56% | 963.6 MiB | 0.37 M/s |
| 1 GiB | Foyer (exact) | 44.16% | 36.65% | 955.1 MiB | 6.75 M/s |
| 2 GiB | Feuer (expanded) | 58.46% | 45.88% | 1980.2 MiB | 0.37 M/s |
| 2 GiB | Foyer (expanded key) | 57.90% | 45.37% | 1983.3 MiB | 8.25 M/s |
| 2 GiB | Feuer (exact) | 44.73% | 37.13% | 1985.1 MiB | 0.40 M/s |
| 2 GiB | Foyer (exact) | 44.38% | 36.84% | 1967.6 MiB | 6.56 M/s |
| 4 GiB | Feuer (expanded) | 59.01% | 46.61% | 4022.1 MiB | 0.35 M/s |
| 4 GiB | Foyer (expanded key) | 58.29% | 45.95% | 4020.1 MiB | 8.27 M/s |
| 4 GiB | Feuer (exact) | 45.01% | 37.37% | 4036.0 MiB | 0.45 M/s |
| 4 GiB | Foyer (exact) | 44.42% | 36.87% | 4022.5 MiB | 6.90 M/s |
| 8 GiB | Feuer (expanded) | 59.35% | 47.07% | 8144.5 MiB | 0.37 M/s |
| 8 GiB | Foyer (expanded key) | 58.55% | 46.44% | 8123.3 MiB | 8.05 M/s |
| 8 GiB | Feuer (exact) | 45.15% | 37.48% | 8124.4 MiB | 0.53 M/s |
| 8 GiB | Foyer (exact) | 44.44% | 36.90% | 8114.2 MiB | 6.44 M/s |
| 16 GiB | Feuer (expanded) | 59.58% | 47.34% | 16339.1 MiB | 0.39 M/s |
| 16 GiB | Foyer (expanded key) | 58.71% | 46.70% | 16302.0 MiB | 7.64 M/s |
| 16 GiB | Feuer (exact) | 45.25% | 37.57% | 16316.8 MiB | 0.66 M/s |
| 16 GiB | Foyer (exact) | 44.48% | 36.97% | 16307.7 MiB | 6.52 M/s |
| 32 GiB | Feuer (expanded) | 59.71% | 47.59% | 32716.9 MiB | 0.50 M/s |
| 32 GiB | Foyer (expanded key) | 58.94% | 47.01% | 32711.9 MiB | 7.97 M/s |
| 32 GiB | Feuer (exact) | 45.32% | 37.64% | 32687.5 MiB | 0.88 M/s |
| 32 GiB | Foyer (exact) | 44.58% | 37.13% | 32675.9 MiB | 7.11 M/s |
