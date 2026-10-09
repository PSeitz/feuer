//! Independent whole-object caches on one SSD, sharing only low-level I/O queues.

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{convert::Infallible, path::PathBuf};

    use bytes::Bytes;
    use feuer::{CacheConfig, IoQueues, TieredMemoryDiskCache};

    let directory = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "cache".to_owned()));
    let io_queues = IoQueues::new()?;
    // Each cache uses a different file in the same directory, with its own capacities,
    // contents, eviction, buffer pool, file lock, and background-write queue.
    let images = TieredMemoryDiskCache::open(
        CacheConfig::new(&directory, 1 << 30, 64 << 20)?.with_file_name("images")?,
        None,
        Some(io_queues.clone()),
    )
    .await?;
    let documents = TieredMemoryDiskCache::open(
        CacheConfig::new(&directory, 2 << 30, 32 << 20)?.with_file_name("documents")?,
        None,
        Some(io_queues),
    )
    .await?;

    // The callbacks return complete immutable objects. Their lengths are not needed.
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
