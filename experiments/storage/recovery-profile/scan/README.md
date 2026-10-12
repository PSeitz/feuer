# Recovery with lowest-free-slot scanning — 3b37ccc

Reran the 100-GB benchmark and detailed profile on the idle
`m8g-32cpu-local-ssd-2`, using source revision
`3b37ccc91be7883f5c555dc0199f95cdf3364926`. The only working-tree patch captured
with the snapshot changes benchmark documentation, not production source.

This revision stores `Option<EntryMetadata>` slots, scans from the lowest possible
free position on insertion, and **counts free slots after recovery instead of
linking them**. It also includes newer write-error diagnostics. Compare with the
[previous slot-array/free-list profile](../slots/README.md).

## Unprofiled benchmark

Same fixture: **80,778 entries / 99,971,787,650 actual payload bytes**, 64 shards,
64 metadata chunks, 256-GiB capacity, recorded download-size frequencies and
synthetic `0x5a` contents. Ten timed opens, with creation, verification and teardown
outside timing. Builds finished before either benchmark ran.

| Open/recovery time | Previous free list | Current free-slot scan |
| --- | ---: | ---: |
| Mean | 14.5452 ms | **13.5524 ms** |
| Median | 14.4567 ms | **13.8193 ms** |
| Min / max | 13.3344 / 16.1602 ms | **10.1087 / 14.7891 ms** |

Mean is **6.8% lower**, median **4.4% lower**. This is a historical comparison of
single ten-open runs, not a randomized matched experiment isolating one change.
Do not treat it as an established speedup across workloads.

Actual disk occupation: **110,779,355,136 bytes**. Whole test: **60.73 s**,
**12.36 s user / 6.63 s system CPU**, **4,913,792 KiB peak RSS (4.69 GiB)**.
These resource totals include fixture creation and verification and are not
recovery-only memory/CPU measurements.

[Benchmark output](../../recovery-scan.log),
[OS resource measurements](../../recovery-scan-time.txt).

## Profiling method

Same phase boundaries and recording modes as the previous profile, except the
last phase now counts slots rather than relinking them. Release build with
`CARGO_PROFILE_RELEASE_DEBUG=2` and
`RUSTFLAGS='--cfg tokio_unstable -C force-frame-pointers=yes'`. No allocator preload.
CPU `cycles:uk` sampling at 1,999 Hz with frame-pointer stacks; DWARF inline chains
expanded using `addr2line -i`. FIFO/ack gates exclude setup, verification,
teardown and the reporting open. Scheduler switches/wakeups, io_uring reads and
physical block requests use the monotonic clock.

| Sequential recording mode | Opens | Mean / median wall | Mean process CPU |
| --- | ---: | ---: | ---: |
| Phase clocks only | 10 | 15.057 / 15.166 ms | 129.311 ms |
| CPU samples and clocks | 40 | 19.600 / 19.744 ms | 131.392 ms |
| CPU/scheduler/I/O and clocks | 50 | 14.794 / 14.455 ms | 118.509 ms |

Do not pool modes into a benchmark or infer profiler overhead by subtracting their
times: modes were sequential, with different allocator/device state. These are
repeated opens, not fresh-process or cold-start measurements.

There were **46,316 CPU samples overall**, with **22,549 samples / 9,708,209,138
sampled cycles** for the final 50 traced opens. The tables below use only those
final 50 opens. No lost CPU samples or `PERF_RECORD_LOST` trace events were reported.

## Wall-time route

Follow initialization and each open's last-finishing shard. These successive
intervals do not overlap; work on other shards and physical I/O does. This is an
observed route, not a complete shared-queue dependency graph.

| Phase | Mean wall | Share |
| --- | ---: | ---: |
| Serial construction of 64 shards/write buffers | **6.677 ms** | **45.1%** |
| Last shard's metadata-read API | **5.470 ms** | **37.0%** |
| Dispatch/schedule last shard | 0.994 ms | 6.7% |
| Validate/decode metadata into slot arrays | 0.585 ms | 4.0% |
| Rebuild entries and indexes | 0.326 ms | 2.2% |
| Count free slots | **0.023 ms** | **0.15%** |
| Remaining file setup, probe and bookkeeping | 0.722 ms | 4.9% |

Shard construction is all running time; it still zeroes 64 MiB of write buffers
serially before recovery dispatch. The metadata-read API interval consists of
**0.484 ms running, 4.536 ms sleeping, and 0.450 ms runnable**. Dominant sleeping
stacks park in Tokio while awaiting the read, not an application metadata mutex.
No reported same-thread interval has unknown scheduler state.

## Active CPU

| Work, aggregated over all shards | Previous CPU-ms/open | Current CPU-ms/open |
| --- | ---: | ---: |
| Read API callers, including buffer preparation | 32.079 | 34.163 |
| Validate/decode and construct slot arrays | 40.222 | 44.882 |
| Rebuild entries/indexes and payload ownership | 19.396 | 20.144 |
| Free-slot bookkeeping: relink → count | **7.069** | **1.936** |
| Serial shard construction | 6.865 | 6.678 |
| Other process work | 10.780 | 10.706 |
| Total process CPU | **116.411** | **118.509** |

The targeted free-slot phase is **72.6% cheaper**, but **total process CPU did not
improve** in this recording (+1.8%). Other phases cost more, offsetting that saving.
The counts are from separate runs; this does not establish a causal regression in
those phases. CPU work across parallel shards is not additive wall time.

### Inline-expanded functions

| Function/work | Inclusive sampled cycles |
| --- | ---: |
| Kernel `do_page_fault` | **46.206%** |
| Collect decoded slots into `Box<[Option<EntryMetadata>]>` | **34.313%** |
| `AlignedBuffer::allocate_zeroed` | **27.566%** |
| `DiskCacheShard::insert_entry` | 13.887% |
| `hold_chunks_for_recovered_payload` | 7.257% |
| `MetadataPages::reset_free_slots` | **2.438%** |
| Actual `decode_entry_metadata` | **1.888%** |

Inclusive rows overlap. Page faults below slot writes account for **22.066
percentage points** of all cycles; those below aligned-buffer zeroing account for
**21.613 points**, with 2.526 points elsewhere. These are active CPU costs, not
SSD-service percentages. Kernel self samples represent **59.469%** of all cycles.

Leading self-cycle costs: `__pi_clear_page` **13.874%**, combined atomic swaps
**9.677%**, `folio_batch_move_lru` **6.173%**, `__pte_offset_map_lock` **5.529%**,
`el0_da` **4.586%**, `__arch_clear_user` **4.366%**, remaining `recover_shard`
**4.016%**, and `__memset_zva64` **3.607%**.

**The next targets are unchanged:** serial write-buffer initialization for wall
latency, and slot/buffer initialization and first-touch page faults for CPU.
No production optimization was implemented during this rerun.

## Memory implications

[DWARF layout](slot-layout.txt) confirms `Option<EntryMetadata>` is still
**64 bytes**, just like the previous enum with free-list links. Therefore this
change **does not shrink slot-array storage**: the fixture still has 1,370,880
positions occupying **83.672 MiB**.

The earlier estimate for **4 decimal TB of live payload with the same size
frequencies**, 64 shards and roughly balanced entry counts remains approximately
3.23 million entries, 192 metadata chunks and **251 MiB of decoded slots**. This is
an extrapolation, not a 4-TB test. Entry indexes, write/read buffers and allocator
retention are additional; whole-test peak RSS above is not a recovery-memory
measurement and should not be scaled to 4 TB.

## I/O and validation

All **3,250 io_uring reads** (50 probes + 3,200 metadata reads) and **6,450 benchmark
block requests** matched, with no I/O errors or outstanding requests. Four
unrelated block requests were excluded. Physical reads remain
**4,460,544 bytes/open (4.254 MiB)**, independently confirmed by `/proc/self/io` and
block totals. Logical metadata reads total 64 MiB; sparse holes return zeros.

| I/O measurement | Mean | p95 |
| --- | ---: | ---: |
| Metadata-read API, all shards | 4.384 ms | 5.616 ms |
| Metadata io_uring submit → completion | 1.762 ms | 3.322 ms |
| Physical block issue → completion | **0.228 ms** | **0.444 ms** |

The union of intervals with physical requests outstanding averages **2.273 ms**
per open, overlapping CPU. Metadata first-submit → last kernel completion averages
5.436 ms; first physical issue → last physical completion averages 4.037 ms.
Neither asynchronous API sleep nor these windows are additive wall-time shares.

All ten unprofiled opens, 100 instrumented opens and their reporting opens passed
entry-count and key/range verification. **128 storage tests passed, 1 ignored**,
both in the ordinary debug test run and the instrumented release binary.
Both fixtures and their dedicated temporary parents were removed. Whole profiling
test time including setup, verification and teardown was **68.87 s**.

## Reproduction and artifacts

- Unprofiled source/binary/logs: `/home/ubuntu/feuer-recovery-scan.xTf3Mb`.
- Instrumented source/binary/raw profiles/scripts:
  `/home/ubuntu/feuer-recovery-scan-profile.XLCuzQ`.
- [Per-open measurements](opens.csv), [full aggregates](summary.json),
  [inline summary](inline-summary.json).
- [Inline self](cpu-inline-self.txt), [inline inclusive](cpu-inline-inclusive.txt),
  physical-symbol [self](cpu-self.txt) and [inclusive](cpu-children.txt) reports.
- [Profiling-only patch](instrumentation.patch): apply with
  `git apply --unidiff-zero` to the recorded source snapshot, not arbitrary HEAD.

The unprofiled snapshot's `run.sh` builds both binaries before measuring them;
the profile snapshot retains `run-profile.sh`, `analyze-recovery-scan.sh`,
`analyze.py`, `inline.py`, raw perf data, full phase CSVs and sleep stacks.
Analysis resolves 1,097 executable PCs at recorded PIE base `0xae6b15330000`.
Sampled/saved user PCs are resolved exactly; ordinary caller return addresses are
moved back one byte. Inclusive weight counts each function once per sample;
cycle totals agree with perf's time-filtered report. Optimized attribution follows
compiler DWARF mappings.

SHA-256 provenance:

- Documentation-only working patch:
  `38d7e922d49ac8837aae88b314c8bb5b0efa3af21b4aec3cbc97aa72bcb9ad71`.
- Lockfile: `95cc47f11e13839bf0ee33b395107cd70aa78886a0694266fb40b44dc7de04e8`.
- Instrumentation: `0af8e614c3e32066a4030172e5c0aa9f5bd3b0a46514840cc797815ef26637f5`.
- Instrumented executable: `35b138ca3a030c5d9bde1c5abc1d01e799f0ab4f9c31289786075e89ca8cad5b`.
