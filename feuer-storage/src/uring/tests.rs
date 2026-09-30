use std::{fs::OpenOptions, future::Future, os::unix::fs::OpenOptionsExt, task::Context};

use super::*;

type IoResultReceiver = oneshot::Receiver<io::Result<IoBuffers>>;

fn request(operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver) {
    let (reply, receive) = oneshot::channel();
    let buffers = if operation == IoOperation::Write {
        let mut buffer = AlignedBuffer::allocate_zeroed(length).unwrap();
        buffer.as_mut_slice().fill(0x99);
        IoBuffers::Write {
            bytes: vec![buffer.into_bytes()],
            vectors: Vec::with_capacity(1),
            _region: None,
        }
    } else {
        IoBuffers::Read(AlignedIoBuffer::new(length, Vec::new(), &buffer_pool()).unwrap())
    };
    (IoRequest::new(offset, buffers, 0..length, reply), receive)
}

fn queue() -> (IoQueue, mpsc::Sender<IoRequest>) {
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
    let (sender, receiver) = mpsc::channel(MAX_IN_FLIGHT_READS);
    (
        IoQueue {
            ring: IoUring::new(MAX_IN_FLIGHT_READS as u32).unwrap(),
            file: Some(Arc::new(file)),
            directory_lock: Some(Arc::new(tempfile::tempfile().unwrap())),
            wake_fd,
            receiver,
            active: (0..MAX_IN_FLIGHT_READS).map(|_| None).collect(),
        },
        sender,
    )
}

fn buffer_pool() -> Arc<BufferPool> {
    feuer_memory::MemoryCache::new(384 * 1024 * 1024).buffer_pool()
}

#[test]
fn pooled_buffers_release_read_guards() {
    use crate::allocation::{CHUNK_BYTES, DiskChunkAllocator};

    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = buffer_pool();
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
        assert_eq!(pool.idle_bytes(), 32 * 1024);
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
        let (mut full_queue, full_sender) = queue();
        let (mut other_queue, other_sender) = queue();
        other_queue.file = full_queue.file.clone();
        other_queue.directory_lock = full_queue.directory_lock.clone();
        let mut full_replies = Vec::new();
        let mut other_replies = Vec::new();
        for i in 0..MAX_IN_FLIGHT_READS {
            let (request, reply) = request(
                operation,
                (i * DIRECT_IO_ALIGNMENT_BYTES) as u64,
                DIRECT_IO_ALIGNMENT_BYTES,
            );
            full_sender.try_send(request).unwrap();
            full_replies.push(reply);
        }
        drop(full_sender);
        full_queue.receive_requests();
        full_queue.ring.submit().unwrap();
        assert_eq!(full_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_READS);

        for i in 0..MAX_IN_FLIGHT_READS {
            // Use disjoint physical pages on the same backing file.
            let (request, reply) = request(
                other_operation,
                ((MAX_IN_FLIGHT_READS + i) * DIRECT_IO_ALIGNMENT_BYTES) as u64,
                DIRECT_IO_ALIGNMENT_BYTES,
            );
            other_sender.try_send(request).unwrap();
            other_replies.push(reply);
        }
        drop(other_sender);
        other_queue.receive_requests();
        assert_eq!(other_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_READS);
        assert_eq!(other_queue.ring.submission().len(), MAX_IN_FLIGHT_READS);
        assert!(other_queue.receiver.is_empty());
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
        }
    }
}

#[tokio::test]
async fn full_active_slots_keep_channel_full_until_completion() {
    let (mut queue, sender) = queue();
    let mut replies = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_READS {
        let (request, reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
        sender.try_send(request).unwrap();
        replies.push(reply);
    }
    queue.receive_requests();
    assert_eq!(sender.capacity(), MAX_IN_FLIGHT_READS);
    for _ in 0..MAX_IN_FLIGHT_READS {
        let (request, reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
        sender.try_send(request).unwrap();
        replies.push(reply);
    }
    assert_eq!(sender.capacity(), 0);
    queue.receive_requests();
    assert_eq!(queue.receiver.len(), MAX_IN_FLIGHT_READS);
    assert_eq!(queue.active.iter().flatten().count(), MAX_IN_FLIGHT_READS);
    {
        let reserve = sender.reserve();
        tokio::pin!(reserve);
        let mut context = Context::from_waker(std::task::Waker::noop());
        assert!(reserve.as_mut().poll(&mut context).is_pending());

        queue.ring.submit_and_wait(MAX_IN_FLIGHT_READS).unwrap();
        for completion in queue.ring.completion() {
            let slot = completion.user_data() as usize;
            let mut request = queue.active[slot].take().unwrap();
            assert!(!request.apply_completion_result(completion.result()).unwrap());
            request.send_result(Ok(()));
        }
        // Completion alone does not release channel capacity: receiving does.
        assert!(reserve.as_mut().poll(&mut context).is_pending());
        queue.receive_requests();
        drop(reserve.await.unwrap());
    }
    assert_eq!(sender.capacity(), MAX_IN_FLIGHT_READS);
    drop(sender);
    queue.process_requests_until_disconnected().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
}

#[tokio::test]
async fn reads_progress_with_write_channel_full_and_after_write_shutdown() {
    let (queue, _) = queue();
    let file = queue.file.as_ref().unwrap();
    let lock = queue.directory_lock.as_ref().unwrap();
    let pool = buffer_pool();
    let read_queue = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Read, pool.clone()).unwrap();
    let write_queue = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Write, pool).unwrap();
    assert!(write_queue.buffer_pool.is_none());
    write_queue
        .write_parts(
            0,
            DIRECT_IO_ALIGNMENT_BYTES,
            &[(0, Bytes::from(vec![0x99; DIRECT_IO_ALIGNMENT_BYTES]))],
            None,
        )
        .await
        .unwrap();
    assert_eq!(read_queue.sender.as_ref().unwrap().capacity(), 64);
    assert_eq!(write_queue.sender.as_ref().unwrap().capacity(), 8);
    let reservations = write_queue
        .sender
        .as_ref()
        .unwrap()
        .reserve_many(MAX_IN_FLIGHT_WRITES)
        .await
        .unwrap();
    assert!(matches!(
        write_queue.sender.as_ref().unwrap().try_reserve(),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read_queue.read(0, DIRECT_IO_ALIGNMENT_BYTES),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    drop(reservations);

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
    let (mut queue, sender) = queue();
    let allocator = crate::allocation::DiskChunkAllocator::for_disk_range(0..MAX_IO_CHUNK_BYTES as u64).unwrap();
    let (mut write, reply) = request(IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    if let IoBuffers::Write { _region: region, .. } = &mut write.buffers {
        *region = allocator.reserve_chunks(1).unwrap().pop();
    }
    sender.try_send(write).unwrap();
    drop(sender);
    queue.receive_requests();
    // The write has reached the kernel, but its completion is not yet processed.
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.receive_requests();
    assert!(allocator.reserve_chunks(1).is_none());
    assert!(queue.active[0].is_some());
    queue.process_requests_until_disconnected().unwrap();
    assert!(queue.active.iter().all(Option::is_none));
    assert_eq!(allocator.available_bytes(), MAX_IO_CHUNK_BYTES as u64);

    // Only read/reuse the region after completion, not after dropping the receiver.
    let (mut read_queue, sender) = self::queue();
    read_queue.file = queue.file.clone();
    read_queue.directory_lock = queue.directory_lock.clone();
    let (read, mut reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    sender.try_send(read).unwrap();
    drop(sender);
    read_queue.process_requests_until_disconnected().unwrap();
    assert_eq!(
        reply.try_recv().unwrap().unwrap().into_read().as_ref(),
        &[0x99; DIRECT_IO_ALIGNMENT_BYTES]
    );
}

#[test]
fn requests_are_submitted_in_channel_order_and_drained_on_shutdown() {
    let (mut queue, sender) = queue();
    let (first, mut first_reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    let (second, mut second_reply) = request(
        IoOperation::Read,
        DIRECT_IO_ALIGNMENT_BYTES as u64,
        DIRECT_IO_ALIGNMENT_BYTES,
    );
    sender.try_send(first).unwrap();
    sender.try_send(second).unwrap();
    drop(sender);
    queue.receive_requests();
    assert_eq!(queue.active[0].as_ref().unwrap().offset, 0);
    assert_eq!(
        queue.active[1].as_ref().unwrap().offset,
        DIRECT_IO_ALIGNMENT_BYTES as u64
    );
    queue.process_requests_until_disconnected().unwrap();
    first_reply.try_recv().unwrap().unwrap();
    second_reply.try_recv().unwrap().unwrap();
}

#[tokio::test]
async fn recovery_skips_full_channel_without_allocating_and_foreground_waits() {
    let (queue, _) = queue();
    let pool = buffer_pool();
    let handle = IoQueueHandle::new(
        queue.file.as_ref().unwrap().clone(),
        queue.directory_lock.as_ref().unwrap().clone(),
        IoOperation::Read,
        pool.clone(),
    )
    .unwrap();
    let allocator = crate::allocation::DiskChunkAllocator::for_disk_range(0..MAX_IO_CHUNK_BYTES as u64).unwrap();
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let page = chunk.slice(0..DIRECT_IO_ALIGNMENT_BYTES as u64);
    let reservations = handle
        .sender
        .as_ref()
        .unwrap()
        .reserve_many(MAX_IN_FLIGHT_READS)
        .await
        .unwrap();
    assert_eq!(pool.idle_bytes(), 0);
    let read = handle.try_read_recovery_page(page.read_guard());
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), read)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_none());
    assert_eq!(pool.idle_bytes(), 0);
    // An available pooled buffer must remain untouched while a foreground read waits.
    drop(handle.allocate_buffer(DIRECT_IO_ALIGNMENT_BYTES, Vec::new()).unwrap());
    let idle_bytes = pool.idle_bytes();
    assert!(idle_bytes > 0);
    {
        let read = handle.read(0, DIRECT_IO_ALIGNMENT_BYTES);
        tokio::pin!(read);
        assert!(
            read.as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
                .is_pending()
        );
        assert_eq!(pool.idle_bytes(), idle_bytes);
        // Cancel the admission waiter before releasing capacity.
    }
    drop(reservations);
    assert_eq!(handle.sender.as_ref().unwrap().capacity(), MAX_IN_FLIGHT_READS);
    assert!(
        handle
            .try_read_recovery_page(page.read_guard())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn queue_exit_releases_admission_waiters_and_queued_requests() {
    let (queue, sender) = queue();
    let handle = IoQueueHandle {
        operation: IoOperation::Read,
        sender: Some(sender),
        wake_fd: queue.wake_fd.clone(),
        thread: None,
        buffer_pool: Some(buffer_pool()),
    };
    let mut replies = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_READS {
        let (request, reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
        handle.sender.as_ref().unwrap().try_send(request).unwrap();
        replies.push(reply);
    }
    let read = handle.read(0, DIRECT_IO_ALIGNMENT_BYTES);
    tokio::pin!(read);
    assert!(
        read.as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    drop(queue);
    assert_eq!(read.await.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    for mut reply in replies {
        assert!(matches!(reply.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
    }
    let allocator = crate::allocation::DiskChunkAllocator::for_disk_range(0..MAX_IO_CHUNK_BYTES as u64).unwrap();
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let page = chunk.slice(0..DIRECT_IO_ALIGNMENT_BYTES as u64);
    assert_eq!(
        handle
            .try_read_recovery_page(page.read_guard())
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn discarded_queued_requests_never_reach_the_ring() {
    let (mut queue, sender) = queue();
    let (request, reply) = request(IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    sender.try_send(request).unwrap();
    drop(reply);
    queue.receive_requests();
    assert!(queue.receiver.is_empty());
    assert!(queue.active.iter().all(Option::is_none));
    assert!(queue.ring.submission().is_empty());
    assert_eq!(sender.capacity(), MAX_IN_FLIGHT_READS);
}

#[test]
fn finished_read_transfers_buffer_ownership() {
    let (mut read, mut reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    let IoBuffers::Read(buffer) = &mut read.buffers else {
        unreachable!()
    };
    buffer.as_mut_slice().fill(0x99);
    let ptr = buffer.as_ref().as_ptr();
    assert!(!read.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    read.send_result(Ok(()));

    let bytes = reply.try_recv().unwrap().unwrap().into_read().into_bytes();
    assert_eq!(bytes.as_ptr(), ptr);
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    let slice = bytes.slice(1..);
    drop(bytes);
    assert_eq!(slice.as_ptr(), ptr.wrapping_add(1));
    assert_eq!(&slice[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES - 1]);
}

#[tokio::test]
async fn reads_into_consecutive_slices_without_reallocating() {
    let (queue, _) = queue();
    let file = queue.file.as_ref().unwrap();
    let lock = queue.directory_lock.as_ref().unwrap();
    let pool = buffer_pool();
    let read = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Read, pool.clone()).unwrap();
    let write = IoQueueHandle::new(file.clone(), lock.clone(), IoOperation::Write, pool).unwrap();
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
    let address = buffer.as_ref().as_ptr() as usize;
    buffer = read.read_into(0, buffer, 0..page).await.unwrap();
    assert_eq!(buffer.as_ref().as_ptr() as usize, address);
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
    let (mut queue, sender) = queue();
    let pool = buffer_pool();
    let (mut read, reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    let buffer = AlignedIoBuffer::new(2 * DIRECT_IO_ALIGNMENT_BYTES, vec![region.read_guard()], &pool).unwrap();
    let address = buffer.as_ref().as_ptr();
    read.buffers = IoBuffers::Read(buffer);
    read.destination = DIRECT_IO_ALIGNMENT_BYTES..2 * DIRECT_IO_ALIGNMENT_BYTES;
    drop(region);
    sender.try_send(read).unwrap();
    drop(sender);
    queue.receive_requests();
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.receive_requests();
    assert_eq!(allocator.available_bytes(), 0);
    assert!(queue.active[0].is_some());
    let other = pool.allocate(DIRECT_IO_ALIGNMENT_BYTES).unwrap();
    assert_ne!(other.as_ref().as_ptr(), address);
    queue.process_requests_until_disconnected().unwrap();
    assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    let reused = pool.allocate(DIRECT_IO_ALIGNMENT_BYTES).unwrap();
    assert_eq!(reused.as_ref().as_ptr(), address);
}

#[test]
fn writes_borrow_aligned_bytes_and_copy_unaligned_bytes() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = buffer_pool();
    let mut source = AlignedIoBuffer::new(2 * page, Vec::new(), &pool).unwrap();
    source.as_mut_slice().fill(0x77);
    let source = source.into_bytes();
    for length in [17, page] {
        let mut dirty = AlignedIoBuffer::new(2 * page, Vec::new(), &pool).unwrap();
        dirty.as_mut_slice().fill(0xff);
        drop(dirty);
        let parts = [(0, source.slice(1..length + 1)), (page, source.slice(..page + 17))];
        let mut buffers = IoBuffers::from_write_parts(4 * page, &parts, None).unwrap();
        assert_eq!(pool.idle_bytes(), 32 * 1024);
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
        assert_eq!(pool.idle_bytes(), 32 * 1024);
    }
}

#[test]
fn completion_state_handles_short_io_and_errors() {
    let (mut read, _reply) = request(IoOperation::Read, 0, 2 * DIRECT_IO_ALIGNMENT_BYTES);
    assert!(read.apply_completion_result(-libc::EINTR).unwrap());
    assert_eq!(read.completed_bytes, 0);
    assert!(read.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert!(!read.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());

    let (mut read, _reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(
        read.apply_completion_result(17).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let (mut write, _reply) = request(IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(
        write.apply_completion_result(0).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    assert_eq!(
        write.apply_completion_result(-libc::ENOSPC).unwrap_err().raw_os_error(),
        Some(libc::ENOSPC)
    );

    let (mut write, _reply) = request(IoOperation::Write, 0, 2 * DIRECT_IO_ALIGNMENT_BYTES);
    assert!(write.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert_eq!(write.completed_bytes, DIRECT_IO_ALIGNMENT_BYTES);
    assert!(!write.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
}

#[test]
#[should_panic(expected = "destination.end <= buffer.as_ref().len()")]
fn rejects_read_into_spare_capacity() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let buffer = AlignedIoBuffer::new(page, Vec::new(), &buffer_pool()).unwrap();
    let (reply, _receive) = oneshot::channel();
    IoRequest::new(0, IoBuffers::Read(buffer), 0..2 * page, reply);
}

#[test]
#[should_panic(expected = "offset.is_multiple_of")]
fn rejects_unaligned_read_offset() {
    request(IoOperation::Read, 1, DIRECT_IO_ALIGNMENT_BYTES);
}

#[test]
#[should_panic(expected = "length.is_multiple_of")]
fn rejects_unaligned_read_length() {
    request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES - 1);
}

#[test]
#[should_panic(expected = "length > 0")]
fn rejects_empty_request() {
    request(IoOperation::Read, 0, 0);
}

#[test]
#[should_panic(expected = "offset.is_multiple_of")]
fn rejects_unaligned_write_offset() {
    request(IoOperation::Write, 1, DIRECT_IO_ALIGNMENT_BYTES);
}

#[test]
#[should_panic(expected = "length.is_multiple_of")]
fn rejects_unaligned_write_length() {
    request(IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES - 1);
}
