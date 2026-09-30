//! Experimental v5 metadata pages. Payload has no page headers: its whole-entry checksum lives in entry metadata.
//! All checksums are XXHash64 with seed zero, stored as little-endian u64s.
//! Each entry metadata page carries the checksum of the complete entry metadata, including its key and mappings.
//! Page checksums additionally bind tag, address, ordinal, links and contents.
//! Chunk metadata binds a cache generation and batch ID; completion is not persistence.

use bytes::Bytes;
use feuer_types::ByteRange;
use twox_hash::XxHash64;

use crate::allocation::DiskRegion;

/// Size in bytes of a metadata page, including its header and padding.
/// An entry's metadata may span multiple linked pages.
pub(super) const METADATA_PAGE_BYTES: usize = 4096;
const PAGE_HEADER_BYTES: usize = 48;
pub(super) const PAGE_CONTENT_BYTES: usize = METADATA_PAGE_BYTES - PAGE_HEADER_BYTES;
pub(super) const ENTRY_METADATA_PAGE_TAG: &[u8; 8] = b"FEUDES05";
pub(super) const CHUNK_METADATA_PAGE_TAG: &[u8; 8] = b"FEUIDX05";
pub(super) const CHUNK_METADATA_CONTENT_BYTES: usize = 64;

/// Content checksum covers complete entry metadata, or the chunk's bitmap and identities.
pub(super) fn encode_page(
    page: &mut [u8],
    page_tag: &[u8; 8],
    content_checksum: u64,
    page_address: u64,
    page_ordinal: u64,
    next_entry_metadata_page_address: u64,
    contents: &[u8],
) {
    assert_eq!(page.len(), METADATA_PAGE_BYTES);
    assert!(contents.len() <= PAGE_CONTENT_BYTES);
    page.fill(0);
    page[8..16].copy_from_slice(&content_checksum.to_le_bytes());
    page[16..24].copy_from_slice(&page_address.to_le_bytes());
    page[24..32].copy_from_slice(&page_ordinal.to_le_bytes());
    page[32..40].copy_from_slice(&next_entry_metadata_page_address.to_le_bytes());
    page[40..48].copy_from_slice(page_tag);
    page[PAGE_HEADER_BYTES..PAGE_HEADER_BYTES + contents.len()].copy_from_slice(contents);
    let checksum = XxHash64::oneshot(0, &page[8..]);
    page[..8].copy_from_slice(&checksum.to_le_bytes());
}

pub(super) fn validate_page<'a>(
    page: &'a [u8],
    page_tag: &[u8; 8],
    content_checksum: u64,
    page_address: u64,
    page_ordinal: u64,
) -> Option<(u64, &'a [u8])> {
    if page.len() != METADATA_PAGE_BYTES
        || page[8..16] != content_checksum.to_le_bytes()
        || page[16..24] != page_address.to_le_bytes()
        || page[24..32] != page_ordinal.to_le_bytes()
        || page[40..48] != *page_tag
        || page[..8] != XxHash64::oneshot(0, &page[8..]).to_le_bytes()
    {
        return None;
    }
    Some((
        u64::from_le_bytes(page[32..40].try_into().unwrap()),
        &page[PAGE_HEADER_BYTES..],
    ))
}

/// Content length, key length, exact object range, region count, payload checksum, physical ranges, full key, batch ID.
/// All integers are little-endian u64s. Page links describe the entry metadata's own allocations.
/// The payload checksum covers exactly the entry bytes in region order, excluding final alignment padding.
pub(super) fn encode_entry_metadata(
    key: &str,
    object_range: ByteRange,
    payload_regions: &[DiskRegion],
    payload_checksum: u64,
    batch_id: &[u8; 16],
) -> Bytes {
    let length = 64 + 16 * payload_regions.len() + key.len();
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
    bytes.extend_from_slice(&payload_checksum.to_le_bytes());
    for region in payload_regions {
        let disk_range = region.range();
        bytes.extend_from_slice(&disk_range.start.to_le_bytes());
        bytes.extend_from_slice(&disk_range.end.to_le_bytes());
    }
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(batch_id);
    Bytes::from(bytes)
}
