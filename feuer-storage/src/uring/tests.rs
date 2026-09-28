use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use super::*;

type IoResultReceiver = oneshot::Receiver<io::Result<AlignedIoBuffer>>;

fn request(queue: &IoQueue, operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver) {
    let (reply, receive) = oneshot::channel();
    let request_permit = queue.admission.request_slots.clone().try_acquire_owned().unwrap();
    let buffer_memory_permit = queue
        .admission
        .buffer_memory
        .clone()
        .try_acquire_many_owned((length / DIRECT_IO_ALIGNMENT_BYTES) as u32)
        .unwrap();
    let mut buffer = AlignedIoBuffer::new(length, Vec::new(), queue.admission.buffer_pool(length)).unwrap();
    if operation == IoOperation::Write {
        buffer.as_mut_slice().fill(0x99);
    }
    (
        IoRequest::new(
            operation,
            offset,
            buffer,
            0..length,
            reply,
            (request_permit, buffer_memory_permit),
        ),
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
    let (_, receiver) = mpsc::sync_channel(MAX_IN_FLIGHT_IO); // disconnected: run() will drain and exit
    IoQueue {
        admission: Arc::new(IoAdmissionBudgets::new(MAX_IN_FLIGHT_IO)),
        ring: IoUring::new(MAX_IN_FLIGHT_IO as u32).unwrap(),
        file: Some(Arc::new(file)),
        directory_lock: Some(Arc::new(tempfile::tempfile().unwrap())),
        wake_fd,
        receiver,
        pending: VecDeque::new(),
        active: (0..MAX_IN_FLIGHT_IO).map(|_| None).collect(),
    }
}

#[test]
fn aligned_buffer_returns_to_pool_only_after_last_bytes_reference() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = Arc::new(Mutex::new(IdleIoBuffers::new(2 * page)));
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
    let pool = Arc::new(Mutex::new(IdleIoBuffers::new(3 * page)));
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
fn small_medium_and_large_buffer_pools_have_independent_budgets() {
    let admission = IoAdmissionBudgets::new(MAX_IN_FLIGHT_IO);
    let mib = 1024 * 1024;
    for (length, index, capacity) in [
        (DIRECT_IO_ALIGNMENT_BYTES, 0, 128 * mib),
        (mib, 0, 128 * mib),
        (mib + DIRECT_IO_ALIGNMENT_BYTES, 1, 256 * mib),
        (10 * mib - DIRECT_IO_ALIGNMENT_BYTES, 1, 256 * mib),
        (10 * mib, 2, 1024 * mib),
        (20 * mib, 2, 1024 * mib),
    ] {
        let pool = admission.buffer_pool(length);
        assert!(Arc::ptr_eq(pool, &admission.idle_buffers[index]));
        assert_eq!(pool.lock().unwrap().capacity, capacity);
    }
    for length in [DIRECT_IO_ALIGNMENT_BYTES, 2 * mib, 10 * mib] {
        let pool = admission.buffer_pool(length);
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
    assert_eq!(
        admission.idle_buffers[0].lock().unwrap().bytes,
        DIRECT_IO_ALIGNMENT_BYTES
    );
    assert_eq!(admission.idle_buffers[1].lock().unwrap().bytes, 2 * mib);
    assert_eq!(admission.idle_buffers[2].lock().unwrap().bytes, 10 * mib);
}

#[test]
fn pooled_buffers_release_read_guards() {
    use crate::allocation::{CHUNK_BYTES, DiskChunkAllocator};

    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = Arc::new(Mutex::new(IdleIoBuffers::new(page)));
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
        for i in 0..MAX_IN_FLIGHT_IO {
            let (request, reply) = request(
                &full_queue,
                operation,
                (i * DIRECT_IO_ALIGNMENT_BYTES) as u64,
                DIRECT_IO_ALIGNMENT_BYTES,
            );
            full_queue.pending.push_back(request);
            full_replies.push(reply);
        }
        full_queue.schedule();
        full_queue.ring.submit().unwrap();
        assert_eq!(full_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
        assert_eq!(full_queue.admission.request_slots.available_permits(), 0);
        assert!(full_queue.admission.request_slots.clone().try_acquire_owned().is_err());

        for i in 0..MAX_IN_FLIGHT_IO {
            // Use disjoint physical pages on the same backing file.
            let (request, reply) = request(
                &other_queue,
                other_operation,
                ((MAX_IN_FLIGHT_IO + i) * DIRECT_IO_ALIGNMENT_BYTES) as u64,
                DIRECT_IO_ALIGNMENT_BYTES,
            );
            other_queue.pending.push_back(request);
            other_replies.push(reply);
        }
        other_queue.schedule();
        assert_eq!(other_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
        assert_eq!(other_queue.ring.submission().len(), MAX_IN_FLIGHT_IO);
        assert!(other_queue.pending.is_empty());
        // Complete an entire ring without processing any completions on the full queue.
        other_queue.run().unwrap();
        for mut reply in other_replies {
            reply.try_recv().unwrap().unwrap();
        }
        assert_eq!(full_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
        full_queue.run().unwrap();
        for mut reply in full_replies {
            reply.try_recv().unwrap().unwrap();
        }
        for queue in [&full_queue, &other_queue] {
            assert!(queue.active.iter().all(Option::is_none));
            assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
            assert_eq!(
                queue.admission.buffer_memory.available_permits(),
                MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
            );
        }
    }
}

#[test]
fn full_write_admission_and_buffers_leave_full_read_capacity() {
    let write_queue = queue();
    let read_queue = queue();
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_IO {
        writes.push(request(&write_queue, IoOperation::Write, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(write_queue.admission.request_slots.available_permits(), 0);
    assert_eq!(write_queue.admission.buffer_memory.available_permits(), 0);
    for _ in 0..MAX_IN_FLIGHT_IO {
        reads.push(request(&read_queue, IoOperation::Read, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(read_queue.admission.request_slots.available_permits(), 0);
    assert_eq!(read_queue.admission.buffer_memory.available_permits(), 0);
    drop((reads, writes));
    for queue in [&read_queue, &write_queue] {
        assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
        assert_eq!(
            queue.admission.buffer_memory.available_permits(),
            MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
        );
    }
}

#[tokio::test]
async fn read_worker_progresses_with_write_admission_exhausted_and_after_write_shutdown() {
    let queue = queue();
    let file = queue.file.as_ref().unwrap();
    let lock = queue.directory_lock.as_ref().unwrap();
    let read_queue = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Read).unwrap();
    let write_queue = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Write).unwrap();
    write_queue
        .execute(0, DIRECT_IO_ALIGNMENT_BYTES, &[0x99; DIRECT_IO_ALIGNMENT_BYTES])
        .await
        .unwrap();
    assert_eq!(read_queue.admission.request_slots.available_permits(), 64);
    assert_eq!(write_queue.admission.request_slots.available_permits(), 8);
    let _requests = write_queue
        .admission
        .request_slots
        .clone()
        .acquire_many_owned(MAX_IN_FLIGHT_WRITES as u32)
        .await
        .unwrap();
    let _buffer_memory = write_queue
        .admission
        .buffer_memory
        .clone()
        .acquire_many_owned((MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES) as u32)
        .await
        .unwrap();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read_queue.execute(0, DIRECT_IO_ALIGNMENT_BYTES, &[]),
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
    let bytes = read_queue.execute(0, DIRECT_IO_ALIGNMENT_BYTES, &[]).await.unwrap();
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    drop(read_queue);
    assert!(lock.upgrade().is_none());
}

#[test]
fn canceled_submitted_write_retains_resources() {
    let mut queue = queue();
    let (write, reply) = request(&queue, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    queue.pending.push_back(write);
    queue.schedule();
    // The write has reached the kernel, but its completion is not yet processed.
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.schedule();
    assert!(queue.active[0].is_some());
    assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO - 1);
    assert_eq!(
        queue.admission.buffer_memory.available_permits(),
        MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES - 1
    );
    queue.run().unwrap();
    assert!(queue.active.iter().all(Option::is_none));
    assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        queue.admission.buffer_memory.available_permits(),
        MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
    );

    // Only read/reuse the region after completion, not after dropping the receiver.
    let mut read_queue = self::queue();
    read_queue.file = queue.file.clone();
    read_queue.directory_lock = queue.directory_lock.clone();
    let (read, mut reply) = request(&read_queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    read_queue.pending.push_back(read);
    read_queue.run().unwrap();
    assert_eq!(
        reply.try_recv().unwrap().unwrap().as_ref(),
        &[0x99; DIRECT_IO_ALIGNMENT_BYTES]
    );
}

#[test]
fn discarded_queued_requests_never_reach_the_ring() {
    let mut queue = queue();
    let (request, reply) = request(&queue, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    queue.pending.push_back(request);
    drop(reply);
    queue.schedule();
    assert!(queue.pending.is_empty());
    assert!(queue.active.iter().all(Option::is_none));
    assert!(queue.ring.submission().is_empty());
    assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
}

#[test]
fn finished_read_transfers_buffer_ownership() {
    let queue = queue();
    let (mut read, mut reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    read.io_buffer.as_mut_slice().fill(0x99);
    let ptr = read.io_buffer.ptr.as_ptr().cast_const();
    assert!(!read.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    read.finish(Ok(()));

    let bytes = reply.try_recv().unwrap().unwrap().into_bytes();
    assert_eq!(bytes.as_ptr(), ptr);
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        queue.admission.buffer_memory.available_permits(),
        MAX_IO_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
    );

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
    let read = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Read).unwrap();
    let write = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Write).unwrap();
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    write.execute(0, page, &vec![0x99; page]).await.unwrap();
    write.execute((2 * page) as u64, page, &vec![0x77; page]).await.unwrap();
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
    read.io_buffer = AlignedIoBuffer::new(
        2 * DIRECT_IO_ALIGNMENT_BYTES,
        vec![region.read_guard()],
        queue.admission.buffer_pool(2 * DIRECT_IO_ALIGNMENT_BYTES),
    )
    .unwrap();
    let address = read.io_buffer.ptr.as_ptr();
    read.destination = DIRECT_IO_ALIGNMENT_BYTES..2 * DIRECT_IO_ALIGNMENT_BYTES;
    drop(region);
    queue.pending.push_back(read);
    queue.schedule();
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.schedule();
    assert_eq!(allocator.available_bytes(), 0);
    assert!(queue.active[0].is_some());
    assert!(
        !queue
            .admission
            .buffer_pool(2 * DIRECT_IO_ALIGNMENT_BYTES)
            .lock()
            .unwrap()
            .by_length
            .contains_key(&(2 * DIRECT_IO_ALIGNMENT_BYTES))
    );
    queue.run().unwrap();
    assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    assert_eq!(
        queue
            .admission
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
fn completion_state_handles_short_io_and_errors() {
    let queue = queue();
    let (mut read, _reply) = request(&queue, IoOperation::Read, 0, 2 * DIRECT_IO_ALIGNMENT_BYTES);
    assert!(read.complete(-libc::EINTR).unwrap());
    assert_eq!(read.completed_bytes, 0);
    assert!(read.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert!(!read.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());

    let (mut read, _reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(read.complete(17).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    let (mut write, _reply) = request(&queue, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(write.complete(0).unwrap_err().kind(), io::ErrorKind::WriteZero);
    assert_eq!(
        write.complete(-libc::ENOSPC).unwrap_err().raw_os_error(),
        Some(libc::ENOSPC)
    );

    let (mut write, _reply) = request(&queue, IoOperation::Write, 0, 2 * DIRECT_IO_ALIGNMENT_BYTES);
    assert!(write.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert_eq!(write.completed_bytes, DIRECT_IO_ALIGNMENT_BYTES);
    assert!(!write.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
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
