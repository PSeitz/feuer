# Feuer

Feuer is a restart-recoverable tiered cache for byte ranges of immutable
objects. It is designed for arbitrary, non-empty ranges and fixed disk
capacities of at least 1 TiB.

> **Status:** active development. The public per-call callback, covering-memory
> path, and prefetch-aware, pressure-driven memory policy are present, but
> best-effort disk population and recovery are not yet complete.
> This repository is not ready for production use.

## Contract

- Every successful lookup returns one contiguous `bytes::Bytes` containing exactly the requested object bytes.
- Each `get_or_fetch` supplies its own callback. On a miss, that callback returns one downloaded start and non-empty `Bytes`; Feuer derives the exact range.
- The application download manager owns debouncing, range selection, source scheduling, retries, and source-memory bounds.
- Feuer imposes no source alignment or public cache block size. Its memory target is soft: an oversized download empties its shard and remains cached;
  returned `Bytes` may share a larger heap allocation.
- Disk population uses a bounded best-effort queue. Queue pressure or memory eviction may skip a write without failing the lookup.
- Cache open selects buffered or direct payload I/O. Direct mode is required on Linux and macOS and never silently falls back.
- Successful lookups append the exact requested range to the key's accessed ranges; downloaded-range population is separate and creates no access.
- Recovery may lose recent entries, but uncertain or corrupt bytes always miss.
  Persistence, checksums, allocation, and recovery formats remain internal.

The authoritative design and acceptance criteria are in
[`tiered-plan.md`](tiered-plan.md). Current implementation status and the next
work are tracked in [`implementation-status.md`](implementation-status.md).

## Current workspace

The public boundary contains `ObjectKey`, `ByteRange`, a keyless validated
`Download`, one soft memory payload target, and a cloneable `Cache`. Each
`get_or_fetch` checks for a covering memory range before independently invoking
that call's asynchronous callback. Successful results are sliced to the exact
request and appended once to the key's accessed ranges, independently of downloaded-range population.

The internal `feuer-storage` crate currently provides an exclusively owned,
fixed-capacity file and checked buffered positional I/O. Direct I/O and the
recoverable range engine remain to be implemented. `feuer-memory` provides a
sharded soft-capacity covering-range index with bounded aged request evidence,
prefetch probation measured in successful same-shard accesses, one ageable
recent demonstrated-reuse signal, sampled value-aware eviction, and
pressure-driven compaction toward observed requests. Its [memory-only comparison](benchmarks/memory/results.md)
records the current policy baseline. `feuer-types` holds shared range
foundations, while `feuer-tokio` remains the Tokio/madsim runtime switch. None of these internal
package boundaries is a public compatibility commitment.

## Development

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

Licensed under the [MIT License](LICENSE). Inspired in part by
[Foyer](https://github.com/foyer-rs/foyer).
