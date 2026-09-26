use feuer_types::ByteRange;

/// Copy only when the plan releases at least one quarter of its source.
const MIN_RECLAIM_DIVISOR: u64 = 4;

/// A plan to trim a cached range: its source, retained ranges, and retained byte count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RangeTrimPlan {
    source_range: ByteRange,
    retained_ranges: Vec<ByteRange>,
    retained_bytes: u64,
}

impl RangeTrimPlan {
    pub(super) const fn source_range(&self) -> ByteRange {
        self.source_range
    }

    pub(super) fn retained_ranges(&self) -> &[ByteRange] {
        &self.retained_ranges
    }

    pub(super) const fn reclaimed_bytes(&self) -> u64 {
        self.source_range.len() - self.retained_bytes
    }
}

/// Projects exact requested ranges onto one downloaded range without mutation.
///
/// Only requests fully covered by `source_range` can give it retention value.
/// Overlapping and adjacent requests are grouped, but gaps remain unretained.
/// Repetition stays available to eviction policy while naturally producing no
/// duplicate copied interval here.
pub(super) fn plan_range_trim(
    source_range: ByteRange,
    requested_ranges: impl IntoIterator<Item = ByteRange>,
) -> Option<RangeTrimPlan> {
    let mut retained_ranges: Vec<_> = requested_ranges
        .into_iter()
        .filter(|requested| source_range.contains(*requested))
        .collect();
    retained_ranges.sort_unstable();

    let mut grouped: Vec<ByteRange> = Vec::with_capacity(retained_ranges.len());
    for requested in retained_ranges {
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
    let reclaimed_bytes = source_range.len() - retained_bytes;
    if reclaimed_bytes < source_range.len().div_ceil(MIN_RECLAIM_DIVISOR) {
        return None;
    }

    Some(RangeTrimPlan {
        source_range,
        retained_ranges: grouped,
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
        let plan = plan_range_trim(
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

        assert_eq!(plan.source_range(), range(10, 30));
        assert_eq!(plan.retained_ranges(), &[range(12, 14), range(18, 24)]);
        assert_eq!(plan.reclaimed_bytes(), 12);
    }

    #[test]
    fn merges_adjacent_requests_so_each_original_request_stays_coverable() {
        let plan = plan_range_trim(range(0, 16), [range(2, 5), range(5, 9)]).unwrap();

        assert_eq!(plan.retained_ranges(), &[range(2, 9)]);
    }

    #[test]
    fn skips_empty_or_low_savings_plans() {
        assert!(plan_range_trim(range(0, 8), [range(8, 10)]).is_none());
        assert!(plan_range_trim(range(0, 8), [range(1, 8)]).is_none());
        assert!(plan_range_trim(range(0, 8), [range(2, 8)]).is_some());
    }
}
