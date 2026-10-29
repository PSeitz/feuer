//! Replay recorded disk-read arrivals without writes during measurement.
//! Run: cargo bench -p feuer-storage --bench read_trace -- /mnt/local-ssd reads.csv

use std::{
    env,
    error::Error,
    fs::read_to_string,
    hint::black_box,
    num::ParseIntError,
    time::{Duration, Instant},
};

use feuer_memory::AlignedBuffer;
use feuer_storage::{DataFile, DataFileError, IoMetrics};
use mixtrics::{metrics::BoxedRegistry, registry::noop::NoopMetricsRegistry};
use tempfile::Builder;
use tokio::{task::JoinSet, time::sleep_until};

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let root = args.next().expect("provide a directory on the target filesystem");
    let trace = read_to_string(args.next().expect("provide a read trace CSV"))?;
    let reads: Vec<[u64; 3]> = trace
        .lines()
        .skip(1)
        .map(|line| {
            let mut fields = line.split(',').map(str::parse::<u64>);
            Ok::<_, ParseIntError>([
                fields.next().unwrap()?,
                fields.next().unwrap()?,
                fields.next().unwrap()?,
            ])
        })
        .collect::<Result<_, _>>()?;
    let capacity = reads
        .iter()
        .map(|[_, offset, length]| (offset + length).next_multiple_of(4096))
        .max()
        .unwrap();
    let temp = Builder::new().prefix("feuer-read-trace-").tempdir_in(root)?;
    let registry: BoxedRegistry = Box::new(NoopMetricsRegistry);
    let file = DataFile::open(temp.path(), capacity, IoMetrics::new(&registry)).await?;

    // Initialize only referenced disk ranges, keeping original addresses in a sparse file.
    let max_length = reads.iter().map(|[_, _, length]| length).max().unwrap();
    let mut buffer = AlignedBuffer::allocate_zeroed((*max_length as usize + 4095).next_multiple_of(4096))?;
    buffer.as_mut_slice().fill(0x5a);
    let payload = buffer.into_bytes();
    for &[_, offset, length] in &reads {
        let start = offset / 4096 * 4096;
        let end = (offset + length).next_multiple_of(4096);
        file.write_at(start, &payload.slice(..(end - start) as usize)).await?;
    }
    drop(payload);

    let mut tasks = JoinSet::new();
    let started = Instant::now();
    for &[time_ns, offset, length] in &reads {
        let arrival = started + Duration::from_nanos(time_ns);
        sleep_until(arrival.into()).await;
        let file = file.clone();
        tasks.spawn(async move {
            let before = Instant::now();
            black_box(file.read_at(offset, length as usize).await?);
            let finished = Instant::now();
            Ok::<_, DataFileError>((
                length,
                finished.duration_since(before).as_secs_f64() * 1e6,
                finished.duration_since(arrival).as_secs_f64() * 1e6,
                before.duration_since(arrival).as_secs_f64() * 1e6,
                finished,
            ))
        });
    }
    let mut samples = Vec::new();
    while let Some(result) = tasks.join_next().await {
        samples.push(result??);
    }
    let seconds = samples
        .iter()
        .map(|s| s.4)
        .max()
        .unwrap()
        .duration_since(started)
        .as_secs_f64();
    let bytes: u64 = samples.iter().map(|s| s.0).sum();
    let max_start_lag = samples.iter().map(|s| s.3).fold(0.0, f64::max);
    println!("reads,bytes,seconds,MiB_s,max_start_lag_us");
    println!(
        "{},{bytes},{seconds:.3},{:.1},{max_start_lag:.1}",
        samples.len(),
        bytes as f64 / 1048576.0 / seconds
    );
    println!("read_bytes,count,api_mean_us,api_p50_us,api_p90_us,api_p99_us,arrival_to_completion_p99_us");
    for (label, min, max) in [
        ("all", 0, u64::MAX),
        ("<=64KiB", 0, 65536),
        ("64KiB..1MiB", 65537, 1048576),
        (">1MiB", 1048577, u64::MAX),
    ] {
        let mut api: Vec<_> = samples
            .iter()
            .filter(|s| (min..=max).contains(&s.0))
            .map(|s| s.1)
            .collect();
        let mut completion: Vec<_> = samples
            .iter()
            .filter(|s| (min..=max).contains(&s.0))
            .map(|s| s.2)
            .collect();
        if api.is_empty() {
            continue;
        }
        api.sort_unstable_by(f64::total_cmp);
        completion.sort_unstable_by(f64::total_cmp);
        let n = api.len();
        println!(
            "{label},{n},{:.1},{:.1},{:.1},{:.1},{:.1}",
            api.iter().sum::<f64>() / n as f64,
            api[n / 2],
            api[n * 90 / 100],
            api[n * 99 / 100],
            completion[n * 99 / 100]
        );
    }
    Ok(())
}
