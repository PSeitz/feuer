# Tier retrieval cost model — 2026-10-11

The model scores memory entries by the retrieval work they avoid: a disk read
when a usable disk copy exists, otherwise a source fetch. It accounts for
whole-entry disk reads, alignment, I/O submissions, and checksum work.

Only memory retention policy changed. Disk victim scoring and write
admission stayed unchanged. No production policy was adopted.

## Cost of each tier

Costs are expressed as modeled seconds per retrieval, not prices or measured
end-to-end latency. For an entry with `b` payload bytes:

```text
aligned_bytes = ceil(max(b, 1) / 4096) × 4096
read_submissions = ceil(aligned_bytes / 1048576)

memory_cost(b) ≈ 0
disk_cost(b) = read_submissions × disk_submission_seconds
               + aligned_bytes × disk_seconds_per_byte
               + b × checksum_seconds_per_byte
source_cost(b) = source_fixed_seconds + b × source_seconds_per_byte
```

- **Memory:** lookup and copying are treated as negligible relative to I/O, not
  measured as zero. Retention is normalized by the actual memory charge, which
  can exceed payload length.
- **Disk:** a whole-entry read transfers 4-KiB-aligned bytes, submits reads in
  pieces of at most 1 MiB, and checksums the entire payload. Submission work can
  overlap, so adding its costs does not predict elapsed read latency.
- **Source:** a fixed fetch cost plus payload transfer cost. The replay returns
  independent zero-filled downloads, so it does not measure real source latency.

| Coefficient | Seconds | Basis |
| --- | ---: | --- |
| Disk submission | 0.00006246274509803923 | Historical assumption |
| Disk aligned byte | 8.187806372549019e-9 | Historical assumption |
| Checksum payload byte | 6.923245120755452e-11 | Measured before replay |
| Source fetch | 0.108599 | Historical assumption |
| Source payload byte | 1.1890459652219313e-8 | Historical assumption |

Checksum calibration used production xxHash64, 32 computations at each of four
payload sizes, excluding allocation. Disk and source coefficients are historical
median-based approximations from different load regimes, not calibrated means.

## Memory retention

```text
fallback_cost = disk_cost(b) if a usable published disk copy exists
                source_cost(b) otherwise
memory_score = decayed_caller_demand × (fallback_cost - memory_cost(b))
               / max(memory_charge, 1)
```

Higher scores retain more retrieval savings per charged memory byte. A mirror
tracks published whole entries, excluding queued writes, pending buffers, and
copies with known read uncertainty. Availability is a snapshot, not a guarantee
that a future disk read succeeds.

## Matched replay results

Two sequential same-host baseline → candidate pairs ran on one isolated
m8gd.8xlarge, with fresh caches and the same instrumented binary:

1. **Fixed-weight baseline:** `demand × (b + 256000) / max(memory_charge, 1)`.
2. **Assumed-disk baseline:** the same whole-entry cost model, but always using
   disk fallback. This isolates the effect of observing disk availability.

Both candidates used the published-disk-or-source model above. There were no
repetitions. Each case replayed the first 600 seconds at original pacing with
100 GB memory, 500 GB disk, 32 workers, 64 shards, exact immutable whole slices,
and a five-second observation tail. Each completed 1,606,645 requests and returned
2,800,015,397,526 bytes, with zero errors and no pending or queued writes at finish.

Decimal GB throughout. Each arrow belongs to its own matched pair.

| Metric | Fixed-weight → candidate | Assumed-disk → candidate |
| --- | ---: | ---: |
| Source GB | 129.972 → 129.695 | 130.129 → 129.699 |
| Source callbacks | 126462 → 125746 | 126903 → 125839 |
| SSD read GB | 6.429 → 7.658 | 6.823 → 7.620 |
| SSD write GB | 99.302 → 98.708 | 99.118 → 98.911 |
| Memory request hit % | 91.97956 → 91.78008 | 91.80267 → 91.78001 |
| Memory byte hit % | 95.12876 → 95.09503 | 95.10923 → 95.09623 |
| Disk request hit % | 0.14926 → 0.39330 | 0.29870 → 0.38758 |
| Disk byte hit % | 0.22943 → 0.27304 | 0.24334 → 0.27168 |
| Combined byte hit % | 95.35818 → 95.36806 | 95.35258 → 95.36791 |
| Useful memory at finish, GB | 97.059 → 96.728 | 96.823 → 96.840 |
| Request CPU seconds | 53 → 60 | 62 → 60 |
| Scheduled-completion p99, ms | 0.507 → 0.682 | 1.218 → 1.099 |
| Memory eviction-triggering insertions | 2215 → 2765 | 2861 → 2636 |
| Disk eviction-triggering insertions | 0 → 0 | 0 → 0 |

SSD counters are whole-device traffic. Useful memory is logical payload, not
RSS. Scheduled completion includes sender lateness, caller queueing, and service.

## Conclusion

Against the fixed-weight baseline, the model avoided 716 source fetches and
0.277 GB of source payload (about 0.21%), but increased SSD reads by 1.229 GB
(19.12%). The assumed-disk comparison also reduced source work while increasing
SSD reads. Observing the fallback changes retention, but the result is a
source-versus-disk tradeoff, not a demonstrated total-cost improvement.

The experiment does not isolate the contribution of alignment, submissions, or
checksum terms. Single pairs do not establish significance or causal latency
changes. Zero disk eviction means disk victim selection was not tested. Current
contended disk costs and real source costs still need calibration before using
this model to choose a production policy.
