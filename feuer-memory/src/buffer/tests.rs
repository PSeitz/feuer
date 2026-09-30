use super::*;
use crate::test_metrics::{registry, value};

#[test]
fn size_boundaries_round_up_without_exposing_spare_capacity() {
    let pool = BufferPool::new(100 * BUFFER_SIZES[5] as u64, MemoryMetrics::noop());
    let mut lower_bound = 0;
    for (index, size) in BUFFER_SIZES.into_iter().enumerate() {
        for length in [lower_bound + 1, size] {
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
fn different_lengths_reuse_one_allocation() {
    let pool = BufferPool::new(100 * BUFFER_SIZES[0] as u64, MemoryMetrics::noop());
    let mut buffer = pool.allocate(BUFFER_ALIGNMENT).unwrap();
    let address = buffer.as_ref().as_ptr();
    buffer.as_mut_slice().fill(0x99);
    drop(buffer);
    let mut larger = pool.allocate(4 * BUFFER_ALIGNMENT).unwrap();
    assert_eq!(larger.as_ref().as_ptr(), address);
    assert_eq!(larger.as_mut_slice().len(), 4 * BUFFER_ALIGNMENT);
    larger.as_mut_slice().fill(0x77);
    drop(larger);
    let smaller = pool.allocate(2 * BUFFER_ALIGNMENT).unwrap();
    assert_eq!(smaller.as_ref().as_ptr(), address);
    let bytes = smaller.into_bytes();
    assert_eq!(&bytes[..], &vec![0x77; 2 * BUFFER_ALIGNMENT]);
    assert_eq!(pool.used_bytes(), 0);
    drop(bytes);
    assert_eq!(pool.used_bytes(), BUFFER_SIZES[0] as u64);
}

#[test]
fn last_bytes_owner_returns_allocation_without_keeping_pool_alive() {
    let pool = BufferPool::new(120 * BUFFER_SIZES[0] as u64, MemoryMetrics::noop());
    let mut buffer = pool.allocate(BUFFER_ALIGNMENT).unwrap();
    buffer.as_mut_slice().fill(0x99);
    let address = buffer.as_ref().as_ptr();
    let bytes = buffer.into_bytes();
    let slice = bytes.slice(1..);
    drop(bytes);
    assert_eq!(pool.idle_bytes(), 0);
    let other = pool.allocate(BUFFER_ALIGNMENT).unwrap();
    assert_ne!(other.as_ref().as_ptr(), address);
    drop(slice);
    let reused = pool.allocate(BUFFER_ALIGNMENT).unwrap();
    assert_eq!(reused.as_ref().as_ptr(), address);
    let weak = Arc::downgrade(&pool);
    let bytes = reused.into_bytes();
    drop(other);
    drop(pool);
    assert!(weak.upgrade().is_none());
    assert_eq!(&bytes[..], &[0x99; BUFFER_ALIGNMENT]);
}

#[test]
fn one_bucket_can_fill_the_idle_limit() {
    for (index, size) in BUFFER_SIZES.into_iter().enumerate() {
        let pool = BufferPool::new((100 * size as u64).div_ceil(7), MemoryMetrics::noop());
        let buffers: Vec<_> = (0..2).map(|_| pool.allocate(size).unwrap()).collect();
        assert_eq!(pool.used_bytes(), 0);
        drop(buffers);
        assert_eq!(pool.used_bytes(), size as u64);
        assert_eq!(pool.idle_bytes(), pool.idle_limit);
        assert_eq!(pool.state.lock().by_size[index].len(), 1);
    }
}

#[test]
fn idle_limit_does_not_reserve_memory_and_cache_pressure_frees_idle_buffers_first() {
    let capacity = 100 * BUFFER_SIZES[2] as u64;
    let pool = BufferPool::new(capacity, MemoryMetrics::noop());
    for size in &BUFFER_SIZES[..3] {
        drop(pool.allocate(*size).unwrap());
    }
    pool.add_cached(capacity - BUFFER_SIZES[0] as u64);
    assert_eq!(pool.used_bytes(), capacity);
    assert_eq!(pool.idle_bytes(), BUFFER_SIZES[0] as u64);
    assert_eq!(pool.state.lock().by_size[0].len(), 1);
    pool.add_cached(BUFFER_SIZES[0] as u64);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(pool.used_bytes(), capacity);
    drop(pool.allocate(BUFFER_SIZES[2]).unwrap());
    assert_eq!(pool.idle_bytes(), 0);
    pool.remove_cached(capacity);
    drop(pool.allocate(BUFFER_SIZES[2]).unwrap());
    assert_eq!(pool.idle_bytes(), BUFFER_SIZES[2] as u64);
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
fn oversized_and_write_scratch_allocations_are_not_pooled() {
    let length = BUFFER_SIZES[5] + BUFFER_ALIGNMENT;
    let pool = BufferPool::new(12 * length as u64, MemoryMetrics::noop());
    let buffer = pool.allocate(length).unwrap();
    assert_eq!(buffer.capacity(), length);
    assert_eq!(buffer.as_ref().len(), length);
    assert!(buffer.pool.upgrade().is_none());
    drop(buffer);
    assert_eq!(pool.used_bytes(), 0);
    let scratch = AlignedBuffer::allocate_zeroed(BUFFER_ALIGNMENT).unwrap();
    assert_eq!(scratch.capacity(), BUFFER_ALIGNMENT);
    assert!(scratch.pool.upgrade().is_none());
    assert!(scratch.as_ref().iter().all(|byte| *byte == 0));
    assert_eq!(
        pool.allocate(usize::MAX).err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn metrics_follow_reuse_idle_limit_and_pool_lifetime() {
    for size in BUFFER_SIZES {
        let (registry, backend) = registry();
        let metrics = MemoryMetrics::new(&backend);
        let capacity = (100 * size as u64).div_ceil(7);
        let pool = BufferPool::new(capacity, metrics.clone());
        let gauge = |name| value(&registry, name, &[]);
        let bucket = size.to_string();
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
        let bytes = pool.allocate(size - BUFFER_ALIGNMENT).unwrap().into_bytes();
        let slice = bytes.slice(1..);
        drop(bytes);
        assert_eq!(buffer_bytes("used"), size as f64);
        assert_eq!(buffer_bytes("idle"), 0.0);
        assert_eq!(gauge("feuer_memory_used_bytes"), 0.0);
        let other = pool.allocate(size).unwrap();
        assert_eq!(buffer_bytes("used"), (2 * size) as f64);
        drop(slice);
        assert_eq!(buffer_bytes("used"), size as f64);
        assert_eq!(buffer_bytes("idle"), size as f64);
        drop(other);
        assert_eq!(buffer_bytes("idle"), size as f64);
        assert_eq!(buffer_bytes("used"), 0.0);
        assert_eq!(gauge("feuer_memory_used_bytes"), size as f64);
        let reused = pool.allocate(size).unwrap();
        assert_eq!(buffer_bytes("idle"), 0.0);
        assert_eq!(buffer_bytes("used"), size as f64);
        assert_eq!(gauge("feuer_memory_used_bytes"), 0.0);
        drop(reused);
        let outstanding = pool.allocate(size).unwrap();
        let second = BufferPool::new(capacity, metrics);
        drop(second.allocate(size).unwrap());
        assert_eq!(gauge("feuer_memory_capacity_bytes"), (2 * capacity) as f64);
        drop(pool);
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
fn bucket_gauges_follow_pressure_reclamation_and_shutdown_independently() {
    let (registry, backend) = registry();
    let capacity = 100 * BUFFER_SIZES[2] as u64;
    let pool = BufferPool::new(capacity, MemoryMetrics::new(&backend));
    let idle = |size: usize| {
        value(
            &registry,
            "feuer_io_buffer_pool_bytes",
            &[("bucket", &size.to_string()), ("status", "idle")],
        )
    };
    for &size in &BUFFER_SIZES[..3] {
        drop(pool.allocate(size).unwrap());
    }
    for (index, &size) in BUFFER_SIZES.iter().enumerate() {
        assert_eq!(idle(size), if index < 3 { size as f64 } else { 0.0 });
    }
    pool.add_cached(capacity - BUFFER_SIZES[0] as u64);
    for (index, &size) in BUFFER_SIZES.iter().enumerate() {
        assert_eq!(idle(size), if index == 0 { size as f64 } else { 0.0 });
    }
    pool.remove_cached(capacity - BUFFER_SIZES[0] as u64);
    drop(pool);
    for size in BUFFER_SIZES {
        assert_eq!(idle(size), 0.0);
    }
    for family in registry.gather() {
        match family.name() {
            "feuer_io_buffer_pool_bytes" => assert_eq!(family.get_metric().len(), 12),
            _ => {}
        }
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
                pool.add_cached(capacity);
                assert_eq!(pool.used_bytes(), capacity);
                assert_eq!(pool.idle_bytes(), 0);
                pool.remove_cached(capacity);
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

#[test]
fn idle_pool_percent_environment_override() {
    const NAME: &str = "FEUER_IDLE_BUFFER_POOL_PERCENT";
    const CHILD: &str = "FEUER_TEST_IDLE_BUFFER_POOL_PERCENT_CHILD";
    if let Ok(expected) = std::env::var(CHILD) {
        let result = std::panic::catch_unwind(|| BufferPool::new(u64::MAX, MemoryMetrics::noop()));
        if expected == "error" {
            assert!(result.is_err());
        } else {
            let percent: u64 = expected.parse().unwrap();
            let pool = result.unwrap();
            assert_eq!(
                pool.idle_limit,
                (u128::from(u64::MAX) * u128::from(percent) / 100) as u64
            );
            drop(pool.allocate(1).unwrap());
            assert_eq!(pool.idle_bytes(), if percent == 0 { 0 } else { BUFFER_SIZES[0] as u64 });
        }
        return;
    }
    for (setting, expected) in [
        (None, "7"),
        (Some("0"), "0"),
        (Some("10"), "10"),
        (Some("100"), "100"),
        (Some("101"), "error"),
        (Some("-1"), "error"),
        (Some("1.5"), "error"),
        (Some("bad"), "error"),
    ] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", "buffer::tests::idle_pool_percent_environment_override"]);
        child.env(CHILD, expected).env_remove(NAME);
        if let Some(setting) = setting {
            child.env(NAME, setting);
        }
        let output = child.output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}
