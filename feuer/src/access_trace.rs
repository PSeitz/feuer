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

const PACKAGE_HEADER: &[u8; 8] = b"FETR\x02\0\0\0";
const EVENT_BYTES: usize = 41;
const PACKAGE_TARGET_BYTES: usize = 8 * 1024 * 1024;

/// An access trace shared by cache handles.
#[derive(Clone)]
pub(crate) struct AccessTrace {
    event_sender: mpsc::Sender<[u8; EVENT_BYTES]>,
    next_request_id: Arc<AtomicU64>,
    started_at: Instant,
}

impl AccessTrace {
    pub(crate) fn new<F, Fut>(mut deliver_package: F) -> Self
    where
        F: FnMut(Bytes) -> Fut + Send + 'static,
        Fut: Future<Output = io::Result<()>> + Send + 'static,
    {
        let (event_sender, mut event_receiver) = mpsc::channel::<[u8; EVENT_BYTES]>(65_536);
        tokio::spawn(async move {
            let mut package = new_package();
            while let Some(event_bytes) = event_receiver.recv().await {
                package.extend_from_slice(&event_bytes);
                if package.len() >= PACKAGE_TARGET_BYTES {
                    deliver_package(Bytes::from(std::mem::replace(&mut package, new_package()))).await?;
                }
            }
            if package.len() > PACKAGE_HEADER.len() {
                deliver_package(Bytes::from(package)).await?;
            }
            Ok::<_, io::Error>(())
        });
        Self {
            event_sender,
            next_request_id: Arc::new(AtomicU64::new(1)),
            started_at: Instant::now(),
        }
    }

    /// Send the request-start event to the access-trace queue and return its bytes for the outcome.
    pub(crate) async fn send_request_start(&self, object_key_hash: ObjectKeyHash) -> [u8; EVENT_BYTES] {
        let mut event_bytes = [0; EVENT_BYTES];
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let elapsed_ns = self.started_at.elapsed().as_nanos() as u64;
        event_bytes[1..9].copy_from_slice(&request_id.to_le_bytes());
        event_bytes[9..17].copy_from_slice(&elapsed_ns.to_le_bytes());
        event_bytes[17..33].copy_from_slice(&object_key_hash.0.to_le_bytes());
        let _ = self.event_sender.send(event_bytes).await;
        event_bytes
    }

    /// Send the lookup outcome, completion time, and payload length to the access-trace queue.
    /// Successful lookups supply the payload length. Empty objects, misses, and errors supply zero.
    pub(crate) async fn send_outcome(
        &self,
        mut event_bytes: [u8; EVENT_BYTES],
        outcome: LookupOutcome,
        payload_length: u64,
    ) {
        event_bytes[0] = outcome as u8 + 1;
        event_bytes[9..17].copy_from_slice(&(self.started_at.elapsed().as_nanos() as u64).to_le_bytes());
        event_bytes[33..41].copy_from_slice(&payload_length.to_le_bytes());
        let _ = self.event_sender.send(event_bytes).await;
    }
}

fn new_package() -> Vec<u8> {
    let mut package = Vec::with_capacity(PACKAGE_TARGET_BYTES + EVENT_BYTES);
    package.extend_from_slice(PACKAGE_HEADER);
    package
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn encodes_lookup_outcomes_and_payload_lengths() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let access_trace = AccessTrace::new(move |package| {
            sender.send(package).unwrap();
            async { Ok(()) }
        });
        let key = ObjectKeyHash(0x1234_5678_9abc_def0_1122_3344_5566_7788);
        let cases = [
            (LookupOutcome::MemoryHit, 1234, 1),
            #[cfg(target_os = "linux")]
            (LookupOutcome::DiskHit, 1234, 2),
            (LookupOutcome::Callback, 1234, 3),
            (LookupOutcome::Callback, 0, 3),
            (LookupOutcome::CallbackError, 0, 4),
            (LookupOutcome::Miss, 0, 5),
        ];
        for (outcome, length, _) in cases {
            let event_bytes = access_trace.send_request_start(key).await;
            access_trace.send_outcome(event_bytes, outcome, length).await;
        }
        drop(access_trace);
        let package = receiver.recv().await.unwrap();
        assert_eq!(&package[..8], b"FETR\x02\0\0\0");
        assert_eq!(package.len(), 8 + cases.len() * 2 * 41);
        for (index, (_, length, kind)) in cases.into_iter().enumerate() {
            let start = &package[8 + index * 82..8 + index * 82 + 41];
            let end = &package[8 + index * 82 + 41..8 + (index + 1) * 82];
            assert_eq!(start[0], 0);
            assert_eq!(end[0], kind);
            for event in [start, end] {
                assert_eq!(&event[1..9], &(index as u64 + 1).to_le_bytes());
                assert_eq!(&event[17..33], &key.0.to_le_bytes());
            }
            assert_eq!(&start[33..41], &0u64.to_le_bytes());
            assert_eq!(&end[33..41], &length.to_le_bytes());
            assert!(
                u64::from_le_bytes(end[9..17].try_into().unwrap())
                    >= u64::from_le_bytes(start[9..17].try_into().unwrap())
            );
        }
        assert!(receiver.recv().await.is_none());
    }
}
