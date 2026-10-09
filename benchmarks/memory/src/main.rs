//! Controlled memory-tier replay against Feuer and a pinned Foyer fork.

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use bytes::Bytes;
use clap::{Parser, ValueEnum};
use feuer_historian::{ACCESS_COUNT_HALF_LIFE, MAX_ACCESS_AGE_ACCESSES, MAX_ACCESS_EVENTS_PER_KEY};
use feuer_memory::{MemoryCache, retention::RetrievalCostScorer};
use feuer_types::{
    ByteRange, Download, ObjectKeyHash,
    config::{parse_config_number, read_env_number},
};
use foyer_memory::{Cache as FoyerCache, CacheBuilder, CostAwareConfig, S3FifoConfig};

const PINNED_FOYER_REVISION: &str = "14c2d88b9d7dd2135bfc723d0967debb59532b4b";
const SOURCE_FIXED_EQUIVALENT_BYTES: u64 = 10_000_000;
// Keep the replay's source-cost assumption explicit rather than relying on the cache default.
static FIXED_RETRIEVAL_EQUIVALENT_BYTES: LazyLock<u64> = LazyLock::new(|| {
    read_env_number(
        "FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES",
        SOURCE_FIXED_EQUIVALENT_BYTES,
        0,
    )
    .unwrap_or_else(|error| panic!("{error}"))
});
/// Per-eviction comparison budget, matched to Feuer's candidate sample.
const FOYER_COST_SAMPLE_SIZE: usize = 64;
const TRACE_FILE: &str = "access_pattern.ndjson";
const COALESCING_DISTANCE_ENV: &str = "COALESCING_DISTANCE_BYTES";
const WHOLE_SPLIT_THRESHOLD_ENV: &str = "WHOLE_SPLIT_THRESHOLD_BYTES";
const DEFAULT_COALESCING_DISTANCE_BYTES: u64 = SOURCE_FIXED_EQUIVALENT_BYTES;
const DEFAULT_WHOLE_SPLIT_THRESHOLD_BYTES: u64 = 8 << 20;
const COALESCING_WINDOW_MILLIS: u64 = 5;
const CSV_HEADER: &str = "workload,downloader,shards,capacity_bytes,engine,requests,requested_bytes,cache_hits,hit_bytes,cache_hit_pct,byte_hit_pct,source_cost_hit_pct,source_gets,source_bytes,used_payload_bytes,used_vs_target_pct,elapsed_ms,operations_per_second";

#[derive(Debug, Parser)]
#[command(about = "Replay the captured trace against Feuer and a pinned Foyer fork")]
struct ReplayArgs {
    /// Soft payload capacities. Repeat the flag or separate values with commas.
    #[arg(
        long = "capacity",
        default_value = "256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,16GiB,32GiB",
        value_delimiter = ',',
        value_parser = parse_byte_count_usize
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
    downloaders: Vec<DownloadRangePolicy>,

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
enum DownloadRangePolicy {
    Expanded,
    Exact,
}

impl DownloadRangePolicy {
    const fn name(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Exact => "exact",
        }
    }

    fn ranges(self, workload: &ReplayWorkload) -> impl Iterator<Item = ByteRange> + '_ {
        let requests = workload.requests.iter().enumerate();
        requests.map(move |(index, request)| match self {
            Self::Expanded => workload.expanded_downloads[index],
            Self::Exact => request.requested_range,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DownloadExpansionConfig {
    coalescing_distance_bytes: u64,
    whole_split_threshold_bytes: u64,
}

impl DownloadExpansionConfig {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            coalescing_distance_bytes: read_env_number(COALESCING_DISTANCE_ENV, DEFAULT_COALESCING_DISTANCE_BYTES, 0)?,
            whole_split_threshold_bytes: read_env_number(
                WHOLE_SPLIT_THRESHOLD_ENV,
                DEFAULT_WHOLE_SPLIT_THRESHOLD_BYTES,
                0,
            )?,
        })
    }
}

#[derive(Clone)]
struct TraceRequest {
    object_key: ObjectKeyHash,
    object_size: u64,
    requested_range: ByteRange,
    timestamp_millis: u64,
}

impl TraceRequest {
    fn expanded_download_range(&self, config: DownloadExpansionConfig) -> ByteRange {
        if self.object_size < config.whole_split_threshold_bytes {
            ByteRange::new(0, self.object_size)
                .expect("an object containing a valid non-empty request must be non-empty")
        } else {
            self.requested_range
        }
    }
}

struct ReplayWorkload {
    name: &'static str,
    requests: Box<[TraceRequest]>,
    expanded_downloads: Box<[ByteRange]>,
}

/// A cache receiving insertions and lookups while replaying a workload.
trait ReplayCache {
    fn name(&self) -> &'static str;

    /// Looks up a request and reports a hit, without returning the cached payload.
    /// Uses the range this downloader would fetch on a miss.
    fn lookup_hit(&mut self, request: &TraceRequest, downloaded_range: ByteRange) -> bool;

    /// Inserts a payload whose length matches the valid download range.
    fn insert(&mut self, request: &TraceRequest, downloaded_range: ByteRange, payload: Bytes);

    fn used_payload_bytes(&self) -> u64;
}

/// Feuer's memory cache used for workload replay.
struct FeuerReplayCache {
    cache: MemoryCache,
}

impl FeuerReplayCache {
    fn new(capacity: usize, num_shards: usize) -> Self {
        Self {
            cache: MemoryCache::with_shards_for_benchmark(capacity as u64, num_shards).with_retention_scorer(Arc::new(
                RetrievalCostScorer {
                    fixed_retrieval_equivalent_bytes: *FIXED_RETRIEVAL_EQUIVALENT_BYTES,
                },
            )),
        }
    }
}

impl ReplayCache for FeuerReplayCache {
    fn name(&self) -> &'static str {
        "feuer-value-density"
    }

    /// Looks up the requested bytes in Feuer and reports whether the lookup hit.
    fn lookup_hit(&mut self, request: &TraceRequest, _downloaded_range: ByteRange) -> bool {
        self.cache
            .access_histories()
            .record_access(&request.object_key, request.requested_range);
        self.cache.get(&request.object_key, request.requested_range).is_some()
    }

    fn insert(&mut self, request: &TraceRequest, downloaded_range: ByteRange, payload: Bytes) {
        let download = Download::new(downloaded_range.start(), payload)
            .expect("the payload matches a representable download range");
        debug_assert_eq!(download.downloaded_range(), downloaded_range);
        self.cache.insert(request.object_key, download);
    }

    fn used_payload_bytes(&self) -> u64 {
        self.cache.used_bytes()
    }
}

type FoyerRangeKey = (ObjectKeyHash, ByteRange);

#[derive(Clone, Copy)]
enum FoyerKeyRange {
    /// Key entries by the application's exact requested range.
    ExactRequest,
    /// Expand before lookup and key entries by that exact expanded range.
    ExpandedDownload,
}

impl FoyerKeyRange {
    const fn range(self, request: &TraceRequest, downloaded_range: ByteRange) -> ByteRange {
        match self {
            Self::ExactRequest => request.requested_range,
            Self::ExpandedDownload => downloaded_range,
        }
    }
}

#[derive(Clone, Copy)]
enum FoyerEvictionPolicy {
    S3Fifo,
    CostAware,
}

impl FoyerEvictionPolicy {
    const fn engine_name(self, key_range: FoyerKeyRange) -> &'static str {
        match (self, key_range) {
            (Self::S3Fifo, FoyerKeyRange::ExactRequest) => "foyer-native-exact-key",
            (Self::S3Fifo, FoyerKeyRange::ExpandedDownload) => "foyer-native-expanded-key",
            (Self::CostAware, FoyerKeyRange::ExactRequest) => "foyer-cost-aware-exact-key",
            (Self::CostAware, FoyerKeyRange::ExpandedDownload) => "foyer-cost-aware-expanded-key",
        }
    }
}

/// Foyer's native key-value cache used for workload replay.
struct FoyerReplayCache {
    cache: FoyerCache<FoyerRangeKey, Bytes>,
    key_range: FoyerKeyRange,
    eviction_policy: FoyerEvictionPolicy,
}

impl FoyerReplayCache {
    fn new(capacity: usize, num_shards: usize, key_range: FoyerKeyRange, eviction_policy: FoyerEvictionPolicy) -> Self {
        Self {
            cache: build_foyer_cache(capacity, num_shards, eviction_policy),
            key_range,
            eviction_policy,
        }
    }

    fn key(&self, request: &TraceRequest, downloaded_range: ByteRange) -> FoyerRangeKey {
        (request.object_key, self.key_range.range(request, downloaded_range))
    }
}

impl ReplayCache for FoyerReplayCache {
    fn name(&self) -> &'static str {
        self.eviction_policy.engine_name(self.key_range)
    }

    /// Looks up the selected Foyer key and reports whether the lookup hit.
    fn lookup_hit(&mut self, request: &TraceRequest, downloaded_range: ByteRange) -> bool {
        let key = self.key(request, downloaded_range);
        self.cache.get(&key).is_some()
    }

    fn insert(&mut self, request: &TraceRequest, downloaded_range: ByteRange, payload: Bytes) {
        let key = self.key(request, downloaded_range);
        self.cache.insert(key, payload);
    }

    fn used_payload_bytes(&self) -> u64 {
        self.cache.usage() as u64
    }
}

/// Builds a Foyer cache with payload-byte accounting and the selected eviction policy.
fn build_foyer_cache(
    capacity: usize,
    num_shards: usize,
    eviction_policy: FoyerEvictionPolicy,
) -> FoyerCache<FoyerRangeKey, Bytes> {
    let builder = CacheBuilder::new(capacity)
        .with_shards(num_shards)
        .with_weighter(|_key: &FoyerRangeKey, payload: &Bytes| payload.len());
    match eviction_policy {
        FoyerEvictionPolicy::S3Fifo => builder.with_eviction_config(S3FifoConfig::default()).build(),
        FoyerEvictionPolicy::CostAware => builder
            .with_eviction_config(CostAwareConfig {
                fixed_retrieval_cost: SOURCE_FIXED_EQUIVALENT_BYTES,
                sample_size: FOYER_COST_SAMPLE_SIZE,
            })
            .build(),
    }
}

#[derive(Default)]
struct ReplayTraffic {
    requests: u64,
    requested_bytes: u64,
    hits: u64,
    hit_bytes: u64,
    source_bytes: u64,
}

struct ReplayReport {
    workload: &'static str,
    downloader: &'static str,
    shards: usize,
    capacity: usize,
    engine: &'static str,
    traffic: ReplayTraffic,
    used_payload_bytes: u64,
    elapsed: Duration,
}

impl ReplayReport {
    fn cache_hit_rate(&self) -> f64 {
        self.traffic.hits as f64 / self.traffic.requests as f64
    }

    fn byte_hit_rate(&self) -> f64 {
        self.traffic.hit_bytes as f64 / self.traffic.requested_bytes.max(1) as f64
    }

    /// Returns the source-cost savings ratio against fetching every exact request; extra bytes can make it negative.
    fn source_cost_savings_ratio(&self) -> f64 {
        let fixed_cost = u128::from(SOURCE_FIXED_EQUIVALENT_BYTES);
        let uncached_source_cost =
            u128::from(self.traffic.requests) * fixed_cost + u128::from(self.traffic.requested_bytes);
        let cached_source_cost =
            u128::from(self.traffic.requests - self.traffic.hits) * fixed_cost + u128::from(self.traffic.source_bytes);
        1.0 - cached_source_cost as f64 / uncached_source_cost as f64
    }

    fn operations_per_second(&self) -> f64 {
        self.traffic.requests as f64 / self.elapsed.as_secs_f64()
    }
}

fn main() -> Result<(), String> {
    let args = ReplayArgs::parse();
    if args.shards.contains(&0) {
        return Err("--shards values must be nonzero".to_owned());
    }
    let download_config = DownloadExpansionConfig::from_env()?;
    let workload = load_trace_workload(&args, download_config)?;

    if args.csv {
        eprintln!(
            "foyer_revision={PINNED_FOYER_REVISION} foyer_source=https://github.com/PSeitz/foyer foyer_cost_estimator=residence_access_rate foyer_cost_fixed_retrieval={SOURCE_FIXED_EQUIVALENT_BYTES} foyer_cost_sample_size={FOYER_COST_SAMPLE_SIZE} feuer_cost_estimator=decayed_frequency feuer_cost_half_life_accesses={} feuer_max_access_age_accesses={} feuer_max_access_events_per_key={} feuer_fixed_retrieval_equivalent_bytes={} trace={TRACE_FILE} shards={:?} downloaders={:?} warmup_iterations={} coalescing_window_ms={COALESCING_WINDOW_MILLIS} coalescing_distance_bytes={} whole_split_threshold_bytes={}",
            *ACCESS_COUNT_HALF_LIFE,
            *MAX_ACCESS_AGE_ACCESSES,
            *MAX_ACCESS_EVENTS_PER_KEY,
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES,
            args.shards,
            args.downloaders,
            args.warmup_iterations,
            download_config.coalescing_distance_bytes,
            download_config.whole_split_threshold_bytes,
        );
        println!("{CSV_HEADER}");
    } else {
        print_human_header(&args, &workload, download_config);
    }

    for &downloader in &args.downloaders {
        let max_download_bytes = max_download_len(&workload, downloader);
        let max_download_bytes =
            usize::try_from(max_download_bytes).map_err(|_| "largest callback payload does not fit usize")?;
        let source_payload = Bytes::from(vec![0x5a; max_download_bytes]);

        for &shards in &args.shards {
            for &capacity in &args.capacities {
                let mut caches: Vec<Box<dyn ReplayCache>> = vec![Box::new(FeuerReplayCache::new(capacity, shards))];
                for eviction_policy in [FoyerEvictionPolicy::S3Fifo, FoyerEvictionPolicy::CostAware] {
                    for key_range in [FoyerKeyRange::ExactRequest, FoyerKeyRange::ExpandedDownload] {
                        caches.push(Box::new(FoyerReplayCache::new(
                            capacity,
                            shards,
                            key_range,
                            eviction_policy,
                        )));
                        if matches!(downloader, DownloadRangePolicy::Exact) {
                            break;
                        }
                    }
                }
                for cache in caches {
                    let report = replay_cache(
                        cache,
                        &workload,
                        downloader,
                        shards,
                        capacity,
                        args.warmup_iterations,
                        &source_payload,
                    );
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

/// Loads the validated trace, checks request order, and precomputes download ranges.
fn load_trace_workload(args: &ReplayArgs, download_config: DownloadExpansionConfig) -> Result<ReplayWorkload, String> {
    let requests = load_trace(args.operations)?;
    if requests.is_empty() {
        return Err("trace is empty".to_owned());
    }
    if !requests.is_sorted_by_key(|request| request.timestamp_millis) {
        return Err("trace timestamps are not in chronological order".to_owned());
    }
    let expanded_downloads = expanded_download_ranges(&requests, download_config);
    Ok(ReplayWorkload {
        name: "trace",
        requests,
        expanded_downloads,
    })
}

fn replay_cache(
    mut cache: Box<dyn ReplayCache>,
    workload: &ReplayWorkload,
    downloader: DownloadRangePolicy,
    shards: usize,
    capacity: usize,
    warmup_iterations: usize,
    source_payload: &Bytes,
) -> ReplayReport {
    for _ in 0..warmup_iterations {
        replay_pass(&mut *cache, workload, downloader, source_payload);
    }

    let started = Instant::now();
    let traffic = replay_pass(&mut *cache, workload, downloader, source_payload);
    let elapsed = started.elapsed();

    ReplayReport {
        workload: workload.name,
        downloader: downloader.name(),
        shards,
        capacity,
        engine: cache.name(),
        traffic,
        used_payload_bytes: cache.used_payload_bytes(),
        elapsed,
    }
}

fn expanded_download_ranges(workload: &[TraceRequest], config: DownloadExpansionConfig) -> Box<[ByteRange]> {
    let mut request_indices_by_object: HashMap<(&ObjectKeyHash, u64), Vec<usize>> = HashMap::new();
    for (request_index, request) in workload.iter().enumerate() {
        request_indices_by_object
            .entry((&request.object_key, request.object_size))
            .or_default()
            .push(request_index);
    }

    // The first batch that claims a request fixes its expansion. Recomputing a
    // sliding window for each member would produce different native Foyer keys
    // for requests served by the same coalesced download.
    let mut expanded_ranges = vec![None; workload.len()];
    for request_indices in request_indices_by_object.values() {
        for (object_request_index, &request_index) in request_indices.iter().enumerate() {
            if expanded_ranges[request_index].is_some() {
                continue;
            }

            let coalescing_deadline_millis = workload[request_index]
                .timestamp_millis
                .saturating_add(COALESCING_WINDOW_MILLIS);
            let mut pending_downloads = request_indices[object_request_index..]
                .iter()
                .copied()
                .take_while(|&candidate_index| workload[candidate_index].timestamp_millis <= coalescing_deadline_millis)
                .filter(|&candidate_index| expanded_ranges[candidate_index].is_none())
                .map(|index| (workload[index].expanded_download_range(config), index))
                .collect::<Vec<_>>();
            let (downloaded_range, members) =
                coalesced_download(&mut pending_downloads, config.coalescing_distance_bytes, request_index);
            for &(_, member_index) in members {
                expanded_ranges[member_index] = Some(downloaded_range);
            }
        }
    }
    expanded_ranges.into_iter().map(Option::unwrap).collect()
}

fn max_download_len(workload: &ReplayWorkload, downloader: DownloadRangePolicy) -> u64 {
    downloader.ranges(workload).map(|range| range.len()).max().unwrap_or(1)
}

/// Replays one pass of the workload through the cache and records its traffic.
fn replay_pass<C: ReplayCache + ?Sized>(
    cache: &mut C,
    workload: &ReplayWorkload,
    downloader: DownloadRangePolicy,
    source_payload: &Bytes,
) -> ReplayTraffic {
    let mut traffic = ReplayTraffic::default();
    for (request, downloaded_range) in workload.requests.iter().zip(downloader.ranges(workload)) {
        let hit = cache.lookup_hit(request, downloaded_range);
        traffic.requests += 1;
        traffic.requested_bytes += request.requested_range.len();
        if hit {
            traffic.hits += 1;
            traffic.hit_bytes += request.requested_range.len();
            continue;
        }

        let payload = source_payload_slice(source_payload, downloaded_range);
        cache.insert(request, downloaded_range, payload);
        traffic.source_bytes += downloaded_range.len();
    }
    traffic
}

/// Returns the coalesced range and member requests containing this request.
/// Inputs include this request and share an object key and size.
fn coalesced_download(
    pending_downloads: &mut [(ByteRange, usize)],
    coalescing_distance_bytes: u64,
    request_index: usize,
) -> (ByteRange, &[(ByteRange, usize)]) {
    pending_downloads.sort_unstable();
    let mut remaining = &*pending_downloads;
    loop {
        let mut range = remaining[0].0;
        let mut end = 1;
        while end < remaining.len() && remaining[end].0.start().saturating_sub(range.end()) < coalescing_distance_bytes
        {
            range = ByteRange::new(range.start(), range.end().max(remaining[end].0.end())).unwrap();
            end += 1;
        }
        let (members, rest) = remaining.split_at(end);
        if members.iter().any(|&(_, index)| index == request_index) {
            return (range, members);
        }
        remaining = rest;
    }
}

/// Slices a download from the source buffer sized to the largest download, whose length fits usize.
fn source_payload_slice(source_payload: &Bytes, downloaded_range: ByteRange) -> Bytes {
    source_payload.slice(..downloaded_range.len() as usize)
}

fn print_human_header(args: &ReplayArgs, workload: &ReplayWorkload, config: DownloadExpansionConfig) {
    let shards = args.shards.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
    println!("Feuer memory benchmark");
    println!("Trace: {TRACE_FILE} ({} operations)", workload.requests.len());
    println!("Shards: {shards} | Warm-up passes: {}", args.warmup_iterations);
    println!("Feuer cost-aware half-life: {} requests", *ACCESS_COUNT_HALF_LIFE);
    println!("Feuer trimming age: {} requests", *MAX_ACCESS_AGE_ACCESSES);
    println!(
        "Feuer trimming history: {} events per object",
        *MAX_ACCESS_EVENTS_PER_KEY
    );
    println!(
        "Feuer fixed retrieval cost: {} equivalent bytes",
        *FIXED_RETRIEVAL_EQUIVALENT_BYTES
    );
    println!(
        "Expanded: {COALESCING_WINDOW_MILLIS}-ms coalescing within {} | Whole below {}",
        format_decimal_bytes(config.coalescing_distance_bytes),
        format_binary_bytes(config.whole_split_threshold_bytes),
    );
    println!("Source model: 125 ms per GET + transfer at 80 MB/s");
    println!("Foyer fork revision: {PINNED_FOYER_REVISION}");
    println!();
    println!(
        "Capacity   Downloader Engine                   Request hit Source-cost hit Source GETs Source bytes         Used   Throughput"
    );
    println!("{}", "-".repeat(131));
}

fn print_human_report(report: &ReplayReport) {
    println!(
        "{:<10} {:<10} {:<24} {:>11.2}% {:>14.2}% {:>11} {:>12} {:>12} {:>12}",
        format_binary_bytes(report.capacity as u64),
        report.downloader,
        human_engine_name(report.engine),
        report.cache_hit_rate() * 100.0,
        report.source_cost_savings_ratio() * 100.0,
        report.traffic.requests - report.traffic.hits,
        format_binary_bytes(report.traffic.source_bytes),
        format_binary_bytes(report.used_payload_bytes),
        format_operations_per_second(report.operations_per_second()),
    );
}

fn human_engine_name(engine: &str) -> &str {
    match engine {
        "feuer-value-density" => "Feuer",
        "foyer-native-exact-key" => "Foyer S3FIFO",
        "foyer-native-expanded-key" => "Foyer S3FIFO (expanded)",
        "foyer-cost-aware-exact-key" => "Foyer cost-aware",
        "foyer-cost-aware-expanded-key" => "Foyer cost-aware (expanded)",
        other => other,
    }
}

fn format_decimal_bytes(bytes: u64) -> String {
    format_bytes(bytes, [(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "KB")])
}

fn format_binary_bytes(bytes: u64) -> String {
    format_bytes(bytes, [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")])
}

fn format_bytes(bytes: u64, units: [(u64, &str); 3]) -> String {
    for (unit_bytes, suffix) in units {
        if bytes >= unit_bytes {
            if bytes.is_multiple_of(unit_bytes) {
                return format!("{} {suffix}", bytes / unit_bytes);
            }
            return format!("{:.1} {suffix}", bytes as f64 / unit_bytes as f64);
        }
    }
    format!("{bytes} B")
}

/// Formats operations per second with decimal K/s or M/s suffixes when appropriate.
fn format_operations_per_second(operations_per_second: f64) -> String {
    if operations_per_second >= 1_000_000.0 {
        format!("{:.2} M/s", operations_per_second / 1_000_000.0)
    } else if operations_per_second >= 1_000.0 {
        format!("{:.2} K/s", operations_per_second / 1_000.0)
    } else {
        format!("{operations_per_second:.0}/s")
    }
}

fn print_csv_report(report: &ReplayReport) {
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
        report.source_cost_savings_ratio() * 100.0,
        report.traffic.requests - report.traffic.hits,
        report.traffic.source_bytes,
        report.used_payload_bytes,
        report.used_payload_bytes as f64 / report.capacity as f64 * 100.0,
        report.elapsed.as_secs_f64() * 1_000.0,
        report.operations_per_second(),
    );
}

fn load_trace(operation_limit: Option<usize>) -> Result<Box<[TraceRequest]>, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(TRACE_FILE);
    let content = fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(operation_limit.unwrap_or(usize::MAX))
        .map(|(index, line)| {
            parse_trace_line(line).map_err(|error| format!("{}:{}: {error}", path.display(), index + 1))
        })
        .collect()
}

fn parse_trace_line(line: &str) -> Result<TraceRequest, String> {
    let object_key = find_json_string(line, "object_id")?;
    let object_size = find_json_u64(line, "object_num_bytes")?;
    let start = find_json_u64(line, "requested_range_start")?;
    let end = find_json_u64(line, "requested_range_end")?;
    let requested_range = ByteRange::new(start, end).map_err(|error| error.to_string())?;
    if end > object_size {
        return Err(format!("requests {start}..{end} beyond object size {object_size}"));
    }
    let timestamp = find_json_string(line, "timestamp")?;
    let timestamp_millis = parse_timestamp_millis(timestamp)?;
    Ok(TraceRequest {
        object_key: ObjectKeyHash::from(object_key),
        object_size,
        requested_range,
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

fn find_json_string<'a>(line: &'a str, field: &str) -> Result<&'a str, String> {
    let value = line_from_json_value(line, field)?
        .strip_prefix('"')
        .ok_or_else(|| format!("field {field:?} is not a string"))?;
    value
        .split_once('"')
        .map(|(value, _)| value)
        .ok_or_else(|| format!("field {field:?} has no closing quote"))
}

fn find_json_u64(line: &str, field: &str) -> Result<u64, String> {
    let value = line_from_json_value(line, field)?;
    let digits = value
        .split(|character: char| !character.is_ascii_digit())
        .next()
        .unwrap();
    digits.parse().map_err(|error| format!("invalid {field:?}: {error}"))
}

/// Returns the line from the named JSON value's starting position, without parsing the value.
fn line_from_json_value<'a>(line: &'a str, field: &str) -> Result<&'a str, String> {
    let quoted_field_name = format!("\"{field}\"");
    let (_, after_field) = line
        .split_once(&quoted_field_name)
        .ok_or_else(|| format!("missing field {field:?}"))?;
    let (_, after_colon) = after_field
        .split_once(':')
        .ok_or_else(|| format!("missing colon after field {field:?}"))?;
    Ok(after_colon.trim_start())
}

fn parse_byte_count_usize(value: &str) -> Result<usize, String> {
    parse_config_number(value.trim().as_ref(), 1)
        .ok_or_else(|| format!("byte count must be nonzero and fit usize (e.g. 512MiB): {value:?}"))
}

#[cfg(test)]
mod history_bench;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CoveringRangeTestCache {
        entries: Vec<(ObjectKeyHash, ByteRange)>,
    }

    impl ReplayCache for CoveringRangeTestCache {
        fn name(&self) -> &'static str {
            "range-test"
        }

        /// Reports a lookup hit when a test entry covers the requested range.
        fn lookup_hit(&mut self, request: &TraceRequest, _downloaded_range: ByteRange) -> bool {
            self.entries.iter().any(|(key, downloaded_range)| {
                key == &request.object_key && downloaded_range.contains(request.requested_range)
            })
        }

        fn insert(&mut self, request: &TraceRequest, downloaded_range: ByteRange, _payload: Bytes) {
            assert!(downloaded_range.contains(request.requested_range));
            self.entries.push((request.object_key, downloaded_range));
        }

        fn used_payload_bytes(&self) -> u64 {
            self.entries.iter().map(|(_, range)| range.len()).sum()
        }
    }

    #[test]
    fn warmup_preserves_cache_state_but_not_reported_traffic() {
        for (payload_bytes, byte_hit_rate) in [(0, 0.0), (1, 1.0)] {
            let requested_range = ByteRange::new(0, payload_bytes).unwrap();
            let workload = ReplayWorkload {
                name: "test",
                requests: Box::new([TraceRequest {
                    object_key: "object".into(),
                    object_size: 1,
                    requested_range,
                    timestamp_millis: 0,
                }]),
                expanded_downloads: Box::new([requested_range]),
            };
            let report = replay_cache(
                Box::new(FeuerReplayCache::new(1, 1)),
                &workload,
                DownloadRangePolicy::Exact,
                1,
                1,
                1,
                &Bytes::from_static(&[0]),
            );

            assert_eq!(report.traffic.requests, 1);
            assert_eq!(report.traffic.hits, 1);
            assert_eq!(report.cache_hit_rate(), 1.0);
            assert_eq!(report.byte_hit_rate(), byte_hit_rate);
            assert_eq!(report.source_cost_savings_ratio(), 1.0);
            assert_eq!(report.used_payload_bytes, payload_bytes);
        }
    }

    #[test]
    fn foyer_expanded_key_reuses_identical_expansions_for_distinct_requests() {
        let request = |start, end| TraceRequest {
            object_key: "object".into(),
            object_size: 20,
            requested_range: ByteRange::new(start, end).unwrap(),
            timestamp_millis: 0,
        };
        let first = request(2, 4);
        let second = request(12, 14);
        let expanded = ByteRange::new(0, 20).unwrap();

        let mut expanded_key_cache =
            FoyerReplayCache::new(1 << 20, 1, FoyerKeyRange::ExpandedDownload, FoyerEvictionPolicy::S3Fifo);
        assert!(!expanded_key_cache.lookup_hit(&first, expanded));
        expanded_key_cache.insert(&first, expanded, Bytes::from(vec![0; 20]));
        assert!(expanded_key_cache.lookup_hit(&second, expanded));

        let mut exact_key_cache =
            FoyerReplayCache::new(1 << 20, 1, FoyerKeyRange::ExactRequest, FoyerEvictionPolicy::S3Fifo);
        exact_key_cache.insert(&first, expanded, Bytes::from(vec![0; 20]));
        assert!(!exact_key_cache.lookup_hit(&second, expanded));
    }

    #[test]
    fn replay_uses_coalesced_ranges_only_for_expanded_downloads() {
        let request = |start, end, timestamp_millis| TraceRequest {
            object_key: "object".into(),
            object_size: 100,
            requested_range: ByteRange::new(start, end).unwrap(),
            timestamp_millis,
        };
        let requests = vec![
            request(0, 10, 0),
            request(10, 20, 1),
            request(20, 30, 5),
            request(50, 60, 6),
        ];
        let download_config = DownloadExpansionConfig {
            coalescing_distance_bytes: DEFAULT_COALESCING_DISTANCE_BYTES,
            whole_split_threshold_bytes: 0,
        };
        let expanded_downloads = expanded_download_ranges(&requests, download_config);
        let workload = ReplayWorkload {
            name: "test",
            requests: requests.into_boxed_slice(),
            expanded_downloads,
        };
        assert_eq!(&workload.expanded_downloads[..3], &[ByteRange::new(0, 30).unwrap(); 3]);
        assert_eq!(max_download_len(&workload, DownloadRangePolicy::Expanded), 30);
        assert_eq!(max_download_len(&workload, DownloadRangePolicy::Exact), 10);

        for (downloader, hits) in [(DownloadRangePolicy::Expanded, 2), (DownloadRangePolicy::Exact, 0)] {
            let report = replay_cache(
                Box::new(CoveringRangeTestCache::default()),
                &workload,
                downloader,
                1,
                100,
                0,
                &Bytes::from(vec![0; 60]),
            );

            assert_eq!(report.traffic.requests, 4);
            assert_eq!(report.traffic.requested_bytes, 40);
            assert_eq!(report.traffic.hits, hits);
            assert_eq!(report.traffic.hit_bytes, hits * 10);
            assert_eq!(report.traffic.source_bytes, 40);
        }
    }

    #[test]
    fn coalescing_sorts_ranges_without_combining_object_keys_or_sizes() {
        let requests =
            [("a", 100, 20), ("b", 100, 5), ("a", 200, 0), ("a", 100, 0)].map(|(key, object_size, start)| {
                TraceRequest {
                    object_key: key.into(),
                    object_size,
                    requested_range: ByteRange::new(start, start + 10).unwrap(),
                    timestamp_millis: 0,
                }
            });
        let ranges = expanded_download_ranges(
            &requests,
            DownloadExpansionConfig {
                coalescing_distance_bytes: 20,
                whole_split_threshold_bytes: 0,
            },
        );
        assert_eq!(
            *ranges,
            [(0, 30), (5, 15), (0, 10), (0, 30)].map(|(start, end)| ByteRange::new(start, end).unwrap())
        );
    }

    #[test]
    fn coalescing_distance_parameter_is_a_strict_upper_bound() {
        let distance = DEFAULT_COALESCING_DISTANCE_BYTES;
        let ranges_for_gap = |gap| {
            let requests = [(1 + gap, 2 + gap, 0), (0, 1, 1)].map(|(start, end, timestamp_millis)| TraceRequest {
                object_key: "object".into(),
                object_size: 2 * distance,
                requested_range: ByteRange::new(start, end).unwrap(),
                timestamp_millis,
            });
            expanded_download_ranges(
                &requests,
                DownloadExpansionConfig {
                    coalescing_distance_bytes: distance,
                    whole_split_threshold_bytes: 0,
                },
            )
        };

        assert_eq!(
            *ranges_for_gap(distance - 1),
            [ByteRange::new(0, distance + 1).unwrap(); 2]
        );
        assert_eq!(
            *ranges_for_gap(distance),
            [(distance + 1, distance + 2), (0, 1)].map(|(start, end)| ByteRange::new(start, end).unwrap())
        );
    }

    #[test]
    fn source_cost_baseline_uses_requested_bytes_not_downloaded_bytes() {
        let report = ReplayReport {
            workload: "test",
            downloader: "expanded",
            shards: 1,
            capacity: 1,
            engine: "test",
            traffic: ReplayTraffic {
                requests: 1,
                requested_bytes: 100,
                source_bytes: 200,
                ..ReplayTraffic::default()
            },
            used_payload_bytes: 0,
            elapsed: Duration::from_secs(1),
        };

        assert!(report.source_cost_savings_ratio() < 0.0);
    }

    #[test]
    fn parses_the_captured_trace_shape() {
        let line = r#"{"object_num_bytes":100,"object_id":"object-a","requested_range_end":11,"requested_range_start":7,"timestamp":"2026-08-08T01:12:49.481Z"}"#;
        let request = parse_trace_line(line).unwrap();
        assert_eq!(request.object_key, ObjectKeyHash::from("object-a"));
        assert_eq!(request.object_size, 100);
        assert_eq!(request.requested_range, ByteRange::new(7, 11).unwrap());
        assert_eq!(
            request.timestamp_millis,
            parse_timestamp_millis("2026-08-08T01:12:49.481Z").unwrap()
        );
        assert!(parse_trace_line(&line.replace("\"object_num_bytes\":100", "\"object_num_bytes\":11")).is_ok());
        assert!(parse_trace_line(&line.replace("\"object_num_bytes\":100", "\"object_num_bytes\":10")).is_err());
    }

    #[test]
    fn expanded_downloader_honors_the_whole_split_threshold() {
        let config = DownloadExpansionConfig {
            coalescing_distance_bytes: DEFAULT_COALESCING_DISTANCE_BYTES,
            whole_split_threshold_bytes: 8 << 20,
        };
        let requested_range = ByteRange::new((1 << 20) + 3, (2 << 20) + 3).unwrap();
        for (object_size, expected_range) in [
            (
                config.whole_split_threshold_bytes - 1,
                ByteRange::new(0, config.whole_split_threshold_bytes - 1).unwrap(),
            ),
            (config.whole_split_threshold_bytes, requested_range),
        ] {
            let request = TraceRequest {
                object_key: "object".into(),
                object_size,
                requested_range,
                timestamp_millis: 0,
            };
            assert_eq!(request.expanded_download_range(config), expected_range);
        }
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
        assert_eq!(parse_byte_count_usize("1MB").unwrap(), 1_000_000);
        assert_eq!(parse_byte_count_usize("8MiB").unwrap(), 8 << 20);
        assert!(parse_byte_count_usize("0").is_err());
    }

    #[test]
    fn human_output_is_default_and_csv_is_opt_in() {
        assert!(!ReplayArgs::try_parse_from(["benchmark"]).unwrap().csv);
        assert!(ReplayArgs::try_parse_from(["benchmark", "--csv"]).unwrap().csv);
        assert_eq!(format_binary_bytes(8 << 20), "8 MiB");
        assert_eq!(format_decimal_bytes(10_000_000), "10 MB");
        assert_eq!(format_decimal_bytes(1_500), "1.5 KB");
        assert_eq!(format_binary_bytes(1_536), "1.5 KiB");
        assert_eq!(format_binary_bytes(0), "0 B");
    }
}
