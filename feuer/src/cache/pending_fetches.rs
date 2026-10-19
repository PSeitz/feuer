use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use tokio::sync::OnceCell;

use super::{ByteRange, FetchResult, ObjectKeyHash};

type FetchKey = (ObjectKeyHash, Option<ByteRange>, TypeId);
pub(super) type PendingFetches = Arc<Mutex<HashMap<FetchKey, Weak<dyn Any + Send + Sync>>>>;

/// One disk-or-source fetch, owned by its callers rather than the registry.
pub(super) struct PendingFetch<E> {
    key: FetchKey,
    pending: PendingFetches,
    pub(super) result: OnceCell<FetchResult<E>>,
}

impl<E: Send + Sync + 'static> PendingFetch<E> {
    pub(super) fn join(pending: &PendingFetches, key: ObjectKeyHash, requested: Option<ByteRange>) -> Arc<Self> {
        let key = (key, requested, TypeId::of::<E>());
        let mut entries = pending.lock().unwrap();
        if let Some(fetch) = entries.get(&key).and_then(Weak::upgrade) {
            return fetch.downcast().expect("fetch key includes its error type");
        }
        let fetch = Arc::new(Self {
            key,
            pending: pending.clone(),
            result: OnceCell::new(),
        });
        entries.insert(key, Arc::downgrade(&(fetch.clone() as Arc<dyn Any + Send + Sync>)));
        fetch
    }
}

impl<E> PendingFetch<E> {
    /// Remove this attempt without removing a newer attempt for the same request.
    pub(super) fn remove(&self) {
        let mut entries = self.pending.lock().unwrap();
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
        self.remove();
    }
}
