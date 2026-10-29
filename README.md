# Feuer

Scared of burning your S3 budget? Feuer will take care of that.

Feuer caches immutable objects in memory and on disk, with application callbacks for S3 and other sources.

## Design

### Cost-aware eviction

It supports a custom cost model for each cache layer. The model can account for:

- The cost of fetching or recomputing an object.
- How often and how recently it is requested.
- Its payload size and the space it occupies in that layer.
- Application-specific information about the object.

Each layer uses its model to assign retention scores, then evicts the lowest-scoring object among sampled candidates.

### SSD I/O and buffer reuse

Disk storage uses Linux `io_uring` for asynchronous I/O and direct I/O to bypass
the kernel page cache. [SSD experiments](experiments/README.md) guide queue sizing
and payload packing.

- Aligned buffer reuse. Downloads can use the disk-read buffer pool. Fill a buffer
  and hand it to the cache without copying its payload.
- Payload packing. Small entries share 1-MiB payload chunks. Larger entries occupy
  consecutive whole chunks. Each payload is contiguous, with metadata in separate chunks.
- Separate read and write queues. Reads and writes have separate `io_uring` rings
  and worker threads, so they can progress concurrently.
- Source fallthrough under disk pressure. Applications can choose to fetch from
  the source when disk reads are backed up.
- In-flight fetch sharing. Concurrent misses for the same key and callback error
  type share disk reads and source work.

The tiered cache API supports memory-only operation on every platform. Disk caching requires Linux.

## Runtime requirements

Disk caching requires Linux with usable `io_uring` and a filesystem supporting
`O_DIRECT` and `STATX_DIOALIGN`. Opening a disk cache fails when these requirements
are unmet. Set disk capacity to `0` for memory-only operation on any platform,
including macOS, without a Tokio runtime or `io_uring`. Nonzero disk capacity
returns an error on other platforms. Access tracing through `with_trace` requires Tokio.

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
use feuer::{CacheConfig, TieredMemoryDiskCache};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Cache directory, disk capacity (1 GiB), memory target (64 MiB). 
    let config = CacheConfig::new("cache", 1 << 30, 64 << 20)?;
    let cache = TieredMemoryDiskCache::open(config, None, None).await?;

    let bytes = cache
        .get_or_fetch("object-v1".to_owned(), || async {
            // Replace this with your download or computation.
            Ok::<_, Infallible>(Bytes::from_static(b"hello world"))
        })
        .await?;

    assert_eq!(bytes.as_ref(), b"hello world");
    Ok(())
}
```

The `None` arguments disable metrics and give this cache its own I/O queues when disk is enabled.
Set disk capacity to `0` for memory-only operation. Pass `None` for I/O queues on
platforms other than Linux. `open`, `open_with_retention_scorer`, and
`open_with_retention_scorers` have the same signatures on every platform.

Memory capacity must be positive. Applications using zero memory capacity to
disable caching should bypass cache construction and fetch directly from the source.
`CacheConfig::new` rejects zero memory capacity.

## How lookups work

1. Memory hit: return the bytes.
2. Disk hit: read and checksum-verify the object, then promote it to memory.
3. Miss: run your callback, cache the result in memory, and queue a disk write.

`get_or_fetch` accepts a callback returning `Result<Bytes, E>` and charges payload
length in memory. `get_or_fetch_buffer` accepts `Result<AlignedBuffer, E>`, charges
the buffer's capacity, and converts it to shared `Bytes` without copying. Use
`cache.allocate_buffer(length)` to acquire a pooled buffer and fill its
`as_mut_slice()` before returning it. `AlignedBuffer` is reexported by `feuer`.
Both APIs have `_with_disk_fallthrough` variants for source fallback under disk
pressure.

Objects can be empty, and their lengths need not be known before fetching. Callback errors
must implement `Clone + Send + Sync + 'static`. Callbacks sharing work must be
interchangeable. Your application owns source retries and concurrency limits.

Use `get_object(key)` to check the cache without fetching. It returns `None` on a
miss and does not wait for an in-progress source callback.

## Before you use it

- Don't use the same key for different payloads. If you mutate an object, use a new key.
  Feuer has no invalidation API. 
- Keys are hashed to 128-bit identities without retaining the strings or checking for collisions.
- Disk caching and recovery are best-effort. Opening waits for metadata recovery.
- Changing capacity, disk format, or application payload version invalidates disk entries. Use
  `config.with_payload_version(n)` when changing your payload encoding.
- Give each cache its own backing file. The file is exclusively locked while
  open. Use separate directories or `config.with_file_name("images")`.

## Tuning and integration

### Eviction

Both tiers sample objects and evict the one with the lowest retention score.
The default scorer combines decayed demand with avoided retrieval cost per
charged byte. Its default cost model weights bytes. Configure a per-request cost
to account for fixed fetch overhead, or provide a scorer for computation costs.
Memory and disk can use different scorers.

See [eviction](eviction.md) for the formula, configuration examples, and trade-offs.

### Environment variables

`CacheConfig::new` reads these defaults. Builder methods override them per cache.

| Variable | Default | Purpose |
| --- | --- | --- |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Candidates per eviction. Must be positive. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `262144` | Requests across keys before demand weight halves. Must be positive. |
| `FEUER_IDLE_BUFFER_POOL_PERCENT` | `7` | Idle-buffer ceiling as a percentage of the memory target. Range 0–100. |

Values accept integers and size suffixes such as `8KiB` or `1.5 GiB`.
Binary suffixes use powers of 1024, decimal suffixes powers of 1000.
Invalid values cause `CacheConfig::new` to fail.

## Development

On Linux with io_uring and direct-I/O support:

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

On other platforms:

```console
cargo test -p feuer -p feuer-memory -p feuer-historian -p feuer-types
```

For internals, see the [architecture](architecture.html), [I/O queues](queues.html),
[disk format](format.md), and [implementation status](implementation-status.md).
[Experiments](experiments/README.md) record historical configurations and results.

## License

[MIT](LICENSE). Inspired in part by [Foyer](https://github.com/foyer-rs/foyer).
