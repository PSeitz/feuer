# Concurrent SSD read duplication

## Conclusion

**Residual concurrent duplicate reads were not material in the tested 10-minute replay. Do not add broader read coalescing for this capture.** Current code already shares reads before I/O admission, not just after submission.

One instrumented replay on `benchmark_arm_128g_local_ssd_3` completed all **1,606,645 requests**, returning **2,800,015,397,526 bytes**. It issued **31,306 payload DataFile reads / 125,459,276,323 logical read bytes**.

- **61 reads overlapped another active read**, but their actual same-object byte intersections totaled only **468,198 bytes**, **0.000373%** of DataFile bytes. The reads themselves totaled 3.101 GB. **That 3.101 GB is not duplicate data** and cannot be eliminated by skipping those reads.
- Including 4-KiB alignment, intersections across all live physical read ranges totaled **765,952 bytes**, **0.000610%** of aligned payload reads. This is an overlap inventory, not a demonstrated implementable saving.
- Only **4 new reads** could have served their initiating request from another already-active covering read. Their complete logical size was **12,062 bytes**, **0.00000961%** of read bytes. Those covering reads completed **95–370 ms later** than the small reads. Joining would incur that extra wait if the other timings stayed unchanged.
- **Zero** selections or actual read starts found the requested bytes available in the observational memory recheck. **Zero** new reads began while an earlier disk result covering their request awaited memory publication.
- **Zero** read initializer restarts, unmatched read completions, I/O short completions, EINTR completions, or submission EAGAIN events. The maintained replay does not cancel successful lookup tasks, simulate source failures, or retry source callbacks.

These are measurements of the historical snapshot below with diagnostics, not a candidate speed comparison. “Current” below refers to that snapshot, not today's checkout.

## What was tested

HEAD **`2f381acdb13e114593e98d1078f29b341279380d`**, plus the actual preexisting working tree, copied into an isolated source snapshot. This includes current writer-prepared and read-learned subrange checksums, read sharing, read admission, and bypass code. The benchmark API supplies a false bypass decision.

- First 600 seconds, original paced arrivals, one task per arrival, 32 runtime threads.
- Exactly **100,000,000,000 memory bytes**, **500,000,000,000 disk bytes**, 64 shards, fresh cache.
- Immediate independent exact-range synthetic sources through the existing buffer pool. No full-object transform or source-delay simulation.
- Jemalloc `5.3.0-2build1`, `MALLOC_CONF` unset. Settings and source/input/allocator/binary hashes are retained.
- The SSD had 1.7 TiB available and no other benchmark was running.

The diagnostic patch records read identities, selected/requested ranges, actual initializer starts/completions, buffered hits, memory publication, and low-level retry events. It probes memory after selection and immediately before I/O, but **does not suppress or change reads**. Events remain in memory until after requests and the observation tail. The extra memory shard lookups and event mutex can alter scheduling, especially narrow races. This run cannot prove absence of such races in an uninstrumented process or a production cancellation workload.

Provenance:

- `settings.json`, `instrumentation.patch`, `run-case.sh`, `analyze.py`, `results.json` are small reproducible artifacts.
- `raw/source.tar` is the original current-source snapshot, including untracked Rust files. `raw/source-manifest.json` gives per-file hashes. `raw/current.patch` preserves tracked working-tree changes.
- `raw/source/` and `raw/instrumented-manifest.json` preserve the instrumented source.
- `raw/results/diagnostic/` contains events, all metric samples and phase snapshots, exact latency summary, process/device telemetry, command/environment, report, metric inventory, and witness records. Raw files are ignored by Git.
- Historical remote workspace: `/home/ubuntu/read-volume-duplicates`.

## Current behavior and correctness

### Identical and overlapping lookups already share work

`feuer/src/cache.rs:389–410` checks memory once, awaits disk, then synchronously inserts the requested disk slice into memory. Memory access history is recorded once before lookup, not by the diagnostic probes.

`feuer-storage/src/disk_cache.rs:350–390` selects an entry and obtains a shared read under the shard index lock. `PayloadRead.result.get_or_try_init` surrounds **both payload read admission and I/O**. Thus callers waiting for the same checksum share the admission wait and the read. Shared waiters do not each acquire a payload permit or allocate another complete read buffer.

`DiskEntry::share_covering_read` at `disk_cache.rs:496–520` first reuses a live whole-entry read, otherwise selects the shortest covering checksum and reuses that checksum's live read. This supports nonidentical requested slices of one verified read. It does not combine disjoint checksums or overlapping entries into a new read.

Measured decomposition:

| Disk-tier path | Count |
| --- | ---: |
| Buffered hits with no DataFile read | 100 |
| Callers using a disk-read result | 32,863 |
| Distinct actual reads | 31,306 |
| Additional callers sharing those reads | 1,557 |
| Reads with multiple callers | 490 |
| Maximum callers on one read | 8 |

The extra shared callers requested only **6,251,515 bytes**. Their selected shared-read lengths sum to an 84.074-GB repeated-read equivalent, but that is **not a measured no-sharing counterfactual**. Without sharing, some callers could select a smaller available checksum. Neither requested bytes nor selected whole-read equivalents establish the bytes saved by existing sharing.

Prior validated offline geometry for this exact capture found no three-level containment chain and no request covered by two non-containing candidate entries. See [read-sharing geometry](../partial-read-checksums/static-analysis/read-sharing.md) and [entry-selection geometry](../partial-read-checksums/static-analysis/entry-selection.md). That rules out the proposed shortest-checksum-versus-larger-active-partial-read improvement at selection time. It does not rule out a covering **whole-entry** read appearing after a partial read was selected.

### The four remaining join opportunities are tiny and slower

For each of these four cases, a roughly 3-KB checksum read had already been selected before a later whole-entry read appeared. At the small read's actual start, the whole-entry read was active and covered its request. Both addressed the same underlying payload. Selection and actual initialization/admission order are not identical under concurrent task scheduling.

| Small read bytes | Active covering read bytes | Additional wait until covering read completed |
| ---: | ---: | ---: |
| 2,946 | 52,531,177 | 94.818 ms |
| 3,057 | 60,975,193 | 360.186 ms |
| 3,008 | 57,775,381 | 369.875 ms |
| 3,051 | 62,328,577 | 192.243 ms |

A second whole-read lookup immediately before starting I/O could avoid these reads. It would add synchronization on every initializer for a **12-KB** opportunity here, and the table shows a significant latency tradeoff. Any implementation must release its own payload permit before awaiting another read that may itself need admission. It must preserve per-caller bypass/cancellation behavior and avoid cycles between shared initializers. There is no justification to implement it for this capture.

The other overlaps are only partial intersections. They do not justify dropping the large read. Reusing a few verified bytes while reading/checksumming the rest introduces buffer assembly, checksum-domain, and entry-replacement complexity for less than one MB of observed physical overlap.

### Memory publication races are possible, but saved zero observed reads

The index retains only weak read references. `fetch_from_disk_with_bypass` can drop the last strong shared-read reference before the tier inserts its requested slice. Another task may miss memory in that interval and create a new read. There is no second production memory lookup after waiting for disk admission.

However, both observational memory probes were always negative, no new read overlapped a pending covering promotion, and no new read started within **0.1, 1, 10, or 100 ms** of an earlier completed read covering its request.

Disk-return-to-memory-publication latency was median **1.929 μs**, p95 **2.706 ms**, p99 **7.850 ms**, maximum **90.750 ms**. Ten disk-read promotions were rejected as already covered, plus two buffered-hit promotions. Those are redundant memory insertions, **not evidence of duplicate SSD I/O**.

If a different workload demonstrates this race, the smallest semantic change is to retain the existing shared-read owner until memory insertion finishes. This would extend the lifetime of the full verified read buffer, including during memory compaction, and require an internal API handoff. It must not imply ownership of disk chunks or prevent current eviction/reuse. A memory recheck after admission needs a separate per-caller memory-hit result: placing an arbitrary requested slice into a shared cell expecting a larger checksum range is incorrect. Neither change saved any measured read bytes here.

### Cancellation and retries

`disk_cache.rs:363–390` leaves the shared cell unset when its initializer is canceled or bypasses. Another waiter may initialize it. An ordinary I/O/checksum error is shared with current waiters, who independently fall back to their own source callbacks. Bypass remains local to the caller rather than becoming a shared miss.

`file.rs:47–64`, `uring.rs:219–245`, and `uring.rs:457–594` distinguish cancellation from completion. Dropped callers discard unsent requests and stop completion retries, but submitted kernel I/O continues with its buffers retained. Cancellation followed by another initializer can therefore repeat submitted bytes. **DataFile metrics are recorded only after the awaited operation returns**, so abandoned reads can consume device bytes without appearing as completed DataFile operations. No such restart or incomplete operation occurred here.

Short successful completions advance the remaining byte range. EINTR retries without advancing it. Submission EAGAIN retains the same queued requests. These paths do not intentionally reread a successfully completed prefix. All associated diagnostic counters were zero in this replay.

Making initialization independent of caller cancellation would require a task/owner that keeps buffers, permits and file lifetime alive even when all requesters disappear. It must preserve replacement checks: `references_read` at `disk_cache.rs:523–530` prevents late checksum learning or corruption removal from modifying another entry. Cancellation frequency and submitted-but-abandoned bytes must be measured in a workload that actually cancels before paying that lifetime/cleanup cost. Recorded failed or incomplete starts are not replay cancellations.

## Captured workload metrics

Decimal GB below. This is one diagnostic case, not a baseline/candidate comparison.

| Metric | Observed |
| --- | ---: |
| Logical returned bytes | 2,800.015 GB |
| Memory / disk-buffer / source outcomes | 1,443,994 / 32,963 / 129,688 |
| Memory / disk-buffer / source returned bytes | 2,592.754 / 67.226 / 140.035 GB |
| Public request / byte hit rates | 91.928% / 94.999% |
| DataFile read calls / logical bytes | 31,306 / 125.459 GB |
| Aligned payload read bytes reconstructed from addresses | 125.525 GB |
| Process / whole-SSD read bytes | 125.592 / 125.592 GB |
| DataFile write calls / bytes, including tail | 31,999 / 104.651 GB |
| Whole-SSD write bytes | 104.744 GB |
| Request-phase CPU / average cores | 186 CPU-seconds / 0.310 cores |
| OS user / system time, including startup/reporting | 42.09 / 162.12 seconds |
| Sampled / OS maximum RSS | 107.233 / 108.009 GB |
| Peak cache-accounted / pending-write bytes | 100.028 / 9.758 GB |
| Peak unstarted / active requests | 1,380 / 1,171 |
| Execution / backlog drain | 599.371 seconds / 11.174 ms |
| Memory eviction-triggering insertions | 13,208 |
| Disk eviction / no-capacity outcomes | 0 / 0 |
| Queue-full write admission skips | 26,185 |
| Writes queued / published / already-covered at publication | 100,868 / 100,613 / 255 |
| Read / write errors, failed or canceled writes | 0 |
| Pending payload / queued writes after tail | 0 / 0 |
| Five-second tail read / write bytes | 0 / 0.066355 GB |
| Device read / write await | 5.272 / 0.188 ms |
| Device average queue size / utilization | 8.590 / 7.833% |

Exact request latency distributions, milliseconds:

| Metric | Mean | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sender lateness | 0.04365 | 0.03029 | 0.05325 | 0.06551 | 6.192 |
| Caller queue wait | 2.280 | 0.04308 | 2.431 | 74.307 | 463.582 |
| Service | 2.983 | 0.000992 | 0.02915 | 13.552 | 1,449.937 |
| Scheduled to completion | 5.307 | 0.07612 | 3.485 | 104.987 | 1,527.690 |

Approximate startup/later split, using the nearest one-second sample at 180 seconds:

| Phase | DataFile read GB | DataFile write GB |
| --- | ---: | ---: |
| First 180 seconds | 21.576 | 66.096 |
| After 180 seconds through request completion | 103.883 | 38.489 |
| Five-second tail | 0 | 0.066355 |

First observed memory eviction was at 16.904 seconds. Successful DataFile reads averaged **32.635 ms**, with histogram p50 in **(2.048, 4.096] ms**, p95 in **(131.072, 262.144] ms**, and p99 in **(524.288, 1,048.576] ms**. These are bucket intervals, not exact percentiles. Disk/buffer lookup duration averaged **141.117 ms**, including admission and shared-read waiting. Source allocation/service averaged **0.697 ms**.

Phase counters exclude opening. Process/device totals include opening, input loading and reporting, and device counters can include other processes. Reconstructed aligned bytes describe payload transfers, not device request count. The five-second tail is an observation, not a forced flush. Full histograms, their bucket intervals, per-buffer accounting and cold/later windows are retained in `raw/results/diagnostic/METRICS.md` and `histograms-and-windows.json`.

## Validation and recommendation

- Replay and runner both exited 0. Exactly-once completion, byte totals, source outcomes, zero errors, empty backlog, and tail accounting passed the retained report checks.
- Diagnostic read counts/bytes exactly reconcile with DataFile counters. Buffered hits plus selected readers exactly reconcile with disk-tier outcomes. Every started read completed successfully once.
- The storage suite had one failure, `disk_cache::adversarial_tests::overflowing_fields_preserve_chunk_ownership`, in recovery arithmetic at `recovery.rs:37,124`. That code was unchanged by the diagnostic patch. The complete failure is retained in `raw/build.log`. The replay's successful validation does not establish that the storage suite passed.
- Analyzer synthetic checks passed, including separation of shared reads, memory-publication intervals and buffered hits.

**Ranked decision:** keep current sharing. A second active-whole-read lookup has only a 12,062-byte opportunity and a measured latency cost. More elaborate overlap assembly targets less than one MB. Memory-publication handoff/recheck and cancellation-independent initialization have no measured saving in this replay.

**Next experiment, only if needed:** capture actual initializer cancellation and submitted-piece completion bytes in a cancellation-bearing workload, at unchanged capacities and arrival semantics. Do not replay original failures as guessed cancellations. For this capture, no additional coalescing benchmark is warranted. If another change materially alters read lifetimes, rerun these diagnostics once and require a substantial byte opportunity before implementing a new owner or lookup path.

## Reproduction

The command below requires local-only `raw/results/diagnostic/read-events.csv` and the adjacent `report.json` for telemetry cross-checks. It writes `duplicates.json` beside the events, which can be compared with the retained `results.json`. A clean checkout does not contain the raw events.

To regenerate the diagnostic summary without another replay:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 experiments/cache/read-volume-duplicates/analyze.py \
  experiments/cache/read-volume-duplicates/raw/results/diagnostic/read-events.csv
```

For a new replay, follow [the timed replay procedure](../../storage/timed-replay.md). The historical runner invocation was `bash run-case.sh ROOT diagnostic range 256000 256000 262144`. It requires an instrumented binary at `ROOT/bin/diagnostic`, the capture at `ROOT/input/trace.tar.zst`, and `ROOT/binary-sha256.txt`. It also records `ROOT/source.tar`, `ROOT/instrumentation.patch` and `ROOT/source/Cargo.lock`. The exact `report.py` and `report-metrics.py` dependencies are retained in `../partial-read-checksums/reproduce/` and must be copied to `ROOT`. Apply `instrumentation.patch` to the frozen source snapshot, not the bare base commit, then build with `cargo build --locked --release -p feuer-replay --bin replay`. Reconstructing that historical source requires the ignored source archive and its untracked files.
