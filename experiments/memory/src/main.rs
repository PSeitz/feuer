//! Controlled memory-tier replay against Feuer and a pinned Foyer fork.

use std::{
    fs,
    path::Path,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use bytes::Bytes;
use clap::Parser;
use feuer_historian::{DEFAULT_ACCESS_COUNT_HALF_LIFE, ObjectAccessHistories};
use feuer_memory::{MemoryCache, retention::AvoidedRetrievalCostScorer};
use feuer_types::{
    Download, ObjectKeyHash,
    config::{parse_config_number, read_env_number},
};
use foyer_memory::{Cache as FoyerCache, CacheBuilder, CostAwareConfig, S3FifoConfig};

const PINNED_FOYER_REVISION: &str = "14c2d88b9d7dd2135bfc723d0967debb59532b4b";
const SOURCE_FIXED_EQUIVALENT_BYTES: u64 = 10_000_000;
static ACCESS_COUNT_HALF_LIFE: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_ACCESS_COUNT_HALF_LIFE", DEFAULT_ACCESS_COUNT_HALF_LIFE, 1)
        .unwrap_or_else(|error| panic!("{error}"))
});
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
const CSV_HEADER: &str = "workload,shards,capacity_bytes,engine,requests,requested_bytes,cache_hits,hit_bytes,cache_hit_pct,byte_hit_pct,source_cost_hit_pct,source_gets,source_bytes,used_payload_bytes,used_vs_target_pct,elapsed_ms,operations_per_second";

#[derive(Debug, Parser)]
#[command(about = "Replay full objects against Feuer and a pinned Foyer fork")]
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

#[derive(Clone)]
struct TraceRequest {
    object_key: ObjectKeyHash,
    payload_len: u64,
    timestamp_millis: u64,
}

struct ReplayWorkload {
    name: &'static str,
    requests: Box<[TraceRequest]>,
}

/// A cache receiving insertions and lookups while replaying a workload.
trait ReplayCache {
    fn name(&self) -> &'static str;

    /// Looks up an object and reports a hit, without returning the cached payload.
    fn lookup_hit(&mut self, request: &TraceRequest) -> bool;

    /// Inserts the object payload.
    fn insert(&mut self, request: &TraceRequest, payload: Bytes);

    fn used_payload_bytes(&self) -> u64;
}

/// Feuer's memory cache used for workload replay.
struct FeuerReplayCache {
    cache: MemoryCache,
}

impl FeuerReplayCache {
    fn new(capacity: usize, num_shards: usize) -> Self {
        Self {
            cache: MemoryCache::with_shards_for_benchmark(capacity as u64, num_shards)
                .with_access_histories(Arc::new(ObjectAccessHistories::with_half_life(*ACCESS_COUNT_HALF_LIFE)))
                .with_retention_scorer(Arc::new(AvoidedRetrievalCostScorer {
                    cost_per_request: *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64,
                    ..Default::default()
                })),
        }
    }
}

impl ReplayCache for FeuerReplayCache {
    fn name(&self) -> &'static str {
        "feuer-value-density"
    }

    fn lookup_hit(&mut self, request: &TraceRequest) -> bool {
        self.cache.access_histories().record_access(&request.object_key);
        self.cache.get(&request.object_key).is_some()
    }

    fn insert(&mut self, request: &TraceRequest, payload: Bytes) {
        self.cache.insert(request.object_key, Download::new(payload));
    }

    fn used_payload_bytes(&self) -> u64 {
        self.cache.used_bytes()
    }
}

#[derive(Clone, Copy)]
enum FoyerEvictionPolicy {
    S3Fifo,
    CostAware,
}

impl FoyerEvictionPolicy {
    const fn engine_name(self) -> &'static str {
        match self {
            Self::S3Fifo => "foyer-native",
            Self::CostAware => "foyer-cost-aware",
        }
    }
}

/// Foyer's native key-value cache used for workload replay.
struct FoyerReplayCache {
    cache: FoyerCache<ObjectKeyHash, Bytes>,
    eviction_policy: FoyerEvictionPolicy,
}

impl FoyerReplayCache {
    fn new(capacity: usize, num_shards: usize, eviction_policy: FoyerEvictionPolicy) -> Self {
        Self {
            cache: build_foyer_cache(capacity, num_shards, eviction_policy),
            eviction_policy,
        }
    }
}

impl ReplayCache for FoyerReplayCache {
    fn name(&self) -> &'static str {
        self.eviction_policy.engine_name()
    }

    fn lookup_hit(&mut self, request: &TraceRequest) -> bool {
        self.cache.get(&request.object_key).is_some()
    }

    fn insert(&mut self, request: &TraceRequest, payload: Bytes) {
        self.cache.insert(request.object_key, payload);
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
) -> FoyerCache<ObjectKeyHash, Bytes> {
    let builder = CacheBuilder::new(capacity)
        .with_shards(num_shards)
        .with_weighter(|_key: &ObjectKeyHash, payload: &Bytes| payload.len());
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

    /// Returns the source-cost savings ratio against fetching every requested object.
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
    let workload = load_trace_workload(&args)?;

    if args.csv {
        eprintln!(
            "foyer_revision={PINNED_FOYER_REVISION} foyer_source=https://github.com/PSeitz/foyer foyer_cost_estimator=residence_access_rate foyer_cost_fixed_retrieval={SOURCE_FIXED_EQUIVALENT_BYTES} foyer_cost_sample_size={FOYER_COST_SAMPLE_SIZE} feuer_cost_estimator=decayed_frequency feuer_cost_half_life_accesses={} feuer_fixed_retrieval_equivalent_bytes={} trace={TRACE_FILE} shards={:?} warmup_iterations={}",
            *ACCESS_COUNT_HALF_LIFE, *FIXED_RETRIEVAL_EQUIVALENT_BYTES, args.shards, args.warmup_iterations,
        );
        println!("{CSV_HEADER}");
    } else {
        print_human_header(&args, &workload);
    }

    let max_payload_len = workload
        .requests
        .iter()
        .map(|request| request.payload_len)
        .max()
        .unwrap();
    let max_payload_len = usize::try_from(max_payload_len).map_err(|_| "largest object payload does not fit usize")?;
    let source_payload = Bytes::from(vec![0x5a; max_payload_len]);

    for &shards in &args.shards {
        for &capacity in &args.capacities {
            let caches: Vec<Box<dyn ReplayCache>> = vec![
                Box::new(FeuerReplayCache::new(capacity, shards)),
                Box::new(FoyerReplayCache::new(capacity, shards, FoyerEvictionPolicy::S3Fifo)),
                Box::new(FoyerReplayCache::new(capacity, shards, FoyerEvictionPolicy::CostAware)),
            ];
            for cache in caches {
                let report = replay_cache(
                    cache,
                    &workload,
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
    Ok(())
}

/// Loads the validated trace and checks request order.
fn load_trace_workload(args: &ReplayArgs) -> Result<ReplayWorkload, String> {
    let requests = load_trace(args.operations)?;
    if requests.is_empty() {
        return Err("trace is empty".to_owned());
    }
    if !requests.is_sorted_by_key(|request| request.timestamp_millis) {
        return Err("trace timestamps are not in chronological order".to_owned());
    }
    Ok(ReplayWorkload {
        name: "trace",
        requests,
    })
}

fn replay_cache(
    mut cache: Box<dyn ReplayCache>,
    workload: &ReplayWorkload,
    shards: usize,
    capacity: usize,
    warmup_iterations: usize,
    source_payload: &Bytes,
) -> ReplayReport {
    for _ in 0..warmup_iterations {
        replay_pass(&mut *cache, workload, source_payload);
    }

    let started = Instant::now();
    let traffic = replay_pass(&mut *cache, workload, source_payload);
    let elapsed = started.elapsed();

    ReplayReport {
        workload: workload.name,
        shards,
        capacity,
        engine: cache.name(),
        traffic,
        used_payload_bytes: cache.used_payload_bytes(),
        elapsed,
    }
}

/// Replays one pass using a source buffer sized to the largest object, whose length fits usize.
fn replay_pass<C: ReplayCache + ?Sized>(
    cache: &mut C,
    workload: &ReplayWorkload,
    source_payload: &Bytes,
) -> ReplayTraffic {
    let mut traffic = ReplayTraffic::default();
    for request in &workload.requests {
        let hit = cache.lookup_hit(request);
        traffic.requests += 1;
        traffic.requested_bytes += request.payload_len;
        if hit {
            traffic.hits += 1;
            traffic.hit_bytes += request.payload_len;
            continue;
        }

        let payload = source_payload.slice(..request.payload_len as usize);
        cache.insert(request, payload);
        traffic.source_bytes += request.payload_len;
    }
    traffic
}

fn print_human_header(args: &ReplayArgs, workload: &ReplayWorkload) {
    let shards = args.shards.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
    println!("Feuer memory benchmark");
    println!("Trace: {TRACE_FILE} ({} operations)", workload.requests.len());
    println!("Shards: {shards} | Warm-up passes: {}", args.warmup_iterations);
    println!("Feuer cost-aware half-life: {} requests", *ACCESS_COUNT_HALF_LIFE);
    println!(
        "Feuer fixed retrieval cost: {} equivalent bytes",
        *FIXED_RETRIEVAL_EQUIVALENT_BYTES
    );
    println!("Source model: 125 ms per GET + transfer at 80 MB/s");
    println!("Foyer fork revision: {PINNED_FOYER_REVISION}");
    println!();
    println!(
        "Capacity   Engine                   Request hit Source-cost hit Source GETs Source bytes         Used   Throughput"
    );
    println!("{}", "-".repeat(120));
}

fn print_human_report(report: &ReplayReport) {
    println!(
        "{:<10} {:<24} {:>11.2}% {:>14.2}% {:>11} {:>12} {:>12} {:>12}",
        format_binary_bytes(report.capacity as u64),
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
        "foyer-native" => "Foyer S3FIFO",
        "foyer-cost-aware" => "Foyer cost-aware",
        other => other,
    }
}

fn format_binary_bytes(bytes: u64) -> String {
    for (unit_bytes, suffix) in [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")] {
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
        "{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{},{},{},{:.4},{:.3},{:.0}",
        report.workload,
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

/// Converts each legacy requested slice into an independent full object.
fn parse_trace_line(line: &str) -> Result<TraceRequest, String> {
    let object_key = find_json_string(line, "object_id")?;
    let object_size = find_json_u64(line, "object_num_bytes")?;
    let start = find_json_u64(line, "requested_range_start")?;
    let end = find_json_u64(line, "requested_range_end")?;
    let payload_len = end
        .checked_sub(start)
        .ok_or_else(|| format!("request ends before it starts: {start}..{end}"))?;
    if end > object_size {
        return Err(format!("requests {start}..{end} beyond object size {object_size}"));
    }
    let timestamp = find_json_string(line, "timestamp")?;
    let timestamp_millis = parse_timestamp_millis(timestamp)?;
    Ok(TraceRequest {
        // Fixed-width offsets keep the original key and both endpoints unambiguous.
        object_key: ObjectKeyHash::from(format!("{object_key}{start:016x}{end:016x}")),
        payload_len,
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

    const TRACE_LINE: &str = r#"{"object_num_bytes":100,"object_id":"object-a","requested_range_end":11,"requested_range_start":7,"timestamp":"2026-08-08T01:12:49.481Z"}"#;

    #[test]
    fn warmup_preserves_cache_state_but_not_reported_traffic() {
        for (payload_len, byte_hit_rate) in [(0, 0.0), (1, 1.0)] {
            let workload = ReplayWorkload {
                name: "test",
                requests: Box::new([TraceRequest {
                    object_key: "object".into(),
                    payload_len,
                    timestamp_millis: 0,
                }]),
            };
            let report = replay_cache(
                Box::new(FeuerReplayCache::new(1, 1)),
                &workload,
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
            assert_eq!(report.used_payload_bytes, payload_len);
        }
    }

    #[test]
    fn all_engines_replay_the_same_independent_objects() {
        let first = parse_trace_line(TRACE_LINE).unwrap();
        let second =
            parse_trace_line(&TRACE_LINE.replace("\"requested_range_start\":7", "\"requested_range_start\":8"))
                .unwrap();
        let workload = ReplayWorkload {
            name: "test",
            requests: vec![first.clone(), second, first].into_boxed_slice(),
        };
        let caches: Vec<Box<dyn ReplayCache>> = vec![
            Box::new(FeuerReplayCache::new(1024, 1)),
            Box::new(FoyerReplayCache::new(1024, 1, FoyerEvictionPolicy::S3Fifo)),
            Box::new(FoyerReplayCache::new(1024, 1, FoyerEvictionPolicy::CostAware)),
        ];
        for cache in caches {
            let report = replay_cache(cache, &workload, 1, 1024, 0, &Bytes::from_static(&[0; 4]));
            assert_eq!(report.traffic.requests, 3, "{}", report.engine);
            assert_eq!(report.traffic.requested_bytes, 11);
            assert_eq!(report.traffic.hits, 1);
            assert_eq!(report.traffic.hit_bytes, 4);
            assert_eq!(report.traffic.source_bytes, 7);
            assert_eq!(report.used_payload_bytes, 7);
            let expected = (SOURCE_FIXED_EQUIVALENT_BYTES + 4) as f64 / (3 * SOURCE_FIXED_EQUIVALENT_BYTES + 11) as f64;
            assert!((report.source_cost_savings_ratio() - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn parses_the_captured_trace_shape() {
        let request = parse_trace_line(TRACE_LINE).unwrap();
        assert_eq!(request.payload_len, 4);
        assert_eq!(
            request.timestamp_millis,
            parse_timestamp_millis("2026-08-08T01:12:49.481Z").unwrap()
        );
        assert!(parse_trace_line(&TRACE_LINE.replace("\"object_num_bytes\":100", "\"object_num_bytes\":11")).is_ok());
        assert!(parse_trace_line(&TRACE_LINE.replace("\"object_num_bytes\":100", "\"object_num_bytes\":10")).is_err());
        assert!(
            parse_trace_line(&TRACE_LINE.replace("\"requested_range_start\":7", "\"requested_range_start\":12"))
                .is_err()
        );
        assert_eq!(
            parse_trace_line(&TRACE_LINE.replace("\"requested_range_start\":7", "\"requested_range_start\":11"))
                .unwrap()
                .payload_len,
            0
        );
    }

    #[test]
    fn object_identity_uses_original_key_and_both_endpoints() {
        let key = parse_trace_line(TRACE_LINE).unwrap().object_key;
        assert_eq!(key, parse_trace_line(TRACE_LINE).unwrap().object_key);
        for changed in [
            TRACE_LINE.replace("object-a", "object-b"),
            TRACE_LINE.replace("\"requested_range_start\":7", "\"requested_range_start\":8"),
            TRACE_LINE.replace("\"requested_range_end\":11", "\"requested_range_end\":12"),
        ] {
            assert_ne!(key, parse_trace_line(&changed).unwrap().object_key);
        }
        let changed_size = TRACE_LINE.replace("\"object_num_bytes\":100", "\"object_num_bytes\":1000");
        assert_eq!(key, parse_trace_line(&changed_size).unwrap().object_key);
        assert_eq!(parse_trace_line(&changed_size).unwrap().payload_len, 4);
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
        assert_eq!(format_binary_bytes(1_536), "1.5 KiB");
        assert_eq!(format_binary_bytes(0), "0 B");
    }
}
