# Feuer Tiered Cache Contract

Status: authoritative behavioral contract. Implementation sequencing and current repository status belong in
[`implementation-status.md`](implementation-status.md).

## 1. Purpose

Feuer is an embedded read-through cache for applications that access immutable objects by byte range. It sits
between the application and an authoritative source such as object storage, combining a soft-capacity
in-memory tier with a fixed-capacity, best-effort restart-recoverable local disk tier designed to scale to at
least 30 TiB.

Its purpose is to reduce source requests, lookup latency, and retrieval cost without forcing source-I/O
boundaries onto callers. On a miss, an application callback may coalesce work or prefetch a downloaded range
larger than the request. Feuer indexes that exact interval so later contained requests can reuse it, while
tracking which bytes are actually requested so unused prefetched data does not have to remain in memory. The
broader download may still be retained on disk for future subrange reads.

Feuer is a performance layer, not authoritative storage. The application owns object identity, source access,
and download coordination. Feuer owns cache retention, range lookup, integrity-checked disk reads, and
best-effort recovery.

Its central result contract is:

> Every successful lookup returns one contiguous `bytes::Bytes` containing exactly the requested object bytes.

Feuer may produce false misses, skip or lose disk writes, and lose recently cached data after a crash. It
must never return bytes whose object identity or integrity is uncertain.

## 2. Object and range contract

- `ObjectKey` is a `String` containing the complete identity of an immutable object. Feuer treats it as opaque.
  Hashes may locate candidates, but equality compares the complete key.
- Callers are responsible for making the key distinguish every object version that can have different bytes,
  including across process restarts and application upgrades.
- A requested range is an exact, valid, non-empty half-open object byte range supplied to a lookup.
- A downloaded range is the exact object byte range represented by one callback result.
- Requested and downloaded ranges may have arbitrary endpoints and lengths. Feuer imposes no source
  alignment and exposes no public cache block size.
- Object-length discovery, EOF behavior, source correctness, and mutable-object invalidation are application
  responsibilities.

The first implementation accepts one contiguous `Bytes` per download. Memory capacity follows Foyer's soft
per-shard contract: the configured value is divided among shards as an eviction target. A download larger than
its shard's target empties that shard and remains cached, so retained payload may exceed the configured value.

## 3. Public lookup and download boundary

The download callback is supplied per lookup rather than registered when the cache is constructed.
Conceptually, usage has this shape:

```rust
let source_key = object_key.clone();

let requested_bytes = cache
    .get_or_fetch(object_key, requested_range, move || {
        download_manager.fetch(source_key, requested_range, query_context)
    })
    .await?;
```

The callback returns exactly one `Download`:

```rust
pub struct Download {
    downloaded_start: u64,
    bytes: Bytes,
}
```

On a cache miss, Feuer invokes that lookup's callback. The application download manager owns debouncing,
downloaded-range selection, source work, and source-memory bounds. Feuer does not coordinate or retry
callbacks.

`Download::new` derives the downloaded range as
`downloaded_start..downloaded_start + bytes.len()`, rejecting an empty payload or an end-offset overflow. A
successful callback result is therefore length-consistent by construction. Feuer only needs to verify that the
derived range contains that call's requested range. A callback error affects only that lookup.

If a cached range already contains the returned downloaded range, Feuer discards the redundant download
instead of retaining it or scheduling another disk write. The lookup can still return its requested bytes from
the callback result.

A partially overlapping download may be retained in full. Storing only the physical difference is deferred as
a coupled optimization with multi-range reads. The product contract neither requires nor prohibits
satisfying a lookup by assembling several cached ranges.

## 4. Lookup results and accessed ranges

Every memory hit, disk hit, and successful callback result returns one `Bytes` whose visible length and
contents are exactly the requested object bytes.

A returned `Bytes` may share a larger heap allocation. Feuer does not require a compact allocation for every
subrange result. Caller-held results are outside cache-capacity accounting and may keep shared backing memory
alive after cache eviction.

A disk hit reads and verifies the whole covering entry using its expected checksum, then returns only the
requested bytes. A subrange lookup may therefore read a complete larger download. Packing independent entries
in one allocation chunk does not require reading the neighboring entries. Returned disk results do not retain
disk storage, allocation guards, or file mappings.

Every started request records its exact range once, before checking either cache tier. Failed requests,
invalid downloads, and cancellation after starting still count as demand for the complete cache key. Accessed ranges are independent of the downloaded or cached range that happened to satisfy the lookup.
Policy and compaction may project them onto currently cached ranges. Download insertion, replacement, and
redundant-insertion suppression create no accesses.

When application callbacks share one source download, every started request still contributes its own
accessed range, while Feuer caches at most the downloaded ranges selected by its ordinary containment rules.

## 5. Target workload and retention objective

Policy and implementation choices should reflect the target workload:

- one immutable object does not exceed available RAM in practical deployments.
- data columns are at most about 40 MiB, but can have a wide size distribution.
- dictionary lookups and headers are typically 4 KiB or smaller.
- posting lists vary from about 4 bytes to 4 MiB, the size roughly following a Zipf distribution.

The current 165,435-access sample in
[`benchmarks/access_pattern.ndjson`](benchmarks/access_pattern.ndjson) has a 3.74-KiB median and
1.96-MiB mean requested range. 72.9% of requests are at most 4 KiB, and the maximum is about 67.2 MiB. Under an
uncached exact-request application of the source model below, fixed request latency contributes about 83.0% of
total source time.

On a miss, the source-bounded expanded benchmark looks 5 ms ahead and coalesces same-object ranges separated by
less than the source model's 10,000,000-byte break-even distance. The initiating callback downloads the merged
range before followers replay. Downloads default to whole splits below 8 MiB and exact ranges otherwise. The
whole-split threshold and coalescing distance are environment-controlled.

For target-workload evaluation, source retrieval cost means elapsed source-service time. The default controlled
model is:

```text
source_time = source_GETs * 125 ms + downloaded_bytes / 80 MB/s
```

The constants describe the target S3 path and are benchmark inputs, not public cache configuration or a claim
about every deployment. Benchmarks must also report source GETs and bytes separately so the weighted result
cannot hide a regression in either component.

At equal configured memory targets and disk capacities, the primary comparative outcome is cost-weighted hit
rate. Reports also include actual used memory because either cache may exceed its soft target:

```text
1 - cached_run_source_time / cache_disabled_run_source_time
```

For a controlled cache-engine comparison, callback download ranges and application-side coordination are held
constant. Prefetch and downloaded-range selection are evaluated in a separate end-to-end benchmark that
charges each strategy for its actual source GETs and downloaded bytes.

The retention objective is expected future source or lower-tier retrieval time avoided per retained footprint,
not raw object hit rate. The shared policy values each exact access at the modeled fixed source-request
cost plus its requested bytes, then compares recent retrieval value per retained payload byte. Disk scoring
uses payload length only. Alignment, metadata and chunk overhead still consume physical capacity but do not
enter the score's denominator. Repeated access must
increase retention value, stale evidence must eventually expire, and only the exact requested interval receives
observed-access credit.

Access evidence is held in a standalone RAM object shared by both tiers. Each request records once before
lookup, regardless of its eventual outcome. Disk reads and writes do not record additional events.
Distinct counters survive all cache evictions for the history object's lifetime and have no capacity limit.
It is not persisted: recovered entries start without pre-restart access evidence.

Every admission gets a short, deterministic shard-local grace before compaction. Policy keeps no separate
prefetch-promotion state: bounded exact request evidence drives both retention and compaction. Grace never protects
a cached range from pressure eviction.

Prefetch remains bounded in both tiers, and compaction grace never blocks admission or pressure eviction.

The internal policy must distinguish whether a range has no disk copy, has a queued or active disk write, or is
available on disk so it can value a memory hit that avoids disk differently from one that avoids source I/O.
The MVP does not require online cost measurement or a public policy interface.

Comparative claims use the same workload, capacities, cold-cache start, concurrency, and source model. The
required baseline is native Foyer at a pinned revision and reported tuning. The controlled benchmark reports
cost-weighted, request, and byte hit rates together with useful-payload utilization, fragmentation, metadata
footprint, read and write amplification, cleaning or relocation traffic, throughput, and tail latency.

The performance hypotheses are that integrated containment can reuse a larger downloaded range and exact
range attribution improves frequency-aware retention. The current disk design uses whole-entry checksum
validation and 4-KiB-aligned entry allocations. It does not claim smaller reads or sub-alignment packing than
Foyer. These are hypotheses to isolate, not evidence of superiority. Feuer must not claim to beat Foyer until
the comparison demonstrates the claim without violating correctness or the stated resource guardrails.

## 6. In-memory cache

Feuer has one sharded in-memory cache sharing its allocation-byte target with an aligned buffer pool.

- The configured target is divided among shards. Admission evicts only from the selected shard.
- A download larger than its shard target is admitted after the shard is emptied. One oversized entry can
  therefore make a shard, and aggregate retained allocation charges, exceed the configured target.
- No memory entry is protected merely because it is queued for disk writes.
- Memory pressure does not wait for disk throughput.
- Disk promotions charge their whole backing allocation capacity, even when the cached range is a small slice.
  Callback downloads use payload length when allocation capacity is unknown. Charges are per cached entry.
- One buffer pool per cache instance retains 32 KiB, 256 KiB, 4 MiB, 16 MiB, 32 MiB, and 64 MiB allocations.
  The idle pool is capped at 7% of configured memory capacity by default, shared by all six buckets
  without per-bucket caps or reservations. `FEUER_IDLE_BUFFER_POOL_PERCENT` configures this percentage (0–100; zero disables
  idle retention). Cached entries and idle buffers together share the full memory capacity. Admission frees
  idle buffers first; released buffers never evict entries. Allocations above 64 MiB are not pooled.
- Metadata, allocator overhead, active reads, transient copies, and caller-only results are outside accounting.

In-memory compaction remains an MVP feature. Feuer observes exact ranges from incoming requests and
can replace a cached larger download with smaller cached payloads biased toward observed requests, releasing
unrequested cache memory.

Compaction is pressure-driven. Policy samples at most 64 cached ranges and selects the one with the lowest recent
retrieval value per retained byte, using independent exact-range access counts with a half-life of
8,192 requests across all keys. Standalone history owns every distinct counter for its full
in-process lifetime, independently of cache shards and eviction. Counter metadata has no capacity limit.
For range trimming only, exact events are bounded to 64 per object by default
(`FEUER_MAX_ACCESS_EVENTS_PER_KEY` overrides this) and expire after 262,144 later
requests across all keys by default (`FEUER_MAX_ACCESS_AGE_ACCESSES` overrides this).
Once its grace of 64 requests across all keys expires, that same
victim is trimmed when its observed requests can release at least one quarter of its payload. Otherwise it is
evicted.
Compacted replacements use only observed requests, merge only overlapping or adjacent intervals, preserve gaps,
create no access, and cannot affect lookup results or caller-held slices.

Lookup and access recording must not scan every live shard entry. Victim selection samples a bounded number
of entries. Scoring work depends on the distinct requested ranges each candidate covers.
Copies made outside the metadata lock require exact-range and cached-range generation revalidation.
Reinsertion of the same immutable range may accept an earlier copy.
Access history is only policy input: newer accesses do not invalidate a trimming snapshot.

## 7. Best-effort disk writes

Disk writes are bounded and best-effort, not mandatory.

- A retained download may be scheduled for a disk write.
- The pending-write queue is bounded by both bytes and entry count.
- Under queue pressure, the internal policy may skip or replace a disk-write candidate. This never fails
  an otherwise successful lookup.
- If the exact key and range are no longer cached in memory when a queued write starts, that write is
  canceled or discarded. Reinsertion of the same immutable range allows an earlier write to proceed.
- A write already issued to the operating system may finish after memory eviction or caller cancellation.
  Its `DiskRegion` must remain reserved and protected against conflicting access until the submitted I/O
  completes. Abandoning its result does not stop the write. The task owning the reservation must keep
  awaiting completion rather than being aborted. A region whose completion is unknown after queue failure
  must not be reused. A completed write may publish a disk entry only if its exact key and range are still
  cached in memory and the disk policy still admits it.
- A failed write is logged and never becomes disk-lookup-visible. Its memory entry, if still present, remains
  subject to ordinary memory policy.

The policy may consider pending-write and disk-residency state when choosing victims, but the contract does
not assign fixed weights to those states.

There is no flush API. Disk writes and persistence are best-effort.

Disk capacity is fixed at open time and must be respected. Internal allocation, indexing, disk-region layout,
partial retention, rewriting, checksums, metadata persistence, and submission engines are implementation
details. Any bytes made lookup-visible on disk must still be attributable to an exact object identity and
known downloaded object bytes.

The upper storage layer, not the I/O queue, prevents conflicting reads and writes across physical byte
ranges rounded outward to the I/O alignment. A `DiskRegionReadGuard` prevents overwriting or reusing a disk
region while a read depends on its contents. Multiple reads may hold guards concurrently. A canceled read
whose result is discarded no longer needs unchanged disk contents, but its submitted I/O buffer must still
survive until completion. The I/O layer owns that buffer lifetime.

The current disk design packs explicit batches of variable-length entries into immutable 1-MiB chunks,
grouping smaller entries together. Each chunk's payload, entry metadata and chunk metadata are finalized before
its only write. Later batches cannot append to it or reuse holes left by removed entries. A chunk becomes
reusable only after all entry owners and read guards release it. Partially filled final chunks consume their
full capacity. Payload starts and allocated lengths are rounded to 4 KiB. Small entries consume at least
4 KiB of payload storage plus metadata within their batch's chunks. Large entries span consecutive chunks.
Each entry's payload is one contiguous disk byte range with no metadata gaps. The allocation header and
entry metadata precede its payload; continuation chunks have no headers. Entry metadata stores one payload
address and length plus its checksum. One read guard retains the entire contiguous allocation.
Multiple entries may share a chunk only when each entry's complete payload and metadata fit inside that chunk.
An entry spanning multiple chunks owns those chunks exclusively. Its unused tail cannot hold another entry.
Disk pressure selects individual entries by sampled retrieval value per payload byte, not all owners of a
shared chunk together. Removing an entry may free no whole chunk. If bounded eviction cannot reclaim enough
contiguous capacity, the write is skipped rather than scattered across free chunks. Still-retained neighbors
are not removed merely to empty the chunk.
The allocator must handle the full size distribution, reclaim
fragmented capacity with bounded work and rewrite traffic, remain practical at 1-TiB-plus capacities, and
avoid a cache-wide hot lock. Free-space structures, relocation, and cleaning remain private mechanisms.

## 8. Payload I/O modes

At open time, deployments choose one payload I/O mode:

- `PayloadIoMode::Buffered`, using normal buffered file I/O.
- `PayloadIoMode::Direct`, requesting platform direct or uncached file I/O.

Disk storage is supported on Linux only and requires usable io_uring. Feuer must fail open when io_uring is
unavailable or the requested mode cannot be honored by the filesystem rather than silently changing backends
or using buffered I/O. Direct mode is required on Linux.

Alignment, envelope buffers, and platform-specific APIs remain internal. Both modes produce identical lookup
results. Direct mode does not imply synchronization or durability.

## 9. Integrity

Feuer validates disk-backed bytes before returning them. Corrupt payload, malformed metadata, stale write
completion, key mismatch, hash collision ambiguity, impossible ranges, or any other uncertainty becomes a
cache miss and invalidates as much affected cache state as necessary.

The checksum algorithm, validation granularity, metadata representation, and corruption-repair strategy are
versioned implementation details.

This integrity guarantee covers cache storage and recovery. The application remains responsible for bytes
returned by its download callback.

## 10. Recovery and compatibility

After an ordinary process or machine crash, Feuer recovers any safe subset of previously completed disk
writes. Incomplete, torn, corrupt, or structurally uncertain state is ignored. Recently returned downloads
and skipped, canceled, or unfinished disk writes may disappear.

The persistence and recovery mechanism is an implementation choice. Feuer does not promise stable on-disk
compatibility across arbitrary releases. When opening an unsupported on-disk format, Feuer resets the
non-authoritative cache state and logs the reset instead of failing application startup.

Filesystem or device loss, authoritative-storage durability, and recovery of every acknowledged lookup are
out of scope. The MVP may require exclusive ownership of the cache directory.

## 11. Concurrency

- Lookup, memory retention, disk indexing, write scheduling, and policy operations must avoid a cache-wide hot lock.
- Application callbacks run without Feuer metadata locks held.
- Concurrent callbacks may return identical, containing, or overlapping downloads.
- Same-object publication must prevent duplicate or stale callback/write results from creating ambiguous lookup state.
- Eviction cannot invalidate an already returned `Bytes`.

Specific sharding, allocator, guard, and transaction designs are implementation details.

## 12. Observability

Normal instrumentation must be low-overhead and use the repository's tracing and metrics facilities.

It must make it possible to observe:

- memory hits, disk hits, callback misses, errors, callback invocations, and returned download bytes.
- lookup, callback, and disk-I/O latency.
- memory pressure, victim trimming, compaction, disk-write queue pressure, and eviction.
- useful disk payload, allocation overhead, dead or fragmented capacity, and cleaner or relocation traffic.
- integrity and recovery outcomes.
- skipped, canceled, failed, and completed disk writes.

Actual source GETs and transferred bytes remain application-owned and are instrumented by the comparative
benchmark harness. Callback counts are not assumed to equal source GETs when application coordination shares
work. Normal labels and spans must not include object-key contents or other unbounded-cardinality values. Exact
metric and span inventories are implementation checklists, not product API commitments. An
incompatible-format reset must be logged. It does not require a dedicated metric in the MVP.

## 13. MVP acceptance criteria

The MVP is complete when tests demonstrate that:

- arbitrary valid unaligned requested ranges return one contiguous `Bytes` containing exactly the requested object bytes.
- memory hits, disk hits, and callback results obey the same result contract.
- a miss invokes the callback supplied to that `get_or_fetch` call.
- the callback may use query-local state and an application download manager may debounce and share work across callbacks.
- each callback returns one start offset and non-empty `Bytes`, the downloaded range is derived from them, and Feuer rejects a result that does not cover the requested range.
- callback errors are returned without Feuer performing source retries.
- a callback result already contained by cached data is not inserted or written again.
- every started request records its exact range once before lookup, including failures and cancellation; downloaded-range insertion records nothing.
- controlled policy tests credit only the requested interval, favor repeated reuse, age stale frequency, and
  bound never-requested prefetch.
- a broader memory admission cannot be compacted until 64 requests across all keys have elapsed, while
  pressure may still evict it.
- after that grace, pressure can trim the selected victim to its observed exact requests without separate
  promotion or prefetch-reuse state.
- returned `Bytes` may safely outlive cache eviction.
- shard pressure evicts toward each shard's assigned target, while an oversized download empties its shard and remains cached even when aggregate retained payload exceeds the configured target.
- pressure-driven in-memory compaction can release unrequested cached payload without changing results, and
  normal access and victim selection avoid full scans of all live shard entries.
- disk-write queues remain bounded and queue pressure does not block or fail successful lookups.
- evicted queued writes cannot later publish stale state, while already-active current writes can complete safely.
- failed or uncertain disk writes never become disk hits.
- explicit batches group small entries in immutable 1-MiB chunks with 4-KiB-aligned payload storage.
- shared chunks contain each entry's complete payload and metadata, while multi-chunk entries own their chunks exclusively.
- written chunks are neither modified nor reused until all entry owners and read guards release them.
- allocator stress tests report useful utilization, fragmentation, allocation latency, and rewrite traffic
  across the target size distribution.
- buffered and direct modes return identical requested bytes, and requested direct mode never silently falls back.
- Linux supports direct mode on a capable filesystem, with simultaneous reads and writes through bounded io_uring submission.
- disk hits verify the whole covering entry, return only requested bytes, and do not read neighboring entries.
- corrupted or uncertain disk bytes always miss and are never returned.
- restart recovers a safe useful subset after injected crashes.
- unsupported persistent formats are reset and logged safely.
- disk capacities of at least 1 TiB are representable with bounded internal accounting.
- the memory-only gate runs exact and expanded downloader controls with 1, 4, 16, and 64 shards through
  32 GiB, compares actual retained payload, and reports policy throughput.
- the controlled native-Foyer comparison and separate end-to-end prefetch benchmark produce the metrics
  defined in Section 5.

## 14. Explicitly deferred

- streaming callback output.
- more than one `Download` returned by one callback.
- physical difference-only storage for partially overlapping downloads.
- guaranteed assembly from multiple cached ranges.
- public policy plug-ins or online disk-versus-network cost measurement.
- hard process-RSS guarantees, including callback and caller-held memory.
- online disk-capacity resize.
- native multi-volume placement.
- mutable objects, invalidation, and tombstones.
- multi-process access.
- disk storage on platforms other than Linux.
- stable on-disk compatibility across arbitrary future releases.
- authoritative-storage or per-entry durability guarantees.
