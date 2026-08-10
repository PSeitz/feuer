//! Controlled memory-tier replay against Feuer and a pinned Foyer revision.

use std::{
    collections::HashMap,
    env::{self, VarError},
    fs,
    path::Path,
    time::{Duration, Instant},
};

use bytes::Bytes;
use clap::{Parser, ValueEnum};
use feuer_memory::MemoryCache;
use feuer_types::{ByteRange, Download, ObjectKey};
use foyer_memory::{Cache as FoyerCache, CacheBuilder, S3FifoConfig};

const PINNED_FOYER_REVISION: &str = "165cde3d4e638aaf2680384c02f57222b40be128";
const SOURCE_FIXED_EQUIVALENT_BYTES: u64 = 10_000_000;
const TRACE_FILE: &str = "access_pattern.ndjson";
const COALESCING_DISTANCE_ENV: &str = "COALESCING_DISTANCE_BYTES";
const WHOLE_SPLIT_THRESHOLD_ENV: &str = "WHOLE_SPLIT_THRESHOLD_BYTES";
const DEFAULT_COALESCING_DISTANCE_BYTES: u64 = SOURCE_FIXED_EQUIVALENT_BYTES;
const DEFAULT_WHOLE_SPLIT_THRESHOLD_BYTES: u64 = 8 << 20;
const COALESCING_WINDOW_MILLIS: u64 = 5;
const CSV_HEADER: &str = "workload,downloader,shards,capacity_bytes,engine,requests,requested_bytes,cache_hits,hit_bytes,cache_hit_pct,byte_hit_pct,source_cost_hit_pct,source_gets,source_bytes,used_payload_bytes,used_vs_target_pct,elapsed_ms,operations_per_second";

#[derive(Debug, Parser)]
#[command(about = "Replay the captured trace against Feuer and pinned Foyer")]
struct Args {
    /// Soft payload capacities. Repeat the flag or separate values with commas.
    #[arg(
        long = "capacity",
        default_value = "256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB",
        value_delimiter = ',',
        value_parser = parse_bytes
    )]
    capacities: Vec<usize>,

    /// Memory shard counts. Repeat the flag or separate values with commas.
    #[arg(long, default_value = "16", value_delimiter = ',')]
    shards: Vec<usize>,

    /// Downloader policies. Repeat the flag or separate values with commas.
    #[arg(
        long = "downloader",
        value_enum,
        default_value = "expanded,exact",
        value_delimiter = ','
    )]
    downloaders: Vec<DownloadPolicy>,

    /// Untimed passes executed against each cache before the measured pass.
    #[arg(long, default_value_t = 0)]
    warmup_iterations: usize,

    /// Restrict the trace to its first N operations.
    #[arg(long)]
    operations: Option<usize>,

    /// Emit machine-readable CSV instead of the human-readable table.
    #[arg(long)]
    csv: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DownloadPolicy {
    Expanded,
    Exact,
}

impl DownloadPolicy {
    const fn name(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Exact => "exact",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DownloadConfig {
    coalescing_distance_bytes: u64,
    whole_split_threshold_bytes: u64,
}

impl DownloadConfig {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            coalescing_distance_bytes: byte_count_from_env(COALESCING_DISTANCE_ENV, DEFAULT_COALESCING_DISTANCE_BYTES)?,
            whole_split_threshold_bytes: byte_count_from_env(
                WHOLE_SPLIT_THRESHOLD_ENV,
                DEFAULT_WHOLE_SPLIT_THRESHOLD_BYTES,
            )?,
        })
    }
}

#[derive(Clone)]
struct Access {
    object_key: ObjectKey,
    object_size: u64,
    requested: ByteRange,
    timestamp_millis: u64,
}

impl Access {
    fn validate(&self, index: usize) -> Result<(), String> {
        if self.requested.end() > self.object_size {
            return Err(format!(
                "operation {index} requests {}..{} beyond object size {}",
                self.requested.start(),
                self.requested.end(),
                self.object_size
            ));
        }
        Ok(())
    }

    fn downloaded_range(&self, policy: DownloadPolicy, config: DownloadConfig) -> ByteRange {
        if matches!(policy, DownloadPolicy::Exact) {
            return self.requested;
        }
        if self.object_size < config.whole_split_threshold_bytes {
            ByteRange::new(0, self.object_size)
                .expect("an object containing a valid non-empty request must be non-empty")
        } else {
            self.requested
        }
    }
}

struct Workload {
    name: &'static str,
    accesses: Vec<Access>,
    expanded_downloads: Vec<ByteRange>,
    download_config: DownloadConfig,
}

trait Engine {
    fn name(&self) -> &'static str;

    /// Looks up an access using the range this downloader would fetch on a miss.
    fn get(&mut self, access: &Access, downloaded: ByteRange) -> bool;

    fn populate(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String>;

    fn used_payload_bytes(&self) -> u64;
}

struct FeuerEngine {
    cache: MemoryCache,
}

impl FeuerEngine {
    fn new(capacity: usize, shards: usize) -> Self {
        Self {
            cache: MemoryCache::with_shards_for_benchmark(capacity as u64, shards),
        }
    }
}

impl Engine for FeuerEngine {
    fn name(&self) -> &'static str {
        "feuer-value-density"
    }

    fn get(&mut self, access: &Access, _downloaded: ByteRange) -> bool {
        let Some(bytes) = self.cache.get(&access.object_key, access.requested) else {
            return false;
        };
        debug_assert_eq!(bytes.len() as u64, access.requested.len());
        true
    }

    fn populate(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String> {
        let download = Download::new(downloaded.start(), payload).map_err(|error| error.to_string())?;
        debug_assert_eq!(download.downloaded_range(), downloaded);
        self.cache
            .insert_and_record(access.object_key.clone(), download, access.requested);
        Ok(())
    }

    fn used_payload_bytes(&self) -> u64 {
        self.cache.used_bytes()
    }
}

type NativeFoyerKey = (ObjectKey, ByteRange);

#[derive(Clone)]
struct NativeFoyerValue {
    downloaded: ByteRange,
    payload: Bytes,
}

#[derive(Clone, Copy)]
enum NativeFoyerKeyMode {
    /// Key entries by the application's exact requested range.
    ExactRequest,
    /// Expand before lookup and key entries by that exact expanded range.
    ExpandedDownload,
}

impl NativeFoyerKeyMode {
    const fn name(self) -> &'static str {
        match self {
            Self::ExactRequest => "foyer-native-exact-key",
            Self::ExpandedDownload => "foyer-native-expanded-key",
        }
    }

    const fn range(self, access: &Access, downloaded: ByteRange) -> ByteRange {
        match self {
            Self::ExactRequest => access.requested,
            Self::ExpandedDownload => downloaded,
        }
    }
}

struct NativeFoyerEngine {
    cache: FoyerCache<NativeFoyerKey, NativeFoyerValue>,
    key_mode: NativeFoyerKeyMode,
}

impl NativeFoyerEngine {
    fn new(capacity: usize, shards: usize, key_mode: NativeFoyerKeyMode) -> Self {
        Self {
            cache: foyer_cache(capacity, shards),
            key_mode,
        }
    }

    fn key(&self, access: &Access, downloaded: ByteRange) -> NativeFoyerKey {
        (access.object_key.clone(), self.key_mode.range(access, downloaded))
    }
}

impl Engine for NativeFoyerEngine {
    fn name(&self) -> &'static str {
        self.key_mode.name()
    }

    fn get(&mut self, access: &Access, downloaded: ByteRange) -> bool {
        let key = self.key(access, downloaded);
        let Some(entry) = self.cache.get(&key) else {
            return false;
        };
        let value = entry.value();
        debug_assert!(value.downloaded.contains(access.requested));
        let result = requested_payload(&value.payload, value.downloaded, access.requested);
        debug_assert_eq!(result.len() as u64, access.requested.len());
        true
    }

    fn populate(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<(), String> {
        let key = self.key(access, downloaded);
        self.cache.insert(key, NativeFoyerValue { downloaded, payload });
        Ok(())
    }

    fn used_payload_bytes(&self) -> u64 {
        self.cache.usage() as u64
    }
}

fn foyer_cache(capacity: usize, shards: usize) -> FoyerCache<NativeFoyerKey, NativeFoyerValue> {
    CacheBuilder::new(capacity)
        .with_shards(shards)
        .with_eviction_config(S3FifoConfig::default())
        .with_weighter(|_key: &NativeFoyerKey, value: &NativeFoyerValue| value.payload.len())
        .build()
}

#[derive(Default)]
struct Traffic {
    requests: u64,
    requested_bytes: u64,
    hits: u64,
    hit_bytes: u64,
    source_requests: u64,
    source_bytes: u64,
}

struct Report {
    workload: &'static str,
    downloader: &'static str,
    shards: usize,
    capacity: usize,
    engine: &'static str,
    traffic: Traffic,
    used_payload_bytes: u64,
    elapsed: Duration,
}

impl Report {
    fn cache_hit_rate(&self) -> f64 {
        ratio(self.traffic.hits, self.traffic.requests)
    }

    fn byte_hit_rate(&self) -> f64 {
        ratio(self.traffic.hit_bytes, self.traffic.requested_bytes)
    }

    fn source_cost_hit_rate(&self) -> f64 {
        let fixed_cost = u128::from(SOURCE_FIXED_EQUIVALENT_BYTES);
        let baseline = u128::from(self.traffic.requests) * fixed_cost + u128::from(self.traffic.requested_bytes);
        let actual = u128::from(self.traffic.source_requests) * fixed_cost + u128::from(self.traffic.source_bytes);
        if baseline == 0 {
            0.0
        } else {
            1.0 - actual as f64 / baseline as f64
        }
    }

    fn operations_per_second(&self) -> f64 {
        self.traffic.requests as f64 / self.elapsed.as_secs_f64()
    }
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    if args.capacities.is_empty() || args.capacities.contains(&0) {
        return Err("--capacity values must be nonzero".to_owned());
    }
    if args.shards.is_empty() || args.shards.contains(&0) {
        return Err("--shards values must be nonzero".to_owned());
    }
    if args.downloaders.is_empty() {
        return Err("at least one --downloader value is required".to_owned());
    }
    let download_config = DownloadConfig::from_env()?;
    let workload = trace_workload(&args, download_config)?;

    if args.csv {
        eprintln!(
            "foyer_revision={PINNED_FOYER_REVISION} trace={TRACE_FILE} shards={:?} downloaders={:?} warmup_iterations={} coalescing_window_ms={COALESCING_WINDOW_MILLIS} coalescing_distance_bytes={} whole_split_threshold_bytes={}",
            args.shards,
            args.downloaders,
            args.warmup_iterations,
            download_config.coalescing_distance_bytes,
            download_config.whole_split_threshold_bytes,
        );
        println!("{CSV_HEADER}");
    } else {
        print_human_header(&args, &workload);
    }

    for &downloader in &args.downloaders {
        let max_download = max_download_len(&workload, downloader);
        let max_download = usize::try_from(max_download).map_err(|_| "largest callback payload does not fit usize")?;
        let source_payload = Bytes::from(vec![0x5a; max_download]);

        for &shards in &args.shards {
            for &capacity in &args.capacities {
                let mut engines: Vec<Box<dyn Engine>> = vec![Box::new(FeuerEngine::new(capacity, shards))];
                engines.push(Box::new(NativeFoyerEngine::new(
                    capacity,
                    shards,
                    NativeFoyerKeyMode::ExactRequest,
                )));
                if matches!(downloader, DownloadPolicy::Expanded) {
                    engines.push(Box::new(NativeFoyerEngine::new(
                        capacity,
                        shards,
                        NativeFoyerKeyMode::ExpandedDownload,
                    )));
                }
                for engine in engines {
                    let report = run_engine(
                        engine,
                        &workload,
                        downloader,
                        shards,
                        capacity,
                        args.warmup_iterations,
                        &source_payload,
                    )?;
                    if args.csv {
                        print_csv_report(&report);
                    } else {
                        print_human_report(&report);
                    }
                }
            }
        }
    }
    Ok(())
}

fn trace_workload(args: &Args, download_config: DownloadConfig) -> Result<Workload, String> {
    let mut accesses = load_trace()?;
    if let Some(operations) = args.operations {
        accesses.truncate(operations);
    }
    if accesses.is_empty() {
        return Err("trace is empty".to_owned());
    }
    for (index, access) in accesses.iter().enumerate() {
        access.validate(index)?;
        if index > 0 && access.timestamp_millis < accesses[index - 1].timestamp_millis {
            return Err(format!(
                "operation {index} has a timestamp earlier than its predecessor"
            ));
        }
    }
    let expanded_downloads = expanded_download_ranges(&accesses, download_config);
    Ok(Workload {
        name: "trace",
        accesses,
        expanded_downloads,
        download_config,
    })
}

fn run_engine(
    mut engine: Box<dyn Engine>,
    workload: &Workload,
    downloader: DownloadPolicy,
    shards: usize,
    capacity: usize,
    warmup_iterations: usize,
    source_payload: &Bytes,
) -> Result<Report, String> {
    for _ in 0..warmup_iterations {
        let mut warmup_traffic = Traffic::default();
        execute_pass(&mut *engine, workload, downloader, source_payload, &mut warmup_traffic)?;
    }

    let mut traffic = Traffic::default();
    let started = Instant::now();
    execute_pass(&mut *engine, workload, downloader, source_payload, &mut traffic)?;
    let elapsed = started.elapsed();

    Ok(Report {
        workload: workload.name,
        downloader: downloader.name(),
        shards,
        capacity,
        engine: engine.name(),
        traffic,
        used_payload_bytes: engine.used_payload_bytes(),
        elapsed,
    })
}

#[derive(Clone, Copy)]
struct PendingDownload<'a> {
    order: usize,
    access: &'a Access,
    downloaded: ByteRange,
}

struct CoalescedDownload<'a> {
    downloaded: ByteRange,
    accesses: Vec<(usize, &'a Access)>,
}

fn expanded_download_ranges(workload: &[Access], config: DownloadConfig) -> Vec<ByteRange> {
    let mut by_object: HashMap<(&str, u64), Vec<usize>> = HashMap::new();
    for (index, access) in workload.iter().enumerate() {
        by_object
            .entry((&access.object_key, access.object_size))
            .or_default()
            .push(index);
    }

    let base_ranges: Vec<_> = workload
        .iter()
        .map(|access| access.downloaded_range(DownloadPolicy::Expanded, config))
        .collect();
    let mut ranges = base_ranges.clone();
    // The first batch that claims a request fixes its expansion. Recomputing a
    // sliding window for each member would produce different native Foyer keys
    // for requests served by the same coalesced download.
    let mut assigned = vec![false; workload.len()];
    for indexes in by_object.values() {
        for (position, &index) in indexes.iter().enumerate() {
            if assigned[index] {
                continue;
            }

            let deadline = workload[index]
                .timestamp_millis
                .saturating_add(COALESCING_WINDOW_MILLIS);
            let pending = indexes[position..]
                .iter()
                .copied()
                .take_while(|&candidate| workload[candidate].timestamp_millis <= deadline)
                .filter(|&candidate| !assigned[candidate])
                .map(|candidate| PendingDownload {
                    order: candidate,
                    access: &workload[candidate],
                    downloaded: base_ranges[candidate],
                })
                .collect();
            let download = coalesced_downloads(pending, config.coalescing_distance_bytes)
                .into_iter()
                .find(|download| download.accesses.iter().any(|(order, _)| *order == index))
                .expect("the current request must belong to one coalesced download");
            for (member, _) in download.accesses {
                ranges[member] = download.downloaded;
                assigned[member] = true;
            }
        }
    }
    ranges
}

fn max_download_len(workload: &Workload, downloader: DownloadPolicy) -> u64 {
    match downloader {
        DownloadPolicy::Expanded => workload
            .expanded_downloads
            .iter()
            .map(|range| range.len())
            .max()
            .unwrap_or(1),
        DownloadPolicy::Exact => workload
            .accesses
            .iter()
            .map(|access| {
                access
                    .downloaded_range(DownloadPolicy::Exact, workload.download_config)
                    .len()
            })
            .max()
            .unwrap_or(1),
    }
}

fn execute_pass<E: Engine + ?Sized>(
    engine: &mut E,
    workload: &Workload,
    downloader: DownloadPolicy,
    source_payload: &Bytes,
    traffic: &mut Traffic,
) -> Result<(), String> {
    for (index, access) in workload.accesses.iter().enumerate() {
        let downloaded = match downloader {
            DownloadPolicy::Expanded => workload.expanded_downloads[index],
            DownloadPolicy::Exact => access.downloaded_range(DownloadPolicy::Exact, workload.download_config),
        };
        let hit = engine.get(access, downloaded);
        record_request(traffic, access, hit);
        if hit {
            continue;
        }

        let payload = source_payload_slice(source_payload, downloaded)?;
        engine.populate(access, downloaded, payload)?;
        traffic.source_requests += 1;
        traffic.source_bytes += downloaded.len();
    }
    Ok(())
}

fn coalesced_downloads(
    mut pending: Vec<PendingDownload<'_>>,
    coalescing_distance_bytes: u64,
) -> Vec<CoalescedDownload<'_>> {
    pending.sort_by(|left, right| {
        left.access
            .object_key
            .cmp(&right.access.object_key)
            .then_with(|| left.access.object_size.cmp(&right.access.object_size))
            .then_with(|| left.downloaded.cmp(&right.downloaded))
            .then_with(|| left.order.cmp(&right.order))
    });

    let mut downloads: Vec<CoalescedDownload<'_>> = Vec::new();
    for pending in pending {
        let merge = downloads.last_mut().filter(|download| {
            let representative = download.accesses[0].1;
            let gap = pending.downloaded.start().saturating_sub(download.downloaded.end());
            representative.object_key == pending.access.object_key
                && representative.object_size == pending.access.object_size
                && gap < coalescing_distance_bytes
        });
        if let Some(download) = merge {
            download.downloaded = ByteRange::new(
                download.downloaded.start().min(pending.downloaded.start()),
                download.downloaded.end().max(pending.downloaded.end()),
            )
            .expect("the union of coalesced non-empty ranges must be non-empty");
            download.accesses.push((pending.order, pending.access));
        } else {
            downloads.push(CoalescedDownload {
                downloaded: pending.downloaded,
                accesses: vec![(pending.order, pending.access)],
            });
        }
    }
    downloads
}

fn record_request(traffic: &mut Traffic, access: &Access, hit: bool) {
    traffic.requests += 1;
    traffic.requested_bytes += access.requested.len();
    if hit {
        traffic.hits += 1;
        traffic.hit_bytes += access.requested.len();
    }
}

fn source_payload_slice(source_payload: &Bytes, downloaded: ByteRange) -> Result<Bytes, String> {
    let payload_len = usize::try_from(downloaded.len()).map_err(|_| "callback payload length does not fit usize")?;
    if payload_len > source_payload.len() {
        return Err("coalesced callback payload exceeded the precomputed source allocation".to_owned());
    }
    Ok(source_payload.slice(..payload_len))
}

fn requested_payload(bytes: &Bytes, downloaded: ByteRange, requested: ByteRange) -> Bytes {
    debug_assert!(downloaded.contains(requested));
    let start = usize::try_from(requested.start() - downloaded.start())
        .expect("an offset within a callback payload must fit usize");
    let end = usize::try_from(requested.end() - downloaded.start())
        .expect("an offset within a callback payload must fit usize");
    bytes.slice(start..end)
}

fn print_human_header(args: &Args, workload: &Workload) {
    let shards = args.shards.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
    println!("Feuer memory benchmark");
    println!("Trace: {TRACE_FILE} ({} operations)", workload.accesses.len());
    println!("Shards: {shards} | Warm-up passes: {}", args.warmup_iterations);
    println!(
        "Expanded: {COALESCING_WINDOW_MILLIS}-ms coalescing within {} | Whole below {}",
        format_decimal_bytes(workload.download_config.coalescing_distance_bytes),
        format_bytes(workload.download_config.whole_split_threshold_bytes),
    );
    println!("Source model: 125 ms per GET + transfer at 80 MB/s");
    println!("Foyer revision: {PINNED_FOYER_REVISION}");
    println!();
    println!(
        "Capacity   Downloader Engine                   Request hit Source-cost hit Source GETs Source bytes         Used   Throughput"
    );
    println!("{}", "-".repeat(131));
}

fn print_human_report(report: &Report) {
    println!(
        "{:<10} {:<10} {:<24} {:>11.2}% {:>14.2}% {:>11} {:>12} {:>12} {:>12}",
        format_bytes(report.capacity as u64),
        report.downloader,
        human_engine_name(report.engine),
        report.cache_hit_rate() * 100.0,
        report.source_cost_hit_rate() * 100.0,
        report.traffic.source_requests,
        format_bytes(report.traffic.source_bytes),
        format_bytes(report.used_payload_bytes),
        format_rate(report.operations_per_second()),
    );
}

fn human_engine_name(engine: &str) -> &str {
    match engine {
        "feuer-value-density" => "Feuer",
        "foyer-native-exact-key" => "Foyer",
        "foyer-native-expanded-key" => "Foyer (expanded key)",
        other => other,
    }
}

fn format_decimal_bytes(bytes: u64) -> String {
    for (unit_bytes, suffix) in [(1_000_000_000_u64, "GB"), (1_000_000, "MB"), (1_000, "KB")] {
        if bytes >= unit_bytes {
            if bytes.is_multiple_of(unit_bytes) {
                return format!("{} {suffix}", bytes / unit_bytes);
            }
            return format!("{:.1} {suffix}", bytes as f64 / unit_bytes as f64);
        }
    }
    format!("{bytes} B")
}

fn format_bytes(bytes: u64) -> String {
    for (unit_bytes, suffix) in [(1_u64 << 30, "GiB"), (1_u64 << 20, "MiB"), (1_u64 << 10, "KiB")] {
        if bytes >= unit_bytes {
            if bytes.is_multiple_of(unit_bytes) {
                return format!("{} {suffix}", bytes / unit_bytes);
            }
            return format!("{:.1} {suffix}", bytes as f64 / unit_bytes as f64);
        }
    }
    format!("{bytes} B")
}

fn format_rate(operations_per_second: f64) -> String {
    if operations_per_second >= 1_000_000.0 {
        format!("{:.2} M/s", operations_per_second / 1_000_000.0)
    } else if operations_per_second >= 1_000.0 {
        format!("{:.2} K/s", operations_per_second / 1_000.0)
    } else {
        format!("{operations_per_second:.0}/s")
    }
}

fn print_csv_report(report: &Report) {
    println!(
        "{},{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{},{},{},{:.4},{:.3},{:.0}",
        report.workload,
        report.downloader,
        report.shards,
        report.capacity,
        report.engine,
        report.traffic.requests,
        report.traffic.requested_bytes,
        report.traffic.hits,
        report.traffic.hit_bytes,
        report.cache_hit_rate() * 100.0,
        report.byte_hit_rate() * 100.0,
        report.source_cost_hit_rate() * 100.0,
        report.traffic.source_requests,
        report.traffic.source_bytes,
        report.used_payload_bytes,
        report.used_payload_bytes as f64 / report.capacity as f64 * 100.0,
        report.elapsed.as_secs_f64() * 1_000.0,
        report.operations_per_second(),
    );
}

fn load_trace() -> Result<Vec<Access>, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(TRACE_FILE);
    let content = fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            parse_trace_line(line).map_err(|error| format!("{}:{}: {error}", path.display(), index + 1))
        })
        .collect()
}

fn parse_trace_line(line: &str) -> Result<Access, String> {
    let object_key = find_json_string(line, "object_id")?;
    let object_size = find_json_u64(line, "object_num_bytes")?;
    let start = find_json_u64(line, "requested_range_start")?;
    let end = find_json_u64(line, "requested_range_end")?;
    let requested = ByteRange::new(start, end).map_err(|error| error.to_string())?;
    let timestamp = find_json_string(line, "timestamp")?;
    let timestamp_millis = parse_timestamp_millis(&timestamp)?;
    Ok(Access {
        object_key: ObjectKey::from(object_key),
        object_size,
        requested,
        timestamp_millis,
    })
}

fn parse_timestamp_millis(value: &str) -> Result<u64, String> {
    let bytes = value.as_bytes();
    let valid_suffix =
        (bytes.len() == 20 && bytes[19] == b'Z') || (bytes.len() == 24 && bytes[19] == b'.' && bytes[23] == b'Z');
    if !valid_suffix
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(format!("timestamp {value:?} is not a supported UTC RFC 3339 value"));
    }

    let component = |start: usize, end: usize, name: &str| {
        value[start..end]
            .parse::<u64>()
            .map_err(|error| format!("invalid timestamp {name}: {error}"))
    };
    let year = component(0, 4, "year")?;
    let month = component(5, 7, "month")?;
    let day = component(8, 10, "day")?;
    let hour = component(11, 13, "hour")?;
    let minute = component(14, 16, "minute")?;
    let second = component(17, 19, "second")?;
    let millis = if bytes.len() == 24 {
        component(20, 23, "millisecond")?
    } else {
        0
    };

    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return Err(format!("timestamp {value:?} contains an out-of-range component"));
    }
    let leap_year = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_lengths = [
        31_u64,
        28 + u64::from(leap_year),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let month_index = usize::try_from(month - 1).expect("a validated month index must fit usize");
    if day == 0 || day > month_lengths[month_index] {
        return Err(format!("timestamp {value:?} contains an out-of-range day"));
    }

    let previous_year = year - 1;
    let days_before_year = previous_year * 365 + previous_year / 4 - previous_year / 100 + previous_year / 400;
    let days_before_month: u64 = month_lengths[..month_index].iter().sum();
    let days = days_before_year + days_before_month + day - 1;
    Ok(((((days * 24 + hour) * 60 + minute) * 60 + second) * 1_000) + millis)
}

fn find_json_string(line: &str, field: &str) -> Result<String, String> {
    let value = field_value(line, field)?;
    let value = value
        .strip_prefix('"')
        .ok_or_else(|| format!("field {field:?} is not a string"))?;
    let end = value
        .find('"')
        .ok_or_else(|| format!("field {field:?} has no closing quote"))?;
    Ok(value[..end].to_owned())
}

fn find_json_u64(line: &str, field: &str) -> Result<u64, String> {
    let value = field_value(line, field)?;
    let end = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    if end == 0 {
        return Err(format!("field {field:?} is not an unsigned integer"));
    }
    value[..end]
        .parse()
        .map_err(|error| format!("invalid {field:?}: {error}"))
}

fn field_value<'a>(line: &'a str, field: &str) -> Result<&'a str, String> {
    let needle = format!("\"{field}\"");
    let after_field = line
        .find(&needle)
        .map(|index| &line[index + needle.len()..])
        .ok_or_else(|| format!("missing field {field:?}"))?;
    let after_colon = after_field
        .find(':')
        .map(|index| &after_field[index + 1..])
        .ok_or_else(|| format!("missing colon after field {field:?}"))?;
    Ok(after_colon.trim_start())
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn byte_count_from_env(name: &str, default: u64) -> Result<u64, String> {
    let value = match env::var(name) {
        Ok(value) => value,
        Err(VarError::NotPresent) => return Ok(default),
        Err(VarError::NotUnicode(_)) => return Err(format!("{name} is not valid Unicode")),
    };
    let bytes = parse_byte_count(&value).map_err(|error| format!("invalid {name}: {error}"))?;
    u64::try_from(bytes).map_err(|_| format!("{name} does not fit u64"))
}

fn parse_bytes(value: &str) -> Result<usize, String> {
    let bytes = parse_byte_count(value)?;
    usize::try_from(bytes).map_err(|_| "byte count does not fit usize".to_owned())
}

fn parse_byte_count(value: &str) -> Result<u128, String> {
    let value = value.trim();
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let number: u128 = value[..split]
        .parse()
        .map_err(|error| format!("invalid byte count: {error}"))?;
    let suffix = value[split..].trim().to_ascii_lowercase();
    let multiplier = match suffix.as_str() {
        "" | "b" => 1_u128,
        "kib" => 1_u128 << 10,
        "mib" => 1_u128 << 20,
        "gib" => 1_u128 << 30,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        _ => return Err(format!("unsupported byte suffix {suffix:?}")),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| "byte count overflowed".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct WarmupEngine {
        populated: bool,
    }

    impl Engine for WarmupEngine {
        fn name(&self) -> &'static str {
            "warmup-test"
        }

        fn get(&mut self, _access: &Access, _downloaded: ByteRange) -> bool {
            self.populated
        }

        fn populate(&mut self, _access: &Access, _downloaded: ByteRange, _payload: Bytes) -> Result<(), String> {
            self.populated = true;
            Ok(())
        }

        fn used_payload_bytes(&self) -> u64 {
            u64::from(self.populated)
        }
    }

    #[derive(Default)]
    struct RangeEngine {
        entries: Vec<(ObjectKey, ByteRange)>,
    }

    impl Engine for RangeEngine {
        fn name(&self) -> &'static str {
            "range-test"
        }

        fn get(&mut self, access: &Access, _downloaded: ByteRange) -> bool {
            self.entries
                .iter()
                .any(|(key, downloaded)| key == &access.object_key && downloaded.contains(access.requested))
        }

        fn populate(&mut self, access: &Access, downloaded: ByteRange, _payload: Bytes) -> Result<(), String> {
            assert!(downloaded.contains(access.requested));
            self.entries.push((access.object_key.clone(), downloaded));
            Ok(())
        }

        fn used_payload_bytes(&self) -> u64 {
            self.entries.iter().map(|(_, range)| range.len()).sum()
        }
    }

    #[test]
    fn warmup_preserves_cache_state_but_not_reported_traffic() {
        let workload = Workload {
            name: "test",
            accesses: vec![Access {
                object_key: "object".into(),
                object_size: 1,
                requested: ByteRange::new(0, 1).unwrap(),
                timestamp_millis: 0,
            }],
            expanded_downloads: vec![ByteRange::new(0, 1).unwrap()],
            download_config: DownloadConfig {
                coalescing_distance_bytes: DEFAULT_COALESCING_DISTANCE_BYTES,
                whole_split_threshold_bytes: DEFAULT_WHOLE_SPLIT_THRESHOLD_BYTES,
            },
        };
        let report = run_engine(
            Box::new(WarmupEngine { populated: false }),
            &workload,
            DownloadPolicy::Exact,
            1,
            1,
            1,
            &Bytes::from_static(&[0]),
        )
        .unwrap();

        assert_eq!(report.traffic.requests, 1);
        assert_eq!(report.traffic.hits, 1);
        assert_eq!(report.traffic.source_requests, 0);
        assert_eq!(report.used_payload_bytes, 1);
    }

    #[test]
    fn foyer_expanded_key_reuses_identical_expansions_for_distinct_requests() {
        let access = |start, end| Access {
            object_key: "object".into(),
            object_size: 20,
            requested: ByteRange::new(start, end).unwrap(),
            timestamp_millis: 0,
        };
        let first = access(2, 4);
        let second = access(12, 14);
        let expanded = ByteRange::new(0, 20).unwrap();

        let mut expanded_key = NativeFoyerEngine::new(1 << 20, 1, NativeFoyerKeyMode::ExpandedDownload);
        assert!(!expanded_key.get(&first, expanded));
        expanded_key
            .populate(&first, expanded, Bytes::from(vec![0; 20]))
            .unwrap();
        assert!(expanded_key.get(&second, expanded));

        let mut exact_key = NativeFoyerEngine::new(1 << 20, 1, NativeFoyerKeyMode::ExactRequest);
        exact_key.populate(&first, expanded, Bytes::from(vec![0; 20])).unwrap();
        assert!(!exact_key.get(&second, expanded));
    }

    #[test]
    fn expanded_downloader_assigns_one_coalesced_range_to_all_batch_members() {
        let access = |start, end, timestamp_millis| Access {
            object_key: "object".into(),
            object_size: 100,
            requested: ByteRange::new(start, end).unwrap(),
            timestamp_millis,
        };
        let accesses = vec![
            access(0, 10, 0),
            access(10, 20, 1),
            access(20, 30, 5),
            access(50, 60, 6),
        ];
        let download_config = DownloadConfig {
            coalescing_distance_bytes: DEFAULT_COALESCING_DISTANCE_BYTES,
            whole_split_threshold_bytes: 0,
        };
        let expanded_downloads = expanded_download_ranges(&accesses, download_config);
        let workload = Workload {
            name: "test",
            accesses,
            expanded_downloads,
            download_config,
        };
        assert_eq!(&workload.expanded_downloads[..3], &[ByteRange::new(0, 30).unwrap(); 3]);

        let report = run_engine(
            Box::new(RangeEngine::default()),
            &workload,
            DownloadPolicy::Expanded,
            1,
            100,
            0,
            &Bytes::from(vec![0; 60]),
        )
        .unwrap();

        assert_eq!(report.traffic.requests, 4);
        assert_eq!(report.traffic.hits, 2);
        assert_eq!(report.traffic.source_requests, 2);
        assert_eq!(report.traffic.source_bytes, 40);
    }

    #[test]
    fn coalescing_distance_parameter_is_a_strict_upper_bound() {
        let distance = DEFAULT_COALESCING_DISTANCE_BYTES;
        let ranges_for_gap = |gap| {
            let accesses = vec![
                Access {
                    object_key: "object".into(),
                    object_size: 2 * distance,
                    requested: ByteRange::new(0, 1).unwrap(),
                    timestamp_millis: 0,
                },
                Access {
                    object_key: "object".into(),
                    object_size: 2 * distance,
                    requested: ByteRange::new(1 + gap, 2 + gap).unwrap(),
                    timestamp_millis: 1,
                },
            ];
            expanded_download_ranges(
                &accesses,
                DownloadConfig {
                    coalescing_distance_bytes: distance,
                    whole_split_threshold_bytes: 0,
                },
            )
        };

        assert_eq!(
            ranges_for_gap(distance - 1)[0],
            ByteRange::new(0, distance + 1).unwrap()
        );
        assert_eq!(ranges_for_gap(distance)[0], ByteRange::new(0, 1).unwrap());
    }

    #[test]
    fn source_cost_baseline_uses_requested_bytes_not_downloaded_bytes() {
        let report = Report {
            workload: "test",
            downloader: "expanded",
            shards: 1,
            capacity: 1,
            engine: "test",
            traffic: Traffic {
                requests: 1,
                requested_bytes: 100,
                source_requests: 1,
                source_bytes: 200,
                ..Traffic::default()
            },
            used_payload_bytes: 0,
            elapsed: Duration::from_secs(1),
        };

        assert!(report.source_cost_hit_rate() < 0.0);
    }

    #[test]
    fn parses_the_captured_trace_shape() {
        let access = parse_trace_line(
            r#"{"object_num_bytes":100,"object_id":"object-a","requested_range_end":11,"requested_range_start":7,"timestamp":"2026-08-08T01:12:49.481Z"}"#,
        )
        .unwrap();
        assert_eq!(access.object_key, ObjectKey::from("object-a"));
        assert_eq!(access.object_size, 100);
        assert_eq!(access.requested, ByteRange::new(7, 11).unwrap());
        assert_eq!(
            access.timestamp_millis,
            parse_timestamp_millis("2026-08-08T01:12:49.481Z").unwrap()
        );
    }

    #[test]
    fn expanded_downloader_honors_the_whole_split_threshold() {
        let config = DownloadConfig {
            coalescing_distance_bytes: DEFAULT_COALESCING_DISTANCE_BYTES,
            whole_split_threshold_bytes: 8 << 20,
        };
        let small_split = Access {
            object_key: "small-split".into(),
            object_size: config.whole_split_threshold_bytes - 1,
            requested: ByteRange::new(3 << 20, 6 << 20).unwrap(),
            timestamp_millis: 0,
        };
        assert_eq!(
            small_split.downloaded_range(DownloadPolicy::Expanded, config),
            ByteRange::new(0, config.whole_split_threshold_bytes - 1).unwrap()
        );
        assert_eq!(
            small_split.downloaded_range(DownloadPolicy::Exact, config),
            small_split.requested
        );

        let threshold_split = Access {
            object_key: "threshold-split".into(),
            object_size: config.whole_split_threshold_bytes,
            requested: ByteRange::new((1 << 20) + 3, (2 << 20) + 3).unwrap(),
            timestamp_millis: 0,
        };
        assert_eq!(
            threshold_split.downloaded_range(DownloadPolicy::Expanded, config),
            threshold_split.requested
        );
    }

    #[test]
    fn timestamp_parser_handles_day_boundaries() {
        let before = parse_timestamp_millis("2026-08-08T23:59:59.999Z").unwrap();
        let after = parse_timestamp_millis("2026-08-09T00:00:00.000Z").unwrap();
        assert_eq!(after - before, 1);
        assert!(
            parse_timestamp_millis("2026-08-09T00:00:01Z")
                .unwrap()
                .is_multiple_of(1_000)
        );
    }

    #[test]
    fn environment_byte_counts_accept_documented_units() {
        assert_eq!(parse_byte_count("1MB").unwrap(), 1_000_000);
        assert_eq!(parse_byte_count("8MiB").unwrap(), 8 << 20);
    }

    #[test]
    fn human_output_is_default_and_csv_is_opt_in() {
        assert!(!Args::try_parse_from(["benchmark"]).unwrap().csv);
        assert!(Args::try_parse_from(["benchmark", "--csv"]).unwrap().csv);
        assert_eq!(format_bytes(8 << 20), "8 MiB");
        assert_eq!(format_decimal_bytes(10_000_000), "10 MB");
    }
}
