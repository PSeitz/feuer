use super::*;
use crate::test_metrics::{registry, value};

#[test]
fn size_boundaries_round_up_without_exposing_spare_capacity() {
    let pool = BufferPool::new(100 * *BUFFER_SIZES.last().unwrap() as u64, MemoryMetrics::noop());
    let mut lower_bound = 0;
    for (index, size) in BUFFER_SIZES.into_iter().enumerate() {
        for length in [lower_bound + 1, size] {
            assert_eq!(BufferPool::allocation_capacity(length), size);
            let buffer = pool.allocate(length).unwrap();
            assert_eq!(buffer.capacity(), size);
            assert_eq!(buffer.as_ref().len(), length);
            assert_eq!(buffer.as_ref().as_ptr() as usize % BUFFER_ALIGNMENT, 0);
            drop(buffer);
            assert_eq!(pool.state.lock().by_size[index].len(), 1);
        }
        lower_bound = size;
    }
    assert_eq!(pool.idle_bytes(), BUFFER_SIZES.iter().sum::<usize>() as u64);
    assert!(pool.idle_bytes() <= pool.capacity * 7 / 100);
}

#[test]
fn download_preserves_capacity_and_returns_only_after_the_last_owner() {
    let pool = BufferPool::new(100 * BUFFER_SIZES[0] as u64, MemoryMetrics::noop());
    let mut buffer = pool.allocate(4).unwrap();
    buffer.as_mut_slice().copy_from_slice(b"abcd");
    let address = buffer.as_ref().as_ptr();
    let download = buffer.into_download(10).unwrap();
    assert_eq!(download.allocation_charge(), BUFFER_SIZES[0]);
    assert_eq!(download.bytes().as_ptr(), address);
    assert_eq!(download.bytes().as_ref(), b"abcd");
    let queued = download.clone();
    let caller = download.bytes_in_range(feuer_types::ByteRange::new(11, 13).unwrap());
    drop(download);
    drop(queued);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(caller.as_ref(), b"bc");
    drop(caller);
    assert_eq!(pool.idle_bytes(), BUFFER_SIZES[0] as u64);
    assert_eq!(pool.allocate(4).unwrap().as_ref().as_ptr(), address);

    // Invalid downloads also release their destination back to the pool.
    assert!(pool.allocate(4).unwrap().into_download(u64::MAX).is_err());
    assert_eq!(pool.idle_bytes(), BUFFER_SIZES[0] as u64);
}

#[test]
fn cache_pressure_frees_larger_idle_buffers_and_updates_bucket_gauges() {
    let (registry, backend) = registry();
    let capacity = 100 * BUFFER_SIZES[2] as u64;
    let pool = BufferPool::new(capacity, MemoryMetrics::new(&backend));
    let idle = |size: usize| {
        let bucket = format!("{:.0}", bytesize::ByteSize(size as u64));
        value(
            &registry,
            "feuer_io_buffer_pool_bytes",
            &[("bucket", &bucket), ("status", "idle")],
        )
    };
    for &size in &BUFFER_SIZES[..3] {
        drop(pool.allocate(size).unwrap());
    }
    for (index, &size) in BUFFER_SIZES.iter().enumerate() {
        assert_eq!(idle(size), if index < 3 { size as f64 } else { 0.0 });
    }
    pool.add_entry_bytes(capacity - BUFFER_SIZES[0] as u64);
    assert_eq!(pool.used_bytes(), capacity);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), capacity as f64);
    assert_eq!(pool.idle_bytes(), BUFFER_SIZES[0] as u64);
    assert_eq!(pool.state.lock().by_size[0].len(), 1);
    for (index, &size) in BUFFER_SIZES.iter().enumerate() {
        assert_eq!(idle(size), if index == 0 { size as f64 } else { 0.0 });
    }
    pool.add_entry_bytes(BUFFER_SIZES[0] as u64);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(pool.used_bytes(), capacity);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), capacity as f64);
    drop(pool.allocate(BUFFER_SIZES[2]).unwrap());
    assert_eq!(pool.idle_bytes(), 0);
    pool.remove_entry_bytes(capacity);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
    drop(pool.allocate(BUFFER_SIZES[2]).unwrap());
    assert_eq!(pool.idle_bytes(), BUFFER_SIZES[2] as u64);
    drop(pool);
    for size in BUFFER_SIZES {
        assert_eq!(idle(size), 0.0);
    }
    for family in registry.gather() {
        if family.name() == "feuer_io_buffer_pool_bytes" {
            assert_eq!(family.get_metric().len(), 2 * BUCKET_COUNT);
        }
    }
}

#[test]
fn small_and_zero_budgets_do_not_retain_buffers() {
    for capacity in [0, 1, (100 * BUFFER_SIZES[0] as u64).div_ceil(7) - 1] {
        let pool = BufferPool::new(capacity, MemoryMetrics::noop());
        drop(pool.allocate(1).unwrap());
        assert_eq!(pool.used_bytes(), 0);
    }
}

#[test]
fn large_buffers_are_resized_on_reuse() {
    let (registry, backend) = registry();
    let length = BUFFER_SIZES.last().unwrap() + BUFFER_ALIGNMENT;
    assert_eq!(BufferPool::allocation_capacity(length), length);
    assert_eq!(BufferPool::allocation_capacity(usize::MAX), usize::MAX);
    let pool = BufferPool::new(100 * 2 * length as u64, MemoryMetrics::new(&backend));
    let large = |status| {
        value(
            &registry,
            "feuer_io_buffer_pool_bytes",
            &[("bucket", ">64 MiB"), ("status", status)],
        )
    };
    let mut buffer = pool.allocate(length).unwrap();
    assert_eq!(buffer.capacity(), length);
    assert_eq!(large("used"), length as f64);
    buffer.as_mut_slice().fill(0x99);
    drop(buffer);
    assert_eq!(pool.idle_bytes(), length as u64);
    assert_eq!(large("idle"), length as f64);

    // Growing keeps the old bytes initialized and zeroes the added bytes.
    let grown = 2 * length;
    let buffer = pool.allocate(grown).unwrap();
    assert_eq!(buffer.capacity(), grown);
    assert_eq!(buffer.as_ref().len(), grown);
    assert_eq!(buffer.as_ref().as_ptr() as usize % BUFFER_ALIGNMENT, 0);
    assert!(buffer.as_ref()[..length].iter().all(|&byte| byte == 0x99));
    assert!(buffer.as_ref()[length..].iter().all(|&byte| byte == 0));
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!((large("idle"), large("used")), (0.0, grown as f64));
    drop(buffer);
    assert_eq!(pool.idle_bytes(), grown as u64);

    // Shrinking exposes only the new capacity.
    let buffer = pool.allocate(length).unwrap();
    assert_eq!(buffer.capacity(), length);
    assert_eq!(large("used"), length as f64);
    drop(buffer);
    assert_eq!(pool.idle_bytes(), length as u64);
    assert_eq!(large("idle"), length as f64);

    // A failed resize frees the idle buffer.
    assert_eq!(
        pool.allocate(usize::MAX).err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!((large("idle"), large("used")), (0.0, 0.0));

    let scratch = AlignedBuffer::allocate_zeroed(BUFFER_ALIGNMENT).unwrap();
    assert_eq!(scratch.capacity(), BUFFER_ALIGNMENT);
    assert!(scratch.pool.is_none());
    assert!(scratch.as_ref().iter().all(|byte| *byte == 0));
}

#[test]
fn large_buffers_shrink_only_when_capacity_is_at_least_ten_percent_larger() {
    let (registry, backend) = registry();
    let mib = 1024 * 1024;
    let capacity = 77 * mib;
    let pool = BufferPool::new(100 * capacity as u64, MemoryMetrics::new(&backend));
    let large = |status| {
        value(
            &registry,
            "feuer_io_buffer_pool_bytes",
            &[("bucket", ">64 MiB"), ("status", status)],
        )
    };
    let buffer = pool.allocate(capacity).unwrap();
    let address = buffer.as_ref().as_ptr();
    drop(buffer);

    // Just below the threshold, preserve capacity but expose only the requested bytes.
    let length = 70 * mib + 1;
    let buffer = pool.allocate(length).unwrap();
    assert_eq!(buffer.as_ref().as_ptr(), address);
    assert_eq!(buffer.as_ref().len(), length);
    assert_eq!(buffer.capacity(), capacity);
    assert_eq!((large("idle"), large("used")), (0.0, capacity as f64));
    let download = buffer.into_download(0).unwrap();
    assert_eq!(download.allocation_charge(), capacity);
    drop(download);
    assert_eq!(pool.idle_bytes(), capacity as u64);
    assert_eq!((large("idle"), large("used")), (capacity as f64, 0.0));

    // At exactly 10% larger, shrink and account for the new capacity.
    let length = 70 * mib;
    let buffer = pool.allocate(length).unwrap();
    assert_eq!(buffer.capacity(), length);
    assert_eq!((large("idle"), large("used")), (0.0, length as f64));
    drop(buffer);
    assert_eq!(pool.idle_bytes(), length as u64);
    assert_eq!((large("idle"), large("used")), (length as f64, 0.0));
}

#[test]
fn large_buffer_growth_zeroes_bytes_after_an_unaligned_shrink() {
    let length = MAX_FIXED_BUFFER_BYTES + BUFFER_ALIGNMENT + 1;
    let pool = BufferPool::new(100 * 2 * length as u64, MemoryMetrics::noop());
    let mut buffer = pool.allocate(2 * length).unwrap();
    buffer.as_mut_slice()[length - 1..length + 2 * BUFFER_ALIGNMENT].fill(0x99);
    drop(buffer);

    let buffer = pool.allocate(length).unwrap();
    assert_eq!(buffer.capacity(), length);
    assert_eq!(buffer.as_ref()[length - 1], 0x99);
    drop(buffer);

    let buffer = pool.allocate(length + 2 * BUFFER_ALIGNMENT).unwrap();
    assert_eq!(buffer.as_ref()[length - 1], 0x99);
    assert!(buffer.as_ref()[length..].iter().all(|&byte| byte == 0));
    assert_eq!(buffer.as_ref().as_ptr() as usize % BUFFER_ALIGNMENT, 0);
}

#[test]
fn metrics_follow_reuse_idle_limit_and_pool_lifetime() {
    for (index, size) in BUFFER_SIZES.into_iter().enumerate() {
        let (registry, backend) = registry();
        let metrics = MemoryMetrics::new(&backend);
        let capacity = (100 * size as u64).div_ceil(7);
        let pool = BufferPool::new(capacity, metrics.clone());
        let gauge = |name| value(&registry, name, &[]);
        let bucket = format!("{:.0}", bytesize::ByteSize(size as u64));
        let buffer_bytes = |status| {
            value(
                &registry,
                "feuer_io_buffer_pool_bytes",
                &[("bucket", &bucket), ("status", status)],
            )
        };
        assert_eq!(buffer_bytes("idle"), 0.0);
        assert_eq!(buffer_bytes("used"), 0.0);
        assert_eq!(gauge("feuer_memory_capacity_bytes"), capacity as f64);
        let mut buffer = pool.allocate(size - 2 * BUFFER_ALIGNMENT).unwrap();
        buffer.as_mut_slice().fill(0x99);
        let bytes = buffer.into_bytes();
        let address = bytes.as_ptr();
        let slice = bytes.slice(1..);
        drop(bytes);
        assert_eq!(buffer_bytes("used"), size as f64);
        assert_eq!(buffer_bytes("idle"), 0.0);
        assert_eq!(gauge("feuer_memory_used_bytes"), 0.0);
        let other = pool.allocate(size).unwrap();
        assert_ne!(other.as_ref().as_ptr(), address);
        assert_eq!(pool.used_bytes(), 0);
        assert_eq!(buffer_bytes("used"), (2 * size) as f64);
        drop(slice);
        assert_eq!(buffer_bytes("used"), size as f64);
        assert_eq!(buffer_bytes("idle"), size as f64);
        drop(other);
        assert_eq!(pool.used_bytes(), size as u64);
        assert_eq!(pool.idle_bytes(), pool.idle_limit);
        assert_eq!(pool.state.lock().by_size[index].len(), 1);
        assert_eq!(buffer_bytes("idle"), size as f64);
        assert_eq!(buffer_bytes("used"), 0.0);
        assert_eq!(gauge("feuer_memory_used_bytes"), size as f64);
        let mut reused = pool.allocate(size).unwrap();
        assert_eq!(reused.as_ref().as_ptr(), address);
        assert_eq!(reused.as_mut_slice().len(), size);
        assert_eq!(buffer_bytes("idle"), 0.0);
        assert_eq!(buffer_bytes("used"), size as f64);
        assert_eq!(gauge("feuer_memory_used_bytes"), 0.0);
        reused.as_mut_slice().fill(0x77);
        drop(reused);
        let outstanding = pool.allocate(size - BUFFER_ALIGNMENT).unwrap().into_bytes();
        assert_eq!(outstanding.as_ptr(), address);
        assert_eq!(outstanding.len(), size - BUFFER_ALIGNMENT);
        let second = BufferPool::new(capacity, metrics);
        drop(second.allocate(size).unwrap());
        assert_eq!(gauge("feuer_memory_capacity_bytes"), (2 * capacity) as f64);
        let weak = Arc::downgrade(&pool);
        drop(pool);
        assert!(weak.upgrade().is_none());
        assert!(outstanding.iter().all(|&byte| byte == 0x77));
        assert_eq!(buffer_bytes("used"), size as f64);
        assert_eq!(buffer_bytes("idle"), size as f64);
        drop(outstanding);
        assert_eq!(buffer_bytes("used"), 0.0);
        drop(second);
        assert_eq!(gauge("feuer_memory_capacity_bytes"), 0.0);
        assert_eq!(gauge("feuer_memory_used_bytes"), 0.0);
        assert_eq!(buffer_bytes("idle"), 0.0);
    }
}

#[test]
fn concurrent_returns_and_cache_charges_share_one_limit() {
    let capacity = 100 * BUFFER_SIZES[0] as u64;
    let pool = BufferPool::new(capacity, MemoryMetrics::noop());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..100 {
                    let buffer = pool.allocate(1).unwrap();
                    std::thread::yield_now();
                    drop(buffer);
                    assert!(pool.used_bytes() <= capacity);
                }
            });
        }
        scope.spawn(|| {
            for _ in 0..100 {
                pool.add_entry_bytes(capacity);
                assert_eq!(pool.used_bytes(), capacity);
                assert_eq!(pool.idle_bytes(), 0);
                pool.remove_entry_bytes(capacity);
            }
        });
    });
    assert!(pool.idle_bytes() <= capacity * 7 / 100);
}

#[test]
fn buckets_compete_for_the_same_idle_limit() {
    let size = BUFFER_SIZES[1];
    let pool = BufferPool::new(100 * size as u64, MemoryMetrics::noop());
    let buffers: Vec<_> = (0..7).map(|_| pool.allocate(size).unwrap()).collect();
    drop(buffers);
    drop(pool.allocate(1).unwrap());
    assert_eq!(pool.idle_bytes(), pool.idle_limit);
    assert!(pool.state.lock().by_size[0].is_empty());

    let buffer = pool.allocate(size).unwrap();
    drop(pool.allocate(1).unwrap());
    assert_eq!(pool.state.lock().by_size[0].len(), 1);
    drop(buffer); // The smaller buffer now occupies part of the shared budget.
    assert_eq!(pool.idle_bytes(), (6 * size + BUFFER_SIZES[0]) as u64);
    assert_eq!(pool.state.lock().by_size[1].len(), 6);
}
