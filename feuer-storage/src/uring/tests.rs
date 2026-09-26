use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use super::*;

type IoResultReceiver = oneshot::Receiver<io::Result<Bytes>>;

fn request(queue: &IoQueue, operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver) {
    let (reply, receive) = oneshot::channel();
    let request_permit = queue.admission.request_slots.clone().try_acquire_owned().unwrap();
    let staging_pages_permit = queue
        .admission
        .staging_pages
        .clone()
        .try_acquire_many_owned((length / DIRECT_IO_ALIGNMENT_BYTES) as u32)
        .unwrap();
    let payload = if operation == IoOperation::Write {
        vec![0x99; length]
    } else {
        vec![]
    };
    (
        IoRequest::new(
            operation,
            offset,
            length,
            &payload,
            reply,
            (request_permit, staging_pages_permit),
        )
        .unwrap(),
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
        admission: Arc::new(IoAdmissionBudgets::new()),
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
                queue.admission.staging_pages.available_permits(),
                MAX_STAGING_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
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
    assert_eq!(write_queue.admission.staging_pages.available_permits(), 0);
    for _ in 0..MAX_IN_FLIGHT_IO {
        reads.push(request(&read_queue, IoOperation::Read, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(read_queue.admission.request_slots.available_permits(), 0);
    assert_eq!(read_queue.admission.staging_pages.available_permits(), 0);
    drop((reads, writes));
    for queue in [&read_queue, &write_queue] {
        assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
        assert_eq!(
            queue.admission.staging_pages.available_permits(),
            MAX_STAGING_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
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
    let _requests = write_queue
        .admission
        .request_slots
        .clone()
        .acquire_many_owned(MAX_IN_FLIGHT_IO as u32)
        .await
        .unwrap();
    let _pages = write_queue
        .admission
        .staging_pages
        .clone()
        .acquire_many_owned((MAX_STAGING_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES) as u32)
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
        queue.admission.staging_pages.available_permits(),
        MAX_STAGING_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES - 1
    );
    queue.run().unwrap();
    assert!(queue.active.iter().all(Option::is_none));
    assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        queue.admission.staging_pages.available_permits(),
        MAX_STAGING_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
    );

    // Only read/reuse the region after completion, not after dropping the receiver.
    let mut read_queue = self::queue();
    read_queue.file = queue.file.clone();
    read_queue.directory_lock = queue.directory_lock.clone();
    let (read, mut reply) = request(&read_queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    read_queue.pending.push_back(read);
    read_queue.run().unwrap();
    assert_eq!(
        &reply.try_recv().unwrap().unwrap()[..],
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

    let bytes = reply.try_recv().unwrap().unwrap();
    assert_eq!(bytes.as_ptr(), ptr);
    assert_eq!(&bytes[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES]);
    assert_eq!(queue.admission.request_slots.available_permits(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        queue.admission.staging_pages.available_permits(),
        MAX_STAGING_BUFFER_BYTES / DIRECT_IO_ALIGNMENT_BYTES
    );

    let slice = bytes.slice(1..);
    drop(bytes);
    drop(queue);
    assert_eq!(slice.as_ptr(), ptr.wrapping_add(1));
    assert_eq!(&slice[..], &[0x99; DIRECT_IO_ALIGNMENT_BYTES - 1]);
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
