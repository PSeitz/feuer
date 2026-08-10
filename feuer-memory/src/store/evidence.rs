use std::collections::VecDeque;

use feuer_types::ByteRange;

/// Target 125-ms source-request cost at 80 MB/s, as equivalent transferred bytes.
pub(super) const FIXED_RETRIEVAL_EQUIVALENT_BYTES: u64 = 10_000_000;
/// Maximum exact access events retained for one object key.
pub(super) const MAX_ACCESS_EVENTS_PER_KEY: usize = 64;
/// Maximum same-shard successful-access age that still contributes.
pub(super) const EVIDENCE_LIFETIME_ACCESSES: u64 = 32_768;

/// One exact requested interval and its shard-local observation clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccessEvent {
    range: ByteRange,
    observed_at: u64,
}

/// Bounded access evidence for one complete object key.
///
/// Events are deliberately not coalesced: repeated requests remain repeated
/// evidence until they expire or are displaced by the per-key bound.
#[derive(Default)]
pub(super) struct AccessEvidence {
    events: VecDeque<AccessEvent>,
}

impl AccessEvidence {
    pub(super) fn record(&mut self, range: ByteRange, access_clock: u64) {
        self.expire(access_clock);
        if self.events.len() == MAX_ACCESS_EVENTS_PER_KEY {
            self.events.pop_front();
        }
        self.events.push_back(AccessEvent {
            range,
            observed_at: access_clock,
        });
    }

    /// Iterates exact requested ranges that still have policy value.
    pub(super) fn active_ranges(&self, access_clock: u64) -> impl Iterator<Item = ByteRange> + '_ {
        self.events
            .iter()
            .filter(move |event| is_active(**event, access_clock))
            .map(|event| event.range)
    }

    /// Sums modeled source retrieval cost for active events covered by an extent.
    pub(super) fn retention_value(&self, extent: ByteRange, access_clock: u64) -> u64 {
        self.events
            .iter()
            .filter(|event| is_active(**event, access_clock) && extent.contains(event.range))
            .map(|event| FIXED_RETRIEVAL_EQUIVALENT_BYTES.saturating_add(event.range.len()))
            .fold(0, u64::saturating_add)
    }

    fn expire(&mut self, access_clock: u64) {
        while self
            .events
            .front()
            .is_some_and(|event| !is_active(*event, access_clock))
        {
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

fn is_active(event: AccessEvent, access_clock: u64) -> bool {
    access_clock.saturating_sub(event.observed_at) <= EVIDENCE_LIFETIME_ACCESSES
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
    fn retains_full_retrieval_value_until_expiration() {
        let requested = range(2, 4);
        let extent = range(0, 8);
        let mut evidence = AccessEvidence::default();
        evidence.record(requested, 0);
        evidence.record(requested, 0);

        let expected = 2 * (FIXED_RETRIEVAL_EQUIVALENT_BYTES + requested.len());
        assert_eq!(evidence.retention_value(extent, 0), expected);
        assert_eq!(evidence.retention_value(extent, EVIDENCE_LIFETIME_ACCESSES), expected);
        assert_eq!(evidence.retention_value(extent, EVIDENCE_LIFETIME_ACCESSES + 1), 0);

        evidence.record(range(6, 7), EVIDENCE_LIFETIME_ACCESSES + 1);
        assert_eq!(evidence.ranges(), vec![range(6, 7)]);
    }

    #[test]
    fn projects_credit_only_to_extents_covering_the_exact_request() {
        let mut evidence = AccessEvidence::default();
        evidence.record(range(3, 7), 0);

        assert_eq!(
            evidence.retention_value(range(0, 8), 0),
            FIXED_RETRIEVAL_EQUIVALENT_BYTES + 4
        );
        assert_eq!(evidence.retention_value(range(3, 5), 0), 0);
        assert_eq!(evidence.retention_value(range(5, 8), 0), 0);
    }
}
