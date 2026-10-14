//! Linux cache replay: one timestamp-paced sender, one task per arrival, real tiered cache.
//! Only request descriptors are queued. Synthetic downloads optionally simulate calibrated S3 delay.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs::{self, File},
        future::Future,
        io::{BufWriter, Write},
        path::PathBuf,
        sync::{Arc, mpsc},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use clap::Parser;
    use feuer::{AvoidedRetrievalCostScorer, CacheConfig, TieredMemoryDiskCache};
    use feuer_replay::{Request, Result, load_requests};
    use feuer_types::Download;
    use mixtrics::{metrics::BoxedRegistry, registry::prometheus::PrometheusMetricsRegistry};
    use prometheus::{Encoder, Gauge, Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};

    const S3_SLOTS: usize = 48;
    const S3_LATENCY_SECONDS: f64 = 0.108599;
    const S3_BYTES_PER_SECOND: f64 = 80.205 * 1_048_576.0;

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
        /// Override the shared fixed retrieval cost for memory. Per-byte cost remains 1.
        #[arg(long, value_parser = parse_cost)]
        memory_cost_per_request: Option<f64>,
        /// Override the shared fixed retrieval cost for disk. Per-byte cost remains 1.
        #[arg(long, value_parser = parse_cost)]
        disk_cost_per_request: Option<f64>,
        /// Caller-demand requests per access-count half-life. Omit to use cache defaults/environment.
        #[arg(long)]
        access_count_half_life: Option<u64>,
        /// Simulate S3 with 48 slots, 108.599 ms fixed latency and 80.205 MiB/s per GET.
        #[arg(long)]
        simulate_s3: bool,
        /// Replay each (recorded hash, start, end) as a separate immutable whole object.
        #[arg(long)]
        full_objects: bool,
    }

    fn parse_cost(value: &str) -> std::result::Result<f64, String> {
        let cost: f64 = value.parse().map_err(|_| "expected a finite nonnegative cost")?;
        if !cost.is_finite() || cost < 0.0 {
            return Err("expected a finite nonnegative cost".into());
        }
        Ok(cost)
    }

    impl Args {
        fn request_costs(&self) -> [f64; 2] {
            [self.memory_cost_per_request, self.disk_cost_per_request]
                .map(|cost| cost.unwrap_or(self.fixed_retrieval_equivalent_bytes as f64))
        }
    }

    // Downloads use the cache's buffer pool but do not coalesce concurrent misses.
    struct Source {
        slots: Option<tokio::sync::Semaphore>,
        gets: IntCounter,
        bytes: IntCounter,
        waiting: IntGauge,
        active: IntGauge,
        queue: Histogram,
        service: Histogram,
    }

    impl Source {
        fn new(registry: &Registry, simulate_s3: bool) -> Result<Self> {
            let source = Self {
                slots: simulate_s3.then(|| tokio::sync::Semaphore::new(S3_SLOTS)),
                gets: IntCounter::new("replay_source_gets_total", "Started synthetic source GETs")?,
                bytes: IntCounter::new("replay_source_bytes_total", "Downloaded synthetic source bytes")?,
                waiting: IntGauge::new("replay_source_waiting_requests", "Source GETs waiting for a slot")?,
                active: IntGauge::new("replay_source_active_requests", "Source GETs in service")?,
                queue: Histogram::with_opts(
                    HistogramOpts::new("replay_source_queue_seconds", "Source slot wait duration")
                        .buckets(prometheus::exponential_buckets(0.001, 2.0, 21)?),
                )?,
                service: Histogram::with_opts(
                    HistogramOpts::new("replay_source_service_seconds", "Source service including allocation")
                        .buckets(prometheus::exponential_buckets(0.001, 2.0, 21)?),
                )?,
            };
            registry.register(Box::new(source.gets.clone()))?;
            registry.register(Box::new(source.bytes.clone()))?;
            registry.register(Box::new(source.waiting.clone()))?;
            registry.register(Box::new(source.active.clone()))?;
            registry.register(Box::new(source.queue.clone()))?;
            registry.register(Box::new(source.service.clone()))?;
            Ok(source)
        }

        async fn download(&self, cache: &TieredMemoryDiskCache, request: &Request) -> std::io::Result<Download> {
            self.gets.inc();
            self.waiting.inc();
            let queued = Instant::now();
            let _slot = match &self.slots {
                Some(slots) => Some(slots.acquire().await.unwrap()),
                None => None,
            };
            self.queue.observe(queued.elapsed().as_secs_f64());
            self.waiting.dec();
            self.active.inc();
            let started = Instant::now();
            if self.slots.is_some() {
                tokio::time::sleep(Duration::from_secs_f64(
                    S3_LATENCY_SECONDS + request.range.len() as f64 / S3_BYTES_PER_SECOND,
                ))
                .await;
            }
            let download = cache.allocate_buffer(request.range.len() as usize).map(|mut buffer| {
                buffer.as_mut_slice().fill(0);
                buffer
                    .into_download(request.range.start())
                    .expect("request range is valid")
            });
            if download.is_ok() {
                self.bytes.inc_by(request.range.len());
            }
            self.service.observe(started.elapsed().as_secs_f64());
            self.active.dec();
            download
        }
    }

    struct Observation {
        index: usize,
        sender_ns: u64,
        queue_ns: u64,
        service_ns: u64,
    }

    /// Preserve arrival timing even under overload. The finite trace can accumulate as descriptors.
    fn emit(
        requests: &[Request],
        origin: Instant,
        sender: async_channel::Sender<(usize, Instant)>,
        queued: &IntGauge,
    ) -> Result<()> {
        for (index, request) in requests.iter().enumerate() {
            let deadline = origin + Duration::from_nanos(request.time_ns);
            while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                thread::sleep(remaining);
            }
            queued.inc();
            sender.try_send((index, Instant::now()))?;
        }
        Ok(())
    }

    /// Start a lookup task for every arrival without waiting for earlier lookups to complete.
    async fn run_lookups<F, Fut>(
        receiver: async_channel::Receiver<(usize, Instant)>,
        queued: IntGauge,
        active: IntGauge,
        mut lookup: F,
    ) -> Result<Vec<Observation>>
    where
        F: FnMut(usize, Instant) -> Fut,
        Fut: Future<Output = Result<Observation>> + Send + 'static,
    {
        let mut lookups = tokio::task::JoinSet::new();
        let mut observations = Vec::new();
        while let Ok((index, sent)) = receiver.recv().await {
            let (queued, active) = (queued.clone(), active.clone());
            let future = lookup(index, sent);
            lookups.spawn(async move {
                queued.dec();
                active.inc();
                let result = future.await;
                active.dec();
                result
            });
            while let Some(result) = lookups.try_join_next() {
                observations.push(result??);
            }
        }
        while let Some(result) = lookups.join_next().await {
            observations.push(result??);
        }
        Ok(observations)
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
        let [memory_cost, disk_cost] = args.request_costs();
        let mut config = CacheConfig::new(&args.cache_dir, args.disk_bytes, args.memory_bytes)?;
        if let Some(half_life) = args.access_count_half_life {
            config = config.with_access_count_half_life(half_life)?;
        }
        let mut trace = load_requests(&args.input, args.trace_seconds)?;
        if args.full_objects {
            for request in &mut trace.requests {
                *request = request.full_object();
            }
        }
        eprintln!(
            "Recorded starts={}, outcomes={:?}, incomplete={}. Replaying every start with successful synthetic downloads. Full objects={}.",
            trace.requests.len(),
            trace.outcomes,
            trace.incomplete,
            args.full_objects,
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
        writeln!(
            summary,
            "scheduling,task_per_arrival\nfull_objects,{}\nfixed_retrieval_equivalent_bytes,{}",
            u8::from(args.full_objects),
            args.fixed_retrieval_equivalent_bytes,
        )?;
        writeln!(
            summary,
            "memory_cost_per_request,{memory_cost}\ndisk_cost_per_request,{disk_cost}\nmemory_cost_per_requested_byte,1\ndisk_cost_per_requested_byte,1\naccess_count_half_life,{}",
            config.access_count_half_life(),
        )?;
        writeln!(
            summary,
            "simulate_s3,{}\nsource_buffer_pool,1",
            u8::from(args.simulate_s3)
        )?;
        if args.simulate_s3 {
            writeln!(
                summary,
                "source_slots,{S3_SLOTS}\nsource_latency_seconds,{S3_LATENCY_SECONDS}\nsource_bytes_per_second,{S3_BYTES_PER_SECOND}"
            )?;
        } else {
            writeln!(
                summary,
                "source_slots,unlimited\nsource_latency_seconds,0\nsource_bytes_per_second,unlimited"
            )?;
        }
        writeln!(summary, "recorded_incomplete,{}", trace.incomplete)?;
        for (kind, count) in trace.outcomes.iter().enumerate().skip(1) {
            writeln!(summary, "recorded_outcome_{kind},{count}")?;
        }
        summary.flush()?;
        let registry = Registry::new();
        registry.register(Box::new(prometheus::process_collector::ProcessCollector::for_self()))?;
        let completed = IntCounter::new("replay_completed_total", "Completed fetch events")?;
        let queued = IntGauge::new("replay_queued_requests", "Arrivals whose lookup task has not started")?;
        let active = IntGauge::new("replay_active_requests", "Started lookup tasks not yet completed")?;
        let elapsed = Gauge::new("replay_elapsed_seconds", "Monotonic seconds since replay origin")?;
        let phase = IntGauge::new(
            "replay_phase",
            "0 opening, 1 requests, 2 five-second observation tail, 3 finished",
        )?;
        registry.register(Box::new(completed.clone()))?;
        registry.register(Box::new(queued.clone()))?;
        registry.register(Box::new(active.clone()))?;
        registry.register(Box::new(elapsed.clone()))?;
        registry.register(Box::new(phase.clone()))?;
        let source = Arc::new(Source::new(&registry, args.simulate_s3)?);
        let metrics: BoxedRegistry = Box::new(PrometheusMetricsRegistry::new(registry.clone()));
        let opening = Instant::now();
        let scorer = |cost_per_request| {
            Arc::new(AvoidedRetrievalCostScorer {
                cost_per_request,
                cost_per_requested_byte: 1.0,
            })
        };
        let cache = TieredMemoryDiskCache::open_with_retention_scorers(
            config,
            Some(&metrics),
            None,
            scorer(memory_cost),
            scorer(disk_cost),
        )
        .await?;
        writeln!(summary, "open_seconds,{}", opening.elapsed().as_secs_f64())?;
        snapshot(&registry, &mut File::create_new(args.output.join("before.prom"))?)?;

        let (sender, receiver) = async_channel::unbounded::<(usize, Instant)>();
        let origin = Instant::now() + Duration::from_millis(100);
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
                elapsed.set(origin.elapsed().as_secs_f64());
                snapshot(&sample_registry, &mut samples)?;
                if stopping {
                    return Ok(());
                }
            }
        });
        phase.set(1);
        let events = requests.clone();
        let sender_queued = queued.clone();
        let sender = thread::spawn(move || emit(&events, origin, sender, &sender_queued));
        let full_objects = args.full_objects;
        let mut observations = run_lookups(receiver, queued, active, |index, sent| {
            let (requests, cache, completed) = (requests.clone(), cache.clone(), completed.clone());
            let source = source.clone();
            async move {
                let request = &requests[index];
                let started = Instant::now();
                let bytes = cache
                    .get_or_fetch_by_hash(request.object_key, (!full_objects).then_some(request.range), || {
                        source.download(&cache, request)
                    })
                    .await?;
                assert_eq!(bytes.len() as u64, request.range.len());
                drop(bytes);
                let observation = Observation {
                    index,
                    sender_ns: sent
                        .saturating_duration_since(origin + Duration::from_nanos(request.time_ns))
                        .as_nanos() as u64,
                    queue_ns: started.duration_since(sent).as_nanos() as u64,
                    service_ns: started.elapsed().as_nanos() as u64,
                };
                completed.inc();
                Ok(observation)
            }
        })
        .await?;
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
        fn scoring_and_source_options() {
            let base = ["replay", "--input=trace.feuer", "--cache-dir=cache", "--output=results"];
            let defaults = Args::try_parse_from(base).unwrap();
            assert_eq!(defaults.request_costs(), [256_000.0; 2]);
            assert_eq!(defaults.access_count_half_life, None);
            assert!(!defaults.simulate_s3);
            let args = Args::try_parse_from(base.into_iter().chain([
                "--fixed-retrieval-equivalent-bytes=12",
                "--memory-cost-per-request=1.5",
                "--access-count-half-life=42",
                "--simulate-s3",
            ]))
            .unwrap();
            assert_eq!(args.request_costs(), [1.5, 12.0]);
            assert_eq!(args.access_count_half_life, Some(42));
            assert!(args.simulate_s3);
            for flag in ["--memory-cost-per-request", "--disk-cost-per-request"] {
                for cost in ["NaN", "inf", "-1"] {
                    assert!(Args::try_parse_from(base.into_iter().chain([format!("{flag}={cost}").as_str()])).is_err());
                }
                assert!(Args::try_parse_from(base.into_iter().chain([format!("{flag}=0").as_str()])).is_ok());
            }
        }

        #[tokio::test]
        async fn source_downloads_charge_buffer_capacity() {
            for full_objects in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let registry = Registry::new();
                let metrics: BoxedRegistry = Box::new(PrometheusMetricsRegistry::new(registry.clone()));
                let config = CacheConfig::new(directory.path(), 0, 4 << 20)
                    .unwrap()
                    .with_idle_buffer_pool_percent(7)
                    .unwrap();
                let cache = TieredMemoryDiskCache::open(config, Some(&metrics), None).await.unwrap();
                let source = Source::new(&registry, false).unwrap();
                let mut request = Request {
                    request_id: 0,
                    time_ns: 0,
                    object_key: ObjectKeyHash(0),
                    range: ByteRange::new(100, 1124).unwrap(),
                };
                if full_objects {
                    request = request.full_object();
                }
                let mut buffer = cache.allocate_buffer(request.range.len() as usize).unwrap();
                let charge = buffer.capacity();
                let address = buffer.as_ref().as_ptr();
                assert!(charge > request.range.len() as usize);
                buffer.as_mut_slice().fill(17);
                drop(buffer);
                let bytes = cache
                    .get_or_fetch_by_hash(request.object_key, (!full_objects).then_some(request.range), || {
                        source.download(&cache, &request)
                    })
                    .await
                    .unwrap();
                assert_eq!(bytes.len(), request.range.len() as usize);
                assert_eq!(bytes.as_ptr(), address);
                assert!(bytes.iter().all(|&byte| byte == 0));
                let used = registry
                    .gather()
                    .into_iter()
                    .find(|family| family.name() == "feuer_memory_used_bytes")
                    .unwrap();
                assert_eq!(used.get_metric()[0].get_gauge().get_value(), charge as f64);
            }
        }

        #[tokio::test]
        async fn source_callbacks_wait_for_slots_then_complete_independently() {
            let directory = tempfile::tempdir().unwrap();
            let cache =
                TieredMemoryDiskCache::open(CacheConfig::new(directory.path(), 0, 4 << 20).unwrap(), None, None)
                    .await
                    .unwrap();
            let source = Arc::new(Source::new(&Registry::new(), true).unwrap());
            let mut held = source
                .slots
                .as_ref()
                .unwrap()
                .acquire_many(S3_SLOTS as u32)
                .await
                .unwrap();
            let mut tasks = tokio::task::JoinSet::new();
            for _ in 0..2 {
                let source = source.clone();
                let cache = cache.clone();
                tasks.spawn(async move {
                    source
                        .download(
                            &cache,
                            &Request {
                                request_id: 0,
                                time_ns: 0,
                                object_key: ObjectKeyHash(0),
                                range: ByteRange::new(0, 16).unwrap(),
                            },
                        )
                        .await
                });
            }
            tokio::time::timeout(Duration::from_secs(5), async {
                while source.waiting.get() != 2 {
                    tokio::task::yield_now().await;
                }
                assert_eq!(source.active.get(), 0);
                assert_eq!(source.bytes.get(), 0);
                drop(held.split(1).unwrap());
                while source.active.get() != 1 {
                    tokio::task::yield_now().await;
                }
                assert_eq!(source.waiting.get(), 1);
                assert_eq!(source.bytes.get(), 0);
                let mut downloads = Vec::new();
                while let Some(result) = tasks.join_next().await {
                    downloads.push(result.unwrap().unwrap());
                }
                assert_ne!(downloads[0].bytes().as_ptr(), downloads[1].bytes().as_ptr());
            })
            .await
            .unwrap();
            assert_eq!(source.gets.get(), 2);
            assert_eq!(source.bytes.get(), 32);
            assert_eq!(source.waiting.get(), 0);
            assert_eq!(source.active.get(), 0);
            assert_eq!(source.queue.get_sample_count(), 2);
            assert!(source.service.get_sample_sum() >= 2.0 * S3_LATENCY_SECONDS);
        }

        #[tokio::test]
        async fn arrivals_start_while_earlier_lookups_wait_on_one_runtime_thread() {
            let (sender, receiver) = async_channel::unbounded();
            let queued = IntGauge::new("queued", "Arrivals not started").unwrap();
            let active = IntGauge::new("active", "Started lookups not completed").unwrap();
            let barrier = Arc::new(tokio::sync::Barrier::new(66));
            let (started, ready) = async_channel::unbounded();
            let lookup_barrier = barrier.clone();
            let lookup_queued = queued.clone();
            let lookup_active = active.clone();
            let lookups = tokio::spawn(async move {
                run_lookups(receiver, lookup_queued, lookup_active, |index, _| {
                    let (barrier, started) = (lookup_barrier.clone(), started.clone());
                    async move {
                        started.try_send(index)?;
                        barrier.wait().await;
                        Ok(Observation {
                            index,
                            sender_ns: 0,
                            queue_ns: 0,
                            service_ns: 0,
                        })
                    }
                })
                .await
            });
            for batch in [0..33, 33..65] {
                for index in batch.clone() {
                    queued.inc();
                    sender.try_send((index, Instant::now())).unwrap();
                }
                let mut indices = Vec::new();
                for _ in batch.clone() {
                    indices.push(
                        tokio::time::timeout(Duration::from_secs(5), ready.recv())
                            .await
                            .unwrap()
                            .unwrap(),
                    );
                }
                indices.sort_unstable();
                assert_eq!(indices, batch.clone().collect::<Vec<_>>());
                assert_eq!(queued.get(), 0);
                assert_eq!(active.get(), batch.end as i64);
                assert!(!lookups.is_finished());
            }
            drop(sender);
            barrier.wait().await;
            let mut observations = tokio::time::timeout(Duration::from_secs(5), lookups)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            observations.sort_unstable_by_key(|o| o.index);
            assert_eq!(
                observations.iter().map(|o| o.index).collect::<Vec<_>>(),
                (0..65).collect::<Vec<_>>()
            );
            assert_eq!(queued.get(), 0);
            assert_eq!(active.get(), 0);
        }

        #[tokio::test]
        async fn lookup_errors_fail_replay() {
            let (sender, receiver) = async_channel::unbounded();
            let queued = IntGauge::new("queued", "Arrivals not started").unwrap();
            let active = IntGauge::new("active", "Started lookups not completed").unwrap();
            queued.inc();
            sender.try_send((0, Instant::now())).unwrap();
            drop(sender);
            let error = run_lookups(receiver, queued.clone(), active.clone(), |_, _| async {
                Err("lookup failed".into())
            })
            .await
            .err()
            .unwrap();
            assert_eq!(error.to_string(), "lookup failed");
            assert_eq!(queued.get(), 0);
            assert_eq!(active.get(), 0);
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
            let queued = IntGauge::new("queued", "Arrivals not started").unwrap();
            let producer = thread::spawn(move || emit(&requests, origin, sender, &queued));
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
