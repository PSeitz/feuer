//! Current writer round trips and trace replay through both cache configurations.
#![cfg(target_os = "linux")]

use std::{convert::Infallible, future::pending};

use bytes::Bytes;
use feuer::{AlignedBuffer, CacheConfig, ObjectKeyHash, TieredMemoryDiskCache};
use feuer_replay::{Result, decode_package, load_requests};

#[tokio::test]
async fn decodes_current_writer_and_preserves_hashes() -> Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let cache = TieredMemoryDiskCache::open(CacheConfig::new("unused", 0, 1 << 20)?, None, None)
        .await?
        .with_trace(move |package| std::future::ready(sender.send(package).map_err(std::io::Error::other)));
    let key = ObjectKeyHash::from("object");
    cache
        .get_or_fetch("object".to_owned(), || async {
            Ok::<_, Infallible>(Bytes::from_static(b"abcd"))
        })
        .await?;
    let hit = cache
        .get_or_fetch_buffer_by_hash(key, || async {
            Err::<AlignedBuffer, _>(std::sync::Arc::new(std::io::Error::other(
                "recorded hash must hit the string-key entry",
            )))
        })
        .await?;
    assert_eq!(hit.as_ref(), b"abcd");
    assert!(
        cache
            .get_or_fetch_buffer_by_hash(ObjectKeyHash(123), || async { Err::<AlignedBuffer, _>("source error") })
            .await
            .is_err()
    );

    let (started, ready) = tokio::sync::oneshot::channel();
    let canceled_cache = cache.clone();
    let task = tokio::spawn(async move {
        canceled_cache
            .get_or_fetch_buffer_by_hash(ObjectKeyHash(125), || async {
                started.send(()).unwrap();
                pending::<std::result::Result<AlignedBuffer, Infallible>>().await
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
        [0, 3, 0, 1, 0, 4, 0]
    );
    assert_eq!(events[0].object_key, key);
    assert_eq!(events[1].payload_len, Some(4));
    assert_eq!(events[2].object_key, key);
    assert_eq!(events[3].payload_len, Some(4));
    Ok(())
}

#[test]
fn replays_objects_with_and_without_disk() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("trace.feuer");
    let key = ObjectKeyHash::from("object");
    let mut package = b"FETR\x01\0\0\0".to_vec();
    for (kind, request_id, time_ns) in [(0u8, 1u64, 0u64), (3, 1, 1), (0, 2, 2)] {
        package.push(kind);
        package.extend(request_id.to_le_bytes());
        package.extend(time_ns.to_le_bytes());
        package.extend(key.0.to_le_bytes());
        package.extend(if kind == 3 { 2u64 } else { 0 }.to_le_bytes());
    }
    std::fs::write(&path, package)?;
    let access_trace = load_requests(&path, None)?;
    assert_eq!(access_trace.requests.len(), 2);
    assert_eq!(access_trace.incomplete, 1);
    assert_eq!(access_trace.outcomes, [0, 0, 0, 1, 0, 0]);
    assert_eq!(access_trace.requests[0].object_key, access_trace.requests[1].object_key);
    assert_eq!(access_trace.requests[0].object_key, key);
    assert_eq!(access_trace.requests[0].payload_len, 2);
    for disk_bytes in [0, 4 << 20] {
        let cache_dir = dir.path().join(format!("cache-{disk_bytes}"));
        let output = dir.path().join(format!("results-{disk_bytes}"));
        let replay = std::process::Command::new(env!("CARGO_BIN_EXE_replay"))
            .arg("--input")
            .arg(&path)
            .arg("--cache-dir")
            .arg(&cache_dir)
            .arg("--output")
            .arg(&output)
            .args(["--memory-bytes", "1024", "--disk-bytes", &disk_bytes.to_string()])
            .output()?;
        assert!(replay.status.success(), "{replay:?}");
        let summary = std::fs::read_to_string(output.join("summary.csv"))?;
        assert!(summary.lines().any(|line| line == "requests,2"));
        assert!(summary.lines().any(|line| line == "scheduling,task_per_arrival"));
        assert!(summary.lines().any(|line| line == "recorded_incomplete,1"));
        assert!(summary.lines().any(|line| line == format!("disk_bytes,{disk_bytes}")));
        let metrics = std::fs::read_to_string(output.join("settled.prom"))?;
        assert!(metrics.contains("replay_completed_total 2"));
        assert!(metrics.contains("replay_queued_requests 0"));
        assert!(metrics.contains("replay_active_requests 0"));
        assert!(metrics.contains("feuer_lookup_total{outcome=\"callback_error\"} 0"));
        assert_eq!(cache_dir.join("data").exists(), disk_bytes != 0);
    }
    Ok(())
}

#[tokio::test]
async fn decodes_empty_object_from_current_writer() -> Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let cache = TieredMemoryDiskCache::open(CacheConfig::new("unused", 0, 1 << 20)?, None, None)
        .await?
        .with_trace(move |package| std::future::ready(sender.send(package).map_err(std::io::Error::other)));
    assert!(cache.get("empty".to_owned()).await.is_none());
    cache
        .get_or_fetch("empty".to_owned(), || async { Ok::<_, Infallible>(Bytes::new()) })
        .await?;
    let hit = cache.get("empty".to_owned()).await.unwrap();
    assert!(hit.is_empty());
    drop(cache);
    let package = receiver.recv().await.ok_or("missing tail")?;
    let events = decode_package(&package)?.collect::<Result<Vec<_>>>()?;
    assert_eq!(
        events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        [0, 5, 0, 3, 0, 1]
    );
    assert_eq!(events[0].request_id, events[1].request_id);
    assert_eq!(events[1].payload_len, None);
    assert_eq!(events[3].payload_len, Some(0));
    assert_eq!(events[5].payload_len, Some(0));
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("objects.feuer");
    std::fs::write(&path, package)?;
    let access_trace = load_requests(&path, None)?;
    assert_eq!(access_trace.requests.len(), 3);
    assert!(
        access_trace
            .requests
            .iter()
            .all(|request| { request.object_key == ObjectKeyHash::from("empty") && request.payload_len == 0 })
    );
    assert_eq!(access_trace.incomplete, 0);
    assert_eq!(access_trace.outcomes, [0, 1, 0, 1, 0, 1]);
    for event in events {
        assert_eq!(event.object_key, ObjectKeyHash::from("empty"));
    }
    Ok(())
}
