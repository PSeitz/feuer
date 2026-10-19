use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::{Arc, Mutex},
    task::Poll,
};

use tokio::sync::{Notify, Semaphore, SemaphorePermit};

use super::MAX_IN_FLIGHT_READS;

/// Limits concurrent payload reads and queues reads waiting to start.
/// At MAX_IN_FLIGHT_READS waiting reads, schedules fallthrough checks for all unchecked reads.
/// Callbacks run in their caller's future, outside the queue lock.
pub(super) struct PayloadReadQueue {
    read_permits: Semaphore,
    // Each notification has a flag indicating whether its fallthrough check was scheduled.
    waiting_reads: Mutex<Vec<(Arc<Notify>, bool)>>,
}

impl PayloadReadQueue {
    pub(super) fn new() -> Self {
        Self {
            read_permits: Semaphore::new(MAX_IN_FLIGHT_READS),
            waiting_reads: Mutex::new(Vec::new()),
        }
    }

    /// Acquires a payload read permit, or returns None if the caller chooses fallthrough.
    pub(super) async fn acquire_permit(
        &self,
        should_fall_through: impl FnOnce() -> bool,
    ) -> Option<SemaphorePermit<'_>> {
        if let Ok(permit) = self.read_permits.try_acquire() {
            return Some(permit);
        }
        let mut waiting_read = self.enqueue();
        let fallthrough_notification = waiting_read.fallthrough_notification.clone();
        let mut notified = pin!(fallthrough_notification.notified());
        let mut acquire = pin!(self.read_permits.acquire());
        let mut should_fall_through = Some(should_fall_through);
        poll_fn(|cx| {
            // Run scheduled checks even if earlier fallthroughs reduced the number of waiting reads.
            if should_fall_through.is_some()
                && notified.as_mut().poll(cx).is_ready()
                && should_fall_through.take().unwrap()()
            {
                return Poll::Ready(None);
            }
            let Poll::Ready(permit) = acquire.as_mut().poll(cx) else {
                return Poll::Pending;
            };
            // A fallthrough check may be scheduled after polling the notification.
            // Remove this read under the queue lock before deciding whether it can start.
            let fallthrough_scheduled = waiting_read.remove();
            if fallthrough_scheduled
                && should_fall_through
                    .take()
                    .is_some_and(|should_fall_through| should_fall_through())
            {
                return Poll::Ready(None);
            }
            Poll::Ready(Some(permit.unwrap()))
        })
        .await
    }

    fn enqueue(&self) -> WaitingPayloadRead<'_> {
        let fallthrough_notification = Arc::new(Notify::new());
        let mut waiting_reads = self.waiting_reads.lock().unwrap();
        waiting_reads.push((fallthrough_notification.clone(), false));
        if waiting_reads.len() >= MAX_IN_FLIGHT_READS {
            for (notification, fallthrough_scheduled) in waiting_reads.iter_mut() {
                if !*fallthrough_scheduled {
                    *fallthrough_scheduled = true;
                    notification.notify_one();
                }
            }
        }
        WaitingPayloadRead {
            queue: self,
            fallthrough_notification,
            is_queued: true,
        }
    }

    #[cfg(test)]
    pub(super) fn hold_all_read_permits(&self) -> SemaphorePermit<'_> {
        self.read_permits.try_acquire_many(MAX_IN_FLIGHT_READS as u32).unwrap()
    }

    /// Holds every read permit and queues one fewer read than triggers fallthrough checks.
    #[cfg(test)]
    pub(super) fn hold_queued_reads(&self) -> impl Sized + '_ {
        let permits = self.hold_all_read_permits();
        let waiting_reads: Vec<_> = (1..MAX_IN_FLIGHT_READS).map(|_| self.enqueue()).collect();
        (permits, waiting_reads)
    }
}

/// A payload read waiting to start. Drop removes it from the queue.
struct WaitingPayloadRead<'a> {
    queue: &'a PayloadReadQueue,
    fallthrough_notification: Arc<Notify>,
    is_queued: bool,
}

impl WaitingPayloadRead<'_> {
    /// Removes this read and returns whether its fallthrough check was scheduled.
    fn remove(&mut self) -> bool {
        if !self.is_queued {
            return false;
        }
        let mut waiting_reads = self.queue.waiting_reads.lock().unwrap();
        let index = waiting_reads
            .iter()
            .position(|(notification, _)| Arc::ptr_eq(notification, &self.fallthrough_notification))
            .unwrap();
        self.is_queued = false;
        waiting_reads.swap_remove(index).1
    }
}

impl Drop for WaitingPayloadRead<'_> {
    fn drop(&mut self) {
        self.remove();
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, pin::Pin};

    use super::*;

    async fn poll<T>(mut future: Pin<&mut impl Future<Output = T>>) -> Poll<T> {
        poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
    }

    #[tokio::test]
    async fn reaching_fallthrough_threshold_checks_all_waiters_once_even_after_pressure_drops() {
        let queue = PayloadReadQueue::new();
        let read_permits = queue.hold_all_read_permits();
        let calls: Vec<_> = (0..MAX_IN_FLIGHT_READS).map(|_| Cell::new(0)).collect();
        let mut reads: Vec<_> = calls
            .iter()
            .enumerate()
            .map(|(index, calls)| {
                Box::pin(queue.acquire_permit(move || {
                    calls.set(calls.get() + 1);
                    index == 0 || index == MAX_IN_FLIGHT_READS - 1
                }))
            })
            .collect();
        let mut last = reads.pop().unwrap();
        for read in &mut reads {
            assert!(poll(read.as_mut()).await.is_pending());
        }
        assert!(calls.iter().all(|calls| calls.get() == 0));
        assert!(matches!(poll(last.as_mut()).await, Poll::Ready(None)));

        // The queue is below the fallthrough threshold, but every earlier waiter still gets its check.
        let mut waiting = Vec::new();
        for (index, mut read) in reads.into_iter().enumerate() {
            match poll(read.as_mut()).await {
                Poll::Ready(None) => assert_eq!(index, 0),
                Poll::Pending => waiting.push(read),
                Poll::Ready(Some(_)) => panic!("all read permits are held"),
            }
        }
        assert!(calls.iter().all(|calls| calls.get() == 1));

        // Reach the threshold again. Only the two new waiters may run their callbacks.
        let new_calls = [Cell::new(0), Cell::new(0)];
        let mut new_reads: Vec<_> = new_calls
            .iter()
            .map(|calls| {
                Box::pin(queue.acquire_permit(move || {
                    calls.set(calls.get() + 1);
                    false
                }))
            })
            .collect();
        for read in &mut new_reads {
            assert!(poll(read.as_mut()).await.is_pending());
        }
        for read in &mut waiting {
            assert!(poll(read.as_mut()).await.is_pending());
        }
        for read in &mut new_reads {
            assert!(poll(read.as_mut()).await.is_pending());
        }
        assert!(calls.iter().all(|calls| calls.get() == 1));
        assert!(new_calls.iter().all(|calls| calls.get() == 1));
        drop(read_permits);
        for read in waiting {
            assert!(read.await.is_some());
        }
        for read in new_reads {
            assert!(read.await.is_some());
        }
        assert!(queue.waiting_reads.lock().unwrap().is_empty());
        assert_eq!(queue.read_permits.available_permits(), MAX_IN_FLIGHT_READS);
    }

    #[tokio::test]
    async fn scheduled_checks_run_before_admission_even_when_permits_become_available() {
        let queue = PayloadReadQueue::new();
        let read_permits = queue.hold_all_read_permits();
        let calls = Cell::new(0);
        let mut reads: Vec<_> = (0..MAX_IN_FLIGHT_READS)
            .map(|_| {
                Box::pin(queue.acquire_permit(|| {
                    calls.set(calls.get() + 1);
                    true
                }))
            })
            .collect();
        let mut last = reads.pop().unwrap();
        for read in &mut reads {
            assert!(poll(read.as_mut()).await.is_pending());
        }
        assert!(matches!(poll(last.as_mut()).await, Poll::Ready(None)));
        drop(read_permits);
        for read in reads {
            assert!(read.await.is_none());
        }
        assert_eq!(calls.get(), MAX_IN_FLIGHT_READS);
        assert!(queue.waiting_reads.lock().unwrap().is_empty());
        assert_eq!(queue.read_permits.available_permits(), MAX_IN_FLIGHT_READS);
    }

    #[tokio::test]
    async fn cancellation_removes_waiters_without_calling_their_callbacks() {
        let queue = PayloadReadQueue::new();
        let read_permits = queue.hold_all_read_permits();
        let mut read = Box::pin(queue.acquire_permit(|| panic!("queue is below the fallthrough threshold")));
        assert!(poll(read.as_mut()).await.is_pending());
        assert_eq!(queue.waiting_reads.lock().unwrap().len(), 1);
        drop(read);
        assert!(queue.waiting_reads.lock().unwrap().is_empty());
        drop(read_permits);
        assert!(
            queue
                .acquire_permit(|| panic!("read can start immediately"))
                .await
                .is_some()
        );
    }
}
