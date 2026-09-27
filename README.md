# Feuer

Feuer is a restart-recoverable tiered cache for byte ranges of immutable
objects. It is designed for arbitrary, non-empty ranges and fixed disk
capacities of at least 1 TiB.

> **Status:** active development. The public per-call callback, covering-memory
> path, and sampled, pressure-driven memory policy are present, but
> best-effort disk population and recovery are not yet complete.
> This repository is not ready for production use.

## Contract

- Every successful lookup returns one contiguous `bytes::Bytes` containing exactly the requested object bytes.
- Each `get_or_fetch` supplies its own callback. On a miss, that callback returns one downloaded start and non-empty `Bytes`; Feuer derives the exact range.
- The application download manager owns debouncing, range selection, source scheduling, retries, and source-memory bounds.
- Feuer imposes no source alignment or public cache block size. Its memory target is soft: an oversized download empties its shard and remains cached;
  returned `Bytes` may share a larger heap allocation.
- Disk population uses a bounded best-effort queue. Queue pressure or memory eviction may skip a write without failing the lookup.
- Disk storage targets Linux with usable io_uring. Direct payload I/O never silently falls back to buffered I/O.
- Successful lookups append the exact requested range to the key's accessed ranges; downloaded-range population is separate and creates no access.
- Recovery may lose recent entries, but uncertain or corrupt bytes always miss.
  Persistence, checksums, allocation, and recovery formats remain internal.

The authoritative design and acceptance criteria are in
[`tiered-plan.md`](tiered-plan.md). Current implementation status and the next
work are tracked in [`implementation-status.md`](implementation-status.md).

## Current workspace

The public boundary contains `ObjectKey`, `ByteRange`, a keyless validated
`Download`, explicit capacities in `CacheConfig`, and a cloneable `TieredMemoryDiskCache`. Each
`get_or_fetch` checks for a covering memory range before independently invoking
that call's asynchronous callback. Successful results are sliced to the exact
request and appended once to the key's accessed ranges, independently of downloaded-range population.

The internal `feuer-storage` crate currently provides an exclusively owned,
fixed-capacity O_DIRECT file and separate read/write io_uring queues (each with
its own thread and QD64), with arbitrary-range reads and 4-KiB-aligned write
offsets and lengths. Each queue schedules requests in arrival order without overlap
checks or a read-triggered write throttle.
Callers must prevent conflicting access to aligned byte ranges and retain a
submitted write's disk region until completion, even after cancellation.
Separate read/write admission
budgets protect read admission from write backlogs. Raw file capacity must be a positive multiple of
4 KiB. Above that, an experimental [`DiskRangeCache`](feuer-storage/disk-prototype.md)
connects allocation, persisted entry metadata, covering-range lookup and integrity-checked subrange reads.
Variable-length entries share 1-MiB chunks with 4-KiB-aligned payload allocations and no payload page headers.
Each entry has one checksum in its metadata; a hit verifies the whole entry and returns only the requested
bytes without reading neighboring entries. Reopen deliberately starts empty. Pressure eviction, recovery,
buffered mode and public-cache integration remain unimplemented.

`feuer-memory` provides a sharded soft-capacity covering-range index with bounded ageable request evidence,
sampled retrieval-cost-per-byte eviction, a short range-trimming grace, and
pressure-driven trimming of the selected victim toward observed requests. Its [memory-only comparison](benchmarks/memory/results.md)
records the current policy baseline. `feuer-types` holds shared range
foundations, while `feuer-tokio` remains the Tokio/madsim runtime switch. None of these internal
package boundaries is a public compatibility commitment.

## Development

Storage builds and tests require Linux, enabled io_uring, and a filesystem that
reports compatible direct-I/O alignment through `statx` (for example, ext4 on a
recent kernel). Unsupported environments fail explicitly; tests do not silently
skip real I/O. On other platforms, memory-only packages can still be tested with
`cargo test -p feuer -p feuer-memory -p feuer-types`.

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

Licensed under the [MIT License](LICENSE). Inspired in part by
[Foyer](https://github.com/foyer-rs/foyer).
