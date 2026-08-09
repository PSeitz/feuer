use std::collections::VecDeque;

use feuer_types::ByteRange;

/// Maximum exact access events retained for one object key.
pub(super) const MAX_ACCESS_EVENTS_PER_KEY: usize = 64;
/// Shard-local successful accesses represented by one evidence epoch.
pub(super) const ACCESSES_PER_EPOCH: u64 = 64;
/// Oldest epoch that can still contribute retention value.
pub(super) const MAX_EVIDENCE_AGE_EPOCHS: u64 = 7;

/// One exact requested interval and its shard-local observation epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccessEvent {
    range: ByteRange,
    epoch: u64,
}

/// Bounded, ageable access evidence for one complete object key.
///
/// Events are deliberately not coalesced: repeated requests remain repeated
/// evidence until they age out or are displaced by the per-key bound.
#[derive(Default)]
pub(super) struct AccessEvidence {
    events: VecDeque<AccessEvent>,
}

impl AccessEvidence {
    pub(super) fn record(&mut self, requested_range: ByteRange, epoch: u64) {
        self.expire(epoch);
        if self.events.len() == MAX_ACCESS_EVENTS_PER_KEY {
            self.events.pop_front();
        }
        self.events.push_back(AccessEvent {
            range: requested_range,
            epoch,
        });
    }

    /// Iterates exact, repeated requested ranges that still have policy value.
    pub(super) fn active_ranges(&self, epoch: u64) -> impl Iterator<Item = ByteRange> + '_ {
        self.events
            .iter()
            .filter(move |event| is_active(**event, epoch))
            .map(|event| event.range)
    }

    /// Projects active events onto an extent that could satisfy them exactly.
    pub(super) fn retention(&self, extent: ByteRange, epoch: u64) -> RetentionEvidence {
        let mut retention = RetentionEvidence::default();
        for event in self.events.iter().filter(|event| is_active(**event, epoch)) {
            if !extent.contains(event.range) {
                continue;
            }

            let age = epoch.saturating_sub(event.epoch);
            let shift = u32::try_from(MAX_EVIDENCE_AGE_EPOCHS - age).expect("an active evidence age must fit in u32");
            retention.weighted_frequency += 1_u64 << shift;
            retention.newest_epoch = Some(retention.newest_epoch.map_or(event.epoch, |seen| seen.max(event.epoch)));
        }
        retention
    }

    fn expire(&mut self, epoch: u64) {
        while self.events.front().is_some_and(|event| !is_active(*event, epoch)) {
            self.events.pop_front();
        }
    }

    #[cfg(test)]
    pub(super) fn ranges(&self) -> Vec<ByteRange> {
        self.events.iter().map(|event| event.range).collect()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.events.len()
    }
}

/// A frequency and recency projection used by shard-local victim ordering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RetentionEvidence {
    pub(super) weighted_frequency: u64,
    pub(super) newest_epoch: Option<u64>,
}

fn is_active(event: AccessEvent, epoch: u64) -> bool {
    epoch.saturating_sub(event.epoch) <= MAX_EVIDENCE_AGE_EPOCHS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn bounds_events_without_coalescing_repeated_ranges() {
        let repeated = range(10, 20);
        let mut evidence = AccessEvidence::default();
        for index in 0..MAX_ACCESS_EVENTS_PER_KEY + 3 {
            let requested = if index >= MAX_ACCESS_EVENTS_PER_KEY {
                repeated
            } else {
                range(index as u64, index as u64 + 1)
            };
            evidence.record(requested, 0);
        }

        assert_eq!(evidence.len(), MAX_ACCESS_EVENTS_PER_KEY);
        assert_eq!(evidence.ranges()[MAX_ACCESS_EVENTS_PER_KEY - 3..], [repeated; 3]);
        assert_eq!(evidence.ranges()[0], range(3, 4));
    }

    #[test]
    fn ages_frequency_deterministically_and_expires_stale_events() {
        let requested = range(2, 4);
        let extent = range(0, 8);
        let mut evidence = AccessEvidence::default();
        evidence.record(requested, 0);
        evidence.record(requested, 0);

        assert_eq!(
            evidence.retention(extent, 0),
            RetentionEvidence {
                weighted_frequency: 256,
                newest_epoch: Some(0),
            }
        );
        assert_eq!(evidence.retention(extent, 1).weighted_frequency, 128);
        assert_eq!(
            evidence.retention(extent, MAX_EVIDENCE_AGE_EPOCHS).weighted_frequency,
            2
        );
        assert_eq!(
            evidence
                .retention(extent, MAX_EVIDENCE_AGE_EPOCHS + 1)
                .weighted_frequency,
            0
        );

        evidence.record(range(6, 7), MAX_EVIDENCE_AGE_EPOCHS + 1);
        assert_eq!(evidence.ranges(), vec![range(6, 7)]);
    }

    #[test]
    fn projects_credit_only_to_extents_covering_the_exact_request() {
        let mut evidence = AccessEvidence::default();
        evidence.record(range(3, 7), 0);

        assert_eq!(evidence.retention(range(0, 8), 0).weighted_frequency, 128);
        assert_eq!(evidence.retention(range(3, 5), 0).weighted_frequency, 0);
        assert_eq!(evidence.retention(range(5, 8), 0).weighted_frequency, 0);
    }
}
