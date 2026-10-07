use std::{fs::OpenOptions, future::Future, os::unix::fs::OpenOptionsExt, task::Context};

use super::*;

type IoResultReceiver = oneshot::Receiver<io::Result<Option<AlignedBuffer>>>;

fn request(operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver) {
    let (reply, receive) = oneshot::channel();
    let buffers = if operation == IoOperation::Write {
        let mut buffer = AlignedBuffer::allocate_zeroed(length).unwrap();
        buffer.as_mut_slice().fill(0x99);
        IoBuffers::from_write_bytes(length, buffer.into_bytes()).unwrap()
    } else {
        IoBuffers::Read(buffer_pool().allocate(length).unwrap())
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
    file.set_len(MAX_IO_REQUEST_BYTES as u64).unwrap();
    // SAFETY: eventfd returns a fresh descriptor, checked before assuming ownership.
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(fd >= 0);
    // SAFETY: fd was just created and has no other owner.
    let wake_fd = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
    let (sender, receiver) = mpsc::channel(MAX_IN_FLIGHT_READS);
    (
        IoQueue {
            ring: IoUring::new(MAX_IN_FLIGHT_READS as u32).unwrap(),
            files: Arc::new(DataFileAndDirectoryLock {
                file,
                _directory_lock: Some(tempfile::tempfile().unwrap()),
            }),
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
fn pooled_read_buffers_do_not_retain_disk_reservations() {
    use crate::allocation::{CHUNK_BYTES, DiskChunkAllocator};

    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let pool = buffer_pool();
    let allocator = DiskChunkAllocator::for_disk_range(0..CHUNK_BYTES);
    for return_bytes in [false, true] {
        let region = allocator.reserve_chunks(1).unwrap();
        let buffer = pool.allocate(page).unwrap();
        drop(region);
        assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
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
        other_queue.files = full_queue.files.clone();
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
            let buffer = reply.try_recv().unwrap().unwrap();
            assert_eq!(buffer.is_some(), other_operation == IoOperation::Read);
        }
        assert_eq!(full_queue.active.iter().flatten().count(), MAX_IN_FLIGHT_READS);
        full_queue.process_requests_until_disconnected().unwrap();
        for mut reply in full_replies {
            let buffer = reply.try_recv().unwrap().unwrap();
            assert_eq!(buffer.is_some(), operation == IoOperation::Read);
        }
        for queue in [&full_queue, &other_queue] {
            assert!(queue.active.iter().all(Option::is_none));
        }
    }
}

#[test]
fn resource_pressure_retries_pending_submissions() {
    for operation in [IoOperation::Read, IoOperation::Write] {
        let (mut queue, sender) = queue();
        let mut replies = Vec::new();
        for page in 0..3 {
            let (request, reply) = request(
                operation,
                (page * DIRECT_IO_ALIGNMENT_BYTES) as u64,
                DIRECT_IO_ALIGNMENT_BYTES,
            );
            sender.try_send(request).unwrap();
            replies.push(reply);
        }
        drop(sender);
        let mut submissions = 0;
        queue
            .process_requests_with_submit(|ring| {
                submissions += 1;
                match submissions {
                    1 | 4 => {
                        assert!(!ring.submission().is_empty());
                        Err(io::Error::from_raw_os_error(libc::EAGAIN))
                    }
                    2 => Err(io::Error::from_raw_os_error(libc::EINTR)),
                    // Accept only one request so EAGAIN also occurs with a partially consumed SQ.
                    // SAFETY: active slots own all SQE pointers; this submits one ordinary request.
                    3 => unsafe { ring.submitter().enter::<libc::sigset_t>(1, 0, 0, None) },
                    _ => ring.submit(),
                }
            })
            .unwrap();
        assert!(submissions >= 5);
        assert!(queue.active.iter().all(Option::is_none));
        assert!(queue.ring.submission().is_empty());
        for mut reply in replies {
            let buffer = reply.try_recv().unwrap().unwrap();
            assert_eq!(buffer.is_some(), operation == IoOperation::Read);
            if let Some(buffer) = buffer {
                assert_eq!(buffer.as_ref(), &[0; DIRECT_IO_ALIGNMENT_BYTES]);
            }
        }
    }
}

#[tokio::test]
async fn active_request_limit_keeps_channel_full_until_completion() {
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
            let request_index = completion.user_data() as usize;
            let mut request = queue.active[request_index].take().unwrap();
            request.apply_completion_result(completion.result()).unwrap();
            assert!(request.buffer_range.is_empty());
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
    let pool = buffer_pool();
    let read_queue = ReadQueue::new(queue.files.clone(), pool).unwrap();
    let write_queue = WriteQueue::new(queue.files.clone()).unwrap();
    write_queue
        .write_padded(
            0,
            DIRECT_IO_ALIGNMENT_BYTES,
            &Bytes::from(vec![0x99; DIRECT_IO_ALIGNMENT_BYTES]),
        )
        .await
        .unwrap();
    assert_eq!(read_queue.handle.sender.as_ref().unwrap().capacity(), 64);
    assert_eq!(write_queue.handle.sender.as_ref().unwrap().capacity(), 8);
    let reservations = write_queue
        .handle
        .sender
        .as_ref()
        .unwrap()
        .reserve_many(MAX_IN_FLIGHT_WRITES)
        .await
        .unwrap();
    assert!(matches!(
        write_queue.handle.sender.as_ref().unwrap().try_reserve(),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    let (bytes, _) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read_queue.read(0, DIRECT_IO_ALIGNMENT_BYTES),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    drop(reservations);

    // The read queue must also keep the shared file and directory lock alive.
    let files = Arc::downgrade(&queue.files);
    drop(queue);
    drop(write_queue);
    assert!(files.upgrade().is_some());
    let (bytes, _) = read_queue.read(0, DIRECT_IO_ALIGNMENT_BYTES).await.unwrap();
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    drop(read_queue);
    assert!(files.upgrade().is_none());
}

#[test]
fn queue_failure_retains_write_descriptors_and_files() {
    let (mut queue, sender) = queue();
    let pool = buffer_pool();
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let mut buffer = pool.allocate(page + 17).unwrap();
    buffer.as_mut_slice().fill(0x77);
    let payload_address = buffer.as_ref().as_ptr();
    let buffers = IoBuffers::from_write_bytes(2 * page, buffer.into_bytes()).unwrap();
    let (reply, mut receive) = oneshot::channel();
    sender.try_send(IoRequest::new(0, buffers, 0..2 * page, reply)).unwrap();
    drop(sender);
    queue.receive_requests();
    let slots_address = queue.active.as_ptr();
    let IoBuffers::Write { vectors, .. } = &queue.active[0].as_ref().unwrap().buffers else {
        unreachable!()
    };
    let descriptors_address = vectors.as_ptr();
    let files = Arc::downgrade(&queue.files);

    let error = queue
        .process_requests_with_submit(|ring| {
            assert_eq!(ring.submit()?, 1);
            // Fail immediately after acceptance, without waiting for or processing a CQE.
            Err(io::Error::from_raw_os_error(libc::EIO))
        })
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EIO));
    assert!(matches!(receive.try_recv(), Err(oneshot::error::TryRecvError::Empty)));

    let retained = queue.retain_active_requests();
    assert!(queue.active.is_empty());
    drop(queue);
    assert_eq!(
        receive.try_recv().unwrap().err().unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(files.upgrade().unwrap()._directory_lock.is_some());
    assert_eq!(retained.as_ptr(), slots_address);
    let IoBuffers::Write { bytes, vectors } = &retained[0].as_ref().unwrap().buffers else {
        unreachable!()
    };
    assert_eq!(vectors.as_ptr(), descriptors_address);
    assert_eq!(bytes[0].as_ptr(), payload_address);
    for (bytes, vector) in bytes.iter().zip(vectors) {
        assert_eq!(vector.iov_base.cast_const().cast::<u8>(), bytes.as_ptr());
        assert_eq!(vector.iov_len, page);
    }
    assert_eq!(pool.idle_bytes(), 0);
}

#[test]
fn canceled_submitted_write_retains_buffers_on_queue_failure() {
    let (mut queue, sender) = queue();
    let allocator = crate::allocation::DiskChunkAllocator::for_disk_range(0..MAX_IO_REQUEST_BYTES as u64);
    let region = allocator.reserve_chunks(1).unwrap();
    let pool = buffer_pool();
    let mut buffer = pool.allocate(DIRECT_IO_ALIGNMENT_BYTES).unwrap();
    buffer.as_mut_slice().fill(0x99);
    let address = buffer.as_ref().as_ptr();
    let (mut write, reply) = request(IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    write.buffers = IoBuffers::from_write_bytes(DIRECT_IO_ALIGNMENT_BYTES, buffer.into_bytes()).unwrap();
    sender.try_send(write).unwrap();
    drop(sender);
    queue.receive_requests();
    // The write has reached the kernel, but its completion is not yet processed.
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    drop(region);
    queue.receive_requests();
    let reused = allocator.reserve_chunks(1).unwrap();
    assert_eq!(reused.disk_byte_range().start, 0);
    assert!(queue.active[0].is_some());
    let other = pool.allocate(DIRECT_IO_ALIGNMENT_BYTES).unwrap();
    assert_ne!(other.as_ref().as_ptr(), address);
    let files = queue.files.clone();
    drop(queue); // Failure retains the active slot, even after caller cancellation.
    let buffer = pool.allocate(DIRECT_IO_ALIGNMENT_BYTES).unwrap();
    assert_ne!(buffer.as_ref().as_ptr(), address);

    // The abandoned write still reached the reused disk range.
    let (mut read_queue, sender) = self::queue();
    read_queue.files = files;
    let (read, mut reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    sender.try_send(read).unwrap();
    drop(sender);
    read_queue.process_requests_until_disconnected().unwrap();
    assert_eq!(
        reply.try_recv().unwrap().unwrap().unwrap().as_ref(),
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
    assert_eq!(
        [0, 1].map(|index| queue.active[index].as_ref().unwrap().disk_end),
        [DIRECT_IO_ALIGNMENT_BYTES as u64, 2 * DIRECT_IO_ALIGNMENT_BYTES as u64]
    );
    queue.process_requests_until_disconnected().unwrap();
    first_reply.try_recv().unwrap().unwrap();
    second_reply.try_recv().unwrap().unwrap();
}

#[tokio::test]
async fn reads_wait_for_channel_capacity_without_taking_idle_buffers() {
    let (queue, _) = queue();
    let pool = buffer_pool();
    let handle = ReadQueue::new(queue.files.clone(), pool.clone()).unwrap();
    let reservations = handle
        .handle
        .sender
        .as_ref()
        .unwrap()
        .reserve_many(MAX_IN_FLIGHT_READS)
        .await
        .unwrap();
    for length in [
        DIRECT_IO_ALIGNMENT_BYTES,
        MAX_IO_REQUEST_BYTES,
        3 * MAX_IO_REQUEST_BYTES,
    ] {
        // Neither payload nor metadata reads take an idle buffer before admission.
        drop(handle.allocate_buffer(length).unwrap());
        let idle_bytes = pool.idle_bytes();
        assert!(idle_bytes > 0);
        let read = handle.read(0, length);
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
    assert_eq!(handle.handle.sender.as_ref().unwrap().capacity(), MAX_IN_FLIGHT_READS);
    assert_eq!(
        handle.read(0, MAX_IO_REQUEST_BYTES).await.unwrap().0.len(),
        MAX_IO_REQUEST_BYTES
    );
}

#[tokio::test]
async fn queue_exit_releases_admission_waiters_and_queued_requests() {
    let (queue, sender) = queue();
    let handle = ReadQueue {
        handle: IoQueueHandle {
            sender: Some(sender),
            wake_fd: queue.wake_fd.clone(),
            thread: None,
        },
        buffer_pool: buffer_pool(),
    };
    let mut replies = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_READS {
        let (request, reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
        handle.handle.sender.as_ref().unwrap().try_send(request).unwrap();
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
    assert_eq!(
        handle.read(0, MAX_IO_REQUEST_BYTES).await.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn canceled_queued_requests_drain_on_shutdown_without_submission() {
    let (mut queue, sender) = queue();
    let (request, reply) = request(IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    sender.try_send(request).unwrap();
    drop(reply);
    drop(sender);
    queue.process_requests_until_disconnected().unwrap();
    assert!(queue.receiver.is_empty());
    assert!(queue.active.iter().all(Option::is_none));
    assert!(queue.ring.submission().is_empty());
}

#[test]
fn finished_read_transfers_buffer_ownership() {
    let (mut read, mut reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    let IoBuffers::Read(buffer) = &mut read.buffers else {
        unreachable!()
    };
    buffer.as_mut_slice().fill(0x99);
    let ptr = buffer.as_ref().as_ptr();
    read.apply_completion_result(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap();
    assert!(read.buffer_range.is_empty());
    read.send_result(Ok(()));

    let bytes = reply.try_recv().unwrap().unwrap().unwrap().into_bytes();
    assert_eq!(bytes.as_ptr(), ptr);
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    let slice = bytes.slice(1..);
    drop(bytes);
    assert_eq!(slice.as_ptr(), ptr.wrapping_add(1));
    assert_eq!(&slice[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES - 1]);
}

#[test]
fn error_completion_releases_the_read_buffer() {
    let (mut queue, sender) = queue();
    let pool = buffer_pool();
    let mut buffer = pool.allocate(DIRECT_IO_ALIGNMENT_BYTES).unwrap();
    buffer.as_mut_slice().fill(0x99);
    let address = buffer.as_ref().as_ptr();
    let (mut read, mut reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    read.buffers = IoBuffers::Read(buffer);
    let temporary = tempfile::NamedTempFile::new().unwrap();
    // Reading a write-only file produces -EBADF in the CQE, not a submission error.
    Arc::get_mut(&mut queue.files).unwrap().file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_DIRECT)
        .open(temporary.path())
        .unwrap();
    sender.try_send(read).unwrap();
    drop(sender);
    queue.process_requests_until_disconnected().unwrap();
    assert_eq!(
        reply.try_recv().unwrap().err().unwrap().raw_os_error(),
        Some(libc::EBADF)
    );
    assert!(queue.active.iter().all(Option::is_none));
    assert_eq!(
        pool.allocate(DIRECT_IO_ALIGNMENT_BYTES).unwrap().as_ref().as_ptr(),
        address
    );
    let files = Arc::downgrade(&queue.files);
    drop(queue);
    assert!(files.upgrade().is_none());
}

#[tokio::test]
async fn multi_request_reads_reuse_one_allocation_after_a_padded_write() {
    let (queue, _) = queue();
    let pool = buffer_pool();
    let read = ReadQueue::new(queue.files.clone(), pool.clone()).unwrap();
    let write = WriteQueue::new(queue.files.clone()).unwrap();
    let length = MAX_IO_REQUEST_BYTES;
    let mut payload = vec![0x99; length + 17];
    payload[length..].fill(0x77);
    write.write_padded(0, 3 * length, &Bytes::from(payload)).await.unwrap();
    let mut buffer = read.allocate_buffer(3 * length).unwrap();
    assert_eq!(buffer.capacity(), 3 * length);
    buffer.as_mut_slice().fill(0x55);
    let address = buffer.as_ref().as_ptr() as usize;
    drop(buffer);
    let (bytes, capacity) = read.read(0, 3 * length).await.unwrap();
    assert_eq!(capacity, 3 * length);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(bytes.as_ptr() as usize, address);
    assert_eq!(&bytes[..length], &vec![0x99; length]);
    assert_eq!(&bytes[length..length + 17], &[0x77; 17]);
    assert!(bytes[length + 17..].iter().all(|&byte| byte == 0));
}

#[test]
fn canceled_read_retains_destination_but_not_disk_reservation_until_completion() {
    use crate::allocation::{CHUNK_BYTES, DiskChunkAllocator};
    let allocator = DiskChunkAllocator::for_disk_range(0..CHUNK_BYTES);
    let region = allocator.reserve_chunks(1).unwrap();
    let (mut queue, sender) = queue();
    let pool = buffer_pool();
    let (mut read, reply) = request(IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    let buffer = pool.allocate(2 * DIRECT_IO_ALIGNMENT_BYTES).unwrap();
    let address = buffer.as_ref().as_ptr();
    read.buffers = IoBuffers::Read(buffer);
    read.buffer_range = DIRECT_IO_ALIGNMENT_BYTES..2 * DIRECT_IO_ALIGNMENT_BYTES;
    drop(region);
    sender.try_send(read).unwrap();
    drop(sender);
    queue.receive_requests();
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.receive_requests();
    assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
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
    let mut source = pool.allocate(2 * page).unwrap();
    source.as_mut_slice().fill(0x77);
    let source = source.into_bytes();
    for (start, length, borrowed_length) in [(0, page + 17, page), (1, page + 17, 0), (0, 2 * page, 2 * page)] {
        let mut dirty = pool.allocate(2 * page).unwrap();
        dirty.as_mut_slice().fill(0xff);
        drop(dirty);
        let payload = source.slice(start..start + length);
        let mut buffers = IoBuffers::from_write_bytes(2 * page, payload.clone()).unwrap();
        assert_eq!(pool.idle_bytes(), 32 * 1024);
        buffers.submission_entry(types::Fd(-1), page as u64, page..2 * page);
        let IoBuffers::Write { bytes, vectors, .. } = buffers else {
            unreachable!()
        };
        assert_eq!(bytes[0].len(), borrowed_length);
        if borrowed_length > 0 {
            assert_eq!(bytes[0].as_ptr(), payload.as_ptr());
        }
        let mut expected = vec![0; 2 * page];
        expected[..length].copy_from_slice(&payload);
        assert_eq!(bytes.concat(), expected);
        let (last, vector) = bytes
            .iter()
            .zip(vectors.iter())
            .find(|(_, vector)| vector.iov_len > 0)
            .unwrap();
        assert_eq!(vector.iov_len, page);
        assert_eq!(vectors.iter().map(|vector| vector.iov_len).sum::<usize>(), page);
        assert_eq!(
            vector.iov_base.cast_const().cast::<u8>(),
            last[last.len() - page..].as_ptr()
        );
        drop(bytes);
        assert_eq!(pool.idle_bytes(), 32 * 1024);
    }
}

#[test]
fn completion_state_handles_short_io_and_errors() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let (mut read, _reply) = request(IoOperation::Read, page as u64, 3 * page);
    read.buffer_range.start = page;
    read.apply_completion_result(-libc::EINTR).unwrap();
    assert_eq!((read.disk_end, read.buffer_range.start), (4 * page as u64, page));
    read.apply_completion_result(page as i32).unwrap();
    assert_eq!(read.disk_end, 4 * page as u64);
    assert_eq!(read.buffer_range, 2 * page..3 * page);
    read.apply_completion_result(page as i32).unwrap();
    assert!(read.buffer_range.is_empty());

    let (mut read, _reply) = request(IoOperation::Read, 0, page);
    assert_eq!(
        read.apply_completion_result(17).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let (mut write, _reply) = request(IoOperation::Write, 0, page);
    assert_eq!(
        write.apply_completion_result(0).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    assert_eq!(
        write.apply_completion_result(-libc::ENOSPC).unwrap_err().raw_os_error(),
        Some(libc::ENOSPC)
    );

    let (mut write, _reply) = request(IoOperation::Write, 0, 2 * page);
    write.apply_completion_result(page as i32).unwrap();
    assert_eq!((write.disk_end, write.buffer_range.start), (2 * page as u64, page));
    write.apply_completion_result(page as i32).unwrap();
    assert!(write.buffer_range.is_empty());
}

#[test]
#[should_panic(expected = "buffer_range.end <= buffer.as_ref().len()")]
fn rejects_read_into_spare_capacity() {
    let page = DIRECT_IO_ALIGNMENT_BYTES;
    let buffer = buffer_pool().allocate(page).unwrap();
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
