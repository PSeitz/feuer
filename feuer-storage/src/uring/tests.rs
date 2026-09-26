use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

use super::*;

type IoResultReceiver = oneshot::Receiver<io::Result<Bytes>>;

fn request(driver: &IoQueue, operation: IoOperation, offset: u64, length: usize) -> (IoRequest, IoResultReceiver) {
    let (reply, receive) = oneshot::channel();
    let class = usize::from(operation != IoOperation::Read);
    let request_permit = driver.admission.requests[class].clone().try_acquire_owned().unwrap();
    let staging_permit = driver.admission.buffers[class]
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
            (request_permit, staging_permit),
        )
        .unwrap(),
        receive,
    )
}

fn driver() -> IoQueue {
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
    let wake = Arc::new(unsafe { OwnedFd::from_raw_fd(fd) });
    let (_, receiver) = mpsc::sync_channel(MAX_ADMITTED_REQUESTS); // disconnected: run() will drain and exit
    IoQueue {
        admission: Arc::new(IoAdmissionBudgets::new()),
        ring: IoUring::new(MAX_IN_FLIGHT_IO as u32).unwrap(),
        file: Some(file),
        lock: Some(tempfile::tempfile().unwrap()),
        wake,
        receiver,
        pending: VecDeque::new(),
        active: (0..MAX_IN_FLIGHT_IO).map(|_| None).collect(),
    }
}

#[test]
fn fills_qd64_with_simultaneous_reads_and_writes_and_bounds_admission() {
    let mut driver = driver();
    let mut replies = Vec::new();
    for i in 0..MAX_ADMITTED_REQUESTS {
        let operation = if i % 2 == 0 {
            IoOperation::Read
        } else {
            IoOperation::Write
        };
        let (request, reply) = request(
            &driver,
            operation,
            (i * DIRECT_IO_ALIGNMENT_BYTES) as u64,
            DIRECT_IO_ALIGNMENT_BYTES,
        );
        driver.pending.push_back(request);
        replies.push(reply);
    }
    assert!(driver.admission.requests.iter().all(|s| s.available_permits() == 0));
    driver.schedule();
    assert_eq!(driver.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        driver
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Read)
            .count(),
        MAX_IN_FLIGHT_IO / 2
    );
    assert_eq!(
        driver
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Write)
            .count(),
        MAX_IN_FLIGHT_IO / 2
    );
    for (index, request) in driver.active.iter().flatten().enumerate() {
        assert_eq!(request.aligned_offset, (index * DIRECT_IO_ALIGNMENT_BYTES) as u64);
    }
    assert_eq!(driver.pending.len(), MAX_ADMITTED_REQUESTS - MAX_IN_FLIGHT_IO);
    assert_eq!(driver.ring.submission().len(), MAX_IN_FLIGHT_IO);
    driver.run().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
    assert!(driver.active.iter().all(Option::is_none));
    assert!(
        driver
            .admission
            .requests
            .iter()
            .all(|s| s.available_permits() == MAX_IN_FLIGHT_IO)
    );
    assert!(
        driver
            .admission
            .buffers
            .iter()
            .all(|s| s.available_permits() == MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES)
    );
}

// Consume just one real completion without running the scheduler. This lets
// tests inspect the exact next-slot decision even if the whole batch completed.
fn complete_one(driver: &mut IoQueue) {
    driver.ring.submit_and_wait(1).unwrap();
    let cqe = driver.ring.completion().next().unwrap();
    let mut request = driver.active[cqe.user_data() as usize].take().unwrap();
    assert!(!request.complete(cqe.result()).unwrap());
    request.finish(Ok(()));
}

#[test]
fn write_only_fills_the_ring_but_an_older_queued_read_gets_the_next_slot() {
    let mut driver = driver();
    let mut replies = Vec::new();
    for i in 0..MAX_IN_FLIGHT_IO {
        let (request, reply) = request(
            &driver,
            IoOperation::Write,
            (i * DIRECT_IO_ALIGNMENT_BYTES) as u64,
            DIRECT_IO_ALIGNMENT_BYTES,
        );
        driver.pending.push_back(request);
        replies.push(reply);
    }
    driver.schedule();
    assert_eq!(driver.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
    // Read admission is still available even with all write permits occupied.
    let (read, read_reply) = request(
        &driver,
        IoOperation::Read,
        MAX_IO_CHUNK_BYTES as u64 - DIRECT_IO_ALIGNMENT_BYTES as u64,
        DIRECT_IO_ALIGNMENT_BYTES,
    );
    driver.pending.push_back(read);
    replies.push(read_reply);
    driver.schedule();
    assert_eq!(driver.pending.len(), 1); // active writes cannot be preempted
    complete_one(&mut driver);
    let (write, reply) = request(
        &driver,
        IoOperation::Write,
        (MAX_IN_FLIGHT_IO * DIRECT_IO_ALIGNMENT_BYTES) as u64,
        DIRECT_IO_ALIGNMENT_BYTES,
    );
    driver.pending.push_back(write);
    replies.push(reply);
    driver.schedule();
    assert_eq!(
        driver
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Read)
            .count(),
        1
    );
    assert_eq!(driver.pending.len(), 1); // the read was queued before the replacement write
    assert_eq!(driver.pending[0].operation, IoOperation::Write);
    driver.run().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
}

#[test]
fn active_read_does_not_throttle_rmw_writes() {
    let mut driver = driver();
    let (read, reply) = request(&driver, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    driver.pending.push_back(read);
    let mut replies = vec![reply];
    driver.schedule();
    for i in 1..=MAX_IN_FLIGHT_IO {
        let (request, reply) = request(
            &driver,
            IoOperation::Write,
            (i * DIRECT_IO_ALIGNMENT_BYTES + 1) as u64,
            1,
        );
        driver.pending.push_back(request);
        replies.push(reply);
    }
    driver.schedule();
    assert_eq!(driver.active.iter().flatten().count(), MAX_IN_FLIGHT_IO);
    assert_eq!(
        driver
            .active
            .iter()
            .flatten()
            .filter(|r| r.operation == IoOperation::Write)
            .count(),
        MAX_IN_FLIGHT_IO - 1
    );
    assert_eq!(driver.pending.len(), 1);
    driver.run().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
}

#[test]
fn full_write_admission_and_buffers_leave_a_full_read_ring_available() {
    let driver = driver();
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    for _ in 0..MAX_IN_FLIGHT_IO {
        writes.push(request(&driver, IoOperation::Write, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(driver.admission.requests[1].available_permits(), 0);
    assert_eq!(driver.admission.buffers[1].available_permits(), 0);
    for _ in 0..MAX_IN_FLIGHT_IO {
        reads.push(request(&driver, IoOperation::Read, 0, MAX_IO_CHUNK_BYTES));
    }
    assert_eq!(driver.admission.requests[0].available_permits(), 0);
    assert_eq!(driver.admission.buffers[0].available_permits(), 0);
    drop((reads, writes));
    assert!(
        driver
            .admission
            .requests
            .iter()
            .all(|s| s.available_permits() == MAX_IN_FLIGHT_IO)
    );
    assert!(
        driver
            .admission
            .buffers
            .iter()
            .all(|s| s.available_permits() == MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES)
    );
}

#[test]
fn overlap_blocks_only_the_requests_it_must() {
    let mut driver = driver();
    let mut replies = Vec::new();
    for (operation, offset, length) in [
        (IoOperation::Write, 1, 1), // RMW holds the entire first page
        (IoOperation::Read, 2, 1),  // blocked despite disjoint logical bytes
        (
            IoOperation::Write,
            DIRECT_IO_ALIGNMENT_BYTES as u64,
            DIRECT_IO_ALIGNMENT_BYTES,
        ),
        (
            IoOperation::Read,
            2 * DIRECT_IO_ALIGNMENT_BYTES as u64,
            DIRECT_IO_ALIGNMENT_BYTES,
        ),
        (
            IoOperation::Write,
            3 * DIRECT_IO_ALIGNMENT_BYTES as u64,
            DIRECT_IO_ALIGNMENT_BYTES,
        ), // independent writes can pass the blocked read
    ] {
        let (request, reply) = request(&driver, operation, offset, length);
        driver.pending.push_back(request);
        replies.push(reply);
    }
    driver.schedule();
    assert_eq!(driver.active.iter().flatten().count(), 4);
    assert_eq!(driver.pending.len(), 1);
    assert_eq!(driver.pending[0].operation, IoOperation::Read);
    assert_eq!(driver.pending[0].aligned_offset, 0);
    driver.run().unwrap();
    for mut reply in replies {
        reply.try_recv().unwrap().unwrap();
    }
}

#[test]
fn discarded_queued_requests_never_reach_the_ring() {
    let mut driver = driver();
    let (request, reply) = request(&driver, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    driver.pending.push_back(request);
    drop(reply);
    driver.schedule();
    assert!(driver.pending.is_empty());
    assert!(driver.active.iter().all(Option::is_none));
    assert!(driver.ring.submission().is_empty());
    assert!(
        driver
            .admission
            .requests
            .iter()
            .all(|s| s.available_permits() == MAX_IN_FLIGHT_IO)
    );
}

#[test]
fn completion_state_handles_short_io_errors_and_rmw() {
    let driver = driver();
    let (mut read, _reply) = request(&driver, IoOperation::Read, 0, 2 * DIRECT_IO_ALIGNMENT_BYTES);
    assert!(read.complete(-libc::EINTR).unwrap());
    assert_eq!(read.completed_bytes, 0);
    assert!(read.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert!(!read.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());

    let (mut read, _reply) = request(&driver, IoOperation::Read, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(read.complete(17).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    let (mut write, _reply) = request(&driver, IoOperation::Write, 0, DIRECT_IO_ALIGNMENT_BYTES);
    assert_eq!(write.complete(0).unwrap_err().kind(), io::ErrorKind::WriteZero);
    assert_eq!(
        write.complete(-libc::ENOSPC).unwrap_err().raw_os_error(),
        Some(libc::ENOSPC)
    );

    let (mut rmw, _reply) = request(&driver, IoOperation::Write, 1, 3);
    rmw.io_buffer.as_mut_slice().fill(0x55);
    assert!(rmw.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
    assert!(!rmw.reading);
    assert_eq!(rmw.completed_bytes, 0);
    assert_eq!(&rmw.io_buffer.as_mut_slice()[..5], &[0x55, 0x99, 0x99, 0x99, 0x55]);
    assert!(!rmw.complete(DIRECT_IO_ALIGNMENT_BYTES as i32).unwrap());
}

#[test]
fn byte_budget_bounds_rmw_requests_and_releases_on_cancel() {
    let driver = driver();
    let mut requests = Vec::new();
    for _ in 0..MAX_STAGING_BUFFER_BYTES / 2 / (2 * MAX_IO_CHUNK_BYTES) {
        requests.push(request(&driver, IoOperation::Write, 1, MAX_IO_CHUNK_BYTES - 1));
    }
    assert_eq!(driver.admission.buffers[1].available_permits(), 0);
    assert!(driver.admission.buffers[1].clone().try_acquire_owned().is_err());
    assert!(driver.admission.requests[1].available_permits() > 0);
    // Saturated write buffers do not consume any read admission or buffers.
    let _read = request(&driver, IoOperation::Read, 0, MAX_IO_CHUNK_BYTES);
    drop(requests);
    assert_eq!(
        driver.admission.buffers[1].available_permits(),
        MAX_STAGING_BUFFER_BYTES / 2 / DIRECT_IO_ALIGNMENT_BYTES
    );
    assert_eq!(driver.admission.requests[1].available_permits(), MAX_IN_FLIGHT_IO);
}
