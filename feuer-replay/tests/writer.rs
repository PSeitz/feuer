//! Round-trip tests using the current cache trace writer and one replayer for both cache configurations.
#![cfg(target_os = "linux")]

use std::{convert::Infallible, future::pending};

use bytes::Bytes;
use feuer::{ByteRange, CacheConfig, Download, ObjectKeyHash, TieredMemoryDiskCache};
use feuer_replay::{Result, decode_package, load_requests};

#[tokio::test]
async fn decodes_current_writer_and_preserves_hashes_in_tiered_replay() -> Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let cache = TieredMemoryDiskCache::open(CacheConfig::new("unused", 0, 1 << 20)?, None, None)
        .await?
        .with_trace(move |package| std::future::ready(sender.send(package).map_err(std::io::Error::other)));
    let key = ObjectKeyHash::from("object");
    let range = ByteRange::new(10, 12)?;
    cache
        .get_or_fetch("object".to_owned(), range, || async {
            Ok::<_, Infallible>(Download::new(9, Bytes::from_static(b"abcd")).unwrap())
        })
        .await?;
    let hit = cache
        .get_or_fetch_by_hash(key, range, || async {
            Err::<Download, _>(std::io::Error::other("recorded hash must hit the string-key entry"))
        })
        .await?;
    assert_eq!(hit.as_ref(), b"bc");
    assert!(
        cache
            .get_or_fetch_by_hash(ObjectKeyHash(123), range, || async {
                Err::<Download, _>("source error")
            })
            .await
            .is_err()
    );
    assert!(
        cache
            .get_or_fetch_by_hash(ObjectKeyHash(124), range, || async {
                Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"a")).unwrap())
            })
            .await
            .is_err()
    );

    let (started, ready) = tokio::sync::oneshot::channel();
    let canceled_cache = cache.clone();
    let task = tokio::spawn(async move {
        canceled_cache
            .get_or_fetch_by_hash(ObjectKeyHash(125), range, || async {
                started.send(()).unwrap();
                pending::<std::result::Result<Download, Infallible>>().await
            })
            .await
    });
    ready.await?;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(cache);
    let package = receiver.recv().await.ok_or("writer did not deliver its tail")?;
    let events = decode_package(&package)?.collect::<Result<Vec<_>>>()?;
    assert_eq!(
        events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        [0, 3, 0, 1, 0, 4, 0, 5, 0]
    );
    assert_eq!(events[0].object_key, key);
    assert_eq!(events[1].downloaded, Some(ByteRange::new(9, 13)?));
    assert_eq!(events[2].object_key, key);

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("trace.feuer");
    std::fs::write(&path, package)?;
    let trace = load_requests(&path, None)?;
    assert_eq!(trace.requests.len(), 5);
    assert_eq!(trace.incomplete, 1);
    assert_eq!(trace.outcomes, [0, 1, 0, 1, 1, 1]);
    assert_eq!(trace.requests[0].object_key, key);
    for (disk_bytes, full_objects) in [(0, false), (4 << 20, false), (0, true), (4 << 20, true)] {
        let case = format!("{disk_bytes}-{full_objects}");
        let cache_dir = dir.path().join(format!("cache-{case}"));
        let output = dir.path().join(format!("results-{case}"));
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_replay"));
        command
            .arg("--input")
            .arg(&path)
            .arg("--cache-dir")
            .arg(&cache_dir)
            .arg("--output")
            .arg(&output)
            .args(["--memory-bytes", "1024", "--disk-bytes", &disk_bytes.to_string()]);
        if full_objects {
            command.arg("--full-objects");
        }
        let replay = command.output()?;
        assert!(replay.status.success(), "{replay:?}");
        let summary = std::fs::read_to_string(output.join("summary.csv"))?;
        assert!(summary.lines().any(|line| line == "requests,5"));
        assert!(summary.lines().any(|line| line == "scheduling,task_per_arrival"));
        assert!(summary.lines().any(|line| line == "recorded_incomplete,1"));
        assert!(
            summary
                .lines()
                .any(|line| line == format!("full_objects,{}", u8::from(full_objects)))
        );
        assert!(summary.lines().any(|line| line == format!("disk_bytes,{disk_bytes}")));
        let metrics = std::fs::read_to_string(output.join("settled.prom"))?;
        assert!(metrics.contains("replay_completed_total 5"));
        assert!(metrics.contains("replay_queued_requests 0"));
        assert!(metrics.contains("replay_active_requests 0"));
        assert!(metrics.contains("feuer_lookup_total{outcome=\"callback_error\"} 0"));
        assert!(metrics.contains("feuer_lookup_total{outcome=\"invalid_download\"} 0"));
        assert_eq!(cache_dir.join("data").exists(), disk_bytes != 0);
    }
    Ok(())
}

#[tokio::test]
async fn decodes_empty_whole_object_from_current_writer() -> Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let cache = TieredMemoryDiskCache::open(CacheConfig::new("unused", 0, 1 << 20)?, None, None)
        .await?
        .with_trace(move |package| std::future::ready(sender.send(package).map_err(std::io::Error::other)));
    cache
        .get_or_fetch_object("empty".to_owned(), || async { Ok::<_, Infallible>(Bytes::new()) })
        .await?;
    let hit = cache
        .get_or_fetch_by_hash(ObjectKeyHash::from("empty"), None, || async {
            Err::<Download, _>(std::io::Error::other("whole-object hash lookup must hit"))
        })
        .await?;
    assert!(hit.is_empty());
    drop(cache);
    let package = receiver.recv().await.ok_or("missing tail")?;
    let events = decode_package(&package)?.collect::<Result<Vec<_>>>()?;
    assert_eq!(events[0].requested, ByteRange::new(0, u64::MAX)?);
    assert_eq!(events[1].downloaded, Some(ByteRange::new(0, 0)?));
    assert_eq!(events[2].requested, ByteRange::new(0, u64::MAX)?);
    assert_eq!(events[3].kind, 1);
    Ok(())
}
