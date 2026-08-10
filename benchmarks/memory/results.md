# Memory-only comparison

Run on 2026-08-09. See [`README.md`](README.md) for workload and accounting
definitions.

## Configuration

- cold-cache command: `target/release/feuer-memory-bench --csv --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB --shards 16 --downloader expanded,exact --feuer-compaction value-aware`
- warm-cache command: the cold-cache command plus `--warmup-iterations 1`
- input: all 165,435 operations from [`access_pattern.ndjson`](../access_pattern.ndjson)
- expanded downloader defaults: look 5 ms ahead and coalesce same-object gaps below 10 MB; whole split below 8 MiB, otherwise exact request (`COALESCING_DISTANCE_BYTES=10MB`, `WHOLE_SPLIT_THRESHOLD_BYTES=8MiB`)
- exact downloader: callback range equals requested range
- Feuer policy: source-cost-weighted pressure selection, eight 4,096-successful-access evidence epochs, and 64 successful same-shard accesses of probation per broader admission
- Foyer: `165cde3d4e638aaf2680384c02f57222b40be128`, default `S3FifoConfig`
- 16 memory shards for every engine
- one measured single-thread pass per variant; the warm-cache run first replays one untimed pass against the same cache
- common source baseline: one 125-ms GET plus exact requested bytes at 80 MB/s per request
- host: Apple M4 Max (16 logical CPUs, 64 GiB), arm64 macOS 26.4.1
- compiler: `rustc 1.96.0 (ac68faa20 2026-05-25)`, release profile

`Used Memory` is retained payload, not RSS. Foyer is exact-only because its
keys cannot reuse expanded downloads. Throughput excludes the simulated wait.

Source-Cost Hit uses a 125-ms fixed cost per GET and 80 MB/s transfer speed:

```text
source time = GETs * 125 ms + bytes / 80 MB/s
Source-Cost Hit = 1 - cached source time / exact-request no-cache source time
```

## One-iteration warm cache — 16 shards

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer (expanded) | 53.73% | 40.10% | 202.7 MiB | 0.64 M/s |
| 256 MiB | Feuer (exact) | 41.83% | 34.72% | 209.5 MiB | 0.62 M/s |
| 256 MiB | Foyer | 42.07% | 34.92% | 222.2 MiB | 7.02 M/s |
| 512 MiB | Feuer (expanded) | 54.47% | 40.86% | 468.2 MiB | 0.51 M/s |
| 512 MiB | Feuer (exact) | 42.36% | 35.15% | 444.2 MiB | 0.55 M/s |
| 512 MiB | Foyer | 42.40% | 35.19% | 426.5 MiB | 6.82 M/s |
| 1 GiB | Feuer (expanded) | 60.02% | 47.07% | 956.4 MiB | 0.22 M/s |
| 1 GiB | Feuer (exact) | 47.04% | 39.05% | 953.7 MiB | 0.19 M/s |
| 1 GiB | Foyer | 51.18% | 42.48% | 954.8 MiB | 6.44 M/s |
| 2 GiB | Feuer (expanded) | 61.77% | 49.07% | 1985.9 MiB | 0.20 M/s |
| 2 GiB | Feuer (exact) | 50.17% | 41.64% | 1971.3 MiB | 0.18 M/s |
| 2 GiB | Foyer | 52.66% | 43.72% | 1967.6 MiB | 6.81 M/s |
| 4 GiB | Feuer (expanded) | 63.54% | 50.84% | 4043.7 MiB | 0.20 M/s |
| 4 GiB | Feuer (exact) | 53.12% | 44.10% | 4029.3 MiB | 0.20 M/s |
| 4 GiB | Foyer | 52.68% | 43.75% | 4022.5 MiB | 7.02 M/s |
| 8 GiB | Feuer (expanded) | 66.51% | 53.50% | 8138.5 MiB | 0.22 M/s |
| 8 GiB | Feuer (exact) | 58.84% | 48.85% | 8117.6 MiB | 0.26 M/s |
| 8 GiB | Foyer | 52.75% | 43.88% | 8117.5 MiB | 6.23 M/s |
| 16 GiB | Feuer (expanded) | 72.60% | 59.16% | 16315.1 MiB | 0.26 M/s |
| 16 GiB | Feuer (exact) | 68.39% | 56.79% | 16319.9 MiB | 0.35 M/s |
| 16 GiB | Foyer | 52.94% | 44.19% | 16315.9 MiB | 6.87 M/s |
| 32 GiB | Feuer (expanded) | 77.05% | 63.50% | 32671.9 MiB | 0.24 M/s |
| 32 GiB | Feuer (exact) | 75.11% | 62.46% | 32699.0 MiB | 0.45 M/s |
| 32 GiB | Foyer | 53.57% | 45.07% | 32700.1 MiB | 6.57 M/s |

## Cold cache — 16 shards

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer (expanded) | 53.71% | 40.08% | 201.3 MiB | 0.69 M/s |
| 256 MiB | Feuer (exact) | 41.84% | 34.73% | 208.8 MiB | 0.69 M/s |
| 256 MiB | Foyer | 42.05% | 34.91% | 222.2 MiB | 7.18 M/s |
| 512 MiB | Feuer (expanded) | 54.46% | 40.85% | 463.9 MiB | 0.58 M/s |
| 512 MiB | Feuer (exact) | 42.35% | 35.15% | 470.1 MiB | 0.63 M/s |
| 512 MiB | Foyer | 42.39% | 35.18% | 426.5 MiB | 6.71 M/s |
| 1 GiB | Feuer (expanded) | 57.73% | 45.24% | 960.5 MiB | 0.27 M/s |
| 1 GiB | Feuer (exact) | 44.05% | 36.56% | 963.6 MiB | 0.27 M/s |
| 1 GiB | Foyer | 44.16% | 36.65% | 955.1 MiB | 6.75 M/s |
| 2 GiB | Feuer (expanded) | 58.45% | 46.13% | 1989.2 MiB | 0.25 M/s |
| 2 GiB | Feuer (exact) | 44.73% | 37.13% | 1985.1 MiB | 0.29 M/s |
| 2 GiB | Foyer | 44.38% | 36.84% | 1967.6 MiB | 6.54 M/s |
| 4 GiB | Feuer (expanded) | 59.02% | 46.82% | 4030.9 MiB | 0.27 M/s |
| 4 GiB | Feuer (exact) | 45.01% | 37.37% | 4036.0 MiB | 0.32 M/s |
| 4 GiB | Foyer | 44.42% | 36.87% | 4022.5 MiB | 6.86 M/s |
| 8 GiB | Feuer (expanded) | 59.34% | 47.19% | 8140.4 MiB | 0.28 M/s |
| 8 GiB | Feuer (exact) | 45.15% | 37.48% | 8124.4 MiB | 0.37 M/s |
| 8 GiB | Foyer | 44.44% | 36.90% | 8114.2 MiB | 6.44 M/s |
| 16 GiB | Feuer (expanded) | 59.57% | 47.46% | 16310.9 MiB | 0.27 M/s |
| 16 GiB | Feuer (exact) | 45.25% | 37.57% | 16316.8 MiB | 0.43 M/s |
| 16 GiB | Foyer | 44.48% | 36.97% | 16307.7 MiB | 6.41 M/s |
| 32 GiB | Feuer (expanded) | 59.68% | 47.60% | 32699.7 MiB | 0.34 M/s |
| 32 GiB | Feuer (exact) | 45.32% | 37.64% | 32687.5 MiB | 0.63 M/s |
| 32 GiB | Foyer | 44.58% | 37.13% | 32675.9 MiB | 6.98 M/s |

