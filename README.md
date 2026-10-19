# Feuer

Feuer is a Rust cache for immutable whole objects, including S3 objects and
expensive computed results. It checks memory, then disk, and calls your async
fetch or computation function on a miss. Every successful lookup returns the
complete object as one `bytes::Bytes`, including empty objects.

The disk tier requires Linux, io_uring, and direct I/O. Opening waits for
best-effort metadata recovery. Disk payloads are checksum-verified when read.
There is no durability or minimum recovery hit-rate guarantee.

## Usage

Configure a cache directory, disk capacity, and positive memory target. Zero disk
capacity uses only memory and ignores the directory. Opening is Linux-only and
requires Tokio when disk is enabled. Cache handles are cloneable.

```rust
use bytes::Bytes;
use feuer::{CacheConfig, Download, TieredMemoryDiskCache};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1 GiB on disk, 64 MiB in memory.
    let config = CacheConfig::new("cache", 1 << 30, 64 << 20)?;
    let cache = TieredMemoryDiskCache::open(config, None, None).await?;

    let bytes = cache
        .get_or_fetch("object-v1".to_owned(), || async {
            Ok::<_, std::convert::Infallible>(Download::new(Bytes::from_static(b"hello world")))
        })
        .await?;

    assert_eq!(bytes.as_ref(), b"hello world");
    Ok(())
}
```

Replace the callback with your source download. `Download::new(bytes)` is
infallible and represents the complete object. The callback returns
`Result<Download, E>`. Alternatively, `get_or_fetch_object(key, callback)` accepts
`Result<Bytes, E>` directly. No lookup requires the object length in advance.

`get_object(key)` checks memory and disk without invoking or waiting for a source
callback. It returns `None` on a miss and promotes disk hits to memory. Use it to
plan work across multiple objects, then fetch the misses.

Keys must identify immutable content, including across restarts. Feuer hashes each
key's UTF-8 bytes with seed-zero XXH3-128. Both tiers, access history, and recovery
store only that identity. Full keys are not retained or checked for collisions,
so keys must not be adversarial.

Concurrent memory misses with the same key and exact callback error type share
one disk-or-callback operation and receive the same success or failure. Matching
callbacks must be interchangeable. Errors implement `Clone + Send + Sync + 'static`,
but callbacks and their futures may borrow caller state. Different error types do
not share work. Cancellation of the executing caller lets a waiter take over using
its own callback. Completed failures are not cached. Source scheduling and retries
remain the application's responsibility.

When changing your payload encoding, set `config.with_payload_version(1)` before
opening. The default is `0`. Any version change invalidates prior disk entries,
including on downgrade. This version is independent of the [disk format](format.md).
Older disk formats are not migrated.

## Falling through from a busy disk

`get_or_fetch_with_disk_fallthrough(key, should_fall_through, callback)` lets a
request use its source callback when at least 64 payload reads are waiting:

```rust,ignore
let bytes = cache.get_or_fetch_with_disk_fallthrough(
    key,
    || s3.has_spare_capacity(),
    || s3.fetch_object(),
).await?;
```

The decision is a synchronous `FnOnce() -> bool`. Capture any request information
and source load in the closure. `true` invokes the usual fetch callback, while
`false` waits for disk. `get_or_fetch_object_with_disk_fallthrough` offers the same
option with a `Bytes` callback. APIs without this decision always wait for disk.

Caches sharing `IoQueues` share 64 payload-read permits. Once 64 reads are waiting,
Feuer schedules a decision for every unchecked waiting read. Each decision runs
at most once in its caller's task, even if other fallthroughs have relieved the
pressure. Waiting for a permit does not allocate payload buffers.

Memory hits, buffered disk hits, absent entries, admitted reads, and shared-read
waiters skip the decision. Shared fetch callers follow the executing caller's
decision. A waiter taking over after cancellation uses its own decision.
Fallthrough does not cancel submitted I/O or remove disk entries. Successful
source downloads enter memory, but objects already on disk are not written again.
Source errors return without retrying disk.

## Independent caches on one SSD

Create one `IoQueues::new()?` and pass clones to
`TieredMemoryDiskCache::open(config, None, Some(io_queues)).await`.
Use separate directories or different filenames in the same directory:

```rust
let images = CacheConfig::new("/ssd/cache", 1 << 30, 64 << 20)?.with_file_name("images")?;
let documents = CacheConfig::new("/ssd/cache", 2 << 30, 32 << 20)?.with_file_name("documents")?;
```

The filename defaults to `data`, with lock file `.feuer.lock`. Other filenames use
`.feuer.<filename>.lock`. Filenames must be single names without NUL bytes or the
reserved `.feuer.` prefix. Opening the same file twice concurrently fails.

Shared queues allow 64 payload reads, 64 active read pieces, and 8 active writes,
with waiting channels for another 64 read pieces and 8 writes. Each I/O piece is
at most 1 MiB. Scheduling follows arrival order without per-cache fairness.

Contents, capacities, eviction, history, pools, file locks, and 512-entry background
write queues remain independent. Dropping one cache does not stop others. Pending
I/O retains its file and lock through completion. `None` creates dedicated queues.

See the [shared-I/O example](feuer/examples/shared_io_objects.rs):

```console
cargo run -p feuer --example shared_io_objects -- /path/on/ssd/cache
```

## Capacity and disk writes

Memory capacity covers cached allocation charges and idle buffers. Callback
downloads default to payload-length charges because `Bytes` does not expose
capacity. `Download::with_allocation_charge` supplies a known backing charge.
`get_or_fetch_object_with_allocation_charge` accepts `(bytes, allocation_charge)`.
Disk promotions and `AlignedBuffer::into_download` charge the retained buffer's
whole capacity. Shared allocations are conservatively charged per cached entry.

Entry targets are divided among shards. An oversized object empties its shard and
remains cached, so the target is soft. Caller-only results, active reads,
metadata, history, and pending disk writes can keep additional memory alive.
The memory target is not an RSS limit.

Disk capacity includes a reserved 1-MiB header chunk, shard metadata, and alignment
overhead. Nonzero capacity must be at least 2 MiB. Changing capacity invalidates
previous disk entries. Lookups first check buffered entries, then read and verify
the complete stored object. Disk hits promote the object to memory without another
write. Busy/flushing chunks may miss until publication. Read errors and checksum
failures become misses.

New downloads admitted to memory are queued for disk immediately, unless already
stored. The queue holds 512 entries and skips writes under pressure. Entries below
512 KiB aligned size share 1-MiB payload chunks, flushed on full/no-fit or every
60 seconds. Larger entries write separately. Closing discards partial chunks.
Memory eviction does not cancel queued writes.

Payloads occupy contiguous disk byte ranges, separate from metadata. A payload
chunk can be reused after its last entry is removed. Readers verify checksums
instead of delaying reuse. Queued and active payload bytes have no byte limit.
See the [disk format](format.md) and [queue guide](queues.html).

## Download buffers and allocator retention

Downloaders can call `cache.allocate_buffer(length)`, fill `buffer.as_mut_slice()`,
and return `buffer.into_download()`. Conversion does not copy the payload and
preserves the backing-capacity charge. The pool is shared with disk reads. The
allocation becomes reusable only after its last owner releases it.

Pool sizes are 32 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 4 MiB, 8 MiB, 16 MiB,
32 MiB, and 64 MiB. Larger buffers share one bucket and resize on reuse. On Linux,
buffers over 64 MiB use `mmap`/`mremap`/`munmap`. All buckets share an idle ceiling
of 7% of memory capacity by default. Admission frees idle buffers first. Returning
buffers never evicts cached entries. Writes use aligned inputs or unpooled scratch.

Downloaders may supply ordinary unaligned `Bytes`. Applications choose their own
allocator. Historical SSD replays found substantial glibc retention and lower RSS
with jemalloc, but allocator behavior depends on the workload.

## Eviction

Both tiers sample eligible objects and evict the lowest retention score.
`AvoidedRetrievalCostScorer` uses decayed per-object demand and avoided retrieval
cost per charged byte. Applications can share one scorer through
`open_with_retention_scorer` or supply separate memory and disk scorers through
`open_with_retention_scorers`.

```rust
let eviction = feuer::SampledEvictionConfig::new()?.with_sample_size(128)?;
let config = CacheConfig::new("cache", 1 << 30, 64 << 20)?
    .with_eviction(eviction)
    .with_access_count_half_life(262_144)?;
```

Objects are retained or evicted whole. See [eviction](eviction.md) for the formula
and scoring contract.

## Environment variables

| Variable | Default | Effect |
| --- | --- | --- |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Candidates per eviction decision, positive. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `262144` | Demand half-life in requests across all keys of one cache, positive. |
| `FEUER_IDLE_BUFFER_POOL_PERCENT` | `7` | Shared idle-buffer ceiling as a percentage of memory capacity, 0–100. Zero disables retention. |

Values accept integers or size suffixes such as `8KiB`, `1.5 GiB`, and `5GB`.
Binary suffixes use powers of 1024, decimal suffixes powers of 1000.
`CacheConfig::new` reads these settings and rejects invalid values. Explicit
configuration overrides the defaults. Both tiers share the cache's half-life.

## Metrics and access tracing

Pass `Some(&metrics_registry)` to `open` to register metrics in a
`mixtrics::metrics::BoxedRegistry`, independently of queue sharing. The application
owns export. `None` disables metrics. Multiple caches in one registry aggregate.
See the [metric reference](metrics.md).

Register an async callback with `cache.with_trace(callback)` for binary whole-object
access records. The receiver owns persistence. Dropping the last handle delivers
the tail asynchronously without waiting. See the [trace format](access-trace.md)
and [file receiver](feuer/examples/trace_to_disk.rs).

## Development

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Storage tests require Linux with io_uring and direct-I/O support. Elsewhere:

```console
cargo test -p feuer -p feuer-memory -p feuer-historian -p feuer-types
```

## Details

- [Architecture](architecture.html), [contract](tiered-plan.md), and [implementation status](implementation-status.md)
- [Disk layout](format.md)
- [Replay tools](feuer-replay/README.md)
- [Experiments](experiments/README.md): historical configurations, data, and summaries, not current API instructions

## License

[MIT](LICENSE). Inspired in part by [Foyer](https://github.com/foyer-rs/foyer).
