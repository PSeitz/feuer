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
broader download may still be stored on disk for future subrange reads.

Feuer is a performance layer, not authoritative storage. The application owns object identity, source access,
and download coordination. Feuer owns cache retention, range lookup, integrity-checked disk reads, and
best-effort recovery.

Its central result contract is:

> Every successful lookup returns one contiguous `bytes::Bytes` containing exactly the requested object bytes.

Cache admission, retention, disk writes, and recovery are best-effort. Feuer may produce false misses,
skip or lose writes, and recover nothing after a restart. Neither tier's residency requires residency in
the other tier. Best-effort does not relax the result contract: disk bytes must pass their expected checksum
before use, subject to the probabilistic key identity and checksum-collision limitations below.

## 2. Object and range contract

- `ObjectKey` is a `String` identifying immutable content. At the public lookup boundary, Feuer hashes its
  UTF-8 bytes once with seed-zero XXH3-128. Both tiers, access history, and recovery compare only that
  128-bit identity; full keys are not stored or checked for collisions. Keys must not be adversarial.
- Callers are responsible for making the key distinguish every object version that can have different bytes,
  including across process restarts and application upgrades.
- A requested range is an exact, valid half-open object byte range supplied to a lookup; it may be empty.
- A downloaded range is the exact object byte range represented by one callback result.
- Requested and downloaded ranges may have arbitrary endpoints and lengths. Feuer imposes no source
  alignment and exposes no public cache block size.
- Object-length discovery, EOF behavior, source correctness, and mutable-object invalidation are application
  responsibilities.

The first implementation accepts one contiguous `Bytes` per download. Memory capacity follows Foyer's soft
per-shard contract: the configured value is divided among shards as an eviction target. A download larger than
its shard's target empties that shard and remains cached, so cached payload bytes may exceed the configured value.

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
`downloaded_start..downloaded_start + bytes.len()`, rejecting an end-offset overflow. A
successful callback result is therefore length-consistent by construction. Feuer only needs to verify that the
derived range contains that call's requested range. A callback error affects only that lookup.

If a cached range already contains the returned downloaded range, Feuer discards the redundant download
instead of retaining it or scheduling another disk write. The lookup can still return its requested bytes from
the callback result.

A partially overlapping download may be cached in full. Storing only the physical difference is deferred as
a coupled optimization with multi-range reads. The product contract neither requires nor prohibits
satisfying a lookup by assembling several cached ranges.

## 4. Lookup results and accessed ranges

Every memory hit, disk hit, and successful callback result returns one `Bytes` whose visible length and
contents are exactly the requested object bytes.

A returned `Bytes` may share a larger heap allocation. Feuer does not require a compact allocation for every
subrange result. Caller-held results are outside cache-capacity accounting and may keep shared backing memory
alive after cache eviction.

A hit on an entry already written to disk reads and verifies the whole covering entry, then returns only the
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

On a miss, the expanded-download benchmark looks 5 ms ahead and coalesces same-object ranges separated by
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

The retention objective is expected future source or lower-tier retrieval time avoided per byte,
not raw object hit rate. The shared policy values each exact access at the modeled fixed source-request
cost plus its requested bytes, then divides recent retrieval value by the memory allocation charge or disk
payload length. Alignment, metadata and chunk overhead still consume physical disk capacity but do not
enter the score's denominator. Repeated access must
increase retention value, stale evidence must eventually expire, and only the exact requested interval receives
observed-access credit. Requested ranges may overlap. A cached range sums the decayed retrieval credit of every
distinct requested range it fully contains; partial overlap alone receives no credit. Exact repeats update one counter.

Access evidence is held in a standalone RAM object shared by both tiers. Each request records once before
lookup, regardless of its eventual outcome. Disk reads and writes do not record additional events.
Distinct counters survive all cache evictions for the history object's lifetime and have no capacity limit.
It is not persisted: recovered entries start without pre-restart access evidence.

Every admission gets a short, deterministic shard-local grace before compaction. Policy keeps no separate
prefetch-promotion state: exact-range access counts drive retention, while recent request events drive compaction.
Grace never protects a cached range from pressure eviction.

Prefetched bytes count toward the memory target and disk capacity, and compaction grace never blocks admission
or pressure eviction.

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
  therefore make a shard, and aggregate entry allocation charges, exceed the configured target.
- No memory entry is protected merely because it is queued for disk writes.
- Memory pressure does not wait for disk throughput.
- Disk promotions copy a slice into an aligned buffer only if the destination allocation saves at least
  25% of backing capacity; otherwise, or if allocation fails, they retain the original slice. Both cache and caller use the chosen allocation,
  charged at its whole capacity. Whole-entry disk reads and checksums are unchanged.
  Callback downloads use payload length when allocation capacity is unknown. Charges are per cached entry.
- One buffer pool per cache instance retains 32 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 4 MiB,
  8 MiB, 16 MiB, 32 MiB, and 64 MiB allocations.
  The idle pool is capped at 7% of configured memory capacity by default, shared by all buckets
  without per-bucket caps or reservations. `FEUER_IDLE_BUFFER_POOL_PERCENT` configures this percentage (0–100; zero disables
  idle retention). Cached entries and idle buffers together share the full memory capacity. Admission frees
  idle buffers first; released buffers never evict entries. Larger allocations share one
  exact-size bucket; reuse resizes them.
- Metadata, allocator overhead, active reads, transient copies, and caller-only results are outside accounting.

In-memory compaction remains an MVP feature. Feuer observes exact ranges from incoming requests and
can replace a cached larger download with smaller cached payloads biased toward observed requests, releasing
unrequested cache memory.

Compaction is pressure-driven. Policy samples at most 64 cached ranges and selects the one with the lowest recent
retrieval value per charged allocation byte, using independent exact-range access counts with a half-life of
262,144 requests across all keys. Standalone history owns every distinct counter for its full
in-process lifetime, independently of cache shards and eviction. Counter metadata has no capacity limit.
For range trimming only, history keeps at most 64 exact events per object by default
(`FEUER_MAX_ACCESS_EVENTS_PER_KEY` overrides this). Events expire after 262,144 later
requests across all keys by default (`FEUER_MAX_ACCESS_AGE_ACCESSES` overrides this).
Once its grace of 64 requests across all keys expires, that same
victim is trimmed when its observed requests can release at least one quarter of its payload. Otherwise it is
evicted.
Compacted replacements use only observed requests, merge only overlapping or adjacent intervals, preserve gaps,
create no access, and cannot affect lookup results or caller-held slices.

Lookup and access recording must not scan every live shard entry. Victim selection samples at most the
configured number of entries (64 by default). Scoring work depends on the distinct requested ranges each
candidate covers.
Copies made outside the metadata lock require the exact source range to still be cached at publication.
Reinsertion of the same immutable range and changes to neighboring ranges do not invalidate a copy.
Replacement ranges already covered by another entry are skipped.
Access history is only policy input: newer accesses do not invalidate a trimming snapshot.

## 7. Best-effort disk writes

`DiskCache` owns the best-effort write queue and buffer-flush timer; a full queue skips new writes.

- A download admitted to memory may be scheduled for a disk write.
- The pending-write queue holds at most 512 entries. Queued and active payload bytes have no byte limit
  and are outside the memory-cache capacity.
- Under queue pressure, the internal policy may skip or replace a disk-write candidate. This never fails
  an otherwise successful lookup.
- A queued write owns its immutable download and may publish after memory eviction or caller
  cancellation. Publication rechecks disk containment, not memory residency. The detached writer retains
  its current payload reservation while awaiting I/O. An abandoned or failed write may leave late I/O
  that overwrites a reused payload; checksum mismatches become misses. Kernel I/O buffers must still
  remain alive until completion, including after cancellation or abnormal queue failure.
- A failed write is logged and never becomes disk-lookup-visible. Its memory entry, if still present, remains
  subject to ordinary memory policy.

The policy may consider pending-write and disk-residency state when choosing victims, but the contract does
not assign fixed weights to those states.

The tiered cache exposes no flush API. Disk writes and persistence are best-effort.

Disk capacity is fixed at open time and must be respected. Internal allocation, indexing, disk-region layout,
partial retention, rewriting, checksums, metadata persistence, and submission engines are implementation
details. Any bytes made lookup-visible on disk must still be attributable to an exact object identity and
known downloaded object bytes.

Readers copy the payload address and expected checksum under the disk index lock, then read into owned
buffers and validate them before use. Readers hold no disk reservation and do not delay reuse. Concurrent
overwrite may cause a miss; no global read/write serialization or read guard is required. The I/O layer
owns submitted buffer lifetime.

Each shard always owns an initialized 1-MiB buffer for entries with aligned size below 512 KiB, flushing on
full/no-fit or every 60 seconds. Buffering consumes no disk capacity. A flush reserves payload chunks with one
eviction budget; metadata positions are taken only during publication after successful I/O. If metadata capacity
is then insufficient for the whole flush, discard the written payload. Queued writes and explicit batches share that buffer. After a memory miss, lookups may
copy requested bytes from a covering buffered entry without disk I/O. Busy buffers and chunks detached for
flushing may miss until disk publication; no in-flight lookup state is retained. Larger entries write separately. Written chunks and individual holes cannot be appended to. A payload chunk becomes reusable when its last indexed entry is
removed, without waiting for readers or metadata writes. Partially filled chunks consume their full capacity.
Payload addresses and storage lengths are rounded to 4 KiB. Each payload is one contiguous disk byte range
with no metadata gaps. Multi-chunk entries reserve consecutive whole chunks exclusively; their unused tails
cannot hold another entry. Metadata is separate: mutable 1-MiB chunks contain checksummed 4-KiB pages,
with one payload address, object range length, and checksum per entry. The last page links to the next
metadata chunk. Metadata chunks remain reserved while open.
Disk pressure selects individual entries by sampled retrieval value per payload byte, not all owners of a
shared chunk together. Removing an entry may free no whole chunk. If eviction exhausts its attempt or chunk
budget before reclaiming enough contiguous capacity, the write is skipped rather than scattered across free chunks.
Neighboring entries are not removed merely to empty the chunk.
Fragmentation may cause a write to be skipped despite sufficient total free space. Relocation and cleaning
are not required for admission; free-space structures remain private mechanisms.

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

Feuer verifies the whole covering payload against its expected checksum before returning disk-backed bytes.
Read failures and checksum mismatches become misses. Recovery validates each metadata page's checksum and
format tag, then relies on the writer's range and alignment contract; it still enforces current shard routing,
bounds, and metadata/payload ownership. Hash collisions and checksum collisions are not detected separately.

The checksum algorithm, validation granularity, metadata representation, and corruption-repair strategy are
versioned implementation details.

This integrity guarantee covers cache storage and recovery. The application remains responsible for bytes
returned by its download callback.

## 10. Recovery and compatibility

Recovery may retain a subset of previously written entries, including none. Invalid record pages are
ignored independently; invalid links terminate a metadata chain. Payloads are verified only when read, so
stale records may recover into the index and subsequently miss. Recently returned downloads and skipped,
canceled, or unfinished writes may disappear.

There is no sync, shutdown flush, minimum recovered hit rate, or stable on-disk compatibility guarantee.
Unsupported metadata page tags are discarded like corrupt pages; older formats are not migrated.

Filesystem or device loss, authoritative-storage durability, and recovery of every acknowledged lookup are
out of scope. The MVP may require exclusive ownership of the cache directory.

## 11. Concurrency

- Lookup, memory retention, disk indexing, write scheduling, and policy operations must avoid a cache-wide hot lock.
- Application callbacks run without Feuer metadata locks held.
- Concurrent callbacks may return identical, containing, or overlapping downloads.
- Containment rules suppress redundant ranges within each tier; publication need not be transactional across tiers.
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
work. Normal labels and spans must not include object-key contents or other values whose distinct count grows
with the workload. Exact metric and span inventories are implementation checklists, not product API commitments. An
incompatible-format page is treated as discarded cache state, not an application error.

## 13. MVP acceptance criteria

The MVP is complete when tests demonstrate that:

- arbitrary valid unaligned requested ranges return one contiguous `Bytes` containing exactly the requested object bytes.
- memory hits, disk hits, and callback results obey the same result contract.
- a miss invokes the callback supplied to that `get_or_fetch` call.
- the callback may use query-local state and an application download manager may debounce and share work across callbacks.
- each callback returns one start offset and `Bytes`, the downloaded range is derived from them, and Feuer rejects a result that does not cover the requested range.
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
- shard pressure evicts toward each shard's assigned target, while an oversized download empties its shard and remains cached even when aggregate cached payload bytes exceed the configured target.
- pressure-driven in-memory compaction can release unrequested cached payload without changing results, and
  normal access and victim selection avoid full scans of all live shard entries.
- the disk-write queue enforces its entry-count limit; queue pressure does not block or fail successful lookups.
- queued and active writes may publish independently of memory residency.
- failed payload writes do not publish live entries; recovered payloads must pass checksum verification.
- small entries share immutable 1-MiB chunks with 4-KiB-aligned storage; partial chunks flush every minute.
- shared payload chunks contain each entry's complete aligned payload; multi-chunk entries own consecutive
  chunks exclusively, with metadata stored separately.
- payload chunks are reused after their last entry is removed; concurrent reads validate checksums instead
  of delaying reuse.
- allocator stress tests report useful utilization, fragmentation, allocation latency, and rewrite traffic
  across the target size distribution.
- buffered and direct modes return identical requested bytes, and requested direct mode never silently falls back.
- Linux supports direct mode on a capable filesystem, with simultaneous reads and writes through io_uring queues
  that enforce waiting-request and active-request limits.
- disk hits verify the whole covering entry, return only requested bytes, and do not read neighboring entries.
- corrupted or uncertain disk bytes always miss and are never returned.
- restart may recover any subset, including none; corrupt recovered payloads become misses.
- unsupported metadata page tags are discarded without requiring migration or a global cache reset.
- disk capacities of at least 1 TiB are representable with limits on memory used by free-space accounting and entry indexes.
- the memory-only gate runs exact and expanded downloader controls with 1, 4, 16, and 64 shards through
  32 GiB, compares actual cached payload bytes, and reports policy throughput.
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
