//! Histogram-driven reads alongside fixed 1-MiB writes through Feuer's DataFile.

use std::{
    env, fs,
    hint::black_box,
    io::{self, BufWriter, Write},
    num::NonZeroUsize,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use clap::Parser;
use feuer_memory::{AlignedBuffer, MemoryCache};
use feuer_storage::{DataFile, DataFileError, IoMetrics};
use tokio::{task::JoinSet, time::sleep_until};

const MIB: u64 = 1024 * 1024;
const WRITERS: u64 = 8;
const SIZE_BANDS: [(&str, usize, usize); 5] = [
    ("<=4KiB", 1, 4096),
    (">4..64KiB", 4097, 65536),
    (">64KiB..1MiB", 65537, MIB as usize),
    (">1..8MiB", MIB as usize + 1, 8 * MIB as usize),
    (">8MiB", 8 * MIB as usize + 1, usize::MAX),
];

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Parser)]
#[command(about = "Read-size histogram benchmark using Feuer's pooled DataFile I/O")]
struct Args {
    /// CSV columns: size_bytes,count. Counts are operation weights.
    histogram: PathBuf,
    /// Existing SSD directory. Only a new temporary directory is modified.
    #[arg(long)]
    directory: PathBuf,
    /// New output directory under experiments/, on a separate device.
    #[arg(long)]
    output: PathBuf,
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
    completed_ns: u64,
    latency_ns: u64,
}

fn histogram(csv: &str, capacity: u64) -> Result<Vec<(usize, u64)>> {
    let mut lines = csv.lines();
    if lines.next() != Some("size_bytes,count") {
        return Err("expected CSV header size_bytes,count".into());
    }
    let mut total = 0_u64;
    let mut sizes = Vec::new();
    for line in lines {
        let (size, count) = line.split_once(',').ok_or("expected size,count")?;
        let (size, count) = (size.parse::<usize>()?, count.parse::<u64>()?);
        if size == 0 || size as u64 > capacity || count == 0 {
            return Err("sizes must fit the read region and counts must be positive".into());
        }
        total = total.checked_add(count).ok_or("histogram count overflow")?;
        sizes.push((size, total));
    }
    if sizes.is_empty() {
        return Err("empty histogram".into());
    }
    Ok(sizes)
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
    rate: Option<u64>,
) -> Result<(Vec<Sample>, f64, f64)> {
    let read_bytes = args.read_mib * MIB;
    let writer_chunks = args.write_mib / WRITERS;
    let readers = args.readers;
    let cpu_before = cpu_seconds()?;
    let started = Instant::now();
    let measured = started + Duration::from_secs(args.warmup);
    let end = measured + Duration::from_secs(args.seconds);
    let mut tasks = JoinSet::new();
    for worker in 0..readers + if rate.is_some() { WRITERS } else { 0 } {
        let (file, sizes, payload) = (file.clone(), sizes.clone(), payload.clone());
        tasks.spawn(async move {
            let write = worker >= readers;
            let mut rng = fastrand::Rng::with_seed(20261102 + worker);
            let mut samples = Vec::new();
            let mut iteration = 0_u64;
            loop {
                if write && let Some(rate) = rate.filter(|rate| *rate > 0) {
                    let slot = iteration * WRITERS + worker - readers;
                    let scheduled = started + Duration::from_secs_f64(slot as f64 * MIB as f64 / rate as f64);
                    if scheduled >= end {
                        break;
                    }
                    sleep_until(scheduled.into()).await;
                }
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
                        completed_ns: finished.duration_since(measured).as_nanos() as u64,
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

fn report(out: &mut impl Write, case: &str, samples: &[Sample], seconds: f64) -> io::Result<()> {
    for (operation, label, min, max) in [("read", "all", 0, usize::MAX), ("write", "all", 0, usize::MAX)]
        .into_iter()
        .chain(SIZE_BANDS.map(|(label, min, max)| ("read", label, min, max)))
    {
        let selected: Vec<_> = samples
            .iter()
            .filter(|s| s.write == (operation == "write") && (min..=max).contains(&s.length))
            .collect();
        if selected.is_empty() {
            continue;
        }
        let bytes: u64 = selected.iter().map(|s| s.length as u64).sum();
        let mut latencies: Vec<_> = selected.iter().map(|s| s.latency_ns).collect();
        latencies.sort_unstable();
        let count = selected.len();
        let mean = latencies.iter().map(|ns| *ns as f64).sum::<f64>() / count as f64 / 1e6;
        let percentile = |p: usize| latencies[((count - 1) * p).div_ceil(100)] as f64 / 1e6;
        let row = format!(
            "{case},{operation},{label},{count},{bytes},{:.3},{:.3},{mean:.3},{:.3},{:.3},{:.3}",
            bytes as f64 / seconds / 1e6,
            count as f64 / seconds,
            percentile(50),
            percentile(95),
            percentile(99),
        );
        writeln!(out, "{row}")?;
        println!("{row}");
    }
    Ok(())
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
    let input = fs::read_to_string(&args.histogram)?;
    let sizes = Arc::new(histogram(&input, args.read_mib * MIB)?);
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&args.output)?;
    fs::write(args.output.join("histogram.csv"), input)?;
    fs::write(
        args.output.join("settings.txt"),
        format!("{args:#?}\ncommand={:?}\n", env::args().collect::<Vec<_>>()),
    )?;
    fs::write(args.output.join("read_write.rs"), include_str!("read_write.rs"))?;
    let mut input_summary = fs::File::create(args.output.join("input-sizes.csv"))?;
    writeln!(input_summary, "size,count,bytes")?;
    for (label, min, max) in SIZE_BANDS {
        let mut previous = 0;
        let (mut count, mut bytes) = (0_u64, 0_u128);
        for &(size, cumulative) in sizes.iter() {
            if (min..=max).contains(&size) {
                count += cumulative - previous;
                bytes += size as u128 * (cumulative - previous) as u128;
            }
            previous = cumulative;
        }
        writeln!(input_summary, "{label},{count},{bytes}")?;
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

    let mut summary = fs::File::create(args.output.join("summary.csv"))?;
    writeln!(
        summary,
        "case,operation,size,count,bytes,MB_s,IOPS,mean_ms,p50_ms,p95_ms,p99_ms"
    )?;
    let mut costs = fs::File::create(args.output.join("process.csv"))?;
    writeln!(
        costs,
        "case,seconds_including_warmup_and_drain,cpu_percent,pool_used_bytes,pool_idle_bytes"
    )?;
    for (case, rate) in [
        ("read-only", None),
        ("writes-450MB", Some(450_000_000)),
        ("writes-uncapped", Some(0)),
    ] {
        tokio::time::sleep(Duration::from_secs(5)).await;
        eprintln!("Running {case}");
        let (samples, elapsed, cpu_percent) = measure(&file, &sizes, &payload, &args, rate).await?;
        if !samples.iter().any(|s| !s.write) || (rate.is_some() && !samples.iter().any(|s| s.write)) {
            return Err("measurement window contained no completed I/O for an enabled stream".into());
        }
        report(&mut summary, case, &samples, args.seconds as f64)?;
        writeln!(
            costs,
            "{case},{elapsed:.3},{cpu_percent:.3},{},{}",
            pool.used_bytes(),
            pool.idle_bytes()
        )?;
        let mut raw = BufWriter::new(fs::File::create(args.output.join(format!("{case}.csv")))?);
        writeln!(raw, "operation,size_bytes,completion_ns,latency_ns")?;
        for sample in samples {
            writeln!(
                raw,
                "{},{},{},{}",
                if sample.write { "write" } else { "read" },
                sample.length,
                sample.completed_ns,
                sample.latency_ns
            )?;
        }
        raw.flush()?;
    }
    drop(file);
    temp.close()?;
    fs::write(
        args.output.join("DONE"),
        "Three cases completed once each. Temporary data removed.\n",
    )?;
    Ok(())
}
