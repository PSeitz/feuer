use std::{
    future::Future,
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use bytes::Bytes;
use feuer_types::ObjectKeyHash;
use tokio::sync::mpsc;

use crate::metrics::LookupOutcome;

const MAGIC: &[u8; 8] = b"FETR\x02\0\0\0";
const EVENT_BYTES: usize = 41;
const BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Trace {
    sender: mpsc::Sender<[u8; EVENT_BYTES]>,
    next_id: Arc<AtomicU64>,
    epoch: Instant,
}

impl Trace {
    pub(crate) fn new<F, Fut>(mut callback: F) -> Self
    where
        F: FnMut(Bytes) -> Fut + Send + 'static,
        Fut: Future<Output = io::Result<()>> + Send + 'static,
    {
        let (sender, mut receiver) = mpsc::channel::<[u8; EVENT_BYTES]>(65_536);
        tokio::spawn(async move {
            let mut batch = header();
            while let Some(event) = receiver.recv().await {
                batch.extend_from_slice(&event);
                if batch.len() >= BATCH_BYTES {
                    callback(Bytes::from(std::mem::replace(&mut batch, header()))).await?;
                }
            }
            if batch.len() > MAGIC.len() {
                callback(Bytes::from(batch)).await?;
            }
            Ok::<_, io::Error>(())
        });
        Self {
            sender,
            next_id: Arc::new(AtomicU64::new(1)),
            epoch: Instant::now(),
        }
    }

    /// Send the request start to the trace queue and retain its bytes for the outcome.
    pub(crate) async fn send_request_start(&self, object_key_hash: ObjectKeyHash) -> Access<'_> {
        let mut start_bytes = [0; EVENT_BYTES];
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let elapsed_ns = self.epoch.elapsed().as_nanos() as u64;
        start_bytes[1..9].copy_from_slice(&request_id.to_le_bytes());
        start_bytes[9..17].copy_from_slice(&elapsed_ns.to_le_bytes());
        start_bytes[17..33].copy_from_slice(&object_key_hash.0.to_le_bytes());
        let _ = self.sender.send(start_bytes).await;
        Access {
            trace: self,
            bytes: start_bytes,
        }
    }
}

/// A cache access's encoded trace bytes, retained to report its lookup outcome.
pub(crate) struct Access<'a> {
    trace: &'a Trace,
    bytes: [u8; EVENT_BYTES],
}

impl Access<'_> {
    /// Fill in the lookup outcome, completion time, and successful object length, then send the bytes.
    pub(crate) async fn send_outcome(mut self, outcome: LookupOutcome, payload_length: Option<u64>) {
        self.bytes[0] = outcome as u8 + 1;
        self.bytes[9..17].copy_from_slice(&(self.trace.epoch.elapsed().as_nanos() as u64).to_le_bytes());
        if let Some(payload_length) = payload_length {
            self.bytes[33..41].copy_from_slice(&payload_length.to_le_bytes());
        }
        let _ = self.trace.sender.send(self.bytes).await;
    }
}

fn header() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(BATCH_BYTES + EVENT_BYTES);
    bytes.extend_from_slice(MAGIC);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_whole_object_outcomes() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let trace = Trace::new(move |batch| {
            sender.send(batch).unwrap();
            async { Ok(()) }
        });
        let key = ObjectKeyHash(0x1234_5678_9abc_def0_1122_3344_5566_7788);
        let cases = [
            (LookupOutcome::MemoryHit, Some(1234), 1),
            #[cfg(target_os = "linux")]
            (LookupOutcome::DiskHit, Some(1234), 2),
            (LookupOutcome::Callback, Some(1234), 3),
            (LookupOutcome::Callback, Some(0), 3),
            (LookupOutcome::CallbackError, None, 4),
            (LookupOutcome::Miss, None, 5),
        ];
        for (outcome, length, _) in cases {
            trace.send_request_start(key).await.send_outcome(outcome, length).await;
        }
        drop(trace);
        let batch = receiver.recv().await.unwrap();
        assert_eq!(&batch[..8], b"FETR\x02\0\0\0");
        assert_eq!(batch.len(), 8 + cases.len() * 2 * 41);
        for (index, (_, length, kind)) in cases.into_iter().enumerate() {
            let start = &batch[8 + index * 82..8 + index * 82 + 41];
            let end = &batch[8 + index * 82 + 41..8 + (index + 1) * 82];
            assert_eq!(start[0], 0);
            assert_eq!(end[0], kind);
            for event in [start, end] {
                assert_eq!(&event[1..9], &(index as u64 + 1).to_le_bytes());
                assert_eq!(&event[17..33], &key.0.to_le_bytes());
            }
            assert_eq!(&start[33..41], &0u64.to_le_bytes());
            assert_eq!(&end[33..41], &length.unwrap_or(0).to_le_bytes());
            assert!(
                u64::from_le_bytes(end[9..17].try_into().unwrap())
                    >= u64::from_le_bytes(start[9..17].try_into().unwrap())
            );
        }
        assert!(receiver.recv().await.is_none());
    }
}
