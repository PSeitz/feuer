use std::{fs::{File, OpenOptions}, os::unix::fs::OpenOptionsExt, sync::Arc, time::{Duration, Instant}};
use futures::{stream::FuturesUnordered, StreamExt};
use hdrhistogram::Histogram;
use serde_json::json;
#[path = "../../../../feuer-storage/src/error.rs"]
mod error;
pub use error::IoOperation;
#[path = "../../../../feuer-storage/src/uring.rs"]
mod uring;
use uring::IoQueueHandle;

const MIB: usize = 1024 * 1024;
const READ_BYTES: u64 = 16 * 1024 * MIB as u64;
const WRITE_BYTES: u64 = 8 * 1024 * MIB as u64;

fn cpu_seconds() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    assert_eq!(unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut t) }, 0);
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

struct Stats { count: u64, latency: Histogram<u64> }
impl Stats {
    fn new() -> Self { Self { count: 0, latency: Histogram::new(3).unwrap() } }
    fn json(&self, size: usize, seconds: f64) -> serde_json::Value {
        json!({"completed": self.count, "MB_s": self.count as f64 * size as f64 / seconds / 1e6,
            "iops": self.count as f64 / seconds, "p50_us": self.latency.value_at_quantile(0.50),
            "p95_us": self.latency.value_at_quantile(0.95), "p99_us": self.latency.value_at_quantile(0.99)})
    }
}

async fn request(queue: &IoQueueHandle, write: bool, offset: u64, size: usize, payload: &[u8]) -> (bool, Instant, u64) {
    let start = Instant::now();
    let result = queue.execute(offset, size, if write { payload } else { &[] }).await.unwrap();
    let end = Instant::now();
    if !write { assert_eq!(result.len(), size); }
    (write, end, end.duration_since(start).as_micros().max(1) as u64)
}

async fn phase(read: &IoQueueHandle, write: &IoQueueHandle, size: usize, rq: usize, wq: usize, seconds: u64) -> serde_json::Value {
    let payload = vec![0xa5; MIB];
    let mut random = 0x123456789abcdefu64;
    let mut write_index = 0u64;
    let mut offset = |is_write: bool| {
        if is_write {
            let result = READ_BYTES + (write_index % (WRITE_BYTES / MIB as u64)) * MIB as u64;
            write_index += 1;
            result
        } else {
            random ^= random << 13; random ^= random >> 7; random ^= random << 17;
            (random % (READ_BYTES / size as u64)) * size as u64
        }
    };
    let mut reads = Stats::new();
    let mut writes = Stats::new();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds);
    let cpu_start = cpu_seconds();
    let mut pending = FuturesUnordered::new();
    for i in 0..rq + wq {
        let w = i >= rq;
        pending.push(request(if w { write } else { read }, w, offset(w), if w { MIB } else { size }, &payload));
    }
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline.into()) => break,
            completion = pending.next() => {
                let (w, end, us) = completion.unwrap();
                if end >= deadline { break; }
                let stats = if w { &mut writes } else { &mut reads };
                stats.count += 1;
                stats.latency.record(us).unwrap();
                pending.push(request(if w { write } else { read }, w, offset(w), if w { MIB } else { size }, &payload));
            }
        }
    }
    let cpu = cpu_seconds() - cpu_start;
    // Drain, but exclude completions after the measurement deadline and drain CPU.
    while let Some((w, end, us)) = pending.next().await {
        if end < deadline {
            let stats = if w { &mut writes } else { &mut reads };
            stats.count += 1;
            stats.latency.record(us).unwrap();
        }
    }
    json!({"seconds": seconds, "cpu_pct": cpu / seconds as f64 * 100.0,
        "read": reads.json(size, seconds as f64), "write": writes.json(MIB, seconds as f64)})
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[2];
    let file = Arc::new(OpenOptions::new().read(true).write(true).custom_flags(libc::O_DIRECT).open(path).unwrap());
    assert_eq!(file.metadata().unwrap().len(), READ_BYTES + WRITE_BYTES);
    let lock = Arc::new(File::open(std::path::Path::new(path).parent().unwrap()).unwrap());
    let read = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Read).unwrap();
    let write = IoQueueHandle::new(file.clone(), lock, IoOperation::Write).unwrap();
    if args[1] == "smoke" {
        for size in [4096, 65536, MIB] {
            let payload: Vec<u8> = (0..size).map(|i| (i.wrapping_mul(37) % 251) as u8).collect();
            write.execute(READ_BYTES, size, &payload).await.unwrap();
            assert_eq!(read.execute(READ_BYTES, size, &[]).await.unwrap().as_ref(), payload.as_slice());
        }
        println!("smoke passed");
        return;
    }
    let size = args[3].parse().unwrap();
    let rq = args[4].parse().unwrap();
    let wq = args[5].parse().unwrap();
    phase(&read, &write, size, rq, wq, 5).await;
    let result = phase(&read, &write, size, rq, wq, 20).await;
    println!("{}", json!({"read_bytes": size, "read_qd": rq, "write_qd": wq, "result": result}));
    file.sync_all().unwrap();
}
