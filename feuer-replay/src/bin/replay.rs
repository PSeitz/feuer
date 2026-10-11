//! Linux cache replay: one timestamp-paced sender, one consumer per available CPU, real tiered cache.
//! Only request descriptors are queued. Fake downloads allocate zero-filled payloads with no network delay.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        convert::Infallible,
        error::Error,
        fs::{self, File},
        io::{BufWriter, Write},
        path::PathBuf,
        sync::{Arc, mpsc},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use bytes::Bytes;
    use clap::Parser;
    use feuer::{CacheConfig, TieredMemoryDiskCache};
    use feuer_replay::{Request, Result, load_requests};
    use feuer_types::Download;
    use mixtrics::{metrics::BoxedRegistry, registry::prometheus::PrometheusMetricsRegistry};
    use prometheus::{Encoder, Gauge, IntCounter, IntGauge, Registry, TextEncoder};

    #[derive(Debug, Parser)]
    struct Args {
        #[arg(long)]
        input: PathBuf,
        /// Must not already exist. Use the local SSD, not the directory holding benchmark results.
        #[arg(long)]
        cache_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 16 << 30)]
        memory_bytes: u64,
        /// Disk capacity in bytes. Zero disables disk without changing request scheduling or metrics.
        #[arg(long, default_value_t = 0)]
        disk_bytes: u64,
        /// Limit arrivals to a half-open original-time prefix. Omit to replay the full trace.
        #[arg(long)]
        trace_seconds: Option<u64>,
        #[arg(long, default_value_t = 256_000)]
        fixed_retrieval_equivalent_bytes: u64,
    }

    struct Observation {
        index: usize,
        sender_ns: u64,
        queue_ns: u64,
        service_ns: u64,
    }

    /// Preserve arrival timing even under overload: the finite trace can accumulate in the queue.
    fn emit(requests: &[Request], origin: Instant, sender: async_channel::Sender<(usize, Instant)>) -> Result<()> {
        for (index, request) in requests.iter().enumerate() {
            let deadline = origin + Duration::from_nanos(request.time_ns);
            while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                thread::sleep(remaining);
            }
            sender.try_send((index, Instant::now()))?;
        }
        Ok(())
    }

    /// Each frame contains every registered metric, with wall-clock timestamps on its samples.
    fn snapshot(registry: &Registry, out: &mut impl Write) -> Result<()> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
        let mut families = registry.gather();
        for family in &mut families {
            for metric in family.mut_metric() {
                metric.set_timestamp_ms(timestamp);
            }
        }
        TextEncoder::new().encode(&families, out)?;
        out.flush()?;
        Ok(())
    }

    fn distribution(out: &mut impl Write, name: &str, mut values: Vec<u64>) -> Result<()> {
        values.sort_unstable();
        let mean = values.iter().map(|&v| v as u128).sum::<u128>() as f64 / values.len() as f64;
        writeln!(out, "{name}_mean_ns,{mean}")?;
        for (label, percent) in [("p50", 50), ("p95", 95), ("p99", 99), ("max", 100)] {
            writeln!(
                out,
                "{name}_{label}_ns,{}",
                values[(values.len() * percent).div_ceil(100) - 1]
            )?;
        }
        Ok(())
    }

    async fn replay(args: Args, workers: usize) -> Result<()> {
        let trace = load_requests(&args.input, args.trace_seconds)?;
        eprintln!(
            "Recorded starts={}, outcomes={:?}, incomplete={}. Replaying every start with successful exact-range synthetic downloads.",
            trace.requests.len(),
            trace.outcomes,
            trace.incomplete,
        );
        let requests = Arc::new(trace.requests);
        fs::create_dir(&args.cache_dir)?;
        fs::create_dir_all(&args.output)?;
        fs::write(args.output.join("pid.txt"), format!("{}\n", std::process::id()))?;
        let mut summary = BufWriter::new(File::create_new(args.output.join("summary.csv"))?);
        writeln!(
            summary,
            "metric,value\nworkers,{workers}\nmemory_bytes,{}\ndisk_bytes,{}\nrequests,{}\ntrace_seconds,{}",
            args.memory_bytes,
            args.disk_bytes,
            requests.len(),
            requests.last().unwrap().time_ns as f64 / 1e9
        )?;
        writeln!(summary, "recorded_incomplete,{}", trace.incomplete)?;
        for (kind, count) in trace.outcomes.iter().enumerate().skip(1) {
            writeln!(summary, "recorded_outcome_{kind},{count}")?;
        }
        summary.flush()?;
        let registry = Registry::new();
        registry.register(Box::new(prometheus::process_collector::ProcessCollector::for_self()))?;
        let completed = IntCounter::new("replay_completed_total", "Completed fetch events")?;
        let queued = IntGauge::new("replay_queued_requests", "Fetch events waiting for a consumer")?;
        let elapsed = Gauge::new("replay_elapsed_seconds", "Monotonic seconds since replay origin")?;
        let phase = IntGauge::new(
            "replay_phase",
            "0 opening, 1 requests, 2 five-second observation tail, 3 finished",
        )?;
        registry.register(Box::new(completed.clone()))?;
        registry.register(Box::new(queued.clone()))?;
        registry.register(Box::new(elapsed.clone()))?;
        registry.register(Box::new(phase.clone()))?;
        let metrics: BoxedRegistry = Box::new(PrometheusMetricsRegistry::new(registry.clone()));
        let opening = Instant::now();
        let cache = TieredMemoryDiskCache::open(
            CacheConfig::new(&args.cache_dir, args.disk_bytes, args.memory_bytes)?
                .with_fixed_retrieval_equivalent_bytes(args.fixed_retrieval_equivalent_bytes),
            Some(&metrics),
            None,
        )
        .await?;
        writeln!(summary, "open_seconds,{}", opening.elapsed().as_secs_f64())?;
        snapshot(&registry, &mut File::create_new(args.output.join("before.prom"))?)?;

        let (sender, receiver) = async_channel::unbounded::<(usize, Instant)>();
        let origin = Instant::now() + Duration::from_millis(100);
        let mut consumers = tokio::task::JoinSet::new();
        for _ in 0..workers {
            let (receiver, requests, cache, completed) =
                (receiver.clone(), requests.clone(), cache.clone(), completed.clone());
            consumers.spawn(async move {
                let mut observations = Vec::new();
                while let Ok((index, sent)) = receiver.recv().await {
                    let request = &requests[index];
                    let started = Instant::now();
                    let bytes = cache
                        .get_or_fetch_by_hash(request.object_key, request.range, || async {
                            Ok::<_, Infallible>(
                                Download::new(
                                    request.range.start(),
                                    Bytes::from(vec![0; request.range.len() as usize]),
                                )
                                .unwrap(),
                            )
                        })
                        .await?;
                    assert_eq!(bytes.len() as u64, request.range.len());
                    drop(bytes);
                    observations.push(Observation {
                        index,
                        sender_ns: sent
                            .saturating_duration_since(origin + Duration::from_nanos(request.time_ns))
                            .as_nanos() as u64,
                        queue_ns: started.duration_since(sent).as_nanos() as u64,
                        service_ns: started.elapsed().as_nanos() as u64,
                    });
                    completed.inc();
                }
                Ok::<_, Box<dyn Error + Send + Sync>>(observations)
            });
        }
        let (stop, stopping) = mpsc::channel();
        let samples = File::create_new(args.output.join("metrics.prom"))?;
        let sample_registry = registry.clone();
        let sampler = thread::spawn(move || -> Result<()> {
            let mut samples = BufWriter::new(samples);
            loop {
                let stopping = !matches!(
                    stopping.recv_timeout(Duration::from_secs(1)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                );
                queued.set(receiver.len() as i64);
                elapsed.set(origin.elapsed().as_secs_f64());
                snapshot(&sample_registry, &mut samples)?;
                if stopping {
                    return Ok(());
                }
            }
        });
        phase.set(1);
        let events = requests.clone();
        let sender = thread::spawn(move || emit(&events, origin, sender));
        let mut observations = Vec::with_capacity(requests.len());
        while let Some(result) = consumers.join_next().await {
            observations.extend(result??);
        }
        sender.join().expect("sender panicked")?;
        let finished = origin.elapsed();
        phase.set(2);
        snapshot(
            &registry,
            &mut File::create_new(args.output.join("requests-finished.prom"))?,
        )?;
        // Observe deferred writes without forcing a flush or claiming this wait guarantees drainage.
        tokio::time::sleep(Duration::from_secs(5)).await;
        snapshot(&registry, &mut File::create_new(args.output.join("settled.prom"))?)?;
        phase.set(3);
        stop.send(())?;
        sampler.join().expect("sampler panicked")?;
        writeln!(
            summary,
            "execution_seconds,{}\nbacklog_drain_seconds,{}\nobservation_tail_seconds,5",
            finished.as_secs_f64(),
            finished
                .saturating_sub(Duration::from_nanos(requests.last().unwrap().time_ns))
                .as_secs_f64()
        )?;
        // Outside the measured phase: verify no omissions/duplication and calculate exact percentiles.
        observations.sort_unstable_by_key(|o| o.index);
        assert_eq!(observations.len(), requests.len());
        assert!(observations.iter().enumerate().all(|(index, o)| index == o.index));
        distribution(
            &mut summary,
            "sender_lateness",
            observations.iter().map(|o| o.sender_ns).collect(),
        )?;
        distribution(
            &mut summary,
            "queue_wait",
            observations.iter().map(|o| o.queue_ns).collect(),
        )?;
        distribution(
            &mut summary,
            "service",
            observations.iter().map(|o| o.service_ns).collect(),
        )?;
        distribution(
            &mut summary,
            "scheduled_to_complete",
            observations
                .iter()
                .map(|o| o.sender_ns + o.queue_ns + o.service_ns)
                .collect(),
        )?;
        summary.flush()?;
        drop(cache);
        Ok(())
    }

    pub fn main() -> Result<()> {
        let args = Args::parse();
        let workers = thread::available_parallelism()?.get();
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(workers)
            .enable_all()
            .build()?
            .block_on(replay(args, workers))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use feuer_types::{ByteRange, ObjectKeyHash};

        #[test]
        fn trace_prefix_is_optional() {
            let args = ["replay", "--input=trace.feuer", "--cache-dir=cache", "--output=results"];
            assert_eq!(Args::try_parse_from(args).unwrap().trace_seconds, None);
            assert_eq!(
                Args::try_parse_from(args.into_iter().chain(["--trace-seconds", "180"]))
                    .unwrap()
                    .trace_seconds,
                Some(180)
            );
        }

        #[test]
        fn sender_preserves_timing_order_and_equal_timestamps() {
            let requests: Vec<_> = [0, 2_000_000, 2_000_000]
                .into_iter()
                .map(|time_ns| Request {
                    request_id: time_ns,
                    time_ns,
                    object_key: ObjectKeyHash(0),
                    range: ByteRange::new(0, 1).unwrap(),
                })
                .collect();
            let deadlines: Vec<_> = requests.iter().map(|r| r.time_ns).collect();
            let origin = Instant::now() + Duration::from_millis(2);
            let (sender, receiver) = async_channel::unbounded();
            let producer = thread::spawn(move || emit(&requests, origin, sender));
            for (index, time_ns) in deadlines.into_iter().enumerate() {
                let (received, sent) = receiver.recv_blocking().unwrap();
                assert_eq!(received, index);
                assert!(sent >= origin + Duration::from_nanos(time_ns));
            }
            producer.join().unwrap().unwrap();
            assert!(receiver.recv_blocking().is_err());
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> feuer_replay::Result<()> {
    linux::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> feuer_replay::Result<()> {
    Err("replay requires Linux, where TieredMemoryDiskCache::open is available".into())
}
