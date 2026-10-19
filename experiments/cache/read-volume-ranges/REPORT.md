# Request-range reuse and replay fidelity

## Conclusion

**Do not prioritize fragment assembly or partial-memory gap reads for this capture without per-lookup evidence.** The necessary request geometry is small. Among 1,606,645 starts in the first 600 seconds, only **661 requests / 47.420 MB** can be fully covered by earlier differently shaped request ranges without any one range covering them. Another **6,864 requests** overlap **322.472 MB** of earlier differently shaped ranges but still need missing bytes. These are optimistic logical coverage counts, not observed memory contents, disk hits, or saved disk bytes.

**The larger finding is a source-model measurement gap.** The capture records **26,894 expanded successful callbacks**, 23.57% of successful callbacks. Their 4.539 GB of requested bytes came from 23.447 GB of reported downloaded ranges. Replay discards that expansion and independently downloads exact request ranges. It therefore does not reconstruct the original memory coverage or the expanded-download-to-compaction-to-disk path. Neither a production disk-byte saving nor its absence can be inferred solely from this exact-range replay.

This is offline geometry analysis, not a new SSD replay or candidate comparison. Alignment/checksum changes and concurrent duplicate-read elimination are outside its scope.

## Scope and validation

- Historical inspected HEAD: `2f381acdb13e114593e98d1078f29b341279380d`, with captured uncommitted changes. “Current” below refers to that snapshot, not today's checkout. `raw/working-tree.patch`, `raw/git-status.txt`, and `raw/source-hashes.txt` identify the inspected tracked state. No current-source binary was built or measured.
- Input SHA-256: `a13aeae5c77da3709814fef42708d6cb55a867a553ba01dfc8c7e8338e7807f7`.
- Half-open prefix starts at 467,995,852,934 ns and lasts 600 seconds. Starts are sorted by timestamp/request ID. Outcomes are joined from the entire archive, including later packages.
- Independently decoded totals match earlier saved analyses: **1,606,645 requests, 2,800,015,397,526 requested bytes, 2,212 objects, 123,885 distinct exact ranges**.
- Validation checks all category counts/byte sums and union totals against the independently decoded prior request cache. 10,000 randomized interval cases agree with an independent byte-set calculation. `raw/validation.json` passed.
- No new replay was necessary to establish the small geometry. An uninstrumented aggregate replay would not establish whether these bytes were resident at each disk miss.

## Current behavior

| Question | Current contract and code |
| --- | --- |
| Can memory assemble fragments? | No. `feuer-memory/src/store/shard.rs:35-42,135-141` selects one exact or covering entry. Adjacent/overlapping entries are not joined. Existing tests at `feuer-memory/src/store/tests.rs:346,399` explicitly require a miss. |
| Can a partial memory hit reduce a disk request? | No. `feuer/src/cache.rs:389-405` receives only hit/miss from memory and passes the entire original request to disk after a miss. It has no partial-coverage result or gap list. |
| Can disk assemble multiple entries? | No. `feuer-storage/src/disk_cache.rs:622-626` requires one covering entry. `feuer-storage/src/disk_cache/tests.rs:947` demonstrates a miss despite union coverage by overlapping entries. |
| What happens if neither tier has one covering entry? | `feuer/src/cache.rs:413-430` invokes the original callback once and requires one download covering the entire request. It does not ask the callback for missing gaps. Source reuse would require a different callback contract or adapter-level coordination. |
| What survives a disk read? | `feuer/src/cache.rs:405-410` indexes only the requested bytes in memory. Extra verified bytes are not indexed as additional coverage, even if the returned slice retains the larger allocation. |
| Does compaction merge ranges? | No. `feuer-memory/src/store/range_trim.rs:3-16` preserves distinct requested ranges and can leave overlaps or fragments. Its copies and subsequent admission must still be charged. |
| Are overlaps deduplicated on insertion? | Fully contained entries can be replaced, but partial overlaps coexist. Disk has the same one-covering-entry lookup contract. This can duplicate payload bytes without making their union a hit. |

Synthetic example, not a benchmark: memory entries `[0,4)` and `[4,8)` still miss `[0,8)`. With a covering disk entry, the full requested range reaches disk. Without one, the full-range source callback runs. Similarly, memory `[0,6)` does not reduce a disk request `[4,10)` by the two bytes already in memory. This request-wide fallback is separate from disk's whole-entry verification fallback when no smaller covering checksum exists. That verification mechanism is covered by the checksum investigation and is not counted here as a range-assembly saving.

The source callback accepts no gap parameters, and callers receive one contiguous `Bytes`. Fragment assembly would need copying or another representation, allocation accounting, and safe retention of all participating buffers. Partial disk reads additionally need valid integrity coverage for the holes. Changing request shape alone does not guarantee fewer physical reads.

## Quantified range geometry

All sizes below are decimal bytes. These are offline demand measurements, not a cache simulation.

### Footprint and repetition

| Metric | Value |
| --- | ---: |
| Requested bytes with repetition | 2,800,015,397,526 |
| Sum of distinct exact-range lengths | 129,661,511,335 |
| Union of requested bytes by original object | 129,574,886,383 |
| Overlap between distinct exact ranges | **86,624,952**, 0.06681% of distinct exact-range bytes |
| Starts repeating an earlier exact range | 1,482,760, 92.289% of starts |
| Bytes in those repeated exact requests | 2,670,353,886,191, 95.369% of requested bytes |
| Starts with that exact range previously completed successfully in the original capture | 1,466,098 |
| Median / p95 / p99 request length | 40,070 / 8,547,683 / 25,220,276 |

Repeated demand is not repeated disk I/O. Most of it is served by memory in existing replay results. Original completion times also are not synthetic replay completion times. Neither count distinguishes sequential disk rereads from memory eviction, source refetches, buffered hits, or concurrent reads.

### Coverage from differently shaped earlier requests

Exclude the identical requested range, even if it appeared earlier. This exposes opportunities when an exact entry has been evicted but fragments might survive. Prior starts are treated as immediately available forever. An additional check removes **every individually covering range**, not just the identical range, to allow covering entries to have been evicted while fragments survive. It produces the same fragment and partial-overlap counts below. Actual publication/completion and residency can only remove ranges from that remaining candidate set.

| Earlier coverage | Requests | Requested bytes | Already-overlapping logical bytes |
| --- | ---: | ---: | ---: |
| One covering range | 35,670 | 221,052,766 | 221,052,766 |
| Multiple fragments cover, no single range covers | **661** | **47,419,646** | **47,419,646** |
| Partial coverage only | **6,864** | 11,134,952,291 | **322,472,383** |
| No overlap | 1,563,450 | 2,788,611,972,823 | 0 |

One covering range already works if resident, so that row is not a fragment-assembly opportunity. Fragment-covered requests are only **2,207–79,433 bytes** long. The partial-coverage requests include large reads, but overlap is sparse: 322.472 MB out of 11.135 GB requested in that row. Combined partial/fragment logical coverage is **369.892 MB**, only **0.01321%** of all requested bytes. Do not compare this directly with DataFile savings, since backing checksum selection, alignment, and residency are not represented.

An even more permissive check allows differently shaped ranges from anywhere in the prefix, including the future. It finds **40 distinct fragment-covered ranges / 875 requests / 57,530,360 bytes**, plus **381,419,275 partially overlapping bytes**. Excluding every individually covering range again leaves these counts unchanged. This is a geometry ceiling for these demand shapes, not a physical-I/O ceiling. It assumes exact-range sources and compaction retaining observed request ranges. Expanded production downloads can introduce other ranges.

If identical prior ranges are allowed and assumed retained forever, only **one first-seen request of 5,663 bytes** requires fragment assembly. Most fragment opportunities above therefore depend on some previous exact coverage disappearing while the right smaller entries remain. Existing aggregate metrics cannot establish that state.

### Broader promotion of already-fetched bytes

The previous independent [promotion analysis](../partial-read-checksums/static-analysis/promotion.md) tested indexing all verified bytes only when request-only shrinking already retains the same allocation. It found **1,157 later requests / 9,173,470 requested bytes** of conditional additional coverage, all productive geometries using 32-KiB allocations. No large-read geometry qualifies with successful shrinking. This is a low-priority opportunity for this input. Keeping whole large reads instead would be a retention-policy change with extra memory cost, not free reuse. These counts may overlap the fragment counts and must not be added.

## Capture and replay do not model the same source coverage

| Recorded first-prefix outcomes | Requests | Requested bytes |
| --- | ---: | ---: |
| Memory | 1,475,641 | 2,664,840,004,580 |
| Disk/buffered | 7,618 | 14,073,754,242 |
| Successful callback | 114,080 | 116,298,929,905 |
| Callback error | 8,465 | 708,868,148 |
| Unmatched start | 841 | Not classified as a miss |

Of successful callbacks:

- **87,186 exact downloads:** 111,760,043,366 requested/downloaded bytes.
- **26,894 expanded downloads:** 4,538,886,539 requested bytes, **23,447,476,484 reported downloaded bytes**, a **5.166x** ratio.
- The expanded callback records contain **18,908,589,945 additional bytes** that an exact-range callback for those same requests would omit.
- Total reported callback downloaded bytes are 135,207,519,850. Their distinct downloaded-byte union is 117,077,683,466. These are per-callback reported ranges, **not actual network bytes**. Shared downloads and source operation identities are absent.

`feuer-replay/src/lib.rs:79-90,159-181` retains starts and outcome counts, not the downloaded ranges as replay-source behavior. `feuer-replay/src/bin/replay.rs:115-140` allocates exactly `request.range.len()` independently on each synthetic miss. Consequences:

1. Expansion affects both coverage and admission. In current cache code, expanded source downloads reach disk through later compaction, while exact source downloads can be queued immediately (`feuer/src/cache.rs:435-445`). Exact-range replay changes the population and timing of disk entries, not just source-byte accounting.
2. Original memory/disk hits that become replay misses have no recorded callback download range. Reusing recorded successful download ranges alone cannot reconstruct the original source policy. Initial cache contents, shared allocation identities/capacities, adapter merging, retries and cancellations are also missing.
3. `--full-objects` derives one identity per exact range. It removes covering/overlapping reuse and changes demand-history grouping. It is not native whole-file retrieval and is not a controlled test of fragment assembly.
4. Every selected start is replayed to successful completion, including 8,465 original callback errors and 841 unmatched starts. Their replay effects must not be labeled reproduction of original hit/miss behavior.
5. Replay zero-fills source payloads and checks returned length (`feuer-replay/src/bin/replay.rs:132-133,367`). It measures real I/O under its model, but cannot validate fragment offset/content correctness. Any implementation needs focused nonuniform-payload correctness tests.

These limitations can either increase or decrease replay disk reads. The 18.909-GB expansion difference is **not a forecast of disk or network savings**.

## Existing measured I/O context

Use the complete immediate-source candidate from [the earlier 10-minute comparison](../partial-read-checksums/REPLAY.md), **not current working-tree performance**. Revision is `033284d264062e438010784cb6a2118568130786` plus that experiment's saved candidate patch. Binary/patch hashes are in `settings.json`. That run used 100-GB memory, 500-GB disk, 64 shards, pooled exact-range downloads, matched jemalloc, and completed all requests exactly once. No new testing was performed on its historical host.

| Captured metric | Historical candidate |
| --- | ---: |
| Logical returned bytes | 2,800,015,397,526 |
| Memory / disk-buffer / callback requests | 1,444,487 / 32,816 / 129,342 |
| Memory / disk-buffer / callback returned bytes | 2,593,362,497,232 / 66,339,901,087 / 140,312,999,207 |
| Source GETs / source bytes | 129,342 / 140,312,999,207 |
| Public request / byte hit rate | 91.9496% / 94.9888% |
| DataFile read bytes / calls | **132,241,798,575 / 31,054** |
| Process read bytes, OS inputs × 512 | 132,306,399,232 |
| Device read bytes / requests | 132,306,399,232 / 1,025,778 |
| DataFile write bytes / calls | 104,333,668,352 / 32,091 |
| CPU during requests / average cores | 275 s / 0.4588 |
| OS user / system CPU | 44.01 s / 251.29 s |
| Sampled peak RSS / OS max RSS | 112,087,470,080 / 112,159,191,040 bytes |
| Mean / p99 lookup service | 3.166 / 12.385 ms |
| Mean / p99 scheduled-to-completion | 6.455 / 117.507 ms |
| Execution / backlog drain | 599.372 / 0.012 s |
| Memory eviction insertions / disk eviction insertions | 13,563 / 0 |
| No-capacity outcomes / queue-full write skips | 0 / 26,625 |
| Tail reads / tail writes | 0 / 67,403,776 bytes |
| Pending payload after tail / I/O errors | 0 / 0 |

DataFile bytes are API-requested bytes, not device traffic. Disk-tier returned bytes include buffered hits. Phase deltas exclude opening, OS totals include startup/final reporting, device totals may include other processes, sampled peaks may miss spikes. The approximate 2:1 DataFile/disk-return ratio does not identify fragment misses or sequential rereads.

The full-trace range result has a pending-payload validation failure, and the later full-trace scorer matrix was OOM-killed. Neither supplies a valid current-source end-to-end baseline or a measurement of this opportunity. See their existing reports rather than treating those partial costs as comparable completed runs.

## Ranked next actions

1. **Measure residency before implementing range assembly.** In an isolated snapshot, add aggregate counters at memory misses for bytes covered by multiple resident entries, requests fully covered by their union, and partial covered bytes. Join those with the actual lower-tier outcome and DataFile selected read length. Distinguish first disk fetches from later sequential rereads using anonymized object/range events and read completion order. Existing metrics cannot attribute either. Keep source bytes, latency, CPU and allocation charge alongside disk-byte totals.
2. **If measured, try memory-only full-coverage assembly first.** It can eliminate a whole lower-tier operation without changing source semantics. This prefix offers only 661 optimistic request candidates and at most 47.420 MB of chronological fragment-covered demand, so copy/allocation/lookup costs may outweigh the benefit. Validate exactly-once completion and nonuniform payload correctness before a paced comparison.
3. **Defer partial-memory plus disk-hole reads.** Only 322.472 MB of earlier cross-shape logical overlap is exposed here. It needs extra lookups, assembly allocation, and integrity-compatible missing-range reads. Fetching the whole original range from source instead merely shifts traffic and is not an unconditional disk-saving improvement.
4. **Fix the measurement gap before production extrapolation.** Record actual source-operation range/identity and shared allocation capacity, or replay a documented adapter source policy as an explicitly different scenario. Do not silently substitute recorded outcome timing or downloaded ranges for independent cache behavior.

If a candidate is justified after instrumentation, follow the timed replay procedure for one fresh-cache first-600-second baseline/candidate pair with **100000000000 memory bytes, 500000000000 disk bytes, 64 shards**, matched allocator, unchanged arrival timing and source model. Capture all metrics required by `experiments/storage/timed-replay.md`. There is currently **no measured disk-byte saving for fragment reuse or partial-memory reuse**.

## Reproduction and artifacts

Run from the repository root after restoring the local-only capture and decoded request cache. A clean checkout does not contain these inputs.

```sh
mkdir -p experiments/cache/read-volume-ranges/raw
PYTHONDONTWRITEBYTECODE=1 python3 experiments/cache/read-volume-ranges/analyze.py \
  experiments/cache/cost-scorer-retest-20261012T1324/raw/input/trace.tar.zst \
  experiments/cache/read-volume-ranges/raw/trace-prefix.json
PYTHONDONTWRITEBYTECODE=1 python3 experiments/cache/read-volume-ranges/validate.py
```

Requires Python, NumPy, and `zstd`. `validate.py` also requires `experiments/cache/partial-read-checksums/raw/static-analysis/promotion/requests.npy`, an independently decoded cache of this prefix. `raw/` contains aggregate results, execution/validation logs and source provenance and is ignored by Git.
