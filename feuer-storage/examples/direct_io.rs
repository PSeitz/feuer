//! Small direct-I/O smoke benchmark, not a cache-engine comparison.
//! Run: cargo run -p feuer-storage --release --example direct_io -- /mnt/local-ssd [seconds] [write_callers]
//! Uses and removes a fresh 8-GiB temporary directory under the supplied mount.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use feuer_storage::{DataFile, IoMetrics};

const MIB: usize = 1024 * 1024;
const CAPACITY: u64 = 8 * 1024 * MIB as u64;
const READ_SPACE_BYTES: u64 = CAPACITY * 3 / 4;

/// Completed I/O counts and sampled latencies for one measurement interval.
#[derive(Default)]
struct IoMeasurements {
    operations: u64,
    bytes: u64,
    latency_samples_micros: Vec<u64>,
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().expect("provide a directory on the target filesystem");
    let seconds: u64 = args.next().unwrap_or_else(|| "5".into()).parse()?;
    assert!(seconds > 0);
    let write_callers: Vec<usize> = match args.next() {
        Some(value) => vec![value.parse()?],
        None => vec![0, 4],
    };
    assert!(
        write_callers
            .iter()
            .all(|&count| count <= ((CAPACITY - READ_SPACE_BYTES) / MIB as u64) as usize)
    );
    let temp = tempfile::Builder::new()
        .prefix("feuer-direct-bench-")
        .tempdir_in(root)?;
    let registry: mixtrics::metrics::BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
    let file = DataFile::open(temp.path(), CAPACITY, IoMetrics::new(&registry)).await?;
    eprintln!("Initializing 8 GiB (direct writes), driver QD64, four Tokio workers");
    let payload = Bytes::from(vec![0x5a; MIB]);
    for offset in (0..CAPACITY).step_by(MIB) {
        file.write_at(offset, &payload).await?;
    }
    println!(
        "read_bytes,read_callers,write_callers,seconds,read_MB_s,write_MB_s,read_iops,read_p50_us,read_p99_us,cpu_percent"
    );
    for read_size in [4096, 65536, MIB] {
        for readers in [1, 32, 64, 128] {
            for &writers in &write_callers {
                run_case(&file, read_size, readers, writers, seconds).await;
            }
        }
    }
    // Verify that read protection does not permanently cap write-only throughput.
    run_case(&file, MIB, 0, 64, seconds).await;
    drop(file); // drain and unlock before removing only the temporary benchmark directory
    Ok(())
}

async fn run_case(file: &DataFile, read_size: usize, readers: usize, writers: usize, seconds: u64) {
    let measure_start = Instant::now() + Duration::from_secs(2);
    let deadline = measure_start + Duration::from_secs(seconds);
    let mut tasks = Vec::new();
    // Random reads and sequential 1-MiB writes use disjoint regions. Write lanes
    // are disjoint too, avoiding overlap serialization as an artificial limit.
    let payload = Arc::new(Bytes::from(vec![0x5a; MIB]));
    for id in 0..readers + writers {
        let file = file.clone();
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            let reading = id < readers;
            let mut random = id as u64 + 1;
            let mut measurements = IoMeasurements::default();
            let mut write_offset = 0;
            let lane_size = (CAPACITY - READ_SPACE_BYTES) / MIB as u64 / writers.max(1) as u64 * MIB as u64;
            while Instant::now() < deadline {
                let started = Instant::now();
                let bytes = if reading {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    let offset = (random % (READ_SPACE_BYTES / read_size as u64)) * read_size as u64;
                    let value = file.read_at(offset, read_size).await.unwrap();
                    assert_eq!(value.len(), read_size);
                    assert_eq!(value[0], 0x5a);
                    assert_eq!(value[read_size - 1], 0x5a);
                    read_size
                } else {
                    let offset = READ_SPACE_BYTES + (id - readers) as u64 * lane_size + write_offset;
                    file.write_at(offset, &payload).await.unwrap();
                    write_offset = (write_offset + MIB as u64) % lane_size;
                    MIB
                };
                let finished = Instant::now();
                if started >= measure_start && finished <= deadline {
                    measurements.operations += 1;
                    measurements.bytes += bytes as u64;
                    // Sample API latency (including queueing) at 1/32 to bound harness overhead.
                    if reading && measurements.operations.is_multiple_of(32) {
                        measurements
                            .latency_samples_micros
                            .push(finished.duration_since(started).as_micros() as u64);
                    }
                }
            }
            (reading, measurements)
        }));
    }
    tokio::time::sleep_until(measure_start.into()).await;
    let cpu_start = cpu_seconds();
    let mut read = IoMeasurements::default();
    let mut write = IoMeasurements::default();
    for task in tasks {
        let (reading, measurements) = task.await.unwrap();
        let total = if reading { &mut read } else { &mut write };
        total.operations += measurements.operations;
        total.bytes += measurements.bytes;
        total.latency_samples_micros.extend(measurements.latency_samples_micros);
    }
    let cpu_percent = (cpu_seconds() - cpu_start) / measure_start.elapsed().as_secs_f64() * 100.0;
    read.latency_samples_micros.sort_unstable();
    let percentile = |p: usize| {
        read.latency_samples_micros
            .get(read.latency_samples_micros.len() * p / 100)
            .copied()
            .unwrap_or(0)
    };
    println!(
        "{read_size},{readers},{writers},{seconds},{:.1},{:.1},{:.0},{},{},{:.1}",
        read.bytes as f64 / seconds as f64 / 1e6,
        write.bytes as f64 / seconds as f64 / 1e6,
        read.operations as f64 / seconds as f64,
        percentile(50),
        percentile(99),
        cpu_percent
    );
}

fn cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: usage is writable and getrusage initializes it on success.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) }, 0);
    // SAFETY: getrusage succeeded above.
    let usage = unsafe { usage.assume_init() };
    let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1e6;
    seconds(usage.ru_utime) + seconds(usage.ru_stime)
}
