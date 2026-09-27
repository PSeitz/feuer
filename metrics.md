# Metrics

On Linux, use `TieredMemoryDiskCache::open_with_metrics(config, &registry).await`
with a `mixtrics::metrics::BoxedRegistry`. This registers the public lookup,
memory, disk I/O, range-cache and population metrics in that registry.
`open(config)` keeps the no-op default. Feuer does not install an exporter;
the application owns the registry and its export endpoint.

All labels below have fixed values. Object keys, paths and caller-defined cache
names are not labels. Multiple caches using the same registry aggregate their
counters and gauges; use separate registries if they must be distinguished.

## Public lookups and callbacks

| Metric | Type | Labels / meaning |
|---|---|---|
| `feuer_lookup_total` | Counter | `outcome`: `memory_hit`, `disk_hit`, `callback`, `callback_error`, `invalid_download` |
| `feuer_lookup_duration_seconds` | Histogram | `outcome`: `memory_hit`, `disk_hit`; entire completed lookup for cache hits only |
| `feuer_lookup_bytes_total` | Counter | `source`: `memory`, `disk`, `callback`; exact requested bytes successfully returned |

Lookup outcomes and duration histograms count completed operations, not canceled
futures. A non-covering callback result is an `invalid_download` lookup.

Use lookup counters for request-weighted hit ratios and lookup byte counters for
byte-weighted hit ratios. Memory-tier counters also include internal memory
operations; use the public lookup counters to measure caller-visible behavior.

## Disk lookups, capacity and packing

| Metric | Type | Labels / meaning |
|---|---|---|
| `feuer_disk_lookup_total` | Counter | `outcome`: `hit`, `absent`, `io_error`, `integrity_failure` |
| `feuer_disk_lookup_duration_seconds` | Histogram | Same outcomes; includes whole-entry reading, checksum verification and copying |
| `feuer_disk_chunks` | Gauge | `state`: `free`, `reserved`, `quarantined`; each chunk is 1 MiB |
| `feuer_disk_payload_bytes` | Gauge | Payload bytes in indexed entries, excluding padding and metadata |
| `feuer_disk_entries` | Gauge | Indexed disk entries |
| `feuer_disk_evictions_total` | Counter | Entries removed by capacity pressure, excluding replacement and invalidation |
| `feuer_disk_batch_bytes_total` | Counter | `kind`: `payload`, `chunk`; payload and whole-chunk bytes of successfully written shard batches, before publication |

Disk read errors and integrity failures still behave as cache misses; metrics
make those distinct from absent entries. `contains()` does not count as a lookup.

Chunk states are disjoint. Reserved chunks remain reserved until **all entry
owners and read guards release them**; eviction does not necessarily free a
chunk. Quarantined chunks cannot be reused during that allocator's lifetime.
Gauges are removed when their owners disappear, including on cache shutdown;
detached writes can outlive the last public cache handle.

`reserved * 1 MiB - indexed payload bytes` measures capacity unavailable for
indexed payload, including metadata, padding, partially dead chunks, active
writes and read-guard retention. It is not a pure fragmentation measurement.
Quarantined capacity is reported separately. Indexed payload bytes count retained
entry payloads, not a deduplicated union of overlapping object ranges.

For byte-weighted packing efficiency, divide the rate of batch `payload` bytes
by the rate of batch `chunk` bytes. Both exclude failed shard batches and include
successful writes later discarded at publication. Raw disk-I/O byte counters
still include successful writes from partially failed batches. Existing read-I/O
byte counters measure completed DataFile read lengths, not alignment padding or
individual kernel submissions.

## Best-effort disk population

| Metric | Type | Labels / meaning |
|---|---|---|
| `feuer_disk_population_queue_total` | Counter | `outcome`: `queued`, `queue_full`, `queue_closed`, `byte_budget`, `oversized`, `stale`, `canceled`, `already_covered`, `redundant` |
| `feuer_disk_population_queued_entries` | Gauge | Entries waiting to begin population |
| `feuer_disk_population_pending_bytes` | Gauge | Queued plus active payload bytes charged to the byte budget |
| `feuer_disk_population_queue_duration_seconds` | Histogram | Queue admission to dequeue, including entries found stale; excludes entries canceled before dequeue |
| `feuer_disk_population_total` | Counter | Terminal per-entry batch-insertion outcome: `published`, `already_covered`, `no_capacity`, `stale`, `failed`, `canceled` |
| `feuer_disk_population_written_entries_total` | Counter | Entries in successfully written shard batches, before publication checks |

Queue and storage counters describe different stages: do not sum all their
values as a total number of population attempts. `queued` is admission, not a
terminal outcome. Queue `already_covered` means disk already covers the callback
download; `redundant` means memory declined a contained download. Queue `stale`
means the original memory admission expired before writing. Storage `stale`
means it expired before publication, after writing.

A storage `failed` outcome means its shard batch write failed. Entries abandoned
before a terminal decision, including later shards skipped after a batch error,
count as `canceled`. Canceling the caller does not cancel a detached storage
writer: that writer continues to report its actual terminal outcome.

Written entries are not necessarily published entries. Successful writes can be
discarded because another entry already covers them or their memory admission
is stale. Gauges follow ownership so rejection, cancellation, errors and normal
completion release their counts along with the associated payload budget.

## Existing metrics

- `feuer_memory_operations_total{operation}`, `feuer_memory_payload_bytes`,
  `feuer_memory_entries`, `feuer_memory_compacted_payload_bytes_total`.
- `feuer_disk_io_total{operation,outcome}`,
  `feuer_disk_io_duration_seconds{operation,outcome}`,
  `feuer_disk_io_bytes_total{operation}`; operations are `read` and `write`,
  outcomes are `success` and `error`.

No recovery, flush, serialization, or source-download-manager metrics are added.
