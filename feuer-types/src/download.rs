use std::fmt;

use bytes::Bytes;

/// An immutable object returned by an application download callback.
///
/// The object identity comes from the lookup call. Constructing a download creates
/// no cache-access event.
#[derive(Clone)]
pub struct Download {
    bytes: Bytes,
    allocation_charge: usize,
}

impl Download {
    /// Creates a download, allowing empty objects.
    pub fn new(bytes: Bytes) -> Self {
        let allocation_charge = bytes.len();
        Self {
            bytes,
            allocation_charge,
        }
    }

    /// Charges the known backing allocation capacity.
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

    /// Returns the object's bytes.
    pub const fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// Returns the object's bytes, consuming the download.
    pub fn into_bytes(self) -> Bytes {
        self.bytes
    }
}

impl fmt::Debug for Download {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Download")
            .field("payload_len", &self.bytes.len())
            .field("allocation_charge", &self.allocation_charge)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_complete_bytes_without_copying() {
        for payload in [Bytes::new(), Bytes::from_static(b"payload")] {
            let download = Download::new(payload.clone());
            assert_eq!(download.bytes().as_ptr(), payload.as_ptr());
            assert_eq!(download.allocation_charge(), payload.len());
            assert_eq!(download.into_bytes(), payload);
        }
    }

    #[test]
    fn charges_backing_capacity() {
        let download = Download::new(Bytes::from_static(b"payload")).with_allocation_charge(128);
        assert_eq!(download.allocation_charge(), 128);
        assert_eq!(download.bytes().len(), 7);
    }

    #[test]
    #[should_panic]
    fn rejects_charge_smaller_than_payload() {
        Download::new(Bytes::from_static(b"payload")).with_allocation_charge(6);
    }

    #[test]
    fn debug_output_omits_payload_bytes() {
        let output = format!("{:?}", Download::new(Bytes::from_static(b"secret")));
        assert!(!output.contains("secret"));
        assert!(output.contains("payload_len: 6"));
    }
}
