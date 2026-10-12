# Recovery: wall time, CPU, scheduler waits and physical I/O

Profiled an isolated snapshot of `d64d246` plus the pending working-tree changes
on the **idle** `m8g-32cpu-local-ssd-2` host. Working-tree patch SHA-256:
`5fd38e6ffacbbf17999dfb4eda24472973c00ff9009be4e571c6febdbf311ff8`.
This is not the earlier rebench snapshot with its temporary payload-lock changes.
No production sources in the local checkout were modified for profiling.

Same fixture: **80,778 entries, 99,971,787,650 actually written payload bytes,
64 shards, 64 metadata chunks**, 256-GiB capacity. Payload contents are synthetic;
only their size distribution comes from recorded source-download frequencies.

## Where elapsed time goes

The 50 scheduler/I/O-traced opens average **16.564 ms** wall time, median
**16.375 ms**, range **10.112–22.158 ms**. Aggregate process CPU averages
**139.475 ms** per open, across all threads.

For each open, follow initialization and then the **last-finishing shard**.
These successive intervals form a non-overlapping wall-time route, unlike
summing durations across all 64 shards:

| Interval on that route | Mean wall | Share of open |
| --- | ---: | ---: |
| Construct 64 shards, including their 64 MiB of zeroed write buffers | **7.395 ms** | **44.6%** |
| Last shard's metadata-read API call | **6.177 ms** | **37.3%** |
| Rebuild metadata positions and entry indexes on that shard | **1.556 ms** | **9.4%** |
| Dispatch and wait for that shard to start | 0.478 ms | 2.9% |
| Queue file-opening task and construct file/I/O rings | 0.411 ms | 2.5% |
| Opening aligned-read probe | 0.243 ms | 1.5% |
| Copy, validate and repair that shard's metadata | 0.230 ms | 1.4% |
| Remaining wakeup, bookkeeping and join | 0.076 ms | 0.5% |

Route boundaries differ from Binggan's timer by about 2 microseconds on average.
This is an observed route through the last task, not a complete dependency graph
of all work performed while it awaits the shared read queue.

**The largest serial cost is before recovery tasks are dispatched.** The
opening thread allocates each shard's `BufferedSmallEntryChunk`, whose constructor
calls `AlignedBuffer::allocate_zeroed(1 MiB)`. The shard-construction interval
has 7.396 ms of thread CPU and no observed scheduler sleep or runnable wait:
it is CPU work, not disk waiting.

[Timeline of a representative open](timeline.svg): orange is initialization,
blue is the read API, purple is rebuilding records. Shard rows overlap and
must not be added.

## What the read wait actually means

Scheduler switches and wakeups split the last shard's **6.177-ms read call**:

| State | Mean |
| --- | ---: |
| Executing on a CPU | **0.196 ms** |
| Sleeping awaiting the asynchronous result | **5.377 ms** |
| Runnable but not yet scheduled | **0.604 ms** |

The independently measured thread CPU delta is 0.197 ms. No unknown scheduler
state remains in these intervals. The dominant sleeping stack is:

```text
futex_wait
  std::sync::Condvar::wait
    tokio::runtime::park::Inner::park
      CachedParkThread::block_on<DiskCacheInner::recover_shard>
```

This is the blocking shard thread parking while its asynchronous read future
waits, **not evidence that an application mutex is contended**. On the last
shard, record/index rebuilding has 1.556 ms CPU and essentially no off-CPU wait.
The phase-marker mutex contributed about 0.0023 ms of observed sleep per open
on this route.

### Separate the API, kernel request and SSD

| Layer | Mean | p95 |
| --- | ---: | ---: |
| Metadata read API, all 3,200 traced shard reads | 4.443 ms | 6.127 ms |
| io_uring submission to completion, those same reads | **1.859 ms** | **3.578 ms** |
| Physical block request issue to completion | **0.226 ms** | **0.442 ms** |

The first row includes buffer allocation, queueing, completion notification and
rescheduling. The second includes kernel/filesystem processing and physical I/O.
These population averages do not identify the exact queueing breakdown for the
single last-finishing read, and must not simply be subtracted from its duration.

Each open requests **64 MiB of metadata plus a 4-KiB probe**, but reads only
**4,460,544 bytes (4.254 MiB)** from the physical device. Both `/proc/self/io`
and matched block requests agree on that byte count. Most metadata pages in
this fixture are unwritten sparse-file holes; the kernel returns zeros for them.
Thus “64-MiB read” does not mean 64 MiB transferred by the SSD.

Per open:

- All metadata io_uring requests span **5.941 ms**, first submission to final completion.
- Physical metadata block requests span **4.401 ms**, including gaps between requests.
- The union of intervals with any benchmark block request outstanding is
  **2.362 ms**, including the opening probe. This overlaps CPU work and is not
  an additive wall-time component or hardware utilization measurement.

All **3,250 io_uring reads** and **6,450 physical block requests** matched their
completions, with no errors or unmatched requests. Two unrelated block requests
were excluded. Block completions were matched even when emitted in interrupt
context rather than by a benchmark thread.

**Conclusion:** the shard does wait for I/O completion, but calling its entire
6.177-ms read interval “SSD waiting” would be wrong. Serial initialization and
the software read pipeline are important; device service alone does not explain
most of the open latency.

## Where aggregate CPU work goes

Measured thread CPU across all 64 shards, not wall-time shares:

| Work | CPU per open | Share of total process CPU |
| --- | ---: | ---: |
| Rebuild records/free positions/entry indexes | **99.778 ms** | **71.5%** |
| Read API callers, including buffer allocation | 8.994 ms | 6.4% |
| Copy metadata into owned chunk storage | 5.553 ms | 4.0% |
| Validate pages and reset invalid pages | 6.515 ms | 4.7% |
| Serial shard construction | 7.396 ms | 5.3% |
| Other: queue thread, runtime, setup and bookkeeping | 11.239 ms | 8.1% |

### Expand the DWARF inline frames

The build already had `debuginfo = 2` (`debug = "full"`, equivalent to `true`)
and `-C force-frame-pointers=yes`, confirmed from Cargo's compiler artifact and
fingerprint. The executable contains `.debug_info`, `.debug_line` and inline
subroutine records. Missing debug information was not the problem.

The initial perf reports used physical symbols and `--no-inline`; even perf's
`--inline` script output on this host did not expand the recorded frames.
Resolving executable instruction addresses directly with `addr2line -a -f -C -i`
exposes the logical inline call chains:

| Function, shortened | Inclusive sampled cycles |
| --- | ---: |
| `MetadataPages::free_entry_metadata` → `BTreeSet::insert` | **62.750%** |
| B-tree `search_tree<EntryMetadataLocation>` | **46.363%** |
| B-tree `search_node<EntryMetadataLocation>` | **42.301%** |
| `EntryMetadataLocation::cmp` | 9.952% |
| `DiskCacheShard::insert_entry` | 5.775% |
| `DiskChunkAllocator::hold_chunks_for_recovered_payload` | 5.174% |
| `decode_entry_metadata` | 0.679% |

These rows overlap: tree/node search and comparison are children of insertion.
The vague 55.60% recovery-loop physical symbol therefore hid the important
finding: **free metadata-position insertion dominates CPU; entry decoding does not**.

Leading expanded self attribution includes metadata-location slice iteration
(`Iter::next`, 18.273%), B-tree `find_key_index` (10.086%) and metadata-location
comparison (7.989%). Source-level attribution in optimized code follows the
compiler's DWARF mappings.

[Expanded self cycles](cpu-inline-self.txt),
[expanded inclusive cycles](cpu-inline-inclusive.txt). These cover the same
25,216 traced-open samples, totaling 16,866,636,329 sampled cycles as the original
perf report. The remote `inline.py` resolves 1,134 unique executable PCs, accounts
for the recorded PIE mapping, and assigns self weight to the innermost frame.
Caller return addresses are moved back one byte to identify their call sites;
inclusive weight is counted once per function per sample. No new recording or
benchmark run was needed.

### Original physical-symbol report

Self sampled CPU cycles for the same 50 traced opens, without inline expansion:

| Function/symbol, shortened | Self cycles |
| --- | ---: |
| Recovery loop inlined into `CachedParkThread::block_on<recover_shard>` | **55.60%** |
| `__aarch64_swp4_rel`, two symbol entries combined | 6.86% |
| B-tree `insert_recursing<EntryMetadataLocation>` | 4.95% |
| `__memcpy_sve` | 4.00% |
| `page_format::encode_page` | 3.48% |
| `__memset_zva64` | 3.05% |
| `__arch_clear_user` | 2.60% |
| `malloc_consolidate` | 2.16% |
| `malloc` | 2.06% |
| `__pi_clear_page` | 1.59% |
| `_int_malloc` | 1.46% |

The first symbol represents **active recovery execution**, not time parked.
Source attribution includes B-tree search (`search.rs:226`, 10.07%) and
metadata-location comparisons (`metadata.rs:13`, 7.99%). The fixture scans
1,370,880 record positions, including **1,290,102 unused positions** inserted
one at a time into a `BTreeSet`. Rebuilding that tree is the prominent CPU target.
This mostly-empty metadata workload is not representative of full metadata
chunks at 40 TB.

CPU cycles by thread group: shard/runtime workers **90.57%**, opening thread
**6.09%**, read-queue thread **3.33%**. The opening thread has a small fraction
of aggregate cycles but a large fraction of wall time because its initialization
is serial. This is why function percentages alone were insufficient.

[Self cycles](cpu-self.txt), [inclusive cycles](cpu-children.txt),
[source attribution](cpu-lines.txt), [thread groups](cpu-threads.txt).
Inclusive percentages overlap. Report whitespace is normalized; values and
symbol names are preserved.

## Measurement controls and artifacts

One process creates the fixture once, then performs 100 opens:

| Mode | Opens | Mean / median wall | Mean process CPU |
| --- | ---: | ---: | ---: |
| Phase clocks only; no active perf events | 10 | 16.305 / 15.316 ms | 153.056 ms |
| CPU sampling plus phase clocks | 40 | 21.230 / 21.350 ms | 145.134 ms |
| CPU, scheduler and I/O recording plus phase clocks | 50 | 16.564 / 16.375 ms | 139.475 ms |

Modes ran sequentially, **not randomized**. Allocator/device state changes across
opens; these differences are not a controlled estimate of profiler overhead.
The pooled Binggan result, 18.405 ms, mixes modes and should not be used as a
replacement benchmark. These are repeated opens after fixture creation, not
cold process startup.

The optimized build retains full debug information and adds frame pointers:
`RUSTFLAGS='--cfg tokio_unstable -C force-frame-pointers=yes'`. CPU recording is
`cycles:uk` at 1,999 Hz. Monotonic timestamps and thread CPU clocks mark phase
boundaries; there are no per-record clock calls. Perf FIFO acknowledgments gate
recording around opens, excluding setup, verification and teardown. Scheduler
switches include call chains; wakeups distinguish sleeping from runnable delay.
52,034 CPU samples were recorded across the 90 sampled opens; CPU and trace
recorders lost no samples.

[Per-open values](opens.csv), [full aggregate measurements](summary.json),
[profiling-only patch](instrumentation.patch). Apply the patch with
`git apply --unidiff-zero` to the recorded snapshot, not an arbitrary current tree.

The exact snapshot, executable, `profile.log`, `cpu.data`, `trace.data`,
`run-profile.sh`, `analyze.py` and `inline.py` remain at
`/home/ubuntu/feuer-recovery-detailed.kTm8FZ`. The analysis pairs io_uring requests
by ring/request/user-data, block requests by device/sector/length, and reconstructs
thread state from switches/wakeups within each gated open.

All 100 timed opens plus the extra untimed reporting open verified successfully.
Ordinary storage tests: **129 passed, 1 ignored**. The test took 68.34 s including
fixture creation and all verification; the fixture was removed, and no benchmark
temporary directories remained on the SSD.
