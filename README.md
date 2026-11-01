# Feuer

![Feuer — cost-aware tiered cache, with a flaming horse logo](full_logo.svg?v=cfeda6a)

Scared of burning your S3 budget? Feuer will take care of that.

Feuer caches immutable objects in memory and on disk, with application callbacks for S3 and other sources.

## Design

Feuer aims to reduce the cost of serving requests, including source fetches, disk I/O,
and memory copies.

### Cost-aware eviction

Memory and disk can use different retention scorers. Scorers can account for fetch
or computation cost, request frequency and recency, and payload size.
Each layer samples objects and evicts the one with the lowest score.

### SSD I/O and buffer reuse

Disk storage uses Linux `io_uring` for asynchronous I/O and direct I/O to bypass
the kernel page cache.

- Aligned buffer reuse. Downloads can use the disk-read buffer pool. Fill a buffer
  and hand it to the cache without copying its payload.
- Batched writes. Small entries are packed together to reduce disk I/O.
- Separate read and write queues. Reads and writes have separate `io_uring` rings
  and worker threads, so they can progress concurrently.
- Source fallthrough under disk pressure. Applications can choose to fetch from
  the source when disk reads are backed up.
- In-flight fetch sharing. Concurrent misses share disk reads and source work.

## Runtime requirements

Disk caching requires Linux with `io_uring` and a filesystem supporting `O_DIRECT`
and `STATX_DIOALIGN`. Memory-only operation works on every supported platform.

Check that the deployment environment permits `io_uring`. Kernel settings and
container seccomp policies can block it, causing `Operation not permitted` errors.

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
        .open()
        .await?;

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

The example uses 64 MiB of memory and 1 GiB of disk. Memory capacity must be
positive. Omit `.disk_capacity(...)` for memory-only operation.
The default disk path is `feuer_cache/data`.

## How lookups work

`get_or_fetch(key, callback)` checks memory, then disk, then calls your callback:

1. Memory hit: return the bytes.
2. Disk hit: read and checksum-verify the object, then promote it to memory.
3. Miss: run the callback, cache the result in memory, and queue a disk write.

Use `get(key)` to read from memory or disk without fetching from the source.

Use `get_or_fetch_with_disk_fallthrough` to fetch from the source rather than wait
for a busy disk. This can use available network bandwidth at the cost of additional
source requests.

## Before you use it

- Use a new key when an object's contents change. Feuer has no invalidation API.
- Concurrent misses for the same key and callback error type share one fetch.
  Their callbacks must be interchangeable.
- Treat cached data as disposable. Disk writes and recovery are best-effort, so
  the source must remain able to supply objects.
- Changing disk capacity, disk format, or application payload version invalidates
  disk entries. Use `.payload_version(n)` when changing your payload encoding.
- Give each cache its own backing file. The file is exclusively locked while
  open. Use separate directories or `.file_name("images")`.

## Tuning and integration

### Eviction

The default scorer uses request frequency and recency with a byte-based cost
model. Customize it to account for fixed fetch overhead or computation costs.
See [eviction](eviction.md) for the formula, configuration examples, and trade-offs.

### Metrics and shared I/O

Use `.with_metrics(&registry)` to enable metrics. When several caches share an
SSD, use `.with_io_queues(queues)` to share their I/O workers. See the
[shared I/O example](feuer/examples/shared_io_objects.rs).

### Environment variables

`.open().await` reads these defaults. Builder setters override them per cache.

| Variable | Default | Purpose |
| --- | --- | --- |
| `FEUER_RECLAIM_SAMPLE_SIZE` | `64` | Candidates per eviction. |
| `FEUER_ACCESS_COUNT_HALF_LIFE` | `262144` | Requests across keys before demand weight halves. |
| `FEUER_HISTORY_ENTRY_LIMIT` | `2000000` | History entry-count cleanup threshold per cache. |

All values must be positive. Values accept integers and size suffixes such as `8KiB` or `1.5 GiB`.
Binary suffixes use powers of 1024, decimal suffixes powers of 1000.
See [eviction](eviction.md) for sampling and access-history behavior.

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
See [timed SSD replay](timed-replay.md) and the
[recovery benchmark](feuer-storage/recovery-benchmark.md) for performance validation.

## License

[MIT](LICENSE). Inspired in part by [Foyer](https://github.com/foyer-rs/foyer).
