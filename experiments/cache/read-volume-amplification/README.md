# Remaining disk-read amplification

## Result and scope

**Prioritize missing checksum coverage, not DataFile alignment or buffer capacity.** Current source already learns requested-range checksums after verified reads **and prepares recent-range checksums from write buffers**. Adding either feature again is not a remaining opportunity.

The most concrete next candidate is to prepare checksums for useful older distinct ranges that have fallen out of the writer's 64-event history. The historian already retains those distinct range counters. A new offline analysis finds substantial eligible range geometry, but no current-revision disk-byte saving has been measured.

This historical analysis inspected `2f381acdb13e114593e98d1078f29b341279380d` plus the captured working tree, including partial-read and bypass changes. “Current” below refers to that snapshot, not today's checkout. `settings.json` identifies the inspection, source hashes, and saved patch. Relevant inspected files are copied under ignored `raw/source/`. No replay or SSD test was run.

## Current source behavior

| Layer | Current behavior and evidence |
| --- | --- |
| Caller demand | Records requested range once before memory lookup. Disk hits promote only requested bytes with actual backing-capacity charge. `feuer/src/cache.rs:356-360,398-410`. |
| Entry sizes | A covering insertion removes contained entries. Lookup selects the covering entry at greatest eligible start. Exact small source downloads can later be subsumed by a large exact source download. `feuer-storage/src/disk_cache.rs:557-592,630-635`. |
| Checksum selection | Reuse an active whole-entry read first, otherwise choose the shortest covering subrange checksum. Without one, verify the entry. `disk_cache.rs:494-518,650-664`. |
| Learning after reads | A successful caller learns its exact nonempty proper subrange, guarded against stale entry replacement. Other requested ranges are not harvested from the same verified bytes. `disk_cache.rs:388-417`. |
| Writer preparation | After writing, snapshot recent observed requests, deduplicate proper contained ranges, hash trusted write bytes outside index/metadata locks, and attach checksums only to the published entry. `feuer-storage/src/disk_cache/writer.rs:53-104`. Present since `937efdc`, not just proposed. |
| History selection | Writer uses at most 64 recent events/key and age 262,144 global requests by default. Repeats occupy separate event positions. Distinct counters remain available separately through `read_request_counts`. `feuer-historian/src/lib.rs:91-121,213-246`. |
| Recovery | Only the entry-wide checksum persists. Recovered entries start with an empty subrange vector. `disk_cache.rs:481-489`, `disk_cache/metadata.rs:26-33`. |
| DataFile reads | `read_payload` counts checksum-range length, not caller-request length or aligned transfer length. `read_slice` reads exactly the containing 4-KiB-aligned disk byte range. `feuer-storage/src/file.rs:171-176,246-292`. No neighboring entries or lookup metadata are read. |
| Kernel requests | Each aligned read is split into disjoint pieces of at most 1 MiB, sharing one allocation. The last piece is not rounded up to 1 MiB. `feuer-storage/src/uring.rs:218-242`. |
| Buffer sizing | Pool buckets start at 32 KiB, then 256 KiB. Exposed length remains the actual I/O length. A returned slice is copied only when that saves at least 25% capacity. `feuer-memory/src/buffer.rs:23-37,97-117`, `file.rs:179-194`. |

`PayloadRead` and its result cell deduplicate concurrent reads. Replacing that with independent tiny reads can reduce bytes but increase calls, and can duplicate work already underway. Preserve sharing, checksum validation, entry-incarnation checks, cancellation behavior, and allocation charging.

## What existing measurements establish

The completed 600-second pair in `../partial-read-checksums/REPLAY.md` tested base `033284d` plus read-time learning, **before writer seeding**. It used exact capacities, pooled immediate independent sources, matched jemalloc, 64 shards, and fresh caches. It is evidence about the mechanism, not a measurement of this working tree.

| Metric | Before read learning | With read learning |
| --- | ---: | ---: |
| Requests / total returned bytes | 1,606,645 / 2,800,015,397,526 | same |
| Memory hits | 1,444,090 | 1,444,487 |
| Disk/buffer hits | 33,109 | 32,816 |
| Source callbacks | 129,446 | 129,342 |
| Disk/buffer returned GB | 66.3057 | 66.3399 |
| DataFile read GB / calls | 134.293 / 31,322 | 132.242 / 31,054 |
| Process reads GB, rounded | 134.358 | 132.306 |
| Device reads GB | 134.358 | 132.306 |
| Source GB | 140.371 | 140.313 |
| Request CPU seconds | 272 | 275 |
| Sampled peak RSS GB | 112.135 | 112.087 |
| Service mean / p99 ms | 3.17932 / 13.6871 | 3.16615 / 12.3849 |
| Completion mean / p99 ms | 6.65377 / 126.291 | 6.45475 / 117.507 |

Both completed exactly once, exited zero, had no I/O/checksum errors and zero pending payload after the observation tail. Read bytes decreased 1.53%, with scheduling/admission differences, not a demonstrated small performance win. DataFile bytes / disk-buffer returned bytes remained approximately 2.0. That ratio is not pure SSD amplification because the denominator includes buffered hits and multiple callers can share one read. Source and device scopes differ from phase DataFile counters.

Writer seeding has **focused tests but no replay result**. The validation report saved at commit `937efdc` documents first reads of 3,006 bytes instead of 98,321 or 2,097,169 entry bytes, and full-entry reads again after restart. Its historical report was removed from the current tree, so a copy is retained at `raw/writer-checksums-validation.md`. Current tests explicitly exercise older ranges dropped from the event history (`disk_cache/metrics_tests.rs:237-318`).

Other evidence is not a current performance baseline:

- `../disk-read-amplification-2cf2f27/README.md`: 256x amplification for two 16-KiB requests covered by 4 MiB, predating both partial-read features.
- `../full-trace-20261013T0734/RESULTS.md`: range mode captured 1.883 TB reads for 602.542 GB disk/buffer returns. Complete range costs failed validation because pending payload remained. Different source model and duration, not a matched comparison here.
- `../full-trace-scoring-20261013T1745/RESULTS.md`: all cases OOM-killed. Do not use these incomplete runs to estimate savings or choose a winner.

## New offline evidence: ranges omitted by writer seeding

`analyze.py` reuses the existing decoded 600-second prefix. It validates **1,606,645 requests, 2,800,015,397,526 bytes, 2,212 objects and 123,885 distinct object/ranges**. Maximum distinct ranges per object is 224 in this prefix.

This deliberately conditional model publishes an exact entry immediately for every uncovered request, removes contained entries, never evicts, and assumes all covered requests read serially and learn their exact checksums. At each publication it applies the current 64-event/262,144-request history selection. It is **not replay-state reconstruction**. Actual memory hits, writer delays/skips, replacements, bypass, shared reads and publication success are unknown.

It reproduces independently established geometry: 122,358 candidate publications, 4,401 above 8 MiB, and 35,670 later proper covered requests. For large entries, writer history supplies 843 seeds whereas all distinct prior contained observations supply 3,444. The additional 2,601 seeds require hashing **7,927,863 bytes** in this model, and about **83,232 bytes** of checksum records before vector/allocator overhead. These are cumulative preparation costs, not peak memory.

| First unseeded later request, entry >8 MiB | Count | Requested bytes | Selected entry bytes in serial model | Exact aligned read geometry |
| --- | ---: | ---: | ---: | ---: |
| Prior exact range lost to 64-event limit | 2,380 | 7,175,798 | 132,503,349,830 | 16,855,040 |
| Range not observed before publication | 293 | 931,747 | 12,465,136,205 | 2,162,688 |
| Prior exact range lost only to age | 0 | 0 | 0 | 0 |

Across all entry sizes, the corresponding counts are 2,570 history-lost and 1,527 previously unseen ranges. Each entry/range is counted only at its first unseeded use, not every repeat. The writer's recent seeds are already excluded from these opportunities.

**Do not report 132.5 GB as expected savings or a guaranteed maximum.** It is the sum of covering lengths for conditional first uses. The prior measured replay read only 132.242 GB in total, illustrating why these joins cannot substitute for real cache state. The result establishes a concrete missing source of checksums and the small payload-hashing cost in this input, not realized SSD traffic. Concurrent whole-entry sharing can eliminate several candidate reads at once.

Validation: two synthetic checks cover existing writer seeds, repeated-event displacement, first learning, and no double-counting of repeats. Independent earlier analysis totals match. Output and decoded-input SHA-256 are in `geometry.json`. From the repository root, reproduce with `python3 experiments/cache/read-volume-amplification/analyze.py`, which writes `raw/geometry.json`. It requires the local-only decoded input `experiments/cache/partial-read-checksums/raw/static-analysis/checksum-learning/requests.jsonl`. A clean checkout does not contain that input.

## Alignment and buffer sizing are not the primary byte problem

For a nonempty selected checksum range of length `L` at disk offset `a`, transferred bytes are

```
ceil(((a % 4096) + L) / 4096) * 4096
```

Edge padding adds at most 8,190 bytes per read. Even assigning that maximum to all 31,054 calls in the historical candidate gives **254,332,260 bytes, only 0.1924% of 132.242 GB**. Whole-entry payload addresses are aligned and add at most 4,095 bytes instead. Its actual device/API difference was about 64 MB, including startup and other scope differences. Padding cannot explain the roughly 66-GB excess of API read bytes over disk/buffer returned bytes.

A 3,006-byte read at an aligned address transfers 4 KiB but may occupy a 32-KiB pool allocation. An unaligned 32-KiB request can transfer 36 KiB, temporarily use the current 256-KiB bucket, then copy down to a 32-KiB allocation. **It does not read 256 KiB.** The existing test comment mentioning a 64-KiB bucket is stale relative to the actual bucket table.

Smaller pool buckets or changed shrinking could improve memory residency and indirectly avoid later disk reads, but need per-bucket occupancy, eviction and source-byte evidence. They do not directly shrink DataFile I/O. Avoid tuning 1-MiB submission size to solve byte amplification: transfer pieces already cover only the aligned selected checksum range.

## Prioritized recommendations

1. **Test distinct prior-range checksum preparation against current writer seeding.** Snapshot eligible ranges from existing permanent historian counters rather than relying solely on the recent event deque. Hash the actual new payload outside cache/history locks. Start with this narrow alternative rather than enlarging the global event-history setting, which also affects range trimming. The 2,380 large-entry first-use candidates and 7.93-MB extra hashing geometry make this the strongest low-code lead. Cost/risk: more scanning and per-entry checksum records, possibly obsolete demand, and workload-specific range counts. A current-revision paired replay is required before adoption.
2. **If new/unseen ranges still dominate actual reads, test fixed-size independently verifiable payload slices.** This covers first-ever ranges without relying on historical observation. An in-memory checksum vector computed from trusted write bytes avoids an initial format change but still loses coverage on restart. At 4 KiB, raw 8-byte checksums cost 0.1953% of payload bytes (128 KiB per 64 MiB). At 64 KiB, they cost 0.01221% (8 KiB per 64 MiB), with more overread for tiny requests. These are contiguous payload byte ranges, not metadata gaps or new payload copies. Requires read/verification changes, careful final-slice handling, and additional hashing. Persist only if restart reads justify metadata-format work. Do not skip verification to obtain smaller reads.
3. **Harvest additional observed ranges from already verified bytes only if telemetry shows post-publication memory-only demand.** Current learning adds only the caller's range. Earlier static work found little first-read opportunity and large late-read figures disappeared when prior calls had already learned checksums. Writer seeding further overlaps this proposal. Validate known-checksum state and actual future disk uses before adding another history scan.
4. **Defer alignment, buffer-only and selection changes as direct byte optimizations.** Existing first-600-second geometry found no beneficial alternative covering entry, no three-level partial-read nesting for wider active-read reuse, and no useful two-checksum coverage. Same-allocation broader promotion had only 1,157 conditional extra requests / 9.17 MB. These findings are workload-specific and are in `../partial-read-checksums/static-analysis/README.md`. Buffer residency belongs in a separate matched memory-policy experiment.

## Historical follow-up proposal

Follow [the timed replay procedure](../../storage/timed-replay.md), including activity and free SSD checks. Freeze the tree being evaluated, including existing patches, into isolated source snapshots. For the inspected range-aware design, the baseline must retain writer seeding, read sharing, bypass/admission and allocator behavior. This proposal does not apply directly to the whole-object-only cache after `d965556`. Candidate changes only the source of eligible writer checksum ranges. Do not use the old read-learning binary as baseline.

Before a replay, focused tests should show an old range displaced by repeated events reads exact bytes on its first disk use, no extra payload writes, same checksum-mismatch behavior, and safe replacement/recovery. Measure only one baseline/candidate pair, first 600 seconds, fresh caches, **100000000000 memory bytes, 500000000000 disk bytes, 64 shards**, matched jemalloc and identical source mode. Preserve paced arrivals and exactly-once completion. Do not change capacities to create pressure or bypass requests to the source to claim a byte win.

A few measurement-only counters in the isolated snapshot should distinguish exact subrange reads from whole-entry fallback and count writer seed records/hash bytes. Separate unique read initialization from shared caller membership, buffered hits, and logical caller bytes. Record aligned submitted bytes alongside existing DataFile API bytes/calls. Without these distinctions, aggregate read/returned ratios cannot identify this candidate's effect.

Retain all standard replay metrics: logical returned bytes and tier outcomes, DataFile read/write bytes/calls/errors/durations, process/device I/O, source gets/bytes, latency distributions, CPU/RSS/buffer accounting, write outcomes and tail/pending work. Reject incomplete runs rather than treating lower traffic from unfinished or source-shifted work as an improvement.
