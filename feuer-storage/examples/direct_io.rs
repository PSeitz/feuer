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
    let writer_counts: Vec<usize> = match args.next() {
        Some(value) => vec![value.parse()?],
        None => vec![0, 4],
    };
    assert!(
        writer_counts
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
            for &writers in &writer_counts {
                benchmark_concurrent_io(&file, read_size, readers, writers, seconds).await;
            }
        }
    }
    // Verify that read protection does not permanently cap write-only throughput.
    benchmark_concurrent_io(&file, MIB, 0, 64, seconds).await;
    drop(file); // drain and unlock before removing only the temporary benchmark directory
    Ok(())
}

/// Benchmarks concurrent reads and writes after warmup, printing throughput, latency, and CPU usage.
async fn benchmark_concurrent_io(file: &DataFile, read_size: usize, readers: usize, writers: usize, seconds: u64) {
    let measure_start = Instant::now() + Duration::from_secs(2);
    let deadline = measure_start + Duration::from_secs(seconds);
    let mut tasks = Vec::new();
    // Random reads and sequential 1-MiB writes use disjoint regions. Write lanes
    // are disjoint too, satisfying DataFile's caller-owned conflict prevention.
    let payload = Arc::new(Bytes::from(vec![0x5a; MIB]));
    for caller_index in 0..readers + writers {
        let file = file.clone();
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            let is_reader = caller_index < readers;
            let mut random_state = caller_index as u64 + 1;
            let mut measurements = IoMeasurements::default();
            let mut write_offset = 0;
            let lane_size = (CAPACITY - READ_SPACE_BYTES) / MIB as u64 / writers.max(1) as u64 * MIB as u64;
            while Instant::now() < deadline {
                let started = Instant::now();
                let transferred_bytes = if is_reader {
                    random_state ^= random_state << 13;
                    random_state ^= random_state >> 7;
                    random_state ^= random_state << 17;
                    let offset = (random_state % (READ_SPACE_BYTES / read_size as u64)) * read_size as u64;
                    let read_bytes = file.read_at(offset, read_size).await.unwrap();
                    assert_eq!(read_bytes.len(), read_size);
                    assert_eq!(read_bytes[0], 0x5a);
                    assert_eq!(read_bytes[read_size - 1], 0x5a);
                    read_size
                } else {
                    let offset = READ_SPACE_BYTES + (caller_index - readers) as u64 * lane_size + write_offset;
                    file.write_at(offset, &payload).await.unwrap();
                    write_offset = (write_offset + MIB as u64) % lane_size;
                    MIB
                };
                let finished = Instant::now();
                if started >= measure_start && finished <= deadline {
                    measurements.operations += 1;
                    measurements.bytes += transferred_bytes as u64;
                    // Sample API latency (including queueing) at 1/32 to bound harness overhead.
                    if is_reader && measurements.operations.is_multiple_of(32) {
                        measurements
                            .latency_samples_micros
                            .push(finished.duration_since(started).as_micros() as u64);
                    }
                }
            }
            (is_reader, measurements)
        }));
    }
    tokio::time::sleep_until(measure_start.into()).await;
    let cpu_start = process_cpu_seconds();
    let mut read_measurements = IoMeasurements::default();
    let mut write_measurements = IoMeasurements::default();
    for task in tasks {
        let (is_reader, measurements) = task.await.unwrap();
        let combined_measurements = if is_reader {
            &mut read_measurements
        } else {
            &mut write_measurements
        };
        combined_measurements.operations += measurements.operations;
        combined_measurements.bytes += measurements.bytes;
        combined_measurements
            .latency_samples_micros
            .extend(measurements.latency_samples_micros);
    }
    let cpu_percent = (process_cpu_seconds() - cpu_start) / measure_start.elapsed().as_secs_f64() * 100.0;
    read_measurements.latency_samples_micros.sort_unstable();
    let read_latency_percentile = |percentile: usize| {
        read_measurements
            .latency_samples_micros
            .get(read_measurements.latency_samples_micros.len() * percentile / 100)
            .copied()
            .unwrap_or(0)
    };
    println!(
        "{read_size},{readers},{writers},{seconds},{:.1},{:.1},{:.0},{},{},{:.1}",
        read_measurements.bytes as f64 / seconds as f64 / 1e6,
        write_measurements.bytes as f64 / seconds as f64 / 1e6,
        read_measurements.operations as f64 / seconds as f64,
        read_latency_percentile(50),
        read_latency_percentile(99),
        cpu_percent
    );
}

/// Returns this process's accumulated user and system CPU time in seconds.
fn process_cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: usage is writable and getrusage initializes it on success.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) }, 0);
    // SAFETY: getrusage succeeded above.
    let usage = unsafe { usage.assume_init() };
    let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1e6;
    seconds(usage.ru_utime) + seconds(usage.ru_stime)
}
