//! Shared S3-FIFO queue bookkeeping.

use std::collections::{BTreeMap, HashMap};

use crate::{ByteRange, ObjectKey};

/// A resident range's FIFO position and saturating access counter.
struct ResidentRange {
    key: ObjectKey,
    range: ByteRange,
    position: u64,
    frequency: u8,
    in_small: bool,
}

/// Byte-weighted small/main FIFO queues and a bounded, payload-free ghost history.
/// Entry IDs belong to the caller and must not be reused. Queue positions permit
/// immediate removal of replaced entries without accumulating stale queue nodes.
#[doc(hidden)]
pub struct S3Fifo {
    small_capacity: u64,
    ghost_capacity: u64,
    small_bytes: u64,
    ghost_bytes: u64,
    next_position: u64,
    next_small: Option<bool>,
    residents: HashMap<u64, ResidentRange>,
    small: BTreeMap<u64, u64>,
    main: BTreeMap<u64, u64>,
    ghosts: HashMap<(ObjectKey, ByteRange), u64>,
    ghost_order: BTreeMap<u64, (ObjectKey, ByteRange)>,
}

impl S3Fifo {
    /// Creates empty queues using payload bytes as their weight.
    pub fn new(capacity: u64) -> Self {
        Self {
            small_capacity: capacity / 10,
            ghost_capacity: capacity - capacity / 10,
            small_bytes: 0,
            ghost_bytes: 0,
            next_position: 0,
            next_small: None,
            residents: HashMap::new(),
            small: BTreeMap::new(),
            main: BTreeMap::new(),
            ghosts: HashMap::new(),
            ghost_order: BTreeMap::new(),
        }
    }

    fn position(&mut self) -> u64 {
        self.next_position = self.next_position.checked_add(1).expect("S3-FIFO positions exhausted");
        self.next_position
    }

    /// Admits a range at frequency zero, directly to main on an exact ghost hit.
    pub fn insert(&mut self, id: u64, key: ObjectKey, range: ByteRange) {
        self.next_small = None;
        let identity = (key, range);
        let in_small = if let Some(position) = self.ghosts.remove(&identity) {
            self.ghost_order.remove(&position);
            self.ghost_bytes -= range.len();
            false
        } else {
            true
        };
        let position = self.position();
        if in_small {
            self.small.insert(position, id);
            self.small_bytes += range.len();
        } else {
            self.main.insert(position, id);
        }
        let previous = self.residents.insert(
            id,
            ResidentRange {
                key: identity.0,
                range,
                position,
                frequency: 0,
                in_small,
            },
        );
        assert!(previous.is_none(), "S3-FIFO entry IDs must be unique");
    }

    /// Records a successful access without moving the entry in its FIFO.
    pub fn record_access(&mut self, id: u64) {
        if let Some(entry) = self.residents.get_mut(&id) {
            entry.frequency = (entry.frequency + 1).min(3);
        }
    }

    /// Removes a resident entry without adding it to the ghost history.
    pub fn remove(&mut self, id: u64) {
        if let Some(entry) = self.residents.remove(&id) {
            if entry.in_small {
                self.small.remove(&entry.position);
                self.small_bytes -= entry.range.len();
            } else {
                self.main.remove(&entry.position);
            }
        }
    }

    /// Advances at most `work_limit` queue heads, returning a victim if ready.
    /// Ineligible entries rotate without changing frequency. `None` can mean
    /// progress through promotions/second chances, not just an empty cache.
    pub fn candidate(
        &mut self,
        work_limit: usize,
        mut eligible: impl FnMut(&ObjectKey, ByteRange) -> bool,
    ) -> Option<(u64, ObjectKey, ByteRange)> {
        for _ in 0..work_limit {
            let from_small = !self.small.is_empty()
                && (self.main.is_empty() || self.next_small.unwrap_or(self.small_bytes >= self.small_capacity));
            self.next_small = None;
            let queue = if from_small { &self.small } else { &self.main };
            let (&position, &id) = queue.first_key_value()?;
            let entry = self.residents.get_mut(&id).unwrap();
            let eligible = eligible(&entry.key, entry.range);
            if eligible && ((from_small && entry.frequency < 2) || (!from_small && entry.frequency == 0)) {
                return Some((id, entry.key.clone(), entry.range));
            }
            // Finish this queue's eviction even across bounded calls. Ineligible
            // heads instead give the other queue a turn to prevent starvation.
            self.next_small = Some(if eligible { from_small } else { !from_small });
            // Hot small entries enter main with a fresh counter. Main entries
            // get one FIFO rotation per frequency credit, up to three.
            if from_small {
                self.small.remove(&position);
                if eligible {
                    self.small_bytes -= entry.range.len();
                    entry.in_small = false;
                    entry.frequency = 0;
                }
            } else {
                self.main.remove(&position);
                if eligible {
                    entry.frequency -= 1;
                }
            }
            let position = self.position();
            let entry = self.residents.get_mut(&id).unwrap();
            entry.position = position;
            if entry.in_small {
                self.small.insert(position, id);
            } else {
                self.main.insert(position, id);
            }
        }
        None
    }

    /// Removes a selected victim, remembering cold small-queue ranges as ghosts.
    /// Main-queue eviction, explicit removal and replacement do not create ghosts.
    pub fn evict(&mut self, id: u64) {
        let entry = &self.residents[&id];
        if entry.in_small && entry.range.len() <= self.ghost_capacity {
            let identity = (entry.key.clone(), entry.range);
            // Subtract before adding to avoid overflow at very large capacities.
            while self.ghost_bytes > self.ghost_capacity - identity.1.len() {
                let (_, oldest) = self.ghost_order.pop_first().unwrap();
                self.ghost_bytes -= oldest.1.len();
                self.ghosts.remove(&oldest);
            }
            let position = self.position();
            self.ghost_bytes += identity.1.len();
            self.ghosts.insert(identity.clone(), position);
            self.ghost_order.insert(position, identity);
        }
        self.remove(id);
    }
}

#[cfg(test)]
mod tests;
