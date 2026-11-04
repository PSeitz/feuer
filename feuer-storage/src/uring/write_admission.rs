use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const WRITE_BYTES_PER_SECOND_WITH_WAITING_READS: usize = 1024 * 1024 * 1024;
const WRITE_LIMIT_AFTER_READ_WAIT: Duration = Duration::from_millis(100);

/// Reads waiting for channel capacity and when the last wait ended.
#[derive(Default)]
pub(super) struct ReadQueueWaiters {
    pub(super) waiting_reads: usize,
    last_wait_ended_at: Option<Instant>,
}

impl ReadQueueWaiters {
    pub(super) fn remove_waiting_read(&mut self, now: Instant) {
        self.waiting_reads -= 1;
        if self.waiting_reads == 0 {
            self.last_wait_ended_at = Some(now);
        }
    }
}

/// Paces new writes by their byte length while reads wait for channel capacity.
/// Admits a write only after active writes finish, retaining the limit for 100 ms after the last read wait.
/// Active writes, including retries, finish without further admission checks.
pub(super) struct WriteRateLimit {
    read_queue_waiters: Arc<Mutex<ReadQueueWaiters>>,
    next_write_at: Instant,
}

impl WriteRateLimit {
    pub(super) fn new(read_queue_waiters: Arc<Mutex<ReadQueueWaiters>>) -> Self {
        Self {
            read_queue_waiters,
            next_write_at: Instant::now(),
        }
    }

    pub(super) fn must_wait(&self, has_active_writes: bool, now: Instant) -> bool {
        let waiters = self.read_queue_waiters.lock().unwrap();
        let limited = waiters.waiting_reads > 0
            || waiters
                .last_wait_ended_at
                .is_some_and(|ended| now.saturating_duration_since(ended) < WRITE_LIMIT_AFTER_READ_WAIT);
        limited && (has_active_writes || self.next_write_at > now)
    }

    pub(super) fn record_write(&mut self, bytes: usize, now: Instant) {
        // Charge the whole request once, including padding. Idle time earns no burst credit.
        self.next_write_at =
            now + Duration::from_secs_f64(bytes as f64 / WRITE_BYTES_PER_SECOND_WITH_WAITING_READS as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_limit_lasts_for_100_ms_after_the_last_read_wait() {
        let waiters = Arc::new(Mutex::new(ReadQueueWaiters::default()));
        let mut limit = WriteRateLimit::new(waiters.clone());
        let now = Instant::now();
        limit.record_write(1024 * 1024, now);
        assert!(!limit.must_wait(true, now));
        waiters.lock().unwrap().waiting_reads = 2;
        assert!(limit.must_wait(true, now));
        assert!(limit.must_wait(false, now));
        waiters.lock().unwrap().remove_waiting_read(now);
        let cleared = now + Duration::from_secs(1);
        assert!(limit.must_wait(true, cleared));
        waiters.lock().unwrap().remove_waiting_read(cleared);
        assert!(limit.must_wait(true, cleared));
        assert!(!limit.must_wait(false, cleared));
        limit.record_write(1024 * 1024, cleared);
        assert!(limit.must_wait(false, cleared));
        assert!(!limit.must_wait(false, cleared + Duration::from_millis(1)));
        let expires = cleared + WRITE_LIMIT_AFTER_READ_WAIT;
        assert!(limit.must_wait(true, expires - Duration::from_nanos(1)));
        assert!(!limit.must_wait(true, expires));
    }

    #[test]
    fn another_read_wait_restarts_the_100_ms_hold() {
        let waiters = Arc::new(Mutex::new(ReadQueueWaiters::default()));
        let limit = WriteRateLimit::new(waiters.clone());
        let now = Instant::now();
        waiters.lock().unwrap().waiting_reads = 1;
        waiters.lock().unwrap().remove_waiting_read(now);
        waiters.lock().unwrap().waiting_reads = 1;
        let cleared = now + Duration::from_millis(150);
        assert!(limit.must_wait(true, cleared));
        waiters.lock().unwrap().remove_waiting_read(cleared);
        let expires = cleared + WRITE_LIMIT_AFTER_READ_WAIT;
        assert!(limit.must_wait(true, expires - Duration::from_nanos(1)));
        assert!(!limit.must_wait(true, expires));
    }

    #[test]
    fn pacing_charges_bytes_without_accumulating_idle_credit() {
        let waiters = Arc::new(Mutex::new(ReadQueueWaiters::default()));
        waiters.lock().unwrap().waiting_reads = 1;
        let mut limit = WriteRateLimit::new(waiters);
        let now = Instant::now();
        assert!(!limit.must_wait(false, now));
        limit.record_write(1024 * 1024, now);
        let spacing = Duration::from_secs_f64(1.0 / 1024.0);
        assert!(limit.must_wait(false, now + spacing - Duration::from_nanos(1)));
        assert!(!limit.must_wait(false, now + spacing));
        assert!(limit.must_wait(true, now + spacing));

        let later = now + Duration::from_secs(10);
        assert!(!limit.must_wait(false, later));
        limit.record_write(4096, later);
        assert!(limit.must_wait(false, later));
        assert!(!limit.must_wait(false, later + Duration::from_micros(4)));
        limit.record_write(512 * 1024, later);
        assert!(limit.must_wait(false, later));
        assert!(!limit.must_wait(false, later + Duration::from_micros(500)));
    }
}
