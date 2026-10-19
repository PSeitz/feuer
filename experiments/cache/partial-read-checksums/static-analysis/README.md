# Static analysis of disk-read reduction proposals

These offline trace analyses compare the range geometry required by five cache
proposals. Recommendations are historical, for the range-aware design before
`d965556`, and conditional on the modeled trace. They measure geometry, not
implementation benefits. The linked scripts and captures are local-only.

## Conclusion

The geometry favored learning observed-range checksums from the writer's trusted payload before publication. The static trace analysis does not support implementing the two read-selection changes, broader same-allocation promotion, or two-checksum coverage to improve this replay.

This revises the initial recommendation to start with the two selection changes. They are valid general ideas, but their necessary range geometry is absent from this input.

Each analysis validated the same 600-second prefix: 1,606,645 requests, 2,800,015,397,526 requested bytes, 2,212 objects, and 123,885 distinct object/range combinations.

## Findings by proposal

| Proposal | Static finding | Decision for this replay |
| --- | --- | --- |
| Reuse any active covering partial read | Zero three-level chains of original entry, proper partial checksum, and smaller request, even ignoring chronology. Large reads cannot be learned partial reads in this prefix, and active whole-entry reads are already reused. | Do not implement for this workload. |
| Choose the cheapest covering entry | Zero requests covered by two non-containing observed ranges, even allowing future ranges as candidates. Containing entries cannot coexist in the disk index. | Do not implement for this workload. |
| Promote all verified bytes when the same allocation is already retained | 1,157 later requests, totaling 9.17 MB requested, have conditional additional-coverage opportunities. Productive cases all use 32-KiB allocations. No large-read geometry qualifies with successful shrinking. | Low priority. These are not measured disk savings. |
| Learn other observed-range checksums from trusted bytes | Source-start history offers 843 seeds across 189 modeled large-entry candidates, hashing 2.53 MB. Of those seeds, 825 are requested later. Publication and later disk use are not established by the trace. | Best candidate, favoring the writer hook. |
| Verify using two observed-range checksums | Zero beneficial cases with a larger original parent, even ignoring chronology. Two diagnostic whole-entry repeats would increase aligned reads from 8 to 12 KiB each. | Do not implement for this workload. |

Negative geometry findings rely on fresh caches, exact-range source downloads, and compaction retaining observed request ranges without merging them. They do not generalize to expanded production downloads or other traces.

## Why writer learning is different

The significant geometry is small requests inside much larger observed ranges. For chronological pairs with a covering range above 8 MiB, covering lengths are about 10–67 MB while requested slices are only 2,479–5,238 bytes. Such slices shrink profitably, so exposing the entire covering allocation at unchanged charge cannot help them. There are no intermediate nested ranges to exploit with the proposed partial-read sharing change.

Historical small-range observations can already exist when the larger payload is written. Hashing those slices from the actual trusted write buffer can prepare the larger entry for its first subsequent partial disk lookup, without another disk read or payload copy.

The writer analysis uses an explicit conditional publication model, not reconstructed replay state. For large candidates:

- Source-start history: 843 seeds across 189 candidates, 2.531 MB hashed, approximately 27 kB of record storage before vector/allocator overhead, 825 seeds requested later.
- Assumed publication delays of 1–1,000 ms: 794–804 seeds, with 776–786 requested later.
- First-later-lookup verified-read hook with assumed 100-ms completion: only 78 seeds across 15 large candidates. The 64-event history has often lost earlier useful observations by then.
- A 60-second delayed read scenario appears much larger, but contributes zero new seeds if preceding requests already learned their checksums. It is not established workload opportunity.

A hypothetical serial all-disk comparison estimates 40.685 GB fewer logical bytes for source-start writer seeding. Adding assumed 1–100-ms shared-read lifetimes reduces that difference to 16.963 GB and increases read calls by 192. Neither is a forecast, measured saving, or guaranteed limit. Actual memory hits, publication failures, entry replacement, read selection, and shared-read membership are missing from the recorded telemetry. Do not add estimates from different hooks or proposals.

## Historical follow-up proposal

If proceeding to implementation, start with publication-time seeding using the existing observed-range history and the exact write buffer. Keep hashing outside disk-index locks, publish metadata only with its own entry, and discard it if publication loses. Keep metadata memory-only and preserve existing checksum verification, entry lifecycle, and payload ownership.

Validate with focused tests before another replay. Counters should distinguish seeds created, bytes hashed, and subsequent reads selecting seeded checksums. A workload saving remains unmeasured.

## Reports and reproduction

The small per-proposal summaries are beside this file. Full reports, scripts, validation logs, and derived data are local and ignored:

- [Read sharing](../raw/static-analysis/read-sharing/REPORT.md)
- [Entry selection](../raw/static-analysis/entry-selection/REPORT.md)
- [Promotion](../raw/static-analysis/promotion/REPORT.md)
- [Checksum learning](../raw/static-analysis/checksum-learning/REPORT.md)
- [Two-checksum coverage](../raw/static-analysis/checksum-coverage/SUMMARY.md)

Exact replay settings and measured results remain in [replay-settings.json](../replay-settings.json) and [REPLAY.md](../REPLAY.md).
