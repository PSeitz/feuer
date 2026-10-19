# Feuer

Scared of burning your S3 budget? Feuer will take care of that.

Feuer is a Rust cache for immutable data, including S3 objects and expensive
computed results. It checks memory, then disk, and calls your async fetch or
computation function on a miss. Requests can cover any byte range without
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
    let cache = TieredMemoryDiskCache::open(config, None, None).await?;

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

`get_or_fetch_object(key, callback)` returns the complete object without requiring its length;
the callback returns `Bytes`. Do not mix whole-object and range lookups for the same key,
including after reopening the disk cache. Whole-object lookups treat the stored entry's
length as the object length. Empty objects are supported.

Keys must identify immutable content. Requested ranges for a key may overlap;
exact repeats update the same access counter. Cached ranges receive retrieval credit
only for recorded request ranges they fully contain.
Downloads may cover multiple requests.
Feuer leaves request coalescing, scheduling, and retries to your downloader.

At the public lookup boundary, Feuer hashes the key's UTF-8 bytes once with XXH3-128
(seed zero). Both tiers, access history, and disk recovery use only this 128-bit identity;
full keys are not stored or checked for collisions. Keys must not be adversarial.
[Disk format v14](format.md) stores the hash in little-endian form in fixed
48-byte entry metadata. Older disk formats are not migrated.

When changing your payload encoding, set `config.with_payload_version(1)` before opening
(the default is `0`). Any version change invalidates the previous disk entries, including on
downgrade. The version is stored once in the global file header and is
independent of Feuer's disk-format version.

## Independent caches on one SSD

Create one `IoQueues::new()?` and pass its clones to
`TieredMemoryDiskCache::open(config, None, Some(io_queues)).await`.
Metrics and queue sharing are independent: replace `None` with
`Some(&metrics_registry)` to collect metrics while sharing queues.
Use separate directories or different backing filenames in the same directory:

```rust
let images = CacheConfig::new("/ssd/cache", 1 << 30, 64 << 20)?.with_file_name("images")?;
let documents = CacheConfig::new("/ssd/cache", 2 << 30, 32 << 20)?.with_file_name("documents")?;
```

The filename defaults to `data`, with lock file `.feuer.lock`. Other filenames use
`.feuer.<filename>.lock`. Filenames must be single names, without NUL bytes or the
reserved `.feuer.` prefix. Opening the same file twice concurrently fails.
The caches share only low-level read/write queues and worker threads: 64 active reads and 8 active writes,
with waiting channels for another 64 reads and 8 writes, across all attached caches.
Scheduling follows arrival order, without per-cache fairness.

Contents, capacities, eviction, access histories, buffer pools, file locks,
and 512-entry background-write queues remain independent. Dropping one cache does
not stop the others. Pending I/O retains its target file and lock through completion;
idle shared queues retain no files. Passing `None` for `io_queues` creates dedicated
I/O queues.

See the [whole-object example](feuer/examples/shared_io_objects.rs):

```console
cargo run -p feuer --example shared_io_objects -- /path/on/ssd/cache
```

The example leaves metrics disabled. With a real registry, caches sharing it
contribute to aggregate metrics; use separate registries for per-cache metrics.

## Capacity and disk writes

Memory capacity covers cached allocation charges and idle read buffers in one cache instance.
Disk promotions copy the requested slice into an aligned buffer only when its allocation capacity is
at least 25% smaller than the original backing allocation. Otherwise, or if allocation fails,
they retain the original slice. Both the cached and returned bytes use the chosen allocation;
memory is charged its whole capacity. Whole-entry disk reads and checksum verification are unchanged.
Callback downloads default to payload-length charges because `Bytes` does not expose capacity.
Downloads created with `AlignedBuffer::into_download` charge the buffer's whole capacity;
other downloaders can supply a known capacity with `Download::with_allocation_charge`.
Whole-object callbacks can return `(bytes, allocation_charge)` through
`get_or_fetch_object_with_allocation_charge`. This preserves a shared slice's backing-capacity charge
without copying. Ordinary `get_or_fetch_object` callbacks charge only payload length.
Shared allocations are conservatively charged once per cached entry.
Correcting the charge does not release unused bytes inside an allocation retained by a slice.
Entry targets remain split across shards; an oversized entry empties its shard and remains cached.
Active reads and caller-only results are outside the budget. Metadata and pending disk writes
can also keep additional memory alive.

Disk capacity includes a reserved 1-MiB header chunk, shard metadata, and alignment overhead.
Nonzero capacity must be at least 2 MiB. Changing the file capacity invalidates disk entries.
The backing file is exclusively locked while open. Lookups first check the shard's buffered entries,
then verify disk bytes against a covering checksum. [Subrange checksums](format.md#overview)
prepared from recent requests during writing or learned during reading let partial reads skip the rest of an entry.
Both paths promote requested bytes to memory.
Busy/flushing chunks may miss until publication; corrupt or uncertain disk reads are misses.

Exact-range downloads and whole objects are queued for disk immediately after memory admission.
Expanded downloads stay memory-only until successful compaction queues their retained ranges.
Compaction requires memory pressure and a 64-request grace period. After that grace, it trims whenever
the retained ranges reduce payload bytes, without a minimum savings percentage.

`DiskCache` owns the 512-entry background-write queue and its buffer-flush timer. Entries below
512 KiB aligned size share 1-MiB chunks, flushed on full/no-fit or every 60 seconds. Each shard always holds
an initialized 1-MiB buffer; disk space is reserved only when flushing. Larger entries write
separately. Closing discards partial chunks. Queue pressure can skip writes. Memory eviction does not cancel queued writes.

Metadata is stored separately. Queued and active payload bytes have no byte limit. A payload chunk
can be reused after its last entry is removed; readers verify checksums rather than delay
reuse. Active writes may publish after memory eviction. Reads and writes use separate I/O
queues. See the [disk layout](feuer-storage/disk-prototype.md) for details.

## Download buffers and allocator retention

Downloaders can acquire a buffer with `cache.allocate_buffer(length)`, fill
`buffer.as_mut_slice()` from their response stream, then return `buffer.into_download(start)`.
Acquisition uses the same aligned, bucketed pool as disk reads. Converting the buffer
into a download does not copy its payload and preserves its backing-capacity charge.
The caller, memory cache, and disk-write queue share the allocation. It becomes available
for reuse only after its last owner releases it, subject to the idle capacity limit.
Acquire after finalizing any coalesced range to avoid allocating one buffer per waiter.

Downloaders may still supply their own `Bytes`; no alignment is required.
Page-aligned buffers can avoid some disk-write scratch copies, though small-entry
packing, partial writes, and padding can still require copies.

In SSD replays, glibc did not return substantial freed memory to the OS, making RSS
much larger than live cache memory. Jemalloc avoided that retention in the tested
workload. Aligning and bucketing synthetic download buffers also reduced RSS, but
the effect of alignment alone remains untested. Applications choose their allocator;
the configured memory target is not an RSS limit.

## Eviction

Both tiers sample eligible entries and evict the lowest retention score.
`AvoidedRetrievalCostScorer` uses decayed avoided retrieval cost per charged byte,
with configurable costs per request and per requested byte. Applications can share
one scorer through `open_with_retention_scorer` or supply separate memory and disk scorers through
`open_with_retention_scorers`.

`CacheConfig` groups candidate sampling under `SampledEvictionConfig`, separate from
capacities, disk location, and idle-buffer retention. Scoring parameters belong to the scorer.

```rust
let eviction = feuer::SampledEvictionConfig::new()?.with_sample_size(128)?;
let config = CacheConfig::new("cache", 1 << 30, 64 << 20)?
    .with_eviction(eviction)
    .with_access_count_half_life(262_144)?;
```

See [eviction policy](eviction.md) for the formula, defaults, and scoring contract.

## Environment variables

| Variable | Default | Effect |
| --- | --- | --- |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Candidates per eviction decision. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `262144` | Default access count half-life for `CacheConfig`, in requests across all keys of that cache. |
| `FEUER_MAX_ACCESS_AGE_ACCESSES` | `262144` | Trimming event age limit in requests across all keys. |
| `FEUER_MAX_ACCESS_EVENTS_PER_KEY` | `64` | Maximum trimming events per key. |
| `FEUER_IDLE_BUFFER_POOL_PERCENT` | `7` | Maximum idle buffers as a percentage of memory capacity, shared by all buckets. Range 0–100; `0` disables idle retention. |

Values accept integers or size suffixes such as `8KiB`, `1.5 GiB`, and `5GB`.
Binary suffixes use powers of 1024 and decimal suffixes use powers of 1000.
Suffixes work for counts too: `FEUER_ACCESS_COUNT_HALF_LIFE=8KiB` means 8192 accesses.
The first four settings require positive values. The idle buffer percentage allows zero.

`CacheConfig::new` reads the sample size, access count half-life, and idle buffer percentage,
returning an error for invalid values. `SampledEvictionConfig::with_sample_size(n)`,
`CacheConfig::with_access_count_half_life(n)`, and `CacheConfig::with_idle_buffer_pool_percent(n)`
override them for that cache. Both tiers share the cache's fixed half-life. Standalone memory
and disk caches use the default half-life unless supplied with configured `ObjectAccessHistories`.
The trimming settings are process-wide, read once on first use, and panic on invalid values.

### Read buffers

`feuer-memory` owns one aligned buffer pool per cache instance, shared by its storage
readers. Allocation sizes are 32 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 4 MiB,
8 MiB, 16 MiB, 32 MiB, and 64 MiB.
Aligned read lengths round up to the smallest fitting size; callers receive only the
requested bytes. Larger allocations share one idle bucket. Reuse grows to fit and shrinks
only when capacity is at least 10% larger than the requested length. On Linux, buffers over
64 MiB use `mmap`/`mremap`/`munmap`.

Disk reads and downloads acquired through `cache.allocate_buffer` share this pool.
Cached entries and idle buffers share the configured memory capacity. The idle pool is
capped at 7% of that capacity by default. All size buckets share this limit;
there are no per-bucket caps or reservations. Cached entries can use the full capacity.
Released buffers are freed if the idle pool or the shared memory budget has no room. Admission
frees idle buffers before retaining new cached allocations; returning buffers never evicts
cached entries. Writes use unpooled scratch buffers, and standalone storage without a
memory cache retains no idle buffers.

Set `FEUER_IDLE_BUFFER_POOL_PERCENT` to change the default idle ceiling, or set it per cache:

```rust
let config = CacheConfig::new("/path/on/ssd/cache", 1 << 30, 256 << 20)?
    .with_idle_buffer_pool_percent(10)?;
```

Use `0` to disable idle buffer retention. `FEUER_IO_BUFFER_POOL_BYTES` and the former
small/medium/large settings are no longer read.

### Memory benchmark only

| Variable | Default | Effect |
| --- | --- | --- |
| `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` | `10000000` | Replay-only fixed request cost in equivalent bytes; passed explicitly to Feuer's scorer. |
| `COALESCING_DISTANCE_BYTES` | `10000000` | Merge requests with gaps below this size. |
| `WHOLE_SPLIT_THRESHOLD_BYTES` | `8MiB` | Download whole objects below this size. |

All accept the same size suffixes and allow zero.

## Metrics

Use `TieredMemoryDiskCache::open(config, Some(&metrics_registry), None).await` with a
`mixtrics::metrics::BoxedRegistry` to collect lookup, download, memory, and disk
metrics. Your application handles export. Passing `None` for `metrics_registry`
disables metrics, independently of the I/O queue choice.
See the [metric reference](metrics.md) for names and accounting rules.

## Access tracing

Register an async callback with `cache.with_trace(callback)`.
Feuer delivers ready-to-persist binary packages; the receiver owns persistence.
Dropping the last cache handle delivers the tail asynchronously, without waiting.
See the [format and contract](access-trace.md) and [file receiver](feuer/examples/trace_to_disk.rs).

## Development

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Storage tests require Linux with io_uring and direct-I/O support. On other platforms:

```console
cargo test -p feuer -p feuer-memory -p feuer-historian -p feuer-types
```

## Details

- [Disk layout](feuer-storage/disk-prototype.md)
- [Implementation status](implementation-status.md) and [design](tiered-plan.md)
- [Experiments](experiments/README.md): configurations, raw data, and summaries
- [Memory benchmarks](experiments/memory/README.md) and [results](experiments/memory/results.md)
- [SSD measurements](experiments/ssd/ssd-concurrent-read-write.md)
- [Tier retrieval cost model](experiments/cache/tier-retrieval-cost-20261011/README.md)

## License

[MIT](LICENSE). Inspired in part by [Foyer](https://github.com/foyer-rs/foyer).
