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
use feuer_types::{ByteRange, ObjectKeyHash};
use tokio::sync::mpsc;

use crate::metrics::LookupOutcome;

const MAGIC: &[u8; 8] = b"FETR\x01\0\0\0";
const EVENT_BYTES: usize = 65;
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
    pub(crate) async fn send_request_start(
        &self,
        object_key_hash: ObjectKeyHash,
        requested_range: ByteRange,
    ) -> Access<'_> {
        let mut start_bytes = [0; EVENT_BYTES];
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let elapsed_ns = self.epoch.elapsed().as_nanos() as u64;
        start_bytes[1..9].copy_from_slice(&request_id.to_le_bytes());
        start_bytes[9..17].copy_from_slice(&elapsed_ns.to_le_bytes());
        start_bytes[17..33].copy_from_slice(&object_key_hash.0.to_le_bytes());
        start_bytes[33..41].copy_from_slice(&requested_range.start().to_le_bytes());
        start_bytes[41..49].copy_from_slice(&requested_range.end().to_le_bytes());
        let _ = self.sender.send(start_bytes).await;
        Access {
            trace: self,
            bytes: start_bytes,
        }
    }
}

/// One cache access's encoded trace bytes, retained to report its lookup outcome.
pub(crate) struct Access<'a> {
    trace: &'a Trace,
    bytes: [u8; EVENT_BYTES],
}

impl Access<'_> {
    /// Fill in the lookup outcome, completion time, and downloaded range, then send the bytes.
    pub(crate) async fn send_outcome(mut self, outcome: LookupOutcome, downloaded_range: Option<ByteRange>) {
        self.bytes[0] = outcome as u8 + 1;
        self.bytes[9..17].copy_from_slice(&(self.trace.epoch.elapsed().as_nanos() as u64).to_le_bytes());
        if let Some(downloaded_range) = downloaded_range {
            self.bytes[49..57].copy_from_slice(&downloaded_range.start().to_le_bytes());
            self.bytes[57..65].copy_from_slice(&downloaded_range.end().to_le_bytes());
        }
        let _ = self.trace.sender.send(self.bytes).await;
    }
}

fn header() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(BATCH_BYTES + EVENT_BYTES);
    bytes.extend_from_slice(MAGIC);
    bytes
}
