# Feuer Tiered Cache Contract

This document describes the cache's behavioral contract. See
[implementation status](implementation-status.md) for current components and open work.

## Object identity and results

Feuer is an embedded read-through cache for immutable objects. It combines a
soft-capacity memory tier with a fixed-capacity, best-effort local disk tier.
It is not authoritative storage.

Successful lookups return the object as `bytes::Bytes`. Empty objects are valid.
Callers do not supply object lengths.
The application owns source correctness and must give different content different
keys, including across process restarts and application upgrades.

The public key is a UTF-8 `String`. Feuer hashes it once with seed-zero XXH3-128.
Both tiers, history, and recovery compare only that 128-bit identity. Full keys
are not retained or checked for collisions. Keys must not be adversarial.

Admission, retention, writes, and recovery are best-effort. False misses and
skipped or lost writes are allowed. One tier need not retain what the other holds.
Disk bytes must pass their expected checksum before use, subject to hash and
checksum collisions.

## Public lookup boundary

- `get_or_fetch(key, callback)` accepts an asynchronous callback returning
  `Result<Bytes, E>` and charges payload length in memory.
- `get_or_fetch_buffer(key, callback)` accepts an asynchronous callback returning
  `Result<AlignedBuffer, E>`. It charges buffer capacity, then converts the buffer
  to shared `Bytes` without copying. `AlignedBuffer` is reexported by `feuer`.
- `get_object(key)` checks memory and disk without invoking or waiting for a source
  callback. A miss returns `None`.

Lookup checks memory, then disk, then invokes the callback on a miss. Disk hits
promote the object to memory without another disk write. Callback errors
return to callers without source retries. Downloads admitted to memory are queued
for disk unless the object is already stored. Duplicate objects are not retained
or written again within a tier.

Concurrent memory misses with the same key and exact callback error type share
one disk-or-callback operation. Callbacks for that group must be interchangeable.
Errors implement `Clone + Send + Sync + 'static`. Callbacks and futures may borrow
caller state. Different error types do not share work. If the executing caller is
canceled, a waiter can take over using its own callback. Completed errors are not
cached for later requests. Application callbacks run without cache metadata locks.

Returned `Bytes` may share a larger allocation and remain valid after eviction.
The cache does not own source scheduling, source retries, or process memory used
by callbacks and caller-held results.

### Disk-pressure fallthrough

The `get_or_fetch_with_disk_fallthrough` and
`get_or_fetch_buffer_with_disk_fallthrough` variants accept a synchronous decision
closure. At least 64 waiting payload reads trigger decisions for unchecked waiters.
A true result invokes the source callback instead of waiting for a disk-read permit.
Each decision runs at most once in its caller's task. Ordinary APIs wait for disk.

Fallthrough neither cancels submitted I/O nor removes the disk entry. Shared fetch
callers follow the executing caller's decision. A takeover after cancellation uses
the new executing caller's closure. Source errors return without retrying disk.

## Demand and retention

Every started lookup records one access to its object before checking either tier,
including failures and cancellation after starting. Shared operations still record
once per caller. Raw tier reads, insertion, and eviction do not record accesses.

`ObjectAccessHistories` holds one decayed counter per object in independently
locked maps, separate from the cache shards. One request clock advances across
all keys. The default half-life is 262,144 requests. Counters survive eviction for
the history object's lifetime, have no capacity limit, and are not persisted.

Both tiers evict the lowest-scoring object from a rotating sample, 64 candidates
by default. The [default scorer](eviction.md) weights avoided retrieval cost
by the object's decayed demand and divides by memory allocation charge or disk
payload length. Separate tier scorers may use the same demand history.

Custom scorers receive the key hash, payload length, tier-specific size, and shared
history. Scores must be finite. Scoring runs under shard locks, so scorers must
avoid I/O, cache re-entry, and incompatible application lock ordering.

## Memory accounting

Cached allocation charges and idle buffers share a memory target.
Entry targets are divided among shards. Admission evicts from the selected shard.
An object larger than its shard's target empties that shard and remains cached,
so aggregate retained charges can exceed the target. Memory pressure never waits
for disk throughput, and queued writes do not protect entries from eviction.

`get_or_fetch` charges payload length because `Bytes` does not expose capacity.
`get_or_fetch_buffer` and disk promotions charge their retained buffer's full
capacity. Shared allocations are conservatively charged once per cached entry.

An aligned buffer pool is shared by reads and downloader-acquired buffers. All
buckets share an idle ceiling, 7% of the memory target by default. Admission frees
idle buffers first. Buffer returns never evict entries. A buffer becomes reusable
only after its last owner releases it.

Metadata, history, allocator overhead, active reads, pending writes, and caller-only
results are outside the memory target. It is not a hard process-RSS limit.

## Disk writes and layout

Disk APIs accept and return payloads as `Bytes`.

The disk tier requires Linux direct I/O and io_uring, with no silent fallback.
Zero disk capacity disables it. Opening waits for all metadata recovery to finish.
Nonzero capacity includes the global header, metadata, payloads, and alignment
overhead, and must be at least 2 MiB. Capacity is fixed while open.

The best-effort write queue holds 512 entries. A full or closed queue skips writes
without failing a successful lookup. Queued and active payload bytes have no byte limit.
Writes can publish after memory eviction or caller cancellation. Publication
rechecks whether the object is already indexed, not whether memory retains it.
Failed payload writes do not publish entries. The tiered API exposes no flush.

Entries below 512 KiB aligned size share per-shard 1-MiB payload chunks. A chunk
flushes when full or when the next small entry will not fit. Partial chunks flush
every 60 seconds. Buffering reserves no disk space. Busy/flushing buffers can miss
until publication. Closing discards partial chunks. Larger entries write independently.

Each payload occupies a contiguous disk byte range with no metadata gaps.
Addresses and storage lengths are aligned to 4 KiB. Multi-chunk objects reserve
consecutive whole chunks exclusively. Written chunks cannot be appended to and
individual payload holes cannot be reused. A chunk becomes reusable when its last
entry is removed. Metadata occupies separate 1-MiB chunks containing 4-KiB pages.
Each record holds an object identity, payload address, length, and checksum.
The last page links to the next metadata chunk. See [the format](format.md).

Eviction selects individual entries rather than every owner of a shared chunk.
Removing an entry may free no chunk. Admission can fail despite sufficient total
free space if it cannot reserve consecutive chunks or exhausts its eviction budget.
Cleaning and relocation are not required for admission.

## I/O and integrity

Readers copy a payload address and expected checksum under the index lock, then
read the object into an owned buffer and verify it. They do not read
neighboring objects or metadata on the lookup path. Buffered entries can be copied
without disk I/O.

Readers hold no disk reservation and do not delay chunk reuse. Concurrent reuse
can cause a checksum miss. Read errors and checksum mismatches never return
uncertain payloads. Submitted I/O buffers remain alive through completion even
if callers cancel. Checksums are probabilistic, not a protection against malicious
content or an independent validation of callback bytes.

Shared `IoQueues` provide 64 payload-read permits plus separate read/write channels,
rings, and worker threads. At most 64 read pieces and 8 writes are active, with
waiting channels of 64 and 8. Each physical operation transfers at most 1 MiB.
Scheduling follows arrival order without per-cache fairness. Waiting for a payload
permit does not allocate its read buffer. Queue limits do not cap total buffer RAM.

## Recovery and compatibility

The backing file is exclusively locked. Different files can share a directory
and I/O queues. Pending I/O retains its target file and lock through completion.

The global header identifies the disk format, application payload version, and
capacity. A mismatch invalidates entries, including payload-version
downgrades. Older disk formats are discarded rather than migrated.

Recovery follows metadata-chunk links without scanning payloads. Corrupt record
pages are discarded independently and invalid links terminate a chain. Payloads
are verified only when read. Recovered stale entries may miss.

There is no `fsync`, shutdown flush, durability, stable on-disk compatibility, or
minimum recovered hit-rate guarantee. Recovery may retain any subset, including
none. The application remains responsible for authoritative data.

## Observability and evaluation

Metrics use fixed operation, outcome, source, and buffer-bucket labels, never
object identities. See [metrics](metrics.md). Optional [access traces](access-trace.md)
record requests and completed outcomes without payloads.

A shared callback result counts as one completed lookup per caller. Source
invocations, retries, and transferred bytes must be measured by the application
or replay harness.

Comparisons must hold workload, capacities, concurrency, cold-cache state, and
source model constant. Report retained memory, source requests and bytes,
request and byte hit rates, throughput, tail latency, disk utilization, metadata,
and read/write amplification. Historical experiment results are not evidence for
a changed implementation. No superiority claim follows from the design alone.

## Outside this contract

- Mutable objects, invalidation, and tombstones.
- Streaming or multiple callback payloads.
- Source clients, source retries, and online retrieval-cost measurement.
- Hard process-memory limits.
- Online disk-capacity resize or multi-volume placement.
- Multi-process access to one backing file.
- Disk storage outside Linux, buffered-I/O fallback, or authoritative durability.
