# Read submissions: keep 1-MiB chunks

**Keep the current 1-MiB default.** Chunks protected small/medium-read tail
latency better. Whole reads improved overall median and large-read p99, but
were not uniformly better. These short tests do not establish an optimal size.

## Results

Two matched pairs on `m8g-32cpu-local-ssd-2`, 2026-10-21. The second pair added
p90 reporting. Below are API latencies from that second pair, in milliseconds.
Each cell is **1-MiB chunks → whole read**.

| Read size | Count | p50 | p90 | p99 |
| --- | ---: | ---: | ---: | ---: |
| All | 1750 | 7.85 → 3.88 | 49.69 → 79.46 | 482.54 → 251.33 |
| ≤64 KiB | 341 | 14.66 → 22.28 | 49.50 → 143.03 | 89.41 → 269.86 |
| >64 KiB–1 MiB | 713 | 2.55 → 1.95 | 31.11 → 58.58 | 75.28 → 226.94 |
| >1 MiB | 696 | 6.07 → 5.03 | 167.44 → 163.70 | 568.59 → 261.27 |

Overall mean: **29.81 → 31.03 ms**. Both completed all reads at **678.3 MiB/s**
under recorded pacing, not saturation. Both exited successfully. Runtime including
setup was **12.82 → 12.42 seconds**. Clippy passed with `-D warnings`.

The first pair also showed worse small-read p99 with whole reads, but its better
small-read median did not repeat. Maximum task-start lag in the second pair was
**91 → 205 ms**, so allocation/runtime scheduling confound kernel-only conclusions.

## Workload and reproduction

A contiguous **[360, 370) second** slice of actual disk-read starts from the
[historical read-duplication replay](../../cache/read-volume-duplicates/README.md):
**1,750 reads, 6.23 GiB**, maximum read **61.06 MiB**. Selected before measurement
as the ten-second window closest to the full log's request/byte size histograms.
It is a busy slice, not a perfect representation of the full capture.

The benchmark preserves sizes, offsets, repetition, ordering, and planned arrival
spacing. Four Tokio workers use DataFile's unpooled read path. Writes only initialize
a fresh sparse file before timing. About 6.1 GiB of disk space is needed. No cache
policy, checksums, pooling, or original filesystem placement is reproduced.

```sh
mkdir -p experiments/storage/read-trace/raw
python3 experiments/storage/read-trace/extract.py \
  experiments/cache/read-volume-duplicates/raw/results/diagnostic/read-events.csv \
  360 10 > experiments/storage/read-trace/raw/reads.csv
cargo bench --locked -p feuer-storage --bench read_trace -- \
  /mnt/local-ssd experiments/storage/read-trace/raw/reads.csv
```

The source log and extracted CSV are local and ignored, not in a clean checkout.
CSV columns are `time_ns,offset,length`, ordered with positive read lengths.
The isolated [candidate patch](whole-read.patch) raises the request limit to 64 MiB,
so each tested read fits in one initial io_uring request. Read slots remain 64.
Linux may split requests internally. **Production code remains unchanged.**

Storage source: `0c0f6a1695f511301067ce44fe3d3b88975faf8f`, plus this benchmark.
Raw measurements and hashes remain in ignored `raw/`, including `comparison.log`
and `p90-comparison.log`. Preserved remote workspaces:
`/home/ubuntu/feuer-read-trace.3FYmkY` and `/home/ubuntu/feuer-read-p90.ftyUcW`.
