//! Controlled memory-tier replay against Feuer and a pinned Foyer revision.

use std::{
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
const SOURCE_FIXED_EQUIVALENT_BYTES: u128 = 6_400_000;
const TRACE_FILE: &str = "access_pattern.ndjson";
const LARGE_REQUEST_THRESHOLD_BYTES: u64 = 4 << 20;
const SMALL_REQUEST_EXPANSION_BYTES: u64 = 4 << 20;
const WHOLE_SPLIT_THRESHOLD_BYTES: u64 = 20 << 20;

#[derive(Debug, Parser)]
#[command(about = "Replay the captured trace against Feuer and pinned Foyer")]
struct Args {
    /// Soft payload capacities. Repeat the flag or separate values with commas.
    #[arg(
        long = "capacity",
        default_value = "256MiB,512MiB,1GiB,2GiB,4GiB,8GiB,128GiB",
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

    /// Feuer's value-aware policy and optional disabled control.
    #[arg(
        long = "feuer-compaction",
        value_enum,
        default_value = "value-aware",
        value_delimiter = ','
    )]
    feuer_compactions: Vec<FeuerCompaction>,

    /// Restrict the trace to its first N operations.
    #[arg(long)]
    operations: Option<usize>,
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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FeuerCompaction {
    /// Product policy: choose the lower estimated value loss per reclaimed byte.
    ValueAware,
    /// Disable compaction while preserving the rest of Feuer's policy.
    Disabled,
}

impl FeuerCompaction {
    const fn name(self) -> &'static str {
        match self {
            Self::ValueAware => "feuer-value-aware",
            Self::Disabled => "feuer-compaction-disabled",
        }
    }
}

#[derive(Clone)]
struct Access {
    object_key: ObjectKey,
    object_size: u64,
    requested: ByteRange,
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

    fn downloaded_range(&self, policy: DownloadPolicy) -> ByteRange {
        if matches!(policy, DownloadPolicy::Exact) {
            return self.requested;
        }
        if self.object_size < WHOLE_SPLIT_THRESHOLD_BYTES {
            return ByteRange::new(0, self.object_size)
                .expect("an object containing a valid non-empty request must be non-empty");
        }
        if self.requested.len() > LARGE_REQUEST_THRESHOLD_BYTES {
            return self.requested;
        }

        let start = self.requested.start().saturating_sub(SMALL_REQUEST_EXPANSION_BYTES);
        let end = self
            .requested
            .end()
            .saturating_add(SMALL_REQUEST_EXPANSION_BYTES)
            .min(self.object_size);
        let downloaded =
            ByteRange::new(start, end).expect("expanded bounds around a valid non-empty request must be non-empty");
        debug_assert!(downloaded.contains(self.requested));
        downloaded
    }
}

struct Workload {
    name: &'static str,
    accesses: Vec<Access>,
}

trait Engine {
    fn name(&self) -> &'static str;

    /// Returns true on a cache hit and populates the supplied callback extent on a miss.
    fn get_or_fetch(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<bool, String>;

    fn used_payload_bytes(&self) -> u64;
}

struct FeuerEngine {
    cache: MemoryCache,
    name: &'static str,
}

impl FeuerEngine {
    fn new(capacity: usize, shards: usize, compaction: FeuerCompaction) -> Self {
        let cache = match compaction {
            FeuerCompaction::ValueAware => MemoryCache::with_shards_for_benchmark(capacity as u64, shards),
            FeuerCompaction::Disabled => MemoryCache::with_compaction_disabled_for_benchmark(capacity as u64, shards),
        };
        Self {
            cache,
            name: compaction.name(),
        }
    }
}

impl Engine for FeuerEngine {
    fn name(&self) -> &'static str {
        self.name
    }

    fn get_or_fetch(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<bool, String> {
        if let Some(bytes) = self.cache.get(&access.object_key, access.requested) {
            debug_assert_eq!(bytes.len() as u64, access.requested.len());
            return Ok(true);
        }

        let download = Download::new(downloaded.start(), payload).map_err(|error| error.to_string())?;
        debug_assert_eq!(download.downloaded_range(), downloaded);
        self.cache
            .insert_and_record(access.object_key.clone(), download, access.requested);
        Ok(false)
    }

    fn used_payload_bytes(&self) -> u64 {
        self.cache.used_bytes()
    }
}

type NativeFoyerKey = (ObjectKey, ByteRange);

struct NativeFoyerEngine {
    cache: FoyerCache<NativeFoyerKey, Bytes>,
}

impl NativeFoyerEngine {
    fn new(capacity: usize, shards: usize) -> Self {
        Self {
            cache: foyer_cache(capacity, shards),
        }
    }
}

impl Engine for NativeFoyerEngine {
    fn name(&self) -> &'static str {
        "foyer-native-exact-key"
    }

    fn get_or_fetch(&mut self, access: &Access, downloaded: ByteRange, payload: Bytes) -> Result<bool, String> {
        let key = (access.object_key.clone(), access.requested);
        if let Some(entry) = self.cache.get(&key) {
            let result = requested_payload(entry.value(), downloaded, access.requested);
            debug_assert_eq!(result.len() as u64, access.requested.len());
            return Ok(true);
        }
        self.cache.insert(key, payload);
        Ok(false)
    }

    fn used_payload_bytes(&self) -> u64 {
        self.cache.usage() as u64
    }
}

fn foyer_cache(capacity: usize, shards: usize) -> FoyerCache<NativeFoyerKey, Bytes> {
    CacheBuilder::new(capacity)
        .with_shards(shards)
        .with_eviction_config(S3FifoConfig::default())
        .with_weighter(|_key: &NativeFoyerKey, value: &Bytes| value.len())
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
    uncached_source_bytes: u64,
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
        let baseline = u128::from(self.traffic.requests) * SOURCE_FIXED_EQUIVALENT_BYTES
            + u128::from(self.traffic.uncached_source_bytes);
        let actual = u128::from(self.traffic.source_requests) * SOURCE_FIXED_EQUIVALENT_BYTES
            + u128::from(self.traffic.source_bytes);
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
    if args.feuer_compactions.is_empty() {
        return Err("at least one --feuer-compaction value is required".to_owned());
    }
    let workload = trace_workload(&args)?;

    eprintln!(
        "foyer_revision={PINNED_FOYER_REVISION} trace={TRACE_FILE} shards={:?} downloaders={:?} feuer_compactions={:?} large_request_threshold_bytes={LARGE_REQUEST_THRESHOLD_BYTES} small_request_expansion_bytes={SMALL_REQUEST_EXPANSION_BYTES} whole_split_threshold_bytes={WHOLE_SPLIT_THRESHOLD_BYTES}",
        args.shards, args.downloaders, args.feuer_compactions
    );
    println!(
        "workload,downloader,shards,capacity_bytes,engine,requests,requested_bytes,cache_hits,hit_bytes,cache_hit_pct,byte_hit_pct,source_cost_hit_pct,source_gets,source_bytes,used_payload_bytes,used_vs_target_pct,elapsed_ms,operations_per_second"
    );

    for &downloader in &args.downloaders {
        let max_download = workload
            .accesses
            .iter()
            .map(|access| access.downloaded_range(downloader).len())
            .max()
            .unwrap_or(1);
        let max_download = usize::try_from(max_download).map_err(|_| "largest callback payload does not fit usize")?;
        let source_payload = Bytes::from(vec![0x5a; max_download]);

        for &shards in &args.shards {
            for &capacity in &args.capacities {
                let mut engines: Vec<Box<dyn Engine>> = args
                    .feuer_compactions
                    .iter()
                    .copied()
                    .map(|compaction| Box::new(FeuerEngine::new(capacity, shards, compaction)) as Box<dyn Engine>)
                    .collect();
                engines.push(Box::new(NativeFoyerEngine::new(capacity, shards)));
                for engine in engines {
                    let report = run_engine(engine, &workload, downloader, shards, capacity, &source_payload)?;
                    print_report(&report);
                }
            }
        }
    }
    Ok(())
}

fn trace_workload(args: &Args) -> Result<Workload, String> {
    let mut accesses = load_trace()?;
    if let Some(operations) = args.operations {
        accesses.truncate(operations);
    }
    if accesses.is_empty() {
        return Err("trace is empty".to_owned());
    }
    for (index, access) in accesses.iter().enumerate() {
        access.validate(index)?;
    }
    Ok(Workload {
        name: "trace",
        accesses,
    })
}

fn run_engine(
    mut engine: Box<dyn Engine>,
    workload: &Workload,
    downloader: DownloadPolicy,
    shards: usize,
    capacity: usize,
    source_payload: &Bytes,
) -> Result<Report, String> {
    let mut traffic = Traffic::default();
    let started = Instant::now();
    execute_pass(
        &mut *engine,
        &workload.accesses,
        downloader,
        source_payload,
        &mut traffic,
    )?;
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

fn execute_pass<E: Engine + ?Sized>(
    engine: &mut E,
    workload: &[Access],
    downloader: DownloadPolicy,
    source_payload: &Bytes,
    traffic: &mut Traffic,
) -> Result<(), String> {
    for access in workload {
        let downloaded = access.downloaded_range(downloader);
        let payload_len =
            usize::try_from(downloaded.len()).map_err(|_| "callback payload length does not fit usize")?;
        let payload = source_payload.slice(..payload_len);
        let hit = engine.get_or_fetch(access, downloaded, payload)?;

        traffic.requests += 1;
        traffic.requested_bytes += access.requested.len();
        traffic.uncached_source_bytes += downloaded.len();
        if hit {
            traffic.hits += 1;
            traffic.hit_bytes += access.requested.len();
        } else {
            traffic.source_requests += 1;
            traffic.source_bytes += downloaded.len();
        }
    }
    Ok(())
}

fn requested_payload(bytes: &Bytes, downloaded: ByteRange, requested: ByteRange) -> Bytes {
    debug_assert!(downloaded.contains(requested));
    let start = usize::try_from(requested.start() - downloaded.start())
        .expect("an offset within a callback payload must fit usize");
    let end = usize::try_from(requested.end() - downloaded.start())
        .expect("an offset within a callback payload must fit usize");
    bytes.slice(start..end)
}

fn print_report(report: &Report) {
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
    Ok(Access {
        object_key: ObjectKey::from(object_key),
        object_size,
        requested,
    })
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

    #[test]
    fn parses_the_captured_trace_shape() {
        let access = parse_trace_line(
            r#"{"object_num_bytes":100,"object_id":"object-a","requested_range_end":11,"requested_range_start":7}"#,
        )
        .unwrap();
        assert_eq!(access.object_key, ObjectKey::from("object-a"));
        assert_eq!(access.object_size, 100);
        assert_eq!(access.requested, ByteRange::new(7, 11).unwrap());
    }

    #[test]
    fn expanded_downloader_uses_whole_small_splits_exact_large_requests_and_four_mib_padding() {
        let small_split = Access {
            object_key: "small-split".into(),
            object_size: WHOLE_SPLIT_THRESHOLD_BYTES - 1,
            requested: ByteRange::new(3 << 20, (8 << 20) + 1).unwrap(),
        };
        assert_eq!(
            small_split.downloaded_range(DownloadPolicy::Expanded),
            ByteRange::new(0, WHOLE_SPLIT_THRESHOLD_BYTES - 1).unwrap()
        );
        assert_eq!(
            small_split.downloaded_range(DownloadPolicy::Exact),
            small_split.requested
        );

        let large_request = Access {
            object_key: "large-request".into(),
            object_size: 64 << 20,
            requested: ByteRange::new((5 << 20) + 3, (9 << 20) + 4).unwrap(),
        };
        assert_eq!(
            large_request.downloaded_range(DownloadPolicy::Expanded),
            large_request.requested
        );

        let four_mib_request = Access {
            object_key: "four-mib-request".into(),
            object_size: 64 << 20,
            requested: ByteRange::new((10 << 20) + 3, (14 << 20) + 3).unwrap(),
        };
        assert_eq!(
            four_mib_request.downloaded_range(DownloadPolicy::Expanded),
            ByteRange::new((6 << 20) + 3, (18 << 20) + 3).unwrap()
        );

        let tail = Access {
            object_key: "tail".into(),
            object_size: (22 << 20) + 3,
            requested: ByteRange::new((21 << 20) + 1, (22 << 20) + 3).unwrap(),
        };
        assert_eq!(
            tail.downloaded_range(DownloadPolicy::Expanded),
            ByteRange::new((17 << 20) + 1, (22 << 20) + 3).unwrap()
        );
    }
}
