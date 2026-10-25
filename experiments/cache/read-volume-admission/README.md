# Disk admission, eviction and bypass: read-volume investigation

## Decision

**Do not enable disk bypass or restrict disk admission solely to lower read-byte counters.** The available replay evidence supports eliminating oversized reads and retaining useful data in memory, not replacing disk hits with downloads. There is no measured disk-pressure problem in the default ten-minute workload on which to justify new disk eviction or admission policy.

This historical review inspected `2f381acdb13e114593e98d1078f29b341279380d` plus the captured working-tree changes. “Current” below refers to that snapshot, not today's checkout. `raw/` preserves the inspected tracked patch, untracked bypass source/tests, file hashes and independently extracted phase counters. The replay results below are **not measurements of that inspected tree**. No new replay was run.

## Current behavior and relevant code

- `feuer/src/cache.rs:349-453`: record demand once, try memory, then disk, then source. A disk hit promotes only the requested bytes to memory with the actual backing-capacity charge.
- `feuer/src/cache.rs:435-452`: source results already covered by disk skip another write. With the new bypass API, bypassed source results still enter memory. Ordinary source results that become disk-covered while the callback is outstanding do **not** enter memory in this branch.
- Exact-range source results enter the disk write queue only when memory insertion succeeds. Expanded downloads are not directly queued. Memory compaction can offer retained ranges to disk through the callback at `feuer/src/cache.rs:136-143`.
- `feuer-storage/src/disk_cache/write_queue.rs:8,45-70`: 512 queued entries, best-effort skip when full. This limits entry count, not backing-buffer bytes. A skipped write is not a dropped fetch.
- `feuer-storage/src/disk_cache/writer.rs:22-40`: disk eviction occurs only when metadata/payload space cannot be reserved. `disk_cache.rs:438-477` samples the existing retention scorer. There is no frequency admission comparison against an incoming entry before filling unused disk.
- `disk_cache.rs:558-583`: a new covering entry removes contained entries. Existing disk coverage rejects subsequent contained writes. This is distinct from capacity eviction and can affect read size even when disk is mostly empty.
- `disk_cache.rs:495-522`: choose a live whole-entry shared read first, otherwise the shortest covering checksum range. `writer.rs:50-80` already prepares recent requested-range checksums from trusted write buffers. `disk_cache.rs:397-421` also learns checksums after verified reads. These reduce bytes without downloading again or writing duplicate small entries.

### Bypass changes are opt-in, not active in replay

`get_or_fetch_by_hash` at `feuer/src/cache.rs:334-342` hardcodes `|| false`. Both replay lookup sites use this method. **The current replay cannot accept a bypass decision**, though it does use the new shared payload-read admission queue.

`feuer-storage/src/uring/read_admission.rs:21-84` admits 64 whole payload reads. At 64 waiting initializers it schedules each unchecked waiter's decision once, even if earlier decisions subsequently relieve pressure. This is separate from io_uring piece concurrency. Memory hits, buffered disk hits, absent entries, already-admitted reads and shared-read waiters do not invoke the decision. Bypass occurs before I/O, retains the disk entry, and returns to the source callback without a disk retry on source failure.

The tests in `experiments/cache/disk-bypass/README.md` validate behavior, not latency or byte savings. The read admission limit can change concurrency, buffer residency and scheduling even with bypass disabled. It is not itself a claim of fewer logical bytes.

**One bypass does not necessarily save one read.** Its shared result remains unset. Another waiter can take over and read the same entry while the bypassed request downloads independently. Conversely, bypassing every initializer can trade one shared read for multiple independent downloads. Count actual DataFile bytes, source bytes/GETs and caller outcomes, not bypass decisions alone.

## Workload evidence

### Most relevant completed ten-minute comparison

Source: `../partial-read-checksums/REPLAY.md` and its raw phase snapshots. This is an earlier range-mode baseline `033284d` versus that revision plus read-learned checksums, before writer-prepared checksums and the current bypass/admission changes. Both use immediate independent pooled sources, decimal 100-GB memory / 500-GB disk, 64 shards, matched jemalloc and unchanged arrivals. Both finish **1,606,645 requests / 2,800,015,397,526 logical returned bytes** exactly once, exit zero, and leave no pending writes after the observation tail.

| Metric | Baseline | Read-learned checksums |
| --- | ---: | ---: |
| DataFile read bytes | 134,292,685,070 | 132,241,798,575 |
| DataFile read calls | 31,322 | 31,054 |
| Device read GB | 134.358 | 132.306 |
| Process read GB, time inputs × 512 | 134.358 | 132.306 |
| DataFile write bytes | 104,557,502,464 | 104,333,668,352 |
| Memory returned GB | 2,593.34 | 2,593.36 |
| Disk/buffer returned GB | 66.3057 | 66.3399 |
| Source bytes, GB | 140.371 | 140.313 |
| Source callbacks | 129,446 | 129,342 |
| Disk/buffer hits | 33,109 | 32,816 |
| Public request hit rate | 91.9431% | 91.9496% |
| Public byte hit rate | 94.9868% | 94.9888% |
| Memory eviction-triggering insertions | 13,453 | 13,563 |
| Disk eviction / no-capacity outcomes | 0 / 0 | 0 / 0 |
| Queue-full disk write skips | 26,693 | 26,625 |
| Covered source-write admissions | 293 | 232 |
| CPU seconds during requests | 272 | 275 |
| Sampled peak RSS, GB | 112.135 | 112.087 |
| Mean / p99 completion latency, ms | 6.654 / 126.291 | 6.455 / 117.507 |
| Maximum completion latency, ms | 1,764.27 | 1,778.23 |

DataFile read reduction is 2.051 GB, 1.53%, without an increase in measured source bytes. It is not all causally attributable to checksums because concurrency changed admissions/hit counts. The remaining read-to-disk/buffer-returned-byte ratio is 1.993, not a pure physical read amplification factor because buffered/shared hits are included in returned bytes. This is evidence for further read-size investigation, **not** a quantified 66-GB removable waste budget.

The candidate's 26,625 queue-full skips are about 20.6% of its 129,342 callbacks. Queue pressure is real, capacity pressure is not. Raising the queue limit could increase writes, retained backing memory and later disk hits/read bytes. Lowering it or requiring a second access might reduce writes but can increase later source misses. Neither is an established read-efficiency improvement.

As an accounting illustration, moving every candidate disk/buffer return to source with otherwise fixed outcomes adds 66.340 GB and 32,816 downloads, roughly **47.3% more source bytes and 25.4% more callbacks**. This is not a bypass forecast: buffered hits never bypass, shared reads complicate attribution and cache state would change. It illustrates why a lower read counter alone is inadequate.

### Slow source and longer captures do not justify bypass or disk scoring

- The historical simulated-source ten-minute range baseline reads 4.045 GB, writes 141.565 GB and downloads 537.811 GB in 517,481 callbacks, with 716 seconds of backlog drain. Its full-object counterpart reads only 0.284 GB. This differs from the immediate pooled-source run above in source model/revision. It is not a policy comparison. Source delay can suppress disk reuse while dramatically increasing independent queued downloads.
- The historical full trace returns 31.592 TB in both modes. Range captures 1.883 TB of DataFile reads against 602.542 GB of disk/buffer returns, and downloads 5.484 TB. Source waiting averages 3,912 seconds, with 3.390 million peak waiters and only 48 source slots. Sending more hits to that queue has no demonstrated latency case.
- Disk capacity eviction first appears at 2,614.508 seconds for range and 3,563.641 seconds for full objects. Only 15 range eviction-triggering insertions occur by the nearest last-arrival sample, and zero for full objects. Final totals of 25,222 / 23,200 mostly describe source backlog drain, not sustained demand pressure. Range still has 8,191,403 pending payload bytes after the tail, so complete range write/drain costs are not validated.
- The later pooled-source full-trace scoring wave OOMs in all 16 cases and records zero disk eviction before termination. It cannot establish improvements from partial, unequal work. See `../full-trace-scoring-20261013T1745/RESULTS.md`.
- The older adaptive-scoring investigation explicitly identifies disk coefficients as unidentifiable because disk never evicts. Its full-object disk-read reduction mostly moves hits to memory, with negligible source change. That is a genuine read-removal mechanism, but its exploratory memory settings and historical source model are not validated current-tree disk policy recommendations.

## Prioritized recommendations and tradeoffs

### 1. Preserve disk admission and validate the existing read-size work first

Highest evidence, no new policy code. Current writer-prepared checksums can avoid the **first** oversized partial disk read, which read-learned checksums cannot. The focused tests at commit `937efdc` read 3,006 bytes instead of 98,321 or 2,097,169 bytes, with no extra read/write bytes during preparation. This is actual removal of unwanted bytes, not a source substitution. No workload replay was run for this feature, so the old 1.53% result must not be presented as its result.

Tradeoffs: hashing CPU and per-entry checksum memory, plus lookup scanning. No additional source callbacks, payload copies on disk or format change. Reduced verification coverage remains defined by the selected checksum range. Keep integrity checking and the existing shared-read behavior.

Do not add nested duplicate disk entries or refuse all broad entries as the first fix. Those policies either consume more write/capacity budget or lose coverage and cause more downloads. Existing writer checksums address the same mechanism more directly.

### 2. Small source-to-memory admission candidate, only after measuring opportunity

At `feuer/src/cache.rs:437-442`, consider admitting **all already-downloaded, disk-covered results to memory**, rather than only results marked as bypassed. Keep the disk-write skip intact. Memory insertion already discards a covering duplicate without a new demand event (`feuer-memory/src/store.rs:166-195`). This can reuse a buffer whose source cost has already been paid and prevent a future disk read. It introduces no new source callback or direct disk write for that completed request.

This is an **unmeasured hypothesis, not a recommended default change**. Only 232 such completions appear in the earlier immediate-source candidate, an upper count of opportunities rather than useful promotions. The historical full trace has 3.560 million covered admissions, but most queued source work drains after arrivals, when promotions cannot reduce caller reads. Completion-driven admission can evict more useful RAM, increase subsequent source traffic, and indirectly trigger compaction writes. Reintroducing stale source completions en masse may be worse than the existing policy.

Minimal experiment: count disk-covered source completions that find memory absent while arrivals remain active, then compare this one-branch change in an isolated snapshot only if that count is material. No new frequency threshold or configuration is justified yet.

### 3. Bypass only as an explicit latency/source-load tradeoff

Keep replay/default APIs declining bypass. A caller with spare source capacity could choose source if expected source queue-plus-service is shorter than disk wait-plus-read. Immediate synthetic downloads make bypass look artificially cheap. With simulated S3, even an empty source queue costs at least 108.599 ms plus transfer, whereas the older candidate's successful DataFile reads have a 101.573-ms mean and a 4.096–8.192-ms median bucket. Those averages do not decide individual requests, but do reject unconditional bypass as a general latency improvement.

The current callback receives no disk read size or wait estimate, only caller-captured context and the queue-pressure trigger. An amplification-aware policy would need more information or caller knowledge. Do not add that API surface before a workload shows useful opportunities. Any test must report extra source bytes/GETs and queue delay alongside actual saved reads. Source failures currently return errors without falling back to disk, another deliberate tradeoff.

### 4. Defer frequency/size admission filters and disk scorer tuning

No default ten-minute disk eviction or no-capacity event supports them. Filtering one-hit entries can remove writes, but entries never read already contribute **zero payload read bytes**. Filtering entries that would have hit lowers reads by shifting work to source. Increasing write admission may improve source avoidance while increasing SSD read bytes, which can be desirable overall. Judge total work and latency, not minimum disk reads.

Disk eviction scoring cannot alter a run with zero capacity eviction. Broad-range replacement is the distinct exception already addressed by checksum selection. Do not change capacities or replay duration to manufacture pressure.

## Recovery and metadata scope

Recovery follows metadata-only chunks at `feuer-storage/src/disk_cache/recovery.rs:61-69`. Normal admission/eviction uses in-memory indexes and dirty metadata writes, not metadata reads. `feuer-replay/src/bin/replay.rs:321-330` finishes cache opening **before** taking `before.prom`.

Independent extraction from both earlier ten-minute phase snapshots finds exactly **64 reads / 67,108,864 bytes before requests**, one 1-MiB metadata-chunk probe per shard. These are excluded from all phase-delta DataFile totals above. That is only 0.051% of candidate phase reads even if counted. OS/device scopes can include startup, and header handling differs by revision. Recovery is not a meaningful replay-total optimization target here. Persisting learned checksums would be a restart feature, not a fix for these fresh-cache runs.

## Next experiment and validation scope

A fresh pair is justified only for a selected candidate with a material observed opportunity. Follow [the timed replay procedure](../../storage/timed-replay.md), using isolated frozen sources plus saved patch, first 600 seconds, exactly 100000000000 memory bytes / 500000000000 disk bytes, 64 shards, matched allocator, and a fresh cache per case. Preserve independent exact-range pooled sources, arrival times and exactly-once completion. No repetitions or capacity changes.

For the source-promotion candidate, compare immediate-source mode to the same immediate-source baseline. Do not silently add simulated-source delay as a second workload. For a future bypass evaluation, explicitly choose the source model before running and expose a benchmark-only decision path in the isolated snapshot, since the current hash API always declines bypass.

Report logical bytes, caller memory/disk/source outcomes, DataFile calls/bytes, process/device bytes, source GETs/bytes, exact completion latency, source queues, CPU/RSS, memory eviction, write admissions and terminal outcomes, and tail work. Reject incomplete/OOM or nonzero-pending results as complete cost comparisons. Validate unchanged requests/bytes and zero errors. One pair cannot establish a small speedup.

## Reproduction

From the repository root, run `python3 experiments/cache/read-volume-admission/extract-evidence.py`. It requires the retained `before.prom` and `settled.prom` files under `../partial-read-checksums/raw/replay/results/{baseline,candidate}/` and writes `raw/phase-io.json`. Those local-only inputs are not included in a clean checkout. The extraction verifies the exact read/write figures and recovery subtraction, not every historical comparison cited above. `settings.json` records provenance. Existing test results are cited, not represented as tests rerun by this investigation.
