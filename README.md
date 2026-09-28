# Feuer

Feuer is a Rust cache for S3. It checks memory,
then disk, and calls your async download function on a miss. Requests can cover
any non-empty byte range, with no alignment or block-size requirements.

**Status: burning hot.** 
recovery is not yet implemented. Every open starts with an empty disk
index. The disk tier requires Linux, io_uring, and direct I/O; there is no buffered
or memory-only fallback.

## Usage

Configure a cache directory, a fixed disk capacity, and a memory target. Opening
the cache requires a Tokio runtime. Cache handles are cloneable.

This example supplies a fixed payload in the callback. In an application, the
callback would fetch bytes from your object store or other source.

```rust
use bytes::Bytes;
use feuer::{ByteRange, CacheConfig, Download, TieredMemoryDiskCache};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1 GiB on disk, 64 MiB in memory.
    let config = CacheConfig::new("cache", 1 << 30, 64 << 20)?;
    let cache = TieredMemoryDiskCache::open(config).await?;

    let bytes = cache
        .get_or_fetch("object-v1".to_owned(), ByteRange::new(6, 11)?, || async {
            Download::new(0, Bytes::from_static(b"hello world"))
        })
        .await?;

    assert_eq!(bytes.as_ref(), b"world");
    Ok(())
}
```

The callback returns a starting offset and a non-empty `Bytes`. Its download
must cover the requested range, but may be larger. Feuer returns exactly the
requested bytes as one contiguous `Bytes`.

Each call supplies its own callback. Feuer does not combine concurrent misses or
retry downloads. Your download manager controls request coalescing, range
selection, scheduling, retries, and download memory limits. Object keys must
identify immutable content.

## Capacity and disk writes

**Memory capacity is a soft payload-byte target**, divided among shards. A
download larger than its shard's target empties that shard and remains cached.
Returned slices may share a larger allocation, and caller-held bytes remain valid
after eviction. The target is not a limit on process memory.

**Disk capacity is fixed** and must be a positive multiple of 1 MiB, including
metadata and alignment overhead. The backing file is exclusively locked while
open. Disk hits verify the checksum of the entire cached entry, return only the
requested bytes, and promote those bytes to memory. Corrupt or uncertain reads
are treated as misses.

Downloads retained in memory are queued for disk without making the lookup wait
for a write. The queue allows up to 256 entries and batches up to 64 entries.
Queued plus active payload bytes are tracked but not limited or charged to the
memory-cache capacity. Queue pressure or memory eviction can skip a disk write
without failing the lookup.

Disk batches pack entries into immutable 1-MiB chunks. A written chunk is never
appended to or reused until all entry owners and read guards release it. Reads
and writes use separate io_uring queues and admission budgets, so a write backlog
does not consume read admission slots. See the
[disk layout](feuer-storage/disk-prototype.md) for storage details.

## Eviction

Both tiers use cost-aware eviction. The policy samples entries and evicts the one
with the lowest estimated retrieval cost saved per retained byte. Memory can also
trim a selected entry to previously requested ranges instead of evicting it entirely.

Both tiers share access history for each object. A successful lookup records its
requested range once; storing downloaded bytes alone does not count as an access.
Distinct requested ranges within an object must not overlap; repeating the same
range is allowed. Downloaded ranges may expand to cover multiple requests.
Each exact requested range has an access count that decays with a half-life of
8,192 successful same-shard accesses by default. A cached range receives the decayed retrieval
cost of every request it fully covers. Counters live until the object's final
history owner is released; their number is not capped per object.
Counters are stored once in a Vec, with a HashMap mapping exact request ranges to
counter indices. Exact cached ranges use a direct lookup; expanded ranges scan
the Vec for fully contained requests. Both tiers use these same counters.

Range trimming separately retains at most 64 request events per object by default,
expiring after 262,144 later successful same-shard accesses. Neither history is
persisted; metadata is outside payload-capacity accounting.

### Configuration

| Environment variable | Default | Purpose |
| --- | --- | --- |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Limit candidates examined per eviction decision. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `8192` | Set cost-aware score decay half-life in successful same-shard accesses. |
| `FEUER_MAX_ACCESS_AGE_ACCESSES` | `262144` | Set the range-trimming history lifetime in successful same-shard accesses. |
| `FEUER_MAX_ACCESS_EVENTS_PER_KEY` | `64` | Limit range-trimming request events per object, not decayed scoring counters. |
| `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` | `10000000` | Fixed source-request cost added to requested bytes when scoring cost-aware retention. |
| `FEUER_SMALL_IO_BUFFER_POOL_BYTES` | `134217728` (128 MiB) | Maximum idle allocation bytes per I/O queue for buffers up to 1 MiB. |
| `FEUER_MEDIUM_IO_BUFFER_POOL_BYTES` | `268435456` (256 MiB) | Maximum idle allocation bytes per I/O queue for buffers larger than 1 MiB and smaller than 10 MiB. |
| `FEUER_LARGE_IO_BUFFER_POOL_BYTES` | `1073741824` (1 GiB) | Maximum idle allocation bytes per I/O queue for buffers of 10 MiB or larger. |

All numeric environment settings above use the same `bytesize` parser. They accept
plain integers and suffixes such as `512MiB`, `5GiB`, `1.5 GiB`, or `5GB`.
Binary suffixes (`KiB`, `MiB`, `GiB`) use powers of 1024; decimal suffixes
(`KB`, `MB`, `GB`) use powers of 1000. Suffixes act as multipliers for count
settings too: `FEUER_ACCESS_COUNT_HALF_LIFE=8KiB` means 8192 accesses.

`CacheConfig::new` reads the sample size, rejecting invalid values.
The sample size must be a positive `usize`. The builder overrides a valid
environment setting for an individual cache:

```rust
use feuer::CacheConfig;

let config = CacheConfig::new("cache", 1 << 30, 64 << 20)?
    .with_reclaim_sample_size(128)?;
```

`FEUER_ACCESS_COUNT_HALF_LIFE` is process-wide and read once on first use,
including in standalone memory caches and the memory benchmark. It must be a
positive value fitting `u64`; invalid values panic. Smaller values forget historical
popularity faster; larger values retain it longer. It affects both tiers'
cost-aware scores, not trimming-history expiration.

The access-age setting is process-wide and read once on first use. It also applies
to standalone `MemoryCache` instances and the memory benchmark. Set it before
starting the process. It must be positive and fit `u64`; invalid values panic.
`18446744073709551615` effectively disables expiration, and `32768` restores the
previous default. This setting affects range trimming, not decayed cost-aware
scores or the per-object event cap.

`FEUER_MAX_ACCESS_EVENTS_PER_KEY` independently sets that history cap. It is also
process-wide, read once on first use, and applies to standalone memory caches and
the memory benchmark. Set it to a positive value fitting `usize`; invalid values panic.
For example, `FEUER_MAX_ACCESS_EVENTS_PER_KEY=256` retains up to 256 events per
object for range trimming. Larger event histories use more metadata memory and
increase trimming work. This setting does not change decayed scoring counters
or the eviction sample size.

`FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` is also process-wide and read once on first
use, including in standalone memory caches. It must be nonnegative and fit
`u64`; invalid values panic. The default represents 125 ms at 80 MB/s. Set it to
`0` to score only requested bytes saved per retained byte, without a fixed reward
for avoiding a request. It affects both tiers' cost-aware scores.

The I/O buffer pool settings are process-wide and read once on first use. Values
must be nonnegative and fit `usize`; invalid values panic. For example,
`FEUER_LARGE_IO_BUFFER_POOL_BYTES=5GiB` sets the large-buffer limit to 5 GiB.
Set `0` to disable idle retention for a size class. Read and write queues each have
their own three pools, so the combined idle limit per disk cache is twice the sum of
these settings. Active buffers and caller-owned results are outside these limits.
Larger limits retain more allocations for exact-size reuse; allocations are freed
when their pool is full or destroyed.

## Metrics

Use `TieredMemoryDiskCache::open_with_metrics(config, &registry).await` with a
`mixtrics::metrics::BoxedRegistry` to collect lookup, callback, memory, disk I/O,
population, capacity, and packing metrics. `open(config)` uses no-op metrics.
Labels are bounded and contain no object identities. Your application handles
export. See the [metric reference](metrics.md) for names and accounting rules.

## Development

Storage builds and tests require Linux with io_uring enabled and a filesystem
that reports compatible direct-I/O alignment through `statx`, such as ext4 on a
recent kernel. Unsupported environments fail explicitly; tests do not silently
skip real I/O.

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

On other platforms, run the configuration, memory, and shared-type tests:

```console
cargo test -p feuer -p feuer-memory -p feuer-types
```

## Further reading

- [Implementation status](implementation-status.md): what works and what remains.
- [Design and acceptance criteria](tiered-plan.md): the intended behavior, including recovery.
- [Memory benchmarks](benchmarks/memory/README.md) and [results](benchmarks/memory/results.md).
- [SSD read/write measurements](benchmarks/ssd/ssd-concurrent-read-write.md).

Internal crate boundaries and storage formats are not compatibility commitments.

## License

[MIT](LICENSE). Inspired in part by [Foyer](https://github.com/foyer-rs/foyer).
