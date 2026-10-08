//! Independent whole-object caches on one SSD, sharing only low-level I/O queues.

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{convert::Infallible, path::PathBuf};

    use bytes::Bytes;
    use feuer::{CacheConfig, IoQueues, TieredMemoryDiskCache};
    use mixtrics::metrics::BoxedRegistry;

    let directory = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "cache".to_owned()));
    let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
    let io_queues = IoQueues::new()?;
    // Both directories must be on the same SSD. Each cache has its own capacities,
    // contents, eviction, buffer pool, directory lock, and background-write queue.
    let images = TieredMemoryDiskCache::open_with_io_queues(
        CacheConfig::new(directory.join("images"), 1 << 30, 64 << 20)?,
        &registry,
        io_queues.clone(),
    )
    .await?;
    let documents = TieredMemoryDiskCache::open_with_io_queues(
        CacheConfig::new(directory.join("documents"), 2 << 30, 32 << 20)?,
        &registry,
        io_queues,
    )
    .await?;

    // The callbacks return complete immutable objects; their lengths are not needed.
    // Replace these callbacks with downloads from the respective sources.
    // The same key can identify different content in independent caches.
    let image = images
        .get_or_fetch_object("object-v1".to_owned(), || async {
            Ok::<_, Infallible>(Bytes::from_static(b"image bytes"))
        })
        .await?;
    assert_eq!(image.as_ref(), b"image bytes");

    let document = documents
        .get_or_fetch_object("object-v1".to_owned(), || async {
            Ok::<_, Infallible>(Bytes::from_static(b"document bytes"))
        })
        .await?;
    assert_eq!(document.as_ref(), b"document bytes");

    let image = images
        .get_or_fetch_object("object-v1".to_owned(), || async {
            Err::<Bytes, _>(std::io::Error::other("cached object must not be downloaded again"))
        })
        .await?;
    assert_eq!(image.as_ref(), b"image bytes");
    // Do not mix get_or_fetch_object and get_or_fetch for the same key within a cache.
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Shared disk I/O queues require Linux.");
}
