# Feuer Tiered Cache Contract

Status: authoritative behavioral contract. Implementation sequencing and current repository status belong in
[`implementation-status.md`](implementation-status.md).

## 1. Purpose

Feuer is an embedded read-through cache for applications that access immutable objects by byte range. It sits
between the application and an authoritative source such as object storage, combining a soft-capacity
in-memory tier with a fixed-capacity, best-effort restart-recoverable local disk tier designed to scale to at
least 1 TiB.

Its purpose is to reduce source requests, lookup latency, and retrieval cost without forcing source-I/O
boundaries onto callers. On a miss, an application callback may coalesce work or prefetch a downloaded range
larger than the request. Feuer indexes that exact interval so later contained requests can reuse it, while
tracking which bytes are actually requested so unused prefetched data does not have to remain in memory. The
broader download may still be retained on disk for future subrange reads.

Feuer is a performance layer, not authoritative storage. The application owns object identity, source access,
and download coordination; Feuer owns cache retention, range lookup, request-sized disk reads, integrity, and
best-effort recovery.

Its central result contract is:

> Every successful lookup returns one contiguous `bytes::Bytes` containing exactly the requested object bytes.

Feuer may produce false misses, skip or lose disk population, and lose recently cached data after a crash. It
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
downloaded-range selection, source work, and source-memory bounds; Feuer does not coordinate or retry
callbacks.

`Download::new` derives the downloaded range as
`downloaded_start..downloaded_start + bytes.len()`, rejecting an empty payload or an end-offset overflow. A
successful callback result is therefore length-consistent by construction. Feuer only needs to verify that the
derived range contains that call's requested range. A callback error affects only that lookup.

If a cached range already contains the returned downloaded range, Feuer discards the redundant download
instead of retaining it or scheduling another disk write. The lookup can still return its requested bytes from
the callback result.

A partially overlapping download may be retained in full. Storing only the physical difference is deferred as
a coupled optimization with multi-extent reads. The product contract neither requires nor prohibits
satisfying a lookup by assembling several cached extents.

## 4. Lookup results and accessed ranges

Every memory hit, disk hit, and successful callback result returns one `Bytes` whose visible length and
contents are exactly the requested object bytes.

A returned `Bytes` may share a larger heap allocation. Feuer does not require a compact allocation for every
subrange result. Caller-held results are outside cache-capacity accounting and may keep shared backing memory
alive after cache eviction.

A disk hit reads only the requested bytes plus bounded integrity and I/O-alignment overhead; it does not
materialize a complete larger download merely to answer a subrange lookup. Returned disk results do not retain
disk storage, allocation guards, or file mappings.

Every successful lookup appends its exact requested range once to the accessed ranges for its complete cache
key. Accessed ranges are independent of the downloaded or cached range that happened to satisfy the lookup;
policy and compaction may project them onto currently cached extents. Download insertion, replacement, and
redundant-population suppression create no accesses.

When application callbacks share one source download, every successful waiter still contributes its own
accessed range, while Feuer caches at most the downloaded extents selected by its ordinary containment rules.

## 5. Target workload and retention objective

Policy and implementation choices should reflect the target workload:

- one immutable object does not exceed available RAM in practical deployments;
- data columns are at most about 40 MiB, but can have a wide size distribution;
- dictionary lookups and headers are typically 4 KiB or smaller; and
- posting lists vary from about 4 bytes to 4 MiB, the size roughly following a Zipf distribution.

The current 165,435-access sample in
[`benchmarks/access_pattern.ndjson`](benchmarks/access_pattern.ndjson) has a 3.74-KiB median and
1.96-MiB mean requested range; 72.9% of requests are at most 4 KiB, and the maximum is about 67.2 MiB. Under an
uncached exact-request application of the source model below, fixed request latency contributes about 83.0% of
total source time.

On a miss, the source-bounded expanded benchmark looks 5 ms ahead and coalesces same-object ranges separated by
less than the source model's 10,000,000-byte break-even distance. The initiating callback downloads the merged
range before followers replay. Downloads default to whole splits below 8 MiB and exact ranges otherwise; the
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
not raw object hit rate. The memory-only policy values each exact access at the modeled fixed source-request
cost plus its requested bytes, then compares recent retrieval value per retained byte. Repeated access must
increase retention value, stale evidence must eventually expire, and only the exact requested interval receives
observed-access credit.

Every admission gets a short, deterministic shard-local grace before compaction. Policy keeps no separate
prefetch-promotion state: bounded exact request evidence drives both retention and compaction. Grace never pins
an extent against pressure eviction.

Prefetch remains bounded in both tiers, and compaction grace never blocks admission or pressure eviction.

The internal policy must distinguish whether a range has no disk copy, has a queued or active disk write, or is
available on disk so it can value a memory hit that avoids disk differently from one that avoids source I/O.
The MVP does not require online cost measurement or a public policy interface.

Comparative claims use the same workload, capacities, cold-cache start, concurrency, and source model. The
required baseline is native Foyer at a pinned revision and reported tuning. The controlled benchmark reports
cost-weighted, request, and byte hit rates together with useful-payload utilization, fragmentation, metadata
footprint, read and write amplification, cleaning or relocation traffic, throughput, and tail latency.

The performance hypotheses are that integrated containment can reuse a larger downloaded range, request-sized
disk reads avoid Foyer's complete-value load path, sub-alignment packing avoids its block engine's per-entry
page rounding for small values, and exact range attribution improves frequency-aware retention.
These are hypotheses to isolate, not evidence of superiority. Feuer must not claim to beat Foyer until the
comparison demonstrates the claim without violating correctness or the stated resource guardrails.

## 6. In-memory cache

Feuer has one sharded in-memory cache with a soft payload-byte target.

- The configured target is divided among shards. Admission evicts only from the selected shard.
- A download larger than its shard target is admitted after the shard is emptied. One oversized entry can
  therefore make a shard, and aggregate retained payload, exceed the configured target.
- No memory entry is protected merely because it is queued for disk population.
- Memory pressure does not wait for disk throughput.
- Usage is charged to payload bytes retained by the cache. Metadata, allocator overhead, callback-owned source
  buffers, transient copies, and caller-held results are outside that accounting.

In-memory compaction remains an MVP feature. Feuer observes exact accessed ranges from successful lookups and
can replace a cached larger download with smaller cached payloads biased toward observed requests, releasing
unrequested cache memory.

Compaction is pressure-driven. Policy samples at most 64 extents and selects the one with the lowest recent
retrieval value per retained byte. Exact events are bounded to 64 per object and currently expire after 32,768
later successful same-shard accesses. Once its grace of 64 successful same-shard accesses expires, that same
victim is trimmed when its observed requests can release at least one quarter of its payload; otherwise it is
evicted.
Compacted replacements use only observed requests, merge only overlapping or adjacent intervals, preserve gaps,
create no access, and cannot affect lookup results or caller-held slices.

Lookup and access recording must not scan every live shard entry. Victim selection has bounded work and metadata;
copies made outside the metadata lock require generation revalidation.

## 7. Best-effort disk population

Disk population is bounded and best-effort, not mandatory.

- A retained download may be scheduled for disk population.
- The pending-write queue is bounded by both bytes and entry count.
- Under queue pressure, the internal policy may skip or replace a disk-population candidate. This never fails
  an otherwise successful lookup.
- If a memory-cached range is evicted before its queued write starts, that write is canceled or discarded.
- A write already issued to the operating system may finish after memory eviction. It may publish a disk entry
  only if its generation is still current and the disk policy still admits it.
- A failed write is logged and never becomes disk-lookup-visible. Its memory entry, if still present, remains
  subject to ordinary memory policy.

The policy may consider pending-write and disk-residency state when choosing victims, but the contract does
not assign fixed weights to those states.

`flush` waits for disk writes already queued or running. It does not retry writes that were skipped or
canceled and does not provide per-entry durability.

Disk capacity is fixed at open time and must be respected. Internal allocation, indexing, extent layout,
partial retention, rewriting, checksums, metadata persistence, and submission engines are implementation
details. Any bytes made lookup-visible on disk must still be attributable to an exact object identity and
known downloaded object bytes.

Physical I/O alignment must not become a minimum allocation charge for every retained value. Values smaller
than the required I/O alignment must be physically packable so the target population of small ranges can use
disk efficiently. The allocator must handle the full size distribution, reclaim fragmented capacity with
bounded work and rewrite traffic, remain practical at 1-TiB-plus capacities, and avoid a cache-wide hot lock.
Size classes, slabs, regions, extents, free-space structures, relocation, and cleaning algorithms remain
private, benchmark-selected mechanisms.

## 8. Payload I/O modes

At open time, deployments choose one payload I/O mode:

- `PayloadIoMode::Buffered`, using normal buffered file I/O; or
- `PayloadIoMode::Direct`, requesting platform direct or uncached file I/O.

Direct mode is required on Linux and macOS. Feuer must fail open when the requested mode cannot be honored by
the platform or filesystem rather than silently using buffered I/O.

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
population. Incomplete, torn, corrupt, or structurally uncertain state is ignored. Recently returned downloads
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

- memory hits, disk hits, callback misses, errors, callback invocations, and returned download bytes;
- lookup, callback, and disk-I/O latency;
- memory pressure, victim trimming, compaction, disk-write queue pressure, and eviction;
- useful disk payload, allocation overhead, dead or fragmented capacity, and cleaner or relocation traffic;
- integrity and recovery outcomes; and
- skipped, canceled, failed, and completed disk population.

Actual source GETs and transferred bytes remain application-owned and are instrumented by the comparative
benchmark harness; callback counts are not assumed to equal source GETs when application coordination shares
work. Normal labels and spans must not include object-key contents or other unbounded-cardinality values. Exact
metric and span inventories are implementation checklists, not product API commitments. An
incompatible-format reset must be logged; it does not require a dedicated metric in the MVP.

## 13. MVP acceptance criteria

The MVP is complete when tests demonstrate that:

- arbitrary valid unaligned requested ranges return one contiguous `Bytes` containing exactly the requested object bytes;
- memory hits, disk hits, and callback results obey the same result contract;
- a miss invokes the callback supplied to that `get_or_fetch` call;
- the callback may use query-local state and an application download manager may debounce and share work across callbacks;
- each callback returns one start offset and non-empty `Bytes`, the downloaded range is derived from them, and Feuer rejects a result that does not cover the requested range;
- callback errors are returned without Feuer performing source retries;
- a callback result already contained by cached data is not populated or written again;
- every successful lookup appends its exact requested range once to its key's accessed ranges, independent of downloaded-range population;
- controlled policy tests credit only the requested interval, favor repeated reuse, age stale frequency, and
  bound never-requested prefetch;
- a broader memory admission cannot be compacted until 64 successful accesses in its shard have elapsed, while
  pressure may still evict it;
- after that grace, pressure can trim the selected victim to its observed exact requests without separate
  promotion or prefetch-reuse state;
- returned `Bytes` may safely outlive cache eviction;
- shard pressure evicts toward each shard's assigned target, while an oversized download empties its shard and remains cached even when aggregate retained payload exceeds the configured target;
- pressure-driven in-memory compaction can release unrequested cached payload without changing results, and
  normal access and victim selection avoid full scans of all live shard entries;
- disk-write queues remain bounded and queue pressure does not block or fail successful lookups;
- evicted queued writes cannot later publish stale state, while already-active current writes can complete safely;
- failed or uncertain disk writes never become disk hits;
- multiple retained values smaller than the physical I/O alignment can share allocation space rather than each
  consuming one full alignment unit;
- allocator stress tests report useful utilization, fragmentation, allocation latency, and rewrite traffic
  across the target size distribution;
- buffered and direct modes return identical requested bytes, and requested direct mode never silently falls back;
- Linux and macOS support direct mode on a capable filesystem;
- disk subrange hits avoid reading a complete larger download;
- corrupted or uncertain disk bytes always miss and are never returned;
- restart recovers a safe useful subset after injected crashes;
- unsupported persistent formats are reset and logged safely;
- disk capacities of at least 1 TiB are representable with bounded internal accounting;
- the memory-only gate runs exact and expanded downloader controls with 1, 4, 16, and 64 shards through
  32 GiB, compares actual retained payload, and reports policy throughput; and
- the controlled native-Foyer comparison and separate end-to-end prefetch benchmark produce the metrics
  defined in Section 5.

## 14. Explicitly deferred

- streaming callback output;
- more than one `Download` returned by one callback;
- physical difference-only storage for partially overlapping downloads;
- guaranteed multi-extent assembly;
- public policy plug-ins or online disk-versus-network cost measurement;
- hard process-RSS guarantees, including callback and caller-held memory;
- online disk-capacity resize;
- native multi-volume placement;
- mutable objects, invalidation, and tombstones;
- multi-process access;
- direct mode on platforms other than Linux and macOS;
- stable on-disk compatibility across arbitrary future releases; and
- authoritative-storage or per-entry durability guarantees.
