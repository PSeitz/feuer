use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use tokio::sync::OnceCell;

use super::{FetchResult, ObjectKeyHash};

type FetchKey = (ObjectKeyHash, TypeId);
pub(super) type PendingFetches = Arc<Mutex<HashMap<FetchKey, Weak<dyn Any + Send + Sync>>>>;

/// A disk-or-source fetch owned by its callers. The registry holds weak references.
/// Cancelling the initializer lets a waiter initialize the result with its own callback.
pub(super) struct PendingFetch<E> {
    key: FetchKey,
    registry: PendingFetches,
    pub(super) result: OnceCell<FetchResult<E>>,
}

impl<E: Send + Sync + 'static> PendingFetch<E> {
    pub(super) fn join(registry: &PendingFetches, key: ObjectKeyHash) -> Arc<Self> {
        let key = (key, TypeId::of::<E>());
        let mut entries = registry.lock().unwrap();
        if let Some(fetch) = entries.get(&key).and_then(Weak::upgrade) {
            return fetch.downcast().expect("fetch key includes its error type");
        }
        let fetch = Arc::new(Self {
            key,
            registry: registry.clone(),
            result: OnceCell::new(),
        });
        entries.insert(key, Arc::downgrade(&(fetch.clone() as Arc<dyn Any + Send + Sync>)));
        fetch
    }
}

impl<E> PendingFetch<E> {
    /// Remove this fetch from the registry. Existing waiters keep its result.
    /// Pointer identity protects a newer fetch for the same key and error type.
    pub(super) fn remove_from_registry(&self) {
        let mut entries = self.registry.lock().unwrap();
        if entries
            .get(&self.key)
            .is_some_and(|entry| std::ptr::addr_eq(entry.as_ptr(), self))
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
