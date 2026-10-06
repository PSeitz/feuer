use feuer_types::ByteRange;

/// Minimum percentage of source payload bytes that range trimming must save.
const MIN_RANGE_TRIM_SAVINGS_PERCENT: u128 = 30;

/// Plans replacement ranges from requests fully contained in `source_range`.
/// Filters and merges the owned request snapshot in place, without retaining gaps.
/// Returns no plan if no requests remain or the saved bytes fall below the minimum.
pub(super) fn plan_range_trim(
    source_range: ByteRange,
    mut requested_ranges: Vec<ByteRange>,
) -> Option<Box<[ByteRange]>> {
    requested_ranges.retain(|requested| source_range.contains(*requested));
    requested_ranges.sort_unstable();
    requested_ranges.dedup_by(|requested_range, previous_range| {
        if requested_range.start() > previous_range.end() {
            return false;
        }
        *previous_range = ByteRange::new(previous_range.start(), previous_range.end().max(requested_range.end()))
            .expect("a union of ordered ranges must remain ordered");
        true
    });

    let reclaimed_bytes = source_range.len() - requested_ranges.iter().map(|range| range.len()).sum::<u64>();
    (!requested_ranges.is_empty()
        && u128::from(reclaimed_bytes) * 100 >= u128::from(source_range.len()) * MIN_RANGE_TRIM_SAVINGS_PERCENT)
        .then(|| requested_ranges.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn merges_duplicate_overlapping_and_adjacent_requests_covered_by_the_source() {
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

        assert_eq!(*replacements, [range(12, 16), range(18, 24)]);
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
