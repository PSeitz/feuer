# S3 GET latency and per-object throughput benchmark

## Environment

- EC2 instance: `m8azn.metal-12xl` in `us-east-1a`
- Host: 48 logical CPUs, 192 GiB RAM, 100-Gbit networking
- S3 bucket region: `us-east-1`
- Endpoint: public regional S3 endpoint; the VPC has no S3 endpoint
- Corpus: 761 objects, 82.092 GiB total
- Object size: 110.110 MiB p50, 117.852 MiB p95, 121.617 MiB p99
- Client: boto3/botocore 1.34.46

## Methodology

- TTFB is the monotonic time from immediately before `get_object` through completion of `StreamingBody.read(1)`. It includes SDK signing, request handling, and network time, but excludes client construction.
- Throughput is object payload throughput after the first byte, keeping it separate from TTFB.
- Full objects were read in 4-MiB chunks and discarded in memory; no filesystem I/O was in the measured path.
- Retries were disabled, and no requests failed or retried.
- Concurrent runs used one Linux process and one warmed persistent TCP/TLS connection per worker. Every worker performed one full-object warmup before measurement.
- Percentiles use R-7 linear interpolation.

## TTFB percentiles

| Mode | Requests | p50 | p90 | p95 | p99 |
|---|---:|---:|---:|---:|---:|
| Fresh TCP/TLS, 1-byte ranged GET | 100 | 186.711 ms | 233.754 ms | 243.292 ms | 257.689 ms |
| Sequential, warmed connection | 200 | 126.881 ms | 165.715 ms | 182.236 ms | 229.491 ms |
| 48 concurrent, warmed | 761 | 108.599 ms | 142.659 ms | 156.537 ms | 194.507 ms |
| 96 concurrent, warmed | 761 | 90.441 ms | 116.854 ms | 126.574 ms | 153.087 ms |
| 192 concurrent, warmed | 761 | 85.822 ms | 114.619 ms | 126.992 ms | 148.088 ms |

## Per-object payload throughput percentiles

Throughput is measured after TTFB. The p10 column represents the slow tail.

| Mode | Requests | p10 | p50 | p90 | p95 | p99 |
|---|---:|---:|---:|---:|---:|---:|
| Sequential, warmed connection | 200 | 36.079 MiB/s | 48.145 MiB/s | 67.755 MiB/s | 86.255 MiB/s | 97.679 MiB/s |
| 48 concurrent, warmed | 761 | 52.153 MiB/s | 80.205 MiB/s | 99.654 MiB/s | 101.559 MiB/s | 105.124 MiB/s |
| 96 concurrent, warmed | 761 | 84.607 MiB/s | 98.877 MiB/s | 103.726 MiB/s | 104.352 MiB/s | 106.382 MiB/s |
| 192 concurrent, warmed | 761 | 47.571 MiB/s | 91.789 MiB/s | 102.954 MiB/s | 104.265 MiB/s | 106.092 MiB/s |

## Notes

- Warmed full-object GETs are the most representative results for an SDK client that reuses connections.
- Fresh-connection TTFB was more variable: an independent 10-request curl spot check produced 81.179 ms p50 and 146.426 ms p95.
- No network errors or packet drops were observed during the benchmark.
