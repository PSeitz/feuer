//! Repeated-open recovery benchmark; see benchmarks/storage/recovery.md.

use std::{any::Any, cell::RefCell, os::unix::fs::MetadataExt, rc::Rc};

use binggan::{
    BenchRunner,
    plugins::{EventListener, PluginEvents},
};

use super::*;
use crate::disk_cache::tests::with_manual_metadata_writes;

/// Checks recovered entries and releases the reopened cache after timing stops.
struct CheckRecovery {
    cache: Rc<RefCell<Option<DiskCache>>>,
    entry_sizes: Rc<[usize]>,
}

impl EventListener for CheckRecovery {
    fn name(&self) -> &'static str {
        "check_recovery"
    }

    fn on_event(&mut self, event: PluginEvents<'_>) {
        match event {
            PluginEvents::GroupBenchNumIters { num_iter } => {
                assert_eq!(
                    num_iter, 1,
                    "recovery needs NUM_ITER_BENCH=1; use NUM_ITER_GROUP for repetitions"
                );
            }
            // Binggan also invokes the benchmark once, untimed, to obtain its output for reporting.
            PluginEvents::BenchStop { .. } | PluginEvents::GroupStop { .. } => {
                let Some(cache) = self.cache.borrow_mut().take() else {
                    return;
                };
                // Verification can take longer than the metadata timer on large fixtures.
                let cache = with_manual_metadata_writes(cache);
                let entries: usize = cache
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
                assert_eq!(entries, self.entry_sizes.len());
                for (i, &size) in self.entry_sizes.iter().enumerate() {
                    assert!(cache.covers_range(&ObjectKeyHash(i as u128), ByteRange::new(0, size as u64).unwrap()));
                }
                drop(cache);
            }
            _ => {}
        }
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Samples recorded source-download frequencies, stopping before exceeding 100 GB.
fn trace_download_sizes() -> Vec<usize> {
    let mut sizes: Vec<usize> = include_str!("../../../../benchmarks/storage/recovery-sizes.csv")
        .lines()
        .skip(1)
        .flat_map(|line| {
            let (size, count) = line.split_once(',').unwrap();
            std::iter::repeat_n(size.parse().unwrap(), count.parse().unwrap())
        })
        .collect();
    // Fixed-seed Fisher-Yates shuffle: sample frequencies without retaining captured event order.
    let mut random = 42u64;
    for i in (1..sizes.len()).rev() {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        sizes.swap(i, random as usize % (i + 1));
    }
    let mut bytes = 0;
    sizes
        .into_iter()
        .take_while(|&size| {
            bytes += size;
            bytes <= 100_000_000_000
        })
        .collect()
}

#[test]
fn trace_size_sample_is_reproducible() {
    let sizes = trace_download_sizes();
    assert_eq!(sizes.len(), 80_778);
    assert_eq!(sizes.iter().sum::<usize>(), 99_971_787_650);
    assert_eq!(sizes, trace_download_sizes());
}

#[test]
#[ignore = "manual SSD recovery benchmark; see benchmarks/storage/recovery.md"]
fn benchmark_recovery() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let mut runner = BenchRunner::with_name("disk recovery");
    runner.config().set_num_iter_for_bench(1).set_num_iter_for_group(10);

    // Only the opt-in 100-GB case uses recorded source-download size frequencies.
    let cases = if std::env::var_os("RECOVERY_100_GB").is_some() {
        vec![(trace_download_sizes(), 256 * 1024, "source download sizes")]
    } else {
        // 50k entries span three metadata chunks on one shard; 100k grow all four chains.
        [(0, 128), (10_000, 128), (50_000, 240), (50_000, 512), (100_000, 512)]
            .map(|(entries, chunks)| (vec![4096; entries], chunks, "4-KiB entries"))
            .into()
    };
    for (entry_sizes, capacity_chunks, label) in cases {
        let entry_sizes: Rc<[usize]> = entry_sizes.into();
        let entries = entry_sizes.len();
        let payload_bytes: usize = entry_sizes.iter().sum();
        let source = Bytes::from(vec![0x5a; entry_sizes.iter().copied().max().unwrap_or(0)]);
        let directory = tempfile::tempdir().unwrap();
        let capacity = capacity_chunks * CHUNK_BYTES;
        let (shards, metadata_chunks) = runtime.block_on(async {
            let cache = with_manual_metadata_writes(
                DiskCache::open(directory.path(), capacity, IoMetrics::noop())
                    .await
                    .unwrap(),
            );
            for start in (0..entries).step_by(4096) {
                let end = (start + 4096).min(entries);
                let batch: Vec<_> = (start..end)
                    .map(|i| {
                        (
                            ObjectKeyHash(i as u128),
                            Download::new(0, source.slice(..entry_sizes[i])).unwrap(),
                        )
                    })
                    .collect();
                assert_eq!(cache.insert_batch(batch).await.unwrap(), end - start);
            }
            cache.write_dirty_metadata_pages().await;
            let metadata_chunks: usize = cache
                .disk
                .shards
                .iter()
                .map(|shard| shard.metadata_pages.lock().unwrap().chunks.len())
                .sum();
            (cache.disk.shards.len(), metadata_chunks)
        });
        let disk_bytes = std::fs::metadata(directory.path().join("data")).unwrap().blocks() * 512;
        println!(
            "Fixture: {payload_bytes} payload bytes, {metadata_chunks} metadata chunks, {disk_bytes} disk bytes, {capacity} capacity bytes",
        );

        let reopened = Rc::new(RefCell::new(None));
        runner.get_plugin_manager().replace_plugin(CheckRecovery {
            cache: reopened.clone(),
            entry_sizes,
        });
        let mut group = runner
            .new_group()
            .name(format!("{label} / {entries} entries / {shards} shards"));
        group.register("open", |_| {
            let cache = runtime
                .block_on(DiskCache::open(directory.path(), capacity, IoMetrics::noop()))
                .unwrap();
            *reopened.borrow_mut() = Some(cache);
            entries
        });
        group.run();
    }
}
