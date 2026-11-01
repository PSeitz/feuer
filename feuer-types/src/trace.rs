use std::{
    future::Future,
    io,
    mem::replace,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use bytes::Bytes;
use tokio::sync::mpsc;

const PACKAGE_HEADER: &[u8; 8] = b"FETR\x02\0\0\0";
const EVENT_BYTES: usize = 41;
const PACKAGE_TARGET_BYTES: usize = 8 * 1024 * 1024;

/// One event queue and clock shared by cache handles and their disk file.
#[derive(Clone)]
pub struct Trace {
    event_sender: mpsc::Sender<[u8; EVENT_BYTES]>,
    next_request_id: Arc<AtomicU64>,
    started_at: Instant,
}

impl Trace {
    pub fn new<F, Fut>(mut deliver_package: F) -> Self
    where
        F: FnMut(Bytes) -> Fut + Send + 'static,
        Fut: Future<Output = io::Result<()>> + Send + 'static,
    {
        let (event_sender, mut event_receiver) = mpsc::channel::<[u8; EVENT_BYTES]>(262_144);
        tokio::spawn(async move {
            let mut package = new_package();
            while let Some(event_bytes) = event_receiver.recv().await {
                package.extend_from_slice(&event_bytes);
                if package.len() >= PACKAGE_TARGET_BYTES {
                    deliver_package(Bytes::from(replace(&mut package, new_package()))).await?;
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

    /// Sends one event. Kind 0 assigns a request ID, other kinds use the supplied ID.
    pub async fn send(&self, kind: u8, request_id: u64, key_or_offset: u128, length: u64) -> u64 {
        let request_id = if kind == 0 {
            self.next_request_id.fetch_add(1, Ordering::Relaxed)
        } else {
            request_id
        };
        let mut event_bytes = [0; EVENT_BYTES];
        event_bytes[0] = kind;
        event_bytes[1..9].copy_from_slice(&request_id.to_le_bytes());
        event_bytes[9..17].copy_from_slice(&(self.started_at.elapsed().as_nanos() as u64).to_le_bytes());
        event_bytes[17..33].copy_from_slice(&key_or_offset.to_le_bytes());
        event_bytes[33..41].copy_from_slice(&length.to_le_bytes());
        let _ = self.event_sender.send(event_bytes).await;
        request_id
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
        let trace = Trace::new(move |package| {
            sender.send(package).unwrap();
            async { Ok(()) }
        });
        let key = 0x1234_5678_9abc_def0_1122_3344_5566_7788;
        let cases = [(1, 1234), (2, 1234), (3, 1234), (3, 0), (4, 0), (5, 0)];
        for (kind, length) in cases {
            let request_id = trace.send(0, 0, key, 0).await;
            trace.send(kind, request_id, key, length).await;
        }
        drop(trace);
        let package = receiver.recv().await.unwrap();
        assert_eq!(&package[..8], b"FETR\x02\0\0\0");
        assert_eq!(package.len(), 8 + cases.len() * 2 * 41);
        for (index, (kind, length)) in cases.into_iter().enumerate() {
            let start = &package[8 + index * 82..8 + index * 82 + 41];
            let end = &package[8 + index * 82 + 41..8 + (index + 1) * 82];
            assert_eq!(start[0], 0);
            assert_eq!(end[0], kind);
            for event in [start, end] {
                assert_eq!(&event[1..9], &(index as u64 + 1).to_le_bytes());
                assert_eq!(&event[17..33], &key.to_le_bytes());
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
