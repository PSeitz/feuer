# Feuer

Scared of burning your S3 budget? Feuer will take care of that.

Feuer is a Rust cache for S3. It checks memory, then disk, and calls your async
download function on a miss. Requests can cover any byte range without
alignment requirements.

The disk tier requires Linux, io_uring, and direct I/O. Opening waits for best-effort
metadata recovery; entries are checksum-verified when read. No persistence or recovery
hit rate is guaranteed.

## Usage

Configure a cache directory, disk capacity, and memory target. Zero disk capacity
uses only memory and ignores the directory. Opening is Linux-only and requires Tokio
when disk is enabled. Cache handles are cloneable.

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

Replace the callback with your source download. It returns a starting offset and
`Bytes` covering at least the requested range. Feuer returns exactly the
requested bytes as one contiguous `Bytes`.

Keys must identify immutable content. Requested ranges for a key may overlap;
exact repeats update the same access counter. Cached ranges receive retrieval credit
for every recorded request range they fully contain, not for partial overlaps.
Downloads may cover multiple requests.
Feuer leaves request coalescing, scheduling, and retries to your downloader.

At the public lookup boundary, Feuer hashes the key's UTF-8 bytes once with XXH3-128
(seed zero). Both tiers, access history, and disk recovery use only this 128-bit identity;
full keys are not stored or checked for collisions. Keys must not be adversarial.
[Disk format v13](format.md) stores the hash in little-endian form in fixed
48-byte entry metadata. Older disk formats are not migrated.

## Capacity and disk writes

Memory capacity covers cached allocation charges and idle read buffers in one cache instance.
Disk promotions copy the requested slice into a pooled buffer when its allocation capacity is
smaller than the original backing allocation. Otherwise, or if allocation fails,
they retain the original slice. Both the cached and returned bytes use the chosen allocation;
memory is charged its whole capacity. Whole-entry disk reads and checksum verification are unchanged.
Callback downloads are charged by payload length because `Bytes` does not expose capacity.
Entry targets remain split across shards; an oversized entry empties its shard and remains cached.
Active reads and caller-only results are outside the budget. Metadata and pending disk writes
can also keep additional memory alive.

Disk capacity includes metadata and alignment overhead.
The backing file is exclusively locked while open. Lookups first check the shard's buffered entries,
then read disk and verify the covering entry's checksum. Both promote requested bytes to memory.
Busy/flushing chunks may miss until publication; corrupt or uncertain disk reads are misses.

Writes run in the background through a 256-entry queue. Entries with aligned size below
128 KiB share 1-MiB chunks, flushed on full/no-fit or every 60 seconds. Each shard always holds
an initialized 1-MiB buffer; disk space is reserved only when flushing. Larger entries write
separately. Closing discards partial chunks. Queue pressure can skip writes; memory eviction does not.

Metadata is stored separately. Queued and active payload bytes have no byte limit. A payload chunk
can be reused after its last entry is removed; readers verify checksums rather than delay
reuse. Active writes may publish after memory eviction. Reads and writes use separate I/O
queues. See the [disk layout](feuer-storage/disk-prototype.md) for details.

## Download buffers and allocator retention

Your downloader allocates the `Bytes` returned by the callback; Feuer does not choose
those buffers' alignment, backing size, or reuse policy. No alignment is required.
Page-aligned buffers can avoid some disk-write scratch copies, though partial writes
and padding can still require scratch buffers.

In SSD replays, glibc did not return substantial freed memory to the OS, making RSS
much larger than live cache memory. Jemalloc avoided that retention in the tested
workload. Aligning and bucketing synthetic download buffers also reduced RSS, but
alignment alone was
not isolated or proven sufficient. These are application allocation choices, not a
requirement imposed by Feuer; the configured memory target is not an RSS limit.

## Eviction

Both tiers sample entries and evict the lowest retention score per byte:
allocation capacity in memory, payload length on disk.
For each request fully contained in a cached range, the score adds:

```text
decayed access count × (fixed request cost + requested bytes)
```

Each started request records its requested range once, before any lookup, in standalone history shared
by both tiers. Failed requests, invalid downloads, and requests canceled after starting all count as demand.
History hashes object keys into 64 independently locked maps, separate from cache shards.
One atomic request clock across all history shards preserves global request-age decay.
Raw cache reads, insertions, and evictions do not record accesses or delete history.

History keeps every distinct `(object key, requested range)` counter for its lifetime,
even after both tiers evict the object. Repeated exact requests update the same counter. This metadata
has no capacity limit and is not persisted across restarts; its memory grows with distinct keys and ranges.
Scores still decay, measured in requests across all keys.

Memory may trim an entry to previously requested ranges instead of evicting it.
Trimming uses recent-access events with per-object count and age limits, not the full counter set.
A trimming plan uses a history snapshot; newer accesses do not invalidate it. Cached-range changes
are still checked before publishing bytes copied outside the memory shard lock.

## Environment variables

| Variable | Default | Effect |
| --- | --- | --- |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Candidates per eviction decision. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `262144` | Score decay half-life in requests across all keys. |
| `FEUER_MAX_ACCESS_AGE_ACCESSES` | `262144` | Trimming event age limit in requests across all keys. |
| `FEUER_MAX_ACCESS_EVENTS_PER_KEY` | `64` | Maximum trimming events per key. |
| `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` | `10000000` | Fixed request cost in equivalent bytes. `0` scores bytes only. |
| `FEUER_IDLE_BUFFER_POOL_PERCENT` | `7` | Maximum idle buffers as a percentage of memory capacity, shared by all buckets. Range 0–100; `0` disables idle retention. |

Values accept integers or size suffixes such as `8KiB`, `1.5 GiB`, and `5GB`.
Binary suffixes use powers of 1024 and decimal suffixes use powers of 1000.
Suffixes work for counts too: `FEUER_ACCESS_COUNT_HALF_LIFE=8KiB` means 8192 accesses.
The first four settings require positive values. The rest allow zero.

`CacheConfig::new` reads the sample size and returns an error for invalid values.
`.with_reclaim_sample_size(n)` overrides it for that cache. The other settings are
process-wide, read once on first use, and panic on invalid values.

### Read buffers

`feuer-memory` owns one aligned buffer pool per cache instance, shared by its storage
readers. Allocation sizes are 32 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 4 MiB,
8 MiB, 16 MiB, 32 MiB, and 64 MiB.
Aligned read lengths round up to the smallest fitting size; callers receive only the
requested bytes. Larger allocations are exact-size and unpooled.

Cached entries and idle buffers share the configured memory capacity. The idle pool is
capped at 7% of that capacity by default. All size buckets share this limit;
there are no per-bucket caps or reservations. Cached entries can use the full capacity.
Released buffers are freed if the idle pool or the shared memory budget has no room. Admission
frees idle buffers before retaining new cached allocations; returning buffers never evicts
cached entries. Writes use unpooled scratch buffers, and standalone storage without a
memory cache retains no idle buffers.

Set `FEUER_IDLE_BUFFER_POOL_PERCENT` to change the idle ceiling. `FEUER_IO_BUFFER_POOL_BYTES`
and the former small/medium/large settings are no longer read.

### Memory benchmark only

| Variable | Default | Effect |
| --- | --- | --- |
| `COALESCING_DISTANCE_BYTES` | `10000000` | Merge requests with gaps below this size. |
| `WHOLE_SPLIT_THRESHOLD_BYTES` | `8MiB` | Download whole objects below this size. |

Both accept the same size suffixes and allow zero.

## Metrics

Use `TieredMemoryDiskCache::open_with_metrics(config, &registry).await` with a
`mixtrics::metrics::BoxedRegistry` to collect lookup, download, memory, and disk
metrics. Your application handles export. `open(config)` disables metrics.
See the [metric reference](metrics.md) for names and accounting rules.

## Development

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Storage tests require Linux with io_uring and direct-I/O support. On other platforms:

```console
cargo test -p feuer -p feuer-memory -p feuer-types
```

## Details

- [Disk layout](feuer-storage/disk-prototype.md)
- [Implementation status](implementation-status.md) and [design](tiered-plan.md)
- [Memory benchmarks](benchmarks/memory/README.md) and [results](benchmarks/memory/results.md)
- [SSD measurements](benchmarks/ssd/ssd-concurrent-read-write.md)

## License

[MIT](LICENSE). Inspired in part by [Foyer](https://github.com/foyer-rs/foyer).
