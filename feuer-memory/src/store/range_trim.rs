use feuer_types::ByteRange;

/// Plans replacement ranges from requests fully contained in `source_range`.
/// Deduplicates requests without merging overlapping, adjacent, or contained ranges.
/// Returns no plan if no requests remain or the separate copies do not reduce payload bytes.
pub(super) fn plan_range_trim(
    source_range: ByteRange,
    mut requested_ranges: Vec<ByteRange>,
) -> Option<Box<[ByteRange]>> {
    requested_ranges.retain(|requested| source_range.contains(*requested));
    requested_ranges.sort_unstable();
    requested_ranges.dedup();

    let replacement_bytes: u128 = requested_ranges.iter().map(|range| u128::from(range.len())).sum();
    (!requested_ranges.is_empty() && replacement_bytes < u128::from(source_range.len()))
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
        assert_eq!(
            *plan_range_trim(range(0, 10), vec![range(1, 5), range(2, 6)]).unwrap(),
            [range(1, 5), range(2, 6)]
        );
        assert!(plan_range_trim(range(0, 10), vec![range(0, 5), range(4, 9)]).is_none());
        assert!(plan_range_trim(range(0, 10), vec![range(0, 7), range(1, 8)]).is_none());
        assert!(plan_range_trim(range(0, u64::MAX), vec![range(0, u64::MAX), range(1, u64::MAX)]).is_none());
        assert_eq!(
            *plan_range_trim(range(0, 10), vec![range(1, 5), range(2, 5)]).unwrap(),
            [range(1, 5), range(2, 5)]
        );
    }

    #[test]
    fn trims_even_one_byte_without_overflow() {
        for source_bytes in [9, u64::MAX] {
            let source = range(0, source_bytes);
            assert!(plan_range_trim(source, vec![source]).is_none());
            let requested = range(0, source_bytes - 1);
            assert_eq!(*plan_range_trim(source, vec![requested]).unwrap(), [requested]);
        }
    }

    #[test]
    fn skips_plans_without_requests_or_byte_reduction() {
        assert!(plan_range_trim(range(0, 8), Vec::new()).is_none());
        assert!(plan_range_trim(range(0, 8), vec![range(8, 10)]).is_none());
        assert!(plan_range_trim(range(0, 8), vec![range(0, 8)]).is_none());
        assert!(plan_range_trim(range(0, 0), vec![range(0, 0)]).is_none());
    }
}
