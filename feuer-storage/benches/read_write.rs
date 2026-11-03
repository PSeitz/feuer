//! Histogram-driven reads alongside fixed 1-MiB writes through Feuer's DataFile.

use std::{
    collections::BTreeMap,
    fs,
    hint::black_box,
    io,
    num::NonZeroUsize,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use clap::Parser;
use feuer_memory::{AlignedBuffer, MemoryCache};
use feuer_storage::{DataFile, DataFileError, IoMetrics};
use tokio::task::JoinSet;

const MIB: u64 = 1024 * 1024;
const WRITERS: u64 = 8;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(about = "Read-size histogram benchmark using Feuer's pooled DataFile I/O")]
struct Args {
    /// Read sizes and relative weights, e.g. 4KiB:50,32KiB:30,1MiB:20. Defaults to the observed mix.
    #[arg(long, default_value = include_str!("read-sizes.txt").trim(), hide_default_value = true,
        value_delimiter = ',', value_parser = size_weight)]
    sizes: Vec<(usize, u64)>,
    /// Existing SSD directory. Only a new temporary directory is modified.
    #[arg(long)]
    directory: PathBuf,
    #[arg(long, default_value_t = 60)]
    seconds: u64,
    #[arg(long, default_value_t = 10)]
    warmup: u64,
    /// Concurrent whole DataFile reads. Production I/O limits remain unchanged.
    #[arg(long, default_value_t = 32)]
    readers: u64,
    #[arg(long, default_value_t = 65536)]
    read_mib: u64,
    /// At least eight MiB. Each writer owns a separate byte range.
    #[arg(long, default_value_t = 8192)]
    write_mib: u64,
    // Cargo passes this to harness-free benchmarks.
    #[arg(long, hide = true)]
    bench: bool,
}

struct Sample {
    write: bool,
    length: usize,
    latency_ns: u64,
}

fn size_weight(value: &str) -> std::result::Result<(usize, u64), String> {
    let (size, weight) = value.split_once(':').ok_or("expected size:weight, e.g. 32KiB:50")?;
    let size = size.trim().parse::<bytesize::ByteSize>().map_err(|e| e.to_string())?;
    let size = usize::try_from(size.as_u64()).map_err(|e| e.to_string())?;
    let weight = weight.trim().parse::<u64>().map_err(|e| e.to_string())?;
    if size == 0 || weight == 0 {
        return Err("size and weight must be positive".into());
    }
    Ok((size, weight))
}

fn cpu_seconds() -> io::Result<f64> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: time is writable storage for the process CPU clock result.
    if unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(time.tv_sec as f64 + time.tv_nsec as f64 / 1e9)
}

async fn measure(
    file: &DataFile,
    sizes: &Arc<Vec<(usize, u64)>>,
    payload: &Bytes,
    args: &Args,
    writes: bool,
) -> Result<(Vec<Sample>, f64, f64)> {
    let read_bytes = args.read_mib * MIB;
    let writer_chunks = args.write_mib / WRITERS;
    let readers = args.readers;
    let cpu_before = cpu_seconds()?;
    let started = Instant::now();
    let measured = started + Duration::from_secs(args.warmup);
    let end = measured + Duration::from_secs(args.seconds);
    let mut tasks = JoinSet::new();
    for worker in 0..readers + if writes { WRITERS } else { 0 } {
        let (file, sizes, payload) = (file.clone(), sizes.clone(), payload.clone());
        tasks.spawn(async move {
            let write = worker >= readers;
            let mut rng = fastrand::Rng::with_seed(20261102 + worker);
            let mut samples = Vec::new();
            let mut iteration = 0_u64;
            loop {
                let (offset, length) = if write {
                    let chunk = (worker - readers) * writer_chunks + iteration % writer_chunks;
                    (read_bytes + chunk * MIB, MIB as usize)
                } else {
                    let draw = rng.u64(..sizes.last().unwrap().1);
                    let length = sizes[sizes.partition_point(|&(_, count)| count <= draw)].0;
                    (rng.u64(..=(read_bytes - length as u64) / 4096) * 4096, length)
                };
                let before = Instant::now();
                if before >= end {
                    break;
                }
                if write {
                    file.write_at(offset, &payload).await?;
                } else {
                    black_box(file.read_at(offset, length).await?);
                }
                let finished = Instant::now();
                if finished >= measured && finished < end {
                    samples.push(Sample {
                        write,
                        length,
                        latency_ns: finished.duration_since(before).as_nanos() as u64,
                    });
                }
                iteration += 1;
            }
            Ok::<_, DataFileError>(samples)
        });
    }
    let mut batches = Vec::new();
    while let Some(batch) = tasks.join_next().await {
        batches.push(batch??);
    }
    let elapsed = started.elapsed().as_secs_f64();
    let cpu_percent = (cpu_seconds()? - cpu_before) / elapsed * 100.0;
    Ok((batches.into_iter().flatten().collect(), elapsed, cpu_percent))
}

fn report(case: &str, samples: &[Sample], seconds: f64) {
    let mut groups = BTreeMap::<_, Vec<&Sample>>::new();
    for sample in samples {
        groups.entry((None, sample.write)).or_default().push(sample);
        if !sample.write {
            groups.entry((Some(sample.length), false)).or_default().push(sample);
        }
    }
    for ((length, write), selected) in groups {
        let operation = if write { "write" } else { "read" };
        let label = length.map_or_else(|| "all".to_owned(), |length| format!("{length}B"));
        let bytes: u64 = selected.iter().map(|s| s.length as u64).sum();
        let mut latencies: Vec<_> = selected.iter().map(|s| s.latency_ns).collect();
        latencies.sort_unstable();
        let count = selected.len();
        let mean = latencies.iter().map(|ns| *ns as f64).sum::<f64>() / count as f64 / 1e6;
        let percentile = |p: usize| latencies[((count - 1) * p).div_ceil(100)] as f64 / 1e6;
        println!(
            "{case:<10} {operation:<5} {label:<14} {count:>9} {bytes:>12} {:>10.3} {:>10.3} {mean:>9.3} {:>9.3} {:>9.3} {:>9.3}",
            bytes as f64 / seconds / 1e6,
            count as f64 / seconds,
            percentile(50),
            percentile(95),
            percentile(99),
        );
    }
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.seconds == 0 || args.readers == 0 || args.read_mib == 0 || args.write_mib < WRITERS {
        return Err("seconds, readers, and read-mib must be positive, write-mib must be at least eight".into());
    }
    let capacity = args
        .read_mib
        .checked_add(args.write_mib)
        .and_then(|mib| mib.checked_mul(MIB))
        .ok_or("file capacity overflow")?;
    let mut total = 0_u64;
    let mut sizes = Vec::new();
    let mut weights = BTreeMap::<usize, u64>::new();
    for &(size, weight) in &args.sizes {
        if size as u64 > args.read_mib * MIB {
            return Err("read sizes must fit the read region".into());
        }
        total = total.checked_add(weight).ok_or("weight sum overflow")?;
        sizes.push((size, total));
        *weights.entry(size).or_default() += weight;
    }
    let sizes = Arc::new(sizes);
    println!(
        "Read mix: {} sizes, total weight {total}. {} read callers, {}s warmup, {}s measurement per case.",
        weights.len(),
        args.readers,
        args.warmup,
        args.seconds,
    );

    let weighted_bytes: u128 = weights
        .iter()
        .map(|(&size, &weight)| size as u128 * weight as u128)
        .sum();
    println!("Input read distribution:");
    println!(
        "{:<14} {:>12} {:>14} {:>14}",
        "Size", "Weight", "Operations %", "Bytes %"
    );
    for (size, weight) in weights {
        println!(
            "{:<14} {weight:>12} {:>14.6} {:>14.6}",
            format!("{size}B"),
            100.0 * weight as f64 / total as f64,
            100.0 * size as f64 * weight as f64 / weighted_bytes as f64,
        );
    }

    let temp = tempfile::Builder::new()
        .prefix("feuer-read-write-")
        .tempdir_in(&args.directory)?;
    let pool = MemoryCache::new(4 * 1024 * MIB).buffer_pool();
    let file =
        DataFile::open_with_io_queues(temp.path(), "data", capacity, IoMetrics::noop(), pool.clone(), None).await?;
    let mut buffer = AlignedBuffer::allocate_zeroed(NonZeroUsize::new(MIB as usize).unwrap())?;
    buffer.as_mut().fill(0x5a);
    let payload = buffer.into_bytes();
    eprintln!("Initializing {} MiB through DataFile", capacity / MIB);
    let mut initialization = JoinSet::new();
    for worker in 0..WRITERS {
        let (file, payload) = (file.clone(), payload.clone());
        initialization.spawn(async move {
            for offset in (worker * MIB..capacity).step_by((WRITERS * MIB) as usize) {
                file.write_at(offset, &payload).await?;
            }
            Ok::<_, DataFileError>(())
        });
    }
    while let Some(result) = initialization.join_next().await {
        result??;
    }
    fs::File::open(temp.path().join("data"))?.sync_all()?;

    println!(
        "{:<10} {:<5} {:<14} {:>9} {:>12} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9}",
        "Case", "I/O", "Size", "Count", "Bytes", "MB/s", "IOPS", "Mean ms", "p50 ms", "p95 ms", "p99 ms",
    );
    for (case, writes) in [("read-only", false), ("read-write", true)] {
        tokio::time::sleep(Duration::from_secs(5)).await;
        eprintln!("Running {case}");
        let (samples, elapsed, cpu_percent) = measure(&file, &sizes, &payload, &args, writes).await?;
        if !samples.iter().any(|s| !s.write) || (writes && !samples.iter().any(|s| s.write)) {
            return Err("measurement window contained no completed I/O for an enabled stream".into());
        }
        report(case, &samples, args.seconds as f64);
        println!(
            "{case}: CPU {cpu_percent:.1}% over {elapsed:.3}s including warmup/drain. Pool: {} used bytes, {} idle bytes.",
            pool.used_bytes(),
            pool.idle_bytes(),
        );
    }
    drop(file);
    temp.close()?;
    Ok(())
}
