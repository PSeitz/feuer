//! Ignored test injected into disk_cache::recovery in isolated benchmark checkouts.
//! Uses production writes, recovery, direct I/O, allocation, and indexing without public API changes.

use super::*;
use std::{fs, fs::File};
use crate::test_metrics::{registry, value};

fn parameter(name: &str) -> usize {
    std::env::var(name).unwrap().parse().unwrap()
}

fn cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the supplied writable structure on success.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) }, 0);
    let usage = unsafe { usage.assume_init() };
    [usage.ru_utime, usage.ru_stime]
        .into_iter()
        .map(|time| time.tv_sec as f64 + time.tv_usec as f64 / 1_000_000.0)
        .sum()
}

fn physical_read_bytes() -> u64 {
    fs::read_to_string("/proc/self/io")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("read_bytes: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// Return the cache only after recovery finishes; retain the underlying open time for comparison.
async fn open_and_wait_for_recovery(directory: &Path, capacity: u64, metrics: Arc<IoMetrics>) -> (DiskCache, f64) {
    let started = Instant::now();
    let cache = DiskCache::open(directory, capacity, metrics).await.unwrap();
    let open_seconds = started.elapsed().as_secs_f64();
    (cache, open_seconds)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual SSD recovery benchmark; see benchmarks/storage/recovery.md"]
async fn benchmark_recovery() {
    let root = std::env::var("RECOVERY_ROOT").unwrap();
    let format = std::env::var("RECOVERY_FORMAT").unwrap();
    let entry_bytes = parameter("RECOVERY_ENTRY_BYTES");
    let entries = parameter("RECOVERY_ENTRIES");
    let batch_entries = parameter("RECOVERY_BATCH_ENTRIES");
    let rounds = parameter("RECOVERY_ROUNDS");
    let capacity = std::env::var("RECOVERY_CAPACITY_GIB")
        .map(|value| value.parse::<u64>().unwrap())
        .unwrap_or(32)
        * 1024
        * CHUNK_BYTES;
    let directory = tempfile::Builder::new().prefix("data-").tempdir_in(root).unwrap();
    let cache = DiskCache::open(directory.path(), capacity, IoMetrics::noop())
        .await
        .unwrap();
    let source = Download::new(0, Bytes::from(vec![0x5a; entry_bytes])).unwrap();
    for start in (0..entries).step_by(batch_entries) {
        let batch: Vec<_> = (start..(start + batch_entries).min(entries))
            .map(|i| (format!("{i:064x}").into(), source.clone()))
            .collect();
        let expected = batch.len();
        assert_eq!(cache.insert_batch(batch).await.unwrap(), expected);
    }
    let scan_chunks: u64 = cache
        .disk
        .shards
        .iter()
        .map(|shard| shard.allocator.chunk_capacity - shard.allocator.available_bytes() / CHUNK_BYTES)
        .sum();
    for i in 0..entries {
        assert!(cache.contains(&format!("{i:064x}").into(), source.downloaded_range()));
    }
    File::options()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .sync_all()
        .unwrap();
    drop(cache);

    for round in 1..=rounds {
        let (registry, backend) = registry();
        let io_metrics = IoMetrics::new(&backend);
        let read_bytes_before = physical_read_bytes();
        let cpu_before = cpu_seconds();
        let started = Instant::now();
        let (cache, open_seconds) = open_and_wait_for_recovery(directory.path(), capacity, io_metrics).await;
        let seconds = started.elapsed().as_secs_f64();
        let cpu_seconds = cpu_seconds() - cpu_before;
        let physical_read_bytes = physical_read_bytes() - read_bytes_before;
        let reads = value(
            &registry,
            "feuer_disk_io_total",
            &[("operation", "read"), ("outcome", "success")],
        );
        let read_bytes = value(&registry, "feuer_disk_io_bytes_total", &[("operation", "read")]);
        let recovered: usize = cache
            .disk
            .shards
            .iter()
            .map(|shard| {
                shard
                    .entry_index
                    .lock()
                    .unwrap()
                    .entries_by_key
                    .values()
                    .map(BTreeMap::len)
                    .sum::<usize>()
            })
            .sum();
        assert_eq!(recovered, entries);
        assert_eq!(
            value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "read"), ("outcome", "error")]
            ),
            0.0
        );
        // Verification is outside the timing and I/O measurements.
        for i in 0..entries {
            assert!(cache.contains(&format!("{i:064x}").into(), source.downloaded_range()));
        }
        for i in [0, entries / 2, entries - 1] {
            assert_eq!(
                cache
                    .get(&format!("{i:064x}").into(), source.downloaded_range())
                    .await
                    .unwrap(),
                source.bytes()
            );
        }
        println!(
            "RECOVERY,{format},{entry_bytes},{entries},{batch_entries},{round},{scan_chunks},{open_seconds:.6},{seconds:.6},{cpu_seconds:.6},{reads:.0},{read_bytes:.0},{physical_read_bytes},{recovered}"
        );
        drop(cache);
    }
}
