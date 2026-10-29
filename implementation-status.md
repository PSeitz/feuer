# Feuer Implementation Status

Feuer caches immutable objects through **memory → integrity-checked disk →
callback**. The [contract](tiered-plan.md) describes behavior, and the
[README](README.md) shows public APIs and configuration.

## Components

| Crate | Responsibility |
| --- | --- |
| `feuer` | Cloneable tiered handle, lookup, shared in-flight work, optional disk-pressure fallthrough, metrics, and access tracing |
| `feuer-types` | XXH3-128 object identity |
| `feuer-historian` | One decayed request counter per object, shared by both tiers and independent of cache residency |
| `feuer-memory` | Sharded object index, sampled eviction, cached allocation accounting, and aligned buffer pool |
| `feuer-storage` | Linux direct I/O and io_uring, object index, payload checksums, chunk packing, background writes, metadata persistence, and recovery |
| `feuer-replay` | Trace decoding and workload replay tooling |

## Lookup and retention

- `get_or_fetch` and `get_or_fetch_with_disk_fallthrough` accept callbacks returning
  `Result<V, E>` where `V: Into<BytesAndMemoryCharge>`. Both accept `Bytes` or
  `AlignedBuffer` and return `Bytes`. Buffer conversion shares the payload without
  copying. `feuer` reexports `AlignedBuffer` and exposes `BytesAndMemoryCharge`
  with private fields and `From` conversions for both payload types. The conversion
  is normally inferred from the callback result. `get` performs lookup without a
  source callback.
- Empty objects are valid. Keys identify immutable content. Both tiers suppress
  duplicate insertions.
- Matching concurrent misses share one disk-or-callback operation by key and
  callback error type, including across `Bytes` and `AlignedBuffer` callbacks.
  Cancellation lets a waiter take over with its own callback.
- Every started lookup records one object access before checking either tier.
  Failures and cancellation count as demand. Insertion and raw tier reads do not.
- Both tiers sample objects and evict the lowest retention score. The default
  scorer combines decayed demand with fixed and per-byte avoided retrieval cost.
- History survives eviction but not restart. There is one counter per object and
  no history-capacity limit. The default half-life is 262,144 requests across all
  keys, configurable per cache.

## Memory and buffers

Cached allocation charges and idle buffers share the memory target. Callback
results of type `Bytes` charge payload length. `AlignedBuffer` results and disk
reads charge retained buffer capacity. Entry targets are split among shards,
and an oversized object can exceed its shard's target after evicting its peers.

Each memory cache owns an aligned pool shared with storage readers and downloaders.
All buckets share an idle ceiling of 7% of memory capacity by default. Caller-held
results, active I/O, metadata, history, and pending writes can retain memory outside
the target. The target is not an RSS limit.

## Disk and I/O

- Disk APIs accept and return payloads as `Bytes`.
- Zero disk capacity disables storage. Otherwise Linux direct I/O and io_uring are
  required, and opening waits for every shard's metadata recovery.
- A 512-entry queue schedules best-effort writes independently of memory retention.
  A full or closed queue skips writes. Payload bytes have no separate queue byte limit.
- Small entries share 1-MiB payload chunks. A chunk flushes when full, when the next
  small entry will not fit, or every minute.
  Entries at least 512 KiB after alignment reserve consecutive chunks independently.
  Each payload is contiguous, with metadata in separate chunks.
- Readers copy buffered objects or read and verify persisted objects.
  Read errors and checksum failures become misses. Readers do not delay chunk reuse.
- Dirty metadata pages are persisted periodically after payload publication.
  Recovery reads linked metadata chunks without scanning payloads. There is no
  durability or minimum recovery hit-rate guarantee.
- Separate caches can share payload-read admission and low-level queues while
  retaining independent files, capacities, indexes, pools, histories, and write queues.
  Shared queues allow 64 payload reads, 64 active read pieces, and 8 active writes.

See [disk format](format.md), [queues](queues.html), and [metrics](metrics.md) for
layout, scheduling, and accounting details.

## Validation and remaining work

Portable tests cover types, history, memory, configuration, and tier orchestration.
Storage execution requires Linux with direct I/O and io_uring. Use the repository's
[SSD testing instructions](timed-replay.md) for real-device validation and the
[recovery benchmark](feuer-storage/recovery-benchmark.md) for repeated-open measurements.

Remaining evaluation includes crash and power-loss recovery testing, target-scale
metadata and fragmentation measurements, contention and tail-latency analysis,
and controlled comparisons with matched object workloads and source costs.
The current linear scorer does not inspect disk residency or queue state. Its
policy quality and allocator behavior require workload-specific measurements.
Feuer requires direct I/O for disk storage, uses a soft memory target, and discards
entries from incompatible disk formats.
