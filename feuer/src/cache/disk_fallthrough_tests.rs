use super::*;
use crate::test_metrics::{registry, value};
use tempfile::tempdir;

#[tokio::test]
async fn fallthrough_callbacks_are_not_called_without_disk_pressure() {
    for disk_capacity in [0, 4 << 20] {
        let directory = tempdir().unwrap();
        let (registry, backend) = registry();
        let cache = Cache::builder(CacheConfig::new(directory.path(), disk_capacity, 1 << 20).unwrap())
            .with_metrics(&backend)
            .open()
            .await
            .unwrap();
        let key = "download".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let source = Bytes::from_static(b"abcd");
        let bytes = cache
            .get_or_fetch_with_disk_fallthrough(
                key.clone(),
                || panic!("disk miss"),
                || async {
                    let mut buffer = cache.allocate_buffer(source.len()).unwrap();
                    buffer.as_mut_slice().copy_from_slice(&source);
                    Ok::<_, &str>(buffer)
                },
            )
            .await
            .unwrap();
        assert_eq!(bytes, "abcd");
        assert_eq!(
            cache
                .get_or_fetch_with_disk_fallthrough(
                    key.clone(),
                    || panic!("memory hit"),
                    || async { Err::<AlignedBuffer, _>("must not fetch") },
                )
                .await
                .unwrap(),
            bytes
        );
        assert!(cache.inner.memory.remove(&hash));

        if let Some(disk) = &cache.inner.disk {
            disk.insert_batch(vec![(hash, source.clone())]).await.unwrap();
            assert_eq!(
                cache
                    .get_or_fetch_with_disk_fallthrough(
                        key,
                        || panic!("queue has room"),
                        || async { Err::<AlignedBuffer, _>("must not fetch") },
                    )
                    .await
                    .unwrap(),
                bytes
            );
            assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", "disk_hit")]), 1.0);
        }
        assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", "callback")]), 1.0);
        assert_eq!(
            value(&registry, "feuer_lookup_total", &[("outcome", "memory_hit")]),
            1.0
        );

        let object = "whole object".to_owned();
        let bytes = Bytes::from_static(b"object");
        assert_eq!(
            cache
                .get_or_fetch_with_disk_fallthrough(
                    object.clone(),
                    || panic!("disk miss"),
                    || async { Ok::<_, &str>(bytes.clone()) },
                )
                .await
                .unwrap(),
            bytes
        );
        assert_eq!(
            cache
                .get_or_fetch_with_disk_fallthrough(
                    object,
                    || panic!("memory hit"),
                    || async { Err::<Bytes, _>("must not fetch") },
                )
                .await
                .unwrap(),
            bytes
        );
    }
}
