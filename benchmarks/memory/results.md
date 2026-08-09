# Memory-only comparison

Run on 2026-08-09. See [`README.md`](README.md) for workload and accounting
definitions.

## Configuration

- command: `target/release/feuer-memory-bench --capacity 256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,128GiB --shards 16 --downloader expanded,exact --feuer-compaction value-aware`
- input: all 165,435 operations from [`access_pattern.ndjson`](../access_pattern.ndjson)
- expanded downloader: whole split below 20 MiB; exact request above 4 MiB; otherwise add 4 MiB on each side
- exact downloader: callback range equals requested range
- Feuer policy: value-aware pressure selection with 64 successful same-shard accesses of probation per broader admission
- Foyer: `165cde3d4e638aaf2680384c02f57222b40be128`, default `S3FifoConfig`
- 16 memory shards for every engine
- one cold-cache pass per variant, single thread
- source model: 80 ms per GET plus transfer at 80 MB/s
- host: Apple M4 Max (16 logical CPUs, 64 GiB), arm64 macOS 26.4.1
- compiler: `rustc 1.96.0 (ac68faa20 2026-05-25)`, release profile

The 128-GiB row is the upper-limit policy and accounting case. `Used Memory` is
the end-of-run retained payload weight in MiB, not process RSS; shared immutable
benchmark payloads keep the test runnable on the reported 64-GiB host.
Throughput is millions of measured operations per second.

## Expanded downloader — 16 shards

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer | 47.12% | 45.03% | 167.2 MiB | 0.89 M/s |
| 256 MiB | Foyer | 34.87% | 33.19% | 171.8 MiB | 8.08 M/s |
| 512 MiB | Feuer | 48.52% | 46.37% | 432.0 MiB | 0.66 M/s |
| 512 MiB | Foyer | 39.41% | 37.51% | 428.0 MiB | 8.41 M/s |
| 1 GiB | Feuer | 50.64% | 48.34% | 961.2 MiB | 0.26 M/s |
| 1 GiB | Foyer | 40.76% | 38.83% | 940.2 MiB | 8.28 M/s |
| 2 GiB | Feuer | 52.67% | 50.19% | 1974.1 MiB | 0.22 M/s |
| 2 GiB | Foyer | 41.80% | 39.79% | 1976.0 MiB | 8.02 M/s |
| 4 GiB | Feuer | 53.75% | 51.24% | 4017.8 MiB | 0.16 M/s |
| 4 GiB | Foyer | 42.52% | 40.48% | 4016.8 MiB | 7.45 M/s |
| 8 GiB | Feuer | 54.26% | 51.84% | 8116.1 MiB | 0.09 M/s |
| 8 GiB | Foyer | 43.06% | 40.97% | 8124.2 MiB | 7.17 M/s |
| 128 GiB | Feuer | 55.50% | 53.22% | 130968.3 MiB | 0.02 M/s |
| 128 GiB | Foyer | 44.71% | 42.61% | 131009.5 MiB | 6.47 M/s |

## Exact downloader — 16 shards

| Capacity | Engine | Request Hit | Source-Cost Hit | Used Memory | Throughput |
| ---: | --- | ---: | ---: | ---: | ---: |
| 256 MiB | Feuer | 41.95% | 31.78% | 212.2 MiB | 0.72 M/s |
| 256 MiB | Foyer | 42.05% | 31.86% | 222.2 MiB | 7.85 M/s |
| 512 MiB | Feuer | 42.39% | 32.11% | 413.1 MiB | 0.64 M/s |
| 512 MiB | Foyer | 42.39% | 32.11% | 426.5 MiB | 7.62 M/s |
| 1 GiB | Feuer | 43.69% | 33.10% | 959.8 MiB | 0.33 M/s |
| 1 GiB | Foyer | 44.16% | 33.46% | 955.1 MiB | 7.36 M/s |
| 2 GiB | Feuer | 43.71% | 33.11% | 1973.6 MiB | 0.33 M/s |
| 2 GiB | Foyer | 44.38% | 33.63% | 1967.6 MiB | 7.09 M/s |
| 4 GiB | Feuer | 43.72% | 33.12% | 4037.6 MiB | 0.32 M/s |
| 4 GiB | Foyer | 44.42% | 33.66% | 4022.5 MiB | 7.20 M/s |
| 8 GiB | Feuer | 43.74% | 33.14% | 8131.5 MiB | 0.32 M/s |
| 8 GiB | Foyer | 44.44% | 33.69% | 8114.2 MiB | 6.64 M/s |
| 128 GiB | Feuer | 45.39% | 34.92% | 130874.6 MiB | 0.42 M/s |
| 128 GiB | Foyer | 45.34% | 34.88% | 130912.9 MiB | 8.16 M/s |

