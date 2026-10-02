//! Experimental v11 metadata-only chunks. Each chunk contains 255 independently checksummed
//! record pages and one checksummed next-chunk page. Zero records are unused slots.
//! Payloads live in separate chunks. Records never cross page boundaries.

use bytes::Bytes;
use feuer_types::{ByteRange, ObjectKeyHash};
use twox_hash::XxHash64;

use std::ops::Range;

/// Size in bytes of a metadata page, including its header and padding.
/// Each entry metadata page holds 84 complete 48-byte records and 16 padding bytes.
pub(super) const METADATA_PAGE_BYTES: usize = 4096;
pub(super) const PAGE_HEADER_BYTES: usize = 48;
pub(super) const ENTRY_METADATA_BYTES: usize = 48;
pub(super) const PAGE_CONTENT_BYTES: usize =
    (METADATA_PAGE_BYTES - PAGE_HEADER_BYTES) / ENTRY_METADATA_BYTES * ENTRY_METADATA_BYTES;
pub(super) const ENTRY_METADATA_PAGE_TAG: &[u8; 8] = b"FEUDES11";
pub(super) const NEXT_CHUNK_PAGE_TAG: &[u8; 8] = b"FEUNXT11";
pub(super) const RECORDS_PER_PAGE: usize = PAGE_CONTENT_BYTES / ENTRY_METADATA_BYTES;
pub(super) const RECORD_PAGES: usize = crate::allocation::CHUNK_BYTES as usize / METADATA_PAGE_BYTES - 1;
pub(super) const RECORDS_PER_CHUNK: usize = RECORD_PAGES * RECORDS_PER_PAGE;
pub(super) const NO_CHUNK: u64 = u64::MAX;

/// The reserved content-checksum field is zero in v11; each page is validated independently.
pub(super) fn encode_page(
    page: &mut [u8],
    page_tag: &[u8; 8],
    content_checksum: u64,
    page_address: u64,
    page_ordinal: u64,
    entry_count: u64,
    contents: &[u8],
) {
    assert_eq!(page.len(), METADATA_PAGE_BYTES);
    assert!(contents.len() <= PAGE_CONTENT_BYTES);
    page.fill(0);
    page[8..16].copy_from_slice(&content_checksum.to_le_bytes());
    page[16..24].copy_from_slice(&page_address.to_le_bytes());
    page[24..32].copy_from_slice(&page_ordinal.to_le_bytes());
    page[32..40].copy_from_slice(&entry_count.to_le_bytes());
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

/// Key hash (u128), object start and length, payload address, payload checksum (u64s).
/// All integers are little-endian. Aligned payload length is derived from object length.
/// Payload checksums exclude alignment padding.
pub(super) fn encode_entry_metadata(
    key: &ObjectKeyHash,
    object_range: ByteRange,
    payload_range: &Range<u64>,
    payload_checksum: u64,
) -> Bytes {
    let mut bytes = Vec::with_capacity(ENTRY_METADATA_BYTES);
    bytes.extend_from_slice(&key.0.to_le_bytes());
    for value in [
        object_range.start(),
        object_range.len(),
        payload_range.start,
        payload_checksum,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Bytes::from(bytes)
}
