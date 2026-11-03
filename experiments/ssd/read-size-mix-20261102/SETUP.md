# SSD capacity with the Feuer 27 read-size mix

## Question and scope

Measure read capacity and read/write interference using the newly captured successful-read size distribution, rather than a uniform 4-KiB workload. This is a native fio SSD benchmark. It does not measure Feuer's implementation throughput or replay production disk arrivals.

The source is one searcher session containing 88,151,303 successful logical accesses over approximately 2,685 seconds: 78,311,306 memory hits, 9,839,997 callback successes, and zero disk-hit outcomes. Its manifest specifies 710,000,000,000 bytes of memory capacity and zero disk capacity. FETR v1 contains no disk-I/O events. The requested-byte distribution therefore serves as a synthetic SSD size mix, not an estimate of actual SSD traffic.

The source has 88,623,097 starts, so some starts lack captured outcomes. Sampling uses successful outcomes directly and does not assume complete start/outcome pairs. Source provenance is retained in `experiments/cache/feuer-27/{archives,s3-manifest,sample-manifest}.json`.

## Input construction

`prepare.py` validates all 864 FETR v1 packages and samples each successful event independently with probability 1/64 using NumPy's generator seeded with 20261102. It retains actual payload lengths, shuffles them, and rounds lengths upward to 4 KiB for native direct I/O. It draws synthetic uniform 4-KiB-aligned offsets within a fully initialized 64-GiB read file. Original timing, addresses, and cache outcomes are not replayed.

The resulting stream has 1,377,574 reads, totaling 2,280,448,906,732 logical bytes and 2,283,602,395,136 aligned bytes. Alignment adds 0.1383%. Lengths range from 2 to 80,598,022 logical bytes. `input.json` records archive and generated-stream hashes. The full sequence is much longer than each measurement. Each case starts from the same sequence and measures its own completed prefix after warmup.

## Measurement

- Host: `m8g-32cpu-local-ssd-2`, EC2 m8gd.8xlarge, 32 Graviton4 CPUs, local NVMe SSD on ext4.
- fio 3.36, Linux 6.17.0-1007-aws, aarch64.
- Native `io_uring`, `O_DIRECT`, no registered files/buffers or SQPOLL.
- One reader, depth 32 whole read requests. Large requests remain whole at the fio API boundary. This differs from Feuer's 1-MiB submissions and active-read limit.
- One optional sequential writer, 1-MiB requests, depth 8, separate fully initialized 8-GiB file on the same SSD.
- Three cases, once each: read-only, 450,000,000 bytes/s write cap, uncapped writes. The 450-MB/s point is a historical comparison setting, not measured production write demand.
- Ten seconds warmup, sixty seconds measured per case, five seconds idle before each case. This is a size-distribution capacity test, not the ten-minute timestamp-paced cache-replay procedure.
- Files initialized before measurement and removed afterward. No other user's files or processes are changed.
- All configuration, input, and output files reside on EBS. Only the initialized data files reside on the SSD.
- Per-I/O read submission, completion, and total-latency logs retain the aligned request size. No interval averaging. Total latency includes fio submission and completion time.
- fio JSON provides measured read/write bandwidth, IOPS, mean and percentile latencies, CPU utilization, queue-depth distribution, and errors. `iostat`, `pidstat`, and `/usr/bin/time -v` capture device activity and process costs. Monitor intervals include job startup and warmup.

## Interpretation

The result describes capacity for this size mix at the selected concurrency. A lower saturated read rate with writes is not evidence of unmet application demand. Production tuning requires the actual SSD read/write rates, bursts, and latency targets. One measurement per case cannot establish small improvements, optimal limits, or variability across runs.

Large reads account for most bytes. Report both operation-weighted size shares and byte shares, and latency by size, to avoid hiding small-read latency behind aggregate bandwidth.
