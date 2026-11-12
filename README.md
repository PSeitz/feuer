# Feuer

![Feuer — cost-aware tiered cache, with a flaming horse logo](https://raw.githubusercontent.com/PSeitz/feuer/main/full_logo.svg?v=cfeda6a)

[Guide](https://feuer-rs.github.io/) · [API reference](https://docs.rs/feuer)

Scared of burning your S3 budget? Feuer will take care of that.

Feuer caches immutable objects in memory and on disk, with application callbacks for S3 and other sources.

## Design

Fetching from source is expensive. Minimizing work to serve requests is the primary goal.
The design for the disk layout follows hardware observations: SSD prefers larger block reads and writes.

### Cost-aware eviction

It supports a custom cost model for each cache layer. The model can account for:

- The cost of fetching or recomputing an object.
- Future reuse based on how often and how recently it is requested.
- Its payload size.
- Application-specific information about the object. E.g. CPU time to compute it, or network bandwidth to fetch it.

Each layer uses its model to assign retention scores, then evicts the lowest-scoring object among sampled candidates.

### SSD I/O and buffer reuse

Disk storage uses Linux `io_uring` for asynchronous I/O and direct I/O to bypass
the kernel page cache.

- Aligned buffer reuse. Downloads can use the disk-read buffer pool. Fill a buffer
  and hand it to the cache without copying its payload.
- Payload packing. Small entries share 1-MiB payload chunks. Larger entries occupy
  consecutive whole chunks. Each payload is contiguous, with metadata in separate chunks.
- Separate read and write queues. Reads and writes have separate `io_uring` rings
  and worker threads, so they can progress concurrently.
- Source fallthrough under disk pressure. Applications can choose to fetch from
  the source when disk reads are backed up.
- In-flight fetch sharing. Concurrent misses for the same key and callback error
  type share disk reads and source work. Any waiter can drive the shared fetch. Cancelling
  one caller preserves it for the others. Dropping the last waiter cancels it.
  Fetch callbacks and their futures must be `Send + 'static`.

The tiered cache API supports memory-only operation on every platform. Disk caching requires Linux.

## Runtime requirements

Disk caching requires Linux with `io_uring` and a filesystem supporting `O_DIRECT` and `STATX_DIOALIGN`.
Set disk capacity to `0` for memory-only operation on any platform.

The `disk` Cargo feature is enabled by default. Set `default-features = false` on your
`feuer` dependency to exclude `feuer-storage` entirely, including on Linux. A build without
this feature rejects nonzero disk capacity.

Verify customer environments before enabling disk caching. Kernel support alone
is insufficient: `kernel.io_uring_disabled` can restrict ring creation, and
container seccomp policies can block the io_uring syscalls. A blocked
`io_uring_setup` can return `Operation not permitted (os error 1)`.

## Quick start

Add to your `Cargo.toml`:

```toml
[dependencies]
feuer = { git = "https://github.com/PSeitz/feuer" }
bytes = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Create a cache and supply a callback for misses:

```rust
use std::convert::Infallible;

use bytes::Bytes;
use feuer::Cache;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cache = Cache::builder()
        .memory_capacity(64 << 20)
        .disk_capacity(1 << 30)
        .directory("feuer_cache")
        .open()?;

    let bytes = cache
        .get_or_fetch("object-v1", || async {
            // Replace this with your download or computation.
            Ok::<_, Infallible>(Bytes::from_static(b"hello world"))
        })
        .await?;

    assert_eq!(bytes.as_ref(), b"hello world");
    Ok(())
}
```

By default, metrics are disabled and the cache creates its own I/O queues when disk is enabled.
Use `.with_metrics(&registry)` to register metrics or `.with_io_queues(queues)`
to share I/O queues on Linux. Disk capacity defaults to `0` for memory-only operation
on any platform. The default directory is `feuer_cache` and file name is `data`.
Configure each tier's scorer with `.with_memory_retention_scorer(scorer)` and
`.with_disk_retention_scorer(scorer)`.

## How lookups work

`get_or_fetch(key, callback)` checks memory, then disk, then calls your callback:

1. Memory hit: return the bytes.
2. Disk hit: read and checksum-verify the object, then promote it to memory.
3. Miss: run the callback, cache the result in memory, and queue a disk write.

Use `get(key)` to read from memory or disk without fetching from the source.

If you'd rather fetch from the source than wait for a busy disk, use `get_or_fetch_with_disk_fallthrough`.
This allows to maximize utilization of your resources. E.g. network bandwidth can be used to fetch from the source while disk is busy.

## Before you use it

- Don't use the same key for different payloads. If you mutate an object, use a new key.
  Feuer has no invalidation API.
- Keys must be non-adversarial. Feuer stores only XXH3-128 hashes, without retaining the
  original keys or checking for collisions. XXH3 is non-cryptographic and provides no protection
  against deliberately constructed collisions. Colliding keys identify the same object, so a
  lookup can return another key's payload.
- Disk caching and recovery are best-effort. Opening completes metadata recovery synchronously.
  Disk caching requires an entered Tokio runtime for background workers. Memory-only opening needs no runtime.
- Changing capacity, disk format, or application payload version invalidates disk entries. Use
  `.payload_version(n)` when changing your payload encoding.
- Give each cache its own backing file. The file is exclusively locked while
  open. Use separate directories or `.file_name("images")`.

## Tuning and integration

### Eviction

Both tiers sample objects and evict the one with the lowest retention score.
The default scorer combines decayed demand with avoided retrieval cost per
charged byte. Its default cost model weights bytes. Configure a per-request cost
to account for fixed fetch overhead, or provide a scorer for computation costs.
Memory and disk can use different scorers.

See [eviction](eviction.md) for the formula, configuration examples, and trade-offs.

### Memory shards

Set `FEUER_MEMORY_SHARD_COUNT` or use `.memory_shard_count(n)` to set a positive shard count per cache. The default
is one shard per 8 GiB of memory capacity, rounded down, with at least one shard.
More shards can reduce lock contention. The memory eviction target is divided among them.

### Environment variables

`.open()` reads these defaults. Builder setters override them per cache.

| Variable | Default | Purpose |
| --- | --- | --- |
| `FEUER_MEMORY_SHARD_COUNT` | `max(1, memory_capacity / 8 GiB)` | Memory cache shard count. Must be positive. |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Candidates per eviction. Must be positive. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `262144` | Requests across keys before demand weight halves. Must be positive. |
| `FEUER_HISTORY_ENTRY_LIMIT` | `10000000` | History entry-count cleanup threshold per cache. Must be positive. |

Values accept integers and size suffixes such as `8KiB` or `1.5 GiB`.
Binary suffixes use powers of 1024, decimal suffixes powers of 1000.

Use `.reclaim_sample_size(n)`, `.access_count_half_life(n)`, and
`.history_entry_limit(n)` to override these defaults.

## Development

On Linux with io_uring and direct-I/O support:

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

On other platforms:

```console
cargo test -p feuer -p feuer-memory -p feuer-historian -p feuer-utils
```

For internals, see the [architecture](architecture.html), [I/O queues](queues.html),
[disk format](format.md), and [implementation status](implementation-status.md).
See [timed SSD replay](timed-replay.md), the
[storage read/write benchmark](feuer-storage/read-write-benchmark.md), and the
[recovery benchmark](feuer-storage/recovery-benchmark.md) for performance validation.

## License

[MIT](LICENSE). Inspired in part by [Foyer](https://github.com/foyer-rs/foyer).
