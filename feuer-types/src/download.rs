use std::fmt;

use bytes::Bytes;
use thiserror::Error;

use crate::ByteRange;

/// One completed result from an application download callback.
///
/// The exact downloaded range is derived from [`Self::downloaded_start`] and
/// the payload length. The object identity comes from the `get_or_fetch` call,
/// so it is deliberately not repeated here. Constructing a download creates no
/// cache-access event.
#[derive(Clone)]
pub struct Download {
    start: u64,
    bytes: Bytes,
    allocation_charge: usize,
}

impl Download {
    /// Creates a download whose range starts at `start`.
    pub fn new(start: u64, bytes: Bytes) -> Result<Self, DownloadError> {
        let payload_bytes = bytes.len() as u64;
        start.checked_add(payload_bytes).ok_or(DownloadError::RangeOverflow {
            downloaded_start: start,
            payload_bytes,
        })?;
        let allocation_charge = bytes.len();
        Ok(Self {
            start,
            bytes,
            allocation_charge,
        })
    }

    /// Charges the known backing allocation capacity rather than just the payload length.
    /// Shared allocations are conservatively charged once per cached entry.
    ///
    /// # Panics
    ///
    /// Panics if `allocation_charge` is smaller than the payload length.
    pub fn with_allocation_charge(mut self, allocation_charge: usize) -> Self {
        assert!(allocation_charge >= self.bytes.len());
        self.allocation_charge = allocation_charge;
        self
    }

    /// Returns the backing allocation charge used by the memory cache.
    /// Defaults to payload length when the allocation capacity is unknown.
    pub const fn allocation_charge(&self) -> usize {
        self.allocation_charge
    }

    /// Returns the first downloaded object offset.
    pub const fn downloaded_start(&self) -> u64 {
        self.start
    }

    /// Returns the exact object range derived from the payload length.
    pub fn downloaded_range(&self) -> ByteRange {
        ByteRange::new(self.start, self.start + self.bytes.len() as u64)
            .expect("the download constructor checked the range")
    }

    /// Returns the contiguous bytes covering the downloaded range.
    pub const fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// Returns a shared byte slice. The caller must supply a contained object range.
    pub fn bytes_in_range(&self, range: ByteRange) -> Bytes {
        let start = (range.start() - self.start) as usize;
        self.bytes.slice(start..start + range.len() as usize)
    }

    /// Decomposes the download into its derived range and payload.
    pub fn into_parts(self) -> (ByteRange, Bytes) {
        (self.downloaded_range(), self.bytes)
    }
}

impl fmt::Debug for Download {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Download")
            .field("downloaded_range", &self.downloaded_range())
            .field("payload_len", &self.bytes.len())
            .finish()
    }
}

/// An invalid download callback result.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DownloadError {
    /// The payload extends beyond the representable object-offset space.
    #[error("download starting at {downloaded_start} with {payload_bytes} bytes exceeds the u64 object-offset space")]
    RangeOverflow {
        /// The first downloaded object offset.
        downloaded_start: u64,
        /// The downloaded payload length.
        payload_bytes: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn derives_the_exact_range_from_the_start_and_payload() {
        for (start, length) in [(3, 13), (u64::MAX - 13, 13), (u64::MAX, 0)] {
            let payload = Bytes::from(vec![7; length]);
            let download = Download::new(start, payload.clone()).unwrap();
            assert_eq!(download.downloaded_start(), start);
            assert_eq!(download.downloaded_range(), range(start, start + length as u64));
            assert_eq!(download.bytes().as_ptr(), payload.as_ptr());
            let requested = range(start + length as u64 / 2, start + length as u64);
            assert_eq!(download.bytes_in_range(requested), payload.slice(length / 2..));
            assert_eq!(download.into_parts(), (range(start, start + length as u64), payload));
        }
    }

    #[test]
    fn rejects_unrepresentable_ranges() {
        assert_eq!(
            Download::new(u64::MAX - 1, Bytes::from_static(b"ab")).unwrap_err(),
            DownloadError::RangeOverflow {
                downloaded_start: u64::MAX - 1,
                payload_bytes: 2,
            }
        );
    }

    #[test]
    fn debug_output_omits_payload_bytes() {
        let download = Download::new(0, Bytes::from_static(b"secret")).unwrap();
        let output = format!("{download:?}");

        assert!(!output.contains("secret"));
        assert!(output.contains("downloaded_range: ByteRange { start: 0, end: 6 }"));
        assert!(output.contains("payload_len: 6"));
    }
}
