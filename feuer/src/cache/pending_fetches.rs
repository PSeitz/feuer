use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    ptr::addr_eq,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};

use super::{FetchResult, ObjectKeyHash};

type FetchKey = (ObjectKeyHash, TypeId);
pub(super) type PendingFetches = Arc<Mutex<HashMap<FetchKey, Weak<dyn Any + Send + Sync>>>>;
pub(super) type BatchResult<E> = Shared<BoxFuture<'static, Arc<Vec<FetchResult<E>>>>>;

enum Fetch<E> {
    One(Shared<BoxFuture<'static, FetchResult<E>>>),
    Batch { result: BatchResult<E>, index: usize },
}

/// A disk-or-source fetch owned by its callers. The registry holds weak references.
/// Any waiter can drive the fetch. Dropping the last caller cancels it.
/// Batch members reference one shared future rather than per-object source futures.
pub(super) struct PendingFetch<E> {
    key: FetchKey,
    registry: PendingFetches,
    result: OnceLock<Fetch<E>>,
}

impl<E: Clone + Send + Sync + 'static> PendingFetch<E> {
    pub(super) fn join(
        registry: &PendingFetches,
        key: ObjectKeyHash,
        create: impl FnOnce() -> Shared<BoxFuture<'static, FetchResult<E>>>,
    ) -> Arc<Self> {
        let mut entries = registry.lock().unwrap();
        let fetch = Self::entry(registry, &mut entries, key);
        if fetch.result.get().is_some() {
            // Discard unused callbacks outside the lock. Their captures can own other fetches.
            drop(entries);
            return fetch;
        }
        fetch.result.get_or_init(|| Fetch::One(create()));
        fetch
    }

    /// Claims and initializes all new members while holding the registry lock.
    /// The new group's future must depend only on these members, never existing fetches.
    /// This keeps overlapping groups independent even when callers request opposite key orders.
    pub(super) fn join_many(
        registry: &PendingFetches,
        keys: &[(usize, ObjectKeyHash)],
        create: impl FnOnce(Vec<(usize, ObjectKeyHash)>) -> BoxFuture<'static, Arc<Vec<FetchResult<E>>>>,
    ) -> (Vec<Arc<Self>>, Option<BatchResult<E>>) {
        let mut entries = registry.lock().unwrap();
        let mut claimed = HashSet::new();
        let mut new_keys = Vec::new();
        let mut new_fetches = Vec::new();
        let mut fetches = Vec::with_capacity(keys.len());
        for &(index, key) in keys {
            let fetch = Self::entry(registry, &mut entries, key);
            // A new entry can occur again in this batch before its result is initialized.
            if fetch.result.get().is_none() && claimed.insert(key) {
                new_keys.push((index, key));
                new_fetches.push(fetch.clone());
            }
            fetches.push(fetch);
        }
        let batch = if new_keys.is_empty() {
            None
        } else {
            let result = create(new_keys);
            let members: Vec<_> = new_fetches.iter().map(Arc::downgrade).collect();
            let result = async move {
                let result = result.await;
                // Remove every completed member even when its caller is still waiting on
                // an unrelated fetch. Weak references keep the group free of ownership cycles.
                for member in members {
                    if let Some(member) = member.upgrade() {
                        member.remove_from_registry();
                    }
                }
                result
            }
            .boxed()
            .shared();
            for (index, fetch) in new_fetches.into_iter().enumerate() {
                assert!(
                    fetch
                        .result
                        .set(Fetch::Batch {
                            result: result.clone(),
                            index
                        })
                        .is_ok()
                );
            }
            Some(result)
        };
        (fetches, batch)
    }

    fn entry(
        registry: &PendingFetches,
        entries: &mut HashMap<FetchKey, Weak<dyn Any + Send + Sync>>,
        key: ObjectKeyHash,
    ) -> Arc<Self> {
        let key = (key, TypeId::of::<E>());
        if let Some(fetch) = entries.get(&key).and_then(Weak::upgrade) {
            return fetch.downcast().expect("fetch key includes its error type");
        }
        let fetch = Arc::new(Self {
            key,
            registry: registry.clone(),
            result: OnceLock::new(),
        });
        entries.insert(key, Arc::downgrade(&(fetch.clone() as Arc<dyn Any + Send + Sync>)));
        fetch
    }

    pub(super) async fn wait(&self) -> FetchResult<E> {
        let result = match self.result.get().expect("initialized under the registry lock") {
            Fetch::One(result) => result.clone().await,
            Fetch::Batch { result, index } => result.clone().await[*index].clone(),
        };
        self.remove_from_registry();
        result
    }
}

impl<E> PendingFetch<E> {
    /// Remove this fetch from the registry. Existing waiters keep its result.
    /// Pointer identity protects a newer fetch for the same key and error type.
    pub(super) fn remove_from_registry(&self) {
        let mut entries = self.registry.lock().unwrap();
        if entries
            .get(&self.key)
            .is_some_and(|entry| addr_eq(entry.as_ptr(), self))
        {
            entries.remove(&self.key);
        }
    }
}

impl<E> Drop for PendingFetch<E> {
    fn drop(&mut self) {
        self.remove_from_registry();
    }
}
