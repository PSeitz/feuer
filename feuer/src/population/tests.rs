use super::*;
use bytes::Bytes;
use feuer_storage::IoMetrics;

fn enqueue(population: &DiskPopulation, memory: &MemoryCache, key: &str) {
    let key = key.to_owned();
    let accesses = memory.access_histories().for_key(&key);
    let download = Download::new(3, Bytes::from_static(b"abcd")).unwrap();
    let id = memory
        .insert_and_record(key.clone(), download.clone(), download.downloaded_range())
        .unwrap();
    population.schedule(key, download, id, accesses);
}

#[test]
fn queue_saturation_is_nonblocking_and_bounded_by_entries_and_payload() {
    for (entries, bytes) in [(1, 16), (8, 6)] {
        let memory = MemoryCache::new(1024);
        let (population, mut receiver) = DiskPopulation::channel(entries, bytes);
        enqueue(&population, &memory, "first");
        enqueue(&population, &memory, "skipped");
        assert_eq!(receiver.len(), 1);
        assert_eq!(population.bytes.available_permits(), bytes as usize - 4);
        // Queue pressure did not reject memory admission or its successful access.
        let key = "skipped".to_owned();
        assert_eq!(memory.access_histories().for_key(&key).lock().generation(), 1);
        assert!(memory.get(&key, ByteRange::new(3, 7).unwrap()).is_some());
        let active = receiver.try_recv().unwrap();
        assert_eq!(population.bytes.available_permits(), bytes as usize - 4);
        drop(active);
        assert_eq!(population.bytes.available_permits(), bytes as usize);
        drop(receiver);
        enqueue(&population, &memory, "closed");
        assert_eq!(population.bytes.available_permits(), bytes as usize);
    }
}

#[test]
fn oversized_download_is_not_queued() {
    let memory = MemoryCache::new(1024);
    let (population, receiver) = DiskPopulation::channel(8, 3);
    enqueue(&population, &memory, "oversized");
    assert_eq!(receiver.len(), 0);
    assert_eq!(population.bytes.available_permits(), 3);
}

#[tokio::test]
async fn queued_eviction_and_readmission_cancel_old_writes_while_live_entries_batch_together() {
    let directory = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryCache::new(4096));
    let disk = DiskRangeCache::open_with_access_histories(
        directory.path(),
        1 << 20,
        IoMetrics::noop(),
        memory.access_histories(),
    )
    .await
    .unwrap();
    let (population, receiver) = DiskPopulation::channel(8, 32);
    let range = ByteRange::new(3, 7).unwrap();
    for key in ["live-a", "evicted", "readmitted", "live-b"] {
        enqueue(&population, &memory, key);
    }
    assert!(memory.remove(&"evicted".to_owned(), range));
    assert!(memory.remove(&"readmitted".to_owned(), range));
    memory.insert(
        "readmitted".to_owned(),
        Download::new(3, Bytes::from_static(b"abcd")).unwrap(),
    );
    let budget = population.bytes.clone();
    drop(population);
    DiskPopulation::run(receiver, memory.clone(), disk.clone()).await;
    assert!(!disk.contains(&"evicted".to_owned(), range));
    assert!(!disk.contains(&"readmitted".to_owned(), range));
    // Both fit on disk only if the drained batch packs them into the same chunk.
    for key in ["live-a", "live-b"] {
        assert_eq!(
            disk.get(&key.to_owned(), range).await.unwrap(),
            Bytes::from_static(b"abcd")
        );
        assert_eq!(
            memory.access_histories().for_key(&key.to_owned()).lock().generation(),
            1
        );
    }
    assert_eq!(budget.available_permits(), 32);
}
