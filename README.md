# Feuer

Scared of burning your S3 budget? Feuer will take care of that.

Feuer is a Rust cache for S3. It checks memory, then disk, and calls your async
download function on a miss. Requests can cover any non-empty byte range without
alignment requirements.

The disk tier requires Linux, io_uring, and direct I/O. Recovery is not implemented.
Each open starts with an empty disk index.

## Usage

Configure a cache directory, disk capacity, and memory target. Opening requires a
Tokio runtime. Cache handles are cloneable.

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
non-empty `Bytes` covering at least the requested range. Feuer returns exactly the
requested bytes as one contiguous `Bytes`.

Keys must identify immutable content. Distinct requested ranges for a key must not
overlap, though exact repeats are allowed. Downloads may cover multiple requests.
Feuer leaves request coalescing, scheduling, and retries to your downloader.

## Capacity and disk writes

Memory capacity is a soft payload target split across shards. An oversized download
empties its shard and remains cached. Caller-held bytes survive eviction and may
keep larger allocations alive. Metadata and pending disk writes use additional memory.

Disk capacity is fixed and must be a positive multiple of 1 MiB, including metadata
and alignment overhead. The backing file is exclusively locked while open. Disk
hits verify the entry's checksum and promote the requested bytes to memory.
Corrupt or uncertain reads count as misses.

Writes run in the background. The queue holds up to 256 entries and batches up to
64. Queued and active payload bytes have no byte limit. Queue pressure or memory
eviction can skip a disk write without failing the lookup.

Batches pack entries into immutable 1-MiB chunks. A chunk can be reused only after
all entry owners and read guards release it. Reads and writes use separate I/O
queues. See the [disk layout](feuer-storage/disk-prototype.md) for details.

## Eviction

Both tiers sample entries and evict the lowest retention score per payload byte.
For each request fully contained in a cached range, the score adds:

```text
decayed access count × (fixed request cost + requested bytes)
```

Each started request records its requested range once, before any lookup, in standalone history shared
by both tiers. Failed requests, invalid downloads, and requests canceled after starting all count as demand.
History has its own lock and one access clock across all keys; it is not part of a cache shard.
Raw cache reads, insertions, and evictions do not record accesses or delete history.

Every distinct `(object key, requested range)` counter is retained for the history object's lifetime,
even after both tiers evict the object. Repeated exact requests update the same counter. This metadata
has no capacity limit and is not persisted across restarts; its memory grows with distinct keys and ranges.
Scores still decay, measured in requests across all keys.

Memory may trim an entry to previously requested ranges instead of evicting it.
Trimming uses bounded recent-access events with an age limit, not the full counter set.
A trimming plan uses a history snapshot; newer accesses do not invalidate it. Cached-range changes
are still checked before publishing bytes copied outside the memory shard lock.

## Environment variables

| Variable | Default | Effect |
| --- | --- | --- |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Candidates per eviction decision. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `8192` | Score decay half-life in requests across all keys. |
| `FEUER_MAX_ACCESS_AGE_ACCESSES` | `262144` | Trimming event age limit in requests across all keys. |
| `FEUER_MAX_ACCESS_EVENTS_PER_KEY` | `64` | Trimming events retained per key. |
| `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` | `10000000` | Fixed request cost in equivalent bytes. `0` scores bytes only. |
| `FEUER_SMALL_IO_BUFFER_POOL_BYTES` | `128MiB` | Idle buffer budget per read queue, allocations ≤ 1 MiB. |
| `FEUER_MEDIUM_IO_BUFFER_POOL_BYTES` | `256MiB` | Idle buffer budget per read queue, allocations > 1 MiB and < 10 MiB. |
| `FEUER_LARGE_IO_BUFFER_POOL_BYTES` | `1GiB` | Idle buffer budget per read queue, allocations ≥ 10 MiB. |

Values accept integers or size suffixes such as `8KiB`, `1.5 GiB`, and `5GB`.
Binary suffixes use powers of 1024 and decimal suffixes use powers of 1000.
Suffixes work for counts too: `FEUER_ACCESS_COUNT_HALF_LIFE=8KiB` means 8192 accesses.
The first four settings require positive values. The rest allow zero.

`CacheConfig::new` reads the sample size and returns an error for invalid values.
`.with_reclaim_sample_size(n)` overrides it for that cache. The other settings are
process-wide, read once on first use, and panic on invalid values.

Read and write queues each have their own buffer pools, so total idle retention
can reach twice the sum of the three budgets. Active buffers and caller-owned
results use additional memory. Set a budget to zero to disable idle retention
for that size class.

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
