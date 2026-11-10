use std::{
    any::{Any, TypeId},
    collections::HashMap,
    ptr::addr_eq,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};

use super::{FetchResult, ObjectKeyHash};

type FetchKey = (ObjectKeyHash, TypeId);
type RegistryEntry = (Weak<dyn Any + Send + Sync>, usize);
pub(super) type PendingFetches = Arc<Mutex<HashMap<FetchKey, RegistryEntry>>>;
type ResultSlot<E> = OnceLock<FetchResult<E>>;

/// One caller's object result and the producer that can fill it.
/// Result slots are shared only by callers requesting that object.
pub(super) struct PendingFetch<E> {
    producer: Arc<FetchProducer<E>>,
    result: Arc<ResultSlot<E>>,
}

/// A scalar or batch source operation. Every key stays discoverable until completion
/// or cancellation, even when only one member still has a caller. The producer owns
/// weak result slots so completed payloads live only as long as their own callers.
struct FetchProducer<E> {
    keys: Vec<FetchKey>,
    registry: PendingFetches,
    // Joining and completion always lock the registry before accessing members.
    members: Mutex<Vec<Weak<ResultSlot<E>>>>,
    future: OnceLock<Shared<BoxFuture<'static, ()>>>,
}

impl<E: Clone + Send + Sync + 'static> PendingFetch<E> {
    pub(super) fn join(
        registry: &PendingFetches,
        key: ObjectKeyHash,
        create: impl FnOnce() -> Shared<BoxFuture<'static, FetchResult<E>>>,
    ) -> Self {
        Self::join_many(registry, &[(0, key)], |keys| {
            debug_assert_eq!(keys.len(), 1);
            let result = create();
            async move { vec![result.await] }.boxed()
        })
        .pop()
        .unwrap()
    }

    /// Claim all new keys under one lock. A new producer depends only on its own
    /// keys, so overlapping requests cannot introduce producer dependency cycles.
    pub(super) fn join_many(
        registry: &PendingFetches,
        keys: &[(usize, ObjectKeyHash)],
        create: impl FnOnce(Vec<(usize, ObjectKeyHash)>) -> BoxFuture<'static, Vec<FetchResult<E>>>,
    ) -> Vec<Self> {
        let mut entries = registry.lock().unwrap();
        let mut new_keys = Vec::new();
        let mut new_slots = Vec::new();
        let mut claimed = HashMap::new();
        let mut fetches = Vec::with_capacity(keys.len());
        for &(index, key) in keys {
            let fetch_key = (key, TypeId::of::<E>());
            if let Some((producer, slot)) = entries
                .get(&fetch_key)
                .and_then(|(producer, slot)| producer.upgrade().map(|producer| (producer, *slot)))
            {
                let producer = producer
                    .downcast::<FetchProducer<E>>()
                    .expect("fetch key includes its error type");
                let result = {
                    let mut members = producer.members.lock().unwrap();
                    members[slot].upgrade().unwrap_or_else(|| {
                        let result = Arc::new(OnceLock::new());
                        members[slot] = Arc::downgrade(&result);
                        result
                    })
                };
                fetches.push(Ok(Self { producer, result }));
            } else {
                let slot = *claimed.entry(key).or_insert_with(|| {
                    new_keys.push((index, key));
                    new_slots.push(Arc::new(OnceLock::new()));
                    new_slots.len() - 1
                });
                fetches.push(Err(slot));
            }
        }
        let producer = if new_keys.is_empty() {
            None
        } else {
            let producer = Arc::new(FetchProducer {
                keys: new_keys.iter().map(|&(_, key)| (key, TypeId::of::<E>())).collect(),
                registry: registry.clone(),
                members: Mutex::new(new_slots.iter().map(Arc::downgrade).collect()),
                future: OnceLock::new(),
            });
            let source = create(new_keys);
            let weak = Arc::downgrade(&producer);
            let future = async move {
                let results = source.await;
                // A polling waiter owns the producer. Upgrade only after the source
                // completes to keep pending operations free of ownership cycles.
                let producer = weak.upgrade().expect("polling waiter owns producer");
                let members = {
                    let mut entries = producer.registry.lock().unwrap();
                    let members: Vec<_> = producer.members.lock().unwrap().iter().map(Weak::upgrade).collect();
                    // Snapshot recipients and remove entries under the join lock.
                    // Later callers start fresh lookups. Existing callers await publication.
                    producer.remove_entries(&mut entries);
                    members
                };
                assert_eq!(results.len(), members.len());
                // Payload destructors can own other fetches. Publish and release
                // unrequested results outside the registry and membership locks.
                for (member, result) in members.into_iter().zip(results) {
                    if let Some(member) = member {
                        assert!(member.set(result).is_ok());
                    }
                }
            }
            .boxed()
            .shared();
            assert!(producer.future.set(future).is_ok());
            for (slot, &key) in producer.keys.iter().enumerate() {
                entries.insert(
                    key,
                    (Arc::downgrade(&(producer.clone() as Arc<dyn Any + Send + Sync>)), slot),
                );
            }
            Some(producer)
        };
        // Unused callbacks and their captures must be dropped outside the lock.
        drop(entries);
        fetches
            .into_iter()
            .map(|fetch| {
                fetch.unwrap_or_else(|slot| Self {
                    producer: producer.as_ref().unwrap().clone(),
                    result: new_slots[slot].clone(),
                })
            })
            .collect()
    }

    pub(super) async fn wait(self) -> FetchResult<E> {
        self.producer
            .future
            .get()
            .expect("initialized under registry lock")
            .clone()
            .await;
        self.result.get().expect("producer published result").clone()
    }
}

impl<E> FetchProducer<E> {
    fn remove_entries(&self, entries: &mut HashMap<FetchKey, RegistryEntry>) {
        for key in &self.keys {
            // A completed caller can outlive a newer operation for the same key.
            if entries.get(key).is_some_and(|(entry, _)| addr_eq(entry.as_ptr(), self)) {
                entries.remove(key);
            }
        }
    }
}

impl<E> Drop for FetchProducer<E> {
    fn drop(&mut self) {
        self.remove_entries(&mut self.registry.lock().unwrap());
    }
}
