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
    range: ByteRange,
    bytes: Bytes,
}

impl Download {
    /// Creates a download whose range starts at `downloaded_start`.
    pub fn new(downloaded_start: u64, bytes: Bytes) -> Result<Self, DownloadError> {
        let payload_bytes = bytes.len() as u64;
        let downloaded_end = downloaded_start
            .checked_add(payload_bytes)
            .ok_or(DownloadError::RangeOverflow {
                downloaded_start,
                payload_bytes,
            })?;
        let range =
            ByteRange::new(downloaded_start, downloaded_end).expect("a payload length produces an ordered range");
        Ok(Self { range, bytes })
    }

    /// Returns the first downloaded object offset.
    pub const fn downloaded_start(&self) -> u64 {
        self.range.start()
    }

    /// Returns the exact object range derived from the payload length.
    pub fn downloaded_range(&self) -> ByteRange {
        self.range
    }

    /// Returns the contiguous bytes covering the downloaded range.
    pub const fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// Decomposes the download into its derived range and payload.
    pub fn into_parts(self) -> (ByteRange, Bytes) {
        (self.range, self.bytes)
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
        let payload = Bytes::from(vec![7; 13]);
        let download = Download::new(3, payload.clone()).unwrap();

        assert_eq!(download.downloaded_start(), 3);
        assert_eq!(download.downloaded_range(), range(3, 16));
        assert_eq!(download.bytes().as_ptr(), payload.as_ptr());
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
