use feuer_types::ByteRange;

/// Minimum percentage of source payload bytes that range trimming must save.
const MIN_RANGE_TRIM_SAVINGS_PERCENT: u64 = 25;

/// The source range to trim, the ranges that will replace it, and their total byte count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RangeTrimPlan {
    source_range: ByteRange,
    replacement_ranges: Vec<ByteRange>,
    replacement_bytes: u64,
}

impl RangeTrimPlan {
    pub(super) const fn source_range(&self) -> ByteRange {
        self.source_range
    }

    pub(super) fn replacement_ranges(&self) -> &[ByteRange] {
        &self.replacement_ranges
    }

    pub(super) const fn reclaimed_bytes(&self) -> u64 {
        self.source_range.len() - self.replacement_bytes
    }
}

/// Builds a trimming plan from requests fully contained in `source_range`.
/// Merges overlapping and adjacent requests without retaining gaps.
/// Returns no plan if no requests remain or the saved bytes fall below the minimum.
pub(super) fn plan_range_trim(
    source_range: ByteRange,
    requested_ranges: impl IntoIterator<Item = ByteRange>,
) -> Option<RangeTrimPlan> {
    let mut contained_requested_ranges: Vec<_> = requested_ranges
        .into_iter()
        .filter(|requested| source_range.contains(*requested))
        .collect();
    contained_requested_ranges.sort_unstable();

    let mut merged_ranges: Vec<ByteRange> = Vec::with_capacity(contained_requested_ranges.len());
    for requested_range in contained_requested_ranges {
        if let Some(previous_range) = merged_ranges.last_mut()
            && requested_range.start() <= previous_range.end()
        {
            *previous_range = ByteRange::new(previous_range.start(), previous_range.end().max(requested_range.end()))
                .expect("a union of non-empty ranges must remain non-empty");
        } else {
            merged_ranges.push(requested_range);
        }
    }

    if merged_ranges.is_empty() {
        return None;
    }
    let replacement_bytes = merged_ranges.iter().map(|range| range.len()).sum();
    let reclaimed_bytes = source_range.len() - replacement_bytes;
    let minimum_saved_bytes =
        (u128::from(source_range.len()) * u128::from(MIN_RANGE_TRIM_SAVINGS_PERCENT)).div_ceil(100);
    if u128::from(reclaimed_bytes) < minimum_saved_bytes {
        return None;
    }

    Some(RangeTrimPlan {
        source_range,
        replacement_ranges: merged_ranges,
        replacement_bytes,
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
        assert_eq!(plan.replacement_ranges(), &[range(12, 14), range(18, 24)]);
        assert_eq!(plan.reclaimed_bytes(), 12);
    }

    #[test]
    fn merges_adjacent_requests_so_each_original_request_stays_coverable() {
        let plan = plan_range_trim(range(0, 16), [range(2, 5), range(5, 9)]).unwrap();

        assert_eq!(plan.replacement_ranges(), &[range(2, 9)]);
    }

    #[test]
    fn range_trim_savings_round_up_without_overflow() {
        for (source_bytes, minimum_saved_bytes) in [(9, 3), (u64::MAX, 1_u64 << 62)] {
            let source = range(0, source_bytes);
            let replacement_end = source_bytes - minimum_saved_bytes;
            assert!(plan_range_trim(source, [range(0, replacement_end + 1)]).is_none());
            let plan = plan_range_trim(source, [range(0, replacement_end)]).unwrap();
            assert_eq!(plan.reclaimed_bytes(), minimum_saved_bytes);
        }
    }

    #[test]
    fn skips_empty_or_low_savings_plans() {
        assert!(plan_range_trim(range(0, 8), [range(8, 10)]).is_none());
        assert!(plan_range_trim(range(0, 8), [range(1, 8)]).is_none());
        assert!(plan_range_trim(range(0, 8), [range(2, 8)]).is_some());
    }
}
