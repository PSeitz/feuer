# Metrics

On Linux, use `TieredMemoryDiskCache::open_with_metrics(config, &registry).await`
with a `mixtrics::metrics::BoxedRegistry`. This registers the public lookup,
memory, disk I/O, disk-cache and disk-write metrics in that registry.
Feuer does not install an exporter, the application owns the registry and its export endpoint.

All labels below have fixed values. Object keys, paths and caller-defined cache
names are not labels. Multiple caches using the same registry aggregate their
counters and gauges. Use separate registries if they must be distinguished.

## Public lookups and callbacks

| Metric | Type | Labels / meaning |
|---|---|---|
| `feuer_lookup_total` | Counter | `outcome`: `memory_hit`, `disk_hit`, `callback`, `callback_error`, `invalid_download` |
| `feuer_lookup_duration_seconds` | Histogram | `outcome`: `disk_hit`, `callback`. Successful lookup after a memory miss, including callback work and synchronous disk-write scheduling |
| `feuer_lookup_bytes_total` | Counter | `source`: `memory`, `disk`, `callback`. Exact requested bytes successfully returned |

Lookup outcomes count completed operations, not canceled futures. Duration
histograms record successful disk hits and callbacks only, starting after the
memory lookup. Memory hits do not read the clock or record a duration; their
outcome and served-byte counters still increase. A non-covering callback result
is an `invalid_download` lookup and does not record a duration.

Use lookup counters for request-weighted hit ratios and lookup byte counters for
byte-weighted hit ratios. Memory-tier counters also include internal memory
operations. Use the public lookup counters to measure caller-visible behavior.

## Disk lookups, capacity and packing

| Metric | Type | Labels / meaning |
|---|---|---|
| `feuer_disk_lookup_total` | Counter | `outcome`: `hit`, `absent`, `io_error`, `checksum_failed` |
| `feuer_disk_lookup_duration_seconds` | Histogram | `outcome`: `hit`. Buffered copying, or whole-entry reading, checksum verification and copying |
| `feuer_disk_chunks` | Gauge | `state`: `free`, `allocated`. Each chunk is 1 MiB |
| `feuer_disk_payload_bytes` | Gauge | Payload bytes in indexed entries, excluding padding and metadata |
| `feuer_disk_entries` | Gauge | Indexed disk entries |
| `feuer_disk_batch_bytes_total` | Counter | `kind`: `payload`, `chunk`. Payload and payload-chunk bytes of successfully written payload regions, before publication. Metadata writes are included in the raw I/O counters, not this packing counter |

Disk-tier hits include copies from the shard's buffer; those perform no disk I/O, reserve no disk capacity, and are not yet indexed.
Read errors and checksum failures are distinct misses. `covers_range()` includes buffered entries but does not count as a lookup.

Chunk states are disjoint. Payload reservations are released after their last indexed
entry is removed, without waiting for readers. Metadata chunks remain reserved while
open; active writers retain their current reservation while awaiting I/O.
Eviction does not necessarily free a shared chunk.
Gauges are removed when their owners disappear, including on cache shutdown.
Detached writes can outlive the last public cache handle.

`allocated * 1 MiB - indexed payload bytes` measures capacity unavailable for
indexed payload, including metadata, padding, partially dead chunks, and active
writes. It is not a pure fragmentation measurement. Indexed payload bytes count
entry payloads, not a deduplicated union of overlapping object ranges.

For byte-weighted packing efficiency, divide the rate of batch `payload` bytes
by the rate of batch `chunk` bytes. Both exclude failed payload regions and include
successful writes later discarded at publication. Raw disk-I/O byte counters
still include successful writes from partially failed batches. Existing read-I/O
byte counters measure completed DataFile read lengths, not alignment padding or
individual kernel submissions.

## Disk recovery

| Metric | Type | Meaning |
|---|---|---|
| `feuer_disk_recovered_chunks` | Gauge | Currently allocated 1-MiB chunks restored by recovery; no labels |

Recovered chunks are a subset of `feuer_disk_chunks{state="allocated"}`, not
additional capacity. Each chunk counts once, including shared chunks and every
chunk of a multi-chunk entry. Temporary scan reservations and normal writes do
not increase this gauge. A recovered payload chunk stops counting after its last
entry is removed; reuse by a new write does not count as recovered.

Recovery validates metadata; payload checksums are verified on read. A positive
value shows that recovery restored chunks still reserved by metadata or payload entries,
not that their payloads have been verified. Zero can mean recovery has not yet
restored anything, or that all recovered chunks have since been released.

### Recovery logs

Enable `feuer::storage=info` in the application's tracing filter:

- `starting disk cache recovery`: recovery has begun.
- `disk cache recovery finished`: scan completed, with elapsed seconds. This
  does not guarantee any chunks were restored.

Recovery follows each shard's metadata-chunk links without scanning payload chunks.
Invalid or unsupported pages are discarded without a global reset or generation change.

## Insertions that trigger eviction

| Metric | Type | Meaning |
|---|---|---|
| `feuer_memory_eviction_triggering_insertions_total` | Counter | Completed memory insertion attempts that evicted at least one entry under capacity pressure |
| `feuer_disk_eviction_triggering_insertions_total` | Counter | Terminal disk insertion attempts that evicted at least one entry under capacity pressure |

Each incoming entry counts at most once, even if it evicts several entries.
Replacement, invalidation, memory compaction and eviction searches that remove
nothing do not count. Disk admission runs once per flush or independent large write;
a shared flush attributes its evictions to its first buffered entry, not every neighbor.
An attempt still counts if it evicts entries but is later skipped, fails or is canceled.

Percentage of memory insertion attempts that trigger eviction:

```promql
100 * sum(rate(feuer_memory_eviction_triggering_insertions_total[5m]))
  / sum(rate(feuer_memory_operations_total{operation=~"insert|replace|redundant"}[5m]))
```

Percentage of disk insertion attempts that trigger eviction:

```promql
100 * sum(rate(feuer_disk_eviction_triggering_insertions_total[5m]))
  / sum(rate(feuer_disk_write_entries_total[5m]))
```

Both denominators include redundant/already-covered attempts. Disk attempts
start when the writer handles each download, not at queue admission. The percentage is undefined
when there are no attempts in the window.

## Disk read sizes

`feuer_disk_read_size_bytes` is a histogram with no labels, recording the requested
byte length of each successful DataFile read. It excludes alignment padding,
failed reads and canceled futures, and records once per DataFile call rather
than per kernel submission. These are not necessarily caller-requested lookup
sizes: disk lookups also read metadata and whole cached entries.

Buckets double from 1 KiB through 1 GiB. For the p95 read size in bytes:

```promql
histogram_quantile(0.95, sum by (le) (rate(feuer_disk_read_size_bytes_bucket[5m])))
```

Use `0.50` or `0.99` for p50 or p99. Percentiles are estimated from the buckets.

## I/O buffer pools

Each memory-cache instance owns one pool with fixed allocation sizes of 32 KiB,
256 KiB, 512 KiB, 1 MiB, 2 MiB, 4 MiB, 8 MiB, 16 MiB, 32 MiB, and 64 MiB.
Cached allocation charges and idle buffers share the configured memory capacity.
The idle pool is capped at 7% of that capacity by default. All buckets share this
ceiling, without per-bucket caps or reservations.
`FEUER_IDLE_BUFFER_POOL_PERCENT` sets this percentage (0–100); zero disables idle retention.
Larger allocations are not pooled. Writes use aligned input slices or unpooled scratch.
These metrics are registered by `MemoryMetrics`, not `IoMetrics`, and have no `pool`
or `operation` label. The former `pool=small|medium|large` labels have been removed.
The buffer gauge has a `bucket` label containing human-readable allocation capacity:
`32 KiB`, `256 KiB`, `512 KiB`, `1 MiB`, `2 MiB`, `4 MiB`, `8 MiB`, `16 MiB`,
`32 MiB`, or `64 MiB`.
Gauge values remain in bytes. Update filters using the former numeric bucket labels.

| Metric | Type | Meaning |
|---|---|---|
| `feuer_memory_used_bytes` | Gauge | Cached allocation charges plus idle allocation capacity; excludes active reads and caller-only results |
| `feuer_memory_capacity_bytes` | Gauge | Configured shared capacity of live memory caches and their pools |
| `feuer_io_buffer_pool_bytes` | Gauge | Allocation capacity per `bucket` and `status`: `idle` (available for reuse) or `used` (held by readers, cached entries, or callers) |

`feuer_memory_used_bytes` replaces `feuer_memory_payload_bytes`; the separate
`feuer_io_buffer_pool_capacity_bytes` gauge has been removed. Update dashboards.
Disk promotions charge the retained allocation's whole capacity, including padding and unused
bytes. If copying the slice into an aligned buffer saves at least 25% of backing capacity,
the charge is the destination buffer's capacity. Otherwise, or if allocation fails,
the original buffer and its charge are retained.
Callback downloads without capacity metadata use their payload length.
Shared allocations are conservatively charged per cached entry. The existing oversized-entry
exception can still make cached charges exceed the target.

**Memory utilization (%)** — cached and idle allocation charges relative to capacity:

```promql
100 * sum(feuer_memory_used_bytes)
  / sum(feuer_memory_capacity_bytes)
```

**Allocation bytes per bucket and status:**

```promql
sum by (bucket, status) (feuer_io_buffer_pool_bytes)
```

Omit `bucket` to aggregate across all sizes. Utilization is undefined for zero capacity.

Buffers remain `used` until their last owner releases them, even if the pool has
already been destroyed. They then become `idle` if returned to the pool, or leave
the gauge if freed. Idle bytes are included in `feuer_memory_used_bytes`; used buffer bytes
are charged there only when held by cache entries. Do not add the two gauges.
Allocations above 64 MiB and write scratch bypass the pool gauge. Admission frees
idle buffers as necessary before retaining cached allocations. Gauges sum across
cache instances sharing a registry. Standalone storage without a memory cache
retains no idle buffers.

## Best-effort disk writes

| Metric | Type | Labels / meaning |
|---|---|---|
| `feuer_disk_write_queue_total` | Counter | `outcome`: `queued`, `queue_full`, `queue_closed`, `canceled`, `already_covered`, `redundant` |
| `feuer_disk_write_queued_entries` | Gauge | Entries waiting to begin disk writes |
| `feuer_disk_write_pending_bytes` | Gauge | Logical payload bytes queued, prepared, or being written; excludes chunk padding and is not charged to memory-cache capacity |
| `feuer_disk_write_queue_duration_seconds` | Histogram | Queue admission to dequeue. Excludes entries canceled before dequeue |
| `feuer_disk_write_entries_total` | Counter | Terminal per-entry write outcome: `published`, `already_covered`, `no_capacity`, `failed`, `canceled` |
| `feuer_disk_written_entries_total` | Counter | Entries in successfully written payload regions, before publication checks |

Queue and storage counters describe different stages: do not sum all their
values as a total number of disk-write attempts. `queued` is admission, not a
terminal outcome. The tiered layer records `already_covered` when disk covers the callback
download, and `redundant` when memory declined a contained download.
Queued and active writes proceed independently of memory retention.

A storage `failed` outcome means the entry's payload write failed. `no_capacity` can also mean
metadata capacity was unavailable after successful payload I/O; the whole flush is then discarded.
Prepared entries discarded
before completion count as `canceled`; downloads not yet handled have no storage attempt.
Canceling an `insert_batch` caller does not cancel its detached writer.

Successful writes may be discarded by disk containment checks. Pending-byte accounting lasts
until each entry completes or is discarded, including its time in a partial chunk. The writer
releases small incoming buffers after copying. Each shard always holds a 1-MiB buffer, even when empty;
flushing retains the detached buffer through I/O as well. This gauge measures logical payload, not backing capacity.

## Existing metrics

- `feuer_memory_operations_total{operation}`: `insert`, `replace`, `redundant`,
  `remove`, `compact`. Internal access/hit/miss counters are not emitted.
  Use public lookup counters for hit ratios.
- `feuer_memory_entries`: cache entries (not idle buffers).
- `feuer_memory_compacted_payload_bytes_total`: cached allocation charges released
  by compaction; does not guarantee the allocation was freed if callers still hold it.
- `feuer_disk_io_total{operation,outcome}`: logical data-file operations `read` and `write`,
  outcomes `success` and `error`. A multi-chunk payload run counts as one write, not one per kernel request.
- `feuer_disk_io_duration_seconds{operation,outcome}`: operations `read` and
  `write`, outcome `success` only. Errors remain counted but are not timed.
- `feuer_disk_io_bytes_total{operation}`: operations `read` and `write`.

No flush, serialization, or source-download-manager metrics are added.
