use feuer_types::ByteRange;

/// Minimum percentage of source payload bytes that range trimming must save.
const MIN_RANGE_TRIM_SAVINGS_PERCENT: u128 = 30;

/// Plans replacement ranges from requests fully contained in `source_range`.
/// Deduplicates requests without merging overlapping, adjacent, or contained ranges.
/// Returns no plan if no requests remain or savings from the separate copies fall below the minimum.
pub(super) fn plan_range_trim(
    source_range: ByteRange,
    mut requested_ranges: Vec<ByteRange>,
) -> Option<Box<[ByteRange]>> {
    requested_ranges.retain(|requested| source_range.contains(*requested));
    requested_ranges.sort_unstable();
    requested_ranges.dedup();

    let replacement_bytes: u128 = requested_ranges.iter().map(|range| u128::from(range.len())).sum();
    let reclaimed_bytes = u128::from(source_range.len()).checked_sub(replacement_bytes)?;
    (!requested_ranges.is_empty()
        && reclaimed_bytes * 100 >= u128::from(source_range.len()) * MIN_RANGE_TRIM_SAVINGS_PERCENT)
        .then(|| requested_ranges.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn preserves_distinct_request_ranges_covered_by_the_source() {
        let replacements = plan_range_trim(
            range(10, 30),
            vec![
                range(14, 16),
                range(12, 14),
                range(12, 14),
                range(18, 22),
                range(21, 24),
                range(28, 32),
                range(2, 4),
            ],
        )
        .unwrap();

        assert_eq!(
            *replacements,
            [range(12, 14), range(14, 16), range(18, 22), range(21, 24)]
        );
    }

    #[test]
    fn savings_count_each_copy_of_overlapping_bytes() {
        assert!(plan_range_trim(range(0, 10), vec![range(1, 5), range(2, 6)]).is_none());
        assert!(plan_range_trim(range(0, 10), vec![range(0, 7), range(1, 8)]).is_none());
        assert!(plan_range_trim(range(0, u64::MAX), vec![range(0, u64::MAX), range(1, u64::MAX)]).is_none());
        assert_eq!(
            *plan_range_trim(range(0, 10), vec![range(1, 5), range(2, 5)]).unwrap(),
            [range(1, 5), range(2, 5)]
        );
    }

    #[test]
    fn range_trim_savings_round_up_without_overflow() {
        for (source_bytes, minimum_saved_bytes) in [(9, 3), (u64::MAX, 5_534_023_222_112_865_485)] {
            let source = range(0, source_bytes);
            let replacement_end = source_bytes - minimum_saved_bytes;
            assert!(plan_range_trim(source, vec![range(0, replacement_end + 1)]).is_none());
            let replacements = plan_range_trim(source, vec![range(0, replacement_end)]).unwrap();
            assert_eq!(*replacements, [range(0, replacement_end)]);
        }
    }

    #[test]
    fn skips_empty_or_low_savings_plans() {
        assert!(plan_range_trim(range(0, 8), Vec::new()).is_none());
        assert!(plan_range_trim(range(0, 8), vec![range(8, 10)]).is_none());
        assert!(plan_range_trim(range(0, 8), vec![range(1, 8)]).is_none());
        assert!(plan_range_trim(range(0, 8), vec![range(2, 8)]).is_none());
        assert!(plan_range_trim(range(0, 8), vec![range(3, 8)]).is_some());
    }
}
