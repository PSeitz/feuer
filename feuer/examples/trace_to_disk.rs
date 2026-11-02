//! Write access-trace packages to files.

#[cfg(target_os = "linux")]
use std::error::Error;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    use std::{
        convert::Infallible,
        env::args,
        fs::{create_dir, write},
        io,
        path::PathBuf,
    };

    use bytes::Bytes;
    use feuer::Cache;
    use tokio::{sync::mpsc::unbounded_channel, task::spawn_blocking};

    let directory = PathBuf::from(args().nth(1).unwrap_or_else(|| "trace".to_owned()));
    // File numbers start at zero. Require a fresh directory.
    create_dir(&directory)?;
    let mut file_number = 0u64;
    let (write_result_sender, mut write_results) = unbounded_channel();
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .open()
        .await?
        .with_trace(move |package| {
            let path = directory.join(format!("{file_number:012}.feuer"));
            file_number += 1;
            let write_result_sender = write_result_sender.clone();
            async move {
                let write_result = spawn_blocking(move || write(path, &package))
                    .await
                    .map_err(io::Error::other)?;
                let _ = write_result_sender.send(write_result);
                Ok(())
            }
        });
    cache
        .get_or_fetch("object-v1", || async {
            Ok::<_, Infallible>(Bytes::from_static(b"data"))
        })
        .await?;
    // Dropping the cache triggers delivery. Wait for the expected package's write result.
    drop(cache);
    write_results
        .recv()
        .await
        .ok_or_else(|| io::Error::other("access-trace delivery stopped"))??;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Opening the cache requires Linux.");
}
