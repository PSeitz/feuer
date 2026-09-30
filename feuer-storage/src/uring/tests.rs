use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use super::*;

type IoResultReceiver = oneshot::Receiver<io::Result<IoBuffers>>;

fn request(queue: &IoQueue, operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver) {
    let (reply, receive) = oneshot::channel();
    let request_permit = queue.resources.request_slots.clone().try_acquire_owned().unwrap();
    let buffers = if operation == IoOperation::Write {
        let mut buffer = AlignedIoBuffer::allocate_zeroed(length).unwrap();
        buffer.as_mut_slice().fill(0x99);
        IoBuffers::Write {
            bytes: vec![buffer.into_bytes()],
            vectors: Vec::with_capacity(1),
            _region: None,
        }
    } else {
        IoBuffers::Read(AlignedIoBuffer::new(length, Vec::new(), queue.resources.buffer_pool(length)).unwrap())
    };
    (
        IoRequest::new(offset, buffers, 0..length, reply, request_permit),
        receive,
    )
}

fn queue() -> IoQueue {
    let temporary = tempfile::NamedTempFile::new().unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_DIRECT)
        .open(temporary.path())
        .unwrap();
    file.set_len(MAX_IO_CHUNK_BYTES as u64).unwrap();
    // SAFETY: eventfd returns a fresh descriptor, checked before assuming ownership.
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(fd >= 0);
    // SAFETY: fd was just created and has no other owner.
    let wake_fd = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
    let (_, receiver) = mpsc::sync_channel(MAX_IN_FLIGHT_READS); // disconnected: processing requests will drain and exit
    IoQueue {
        resources: Arc::new(IoQueueResources::new(
            MAX_IN_FLIGHT_READS,
            IoOperation::Read,
            &IoMetrics::noop(),
        )),
        ring: IoUring::new(MAX_IN_FLIGHT_READS as u32).unwrap(),
        file: Some(Arc::new(file)),
        directory_lock: Some(Arc::new(tempfile::tempfile().unwrap())),
        wake_fd,
        receiver,
        pending: VecDeque::new(),
        active: (0..MAX_IN_FLIGHT_READS).map(|_| None).collect(),
    }
}

#[test]
fn buffer_pool_metrics_follow_reuse_capacity_and_pool_lifetime() {
    use crate::test_metrics::{registry, value};

    let (registry, backend) = registry();
    let metrics = IoMetrics::new(&backend);
    for (index, (name, length)) in [
        ("small", DIRECT_IO_ALIGNMENT_BYTES),
        ("medium", 2 * 1024 * 1024),
        ("large", 10 * 1024 * 1024),
    ]
    .into_iter()
    .enumerate()
    {
        let handles = metrics.read_buffer_pools[index].clone();
        let pool = Arc::new(Mutex::new(IdleIoBuffers::new(length, handles.clone())));
        let labels = [("pool", name)];
        let gauge = |name| value(&registry, name, &labels);
        let returns = |outcome| {
            value(
                &registry,
                "feuer_io_buffer_pool_returns_total",
                &[labels[0], ("outcome", outcome)],
            )
        };
        assert_eq!(gauge("feuer_io_buffer_pool_capacity_bytes"), length as f64);
        assert_eq!(gauge("feuer_io_buffer_pool_idle_bytes"), 0.0);
        let bytes = AlignedIoBuffer::new(length, Vec::new(), &pool).unwrap().into_bytes();
        let slice = bytes.slice(1..);
        drop(bytes);
        assert_eq!(gauge("feuer_io_buffer_pool_idle_bytes"), 0.0);
        let other = AlignedIoBuffer::new(length, Vec::new(), &pool).unwrap();
        assert_eq!(returns("returned"), 0.0);
        assert_eq!(returns("dropped"), 0.0);
        drop(slice);
        assert_eq!(gauge("feuer_io_buffer_pool_idle_bytes"), length as f64);
        assert_eq!(returns("returned"), 1.0);
        drop(other);
        assert_eq!(returns("dropped"), 1.0);
        assert_eq!(
            100.0 * returns("dropped") / (returns("returned") + returns("dropped")),
            50.0
        );
        let reused = AlignedIoBuffer::new(length, Vec::new(), &pool).unwrap();
        assert_eq!(gauge("feuer_io_buffer_pool_idle_bytes"), 0.0);
        drop(reused);
        let outstanding = AlignedIoBuffer::new(length + DIRECT_IO_ALIGNMENT_BYTES, Vec::new(), &pool).unwrap();
        assert_eq!(returns("returned"), 2.0);
        let second_pool = Arc::new(Mutex::new(IdleIoBuffers::new(length, handles)));
        drop(AlignedIoBuffer::new(length, Vec::new(), &second_pool).unwrap());
        assert_eq!(gauge("feuer_io_buffer_pool_capacity_bytes"), (2 * length) as f64);
        assert_eq!(gauge("feuer_io_buffer_pool_idle_bytes"), (2 * length) as f64);
        drop(pool);
        assert_eq!(gauge("feuer_io_buffer_pool_capacity_bytes"), length as f64);
        assert_eq!(gauge("feuer_io_buffer_pool_idle_bytes"), length as f64);
        drop(outstanding);
        // Shutdown and results outliving their pool are not returns.
        drop(second_pool);
        assert_eq!(returns("dropped"), 1.0);
        assert_eq!(returns("returned"), 3.0);
        assert_eq!(gauge("feuer_io_buffer_pool_capacity_bytes"), 0.0);
        assert_eq!(gauge("feuer_io_buffer_pool_idle_bytes"), 0.0);
    }
}

#[test]
fn write_queue_has_no_buffer_pools() {
    let (registry, backend) = crate::test_metrics::registry();
    let metrics = IoMetrics::new(&backend);
    let resources = IoQueueResources::new(MAX_IN_FLIGHT_WRITES, IoOperation::Write, &metrics);
    assert!(resources.idle_buffers.is_none());
    for family in registry.gather() {
        if family.name().starts_with("feuer_io_buffer_pool_") {
            for metric in family.get_metric() {
                assert!(metric.get_label().iter().all(|label| label.name() != "operation"));
            }
        }
    }
    for pool in ["small", "medium", "large"] {
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_io_buffer_pool_capacity_bytes", &[("pool", pool)]),
            0.0
        );
    }
}

#[test]
fn disabled_buffer_pool_metrics_count_all_returns_as_dropped() {
    use crate::test_metrics::{registry, value};

    let (registry, backend) = registry();
    let metrics = IoMetrics::new(&backend);
    let pool = Arc::new(Mutex::new(IdleIoBuffers::new(0, metrics.read_buffer_pools[0].clone())));
    let labels = [("pool", "small")];
    for _ in 0..2 {
        drop(AlignedIoBuffer::new(DIRECT_IO_ALIGNMENT_BYTES, Vec::new(), &pool).unwrap());
    }
    assert_eq!(value(&registry, "feuer_io_buffer_pool_capacity_bytes", &labels), 0.0);
    assert_eq!(value(&registry, "feuer_io_buffer_pool_idle_bytes", &labels), 0.0);
    for (outcome, expected) in [("returned", 0.0), ("dropped", 2.0)] {
        assert_eq!(
            value(
                &registry,
                "feuer_io_buffer_pool_returns_total",
                &[labels[0], ("outcome", outcome)]
            ),
            expected
        );
    }
}

#[test]
fn aligned_buffer_returns_to_pool_only_after_last_bytes_reference() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = Arc::new(Mutex::new(IdleIoBuffers::new(
        2 * page,
        IoMetrics::noop().read_buffer_pools[0].clone(),
    )));
    let mut buffer = AlignedIoBuffer::new(page, Vec::new(), &pool).unwrap();
    buffer.as_mut_slice().fill(0x99);
    let address = buffer.ptr.as_ptr();
    let bytes = buffer.into_bytes();
    let slice = bytes.slice(1..);
    drop(bytes);
    assert_eq!(pool.lock().unwrap().bytes, 0);
    let other = AlignedIoBuffer::new(page, Vec::new(), &pool).unwrap();
    assert_ne!(other.ptr.as_ptr(), address);
    assert_eq!(&slice[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES - 1]);
    drop(slice);
    assert_eq!(pool.lock().unwrap().bytes, page);
    let reused = AlignedIoBuffer::new(page, Vec::new(), &pool).unwrap();
    assert_eq!(reused.ptr.as_ptr(), address);
    assert_eq!(pool.lock().unwrap().bytes, 0);
    assert!(pool.lock().unwrap().by_length.is_empty());
    // Outstanding results remain valid and do not keep the pool alive.
    let weak = Arc::downgrade(&pool);
    let bytes = reused.into_bytes();
    drop(other);
    drop(pool);
    assert!(weak.upgrade().is_none());
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    drop(bytes);
}

#[test]
fn aligned_buffer_pool_matches_sizes_and_bounds_idle_memory() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = Arc::new(Mutex::new(IdleIoBuffers::new(
        3 * page,
        IoMetrics::noop().read_buffer_pools[0].clone(),
    )));
    let small = AlignedIoBuffer::new(page, Vec::new(), &pool).unwrap();
    let address = small.ptr.as_ptr();
    drop(small);
    let larger = AlignedIoBuffer::new(2 * page, Vec::new(), &pool).unwrap();
    assert_ne!(larger.ptr.as_ptr(), address);
    assert_eq!(pool.lock().unwrap().bytes, page);
    drop(larger);
    assert_eq!(pool.lock().unwrap().bytes, 3 * page);
    // These unmatched allocations cannot fit in the full pool.
    drop(AlignedIoBuffer::new(3 * page, Vec::new(), &pool).unwrap());
    drop(AlignedIoBuffer::new(4 * page, Vec::new(), &pool).unwrap());
    assert_eq!(pool.lock().unwrap().bytes, 3 * page);
    assert_eq!(pool.lock().unwrap().by_length.len(), 2);
    let reused = AlignedIoBuffer::new(page, Vec::new(), &pool).unwrap();
    assert_eq!(reused.ptr.as_ptr(), address);
    assert_eq!(pool.lock().unwrap().bytes, 2 * page);
    drop(reused);
    assert_eq!(pool.lock().unwrap().bytes, 3 * page);
}

#[test]
fn io_buffer_pool_capacities_follow_environment() {
    // A child process avoids changing the environment of concurrent tests.
    if let Ok(expected_small) = std::env::var("FEUER_TEST_IO_BUFFER_POOLS_CHILD") {
        let (registry, backend) = crate::test_metrics::registry();
        let metrics = IoMetrics::new(&backend);
        let resources = IoQueueResources::new(MAX_IN_FLIGHT_READS, IoOperation::Read, &metrics);
        for (index, name) in ["small", "medium", "large"].into_iter().enumerate() {
            assert_eq!(
                crate::test_metrics::value(&registry, "feuer_io_buffer_pool_capacity_bytes", &[("pool", name)]),
                IDLE_IO_BUFFER_CAPACITIES[index] as f64
            );
        }
        let expected = [expected_small.parse::<usize>().unwrap(), 8192, 5 * 1024 * 1024 * 1024];
        for (pool, capacity) in resources.idle_buffers.as_ref().unwrap().iter().zip(expected) {
            assert_eq!(pool.lock().unwrap().capacity, capacity);
            drop(AlignedIoBuffer::new(DIRECT_IO_ALIGNMENT_BYTES, Vec::new(), pool).unwrap());
            let retained = if capacity >= DIRECT_IO_ALIGNMENT_BYTES {
                DIRECT_IO_ALIGNMENT_BYTES
            } else {
                0
            };
            assert_eq!(pool.lock().unwrap().bytes, retained);
        }
        return;
    }
    for (small_capacity, expected) in [
        ("0", Some(0)),
        ("1024", Some(1024)),
        ("1KiB", Some(1024)),
        ("1KB", Some(1000)),
        ("1.5 MiB", Some(1572864)),
        ("2mb", Some(2000000)),
        ("invalid", None),
        ("-1", None),
        ("18446744073709551616", None),
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "uring::tests::io_buffer_pool_capacities_follow_environment",
                "--nocapture",
            ])
            .env("FEUER_TEST_IO_BUFFER_POOLS_CHILD", expected.unwrap_or(0).to_string())
            .env("FEUER_SMALL_IO_BUFFER_POOL_BYTES", small_capacity)
            .env("FEUER_MEDIUM_IO_BUFFER_POOL_BYTES", "8KiB")
            .env("FEUER_LARGE_IO_BUFFER_POOL_BYTES", "5GiB")
            .output()
            .unwrap();
        assert_eq!(output.status.success(), expected.is_some(), "{output:?}");
        if expected.is_none() {
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("FEUER_SMALL_IO_BUFFER_POOL_BYTES must be a number >= 0 fitting usize")
            );
        }
    }
}

#[test]
fn small_medium_and_large_buffer_pools_have_independent_budgets() {
    let resources = IoQueueResources::new(MAX_IN_FLIGHT_READS, IoOperation::Read, &IoMetrics::noop());
    let pools = resources.idle_buffers.as_ref().unwrap();
    let mib = 1024 * 1024;
    for (length, index) in [
        (DIRECT_IO_ALIGNMENT_BYTES, 0),
        (mib, 0),
        (mib + DIRECT_IO_ALIGNMENT_BYTES, 1),
        (10 * mib - DIRECT_IO_ALIGNMENT_BYTES, 1),
        (10 * mib, 2),
        (20 * mib, 2),
    ] {
        let pool = resources.buffer_pool(length);
        assert!(Arc::ptr_eq(pool, &pools[index]));
        assert_eq!(pool.lock().unwrap().capacity, IDLE_IO_BUFFER_CAPACITIES[index]);
    }
    for length in [DIRECT_IO_ALIGNMENT_BYTES, 2 * mib, 10 * mib] {
        let pool = resources.buffer_pool(length);
        // Exercise each independent cap without allocating gigabytes for the test.
        pool.lock().unwrap().capacity = length;
        let first = AlignedIoBuffer::new(length, Vec::new(), pool).unwrap();
        let second = AlignedIoBuffer::new(length, Vec::new(), pool).unwrap();
        let address = first.ptr.as_ptr();
        drop(first);
        drop(second);
        assert_eq!(pool.lock().unwrap().bytes, length);
        let reused = AlignedIoBuffer::new(length, Vec::new(), pool).unwrap();
        assert_eq!(reused.ptr.as_ptr(), address);
    }
    assert_eq!(pools[0].lock().unwrap().bytes, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(pools[1].lock().unwrap().bytes, 2 * mib);
    assert_eq!(pools[2].lock().unwrap().bytes, 10 * mib);
}

#[test]
fn pooled_buffers_release_read_guards() {
    use crate::allocation::{CHUNK_BYTES, DiskChunkAllocator};

    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = Arc::new(Mutex::new(IdleIoBuffers::new(
        page,
        IoMetrics::noop().read_buffer_pools[0].clone(),
    )));
    let allocator = DiskChunkAllocator::for_disk_range(0..CHUNK_BYTES).unwrap();
    for return_bytes in [false, true] {
        let region = allocator.reserve_chunks(1).unwrap().pop().unwrap();
        let buffer = AlignedIoBuffer::new(page, vec![region.read_guard()], &pool).unwrap();
        drop(region);
        assert_eq!(allocator.available_bytes(), 0);
        if return_bytes {
            let bytes = buffer.into_bytes();
            assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
            drop(bytes);
        } else {
            drop(buffer);
        }
        assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
        assert_eq!(pool.lock().unwrap().bytes, page);
        assert!(pool.lock().unwrap().by_length[&page][0].read_guards.is_empty());
    }
}

#[test]
fn full_ring_does_not_block_the_other_direction() {
    for operation in [IoOperation::Read, IoOperation::Write] {
        let other_operation = if operation == IoOperation::Read {
            IoOperation::Write
        } else {
            IoOperation::Read
        };
        let mut full_queue = queue();
        let mut other_queue = queue();
        other_queue.file = full_queue.file.clone();
        other_queue.directory_lock = full_queue.directory_lock.clone();
        let mut full_replies = Vec::new();
        let mut other_replies = Vec::new();
        for i in 0..MAX_IN_FLIGHT_READS {
            let (request, reply) = request(
                &full_queue,
                operation,
                (i * DIRECT_IO_ALIGNMENT_BYTES) as u64,
                DIRECT_IO_ALIGNMENT_BYTES,
            );
            full_queue.pending.push_back(request);
            full_replies.push(reply);
        }
        full_queue.queue_pending_requests();
        full_queue.ring.submit().unwrap();
        assert_eq!(full_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_READS);
        assert_eq!(full_queue.resources.request_slots.available_permits(), 0);
        assert!(full_queue.resources.request_slots.clone().try_acquire_owned().is_err());

        for i in 0..MAX_IN_FLIGHT_READS {
            // Use disjoint physical pages on the same backing file.
            let (request, reply) = request(
                &other_queue,
                other_operation,
                ((MAX_IN_FLIGHT_READS + i) * DIRECT_IO_ALIGNMENT_BYTES) as u64,
                DIRECT_IO_ALIGNMENT_BYTES,
            );
            other_queue.pending.push_back(request);
            other_replies.push(reply);
        }
        other_queue.queue_pending_requests();
        assert_eq!(other_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_READS);
        assert_eq!(other_queue.ring.submission().len(), MAX_IN_FLIGHT_READS);
        assert!(other_queue.pending.is_empty());
        // Complete an entire ring without processing any completions on the full queue.
        other_queue.process_requests_until_disconnected().unwrap();
        for mut reply in other_replies {
            reply.try_recv().unwrap().unwrap();
        }
        assert_eq!(full_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_READS);
        full_queue.process_requests_until_disconnected().unwrap();
        for mut reply in full_replies {
            reply.try_recv().unwrap().unwrap();
        }
        for queue in [&full_queue, &other_queue] {
            assert!(queue.active.iter().all(Option::is_none));
            assert_eq!(queue.resources.request_slots.available_permits(), MAX_IN_FLIGHT_READS);
        }
    }
}

#[test]
fn exhausted_write_request_slots_leave_read_capacity() {
    let write_queue = queue();
    let read_queue = queue();
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_READS {
        writes.push(request(&write_queue, IoOperation::Write, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(write_queue.resources.request_slots.available_permits(), 0);
    for _ in 0..MAX_IN_FLIGHT_READS {
        reads.push(request(&read_queue, IoOperation::Read, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(read_queue.resources.request_slots.available_permits(), 0);
    drop((reads, writes));
    for queue in [&read_queue, &write_queue] {
        assert_eq!(queue.resources.request_slots.available_permits(), MAX_IN_FLIGHT_READS);
    }
}

#[tokio::test]
async fn reads_progress_with_write_request_slots_exhausted_and_after_write_shutdown() {
    let queue = queue();
    let file = queue.file.as_ref().unwrap();
    let lock = queue.directory_lock.as_ref().unwrap();
    let read_queue = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Read, &IoMetrics::noop()).unwrap();
    let write_queue = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Write, &IoMetrics::noop()).unwrap();
    write_queue
        .write_parts(
            0,
            DIRECT_IO_ALIGNMENT_BYTES,
            &[(0, Bytes::from(vec![0x99; DIRECT_IO_ALIGNMENT_BYTES]))],
            None,
        )
        .await
        .unwrap();
    assert_eq!(read_queue.resources.request_slots.available_permits(), 64);
    assert_eq!(write_queue.resources.request_slots.available_permits(), 8);
    let _requests = write_queue
        .resources
        .request_slots
        .clone()
        .acquire_many_owned(MAX_IN_FLIGHT_WRITES as u32)
        .await
        .unwrap();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read_queue.read(0, DIRECT_IO_ALIGNMENT_BYTES),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);

    // The read queue must also keep the shared file and directory lock alive.
    let lock = Arc::downgrade(lock);
    drop(queue);
    drop(write_queue);
    assert!(lock.upgrade().is_some());
    let bytes = read_queue.read(0, DIRECT_IO_ALIGNMENT_BYTES).await.unwrap();
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    drop(read_queue);
    assert!(lock.upgrade().is_none());
}

#[test]
fn canceled_submitted_write_retains_resources() {
    let mut queue = queue();
    let allocator = crate::allocation::DiskChunkAllocator::for_disk_range(0..MAX_IO_CHUNK_BYTES as u64).unwrap();
    let (mut write, reply) = request(&queue, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    if let IoBuffers::Write { _region: region, .. } = &mut write.buffers {
        *region = allocator.reserve_chunks(1).unwrap().pop();
    }
    queue.pending.push_back(write);
    queue.queue_pending_requests();
    // The write has reached the kernel, but its completion is not yet processed.
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.queue_pending_requests();
    assert!(allocator.reserve_chunks(1).is_none());
    assert!(queue.active[0].is_some());
    assert_eq!(
        queue.resources.request_slots.available_permits(),
        MAX_IN_FLIGHT_READS - 1
    );
    queue.process_requests_until_disconnected().unwrap();
    assert!(queue.active.iter().all(Option::is_none));
    assert_eq!(queue.resources.request_slots.available_permits(), MAX_IN_FLIGHT_READS);

    assert_eq!(allocator.available_bytes(), MAX_IO_CHUNK_BYTES as u64);

    // Only read/reuse the region after completion, not after dropping the receiver.
    let mut read_queue = self::queue();
    read_queue.file = queue.file.clone();
    read_queue.directory_lock = queue.directory_lock.clone();
    let (read, mut reply) = request(&read_queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    read_queue.pending.push_back(read);
    read_queue.process_requests_until_disconnected().unwrap();
    assert_eq!(
        reply.try_recv().unwrap().unwrap().into_read().as_ref(),
        &[0x99; DIRECT_IO_ALIGNMENT_BYTES]
    );
}

#[test]
fn foreground_reads_are_submitted_before_queued_recovery_reads() {
    let mut queue = queue();
    let (mut recovery, mut recovery_reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    recovery.recovery = true;
    let (foreground, mut foreground_reply) = request(
        &queue,
        IoOperation::Read,
        DIRECT_IO_ALIGNMENT_BYTES as u64,
        DIRECT_IO_ALIGNMENT_BYTES,
    );
    queue.pending.push_back(recovery);
    queue.pending.push_back(foreground);
    queue.queue_pending_requests();
    assert!(!queue.active[0].as_ref().unwrap().recovery);
    assert!(queue.active[1].as_ref().unwrap().recovery);
    queue.process_requests_until_disconnected().unwrap();
    foreground_reply.try_recv().unwrap().unwrap();
    recovery_reply.try_recv().unwrap().unwrap();
}

#[tokio::test]
async fn recovery_skips_read_without_request_slot() {
    let queue = queue();
    let handle = IoQueueHandle::new(
        queue.file.as_ref().unwrap().clone(),
        queue.directory_lock.as_ref().unwrap().clone(),
        IoOperation::Read,
        &IoMetrics::noop(),
    )
    .unwrap();
    let allocator = crate::allocation::DiskChunkAllocator::for_disk_range(0..MAX_IO_CHUNK_BYTES as u64).unwrap();
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let page = chunk.slice(0..DIRECT_IO_ALIGNMENT_BYTES as u64);
    let request_slots = handle
        .resources
        .request_slots
        .clone()
        .acquire_many_owned(MAX_IN_FLIGHT_READS as u32)
        .await
        .unwrap();
    let read = handle.try_read_recovery_page(page.read_guard());
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), read)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_none());
    assert!(
        handle
            .resources
            .buffer_pool(DIRECT_IO_ALIGNMENT_BYTES)
            .lock()
            .unwrap()
            .by_length
            .is_empty()
    );
    drop(request_slots);
    assert!(
        handle
            .try_read_recovery_page(page.read_guard())
            .await
            .unwrap()
            .is_some()
    );
}

#[test]
fn discarded_queued_requests_never_reach_the_ring() {
    let mut queue = queue();
    let (request, reply) = request(&queue, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    queue.pending.push_back(request);
    drop(reply);
    queue.queue_pending_requests();
    assert!(queue.pending.is_empty());
    assert!(queue.active.iter().all(Option::is_none));
    assert!(queue.ring.submission().is_empty());
    assert_eq!(queue.resources.request_slots.available_permits(), MAX_IN_FLIGHT_READS);
}

#[test]
fn finished_read_transfers_buffer_ownership() {
    let queue = queue();
    let (mut read, mut reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    let IoBuffers::Read(buffer) = &mut read.buffers else {
        unreachable!()
    };
    buffer.as_mut_slice().fill(0x99);
    let ptr = buffer.ptr.as_ptr().cast_const();
    assert!(!read.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    read.send_result(Ok(()));

    let bytes = reply.try_recv().unwrap().unwrap().into_read().into_bytes();
    assert_eq!(bytes.as_ptr(), ptr);
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    assert_eq!(queue.resources.request_slots.available_permits(), MAX_IN_FLIGHT_READS);

    let slice = bytes.slice(1..);
    drop(bytes);
    drop(queue);
    assert_eq!(slice.as_ptr(), ptr.wrapping_add(1));
    assert_eq!(&slice[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES - 1]);
}

#[tokio::test]
async fn reads_into_consecutive_slices_without_reallocating() {
    let queue = queue();
    let file = queue.file.as_ref().unwrap();
    let lock = queue.directory_lock.as_ref().unwrap();
    let read = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Read, &IoMetrics::noop()).unwrap();
    let write = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Write, &IoMetrics::noop()).unwrap();
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    write
        .write_parts(0, page, &[(0, Bytes::from(vec![0x99; page]))], None)
        .await
        .unwrap();
    write
        .write_parts((2 * page) as u64, page, &[(0, Bytes::from(vec![0x77; page]))], None)
        .await
        .unwrap();
    let mut buffer = read.allocate_buffer(3 * page, Vec::new()).unwrap();
    buffer.as_mut_slice().fill(0x55);
    let address = buffer.ptr.as_ptr() as usize;
    buffer = read.read_into(0, buffer, 0..page).await.unwrap();
    assert_eq!(buffer.ptr.as_ptr() as usize, address);
    buffer = read.read_into((2 * page) as u64, buffer, page..2 * page).await.unwrap();
    let bytes = buffer.into_bytes();
    assert_eq!(bytes.as_ptr() as usize, address);
    assert_eq!(&bytes[..page], &vec![0x99; page]);
    assert_eq!(&bytes[page..2 * page], &vec![0x77; page]);
    assert_eq!(&bytes[2 * page..], &vec![0x55; page]);
}

#[test]
fn canceled_read_retains_destination_and_disk_guard_until_completion() {
    use crate::allocation::{CHUNK_BYTES, DiskChunkAllocator};
    let allocator = DiskChunkAllocator::for_disk_range(0..CHUNK_BYTES).unwrap();
    let region = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let mut queue = queue();
    let (mut read, reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    let buffer = AlignedIoBuffer::new(
        2 * DIRECT_IO_ALIGNMENT_BYTES,
        vec![region.read_guard()],
        queue.resources.buffer_pool(2 * DIRECT_IO_ALIGNMENT_BYTES),
    )
    .unwrap();
    let address = buffer.ptr.as_ptr();
    read.buffers = IoBuffers::Read(buffer);
    read.destination = DIRECT_IO_ALIGNMENT_BYTES..2 * DIRECT_IO_ALIGNMENT_BYTES;
    drop(region);
    queue.pending.push_back(read);
    queue.queue_pending_requests();
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.queue_pending_requests();
    assert_eq!(allocator.available_bytes(), 0);
    assert!(queue.active[0].is_some());
    assert!(
        !queue
            .resources
            .buffer_pool(2 * DIRECT_IO_ALIGNMENT_BYTES)
            .lock()
            .unwrap()
            .by_length
            .contains_key(&(2 * DIRECT_IO_ALIGNMENT_BYTES))
    );
    queue.process_requests_until_disconnected().unwrap();
    assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    assert_eq!(
        queue
            .resources
            .buffer_pool(2 * DIRECT_IO_ALIGNMENT_BYTES)
            .lock()
            .unwrap()
            .by_length[&(2 * DIRECT_IO_ALIGNMENT_BYTES)][0]
            .ptr
            .as_ptr(),
        address
    );
}

#[test]
fn writes_borrow_aligned_bytes_and_copy_unaligned_bytes() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let resources = IoQueueResources::new(MAX_IN_FLIGHT_READS, IoOperation::Read, &IoMetrics::noop());
    let pool = resources.buffer_pool(4 * page);
    let mut source = AlignedIoBuffer::new(2 * page, Vec::new(), pool).unwrap();
    source.as_mut_slice().fill(0x77);
    let source = source.into_bytes();
    for length in [17, page] {
        let mut dirty = AlignedIoBuffer::new(2 * page, Vec::new(), pool).unwrap();
        dirty.as_mut_slice().fill(0xff);
        drop(dirty);
        let parts = [(0, source.slice(1..length + 1)), (page, source.slice(..page + 17))];
        let mut buffers = IoBuffers::from_write_parts(4 * page, &parts, None).unwrap();
        assert_eq!(pool.lock().unwrap().bytes, 2 * page);
        buffers.submission_entry(types::Fd(-1), page as u64, page..4 * page);
        let IoBuffers::Write { bytes, vectors, .. } = buffers else {
            unreachable!()
        };
        assert_eq!(bytes.len(), 3);
        assert_ne!(bytes[0].as_ptr(), parts[0].1.as_ptr());
        assert_eq!(&bytes[0][..length], parts[0].1);
        assert!(bytes[0][length..].iter().all(|&byte| byte == 0));
        assert_eq!(bytes[1].as_ptr(), source.as_ptr());
        assert_eq!(bytes[1].len(), page);
        assert_eq!(&bytes[2][..17], &source[..17]);
        assert!(bytes[2][17..].iter().all(|&byte| byte == 0));
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].iov_base.cast_const().cast::<u8>(), source.as_ptr());
        drop(bytes);
        assert_eq!(pool.lock().unwrap().bytes, 2 * page);
    }
}

#[test]
fn completion_state_handles_short_io_and_errors() {
    let queue = queue();
    let (mut read, _reply) = request(&queue, IoOperation::Read, 0, 2 * DIRECT_IO_ALIGNMENT_BYTES);
    assert!(read.apply_completion_result(-libc::EINTR).unwrap());
    assert_eq!(read.completed_bytes, 0);
    assert!(read.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert!(!read.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());

    let (mut read, _reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(
        read.apply_completion_result(17).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let (mut write, _reply) = request(&queue, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(
        write.apply_completion_result(0).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    assert_eq!(
        write.apply_completion_result(-libc::ENOSPC).unwrap_err().raw_os_error(),
        Some(libc::ENOSPC)
    );

    let (mut write, _reply) = request(&queue, IoOperation::Write, 0, 2 * DIRECT_IO_ALIGNMENT_BYTES);
    assert!(write.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert_eq!(write.completed_bytes, DIRECT_IO_ALIGNMENT_BYTES);
    assert!(!write.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
}

#[test]
#[should_panic(expected = "offset.is_multiple_of")]
fn rejects_unaligned_read_offset() {
    request(&queue(), IoOperation::Read, 1, DIRECT_IO_ALIGNMENT_BYTES);
}

#[test]
#[should_panic(expected = "length.is_multiple_of")]
fn rejects_unaligned_read_length() {
    request(&queue(), IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES - 1);
}

#[test]
#[should_panic(expected = "length > 0")]
fn rejects_empty_request() {
    request(&queue(), IoOperation::Read, 0, 0);
}

#[test]
#[should_panic(expected = "offset.is_multiple_of")]
fn rejects_unaligned_write_offset() {
    request(&queue(), IoOperation::Write, 1, DIRECT_IO_ALIGNMENT_BYTES);
}

#[test]
#[should_panic(expected = "length.is_multiple_of")]
fn rejects_unaligned_write_length() {
    request(&queue(), IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES - 1);
}
