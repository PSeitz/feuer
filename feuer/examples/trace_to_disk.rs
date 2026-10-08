//! A receiver persists Feuer's binary packages unchanged.

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{io, path::PathBuf};

    use bytes::Bytes;
    use feuer::{ByteRange, CacheConfig, Download, TieredMemoryDiskCache};

    let directory = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "trace".to_owned()));
    // Each receiver numbers its own files; require a fresh directory.
    std::fs::create_dir(&directory)?;
    let mut file_number = 0u64;
    let (sender, mut results) = tokio::sync::mpsc::unbounded_channel();
    let cache = TieredMemoryDiskCache::open(CacheConfig::new("unused", 0, 1 << 20).unwrap(), None, None)
        .await?
        .with_trace(move |package| {
            let path = directory.join(format!("{file_number:012}.feuer"));
            file_number += 1;
            let sender = sender.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || std::fs::write(path, &package))
                    .await
                    .map_err(io::Error::other)?;
                let _ = sender.send(result);
                Ok(())
            }
        });
    cache
        .get_or_fetch("object-v1".to_owned(), ByteRange::new(0, 4)?, || async {
            Download::new(0, Bytes::from_static(b"data"))
        })
        .await?;
    // This example expects one package; the receiver waits for its own write result.
    drop(cache);
    results
        .recv()
        .await
        .ok_or_else(|| io::Error::other("trace receiver stopped"))??;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Opening the public cache currently requires Linux.");
}
