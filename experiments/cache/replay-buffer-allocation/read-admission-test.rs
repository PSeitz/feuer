
#[tokio::test]
async fn split_read_admission_retains_more_buffers_than_io_slots() {
    use std::{collections::HashMap, sync::Weak};

    let (mut queue, sender, files) = queue();
    let pool = feuer_memory::MemoryCache::new(16 * 1024 * 1024).buffer_pool();
    let read = ReadQueue {
        handle: Arc::new(IoQueueHandle {
            sender: Some(sender),
            wake_fd: queue.wake_fd.clone(),
            thread: None,
        }),
        files,
        buffer_pool: pool.clone(),
    };
    let count = 4 * MAX_IN_FLIGHT_READS;
    let length = 3 * MAX_IO_REQUEST_BYTES;
    let mut futures: Vec<_> = (0..count).map(|_| Some(Box::pin(read.read(0, length)))).collect();
    let mut owners: HashMap<usize, Weak<ReadBuffer>> = HashMap::new();
    let mut peak = 0;
    let mut completed = 0;
    let mut context = Context::from_waker(std::task::Waker::noop());
    for _ in 0..count * 4 + 10 {
        tokio::task::yield_now().await;
        for future in &mut futures {
            if let Some(pending) = future
                && let std::task::Poll::Ready(result) = pending.as_mut().poll(&mut context)
            {
                drop(result.unwrap());
                *future = None;
                completed += 1;
            }
        }
        if let Ok(request) = queue.receiver.try_recv() {
            let IoBuffers::Read(buffer) = &request.buffers else { unreachable!() };
            owners.insert(buffer.ptr as usize, Arc::downgrade(buffer));
            // Simulate immediate completion without retaining requests or issuing disk I/O.
            request.send_result(Ok(()));
        }
        let live = owners.values().filter(|owner| owner.strong_count() > 0).count();
        peak = peak.max(live);
        assert_eq!(pool.used_bytes(), 0);
        assert_eq!(pool.idle_bytes(), 0);
        if completed == count {
            break;
        }
    }
    println!("read count={count}, piece channel slots={MAX_IN_FLIGHT_READS}, peak live read buffers={peak}, bytes={}, cache charge={}, idle bytes={}", peak * length, pool.used_bytes(), pool.idle_bytes());
    assert_eq!(completed, count);
    assert!(peak > 2 * MAX_IN_FLIGHT_READS);
    assert!(owners.values().all(|owner| owner.strong_count() == 0));
}
