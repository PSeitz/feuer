//! Experimental v13 metadata-only chunks. Each chunk contains 255 independently checksummed
//! record pages and one checksummed next-chunk page. All-zero entry metadata is unused.
//! Payloads live in separate chunks. Records never cross page boundaries.

use feuer_types::{ByteRange, ObjectKeyHash};
use twox_hash::XxHash64;

use crate::allocation::CHUNK_BYTES;

/// Size in bytes of a metadata page, including its header and padding.
/// Each entry metadata page holds 84 complete 48-byte records and 32 padding bytes.
pub(super) const METADATA_PAGE_BYTES: usize = 4096;
pub(super) const PAGE_HEADER_BYTES: usize = 32;
pub(super) const ENTRY_METADATA_BYTES: usize = 48;
pub(super) const PAGE_CONTENT_BYTES: usize =
    (METADATA_PAGE_BYTES - PAGE_HEADER_BYTES) / ENTRY_METADATA_BYTES * ENTRY_METADATA_BYTES;
pub(super) const ENTRY_METADATA_PAGE_TAG: &[u8; 8] = b"FEUDES13";
pub(super) const NEXT_CHUNK_PAGE_TAG: &[u8; 8] = b"FEUNXT13";
pub(super) const RECORDS_PER_PAGE: usize = PAGE_CONTENT_BYTES / ENTRY_METADATA_BYTES;
pub(super) const RECORD_PAGES: usize = CHUNK_BYTES as usize / METADATA_PAGE_BYTES - 1;
pub(super) const RECORDS_PER_CHUNK: usize = RECORD_PAGES * RECORDS_PER_PAGE;
pub(super) const NO_CHUNK: u64 = u64::MAX;

/// The reserved content-checksum field is zero; each page is validated independently.
pub(super) fn encode_page(page: &mut [u8], page_tag: &[u8; 8], entry_count: u64, contents: &[u8]) {
    assert_eq!(page.len(), METADATA_PAGE_BYTES);
    assert!(contents.len() <= PAGE_CONTENT_BYTES);
    page.fill(0);
    page[16..24].copy_from_slice(&entry_count.to_le_bytes());
    page[24..32].copy_from_slice(page_tag);
    page[PAGE_HEADER_BYTES..PAGE_HEADER_BYTES + contents.len()].copy_from_slice(contents);
    let checksum = XxHash64::oneshot(0, &page[8..]);
    page[..8].copy_from_slice(&checksum.to_le_bytes());
}

/// Validates one complete metadata page. The tag identifies the writer's format;
/// an intact checksum lets readers rely on its field, padding, and alignment guarantees.
pub(super) fn validate_page<'a>(page: &'a [u8], page_tag: &[u8; 8]) -> Option<&'a [u8]> {
    if page[24..32] != *page_tag || page[..8] != XxHash64::oneshot(0, &page[8..]).to_le_bytes() {
        return None;
    }
    Some(&page[PAGE_HEADER_BYTES..])
}

/// Key hash (u128), object start and length, payload address, payload checksum (u64s).
/// All integers are little-endian. Aligned payload length is derived from object length.
/// Payload checksums exclude alignment padding.
pub(super) fn encode_entry_metadata(
    key: &ObjectKeyHash,
    object_range: ByteRange,
    payload_address: u64,
    payload_checksum: u64,
) -> [u8; ENTRY_METADATA_BYTES] {
    let mut bytes = [0; ENTRY_METADATA_BYTES];
    bytes[..16].copy_from_slice(&key.0.to_le_bytes());
    bytes[16..24].copy_from_slice(&object_range.start().to_le_bytes());
    bytes[24..32].copy_from_slice(&object_range.len().to_le_bytes());
    bytes[32..40].copy_from_slice(&payload_address.to_le_bytes());
    bytes[40..48].copy_from_slice(&payload_checksum.to_le_bytes());
    bytes
}
