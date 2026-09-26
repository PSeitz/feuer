use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use super::*;

type IoResultReceiver = oneshot::Receiver<io::Result<Bytes>>;

fn request(queue: &IoQueue, operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver) {
    let (reply, receive) = oneshot::channel();
    let class = usize::from(operation != IoOperation::Read);
    let request_permit = queue.admission.request_slots[class]
        .clone()
        .try_acquire_owned()
        .unwrap();
    let staging_pages_permit = queue.admission.staging_pages[class]
        .clone()
        .try_acquire_many_owned(staging_pages_for(operation, offset, length))
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
    let (_, receiver) = mpsc::sync_channel(MAX_ADMITTED_REQUESTS); // disconnected: run() will drain and exit
    IoQueue {
        admission: Arc::new(IoAdmissionBudgets::new()),
        ring: IoUring::new(MAX_IN_FLIGHT_IO as u32).unwrap(),
        file: Some(file),
        directory_lock: Some(tempfile::tempfile().unwrap()),
        wake_fd,
        receiver,
        pending: VecDeque::new(),
        active: (0..MAX_IN_FLIGHT_IO).map(|_| None).collect(),
    }
}

#[test]
fn fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission() {
    let mut queue = queue();
    let mut replies = Vec::new();
    for i in 0..MAX_ADMITTED_REQUESTS {
        let operation = if i % 2 == 0 {
            IoOperation::Read
        } else {
            IoOperation::Write
        };
        let (request, reply) = request(
            &queue,
            operation,
            (i * DIRECT_IO_ALIGNMENT_BYTES) as u64,
            DIRECT_IO_ALIGNMENT_BYTES,
        );
        queue.pending.push_back(request);
        replies.push(reply);
    }
    assert!(queue.admission.request_slots.iter().all(|s| s.available_permits() == 0));
    queue.schedule();
    assert_eq!(queue.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        queue
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Read)
            .count(),
        MAX_IN_FLIGHT_IO / 2
    );
    assert_eq!(
        queue
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Write)
            .count(),
        MAX_IN_FLIGHT_IO / 2
    );
    for (index, request) in queue.active.iter().flatten().enumerate() {
        assert_eq!(request.aligned_offset, (index * DIRECT_IO_ALIGNMENT_BYTES) as u64);
    }
    assert_eq!(queue.pending.len(), MAX_ADMITTED_REQUESTS - MAX_IN_FLIGHT_IO);
    assert_eq!(queue.ring.submission().len(), MAX_IN_FLIGHT_IO);
    queue.run().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
    assert!(queue.active.iter().all(Option::is_none));
    assert!(
        queue
            .admission
            .request_slots
            .iter()
            .all(|s| s.available_permits() == MAX_IN_FLIGHT_IO)
    );
    assert!(
        queue
            .admission
            .staging_pages
            .iter()
            .all(|s| s.available_permits() == MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES)
    );
}

// Consume just one real completion without running the scheduler. This lets
// tests inspect the exact next-slot decision even if the whole batch completed.
fn complete_one(queue: &mut IoQueue) {
    queue.ring.submit_and_wait(1).unwrap();
    let cqe = queue.ring.completion().next().unwrap();
    let mut request = queue.active[cqe.user_data() as usize].take().unwrap();
    assert!(!request.complete(cqe.result()).unwrap());
    request.finish(Ok(()));
}

#[test]
fn write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot() {
    let mut queue = queue();
    let mut replies = Vec::new();
    for i in 0..MAX_IN_FLIGHT_IO {
        let (request, reply) = request(
            &queue,
            IoOperation::Write,
            (i * DIRECT_IO_ALIGNMENT_BYTES) as u64,
            DIRECT_IO_ALIGNMENT_BYTES,
        );
        queue.pending.push_back(request);
        replies.push(reply);
    }
    queue.schedule();
    assert_eq!(queue.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
    // Read admission is still available even with all write permits occupied.
    let (read, read_reply) = request(
        &queue,
        IoOperation::Read,
        MAX_IO_CHUNK_BYTES as u64 - DIRECT_IO_ALIGNMENT_BYTES as u64,
        DIRECT_IO_ALIGNMENT_BYTES,
    );
    queue.pending.push_back(read);
    replies.push(read_reply);
    queue.schedule();
    assert_eq!(queue.pending.len(), 1); // active writes cannot be preempted
    complete_one(&mut queue);
    let (write, reply) = request(
        &queue,
        IoOperation::Write,
        (MAX_IN_FLIGHT_IO * DIRECT_IO_ALIGNMENT_BYTES) as u64,
        DIRECT_IO_ALIGNMENT_BYTES,
    );
    queue.pending.push_back(write);
    replies.push(reply);
    queue.schedule();
    assert_eq!(
        queue
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Read)
            .count(),
        1
    );
    assert_eq!(queue.pending.len(), 1); // the read was queued before the replacement write
    assert_eq!(queue.pending[0].operation, IoOperation::Write);
    queue.run().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
}

#[test]
fn active_read_does_not_throttle_rmw_writes() {
    let mut queue = queue();
    let (read, reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    queue.pending.push_back(read);
    let mut replies = vec![reply];
    queue.schedule();
    for i in 1..=MAX_IN_FLIGHT_IO {
        let (request, reply) = request(
            &queue,
            IoOperation::Write,
            (i * DIRECT_IO_ALIGNMENT_BYTES + 1) as u64,
            1,
        );
        queue.pending.push_back(request);
        replies.push(reply);
    }
    queue.schedule();
    assert_eq!(queue.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        queue
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Write)
            .count(),
        MAX_IN_FLIGHT_IO - 1
    );
    assert_eq!(queue.pending.len(), 1);
    queue.run().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
}

#[test]
fn full_write_admission_and_buffers_leave_a_full_read_ring_available() {
    let queue = queue();
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_IO {
        writes.push(request(&queue, IoOperation::Write, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(queue.admission.request_slots[1].available_permits(), 0);
    assert_eq!(queue.admission.staging_pages[1].available_permits(), 0);
    for _ in 0..MAX_IN_FLIGHT_IO {
        reads.push(request(&queue, IoOperation::Read, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(queue.admission.request_slots[0].available_permits(), 0);
    assert_eq!(queue.admission.staging_pages[0].available_permits(), 0);
    drop((reads, writes));
    assert!(
        queue
            .admission
            .request_slots
            .iter()
            .all(|s| s.available_permits() == MAX_IN_FLIGHT_IO)
    );
    assert!(
        queue
            .admission
            .staging_pages
            .iter()
            .all(|s| s.available_permits() == MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES)
    );
}

#[test]
fn canceled_submitted_rmw_retains_resources_and_still_writes() {
    let mut queue = queue();
    let (write, reply) = request(&queue, IoOperation::Write, 1, 1);
    queue.pending.push_back(write);
    queue.schedule();
    // The RMW read has reached the kernel, but its completion is not yet processed.
    queue.ring.submit_and_wait(1).unwrap();
    drop(reply);
    queue.schedule();
    assert!(queue.active[0].is_some());
    assert_eq!(
        queue.admission.request_slots[1].available_permits(),
        MAX_IN_FLIGHT_IO - 1
    );
    assert_eq!(
        queue.admission.staging_pages[1].available_permits(),
        MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES - 2
    );
    queue.run().unwrap();
    assert!(queue.active.iter().all(Option::is_none));
    assert_eq!(queue.admission.request_slots[1].available_permits(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        queue.admission.staging_pages[1].available_permits(),
        MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES
    );

    // Only read/reuse the region after completion, not after dropping the receiver.
    let (read, mut reply) = request(&queue, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    queue.pending.push_back(read);
    queue.run().unwrap();
    assert_eq!(&reply.try_recv().unwrap().unwrap()[..3], &[0, 0x99, 0]);
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
    assert!(
        queue
            .admission
            .request_slots
            .iter()
            .all(|s| s.available_permits() == MAX_IN_FLIGHT_IO)
    );
}

#[test]
fn completion_state_handles_short_io_errors_and_rmw() {
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

    let (mut rmw, _reply) = request(&queue, IoOperation::Write, 1, 3);
    assert!(rmw.is_reading());
    assert_eq!(rmw.aligned_length(), DIRECT_IO_ALIGNMENT_BYTES);
    rmw.io_buffer.as_mut_slice().fill(0x55);
    assert!(rmw.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert!(!rmw.is_reading());
    assert_eq!(rmw.completed_bytes, 0);
    assert_eq!(&rmw.io_buffer.as_mut_slice()[..5], &[0x55, 0x99, 0x99, 0x99, 0x55]);
    assert!(!rmw.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
}

#[test]
fn byte_budget_bounds_rmw_requests_and_releases_on_cancel() {
    let queue = queue();
    let mut requests = Vec::new();
    for _ in 0..MAX_STAGING_BUFFER_BYTES / 2 / (2 * MAX_IO_CHUNK_BYTES) {
        requests.push(request(&queue, IoOperation::Write, 1, MAX_IO_CHUNK_BYTES - 1));
    }
    assert_eq!(queue.admission.staging_pages[1].available_permits(), 0);
    assert!(queue.admission.staging_pages[1].clone().try_acquire_owned().is_err());
    assert!(queue.admission.request_slots[1].available_permits() > 0);
    // Saturated write buffers do not consume any read admission or buffers.
    let _read = request(&queue, IoOperation::Read, 0, MAX_IO_CHUNK_BYTES);
    drop(requests);
    assert_eq!(
        queue.admission.staging_pages[1].available_permits(),
        MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES
    );
    assert_eq!(queue.admission.request_slots[1].available_permits(), MAX_IN_FLIGHT_IO);
}
