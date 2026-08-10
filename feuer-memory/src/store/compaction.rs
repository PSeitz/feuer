use feuer_types::ByteRange;

/// Copy only when the plan releases at least one quarter of its source.
const MIN_RECLAIM_DIVISOR: u64 = 4;

/// A pure replacement plan for one retained downloaded extent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CompactionPlan {
    source: ByteRange,
    retained: Vec<ByteRange>,
    retained_bytes: u64,
}

impl CompactionPlan {
    pub(super) const fn source(&self) -> ByteRange {
        self.source
    }

    pub(super) fn retained(&self) -> &[ByteRange] {
        &self.retained
    }

    pub(super) const fn reclaimed_bytes(&self) -> u64 {
        self.source.len() - self.retained_bytes
    }
}

/// Projects exact requested ranges onto one downloaded extent without mutation.
///
/// Only requests fully covered by `source` can give it retention value.
/// Overlapping and adjacent requests are grouped, but gaps remain unretained.
/// Repetition stays available to eviction policy while naturally producing no
/// duplicate copied interval here.
pub(super) fn plan_compaction(
    source: ByteRange,
    requested_ranges: impl IntoIterator<Item = ByteRange>,
) -> Option<CompactionPlan> {
    let mut retained: Vec<_> = requested_ranges
        .into_iter()
        .filter(|requested| source.contains(*requested))
        .collect();
    retained.sort_unstable();

    let mut grouped: Vec<ByteRange> = Vec::with_capacity(retained.len());
    for requested in retained {
        if let Some(previous) = grouped.last_mut()
            && requested.start() <= previous.end()
        {
            *previous = ByteRange::new(previous.start(), previous.end().max(requested.end()))
                .expect("a union of non-empty ranges must remain non-empty");
        } else {
            grouped.push(requested);
        }
    }

    if grouped.is_empty() {
        return None;
    }
    let retained_bytes = grouped.iter().map(|range| range.len()).sum();
    let reclaimed_bytes = source.len() - retained_bytes;
    if reclaimed_bytes < source.len().div_ceil(MIN_RECLAIM_DIVISOR) {
        return None;
    }

    Some(CompactionPlan {
        source,
        retained: grouped,
        retained_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn projects_and_groups_only_exact_requests_covered_by_the_source() {
        let plan = plan_compaction(
            range(10, 30),
            [
                range(12, 14),
                range(12, 14),
                range(18, 22),
                range(21, 24),
                range(28, 32),
                range(2, 4),
            ],
        )
        .unwrap();

        assert_eq!(plan.source(), range(10, 30));
        assert_eq!(plan.retained(), &[range(12, 14), range(18, 24)]);
        assert_eq!(plan.reclaimed_bytes(), 12);
    }

    #[test]
    fn merges_adjacent_requests_so_each_original_request_stays_coverable() {
        let plan = plan_compaction(range(0, 16), [range(2, 5), range(5, 9)]).unwrap();

        assert_eq!(plan.retained(), &[range(2, 9)]);
    }

    #[test]
    fn skips_empty_or_low_savings_plans() {
        assert!(plan_compaction(range(0, 8), [range(8, 10)]).is_none());
        assert!(plan_compaction(range(0, 8), [range(1, 8)]).is_none());
        assert!(plan_compaction(range(0, 8), [range(2, 8)]).is_some());
    }
}
