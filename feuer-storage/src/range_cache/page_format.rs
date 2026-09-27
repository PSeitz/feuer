//! Experimental v3 metadata pages. Payload has no page headers: its whole-entry checksum lives in entry metadata.
//! Each entry metadata page carries the checksum of the complete entry metadata, including its key and mappings.
//! Page checksums additionally bind tag, address, ordinal, links and contents.
//! Completion is not persistence; there is deliberately no recovery decoder yet.

use bytes::Bytes;
use feuer_types::ByteRange;

use crate::allocation::DiskRegion;

pub(super) const PAGE_BYTES: usize = 4096;
const PAGE_HEADER_BYTES: usize = 96;
pub(super) const PAGE_CONTENT_BYTES: usize = PAGE_BYTES - PAGE_HEADER_BYTES;
pub(super) const ENTRY_METADATA_PAGE_TAG: &[u8; 8] = b"FEUDES03";
pub(super) const ENTRY_METADATA_INDEX_PAGE_TAG: &[u8; 8] = b"FEUIDX03";

/// Content checksum covers the complete entry metadata, or the index's 32-byte bitmap of entry metadata starts.
pub(super) fn encode_page(
    page: &mut [u8],
    page_tag: &[u8; 8],
    content_checksum: &[u8; 32],
    page_address: u64,
    page_ordinal: u64,
    next_entry_metadata_page_address: u64,
    contents: &[u8],
) {
    assert_eq!(page.len(), PAGE_BYTES);
    assert!(contents.len() <= PAGE_CONTENT_BYTES);
    page.fill(0);
    page[32..64].copy_from_slice(content_checksum);
    page[64..72].copy_from_slice(&page_address.to_le_bytes());
    page[72..80].copy_from_slice(&page_ordinal.to_le_bytes());
    page[80..88].copy_from_slice(&next_entry_metadata_page_address.to_le_bytes());
    page[88..96].copy_from_slice(page_tag);
    page[PAGE_HEADER_BYTES..PAGE_HEADER_BYTES + contents.len()].copy_from_slice(contents);
    let checksum = blake3::hash(&page[32..]);
    page[..32].copy_from_slice(checksum.as_bytes());
}

#[cfg(test)] // Metadata is decoded only by format tests until recovery is implemented.
pub(super) fn validate_page<'a>(
    page: &'a [u8],
    page_tag: &[u8; 8],
    content_checksum: &[u8; 32],
    page_address: u64,
    page_ordinal: u64,
) -> Option<(u64, &'a [u8])> {
    if page.len() != PAGE_BYTES
        || page[32..64] != *content_checksum
        || page[64..72] != page_address.to_le_bytes()
        || page[72..80] != page_ordinal.to_le_bytes()
        || page[88..96] != *page_tag
        || page[..32] != *blake3::hash(&page[32..]).as_bytes()
    {
        return None;
    }
    Some((
        u64::from_le_bytes(page[80..88].try_into().unwrap()),
        &page[PAGE_HEADER_BYTES..],
    ))
}

/// Content length, key length, exact object range, region count, payload checksum, physical ranges, full key.
/// All integers are little-endian u64s. Page links describe the entry metadata's own allocations.
/// The payload checksum covers exactly the entry bytes in region order, excluding final alignment padding.
pub(super) fn encode_entry_metadata(
    key: &str,
    object_range: ByteRange,
    payload_regions: &[DiskRegion],
    payload_checksum: &blake3::Hash,
) -> Bytes {
    let length = 72 + 16 * payload_regions.len() + key.len();
    let mut bytes = Vec::with_capacity(length);
    for value in [
        length as u64,
        key.len() as u64,
        object_range.start(),
        object_range.end(),
        payload_regions.len() as u64,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(payload_checksum.as_bytes());
    for region in payload_regions {
        let disk_range = region.range();
        bytes.extend_from_slice(&disk_range.start.to_le_bytes());
        bytes.extend_from_slice(&disk_range.end.to_le_bytes());
    }
    bytes.extend_from_slice(key.as_bytes());
    Bytes::from(bytes)
}
