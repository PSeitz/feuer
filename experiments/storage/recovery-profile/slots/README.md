# Recovery profile after replacing the free-position B-tree

Profile of the **exact source snapshot used by the 14.5452-ms unprofiled retest**,
not subsequent working-tree edits. The snapshot uses contiguous decoded metadata
slot arrays and a free list linked through unused slots. No production source was
changed for profiling.

**Result:** free-position tree insertion is gone. Relinking free slots costs
7.069 CPU-ms/open. Allocation, first-touch page faults and initialization of the
slot arrays and aligned buffers now dominate active CPU. Serial construction of
64 shards still takes 46.2% of the observed wall-time route.

## Workload and recording

- Idle `m8g-32cpu-local-ssd-2`: ARM64, 32 CPUs, 123 GiB RAM, local NVMe SSD.
- Same synthetic 100-GB fixture: **80,778 entries / 99,971,787,650 payload bytes**,
  64 shards, 64 metadata chunks, 256-GiB capacity. Payload contents are `0x5a`;
  recorded download frequencies provide lengths, not real object contents.
- Release build, `CARGO_PROFILE_RELEASE_DEBUG=2`,
  `RUSTFLAGS='--cfg tokio_unstable -C force-frame-pointers=yes'`; no allocator preload.
- FIFO/ack-gated recorders exclude fixture creation, verification, teardown and
  the reporting open. Four Tokio workers, blocking shard recovery tasks.
- 100 sequential opens: 10 with phase clocks only, 40 with CPU sampling, then
  50 with CPU sampling plus scheduler, io_uring and physical block traces.
- CPU event: `cycles:uk` at 1,999 Hz, frame-pointer stacks, monotonic clock.
  46,683 CPU samples overall; **23,196 samples / 9,932,710,587 sampled cycles**
  for the final 50 traced opens. Reports below use only those final 50 opens.
  No lost CPU samples or `PERF_RECORD_LOST` trace events were reported.
- Scheduler switches and wakeups distinguish running, sleeping and runnable time.
  There are no unknown scheduler intervals in the reported same-thread phases.

| Recording mode | Opens | Mean wall | Median wall | Mean process CPU |
| --- | ---: | ---: | ---: | ---: |
| Phase clocks only | 10 | 14.573 ms | 14.817 ms | 122.373 ms |
| CPU samples and clocks | 40 | 19.494 ms | 19.799 ms | 128.170 ms |
| CPU/scheduler/I/O and clocks | 50 | 14.849 ms | 15.553 ms | 116.411 ms |

Modes ran sequentially, not randomized. Allocator/device state and instrumentation
vary; do not infer profiler overhead from their differences or replace the
**14.5452-ms unprofiled result** with a pooled profiling mean. These are repeated
opens, not fresh-process or cold-device startup measurements.

## Non-overlapping wall-time route

Follow initialization and then each open's last-finishing shard. This is an
observed route, not a complete shared-queue dependency graph. Work on other shards
and SSD requests overlaps these intervals.

| Phase | Mean wall | Share of traced open |
| --- | ---: | ---: |
| Construct 64 shards, including 64 MiB zeroed write buffers | **6.865 ms** | **46.2%** |
| Last shard's metadata-read API call | **5.333 ms** | **35.9%** |
| Dispatch/schedule last shard | 0.859 ms | 5.8% |
| Validate pages and construct decoded metadata slots | 0.628 ms | 4.2% |
| Rebuild records and indexes | 0.313 ms | 2.1% |
| Relink free slots | **0.112 ms** | **0.8%** |
| File queue/setup, probe and remaining bookkeeping | 0.736 ms | 5.0% |

Shard construction is all running time, with 6.865 ms thread CPU and no observed
sleep/runnable delay. It still happens serially before recovery dispatch.

The last shard's 5.333-ms metadata read consists of:

- **0.501 ms running**;
- **4.603 ms sleeping**;
- **0.229 ms runnable**, waiting to be scheduled.

Dominant sleep stack: `futex_wait → Condvar::wait → Tokio Inner::park →
CachedParkThread::block_on<recover_shard>`. This is waiting for an asynchronous
read, not evidence of application-mutex contention. The profiling marker mutex
itself contributes about 0.021 ms sleep/open on the route and is not production
behavior.

## Active CPU: where the work moved

Thread CPU summed across the 64 shards, plus serial initialization:

| Work | CPU-ms/open | Share of process CPU |
| --- | ---: | ---: |
| Validate/decode pages and construct slot arrays | **40.222** | **34.6%** |
| Metadata-read API callers, including buffer preparation | **32.079** | **27.6%** |
| Records, payload chunk ownership and entry indexes | 19.396 | 16.7% |
| Relink free slots | **7.069** | **6.1%** |
| Serial shard construction | 6.865 | 5.9% |
| Remaining process work | 10.780 | 9.3% |
| Total | **116.411** | 100% |

The previous detailed profile measured **139.475 CPU-ms/open**: this profile is
16.5% lower. Its record/free-position/index phase took 99.778 CPU-ms/open; records
plus free-list relinking now take 26.465 CPU-ms/open. However, decoding has moved
into metadata loading, and the loading phases are more expensive. For a broader
comparison, old copy + validation + record reconstruction totaled 111.845 CPU-ms;
new decoding + record reconstruction + relinking totals 66.687 CPU-ms.

These are historical profiles with other revision changes, not a matched isolated
measurement of the data-structure change. Parallel CPU savings do not translate
one-for-one into wall-time savings.

### DWARF-expanded functions

| Function, shortened | Inclusive sampled cycles |
| --- | ---: |
| `do_page_fault` | **42.805%** |
| Collect decoded `MetadataSlot`s into `Box<[MetadataSlot]>` | **31.668%** |
| `AlignedBuffer::allocate_zeroed` | **26.269%** |
| `DiskCacheShard::insert_entry` | 11.608% |
| `MetadataPages::relink_free_slots` → `link_free_slots` | **8.864%** |
| `BufferedSmallEntryChunk::new` | 8.852% |
| `hold_chunks_for_recovered_payload` | 7.806% |
| `decode_entry_metadata` | **2.326%** |

Inclusive rows overlap. In particular, page faults occur underneath slot writes
and buffer zeroing, and small-entry buffer construction calls aligned allocation.
Kernel self samples account for **54.985%** of all sampled cycles.

The 42.805% page-fault branch splits by expanded callers into **19.917 percentage
points writing metadata slots**, **20.874 points zeroing aligned buffers**, and
2.014 points elsewhere. These are active kernel execution, not SSD read waits.
The allocation/first-touch work is substantially larger than the 2.326% attributed
to decoding the 48-byte records themselves. Slot-vector growth/reservation is
about 1.49% inclusive; this profile does not support calling reallocations the
main cost.

DWARF confirms `MetadataSlot` occupies **64 bytes** in this executable. Across
1,370,880 positions, the decoded arrays occupy **87,736,320 bytes (83.672 MiB)**,
including unused positions. Recovery initializes those arrays and then traverses
them for entry reconstruction and free-list linking.

Leading expanded self costs:

| Function | Self sampled cycles |
| --- | ---: |
| `__pi_clear_page` | 12.186% |
| `__aarch64_swp4_rel`, combined | 9.258% |
| `mem::replace<Option<EntryMetadataLocation>>`, in free-list linking | 7.547% |
| `folio_batch_move_lru` | 5.589% |
| `__pte_offset_map_lock` | 5.277% |
| `__arch_clear_user` | 4.130% |
| `el0_da` | 4.022% |
| `recover_shard` remaining self attribution | 3.574% |
| `__memset_zva64` | 3.123% |

The **wall-latency target remains serial write-buffer initialization**. The current
CPU target is memory initialization/first-touch work, particularly constructing
slots for every unused metadata position. No optimization is implemented here.

## Logical reads versus physical SSD service

All **3,250 io_uring reads** (50 probes + 3,200 metadata reads) and **6,450 benchmark
block requests** matched successfully, with no pending requests or I/O errors.
Two unrelated block requests were excluded.

| I/O measurement | Mean | p95 |
| --- | ---: | ---: |
| Metadata read API, all shards | 4.092 ms | 5.581 ms |
| io_uring metadata submit → completion | 1.593 ms | 2.899 ms |
| Physical block issue → completion | **0.220 ms** | **0.436 ms** |

Physical bytes per open remain **4,460,544 (4.254 MiB)** including the probe,
independently confirmed by `/proc/self/io` and block totals. Logical metadata
reads request 64 MiB; the unused pages are sparse-file holes returned as zeros.

Metadata first-submission → last kernel completion averages 5.932 ms; metadata
first-block-issue → last block completion averages 5.157 ms. The union of intervals
with physical requests outstanding, including the probe, averages 2.435 ms/open.
These overlap CPU and each other. Neither the 5.333-ms last-shard API interval nor
its 4.603-ms sleep interval is an SSD-service percentage. Population averages
cannot be subtracted to assign exact queueing time to that shard.

## Artifacts and reproduction

- [Per-open measurements](opens.csv), [complete aggregates](summary.json).
- [Inline-expanded self](cpu-inline-self.txt) and
  [inclusive](cpu-inline-inclusive.txt) cycle attribution;
  [inline summary](inline-summary.json).
- Original physical-symbol [self](cpu-self.txt) and
  [inclusive](cpu-children.txt) reports.
- [Profiling-only instrumentation patch](instrumentation.patch), applied with
  `git apply --unidiff-zero` to the exact recorded source snapshot.

Remote source and raw data: **`/home/ubuntu/feuer-recovery-slots-profile.WSm4bk`**.
This retains `benchmark`, `cpu.data`, `trace.data`, `trace-events.txt`, `profile.log`,
`run-profile.sh`, `build-profile.sh`, `analyze.py`, `inline.py`, full phase CSVs,
sleep stacks, build logs, tests and the source working-tree patch. Run the scripts
there in an equivalent fresh snapshot; the FIFO gates select the recording modes.
The benchmark verified every entry count and key/range after all 100 timed opens
plus the reporting open. Instrumented release binary ordinary tests:
**127 passed, 1 ignored**. The fixture and its dedicated temporary parent were
removed. Setup, verification and teardown brought whole-test time to 68.51 s.

Source base: `b77db4c2b6d3a834e25b99e523825886399bc059`.
Working patch SHA-256:
`0e62cfa0aab382b00364fa4a238d513209102d76b6cb429581f4d894a73cfd86`.
Lockfile SHA-256:
`95cc47f11e13839bf0ee33b395107cd70aa78886a0694266fb40b44dc7de04e8`.
Instrumentation patch SHA-256:
`8eba163804929c97679242f56ec98738ab154e87a366902f0d977795be73e604`.
Executable SHA-256:
`cfb5ca89f7ba157ae259a4a9e109a461d86fb50201ac143dda801d6c916dbc11`.

Inline expansion resolves 1,073 executable PCs using `addr2line -a -f -C -i` and
PIE load address `0xbf328e890000`, validated against the recorded mmap and ELF load
segment. Sampled PCs and saved user PCs below kernel frames are resolved exactly;
ordinary caller return addresses are moved back one byte. Self weight goes to the
innermost frame; inclusive weight counts a function once per sample. Cycle totals
are checked against perf's time-filtered report. Optimized source attribution
follows compiler DWARF mappings and is not an instruction-by-instruction cost model.
