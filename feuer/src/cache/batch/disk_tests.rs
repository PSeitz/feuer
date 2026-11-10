use super::*;
use crate::test_metrics::{registry, value};

#[tokio::test]
async fn disk_hits_promote_and_all_source_misses_are_admitted_together() {
    let directory = tempfile::tempdir().unwrap();
    let (registry, backend) = registry();
    let cache = Cache::builder()
        .directory(directory.path())
        .disk_capacity(8 << 20)
        .memory_capacity(8 << 20)
        .with_metrics(&backend)
        .open()
        .await
        .unwrap();
    let disk = cache.inner.disk.as_ref().unwrap();
    let disk_key = ObjectKeyHash::from("disk");
    disk.insert_batch(vec![(disk_key, Bytes::from_static(b"disk payload"))])
        .await
        .unwrap();
    disk.flush().await.unwrap();
    cache
        .inner
        .memory
        .insert(ObjectKeyHash::from("memory"), Bytes::from_static(b"m"), 1);
    let mut keys = vec!["memory".to_string(), "disk".to_string(), "disk".to_string()];
    keys.extend((0..12).map(|index| format!("source-{index}")));
    let fetched = cache
        .get_or_fetch_many(&keys, |indices| async move {
            assert_eq!(indices, (3..15).collect::<Vec<_>>());
            indices
                .into_iter()
                .map(|index| Ok::<_, &str>(Bytes::from(vec![index as u8; 3])))
                .collect()
        })
        .await;
    assert_eq!(fetched[0].as_ref().unwrap(), "m");
    assert_eq!(fetched[1].as_ref().unwrap(), "disk payload");
    assert_eq!(fetched[1], fetched[2]);
    assert_eq!(
        fetched[1].as_ref().unwrap().as_ptr(),
        fetched[2].as_ref().unwrap().as_ptr()
    );
    assert_eq!(
        cache.inner.memory.get(&disk_key).unwrap().as_ptr(),
        fetched[1].as_ref().unwrap().as_ptr()
    );
    assert_eq!(cache.inner.memory.access_histories().request_count(), 15);
    for (outcome, count) in [("memory_hit", 1.0), ("disk_hit", 2.0), ("callback", 12.0)] {
        assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", outcome)]), count);
    }
    // The unique disk key is read once even though it occurs twice in the batch.
    assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", "hit")]), 1.0);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while keys[3..]
            .iter()
            .any(|key| !disk.contains(&ObjectKeyHash::from(key.as_str())))
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    disk.flush().await.unwrap();
    for key in &keys {
        cache.inner.memory.remove(&ObjectKeyHash::from(key.as_str()));
    }
    let disk_hits = cache
        .get_or_fetch_many::<_, _, _, Bytes, &str>(&keys[1..], |_| async {
            panic!("admitted source results must be on disk")
        })
        .await;
    assert_eq!(disk_hits, fetched[1..]);
}
