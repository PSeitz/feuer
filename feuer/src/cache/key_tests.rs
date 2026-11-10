use std::convert::Infallible;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

// These keys intentionally do not implement Eq.
#[derive(Hash)]
struct Key<T> {
    split: T,
    range: Range<u64>,
}

#[tokio::test]
async fn structured_keys_fetch_and_lookup() {
    let cache = Cache::builder().memory_capacity(1_000_000).open().unwrap();
    let key = Key {
        split: "split",
        range: 100..200,
    };
    let bytes = cache
        .get_or_fetch(&key, || async { Ok::<_, Infallible>(Bytes::from_static(b"payload")) })
        .await
        .unwrap();
    let mut owned = Key {
        split: "split".to_owned(),
        range: 100..200,
    };
    assert_eq!(cache.get(&owned).await, Some(bytes.clone()));
    assert_eq!(
        cache
            .get_or_fetch(&owned, || async {
                Err::<Bytes, _>("cached objects must not be fetched again")
            })
            .await
            .unwrap(),
        bytes
    );
    owned.range.end += 1;
    assert_eq!(cache.get(&owned).await, None);
    owned.range = 100..200;
    owned.split = "other-split".to_owned();
    assert_eq!(cache.get(&owned).await, None);
}

#[tokio::test]
async fn structured_keys_share_concurrent_fetches() {
    let cache = Cache::builder().memory_capacity(1_000_000).open().unwrap();
    let key = Key {
        split: "split",
        range: 100..200,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let fetch = || {
        let calls = calls.clone();
        move || async move {
            calls.fetch_add(1, Ordering::Relaxed);
            tokio::task::yield_now().await;
            Ok::<_, Infallible>(Bytes::from_static(b"payload"))
        }
    };
    let (first, second) = tokio::join!(cache.get_or_fetch(&key, fetch()), cache.get_or_fetch(&key, fetch()));
    assert_eq!(first.unwrap(), second.unwrap());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn lookup_futures_release_keys_and_are_send() {
    fn require_send<T: Send>(value: T) -> T {
        value
    }
    let cache = Cache::builder().memory_capacity(1_000_000).open().unwrap();
    let lookup = {
        let key = ("key".to_owned(), std::rc::Rc::new(1u64));
        require_send(cache.get(&key))
    };
    assert_eq!(lookup.await, None);
    let fetch = {
        let mut key = ("key".to_owned(), std::rc::Rc::new(1u64));
        let fetch =
            require_send(cache.get_or_fetch(&key, || async { Ok::<_, Infallible>(Bytes::from_static(b"payload")) }));
        *std::rc::Rc::get_mut(&mut key.1).unwrap() = 2; // The fingerprint already captures the original value.
        fetch
    };
    let bytes = fetch.await.unwrap();
    let lookup = {
        let key = ("key".to_owned(), std::rc::Rc::new(1u64));
        require_send(cache.get_or_fetch_with_disk_fallthrough(
            &key,
            || false,
            || async { Err::<Bytes, _>("the original key must hit") },
        ))
    };
    assert_eq!(lookup.await.unwrap(), bytes);
}
