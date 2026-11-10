use futures_util::{StreamExt, stream};

use super::*;

impl Cache {
    /// Returns objects from memory, disk, or one shared batch callback.
    ///
    /// Results follow the input order. Every input records demand once, including duplicates.
    /// The callback receives the original indices of unique source misses in input order.
    /// It must return exactly one result per supplied index, in the same order. A different
    /// result count is a contract violation and panics. An empty miss list skips the callback.
    /// Successful results use the same memory accounting and admission as [`Self::get_or_fetch`].
    ///
    /// Matching keys and exact error types share work with scalar calls and other batches.
    /// Existing fetches are joined. Newly claimed keys check disk with at most five concurrent
    /// lookups, then forward all source misses together. There is one source future per batch,
    /// regardless of its size. Callbacks for matching requests must be interchangeable.
    ///
    /// Any waiter can drive the batch it joined. Cancelling its original caller preserves the
    /// callback for surviving waiters. The source batch is the cancellation unit. Dropping its
    /// last waiter cancels it, while one surviving member retains the whole source batch.
    /// Completed failures are not cached for later requests.
    pub async fn get_or_fetch_many<K, F, Fut, V, E>(
        &self,
        object_keys: &[K],
        callback: F,
    ) -> Vec<Result<Bytes, GetOrFetchError<E>>>
    where
        K: AsRef<str> + Sync,
        E: Clone + Send + Sync + 'static,
        F: FnOnce(Vec<usize>) -> Fut + Send + 'static,
        Fut: Future<Output = Vec<Result<V, E>>> + Send + 'static,
        V: Into<BytesAndMemoryCharge>,
    {
        let keys: Vec<_> = object_keys
            .iter()
            .map(|key| ObjectKeyHash::from(key.as_ref()))
            .collect();
        for key in &keys {
            self.inner.memory.access_histories().record_request(key);
        }
        let mut request_ids = Vec::with_capacity(keys.len());
        for &key in &keys {
            request_ids.push(self.trace_request_start(key).await);
        }
        let started = Instant::now();
        let mut results: Vec<_> = keys
            .iter()
            .map(|key| {
                self.inner
                    .memory
                    .get(key)
                    .map(|bytes| (LookupOutcome::MemoryHit, Ok(bytes)))
            })
            .collect();
        let missing: Vec<_> = keys
            .iter()
            .enumerate()
            .filter(|&(index, _)| results[index].is_none())
            .map(|(index, &key)| (index, key))
            .collect();
        let (fetches, batch) = PendingFetch::<E>::join_many(&self.inner.pending_fetches, &missing, |new_keys| {
            let cache = self.clone();
            async move { Arc::new(cache.fetch_many(new_keys, callback).await) }.boxed()
        });
        // Drive our source batch even if an earlier input joined an older, slow fetch.
        futures_util::future::join(
            async {
                for ((index, _), fetch) in missing.into_iter().zip(&fetches) {
                    results[index] = Some(fetch.wait().await);
                }
            },
            async {
                if let Some(batch) = batch {
                    batch.await;
                }
            },
        )
        .await;
        drop(fetches);
        let mut output = Vec::with_capacity(keys.len());
        for ((key, request_id), result) in keys.into_iter().zip(request_ids).zip(results) {
            let (outcome, result) = result.expect("each input is a memory hit or a joined fetch");
            let payload_length = result.as_ref().map(|bytes| bytes.len() as u64).unwrap_or(0);
            self.inner.metrics.record(outcome, started.elapsed(), payload_length);
            if let (Some(access_trace), Some(request_id)) = (&self.access_trace, request_id) {
                access_trace
                    .send_outcome(request_id, key, outcome, payload_length)
                    .await;
            }
            output.push(result);
        }
        output
    }

    async fn fetch_many<F, Fut, V, E>(&self, keys: Vec<(usize, ObjectKeyHash)>, callback: F) -> Vec<FetchResult<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce(Vec<usize>) -> Fut + Send + 'static,
        Fut: Future<Output = Vec<Result<V, E>>> + Send + 'static,
        V: Into<BytesAndMemoryCharge>,
    {
        let mut results: Vec<_> = stream::iter(keys.iter().copied())
            .map(|(_, key)| async move {
                let (outcome, bytes) = self.lookup(key, || false).await;
                bytes.map(|bytes| (outcome, Ok(bytes)))
            })
            .buffered(5)
            .collect()
            .await;
        let misses: Vec<_> = keys
            .iter()
            .enumerate()
            .filter(|&(slot, _)| results[slot].is_none())
            .map(|(slot, &(index, key))| (slot, index, key))
            .collect();
        if !misses.is_empty() {
            let fetched = callback(misses.iter().map(|&(_, index, _)| index).collect()).await;
            assert_eq!(
                fetched.len(),
                misses.len(),
                "batch callback must return one result per miss"
            );
            for ((slot, _, key), value) in misses.into_iter().zip(fetched) {
                results[slot] = Some(match value {
                    Ok(value) => (LookupOutcome::Callback, Ok(self.admit(key, value.into(), false))),
                    Err(error) => (LookupOutcome::CallbackError, Err(GetOrFetchError::Callback(error))),
                });
            }
        }
        results
            .into_iter()
            .map(|result| result.expect("every source miss was filled"))
            .collect()
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, target_os = "linux", feature = "disk"))]
mod disk_tests;
