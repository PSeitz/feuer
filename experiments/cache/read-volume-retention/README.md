# Reducing SSD reads through retention and promotion

## Result

**Retention is a real contributor, not just a hit-rate hypothesis.** One diagnostic replay of the frozen historical tree read **125.009 GB**. Of that, **87.956 GB (70.36%)** repeated a previously read checksum range. The repeated work separates into:

- **38.795 GB / 14,766 reads** of ranges requested exactly, with no different requested slice observed for that checksum range.
- **49.162 GB / 938 reads** of covering ranges that served multiple distinct requested slices.

These are observed repeated transfers, **not achievable savings estimates**. Retaining everything previously read is not feasible at the fixed capacity, and wider retention can displace more valuable entries. No candidate policy was tested in this investigation.

The strongest small implementation opportunity identified here was **recovering backing-capacity slack under memory pressure without changing the retained byte range**. At the inspected revision, compaction compares replacement payload with source payload, so an exact requested range in an oversized pool buffer is ineligible even when copying it would release substantial charged capacity.

**Historical scope:** “Current” below refers to the frozen source described here, not today's checkout. The subsequent [same-range compaction comparison](../same-range-compaction-20261017/README.md) tested this first recommendation. Its measured result is separate from the diagnostic evidence below.

## Measurement and validation

One first-600-second range replay on `benchmark_arm_128g_local_ssd_2`, reached through its Terraform alias `m8g-32cpu-local-ssd-2`. The machine was idle before measurement and had more than the required disk capacity plus 32 GiB headroom. Memory **100000000000**, disk **500000000000**, 64 shards, fresh cache, 32 runtime threads, immediate independent pool-backed downloads, no source coalescing, request cost 256000 in both tiers, request-count half-life 262144, 64 recent events per key, sample size 64, idle ceiling 7%. Capacities were not changed.

Source is HEAD `2f381acdb13e114593e98d1078f29b341279380d` plus the captured existing working tree and untracked source, frozen in `raw/source.tar`. `settings.json` records archive hashes. The isolated instrumentation adds only stderr events for actual payload-read initializers, disk promotions, and sampled memory victims. It does not change scoring or admission. **Logging can change scheduling, so these measurements are diagnostic, not a speed comparison against historical runs.**

- Replay and reporting exit 0. All **1,606,645 requests / 2,800,015,397,526 returned bytes** completed exactly once.
- Zero read/write errors, checksum failures, callback errors, invalid downloads, failed writes, or pending payload after the five-second observation tail.
- The storage suite had one failure, `disk_cache::adversarial_tests::overflowing_fields_preserve_chunk_ownership`, with integer overflow at `feuer-storage/src/disk_cache/recovery.rs:124`. The failure is retained in `raw/library-tests.log`. That recovery code was not edited by this investigation. Successful replay validation does not establish that the storage suite passed.

### Captured workload costs

Decimal GB throughout. DataFile operations are API calls, not kernel/device requests.

| Metric | Diagnostic |
| --- | ---: |
| Logical returned bytes | 2,800.015 GB |
| Memory / disk-buffer / source requests | 1,444,358 / 32,875 / 129,412 |
| Memory / disk-buffer / source returned bytes | 2,593.369 / 66.600 / 140.046 GB |
| Memory request hit rate / byte hit rate | 89.8990% / 92.6198% |
| Combined request hit rate / byte hit rate | 91.9452% / 94.9984% |
| DataFile read bytes / calls | 125.009 GB / 31,212 |
| Process read bytes / device read bytes | 125.142 / 125.142 GB |
| DataFile write bytes / calls | 104.887 GB / 31,805 |
| Source bytes / GETs | 140.046 GB / 129,412 |
| Execution / backlog drain | 599.372 / 0.0123 s |
| Request-phase CPU / mean cores | 199 s / 0.332 |
| Sampled / OS maximum RSS | 107.812 / 108.581 GB |
| Peak cache charge / pending write payload | 100.038 / 9.802 GB |
| Final entry charge / idle buffers | 99.407 / 0.583 GB |
| Peak unstarted arrivals / active lookups | 616 / 1,171 |
| Memory eviction-triggering insertions | 13,186 |
| Disk eviction / no-capacity outcomes | 0 / 0 |
| Queue-full disk write skips | 26,827 |
| Tail read / write bytes | 0 / 0.067424 GB |
| Device read await / mean queue / utilization | 5.217 ms / 8.454 / 7.737% |

Exact latency distributions, in milliseconds:

| Latency | Mean | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sender lateness | 0.0559 | 0.0305 | 0.0533 | 0.8152 | 9.0187 |
| Caller queue | 2.5489 | 0.0462 | 2.5010 | 75.5008 | 495.1845 |
| Service | 2.8529 | 0.0010 | 0.0376 | 12.2483 | 1,405.6935 |
| Scheduled completion | 5.4577 | 0.0796 | 3.4708 | 113.1338 | 1,478.2098 |

Phase counters exclude opening. OS totals include loading/reporting, device totals can include other processes, and sampled peaks can miss spikes. The tail is observation, not forced flush. `raw/results/diagnostic/METRICS.md` retains all metric families, histograms and cold/later windows. `report.json`, raw Prometheus samples, process/device telemetry, allocator/binary/input hashes and commands are adjacent.

## Repeated hits and promotion sizes

Instrumentation agrees **exactly** with DataFile read calls and bytes. There were 15,508 distinct read ranges and 17,967 distinct requested ranges promoted from payload reads. Disk-tier totals also include 90 buffered hits, which are not payload-read promotions.

There were **14,818 repeated exact requested-range promotions**, returning **38.797 GB**. **14,767** of them, returning **38.795 GB**, had a logged eviction of that exact range between promotions. Concurrent requests can still race, and removals due to replacement/compaction were not logged, but this establishes substantial eviction-and-reread behavior rather than treating all disk hits as cold.

Reads larger than 8 MiB account for **105.613 GB (84.48%)** of DataFile bytes, despite only **3,534 / 31,212 calls**. Policies that mainly save small requests can therefore improve request hit rate while worsening read volume.

Cumulative promotions, not simultaneous residency:

| Requested slice | Promotions | Payload GB | Charged GB |
| --- | ---: | ---: | ---: |
| ≤32 KiB | 3,476 | 0.0120 | 0.1139 |
| >32–256 KiB | 14,295 | 1.7332 | 3.7473 |
| >256 KiB–1 MiB | 6,748 | 2.5143 | 4.2121 |
| >1–8 MiB | 5,806 | 15.0965 | 21.2379 |
| >8–64 MiB | 2,460 | 47.2333 | 58.4518 |
| Total | 32,785 | 66.5893 | 87.7631 |

Current shrinking is important: **2,691 promotions** copied into smaller backing buffers. **2,504** changed from a 64-MiB buffer to 32 KiB. Across all shrinking events, the sum of before-minus-after charges was **170.401 GB**. This is cumulative per-caller charge reduction, **not physical memory freed or SSD bytes saved**, because concurrent callers can share one original allocation.

There were **2,609 tiny requests (≤32 KiB)** served from selected ranges larger than 8 MiB. They returned only **7.899 MB**, then charged **85.492 MB** after shrinking. Their selected-range lengths sum to 143.778 GB, but that sum includes shared reads and is **not actual SSD traffic**. Blindly retaining those complete ranges would trade substantial memory pressure for potential coverage. It is not a free promotion optimization.

## Accounting and current scoring

Relevant current code:

- `feuer/src/cache.rs:367–450`: records demand once before tier lookup, promotes exact requested bytes with backing-capacity charge, and suppresses disk writes already covered by disk.
- `feuer-storage/src/disk_cache.rs:332–418`: shares checksum-covered reads, learns partial checksums, shrinks results and returns exact slices. Weak read owners do not retain completed payload indefinitely.
- `feuer-storage/src/file.rs:183–198`: copies only when pool destination saves at least 25% of capacity. It compares **bucket capacity**, not payload length.
- `feuer-memory/src/store/shard.rs:149–228`: per-shard pressure, one-time compaction checks after 64 global requests, then sampled victim selection. All admitted ranges are cached even if low-scoring. The incoming range does not compete against a victim for rejection.
- `feuer-memory/src/store/range_trim.rs:6–18`: byte-saving test uses source **payload length**, not allocation charge. A requested range equal to the source has no plan even when backing capacity is much larger.
- `feuer-memory/src/retention.rs:58–78,113–131`: score is contained decayed caller cost divided by memory allocation charge. All demand counts, including memory hits, survives eviction. The scorer does not know actual disk read ranges or whether a usable disk copy exists.
- `feuer-memory/src/buffer.rs:120–145,318–334`: cached entry charge and idle capacity share a budget. Admission frees idle capacity first. **The 7% setting is a ceiling, not a permanently reserved 7 GB.** Active buffers and caller-only owners are excluded from cache charge. Pool used gauges overlap cached entry charge and must not be added to it.
- `feuer-historian/src/lib.rs:117–136,208–244`: recent events control trimming, while permanent exact-range counters control scoring. Decay age is global request count, not elapsed time. Shortening half-life changes behavior with arrival rate and burst size. It does not make eviction forget a key's history.

The diagnostic logged **66,970 sampled victims**, with **164.307 GB charged / 126.735 GB payload** over all eviction events. Thus **37.572 GB of cumulative victim charge was spare capacity**. Of these, 46,653 victims had at least 25% spare capacity, totaling 21.349 GB spare. These are repeated-event sums, not 37.572 GB concurrently reclaimable. Existing range compaction performed only **116 operations**, releasing **2.131 GB** of charge.

Independent demand geometry finds 123,885 distinct exact ranges totaling **129.662 GB payload / 164.945 GB fresh pool capacity**. The 35.284-GB difference is a workload geometry fact, not resident occupancy. Tiny ≤32-KiB demand is **47.11% of requests but only 0.101% of returned bytes**. Its distinct payload is 0.369 GB versus 2.020 GB fresh capacity. Most absolute spare capacity is in larger ranges, so only adding tiny buckets misses most of this opportunity.

The historical full-trace OOM investigation showed active pool allocations can exceed cache charge. Current read admission differs from that older build. Neither lower idle retention nor falsifying allocation charge fixes caller/write/read ownership. Keep actual capacity charges correct and measure transient copies/RSS when testing compaction.

## Ranked minimal opportunities

### 1. Recover spare backing capacity before evicting useful exact ranges

**Best first experiment.** Extend pressure compaction to consider actual allocation charge and allow a same-range exact-length copy when savings are material. Preserve the complete byte range rather than trimming it merely to justify a copy. Reuse the current copy-outside-lock and publication paths. Do not change accounting without releasing ownership.

Evidence: 37.572 GB cumulative spare victim charge, 21.349 GB in victims with ≥25% spare, 21.174 GB cumulative promotion charge above payload, and 38.795 GB of exact-request rereads. This offers more useful bytes within the same 100-GB target and could keep either source results or disk promotions resident. **No SSD savings were measured by this diagnostic.** The 38.795 GB is observed repeat traffic, not a prediction that all can be eliminated.

Costs: copying and allocation CPU, loss of aligned-buffer reuse, temporary coexistence with source/caller/write owners, and possibly more write-side copying. Avoid eager copying of every result. Current one-time pressure compaction is the smallest place to test this.

### 2. Tune memory retention for bytes, with source traffic as a separate guard

**Smallest configuration-only test**, but no universal winning coefficient is established. Keep disk scorer fixed because this prefix has zero disk eviction. Evaluate modest memory-cost changes against DataFile bytes, not read calls or memory request hit rate. Do not shorten half-life blindly.

Historical complete ten-minute simulated-source results provide a warning: raising memory request cost to 1e9 reduced range read calls **1,099→478**, but increased read bytes **4.045→9.679 GB (+139%)**. Memory request hits improved **67.7226→67.7627%** while memory byte hits fell **80.6481→80.4470%**. A full-object configuration once reduced reads **0.284→0.081 GB**, but only saved 0.203 GB and barely changed source work or overall runtime. These were older exact-length source allocations, not current pool-backed controls. The newer full-trace scoring runs all OOM-killed and cannot establish a winner.

A modeled source-fallback-aware policy also historically reduced source bytes by 0.277 GB while increasing SSD reads by 1.229 GB. This can be a legitimate overall-cost tradeoff, but it is the opposite of an unconditional SSD-byte win.

### 3. Selectively retain covering payloads only when additional slice demand justifies their charge

Potential target: **49.162 GB** of repeated reads of ranges serving multiple distinct requests. Preserving the original covering memory entry or promoting more verified bytes can avoid future first reads of other slices. However, returning and charging the entire covering range requires an explicit interface/policy change, so this ranks behind charge recovery and existing configuration controls.

Do not apply this to every tiny disk hit. Current shrinking takes many 64-MiB buffers down to 32 KiB. A prior offline no-extra-allocation promotion analysis found only **1,157 later requests / 9.173 MB**, all in 32-KiB geometries, and no large-read cases. That cheap variant has little demonstrated scope here. A larger-range policy needs measured reuse distance and competition with other entries, not just a covering-range length.

Do not score a small slice forever using its first large read cost: after learning a checksum, its next disk read may be only that slice. A scorer using actual fallback cost must reflect the current read plan without acquiring disk locks under memory shard locks.

### 4. Admit completed source results to memory even if disk became covered while downloading

`cache.rs:437–442` skips normal memory insertion when disk now covers the completed download, except for explicit disk bypass. Separating memory admission from redundant disk-write suppression is a small possible change that could prevent later disk reads. This diagnostic saw only **291 already-covered source outcomes**, so it is lower priority for immediate sources. Delayed-source captures see far more, but no matched benefit has been measured. Re-admission can displace hotter entries, so measure bytes and source callbacks rather than assuming a win.

### Not recommended for this objective

- Reducing idle ceiling to pretend it frees a fixed 7 GB. Entries already displace idle buffers, and final idle was only 0.583 GB.
- Skipping promotions or disk lookups merely to reduce read counts. This can increase repeat reads or source traffic.
- Whole-range promotion without capacity accounting, or charging pool buffers as payload length. Both hide memory pressure rather than improve retention.
- Tuning disk eviction on this prefix. No disk eviction happened.

## Proposed follow-up

The later [same-range compaction report](../same-range-compaction-20261017/README.md) evaluates this proposal. The original experimental design was one matched baseline/candidate pair with the **same frozen current source and allocator**, unchanged 100-GB memory / 500-GB disk / 64 shards / first 600 seconds / immediate pooled source. Candidate changes only pressure compaction for material backing-capacity savings while preserving the same range. Keep arrival scheduling and independent downloads unchanged. Record copied bytes, released charge, transient pool used bytes, source GETs/bytes, exact versus multi-slice repeated reads, DataFile and device bytes, CPU, RSS and all latency distributions. Require exact completion and zero settled pending work. An SSD reduction accompanied by added source traffic must be reported as a tradeoff.

## Reproduction and artifacts

Run from the repository root:

```sh
python3 experiments/cache/read-volume-retention/analyze-events.py
python3 experiments/cache/read-volume-retention/analyze-demand.py
```

The first command requires local-only `raw/results/diagnostic/stderr.txt` and writes `raw/event-summary.json`. The second requires NumPy and `experiments/cache/partial-read-checksums/raw/static-analysis/promotion/requests.npy`, and writes `raw/demand-geometry.json`. A clean checkout does not contain these inputs. Raw events contain private identities and remain ignored. Narrative tables contain none.

`instrumentation.patch` and `run-case.sh` preserve the observation changes and runner. The historical invocation was `bash run-case.sh ROOT diagnostic range 256000 256000 262144`. The runner requires `ROOT/bin/diagnostic`, `ROOT/input/trace.tar.zst`, `ROOT/binary-sha256.txt`, `ROOT/source.tar`, `ROOT/instrumentation.patch` and `ROOT/diagnostic/Cargo.lock`. Its exact `report.py` and `report-metrics.py` dependencies are retained in `../partial-read-checksums/reproduce/` and must be copied to `ROOT`. Follow [the timed replay procedure](../../storage/timed-replay.md) for any new run.

Reconstructing the tested tree requires the ignored source archive plus `raw/workspace-member.tar` for the `experiments/memory` workspace member. Apply `instrumentation.patch` before building with `cargo build --locked --release -p feuer-replay --bin replay`. Library tests additionally require `experiments/storage/recovery-sizes.csv`, retained at `raw/recovery-sizes.csv`. The historical remote workspace was `/home/ubuntu/feuer-read-volume-retention` on `m8g-32cpu-local-ssd-2`.

Historical references: `../partial-read-checksums/REPLAY.md`, `../partial-read-checksums/static-analysis/promotion.md`, `../adaptive-scoring-20261012T1831/RESULTS.md`, `../tier-retrieval-cost-20261011/README.md`, `../full-trace-scoring-20261013T1745/RESULTS.md`, `../replay-buffer-allocation/ACCOUNTING.md`.
