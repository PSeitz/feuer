//! Independent caches on one SSD, sharing only low-level I/O queues.

#[cfg(target_os = "linux")]
use std::error::Error;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    use std::{convert::Infallible, env::args, path::PathBuf};

    use bytes::Bytes;
    use feuer::{Cache, CacheConfig, IoQueues};

    let directory = PathBuf::from(args().nth(1).unwrap_or_else(|| "cache".to_owned()));
    let io_queues = IoQueues::new()?;
    // Each cache uses a different file in the same directory, with its own capacities,
    // contents, eviction, buffer pool, file lock, and background-write queue.
    let images = Cache::open(
        CacheConfig::new(&directory, 1 << 30, 64 << 20)?.with_file_name("images")?,
        None,
        Some(io_queues.clone()),
    )
    .await?;
    let documents = Cache::open(
        CacheConfig::new(&directory, 2 << 30, 32 << 20)?.with_file_name("documents")?,
        None,
        Some(io_queues),
    )
    .await?;

    // The callbacks return immutable objects.
    // Replace these callbacks with downloads from the respective sources.
    // The same key can identify different content in independent caches.
    let image = images
        .get_or_fetch("object-v1".to_owned(), || async {
            Ok::<_, Infallible>(Bytes::from_static(b"image bytes"))
        })
        .await?;
    assert_eq!(image.as_ref(), b"image bytes");

    let document = documents
        .get_or_fetch("object-v1".to_owned(), || async {
            Ok::<_, Infallible>(Bytes::from_static(b"document bytes"))
        })
        .await?;
    assert_eq!(document.as_ref(), b"document bytes");

    let image = images.get("object-v1".to_owned()).await.unwrap();
    assert_eq!(image.as_ref(), b"image bytes");
    // Callbacks for the same key must return interchangeable objects.
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Shared disk I/O queues require Linux.");
}
