# Requested byte-range sizes

Results from the 2026-10-11 session's offline analysis of the full FETR capture:
**17,369,893 request starts** across 269 packages, spanning **2,694.825 seconds**
(44.91 minutes). Total requested bytes: **31,592,172,620,377 (28.733 TiB)**.

Each observation is the exact requested range length, without alignment padding.
All recorded starts count, including failed requests and starts without a recorded
completion. Repeated and overlapping requests count independently. These are
**logical demand bytes, not unique object bytes, downloaded payload sizes, stored
entry sizes, or SSD traffic**. No cache replay was needed for these results.

## Full histogram

Byte-denominated intervals include both endpoints. KiB/MiB intervals exclude
their lower bound and include their upper bound. There were no zero-length
requests. Percentages are rounded independently.

| Requested size | Requests | Request share | Requested-byte share |
| --- | ---: | ---: | ---: |
| 1–16 bytes | 475,451 | 2.74% | <0.0001% |
| 17–32 bytes | 253 | <0.01% | <0.0001% |
| 33–64 bytes | 59,011 | 0.34% | <0.0001% |
| 65–128 bytes | 99,760 | 0.57% | <0.0001% |
| 129–256 bytes | 7,893 | 0.05% | <0.0001% |
| 257–512 bytes | 27,071 | 0.16% | <0.0001% |
| 513–1,024 bytes | 1,138,373 | 6.55% | 0.0025% |
| 1,025–2,048 bytes | 733,867 | 4.22% | 0.0041% |
| 2,049–4,096 bytes | 4,354,498 | 25.07% | 0.0434% |
| >4–16 KiB | 732,220 | 4.22% | 0.01% |
| >16–64 KiB | 1,148,123 | 6.61% | 0.14% |
| >64–256 KiB | 2,588,104 | 14.90% | 1.44% |
| >256–512 KiB | 733,315 | 4.22% | 0.76% |
| >512 KiB–1 MiB | 731,410 | 4.21% | 1.98% |
| >1–2 MiB | 1,282,451 | 7.38% | 6.84% |
| >2–4 MiB | 1,280,122 | 7.37% | 11.47% |
| >4–8 MiB | 961,225 | 5.53% | 21.70% |
| >8–16 MiB | 609,630 | 3.51% | 19.91% |
| >16–32 MiB | 357,339 | 2.06% | 27.11% |
| >32–64 MiB | 49,361 | 0.28% | 8.52% |
| >64 MiB | 416 | <0.01% | 0.09% |

**73.86% of requests were ≤1 MiB, accounting for only 4.37% of requested bytes.**
Conversely, requests >1 MiB accounted for 95.63% of requested bytes.

| Statistic | Requested bytes | Human-readable size |
| --- | ---: | ---: |
| Minimum | 4 | 4 bytes |
| Median | 58,785 | 57.41 KiB |
| p95 | 8,785,140 | 8.38 MiB |
| p99 | 25,442,054 | 24.26 MiB |
| Maximum | 106,541,781 | 101.606 MiB |

Percentiles were computed from individual requested lengths using NumPy's
linear percentile method, not estimated from histogram buckets.

## Detail within ≤4 KiB

Requests ≤4 KiB total **6,896,177 (39.70% of all requests)** and account for
**0.05% of requested bytes**. Of these requests, **63.14% were 2,049–4,096 bytes**.

Within this bucket, median / p95 / p99 were **2,589 / 3,971 / 4,046 bytes**.
These percentiles use the smallest exact length reaching the cumulative count.
Only **199 requests were exactly 4,096 bytes**. Most requests in this bucket were
variable lengths between 2 and 4 KiB, not fixed 4-KiB requests.

The most common exact length was **4 bytes: 471,608 requests**, or 6.84% of this
bucket. The next most common lengths were 610 bytes (94,864 requests), 75 bytes
(62,717), and 729 bytes (42,544).

Request counts do not establish tiny-entry disk occupancy or write padding: a
request can hit memory or read a slice of a larger covering entry.

## Provenance

Only identifier-free aggregates are documented here. The private archive is
`../replay-feuer/feuer-7-searcher-0.tar.zst`, with SHA-256:

```text
a13aeae5c77da3709814fef42708d6cb55a867a553ba01dfc8c7e8338e7807f7
```

Original reports, exact counts, and analysis scripts remain in the private
`../replay-feuer/request-size-histogram-fb7a002/` directory:

- `REPORT.md` and `histogram.json`: full histogram and exact byte totals.
- `SMALL-SIZES.md` and `small-sizes.json`: small-range histogram and exact lengths.
- `histogram.py` and `small-sizes.py`: offline analysis using the sibling
  `read-locality-fetr/decode.py` FETR decoder.

To recompute from the existing private capture, without launching an SSD replay:

```sh
cd ../replay-feuer/request-size-histogram-fb7a002
python3 histogram.py
python3 small-sizes.py
```

The earlier **122,838 whole-entry SSD reads** histogram covered only a
180-second cold-cache replay. It measured aligned whole-entry reads, not these
full-capture requested ranges. Do not combine the two populations.
