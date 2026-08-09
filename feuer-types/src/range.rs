use std::ops::Range;

use thiserror::Error;

/// An exact, non-empty, half-open byte range within an immutable object.
///
/// Feuer does not align or otherwise normalize either endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    /// Creates the exact range `start..end`.
    pub const fn new(start: u64, end: u64) -> Result<Self, InvalidRange> {
        if start >= end {
            return Err(InvalidRange { start, end });
        }
        Ok(Self { start, end })
    }

    /// Returns the inclusive start offset.
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Returns the exclusive end offset.
    pub const fn end(self) -> u64 {
        self.end
    }

    /// Returns the number of bytes in the range.
    pub const fn len(self) -> u64 {
        self.end - self.start
    }

    /// Returns `false`; a [`ByteRange`] is non-empty by construction.
    pub const fn is_empty(self) -> bool {
        false
    }

    /// Returns whether this range completely covers `other`.
    pub const fn contains(self, other: Self) -> bool {
        self.start <= other.start && self.end >= other.end
    }

    /// Returns whether this range overlaps `other`.
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

impl TryFrom<Range<u64>> for ByteRange {
    type Error = InvalidRange;

    fn try_from(range: Range<u64>) -> Result<Self, Self::Error> {
        Self::new(range.start, range.end)
    }
}

impl From<ByteRange> for Range<u64> {
    fn from(range: ByteRange) -> Self {
        range.start..range.end
    }
}

/// The error returned for an empty or reversed byte range.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("byte range must be non-empty and ordered, got {start}..{end}")]
pub struct InvalidRange {
    start: u64,
    end: u64,
}

impl InvalidRange {
    /// Returns the rejected start offset.
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Returns the rejected end offset.
    pub const fn end(self) -> u64 {
        self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_arbitrary_unaligned_endpoints() {
        let range = ByteRange::new(3, 4_194_309).unwrap();

        assert_eq!(range.start(), 3);
        assert_eq!(range.end(), 4_194_309);
        assert_eq!(range.len(), 4_194_306);
    }

    #[test]
    fn rejects_empty_and_reversed_ranges() {
        assert_eq!(ByteRange::new(7, 7).unwrap_err(), InvalidRange { start: 7, end: 7 });
        assert_eq!(ByteRange::new(8, 7).unwrap_err(), InvalidRange { start: 8, end: 7 });
    }

    #[test]
    fn containment_and_overlap_use_exact_boundaries() {
        let outer = ByteRange::new(10, 20).unwrap();

        assert!(outer.contains(ByteRange::new(10, 20).unwrap()));
        assert!(outer.contains(ByteRange::new(11, 19).unwrap()));
        assert!(!outer.overlaps(ByteRange::new(20, 30).unwrap()));
        assert!(outer.overlaps(ByteRange::new(19, 30).unwrap()));
    }
}
