use super::*;
use crate::test_metrics::{registry, value};

#[tokio::test]
async fn fallthrough_callbacks_are_not_called_without_disk_pressure() {
    for disk_capacity in [0, 4 << 20] {
        let directory = tempfile::tempdir().unwrap();
        let (registry, backend) = registry();
        let cache = TieredMemoryDiskCache::open(
            CacheConfig::new(directory.path(), disk_capacity, 1 << 20).unwrap(),
            Some(&backend),
            None,
        )
        .await
        .unwrap();
        let key = "download".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let source = Download::new(Bytes::from_static(b"abcd"));
        let bytes = cache
            .get_or_fetch_with_disk_fallthrough(
                key.clone(),
                || panic!("disk miss"),
                || async { Ok::<_, &str>(source.clone()) },
            )
            .await
            .unwrap();
        assert_eq!(bytes, "abcd");
        assert_eq!(
            cache
                .get_or_fetch_with_disk_fallthrough(
                    key.clone(),
                    || panic!("memory hit"),
                    || async { Err::<Download, _>("must not fetch") },
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
                        || async { Err::<Download, _>("must not fetch") },
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
                .get_or_fetch_object_with_disk_fallthrough(
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
                .get_or_fetch_object_with_disk_fallthrough(
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
