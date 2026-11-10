---
title: Quick start
sidebar_position: 2
---

## Add dependencies

To use the current development version, add these dependencies to `Cargo.toml`:

```toml
[dependencies]
feuer = { git = "https://github.com/PSeitz/feuer" }
bytes = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Create a memory cache

This example works on every supported platform:

```rust
use std::convert::Infallible;

use bytes::Bytes;
use feuer::Cache;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cache = Cache::builder()
        .memory_capacity(64 * 1024 * 1024)
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

Use `get` to check the cache without fetching from the source. Cloning a `Cache` shares its contents and in-flight work.

## Add disk caching on Linux

Set a disk capacity and backing directory when building the cache:

```rust
let cache = Cache::builder()
    .memory_capacity(64 * 1024 * 1024)
    .disk_capacity(1024 * 1024 * 1024)
    .directory("feuer_cache")
    .open()?;
```

Opening is synchronous and blocks until metadata recovery completes. Disk caching requires Linux, an entered Tokio runtime for background workers, `io_uring`, and a filesystem supporting direct I/O. Container security policies and host settings must permit `io_uring`. Each cache needs its own backing file.

Disk caching is disabled by default. Opening a disk cache waits for metadata recovery.

## Object identity and recovery

Each key must identify one immutable payload. Use a new key when the content changes. Feuer has no invalidation API.

Treat cached data as disposable. Disk writes and recovery are best-effort. Your source must remain able to supply objects after eviction or a restart.

See the [API reference](https://docs.rs/feuer) for configuration options and the [project README](https://github.com/PSeitz/feuer#runtime-requirements) for runtime requirements.
